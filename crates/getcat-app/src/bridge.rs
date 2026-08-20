//! tokio ⇄ gpui 桥接。
use getcat_core::http::{self, Client, HttpRequest, HttpResponse, Progress, RequestError};
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
