//! 数据目录设置：当前目录与下次启动目录分别展示，所有磁盘操作交给后台状态模块。

use crate::{
    i18n::tr,
    state::data_directory::{self, Notice},
};
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    setting::{SettingGroup, SettingItem, SettingPage},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled, div};
use std::path::Path;

pub fn page() -> SettingPage {
    SettingPage::new(tr!("settings.data"))
        .icon(IconName::HardDrive)
        .group(
            SettingGroup::new()
                .title(tr!("settings.storage"))
                .description(tr!("settings.storage_desc"))
                .item(SettingItem::render(|_, _, cx| render(cx))),
        )
}

fn path_row(label: SharedString, path: Option<&Path>, cx: &App) -> AnyElement {
    let id = SharedString::from(format!("reveal-{label}"));
    let path = path.map(|p| p.display().to_string());
    let path_for_reveal = path.clone();
    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .justify_between()
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().child(label))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .font_family(cx.theme().mono_font_family.clone())
                        .truncate()
                        .child(
                            path.unwrap_or_else(|| {
                                tr!("settings.data_dir_unavailable").to_string()
                            }),
                        ),
                ),
        )
        .child(
            Button::new(id)
                .outline()
                .small()
                .label(if cfg!(target_os = "macos") {
                    tr!("settings.reveal_finder")
                } else {
                    tr!("settings.reveal_folder")
                })
                .disabled(path_for_reveal.is_none())
                .on_click(move |_, _, cx| {
                    if let Some(path) = &path_for_reveal {
                        cx.reveal_path(Path::new(path));
                    }
                }),
        )
        .into_any_element()
}

fn render(cx: &App) -> AnyElement {
    let Some(state) = data_directory::state(cx) else {
        return div()
            .text_sm()
            .child(tr!("settings.data_dir_unavailable"))
            .into_any_element();
    };
    let disabled = state.busy || state.location.is_none();
    let mut body = v_flex().w_full().gap_3().child(path_row(
        tr!("settings.data_dir"),
        state.active.as_deref(),
        cx,
    ));
    if state.pending() {
        body = body.child(path_row(
            tr!("settings.next_data_dir"),
            state.selected.as_deref(),
            cx,
        ));
    }
    body = body
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("choose-data-directory")
                        .outline()
                        .small()
                        .label(tr!("settings.choose_data_dir"))
                        .disabled(disabled)
                        .on_click(|_, _, cx| data_directory::choose(cx)),
                )
                .child(
                    Button::new("reset-data-directory")
                        .ghost()
                        .small()
                        .label(tr!("settings.reset_data_dir"))
                        .disabled(disabled)
                        .on_click(|_, _, cx| data_directory::reset(cx)),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr!("settings.data_dir_restart")),
        );
    if let Some(location) = &state.location {
        body = body.child(path_row(
            tr!("settings.system_data_dir"),
            Some(&location.system_root),
            cx,
        ));
    }
    body = body
        .child(
            Button::new("migrate-system-data")
                .outline()
                .small()
                .label(tr!("settings.migrate_data_dir"))
                .disabled(
                    disabled
                        || !state.pending()
                        || state.selected.is_none()
                        || state.migration_error.is_some(),
                )
                .on_click(|_, _, cx| data_directory::migrate(cx)),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr!("settings.data_dir_migration_desc")),
        );
    if !state.pending() && state.migration_error.is_none() {
        body = body.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr!("settings.data_dir_select_first")),
        );
    }
    if let Some(error) = &state.migration_error {
        body = body.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(data_directory::error_line(error)),
        );
    }
    if state.busy {
        body = body.child(div().text_sm().child(tr!("settings.data_dir_busy")));
    } else if let Some(notice) = &state.notice {
        body = body.child(
            div()
                .text_sm()
                .when(matches!(notice, Notice::Failed(_)), |el| {
                    el.text_color(cx.theme().danger)
                })
                .child(data_directory::notice_line(notice)),
        );
    }
    body.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::data_directory::DirectoryState;
    use getcat_core::store::location::DataLocation;
    use gpui_kit::component::{Root, setting::Settings};
    use gpui_kit::{AppContext, Context, Render, TestAppContext, Window};

    struct Panel;
    impl Render for Panel {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Settings::new("data-directory-test").page(page())
        }
    }

    #[gpui_kit::test]
    fn settings_page_renders_current_and_pending_paths_across_locales(cx: &mut TestAppContext) {
        let _locale = crate::i18n::locale_test_lock();
        let previous_locale = rust_i18n::locale().to_string();
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let location = DataLocation::new(dir.path().join("system"), None);
        let (state, opened, _) = DirectoryState::load(location.clone());
        let store = opened.unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::bridge::init(cx);
        });
        cx.update(|cx| {
            crate::state::store::install(cx, Ok(store));
            cx.set_global(state);
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(|_| Panel);
            Root::new(panel, window, cx)
        });
        for locale in ["en", "zh-CN", "zh-TW", "ja", "fr"] {
            rust_i18n::set_locale(locale);
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            cx.update(|_, cx| data_directory::choose(cx));
            let target = dir.path().join(format!("selected-{locale}"));
            std::fs::create_dir(&target).unwrap();
            cx.simulate_path_prompt_response(move |_| Some(vec![target]));
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
                assert!(cx.global::<DirectoryState>().pending());
            });
        }
        rust_i18n::set_locale(&previous_locale);
    }
}
