//! tokio ⇄ gpui 桥接。
use std::path::PathBuf;

use getcat_core::http::{
    self, BodyStore, Client, HttpRequest, HttpResponse, Progress, RequestError,
};
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

/// 在 tokio 上把响应体写到 `dest`：Memory 直接写，Spilled 拷贝临时文件。
/// 返回的 Task 被 drop 即中止写入（目标文件可能残缺，由用户重试）。
pub fn save_body(cx: &App, body: BodyStore, dest: PathBuf) -> Task<anyhow::Result<()>> {
    Tokio::spawn_result(cx, async move {
        match body {
            BodyStore::Memory(bytes) => tokio::fs::write(&dest, &bytes[..]).await?,
            BodyStore::Spilled { file, .. } => {
                tokio::fs::copy(file.path(), &dest).await?;
            }
        }
        Ok(())
    })
}
