//! 领域模型：描述"一个具体的请求实例"与"一次响应的元数据"。

use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

/// 已保存请求与 Tab 的标识；26 字符 Crockford Base32 字符串落盘。ulid 3.0 的构造函数是 `Ulid::generate()`。
pub use ulid::Ulid;

/// Tab 的稳定标识：作为草稿文件名持久化，重启后不变。
pub type TabId = Ulid;

/// 当前 Unix 毫秒时间戳（系统时钟早于 1970 时返回 0）。
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

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
    FormUrlEncoded {
        fields: Vec<KeyValue>,
    },
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

/// 一条已保存请求：一个文件一条（`requests/<ulid>.json`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRequest {
    pub id: Ulid,
    pub name: String,
    pub draft: RequestDraft,
    /// v2 分组；v1 只保留字段，UI 不展示。
    #[serde(default)]
    pub group: Option<String>,
    /// v2 导入溯源；v1 只保留字段。
    #[serde(default)]
    pub source: Option<Source>,
    /// Unix 毫秒。
    pub created_at: i64,
    pub updated_at: i64,
}

impl SavedRequest {
    pub fn new(name: impl Into<String>, draft: RequestDraft) -> SavedRequest {
        let now = now_ms();
        SavedRequest {
            id: Ulid::generate(),
            name: name.into(),
            draft,
            group: None,
            source: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// 一个 Tab 的草稿文件（`drafts/<tab-id>.json`）：重启后原样恢复，含"来自哪条已保存请求"与"是否有未保存修改"。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabDraft {
    pub id: TabId,
    pub draft: RequestDraft,
    #[serde(default)]
    pub saved_id: Option<Ulid>,
    #[serde(default)]
    pub dirty: bool,
}

/// 主题偏好：跟随系统，或固定明 / 暗。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [ThemePref::System, ThemePref::Light, ThemePref::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemePref::System => "跟随系统",
            ThemePref::Light => "浅色",
            ThemePref::Dark => "深色",
        }
    }

    /// 循环切换：系统 → 浅色 → 深色 → 系统。
    pub fn next(self) -> ThemePref {
        match self {
            ThemePref::System => ThemePref::Light,
            ThemePref::Light => ThemePref::Dark,
            ThemePref::Dark => ThemePref::System,
        }
    }
}

/// 工作区状态（`workspace.json`）：只有布局与顺序，不含任何请求内容。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkspaceState {
    #[serde(default)]
    pub tab_order: Vec<TabId>,
    #[serde(default)]
    pub active: Option<TabId>,
    /// 侧栏宽度（逻辑像素）；None 表示默认 240。
    #[serde(default)]
    pub sidebar_width: Option<f32>,
    #[serde(default)]
    pub sidebar_collapsed: bool,
    #[serde(default)]
    pub theme: ThemePref,
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

    #[test]
    fn form_body_serde_roundtrip() {
        let d = RequestDraft {
            body: BodyKind::FormUrlEncoded {
                fields: vec![KeyValue::new("a", "1")],
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"kind\":\"form_url_encoded\""), "{}", json);
        let back: RequestDraft = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn saved_request_serde_roundtrip_keeps_reserved_fields() {
        let mut req = SavedRequest::new(
            "用户列表",
            RequestDraft {
                method: Method::Get,
                url: "https://api.example.com/users/{id}".into(),
                path_params: vec![KeyValue::new("id", "1")],
                ..Default::default()
            },
        );
        req.group = Some("users".into());
        req.source = Some(Source {
            kind: SourceKind::OpenApi,
            spec: "https://api.example.com/openapi.json".into(),
            operation_id: Some("listUsers".into()),
        });
        assert_eq!(req.created_at, req.updated_at);
        let json = serde_json::to_string(&req).unwrap();
        // ULID 以 26 字符字符串落盘
        assert!(json.contains(&format!("\"id\":\"{}\"", req.id)), "{json}");
        let back: SavedRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn tab_draft_and_workspace_state_tolerate_missing_fields() {
        let id = Ulid::generate();
        let minimal = format!(r#"{{"id":"{id}","draft":{{"method":"GET","url":""}}}}"#);
        let draft: TabDraft = serde_json::from_str(&minimal).unwrap();
        assert_eq!(draft.id, id);
        assert_eq!(draft.saved_id, None);
        assert!(!draft.dirty);

        let ws: WorkspaceState = serde_json::from_str("{}").unwrap();
        assert_eq!(ws, WorkspaceState::default());
        assert_eq!(ws.theme, ThemePref::System);
    }

    #[test]
    fn theme_pref_cycles_and_serializes_snake_case() {
        assert_eq!(ThemePref::System.next(), ThemePref::Light);
        assert_eq!(ThemePref::Light.next(), ThemePref::Dark);
        assert_eq!(ThemePref::Dark.next(), ThemePref::System);
        assert_eq!(serde_json::to_string(&ThemePref::Dark).unwrap(), "\"dark\"");
        assert_eq!(ThemePref::System.label(), "跟随系统");
    }

    #[test]
    fn now_ms_is_after_2026() {
        assert!(now_ms() > 1_767_225_600_000); // 2026-01-01T00:00:00Z
    }
}
