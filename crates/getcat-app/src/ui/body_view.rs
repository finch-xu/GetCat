//! B/C 档纯文本视图与 Headers 列表：gpui uniform_list 只渲染可见行，每帧工作量 O(可见行数)。

use std::sync::Arc;

use getcat_core::body::text::{TextDoc, clip_line};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, IntoElement, ListHorizontalSizingBehavior, ParentElement, SharedString, Styled,
    UniformListScrollHandle, div, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, h_flex,
    scroll::{Scrollbar, ScrollbarAxis},
};

/// 每行固定高度（uniform_list 要求等高；按第一项测量，所有行都用同一个 h()）。
pub const LINE_HEIGHT_PX: f32 = 20.;

fn digits(n: usize) -> usize {
    n.max(1).ilog10() as usize + 1
}

/// 按行虚拟化渲染整份文档：行号 + 文本；超过 MAX_LINE_CHARS 的行截断并标注。
/// `doc` 被 move 进渲染闭包，闭包每帧只对可见区间切片并分配对应数量的 SharedString。
pub fn render_text_lines(
    id: &'static str,
    doc: Arc<TextDoc>,
    handle: &UniformListScrollHandle,
    cx: &App,
) -> impl IntoElement {
    let line_count = doc.line_count();
    let longest = doc.longest_line();
    let gutter = px(16. + 8. * digits(line_count) as f32);
    let muted = cx.theme().muted_foreground;
    let warning = cx.theme().warning;

    let list = uniform_list(id, line_count, move |range, _window, _cx| {
        range
            .map(|ix| {
                let clipped = clip_line(doc.line(ix));
                h_flex()
                    .h(px(LINE_HEIGHT_PX))
                    .items_center()
                    .whitespace_nowrap()
                    .child(
                        div()
                            .w(gutter)
                            .flex_none()
                            .pr_2()
                            .text_right()
                            .text_color(muted)
                            .child(SharedString::from((ix + 1).to_string())),
                    )
                    .child(div().child(SharedString::from(clipped.text.to_string())))
                    .when(clipped.hidden_chars > 0, |h| {
                        h.child(div().ml_2().text_color(warning).child(SharedString::from(
                            format!("… 已截断，剩余 {} 字符", clipped.hidden_chars),
                        )))
                    })
            })
            .collect::<Vec<_>>()
    })
    // 横向宽度按最长行测量，并允许横向滚动
    .with_width_from_item(longest)
    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
    .track_scroll(handle)
    .size_full()
    .font_family(cx.theme().mono_font_family.clone())
    .text_size(cx.theme().mono_font_size);

    div()
        .relative()
        .size_full()
        .child(list)
        .child(Scrollbar::new(handle).axis(ScrollbarAxis::Both))
}

/// 响应头列表：每行一个 Header，名称列定宽、值列截断。
pub fn render_header_rows(
    rows: Arc<[(SharedString, SharedString)]>,
    handle: &UniformListScrollHandle,
    cx: &App,
) -> impl IntoElement {
    let count = rows.len();
    let muted = cx.theme().muted_foreground;
    let list = uniform_list("response-headers", count, move |range, _window, _cx| {
        range
            .map(|ix| {
                let (name, value) = &rows[ix];
                h_flex()
                    .h(px(24.))
                    .items_center()
                    .gap_3()
                    .px_3()
                    .text_sm()
                    .child(
                        div()
                            .w(px(260.))
                            .flex_none()
                            .text_color(muted)
                            .truncate()
                            .child(name.clone()),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(value.clone()))
            })
            .collect::<Vec<_>>()
    })
    .track_scroll(handle)
    .size_full()
    .font_family(cx.theme().mono_font_family.clone());

    div()
        .relative()
        .size_full()
        .child(list)
        .child(Scrollbar::vertical(handle))
}
