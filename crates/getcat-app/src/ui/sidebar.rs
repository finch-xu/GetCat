//! 左侧栏 = 固定图标栏 + 可展开的功能面板。
//!
//! - **图标栏**（48 px，始终可见、从不移动）：顶部 logo，下面每个图标代表一个功能
//!   （目前只有「已保存请求」），底部是主题切换（一个按钮三种图标）与设置。
//! - **面板**（240 px，可拖宽）：点某个功能图标展开，顶部显示该功能的名字，下面是内容；
//!   再点同一个图标（或顶部的收起按钮、⌘B）收起。
//!
//! 没有用 gpui-component 的 `Sidebar`：它要求子项实现 `SidebarItem`（菜单式），
//! 而这里的列表是 `uniform_list` 虚拟化 + 每行带删除按钮，自绘更贴合。

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, Icon, IconName, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    kbd::Kbd,
    menu::{ContextMenuExt as _, PopupMenuItem},
    scroll::Scrollbar,
    v_flex,
};

use getcat_core::model::ThemePref;

use crate::assets::LOGO_PATH;
use crate::brand::APP_NAME;
use crate::i18n::tr;
use crate::state::request_tab::tab_title;
use crate::state::workspace::{SidebarSection, Workspace};
use crate::ui::method_color;
use crate::ui::text::theme_label;
use crate::{OpenSettings, SaveRequest, ToggleSidebar};

/// 列表行高：布局用 `h_11()`（2.75 rem），默认 rem 下等于 44 px；
/// 测试按这个像素值核对 uniform_list 的内容高度。
#[cfg(test)]
pub const SAVED_ROW_HEIGHT: f32 = 44.;

fn theme_icon(pref: ThemePref) -> IconName {
    match pref {
        ThemePref::System => IconName::Palette,
        ThemePref::Light => IconName::Sun,
        ThemePref::Dark => IconName::Moon,
    }
}

impl SidebarSection {
    pub fn title(self) -> SharedString {
        match self {
            SidebarSection::Saved => tr!("sidebar.saved.title"),
        }
    }

    fn icon(self) -> IconName {
        match self {
            SidebarSection::Saved => IconName::Inbox,
        }
    }
}

impl Workspace {
    /// 固定图标栏。
    pub fn render_sidebar_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let expanded = !self.sidebar_collapsed();
        let current = self.sidebar_section();
        let saved_count = self.saved().len();
        v_flex()
            .id("sidebar-rail")
            .role(Role::Group)
            .aria_label(tr!("sidebar.rail_aria"))
            // 48 px：与 gpui-component `Sidebar` 的折叠宽度一致
            .w_12()
            .h_full()
            .flex_none()
            .items_center()
            .py_2()
            .gap_1()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                div()
                    .id("logo")
                    .size_9()
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new(APP_NAME).build(window, cx)
                    })
                    // 26 px 是位图本身的尺寸：设计指南允许的像素例外
                    .child(img(LOGO_PATH).size(px(26.)).flex_none()),
            )
            .child(
                // 1 px 细线是物理边界，宽度走 rem 刻度
                div().h(px(1.)).w_6().my_1().bg(cx.theme().sidebar_border),
            )
            .children(SidebarSection::ALL.iter().map(|section| {
                let section = *section;
                let tooltip: SharedString = match (section, saved_count) {
                    (SidebarSection::Saved, n) if n > 0 => {
                        tr!("sidebar.saved.title_with_count", count = n)
                    }
                    _ => section.title(),
                };
                Button::new(("rail-section", section as usize))
                    .ghost()
                    .selected(expanded && current == section)
                    .icon(Icon::new(section.icon()).size_4())
                    .tooltip(tooltip)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_sidebar_section(section, cx)),
                    )
            }))
            .child(div().flex_1())
            .child(
                Button::new("rail-theme")
                    .ghost()
                    .icon(Icon::new(theme_icon(theme)).size_4())
                    .tooltip(tr!("sidebar.theme_tooltip", theme = theme_label(theme)))
                    .on_click(cx.listener(|this, _, window, cx| this.cycle_theme(window, cx))),
            )
            .child(
                Button::new("rail-settings")
                    .ghost()
                    .icon(Icon::new(IconName::Settings).size_4())
                    .tooltip_with_action(tr!("sidebar.settings"), &OpenSettings, None)
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            )
    }

    /// 展开的功能面板：标题行 + 当前功能的内容。
    pub fn render_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self.sidebar_section();
        v_flex()
            .id("sidebar-panel")
            .role(Role::Group)
            .aria_label(section.title())
            .size_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                h_flex()
                    .h_10()
                    .flex_none()
                    .pl_4()
                    .pr_2()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().sidebar_foreground)
                            .truncate()
                            .child(section.title()),
                    )
                    .child(
                        Button::new("collapse-sidebar")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::PanelLeftClose).size_4())
                            .tooltip_with_action(tr!("sidebar.collapse"), &ToggleSidebar, None)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    ),
            )
            .child(match section {
                SidebarSection::Saved => self.render_saved_list(window, cx),
            })
    }

    fn render_saved_list(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.saved().is_empty() {
            // 快捷键用 Kbd 按当前绑定渲染（平台自动显示 ⌘ 或 Ctrl），不手写字符串
            let save_key = Kbd::binding_for_action(&SaveRequest, None, window);
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_1()
                .px_4()
                .text_sm()
                .text_center()
                .text_color(cx.theme().muted_foreground)
                .child(tr!("sidebar.saved.empty_title"))
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .text_xs()
                        .child(tr!("sidebar.saved.empty_hint_prefix"))
                        .children(save_key)
                        .child(tr!("sidebar.saved.empty_hint_suffix")),
                )
                .into_any_element();
        }
        // 渲染闭包只持有 Rc 与弱引用：每帧 O(可见行)，不复制列表内容。
        let saved = self.saved_rc();
        let active_saved = self.active_tab().read(cx).saved_id;
        let weak = cx.entity().downgrade();
        let list = uniform_list("saved-requests", saved.len(), move |range, _window, cx| {
            let active_bg = cx.theme().list_active;
            let hover_bg = cx.theme().list_hover;
            let muted = cx.theme().muted_foreground;
            let radius = cx.theme().radius;
            range
                .map(|ix| {
                    let request = &saved[ix];
                    let id = request.id;
                    let method = request.draft.method;
                    let name: SharedString = request.name.clone().into();
                    let tail = tab_title(&request.draft.url);
                    let selected = active_saved == Some(id);
                    let open = weak.clone();
                    let delete = weak.clone();
                    let menu_ws = weak.clone();
                    // 行本身留 2 px 上下间隙，让选中底色成为一个悬浮的圆角块而不是整条色带
                    div().h_11().w_full().py_0p5().child(
                        h_flex()
                            .id(("saved-row", ix))
                            .group("saved-row")
                            .size_full()
                            .px_2()
                            .gap_2()
                            .items_center()
                            .rounded(radius)
                            .when(selected, |row| row.bg(active_bg))
                            .hover(|style| style.bg(hover_bg))
                            .aria_label(tr!("sidebar.saved.row_aria", name = name))
                            .on_click(move |_, window, cx| {
                                if let Some(ws) = open.upgrade() {
                                    ws.update(cx, |ws, cx| ws.open_saved(id, window, cx));
                                }
                            })
                            .child(
                                div()
                                    .w_12()
                                    .flex_none()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(method_color(method, cx))
                                    .child(method.as_str()),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().text_sm().truncate().child(name))
                                    .child(
                                        div().text_xs().text_color(muted).truncate().child(tail),
                                    ),
                            )
                            .child(
                                Button::new(("saved-delete", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Delete)
                                    // Button 只实现 InteractiveElement（非 Stateful），没有
                                    // aria_label；tooltip 就是它对外的可读名字。
                                    .tooltip(tr!("sidebar.saved.delete"))
                                    .on_click(move |_, window, cx| {
                                        // 不让点击冒泡成"打开"
                                        cx.stop_propagation();
                                        if let Some(ws) = delete.upgrade() {
                                            ws.update(cx, |ws, cx| {
                                                ws.confirm_delete_saved(id, window, cx)
                                            });
                                        }
                                    }),
                            )
                            // 对象级命令放右键菜单（设计指南）：与行内按钮是同一组动作，
                            // 键盘与辅助技术用户多一条可达路径
                            .context_menu(move |menu, _, _| {
                                let open = menu_ws.clone();
                                let delete = menu_ws.clone();
                                menu.item(
                                    PopupMenuItem::new(tr!("sidebar.saved.menu_open")).on_click(
                                        move |_, window, cx| {
                                            if let Some(ws) = open.upgrade() {
                                                ws.update(cx, |ws, cx| {
                                                    ws.open_saved(id, window, cx)
                                                });
                                            }
                                        },
                                    ),
                                )
                                .separator()
                                .item(
                                    PopupMenuItem::new(tr!("sidebar.saved.menu_delete")).on_click(
                                        move |_, window, cx| {
                                            if let Some(ws) = delete.upgrade() {
                                                ws.update(cx, |ws, cx| {
                                                    ws.confirm_delete_saved(id, window, cx)
                                                });
                                            }
                                        },
                                    ),
                                )
                            }),
                    )
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(self.saved_scroll())
        .size_full();

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .px_2()
            .pb_2()
            .child(list)
            .child(Scrollbar::vertical(self.saved_scroll()))
            .into_any_element()
    }
}
