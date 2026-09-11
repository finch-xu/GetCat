//! tokio ⇄ gpui 桥接：把 tokio 任务包装成 gpui 的 `Task`，drop `Task` 即 abort 底层 tokio 任务。
//!
//! 移植自 zed 仓库的 `crates/gpui_tokio`（Apache-2.0，见同目录 LICENSE-APACHE）。
//! GPUI Kit 0.6 起 gpui 以 crates.io 上的 `gpui-pre` 系列发布，其中没有 tokio 桥接这一包
//! （Kit 全线用 smol），而 zed git 源的 `gpui_tokio` 引用的又是另一份 gpui，类型不通，
//! 所以在仓库内保留这份 100 行的移植。与上游的差异：
//! - `gpui::` 路径改为 `gpui_kit::`；
//! - 上游用的 `gpui_util::defer` 在此处内联为 [`Defer`]（crates.io 上没有对应包）。

use std::future::Future;

use gpui_kit::{App, AppContext, Global, ReadGlobal, Task};

pub use tokio::task::JoinError;

/// drop 时执行闭包；用来在 gpui `Task` 被丢弃时 abort 对应的 tokio 任务。
struct Defer<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for Defer<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

fn defer<F: FnOnce()>(f: F) -> Defer<F> {
    Defer(Some(f))
}

/// 用 2 个工作线程新建一个 tokio runtime 并注册为 gpui 全局。
///
/// 需要更多线程（或要在 gpui 之外使用 runtime）时，自己建 runtime 再调 [`init_from_handle`]。
pub fn init(cx: &mut App) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        // 已经有 gpui 自己的执行器了，tokio 这边保持小脚印
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("Failed to initialize Tokio");

    let handle = runtime.handle().clone();
    cx.set_global(GlobalTokio {
        owned_runtime: Some(runtime),
        handle,
    });
}

/// 用现成的 tokio runtime handle 注册全局。
pub fn init_from_handle(cx: &mut App, handle: tokio::runtime::Handle) {
    cx.set_global(GlobalTokio {
        owned_runtime: None,
        handle,
    });
}

struct GlobalTokio {
    owned_runtime: Option<tokio::runtime::Runtime>,
    handle: tokio::runtime::Handle,
}

impl Global for GlobalTokio {}

impl Drop for GlobalTokio {
    fn drop(&mut self) {
        if let Some(runtime) = self.owned_runtime.take() {
            runtime.shutdown_background();
        }
    }
}

pub struct Tokio {}

impl Tokio {
    /// 把 future 投到 tokio 线程池，以 gpui `Task` 的形式返回；`Task` 被 drop 时 tokio 任务随之取消。
    pub fn spawn<C, Fut, R>(cx: &C, f: Fut) -> Task<Result<R, JoinError>>
    where
        C: AppContext,
        Fut: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        cx.read_global(|tokio: &GlobalTokio, cx| {
            let join_handle = tokio.handle.spawn(f);
            let abort_handle = join_handle.abort_handle();
            let cancel = defer(move || {
                abort_handle.abort();
            });
            cx.background_spawn(async move {
                let result = join_handle.await;
                drop(cancel);
                result
            })
        })
    }

    /// 同 [`Tokio::spawn`]，但 future 本身返回 `anyhow::Result`，JoinError 也折叠进同一个 Result。
    pub fn spawn_result<C, Fut, R>(cx: &C, f: Fut) -> Task<anyhow::Result<R>>
    where
        C: AppContext,
        Fut: Future<Output = anyhow::Result<R>> + Send + 'static,
        R: Send + 'static,
    {
        cx.read_global(|tokio: &GlobalTokio, cx| {
            let join_handle = tokio.handle.spawn(f);
            let abort_handle = join_handle.abort_handle();
            let cancel = defer(move || {
                abort_handle.abort();
            });
            cx.background_spawn(async move {
                let result = join_handle.await?;
                drop(cancel);
                result
            })
        })
    }

    pub fn handle(cx: &App) -> tokio::runtime::Handle {
        GlobalTokio::global(cx).handle.clone()
    }
}
