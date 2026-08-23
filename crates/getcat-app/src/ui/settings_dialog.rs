//! 设置对话框：左侧分类（gpui-component `Settings` 自带的侧栏 + 搜索），右侧是具体设置项。
//!
//! 设置值的读写都走 [`crate::state::settings`]（请求段改了重建 HTTP client、字号直接套到主题），
//! 主题偏好仍记在 [`Workspace`]（它属于布局状态，写进 `workspace.json`）。

use getcat_core::model::{EDITOR_FONT_SIZE_RANGE, LanguagePref, ThemePref};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Entity, FontWeight, IntoElement, ParentElement, SharedString, Styled,
    WeakEntity, Window, div, img, px, rems,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    description_list::DescriptionList,
    dialog::DialogFooter,
    h_flex,
    link::Link,
    progress::Progress,
    setting::{
        NumberFieldOptions, SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage,
        Settings,
    },
    v_flex,
};
use gpui_updater::UpdateStatus;

use crate::assets::LOGO_PATH;
use crate::brand;
use crate::i18n::tr;
use crate::state::settings;
use crate::state::store::store;
use crate::state::update::{self, InstallKind};
use crate::state::workspace::Workspace;
use crate::ui::text::{language_label, theme_label};

// 对话框尺寸按像素定：Dialog 的 `w()` 只收 Pixels，内容高度要和它配套，
// 这是设计指南允许的「与外部表面匹配的几何」例外。
const DIALOG_WIDTH: f32 = 760.;
const CONTENT_HEIGHT: f32 = 460.;
/// 「关于」页的 logo 边长：位图资产是 512 px 见方，这里按对话框排版取值（同上，几何例外）。
const LOGO_SIZE: f32 = 56.;

/// 设置对话框的页。判别值必须等于它在 [`PAGE_ORDER`] 里的下标——`default_selected_index`
/// 用 `page as usize` 索引侧栏，错位就会跳到别的页，而编译器看不出来。
/// 目前只有「通用」（默认）与「检查更新」（状态栏更新提示）会被指定打开，其余变体留着对齐页序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SettingsPage {
    General = 0,
    Request = 1,
    Data = 2,
    Updates = 3,
    About = 4,
}

/// 侧栏里页的排列顺序，也是 [`render_settings`] 构建页的唯一依据。
/// 加一页 = 加一个枚举变体（判别值接在末尾）+ 在这里排好位置 + 在 `render_settings`
/// 的 match 里给出构造函数；漏了哪一步都有编译错误或测试失败兜住。
const PAGE_ORDER: [SettingsPage; 5] = [
    SettingsPage::General,
    SettingsPage::Request,
    SettingsPage::Data,
    SettingsPage::Updates,
    SettingsPage::About,
];

/// 打开设置对话框（⌘, / 侧栏齿轮），停在「通用」页。
pub fn open_settings(workspace: Entity<Workspace>, window: &mut Window, cx: &mut App) {
    open_settings_page(workspace, SettingsPage::General, window, cx);
}

/// 打开设置对话框并停在指定页（状态栏的更新提示直接跳到「检查更新」）。已有对话框时不叠加第二个。
pub fn open_settings_page(
    workspace: Entity<Workspace>,
    page: SettingsPage,
    window: &mut Window,
    cx: &mut App,
) {
    if window.has_active_dialog(cx) {
        return;
    }
    let weak = workspace.downgrade();
    window.open_dialog(cx, move |dialog, _, _| {
        let weak = weak.clone();
        dialog
            .title(tr!("settings.title"))
            .w(px(DIALOG_WIDTH))
            .content(move |content, _, cx| {
                content.child(div().w_full().h(px(CONTENT_HEIGHT)).child(render_settings(
                    weak.clone(),
                    page,
                    cx,
                )))
            })
            .footer(
                // 不用 DialogClose 包按钮：它外面套了一层 size_full 的 div，会把按钮拉成整行
                DialogFooter::new()
                    .w_full()
                    .justify_between()
                    .child(
                        Button::new("reset-settings")
                            .ghost()
                            .label(tr!("settings.reset"))
                            .on_click(|_, _, cx| settings::reset(cx)),
                    )
                    .child(
                        Button::new("close-settings")
                            .primary()
                            .label(tr!("common.done"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}

fn render_settings(workspace: WeakEntity<Workspace>, page: SettingsPage, cx: &App) -> Settings {
    // id 按初始页区分：选中页记在按 id 索引的窗口状态里，同一个 id 会复用上一次的选择
    let mut settings = Settings::new(SharedString::from(format!(
        "app-settings-{}",
        page as usize
    )))
    .default_selected_index(SelectIndex {
        page_ix: page as usize,
        group_ix: None,
    })
    .sidebar_width(px(180.));

    // 按 PAGE_ORDER 迭代而不是手写一串 .page(..)：页序只有一个来源，
    // 判别值与下标的对应由 page_discriminants_match_sidebar_order 测试保证
    for entry in PAGE_ORDER {
        settings = settings.page(match entry {
            SettingsPage::General => general_page(workspace.clone()),
            SettingsPage::Request => request_page(),
            SettingsPage::Data => data_page(cx),
            SettingsPage::Updates => updates_page(),
            SettingsPage::About => about_page(),
        });
    }
    settings
}

fn general_page(workspace: WeakEntity<Workspace>) -> SettingPage {
    let theme_for_read = workspace.clone();
    SettingPage::new(tr!("settings.general"))
        .icon(IconName::Settings)
        .group(
            SettingGroup::new()
                .title(tr!("settings.appearance"))
                .item(
                    SettingItem::new(
                        tr!("settings.theme"),
                        SettingField::dropdown(
                            ThemePref::ALL
                                .iter()
                                .map(|p| (theme_key(*p), theme_label(*p)))
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
                    .description(tr!("settings.theme_desc")),
                )
                .item(
                    SettingItem::new(
                        tr!("settings.language"),
                        SettingField::dropdown(
                            LanguagePref::ALL
                                .iter()
                                .map(|p| (language_key(*p), language_label(*p)))
                                .collect(),
                            |cx| language_key(settings::settings(cx).language),
                            |value, cx| {
                                let pref = LanguagePref::ALL
                                    .iter()
                                    .copied()
                                    .find(|p| language_key(*p) == value)
                                    .unwrap_or_default();
                                settings::update(cx, |s| s.language = pref);
                            },
                        )
                        .default_value(language_key(LanguagePref::System)),
                    )
                    .description(tr!("settings.language_desc")),
                )
                .item(
                    SettingItem::new(
                        tr!("settings.editor_font_size"),
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
                    .description(tr!("settings.editor_font_size_desc")),
                ),
        )
}

fn request_page() -> SettingPage {
    SettingPage::new(tr!("settings.request"))
        .icon(IconName::Globe)
        .group(
            SettingGroup::new().title(tr!("settings.timeout")).item(
                SettingItem::new(
                    tr!("settings.timeout_total"),
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
                .description(tr!("settings.timeout_desc")),
            ),
        )
        .group(
            SettingGroup::new()
                .title(tr!("settings.redirects"))
                .item(
                    SettingItem::new(
                        tr!("settings.follow_redirects"),
                        SettingField::switch(
                            |cx| settings::settings(cx).request.follow_redirects,
                            |value, cx| {
                                settings::update(cx, |s| s.request.follow_redirects = value)
                            },
                        )
                        .default_value(true),
                    )
                    .description(tr!("settings.follow_redirects_desc")),
                )
                .item(SettingItem::new(
                    tr!("settings.max_redirects"),
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
            SettingGroup::new().title(tr!("settings.security")).item(
                SettingItem::new(
                    tr!("settings.verify_tls"),
                    SettingField::switch(
                        |cx| settings::settings(cx).request.verify_tls,
                        |value, cx| settings::update(cx, |s| s.request.verify_tls = value),
                    )
                    .default_value(false),
                )
                .description(tr!("settings.verify_tls_desc")),
            ),
        )
}

fn data_page(cx: &App) -> SettingPage {
    let root: Option<SharedString> = store(cx).map(|s| s.root().display().to_string().into());
    let path_for_reveal = root.clone();
    SettingPage::new(tr!("settings.data"))
        .icon(IconName::HardDrive)
        .group(
            SettingGroup::new()
                .title(tr!("settings.storage"))
                .description(tr!("settings.storage_desc"))
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
                                        .child(tr!("settings.data_dir")),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .font_family(mono)
                                        .truncate()
                                        .child(root.clone().unwrap_or_else(|| {
                                            tr!("settings.data_dir_unavailable")
                                        })),
                                ),
                        )
                        .child({
                            let path = path_for_reveal.clone();
                            Button::new("reveal-data-dir")
                                .outline()
                                .small()
                                .label(if cfg!(target_os = "macos") {
                                    tr!("settings.reveal_finder")
                                } else {
                                    tr!("settings.reveal_folder")
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

/// 「检查更新」页：启动时自动检查的开关 + 当前状态与动作。
///
/// group 不给标题：页名已经是「检查更新」，再套一层「更新」只是重复。
fn updates_page() -> SettingPage {
    SettingPage::new(tr!("settings.updates_page"))
        .icon(IconName::ArrowDown)
        .group(
            SettingGroup::new()
                .item(
                    SettingItem::new(
                        tr!("settings.check_on_launch"),
                        SettingField::switch(
                            |cx| settings::settings(cx).check_updates_on_launch,
                            |value, cx| settings::update(cx, |s| s.check_updates_on_launch = value),
                        )
                        .default_value(true),
                    )
                    .description(tr!("settings.check_on_launch_desc")),
                )
                .item(SettingItem::render(|_, _, cx| render_update_row(cx))),
        )
}

fn about_page() -> SettingPage {
    SettingPage::new(tr!("settings.about"))
        .icon(IconName::Info)
        .group(
            SettingGroup::new()
                .title(brand::APP_NAME)
                .item(SettingItem::render(|_, _, cx| {
                    let muted = cx.theme().muted_foreground;
                    v_flex()
                        .gap_4()
                        .text_sm()
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                // 位图 logo 走 img()：svg() 是单色蒙版，多色 logo 用不了
                                .child(img(LOGO_PATH).size(px(LOGO_SIZE)).flex_none())
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_base()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(brand::APP_NAME),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .child(tr!("settings.about_blurb")),
                                        ),
                                ),
                        )
                        .child(
                            DescriptionList::new()
                                .columns(1)
                                .bordered(false)
                                .small()
                                .label_width(rems(4.))
                                .item(tr!("settings.version"), env!("CARGO_PKG_VERSION"), 1)
                                .item(tr!("settings.author"), brand::AUTHOR, 1)
                                .item(tr!("settings.license"), brand::LICENSE, 1),
                        )
                        .child(repo_link())
                })),
        )
}

/// 项目主页链接：和发布页链接同一个形状（`Link` + 外链图标），只是换了图标与地址。
fn repo_link() -> Link {
    Link::new("repo-link")
        .href(brand::REPO_URL)
        .text_sm()
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(Icon::new(IconName::Github).size_3p5())
                .child(tr!("settings.repository")),
        )
}

/// 「更新」组的状态行：左侧一句话状态，右侧按当前状态给出动作；下载中在下方画进度条。
fn render_update_row(cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let danger = cx.theme().danger;

    if !update::supported(cx) {
        return h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(tr!("settings.update_unsupported")),
            )
            .child(releases_button("open-releases"))
            .into_any_element();
    }

    let status = update::status(cx);
    let kind = update::install_kind_of(cx);
    let line = update::status_line(&status, env!("CARGO_PKG_VERSION"));
    let is_error = matches!(status, UpdateStatus::Errored(_));

    let mut actions = h_flex().gap_2().items_center().flex_shrink_0();
    match &status {
        UpdateStatus::Idle | UpdateStatus::UpToDate => actions = actions.child(check_button(false)),
        UpdateStatus::Checking => actions = actions.child(check_button(true)),
        UpdateStatus::Available(_) => {
            let install = Button::new("install-update")
                .small()
                .label(tr!("settings.install_update"));
            actions = actions
                .child(if kind == InstallKind::Installed {
                    install
                        .primary()
                        .on_click(|_, _, cx| update::download_and_install(cx))
                } else {
                    install.outline().disabled(true)
                })
                .child(releases_button("open-releases"));
        }
        UpdateStatus::Downloading { .. } | UpdateStatus::Installing => {}
        UpdateStatus::Staged(_) => {
            actions = actions.child(
                Button::new("restart-update")
                    .primary()
                    .small()
                    .label(tr!("settings.restart"))
                    .on_click(|_, _, cx| update::restart(cx)),
            );
        }
        UpdateStatus::Errored(_) => {
            actions = actions
                .child(
                    Button::new("retry-update")
                        .outline()
                        .small()
                        .label(tr!("settings.retry"))
                        .on_click(|_, _, cx| update::check(cx)),
                )
                .child(releases_button("open-releases"));
        }
    }

    let note = match (&status, kind) {
        (UpdateStatus::Available(_), InstallKind::DevBuild) => {
            Some(tr!("settings.update_dev_build"))
        }
        (UpdateStatus::Available(_), InstallKind::Translocated) => {
            Some(tr!("settings.update_translocated"))
        }
        _ => None,
    };
    let percent = update::progress_percent(&status);
    let indeterminate = matches!(
        status,
        UpdateStatus::Downloading { total: None, .. } | UpdateStatus::Installing
    );

    v_flex()
        .w_full()
        .gap_2()
        .child(
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
                                .when(is_error, |d| d.text_color(danger))
                                .child(line),
                        )
                        .when_some(note, |v, note| {
                            v.child(div().text_xs().text_color(muted).child(note))
                        }),
                )
                .child(actions),
        )
        .when_some(percent, |v, pct| {
            v.child(Progress::new("update-progress").value(pct))
        })
        .when(indeterminate, |v| {
            v.child(Progress::new("update-progress").loading(true))
        })
        .into_any_element()
}

fn check_button(checking: bool) -> Button {
    Button::new("check-updates")
        .outline()
        .small()
        .label(tr!("settings.check_updates"))
        .loading(checking)
        .on_click(|_, _, cx| update::check(cx))
}

/// 发布页是外部网页：按设计指南用 `Link`（手型光标、下划线），不是 Button。
fn releases_button(id: &'static str) -> Link {
    Link::new(id).href(update::RELEASES_URL).text_sm().child(
        h_flex()
            .gap_1()
            .items_center()
            .child(tr!("settings.releases"))
            .child(Icon::new(IconName::ExternalLink).size_3p5()),
    )
}

fn language_key(pref: LanguagePref) -> SharedString {
    match pref {
        LanguagePref::System => "system".into(),
        LanguagePref::English => "en".into(),
        LanguagePref::Chinese => "zh-CN".into(),
    }
}

fn theme_key(pref: ThemePref) -> SharedString {
    match pref {
        ThemePref::System => "system".into(),
        ThemePref::Light => "light".into(),
        ThemePref::Dark => "dark".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `default_selected_index` 拿 `page as usize` 当侧栏下标，而侧栏顺序来自 `PAGE_ORDER`。
    /// 两者一旦错位，「跳到检查更新」会打开别的页，且没有任何编译期提示——所以在这里钉住。
    #[test]
    fn page_discriminants_match_sidebar_order() {
        for (ix, page) in PAGE_ORDER.iter().enumerate() {
            assert_eq!(
                *page as usize, ix,
                "{page:?} 的判别值与它在 PAGE_ORDER 里的位置对不上"
            );
        }
    }

    /// 状态栏的更新提示跳的是「检查更新」页；这一条同时确认该页确实还在 PAGE_ORDER 里。
    #[test]
    fn updates_page_is_reachable_from_the_status_bar_hint() {
        assert!(PAGE_ORDER.contains(&SettingsPage::Updates));
        assert!(PAGE_ORDER.contains(&SettingsPage::About));
    }
}
