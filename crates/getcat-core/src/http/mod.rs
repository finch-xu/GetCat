//! HTTP 引擎：把 RequestDraft 变成 reqwest 请求，流式接收响应并上报进度。

mod error;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderName, HeaderValue};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use url::Url;

pub use error::RequestError;

use crate::body::spill::{HEAD_BYTES, SpillFile};
use crate::model::{BodyKind, Method, RequestDraft, ResponseMeta};
use crate::url::build_url;

pub type Client = reqwest::Client;

pub const USER_AGENT_VALUE: &str = concat!("GetCat/", env!("CARGO_PKG_VERSION"));
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(33);
/// 响应体驻留内存的阈值；超过即落盘为 `BodyStore::Spilled`，没有总上限。
pub const MAX_BODY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundBody {
    Empty,
    Bytes {
        content_type: String,
        data: Vec<u8>,
    },
    /// 文件流式上传：发送时打开、按块读取，内容不进内存。
    File {
        path: PathBuf,
        content_type: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: OutboundBody,
}

/// 响应体存储。
#[derive(Debug, Clone)]
pub enum BodyStore {
    /// ≤ MAX_BODY_BYTES：全部在内存。`Bytes` 由接收缓冲 `freeze()` 而来，不再整体拷贝一次。
    Memory(Bytes),
    /// > MAX_BODY_BYTES：内容在临时文件，内存只保留前 HEAD_BYTES
    Spilled {
        file: Arc<SpillFile>,
        len: u64,
        head: Arc<[u8]>,
    },
}

impl BodyStore {
    /// 驻留内存的响应体；`Vec<u8>` / `&'static [u8]` 零拷贝接管（测试用，免去调用方导入 `bytes`）。
    pub fn in_memory(bytes: impl Into<Bytes>) -> BodyStore {
        BodyStore::Memory(bytes.into())
    }

    pub fn len(&self) -> u64 {
        match self {
            BodyStore::Memory(b) => b.len() as u64,
            BodyStore::Spilled { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_spilled(&self) -> bool {
        matches!(self, BodyStore::Spilled { .. })
    }

    /// 全部字节；仅 Memory 有。
    pub fn memory(&self) -> Option<&[u8]> {
        match self {
            BodyStore::Memory(b) => Some(&b[..]),
            BodyStore::Spilled { .. } => None,
        }
    }

    /// 前 `n` 字节：Memory 取切片；Spilled 取 head 的切片（最多 HEAD_BYTES）。
    pub fn head(&self, n: usize) -> &[u8] {
        let bytes: &[u8] = match self {
            BodyStore::Memory(b) => b,
            BodyStore::Spilled { head, .. } => head,
        };
        &bytes[..bytes.len().min(n)]
    }

    /// 落盘文件路径；仅 Spilled 有。
    pub fn path(&self) -> Option<&Path> {
        match self {
            BodyStore::Memory(_) => None,
            BodyStore::Spilled { file, .. } => Some(file.path()),
        }
    }
}

/// 接收端：先进内存，超过阈值时把已收内容写入临时文件并继续追加。
enum Sink {
    Memory(BytesMut),
    Disk {
        file: tokio::fs::File,
        guard: SpillFile,
        head: Vec<u8>,
        len: u64,
    },
}

fn spill_err(e: std::io::Error) -> RequestError {
    RequestError::Spill(e.to_string())
}

impl Sink {
    fn with_capacity(expected: Option<u64>, threshold: u64) -> Sink {
        Sink::Memory(BytesMut::with_capacity(
            expected.unwrap_or(0).min(threshold) as usize,
        ))
    }

    fn len(&self) -> u64 {
        match self {
            Sink::Memory(buf) => buf.len() as u64,
            Sink::Disk { len, .. } => *len,
        }
    }

    async fn push(&mut self, chunk: &[u8], threshold: u64) -> Result<(), RequestError> {
        let must_spill = matches!(
            self,
            Sink::Memory(buf) if (buf.len() + chunk.len()) as u64 > threshold
        );
        if must_spill {
            let Sink::Memory(buf) = std::mem::replace(self, Sink::Memory(BytesMut::new())) else {
                unreachable!("checked above");
            };
            let (guard, file) = SpillFile::create().map_err(spill_err)?;
            let mut file = tokio::fs::File::from_std(file);
            file.write_all(&buf).await.map_err(spill_err)?;
            let head = buf[..buf.len().min(HEAD_BYTES)].to_vec();
            *self = Sink::Disk {
                file,
                guard,
                head,
                len: buf.len() as u64,
            };
        }
        match self {
            Sink::Memory(buf) => buf.extend_from_slice(chunk),
            Sink::Disk {
                file, head, len, ..
            } => {
                file.write_all(chunk).await.map_err(spill_err)?;
                if head.len() < HEAD_BYTES {
                    let take = chunk.len().min(HEAD_BYTES - head.len());
                    head.extend_from_slice(&chunk[..take]);
                }
                *len += chunk.len() as u64;
            }
        }
        Ok(())
    }

    async fn finish(self) -> Result<BodyStore, RequestError> {
        match self {
            Sink::Memory(buf) => Ok(BodyStore::Memory(buf.freeze())),
            Sink::Disk {
                mut file,
                guard,
                head,
                len,
            } => {
                file.flush().await.map_err(spill_err)?;
                drop(file);
                Ok(BodyStore::Spilled {
                    file: Arc::new(guard),
                    len,
                    head: head.into(),
                })
            }
        }
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
        BodyKind::File { path, content_type } => {
            if path.as_os_str().is_empty() {
                return Err(RequestError::FileBody("未选择文件".to_string()));
            }
            OutboundBody::File {
                path: path.clone(),
                content_type: content_type.clone(),
            }
        }
    };

    Ok(HttpRequest {
        method: draft.method,
        url,
        headers,
        body,
    })
}

/// 按扩展名猜测文件 Body 的 Content-Type；未知类型用 application/octet-stream。
pub fn guess_content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("txt" | "log" | "md") => "text/plain",
        Some("csv") => "text/csv",
        Some("html" | "htm") => "text/html",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

fn file_err(path: &Path, e: std::io::Error) -> RequestError {
    RequestError::FileBody(format!("{}：{e}", path.display()))
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
    execute_with_threshold(client, req, progress, MAX_BODY_BYTES).await
}

/// `execute` 的参数化版本：`spill_threshold` 为驻留内存的字节上限（测试用小值触发落盘）。
pub(crate) async fn execute_with_threshold(
    client: &Client,
    req: HttpRequest,
    progress: Option<mpsc::Sender<Progress>>,
    spill_threshold: u64,
) -> Result<HttpResponse, RequestError> {
    let started = Instant::now();
    let mut builder = client.request(to_reqwest_method(req.method), req.url.clone());

    let mut has_content_type = false;
    for (k, v) in &req.headers {
        if k.eq_ignore_ascii_case("content-type") {
            has_content_type = true;
        }
        // Content-Length / Transfer-Encoding / Host 由 reqwest/hyper 根据实际 body 与连接
        // 自行计算和设置；透传用户在这些头上填的值会导致长度不匹配或被 hyper 拒绝，因此丢弃。
        if k.eq_ignore_ascii_case("content-length")
            || k.eq_ignore_ascii_case("transfer-encoding")
            || k.eq_ignore_ascii_case("host")
        {
            continue;
        }
        builder = builder.header(k.as_str(), v.as_str());
    }
    match req.body {
        OutboundBody::Empty => {}
        OutboundBody::Bytes { content_type, data } => {
            if !has_content_type {
                builder = builder.header(CONTENT_TYPE, content_type);
            }
            builder = builder.body(data);
        }
        OutboundBody::File { path, content_type } => {
            let file = tokio::fs::File::open(&path)
                .await
                .map_err(|e| file_err(&path, e))?;
            let meta = file.metadata().await.map_err(|e| file_err(&path, e))?;
            if !meta.is_file() {
                return Err(RequestError::FileBody(format!(
                    "{}：不是普通文件",
                    path.display()
                )));
            }
            let len = meta.len();
            if !has_content_type && let Some(ct) = content_type {
                builder = builder.header(CONTENT_TYPE, ct);
            }
            // 流式 Body 本身不知道长度：显式给出 Content-Length，hyper 会尊重用户设置的该头并按定长发送
            builder = builder
                .header(CONTENT_LENGTH, len)
                .body(reqwest::Body::from(file));
        }
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

    let mut sink = Sink::with_capacity(total, spill_threshold);
    let mut stream = resp.bytes_stream();
    let mut last_report = Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        sink.push(&chunk, spill_threshold).await?;
        if let Some(tx) = &progress
            && last_report.elapsed() >= PROGRESS_INTERVAL
        {
            let _ = tx.try_send(Progress {
                received: sink.len(),
                total,
                elapsed: started.elapsed(),
            });
            last_report = Instant::now();
        }
    }
    let duration = started.elapsed();
    if let Some(tx) = &progress {
        let _ = tx.try_send(Progress {
            received: sink.len(),
            total,
            elapsed: duration,
        });
    }

    let body = sink.finish().await?;
    Ok(HttpResponse {
        meta: ResponseMeta {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("").to_string(),
            headers,
            duration,
            body_len: body.len(),
            content_type,
        },
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::spill::HEAD_BYTES;
    use crate::model::{BodyKind, KeyValue, Method, RawFormat, RequestDraft};
    use std::io::Write;
    use wiremock::matchers::{body_bytes, body_string, header, method, path, query_param};
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
        assert_eq!(resp.body.memory().unwrap(), b"hi");
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

    /// Content-Length / Transfer-Encoding / Host 由 reqwest/hyper 自行计算，用户手填的值
    /// 不得透传：服务端按真实 body 长度（3 字节）匹配，若用户的 "1" 被透传就会请求失败。
    #[tokio::test]
    async fn user_content_length_header_is_not_forwarded() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("content-length", "3"))
            .and(body_string("abc"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut d = draft(Method::Post, server.uri());
        d.headers = vec![KeyValue::new("Content-Length", "1")];
        d.body = BodyKind::Raw {
            format: RawFormat::Text,
            text: "abc".into(),
        };
        assert_eq!(run(&d).await.unwrap().meta.status, 200);
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
        assert_eq!(resp.body.memory().unwrap(), b"hello gzip");
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

    async fn run_with_threshold(d: &RequestDraft, threshold: u64) -> HttpResponse {
        let client = build_client();
        execute_with_threshold(&client, prepare(d).unwrap(), None, threshold)
            .await
            .unwrap()
    }

    async fn serve_bytes(body: Vec<u8>) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn body_at_or_below_threshold_stays_in_memory() {
        for size in [1023usize, 1024] {
            let server = serve_bytes(vec![b'a'; size]).await;
            let resp = run_with_threshold(&draft(Method::Get, server.uri()), 1024).await;
            assert!(!resp.body.is_spilled(), "{size} bytes must stay in memory");
            assert_eq!(resp.body.memory().unwrap().len(), size);
            assert_eq!(resp.body.path(), None);
            assert_eq!(resp.meta.body_len, size as u64);
        }
    }

    #[tokio::test]
    async fn body_over_threshold_is_spilled_to_disk_with_head() {
        let mut body = vec![b'a'; 1025];
        body[0] = b'{';
        body[1024] = b'z';
        let server = serve_bytes(body.clone()).await;
        let resp = run_with_threshold(&draft(Method::Get, server.uri()), 1024).await;
        assert!(resp.body.is_spilled());
        assert!(resp.body.memory().is_none());
        assert_eq!(resp.body.len(), 1025);
        assert_eq!(resp.meta.body_len, 1025);
        assert_eq!(resp.body.head(4), b"{aaa");
        // 1025 < HEAD_BYTES：head 就是全文
        assert_eq!(resp.body.head(usize::MAX), &body[..]);
        let path = resp.body.path().unwrap().to_path_buf();
        assert_eq!(std::fs::read(&path).unwrap(), body);
        drop(resp);
        assert!(
            !path.exists(),
            "spill file must be deleted when the last BodyStore is dropped"
        );
    }

    #[tokio::test]
    async fn spilled_head_is_capped_at_head_bytes() {
        let size = HEAD_BYTES + 1;
        let server = serve_bytes(vec![b'b'; size]).await;
        let resp = run_with_threshold(&draft(Method::Get, server.uri()), 4096).await;
        assert!(resp.body.is_spilled());
        assert_eq!(resp.body.head(usize::MAX).len(), HEAD_BYTES);
        assert_eq!(resp.body.len(), size as u64);
        assert_eq!(
            std::fs::metadata(resp.body.path().unwrap()).unwrap().len(),
            size as u64
        );
    }

    #[tokio::test]
    async fn chunked_gzip_body_over_threshold_is_spilled() {
        // gzip 响应无可用 Content-Length（reqwest 解压后 content_length() 为 None），走流式累积中的落盘分支
        let server = MockServer::start().await;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&vec![b'a'; 4096]).unwrap();
        let gz = enc.finish().unwrap();
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(gz)
                    .insert_header("content-encoding", "gzip"),
            )
            .mount(&server)
            .await;
        let resp = run_with_threshold(&draft(Method::Get, server.uri()), 512).await;
        assert!(resp.body.is_spilled());
        assert_eq!(resp.body.len(), 4096);
        assert_eq!(resp.body.head(usize::MAX), &vec![b'a'; 4096][..]);
    }

    #[tokio::test]
    async fn cloned_body_stores_share_one_spill_file() {
        let server = serve_bytes(vec![b'c'; 2048]).await;
        let resp = run_with_threshold(&draft(Method::Get, server.uri()), 1024).await;
        let path = resp.body.path().unwrap().to_path_buf();
        let second = resp.body.clone();
        drop(resp);
        assert!(path.exists(), "still referenced by the clone");
        drop(second);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn progress_reports_received_bytes_for_spilled_bodies() {
        let server = serve_bytes(vec![b'd'; 8192]).await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let client = build_client();
        let resp = execute_with_threshold(
            &client,
            prepare(&draft(Method::Get, server.uri())).unwrap(),
            Some(tx),
            1024,
        )
        .await
        .unwrap();
        assert!(resp.body.is_spilled());
        let mut last = None;
        while let Ok(p) = rx.try_recv() {
            last = Some(p);
        }
        assert_eq!(last.expect("progress").received, 8192);
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

    fn temp_upload_file(name: &str, payload: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("getcat-filebody-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, payload).unwrap();
        path
    }

    #[tokio::test]
    async fn file_body_is_streamed_with_content_length_and_type() {
        // 300 KB：大于 ReaderStream 的单块（4 KiB），保证走多块流式路径
        let payload: Vec<u8> = (0..300_000u32).map(|i| b'0' + (i % 10) as u8).collect();
        let path = temp_upload_file("upload.json", &payload);
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(header("content-type", "application/json"))
            .and(header("content-length", "300000"))
            .and(body_bytes(payload.clone()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut d = draft(Method::Put, server.uri());
        d.body = BodyKind::File {
            path: path.clone(),
            content_type: Some(guess_content_type(&path).to_string()),
        };
        assert_eq!(run(&d).await.unwrap().meta.status, 200);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn user_content_type_overrides_file_guess() {
        let path = temp_upload_file("upload.bin", b"xyz");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("content-type", "text/plain"))
            .and(body_bytes(b"xyz".to_vec()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let mut d = draft(Method::Post, server.uri());
        d.headers = vec![KeyValue::new("Content-Type", "text/plain")];
        d.body = BodyKind::File {
            path: path.clone(),
            content_type: Some("application/octet-stream".into()),
        };
        assert_eq!(run(&d).await.unwrap().meta.status, 200);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn directory_as_file_body_is_reported_as_file_body_error() {
        let dir = std::env::temp_dir().join(format!("getcat-filebody-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = draft(Method::Post, "http://127.0.0.1:1/".into());
        d.body = BodyKind::File {
            path: dir.clone(),
            content_type: None,
        };
        let err = run(&d).await.unwrap_err();
        assert!(matches!(err, RequestError::FileBody(_)), "{err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_file_is_reported_as_file_body_error() {
        let mut d = draft(Method::Post, "http://127.0.0.1:1/".into());
        d.body = BodyKind::File {
            path: "/nonexistent/getcat/upload.bin".into(),
            content_type: None,
        };
        let err = run(&d).await.unwrap_err();
        assert!(matches!(err, RequestError::FileBody(_)), "{err:?}");
        assert!(err.to_string().contains("upload.bin"));
    }

    #[test]
    fn prepare_rejects_empty_file_path() {
        let mut d = draft(Method::Post, "https://x.com".into());
        d.body = BodyKind::File {
            path: std::path::PathBuf::new(),
            content_type: None,
        };
        assert!(matches!(prepare(&d), Err(RequestError::FileBody(_))));
    }

    #[test]
    fn content_type_is_guessed_from_extension() {
        use std::path::Path;
        assert_eq!(guess_content_type(Path::new("a.JSON")), "application/json");
        assert_eq!(guess_content_type(Path::new("a.xml")), "application/xml");
        assert_eq!(guess_content_type(Path::new("a.txt")), "text/plain");
        assert_eq!(guess_content_type(Path::new("a.png")), "image/png");
        assert_eq!(guess_content_type(Path::new("a.jpeg")), "image/jpeg");
        assert_eq!(
            guess_content_type(Path::new("a")),
            "application/octet-stream"
        );
        assert_eq!(
            guess_content_type(Path::new("a.weird")),
            "application/octet-stream"
        );
    }
}
