//! 「生成代码」抽屉：把当前 Tab 的请求转成 curl / Python 示例，可切目标、可一键复制。
//!
//! 抽屉用 gpui-component 的 `Sheet`（`window.open_sheet` 默认就是从右侧滑入），
//! 它自带标题栏、关闭按钮、Esc 与点遮罩关闭、边缘拖宽，这里只负责正文。
//!
//! # 为什么正文是独立实体而不是 `Workspace` 上的一个 render 方法
//!
//! `Root::render_sheet_layer` 是在 `Workspace::render` **内部**调用的，而 `Sheet` 的 builder
//! 是 `Fn`、每帧都会被执行一次。若 builder 里写 `workspace.update(...)`，就等于在 `Workspace`
//! 的 render 借用还没释放时再借一次，gpui 会直接 panic：
//!
//! ```text
//! cannot update Workspace while it is already being updated
//! ```
//!
//! 表现是「点一下按钮就闪退」——因为点击后的第一次重绘必然踩中。
//! 所以正文做成独立实体，builder 里只 `.child(entity.clone())`：gpui 渲染它时单独借
//! `CodeSheet`，从不碰 `Workspace`。主工作区把 `Entity<RequestTab>` 当 child 传是同一个套路。
//!
//! 同理，代码只在「装载草稿」与「切换目标」这两个离散事件里生成，绝不在渲染期算。

use gpui::*;
use gpui_component::{
    ActiveTheme, Sizable,
    clipboard::Clipboard,
    h_flex,
    input::{Editor, EditorState},
    tab::{Tab, TabBar},
    v_flex,
};

use getcat_core::codegen::{self, CodeTarget};
use getcat_core::http::RequestError;
use getcat_core::model::RequestDraft;

use crate::i18n::tr;
use crate::ui::text::prepare_error_line;

/// 生成代码用到的语法高亮语言，每种一个常驻编辑器。
const CODE_LANGUAGES: [&str; 2] = ["bash", "python"];

/// 抽屉的起始宽度：`Sheet` 默认的 350 px 装不下一行 curl 命令，用户仍可拖窄 / 拖宽。
pub const CODE_SHEET_WIDTH: f32 = 560.;

/// 打开抽屉那一刻的请求快照。抽屉是覆盖式的，打开期间请求改不了，
/// 所以切换目标时拿它重新生成就够，不必反过来持有 `Workspace`。
struct Source {
    draft: RequestDraft,
    /// 被用户关掉的默认请求头（全局设置，取自 `settings.json`）。
    disabled_default_headers: Vec<String>,
}

pub struct CodeSheet {
    target: CodeTarget,
    editors: Vec<(&'static str, Entity<EditorState>)>,
    /// 当前代码原文，复制按钮直接取它（省得再从编辑器读一遍）。
    text: SharedString,
    /// 生成失败的原因（URL 为空 / header 非法 / form-data 未选文件），替代代码显示。
    error: Option<RequestError>,
    source: Option<Source>,
}

impl CodeSheet {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            target: CodeTarget::default(),
            editors: CODE_LANGUAGES
                .iter()
                .map(|language| {
                    (
                        *language,
                        cx.new(|cx| {
                            EditorState::new(window, cx)
                                .language(*language)
                                .line_number(true)
                                .soft_wrap(false)
                        }),
                    )
                })
                .collect(),
            text: SharedString::default(),
            error: None,
            source: None,
        }
    }

    /// 装载一份请求快照并立刻生成代码。每次打开抽屉时由 `Workspace` 调用。
    pub fn load(
        &mut self,
        draft: RequestDraft,
        disabled_default_headers: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.source = Some(Source {
            draft,
            disabled_default_headers,
        });
        self.regenerate(window, cx);
    }

    #[cfg(test)]
    pub fn text(&self) -> &SharedString {
        &self.text
    }

    #[cfg(test)]
    pub fn error(&self) -> Option<&RequestError> {
        self.error.as_ref()
    }

    #[cfg(test)]
    pub fn target(&self) -> CodeTarget {
        self.target
    }

    /// 测试用：分段控件的点击回调走的是 `cx.listener`，测试里没法直接触发。
    #[cfg(test)]
    pub fn set_target_for_test(
        &mut self,
        target: CodeTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_target(target, window, cx);
    }

    fn set_target(&mut self, target: CodeTarget, window: &mut Window, cx: &mut Context<Self>) {
        if self.target == target {
            return;
        }
        self.target = target;
        self.regenerate(window, cx);
    }

    pub(crate) fn regenerate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.as_ref() else {
            return;
        };
        match codegen::generate(&source.draft, &source.disabled_default_headers, self.target) {
            Ok(code) => {
                self.error = None;
                self.text = code.clone().into();
                let editor = self.editor().clone();
                editor.update(cx, |editor, cx| editor.set_value(code, window, cx));
            }
            Err(error) => {
                self.error = Some(error);
                self.text = SharedString::default();
            }
        }
        cx.notify();
    }

    /// 当前目标对应的编辑器。`CODE_LANGUAGES` 覆盖了所有 `CodeTarget`，
    /// 找不到只可能是漏加语言，退回第一个而不是 panic。
    fn editor(&self) -> &Entity<EditorState> {
        let language = self.target.editor_language();
        self.editors
            .iter()
            .find(|(candidate, _)| *candidate == language)
            .map(|(_, editor)| editor)
            .unwrap_or(&self.editors[0].1)
    }
}

impl Render for CodeSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = self.target;
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .child(
                        TabBar::new("code-targets")
                            .underline()
                            .small()
                            .flex_1()
                            .min_w_0()
                            .selected_index(target.index())
                            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                                this.set_target(CodeTarget::from_index(*ix), window, cx)
                            }))
                            .children(
                                CodeTarget::ALL
                                    .iter()
                                    .map(|target| Tab::new().label(target.label())),
                            ),
                    )
                    .child(
                        Clipboard::new("copy-code")
                            .value(self.text.clone())
                            .tooltip(tr!("tools.code.copy")),
                    ),
            )
            .child(match self.error.as_ref() {
                Some(error) => v_flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .px_4()
                    .text_sm()
                    .text_center()
                    .text_color(cx.theme().danger)
                    .child(prepare_error_line(error))
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .min_h_0()
                    .child(
                        Editor::new(self.editor())
                            .aria_label(tr!("tools.code.editor_aria"))
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(cx.theme().mono_font_size)
                            .readonly(true)
                            .size_full(),
                    )
                    .into_any_element(),
            })
    }
}
