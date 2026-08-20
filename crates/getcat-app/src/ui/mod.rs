//! UI 公共小工具：颜色映射与数字格式化。

pub mod body_view;
pub mod kv_table;
pub mod request_pane;
pub mod response_pane;
pub mod sidebar;
pub mod url_bar;

use std::time::Duration;

use getcat_core::model::Method;
use gpui::{App, Hsla};
use gpui_component::ActiveTheme;

pub fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

pub fn format_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1000.0 {
        format!("{} ms", ms.round().max(1.0) as u64)
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

pub fn method_color(method: Method, cx: &App) -> Hsla {
    let t = cx.theme();
    match method {
        Method::Get => t.success,
        Method::Post => t.warning,
        Method::Put => t.info,
        Method::Patch => t.primary,
        Method::Delete => t.danger,
        Method::Head | Method::Options => t.muted_foreground,
    }
}

pub fn status_color(status: u16, cx: &App) -> Hsla {
    let t = cx.theme();
    match status {
        200..=299 => t.success,
        300..=399 => t.info,
        400..=499 => t.warning,
        500..=599 => t.danger,
        _ => t.muted_foreground,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bytes_formatting() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(312), "312 B");
        assert_eq!(format_bytes(49_357), "48.2 KB");
        assert_eq!(format_bytes(1_572_864), "1.5 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(Duration::from_millis(312)), "312 ms");
        assert_eq!(format_duration(Duration::from_millis(1250)), "1.25 s");
        assert_eq!(format_duration(Duration::from_micros(800)), "1 ms");
    }
}
