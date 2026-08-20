//! 请求面板：Params / Headers / Body 三段。

use getcat_core::model::RawFormat;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, Sizable, h_flex,
    input::Editor,
    tab::{Tab, TabBar},
    v_flex,
};

use crate::state::request_tab::{BodyMode, RequestSection, RequestTab};

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
                    .p_3()
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
        let raw_ix = RawFormat::ALL
            .iter()
            .position(|f| *f == self.raw_format)
            .unwrap_or(0);
        v_flex()
            .size_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        TabBar::new("body-mode")
                            .segmented()
                            .xsmall()
                            .selected_index(mode.index())
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                this.body_mode = BodyMode::from_index(*ix);
                                cx.notify();
                            }))
                            .child("none")
                            .child("raw")
                            .child("form-urlencoded"),
                    )
                    .when(mode == BodyMode::Raw, |h| {
                        h.child(
                            TabBar::new("raw-format")
                                .pill()
                                .xsmall()
                                .selected_index(raw_ix)
                                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                    this.raw_format =
                                        RawFormat::ALL.get(*ix).copied().unwrap_or(RawFormat::Json);
                                    cx.notify();
                                }))
                                .children(
                                    RawFormat::ALL.iter().map(|f| Tab::new().label(f.label())),
                                ),
                        )
                    }),
            )
            .child(match mode {
                BodyMode::None => div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("此请求没有 Body")
                    .into_any_element(),
                BodyMode::Raw => div()
                    .flex_1()
                    .min_h_0()
                    .child(
                        Editor::new(self.editor_for(self.raw_format))
                            .aria_label("请求 Body 编辑器")
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(cx.theme().mono_font_size)
                            .size_full(),
                    )
                    .into_any_element(),
                BodyMode::Form => div()
                    .id("form-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.form.clone())
                    .into_any_element(),
            })
            .into_any_element()
    }
}
