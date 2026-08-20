//! 请求错误分类：把 reqwest 的错误链映射成用户可理解的类别。

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    #[error("无效的 URL：{0}")]
    InvalidUrl(String),
    #[error("非法的 Header：{0}")]
    InvalidHeader(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("DNS 解析失败：{0}")]
    Dns(String),
    #[error("连接被拒绝：{0}")]
    ConnectionRefused(String),
    #[error("TLS 错误：{0}")]
    Tls(String),
    #[error("连接超时")]
    Timeout,
    #[error("临时文件写入失败：{0}")]
    Spill(String),
    #[error("已取消")]
    Cancelled,
    #[error("网络错误：{0}")]
    Other(String),
}

impl RequestError {
    pub fn kind_label(&self) -> &'static str {
        match self {
            RequestError::InvalidUrl(_) => "URL 无效",
            RequestError::InvalidHeader(_) => "Header 无效",
            RequestError::Unsupported(_) => "暂不支持",
            RequestError::Dns(_) => "DNS 失败",
            RequestError::ConnectionRefused(_) => "连接被拒绝",
            RequestError::Tls(_) => "TLS 错误",
            RequestError::Timeout => "超时",
            RequestError::Spill(_) => "落盘失败",
            RequestError::Cancelled => "已取消",
            RequestError::Other(_) => "网络错误",
        }
    }
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
