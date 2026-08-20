//! 根据 Content-Type 与内容采样推断响应体的展示类型。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Json,
    Xml,
    Html,
    Text,
    Binary,
}

impl ContentKind {
    pub fn editor_language(self) -> &'static str {
        match self {
            ContentKind::Json => "json",
            ContentKind::Xml | ContentKind::Html => "html",
            ContentKind::Text | ContentKind::Binary => "text",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ContentKind::Json => "JSON",
            ContentKind::Xml => "XML",
            ContentKind::Html => "HTML",
            ContentKind::Text => "Text",
            ContentKind::Binary => "Binary",
        }
    }

    pub fn is_text(self) -> bool {
        !matches!(self, ContentKind::Binary)
    }
}

pub const SNIFF_LEN: usize = 8 * 1024;

pub fn detect(content_type: Option<&str>, sample: &[u8]) -> ContentKind {
    if let Some(ct) = content_type {
        let lower = ct.to_ascii_lowercase();
        let mime = lower.split(';').next().unwrap_or("").trim();
        if mime == "text/html" || mime == "application/xhtml+xml" {
            return ContentKind::Html;
        }
        if mime.ends_with("/json") || mime.ends_with("+json") {
            return ContentKind::Json;
        }
        if mime.ends_with("/xml") || mime.ends_with("+xml") {
            return ContentKind::Xml;
        }
        if mime.starts_with("text/")
            || matches!(
                mime,
                "application/javascript" | "application/x-www-form-urlencoded"
            )
        {
            return ContentKind::Text;
        }
        if mime.starts_with("image/")
            || mime.starts_with("audio/")
            || mime.starts_with("video/")
            || mime.starts_with("font/")
            || matches!(
                mime,
                "application/octet-stream"
                    | "application/pdf"
                    | "application/zip"
                    | "application/gzip"
            )
        {
            return ContentKind::Binary;
        }
    }
    sniff(sample)
}

fn sniff(sample: &[u8]) -> ContentKind {
    let sample = &sample[..sample.len().min(SNIFF_LEN)];
    if sample.is_empty() {
        return ContentKind::Text;
    }
    if sample.contains(&0) {
        return ContentKind::Binary;
    }
    match std::str::from_utf8(sample) {
        Ok(_) => {}
        // error_len() == None 表示在末尾被截断的多字节字符，不算二进制
        Err(e) if e.error_len().is_none() => {}
        Err(_) => return ContentKind::Binary,
    }
    let first = sample.iter().copied().find(|b| !b.is_ascii_whitespace());
    match first {
        Some(b'{') | Some(b'[') => ContentKind::Json,
        Some(b'<') => {
            let head =
                String::from_utf8_lossy(&sample[..sample.len().min(256)]).to_ascii_lowercase();
            if head.contains("<html") || head.contains("<!doctype html") {
                ContentKind::Html
            } else {
                ContentKind::Xml
            }
        }
        _ => ContentKind::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_wins() {
        assert_eq!(
            detect(Some("application/json; charset=utf-8"), b"not json"),
            ContentKind::Json
        );
        assert_eq!(
            detect(Some("application/vnd.api+json"), b""),
            ContentKind::Json
        );
        assert_eq!(
            detect(Some("text/html; charset=utf-8"), b""),
            ContentKind::Html
        );
        assert_eq!(
            detect(Some("application/xhtml+xml"), b""),
            ContentKind::Html
        );
        assert_eq!(detect(Some("application/xml"), b""), ContentKind::Xml);
        assert_eq!(detect(Some("image/svg+xml"), b""), ContentKind::Xml);
        assert_eq!(detect(Some("text/plain"), b""), ContentKind::Text);
        assert_eq!(
            detect(Some("application/javascript"), b""),
            ContentKind::Text
        );
        assert_eq!(detect(Some("image/png"), b"\x89PNG"), ContentKind::Binary);
        assert_eq!(
            detect(Some("application/octet-stream"), b"abc"),
            ContentKind::Binary
        );
    }

    #[test]
    fn sniffs_when_content_type_missing_or_generic() {
        assert_eq!(detect(None, b"  {\"a\":1}"), ContentKind::Json);
        assert_eq!(detect(None, b"[1,2]"), ContentKind::Json);
        assert_eq!(
            detect(None, b"<?xml version=\"1.0\"?><r/>"),
            ContentKind::Xml
        );
        assert_eq!(detect(None, b"<!DOCTYPE html><html>"), ContentKind::Html);
        assert_eq!(detect(None, b"plain words"), ContentKind::Text);
        assert_eq!(
            detect(Some("application/unknown"), b"{\"x\":1}"),
            ContentKind::Json
        );
    }

    #[test]
    fn binary_detection() {
        assert_eq!(detect(None, b"abc\0def"), ContentKind::Binary);
        assert_eq!(detect(None, &[0xFF, 0xFE, 0x00, 0x01]), ContentKind::Binary);
        // 采样边界截断了多字节字符：不算二进制
        let mut s = b"hello ".to_vec();
        s.extend_from_slice(&"é".as_bytes()[..1]);
        assert_eq!(detect(None, &s), ContentKind::Text);
        assert_eq!(detect(None, b""), ContentKind::Text);
    }

    #[test]
    fn editor_language_mapping() {
        assert_eq!(ContentKind::Json.editor_language(), "json");
        assert_eq!(ContentKind::Html.editor_language(), "html");
        assert_eq!(ContentKind::Xml.editor_language(), "html");
        assert_eq!(ContentKind::Text.editor_language(), "text");
        assert!(!ContentKind::Binary.is_text());
    }
}
