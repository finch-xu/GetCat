//! UI 公共小工具：颜色映射与数字格式化。

pub mod body_view;
pub mod code_sheet;
pub mod curl_sheet;
pub mod kv_table;
pub mod request_pane;
pub mod response_pane;
pub mod settings_dialog;
pub mod sidebar;
pub mod tab_strip;
pub mod text;
pub mod tool_rail;
pub mod url_bar;

use std::time::Duration;

use getcat_core::model::Method;
use gpui::{App, Hsla};
use gpui_component::ActiveTheme;

use crate::theme::palette;

pub fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = 0;
    // 1023.95 及以上用 {:.1} 会显示成 "1024.0"：继续进到下一单位
    while value >= 1023.95 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

pub fn format_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    let rounded = ms.round();
    if rounded < 1000.0 {
        format!("{} ms", rounded.max(1.0) as u64)
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

/// HTTP 方法的语义色，取自产品 token 层（见 [`crate::theme::Palette`]）。
pub fn method_color(method: Method, cx: &App) -> Hsla {
    palette(cx).method(method)
}

/// 状态码的语义色；范围外的状态码退回 muted。
pub fn status_color(status: u16, cx: &App) -> Hsla {
    palette(cx)
        .status(status)
        .unwrap_or(cx.theme().muted_foreground)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use gpui_component::{Theme, ThemeMode};
    use std::time::Duration;

    fn hex(color: Hsla) -> u32 {
        let rgba = gpui::Rgba::from(color);
        let to8 = |v: f32| (v * 255.0).round() as u32;
        (to8(rgba.r) << 16) | (to8(rgba.g) << 8) | to8(rgba.b)
    }

    /// 方法名在侧栏是 11px 粗体，直接复用语义色只有 2.9–4.2:1；
    /// 这组值在浅色的白 / 面板 / 侧栏 / 选中行四种底色上都 ≥ 4.8:1。
    #[gpui::test]
    fn method_colors_use_the_dedicated_light_palette(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            Theme::change(ThemeMode::Light, None, cx);

            assert_eq!(hex(method_color(Method::Get, cx)), 0x007762);
            assert_eq!(hex(method_color(Method::Post, cx)), 0x925d0c);
            assert_eq!(hex(method_color(Method::Put, cx)), 0x226ea2);
            assert_eq!(hex(method_color(Method::Patch, cx)), 0x6d5ea6);
            assert_eq!(hex(method_color(Method::Delete, cx)), 0xb74131);
            assert_eq!(hex(method_color(Method::Head, cx)), 0x5e6a77);
            assert_eq!(hex(method_color(Method::Options, cx)), 0x5e6a77);
        });
    }

    #[gpui::test]
    fn method_colors_use_the_dedicated_dark_palette(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);

            assert_eq!(hex(method_color(Method::Get, cx)), 0x4bb39a);
            assert_eq!(hex(method_color(Method::Post, cx)), 0xd7a042);
            assert_eq!(hex(method_color(Method::Put, cx)), 0x6fa8d4);
            assert_eq!(hex(method_color(Method::Patch, cx)), 0xa596d8);
            assert_eq!(hex(method_color(Method::Delete, cx)), 0xe77968);
            assert_eq!(hex(method_color(Method::Head, cx)), 0x8f9aa3);
        });
    }

    /// 状态码 chip 的文字压在同色 16% 底上，比方法色还需要再深一档。
    #[gpui::test]
    fn status_colors_are_readable_on_their_own_tint(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);

            Theme::change(ThemeMode::Light, None, cx);
            assert_eq!(hex(status_color(200, cx)), 0x007460);
            assert_eq!(hex(status_color(204, cx)), 0x007460);
            assert_eq!(hex(status_color(302, cx)), 0x67599f);
            assert_eq!(hex(status_color(404, cx)), 0x8a5600);
            assert_eq!(hex(status_color(500, cx)), 0xaf3829);

            Theme::change(ThemeMode::Dark, None, cx);
            assert_eq!(hex(status_color(200, cx)), 0x6ccbb2);
            assert_eq!(hex(status_color(302, cx)), 0xb5a8e0);
            assert_eq!(hex(status_color(404, cx)), 0xe0ad50);
            assert_eq!(hex(status_color(500, cx)), 0xe58474);
        });
    }

    /// 范围外的状态码（如 HTTP/0.9 或异常值）不给语义色。
    #[gpui::test]
    fn status_color_outside_known_ranges_falls_back_to_muted(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
            assert_eq!(status_color(100, cx), cx.theme().muted_foreground);
            assert_eq!(status_color(600, cx), cx.theme().muted_foreground);
        });
    }

    /// WCAG 2.1 相对亮度。
    fn relative_luminance(color: Hsla) -> f32 {
        let rgba = gpui::Rgba::from(color);
        let channel = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgba.r) + 0.7152 * channel(rgba.g) + 0.0722 * channel(rgba.b)
    }

    fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
        let (la, lb) = (relative_luminance(a), relative_luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// 选中标签的底色从「纯白 / 纯背景色」换成了淡主题色——原来它与标签栏底色只差
    /// 1.02:1，扫一眼根本认不出当前是哪个标签。方法角标压在这层新底色上，可读性
    /// 必须一并钉住：调色时任何一支方法色跌破 4.8:1，这条就会红。
    #[gpui::test]
    fn method_colors_stay_readable_on_the_active_tab(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::install(cx);

            for (label, mode) in [("浅色", ThemeMode::Light), ("深色", ThemeMode::Dark)] {
                Theme::change(mode, None, cx);
                let active: Hsla = *cx.theme().tokens.tab_active;
                for method in Method::ALL {
                    let ratio = contrast_ratio(method_color(method, cx), active);
                    assert!(
                        ratio >= 4.8,
                        "{label}下 {} 在选中标签底色上只有 {ratio:.2}:1",
                        method.as_str()
                    );
                }
                // 选中与未选中要一眼分得出，否则这次调色就白改了
                let ratio = contrast_ratio(active, *cx.theme().tokens.tab_bar);
                assert!(
                    ratio >= 1.08,
                    "{label}下选中标签与标签栏底色只差 {ratio:.3}:1，看不出哪个是当前标签"
                );
            }
        });
    }

    /// PUT 与 PATCH 过去分别取 info 与 primary，在同一支蓝上撞色。
    #[gpui::test]
    fn put_and_patch_no_longer_share_a_hue(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                Theme::change(mode, None, cx);
                assert_ne!(
                    method_color(Method::Put, cx),
                    method_color(Method::Patch, cx)
                );
            }
        });
    }

    #[test]
    fn bytes_formatting() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(312), "312 B");
        assert_eq!(format_bytes(49_357), "48.2 KB");
        assert_eq!(format_bytes(1_572_864), "1.5 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn bytes_formatting_carries_at_unit_boundaries() {
        // 1_048_570 B = 1023.99 KB：一位小数会显示成 "1024.0 KB"，必须进位
        assert_eq!(format_bytes(1_048_570), "1.0 MB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
        assert_eq!(format_bytes(1_047_000), "1022.5 KB");
        assert_eq!(format_bytes(u64::MAX), "16777216.0 TB");
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(Duration::from_millis(312)), "312 ms");
        assert_eq!(format_duration(Duration::from_millis(1250)), "1.25 s");
        assert_eq!(format_duration(Duration::from_micros(800)), "1 ms");
    }

    #[test]
    fn duration_formatting_carries_at_one_second() {
        assert_eq!(format_duration(Duration::from_micros(999_600)), "1.00 s");
        assert_eq!(format_duration(Duration::from_micros(999_400)), "999 ms");
    }
}
