//! 应用设置（全局）：内存里一份 [`AppSettings`]，改动后同步写 `settings.json`、
//! 重建 HTTP client（请求段改了才重建）、把编辑器字号套到主题上、切换界面语言。

use getcat_core::model::{AppSettings, EDITOR_FONT_SIZE_RANGE};
use gpui::{App, Global, px};
use gpui_component::Theme;

use crate::bridge;
use crate::i18n;
use crate::state::store::store;

pub struct SettingsHandle {
    settings: AppSettings,
}

impl Global for SettingsHandle {}

/// 启动时安装：`loaded` 为 `settings.json` 的内容（没有文件则用默认值），并立刻生效。
pub fn install(cx: &mut App, loaded: Option<AppSettings>) {
    let settings = loaded.unwrap_or_default();
    apply(&settings, cx);
    cx.set_global(SettingsHandle { settings });
}

/// 当前设置；未安装（只有测试会这样）时返回默认值的静态副本。
pub fn settings(cx: &App) -> AppSettings {
    cx.try_global::<SettingsHandle>()
        .map(|h| h.settings.clone())
        .unwrap_or_default()
}

/// 修改设置：`f` 在副本上改；没变化则什么都不做，否则落盘 + 生效。
pub fn update(cx: &mut App, f: impl FnOnce(&mut AppSettings)) {
    let before = settings(cx);
    let mut next = before.clone();
    f(&mut next);
    next.editor_font_size = next.editor_font_size.clamp(
        *EDITOR_FONT_SIZE_RANGE.start(),
        *EDITOR_FONT_SIZE_RANGE.end(),
    );
    if next == before {
        return;
    }
    if next.request != before.request {
        bridge::rebuild_client(cx, &next.request);
    }
    if next.editor_font_size != before.editor_font_size {
        Theme::global_mut(cx).mono_font_size = px(next.editor_font_size as f32);
        cx.refresh_windows();
    }
    if next.language != before.language {
        i18n::apply(next.language, cx);
    }
    if let Some(store) = store(cx) {
        store.write_settings(next.clone());
    }
    cx.set_global(SettingsHandle { settings: next });
}

/// 恢复默认并落盘（设置对话框的"恢复默认"）。
pub fn reset(cx: &mut App) {
    update(cx, |s| *s = AppSettings::default());
}

/// 把一份设置整体套到运行时（启动与恢复默认时用）。
fn apply(settings: &AppSettings, cx: &mut App) {
    bridge::rebuild_client(cx, &settings.request);
    Theme::global_mut(cx).mono_font_size = px(settings.editor_font_size as f32);
    i18n::apply(settings.language, cx);
}
