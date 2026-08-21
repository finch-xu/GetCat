//! 响应状态机与"已准备好可直接渲染"的响应视图（三档选档在后台线程完成）。

use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use getcat_core::body::pretty::pretty_json_cancellable;
use getcat_core::body::spill::HEAD_BYTES;
use getcat_core::body::text::{TextDoc, trim_partial_utf8};
use getcat_core::body::tier::{ViewTier, select_tier};
use getcat_core::detect::{ContentKind, SNIFF_LEN, detect};
use getcat_core::http::{BodyStore, RequestError};
use getcat_core::model::ResponseMeta;
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
    /// 响应头的渲染行：后台一次性转成 SharedString，渲染时只 clone Arc
    pub header_rows: Arc<[(SharedString, SharedString)]>,
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
                let raw = PreparedDoc::new(
                    TextDoc::from_bytes_cancellable(bytes.to_vec(), &mut should_cancel)?,
                    false,
                );
                Some(ResponseView {
                    meta,
                    kind,
                    raw: Some(raw),
                    pretty,
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

/// 后台准备失败时的用户可见前缀（完整文案 `后台处理异常：<panic 信息>`）。
pub(crate) const PREPARE_PANIC_PREFIX: &str = "后台处理异常";

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
                "{PREPARE_PANIC_PREFIX}：{message}"
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

pub enum ResponseState {
    Idle,
    InFlight {
        /// 请求发起时刻；状态行每 TICK_INTERVAL 重绘一次以显示实时耗时。
        started: Instant,
        received: u64,
        total: Option<u64>,
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
            body_len: len,
            content_type: ct.map(str::to_string),
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
