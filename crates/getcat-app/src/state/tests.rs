//! gpui TestAppContext 测试：Tab 增删激活、发送状态流转、取消与 generation 丢弃、实时耗时。
//!
//! 约定：凡会触发真实 tokio 任务（发请求、写文件）的测试，开头必须 `cx.executor().allow_parking()`，
//! 否则 gpui 测试调度器会因为 tokio 线程唤醒任务而判定"测试不确定"并 panic。
//! gpui 测试时钟是虚拟的：只有 `advance_clock` 会推进，`wait_until` 每轮推进 10 ms 让计时器也能触发。

use std::{
    cell::Cell,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use getcat_core::body::spill::SpillFile;
use getcat_core::body::tier::{EDITOR_MAX_LINES, ViewTier};
use getcat_core::http::{BodyStore, RequestError};
use getcat_core::model::{BodyKind, RawFormat, ResponseMeta};
use gpui::{AppContext, Entity, IntoElement, TestAppContext, VisualTestContext, point, px, size};
use gpui_component::input::InputEvent;

use crate::state::request_tab::{BODY_HINT_BYTES, BodyMode, RequestTab, ResponseSection};
use crate::state::response::{ResponseState, ResponseView};
use crate::state::workspace::Workspace;
use crate::ui::body_view::LINE_HEIGHT_PX;

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

/// B 档端到端（无 GUI）：超过 EDITOR_MAX_LINES 的 text/plain 响应不写编辑器、没有 Pretty 切换，
/// 并且行视图与 Headers 列表都能真正绘制一帧（uniform_list + Scrollbar 的运行时路径）。
#[gpui::test]
fn large_text_body_renders_as_virtual_rows(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);

    let text: String = (0..EDITOR_MAX_LINES + 1)
        .map(|i| format!("line {i}\n"))
        .collect();
    let body = BodyStore::Memory(Arc::from(text.as_bytes()));
    let meta = ResponseMeta {
        status: 200,
        status_text: "OK".into(),
        headers: vec![("content-type".into(), "text/plain".into())],
        duration: Duration::from_millis(1),
        body_len: text.len() as u64,
        content_type: Some("text/plain".into()),
    };
    let view = ResponseView::prepare(meta, &body);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            let g = t.generation;
            t.apply_outcome(g, Ok((body.clone(), view)), window, cx);
        })
    });

    cx.read(|app| {
        let t = tab.read(app);
        let ResponseState::Done { view, .. } = &t.response else {
            panic!("expected Done");
        };
        // text/plain 没有美化文本 → 面板隐藏 Pretty/Raw 切换
        assert!(!view.has_pretty());
        assert!(!view.is_preview());
        let doc = view.doc(true).expect("text body has a doc");
        assert_eq!(doc.tier, ViewTier::Virtual);
        assert_eq!(doc.doc.line_count(), EDITOR_MAX_LINES + 1);
        // B 档不经过只读编辑器：编辑器仍为空，主线程没有搬运过 2 MB 文本
        assert!(
            t.response_editor_for("text")
                .read(app)
                .value()
                .as_ref()
                .is_empty()
        );
    });

    // 真正绘制整个 Tab（Body 页签 → B 档行视图，再切到 Headers 页签 → Headers 列表）。
    // 渲染闭包只对可见区间切片，20 万行必须在瞬间完成（每帧 O(n) 的实现会慢上几个数量级）。
    let started = Instant::now();
    draw_tab(&tab, cx);
    cx.update(|_, cx| {
        tab.update(cx, |t, cx| {
            t.response_section = ResponseSection::Headers;
            cx.notify();
        })
    });
    draw_tab(&tab, cx);
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(10),
        "rendering must be O(visible lines), took {elapsed:?}"
    );

    // uniform_list 真的完成了布局：`contents` 是「行高 × 总行数」，`item` 是视口。
    // 视口远小于内容 → 这一帧只渲染了可见的那几十行。
    cx.read(|app| {
        let t = tab.read(app);
        let body = t
            .body_scroll
            .0
            .borrow()
            .last_item_size
            .expect("body list was laid out");
        assert_eq!(
            body.contents.height,
            px(LINE_HEIGHT_PX * (EDITOR_MAX_LINES + 1) as f32)
        );
        assert!(body.item.height < body.contents.height / 100.);
        let headers = t
            .headers_scroll
            .0
            .borrow()
            .last_item_size
            .expect("headers list was laid out");
        assert_eq!(headers.contents.height, px(24.));
    });
}

fn draw_tab(tab: &Entity<RequestTab>, cx: &mut VisualTestContext) {
    let tab = tab.clone();
    cx.draw(point(px(0.), px(0.)), size(px(1000.), px(800.)), |_, _| {
        tab.into_any_element()
    });
}

pub(crate) fn meta(content_type: &str, body_len: u64) -> ResponseMeta {
    ResponseMeta {
        status: 200,
        status_text: "OK".into(),
        headers: vec![],
        duration: Duration::from_millis(3),
        body_len,
        content_type: Some(content_type.into()),
    }
}

/// 直接把一份准备好的响应灌进 Tab（绕过网络），generation 对齐。
pub(crate) fn install_done(tab: &Entity<RequestTab>, body: BodyStore, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.generation += 1;
            let g = t.generation;
            let view = ResponseView::prepare(meta("application/json", body.len()), &body);
            t.apply_outcome(g, Ok((body, view)), window, cx);
        })
    });
}

#[gpui::test]
fn save_body_writes_memory_body_to_chosen_path(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let cx = init(cx);
    let tab = new_tab(cx);
    set_url_and_send(&tab, &fake_json_server(r#"{"a":1}"#), cx);
    wait_until(cx, |cx| cx.read(|app| tab.read(app).response.is_done()));
    let dest = std::env::temp_dir().join(format!("getcat-save-mem-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&dest);

    cx.update(|window, cx| tab.update(cx, |t, cx| t.save_body(window, cx)));
    assert!(cx.did_prompt_for_new_path());
    let chosen = dest.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    wait_until(cx, |cx| cx.read(|app| tab.read(app).save_notice.is_some()));
    cx.read(|app| {
        let notice = tab.read(app).save_notice.clone().unwrap();
        assert!(notice.starts_with("已保存到"), "{notice}");
    });
    assert_eq!(std::fs::read(&dest).unwrap(), br#"{"a":1}"#);
    let _ = std::fs::remove_file(&dest);
}

#[gpui::test]
fn save_body_copies_spilled_file(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let cx = init(cx);
    let tab = new_tab(cx);
    let (guard, mut file) = SpillFile::create().unwrap();
    std::io::Write::write_all(&mut file, b"0123456789").unwrap();
    drop(file);
    let body = BodyStore::Spilled {
        file: Arc::new(guard),
        len: 10,
        head: Arc::from(&b"0123456789"[..]),
    };
    install_done(&tab, body, cx);
    cx.read(|app| {
        let t = tab.read(app);
        assert!(t.response.is_done());
        let ResponseState::Done { view, .. } = &t.response else {
            unreachable!()
        };
        assert!(view.is_preview());
    });
    let dest = std::env::temp_dir().join(format!("getcat-save-spill-{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&dest);

    cx.update(|window, cx| tab.update(cx, |t, cx| t.save_body(window, cx)));
    let chosen = dest.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    wait_until(cx, |cx| cx.read(|app| tab.read(app).save_notice.is_some()));
    assert_eq!(std::fs::read(&dest).unwrap(), b"0123456789");
    let _ = std::fs::remove_file(&dest);
}

#[gpui::test]
fn cancelled_save_dialog_leaves_no_notice(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    install_done(&tab, BodyStore::Memory(Arc::from(&b"{}"[..])), cx);
    cx.update(|window, cx| tab.update(cx, |t, cx| t.save_body(window, cx)));
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.read(|app| assert!(tab.read(app).save_notice.is_none()));
}

#[gpui::test]
fn save_body_does_nothing_when_not_done(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| tab.update(cx, |t, cx| t.save_body(window, cx)));
    assert!(!cx.did_prompt_for_new_path());
}

#[gpui::test]
fn choose_file_sets_file_body_and_clear_resets_it(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    let path = std::env::temp_dir().join(format!("getcat-choose-{}.json", std::process::id()));
    std::fs::write(&path, b"{}").unwrap();

    cx.update(|window, cx| tab.update(cx, |t, cx| t.choose_file(window, cx)));
    assert!(cx.did_prompt_for_paths());
    let chosen = path.clone();
    cx.simulate_path_prompt_response(move |opts| {
        assert!(opts.files && !opts.directories && !opts.multiple);
        Some(vec![chosen])
    });
    // metadata 在 gpui 后台执行器上读取，跑到空闲即可
    cx.run_until_parked();
    cx.read(|app| {
        let t = tab.read(app);
        assert_eq!(t.body_mode, BodyMode::File);
        assert_eq!(t.file_size, Some(2));
        assert_eq!(
            t.draft(app).body,
            BodyKind::File {
                path: path.clone(),
                content_type: Some("application/json".into()),
            }
        );
    });

    cx.update(|_, cx| tab.update(cx, |t, cx| t.clear_file(cx)));
    cx.read(|app| {
        let t = tab.read(app);
        assert_eq!(t.file_size, None);
        assert_eq!(
            t.draft(app).body,
            BodyKind::File {
                path: PathBuf::new(),
                content_type: None,
            }
        );
    });
    let _ = std::fs::remove_file(&path);
}

#[gpui::test]
fn cancelled_file_dialog_keeps_previous_state(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| tab.update(cx, |t, cx| t.choose_file(window, cx)));
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.read(|app| {
        let t = tab.read(app);
        assert_eq!(t.body_mode, BodyMode::None);
        assert!(t.file_path.is_none());
    });
}

#[gpui::test]
fn oversized_raw_body_shows_file_hint(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    let big = "a".repeat(BODY_HINT_BYTES + 1);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.body_mode = BodyMode::Raw;
            let editor = t.editor_for(RawFormat::Json).clone();
            // set_value 不发 Change 事件（gpui-component 如此设计），这里直接驱动事件处理器模拟一次粘贴
            editor.update(cx, |e, cx| e.set_value(big, window, cx));
            t.on_body_editor_event(&editor, &InputEvent::Change, window, cx);
            assert!(t.body_hint.as_ref().unwrap().contains("10 MB"));
            editor.update(cx, |e, cx| e.set_value("{}", window, cx));
            t.on_body_editor_event(&editor, &InputEvent::Change, window, cx);
            assert!(t.body_hint.is_none());
        })
    });
}

/// P2-3：切换 raw_format / body_mode 必须重新计算提示，否则会残留上一个编辑器的提示，
/// 或者漏掉一个只通过 `set_value`（不发 Change 事件）灌入内容的编辑器。
#[gpui::test]
fn switching_raw_format_or_body_mode_recomputes_hint(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    let big = "a".repeat(BODY_HINT_BYTES + 1);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.body_mode = BodyMode::Raw;
            // JSON 编辑器灌入超大内容，但不经过事件处理器（模拟程序化写入 / 未触发 Change）
            let json_editor = t.editor_for(RawFormat::Json).clone();
            json_editor.update(cx, |e, cx| e.set_value(big, window, cx));
            t.refresh_body_hint(cx);
            assert!(t.body_hint.as_ref().unwrap().contains("10 MB"));

            // 切到 Text 格式：该编辑器是空的，提示应清空
            t.raw_format = RawFormat::Text;
            t.refresh_body_hint(cx);
            assert!(t.body_hint.is_none());

            // 切回 JSON：重新看到超大内容的提示
            t.raw_format = RawFormat::Json;
            t.refresh_body_hint(cx);
            assert!(t.body_hint.as_ref().unwrap().contains("10 MB"));

            // 离开 raw 模式：提示必须清空
            t.body_mode = BodyMode::None;
            t.refresh_body_hint(cx);
            assert!(t.body_hint.is_none());
        })
    });
}
