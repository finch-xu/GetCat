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
use getcat_core::codegen::{CodeTarget, PLACEHOLDER_URL};
use getcat_core::http::{BodyStore, RequestError};
use getcat_core::model::{
    AppSettings, BodyKind, FormField, FormValue, HttpVersionPref, KeyValue, Method, RawFormat,
    RequestDraft, ResponseMeta, SavedRequest, SplitDirection, TabDraft, TabId, ThemePref, Ulid,
    WorkspaceState,
};
use getcat_core::store::{Store, codec::decode};
use getcat_core::tls::{CertWarning, CertificateInfo};
use gpui::{
    AppContext, Entity, Focusable, IntoElement, TestAppContext, VisualTestContext, point, px, size,
};
use gpui_component::{ActiveTheme, input::InputEvent};
use tempfile::TempDir;

use crate::i18n::Locale;
use crate::state::request_tab::{
    BODY_HINT_BYTES, BodyHint, BodyMode, DRAFT_DEBOUNCE, Notice, RequestTab, ResponseSection,
};
use crate::state::response::{ResponseState, ResponseView};
use crate::state::settings;
use crate::state::store;
use crate::state::update::{self, InstallKind};
use crate::state::workspace::{SidebarSection, ToolSection, Workspace};
use crate::ui::body_view::LINE_HEIGHT_PX;
use crate::ui::kv_table::{KvPlaceholder, KvTable, RowKind};
use crate::ui::sidebar::SAVED_ROW_HEIGHT;
use getcat_core::model::LanguagePref;

pub(crate) fn init(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::theme::install(cx);
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

pub(crate) fn read_settings(store: &Store) -> Option<AppSettings> {
    let bytes = std::fs::read(store.layout().settings_path()).ok()?;
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
        http_version: None,
        certificate: None,
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
        http_version: None,
        certificate: None,
    }
}

/// 一张自签名测试证书的解析结果；`warnings` 由调用方决定要试哪条分支。
pub(crate) fn cert_info(warnings: Vec<CertWarning>) -> CertificateInfo {
    CertificateInfo {
        subject: "CN=localhost, O=GetCat Local Debug".into(),
        issuer: "CN=localhost, O=GetCat Local Debug".into(),
        not_before: "Jan  1 00:00:00 2020 +00:00".into(),
        not_after: "Jan  1 00:00:00 2100 +00:00".into(),
        san: vec!["localhost".into(), "*.example.com".into()],
        serial: "4A:2B:1C".into(),
        signature_algorithm: "ecdsa-with-SHA256".into(),
        sha256_fingerprint: "69:0A:78:ED".into(),
        warnings,
    }
}

/// 直接把一份准备好的响应灌进 Tab（绕过网络），generation 对齐。
pub(crate) fn install_done(tab: &Entity<RequestTab>, body: BodyStore, cx: &mut VisualTestContext) {
    install_done_with(tab, "application/json", body, cx);
}

/// `install_done` 的带 content-type 版本（二进制 / 纯文本响应的分档由它决定）。
pub(crate) fn install_done_with(
    tab: &Entity<RequestTab>,
    content_type: &str,
    body: BodyStore,
    cx: &mut VisualTestContext,
) {
    let view = ResponseView::prepare(meta(content_type, body.len()), &body);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.generation += 1;
            let g = t.generation;
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
        assert!(matches!(notice, Notice::SavedTo(_)), "{notice:?}");
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
    // 用户"另存为"目标按系统 umask 创建，不继承数据目录内部文件的 0600（Ruling P4-3）。
    // 与同目录里 File::create 出来的探针文件比较，断言就与当前 umask 无关。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let probe = dir.path().join("umask-probe");
        std::fs::File::create(&probe).unwrap();
        let expected = std::fs::metadata(&probe).unwrap().permissions().mode() & 0o777;
        std::fs::remove_file(&probe).unwrap();
        let mode = std::fs::metadata(&second).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, expected, "{mode:o} != {expected:o}");
    }
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
        assert_eq!(t.body_mode, BodyMode::Binary);
        assert_eq!(t.file_size, Some(2));
        assert_eq!(
            t.draft(app).body,
            BodyKind::Binary {
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
            BodyKind::Binary {
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
            assert!(t.body_hint.unwrap().text().contains("10 MB"));
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
            assert!(t.body_hint.unwrap().text().contains("10 MB"));

            // 切到 Text 格式：该编辑器是空的，提示应清空
            t.raw_format = RawFormat::Text;
            t.refresh_body_hint(cx);
            assert!(t.body_hint.is_none());

            // 切回 JSON：重新看到超大内容的提示
            t.raw_format = RawFormat::Json;
            t.refresh_body_hint(cx);
            assert!(t.body_hint.unwrap().text().contains("10 MB"));

            // 离开 raw 模式：提示必须清空
            t.body_mode = BodyMode::None;
            t.refresh_body_hint(cx);
            assert!(t.body_hint.is_none());
        })
    });
}

/// 格式化必须保住字段顺序：这正是用 core 的单遍美化器、而不是 serde 的
/// `to_string_pretty`（默认 BTreeMap，会按字母序重排 key）的理由，值得钉死。
#[gpui::test]
fn format_body_reindents_without_reordering_keys(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.body_mode = BodyMode::Raw;
            t.raw_format = RawFormat::Json;
            let editor = t.editor_for(RawFormat::Json).clone();
            // 字母序会把 model 排到 messages 后面；这里刻意反着写
            editor.update(cx, |e, cx| {
                e.set_value(
                    r#"{"model":"gpt-5.6","messages":[{"role":"user"}],"a":1}"#,
                    window,
                    cx,
                )
            });
            t.dirty = false;

            t.format_body(window, cx);

            let out = editor.read(cx).text().to_string();
            assert!(out.contains('\n'), "格式化后应该有换行：{out}");
            assert!(out.contains("  \"model\""), "应该是 2 空格缩进：{out}");
            let model_at = out.find("\"model\"").expect("model 还在");
            let messages_at = out.find("\"messages\"").expect("messages 还在");
            let a_at = out.find("\"a\"").expect("a 还在");
            assert!(
                model_at < messages_at && messages_at < a_at,
                "字段顺序被重排了：{out}"
            );
        })
    });
    // 置脏走的是 replace_all 发出的 Change 事件 → on_body_editor_event 订阅，
    // 而 gpui 的事件要等 effect 派发，所以断言必须在 update 闭包之外
    cx.read(|app| {
        let t = tab.read(app);
        assert!(t.dirty, "格式化算一次用户改动");
        assert!(t.body_hint.is_none());
    });
}

#[gpui::test]
fn format_body_rejects_invalid_json(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.body_mode = BodyMode::Raw;
            t.raw_format = RawFormat::Json;
            let editor = t.editor_for(RawFormat::Json).clone();
            let broken = r#"{"a": 1,}"#;
            editor.update(cx, |e, cx| e.set_value(broken, window, cx));
            t.dirty = false;

            t.format_body(window, cx);

            assert_eq!(editor.read(cx).text().to_string(), broken, "内容一字未改");
            assert_eq!(t.body_hint, Some(BodyHint::InvalidJson));
            assert!(!t.dirty, "失败不该置脏");

            // 用户一动手改，提示就该消失
            editor.update(cx, |e, cx| e.set_value(r#"{"a": 1}"#, window, cx));
            t.on_body_editor_event(&editor, &InputEvent::Change, window, cx);
            assert!(t.body_hint.is_none());
        })
    });
}

#[gpui::test]
fn format_body_is_idempotent_and_scoped_to_json(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.body_mode = BodyMode::Raw;
            t.raw_format = RawFormat::Json;
            let editor = t.editor_for(RawFormat::Json).clone();
            editor.update(cx, |e, cx| e.set_value(r#"{"a":1}"#, window, cx));
            t.format_body(window, cx);
            let once = editor.read(cx).text().to_string();

            // 再格式化一次：内容不变，也不再置脏
            t.dirty = false;
            t.format_body(window, cx);
            assert_eq!(editor.read(cx).text().to_string(), once);
            assert!(!t.dirty, "已经格式化好的不该再产生一次改动");

            // 空请求体：no-op，也不报「非法 JSON」
            editor.update(cx, |e, cx| e.set_value("   ", window, cx));
            t.body_hint = None;
            t.format_body(window, cx);
            assert!(t.body_hint.is_none(), "空请求体不该报错");

            // 非 JSON 格式 / 非 raw 模式：no-op
            let text_editor = t.editor_for(RawFormat::Text).clone();
            text_editor.update(cx, |e, cx| e.set_value(r#"{"a":1}"#, window, cx));
            t.raw_format = RawFormat::Text;
            t.format_body(window, cx);
            assert_eq!(text_editor.read(cx).text().to_string(), r#"{"a":1}"#);

            t.raw_format = RawFormat::Json;
            t.body_mode = BodyMode::None;
            editor.update(cx, |e, cx| e.set_value(r#"{"a":1}"#, window, cx));
            t.format_body(window, cx);
            assert_eq!(editor.read(cx).text().to_string(), r#"{"a":1}"#);
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
    let table = cx.update(|window, cx| cx.new(|cx| KvTable::new(KvPlaceholder::Param, window, cx)));
    let values = vec![
        KeyValue {
            description: "查询词".into(),
            ..KeyValue::new("a", "1")
        },
        KeyValue {
            enabled: false,
            ..KeyValue::new("b", "")
        },
        // 只有描述也是一行有效数据，不能被当成空行丢掉
        KeyValue {
            description: "占位".into(),
            ..KeyValue::new("", "")
        },
    ];
    cx.update(|window, cx| {
        table.update(cx, |t, cx| {
            t.set_values(&values, window, cx);
            assert_eq!(t.values(cx), values);
            // 末尾保留一个空行用于新增
            assert_eq!(t.row_count(), 4);
            t.set_values(&[], window, cx);
            assert!(t.values(cx).is_empty());
            assert_eq!(t.row_count(), 1);
        })
    });
    // Path 参数表（锁定 key）：不补空行
    let locked = cx.update(|window, cx| {
        cx.new(|cx| KvTable::new(KvPlaceholder::Param, window, cx).locked_keys(true))
    });
    cx.update(|window, cx| {
        locked.update(cx, |t, cx| {
            t.set_values(&values, window, cx);
            assert_eq!(t.row_count(), 3);
            assert_eq!(t.values(cx), values);
        })
    });
}

#[gpui::test]
fn kv_table_sync_keys_keeps_description(cx: &mut TestAppContext) {
    let cx = init(cx);
    let table = cx.update(|window, cx| {
        cx.new(|cx| KvTable::new(KvPlaceholder::Param, window, cx).locked_keys(true))
    });
    cx.update(|window, cx| {
        table.update(cx, |t, cx| {
            t.set_values(
                &[KeyValue {
                    description: "用户 ID".into(),
                    ..KeyValue::new("id", "7")
                }],
                window,
                cx,
            );
            t.sync_keys(&["tenant".into(), "id".into()], window, cx);
            let v = t.values(cx);
            assert_eq!(v.len(), 2);
            assert_eq!(v[0].key, "tenant");
            assert_eq!(
                v[1],
                KeyValue {
                    description: "用户 ID".into(),
                    ..KeyValue::new("id", "7")
                }
            );
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
                enabled: false,
                ..KeyValue::new("q", "v")
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
            body: BodyKind::Binary {
                path: file.clone(),
                content_type: Some("application/json".into()),
            },
            ..Default::default()
        },
        RequestDraft {
            method: Method::Post,
            url: "https://x.test/upload".into(),
            body: BodyKind::FormData {
                fields: vec![
                    FormField {
                        description: "说明".into(),
                        ..FormField::text("note", "hi")
                    },
                    FormField::file("doc", file.clone()),
                    FormField::file("pending", PathBuf::new()),
                ],
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
fn form_data_mode_warns_when_user_sets_content_type(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.body_mode = BodyMode::FormData;
            t.refresh_body_hint(cx);
            assert_eq!(t.body_hint, None);
            let headers = t.headers.clone();
            headers.update(cx, |h, cx| {
                h.set_values(&[KeyValue::new("content-type", "text/plain")], window, cx)
            });
            t.refresh_body_hint(cx);
            assert_eq!(t.body_hint, Some(BodyHint::FormDataContentType));
            // 禁用那一行：提示消失
            headers.update(cx, |h, cx| {
                h.set_values(
                    &[KeyValue {
                        enabled: false,
                        ..KeyValue::new("content-type", "text/plain")
                    }],
                    window,
                    cx,
                )
            });
            t.refresh_body_hint(cx);
            assert_eq!(t.body_hint, None);
            // 其他模式不提示
            headers.update(cx, |h, cx| {
                h.set_values(&[KeyValue::new("Content-Type", "text/plain")], window, cx)
            });
            t.body_mode = BodyMode::Raw;
            t.refresh_body_hint(cx);
            assert_eq!(t.body_hint, None);
        })
    });
}

#[gpui::test]
fn form_data_and_urlencoded_tables_are_independent(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.load_draft(
                &RequestDraft {
                    body: BodyKind::FormUrlEncoded {
                        fields: vec![KeyValue::new("a", "1")],
                    },
                    ..Default::default()
                },
                window,
                cx,
            );
            t.body_mode = BodyMode::FormData;
            assert_eq!(t.draft(cx).body, BodyKind::FormData { fields: vec![] });
            t.body_mode = BodyMode::FormUrlEncoded;
            assert_eq!(
                t.draft(cx).body,
                BodyKind::FormUrlEncoded {
                    fields: vec![KeyValue::new("a", "1")]
                }
            );
        })
    });
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
        sidebar_collapsed: false,
        theme: ThemePref::Dark,
        split: SplitDirection::Horizontal,
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
        // 文件里写的是展开（与默认值相反），证明读的是文件而不是默认
        assert!(!ws.sidebar_collapsed());
        assert_eq!(ws.sidebar_width(), Some(300.));
        assert_eq!(ws.theme(), ThemePref::Dark);
        assert_eq!(ws.split(), SplitDirection::Horizontal);
        assert_eq!(ws.tab_at(0).read(app).split, SplitDirection::Horizontal);
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
        // 首次启动侧栏收成图标栏
        assert!(ws.sidebar_collapsed());
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
    // 默认收起，toggle 一次后是展开
    assert!(!state.sidebar_collapsed);
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

/// 主题偏好在 System / Light / Dark 之间循环时，每一档都必须还是 GetCat 的配色。
/// `Theme::change` 会整套重刷 ThemeColor，若配色只是切换后打的补丁就会在这里丢掉。
#[gpui::test]
fn cycling_theme_keeps_the_getcat_palette(cx: &mut TestAppContext) {
    fn hex(color: gpui::Hsla) -> u32 {
        let rgba = gpui::Rgba::from(color);
        let to8 = |v: f32| (v * 255.0).round() as u32;
        (to8(rgba.r) << 16) | (to8(rgba.g) << 8) | to8(rgba.b)
    }

    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            for _ in 0..4 {
                ws.cycle_theme(window, cx);
                let expected = if cx.theme().mode.is_dark() {
                    0x6fa8d4
                } else {
                    0x3f87bd
                };
                assert_eq!(
                    hex(cx.theme().primary),
                    expected,
                    "主题切到 {:?} 后丢失了 GetCat 配色",
                    ws.theme()
                );
            }
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
fn open_template_prefills_a_new_tab(cx: &mut TestAppContext) {
    let (cx, _store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let template = crate::templates::find("openai-chat-vision").unwrap();

    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_template("openai-chat-vision", window, cx)
        })
    });
    cx.read(|app| {
        let ws = ws.read(app);
        assert_eq!(ws.tab_count(), 2, "模板开在新 Tab 里，不覆盖原有的");
        let t = ws.active_tab();
        let t = t.read(app);
        let draft = t.draft(app);
        assert_eq!(draft.method, Method::Post);
        assert_eq!(draft.url, template.url);
        assert_eq!(draft.headers.len(), template.headers.len());
        assert!(
            draft
                .headers
                .iter()
                .any(|h| h.key == "Authorization" && h.value == "Bearer YOUR_API_KEY")
        );
        match draft.body {
            BodyKind::Raw { format, ref text } => {
                assert_eq!(format, RawFormat::Json);
                assert_eq!(text, template.body, "请求体应原样进编辑器");
            }
            ref other => panic!("期望 Raw JSON，实际 {other:?}"),
        }
        // 模板产出的是全新未保存请求；标题走 URL 末段，saved_name 只属于真保存过的
        assert!(t.saved_id.is_none());
        assert!(t.saved_name.is_none());
        assert!(t.dirty);
        assert_eq!(t.title(app).as_ref(), "/v1/chat/completions");
    });

    // 同一个模板允许再开一份（不像 open_saved 那样聚焦已有 Tab）
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_template("openai-chat-vision", window, cx)
        })
    });
    cx.read(|app| assert_eq!(ws.read(app).tab_count(), 3));

    // 不存在的 id：no-op
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.open_template("nope", window, cx)));
    cx.read(|app| assert_eq!(ws.read(app).tab_count(), 3));
}

#[gpui::test]
fn duplicate_active_copies_content_next_to_the_source(cx: &mut TestAppContext) {
    let (cx, _store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));

    // 攒出 [空, 模板, 空] 三个 Tab：复制中间那个，插入位置才看得出来
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_template("openai-chat-vision", window, cx);
            ws.new_tab(window, cx);
            ws.activate(1, cx);
        })
    });

    // 给源挂上 saved_id，才测得出副本没有继承它
    let source = cx.read(|app| ws.read(app).tab_at(1));
    let saved_id = Ulid::generate();
    cx.update(|_, cx| {
        source.update(cx, |t, _| {
            t.saved_id = Some(saved_id);
            t.dirty = false;
        })
    });
    let (source_draft, source_id) = cx.read(|app| {
        let t = source.read(app);
        (t.draft(app), t.id)
    });

    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.duplicate_active(window, cx)));

    cx.read(|app| {
        let ws = ws.read(app);
        assert_eq!(ws.tab_count(), 4);
        // 副本紧跟在源右边，而不是被追加到末尾
        assert_eq!(ws.active_index(), 2);
        let copy = ws.tab_at(2);
        let copy = copy.read(app);
        assert_eq!(copy.draft(app), source_draft, "填过的内容要原样带过来");
        // 副本是全新的未保存请求：继承 saved_id 的话两个 Tab 会互相覆盖对方的保存
        assert!(copy.saved_id.is_none());
        assert!(copy.dirty);
        assert_ne!(copy.id, source_id, "草稿文件名必须是新的，不能共用");
        // 源本身不受影响
        let src = ws.tab_at(1);
        let src = src.read(app);
        assert_eq!(src.saved_id, Some(saved_id));
        assert!(!src.dirty);
    });
}

#[gpui::test]
fn duplicate_active_on_the_last_tab_appends(cx: &mut TestAppContext) {
    let (cx, _store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    // 只有一个 Tab 时「插到源右边」就是末尾，不该把下标算错
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.duplicate_active(window, cx)));
    cx.read(|app| {
        let ws = ws.read(app);
        assert_eq!((ws.tab_count(), ws.active_index()), (2, 1));
    });
}

#[gpui::test]
fn template_panel_switches_and_draws(cx: &mut TestAppContext) {
    let (cx, _store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));

    // 图标栏点「模板」：展开面板并切过去
    cx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_sidebar_section(SidebarSection::Templates, cx)
        })
    });
    cx.read(|app| {
        let ws = ws.read(app);
        assert_eq!(ws.sidebar_section(), SidebarSection::Templates);
        assert!(!ws.sidebar_collapsed());
    });

    // 真正绘制一帧：模板行是手工平铺的，element id 冲突或借用错误只有在布局时才暴露。
    // blur 的原因同 sidebar_lists_newest_first_and_draws_rows。
    cx.update(|window, _| window.blur());
    let ws_element = ws.clone();
    cx.draw(point(px(0.), px(0.)), size(px(1200.), px(800.)), |_, _| {
        ws_element.into_any_element()
    });

    // 再点同一个图标：收起
    cx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_sidebar_section(SidebarSection::Templates, cx)
        })
    });
    cx.read(|app| assert!(ws.read(app).sidebar_collapsed()));
}

/// 标签多到溢出时，标签栏要多渲染箭头、溢出菜单和末尾占位。这些都只在真实布局
/// 阶段才组装，`cargo check` 抓不到 element id 冲突之类的问题，所以画一帧。
#[gpui::test]
fn tab_bar_draws_with_many_tabs(cx: &mut TestAppContext) {
    let (cx, _store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    for _ in 0..12 {
        cx.update(|window, cx| ws.update(cx, |ws, cx| ws.new_tab(window, cx)));
    }
    cx.read(|app| {
        let ws = ws.read(app);
        assert_eq!(ws.tab_count(), 13);
        assert_eq!(ws.active_index(), 12, "新建后激活最后一个");
    });

    // blur 的原因同 sidebar_lists_newest_first_and_draws_rows
    cx.update(|window, _| window.blur());
    let ws_element = ws.clone();
    cx.draw(point(px(0.), px(0.)), size(px(900.), px(600.)), |_, _| {
        ws_element.into_any_element()
    });

    // 画过一帧后布局才算出溢出量，这时箭头才有意义
    cx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.scroll_tabs(1., cx);
            ws.scroll_tabs(-1., cx);
            // 反复滚到头也不该把偏移推出边界
            for _ in 0..20 {
                ws.scroll_tabs(-1., cx);
            }
        })
    });
    cx.draw(point(px(0.), px(0.)), size(px(900.), px(600.)), |_, _| {
        ws.clone().into_any_element()
    });
}

/// 图标栏的 Button id 用 `section as usize`，面板切换也按数组下标走：
/// `ALL` 的顺序一旦与变体声明顺序错开，点第二个图标会展开第一个面板。
#[test]
fn sidebar_sections_are_indexed_by_discriminant() {
    for (ix, section) in SidebarSection::ALL.iter().enumerate() {
        assert_eq!(*section as usize, ix, "ALL[{ix}] 与判别值对不上");
    }
}

/// 右侧图标栏同理：Button id 用 `section as usize`，顺序错开就会点错功能。
#[test]
fn tool_sections_are_indexed_by_discriminant() {
    for (ix, section) in ToolSection::ALL.iter().enumerate() {
        assert_eq!(*section as usize, ix, "ALL[{ix}] 与判别值对不上");
    }
}

/// 抽屉里的代码来自**当前** Tab：切了 Tab 再打开，看到的必须是新那条请求。
#[gpui::test]
fn code_sheet_generates_from_the_active_tab(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));

    let first = cx.read(|app| ws.read(app).active_tab());
    change_url(&first, "https://api.test/v1/first", cx);
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.refresh_code_sheet(window, cx)));
    let code = cx.read(|app| ws.read(app).code_sheet.read(app).text().clone());
    assert!(
        code.contains("curl -X GET 'https://api.test/v1/first'"),
        "{code}"
    );
    assert!(cx.read(|app| ws.read(app).code_sheet.read(app).error().is_none()));

    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.new_tab(window, cx)));
    let second = cx.read(|app| ws.read(app).active_tab());
    change_url(&second, "https://api.test/v2/second", cx);
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.refresh_code_sheet(window, cx)));
    let code = cx.read(|app| ws.read(app).code_sheet.read(app).text().clone());
    assert!(code.contains("https://api.test/v2/second"), "{code}");
    assert!(
        !code.contains("v1/first"),
        "还留着上一个 Tab 的内容：{code}"
    );
}

/// 切换生成目标要重新生成，而不是把旧代码留在编辑器里。
#[gpui::test]
fn switching_the_target_regenerates_the_code(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/ping", cx);
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.refresh_code_sheet(window, cx)));
    assert!(cx.read(|app| ws.read(app).code_sheet.read(app).text().starts_with("curl")));

    cx.update(|window, cx| {
        ws.read(cx).code_sheet.clone().update(cx, |sheet, cx| {
            sheet.set_target_for_test(CodeTarget::PythonRequests, window, cx)
        })
    });
    let code = cx.read(|app| ws.read(app).code_sheet.read(app).text().clone());
    assert!(code.starts_with("import requests"), "{code}");
    assert_eq!(
        cx.read(|app| ws.read(app).code_sheet.read(app).target()),
        CodeTarget::PythonRequests
    );
}

/// 抽屉正文必须是**能独立渲染的实体**。
///
/// 它由 `Sheet` 的 builder 在 `Workspace::render` 内部渲染；一旦有人把它改回
/// `Workspace` 上的 render 方法（builder 里 `workspace.update(...)`），运行时就会
/// 二次借用 Workspace 而 panic「cannot update Workspace while it is already being
/// updated」，表现为点一下右侧图标栏就闪退。
///
/// 真实的 `Root` 窗口在测试里建不起来（`Root::new` 要装 macOS hit-test 转发器，
/// 需要真实 NSView），所以这里钉的是结构：CodeSheet 自己就是 `Render`，
/// 单独画一帧不碰 Workspace。改回去会直接编译失败。
#[gpui::test]
fn code_sheet_renders_without_touching_the_workspace(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/ping", cx);
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.refresh_code_sheet(window, cx)));

    let sheet = cx.read(|app| ws.read(app).code_sheet.clone());
    cx.update(|window, _| window.blur());
    // Workspace 同时被自己的 render 借着，抽屉照样画得出来——这正是修复的要点
    cx.draw(point(px(0.), px(0.)), size(px(560.), px(700.)), |_, _| {
        sheet.clone().into_any_element()
    });
    cx.draw(point(px(0.), px(0.)), size(px(900.), px(600.)), |_, _| {
        ws.clone().into_any_element()
    });
}

/// URL 还没填就打开抽屉：给一段占位骨架，而不是一条红字。
/// 新建 Tab 本来就是空 URL，报错会让人以为是自己弄坏了什么。
#[gpui::test]
fn an_unfilled_url_shows_a_placeholder_skeleton(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.refresh_code_sheet(window, cx)));
    cx.read(|app| {
        let sheet = ws.read(app).code_sheet.read(app);
        assert!(sheet.error().is_none(), "空 URL 不该报错");
        assert!(sheet.text().contains(PLACEHOLDER_URL), "{}", sheet.text());
    });
}

/// 但真填错了还是要报出来——那不是「还没填」，是需要用户去改。
#[gpui::test]
fn a_malformed_url_still_shows_the_error(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "ftp://files.example.com", cx);
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.refresh_code_sheet(window, cx)));
    cx.read(|app| {
        let sheet = ws.read(app).code_sheet.read(app);
        assert!(matches!(sheet.error(), Some(RequestError::InvalidUrl(_))));
        assert!(sheet.text().is_empty(), "报错时不该留着上一次的代码");
    });
}

/// 默认请求头的开关是全局设置，抽屉每次生成都现取。
#[gpui::test]
fn disabling_a_default_header_shows_up_in_the_generated_code(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/ping", cx);

    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.refresh_code_sheet(window, cx)));
    assert!(cx.read(|app| {
        ws.read(app)
            .code_sheet
            .read(app)
            .text()
            .contains("User-Agent")
    }));

    cx.update(|_, cx| {
        settings::update(cx, |s| {
            s.request.disabled_default_headers = vec!["user-agent".into()]
        })
    });
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.refresh_code_sheet(window, cx)));
    let code = cx.read(|app| ws.read(app).code_sheet.read(app).text().clone());
    assert!(!code.contains("User-Agent"), "{code}");
    assert!(code.contains("Accept"), "其余默认头还在：{code}");
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
    // 侧栏默认收成图标栏（不画列表），先展开；
    // 再 blur：聚焦中的 Input 在渲染时会调用 macOS 的 set_text_content_type，
    // 而测试窗口没有真实平台窗口句柄（gpui TestWindow::window_handle 是 unimplemented!）。
    cx.update(|_, cx| ws.update(cx, |ws, cx| ws.toggle_sidebar(cx)));
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

#[gpui::test]
fn clear_response_resets_everything_including_the_editor(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    install_done(&tab, BodyStore::in_memory(&br#"{"a":1}"#[..]), cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.response_section = ResponseSection::Headers;
            t.notice = Some(Notice::NoResponse);
            let before = t.generation;
            assert!(
                !t.response_editor_for("json")
                    .read(cx)
                    .text()
                    .to_string()
                    .is_empty()
            );

            t.clear_response(window, cx);

            assert!(matches!(t.response, ResponseState::Idle));
            // 编辑器是常驻实体、不随 response 一起 drop，留着旧文本会在 ⌘F 里冒出来
            assert_eq!(
                t.response_editor_for("json").read(cx).text().to_string(),
                ""
            );
            assert_eq!(t.response_section, ResponseSection::Body);
            assert!(t.notice.is_none());
            // generation 必须往前走，否则在途请求的回调还能把旧响应写回来
            assert_eq!(t.generation, before + 1);
        })
    });
    cx.run_until_parked();
}

/// 已经是空态时再点一次不该白白递增 generation——那会让一个正常在途的请求
/// 悄悄失效（此路径下 response 是 Idle，但重发刚起步时也短暂如此）。
#[gpui::test]
fn clear_response_on_idle_is_a_no_op(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            let before = t.generation;
            t.clear_response(window, cx);
            assert_eq!(t.generation, before);
        })
    });
}

#[gpui::test]
fn find_in_response_focuses_the_editor_on_editor_tier(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    install_done(&tab, BodyStore::in_memory(&br#"{"a":1}"#[..]), cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.response_section = ResponseSection::Headers;
            t.find_in_response(window, cx);
            assert_eq!(t.response_section, ResponseSection::Body);
            assert!(
                t.response_editor_for("json")
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window),
                "the read-only editor must take focus so its search panel can open"
            );
            assert!(t.notice.is_none());
        })
    });
    cx.run_until_parked();
}

#[gpui::test]
fn find_in_response_only_notices_on_virtual_tier(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    let text: String = (0..EDITOR_MAX_LINES + 1)
        .map(|i| format!("line {i}\n"))
        .collect();
    let body = BodyStore::in_memory(text.as_bytes().to_vec());
    let view = ResponseView::prepare(meta("text/plain", body.len()), &body);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            let g = t.generation;
            t.apply_outcome(g, Ok((body, view)), window, cx);
            t.find_in_response(window, cx);
            assert_eq!(t.notice, Some(Notice::VirtualSearch));
            assert!(
                !t.response_editor_for("text")
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
    });
}

#[gpui::test]
fn find_in_response_without_a_response_notices(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.find_in_response(window, cx);
            assert_eq!(t.notice, Some(Notice::NoResponse));
        })
    });
}

/// 二进制响应连 raw 文档都不准备（`view.doc()` 是 None）：只提示，不抢焦点、不切回 Body。
#[gpui::test]
fn find_in_response_on_a_binary_body_notices(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    install_done_with(
        &tab,
        "image/png",
        BodyStore::in_memory(&b"\x89PNG\0"[..]),
        cx,
    );
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.response_section = ResponseSection::Headers;
            t.find_in_response(window, cx);
            assert_eq!(t.notice, Some(Notice::BinarySearch));
            // 提前返回：既不切回 Body，也不把焦点交给（二进制用的）text 编辑器
            assert_eq!(t.response_section, ResponseSection::Headers);
            assert!(
                !t.response_editor_for("text")
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
    });
}

/// 空 Body 虽然被判为 A 档，但画的是「响应体为空」占位而非编辑器：只提示，不抢焦点。
#[gpui::test]
fn find_in_response_on_an_empty_body_notices(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    install_done(&tab, BodyStore::in_memory(&b""[..]), cx);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.find_in_response(window, cx);
            assert_eq!(t.notice, Some(Notice::EmptyBodySearch));
            assert!(
                !t.response_editor_for("json")
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
    });
}

/// 两段式按钮直接指定方向，而不是翻转：重复点当前那一段不应有任何变化。
#[gpui::test]
fn set_split_is_idempotent_for_the_current_direction(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.new_tab(window, cx);
            assert_eq!(ws.split(), SplitDirection::Horizontal);
            // 点「右侧」——已经是右侧，保持不变
            ws.set_split(SplitDirection::Horizontal, cx);
            assert_eq!(ws.split(), SplitDirection::Horizontal);
            assert_eq!(ws.tab_at(0).read(cx).split, SplitDirection::Horizontal);
            // 点「下方」
            ws.set_split(SplitDirection::Vertical, cx);
            assert_eq!(ws.split(), SplitDirection::Vertical);
            ws.set_split(SplitDirection::Vertical, cx);
            assert_eq!(ws.split(), SplitDirection::Vertical);
        })
    });
    assert!(store.flush());
    assert_eq!(
        read_workspace(&store).unwrap().split,
        SplitDirection::Vertical
    );
}

#[gpui::test]
fn split_direction_applies_to_all_tabs_and_persists(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.new_tab(window, cx);
            // 默认左右分栏：响应区在右侧
            assert_eq!(ws.split(), SplitDirection::Horizontal);
            ws.set_split(SplitDirection::Vertical, cx);
            assert_eq!(ws.split(), SplitDirection::Vertical);
            assert_eq!(ws.tab_at(0).read(cx).split, SplitDirection::Vertical);
            assert_eq!(ws.tab_at(1).read(cx).split, SplitDirection::Vertical);
            // 切换后新建的 Tab 继承方向
            ws.new_tab(window, cx);
            assert_eq!(ws.tab_at(2).read(cx).split, SplitDirection::Vertical);
        })
    });
    assert!(store.flush());
    assert_eq!(
        read_workspace(&store).unwrap().split,
        SplitDirection::Vertical
    );
    cx.update(|_, cx| ws.update(cx, |ws, cx| ws.set_split(SplitDirection::Horizontal, cx)));
    assert!(store.flush());
    assert_eq!(
        read_workspace(&store).unwrap().split,
        SplitDirection::Horizontal
    );
    cx.read(|app| {
        assert_eq!(
            ws.read(app).tab_at(2).read(app).split,
            SplitDirection::Horizontal
        )
    });
}

#[gpui::test]
fn workspace_draws_with_title_bar(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    let tab = cx.read(|app| ws.read(app).active_tab());
    change_url(&tab, "https://api.test/users/1", cx);
    cx.read(|app| assert_eq!(ws.read(app).title_bar_subtitle(app).as_ref(), "/users/1"));
    // 整个 Workspace（含 TitleBar）在 TestPlatform 下能画出一帧：TitleBar 会查询 window_decorations /
    // is_fullscreen / window_controls，这些在测试窗口上都有实现或默认值。
    // 先 blur：聚焦中的 Input 渲染时会去拿真实平台窗口句柄（TestWindow 未实现），与
    // sidebar_lists_newest_first_and_draws_rows 同样的原因。
    cx.update(|window, _| window.blur());
    let ws_element = ws.clone();
    cx.draw(point(px(0.), px(0.)), size(px(1200.), px(800.)), |_, _| {
        ws_element.into_any_element()
    });
    // 新建的空 Tab 成为激活 Tab：副标题跟着变（测试进程的 locale 是 en）
    let _locale = crate::i18n::locale_test_lock();
    cx.update(|window, cx| ws.update(cx, |ws, cx| ws.new_tab(window, cx)));
    cx.read(|app| assert_eq!(ws.read(app).title_bar_subtitle(app).as_ref(), "New request"));
}

/// 标签栏上每个标签都带 method 角标；多开几个把它画出来，确认 prefix 与
/// dirty 圆点合并后仍能布局（prefix 只能设一次，合并写错会丢圆点或 panic）。
#[gpui::test]
fn tab_bar_with_method_badges_draws(cx: &mut TestAppContext) {
    let cx = init(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            // 六个标签：够触发溢出滚动与箭头按钮
            for _ in 0..5 {
                ws.new_tab(window, cx);
            }
            assert_eq!(ws.tab_count(), 6);
        })
    });
    // 其中一个标记为有改动：圆点要和角标并排，而不是互相覆盖
    let dirty_tab = cx.read(|app| ws.read(app).tab_at(2));
    cx.update(|_, cx| dirty_tab.update(cx, |t, cx| t.mark_dirty(cx)));

    cx.update(|window, _| window.blur());
    let element = ws.clone();
    cx.draw(point(px(0.), px(0.)), size(px(1200.), px(800.)), |_, _| {
        element.into_any_element()
    });
}

/// URL 栏：发送按钮在前、保存拆成「保存 + ∨」两半，输入框里还嵌了版本选择器。
/// 这些都是新加的元素，先确认整行能画出来。
#[gpui::test]
fn url_bar_with_split_save_and_version_picker_draws(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    change_url(&tab, "https://api.test/users/1", cx);
    // 非默认版本：标签从「自动」换成协议名，宽度也跟着变
    cx.update(|_, cx| {
        tab.update(cx, |t, cx| {
            t.http_version = HttpVersionPref::Http2;
            cx.notify();
        })
    });

    cx.update(|window, _| window.blur());
    let element = tab.clone();
    cx.draw(point(px(0.), px(0.)), size(px(1200.), px(800.)), |_, _| {
        element.into_any_element()
    });
}

/// 证书页签只在拿到证书时出现，且体检有结论时上方挂横幅。
#[gpui::test]
fn certificate_tab_appears_only_with_a_certificate(cx: &mut TestAppContext) {
    // 纯函数部分：http 请求不该多出一页
    assert_eq!(
        ResponseSection::visible(false),
        vec![ResponseSection::Body, ResponseSection::Headers]
    );
    assert_eq!(
        ResponseSection::visible(true),
        vec![
            ResponseSection::Body,
            ResponseSection::Headers,
            ResponseSection::Certificate
        ]
    );

    let cx = init(cx);
    let tab = new_tab(cx);
    let body = BodyStore::in_memory(&b"{}"[..]);
    let mut m = meta("application/json", body.len());
    m.http_version = Some("HTTP/2".into());
    m.certificate = Some(Box::new(cert_info(vec![CertWarning::SelfSigned])));
    let view = ResponseView::prepare(m, &body);
    cx.update(|window, cx| {
        tab.update(cx, |t, cx| {
            t.generation += 1;
            let g = t.generation;
            t.apply_outcome(g, Ok((body, view)), window, cx);
            // 切到证书页：横幅 + 字段表都要能画
            t.response_section = ResponseSection::Certificate;
            cx.notify();
        })
    });

    cx.update(|window, _| window.blur());
    let element = tab.clone();
    cx.draw(point(px(0.), px(0.)), size(px(1200.), px(800.)), |_, _| {
        element.into_any_element()
    });
}

/// 上一条响应有证书、下一条没有：页签消失后停在 Certificate 上不能画白板。
#[gpui::test]
fn certificate_section_falls_back_to_body_without_a_certificate(cx: &mut TestAppContext) {
    let cx = init(cx);
    let tab = new_tab(cx);
    let body = BodyStore::in_memory(&b"{\"a\":1}"[..]);
    install_done(&tab, body, cx); // meta() 里 certificate 为 None
    cx.update(|_, cx| {
        tab.update(cx, |t, cx| {
            t.response_section = ResponseSection::Certificate;
            cx.notify();
        })
    });

    cx.update(|window, _| window.blur());
    let element = tab.clone();
    cx.draw(point(px(0.), px(0.)), size(px(1200.), px(800.)), |_, _| {
        element.into_any_element()
    });
}

fn temp_form_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("getcat-form-{}-{name}", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path
}

#[gpui::test]
fn kv_table_form_fields_roundtrip_and_refresh_size(cx: &mut TestAppContext) {
    let cx = init(cx);
    let file = temp_form_file("doc.json", b"{}");
    let table = cx.update(|window, cx| {
        cx.new(|cx| KvTable::new(KvPlaceholder::Field, window, cx).file_capable(true))
    });
    let fields = vec![
        FormField {
            description: "备注".into(),
            ..FormField::text("note", "hi")
        },
        FormField {
            value: FormValue::File {
                path: file.clone(),
                content_type: Some("text/csv".into()),
            },
            ..FormField::file("doc", PathBuf::new())
        },
        FormField {
            enabled: false,
            ..FormField::text("off", "")
        },
        // 文件行未选文件：也是一行有效数据（draft 会据此报"未选择文件"）
        FormField::file("avatar", PathBuf::new()),
    ];
    cx.update(|window, cx| {
        table.update(cx, |t, cx| {
            t.set_form_fields(&fields, window, cx);
            assert_eq!(t.form_fields(cx), fields);
            assert_eq!(t.row_count(), 5);
        })
    });
    cx.run_until_parked();
    cx.read(|app| assert_eq!(table.read(app).row_file_size(1), Some(2)));
    let _ = std::fs::remove_file(&file);
}

#[gpui::test]
fn kv_table_choose_row_file_sets_path_and_switching_back_drops_it(cx: &mut TestAppContext) {
    let cx = init(cx);
    let file = temp_form_file("pic.png", b"png!");
    let table = cx.update(|window, cx| {
        cx.new(|cx| KvTable::new(KvPlaceholder::Field, window, cx).file_capable(true))
    });
    cx.update(|window, cx| {
        table.update(cx, |t, cx| {
            t.set_form_fields(&[FormField::text("avatar", "")], window, cx);
            t.set_row_kind(0, RowKind::File, cx);
            t.choose_row_file(0, window, cx);
        })
    });
    assert!(cx.did_prompt_for_paths());
    let chosen = file.clone();
    cx.simulate_path_prompt_response(move |opts| {
        assert!(opts.files && !opts.directories && !opts.multiple);
        Some(vec![chosen])
    });
    cx.run_until_parked();
    cx.read(|app| {
        let t = table.read(app);
        assert_eq!(
            t.form_fields(app)[0].value,
            FormValue::File {
                path: file.clone(),
                content_type: None
            }
        );
        assert_eq!(t.row_file_size(0), Some(4));
    });
    // 切回 Text：丢弃路径，值为空文本
    cx.update(|_, cx| table.update(cx, |t, cx| t.set_row_kind(0, RowKind::Text, cx)));
    cx.read(|app| {
        let f = &table.read(app).form_fields(app)[0];
        assert_eq!(f.key, "avatar");
        assert_eq!(
            f.value,
            FormValue::Text {
                value: String::new()
            }
        );
    });
    let _ = std::fs::remove_file(&file);
}

#[gpui::test]
fn kv_table_cancelled_row_file_dialog_keeps_row(cx: &mut TestAppContext) {
    let cx = init(cx);
    let table = cx.update(|window, cx| {
        cx.new(|cx| KvTable::new(KvPlaceholder::Field, window, cx).file_capable(true))
    });
    cx.update(|window, cx| {
        table.update(cx, |t, cx| {
            t.set_form_fields(&[FormField::file("doc", PathBuf::new())], window, cx);
            t.choose_row_file(0, window, cx);
        })
    });
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(
            table.read(app).form_fields(app),
            vec![FormField::file("doc", PathBuf::new())]
        );
    });
}

/// 在末尾空行上选文件：该行变成有内容的一行，末尾必须再补一个空行，否则用户没法继续加字段。
#[gpui::test]
fn kv_table_choosing_file_on_trailing_row_appends_empty_row(cx: &mut TestAppContext) {
    let cx = init(cx);
    let file = temp_form_file("trailing.txt", b"hello");
    let table = cx.update(|window, cx| {
        cx.new(|cx| KvTable::new(KvPlaceholder::Field, window, cx).file_capable(true))
    });
    cx.update(|window, cx| {
        table.update(cx, |t, cx| {
            t.set_form_fields(&[], window, cx);
            assert_eq!(t.row_count(), 1);
            t.set_row_kind(0, RowKind::File, cx);
            t.choose_row_file(0, window, cx);
        })
    });
    let chosen = file.clone();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
    cx.run_until_parked();
    cx.read(|app| {
        let t = table.read(app);
        assert_eq!(t.row_count(), 2);
        assert_eq!(t.row_file_size(0), Some(5));
        assert_eq!(t.form_fields(app), vec![FormField::file("", file.clone())]);
    });
    let _ = std::fs::remove_file(&file);
}

#[gpui::test]
fn rail_click_expands_then_collapses_the_same_section(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.read(|app| assert!(ws.read(app).sidebar_collapsed()));

    // 收起状态下点功能图标：展开并切到它
    cx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_sidebar_section(SidebarSection::Saved, cx)
        })
    });
    cx.read(|app| {
        let ws = ws.read(app);
        assert!(!ws.sidebar_collapsed());
        assert_eq!(ws.sidebar_section(), SidebarSection::Saved);
    });
    // 再点同一个：收起
    cx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_sidebar_section(SidebarSection::Saved, cx)
        })
    });
    cx.read(|app| assert!(ws.read(app).sidebar_collapsed()));
    // 折叠状态落盘
    assert!(store.flush());
    assert!(read_workspace(&store).unwrap().sidebar_collapsed);
}

#[gpui::test]
fn settings_update_persists_and_applies_font_size(cx: &mut TestAppContext) {
    // settings::install / update 会触碰进程级的 locale
    let _locale = crate::i18n::locale_test_lock();
    let (cx, store, _dir) = init_with_store(cx);
    cx.update(|_, cx| settings::install(cx, None));
    cx.read(|app| assert_eq!(settings::settings(app), AppSettings::default()));

    cx.update(|_, cx| {
        settings::update(cx, |s| {
            s.editor_font_size = 16;
            s.request.follow_redirects = false;
        })
    });
    cx.read(|app| {
        let s = settings::settings(app);
        assert_eq!(s.editor_font_size, 16);
        assert!(!s.request.follow_redirects);
        assert_eq!(app.theme().mono_font_size, px(16.));
    });
    assert!(store.flush());
    let on_disk = read_settings(&store).expect("settings.json written");
    assert_eq!(on_disk.editor_font_size, 16);
    assert!(!on_disk.request.follow_redirects);

    // 字号越界被夹回范围；没有实际变化的 update 不写文件
    let writes = store.write_count();
    cx.update(|_, cx| settings::update(cx, |s| s.editor_font_size = 99));
    cx.read(|app| assert_eq!(settings::settings(app).editor_font_size, 24));
    cx.update(|_, cx| settings::update(cx, |_| {}));
    assert!(store.flush());
    assert_eq!(store.write_count(), writes + 1);

    cx.update(|_, cx| settings::reset(cx));
    cx.read(|app| assert_eq!(settings::settings(app), AppSettings::default()));
}

/// 设置里切换语言：rust-i18n 的 locale、`Locale` 全局与驻留在 InputState 里的占位符都立即更新，
/// 不需要重启。末尾切回英文：locale 是进程级全局，不能把别的测试留在中文里。
#[gpui::test]
fn switching_language_updates_placeholders_immediately(cx: &mut TestAppContext) {
    let _locale = crate::i18n::locale_test_lock();
    let cx = init(cx);
    cx.update(|_, cx| settings::install(cx, None));
    let tab = new_tab(cx);
    cx.read(|app| {
        assert_eq!(app.global::<Locale>().0, "en");
        assert_eq!(
            tab.read(app)
                .url
                .read(app)
                .presentation()
                .placeholder()
                .as_ref(),
            "Enter a request URL, e.g. https://api.example.com/users/{id}"
        );
        assert_eq!(
            tab.read(app)
                .params
                .read(app)
                .key_placeholder(0, app)
                .as_ref(),
            "Name"
        );
    });

    cx.update(|_, cx| settings::update(cx, |s| s.language = LanguagePref::Chinese));
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(app.global::<Locale>().0, "zh-CN");
        assert_eq!(settings::settings(app).language, LanguagePref::Chinese);
        assert_eq!(
            tab.read(app)
                .url
                .read(app)
                .presentation()
                .placeholder()
                .as_ref(),
            "输入请求 URL，例如 https://api.example.com/users/{id}"
        );
        assert_eq!(
            tab.read(app)
                .params
                .read(app)
                .key_placeholder(0, app)
                .as_ref(),
            "参数名"
        );
        assert_eq!(
            tab.read(app)
                .headers
                .read(app)
                .key_placeholder(0, app)
                .as_ref(),
            "Header 名"
        );
    });

    cx.update(|_, cx| settings::update(cx, |s| s.language = LanguagePref::English));
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(app.global::<Locale>().0, "en");
        assert_eq!(
            tab.read(app)
                .params
                .read(app)
                .key_placeholder(0, app)
                .as_ref(),
            "Name"
        );
    });
}

#[gpui::test]
fn settings_dropdown_theme_change_needs_no_window(cx: &mut TestAppContext) {
    let (cx, store, _dir) = init_with_store(cx);
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    cx.update(|_, cx| ws.update(cx, |ws, cx| ws.set_theme_global(ThemePref::Dark, cx)));
    cx.read(|app| {
        assert_eq!(ws.read(app).theme(), ThemePref::Dark);
        assert!(app.theme().mode.is_dark());
    });
    assert!(store.flush());
    assert_eq!(read_workspace(&store).unwrap().theme, ThemePref::Dark);
}

/// ⌘, 的按键串必须能被 gpui 解析，否则 `bind_keys` 会在启动时 panic。
#[test]
fn settings_shortcut_keystroke_parses() {
    for ks in ["cmd-,", "ctrl-,"] {
        assert!(gpui::Keystroke::parse(ks).is_ok(), "{ks}");
    }
}

// ---------------------------------------------------------------------------
// 应用内更新：用假源驱动 gpui-updater，不碰网络
// ---------------------------------------------------------------------------

/// 返回固定结果的 `UpdateSource`；闭包每次调用构造新值（`Error` 不是 Clone）。
struct FakeSource(Box<dyn Fn() -> gpui_updater::Result<gpui_updater::Release> + Send + Sync>);

impl gpui_updater::UpdateSource for FakeSource {
    fn fetch_latest(&self) -> gpui_updater::Result<gpui_updater::Release> {
        (self.0)()
    }
}

/// 带签名与校验和声明的 release：`Verification::Strict` 在检查阶段只验"有没有"，不访问网络。
fn fake_release(version: gpui_updater::Version) -> gpui_updater::Release {
    gpui_updater::Release {
        version,
        notes: None,
        asset: gpui_updater::Asset {
            name: "GetCat-macos-arm64.dmg".into(),
            url: "https://example.invalid/GetCat-macos-arm64.dmg".into(),
            size: 0,
        },
        signature: Some("untrusted comment: test\nRUSTtest".into()),
        signature_url: None,
        sha256: Some("00".repeat(32)),
    }
}

fn install_fake_updater(
    cx: &mut VisualTestContext,
    kind: InstallKind,
    fetch: impl Fn() -> gpui_updater::Result<gpui_updater::Release> + Send + Sync + 'static,
) {
    cx.update(|_, cx| {
        update::install_with_source(
            cx,
            FakeSource(Box::new(fetch)),
            update::engine_config(),
            kind,
        )
    });
}

fn v(major: u64) -> gpui_updater::Version {
    gpui_updater::Version::new(major, 0, 0)
}

#[gpui::test]
async fn update_check_surfaces_new_version_in_workspace(cx: &mut TestAppContext) {
    let cx = init(cx);
    install_fake_updater(cx, InstallKind::Installed, || Ok(fake_release(v(99))));
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    assert_eq!(
        cx.update(|_, cx| ws.read(cx).update_status().clone()),
        gpui_updater::UpdateStatus::Idle
    );

    cx.update(|_, cx| update::check(cx));
    cx.run_until_parked();

    assert_eq!(
        cx.update(|_, cx| update::status(cx)),
        gpui_updater::UpdateStatus::Available(v(99))
    );
    // Workspace 通过 observe 同步到了同一状态
    let status = cx.update(|_, cx| ws.read(cx).update_status().clone());
    assert_eq!(update::hint_version(&status), Some((&v(99), false)));
    assert!(cx.update(|_, cx| update::can_install(cx)));

    // 状态栏带提示时能画出一帧
    // 聚焦中的 URL 输入框在测试窗口里渲染会碰真实平台句柄（见 sidebar 测试的说明），先 blur
    cx.update(|window, _| window.blur());
    let ws_element = ws.clone();
    cx.draw(point(px(0.), px(0.)), size(px(1000.), px(800.)), |_, _| {
        ws_element.into_any_element()
    });
}

#[gpui::test]
async fn update_check_up_to_date_has_no_hint(cx: &mut TestAppContext) {
    let cx = init(cx);
    install_fake_updater(cx, InstallKind::Installed, || Ok(fake_release(v(0))));
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));

    cx.update(|_, cx| update::check(cx));
    cx.run_until_parked();

    assert_eq!(
        cx.update(|_, cx| update::status(cx)),
        gpui_updater::UpdateStatus::UpToDate
    );
    let status = cx.update(|_, cx| ws.read(cx).update_status().clone());
    assert_eq!(update::hint_version(&status), None);
    assert!(!cx.update(|_, cx| update::can_install(cx)));
}

#[gpui::test]
async fn update_check_error_is_surfaced(cx: &mut TestAppContext) {
    let cx = init(cx);
    install_fake_updater(cx, InstallKind::Installed, || {
        Err(gpui_updater::Error::Http("offline".into()))
    });
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));

    cx.update(|_, cx| update::check(cx));
    cx.run_until_parked();

    let status = cx.update(|_, cx| update::status(cx));
    assert!(
        matches!(&status, gpui_updater::UpdateStatus::Errored(msg) if msg.contains("offline")),
        "{status:?}"
    );
    assert_eq!(update::hint_version(&status), None);
    assert_eq!(
        cx.update(|_, cx| ws.read(cx).update_status().clone()),
        status
    );
    // 离线启动不能把状态栏搞坏
    cx.update(|window, _| window.blur());
    let ws_element = ws.clone();
    cx.draw(point(px(0.), px(0.)), size(px(1000.), px(800.)), |_, _| {
        ws_element.into_any_element()
    });
}

#[gpui::test]
async fn launch_check_respects_setting(cx: &mut TestAppContext) {
    let _locale = crate::i18n::locale_test_lock();
    let cx = init(cx);
    install_fake_updater(cx, InstallKind::Installed, || Ok(fake_release(v(99))));
    cx.update(|_, cx| {
        settings::install(
            cx,
            Some(AppSettings {
                check_updates_on_launch: false,
                ..Default::default()
            }),
        )
    });

    cx.update(|_, cx| update::schedule_launch_check(cx));
    cx.executor()
        .advance_clock(update::LAUNCH_CHECK_DELAY + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| update::status(cx)),
        gpui_updater::UpdateStatus::Idle,
        "关闭开关后启动不应检查"
    );

    cx.update(|_, cx| settings::update(cx, |s| s.check_updates_on_launch = true));
    cx.update(|_, cx| update::schedule_launch_check(cx));
    // 延迟未到：仍未开始
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| update::status(cx)),
        gpui_updater::UpdateStatus::Idle
    );
    cx.executor()
        .advance_clock(update::LAUNCH_CHECK_DELAY + Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| update::status(cx)),
        gpui_updater::UpdateStatus::Available(v(99))
    );
}

#[gpui::test]
async fn dev_builds_can_check_but_not_install(cx: &mut TestAppContext) {
    let cx = init(cx);
    install_fake_updater(cx, InstallKind::DevBuild, || Ok(fake_release(v(99))));

    cx.update(|_, cx| update::check(cx));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| update::status(cx)),
        gpui_updater::UpdateStatus::Available(v(99))
    );
    assert!(!cx.update(|_, cx| update::can_install(cx)));

    cx.update(|_, cx| update::download_and_install(cx));
    cx.run_until_parked();
    // 没有进入 Downloading / Errored：开发构建直接拒绝安装
    assert_eq!(
        cx.update(|_, cx| update::status(cx)),
        gpui_updater::UpdateStatus::Available(v(99))
    );
}

#[gpui::test]
async fn unsupported_platform_has_no_updater(cx: &mut TestAppContext) {
    let cx = init(cx);
    assert!(!cx.update(|_, cx| update::supported(cx)));
    assert_eq!(
        cx.update(|_, cx| update::status(cx)),
        gpui_updater::UpdateStatus::Idle
    );
    // 没有更新器时这些动作都是空操作，不能 panic
    cx.update(|_, cx| {
        update::check(cx);
        update::download_and_install(cx);
        update::schedule_launch_check(cx);
    });
    cx.run_until_parked();
    let ws = cx.update(|window, cx| cx.new(|cx| Workspace::new(window, cx)));
    assert_eq!(
        cx.update(|_, cx| ws.read(cx).update_status().clone()),
        gpui_updater::UpdateStatus::Idle
    );
}
