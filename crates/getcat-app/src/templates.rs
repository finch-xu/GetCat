//! 内置请求模板：点一下就得到一个填好 URL / 认证头 / 请求体的新 Tab。
//!
//! 全是编译期常量，只读、不落盘、不支持用户自定义。
//! 模板的主要价值在多模态请求体上——三家接口的写法互不兼容：
//! OpenAI Chat Completions 用 `image_url`、Responses 用 `input_image`、
//! Anthropic 用 `image` + `source`，且 Anthropic 把图片块排在文字块之前。
//!
//! 认证一律填占位符文本，由用户手动替换（应用没有变量 / 环境系统）。
//! 多模态模板共用一张 Wikimedia 上的公开图片，两家接口都能直接取到。

use getcat_core::model::{BodyKind, KeyValue, Method, RawFormat, RequestDraft};
use gpui::SharedString;

use crate::i18n::tr;

/// 模板分组。名字是产品专名，不进 i18n。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateGroup {
    OpenAi,
    Anthropic,
    Mcp,
}

impl TemplateGroup {
    pub const ALL: [TemplateGroup; 3] = [
        TemplateGroup::OpenAi,
        TemplateGroup::Anthropic,
        TemplateGroup::Mcp,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TemplateGroup::OpenAi => "OpenAI",
            TemplateGroup::Anthropic => "Anthropic",
            TemplateGroup::Mcp => "MCP",
        }
    }
}

/// 行副标题的维度。大模型接口：最简纯文本 / 带图片的多模态；
/// MCP：新版无状态（2026-07-28）/ 旧版会话式（2025-06-18）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateVariant {
    Text,
    Vision,
    McpStateless,
    McpSession,
}

impl TemplateVariant {
    pub fn label(self) -> SharedString {
        match self {
            TemplateVariant::Text => tr!("sidebar.templates.variant_text"),
            TemplateVariant::Vision => tr!("sidebar.templates.variant_vision"),
            TemplateVariant::McpStateless => tr!("sidebar.templates.variant_mcp_stateless"),
            TemplateVariant::McpSession => tr!("sidebar.templates.variant_mcp_session"),
        }
    }
}

pub struct RequestTemplate {
    /// 稳定标识：既是列表的 element id，也是 `find` 的查找键。
    pub id: &'static str,
    pub group: TemplateGroup,
    /// 接口名，如 "Chat Completions"。技术专名，不进 i18n。
    pub api: &'static str,
    pub variant: TemplateVariant,
    pub method: Method,
    pub url: &'static str,
    pub headers: &'static [(&'static str, &'static str)],
    /// JSON 请求体，已格式化好，直接进编辑器。
    pub body: &'static str,
}

impl RequestTemplate {
    /// 展开成一份普通草稿，交给 `RequestTab::load_draft`。
    pub fn draft(&self) -> RequestDraft {
        RequestDraft {
            method: self.method,
            url: self.url.to_string(),
            path_params: Vec::new(),
            params: Vec::new(),
            headers: self
                .headers
                .iter()
                .map(|(k, v)| KeyValue::new(*k, *v))
                .collect(),
            body: BodyKind::Raw {
                format: RawFormat::Json,
                text: self.body.to_string(),
            },
        }
    }

    /// Tab 标题与列表主标题。
    pub fn display_name(&self) -> SharedString {
        SharedString::from(self.api)
    }
}

pub fn all() -> &'static [RequestTemplate] {
    TEMPLATES
}

pub fn find(id: &str) -> Option<&'static RequestTemplate> {
    TEMPLATES.iter().find(|t| t.id == id)
}

const OPENAI_HEADERS: &[(&str, &str)] = &[
    ("Content-Type", "application/json"),
    ("Authorization", "Bearer YOUR_API_KEY"),
];

const ANTHROPIC_HEADERS: &[(&str, &str)] = &[
    ("Content-Type", "application/json"),
    ("x-api-key", "YOUR_API_KEY"),
    ("anthropic-version", "2023-06-01"),
];

// ── MCP（Model Context Protocol，Streamable HTTP 传输）────────────────
//
// 新版 2026-07-28 是无状态的：`MCP-Protocol-Version` / `Mcp-Method` / `Mcp-Name`
// 三个头必须与 body 镜像一致（不一致服务器返回 -32020 HeaderMismatch），
// 版本与客户端身份放在 params._meta 里。
// 旧版 2025-06-18 是会话式的：先 initialize 从响应头拿 Mcp-Session-Id，
// 发 notifications/initialized，之后的请求都带会话头。
// Accept 两个时代都必须同时声明 application/json 与 text/event-stream。
// URL 默认指向仓库自带的手测服务器（tools/testserver/server.py 的 /mcp）。

const MCP_URL: &str = "http://127.0.0.1:8765/mcp";

const MCP_TOOLS_LIST_HEADERS: &[(&str, &str)] = &[
    ("Content-Type", "application/json"),
    ("Accept", "application/json, text/event-stream"),
    ("MCP-Protocol-Version", "2026-07-28"),
    ("Mcp-Method", "tools/list"),
    ("Authorization", "Bearer YOUR_API_KEY"),
];

const MCP_TOOLS_CALL_HEADERS: &[(&str, &str)] = &[
    ("Content-Type", "application/json"),
    ("Accept", "application/json, text/event-stream"),
    ("MCP-Protocol-Version", "2026-07-28"),
    ("Mcp-Method", "tools/call"),
    ("Mcp-Name", "echo"),
    ("Authorization", "Bearer YOUR_API_KEY"),
];

// initialize 不带 MCP-Protocol-Version 头：旧版规范里该头用于握手之后的请求。
const MCP_INITIALIZE_HEADERS: &[(&str, &str)] = &[
    ("Content-Type", "application/json"),
    ("Accept", "application/json, text/event-stream"),
    ("Authorization", "Bearer YOUR_API_KEY"),
];

// YOUR_SESSION_ID 占位符：值从 initialize 响应头的 Mcp-Session-Id 复制。
const MCP_SESSION_HEADERS: &[(&str, &str)] = &[
    ("Content-Type", "application/json"),
    ("Accept", "application/json, text/event-stream"),
    ("MCP-Protocol-Version", "2025-06-18"),
    ("Mcp-Session-Id", "YOUR_SESSION_ID"),
    ("Authorization", "Bearer YOUR_API_KEY"),
];

static TEMPLATES: &[RequestTemplate] = &[
    RequestTemplate {
        id: "openai-chat-text",
        group: TemplateGroup::OpenAi,
        api: "Chat Completions",
        variant: TemplateVariant::Text,
        method: Method::Post,
        url: "https://api.openai.com/v1/chat/completions",
        headers: OPENAI_HEADERS,
        body: OPENAI_CHAT_TEXT,
    },
    RequestTemplate {
        id: "openai-chat-vision",
        group: TemplateGroup::OpenAi,
        api: "Chat Completions",
        variant: TemplateVariant::Vision,
        method: Method::Post,
        url: "https://api.openai.com/v1/chat/completions",
        headers: OPENAI_HEADERS,
        body: OPENAI_CHAT_VISION,
    },
    RequestTemplate {
        id: "openai-responses-text",
        group: TemplateGroup::OpenAi,
        api: "Responses",
        variant: TemplateVariant::Text,
        method: Method::Post,
        url: "https://api.openai.com/v1/responses",
        headers: OPENAI_HEADERS,
        body: OPENAI_RESPONSES_TEXT,
    },
    RequestTemplate {
        id: "openai-responses-vision",
        group: TemplateGroup::OpenAi,
        api: "Responses",
        variant: TemplateVariant::Vision,
        method: Method::Post,
        url: "https://api.openai.com/v1/responses",
        headers: OPENAI_HEADERS,
        body: OPENAI_RESPONSES_VISION,
    },
    RequestTemplate {
        id: "anthropic-messages-text",
        group: TemplateGroup::Anthropic,
        api: "Messages",
        variant: TemplateVariant::Text,
        method: Method::Post,
        url: "https://api.anthropic.com/v1/messages",
        headers: ANTHROPIC_HEADERS,
        body: ANTHROPIC_MESSAGES_TEXT,
    },
    RequestTemplate {
        id: "anthropic-messages-vision",
        group: TemplateGroup::Anthropic,
        api: "Messages",
        variant: TemplateVariant::Vision,
        method: Method::Post,
        url: "https://api.anthropic.com/v1/messages",
        headers: ANTHROPIC_HEADERS,
        body: ANTHROPIC_MESSAGES_VISION,
    },
    RequestTemplate {
        id: "mcp-tools-list",
        group: TemplateGroup::Mcp,
        api: "tools/list",
        variant: TemplateVariant::McpStateless,
        method: Method::Post,
        url: MCP_URL,
        headers: MCP_TOOLS_LIST_HEADERS,
        body: MCP_TOOLS_LIST,
    },
    RequestTemplate {
        id: "mcp-tools-call",
        group: TemplateGroup::Mcp,
        api: "tools/call",
        variant: TemplateVariant::McpStateless,
        method: Method::Post,
        url: MCP_URL,
        headers: MCP_TOOLS_CALL_HEADERS,
        body: MCP_TOOLS_CALL,
    },
    RequestTemplate {
        id: "mcp-initialize",
        group: TemplateGroup::Mcp,
        api: "initialize",
        variant: TemplateVariant::McpSession,
        method: Method::Post,
        url: MCP_URL,
        headers: MCP_INITIALIZE_HEADERS,
        body: MCP_INITIALIZE,
    },
    RequestTemplate {
        id: "mcp-initialized",
        group: TemplateGroup::Mcp,
        api: "notifications/initialized",
        variant: TemplateVariant::McpSession,
        method: Method::Post,
        url: MCP_URL,
        headers: MCP_SESSION_HEADERS,
        body: MCP_INITIALIZED,
    },
    RequestTemplate {
        id: "mcp-session-tools-call",
        group: TemplateGroup::Mcp,
        api: "tools/call",
        variant: TemplateVariant::McpSession,
        method: Method::Post,
        url: MCP_URL,
        headers: MCP_SESSION_HEADERS,
        body: MCP_SESSION_TOOLS_CALL,
    },
];

const OPENAI_CHAT_TEXT: &str = r#"{
  "model": "gpt-5.6",
  "messages": [
    { "role": "user", "content": "Explain HTTP status code 429 in one sentence." }
  ]
}"#;

// 图片块用 `image_url`，token 上限字段是 `max_completion_tokens`（不是 `max_tokens`）。
// `url` 也可以换成 data URI：`data:image/jpeg;base64,<BASE64>`。
const OPENAI_CHAT_VISION: &str = r#"{
  "model": "gpt-5.6",
  "messages": [
    {
      "role": "user",
      "content": [
        { "type": "text", "text": "What is in this image?" },
        {
          "type": "image_url",
          "image_url": {
            "url": "https://upload.wikimedia.org/wikipedia/commons/a/a7/Camponotus_flavomarginatus_ant.jpg",
            "detail": "auto"
          }
        }
      ]
    }
  ],
  "max_completion_tokens": 1024
}"#;

const OPENAI_RESPONSES_TEXT: &str = r#"{
  "model": "gpt-5.6",
  "input": "Explain HTTP status code 429 in one sentence."
}"#;

// Responses 的内容块换了一套名字：`input_text` / `input_image`，
// 且 `input_image` 的 `image_url` 直接是字符串，不是对象。上限字段是 `max_output_tokens`。
const OPENAI_RESPONSES_VISION: &str = r#"{
  "model": "gpt-5.6",
  "input": [
    {
      "role": "user",
      "content": [
        { "type": "input_text", "text": "What is in this image?" },
        {
          "type": "input_image",
          "image_url": "https://upload.wikimedia.org/wikipedia/commons/a/a7/Camponotus_flavomarginatus_ant.jpg"
        }
      ]
    }
  ],
  "max_output_tokens": 1024
}"#;

// `max_tokens` 在 Anthropic 是必填的。
const ANTHROPIC_MESSAGES_TEXT: &str = r#"{
  "model": "claude-opus-5",
  "max_tokens": 1024,
  "messages": [
    { "role": "user", "content": "Explain HTTP status code 429 in one sentence." }
  ]
}"#;

// 图片块是 `image` + `source`（`type` 可为 `url` 或 `base64`），
// 且官方建议图片排在文字之前。
const ANTHROPIC_MESSAGES_VISION: &str = r#"{
  "model": "claude-opus-5",
  "max_tokens": 1024,
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "image",
          "source": {
            "type": "url",
            "url": "https://upload.wikimedia.org/wikipedia/commons/a/a7/Camponotus_flavomarginatus_ant.jpg"
          }
        },
        { "type": "text", "text": "What is in this image?" }
      ]
    }
  ]
}"#;

// 新版无状态请求：版本、客户端身份、能力全在 params._meta 里逐请求携带。
// clientInfo.version 是示例数据，固定 "1.0"，不跟随应用发版。
const MCP_TOOLS_LIST: &str = r#"{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/list",
  "params": {
    "_meta": {
      "io.modelcontextprotocol/protocolVersion": "2026-07-28",
      "io.modelcontextprotocol/clientInfo": { "name": "GetCat", "version": "1.0" },
      "io.modelcontextprotocol/clientCapabilities": {}
    }
  }
}"#;

// 改 params.name 时记得同步 Mcp-Name 头（测试 mcp_headers_mirror_body 钉着）。
const MCP_TOOLS_CALL: &str = r#"{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "echo",
    "arguments": { "text": "Hello from GetCat" },
    "_meta": {
      "io.modelcontextprotocol/protocolVersion": "2026-07-28",
      "io.modelcontextprotocol/clientInfo": { "name": "GetCat", "version": "1.0" },
      "io.modelcontextprotocol/clientCapabilities": {}
    }
  }
}"#;

// 旧版握手第一步：响应头里的 Mcp-Session-Id 要复制到后续请求。
const MCP_INITIALIZE: &str = r#"{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "protocolVersion": "2025-06-18",
    "capabilities": {},
    "clientInfo": { "name": "GetCat", "version": "1.0" }
  }
}"#;

// 握手第二步：通知没有 id，服务器应答 202 无 body。
// 严格实现（官方 SDK）在收到这条之前会拒绝 tools/* 请求。
const MCP_INITIALIZED: &str = r#"{
  "jsonrpc": "2.0",
  "method": "notifications/initialized"
}"#;

const MCP_SESSION_TOOLS_CALL: &str = r#"{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "echo",
    "arguments": { "text": "Hello from GetCat" }
  }
}"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_body_is_valid_json() {
        for template in all() {
            serde_json::from_str::<serde_json::Value>(template.body)
                .unwrap_or_else(|e| panic!("模板 {} 的请求体不是合法 JSON：{e}", template.id));
        }
    }

    #[test]
    fn ids_are_unique_and_findable() {
        let mut seen = HashSet::new();
        for template in all() {
            assert!(seen.insert(template.id), "模板 id 重复：{}", template.id);
            assert!(find(template.id).is_some());
        }
        assert!(find("nope").is_none());
    }

    /// 每个分组应当出现的变体。大模型接口的维度是"纯文本 / 含图片"，
    /// MCP 的维度是协议时代（2026-07-28 无状态 / 2025-06-18 会话式）。
    fn variants_of(group: TemplateGroup) -> &'static [TemplateVariant] {
        match group {
            TemplateGroup::OpenAi | TemplateGroup::Anthropic => {
                &[TemplateVariant::Text, TemplateVariant::Vision]
            }
            TemplateGroup::Mcp => &[TemplateVariant::McpStateless, TemplateVariant::McpSession],
        }
    }

    #[test]
    fn every_group_and_variant_has_a_template() {
        for group in TemplateGroup::ALL {
            for variant in variants_of(group) {
                assert!(
                    all()
                        .iter()
                        .any(|t| t.group == group && t.variant == *variant),
                    "{} 缺少 {variant:?} 模板",
                    group.label()
                );
            }
        }
        // 反向：没有模板挂在不属于自己分组的变体上
        for template in all() {
            assert!(
                variants_of(template.group).contains(&template.variant),
                "模板 {} 的变体 {:?} 不属于分组 {}",
                template.id,
                template.variant,
                template.group.label()
            );
        }
    }

    fn header<'a>(template: &'a RequestTemplate, name: &str) -> Option<&'a str> {
        template
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
    }

    /// MCP 2026-07-28 规范要求 `Mcp-Method` / `Mcp-Name` / `MCP-Protocol-Version`
    /// 请求头与 body 完全一致，不一致服务器返回 -32020 HeaderMismatch。
    /// 全表扫一遍，防止日后改 body 忘改头。
    #[test]
    fn mcp_headers_mirror_body() {
        for template in all().iter().filter(|t| t.group == TemplateGroup::Mcp) {
            let body: serde_json::Value = serde_json::from_str(template.body).unwrap();

            if let Some(method) = header(template, "Mcp-Method") {
                assert_eq!(
                    Some(method),
                    body["method"].as_str(),
                    "模板 {} 的 Mcp-Method 头与 body.method 不一致",
                    template.id
                );
            }
            if let Some(name) = header(template, "Mcp-Name") {
                assert_eq!(
                    Some(name),
                    body["params"]["name"].as_str(),
                    "模板 {} 的 Mcp-Name 头与 params.name 不一致",
                    template.id
                );
            }
            // 旧版会话式模板的 body 没有 _meta，此断言只约束新版无状态模板
            let meta_version =
                body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str();
            if let Some(version) = meta_version {
                assert_eq!(
                    Some(version),
                    header(template, "MCP-Protocol-Version"),
                    "模板 {} 的 MCP-Protocol-Version 头与 _meta 里的版本不一致",
                    template.id
                );
            }
        }
    }

    /// Streamable HTTP 规范：客户端 MUST 在 Accept 里同时声明两种响应形态
    /// （服务器逐请求选择返回单 JSON 还是 SSE 流）。
    #[test]
    fn mcp_templates_accept_both_response_types() {
        for template in all().iter().filter(|t| t.group == TemplateGroup::Mcp) {
            let accept = header(template, "Accept")
                .unwrap_or_else(|| panic!("模板 {} 缺 Accept 头", template.id));
            assert!(
                accept.contains("application/json") && accept.contains("text/event-stream"),
                "模板 {} 的 Accept 必须同时含 application/json 与 text/event-stream：{accept}",
                template.id
            );
        }
    }

    #[test]
    fn draft_carries_placeholder_auth_and_json_body() {
        let template = find("anthropic-messages-vision").unwrap();
        let draft = template.draft();
        assert_eq!(draft.method, Method::Post);
        assert_eq!(draft.url, "https://api.anthropic.com/v1/messages");

        let key = draft
            .headers
            .iter()
            .find(|h| h.key == "x-api-key")
            .expect("缺 x-api-key");
        assert_eq!(
            key.value, "YOUR_API_KEY",
            "认证值必须是占位符，不能是真 key"
        );
        assert!(key.enabled, "模板带来的 header 应当默认勾选");
        assert!(
            draft.headers.iter().any(|h| h.key == "anthropic-version"),
            "Anthropic 少了必填的 anthropic-version"
        );

        match draft.body {
            BodyKind::Raw { format, ref text } => {
                assert_eq!(format, RawFormat::Json);
                // 三家的图片块写法各不相同，钉住 Anthropic 这一种
                assert!(text.contains(r#""type": "image""#));
                assert!(text.contains(r#""source""#));
            }
            ref other => panic!("期望 Raw JSON 请求体，实际是 {other:?}"),
        }
    }

    /// 认证占位符一处写错就会让人拿真 key 去试，全表扫一遍。
    #[test]
    fn no_template_ships_a_real_looking_key() {
        for template in all() {
            for (key, value) in template.headers {
                if key.eq_ignore_ascii_case("authorization")
                    || key.eq_ignore_ascii_case("x-api-key")
                {
                    assert!(
                        value.contains("YOUR_API_KEY"),
                        "模板 {} 的 {key} 不是占位符：{value}",
                        template.id
                    );
                }
            }
        }
    }
}
