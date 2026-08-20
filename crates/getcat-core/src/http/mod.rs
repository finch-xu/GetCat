//! HTTP 引擎：把 RequestDraft 变成 reqwest 请求，流式接收响应并上报进度。

mod error;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::BytesMut;
use futures::StreamExt;
use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use tokio::sync::mpsc;
use url::Url;

pub use error::RequestError;

use crate::model::{BodyKind, Method, RequestDraft, ResponseMeta};
use crate::url::build_url;

pub type Client = reqwest::Client;

pub const USER_AGENT_VALUE: &str = concat!("GetCat/", env!("CARGO_PKG_VERSION"));
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(33);
/// 响应体驻留内存的上限；超出即报 `RequestError::TooLarge`。
/// Plan 2 落盘（`BodyStore::Spilled`）后，超限分支会改成溢出到磁盘而不是报错。
pub const MAX_BODY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundBody {
    Empty,
    Bytes { content_type: String, data: Vec<u8> },
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: OutboundBody,
}

/// 响应体存储。Plan 2 会增加落盘变体。
#[derive(Debug, Clone)]
pub enum BodyStore {
    Memory(Arc<[u8]>),
}

impl BodyStore {
    pub fn len(&self) -> u64 {
        match self {
            BodyStore::Memory(b) => b.len() as u64,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            BodyStore::Memory(b) => b,
        }
    }

    pub fn head(&self, n: usize) -> &[u8] {
        let bytes = self.as_bytes();
        &bytes[..bytes.len().min(n)]
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub meta: ResponseMeta,
    pub body: BodyStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub received: u64,
    pub total: Option<u64>,
    pub elapsed: Duration,
}

pub fn build_client() -> Client {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(USER_AGENT_VALUE)
        .build()
        .expect("reqwest client")
}

pub fn prepare(draft: &RequestDraft) -> Result<HttpRequest, RequestError> {
    let url = build_url(draft).map_err(|e| RequestError::InvalidUrl(e.to_string()))?;

    let mut headers = Vec::new();
    for h in draft
        .headers
        .iter()
        .filter(|h| h.enabled && !h.key.trim().is_empty())
    {
        let key = h.key.trim();
        let value = h.value.trim();
        HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| RequestError::InvalidHeader(key.to_string()))?;
        HeaderValue::from_str(value).map_err(|_| RequestError::InvalidHeader(key.to_string()))?;
        headers.push((key.to_string(), value.to_string()));
    }

    let body = match &draft.body {
        BodyKind::None => OutboundBody::Empty,
        BodyKind::Raw { format, text } => OutboundBody::Bytes {
            content_type: format.content_type().to_string(),
            data: text.as_bytes().to_vec(),
        },
        BodyKind::FormUrlEncoded { fields } => {
            let mut ser = url::form_urlencoded::Serializer::new(String::new());
            for f in fields.iter().filter(|f| f.enabled && !f.key.is_empty()) {
                ser.append_pair(&f.key, &f.value);
            }
            OutboundBody::Bytes {
                content_type: "application/x-www-form-urlencoded".to_string(),
                data: ser.finish().into_bytes(),
            }
        }
        BodyKind::File { .. } => {
            return Err(RequestError::Unsupported(
                "文件 Body 将在后续版本支持".to_string(),
            ));
        }
    };

    Ok(HttpRequest {
        method: draft.method,
        url,
        headers,
        body,
    })
}

fn to_reqwest_method(m: Method) -> reqwest::Method {
    match m {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
        Method::Put => reqwest::Method::PUT,
        Method::Patch => reqwest::Method::PATCH,
        Method::Delete => reqwest::Method::DELETE,
        Method::Head => reqwest::Method::HEAD,
        Method::Options => reqwest::Method::OPTIONS,
    }
}

pub async fn execute(
    client: &Client,
    req: HttpRequest,
    progress: Option<mpsc::Sender<Progress>>,
) -> Result<HttpResponse, RequestError> {
    execute_with_limit(client, req, progress, MAX_BODY_BYTES).await
}

/// `execute` 的参数化版本：`max_body` 为响应体字节上限（测试用小值）。
pub async fn execute_with_limit(
    client: &Client,
    req: HttpRequest,
    progress: Option<mpsc::Sender<Progress>>,
    max_body: u64,
) -> Result<HttpResponse, RequestError> {
    let started = Instant::now();
    let mut builder = client.request(to_reqwest_method(req.method), req.url.clone());

    let mut has_content_type = false;
    for (k, v) in &req.headers {
        if k.eq_ignore_ascii_case("content-type") {
            has_content_type = true;
        }
        builder = builder.header(k.as_str(), v.as_str());
    }
    if let OutboundBody::Bytes { content_type, data } = req.body {
        if !has_content_type {
            builder = builder.header(CONTENT_TYPE, content_type);
        }
        builder = builder.body(data);
    }

    let resp = builder.send().await?;
    let status = resp.status();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let total = resp.content_length();

    // Content-Length 已超限：直接放弃，不读取 body。
    // Plan 2 的落盘（BodyStore::Spilled）会替换这里的报错分支。
    if let Some(len) = total
        && len > max_body
    {
        return Err(RequestError::TooLarge(len));
    }

    let mut buf = BytesMut::with_capacity(total.unwrap_or(0).min(max_body) as usize);
    let mut stream = resp.bytes_stream();
    let mut last_report = Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);
        // 无 Content-Length（chunked / 压缩）或声明值不实时，仍需在累积中截断。
        // drop(stream) 会关闭连接，不会把剩余字节读完。
        // Plan 2 的落盘（BodyStore::Spilled）会替换这里的报错分支。
        if buf.len() as u64 > max_body {
            return Err(RequestError::TooLarge(buf.len() as u64));
        }
        if let Some(tx) = &progress
            && last_report.elapsed() >= PROGRESS_INTERVAL
        {
            let _ = tx.try_send(Progress {
                received: buf.len() as u64,
                total,
                elapsed: started.elapsed(),
            });
            last_report = Instant::now();
        }
    }
    let duration = started.elapsed();
    if let Some(tx) = &progress {
        let _ = tx.try_send(Progress {
            received: buf.len() as u64,
            total,
            elapsed: duration,
        });
    }

    let body: Arc<[u8]> = Arc::from(buf.freeze().as_ref());
    Ok(HttpResponse {
        meta: ResponseMeta {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("").to_string(),
            headers,
            duration,
            body_len: body.len() as u64,
            content_type,
        },
        body: BodyStore::Memory(body),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BodyKind, KeyValue, Method, RawFormat, RequestDraft};
    use std::io::Write;
    use wiremock::matchers::{body_string, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn draft(m: Method, url: String) -> RequestDraft {
        RequestDraft {
            method: m,
            url,
            ..Default::default()
        }
    }

    async fn run(d: &RequestDraft) -> Result<HttpResponse, RequestError> {
        let client = build_client();
        execute(&client, prepare(d)?, None).await
    }

    #[tokio::test]
    async fn get_returns_status_headers_and_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hello"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("hi")
                    .insert_header("x-test", "1")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&server)
            .await;
        let resp = run(&draft(Method::Get, format!("{}/hello", server.uri())))
            .await
            .unwrap();
        assert_eq!(resp.meta.status, 200);
        assert_eq!(resp.meta.status_text, "OK");
        assert_eq!(resp.body.as_bytes(), b"hi");
        assert_eq!(resp.meta.body_len, 2);
        assert_eq!(resp.meta.content_type.as_deref(), Some("text/plain"));
        assert!(
            resp.meta
                .headers
                .iter()
                .any(|(k, v)| k == "x-test" && v == "1")
        );
    }

    #[tokio::test]
    async fn post_raw_json_sets_content_type_and_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/items"))
            .and(header("content-type", "application/json"))
            .and(header("user-agent", USER_AGENT_VALUE))
            .and(body_string(r#"{"a":1}"#))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;
        let mut d = draft(Method::Post, format!("{}/items", server.uri()));
        d.body = BodyKind::Raw {
            format: RawFormat::Json,
            text: r#"{"a":1}"#.into(),
        };
        assert_eq!(run(&d).await.unwrap().meta.status, 201);
    }

    #[tokio::test]
    async fn user_content_type_overrides_default() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("content-type", "text/plain"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut d = draft(Method::Post, server.uri());
        d.headers = vec![KeyValue::new("Content-Type", "text/plain")];
        d.body = BodyKind::Raw {
            format: RawFormat::Json,
            text: "{}".into(),
        };
        assert_eq!(run(&d).await.unwrap().meta.status, 200);
    }

    #[tokio::test]
    async fn form_urlencoded_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string("a=1&b=x+y"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut d = draft(Method::Post, server.uri());
        d.body = BodyKind::FormUrlEncoded {
            fields: vec![
                KeyValue::new("a", "1"),
                KeyValue::new("b", "x y"),
                KeyValue {
                    key: "c".into(),
                    value: "off".into(),
                    enabled: false,
                },
            ],
        };
        assert_eq!(run(&d).await.unwrap().meta.status, 200);
    }

    #[tokio::test]
    async fn path_and_query_params_are_applied() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/7"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut d = draft(Method::Get, format!("{}/users/{{id}}", server.uri()));
        d.path_params = vec![KeyValue::new("id", "7")];
        d.params = vec![KeyValue::new("page", "2")];
        assert_eq!(run(&d).await.unwrap().meta.status, 200);
    }

    #[tokio::test]
    async fn gzip_is_transparently_decoded() {
        let server = MockServer::start().await;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(b"hello gzip").unwrap();
        let gz = enc.finish().unwrap();
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(gz)
                    .insert_header("content-encoding", "gzip"),
            )
            .mount(&server)
            .await;
        let resp = run(&draft(Method::Get, server.uri())).await.unwrap();
        assert_eq!(resp.body.as_bytes(), b"hello gzip");
    }

    #[tokio::test]
    async fn progress_reports_final_total() {
        let server = MockServer::start().await;
        let size = 1usize << 20;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'a'; size]))
            .mount(&server)
            .await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let client = build_client();
        let resp = execute(
            &client,
            prepare(&draft(Method::Get, server.uri())).unwrap(),
            Some(tx),
        )
        .await
        .unwrap();
        assert_eq!(resp.body.len(), size as u64);
        let mut last = None;
        while let Ok(p) = rx.try_recv() {
            last = Some(p);
        }
        let last = last.expect("at least one progress event");
        assert_eq!(last.received, size as u64);
    }

    #[tokio::test]
    async fn content_length_over_limit_is_rejected_without_reading_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'a'; 1024]))
            .mount(&server)
            .await;
        let client = build_client();
        let err = execute_with_limit(
            &client,
            prepare(&draft(Method::Get, server.uri())).unwrap(),
            None,
            512,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, RequestError::TooLarge(1024)), "{err:?}");
    }

    #[tokio::test]
    async fn streaming_over_limit_is_aborted() {
        // gzip 响应的 Content-Length 是压缩后的长度（且 reqwest 解压时 content_length() 为 None），
        // 因此这里走的是流式累积中的截断分支。
        let server = MockServer::start().await;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&vec![b'a'; 4096]).unwrap();
        let gz = enc.finish().unwrap();
        assert!(
            gz.len() < 512,
            "compressed payload must pass the header check"
        );
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(gz)
                    .insert_header("content-encoding", "gzip"),
            )
            .mount(&server)
            .await;
        let client = build_client();
        let err = execute_with_limit(
            &client,
            prepare(&draft(Method::Get, server.uri())).unwrap(),
            None,
            512,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, RequestError::TooLarge(n) if n > 512),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn connection_refused_is_classified() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = run(&draft(Method::Get, format!("http://127.0.0.1:{port}/")))
            .await
            .unwrap_err();
        assert!(matches!(err, RequestError::ConnectionRefused(_)), "{err:?}");
    }

    /// 回归：reqwest 会把 ` for url (<url>)` 拼进顶层错误文本，URL 里的
    /// "dns"/"tls"/"resolve" 等字样不得影响分类。
    #[tokio::test]
    async fn url_keywords_do_not_affect_classification() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = run(&draft(
            Method::Get,
            format!("http://127.0.0.1:{port}/dns/tls/resolve"),
        ))
        .await
        .unwrap_err();
        assert!(matches!(err, RequestError::ConnectionRefused(_)), "{err:?}");
    }

    #[tokio::test]
    async fn dns_failure_is_classified() {
        // 某些本地代理（如 Clash 的 fake-ip 模式）会把任意域名解析到 198.18.0.0/15，
        // 此时无法测到真正的 DNS 失败，跳过而不是误报。
        if tokio::net::lookup_host("nonexistent.invalid:80")
            .await
            .is_ok()
        {
            eprintln!("skipped: environment resolves nonexistent.invalid (fake-ip DNS?)");
            return;
        }
        let err = run(&draft(Method::Get, "http://nonexistent.invalid/".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, RequestError::Dns(_)), "{err:?}");
    }

    #[test]
    fn prepare_rejects_invalid_header_and_url() {
        let mut d = draft(Method::Get, "https://x.com".into());
        d.headers = vec![KeyValue::new("bad header", "x")];
        assert!(matches!(prepare(&d), Err(RequestError::InvalidHeader(_))));
        let d = draft(Method::Get, "".into());
        assert!(matches!(prepare(&d), Err(RequestError::InvalidUrl(_))));
    }

    #[test]
    fn prepare_skips_disabled_and_blank_headers() {
        let mut d = draft(Method::Get, "https://x.com".into());
        d.headers = vec![
            KeyValue::new("X-A", "1"),
            KeyValue {
                key: "X-B".into(),
                value: "2".into(),
                enabled: false,
            },
            KeyValue::new("  ", ""),
        ];
        let req = prepare(&d).unwrap();
        assert_eq!(req.headers, vec![("X-A".to_string(), "1".to_string())]);
    }

    #[test]
    fn prepare_rejects_file_body_for_now() {
        let mut d = draft(Method::Post, "https://x.com".into());
        d.body = BodyKind::File {
            path: "/tmp/x".into(),
            content_type: None,
        };
        assert!(matches!(prepare(&d), Err(RequestError::Unsupported(_))));
    }
}
