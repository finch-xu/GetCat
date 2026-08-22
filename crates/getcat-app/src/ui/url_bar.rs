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

use crate::SaveRequest;
use crate::i18n::tr;
use crate::state::request_tab::RequestTab;
use crate::ui::method_color;
use crate::ui::text::prepare_error_line;

impl RequestTab {
    pub fn render_url_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let in_flight = self.response.is_in_flight();
        let method = self.current_method(cx);
        // 取消请求不是破坏性提交：按设计指南用 outline，danger 只留给删除类动作
        let action_button = if in_flight {
            Button::new("cancel")
                .outline()
                .label(tr!("url_bar.cancel"))
                .icon(IconName::Close)
                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
        } else {
            Button::new("send")
                .primary()
                .label(tr!("url_bar.send"))
                .icon(IconName::Play)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
        };

        v_flex()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        // gpui-component 的 Select 外层无条件 size_full()，必须用定宽 flex_none 容器约束，
                        // 否则它会撑满整行，把 URL 输入框挤没。
                        // 可访问名称：ui 层 Select 没有透传 base 的 accessibility_label，外层组给它一个名字。
                        div()
                            .id("method-select")
                            .role(Role::Group)
                            .aria_label(tr!("url_bar.method_aria"))
                            .w_32()
                            .flex_none()
                            .child(Select::new(&self.method).text_color(method_color(method, cx))),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.url)
                                .cleanable(true)
                                .aria_label(tr!("url_bar.url_aria")),
                        ),
                    )
                    .child(
                        Button::new("save-request")
                            .outline()
                            .label(tr!("url_bar.save"))
                            .tooltip_with_action(tr!("url_bar.save_tooltip"), &SaveRequest, None)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(SaveRequest), cx)
                            }),
                    )
                    .child(action_button),
            )
            .when_some(self.prepare_error.as_ref(), |v, err| {
                v.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(prepare_error_line(err)),
                )
            })
    }
}
