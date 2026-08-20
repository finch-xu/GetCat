//! 顶层工作区：Tab 列表、侧栏折叠、全局动作。

use gpui::*;
use gpui_component::{
    ActiveTheme, IconName, Sizable,
    button::{Button, ButtonVariants},
    resizable::{h_resizable, resizable_panel},
    tab::{Tab, TabBar},
    v_flex,
};

use crate::state::request_tab::RequestTab;
use crate::ui::sidebar::render_sidebar;
use crate::{CloseTab, NewTab, SendRequest, ToggleSidebar};

pub struct Workspace {
    tabs: Vec<Entity<RequestTab>>,
    active: usize,
    next_id: u64,
    sidebar_collapsed: bool,
    focus_handle: FocusHandle,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut ws = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            sidebar_collapsed: false,
            focus_handle: cx.focus_handle(),
        };
        ws.new_tab(window, cx);
        ws
    }

    pub fn active_tab(&self) -> Entity<RequestTab> {
        self.tabs[self.active].clone()
    }

    pub fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;
        let tab = cx.new(|cx| RequestTab::new(id, window, cx));
        tab.update(cx, |t, cx| t.focus_url(window, cx));
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        cx.notify();
    }

    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            self.tabs.clear();
            self.new_tab(window, cx);
            return;
        }
        self.tabs.remove(ix);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if ix < self.active {
            self.active -= 1;
        }
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
