//! 请求面板：Params / Headers / Body 三段。

use getcat_core::model::RawFormat;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Editor,
    menu::{DropdownMenu as _, PopupMenuItem},
    tab::{Tab, TabBar},
    v_flex,
};

use crate::state::request_tab::{BodyMode, RequestSection, RequestTab};
use crate::ui::format_bytes;

fn section_label(text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

impl RequestTab {
    pub fn render_request_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let params_count = self.params.read(cx).count(cx) + self.path_params.read(cx).count(cx);
        let headers_count = self.headers.read(cx).count(cx);
        let section = self.request_section;
        let label = |name: &str, n: usize| -> SharedString {
            if n > 0 {
                format!("{name} ({n})").into()
            } else {
                name.to_string().into()
            }
        };

        v_flex()
            .size_full()
            .min_h_0()
            .child(
                TabBar::new("request-sections")
                    .underline()
                    .small()
                    .px_3()
                    .selected_index(section.index())
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        this.request_section = RequestSection::from_index(*ix);
                        cx.notify();
                    }))
                    .child(Tab::new().label(label("Params", params_count)))
                    .child(Tab::new().label(label("Headers", headers_count)))
                    .child(
                        Tab::new()
                            .label(label("Body", usize::from(self.body_mode != BodyMode::None))),
                    ),
            )
            .child(
                div()
                    .id("request-section")
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .py_3()
                    .when(section != RequestSection::Body, |d| d.overflow_y_scroll())
                    .child(match section {
                        RequestSection::Params => v_flex()
                            .gap_3()
                            .when(self.has_path_params(cx), |v| {
                                v.child(section_label("Path 参数", cx))
                                    .child(self.path_params.clone())
                            })
                            .child(section_label("Query 参数", cx))
                            .child(self.params.clone())
                            .into_any_element(),
                        RequestSection::Headers => self.headers.clone().into_any_element(),
                        RequestSection::Body => self.render_body_section(cx),
                    }),
            )
    }

    pub fn render_body_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.body_mode;
        let weak = cx.entity().downgrade();
        let current_format = self.raw_format;
        v_flex()
            .size_full()
            .gap_3()
            // 一行工具条：左边是 Body 类型分段控件，右边（raw 时）是格式下拉。
            // flex_wrap 让格式下拉在请求区很窄时自动换到下一行，而不是把分段控件裁掉。
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        TabBar::new("body-mode")
                            .segmented()
                            .small()
                            .selected_index(mode.index())
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                this.body_mode = BodyMode::from_index(*ix);
                                this.refresh_body_hint(cx);
                                this.mark_dirty(cx);
                            }))
                            .child("none")
                            .child("form-data")
                            // 标签缩短；发出的 Content-Type 仍是 application/x-www-form-urlencoded
                            .child("urlencoded")
                            .child("raw")
                            .child("binary"),
                    )
                    .when(mode == BodyMode::Raw, |h| {
                        h.child(
                            Button::new("raw-format")
                                .outline()
                                .small()
                                .label(current_format.label())
                                .tooltip("raw 内容的格式（决定高亮与 Content-Type）")
                                .dropdown_caret(true)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for format in RawFormat::ALL {
                                        let weak = weak.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(format.label())
                                                .checked(format == current_format)
                                                .on_click(move |_, _, cx| {
                                                    if let Some(tab) = weak.upgrade() {
                                                        tab.update(cx, |this, cx| {
                                                            this.raw_format = format;
                                                            this.refresh_body_hint(cx);
                                                            this.mark_dirty(cx);
                                                        });
                                                    }
                                                }),
                                        );
                                    }
                                    menu
                                }),
                        )
                    }),
            )
            .child(match mode {
                BodyMode::None => div()
                    .py_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("此请求没有 Body")
                    .into_any_element(),
                BodyMode::Raw => v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .overflow_hidden()
                            .child(
                                Editor::new(self.editor_for(self.raw_format))
                                    .aria_label("请求 Body 编辑器")
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_size(cx.theme().mono_font_size)
                                    .size_full(),
                            ),
                    )
                    .when_some(self.body_hint.clone(), |v, hint| {
                        v.child(div().text_xs().text_color(cx.theme().warning).child(hint))
                    })
                    .into_any_element(),
                BodyMode::FormData => v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .child(
                        div()
                            .id("form-data-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(self.form_data.clone()),
                    )
                    .when_some(self.body_hint.clone(), |v, hint| {
                        v.child(div().text_xs().text_color(cx.theme().warning).child(hint))
                    })
                    .into_any_element(),
                BodyMode::FormUrlEncoded => div()
                    .id("form-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.form.clone())
                    .into_any_element(),
                BodyMode::Binary => self.render_file_body(cx),
            })
            .into_any_element()
    }

    fn render_file_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("choose-file")
                            .outline()
                            .small()
                            .label("选择文件…")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.choose_file(window, cx)),
                            ),
                    )
                    .when(self.file_path.is_some(), |h| {
                        h.child(
                            Button::new("clear-file")
                                .ghost()
                                .small()
                                .label("清除")
                                .on_click(cx.listener(|this, _, _, cx| this.clear_file(cx))),
                        )
                    }),
            )
            .child(match &self.file_path {
                Some(path) => h_flex()
                    .gap_3()
                    .text_sm()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(path.display().to_string()),
                    )
                    .when_some(self.file_size, |h, size| {
                        h.child(
                            div()
                                .flex_none()
                                .text_color(muted)
                                .child(format_bytes(size)),
                        )
                    })
                    .into_any_element(),
                None => div()
                    .text_sm()
                    .text_color(muted)
                    .child("尚未选择文件；发送时将以流式方式上传，内容不进入内存")
                    .into_any_element(),
            })
            .into_any_element()
    }
}
