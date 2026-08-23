//! URL 栏：方法下拉 + URL 输入 + 发送/取消按钮 + 校验提示。

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, IconName, Sizable as _,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    select::Select,
    v_flex,
};

use getcat_core::model::HttpVersionPref;

use crate::i18n::tr;
use crate::state::request_tab::RequestTab;
use crate::ui::method_color;
use crate::ui::text::prepare_error_line;
use crate::{DuplicateTab, SaveRequest};

/// 收起来显示在输入框里的短标签。协议名照写，只有 `Auto` 需要本地化。
fn version_label(pref: HttpVersionPref) -> SharedString {
    match pref {
        HttpVersionPref::Auto => tr!("url_bar.http_version_auto"),
        other => other.label().into(),
    }
}

/// 菜单里的完整标签：`Auto` 多带一句它到底做了什么。
fn version_menu_label(pref: HttpVersionPref) -> SharedString {
    match pref {
        HttpVersionPref::Auto => tr!("url_bar.http_version_auto_hint"),
        other => other.label().into(),
    }
}

impl RequestTab {
    pub fn render_url_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let in_flight = self.response.is_in_flight();
        let method = self.current_method(cx);
        let this = cx.entity().downgrade();
        let version_owner = this.clone();
        let http_version = self.http_version;
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
                                .aria_label(tr!("url_bar.url_aria"))
                                // 版本选择贴在输入框内的最右侧。Input 会把 cleanable
                                // 的清除按钮和这个 suffix 放进同一个容器，互不挤占。
                                .suffix(
                                    Button::new("http-version")
                                        .ghost()
                                        .xsmall()
                                        .label(version_label(http_version))
                                        .dropdown_caret(true)
                                        .tooltip(tr!("url_bar.http_version_tooltip"))
                                        .dropdown_menu(move |menu, _, _| {
                                            let owner = version_owner.clone();
                                            let mut menu = menu;
                                            for pref in HttpVersionPref::ALL {
                                                let owner = owner.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(version_menu_label(pref))
                                                        .checked(pref == http_version)
                                                        .on_click(move |_, _, cx| {
                                                            let Some(tab) = owner.upgrade() else {
                                                                return;
                                                            };
                                                            // 只影响下一次发送，不进 draft，
                                                            // 所以不置脏
                                                            tab.update(cx, |t, cx| {
                                                                t.http_version = pref;
                                                                cx.notify();
                                                            });
                                                        }),
                                                );
                                            }
                                            // h3 需要 reqwest 的 unstable feature，
                                            // 列出来但点不动，免得让人以为漏做了
                                            menu.item(
                                                PopupMenuItem::new(tr!(
                                                    "url_bar.http3_unsupported"
                                                ))
                                                .disabled(true),
                                            )
                                        }),
                                ),
                        ),
                    )
                    // 发送在前：它才是这一行的主操作，主按钮紧贴 URL 更顺手
                    .child(action_button)
                    .child(
                        // 保存做成 split button：左半按下即保存，右半只管展开菜单。
                        // dropdown_menu 会把整个 Button 包成 popover 触发器，两半
                        // 因此必须是独立按钮，去掉相接侧的圆角拼成一体。
                        h_flex()
                            .child(
                                Button::new("save-request")
                                    .outline()
                                    .label(tr!("url_bar.save"))
                                    .rounded_r_none()
                                    .tooltip_with_action(
                                        tr!("url_bar.save_tooltip"),
                                        &SaveRequest,
                                        None,
                                    )
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(SaveRequest), cx)
                                    }),
                            )
                            .child(
                                Button::new("save-more")
                                    .outline()
                                    .rounded_l_none()
                                    .dropdown_caret(true)
                                    .tooltip(tr!("url_bar.more_tooltip"))
                                    .dropdown_menu(move |menu, _, _| {
                                        let this = this.clone();
                                        menu.item(
                                            PopupMenuItem::new(tr!("url_bar.duplicate_tab"))
                                                .on_click(move |_, window, cx| {
                                                    // 菜单是 popover，焦点在它自己身上；
                                                    // 先把焦点交还给 URL 输入框，动作才能
                                                    // 冒泡到 Workspace 的 on_action
                                                    if let Some(tab) = this.upgrade() {
                                                        tab.update(cx, |t, cx| {
                                                            t.focus_url(window, cx)
                                                        });
                                                    }
                                                    window.dispatch_action(
                                                        Box::new(DuplicateTab),
                                                        cx,
                                                    );
                                                }),
                                        )
                                    }),
                            ),
                    ),
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
