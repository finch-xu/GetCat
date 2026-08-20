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
            RequestError::Cancelled => "已取消",
            RequestError::Other(_) => "网络错误",
        }
    }
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut parts = vec![e.to_string()];
    let mut cur = e.source();
    while let Some(s) = cur {
        parts.push(s.to_string());
        cur = s.source();
    }
    parts.join(": ")
}

impl From<reqwest::Error> for RequestError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            return RequestError::Timeout;
        }
        let chain = error_chain(&e);
        let lower = chain.to_ascii_lowercase();
        if lower.contains("certificate") || lower.contains("tls") || lower.contains("handshake") {
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
        RequestError::Other(chain)
    }
}
