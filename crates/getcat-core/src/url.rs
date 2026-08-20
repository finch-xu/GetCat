//! 把 RequestDraft 的 URL、Path 参数、Query 参数合成最终 `url::Url`。

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

fn substitute_path_params(url: &str, params: &[KeyValue]) -> String {
    let mut out = url.to_string();
    for p in params.iter().filter(|p| p.enabled && !p.key.is_empty()) {
        out = out.replace(&format!("{{{}}}", p.key), &p.value);
    }
    out
}

pub fn build_url(draft: &RequestDraft) -> Result<Url, UrlError> {
    let raw = draft.url.trim();
    if raw.is_empty() {
        return Err(UrlError::Empty);
    }
    let substituted = substitute_path_params(raw, &draft.path_params);
    let with_scheme = if substituted.contains("://") {
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
                key: "post".into(),
                value: "9".into(),
                enabled: false,
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
                key: "skip".into(),
                value: "x".into(),
                enabled: false,
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
}
