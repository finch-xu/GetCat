//! gpui TestAppContext 测试：Tab 增删激活、发送状态流转、取消与 generation 丢弃、实时耗时。
//!
//! 约定：凡会触发真实 tokio 任务（发请求、写文件）的测试，开头必须 `cx.executor().allow_parking()`，
//! 否则 gpui 测试调度器会因为 tokio 线程唤醒任务而判定"测试不确定"并 panic。
//! gpui 测试时钟是虚拟的：只有 `advance_clock` 会推进，`wait_until` 每轮推进 10 ms 让计时器也能触发。

use std::{
    cell::Cell,
    io::{Read, Write},
    net::TcpListener,
    rc::Rc,
    time::Duration,
};

use getcat_core::http::RequestError;
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};

use crate::state::request_tab::RequestTab;
use crate::state::response::ResponseState;
use crate::state::workspace::Workspace;

pub(crate) fn init(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::bridge::init(cx);
    });
    cx.add_empty_window()
}

pub(crate) fn new_tab(cx: &mut VisualTestContext) -> Entity<RequestTab> {
    cx.update(|window, cx| cx.new(|cx| RequestTab::new(1, window, cx)))
}

pub(crate) fn set_url_and_send(tab: &Entity<RequestTab>, url: &str, cx: &mut VisualTestContext) {
    let url = url.to_string();
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.url.update(cx, |u, cx| u.set_value(url, window, cx));
            t.send(window, cx);
        })
    });
}

/// 轮询直到条件成立（最多 5 s）：每轮先把 gpui 调度器跑到空闲，推进虚拟时钟 10 ms，再让出真实时间给 tokio 线程。
pub(crate) fn wait_until(
    cx: &mut VisualTestContext,
    mut pred: impl FnMut(&mut VisualTestContext) -> bool,
) {
    for _ in 0..500 {
        cx.run_until_parked();
        if pred(cx) {
            return;
        }
        cx.executor().advance_clock(Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("condition not met within 5 s");
}

/// 只回一次固定响应的本地 HTTP 服务（std 线程，不依赖 tokio）。
pub(crate) fn fake_json_server(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let mut got = Vec::new();
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                if got.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/")
}

/// 接受连接但永不回应，直到客户端断开。
pub(crate) fn hanging_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
            }
        }
    });
    format!("http://{addr}/")
}

pub(crate) fn refused_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/")
}

#[gpui::test]
fn workspace_tabs_add_close_activate(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.tab_count(), 1);
            assert_eq!(ws.active_tab().read(cx).id, 1);
            ws.new_tab(window, cx);
            ws.new_tab(window, cx);
            assert_eq!((ws.tab_count(), ws.active_index()), (3, 2));
            ws.activate(0, cx);
            assert_eq!(ws.active_index(), 0);
            ws.close_tab(0, window, cx);
            assert_eq!((ws.tab_count(), ws.active_index()), (2, 0));
            assert_eq!(ws.active_tab().read(cx).id, 2);
            ws.close_tab(1, window, cx);
            ws.close_tab(0, window, cx);
            // 关掉最后一个 Tab 会自动新建一个空 Tab
            assert_eq!((ws.tab_count(), ws.active_index()), (1, 0));
            assert_eq!(ws.active_tab().read(cx).id, 4);
        });
    });
}

#[gpui::test]
fn send_to_refused_port_ends_in_failed(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let cx = init(cx);
    let tab = new_tab(cx);
    set_url_and_send(&tab, &refused_url(), cx);
    cx.read(|app| assert!(tab.read(app).response.is_in_flight()));
    wait_until(cx, |cx| {
        cx.read(|app| !tab.read(app).response.is_in_flight())
    });
    cx.read(|app| {
        assert!(
            matches!(
                tab.read(app).response.error(),
                Some(RequestError::ConnectionRefused(_))
            ),
            "{:?}",
            tab.read(app).response.error()
        );
    });
}

#[gpui::test]
fn send_to_json_server_ends_in_done(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let cx = init(cx);
    let tab = new_tab(cx);
    set_url_and_send(&tab, &fake_json_server(r#"{"a":1}"#), cx);
    wait_until(cx, |cx| {
        cx.read(|app| !tab.read(app).response.is_in_flight())
    });
    cx.read(|app| {
        let tab = tab.read(app);
        assert!(tab.response.is_done(), "{:?}", tab.response.error());
        // 响应编辑器已被写入美化后的文本
        assert_eq!(
            tab.response_editor_for("json").read(app).value().as_ref(),
            "{\n  \"a\": 1\n}"
        );
    });
}

#[gpui::test]
fn cancel_marks_cancelled_and_bumps_generation(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let cx = init(cx);
    let tab = new_tab(cx);
    set_url_and_send(&tab, &hanging_server(), cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            assert!(t.response.is_in_flight());
            let g = t.generation;
            // 在途时再次 send 不重发（Plan 1 Ruling 15）
            t.send(window, cx);
            assert_eq!(t.generation, g);
            t.cancel(cx);
            assert_eq!(t.generation, g + 1);
            assert!(matches!(t.response.error(), Some(RequestError::Cancelled)));
            // 未在途时再次取消不改变任何状态
            t.cancel(cx);
            assert_eq!(t.generation, g + 1);
        })
    });
    // 被 drop 的任务完成清理后，状态不得被旧任务改写
    std::thread::sleep(Duration::from_millis(50));
    cx.run_until_parked();
    cx.read(|app| {
        assert!(matches!(
            tab.read(app).response.error(),
            Some(RequestError::Cancelled)
        ))
    });
}

#[gpui::test]
fn stale_generation_outcome_is_discarded(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.generation = 5;
            t.apply_outcome(4, Err(RequestError::Other("stale".into())), window, cx);
            assert!(matches!(t.response, ResponseState::Idle));
            t.apply_outcome(5, Err(RequestError::Timeout), window, cx);
            assert!(matches!(t.response.error(), Some(RequestError::Timeout)));
        })
    });
}

#[gpui::test]
fn elapsed_ticker_notifies_while_in_flight_and_stops_after_cancel(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let cx = init(cx);
    let tab = new_tab(cx);
    let ticks = Rc::new(Cell::new(0usize));
    let counter = ticks.clone();
    let _sub = cx.update(|_, cx| cx.observe(&tab, move |_, _| counter.set(counter.get() + 1)));
    set_url_and_send(&tab, &hanging_server(), cx);
    cx.run_until_parked();
    let baseline = ticks.get();
    // wait_until 每轮推进虚拟时钟 10 ms；100 ms 的计时器应在 ≤ 1 s 虚拟时间内至少触发 3 次
    wait_until(cx, |_| ticks.get() >= baseline + 3);
    cx.update(|_, cx| tab.update(cx, |t, cx| t.cancel(cx)));
    cx.run_until_parked();
    let after_cancel = ticks.get();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        ticks.get(),
        after_cancel,
        "ticker must stop once the request is cancelled"
    );
}
