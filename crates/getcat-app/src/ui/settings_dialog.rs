//! 设置对话框：左侧分类（gpui-component `Settings` 自带的侧栏 + 搜索），右侧是具体设置项。
//!
//! 设置值的读写都走 [`crate::state::settings`]（请求段改了重建 HTTP client、字号直接套到主题），
//! 主题偏好仍记在 [`Workspace`]（它属于布局状态，写进 `workspace.json`）。

use getcat_core::model::{EDITOR_FONT_SIZE_RANGE, ThemePref};
use gpui::{
    App, Entity, FontWeight, ParentElement, SharedString, Styled, WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogFooter,
    h_flex,
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};

use crate::state::settings;
use crate::state::store::store;
use crate::state::workspace::Workspace;

const DIALOG_WIDTH: f32 = 760.;
const CONTENT_HEIGHT: f32 = 460.;

/// 打开设置对话框（⌘, / 侧栏齿轮）。已有对话框时不叠加第二个。
pub fn open_settings(workspace: Entity<Workspace>, window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let weak = workspace.downgrade();
    window.open_dialog(cx, move |dialog, _, _| {
        let weak = weak.clone();
        dialog
            .title("设置")
            .w(px(DIALOG_WIDTH))
            .content(move |content, _, cx| {
                content.child(
                    div()
                        .w_full()
                        .h(px(CONTENT_HEIGHT))
                        .child(render_settings(weak.clone(), cx)),
                )
            })
            .footer(
                // 不用 DialogClose 包按钮：它外面套了一层 size_full 的 div，会把按钮拉成整行
                DialogFooter::new()
                    .w_full()
                    .justify_between()
                    .child(
                        Button::new("reset-settings")
                            .ghost()
                            .label("恢复默认")
                            .on_click(|_, _, cx| settings::reset(cx)),
                    )
                    .child(
                        Button::new("close-settings")
                            .primary()
                            .label("完成")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}

fn render_settings(workspace: WeakEntity<Workspace>, cx: &App) -> Settings {
    Settings::new("app-settings")
        .sidebar_width(px(180.))
        .page(general_page(workspace))
        .page(request_page())
        .page(data_page(cx))
        .page(about_page())
}

fn general_page(workspace: WeakEntity<Workspace>) -> SettingPage {
    let theme_for_read = workspace.clone();
    SettingPage::new("通用").icon(IconName::Settings).group(
        SettingGroup::new()
            .title("外观")
            .item(
                SettingItem::new(
                    "主题",
                    SettingField::dropdown(
                        ThemePref::ALL
                            .iter()
                            .map(|p| (theme_key(*p), SharedString::from(p.label())))
                            .collect(),
                        move |cx| {
                            theme_for_read
                                .upgrade()
                                .map(|ws| theme_key(ws.read(cx).theme()))
                                .unwrap_or_else(|| theme_key(ThemePref::System))
                        },
                        move |value, cx| {
                            if let Some(ws) = workspace.upgrade() {
                                let pref = ThemePref::ALL
                                    .iter()
                                    .copied()
                                    .find(|p| theme_key(*p) == value)
                                    .unwrap_or_default();
                                ws.update(cx, |ws, cx| ws.set_theme_global(pref, cx));
                            }
                        },
                    )
                    .default_value(theme_key(ThemePref::System)),
                )
                .description("跟随系统会随 macOS 的外观自动切换"),
            )
            .item(
                SettingItem::new(
                    "编辑器字号",
                    SettingField::number_input(
                        NumberFieldOptions {
                            min: *EDITOR_FONT_SIZE_RANGE.start() as f64,
                            max: *EDITOR_FONT_SIZE_RANGE.end() as f64,
                            step: 1.0,
                        },
                        |cx| settings::settings(cx).editor_font_size as f64,
                        |value, cx| {
                            settings::update(cx, |s| s.editor_font_size = value.round() as u32)
                        },
                    )
                    .default_value(13.0),
                )
                .description("请求 Body 与响应正文使用的等宽字号（px）"),
            ),
    )
}

fn request_page() -> SettingPage {
    SettingPage::new("请求")
        .icon(IconName::Globe)
        .group(
            SettingGroup::new().title("超时").item(
                SettingItem::new(
                    "总超时（秒）",
                    SettingField::number_input(
                        NumberFieldOptions {
                            min: 0.0,
                            max: 3600.0,
                            step: 5.0,
                        },
                        |cx| settings::settings(cx).request.timeout_secs as f64,
                        |value, cx| {
                            settings::update(cx, |s| {
                                s.request.timeout_secs = value.max(0.0).round() as u64
                            })
                        },
                    )
                    .default_value(30.0),
                )
                .description("从连接到读完响应的总时长上限；0 表示不限。连接超时固定 10 秒"),
            ),
        )
        .group(
            SettingGroup::new()
                .title("跳转")
                .item(
                    SettingItem::new(
                        "自动跟随 3xx 跳转",
                        SettingField::switch(
                            |cx| settings::settings(cx).request.follow_redirects,
                            |value, cx| {
                                settings::update(cx, |s| s.request.follow_redirects = value)
                            },
                        )
                        .default_value(true),
                    )
                    .description("关闭后直接显示 301 / 302 等响应本身"),
                )
                .item(SettingItem::new(
                    "最大跳转次数",
                    SettingField::number_input(
                        NumberFieldOptions {
                            min: 1.0,
                            max: 50.0,
                            step: 1.0,
                        },
                        |cx| settings::settings(cx).request.max_redirects as f64,
                        |value, cx| {
                            settings::update(cx, |s| {
                                s.request.max_redirects = value.clamp(1.0, 50.0).round() as u32
                            })
                        },
                    )
                    .default_value(10.0),
                )),
        )
        .group(
            SettingGroup::new().title("安全").item(
                SettingItem::new(
                    "校验 TLS 证书",
                    SettingField::switch(
                        |cx| settings::settings(cx).request.verify_tls,
                        |value, cx| settings::update(cx, |s| s.request.verify_tls = value),
                    )
                    .default_value(true),
                )
                .description("关闭后接受自签名 / 过期证书，仅建议用于本地调试"),
            ),
        )
}

fn data_page(cx: &App) -> SettingPage {
    let root: Option<SharedString> = store(cx).map(|s| s.root().display().to_string().into());
    let path_for_reveal = root.clone();
    SettingPage::new("数据").icon(IconName::HardDrive).group(
        SettingGroup::new()
            .title("存储位置")
            .description(
                "已保存请求、草稿与设置都以 JSON 文件形式放在这里，可直接备份或用 Git 管理",
            )
            .item(SettingItem::render(move |_, _, cx| {
                let mono = cx.theme().mono_font_family.clone();
                let muted = cx.theme().muted_foreground;
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("数据目录"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .font_family(mono)
                                    .truncate()
                                    .child(
                                        root.clone().unwrap_or_else(|| "（持久化不可用）".into()),
                                    ),
                            ),
                    )
                    .child({
                        let path = path_for_reveal.clone();
                        Button::new("reveal-data-dir")
                            .outline()
                            .small()
                            .label(if cfg!(target_os = "macos") {
                                "在访达中显示"
                            } else {
                                "打开所在文件夹"
                            })
                            .disabled(path.is_none())
                            .on_click(move |_, _, cx| {
                                if let Some(p) = &path {
                                    cx.reveal_path(std::path::Path::new(p.as_ref()));
                                }
                            })
                    })
            })),
    )
}

fn about_page() -> SettingPage {
    SettingPage::new("关于")
        .icon(IconName::Info)
        .group(
            SettingGroup::new()
                .title("GetCat")
                .item(SettingItem::render(|_, _, cx| {
                    let muted = cx.theme().muted_foreground;
                    v_flex()
                        .gap_1()
                        .text_sm()
                        .child(format!("版本 {}", env!("CARGO_PKG_VERSION")))
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child("基于 gpui 与 gpui-component 构建的 HTTP 调试工具。"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child("许可证：Apache-2.0"),
                        )
                })),
        )
}

fn theme_key(pref: ThemePref) -> SharedString {
    match pref {
        ThemePref::System => "system".into(),
        ThemePref::Light => "light".into(),
        ThemePref::Dark => "dark".into(),
    }
}
