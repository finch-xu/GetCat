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
use getcat_core::model::{
    BodyKind, KeyValue, Method, RawFormat, RequestDraft, ResponseMeta, SavedRequest, TabDraft,
    TabId, ThemePref, Ulid, WorkspaceState,
};
use getcat_core::store::{Store, codec::decode};
use gpui::{AppContext, Entity, IntoElement, TestAppContext, VisualTestContext, point, px, size};
use gpui_component::{ActiveTheme, input::InputEvent};
use tempfile::TempDir;

use crate::state::request_tab::{
    BODY_HINT_BYTES, BodyMode, DRAFT_DEBOUNCE, RequestTab, ResponseSection,
};
use crate::state::response::{ResponseState, ResponseView};
use crate::state::store;
use crate::state::workspace::Workspace;
use crate::ui::body_view::LINE_HEIGHT_PX;
use crate::ui::kv_table::KvTable;
use crate::ui::sidebar::SAVED_ROW_HEIGHT;

pub(crate) fn init(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::bridge::init(cx);
    });
    cx.add_empty_window()
}

pub(crate) fn new_tab(cx: &mut VisualTestContext) -> Entity<RequestTab> {
    cx.update(|window, cx| cx.new(|cx| RequestTab::new(Ulid::generate(), window, cx)))
}

/// 带独立临时数据目录的基座：写入线程是独立 std 线程、`flush` 会阻塞测试线程，
/// 必须 `allow_parking`；合并窗口取 0，让 `flush` 后立刻能读到文件。
pub(crate) fn init_with_store(cx: &mut TestAppContext) -> (&mut VisualTestContext, Store, TempDir) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_with_delay(dir.path().to_path_buf(), Duration::ZERO).unwrap();
    cx.update(|cx| store::install(cx, Ok(store.clone())));
    (init(cx), store, dir)
}

/// 模拟用户在 URL 栏键入：`set_value` 不发 Change 事件，所以再直接驱动一次事件处理器。
pub(crate) fn change_url(tab: &Entity<RequestTab>, url: &str, cx: &mut VisualTestContext) {
    let url = url.to_string();
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            let input = t.url.clone();
            input.update(cx, |u, cx| u.set_value(url, window, cx));
            t.on_url_event(&input, &InputEvent::Change, window, cx);
        })
    });
}

pub(crate) fn read_draft(store: &Store, id: TabId) -> Option<TabDraft> {
    let bytes = std::fs::read(store.layout().draft_path(id)).ok()?;
    decode(&bytes).ok()
}

pub(crate) fn read_workspace(store: &Store) -> Option<WorkspaceState> {
    let bytes = std::fs::read(store.layout().workspace_path()).ok()?;
    decode(&bytes).ok()
}

pub(crate) fn read_request(store: &Store, id: Ulid) -> Option<SavedRequest> {
    let bytes = std::fs::read(store.layout().request_path(id)).ok()?;
    decode(&bytes).ok()
}

/// requests/ 目录下 .json 文件数。
pub(crate) fn request_files(store: &Store) -> usize {
    std::fs::read_dir(store.layout().requests_dir())
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
                .count()
        })
        .unwrap_or(0)
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
            let first = ws.active_tab();
            ws.new_tab(window, cx);
            let second = ws.active_tab();
            ws.new_tab(window, cx);
            assert_eq!((ws.tab_count(), ws.active_index()), (3, 2));
            ws.activate(0, cx);
            assert_eq!(ws.active_index(), 0);
            assert_eq!(ws.active_tab(), first);
            ws.close_tab(0, window, cx);
            assert_eq!((ws.tab_count(), ws.active_index()), (2, 0));
            assert_eq!(ws.active_tab(), second);
            ws.close_tab(1, window, cx);
            ws.close_tab(0, window, cx);
            // 关掉最后一个 Tab 会自动新建一个空 Tab
            assert_eq!((ws.tab_count(), ws.active_index()), (1, 0));
            assert_ne!(ws.active_tab(), second);
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
    let body = BodyStore::in_memory(text.as_bytes().to_vec());
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
    wait_until(cx, |cx| cx.read(|app| tab.read(app).notice.is_some()));
    cx.read(|app| {
        let notice = tab.read(app).notice.clone().unwrap();
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
    wait_until(cx, |cx| cx.read(|app| tab.read(app).notice.is_some()));
    assert_eq!(std::fs::read(&dest).unwrap(), b"0123456789");
    let _ = std::fs::remove_file(&dest);
}

#[gpui::test]
fn cancelled_save_dialog_leaves_no_notice(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    install_done(&tab, BodyStore::in_memory(&b"{}"[..]), cx);
    cx.update(|window, cx| tab.update(cx, |t, cx| t.save_body(window, cx)));
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.read(|app| assert!(tab.read(app).notice.is_none()));
}

#[gpui::test]
fn save_body_does_nothing_when_not_done(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| tab.update(cx, |t, cx| t.save_body(window, cx)));
    assert!(!cx.did_prompt_for_new_path());
}

#[gpui::test]
fn save_body_is_atomic_and_remembers_the_directory(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let cx = init(cx);
    let tab = new_tab(cx);
    install_done(&tab, BodyStore::in_memory(&br#"{"v":2}"#[..]), cx);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("response.json");
    std::fs::write(&dest, b"old content").unwrap();

    // 第一次：对话框从默认目录打开（不是我们的临时目录）
    cx.update(|window, cx| tab.update(cx, |t, cx| t.save_body(window, cx)));
    let chosen = dest.clone();
    let expected_dir = dir.path().to_path_buf();
    cx.simulate_new_path_selection(move |opened_in| {
        assert_ne!(opened_in, expected_dir.as_path());
        Some(chosen)
    });
    wait_until(cx, |cx| cx.read(|app| tab.read(app).notice.is_some()));
    assert_eq!(std::fs::read(&dest).unwrap(), br#"{"v":2}"#);
    // 原子写：目录里只有目标文件，没有 .tmp* 残留
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["response.json"]);

    // 第二次：对话框从上次保存的目录打开
    cx.update(|_, cx| {
        tab.update(cx, |t, cx| {
            t.notice = None;
            cx.notify();
        })
    });
    cx.update(|window, cx| tab.update(cx, |t, cx| t.save_body(window, cx)));
    let second = dir.path().join("again.json");
    let chosen = second.clone();
    let expected_dir = dir.path().to_path_buf();
    cx.simulate_new_path_selection(move |opened_in| {
        assert_eq!(opened_in, expected_dir.as_path());
        Some(chosen)
    });
    wait_until(cx, |cx| cx.read(|app| tab.read(app).notice.is_some()));
    assert_eq!(std::fs::read(&second).unwrap(), br#"{"v":2}"#);
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

/// F1：draft() 的 Raw 分支直接从编辑器的 Rope 拷贝一次文本，不经过 `value()`（SharedString）
/// 这道额外的中间拷贝；这里只断言最终结果，实现细节由 request_tab.rs 里的调用决定。
#[gpui::test]
fn raw_body_draft_reads_editor_text_directly(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.body_mode = BodyMode::Raw;
            t.raw_format = RawFormat::Json;
            let editor = t.editor_for(RawFormat::Json).clone();
            editor.update(cx, |e, cx| e.set_value(r#"{"a":1}"#, window, cx));
            assert_eq!(
                t.draft(cx).body,
                BodyKind::Raw {
                    format: RawFormat::Json,
                    text: r#"{"a":1}"#.into(),
                }
            );
        })
    });
}

#[gpui::test]
fn kv_table_set_values_roundtrip(cx: &mut TestAppContext) {
    let cx = init(cx);
    let table = cx.update(|window, cx| cx.new(|cx| KvTable::new("k", "v", window, cx)));
    let values = vec![
        KeyValue::new("a", "1"),
        KeyValue {
            key: "b".into(),
            value: String::new(),
            enabled: false,
        },
    ];
    cx.update(|window, cx| {
        table.update(cx, |t, cx| {
            t.set_values(&values, window, cx);
            assert_eq!(t.values(cx), values);
            // 末尾保留一个空行用于新增
            assert_eq!(t.row_count(), 3);
            t.set_values(&[], window, cx);
            assert!(t.values(cx).is_empty());
            assert_eq!(t.row_count(), 1);
        })
    });
    // Path 参数表（锁定 key）：不补空行
    let locked =
        cx.update(|window, cx| cx.new(|cx| KvTable::new("k", "v", window, cx).locked_keys(true)));
    cx.update(|window, cx| {
        locked.update(cx, |t, cx| {
            t.set_values(&values, window, cx);
            assert_eq!(t.row_count(), 2);
            assert_eq!(t.values(cx), values);
        })
    });
}

#[gpui::test]
fn load_draft_restores_every_body_kind_without_dirtying(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    let file = std::env::temp_dir().join(format!("getcat-load-{}.json", std::process::id()));
    let drafts = vec![
        RequestDraft {
            method: Method::Post,
            url: "https://x.test/{id}?a=1".into(),
            path_params: vec![KeyValue::new("id", "7")],
            params: vec![KeyValue {
                key: "q".into(),
                value: "v".into(),
                enabled: false,
            }],
            headers: vec![KeyValue::new("X-Token", "t")],
            body: BodyKind::Raw {
                format: RawFormat::Xml,
                text: "<a/>".into(),
            },
        },
        RequestDraft {
            method: Method::Put,
            url: "https://x.test/form".into(),
            body: BodyKind::FormUrlEncoded {
                fields: vec![KeyValue::new("a", "1")],
            },
            ..Default::default()
        },
        RequestDraft {
            method: Method::Delete,
            url: "https://x.test/file".into(),
            body: BodyKind::File {
                path: file.clone(),
                content_type: Some("application/json".into()),
            },
            ..Default::default()
        },
        RequestDraft::default(),
    ];
    for draft in drafts {
        cx.update(|window, cx| {
            tab.update(cx, |t, cx| {
                t.load_draft(&draft, window, cx);
                assert_eq!(t.draft(cx), draft);
            })
        });
        // 置脏可能来自订阅回调，在事件循环跑空之后再看才靠得住，
        // 而不是在同一个 cx.update 闭包里立刻读 t.dirty。
        cx.run_until_parked();
        assert!(
            !cx.read(|app| tab.read(app).dirty),
            "programmatic load must not dirty the tab"
        );
    }
}

#[gpui::test]
fn edits_mark_tab_dirty_and_title_prefers_saved_name(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.read(|app| assert!(!tab.read(app).dirty));
    change_url(&tab, "https://api.test/users/1", cx);
    cx.update(|_, cx| {
        tab.update(cx, |t, cx| {
            assert!(t.dirty);
            assert_eq!(t.title(cx).as_ref(), "/users/1");
            t.saved_name = Some("用户详情".into());
            assert_eq!(t.title(cx).as_ref(), "用户详情");
            t.mark_clean(cx);
            assert!(!t.dirty);
        })
    });
}

#[gpui::test]
fn draft_autosaves_after_debounce(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let tab = new_tab(cx);
    let id = cx.read(|app| tab.read(app).id);
    change_url(&tab, "https://api.test/a", cx);
    change_url(&tab, "https://api.test/ab", cx);
    // 去抖窗口未到：还没有任何草稿写入
    cx.run_until_parked();
    assert!(store.flush());
    assert!(read_draft(&store, id).is_none());

    cx.executor().advance_clock(DRAFT_DEBOUNCE);
    cx.run_until_parked();
    assert!(store.flush());
    let draft = read_draft(&store, id).expect("draft file written after debounce");
    assert_eq!(draft.draft.url, "https://api.test/ab");
    assert!(draft.dirty);
    assert_eq!(draft.saved_id, None);
    // 快照只做一次：两次键入只产生一个草稿写入
    assert_eq!(store.write_count(), 1);
}

#[gpui::test]
fn new_tab_writes_draft_and_close_deletes_it(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let (first, second) = cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            let first = ws.active_tab().read(cx).id;
            ws.new_tab(window, cx);
            (first, ws.active_tab().read(cx).id)
        })
    });
    assert!(store.flush());
    assert!(read_draft(&store, first).is_some());
    assert!(read_draft(&store, second).is_some());

    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_tab(1, window, cx)));
    assert!(store.flush());
    assert!(read_draft(&store, first).is_some());
    assert!(read_draft(&store, second).is_none());

    // 关掉最后一个：旧草稿删除，新空 Tab 的草稿出现
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_tab(0, window, cx)));
    let third = cx.read(|app| ws.read(app).active_tab().read(app).id);
    assert!(store.flush());
    assert!(read_draft(&store, first).is_none());
    assert!(read_draft(&store, third).is_some());
}

#[gpui::test]
fn without_store_edits_are_harmless(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/x", cx);
    cx.executor().advance_clock(DRAFT_DEBOUNCE);
    cx.run_until_parked();
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_tab(0, window, cx)));
    cx.read(|app| assert_eq!(ws.read(app).tab_count(), 1));
}

#[gpui::test]
fn restore_rebuilds_tabs_from_prepared_root(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ids: Vec<Ulid> = (0..3).map(|_| Ulid::generate()).collect();
    for (i, id) in ids.iter().enumerate() {
        store.write_draft(TabDraft {
            id: *id,
            draft: RequestDraft {
                url: format!("https://api.test/{i}"),
                ..Default::default()
            },
            saved_id: None,
            dirty: i == 1,
        });
    }
    store.write_workspace(WorkspaceState {
        tab_order: vec![ids[2], ids[0], ids[1]],
        active: Some(ids[0]),
        sidebar_width: Some(300.),
        sidebar_collapsed: true,
        theme: ThemePref::Dark,
    });
    assert!(store.flush());
    let loaded = store.load_all();
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);

    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::restore(loaded, window, cx)));
    cx.read(|app| {
        let ws = ws.read(app);
        let urls: Vec<String> = (0..ws.tab_count())
            .map(|i| ws.tab_at(i).read(app).url.read(app).value().to_string())
            .collect();
        assert_eq!(
            urls,
            [
                "https://api.test/2",
                "https://api.test/0",
                "https://api.test/1"
            ]
        );
        assert_eq!(ws.active_index(), 1);
        assert_eq!(ws.tab_at(1).read(app).id, ids[0]);
        assert!(ws.tab_at(2).read(app).dirty);
        assert!(!ws.tab_at(1).read(app).dirty);
        assert!(ws.sidebar_collapsed());
        assert_eq!(ws.sidebar_width(), Some(300.));
        assert_eq!(ws.theme(), ThemePref::Dark);
        assert!(app.theme().mode.is_dark());
    });
}

#[gpui::test]
fn restore_without_files_creates_one_tab(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let loaded = store.load_all();
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::restore(loaded, window, cx)));
    cx.read(|app| {
        let ws = ws.read(app);
        assert_eq!(ws.tab_count(), 1);
        assert_eq!(ws.theme(), ThemePref::System);
        assert!(!ws.sidebar_collapsed());
    });
    // 新建的空 Tab 已经有草稿文件
    let id = cx.read(|app| ws.read(app).active_tab().read(app).id);
    assert!(store.flush());
    assert!(read_draft(&store, id).is_some());
}

#[gpui::test]
fn restore_clears_orphan_saved_id(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let id = Ulid::generate();
    store.write_draft(TabDraft {
        id,
        draft: RequestDraft {
            url: "https://api.test/x".into(),
            ..Default::default()
        },
        saved_id: Some(Ulid::generate()),
        dirty: false,
    });
    assert!(store.flush());
    let loaded = store.load_all();
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::restore(loaded, window, cx)));
    cx.read(|app| {
        let tab = ws.read(app).active_tab();
        let tab = tab.read(app);
        assert_eq!(tab.id, id);
        assert_eq!(tab.saved_id, None);
        assert!(tab.saved_name.is_none());
        assert!(tab.dirty, "orphaned tab must show as unsaved");
    });
}

#[gpui::test]
fn flush_drafts_writes_every_tab_immediately(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/unflushed", cx);
    // 不推进虚拟时钟：去抖任务还没触发，由 flush_on_exit 兜底
    cx.update(|_, cx| store::flush_on_exit(&ws, cx));
    let id = cx.read(|app| tab.read(app).id);
    assert_eq!(
        read_draft(&store, id).unwrap().draft.url,
        "https://api.test/unflushed"
    );
}

#[gpui::test]
fn workspace_changes_are_persisted(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.new_tab(window, cx);
            ws.activate(0, cx);
            ws.toggle_sidebar(cx);
            ws.set_theme(ThemePref::Dark, window, cx);
        })
    });
    assert!(store.flush());
    let state = read_workspace(&store).expect("workspace.json written");
    cx.read(|app| {
        let ws = ws.read(app);
        assert_eq!(
            state.tab_order,
            vec![ws.tab_at(0).read(app).id, ws.tab_at(1).read(app).id]
        );
        assert_eq!(state.active, Some(ws.tab_at(0).read(app).id));
        assert!(app.theme().mode.is_dark());
    });
    assert!(state.sidebar_collapsed);
    assert_eq!(state.theme, ThemePref::Dark);
    assert_eq!(state.sidebar_width, None);

    // 关闭 Tab 后顺序与激活项随之更新
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_tab(0, window, cx)));
    assert!(store.flush());
    let state = read_workspace(&store).unwrap();
    let remaining = cx.read(|app| ws.read(app).tab_at(0).read(app).id);
    assert_eq!(state.tab_order, vec![remaining]);
    assert_eq!(state.active, Some(remaining));
}

#[gpui::test]
fn cycle_theme_walks_system_light_dark(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.theme(), ThemePref::System);
            ws.cycle_theme(window, cx);
            assert_eq!(ws.theme(), ThemePref::Light);
            assert!(!cx.theme().mode.is_dark());
            ws.cycle_theme(window, cx);
            assert_eq!(ws.theme(), ThemePref::Dark);
            assert!(cx.theme().mode.is_dark());
            ws.cycle_theme(window, cx);
            assert_eq!(ws.theme(), ThemePref::System);
        })
    });
}

#[gpui::test]
fn finish_save_writes_request_file_and_marks_tab_clean(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/users", cx);
    cx.read(|app| assert!(tab.read(app).dirty));

    let id = cx
        .update(|_, cx| {
            ws.update(cx, |ws, cx| {
                ws.finish_save(tab.clone(), "  用户列表 ".into(), cx)
            })
        })
        .expect("tab still open");
    assert!(store.flush());
    let req = read_request(&store, id).expect("requests/<ulid>.json written");
    assert_eq!(req.name, "用户列表");
    assert_eq!(req.draft.url, "https://api.test/users");
    assert_eq!(req.draft.method, Method::Get);
    assert_eq!(req.created_at, req.updated_at);
    cx.read(|app| {
        let ws = ws.read(app);
        assert_eq!(ws.saved().len(), 1);
        let t = tab.read(app);
        assert_eq!(t.saved_id, Some(id));
        assert!(!t.dirty);
        assert_eq!(t.title(app).as_ref(), "用户列表");
    });
    // 草稿文件也记录了来源与干净状态
    let tab_id = cx.read(|app| tab.read(app).id);
    let draft = read_draft(&store, tab_id).unwrap();
    assert_eq!((draft.saved_id, draft.dirty), (Some(id), false));
}

/// F3：保存对话框在用户关闭 Tab 之后才确认——`finish_save` 必须是 no-op，
/// 不能凭空写出请求文件，也不能复活已随 close_tab 删除的草稿文件。
#[gpui::test]
fn finish_save_on_closed_tab_is_noop(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.new_tab(window, cx)));
    let tab = cx.read(|app| ws.read(app).tab_at(1));
    let tab_id = cx.read(|app| tab.read(app).id);
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_tab(1, window, cx)));

    let result =
        cx.update(|_, cx| ws.update(cx, |ws, cx| ws.finish_save(tab.clone(), "x".into(), cx)));
    assert!(result.is_none());
    assert!(store.flush());
    assert_eq!(request_files(&store), 0);
    assert!(read_draft(&store, tab_id).is_none());
}

#[gpui::test]
fn save_active_overwrites_existing_without_prompt(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/v1", cx);
    let id = cx
        .update(|_, cx| ws.update(cx, |ws, cx| ws.finish_save(tab.clone(), "接口".into(), cx)))
        .unwrap();
    // 真实时钟前进，让 updated_at 可区分
    std::thread::sleep(Duration::from_millis(2));
    change_url(&tab, "https://api.test/v2", cx);
    cx.read(|app| assert!(tab.read(app).dirty));

    // 已有 saved_id：直接覆盖，不弹对话框（测试窗口没有 Root，若弹窗会 panic）
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.save_active(window, cx)));
    assert!(store.flush());
    assert_eq!(request_files(&store), 1);
    let req = read_request(&store, id).unwrap();
    assert_eq!(req.name, "接口");
    assert_eq!(req.draft.url, "https://api.test/v2");
    assert!(req.updated_at > req.created_at);
    cx.read(|app| {
        assert!(!tab.read(app).dirty);
        assert_eq!(ws.read(app).saved()[0].id, id);
    });
}

#[gpui::test]
fn empty_name_falls_back_to_tab_title(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/users/42", cx);
    let id = cx
        .update(|_, cx| ws.update(cx, |ws, cx| ws.finish_save(tab.clone(), "   ".into(), cx)))
        .unwrap();
    assert!(store.flush());
    assert_eq!(read_request(&store, id).unwrap().name, "/users/42");
}

#[gpui::test]
fn open_saved_opens_tab_then_focuses_existing(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/items/7", cx);
    let id = cx
        .update(|_, cx| ws.update(cx, |ws, cx| ws.finish_save(tab.clone(), "条目".into(), cx)))
        .unwrap();
    // 关掉这个 Tab（自动补一个空 Tab），再从侧栏打开
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_tab(0, window, cx)));
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.open_saved(id, window, cx)));
    cx.read(|app| {
        let ws = ws.read(app);
        assert_eq!(ws.tab_count(), 2);
        assert_eq!(ws.active_index(), 1);
        let t = ws.active_tab();
        let t = t.read(app);
        assert_eq!(t.saved_id, Some(id));
        assert_eq!(t.title(app).as_ref(), "条目");
        assert_eq!(t.url.read(app).value().as_ref(), "https://api.test/items/7");
        assert!(!t.dirty);
    });
    // 再次打开同一条：聚焦已有 Tab，不新建
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.activate(0, cx);
            ws.open_saved(id, window, cx);
        })
    });
    cx.read(|app| {
        let ws = ws.read(app);
        assert_eq!(ws.tab_count(), 2);
        assert_eq!(ws.active_index(), 1);
    });
    // 打开的 Tab 的草稿记录了来源
    let opened = cx.read(|app| ws.read(app).active_tab().read(app).id);
    assert!(store.flush());
    assert_eq!(read_draft(&store, opened).unwrap().saved_id, Some(id));
    // 不存在的 id：no-op
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.open_saved(Ulid::generate(), window, cx)));
    cx.read(|app| assert_eq!(ws.read(app).tab_count(), 2));
}

#[gpui::test]
fn delete_saved_removes_file_and_detaches_tabs(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/gone", cx);
    let id = cx
        .update(|_, cx| ws.update(cx, |ws, cx| ws.finish_save(tab.clone(), "待删".into(), cx)))
        .unwrap();
    assert!(store.flush());
    assert_eq!(request_files(&store), 1);

    cx.update(|_, cx| ws.update(cx, |ws, cx| ws.delete_saved(id, cx)));
    assert!(store.flush());
    assert_eq!(request_files(&store), 0);
    cx.read(|app| {
        assert!(ws.read(app).saved().is_empty());
        let t = tab.read(app);
        assert_eq!(t.saved_id, None);
        assert!(t.saved_name.is_none());
        assert!(t.dirty, "tab content survives as an unsaved draft");
        assert_eq!(t.title(app).as_ref(), "/gone");
    });
    let tab_id = cx.read(|app| tab.read(app).id);
    assert_eq!(read_draft(&store, tab_id).unwrap().saved_id, None);
    // 删除不存在的 id 是 no-op
    cx.update(|_, cx| ws.update(cx, |ws, cx| ws.delete_saved(Ulid::generate(), cx)));
}

#[gpui::test]
fn sidebar_lists_newest_first_and_draws_rows(cx: &mut TestAppContext) {
    let (cx, _store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let first_tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&first_tab, "https://api.test/first", cx);
    let first = cx
        .update(|_, cx| {
            ws.update(cx, |ws, cx| {
                ws.finish_save(first_tab.clone(), "第一".into(), cx)
            })
        })
        .unwrap();
    // 真实时钟前进，保证 updated_at 不同
    std::thread::sleep(Duration::from_millis(2));
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.new_tab(window, cx)));
    let second_tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&second_tab, "https://api.test/second", cx);
    let second = cx
        .update(|_, cx| {
            ws.update(cx, |ws, cx| {
                ws.finish_save(second_tab.clone(), "第二".into(), cx)
            })
        })
        .unwrap();
    cx.read(|app| {
        let saved = ws.read(app).saved();
        let ids: Vec<Ulid> = saved.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![second, first]);
    });

    // 真正绘制一帧：侧栏 uniform_list 完成布局，内容高度 = 行高 × 2。
    // 先 blur：聚焦中的 Input 在渲染时会调用 macOS 的 set_text_content_type，
    // 而测试窗口没有真实平台窗口句柄（gpui TestWindow::window_handle 是 unimplemented!）。
    cx.update(|window, _| window.blur());
    let ws_element = ws.clone();
    cx.draw(point(px(0.), px(0.)), size(px(1200.), px(800.)), |_, _| {
        ws_element.into_any_element()
    });
    cx.read(|app| {
        let laid_out = ws
            .read(app)
            .saved_scroll()
            .0
            .borrow()
            .last_item_size
            .expect("saved list was laid out");
        assert_eq!(laid_out.contents.height, px(SAVED_ROW_HEIGHT * 2.));
    });
}
