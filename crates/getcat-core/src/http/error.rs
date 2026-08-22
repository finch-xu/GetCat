//! 请求错误分类：把 reqwest 的错误链映射成用户可理解的类别。
//!
//! `Display` 是英文的技术文案（日志与测试用）；界面上的种类标签与说明由 app 层按变体翻译。

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
    #[error("Invalid header: {0}")]
    InvalidHeader(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("DNS lookup failed: {0}")]
    Dns(String),
    #[error("Connection refused: {0}")]
    ConnectionRefused(String),
    #[error("TLS error: {0}")]
    Tls(String),
    #[error("Connection timed out")]
    Timeout,
    #[error("Couldn't write temp file: {0}")]
    Spill(String),
    #[error("Couldn't read file: {0}")]
    FileBody(String),
    #[error("Cancelled")]
    Cancelled,
    #[error("Network error: {0}")]
    Other(String),
}

/// 完整错误链（含顶层）——只用于展示文本。
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut parts = vec![e.to_string()];
    parts.extend(source_parts(e));
    parts.join(": ")
}

/// 仅 source 链，**不含**顶层 `e.to_string()`——用于关键词分类。
/// reqwest 会把 ` for url (<url>)` 拼进顶层文本，若拿它做关键词匹配，
/// 请求 `https://dns.google/resolve` 会被误判成 DNS 失败、
/// `http://localhost:1/tls-status` 会被误判成 TLS 错误。
fn source_chain(e: &dyn std::error::Error) -> String {
    source_parts(e).join(": ")
}

fn source_parts(e: &dyn std::error::Error) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = e.source();
    while let Some(s) = cur {
        parts.push(s.to_string());
        cur = s.source();
    }
    parts
}

impl From<reqwest::Error> for RequestError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            return RequestError::Timeout;
        }
        let chain = error_chain(&e);
        // 只有连接阶段的错误才做 Dns/Tls/Refused 细分，且只看 source 链的关键词。
        if e.is_connect() {
            let lower = source_chain(&e).to_ascii_lowercase();
            if lower.contains("certificate") || lower.contains("tls") || lower.contains("handshake")
            {
                return RequestError::Tls(chain);
            }
            if lower.contains("dns")
                || lower.contains("lookup")
                || lower.contains("resolve")
                || lower.contains("nodename")
                || lower.contains("name or service not known")
            {
                return RequestError::Dns(chain);
            }
            if lower.contains("refused") {
                return RequestError::ConnectionRefused(chain);
            }
        }
        RequestError::Other(chain)
    }
}
