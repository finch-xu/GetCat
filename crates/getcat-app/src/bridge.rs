//! tokio ⇄ gpui 桥接。
use getcat_core::http::{self, Client};
use gpui::{App, Global};

/// 全局共享的 reqwest Client；Task 9 的发送流程从这里取用。
#[allow(dead_code)]
pub struct HttpClient(pub Client);
impl Global for HttpClient {}

pub fn init(cx: &mut App) {
    gpui_tokio::init(cx);
    cx.set_global(HttpClient(http::build_client()));
}
