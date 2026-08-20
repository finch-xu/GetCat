//! 左侧栏：v1 只有“已保存请求”的空状态；Plan 3 填充列表。

use gpui::*;
use gpui_component::{ActiveTheme, h_flex, v_flex};

pub fn render_sidebar(cx: &App) -> impl IntoElement {
    v_flex()
        .size_full()
        .bg(cx.theme().sidebar)
        .border_r_1()
        .border_color(cx.theme().sidebar_border)
        .child(
            h_flex()
                .h(px(40.))
                .px_3()
                .items_center()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().sidebar_foreground)
                .child("已保存请求"),
        )
        .child(
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_1()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("尚无已保存的请求")
                .child(div().text_xs().child("保存功能将在后续版本提供")),
        )
}
