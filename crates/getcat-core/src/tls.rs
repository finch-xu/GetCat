//! TLS 证书体检：把对端的叶子证书解析成可展示的字段，并挑出常见问题。
//!
//! 证书校验默认关着（本地调试大量用自签名接口），握手因此会成功、证书也就拿得到；
//! 「这张证书哪里不对」改由这里离线判断，UI 再决定怎么提示。
//! 反过来说，一旦打开校验，握手在出问题时直接失败、连接都没建立，
//! 这里也就无从下手 —— 那条路上只剩 [`crate::http::RequestError::Tls`] 的文本。

use sha2::{Digest, Sha256};
use x509_parser::prelude::*;

/// 证书上值得提醒用户的问题。按严重程度排序，UI 直接取第一条当横幅主文案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertWarning {
    /// 已过有效期
    Expired,
    /// 尚未生效：签发时间在未来，通常是本机时钟不对
    NotYetValid,
    /// 请求的主机名不在证书的 SAN 里
    HostnameMismatch,
    /// 自签名：签发者就是自己，没有可追溯的信任链
    SelfSigned,
}

/// 叶子证书的可展示信息。字段都预先格式化成字符串——UI 只负责摆出来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateInfo {
    pub subject: String,
    pub issuer: String,
    pub not_before: String,
    pub not_after: String,
    /// Subject Alternative Name；现代规则只看它，CN 不再作为主机名依据
    pub san: Vec<String>,
    pub serial: String,
    pub signature_algorithm: String,
    /// 大写十六进制、冒号分隔，与 openssl / 浏览器的显示一致
    pub sha256_fingerprint: String,
    pub warnings: Vec<CertWarning>,
}

impl CertificateInfo {
    pub fn is_trustworthy(&self) -> bool {
        self.warnings.is_empty()
    }
}

/// 解析 DER 编码的叶子证书；`host` 是请求的主机名，用于 SAN 匹配。
///
/// 解析失败返回 `None`：拿不到证书细节只是少一块展示，不该让整个响应失败。
pub fn inspect(der: &[u8], host: &str) -> Option<CertificateInfo> {
    let (_, cert) = X509Certificate::from_der(der).ok()?;

    let san = collect_san(&cert);
    let validity = cert.validity();
    let now = ASN1Time::now();

    let mut warnings = Vec::new();
    if validity.not_after < now {
        warnings.push(CertWarning::Expired);
    }
    if validity.not_before > now {
        warnings.push(CertWarning::NotYetValid);
    }
    // 空 SAN 也算不匹配：没有 SAN 的证书按现代规则本来就不该被信任
    if !host.is_empty() && !san.iter().any(|n| matches_host(n, host)) {
        warnings.push(CertWarning::HostnameMismatch);
    }
    if cert.subject() == cert.issuer() {
        warnings.push(CertWarning::SelfSigned);
    }

    Some(CertificateInfo {
        subject: cert.subject().to_string(),
        issuer: cert.issuer().to_string(),
        not_before: validity.not_before.to_string(),
        not_after: validity.not_after.to_string(),
        san,
        serial: cert.raw_serial_as_string(),
        signature_algorithm: signature_algorithm(&cert),
        sha256_fingerprint: fingerprint(der),
        warnings,
    })
}

fn collect_san(cert: &X509Certificate<'_>) -> Vec<String> {
    let Ok(Some(ext)) = cert.subject_alternative_name() else {
        return Vec::new();
    };
    ext.value
        .general_names
        .iter()
        .filter_map(|name| match name {
            GeneralName::DNSName(s) => Some((*s).to_string()),
            GeneralName::IPAddress(bytes) => Some(format_ip(bytes)),
            GeneralName::URI(s) => Some((*s).to_string()),
            GeneralName::RFC822Name(s) => Some((*s).to_string()),
            _ => None,
        })
        .collect()
}

/// SAN 里的 IP 是原始网络字节序：4 字节是 v4，16 字节是 v6，其余原样十六进制。
fn format_ip(bytes: &[u8]) -> String {
    match bytes.len() {
        4 => std::net::Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).to_string(),
        16 => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(bytes);
            std::net::Ipv6Addr::from(octets).to_string()
        }
        _ => bytes.iter().map(|b| format!("{b:02x}")).collect(),
    }
}

/// 优先取 OID 注册表里的短名（如 `sha256WithRSAEncryption`），查不到就退回点分 OID。
fn signature_algorithm(cert: &X509Certificate<'_>) -> String {
    let oid = &cert.signature_algorithm.algorithm;
    oid_registry()
        .get(oid)
        .map(|entry| entry.sn().to_string())
        .unwrap_or_else(|| oid.to_id_string())
}

fn fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// SAN 条目与主机名比对。通配符只顶最左一级：`*.example.com` 配 `a.example.com`，
/// 既不配 `example.com` 本身，也不配 `a.b.example.com`。
fn matches_host(pattern: &str, host: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let host = host.trim().to_ascii_lowercase();
    match pattern.strip_prefix("*.") {
        Some(suffix) => host.split_once('.').is_some_and(|(_, tail)| tail == suffix),
        None => pattern == host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matches_exactly_one_label() {
        assert!(matches_host("*.example.com", "api.example.com"));
        assert!(matches_host("*.example.com", "API.Example.com"));
        // 通配符不覆盖裸域，也不跨级
        assert!(!matches_host("*.example.com", "example.com"));
        assert!(!matches_host("*.example.com", "a.b.example.com"));
    }

    #[test]
    fn exact_match_is_case_insensitive() {
        assert!(matches_host("Localhost", "localhost"));
        assert!(!matches_host("localhost", "localhost.localdomain"));
    }

    #[test]
    fn ip_bytes_render_as_addresses() {
        assert_eq!(format_ip(&[127, 0, 0, 1]), "127.0.0.1");
        assert_eq!(format_ip(&[0u8; 16]), "::");
        // 长度不认识时不猜，原样十六进制
        assert_eq!(format_ip(&[0xde, 0xad]), "dead");
    }

    #[test]
    fn fingerprint_is_uppercase_colon_separated() {
        // SHA-256 of empty input
        let fp = fingerprint(b"");
        assert!(fp.starts_with("E3:B0:C4:42:98:FC"), "{fp}");
        assert_eq!(fp.split(':').count(), 32);
    }

    #[test]
    fn garbage_der_is_ignored_not_fatal() {
        assert!(inspect(b"not a certificate", "localhost").is_none());
    }

    // 固定的自签名测试证书，有效期写死在过去 / 未来，结论不随系统时钟漂移。
    // 生成方式见 testdata/README.md。
    const EXPIRED: &[u8] = include_bytes!("../testdata/expired.der");
    const NOT_YET_VALID: &[u8] = include_bytes!("../testdata/not-yet-valid.der");
    const SELF_SIGNED: &[u8] = include_bytes!("../testdata/self-signed.der");

    #[test]
    fn self_signed_but_otherwise_fine_reports_only_the_trust_chain() {
        let info = inspect(SELF_SIGNED, "localhost").unwrap();
        assert_eq!(info.warnings, vec![CertWarning::SelfSigned]);
        assert!(!info.is_trustworthy());
        assert_eq!(info.subject, "CN=localhost");
        assert_eq!(info.issuer, "CN=localhost");
        assert_eq!(info.san, vec!["localhost", "*.example.com"]);
        // 指纹是 32 段两位大写十六进制
        assert_eq!(info.sha256_fingerprint.split(':').count(), 32);
        assert!(info.signature_algorithm.contains("ecdsa"));
    }

    #[test]
    fn san_wildcard_is_honoured_against_the_requested_host() {
        // *.example.com 覆盖 api.example.com，不该报主机名不符
        let info = inspect(SELF_SIGNED, "api.example.com").unwrap();
        assert_eq!(info.warnings, vec![CertWarning::SelfSigned]);

        // 裸域不在 SAN 里（通配符不覆盖它），这才该报
        let info = inspect(SELF_SIGNED, "example.com").unwrap();
        assert!(info.warnings.contains(&CertWarning::HostnameMismatch));
    }

    #[test]
    fn expiry_is_reported_alongside_the_trust_chain() {
        let info = inspect(EXPIRED, "localhost").unwrap();
        assert!(info.warnings.contains(&CertWarning::Expired));
        assert!(info.warnings.contains(&CertWarning::SelfSigned));
        assert!(!info.warnings.contains(&CertWarning::NotYetValid));
        // 最严重的排最前：横幅只取第一条
        assert_eq!(info.warnings[0], CertWarning::Expired);
    }

    #[test]
    fn not_yet_valid_is_distinct_from_expired() {
        let info = inspect(NOT_YET_VALID, "localhost").unwrap();
        assert!(info.warnings.contains(&CertWarning::NotYetValid));
        assert!(!info.warnings.contains(&CertWarning::Expired));
    }

    /// 主机名为空（拿不到 host）时不该凭空报不匹配。
    #[test]
    fn empty_host_skips_the_hostname_check() {
        let info = inspect(SELF_SIGNED, "").unwrap();
        assert_eq!(info.warnings, vec![CertWarning::SelfSigned]);
    }
}
