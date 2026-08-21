//! tokio ⇄ gpui 桥接。
use std::path::PathBuf;

use getcat_core::http::{
    self, BodyStore, Client, HttpRequest, HttpResponse, Progress, RequestError,
};
use getcat_core::store::{copy_atomic, write_atomic};
use gpui::{App, Global, Task};
use gpui_tokio::Tokio;
use tokio::sync::mpsc;

/// 全局共享的 reqwest Client；发送流程从这里取用。
pub struct HttpClient(pub Client);
impl Global for HttpClient {}

pub fn init(cx: &mut App) {
    gpui_tokio::init(cx);
    cx.set_global(HttpClient(http::build_client()));
}

/// 在 tokio runtime 上执行请求；返回的 gpui Task 被 drop 时底层 tokio 任务自动 abort。
pub fn send(
    cx: &App,
    req: HttpRequest,
    progress: mpsc::Sender<Progress>,
) -> Task<anyhow::Result<Result<HttpResponse, RequestError>>> {
    let client = cx.global::<HttpClient>().0.clone();
    Tokio::spawn_result(cx, async move {
        Ok(http::execute(&client, req, Some(progress)).await)
    })
}

/// 在 tokio 的阻塞线程池上把响应体**原子**写到 `dest`：Memory 走 `write_atomic`，Spilled 走 `copy_atomic`
/// （同目录临时文件 → fsync → rename）。中途失败不会留下半个目标文件；
/// drop 返回的 Task 只是不再等待结果（阻塞任务本身无法打断），目标路径仍然要么完整要么不变。
pub fn save_body(cx: &App, body: BodyStore, dest: PathBuf) -> Task<anyhow::Result<()>> {
    Tokio::spawn_result(cx, async move {
        tokio::task::spawn_blocking(move || match &body {
            BodyStore::Memory(bytes) => write_atomic(&dest, bytes),
            // `body` 持有 Arc<SpillFile>，拷贝期间临时文件不会被删除
            BodyStore::Spilled { file, .. } => copy_atomic(file.path(), &dest),
        })
        .await??;
        Ok(())
    })
}
