//! 领域模型：描述"一个具体的请求实例"与"一次响应的元数据"。

use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::tls::CertificateInfo;

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

    /// 标签栏角标用的缩写：那里的空间比侧栏的 method 列还紧，
    /// 只有 DELETE / OPTIONS 需要收，其余原样返回。
    pub fn short(self) -> &'static str {
        match self {
            Method::Delete => "DEL",
            Method::Options => "OPT",
            other => other.as_str(),
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
    /// 备注，不参与发送；空串不落盘。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled: true,
            description: String::new(),
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

    /// `ALL` 里的下标，就是格式分段控件上的位置（与 `BodyMode::index` 对称）。
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    pub fn from_index(ix: usize) -> Self {
        Self::ALL.get(ix).copied().unwrap_or(RawFormat::Json)
    }

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

/// multipart/form-data 字段的值：文本或文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FormValue {
    Text {
        value: String,
    },
    /// 只存路径；`content_type` 为 None 表示发送时按扩展名猜（`http::guess_content_type`）。
    File {
        path: PathBuf,
        #[serde(default)]
        content_type: Option<String>,
    },
}

impl Default for FormValue {
    fn default() -> Self {
        FormValue::Text {
            value: String::new(),
        }
    }
}

/// multipart/form-data 的一个字段。文件只可能出现在这里，Header / Query / urlencoded 用 `KeyValue`。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FormField {
    pub key: String,
    pub enabled: bool,
    /// 备注，不参与发送；空串不落盘。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub value: FormValue,
}

impl FormField {
    pub fn text(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            enabled: true,
            description: String::new(),
            value: FormValue::Text {
                value: value.into(),
            },
        }
    }

    pub fn file(key: impl Into<String>, path: PathBuf) -> Self {
        Self {
            key: key.into(),
            enabled: true,
            description: String::new(),
            value: FormValue::File {
                path,
                content_type: None,
            },
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
    /// multipart/form-data：字段可为文本或文件。
    FormData {
        fields: Vec<FormField>,
    },
    FormUrlEncoded {
        fields: Vec<KeyValue>,
    },
    /// 整个请求体就是一个文件（Postman 的 binary）。旧文件写的是 `"kind":"file"`，读入后再保存为 `binary`。
    #[serde(alias = "file")]
    Binary {
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
    /// 分类名；侧栏按它分组展示（trim 后非空才生效），`None` = 未分类。v1 只保留字段。
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

    /// 循环切换：系统 → 浅色 → 深色 → 系统。
    pub fn next(self) -> ThemePref {
        match self {
            ThemePref::System => ThemePref::Light,
            ThemePref::Light => ThemePref::Dark,
            ThemePref::Dark => ThemePref::System,
        }
    }
}

/// 界面语言偏好：跟随系统，或固定英文 / 中文。
///
/// 序列化值就是 BCP 47 语言标签（`"en"` / `"zh-CN"`），`"system"` 表示跟随系统。
/// 简体与繁体系统语言都落到 `zh-CN`（目前只有一套中文文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LanguagePref {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    Chinese,
}

impl LanguagePref {
    pub const ALL: [LanguagePref; 3] = [
        LanguagePref::System,
        LanguagePref::English,
        LanguagePref::Chinese,
    ];
}

/// 这次请求走哪个 HTTP 版本。只活在当前 Tab 的这次会话里，不进 [`RequestDraft`]、不落盘。
///
/// 版本是 ALPN 协商的结果，只能在 client 级别定死（`http1_only` /
/// `http2_prior_knowledge`）——reqwest 的 per-request `version()` 只是个断言，
/// 协商不上就直接报 `UserUnsupportedVersion`。所以每个偏好各配一个 client。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HttpVersionPref {
    /// ALPN 自己谈：https 优先 h2，谈不拢回落 http/1.1
    #[default]
    Auto,
    Http1,
    Http2,
}

impl HttpVersionPref {
    /// 顺序即下拉菜单里的顺序。
    pub const ALL: [HttpVersionPref; 3] = [
        HttpVersionPref::Auto,
        HttpVersionPref::Http1,
        HttpVersionPref::Http2,
    ];

    /// 协议名照写不翻译；`Auto` 不是协议名，UI 那边会换成本地化文案。
    pub fn label(self) -> &'static str {
        match self {
            HttpVersionPref::Auto => "Auto",
            HttpVersionPref::Http1 => "HTTP/1.1",
            HttpVersionPref::Http2 => "HTTP/2",
        }
    }
}

/// 发送请求的行为设置（`settings.json` 的 `request` 段）；改动后要重建 HTTP client。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestSettings {
    /// 整个请求（连接 + 读完响应）的总超时，秒；0 表示不限（默认值，见 [`default_timeout_secs`]）。
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// 是否自动跟随 3xx 跳转。
    #[serde(default = "default_true")]
    pub follow_redirects: bool,
    /// 跟随跳转的最大次数（仅 follow_redirects 为真时有效）。
    #[serde(default = "default_max_redirects")]
    pub max_redirects: u32,
    /// 校验 HTTPS 证书。默认**打开**（安全默认，与 curl / Postman / Insomnia 一致）：
    /// 坏证书直接握手失败。调试本地自签名接口时可在设置里关闭——关掉后仍会
    /// 解析对端证书并在有问题时提示（见 [`crate::tls`]），不是「不管了」。
    #[serde(default = "default_true")]
    pub verify_tls: bool,
    /// 被用户关掉的默认请求头（小写 key，取值见 [`crate::http::DEFAULT_HEADERS`]）。
    /// 空表示全部启用。
    ///
    /// 存「禁用清单」而不是「启用清单」：往 `DEFAULT_HEADERS` 里加条目时，
    /// 老的 `settings.json` 不必迁移，新头自动生效。
    #[serde(default)]
    pub disabled_default_headers: Vec<String>,
}

/// 默认**不限**总超时。调接口时「慢」和「挂了」得由人来判断：卡在 30 s 上限
/// 会把一个还在跑的慢查询报成超时错误，而真正卡死的请求随时可以按取消。
fn default_timeout_secs() -> u64 {
    0
}
fn default_max_redirects() -> u32 {
    10
}
fn default_true() -> bool {
    true
}

impl Default for RequestSettings {
    fn default() -> Self {
        Self {
            timeout_secs: default_timeout_secs(),
            follow_redirects: true,
            max_redirects: default_max_redirects(),
            verify_tls: true,
            disabled_default_headers: Vec::new(),
        }
    }
}

/// 应用设置（`settings.json`）。布局类状态在 [`WorkspaceState`]，这里只放用户显式调整的偏好。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub request: RequestSettings,
    /// 请求 / 响应编辑器的等宽字号（px）。
    #[serde(default = "default_editor_font_size")]
    pub editor_font_size: u32,
    /// 启动后自动向 GitHub Releases 查询一次新版本（只检查，不下载）。
    #[serde(default = "default_true")]
    pub check_updates_on_launch: bool,
    /// 界面语言；旧版 settings.json 没有这个字段时按跟随系统处理。
    #[serde(default)]
    pub language: LanguagePref,
    /// 请求体编辑器自动换行。**默认开**：请求体是自己写的，横向找不到行尾比多几行更烦。
    #[serde(default = "default_true")]
    pub wrap_request_body: bool,
    /// 响应体编辑器自动换行。**默认关**：响应多是美化过的结构化文本，保持行与缩进
    /// 对齐才好扫；需要看长行时随时可以开。
    #[serde(default)]
    pub wrap_response_body: bool,
}

pub const EDITOR_FONT_SIZE_RANGE: std::ops::RangeInclusive<u32> = 10..=24;

fn default_editor_font_size() -> u32 {
    13
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            request: RequestSettings::default(),
            editor_font_size: default_editor_font_size(),
            check_updates_on_launch: true,
            language: LanguagePref::System,
            wrap_request_body: true,
            wrap_response_body: false,
        }
    }
}

/// 请求 / 响应分栏方向（spec §7.1）：左右（默认，响应区在右侧）或上下。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    Vertical,
    #[default]
    Horizontal,
}

/// 工作区状态（`workspace.json`）：只有布局与顺序，不含任何请求内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceState {
    #[serde(default)]
    pub tab_order: Vec<TabId>,
    #[serde(default)]
    pub active: Option<TabId>,
    /// 侧栏宽度（逻辑像素）；None 表示用 app 层的默认值（`SIDEBAR_DEFAULT_WIDTH`）。
    #[serde(default)]
    pub sidebar_width: Option<f32>,
    /// 侧栏是否收成图标栏。首次启动默认收起：主工作区是请求 / 响应，
    /// 列表按需展开；旧文件没有此字段时同样视为收起。
    #[serde(default = "default_sidebar_collapsed")]
    pub sidebar_collapsed: bool,
    #[serde(default)]
    pub theme: ThemePref,
    /// 请求 / 响应分栏方向；旧文件没有此字段时为左右（响应区在右侧）。
    #[serde(default)]
    pub split: SplitDirection,
    /// 标签栏行数：1 = 单行横向滚动，[`MAX_TAB_ROWS`] = 多行分页。
    /// 旧文件没有此字段时为单行；越界值读入后夹回合法范围。
    #[serde(default = "default_tab_rows")]
    pub tab_rows: u8,
}

/// 标签栏多行模式的行数上限。再多行标签栏就该自己占半屏了。
pub const MAX_TAB_ROWS: u8 = 3;

fn default_tab_rows() -> u8 {
    1
}

fn default_sidebar_collapsed() -> bool {
    true
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            tab_order: Vec::new(),
            active: None,
            sidebar_width: None,
            sidebar_collapsed: default_sidebar_collapsed(),
            theme: ThemePref::default(),
            split: SplitDirection::default(),
            tab_rows: default_tab_rows(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseMeta {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub duration: Duration,
    /// 首个 body 字节到达的耗时（TTFB）。空 body 的响应为 None。
    pub ttfb: Option<Duration>,
    pub body_len: u64,
    pub content_type: Option<String>,
    /// 实际协商到的版本（"HTTP/1.1" / "HTTP/2"）。选了 Auto 时这是唯一能知道
    /// 到底走了哪条路的地方。
    pub http_version: Option<String>,
    /// 对端叶子证书；只有 https 且握手成功才有。响应不落盘，所以这里放
    /// 解析好的结构没有兼容负担。
    ///
    /// Box 是因为它有近 240 字节，而绝大多数请求（http、以及不看证书的场景）
    /// 用不上——inline 会把整个 `ResponseState` 撑到每次移动都拖着这坨。
    pub certificate: Option<Box<CertificateInfo>>,
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

    /// 标签栏的 method 角标列只有 40 px，缩写不能比它还宽。
    #[test]
    fn method_short_names_fit_the_tab_badge() {
        for m in Method::ALL {
            assert!(m.short().len() <= 5, "{} 太长，会挤掉标签标题", m.short());
        }
        assert_eq!(Method::Delete.short(), "DEL");
        assert_eq!(Method::Options.short(), "OPT");
        // 本来就短的不动，保持和侧栏的 method 列一致
        assert_eq!(Method::Get.short(), "GET");
        assert_eq!(Method::Patch.short(), "PATCH");
    }

    #[test]
    fn http_version_pref_defaults_to_auto() {
        assert_eq!(HttpVersionPref::default(), HttpVersionPref::Auto);
        // ALL 的顺序就是下拉菜单顺序，Auto 排最前
        assert_eq!(HttpVersionPref::ALL[0], HttpVersionPref::Auto);
        // 协议名照写不翻译
        assert_eq!(HttpVersionPref::Http1.label(), "HTTP/1.1");
        assert_eq!(HttpVersionPref::Http2.label(), "HTTP/2");
    }

    /// `ALL` 的顺序就是格式分段控件上的位置，UI 靠下标做选中与切换。
    #[test]
    fn raw_format_roundtrips_through_index() {
        for (ix, format) in RawFormat::ALL.iter().enumerate() {
            assert_eq!(format.index(), ix);
            assert_eq!(RawFormat::from_index(ix), *format);
        }
        assert_eq!(RawFormat::ALL[0], RawFormat::Json, "JSON 排在第一位");
        // 越界回落到 JSON，不 panic
        assert_eq!(RawFormat::from_index(99), RawFormat::Json);
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
                enabled: false,
                ..KeyValue::new("X", "1")
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
        // 旧 workspace.json 没有 tab_rows：单行，与改动前的标签栏一致
        assert_eq!(ws.tab_rows, 1);
        let rows: WorkspaceState = serde_json::from_str(r#"{"tab_rows":3}"#).unwrap();
        assert_eq!(rows.tab_rows, MAX_TAB_ROWS);
        // 首次启动侧栏收成图标栏；旧文件缺字段时同样收起
        assert!(ws.sidebar_collapsed);
        let expanded: WorkspaceState =
            serde_json::from_str(r#"{"sidebar_collapsed": false}"#).unwrap();
        assert!(!expanded.sidebar_collapsed);
    }

    #[test]
    fn app_settings_tolerate_missing_fields_and_round_trip() {
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, AppSettings::default());
        // 默认不限：慢接口不该被报成超时，卡死的随时可以取消
        assert_eq!(s.request.timeout_secs, 0);
        assert!(s.request.follow_redirects);
        assert_eq!(s.request.max_redirects, 10);
        // 默认打开（安全默认）：坏证书直接握手失败；调自签接口在设置里显式关
        assert!(s.request.verify_tls);
        // 空清单 = 默认头全启用；老 settings.json 没有这个字段时就落到这里
        assert!(s.request.disabled_default_headers.is_empty());
        assert_eq!(s.editor_font_size, 13);
        assert!(s.check_updates_on_launch);
        // 请求体默认换行、响应体默认不换行；两者都能被老 settings.json 的缺字段兜住
        assert!(s.wrap_request_body);
        assert!(!s.wrap_response_body);

        // 老配置里显式关过的必须原样保留：改默认值不该动用户已经做过的选择
        let partial: AppSettings = serde_json::from_str(
            r#"{"request":{"verify_tls":false},"editor_font_size":16,"check_updates_on_launch":false}"#,
        )
        .unwrap();
        assert!(!partial.request.verify_tls);
        assert_eq!(partial.request.timeout_secs, 0);
        assert!(partial.request.disabled_default_headers.is_empty());

        // 关掉过默认头的配置读回来要原样保留
        let disabled: AppSettings = serde_json::from_str(
            r#"{"request":{"disabled_default_headers":["user-agent","connection"]}}"#,
        )
        .unwrap();
        assert_eq!(
            disabled.request.disabled_default_headers,
            ["user-agent", "connection"]
        );
        assert_eq!(partial.editor_font_size, 16);
        assert!(!partial.check_updates_on_launch);

        let back: AppSettings =
            serde_json::from_str(&serde_json::to_string(&partial).unwrap()).unwrap();
        assert_eq!(back, partial);
    }

    #[test]
    fn theme_pref_cycles_and_serializes_snake_case() {
        assert_eq!(ThemePref::System.next(), ThemePref::Light);
        assert_eq!(ThemePref::Light.next(), ThemePref::Dark);
        assert_eq!(ThemePref::Dark.next(), ThemePref::System);
        assert_eq!(serde_json::to_string(&ThemePref::Dark).unwrap(), "\"dark\"");
    }

    /// 语言偏好序列化成 BCP 47 标签；旧 settings.json 没有 language 字段时回落到跟随系统。
    #[test]
    fn language_pref_serializes_as_language_tag_and_defaults_to_system() {
        assert_eq!(
            serde_json::to_string(&LanguagePref::Chinese).unwrap(),
            "\"zh-CN\""
        );
        assert_eq!(
            serde_json::to_string(&LanguagePref::English).unwrap(),
            "\"en\""
        );
        assert_eq!(
            serde_json::from_str::<LanguagePref>("\"system\"").unwrap(),
            LanguagePref::System
        );
        let legacy: AppSettings =
            serde_json::from_str(r#"{"editor_font_size":14,"check_updates_on_launch":false}"#)
                .unwrap();
        assert_eq!(legacy.language, LanguagePref::System);
        assert_eq!(legacy.editor_font_size, 14);
        assert!(legacy.wrap_request_body);
        assert!(!legacy.wrap_response_body);
    }

    #[test]
    fn now_ms_is_after_2026() {
        assert!(now_ms() > 1_767_225_600_000); // 2026-01-01T00:00:00Z
    }

    #[test]
    fn split_direction_defaults_to_horizontal_and_roundtrips() {
        // 响应区默认在右侧：没有 split 字段的旧 workspace.json 也按左右处理
        let ws: WorkspaceState = serde_json::from_str("{}").unwrap();
        assert_eq!(ws.split, SplitDirection::Horizontal);
        let ws: WorkspaceState = serde_json::from_str(r#"{"split":"horizontal"}"#).unwrap();
        assert_eq!(ws.split, SplitDirection::Horizontal);
        assert_eq!(
            serde_json::to_string(&SplitDirection::Horizontal).unwrap(),
            "\"horizontal\""
        );
    }

    #[test]
    fn key_value_description_defaults_empty_and_is_skipped_when_empty() {
        let kv: KeyValue =
            serde_json::from_str(r#"{"key":"a","value":"b","enabled":true}"#).unwrap();
        assert_eq!(kv.description, "");
        let json = serde_json::to_string(&KeyValue::new("a", "b")).unwrap();
        assert!(!json.contains("description"), "{json}");
        let with = KeyValue {
            description: "备注".into(),
            ..KeyValue::new("a", "b")
        };
        let back: KeyValue = serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
        assert_eq!(back, with);
    }

    #[test]
    fn form_data_body_serde_roundtrip() {
        let d = RequestDraft {
            body: BodyKind::FormData {
                fields: vec![
                    FormField::text("note", "hi"),
                    FormField {
                        description: "头像".into(),
                        ..FormField::file("avatar", PathBuf::from("/tmp/a.png"))
                    },
                    FormField {
                        enabled: false,
                        ..FormField::text("off", "")
                    },
                ],
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains(r#""kind":"form_data""#), "{json}");
        assert!(json.contains(r#""kind":"file""#), "{json}");
        assert!(json.contains(r#""kind":"text""#), "{json}");
        let back: RequestDraft = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn legacy_file_body_loads_as_binary_and_saves_as_binary() {
        let legacy = r#"{"method":"POST","url":"","body":{"kind":"file","path":"/tmp/x.bin","content_type":null}}"#;
        let d: RequestDraft = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            d.body,
            BodyKind::Binary {
                path: PathBuf::from("/tmp/x.bin"),
                content_type: None
            }
        );
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains(r#""kind":"binary""#), "{json}");
        assert!(!json.contains(r#""kind":"file""#), "{json}");
    }

    #[test]
    fn form_value_defaults_to_empty_text() {
        assert_eq!(
            FormValue::default(),
            FormValue::Text {
                value: String::new()
            }
        );
        let f = FormField::file("doc", PathBuf::from("/tmp/d.pdf"));
        assert!(f.enabled);
        assert_eq!(
            f.value,
            FormValue::File {
                path: "/tmp/d.pdf".into(),
                content_type: None
            }
        );
    }
}
