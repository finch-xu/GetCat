//! 左侧栏：标题 + 主题切换按钮 + 已保存请求列表（gpui uniform_list，按 updated_at 降序，O(可见行)）。

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::Scrollbar,
    v_flex,
};

use getcat_core::model::ThemePref;

use crate::state::request_tab::tab_title;
use crate::state::workspace::Workspace;
use crate::ui::method_color;

/// 列表行高（uniform_list 要求等高）。
pub const SAVED_ROW_HEIGHT: f32 = 44.;

fn theme_icon(pref: ThemePref) -> IconName {
    match pref {
        ThemePref::System => IconName::Palette,
        ThemePref::Light => IconName::Sun,
        ThemePref::Dark => IconName::Moon,
    }
}

impl Workspace {
    pub fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                h_flex()
                    .h(px(40.))
                    .px_3()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().sidebar_foreground)
                            .child("已保存请求"),
                    )
                    .child(
                        Button::new("theme-toggle")
                            .ghost()
                            .xsmall()
                            .icon(theme_icon(theme))
                            .tooltip(format!("主题：{}（点击切换）", theme.label()))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.cycle_theme(window, cx)),
                            ),
                    ),
            )
            .child(self.render_saved_list(cx))
    }

    fn render_saved_list(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.saved().is_empty() {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_1()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("尚无已保存的请求")
                .child(div().text_xs().child("用 ⌘S / Ctrl S 保存当前请求"))
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
                    h_flex()
                        .id(("saved-row", ix))
                        .w_full()
                        .h(px(SAVED_ROW_HEIGHT))
                        .px_2()
                        .gap_2()
                        .items_center()
                        .rounded_md()
                        .when(selected, |row| row.bg(active_bg))
                        .hover(|style| style.bg(hover_bg))
                        .aria_label(format!("已保存请求：{name}"))
                        .on_click(move |_, window, cx| {
                            if let Some(ws) = open.upgrade() {
                                ws.update(cx, |ws, cx| ws.open_saved(id, window, cx));
                            }
                        })
                        .child(
                            div()
                                .w(px(52.))
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
                                .child(div().text_xs().text_color(muted).truncate().child(tail)),
                        )
                        .child(
                            Button::new(("saved-delete", ix))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Delete)
                                // Button 只实现 InteractiveElement（非 Stateful），没有
                                // aria_label；tooltip 就是它对外的可读名字。
                                .tooltip("删除")
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
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(self.saved_scroll())
        .size_full();

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .p_1()
            .child(list)
            .child(Scrollbar::vertical(self.saved_scroll()))
            .into_any_element()
    }
}
