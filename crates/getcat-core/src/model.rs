//! 领域模型：描述"一个具体的请求实例"与"一次响应的元数据"。

use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl Method {
    pub const ALL: [Method; 7] = [
        Method::Get,
        Method::Post,
        Method::Put,
        Method::Patch,
        Method::Delete,
        Method::Head,
        Method::Options,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
            Method::Options => "OPTIONS",
        }
    }

    pub fn parse(s: &str) -> Option<Method> {
        Method::ALL
            .into_iter()
            .find(|m| m.as_str().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    pub enabled: bool,
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RawFormat {
    Json,
    Text,
    Xml,
}

impl RawFormat {
    pub const ALL: [RawFormat; 3] = [RawFormat::Json, RawFormat::Text, RawFormat::Xml];

    pub fn content_type(self) -> &'static str {
        match self {
            RawFormat::Json => "application/json",
            RawFormat::Text => "text/plain",
            RawFormat::Xml => "application/xml",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RawFormat::Json => "JSON",
            RawFormat::Text => "Text",
            RawFormat::Xml => "XML",
        }
    }

    /// gpui-component 编辑器使用的语言 id（XML 复用 html 语法高亮）。
    pub fn editor_language(self) -> &'static str {
        match self {
            RawFormat::Json => "json",
            RawFormat::Text => "text",
            RawFormat::Xml => "html",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BodyKind {
    #[default]
    None,
    Raw {
        format: RawFormat,
        text: String,
    },
    FormUrlEncoded(Vec<KeyValue>),
    File {
        path: PathBuf,
        content_type: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RequestDraft {
    pub method: Method,
    /// 允许包含 `{name}` 占位符，由 `path_params` 替换。
    pub url: String,
    #[serde(default)]
    pub path_params: Vec<KeyValue>,
    #[serde(default)]
    pub params: Vec<KeyValue>,
    #[serde(default)]
    pub headers: Vec<KeyValue>,
    #[serde(default)]
    pub body: BodyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    OpenApi,
    Postman,
    Curl,
}

/// v2 导入功能的溯源信息；v1 只定义结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub kind: SourceKind,
    pub spec: String,
    pub operation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseMeta {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub duration: Duration,
    pub body_len: u64,
    pub content_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_roundtrip() {
        for m in Method::ALL {
            assert_eq!(Method::parse(m.as_str()), Some(m));
        }
        assert_eq!(Method::parse("get"), Some(Method::Get));
        assert_eq!(Method::parse("BREW"), None);
    }

    #[test]
    fn key_value_defaults_enabled() {
        let kv = KeyValue::new("a", "b");
        assert!(kv.enabled);
        assert_eq!(kv.key, "a");
    }

    #[test]
    fn draft_default_is_empty_get() {
        let d = RequestDraft::default();
        assert_eq!(d.method, Method::Get);
        assert!(d.url.is_empty());
        assert_eq!(d.body, BodyKind::None);
    }

    #[test]
    fn draft_serde_roundtrip() {
        let d = RequestDraft {
            method: Method::Post,
            url: "https://x.com/{id}".into(),
            path_params: vec![KeyValue::new("id", "1")],
            params: vec![KeyValue::new("q", "v")],
            headers: vec![KeyValue {
                key: "X".into(),
                value: "1".into(),
                enabled: false,
            }],
            body: BodyKind::Raw {
                format: RawFormat::Json,
                text: "{}".into(),
            },
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: RequestDraft = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn raw_format_content_types() {
        assert_eq!(RawFormat::Json.content_type(), "application/json");
        assert_eq!(RawFormat::Text.content_type(), "text/plain");
        assert_eq!(RawFormat::Xml.content_type(), "application/xml");
    }
}
