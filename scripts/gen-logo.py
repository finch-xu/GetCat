#!/usr/bin/env python3
"""把 assets/logo/ 下的两份 SVG 原画合成成应用 logo，输出浅 / 深两套 PNG 与一份 ICO。

  scripts/gen-logo.py

原画（只画猫与抓娃娃机的爪子，透明底，512×512 坐标系）：
  crates/getcat-app/assets/logo/getcat-art.svg       完整版 —— ≥ 48 px 的所有产物
  crates/getcat-app/assets/logo/getcat-art-mini.svg  简化版 —— ≤ 32 px：去掉脸部细节与条纹，
                                                     吊绳、爪子、四肢加粗，缩小后仍认得出

两套配色，取值与画布 Logo 画板一致（浅色 = 「浅色底」那张的 .t-light，深色 = .t-dark）：
  浅色（默认）  奶油底板，石墨色爪子 —— README、三平台系统图标、app 浅色主题
  深色          夜蓝底板，浅金属爪子 —— 只给 app 深色主题用
猫身上的颜色两套一样，只有底板、吊绳和爪子随配色变。原画里吊绳、爪子主体、爪齿、
爪子暗部按角色标了 class（cable / metal / metal-line / metal-shade）；栅格化前按 Variant
的色表往 SVG 里注入一段 <style> 给这几个角色上色。不按色值替换：深色稿里吊绳和爪子暗部
是同一个灰（#AEB9CC），浅色稿却要把它们拆成两种，色值区分不开，只能按角色。

产物：
  crates/getcat-app/assets/logo/getcat.png            512×512，浅色，超椭圆底板铺满画布
                                                      —— README、Linux 窗口图标、app 浅色主题的关于页
  crates/getcat-app/assets/logo/getcat-dark.png       512×512，深色 —— app 深色主题的关于页
  crates/getcat-app/assets/logo/getcat-mini.png       128×128，浅色简化版 —— app 浅色主题的图标栏（26 px 显示）
  crates/getcat-app/assets/logo/getcat-mini-dark.png  128×128，深色简化版 —— app 深色主题的图标栏
  crates/getcat-app/resources/macos/getcat-1024.png
                                                      1024×1024，浅色，底板缩到 824 居中 —— macOS .icns 用
  crates/getcat-app/resources/windows/getcat.ico      多尺寸（16…256），浅色，底板铺满画布；16/24/32 用简化版
                                                      —— build.rs 嵌进 exe 资源（资源管理器 / 任务栏 / 窗口图标）

只在 logo 改动时由开发者手动跑一次并提交产物；CI 不调用本脚本。依赖：
  pip install pillow numpy
SVG 栅格化交给 tools/svg-raster（resvg，版本与根 Cargo.lock 对齐），首次运行时自动 cargo build。

为什么是「超椭圆」而不是普通圆角矩形：macOS Big Sur 之后的应用图标是连续曲率的
squircle，圆弧角在角落有肉眼可见的曲率突变，一眼就不像系统自带图标。这里用超椭圆
|x/a|^n + |y/a|^n = 1（n=5）近似，配 4× 超采样做抗锯齿。底板因此不画在 SVG 里，
由本脚本合成，原画越出底板的部分（伸出顶边的吊绳）随遮罩一起裁掉。

为什么 macOS 那份要缩到 824/1024（≈80.5%）：Apple 的图标网格要求圆角方块只占画布
的这个比例、四周留透明边距，否则在 Dock 里会比其他应用的图标大一圈。

为什么 Windows 的 .ico 用铺满版而不是 macOS 那份：Windows 图标没有 Apple 那套留边
规范，带边距的图在任务栏里会明显偏小。ICO 每档尺寸都是独立位图（不是让系统缩一张
大图），所以逐档单独栅格化，小档还换成简化版，16 px 那档才不会糊成一团。
"""

import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from PIL import Image

REPO_ROOT = Path(__file__).resolve().parent.parent
LOGO_DIR = REPO_ROOT / "crates/getcat-app/assets/logo"
ART_FULL = LOGO_DIR / "getcat-art.svg"
ART_MINI = LOGO_DIR / "getcat-art-mini.svg"
APP_LOGO = LOGO_DIR / "getcat.png"
APP_LOGO_DARK = LOGO_DIR / "getcat-dark.png"
APP_LOGO_MINI = LOGO_DIR / "getcat-mini.png"
APP_LOGO_MINI_DARK = LOGO_DIR / "getcat-mini-dark.png"
MACOS_ICON = REPO_ROOT / "crates/getcat-app/resources/macos/getcat-1024.png"
WINDOWS_ICON = REPO_ROOT / "crates/getcat-app/resources/windows/getcat.ico"
RASTER_TOOL_DIR = REPO_ROOT / "tools/svg-raster"
RASTER_TOOL = RASTER_TOOL_DIR / "target/release/svg-raster"

SUPERSAMPLE = 4          # 底板边缘的抗锯齿倍率
SQUIRCLE_EXPONENT = 5.0  # 超椭圆指数，越大越方；5 最接近 macOS 图标
GLINT_OPACITY = 0.05     # 左上角那道玻璃反光（抓娃娃机的玻璃罩）的不透明度
GLINT_EXTENT = 300 / 512  # 反光三角形两条直角边占边长的比例
MINI_MAX_SIDE = 32       # 不超过这个边长的产物用简化版

MACOS_CANVAS = 1024
MACOS_TILE = 824
APP_LOGO_SIZE = 512
APP_LOGO_MINI_SIZE = 128
# Windows 图标习惯提供的档位；256 那档 Pillow 会自动用 PNG 压缩存放
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)


@dataclass(frozen=True)
class Variant:
    tile: str         # 底板
    cable: str        # 吊绳
    metal: str        # 爪子主体与爪齿
    metal_shade: str  # 爪子暗部：吊绳卡箍、底部阴影条、两个关节圆点

    def style(self) -> str:
        """注入原画的 <style>：CSS 优先级高于元素上的 fill / stroke 属性，按 class 覆盖。"""
        return (
            "<style>"
            f".cable{{stroke:{self.cable}}}"
            f".metal{{fill:{self.metal}}}"
            f".metal-line{{stroke:{self.metal}}}"
            f".metal-shade{{fill:{self.metal_shade}}}"
            "</style>"
        )

    def tile_rgb(self) -> tuple[int, int, int]:
        h = self.tile.lstrip("#")
        return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16))


# 夜蓝底板，浅金属爪子（原画元素上写的就是这一套）
DARK = Variant(tile="#1A2540", cable="#AEB9CC", metal="#D9E0EB", metal_shade="#AEB9CC")
# 奶油底板，石墨色爪子：橘猫在暖底上依旧醒目，爪子压深才不会和浅底糊成一片
LIGHT = Variant(tile="#FFF1DF", cable="#8B96A8", metal="#5D6879", metal_shade="#46505F")

# 每份原画至少要有的角色：缺了说明原画改过而脚本没跟上，宁可报错也别悄悄产出没换色的图
REQUIRED_ROLES = ("cable", "metal", "metal-line")


def ensure_raster_tool() -> Path:
    """tools/svg-raster 还没编过就编一次（离线也行：依赖都在根 Cargo.lock 的缓存里）。"""
    if not RASTER_TOOL.exists():
        subprocess.run(
            ["cargo", "build", "--release", "--quiet"],
            cwd=RASTER_TOOL_DIR,
            check=True,
        )
    return RASTER_TOOL


def render_art(svg: Path, side: int, variant: Variant) -> Image.Image:
    """把原画按 variant 的角色色表上色后栅格化成 side×side 的 RGBA。"""
    text = svg.read_text(encoding="utf-8")
    for role in REQUIRED_ROLES:
        assert f'class="{role}"' in text, f"{svg.name} 缺 {role} 角色，同步改原画或 REQUIRED_ROLES"
    # <style> 紧跟在 <svg …> 开标签之后
    tag_end = text.index(">", text.index("<svg")) + 1
    text = text[:tag_end] + variant.style() + text[tag_end:]
    with tempfile.TemporaryDirectory() as tmp:
        src = Path(tmp) / "art.svg"
        src.write_text(text, encoding="utf-8")
        out = Path(tmp) / "art.png"
        subprocess.run(
            [str(ensure_raster_tool()), str(src), str(out), str(side)],
            check=True,
        )
        return Image.open(out).convert("RGBA")


def squircle_mask(side: int) -> Image.Image:
    """边长 side 的超椭圆遮罩（"L" 模式），边缘由超采样得到灰度过渡。"""
    hi = side * SUPERSAMPLE
    # 像素中心归一化到 [-1, 1]
    axis = (np.arange(hi, dtype=np.float64) + 0.5) / hi * 2.0 - 1.0
    power = np.abs(axis) ** SQUIRCLE_EXPONENT
    inside = power[None, :] + power[:, None] <= 1.0
    mask = Image.fromarray((inside * 255).astype(np.uint8), mode="L")
    return mask.resize((side, side), Image.Resampling.BOX)


def glint(side: int) -> Image.Image:
    """左上角一道很淡的白色三角反光：x + y < GLINT_EXTENT · side 的区域。"""
    ys, xs = np.mgrid[0:side, 0:side].astype(np.float64) + 0.5
    inside = xs + ys < GLINT_EXTENT * side
    alpha = (inside * round(255 * GLINT_OPACITY)).astype(np.uint8)
    layer = Image.new("RGBA", (side, side), (255, 255, 255, 0))
    layer.putalpha(Image.fromarray(alpha, mode="L"))
    return layer


def render_tile(side: int, variant: Variant = LIGHT, mini: bool | None = None) -> Image.Image:
    """铺满 side×side 的 logo（超椭圆外透明）。mini 为 None 时按边长自动选原画。"""
    if mini is None:
        mini = side <= MINI_MAX_SIDE
    tile = Image.new("RGBA", (side, side), variant.tile_rgb() + (255,))
    if not mini:
        tile = Image.alpha_composite(tile, glint(side))
    art = render_art(ART_MINI if mini else ART_FULL, side, variant)
    tile = Image.alpha_composite(tile, art)
    tile.putalpha(squircle_mask(side))
    return tile


def save(image: Image.Image, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    image.save(path, optimize=True)
    print(f"已写入 {path.relative_to(REPO_ROOT)}（{image.width}×{image.height}）")


def main() -> None:
    save(render_tile(APP_LOGO_SIZE), APP_LOGO)
    save(render_tile(APP_LOGO_SIZE, DARK), APP_LOGO_DARK)
    save(render_tile(APP_LOGO_MINI_SIZE, mini=True), APP_LOGO_MINI)
    save(render_tile(APP_LOGO_MINI_SIZE, DARK, mini=True), APP_LOGO_MINI_DARK)

    icon = Image.new("RGBA", (MACOS_CANVAS, MACOS_CANVAS), (0, 0, 0, 0))
    offset = (MACOS_CANVAS - MACOS_TILE) // 2
    icon.alpha_composite(render_tile(MACOS_TILE), (offset, offset))
    save(icon, MACOS_ICON)

    # 每档单独栅格化：Pillow 存 ICO 时，append_images 里尺寸对得上的位图原样写入，不再缩放
    WINDOWS_ICON.parent.mkdir(parents=True, exist_ok=True)
    largest, *rest = sorted(ICO_SIZES, reverse=True)
    render_tile(largest).save(
        WINDOWS_ICON,
        format="ICO",
        sizes=[(s, s) for s in ICO_SIZES],
        append_images=[render_tile(s) for s in rest],
    )
    print(f"已写入 {WINDOWS_ICON.relative_to(REPO_ROOT)}（{', '.join(map(str, ICO_SIZES))}）")


if __name__ == "__main__":
    main()
