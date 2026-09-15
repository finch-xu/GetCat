//! Postman environment / globals JSON 的读写（互操作，spec「Postman environment 导入 / 导出」）。
//!
//! 格式：`{ "name", "values": [{ "key", "value", "enabled", "type": "default" | "secret" }],
//! "_postman_variable_scope": "environment" | "globals" }`。`type: secret` ↔ [`Variable::secret`]。
//! 导出的 secret 值是明文——Postman 自己的导出也是，界面上要提示。

use serde::{Deserialize, Serialize};

use crate::model::{Ulid, Variable};
use crate::vars::iso_utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PostmanScope {
    Environment,
    Globals,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostmanEnv {
    pub name: String,
    pub scope: PostmanScope,
    pub variables: Vec<Variable>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PostmanEnvError {
    #[error("not valid JSON: {0}")]
    Json(String),
    #[error("not a Postman environment: expected an object with a \"values\" array")]
    NotEnvironment,
}

#[derive(Serialize, Deserialize)]
struct PostmanValue {
    key: String,
    #[serde(default)]
    value: String,
    #[serde(default = "default_true")]
    enabled: bool,
    /// `"default"` / `"secret"`；其它值当 default。
    #[serde(default, rename = "type")]
    kind: String,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
struct PostmanFile {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    values: Vec<PostmanValue>,
    #[serde(default, rename = "_postman_variable_scope")]
    scope: Option<PostmanScope>,
    #[serde(
        default,
        rename = "_postman_exported_at",
        skip_serializing_if = "Option::is_none"
    )]
    exported_at: Option<String>,
    #[serde(
        default,
        rename = "_postman_exported_using",
        skip_serializing_if = "Option::is_none"
    )]
    exported_using: Option<String>,
}

pub fn parse(text: &str) -> Result<PostmanEnv, PostmanEnvError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| PostmanEnvError::Json(e.to_string()))?;
    // 先验形状再反序列化：serde 的错误分不清「不是 JSON」和「不是这个格式」
    if !value.get("values").is_some_and(serde_json::Value::is_array) {
        return Err(PostmanEnvError::NotEnvironment);
    }
    let file: PostmanFile =
        serde_json::from_value(value).map_err(|_| PostmanEnvError::NotEnvironment)?;
    Ok(PostmanEnv {
        name: file.name,
        scope: file.scope.unwrap_or(PostmanScope::Environment),
        variables: file
            .values
            .into_iter()
            .map(|v| Variable {
                key: v.key,
                value: v.value,
                enabled: v.enabled,
                secret: v.kind == "secret",
                description: String::new(),
            })
            .collect(),
    })
}

/// 美化输出（两空格缩进），与 Postman 导出的观感一致。
pub fn render(name: &str, scope: PostmanScope, variables: &[Variable]) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file = PostmanFile {
        id: Ulid::generate().to_string(),
        name: name.to_string(),
        values: variables
            .iter()
            .map(|v| PostmanValue {
                key: v.key.clone(),
                value: v.value.clone(),
                enabled: v.enabled,
                kind: if v.secret { "secret" } else { "default" }.to_string(),
            })
            .collect(),
        scope: Some(scope),
        exported_at: Some(iso_utc(secs)),
        exported_using: Some(format!("GetCat/{}", env!("CARGO_PKG_VERSION"))),
    };
    serde_json::to_string_pretty(&file).expect("plain structs serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_environment_with_secret_and_disabled_values() {
        let text = r#"{
          "id": "abc", "name": "Dev",
          "values": [
            {"key": "host", "value": "h", "enabled": true, "type": "default"},
            {"key": "token", "value": "t", "enabled": false, "type": "secret"},
            {"key": "legacy", "value": "x"}
          ],
          "_postman_variable_scope": "environment",
          "_postman_exported_at": "2026-09-15T00:00:00.000Z"
        }"#;
        let env = parse(text).unwrap();
        assert_eq!(env.name, "Dev");
        assert_eq!(env.scope, PostmanScope::Environment);
        assert_eq!(env.variables.len(), 3);
        assert_eq!(env.variables[0], Variable::new("host", "h"));
        assert!(env.variables[1].secret && !env.variables[1].enabled);
        assert!(env.variables[2].enabled, "缺 enabled 视为启用");
    }

    #[test]
    fn scope_defaults_to_environment_and_globals_is_recognised() {
        let env = parse(r#"{"values":[]}"#).unwrap();
        assert_eq!(env.scope, PostmanScope::Environment);
        assert_eq!(env.name, "", "缺名字由调用方补");
        let g = parse(r#"{"name":"g","values":[],"_postman_variable_scope":"globals"}"#).unwrap();
        assert_eq!(g.scope, PostmanScope::Globals);
    }

    #[test]
    fn rejects_non_json_and_non_environments() {
        assert!(matches!(parse("{"), Err(PostmanEnvError::Json(_))));
        assert_eq!(parse("[]"), Err(PostmanEnvError::NotEnvironment));
        assert_eq!(
            parse(r#"{"name":"x"}"#),
            Err(PostmanEnvError::NotEnvironment)
        );
        assert_eq!(
            parse(r#"{"values":{}}"#),
            Err(PostmanEnvError::NotEnvironment)
        );
    }

    #[test]
    fn render_round_trips_through_parse() {
        let vars = vec![
            Variable::new("host", "h"),
            Variable {
                secret: true,
                enabled: false,
                ..Variable::new("token", "t")
            },
        ];
        let text = render("Dev", PostmanScope::Globals, &vars);
        assert!(
            text.contains(r#""_postman_variable_scope": "globals""#),
            "{text}"
        );
        assert!(text.contains(r#""type": "secret""#), "{text}");
        let back = parse(&text).unwrap();
        assert_eq!(back.name, "Dev");
        assert_eq!(back.scope, PostmanScope::Globals);
        assert_eq!(back.variables, vars);
    }
}
