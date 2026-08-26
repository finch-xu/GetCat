//! 左侧栏 = 固定图标栏 + 可展开的功能面板。
//!
//! - **图标栏**（48 px，始终可见、从不移动）：顶部 logo，下面每个图标代表一个功能
//!   （目前只有「已保存请求」），底部是主题切换（一个按钮三种图标）与设置。
//! - **面板**（可拖宽）：点某个功能图标展开，顶部显示该功能的名字，下面是内容；
//!   再点同一个图标（或顶部的收起按钮、⌘B）收起。「已保存请求」内部是两栏主从式
//!   （spec §4.1）：左边固定宽的分类列，右边是当前分类下的请求列表。
//!
//! 没有用 gpui-component 的 `Sidebar`：它要求子项实现 `SidebarItem`（菜单式），
//! 而这里的列表是 `uniform_list` 虚拟化 + 每行带删除按钮，自绘更贴合。

use std::rc::Rc;

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
use crate::state::saved_filter::{self, SavedFilter};
use crate::state::workspace::{SidebarSection, Workspace};
use crate::templates::{self, TemplateGroup};
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
            SidebarSection::Templates => tr!("sidebar.templates.title"),
        }
    }

    fn icon(self) -> IconName {
        match self {
            SidebarSection::Saved => IconName::Inbox,
            SidebarSection::Templates => IconName::LayoutDashboard,
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
            // 测试用锚点：回归测试量的是渲染后的实际宽度（ResizableState::sizes
            // 反映不出 flex 收缩），非测试构建下这是内联空操作
            .debug_selector(|| "sidebar-panel".into())
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
                SidebarSection::Templates => self.render_template_list(cx),
            })
    }

    /// 模板面板：内置模板是编译期常量、只有几条，用不上 uniform_list 虚拟化，
    /// 直接把「组标题 + 行」平铺成一列。
    fn render_template_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let hover_bg = cx.theme().list_hover;
        let muted = cx.theme().muted_foreground;
        let radius = cx.theme().radius;
        let weak = cx.entity().downgrade();

        let mut items: Vec<AnyElement> = Vec::new();
        for group in TemplateGroup::ALL {
            items.push(
                div()
                    .px_2()
                    .pt_2()
                    .pb_0p5()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(group.label())
                    .into_any_element(),
            );
            for (ix, template) in templates::all()
                .iter()
                .enumerate()
                .filter(|(_, t)| t.group == group)
            {
                let id = template.id;
                let name = template.display_name();
                let variant = template.variant.label();
                let method = template.method;
                let open = weak.clone();
                items.push(
                    // 与已保存列表同款行高与 2 px 上下间隙，两个面板看起来是一套
                    div()
                        .h_11()
                        .w_full()
                        .py_0p5()
                        .child(
                            h_flex()
                                .id(("template-row", ix))
                                .size_full()
                                .px_2()
                                .gap_2()
                                .items_center()
                                .rounded(radius)
                                .hover(|style| style.bg(hover_bg))
                                .aria_label(tr!(
                                    "sidebar.templates.row_aria",
                                    name = name.clone(),
                                    variant = variant.clone()
                                ))
                                .on_click(move |_, window, cx| {
                                    if let Some(ws) = open.upgrade() {
                                        ws.update(cx, |ws, cx| ws.open_template(id, window, cx));
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
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .truncate()
                                                .child(variant),
                                        ),
                                ),
                        )
                        .into_any_element(),
                );
            }
        }

        v_flex()
            .id("templates")
            .flex_1()
            .min_h_0()
            .px_2()
            .pb_2()
            .overflow_y_scroll()
            .child(
                // 认证头填的是占位符，先提醒一句，省掉一次 401
                div()
                    .px_2()
                    .pt_1()
                    .text_xs()
                    .text_color(muted)
                    .child(tr!("sidebar.templates.hint")),
            )
            .children(items)
            .into_any_element()
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
        h_flex()
            .flex_1()
            .min_h_0()
            .items_stretch()
            .child(self.render_saved_groups(cx))
            .child(self.render_saved_rows(window, cx))
            .into_any_element()
    }

    /// 分类列（spec §4.1）：固定 116px，「全部」+（有未分类时）「未分类」+ 分隔线 + 分类。
    fn render_saved_groups(&self, cx: &mut Context<Self>) -> AnyElement {
        let groups = saved_filter::derive_groups(self.saved());
        let uncategorized = saved_filter::uncategorized_count(self.saved());
        let current = self.saved_filter().clone();
        let active_bg = cx.theme().list_active;
        let hover_bg = cx.theme().list_hover;
        let muted = cx.theme().muted_foreground;
        let radius = cx.theme().radius;
        let weak = cx.entity().downgrade();

        // (过滤器, 显示名, 计数, 是否真分类——真分类才有右键菜单)
        let mut entries: Vec<(SavedFilter, SharedString, usize, bool)> = vec![(
            SavedFilter::All,
            tr!("sidebar.saved.filter_all"),
            self.saved().len(),
            false,
        )];
        if uncategorized > 0 {
            entries.push((
                SavedFilter::Uncategorized,
                tr!("sidebar.saved.filter_uncategorized"),
                uncategorized,
                false,
            ));
        }
        let fixed = entries.len();
        for (name, count) in &groups {
            entries.push((
                SavedFilter::Group(name.clone()),
                SharedString::from(name.clone()),
                *count,
                true,
            ));
        }

        let mut rows: Vec<AnyElement> = Vec::new();
        for (ix, (filter, label, count, is_group)) in entries.into_iter().enumerate() {
            if ix == fixed && is_group {
                // 固定项与分类项之间的分隔线（spec §4.1：伪项靠位置和分隔线区分）
                rows.push(
                    div()
                        .h(px(1.))
                        .mx_2()
                        .my_1()
                        .bg(cx.theme().sidebar_border)
                        .into_any_element(),
                );
            }
            let selected = current == filter;
            let click_ws = weak.clone();
            let click_filter = filter.clone();
            let tooltip_label = label.clone();
            let row = h_flex()
                .id(("saved-group", ix))
                .h_8()
                .px_2()
                .mx_1()
                .gap_1()
                .items_center()
                .rounded(radius)
                .text_xs()
                .when(selected, |r| r.bg(active_bg))
                .hover(|s| s.bg(hover_bg))
                .aria_label(tr!(
                    "sidebar.saved.group_row_aria",
                    name = label.clone(),
                    count = count
                ))
                // 超长名 truncate 后靠它看全名；Div 上的 tooltip 要求闭包形式（见 #logo）
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tooltip_label.clone()).build(window, cx)
                })
                .on_click(move |_, _, cx| {
                    if let Some(ws) = click_ws.upgrade() {
                        ws.update(cx, |ws, cx| ws.set_saved_filter(click_filter.clone(), cx));
                    }
                })
                .child(div().flex_1().min_w_0().truncate().child(label.clone()))
                .child(div().flex_none().text_color(muted).child(count.to_string()));
            let row: AnyElement = if is_group {
                let name = label.to_string();
                let menu_ws = weak.clone();
                row.context_menu(move |menu, _, _| {
                    let rename_ws = menu_ws.clone();
                    let dissolve_ws = menu_ws.clone();
                    let rename_name = name.clone();
                    let dissolve_name = name.clone();
                    menu.item(
                        PopupMenuItem::new(tr!("sidebar.saved.menu_rename_group")).on_click(
                            move |_, window, cx| {
                                if let Some(ws) = rename_ws.upgrade() {
                                    ws.update(cx, |ws, cx| {
                                        ws.prompt_rename_group(rename_name.clone(), window, cx)
                                    });
                                }
                            },
                        ),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new(tr!("sidebar.saved.menu_dissolve_group")).on_click(
                            move |_, window, cx| {
                                if let Some(ws) = dissolve_ws.upgrade() {
                                    ws.update(cx, |ws, cx| {
                                        ws.confirm_dissolve_group(dissolve_name.clone(), window, cx)
                                    });
                                }
                            },
                        ),
                    )
                })
                .into_any_element()
            } else {
                row.into_any_element()
            };
            rows.push(row);
        }

        v_flex()
            .id("saved-groups")
            .role(Role::Group)
            .aria_label(tr!("sidebar.saved.groups_aria"))
            .debug_selector(|| "saved-groups".into())
            .w(px(116.))
            .flex_none()
            .h_full()
            .py_1()
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .overflow_y_scroll()
            .children(rows)
            .into_any_element()
    }

    /// 请求列：当前分类过滤下的已保存请求，右键菜单多一条「移动到分类 ▸」。
    fn render_saved_rows(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // 渲染闭包只持有 Rc 与弱引用：每帧 O(可见行)，不复制列表内容。
        let saved = self.saved_rc();
        let indices = Rc::new(self.filtered_saved_indices());
        let group_names: Rc<Vec<String>> = Rc::new(
            saved_filter::derive_groups(self.saved())
                .into_iter()
                .map(|(n, _)| n)
                .collect(),
        );
        let active_saved = self.active_tab().read(cx).saved_id;
        let weak = cx.entity().downgrade();
        let list = uniform_list(
            "saved-requests",
            indices.len(),
            move |range, _window, cx| {
                let active_bg = cx.theme().list_active;
                let hover_bg = cx.theme().list_hover;
                let muted = cx.theme().muted_foreground;
                let radius = cx.theme().radius;
                range
                    .map(|ix| {
                        let request = &saved[indices[ix]];
                        let id = request.id;
                        let method = request.draft.method;
                        let name: SharedString = request.name.clone().into();
                        let tail = tab_title(&request.draft.url);
                        let selected = active_saved == Some(id);
                        let current_group = request.group.clone();
                        let open = weak.clone();
                        let delete = weak.clone();
                        let menu_ws = weak.clone();
                        let names = group_names.clone();
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
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .truncate()
                                                .child(tail),
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
                                .context_menu(move |menu, window, cx| {
                                    let open = menu_ws.clone();
                                    let delete = menu_ws.clone();
                                    let move_ws = menu_ws.clone();
                                    let names = names.clone();
                                    let current_group = current_group.clone();
                                    menu.item(
                                        PopupMenuItem::new(tr!("sidebar.saved.menu_open"))
                                            .on_click(move |_, window, cx| {
                                                if let Some(ws) = open.upgrade() {
                                                    ws.update(cx, |ws, cx| {
                                                        ws.open_saved(id, window, cx)
                                                    });
                                                }
                                            }),
                                    )
                                    .separator()
                                    .submenu(
                                        tr!("sidebar.saved.menu_move_to"),
                                        window,
                                        cx,
                                        move |menu, _, _| {
                                            let mut menu = menu;
                                            for name in names.iter() {
                                                let target_ws = move_ws.clone();
                                                let target = name.clone();
                                                let disabled =
                                                    current_group.as_deref() == Some(name.as_str());
                                                menu = menu.item(
                                                    PopupMenuItem::new(SharedString::from(
                                                        name.clone(),
                                                    ))
                                                    .disabled(disabled)
                                                    .on_click(move |_, _, cx| {
                                                        if let Some(ws) = target_ws.upgrade() {
                                                            ws.update(cx, |ws, cx| {
                                                                ws.move_saved_to_group(
                                                                    id,
                                                                    Some(target.clone()),
                                                                    cx,
                                                                )
                                                            });
                                                        }
                                                    }),
                                                );
                                            }
                                            if current_group.is_some() {
                                                let clear_ws = move_ws.clone();
                                                menu = menu.separator().item(
                                                    PopupMenuItem::new(tr!(
                                                        "sidebar.saved.menu_clear_group"
                                                    ))
                                                    .on_click(move |_, _, cx| {
                                                        if let Some(ws) = clear_ws.upgrade() {
                                                            ws.update(cx, |ws, cx| {
                                                                ws.move_saved_to_group(id, None, cx)
                                                            });
                                                        }
                                                    }),
                                                );
                                            }
                                            let new_ws = move_ws.clone();
                                            menu.separator().item(
                                                PopupMenuItem::new(tr!(
                                                    "sidebar.saved.menu_new_group"
                                                ))
                                                .on_click(move |_, window, cx| {
                                                    if let Some(ws) = new_ws.upgrade() {
                                                        ws.update(cx, |ws, cx| {
                                                            ws.prompt_move_to_new_group(
                                                                id, window, cx,
                                                            )
                                                        });
                                                    }
                                                }),
                                            )
                                        },
                                    )
                                    .separator()
                                    .item(
                                        PopupMenuItem::new(tr!("sidebar.saved.menu_delete"))
                                            .on_click(move |_, window, cx| {
                                                if let Some(ws) = delete.upgrade() {
                                                    ws.update(cx, |ws, cx| {
                                                        ws.confirm_delete_saved(id, window, cx)
                                                    });
                                                }
                                            }),
                                    )
                                }),
                        )
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(self.saved_scroll())
        .size_full();

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .px_2()
            .pb_2()
            .child(list)
            .child(Scrollbar::vertical(self.saved_scroll()))
            .into_any_element()
    }
}
