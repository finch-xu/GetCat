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
}

impl TemplateGroup {
    pub const ALL: [TemplateGroup; 2] = [TemplateGroup::OpenAi, TemplateGroup::Anthropic];

    pub fn label(self) -> &'static str {
        match self {
            TemplateGroup::OpenAi => "OpenAI",
            TemplateGroup::Anthropic => "Anthropic",
        }
    }
}

/// 同一个接口的两种请求体：最简纯文本 / 带图片的多模态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateVariant {
    Text,
    Vision,
}

impl TemplateVariant {
    pub fn label(self) -> SharedString {
        match self {
            TemplateVariant::Text => tr!("sidebar.templates.variant_text"),
            TemplateVariant::Vision => tr!("sidebar.templates.variant_vision"),
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

    #[test]
    fn every_group_and_variant_has_a_template() {
        for group in TemplateGroup::ALL {
            for variant in [TemplateVariant::Text, TemplateVariant::Vision] {
                assert!(
                    all()
                        .iter()
                        .any(|t| t.group == group && t.variant == variant),
                    "{} 缺少 {variant:?} 模板",
                    group.label()
                );
            }
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
