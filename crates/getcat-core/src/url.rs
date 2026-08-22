//! 把 RequestDraft 的 URL、Path 参数、Query 参数合成最终 `url::Url`。

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

use crate::model::{KeyValue, RequestDraft};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    #[error("URL 不能为空")]
    Empty,
    #[error("无效的 URL：{0}")]
    Invalid(String),
}

/// 提取 `{name}` 形式的 Path 参数名，按首次出现顺序去重；`{}` 与未闭合的 `{` 忽略。
pub fn extract_path_params(url: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut rest = url;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let name = &after[..end];
        if !name.is_empty() && !name.contains('{') && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
        rest = &after[end + 1..];
    }
    names
}

/// 路径段编码集：除 RFC 3986 unreserved（字母数字与 `-._~`）外全部百分号编码，
/// 因此参数值中的 `/`、`?`、`#`、`{}` 都不会改变 URL 结构。
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// 对原始 URL 做单遍扫描：每个 `{name}` 只根据 `params` 查找一次，
/// 替换结果不再参与后续匹配，因此参数值里出现 `{other}` 也不会被二次替换。
fn substitute_path_params(url: &str, params: &[KeyValue]) -> String {
    let mut out = String::with_capacity(url.len());
    let mut rest = url;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start + 1..].find('}') else {
            break;
        };
        let name = &rest[start + 1..start + 1 + len];
        let token_end = start + 1 + len + 1;
        out.push_str(&rest[..start]);
        match params
            .iter()
            .find(|p| p.enabled && !p.key.is_empty() && p.key == name)
        {
            Some(p) => out.push_str(&utf8_percent_encode(&p.value, PATH_SEGMENT).to_string()),
            None => out.push_str(&rest[start..token_end]),
        }
        rest = &rest[token_end..];
    }
    out.push_str(rest);
    out
}

/// `://` 之前是否是一个真正的 scheme（RFC 3986：字母开头，只含字母数字与 `+-.`）。
/// `localhost:8080/cb?to=https://x` 的 `://` 在 query 里，前面含 `:` `/` `?`，不算。
fn has_scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once("://") else {
        return false;
    };
    let mut chars = scheme.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

pub fn build_url(draft: &RequestDraft) -> Result<Url, UrlError> {
    let raw = draft.url.trim();
    if raw.is_empty() {
        return Err(UrlError::Empty);
    }
    let substituted = substitute_path_params(raw, &draft.path_params);
    let with_scheme = if has_scheme(&substituted) {
        substituted
    } else {
        format!("http://{substituted}")
    };
    let mut url = Url::parse(&with_scheme).map_err(|e| UrlError::Invalid(e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(UrlError::Invalid(format!("不支持的协议：{}", url.scheme())));
    }
    let enabled: Vec<&KeyValue> = draft
        .params
        .iter()
        .filter(|p| p.enabled && !p.key.is_empty())
        .collect();
    if !enabled.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for p in enabled {
            pairs.append_pair(&p.key, &p.value);
        }
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyValue, RequestDraft};

    fn draft(url: &str) -> RequestDraft {
        RequestDraft {
            url: url.into(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_url_is_error() {
        assert_eq!(build_url(&draft("   ")), Err(UrlError::Empty));
    }

    #[test]
    fn missing_scheme_defaults_to_http() {
        let u = build_url(&draft("example.com/a")).unwrap();
        assert_eq!(u.as_str(), "http://example.com/a");
    }

    #[test]
    fn unsupported_scheme_is_error() {
        assert!(matches!(
            build_url(&draft("ftp://x.com")),
            Err(UrlError::Invalid(_))
        ));
    }

    #[test]
    fn substitutes_enabled_path_params() {
        let mut d = draft("https://x.com/users/{id}/posts/{post}");
        d.path_params = vec![
            KeyValue::new("id", "42"),
            KeyValue {
                enabled: false,
                ..KeyValue::new("post", "9")
            },
        ];
        let u = build_url(&d).unwrap();
        assert_eq!(u.path(), "/users/42/posts/%7Bpost%7D");
    }

    #[test]
    fn appends_enabled_query_params_and_keeps_existing() {
        let mut d = draft("https://x.com/s?q=1");
        d.params = vec![
            KeyValue::new("page", "2"),
            KeyValue {
                enabled: false,
                ..KeyValue::new("skip", "x")
            },
            KeyValue::new("", "ignored-empty-key"),
            KeyValue::new("t", "a b"),
        ];
        let u = build_url(&d).unwrap();
        assert_eq!(u.query(), Some("q=1&page=2&t=a+b"));
    }

    #[test]
    fn no_params_leaves_query_untouched() {
        let u = build_url(&draft("https://x.com/s")).unwrap();
        assert_eq!(u.as_str(), "https://x.com/s");
    }

    #[test]
    fn extracts_path_param_names_in_order_without_duplicates() {
        assert_eq!(
            extract_path_params("/a/{id}/b/{id}/{name}?x={}"),
            vec!["id".to_string(), "name".to_string()]
        );
        assert!(extract_path_params("https://x.com/plain").is_empty());
        assert!(extract_path_params("https://x.com/{unclosed").is_empty());
    }

    #[test]
    fn path_param_values_are_encoded_as_single_segment() {
        let mut d = draft("https://x.com/files/{name}");
        d.path_params = vec![KeyValue::new("name", "a/b c")];
        assert_eq!(build_url(&d).unwrap().path(), "/files/a%2Fb%20c");
    }

    #[test]
    fn path_param_values_are_not_resubstituted() {
        let mut d = draft("https://x.com/users/{id}");
        d.path_params = vec![KeyValue::new("id", "{post}"), KeyValue::new("post", "999")];
        assert_eq!(build_url(&d).unwrap().path(), "/users/%7Bpost%7D");
    }

    #[test]
    fn scheme_in_query_does_not_count_as_url_scheme() {
        let u = build_url(&draft("localhost:8080/cb?to=https://x")).unwrap();
        assert_eq!(u.scheme(), "http");
        assert_eq!(u.host_str(), Some("localhost"));
        assert_eq!(u.port(), Some(8080));
        assert_eq!(u.query(), Some("to=https://x"));
        assert!(has_scheme("https://a.b"));
        assert!(has_scheme("h2+c.x://a"));
        assert!(!has_scheme("localhost:8080/cb?to=https://x"));
        assert!(!has_scheme("1abc://x"));
        assert!(!has_scheme("example.com/a"));
    }
}
