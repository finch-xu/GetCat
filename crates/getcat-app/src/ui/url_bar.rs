//! URL 栏：方法下拉 + URL 输入 + 发送/取消按钮 + 校验提示。

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, IconName,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    select::Select,
    v_flex,
};

use crate::state::request_tab::RequestTab;
use crate::ui::method_color;

impl RequestTab {
    pub fn render_url_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let in_flight = self.response.is_in_flight();
        let method = self.current_method(cx);
        let action_button = if in_flight {
            Button::new("cancel")
                .danger()
                .label("取消")
                .icon(IconName::Close)
                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
        } else {
            Button::new("send")
                .primary()
                .label("发送")
                .icon(IconName::Play)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
        };

        v_flex()
            .gap_1()
            .p_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Select::new(&self.method)
                            .w(px(120.))
                            .text_color(method_color(method, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.url).cleanable(true).aria_label("请求 URL")),
                    )
                    .child(action_button),
            )
            .when_some(self.url_error.clone(), |v, err| {
                v.child(div().text_xs().text_color(cx.theme().danger).child(err))
            })
    }
}
