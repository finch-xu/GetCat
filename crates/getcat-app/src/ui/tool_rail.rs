//! 右侧固定图标栏：与左侧的功能栏对称，点图标从右侧滑出抽屉。
//!
//! 结构照 [`crate::ui::sidebar`] 的图标栏来（窗口底色、不画分隔线——两边的卡片
//! 自己就是边界），区别是没有 logo（那是左上角的品牌位），也更窄：40 px 刚好裹住 32 px 的
//! 按钮，图标到卡片与窗口边各留 12 px 左右。
//!
//! 底部放两样原本在底部状态栏里的东西：有新版本时的更新按钮，以及请求 / 响应的分栏切换。
//! 状态栏里只剩这两样，挪到这里就能把整条状态栏删掉，窗口少一行；它们和右侧工具一样是
//! 「对当前工作区」的动作，放在右栏底部比放在左栏（主题 / 设置是应用级偏好）更贴切。
//! 用弹性空白与上面的三个工具隔开，读起来是两组。
//!
//! 图标不做选中高亮：抽屉是覆盖式的（`Sheet` 绝对定位在 `right_0`），打开时正好压住
//! 这条栏，高亮了也看不见。

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, ThemeStyled as _,
    button::{Button, ButtonVariants},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use getcat_core::model::SplitDirection;

use crate::assets::{ICON_BRACES, ICON_FILE_INPUT};
use crate::i18n::tr;
use crate::state::update;
use crate::state::workspace::{ToolSection, Workspace};
use crate::ui::settings_dialog::SettingsPage;

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
    pub fn render_tool_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // 只在「有新版本」与「已装好待重启」时出现；下载 / 安装进度只在「检查更新」页显示，
        // 离线启动得到的错误也不在这里冒出来
        let update_hint = update::hint_version(self.update_status())
            .map(|(version, staged)| (version.to_string(), staged));
        v_flex()
            .id("tool-rail")
            .role(Role::Group)
            .aria_label(tr!("tools.rail_aria"))
            // 比左侧图标栏（48 px）窄一档：这边只有一列图标，没有 logo 撑宽度
            .w_10()
            .h_full()
            .flex_none()
            .items_center()
            .py_2()
            .gap_1()
            .bg(cx.theme().sidebar)
            .children(ToolSection::ALL.iter().map(|section| {
                let section = *section;
                Button::new(("rail-tool", section as usize))
                    .ghost()
                    .icon(section.icon().size_4())
                    // 纯图标按钮：gpui-component 的可访问名称取 accessibility_label.or(label)，
                    // 不会退回 tooltip，必须显式给一份，否则屏幕阅读器读不到名字。
                    .accessibility_label(section.title())
                    .tooltip(section.title())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_tool_section(section, window, cx)
                    }))
            }))
            .child(div().flex_1())
            .when_some(update_hint, |rail, (version, staged)| {
                rail.child(self.render_update_button(version, staged, cx))
            })
            .child(self.render_split_toggle(cx))
    }

    /// 更新按钮：图标 + 右上角一个主色小圆点。名字带版本号；已装好待重启时点了就重启，
    /// 否则打开设置里的「检查更新」页。
    fn render_update_button(
        &self,
        version: String,
        staged: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (label, hint) = if staged {
            (
                tr!("tools.update_restart", version = version),
                tr!("tools.update_restart_tooltip"),
            )
        } else {
            (
                tr!("tools.update_available", version = version),
                tr!("tools.update_available_tooltip"),
            )
        };
        let tooltip: SharedString = format!("{label} — {hint}").into();
        let button = Button::new(if staged {
            "update-restart"
        } else {
            "update-available"
        })
        .ghost()
        .icon(Icon::new(IconName::ArrowUp).size_4())
        .accessibility_label(label)
        .tooltip(tooltip);
        let button = if staged {
            button.on_click(|_, _, cx| update::restart(cx))
        } else {
            button.on_click(cx.listener(|this, _, window, cx| {
                this.open_settings_page(SettingsPage::Updates, window, cx)
            }))
        };
        div().relative().child(button).child(
            // 6 px 的提示点，贴着按钮右上角；纯装饰，名字已经在按钮上
            div()
                .absolute()
                .top_1()
                .right_1()
                .size_1p5()
                .rounded_full_style(cx)
                .bg(cx.theme().primary),
        )
    }

    /// 请求 / 响应分栏切换：一个按钮，图标画的是**当前**布局，点一下换成另一种；
    /// 名字说的是点下去会发生什么（「切换为上下布局」），不用先读懂图标。
    fn render_split_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (icon, label, next) = match self.split() {
            SplitDirection::Horizontal => (
                IconName::PanelRight,
                tr!("tools.layout_to_vertical"),
                SplitDirection::Vertical,
            ),
            SplitDirection::Vertical => (
                IconName::PanelBottom,
                tr!("tools.layout_to_horizontal"),
                SplitDirection::Horizontal,
            ),
        };
        Button::new("split-toggle")
            .ghost()
            .icon(Icon::new(icon).size_4())
            .accessibility_label(label.clone())
            .tooltip(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_split(next, cx)))
    }
}
