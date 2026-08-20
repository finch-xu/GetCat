//! 响应状态机与"已准备好可直接渲染"的响应视图。

use std::time::Instant;

use getcat_core::body::pretty::pretty_json;
use getcat_core::detect::{ContentKind, SNIFF_LEN, detect};
use getcat_core::http::{HttpResponse, RequestError};
use getcat_core::model::ResponseMeta;
use gpui::{SharedString, Task};

/// 本阶段文本视图的体积上限；超过只显示前 PREVIEW_BYTES。
pub const MAX_TEXT_BYTES: usize = 5 * 1024 * 1024;
pub const PREVIEW_BYTES: usize = 1024 * 1024;

pub struct ResponseView {
    pub meta: ResponseMeta,
    pub kind: ContentKind,
    pub raw: SharedString,
    pub pretty: Option<SharedString>,
    pub truncated: bool,
}

impl ResponseView {
    /// 在后台线程调用：所有 O(n) 工作都在这里完成。
    pub fn prepare(resp: HttpResponse) -> ResponseView {
        let kind = detect(resp.meta.content_type.as_deref(), resp.body.head(SNIFF_LEN));
        // 落盘响应暂时只用内存中的 head 当作"超长截断"预览；Task 5 换成 C 档摘要视图
        let (bytes, spilled) = match resp.body.memory() {
            Some(b) => (b, false),
            None => (resp.body.head(PREVIEW_BYTES), true),
        };

        if !kind.is_text() {
            return ResponseView {
                raw: format!(
                    "二进制内容（{} 字节），当前版本暂不支持预览",
                    resp.body.len()
                )
                .into(),
                pretty: None,
                truncated: false,
                kind,
                meta: resp.meta,
            };
        }

        if spilled || bytes.len() > MAX_TEXT_BYTES {
            let preview =
                String::from_utf8_lossy(&bytes[..bytes.len().min(PREVIEW_BYTES)]).into_owned();
            return ResponseView {
                raw: preview.into(),
                pretty: None,
                truncated: true,
                kind,
                meta: resp.meta,
            };
        }

        let raw: SharedString = String::from_utf8_lossy(bytes).into_owned().into();
        let pretty = (kind == ContentKind::Json).then(|| {
            String::from_utf8_lossy(&pretty_json(bytes))
                .into_owned()
                .into()
        });
        ResponseView {
            raw,
            pretty,
            truncated: false,
            kind,
            meta: resp.meta,
        }
    }

    pub fn text(&self, pretty: bool) -> SharedString {
        if pretty {
            self.pretty.clone().unwrap_or_else(|| self.raw.clone())
        } else {
            self.raw.clone()
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
        /// 持有进度任务、计时任务与完成任务；状态被替换即 drop → 底层 tokio 任务 abort。
        _tasks: Vec<Task<()>>,
    },
    Done(ResponseView),
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
        matches!(self, ResponseState::Done(_))
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
    use getcat_core::http::BodyStore;
    use getcat_core::model::ResponseMeta;
    use std::{sync::Arc, time::Duration};

    fn resp(ct: Option<&str>, body: &[u8]) -> HttpResponse {
        HttpResponse {
            meta: ResponseMeta {
                status: 200,
                status_text: "OK".into(),
                headers: vec![],
                duration: Duration::from_millis(1),
                body_len: body.len() as u64,
                content_type: ct.map(str::to_string),
            },
            body: BodyStore::Memory(Arc::from(body)),
        }
    }

    #[test]
    fn json_gets_pretty_text() {
        let v = ResponseView::prepare(resp(Some("application/json"), br#"{"a":1}"#));
        assert_eq!(v.kind, ContentKind::Json);
        assert_eq!(v.raw.as_ref(), r#"{"a":1}"#);
        assert_eq!(v.pretty.as_deref(), Some("{\n  \"a\": 1\n}"));
        assert!(!v.truncated);
    }

    #[test]
    fn text_has_no_pretty() {
        let v = ResponseView::prepare(resp(Some("text/plain"), b"hi"));
        assert_eq!(v.kind, ContentKind::Text);
        assert!(v.pretty.is_none());
    }

    #[test]
    fn oversized_body_is_truncated_to_preview() {
        let big = vec![b'x'; MAX_TEXT_BYTES + 1];
        let v = ResponseView::prepare(resp(Some("text/plain"), &big));
        assert!(v.truncated);
        assert_eq!(v.raw.len(), PREVIEW_BYTES);
        assert!(v.pretty.is_none());
    }

    #[test]
    fn binary_body_shows_placeholder() {
        let v = ResponseView::prepare(resp(Some("image/png"), b"\x89PNG\0"));
        assert_eq!(v.kind, ContentKind::Binary);
        assert!(v.raw.contains("二进制"));
    }
}
