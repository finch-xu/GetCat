//! tokio ⇄ gpui 桥接。
use std::path::PathBuf;

use getcat_core::http::{
    self, BodyStore, HttpClients, HttpRequest, HttpResponse, RequestError, StreamEvent,
};
use getcat_core::model::{HttpVersionPref, RequestSettings};
use getcat_core::store::{copy_atomic_user, write_atomic_user};
use gpui::{App, Global, Task};
use gpui_tokio::Tokio;
use tokio::sync::mpsc;

/// 全局共享的 reqwest Client 组；发送流程按 Tab 选的 HTTP 版本从这里取用。
pub struct HttpClient(pub HttpClients);
impl Global for HttpClient {}

pub fn init(cx: &mut App) {
    gpui_tokio::init(cx);
    cx.set_global(HttpClient(http::build_clients()));
}

/// 按请求设置重建全局 client（设置改动后调用）。正在进行的请求继续用旧 client 直到结束。
pub fn rebuild_client(cx: &mut App, settings: &RequestSettings) {
    cx.set_global(HttpClient(http::build_clients_with(settings)));
}

/// 在 tokio runtime 上执行请求；返回的 gpui Task 被 drop 时底层 tokio 任务自动 abort。
pub fn send(
    cx: &App,
    req: HttpRequest,
    version: HttpVersionPref,
    progress: mpsc::Sender<StreamEvent>,
) -> Task<anyhow::Result<Result<HttpResponse, RequestError>>> {
    let client = cx.global::<HttpClient>().0.get(version).clone();
    Tokio::spawn_result(cx, async move {
        Ok(http::execute(&client, req, Some(progress)).await)
    })
}

/// 在 tokio 的阻塞线程池上把响应体**原子**写到 `dest`：Memory 走 `write_atomic_user`，
/// Spilled 走 `copy_atomic_user`（同目录临时文件 → fsync → rename）。中途失败不会留下半个目标文件；
/// drop 返回的 Task 只是不再等待结果（阻塞任务本身无法打断），目标路径仍然要么完整要么不变。
/// 用 `*_user` 变体：用户显式选择的"另存为"目标按系统 umask 创建（通常 0644），
/// 不继承数据目录内部文件的 0600（见 Ruling P4-3）。
pub fn save_body(cx: &App, body: BodyStore, dest: PathBuf) -> Task<anyhow::Result<()>> {
    Tokio::spawn_result(cx, async move {
        tokio::task::spawn_blocking(move || match &body {
            BodyStore::Memory(bytes) => write_atomic_user(&dest, bytes),
            // `body` 持有 Arc<SpillFile>，拷贝期间临时文件不会被删除
            BodyStore::Spilled { file, .. } => copy_atomic_user(file.path(), &dest),
        })
        .await??;
        Ok(())
    })
}
