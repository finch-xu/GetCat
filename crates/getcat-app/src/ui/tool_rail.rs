//! 标题栏右侧的工具按钮：生成代码 / 导入 cURL / 变量，点了从右侧滑出抽屉。
//!
//! 2026-10 重设计前它们是右侧一条 48 px 的固定图标栏；并进标题栏后窗口少一列，
//! 抽屉（`Sheet`，绝对定位在 `right_0`）打开时也不再压住任何常驻控件。
//! 不做选中高亮：抽屉是覆盖式的，打开时焦点和视线都在抽屉里。

use gpui_kit::component::{
    Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants},
    h_flex,
};
use gpui_kit::*;

use crate::assets::{ICON_BRACES, ICON_FILE_INPUT};
use crate::i18n::tr;
use crate::state::workspace::{ToolSection, Workspace};

impl ToolSection {
    pub fn title(self) -> SharedString {
        match self {
            ToolSection::CodeGen => tr!("tools.code.title"),
            ToolSection::ImportCurl => tr!("tools.import_curl.title"),
            ToolSection::Variables => tr!("tools.variables.title"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            ToolSection::CodeGen => Icon::new(IconName::SquareTerminal),
            // 内置图标集里没有合适的「导入」图形，用补进 AppAssets 的那枚
            ToolSection::ImportCurl => Icon::empty().path(ICON_FILE_INPUT),
            ToolSection::Variables => Icon::empty().path(ICON_BRACES),
        }
    }
}

impl Workspace {
    pub fn render_tool_buttons(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("tool-buttons")
            .role(Role::Group)
            .aria_label(tr!("tools.rail_aria"))
            .flex_none()
            .items_center()
            .gap_0p5()
            .children(ToolSection::ALL.iter().map(|section| {
                let section = *section;
                Button::new(("rail-tool", section as usize))
                    .ghost()
                    .small()
                    .icon(section.icon().size_4())
                    // 纯图标按钮：gpui-component 的可访问名称取 accessibility_label.or(label)，
                    // 不会退回 tooltip，必须显式给一份，否则屏幕阅读器读不到名字。
                    .accessibility_label(section.title())
                    .tooltip(section.title())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_tool_section(section, window, cx)
                    }))
            }))
    }
}
