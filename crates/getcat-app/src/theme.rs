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

use gpui::App;
use gpui_component::{Theme, ThemeRegistry};

const THEME_JSON: &str = include_str!("theme.json");
const LIGHT: &str = "GetCat Light";
const DARK: &str = "GetCat Dark";

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
