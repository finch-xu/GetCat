#!/usr/bin/env python3
"""把 assets/logo/cat.png（去背的猫）合成成应用 logo，输出两份 PNG。

  scripts/gen-logo.py

产物：
  crates/getcat-app/assets/logo/getcat.png       512×512，圆角块铺满画布 —— app 侧栏与 README 用
  crates/getcat-app/resources/macos/getcat-1024.png
                                                 1024×1024，圆角块缩到 824 居中 —— macOS .icns 用

只在 logo 改动时由开发者手动跑一次并提交产物；CI 不调用本脚本（runner 上不装
Pillow，且位图合成结果固定下来才可复现）。依赖：pip install pillow numpy

为什么是「超椭圆」而不是普通圆角矩形：macOS Big Sur 之后的应用图标是连续曲率的
squircle，圆弧角在角落有肉眼可见的曲率突变，一眼就不像系统自带图标。这里用超椭圆
|x/a|^n + |y/a|^n = 1（n=5）近似，配 4× 超采样做抗锯齿。

为什么 macOS 那份要缩到 824/1024（≈80.5%）：Apple 的图标网格要求圆角方块只占画布
的这个比例、四周留透明边距，否则在 Dock 里会比其他应用的图标大一圈。
"""

from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

REPO_ROOT = Path(__file__).resolve().parent.parent
CAT_SRC = REPO_ROOT / "crates/getcat-app/assets/logo/cat.png"
APP_LOGO = REPO_ROOT / "crates/getcat-app/assets/logo/getcat.png"
MACOS_ICON = REPO_ROOT / "crates/getcat-app/resources/macos/getcat-1024.png"

SUPERSAMPLE = 4          # 圆角边缘的抗锯齿倍率
SQUIRCLE_EXPONENT = 5.0  # 超椭圆指数，越大越方；5 最接近 macOS 图标
GRADIENT_TOP = (255, 252, 245)
GRADIENT_BOTTOM = (255, 233, 201)
CAT_HEIGHT_RATIO = 0.82  # 猫的高度占圆角块边长的比例
CAT_OPTICAL_LIFT = 0.006 # 视觉重心在头部，整体上提一点点才「居中」
SHADOW_BLUR_RATIO = 0.013
SHADOW_OFFSET_RATIO = 0.012
SHADOW_COLOR = (120, 72, 20)
SHADOW_OPACITY = 0.16

MACOS_CANVAS = 1024
MACOS_TILE = 824
APP_LOGO_SIZE = 512


def squircle_mask(side: int) -> Image.Image:
    """边长 side 的超椭圆遮罩（"L" 模式），边缘由超采样得到灰度过渡。"""
    hi = side * SUPERSAMPLE
    # 像素中心归一化到 [-1, 1]
    axis = (np.arange(hi, dtype=np.float64) + 0.5) / hi * 2.0 - 1.0
    power = np.abs(axis) ** SQUIRCLE_EXPONENT
    inside = power[None, :] + power[:, None] <= 1.0
    mask = Image.fromarray((inside * 255).astype(np.uint8), mode="L")
    return mask.resize((side, side), Image.Resampling.BOX)


def gradient(side: int) -> Image.Image:
    """自上而下的线性渐变底板。"""
    t = np.linspace(0.0, 1.0, side, dtype=np.float64)[:, None]
    top = np.array(GRADIENT_TOP, dtype=np.float64)
    bottom = np.array(GRADIENT_BOTTOM, dtype=np.float64)
    column = top + (bottom - top) * t                   # (side, 3)
    rows = np.repeat(column[:, None, :], side, axis=1)  # (side, side, 3)
    return Image.fromarray(np.round(rows).astype(np.uint8), mode="RGB")


def fit_cat(side: int) -> tuple[Image.Image, int, int]:
    """按 side 缩放猫，返回（缩放后的图，左上角 x，左上角 y）。"""
    cat = Image.open(CAT_SRC).convert("RGBA")
    box = cat.getbbox()  # 忽略源图四周可能残留的透明留白
    if box:
        cat = cat.crop(box)

    scale = side * CAT_HEIGHT_RATIO / cat.height
    size = (max(1, round(cat.width * scale)), max(1, round(cat.height * scale)))
    # 在预乘 alpha 空间里缩放，否则透明像素的 RGB 会被插值带进边缘，形成暗边
    cat = cat.convert("RGBa").resize(size, Image.Resampling.LANCZOS).convert("RGBA")

    x = (side - cat.width) // 2
    y = (side - cat.height) // 2 - round(side * CAT_OPTICAL_LIFT)
    return cat, x, y


def render_tile(side: int) -> Image.Image:
    """铺满 side×side 的圆角 logo（方块外透明）。"""
    tile = gradient(side).convert("RGBA")
    cat, x, y = fit_cat(side)

    # 猫脚下一层很淡的暖色投影，让它从奶油底上「浮」起来
    shadow = Image.new("RGBA", tile.size, SHADOW_COLOR + (0,))
    alpha = Image.new("L", tile.size, 0)
    alpha.paste(cat.getchannel("A"), (x, y + round(side * SHADOW_OFFSET_RATIO)))
    alpha = alpha.filter(ImageFilter.GaussianBlur(side * SHADOW_BLUR_RATIO))
    shadow.putalpha(alpha.point(lambda v: round(v * SHADOW_OPACITY)))
    tile = Image.alpha_composite(tile, shadow)

    tile.alpha_composite(cat, (x, y))
    tile.putalpha(squircle_mask(side))
    return tile


def main() -> None:
    app_logo = render_tile(APP_LOGO_SIZE)
    APP_LOGO.parent.mkdir(parents=True, exist_ok=True)
    app_logo.save(APP_LOGO, optimize=True)
    print(f"已写入 {APP_LOGO.relative_to(REPO_ROOT)}（{app_logo.width}×{app_logo.height}）")

    icon = Image.new("RGBA", (MACOS_CANVAS, MACOS_CANVAS), (0, 0, 0, 0))
    offset = (MACOS_CANVAS - MACOS_TILE) // 2
    icon.alpha_composite(render_tile(MACOS_TILE), (offset, offset))
    MACOS_ICON.parent.mkdir(parents=True, exist_ok=True)
    icon.save(MACOS_ICON, optimize=True)
    print(f"已写入 {MACOS_ICON.relative_to(REPO_ROOT)}（{icon.width}×{icon.height}）")


if __name__ == "__main__":
    main()
