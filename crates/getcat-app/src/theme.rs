//! GetCat 配色：把 BucketCat 的主题变量装进 gpui-component 的 Theme。
//!
//! 色值取自 BucketCat 的 `src/index.css`（`:root` 与 `.dark` 两组变量），
//! 只有 `muted.foreground` 是相对它的调整：原值 #8794a1 在白底上只有
//! 2.99:1，而它承载了侧栏副标题、参数描述、耗时与大小等大量次要文字。
//!
//! 主题不是每次切换后打补丁，而是替换 [`Theme`] 的 `light_theme` /
//! `dark_theme` 两个配置：此后 `Theme::change` 与
//! `Theme::sync_system_appearance` 自动用这两套，跟随系统外观也不会退回
//! gpui-component 的默认灰。

use getcat_core::model::Method;
use gpui::{App, Hsla, rgb};
use gpui_component::{ActiveTheme, Theme, ThemeRegistry, scroll::ScrollbarMode};

const THEME_JSON: &str = include_str!("theme.json");
const LIGHT: &str = "GetCat Light";
const DARK: &str = "GetCat Dark";

/// 产品级调色板：gpui-component 的 Theme 没有「HTTP 方法」「状态码」这两个语义角色，
/// 按设计指南它们属于产品自己的 token 层——原始色值只允许出现在这里，
/// 调用点一律通过 [`palette`] 按语义取色。
///
/// 方法色不复用语义色：方法名在侧栏是 11px 粗体、URL 栏 12px 粗体，
/// 直接取 success / warning / info / primary 在浅色底上只有 2.9–4.2:1。
/// 这两组是在 OKLCH 里保持色相与彩度、只调明度求出来的，在各自主题的
/// 背景 / 面板 / 侧栏 / 选中行四种底色上都 ≥ 4.8:1。
/// 状态码色比方法色再深一档：它压在同色描边的 Tag 里，不是压在页面背景上。
pub struct Palette {
    method_get: u32,
    method_post: u32,
    method_put: u32,
    method_patch: u32,
    method_delete: u32,
    method_other: u32,
    status_2xx: u32,
    status_3xx: u32,
    status_4xx: u32,
    status_5xx: u32,
}

const LIGHT_PALETTE: Palette = Palette {
    method_get: 0x007762,
    method_post: 0x925d0c,
    method_put: 0x226ea2,
    method_patch: 0x6d5ea6,
    method_delete: 0xb74131,
    method_other: 0x5e6a77,
    status_2xx: 0x007460,
    status_3xx: 0x67599f,
    status_4xx: 0x8a5600,
    status_5xx: 0xaf3829,
};

const DARK_PALETTE: Palette = Palette {
    method_get: 0x4bb39a,
    method_post: 0xd7a042,
    method_put: 0x6fa8d4,
    method_patch: 0xa596d8,
    method_delete: 0xe77968,
    method_other: 0x8f9aa3,
    status_2xx: 0x6ccbb2,
    status_3xx: 0xb5a8e0,
    status_4xx: 0xe0ad50,
    status_5xx: 0xe58474,
};

impl Palette {
    pub fn method(&self, method: Method) -> Hsla {
        rgb(match method {
            Method::Get => self.method_get,
            Method::Post => self.method_post,
            Method::Put => self.method_put,
            Method::Patch => self.method_patch,
            Method::Delete => self.method_delete,
            Method::Head | Method::Options => self.method_other,
        })
        .into()
    }

    /// 范围外的状态码（如 HTTP/0.9 或异常值）不给语义色。
    pub fn status(&self, status: u16) -> Option<Hsla> {
        let hex = match status {
            200..=299 => self.status_2xx,
            300..=399 => self.status_3xx,
            400..=499 => self.status_4xx,
            500..=599 => self.status_5xx,
            _ => return None,
        };
        Some(rgb(hex).into())
    }
}

/// 当前主题模式对应的产品调色板。
pub fn palette(cx: &App) -> &'static Palette {
    if cx.theme().is_dark() {
        &DARK_PALETTE
    } else {
        &LIGHT_PALETTE
    }
}

/// 把 GetCat 的浅色 / 深色配色装成当前主题。
///
/// 必须在 `gpui_component::init` 之后调用。任何一步失败都只记日志：
/// 配色装不上时界面退回 gpui-component 的默认主题，功能不受影响。
pub fn install(cx: &mut App) {
    if let Err(err) = ThemeRegistry::global_mut(cx).load_themes_from_str(THEME_JSON) {
        tracing::error!("加载 GetCat 主题失败，沿用默认配色：{err}");
        return;
    }

    let registry = ThemeRegistry::global(cx);
    let (Some(light), Some(dark)) = (
        registry.themes().get(LIGHT).cloned(),
        registry.themes().get(DARK).cloned(),
    ) else {
        tracing::error!("GetCat 主题未出现在注册表里，沿用默认配色");
        return;
    };

    let theme = Theme::global_mut(cx);
    theme.light_theme = light;
    theme.dark_theme = dark;
    let mode = theme.mode;
    // 重新套用当前模式，让刚换上的配置立刻生效（并同步 Base 层）。
    Theme::change(mode, None, cx);

    // 滚动条常驻。上游默认是 `Scrolling`——只在滚动时画、静止后淡出，于是「这块内容
    // 还能往右拉」这件事根本没有可见线索：响应体里一行超宽 JSON 看上去就只是被切掉了。
    // 接口调试要盯的恰恰是长行，所以这里换成 `Always`。
    //
    // 必须放在 `Theme::change` **之后**：`change` 会用 `scrollbar_mode` 重建 Base 层投影，
    // 反过来写就被它按旧值覆盖了。此后 `change` / `sync_system_appearance` 都从
    // `Theme::scrollbar_mode` 读，切明暗不会退回默认（有测试钉住）。
    Theme::set_scrollbar_mode(ScrollbarMode::Always, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Hsla, TestAppContext};
    use gpui_component::{ActiveTheme, ThemeMode};

    fn hex(color: Hsla) -> u32 {
        let rgba = gpui::Rgba::from(color);
        let to8 = |v: f32| (v * 255.0).round() as u32;
        (to8(rgba.r) << 16) | (to8(rgba.g) << 8) | to8(rgba.b)
    }

    /// 滚动条常驻，且切换明暗后不退回上游默认的「滚动时才画」。
    #[gpui::test]
    fn install_pins_the_scrollbar_to_always_visible(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            install(cx);
            assert_eq!(Theme::global(cx).scrollbar_mode, ScrollbarMode::Always);

            for mode in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::Light] {
                Theme::change(mode, None, cx);
                assert_eq!(
                    Theme::global(cx).scrollbar_mode,
                    ScrollbarMode::Always,
                    "切到 {mode:?} 后滚动条退回了默认模式"
                );
            }
        });
    }

    /// 配色取自 BucketCat 的 src/index.css；muted_foreground 是唯一一处
    /// 相对它的调整（原值 #8794a1 在白底上只有 2.99:1）。
    #[gpui::test]
    fn install_replaces_the_default_palette(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            install(cx);

            Theme::change(ThemeMode::Light, None, cx);
            assert_eq!(hex(cx.theme().background), 0xffffff);
            assert_eq!(hex(cx.theme().foreground), 0x1c2329);
            assert_eq!(hex(cx.theme().primary), 0x3f87bd);
            assert_eq!(hex(cx.theme().sidebar), 0xf4f6f8);
            assert_eq!(hex(cx.theme().title_bar), 0xf4f6f8);
            assert_eq!(hex(cx.theme().border), 0xe3e8ed);
            assert_eq!(hex(cx.theme().muted_foreground), 0x5e6a77);

            Theme::change(ThemeMode::Dark, None, cx);
            assert_eq!(hex(cx.theme().background), 0x16191c);
            assert_eq!(hex(cx.theme().foreground), 0xeef1f4);
            assert_eq!(hex(cx.theme().primary), 0x6fa8d4);
            assert_eq!(hex(cx.theme().sidebar), 0x14171a);
            assert_eq!(hex(cx.theme().title_bar), 0x14171a);
            assert_eq!(hex(cx.theme().border), 0x2a3037);
            assert_eq!(hex(cx.theme().muted_foreground), 0x8f9aa3);
        });
    }
}
