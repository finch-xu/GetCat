//! 响应状态机与"已准备好可直接渲染"的响应视图（三档选档在后台线程完成）。

use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use getcat_core::body::pretty::pretty_json_cancellable;
use getcat_core::body::spill::HEAD_BYTES;
use getcat_core::body::text::{TextDoc, trim_partial_utf8};
use getcat_core::body::tier::{ViewTier, select_tier};
use getcat_core::detect::{ContentKind, SNIFF_LEN, detect};
use getcat_core::http::{BodyStore, RequestError};
use getcat_core::model::ResponseMeta;
use getcat_core::sse::{self, LlmStream, SseParser, Usage};
use gpui::{SharedString, Task};

/// 一份可渲染的文本：A 档交给 Editor（整段文本），B/C 档交给虚拟列表（按行切片）。
/// 通过 `Arc<TextDoc>` 与渲染闭包共享，每帧只 clone Arc。
#[derive(Debug, Clone)]
pub struct PreparedDoc {
    pub doc: Arc<TextDoc>,
    pub tier: ViewTier,
}

impl PreparedDoc {
    fn new(doc: TextDoc, spilled: bool) -> PreparedDoc {
        let tier = if spilled {
            ViewTier::Preview
        } else {
            select_tier(doc.len_bytes(), doc.line_count())
        };
        PreparedDoc {
            doc: Arc::new(doc),
            tier,
        }
    }

    /// Editor 用的整段文本：只在 A 档（≤ 5 MiB）写入编辑器时调用，拷贝一次。
    pub fn shared_text(&self) -> SharedString {
        SharedString::from(self.doc.text().to_string())
    }
}

pub struct ResponseView {
    pub meta: ResponseMeta,
    pub kind: ContentKind,
    /// 原始文本；二进制内容为 None
    pub raw: Option<PreparedDoc>,
    /// 美化后的 JSON；非 JSON 或落盘响应为 None
    pub pretty: Option<PreparedDoc>,
    /// SSE 响应的事件视图（Content-Type 为 text/event-stream 且未落盘时）
    pub sse: Option<SseView>,
    /// 响应头的渲染行：后台一次性转成 SharedString，渲染时只 clone Arc
    pub header_rows: Arc<[(SharedString, SharedString)]>,
}

/// SSE 响应完成后的结构化视图：整份 body 在后台重放解析得到。
///
/// 不搬运在途 `SseLive` 的解析结果而是重放，是为了让"边收边看"与
/// "一次性收到"两条路径产出完全一致；代价是一次 O(n) 重解析（可取消）。
/// 唯一只有在途才知道的信息是 `first_delta`（TTFT），由 `apply_outcome`
/// 从旧 InFlight 状态里合并进来。
pub struct SseView {
    /// 事件行 `(event 名, data)`：event 未指定时为空串，渲染层自行标注。
    pub events: Arc<[(SharedString, SharedString)]>,
    /// 按大模型流格式拼装出的完整文本；识别不出 delta 时为 None。
    pub text: Option<PreparedDoc>,
    pub usage: Usage,
    /// 首个内容 delta 的到达耗时（TTFT）。
    pub first_delta: Option<Duration>,
}

impl SseView {
    fn prepare(body: &[u8], mut should_cancel: impl FnMut() -> bool) -> Option<SseView> {
        if should_cancel() {
            return None;
        }
        let events = sse::parse_all(body);
        let mut stream = LlmStream::new();
        for ev in &events {
            stream.push(ev);
        }
        if should_cancel() {
            return None;
        }
        let text = if stream.has_text() {
            let doc =
                TextDoc::from_bytes_cancellable(stream.text.into_bytes(), &mut should_cancel)?;
            Some(PreparedDoc::new(doc, false))
        } else {
            None
        };
        let rows: Arc<[(SharedString, SharedString)]> = events
            .into_iter()
            .map(|ev| {
                (
                    SharedString::from(ev.event.unwrap_or_default()),
                    SharedString::from(ev.data),
                )
            })
            .collect();
        Some(SseView {
            events: rows,
            text,
            usage: stream.usage,
            first_delta: None,
        })
    }
}

impl ResponseView {
    /// 不可取消的同步入口；只在测试里用（生产路径一律走 `prepare_cancellable`）。
    #[cfg(test)]
    pub fn prepare(meta: ResponseMeta, body: &BodyStore) -> ResponseView {
        Self::prepare_cancellable(meta, body, || false).expect("never cancelled")
    }

    /// 在后台线程调用：所有 O(n) 工作（嗅探、美化、UTF-8 转换、建索引）都在这里完成。
    /// `should_cancel` 在每个阶段开头与美化 / 建索引的每 1 MiB 检查点被调用，返回 true 则整体放弃（None）。
    pub fn prepare_cancellable(
        meta: ResponseMeta,
        body: &BodyStore,
        mut should_cancel: impl FnMut() -> bool,
    ) -> Option<ResponseView> {
        let kind = detect(meta.content_type.as_deref(), body.head(SNIFF_LEN));
        let header_rows: Arc<[(SharedString, SharedString)]> = meta
            .headers
            .iter()
            .map(|(k, v)| (SharedString::from(k.clone()), SharedString::from(v.clone())))
            .collect();
        if !kind.is_text() {
            return Some(ResponseView {
                meta,
                kind,
                raw: None,
                pretty: None,
                sse: None,
                header_rows,
            });
        }
        match body.memory() {
            Some(bytes) => {
                let pretty = if kind == ContentKind::Json {
                    let out = pretty_json_cancellable(bytes, &mut should_cancel)?;
                    Some(PreparedDoc::new(
                        TextDoc::from_bytes_cancellable(out, &mut should_cancel)?,
                        false,
                    ))
                } else {
                    None
                };
                let sse = if sse::is_sse(meta.content_type.as_deref()) {
                    Some(SseView::prepare(bytes, &mut should_cancel)?)
                } else {
                    None
                };
                let raw = PreparedDoc::new(
                    TextDoc::from_bytes_cancellable(bytes.to_vec(), &mut should_cancel)?,
                    false,
                );
                Some(ResponseView {
                    meta,
                    kind,
                    raw: Some(raw),
                    pretty,
                    sse,
                    header_rows,
                })
            }
            None => {
                // 落盘：不美化，只对前 HEAD_BYTES 建索引（C 档）；切口可能落在一个多字节字符中间，先裁掉残缺的尾巴
                let head = trim_partial_utf8(body.head(HEAD_BYTES)).to_vec();
                let raw = PreparedDoc::new(
                    TextDoc::from_bytes_cancellable(head, &mut should_cancel)?,
                    true,
                );
                Some(ResponseView {
                    meta,
                    kind,
                    raw: Some(raw),
                    pretty: None,
                    sse: None,
                    header_rows,
                })
            }
        }
    }

    /// 当前应显示的文档：要 Pretty 且有 Pretty 时给 Pretty，否则给 Raw；二进制为 None。
    pub fn doc(&self, pretty: bool) -> Option<&PreparedDoc> {
        if pretty {
            self.pretty.as_ref().or(self.raw.as_ref())
        } else {
            self.raw.as_ref()
        }
    }

    pub fn has_pretty(&self) -> bool {
        self.pretty.is_some()
    }

    /// 落盘响应：raw 只是前 HEAD_BYTES 的预览。`response_pane.rs` 据此决定是否显示
    /// "查看完整响应"的入口提示。
    pub fn is_preview(&self) -> bool {
        matches!(
            self.raw,
            Some(PreparedDoc {
                tier: ViewTier::Preview,
                ..
            })
        )
    }
}

/// 后台准备失败时的用户可见前缀（完整文案 `Background processing failed: <panic 信息>`；
/// 它是 `RequestError::Other` 的载荷，界面上按技术细节原样显示）。
pub(crate) const PREPARE_PANIC_PREFIX: &str = "Background processing failed";

/// 后台线程的总入口：`catch_unwind` 包住全部 O(n) 工作（spec §11：后台 panic 不得传播到主线程）。
/// - `None`：已取消，调用方什么都不回写；
/// - `Some(Err(Other("后台处理异常：…")))`：准备阶段 panic，Tab 显示为失败而不是永远停在"发送中"。
pub(crate) fn prepare_guarded(
    meta: ResponseMeta,
    body: BodyStore,
    should_cancel: impl FnMut() -> bool,
) -> Option<Result<(BodyStore, ResponseView), RequestError>> {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        ResponseView::prepare_cancellable(meta, &body, should_cancel)
    }));
    match outcome {
        Ok(Some(view)) => Some(Ok((body, view))),
        Ok(None) => None,
        Err(payload) => {
            let message = panic_message(payload.as_ref());
            tracing::error!("response preparation panicked: {message}");
            Some(Err(RequestError::Other(format!(
                "{PREPARE_PANIC_PREFIX}: {message}"
            ))))
        }
    }
}

/// `panic!` 的载荷通常是 `&str` 或 `String`；其它类型给一个固定文案。
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// 后台准备阶段的取消旗标：随 `InFlight` 一起被 drop（取消、重发、关 Tab）时自动置位，
/// 正在运行的美化 / 建索引会在下一个 1 MiB 检查点退出。
#[derive(Debug)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> CancelFlag {
        CancelFlag(Arc::new(AtomicBool::new(false)))
    }

    pub fn handle(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

impl Default for CancelFlag {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CancelFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// SSE 在途的实时状态：每个 `StreamEvent::Chunk` 到达时增量解析、拼装、记时。
///
/// 这是"收到就展示"的数据源——渲染层在 InFlight 期间直接读它。
/// 解析工作在主线程完成：SSE 分片小（一个大模型 token 几十字节），
/// 单块解析是 O(chunk) 的，远低于一帧的预算。
#[derive(Default)]
pub struct SseLive {
    parser: SseParser,
    /// delta 拼装与 usage 收集（`stream.text` 就是实时展示的正文）。
    pub stream: LlmStream,
    /// 已派发的事件数（实时状态行展示）。
    pub event_count: usize,
    /// 首个非空内容 delta 的到达耗时（TTFT）；完成后由 `apply_outcome` 并入 `SseView`。
    pub first_delta: Option<Duration>,
    /// 原始 body 字节：识别不出 delta 的流（如 MCP）退回原文展示。
    raw: Vec<u8>,
}

/// 在途累积的字节上限。正常大模型流远小于此；这是对"错标成 text/event-stream
/// 的超大响应"的保险——body 侧超过 64 MiB 会落盘，而 live 侧全在内存，必须自己设界。
/// 到顶后停止解析与累积，完成后照常走 Done 的完整视图 / 落盘预览路径。
pub(crate) const LIVE_CAP_BYTES: usize = 8 * 1024 * 1024;

impl SseLive {
    /// 喂入一个 body 分片；`elapsed` 是分片到达时相对请求发起的耗时。
    pub fn push(&mut self, chunk: &[u8], elapsed: Duration) {
        if self.raw.len() >= LIVE_CAP_BYTES {
            return;
        }
        self.raw.extend_from_slice(chunk);
        for ev in self.parser.push(chunk) {
            self.event_count += 1;
            if self.stream.push(&ev) && self.first_delta.is_none() {
                self.first_delta = Some(elapsed);
            }
        }
    }

    /// 实时展示的正文：拼装出了文本给文本，否则给原始流（裁掉半个字符的尾巴）。
    pub fn display_text(&self) -> SharedString {
        if self.stream.has_text() {
            SharedString::from(self.stream.text.clone())
        } else {
            SharedString::from(String::from_utf8_lossy(trim_partial_utf8(&self.raw)).into_owned())
        }
    }
}

pub enum ResponseState {
    Idle,
    InFlight {
        /// 请求发起时刻；状态行每 TICK_INTERVAL 重绘一次以显示实时耗时。
        started: Instant,
        received: u64,
        total: Option<u64>,
        /// SSE 响应的实时解析状态；`Head` 事件确认是 SSE 后建立。
        live: Option<SseLive>,
        /// 持有进度任务、计时任务与完成任务；状态被替换即 drop → 底层 tokio 任务 abort。
        _tasks: Vec<Task<()>>,
        /// 随状态一起 drop → 后台准备阶段在下一个检查点退出。
        _cancel: CancelFlag,
    },
    Done {
        /// 保留原始存储：`save_body` / `open_body_with_system` 需要它读回完整字节，
        /// 且 `Spilled` 的临时文件守卫必须随 Done 状态一起存活，否则文件会被 drop 删除。
        body: BodyStore,
        view: ResponseView,
    },
    Failed {
        error: RequestError,
    },
}

impl ResponseState {
    pub fn is_in_flight(&self) -> bool {
        matches!(self, ResponseState::InFlight { .. })
    }

    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        matches!(self, ResponseState::Done { .. })
    }

    #[cfg(test)]
    pub fn error(&self) -> Option<&RequestError> {
        match self {
            ResponseState::Failed { error } => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use getcat_core::body::spill::SpillFile;
    use getcat_core::body::tier::{EDITOR_MAX_BYTES, EDITOR_MAX_LINES};
    use std::time::Duration;

    fn meta(ct: Option<&str>, len: u64) -> ResponseMeta {
        ResponseMeta {
            status: 200,
            status_text: "OK".into(),
            headers: vec![("x-a".into(), "1".into()), ("x-b".into(), "2".into())],
            duration: Duration::from_millis(1),
            ttfb: None,
            body_len: len,
            content_type: ct.map(str::to_string),
            http_version: None,
            certificate: None,
        }
    }

    fn mem(body: &[u8]) -> BodyStore {
        BodyStore::in_memory(body.to_vec())
    }

    fn prepare(ct: Option<&str>, body: &[u8]) -> ResponseView {
        ResponseView::prepare(meta(ct, body.len() as u64), &mem(body))
    }

    #[test]
    fn small_json_gets_editor_tier_with_pretty() {
        let v = prepare(Some("application/json"), br#"{"a":1}"#);
        assert_eq!(v.kind, ContentKind::Json);
        let raw = v.raw.as_ref().unwrap();
        let pretty = v.pretty.as_ref().unwrap();
        assert_eq!(raw.tier, ViewTier::Editor);
        assert_eq!(pretty.tier, ViewTier::Editor);
        assert_eq!(raw.doc.text(), r#"{"a":1}"#);
        assert_eq!(pretty.doc.text(), "{\n  \"a\": 1\n}");
        assert_eq!(pretty.doc.line_count(), 3);
        assert!(v.has_pretty());
        assert!(!v.is_preview());
        assert_eq!(v.doc(true).unwrap().doc.text(), pretty.doc.text());
        assert_eq!(v.doc(false).unwrap().doc.text(), raw.doc.text());
    }

    #[test]
    fn text_has_no_pretty_and_doc_falls_back_to_raw() {
        let v = prepare(Some("text/plain"), b"hi");
        assert!(v.pretty.is_none());
        assert!(!v.has_pretty());
        assert_eq!(v.doc(true).unwrap().doc.text(), "hi");
    }

    #[test]
    fn oversized_text_switches_to_virtual_tier() {
        let big = vec![b'x'; EDITOR_MAX_BYTES + 1];
        let v = prepare(Some("text/plain"), &big);
        assert_eq!(v.raw.as_ref().unwrap().tier, ViewTier::Virtual);
        assert_eq!(v.raw.as_ref().unwrap().doc.line_count(), 1);
    }

    #[test]
    fn too_many_lines_switch_to_virtual_tier() {
        let many = "a\n".repeat(EDITOR_MAX_LINES + 1);
        let v = prepare(Some("text/plain"), many.as_bytes());
        assert_eq!(v.raw.as_ref().unwrap().tier, ViewTier::Virtual);
        assert_eq!(
            v.raw.as_ref().unwrap().doc.line_count(),
            EDITOR_MAX_LINES + 1
        );
    }

    #[test]
    fn pretty_and_raw_choose_tiers_independently() {
        // 紧凑 JSON 一行 500 KB → raw 是 A 档；美化后 25 万行 → pretty 是 B 档
        let compact = format!("[{}1]", "1,".repeat(250_000));
        let v = prepare(Some("application/json"), compact.as_bytes());
        assert_eq!(v.raw.as_ref().unwrap().tier, ViewTier::Editor);
        assert_eq!(v.pretty.as_ref().unwrap().tier, ViewTier::Virtual);
    }

    #[test]
    fn spilled_body_is_preview_of_head_only() {
        let (guard, _file) = SpillFile::create().unwrap();
        let body = BodyStore::Spilled {
            file: Arc::new(guard),
            len: 100 * 1024 * 1024,
            head: Arc::from(&br#"{"a":1}"#[..]),
        };
        let v = ResponseView::prepare(meta(Some("application/json"), 100 * 1024 * 1024), &body);
        assert!(v.is_preview());
        assert!(
            v.pretty.is_none(),
            "spilled bodies are never pretty-printed"
        );
        let raw = v.raw.as_ref().unwrap();
        assert_eq!(raw.tier, ViewTier::Preview);
        assert_eq!(raw.doc.text(), r#"{"a":1}"#);
    }

    #[test]
    fn binary_body_has_no_docs() {
        let v = prepare(Some("image/png"), b"\x89PNG\0");
        assert_eq!(v.kind, ContentKind::Binary);
        assert!(v.raw.is_none() && v.pretty.is_none());
        assert!(v.doc(true).is_none());
    }

    #[test]
    fn header_rows_are_prepared_once() {
        let v = prepare(None, b"x");
        assert_eq!(v.header_rows.len(), 2);
        assert_eq!(v.header_rows[0].0.as_ref(), "x-a");
        assert_eq!(v.header_rows[1].1.as_ref(), "2");
    }

    #[test]
    fn cancellation_aborts_preparation() {
        let body = mem(br#"{"a":1}"#);
        assert!(
            ResponseView::prepare_cancellable(meta(Some("application/json"), 7), &body, || true)
                .is_none()
        );
    }

    #[test]
    fn cancel_flag_is_raised_on_drop() {
        let flag = CancelFlag::new();
        let handle = flag.handle();
        assert!(!handle.load(Ordering::Relaxed));
        drop(flag);
        assert!(handle.load(Ordering::Relaxed));
    }

    #[test]
    fn spilled_head_is_trimmed_at_a_utf8_boundary() {
        let (guard, _file) = SpillFile::create().unwrap();
        let full = "名名".as_bytes();
        let body = BodyStore::Spilled {
            file: Arc::new(guard),
            len: 100 * 1024 * 1024,
            // head 的切口落在第二个字中间
            head: Arc::from(&full[..full.len() - 1]),
        };
        let v = ResponseView::prepare(meta(Some("text/plain"), 100 * 1024 * 1024), &body);
        assert_eq!(v.raw.as_ref().unwrap().doc.text(), "名");
    }

    #[test]
    fn sse_response_gets_event_view_with_text_and_usage() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n",
        );
        let v = prepare(Some("text/event-stream; charset=utf-8"), body.as_bytes());
        assert_eq!(v.kind, ContentKind::Text);
        let sse = v.sse.as_ref().expect("SSE 响应必须有事件视图");
        assert_eq!(sse.events.len(), 4);
        let text = sse.text.as_ref().expect("拼装文本");
        assert_eq!(text.doc.text(), "Hello");
        assert_eq!(sse.usage.input_tokens, Some(3));
        assert_eq!(sse.usage.output_tokens, Some(2));
        // TTFT 是时序事实，重放解析不出来，由 apply_outcome 合并
        assert_eq!(sse.first_delta, None);
        // 原始视图照常存在，事件视图是额外的
        assert!(v.raw.is_some());
    }

    #[test]
    fn sse_without_recognizable_deltas_has_events_but_no_text() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"result\":{}}\n\n";
        let v = prepare(Some("text/event-stream"), body.as_bytes());
        let sse = v.sse.as_ref().unwrap();
        assert_eq!(sse.events.len(), 1);
        assert_eq!(sse.events[0].0.as_ref(), "message");
        assert!(sse.text.is_none());
        assert!(sse.usage.is_empty());
    }

    #[test]
    fn non_sse_text_has_no_sse_view() {
        let v = prepare(Some("application/json"), br#"{"a":1}"#);
        assert!(v.sse.is_none());
    }

    #[test]
    fn sse_live_assembles_incrementally_and_records_first_delta() {
        let mut live = SseLive::default();
        // 第一块只有半个事件：还拼不出任何东西
        live.push(
            b"data: {\"choices\":[{\"delta\":{\"cont",
            Duration::from_millis(10),
        );
        assert_eq!(live.event_count, 0);
        assert_eq!(live.first_delta, None);
        // 补齐后事件派发、文本出现、TTFT 定格在本块的时刻
        live.push(
            b"ent\":\"Hi\"}}]}\n\ndata: [DONE]\n\n",
            Duration::from_millis(30),
        );
        assert_eq!(live.event_count, 2);
        assert_eq!(live.first_delta, Some(Duration::from_millis(30)));
        assert_eq!(live.display_text().as_ref(), "Hi");
    }

    #[test]
    fn sse_live_stops_accumulating_at_the_cap() {
        let mut live = SseLive::default();
        // 一块直接越过上限：允许略超（按块判断），但之后不再增长
        live.push(&vec![b'x'; LIVE_CAP_BYTES + 1], Duration::from_millis(1));
        let len = live.display_text().len();
        live.push(b"data: more\n\n", Duration::from_millis(2));
        assert_eq!(live.display_text().len(), len, "到顶后不再累积");
        assert_eq!(live.event_count, 0, "到顶后不再解析事件");
    }

    #[test]
    fn sse_live_falls_back_to_raw_text_when_no_delta() {
        let mut live = SseLive::default();
        live.push("data: 你好\n\n".as_bytes(), Duration::from_millis(5));
        assert_eq!(live.event_count, 1);
        assert!(live.first_delta.is_none());
        // 拼不出 delta：原样展示已收到的流
        assert_eq!(live.display_text().as_ref(), "data: 你好\n\n");
        // 跨界的多字节字符不会以乱码出现
        let mut partial = SseLive::default();
        let bytes = "data: 名".as_bytes();
        partial.push(&bytes[..bytes.len() - 1], Duration::from_millis(1));
        assert_eq!(partial.display_text().as_ref(), "data: ");
    }

    #[test]
    fn prepare_guarded_turns_a_panic_into_failed() {
        let body = mem(br#"{"a":1}"#);
        let result = prepare_guarded(meta(Some("application/json"), 7), body, || {
            panic!("boom in background")
        });
        match result {
            Some(Err(RequestError::Other(msg))) => {
                assert!(msg.starts_with(PREPARE_PANIC_PREFIX), "{msg}");
                assert!(msg.contains("boom in background"), "{msg}");
            }
            other => panic!(
                "expected Some(Err(Other)), got {:?}",
                other.map(|r| r.map(|_| ()))
            ),
        }
        // 正常路径与取消路径不受影响
        assert!(matches!(
            prepare_guarded(
                meta(Some("application/json"), 7),
                mem(br#"{"a":1}"#),
                || false
            ),
            Some(Ok(_))
        ));
        assert!(prepare_guarded(meta(None, 2), mem(b"{}"), || true).is_none());
    }
}
