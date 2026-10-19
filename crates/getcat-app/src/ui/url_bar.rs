//! URL 栏：标题行（分类 › 请求名）+ 一整条胶囊（方法 | URL | HTTP 版本）+ 发送 / 取消 +
//! 保存，以及校验提示。

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    select::Select,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use getcat_core::model::HttpVersionPref;

use crate::i18n::tr;
use crate::state::request_tab::RequestTab;
use crate::ui::method_color;
use crate::ui::text::{prepare_error_line, unresolved_vars_line};
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

/// 发送快捷键的显示文字，与 `main.rs` 里 `primary("enter")` 的绑定一致。
/// 是按键符号不是文案，不走 i18n。
fn send_shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘↩"
    } else {
        "Ctrl+↩"
    }
}

impl RequestTab {
    /// 标题行：已保存请求显示「分类 › 请求名」（未分类只有名字），未保存的显示 URL 末段；
    /// 有未保存的改动时跟一句提示。只是展示，改名仍走保存对话框。
    pub fn render_title_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        h_flex()
            .h_7()
            .px_1()
            .gap_2()
            .items_center()
            .min_w_0()
            .when_some(self.saved_group.clone(), |h, group| {
                h.child(div().flex_none().text_sm().text_color(muted).child(group))
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .xsmall()
                            .text_color(muted)
                            .flex_none(),
                    )
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title(cx)),
            )
            .when(self.dirty, |h| {
                h.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(muted)
                        .child(tr!("url_bar.unsaved_changes")),
                )
            })
    }

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
                .large()
                .label(tr!("url_bar.cancel"))
                .icon(IconName::Close)
                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
        } else {
            Button::new("send")
                .primary()
                .large()
                .label(tr!("url_bar.send"))
                .icon(IconName::Play)
                // 快捷键直接写在按钮上：它是这一行最常用的动作，不该藏进 tooltip
                .child(div().text_xs().opacity(0.75).child(send_shortcut_label()))
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
        };

        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        // 方法 | URL | HTTP 版本 拼成一整条胶囊：边框与圆角画在这一层，
                        // 里面的 Select / Input 去掉各自的外观，读起来是一个输入而不是三个。
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .h_10()
                            .items_center()
                            .bg(cx.theme().background)
                            .border_1()
                            .border_color(cx.theme().input)
                            .rounded(cx.theme().radius_lg)
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
                                    .child(
                                        Select::new(&self.method)
                                            .appearance(false)
                                            .text_color(method_color(method, cx)),
                                    ),
                            )
                            // 1 px 细线是物理分隔
                            .child(div().flex_none().w(px(1.)).h_5().bg(cx.theme().border))
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.url)
                                        .appearance(false)
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
                                                            PopupMenuItem::new(version_menu_label(
                                                                pref,
                                                            ))
                                                            .checked(pref == http_version)
                                                            .on_click(move |_, _, cx| {
                                                                let Some(tab) = owner.upgrade()
                                                                else {
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
                                    .large()
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
                                    .large()
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
            .when(!self.unresolved_vars.is_empty(), |v| {
                v.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child(unresolved_vars_line(&self.unresolved_vars)),
                )
            })
    }
}
