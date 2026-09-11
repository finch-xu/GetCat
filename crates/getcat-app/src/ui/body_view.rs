//! B/C 档纯文本视图与 Headers 列表：gpui uniform_list 只渲染可见行，每帧工作量 O(可见行数)。
//! Headers 列表的值要换行（行高不等），用的是 gpui 的 `list`（变高虚拟化）。

use std::sync::Arc;

use getcat_core::body::text::{TextDoc, clip_line};
use gpui_kit::base::SelectableText;
use gpui_kit::component::{
    ActiveTheme,
    clipboard::Clipboard,
    h_flex,
    input::SelectAll,
    scroll::{Scrollbar, ScrollbarAxis},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, InteractiveElement, IntoElement, ListHorizontalSizingBehavior, ListState, ParentElement,
    Role, SharedString, StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, div,
    list, px, uniform_list,
};

use crate::i18n::tr;
use crate::ui::format_bytes;
use crate::ui::selectable_lines::{LinesSelection, participant_layer};

/// 每行固定高度（uniform_list 要求等高；按第一项测量，所有行都用同一个 h()）。
pub const LINE_HEIGHT_PX: f32 = 20.;

fn digits(n: usize) -> usize {
    n.max(1).ilog10() as usize + 1
}

/// 行号栏宽度（px）：行文本从这个 x 偏移开始。选择引擎把鼠标 x 换算成列时也用它。
pub fn gutter_px(line_count: usize) -> f32 {
    16. + 8. * digits(line_count) as f32
}

/// 按行虚拟化渲染整份文档：行号 + 文本；超过 MAX_LINE_CHARS 的行截断并标注。
/// `doc` 被 move 进渲染闭包，闭包每帧只对可见区间切片并分配对应数量的 SharedString。
/// 行视图的按键上下文：main.rs 在它上面绑 ⌘A → `SelectAll`。
pub const LINES_KEY_CONTEXT: &str = "ResponseLines";

pub fn render_text_lines(
    id: &'static str,
    doc: Arc<TextDoc>,
    handle: &UniformListScrollHandle,
    selection: &LinesSelection,
    on_select_all: impl Fn(&SelectAll, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let line_count = doc.line_count();
    let longest = doc.longest_line();
    let gutter = px(gutter_px(line_count));
    let muted = cx.theme().muted_foreground;
    let warning = cx.theme().warning;
    let selection_color = cx.theme().selection;
    let lines = selection.clone_for_rows();

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
                    .child(lines.line_text(
                        ix,
                        SharedString::from(clipped.text.to_string()),
                        selection_color,
                    ))
                    .when(clipped.hidden_bytes > 0, |h| {
                        h.child(div().ml_2().text_color(warning).child(tr!(
                            "response.line_truncated",
                            size = format_bytes(clipped.hidden_bytes as u64)
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

    // uniform_list 只实现 InteractiveElement（没有 role）；外层组让屏幕阅读器知道这里是响应正文。
    // overflow_hidden 让参与者的内容遮罩就是这块视口：拖选到边缘时引擎合成的滚轮事件会落在列表上。
    div()
        .id("response-lines-region")
        .role(Role::Group)
        .aria_label(tr!("response.lines_aria"))
        .key_context(LINES_KEY_CONTEXT)
        .track_focus(&selection.focus)
        .on_action(on_select_all)
        .relative()
        .overflow_hidden()
        .size_full()
        .child(participant_layer(selection, handle))
        .child(list)
        .child(Scrollbar::new(handle).axis(ScrollbarAxis::Both))
}

/// SSE 事件列表：每行一个事件——序号 + event 名 + data（data 内的换行以 ⏎ 标注）。
/// 行宽按最长事件测量、可横向滚动（与 B/C 档行视图一致）；超过 MAX_LINE_CHARS
/// 的 data 截断并标注，完整原文在「原始」视图里。
pub fn render_sse_events(
    rows: Arc<[(SharedString, SharedString)]>,
    handle: &UniformListScrollHandle,
    cx: &App,
) -> impl IntoElement {
    let count = rows.len();
    let muted = cx.theme().muted_foreground;
    let accent = cx.theme().primary;
    let warning = cx.theme().warning;
    let gutter = px(16. + 8. * digits(count) as f32);
    // 最长行给 with_width_from_item 量宽用。event 名列定宽，行宽只由 data 决定；
    // 按字节数近似字符数（len() 是 O(1)，整趟 O(行数)），并对齐 clip_line 的截断上限。
    let longest = rows
        .iter()
        .enumerate()
        .max_by_key(|(_, (_, data))| data.len().min(getcat_core::body::text::MAX_LINE_CHARS * 4))
        .map(|(ix, _)| ix);
    let list = uniform_list("sse-events", count, move |range, _window, _cx| {
        range
            .map(|ix| {
                let (event, data) = &rows[ix];
                let clipped = clip_line(data);
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
                    .child(
                        div()
                            .w(px(200.))
                            .flex_none()
                            .pr_3()
                            .truncate()
                            .text_color(accent)
                            .child(if event.is_empty() {
                                // event 字段缺省时规范默认为 "message"
                                tr!("response.sse_default_event")
                            } else {
                                event.clone()
                            }),
                    )
                    .child(div().child(SharedString::from(clipped.text.replace('\n', "⏎"))))
                    .when(clipped.hidden_bytes > 0, |h| {
                        h.child(div().ml_2().text_color(warning).child(tr!(
                            "response.line_truncated",
                            size = format_bytes(clipped.hidden_bytes as u64)
                        )))
                    })
            })
            .collect::<Vec<_>>()
    })
    // 横向宽度按最长事件测量，并允许横向滚动
    .with_width_from_item(longest)
    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
    .track_scroll(handle)
    .size_full()
    .font_family(cx.theme().mono_font_family.clone())
    .text_size(cx.theme().mono_font_size);

    div()
        .id("sse-events-region")
        .role(Role::Group)
        .aria_label(tr!("response.sse_events_aria"))
        .relative()
        .size_full()
        .child(list)
        .child(Scrollbar::new(handle).axis(ScrollbarAxis::Both))
}

/// Headers 名称列的宽度与列间距（px）。值列的左缩进 = 两者之和。
const HEADER_NAME_WIDTH: f32 = 256.;
const HEADER_COL_GAP: f32 = 12.;
/// 行内悬浮复制按钮靠 group hover 显示；gpui 的 group 边界按名字入栈，同名跨行也只匹配当前行
const HEADER_ROW_GROUP: &str = "response-header-row";

/// 响应头列表：每行一个 Header，名称列定宽，值列自然换行。
///
/// 值要完整可读（不截断、不横滚），行高因此不等——不能用 uniform_list，
/// 用 gpui 的 `list`（变高虚拟化，按需测量，仍是每帧 O(可见行)）。
/// 超长无空格的值（base64 之类）gpui 的折行会在字符边界硬断，不会撑破布局。
///
/// 行内**必须是 block 布局，不能用 flex**：`list` 测量 item 走
/// `layout_as_root(Definite(宽), MinContent)`，这种探测下 taffy 不向 flex
/// 子项传递已知宽度，文本会按单行测量、折行完全失效（行高全部塌成一行）；
/// block 布局才会把宽度传导给文本。名称列因此用绝对定位排在左侧。
pub fn render_header_rows(
    rows: Arc<[(SharedString, SharedString)]>,
    state: &ListState,
    cx: &App,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let list = list(state.clone(), move |ix, _window, _cx| {
        let Some((name, value)) = rows.get(ix) else {
            // reset 与新 rows 之间隔了一帧时的防御：宁可空一行也不 panic
            return div().into_any_element();
        };
        // 名称与值都是窗口级选择的参与者（gpui-base 的 SelectableText）：能拖选、⌘C 复制；
        // document_order 让跨行拖选按 名称 → 值 → 下一行 的阅读顺序拼接
        div()
            .relative()
            .group(HEADER_ROW_GROUP)
            .px_3()
            .py_1()
            .text_sm()
            // 名称列：绝对定位、定宽截断；值折成多行时贴着第一行
            .child(
                div()
                    .absolute()
                    .left_3()
                    .top_1()
                    .w(px(HEADER_NAME_WIDTH))
                    .text_color(muted)
                    .truncate()
                    .child(
                        SelectableText::new(("header-name", ix), name.clone())
                            .document_order(2 * ix as u64),
                    ),
            )
            // 值列：block 流内用左 margin 让出名称列，右侧给悬浮复制按钮留位；min_h 兜住空值行的高度
            .child(
                div()
                    .ml(px(HEADER_NAME_WIDTH + HEADER_COL_GAP))
                    .pr_8()
                    .min_h_5()
                    .child(
                        SelectableText::new(("header-value", ix), value.clone())
                            .document_order(2 * ix as u64 + 1),
                    ),
            )
            // 悬浮才出现的单行复制：`Name: value`
            .child(
                div()
                    .absolute()
                    .right_1()
                    .top_0()
                    .invisible()
                    .group_hover(HEADER_ROW_GROUP, |s| s.visible())
                    .child(
                        Clipboard::new(("copy-header", ix))
                            .value(format!("{name}: {value}"))
                            .tooltip(tr!("response.copy_header")),
                    ),
            )
            .into_any_element()
    })
    .size_full()
    .font_family(cx.theme().mono_font_family.clone());

    div()
        .id("response-headers-region")
        .role(Role::Group)
        .aria_label(tr!("response.headers_aria"))
        .relative()
        .size_full()
        .child(list)
        .child(Scrollbar::vertical(state))
}
