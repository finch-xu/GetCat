//! 响应面板：状态行、Pretty/Raw 与 Body/Headers 切换、按档位分派的 Body 视图、虚拟化 Headers 列表。

use getcat_core::body::tier::ViewTier;
use getcat_core::http::{BodyStore, RequestError};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Editor,
    tab::{Tab, TabBar},
    v_flex,
};

use crate::state::request_tab::{RequestTab, ResponseSection};
use crate::state::response::{ResponseState, ResponseView};
use crate::ui::body_view::{render_header_rows, render_text_lines};
use crate::ui::{format_bytes, format_duration, status_color};

fn empty_state(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

fn notice_bar(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .px_3()
        .py_1()
        .text_xs()
        .bg(cx.theme().warning.opacity(0.15))
        .text_color(cx.theme().warning)
        .child(text.into())
        .into_any_element()
}

impl RequestTab {
    pub fn render_response_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (is_done, has_pretty, headers_count) = match &self.response {
            ResponseState::Done { view, .. } => (true, view.has_pretty(), view.header_rows.len()),
            _ => (false, false, 0),
        };
        let section = self.response_section;

        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .h(px(40.))
                    .px_3()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.render_status_line(cx))
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .when_some(self.notice.clone(), |h, notice| {
                                h.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .max_w(px(360.))
                                        .truncate()
                                        .child(notice),
                                )
                            })
                            .when(is_done, |h| {
                                h.child(
                                    Button::new("find-in-response")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Search)
                                        .tooltip("响应内搜索（⌘F / Ctrl F）")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.find_in_response(window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("save-body")
                                        .ghost()
                                        .xsmall()
                                        .label("保存到文件")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.save_body(window, cx)
                                        })),
                                )
                            })
                            // 只有存在美化文本时才提供 Pretty/Raw 切换
                            .when(has_pretty, |h| {
                                h.child(
                                    TabBar::new("pretty-raw")
                                        .segmented()
                                        .xsmall()
                                        .selected_index(if self.pretty { 0 } else { 1 })
                                        .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                            this.set_pretty(*ix == 0, window, cx)
                                        }))
                                        .child("Pretty")
                                        .child("Raw"),
                                )
                            })
                            .child(
                                TabBar::new("response-sections")
                                    .underline()
                                    .xsmall()
                                    .selected_index(if section == ResponseSection::Body {
                                        0
                                    } else {
                                        1
                                    })
                                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                        this.response_section = if *ix == 0 {
                                            ResponseSection::Body
                                        } else {
                                            ResponseSection::Headers
                                        };
                                        cx.notify();
                                    }))
                                    .child("Body")
                                    .child(Tab::new().label(if headers_count > 0 {
                                        format!("Headers ({headers_count})")
                                    } else {
                                        "Headers".to_string()
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_response_body(section, cx)),
            )
    }

    fn render_response_body(&self, section: ResponseSection, cx: &mut Context<Self>) -> AnyElement {
        match &self.response {
            ResponseState::Idle => empty_state("输入 URL 后点击发送，或按 ⌘⏎ / Ctrl+Enter", cx),
            ResponseState::InFlight {
                received, total, ..
            } => {
                let text = match total {
                    Some(t) => format!(
                        "发送中… 已接收 {} / {}",
                        format_bytes(*received),
                        format_bytes(*t)
                    ),
                    None => format!("发送中… 已接收 {}", format_bytes(*received)),
                };
                empty_state(text, cx)
            }
            ResponseState::Failed {
                error: RequestError::Cancelled,
            } => empty_state("请求已取消", cx),
            ResponseState::Failed { error } => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().danger)
                        .child(error.kind_label()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(error.to_string()),
                )
                .into_any_element(),
            ResponseState::Done { body, view } => match section {
                ResponseSection::Body => self.render_body_view(body, view, cx),
                ResponseSection::Headers => {
                    render_header_rows(view.header_rows.clone(), &self.headers_scroll, cx)
                        .into_any_element()
                }
            },
        }
    }

    /// 按档位分派：A 档只读 Editor；B 档 uniform_list 行视图；C 档摘要 + 前 1 MiB 行视图；二进制只有摘要。
    fn render_body_view(
        &self,
        body: &BodyStore,
        view: &ResponseView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(doc) = view.doc(self.pretty) else {
            return v_flex()
                .size_full()
                .child(self.render_preview_summary(body, view, cx))
                .child(empty_state(
                    "二进制内容不提供文本预览，可用右上角「保存到文件」导出",
                    cx,
                ))
                .into_any_element();
        };
        let lines = if doc.doc.line_count() == 0 {
            empty_state("响应体为空", cx)
        } else {
            match doc.tier {
                ViewTier::Editor => {
                    Editor::new(self.response_editor_for(view.kind.editor_language()))
                        .aria_label("响应 Body")
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(cx.theme().mono_font_size)
                        .readonly(true)
                        .size_full()
                        .into_any_element()
                }
                ViewTier::Virtual | ViewTier::Preview => {
                    render_text_lines("response-lines", doc.doc.clone(), &self.body_scroll, cx)
                        .into_any_element()
                }
            }
        };
        v_flex()
            .size_full()
            .when_some(doc.tier.notice(), |v, text| v.child(notice_bar(text, cx)))
            .when(view.is_preview(), |v| {
                v.child(self.render_preview_summary(body, view, cx))
            })
            .child(div().flex_1().min_h_0().child(lines))
            .into_any_element()
    }

    /// C 档 / 二进制的摘要块：大小、类型、耗时、临时文件路径与"用系统程序打开"。
    fn render_preview_summary(
        &self,
        body: &BodyStore,
        view: &ResponseView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let row = |label: &'static str, value: String| {
            h_flex()
                .gap_2()
                .text_sm()
                .child(div().w(px(72.)).flex_none().text_color(muted).child(label))
                .child(div().flex_1().min_w_0().truncate().child(value))
        };
        v_flex()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(row("大小", format_bytes(view.meta.body_len)))
            .child(row(
                "类型",
                view.meta
                    .content_type
                    .clone()
                    .unwrap_or_else(|| view.kind.label().to_string()),
            ))
            .child(row("耗时", format_duration(view.meta.duration)))
            .when_some(body.path(), |v, path| {
                v.child(row("临时文件", path.display().to_string())).child(
                    h_flex().pt_1().child(
                        Button::new("open-with-system")
                            .outline()
                            .xsmall()
                            .label("用系统程序打开")
                            .on_click(cx.listener(|this, _, _, cx| this.open_body_with_system(cx))),
                    ),
                )
            })
            .into_any_element()
    }

    pub fn render_status_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        match &self.response {
            ResponseState::Idle => h_flex()
                .text_sm()
                .text_color(muted)
                .child("尚未发送")
                .into_any_element(),
            ResponseState::InFlight {
                started, received, ..
            } => h_flex()
                .gap_3()
                .text_sm()
                .text_color(muted)
                .child(format!("发送中… {}", format_duration(started.elapsed())))
                .child(format_bytes(*received))
                .into_any_element(),
            ResponseState::Failed { error } => {
                let cancelled = matches!(error, RequestError::Cancelled);
                h_flex()
                    .text_sm()
                    .text_color(if cancelled { muted } else { cx.theme().danger })
                    .child(if cancelled {
                        "已取消"
                    } else {
                        "请求失败"
                    })
                    .into_any_element()
            }
            ResponseState::Done { view, .. } => {
                let color = status_color(view.meta.status, cx);
                h_flex()
                    .gap_3()
                    .items_center()
                    .text_sm()
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .bg(color.opacity(0.15))
                            .text_color(color)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{} {}", view.meta.status, view.meta.status_text)),
                    )
                    .child(
                        div()
                            .text_color(muted)
                            .child(format_duration(view.meta.duration)),
                    )
                    .child(
                        div()
                            .text_color(muted)
                            .child(format_bytes(view.meta.body_len)),
                    )
                    .child(div().text_color(muted).child(view.kind.label()))
                    .into_any_element()
            }
        }
    }
}
