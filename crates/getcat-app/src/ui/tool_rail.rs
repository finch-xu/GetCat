//! 右侧固定图标栏：与左侧的功能栏对称，点图标从右侧滑出抽屉。
//!
//! 结构照 [`crate::ui::sidebar`] 的图标栏来（48 px、`sidebar` 底色、一条分隔边框），
//! 区别只有三处：边框在左而不是右；没有 logo（那是左上角的品牌位）；底部不放常驻按钮，
//! 免得和左下角的主题 / 设置看起来像一对配套的东西。
//!
//! 图标不做选中高亮：抽屉是覆盖式的（`Sheet` 绝对定位在 `right_0`），打开时正好压住
//! 这条 48 px 的栏，高亮了也看不见。

use gpui::*;
use gpui_component::{
    ActiveTheme, Icon, IconName,
    button::{Button, ButtonVariants},
    v_flex,
};

use crate::i18n::tr;
use crate::state::workspace::{ToolSection, Workspace};

impl ToolSection {
    pub fn title(self) -> SharedString {
        match self {
            ToolSection::CodeGen => tr!("tools.code.title"),
        }
    }

    fn icon(self) -> IconName {
        match self {
            ToolSection::CodeGen => IconName::SquareTerminal,
        }
    }
}

impl Workspace {
    pub fn render_tool_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("tool-rail")
            .role(Role::Group)
            .aria_label(tr!("tools.rail_aria"))
            // 与左侧图标栏同宽，两边看起来是一套
            .w_12()
            .h_full()
            .flex_none()
            .items_center()
            .py_2()
            .gap_1()
            .bg(cx.theme().sidebar)
            .border_l_1()
            .border_color(cx.theme().sidebar_border)
            .children(ToolSection::ALL.iter().map(|section| {
                let section = *section;
                Button::new(("rail-tool", section as usize))
                    .ghost()
                    .icon(Icon::new(section.icon()).size_4())
                    .tooltip(section.title())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_tool_section(section, window, cx)
                    }))
            }))
    }
}
