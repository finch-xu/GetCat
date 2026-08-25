//! 「导入 cURL」抽屉：粘一条 curl 命令，实时看解析结果，确认后开成新 Tab。
//!
//! 与 [`crate::ui::code_sheet`] 左右对称——那边是「这条请求变成命令」，这边是
//! 「命令变回这条请求」。**同一条模块约束在这里同样成立**：正文必须是独立实体，
//! 因为 `Sheet` 的 builder 是 `Fn`、每帧在 `Workspace::render` 内部执行，builder 里
//! 碰一下 `Workspace` 就会二次借用而 panic（详见 `code_sheet` 顶部那段注释）。
//!
//! 先看结果再导入，而不是粘完直接改掉当前 Tab：curl 命令常带一堆浏览器塞的头，
//! 用户需要先确认「搬过来的是不是我要的」，以及「哪些没搬过来」。

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Editor, EditorState, InputEvent},
    v_flex,
};

use getcat_core::import::curl::{self, CurlParseError, CurlWarning};
use getcat_core::model::{BodyKind, RequestDraft};

use crate::i18n::tr;

/// 抽屉起始宽度。与「生成代码」同宽：一条 curl 命令折行后大致需要这么多。
pub const CURL_SHEET_WIDTH: f32 = 560.;

/// 解析出来的东西，渲染时不再重算——解析只发生在输入变化那一刻。
struct Parsed {
    draft: RequestDraft,
    warnings: Vec<CurlWarning>,
}

pub struct CurlSheet {
    input: Entity<EditorState>,
    parsed: Option<Parsed>,
    error: Option<CurlParseError>,
    _subs: Vec<Subscription>,
}

impl CurlSheet {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 用代码编辑器而不是纯文本框：curl 命令本来就是 shell，顺手拿到语法高亮，
        // 长命令折行也比 Textarea 好读。
        let input = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("bash")
                .soft_wrap(true)
                .placeholder(tr!("tools.import_curl.placeholder"))
        });
        let subs = vec![
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.reparse(cx);
                }
            }),
        ];
        Self {
            input,
            parsed: None,
            error: None,
            _subs: subs,
        }
    }

    /// 每次输入变化重新解析。命令再长也就几 KB，解析是纯字符串处理，
    /// 直接在主线程做比排一次后台任务便宜。
    fn reparse(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            self.parsed = None;
            self.error = None;
        } else {
            match curl::parse(&text) {
                Ok(result) => {
                    self.parsed = Some(Parsed {
                        draft: result.draft,
                        warnings: result.warnings,
                    });
                    self.error = None;
                }
                Err(e) => {
                    self.parsed = None;
                    self.error = Some(e);
                }
            }
        }
        cx.notify();
    }

    /// 解析成功时的草稿；`Workspace` 按下「导入」时取它。
    pub fn draft(&self) -> Option<&RequestDraft> {
        self.parsed.as_ref().map(|p| &p.draft)
    }

    /// 导入后清空，免得下次打开还留着上一条命令。
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.parsed = None;
        self.error = None;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// 测试用：解析回调走的是输入框事件订阅，测试里直接调这个更省事。
    #[cfg(test)]
    pub fn set_text_for_test(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.reparse(cx);
    }

    #[cfg(test)]
    pub fn error(&self) -> Option<&CurlParseError> {
        self.error.as_ref()
    }

    #[cfg(test)]
    pub fn warnings(&self) -> &[CurlWarning] {
        self.parsed
            .as_ref()
            .map(|p| p.warnings.as_slice())
            .unwrap_or(&[])
    }
}

impl Render for CurlSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .child(
                div().h_48().flex_none().child(
                    Editor::new(&self.input)
                        .aria_label(tr!("tools.import_curl.input_aria"))
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(cx.theme().mono_font_size)
                        .size_full(),
                ),
            )
            .child(div().flex_1().min_h_0().child(self.render_result(cx)))
    }
}

impl CurlSheet {
    fn render_result(&self, cx: &Context<Self>) -> AnyElement {
        if let Some(error) = &self.error {
            return div()
                .px_1()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(SharedString::from(error.to_string()))
                .into_any_element();
        }
        let Some(parsed) = &self.parsed else {
            return div()
                .px_1()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(tr!("tools.import_curl.empty_hint"))
                .into_any_element();
        };

        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(
                div()
                    .px_1()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .truncate()
                    .child(SharedString::from(parsed.draft.url.clone())),
            )
            .child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr!(
                        "tools.import_curl.summary",
                        method = parsed.draft.method.as_str(),
                        headers = parsed.draft.headers.len(),
                        body = body_label(&parsed.draft.body)
                    )),
            )
            .when(!parsed.warnings.is_empty(), |v| {
                // 没搬过来的东西必须显式列出来：用户拿这条命令是要复现某个请求的，
                // 悄悄丢掉一个 `-x proxy` 会让他对着「为什么结果不一样」白查半天。
                v.child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .mt_1()
                        .gap_1()
                        .child(
                            div()
                                .px_1()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(cx.theme().warning)
                                .child(tr!(
                                    "tools.import_curl.warnings",
                                    count = parsed.warnings.len()
                                )),
                        )
                        .child(
                            v_flex()
                                .id("curl-warnings")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .gap_0p5()
                                .children(parsed.warnings.iter().map(|w| {
                                    div()
                                        .px_1()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(warning_label(w))
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}

/// 请求体的一句话描述，给摘要行用。
fn body_label(body: &BodyKind) -> SharedString {
    match body {
        BodyKind::None => tr!("tools.import_curl.body_none"),
        BodyKind::Raw { format, .. } => {
            tr!("tools.import_curl.body_raw", format = format.label())
        }
        BodyKind::FormData { fields } => {
            tr!("tools.import_curl.body_form_data", count = fields.len())
        }
        BodyKind::FormUrlEncoded { fields } => {
            tr!("tools.import_curl.body_urlencoded", count = fields.len())
        }
        BodyKind::Binary { .. } => tr!("tools.import_curl.body_binary"),
    }
}

fn warning_label(warning: &CurlWarning) -> SharedString {
    match warning {
        CurlWarning::RuntimeOption(flag) => {
            tr!("tools.import_curl.warn_runtime", flag = flag)
        }
        CurlWarning::Unknown(flag) => tr!("tools.import_curl.warn_unknown", flag = flag),
        CurlWarning::Unsupported(flag) => {
            tr!("tools.import_curl.warn_unsupported", flag = flag)
        }
    }
}

/// 抽屉底部的「导入为新 Tab」按钮。放在 `Sheet` 的 footer 而不是正文里，
/// 因为它要 `update` 宿主 `Workspace`——而 footer 的回调不在 builder 的 `Fn` 里跑。
pub fn import_button(
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex().w_full().justify_end().child(
        Button::new("curl-import")
            .primary()
            .small()
            .label(tr!("tools.import_curl.import"))
            .disabled(!enabled)
            .on_click(on_click),
    )
}
