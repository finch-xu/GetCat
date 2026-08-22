//! 响应面板：状态行、Pretty/Raw 与 Body/Headers 切换、按档位分派的 Body 视图、虚拟化 Headers 列表。

use getcat_core::body::tier::ViewTier;
use getcat_core::http::{BodyStore, RequestError};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, IconName, Sizable,
    alert::Alert,
    button::{Button, ButtonVariants},
    description_list::DescriptionList,
    h_flex,
    input::Editor,
    kbd::Kbd,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};

use crate::i18n::tr;
use crate::state::request_tab::{RequestTab, ResponseSection};
use crate::state::response::{ResponseState, ResponseView};
use crate::ui::body_view::{render_header_rows, render_text_lines};
use crate::ui::text::{content_kind_label, error_detail, error_kind, tier_notice};
use crate::ui::{format_bytes, format_duration, status_color};
use crate::{FindInResponse, SendRequest};

fn empty_state(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    empty_state_frame(cx).child(text.into()).into_any_element()
}

fn empty_state_frame(cx: &App) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
}

/// 档位提示：官方 `Alert` 的 banner 形态，底色 / 边框 / 图标全部来自主题。
fn notice_bar(text: impl Into<SharedString>) -> AnyElement {
    Alert::warning("tier-notice", text.into())
        .banner()
        .xsmall()
        .into_any_element()
}

impl RequestTab {
    pub fn render_response_pane(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
                    .h_10()
                    .px_3()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().min_w_0().child(self.render_status_line(cx)))
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_3()
                            .items_center()
                            .when_some(self.notice.clone(), |h, notice| {
                                h.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .max_w_96()
                                        .truncate()
                                        .child(notice.text()),
                                )
                            })
                            .when(is_done, |h| {
                                h.child(
                                    Button::new("find-in-response")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Search)
                                        .tooltip_with_action(
                                            tr!("response.find"),
                                            &FindInResponse,
                                            None,
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.find_in_response(window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("save-body")
                                        .ghost()
                                        .xsmall()
                                        .label(tr!("response.save_to_file"))
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
                    .child(self.render_response_body(section, window, cx)),
            )
    }

    fn render_response_body(
        &self,
        section: ResponseSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &self.response {
            ResponseState::Idle => {
                let send_key = Kbd::binding_for_action(&SendRequest, None, window);
                empty_state_frame(cx)
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(tr!("response.idle_prefix"))
                            .children(send_key),
                    )
                    .into_any_element()
            }
            ResponseState::InFlight {
                received, total, ..
            } => {
                let text = match total {
                    Some(t) => tr!(
                        "response.in_flight",
                        received = format_bytes(*received),
                        total = format_bytes(*t)
                    ),
                    None => tr!(
                        "response.in_flight_unknown",
                        received = format_bytes(*received)
                    ),
                };
                empty_state(text, cx)
            }
            ResponseState::Failed {
                error: RequestError::Cancelled,
            } => empty_state(tr!("response.cancelled"), cx),
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
                        .child(error_kind(error)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(error_detail(error)),
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
                .child(empty_state(tr!("response.binary_no_preview"), cx))
                .into_any_element();
        };
        let lines = if doc.doc.line_count() == 0 {
            empty_state(tr!("response.empty_body"), cx)
        } else {
            match doc.tier {
                ViewTier::Editor => {
                    Editor::new(self.response_editor_for(view.kind.editor_language()))
                        .aria_label(tr!("response.body_aria"))
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
            .when_some(tier_notice(doc.tier), |v, text| v.child(notice_bar(text)))
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
        // 标签 / 值成对的元数据用官方 DescriptionList：标签列宽、间距与字色都由组件定
        let list = DescriptionList::new()
            .columns(1)
            .bordered(false)
            .small()
            .label_width(rems(4.5))
            .item(
                tr!("response.summary.size"),
                format_bytes(view.meta.body_len),
                1,
            )
            .item(
                tr!("response.summary.type"),
                view.meta
                    .content_type
                    .clone()
                    .map(SharedString::from)
                    .unwrap_or_else(|| content_kind_label(view.kind)),
                1,
            )
            .item(
                tr!("response.summary.duration"),
                format_duration(view.meta.duration),
                1,
            )
            .when_some(body.path(), |list, path| {
                list.item(
                    tr!("response.summary.temp_file"),
                    path.display().to_string(),
                    1,
                )
            });
        v_flex()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(list)
            .when(body.path().is_some(), |v| {
                v.child(
                    h_flex().child(
                        Button::new("open-with-system")
                            .outline()
                            .xsmall()
                            .label(tr!("response.open_with_system"))
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
                .child(tr!("response.status_idle"))
                .into_any_element(),
            ResponseState::InFlight {
                started, received, ..
            } => h_flex()
                .gap_3()
                .text_sm()
                .text_color(muted)
                .child(tr!(
                    "response.status_in_flight",
                    elapsed = format_duration(started.elapsed())
                ))
                .child(format_bytes(*received))
                .into_any_element(),
            ResponseState::Failed { error } => {
                let cancelled = matches!(error, RequestError::Cancelled);
                h_flex()
                    .text_sm()
                    .text_color(if cancelled { muted } else { cx.theme().danger })
                    .child(if cancelled {
                        tr!("response.status_cancelled")
                    } else {
                        tr!("response.status_failed")
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
                        // 状态码用官方 Tag 的描边形态：圆角与内边距跟主题走，不再手调透明度
                        Tag::custom(color, color, color)
                            .outline()
                            .text_sm()
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
                    .child(div().text_color(muted).child(content_kind_label(view.kind)))
                    .into_any_element()
            }
        }
    }
}
