//! 左侧栏：标题 + 主题切换按钮 + 已保存请求列表（Task 8 填充列表）。

use gpui::*;
use gpui_component::{
    ActiveTheme, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

use getcat_core::model::ThemePref;

use crate::state::workspace::Workspace;

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
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_1()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child("尚无已保存的请求")
            .child(div().text_xs().child("用 ⌘S / Ctrl S 保存当前请求"))
            .into_any_element()
    }
}
