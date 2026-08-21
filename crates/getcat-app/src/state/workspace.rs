//! 顶层工作区：Tab 列表、侧栏折叠、全局动作。

// 显式导入而非 `use gpui::*`：本文件含 `#[cfg(test)] mod tests`，通配符会引入
// gpui 重导出的 `#[proc_macro_attribute] test`，与标准库 `#[test]` 同名冲突，
// 导致该属性宏对自身生成的 `#[test]` 反复展开直至递归上限溢出。
use gpui::{
    AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable,
    button::{Button, ButtonVariants},
    resizable::{h_resizable, resizable_panel},
    tab::{Tab, TabBar},
    v_flex,
};

use getcat_core::model::Ulid;

use crate::state::request_tab::RequestTab;
use crate::state::store::store;
use crate::ui::sidebar::render_sidebar;
use crate::{CloseTab, NewTab, SendRequest, ToggleSidebar};

pub struct Workspace {
    tabs: Vec<Entity<RequestTab>>,
    active: usize,
    sidebar_collapsed: bool,
    focus_handle: FocusHandle,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut ws = Self {
            tabs: Vec::new(),
            active: 0,
            sidebar_collapsed: false,
            focus_handle: cx.focus_handle(),
        };
        ws.new_tab(window, cx);
        ws
    }

    pub fn active_tab(&self) -> Entity<RequestTab> {
        self.tabs[self.active].clone()
    }

    #[cfg(test)]
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    #[cfg(test)]
    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab = cx.new(|cx| RequestTab::new(Ulid::generate(), window, cx));
        tab.update(cx, |t, cx| {
            t.focus_url(window, cx);
            // 立即写一份空草稿：重启时 workspace.json 的 tab_order 才找得到它
            t.save_draft_now(cx);
        });
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        cx.notify();
    }

    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // 关闭 Tab = 删草稿文件（spec §9.3）；关最后一个时先删再新建
        let closing = self.tabs[ix].read(cx).id;
        if let Some(store) = store(cx) {
            store.delete_draft(closing);
        }
        if self.tabs.len() == 1 {
            self.tabs.clear();
            self.new_tab(window, cx);
            return;
        }
        let len = self.tabs.len();
        self.tabs.remove(ix);
        self.active = active_after_close(self.active, ix, len);
        cx.notify();
    }

    pub fn activate(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() && ix != self.active {
            self.active = ix;
            cx.notify();
        }
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let titles: Vec<SharedString> = self.tabs.iter().map(|t| t.read(cx).title(cx)).collect();
        TabBar::new("request-tabs")
            .w_full()
            .selected_index(self.active)
            .on_click(cx.listener(|this, ix: &usize, _, cx| this.activate(*ix, cx)))
            .children(titles.into_iter().enumerate().map(|(ix, title)| {
                Tab::new()
                    .label(title.clone())
                    .aria_label(format!("请求 Tab：{title}"))
                    .suffix(
                        Button::new(("close-tab", ix))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip("关闭 Tab")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // 阻止冒泡到 Tab 的 on_click，否则关闭后会再触发 activate(ix)
                                cx.stop_propagation();
                                this.close_tab(ix, window, cx)
                            })),
                    )
            }))
            .suffix(
                Button::new("new-tab")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("新建请求")
                    .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx))),
            )
    }
}

/// 关闭 `closing` 后新的 active 下标；`len` 为关闭前的 Tab 数（≥ 2）。
pub(crate) fn active_after_close(active: usize, closing: usize, len: usize) -> usize {
    let new_len = len - 1;
    if active >= new_len {
        // 关掉的是末尾且它就是 active：夹到新的末尾。
        new_len - 1
    } else if closing < active {
        active - 1
    } else {
        active
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active_tab();
        div()
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.new_tab(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                let ix = this.active;
                this.close_tab(ix, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_collapsed = !this.sidebar_collapsed;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SendRequest, window, cx| {
                this.active_tab().update(cx, |tab, cx| tab.send(window, cx))
            }))
            .child(
                h_resizable("workspace")
                    .child(
                        resizable_panel()
                            .size(px(240.))
                            .size_range(px(180.)..px(420.))
                            .visible(!self.sidebar_collapsed)
                            .child(render_sidebar(cx)),
                    )
                    .child(
                        resizable_panel().child(
                            v_flex()
                                .size_full()
                                .min_w_0()
                                .child(self.render_tab_bar(cx))
                                .child(div().flex_1().min_h_0().child(active)),
                        ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::active_after_close;

    #[test]
    fn closing_left_of_active_shifts_active_left() {
        assert_eq!(active_after_close(2, 0, 4), 1);
        assert_eq!(active_after_close(1, 0, 3), 0);
    }

    #[test]
    fn closing_active_keeps_index_pointing_at_right_neighbour() {
        assert_eq!(active_after_close(1, 1, 3), 1);
        assert_eq!(active_after_close(0, 0, 2), 0);
    }

    #[test]
    fn closing_last_active_clamps_to_new_last() {
        assert_eq!(active_after_close(2, 2, 3), 1);
        assert_eq!(active_after_close(1, 1, 2), 0);
    }

    #[test]
    fn closing_right_of_active_leaves_active_alone() {
        assert_eq!(active_after_close(0, 1, 3), 0);
        assert_eq!(active_after_close(1, 2, 4), 1);
    }
}
