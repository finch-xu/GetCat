//! 持久化句柄（全局）。Store 不可用时所有写入都是 no-op，UI 只显示横幅（spec §9.4 / §11）。

use getcat_core::store::{Store, StoreError};
use gpui::{App, Entity, Global};

use crate::i18n::tr;
use crate::state::workspace::Workspace;

pub struct StoreHandle {
    store: Option<Store>,
    /// 打开数据目录失败的原因；None 表示可写。存错误本身，横幅文案在渲染时按当前语言翻译
    /// （`store::install` 先于 `settings::install` 运行，此时 locale 还没定）。
    error: Option<StoreError>,
}

impl Global for StoreHandle {}

/// 安装全局句柄：Ok → 可写；Err → 记录错误文案并以只读模式运行（已读出的数据照常显示）。
pub fn install(cx: &mut App, opened: Result<Store, StoreError>) {
    let handle = match opened {
        Ok(store) => StoreHandle {
            store: Some(store),
            error: None,
        },
        Err(err) => {
            tracing::warn!("persistence unavailable: {err}");
            StoreHandle {
                store: None,
                error: Some(err),
            }
        }
    };
    cx.set_global(handle);
}

/// 可用的 Store；未安装或不可写时为 None（调用方据此跳过写入）。
pub fn store(cx: &App) -> Option<&Store> {
    cx.try_global::<StoreHandle>()
        .and_then(|handle| handle.store.as_ref())
}

/// 顶部横幅文案：打开数据目录失败，或写入线程最近一次失败（下一次成功后自动消失）。O(1)，可在渲染路径调用。
pub fn banner(cx: &App) -> Option<String> {
    let handle = cx.try_global::<StoreHandle>()?;
    let unavailable = handle
        .error
        .as_ref()
        .map(|e| tr!("store.unavailable", error = e).to_string());
    unavailable.or_else(|| {
        handle
            .store
            .as_ref()
            .and_then(|s| s.last_error())
            .map(|e| tr!("store.write_failed", error = e).to_string())
    })
}

/// 退出 / 关窗前：每个 Tab 立即投递草稿快照（跳过去抖），再等写入线程清空队列（≤ 2 s）。
pub fn flush_on_exit(workspace: &Entity<Workspace>, cx: &mut App) {
    workspace.update(cx, |ws, cx| ws.flush_drafts(cx));
    if let Some(store) = store(cx)
        && !store.flush()
    {
        tracing::warn!("store flush timed out; the last edits may be lost");
    }
}
