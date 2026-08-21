//! 持久化句柄（全局）。Store 不可用时所有写入都是 no-op，UI 只显示横幅（spec §9.4 / §11）。

use getcat_core::store::{Store, StoreError};
use gpui::{App, Global};

pub struct StoreHandle {
    store: Option<Store>,
    /// 打开数据目录失败的文案；None 表示可写。（Task 5 的 `banner` 读取后移除 allow。）
    #[allow(dead_code)]
    error: Option<String>,
}

impl Global for StoreHandle {}

/// 安装全局句柄：Ok → 可写；Err → 记录错误文案并以只读模式运行（已读出的数据照常显示）。
/// （目前只有测试调用；Task 5 在 main.rs 启动时调用后移除 allow。）
#[allow(dead_code)]
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
                error: Some(format!("持久化不可用：{err}")),
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
