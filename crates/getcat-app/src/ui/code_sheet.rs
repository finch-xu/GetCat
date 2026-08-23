//! 「生成代码」抽屉：把当前 Tab 的请求转成 curl / Python 示例，可切目标、可一键复制。
//!
//! 抽屉用 gpui-component 的 [`Sheet`]（`window.open_sheet` 默认就是从右侧滑入），
//! 它自带标题栏、关闭按钮、Esc 与点遮罩关闭、边缘拖宽，这里只负责正文。
//!
//! 代码在「打开抽屉」与「切换目标」这两个离散事件里生成一次，不在渲染期算：
//! `Sheet` 的 builder 是 `Fn`，每帧都会被调用，把 `set_value` 放进去会变成每帧写一次编辑器。
//! 抽屉是覆盖式的，打开期间请求本来也改不了，生成一次正合适。

use gpui::*;
use gpui_component::WindowExt as _;
use gpui_component::{
    ActiveTheme, Sizable,
    clipboard::Clipboard,
    h_flex,
    input::Editor,
    tab::{Tab, TabBar},
    v_flex,
};

use getcat_core::codegen::{self, CodeTarget};

use crate::i18n::tr;
use crate::state::settings;
use crate::state::workspace::{ToolSection, Workspace};
use crate::ui::text::prepare_error_line;

/// 抽屉的起始宽度：`Sheet` 默认的 350 px 装不下一行 curl 命令，用户仍可拖窄 / 拖宽。
const CODE_SHEET_WIDTH: f32 = 560.;

impl Workspace {
    /// 右侧图标栏的点击入口。
    pub fn open_tool_section(
        &mut self,
        section: ToolSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match section {
            ToolSection::CodeGen => self.open_code_sheet(window, cx),
        }
    }

    /// 打开（或收起）代码抽屉。再点一次同一个图标就收起，与左侧栏「点当前功能收起」同款手感。
    pub fn open_code_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_sheet(cx) {
            window.close_sheet(cx);
            return;
        }
        self.regenerate_code(window, cx);

        let weak = cx.entity().downgrade();
        window.open_sheet(cx, move |sheet, window, cx| {
            let sheet = sheet
                .size(px(CODE_SHEET_WIDTH))
                .title(div().child(tr!("tools.code.title")));
            let Some(workspace) = weak.upgrade() else {
                return sheet;
            };
            sheet.child(workspace.update(cx, |workspace, cx| {
                workspace.render_code_sheet(window, cx).into_any_element()
            }))
        });
    }

    /// 切换生成目标并重新生成。
    pub(crate) fn set_code_target(
        &mut self,
        target: CodeTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.code_target == target {
            return;
        }
        self.code_target = target;
        self.regenerate_code(window, cx);
    }

    /// 按当前 Tab 的草稿重新生成代码，灌进对应语言的编辑器。
    ///
    /// 默认请求头的开关是全局设置，所以这里现取——用户刚在设置里关掉 User-Agent，
    /// 下一次打开抽屉就该看到它消失。
    pub(crate) fn regenerate_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.active_tab().read(cx).draft(cx);
        let disabled = settings::settings(cx).request.disabled_default_headers;
        match codegen::generate(&draft, &disabled, self.code_target) {
            Ok(code) => {
                self.code_error = None;
                self.code_text = code.clone().into();
                let editor = self.code_editor().clone();
                editor.update(cx, |editor, cx| editor.set_value(code, window, cx));
            }
            Err(error) => {
                self.code_error = Some(error);
                self.code_text = SharedString::default();
            }
        }
        cx.notify();
    }

    /// 当前目标对应的编辑器实体。`CODE_LANGUAGES` 覆盖了所有 `CodeTarget`，
    /// 找不到只可能是漏加语言，退回第一个而不是 panic。
    fn code_editor(&self) -> &Entity<gpui_component::input::EditorState> {
        let language = self.code_target.editor_language();
        self.code_editors
            .iter()
            .find(|(lang, _)| *lang == language)
            .map(|(_, editor)| editor)
            .unwrap_or_else(|| &self.code_editors[0].1)
    }

    fn render_code_sheet(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = self.code_target;
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
                                this.set_code_target(CodeTarget::from_index(*ix), window, cx)
                            }))
                            .children(
                                CodeTarget::ALL
                                    .iter()
                                    .map(|target| Tab::new().label(target.label())),
                            ),
                    )
                    .child(
                        Clipboard::new("copy-code")
                            .value(self.code_text.clone())
                            .tooltip(tr!("tools.code.copy")),
                    ),
            )
            .child(match self.code_error.as_ref() {
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
                        Editor::new(self.code_editor())
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
