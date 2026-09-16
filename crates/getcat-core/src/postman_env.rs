//! Postman environment / globals JSON 的读写（互操作，spec「Postman environment 导入 / 导出」）。
//!
//! 格式：`{ "name", "values": [{ "key", "value", "enabled", "type": "default" | "secret" }],
//! "_postman_variable_scope": "environment" | "globals" }`。`type: secret` ↔ [`Variable::secret`]。
//! 导出的 secret 值是明文——Postman 自己的导出也是，界面上要提示。

use serde::{Deserialize, Deserializer, Serialize};

use crate::model::{Variable, default_true};
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

/// 导入用：只读需要的字段；其它字段（`id`、`_postman_exported_at` …）不论取值与类型一律忽略。
#[derive(Deserialize)]
struct PostmanFileIn {
    #[serde(default, deserialize_with = "string_or_empty")]
    name: String,
    values: Vec<PostmanValueIn>,
    #[serde(default, rename = "_postman_variable_scope")]
    scope: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct PostmanValueIn {
    key: String,
    #[serde(default)]
    value: serde_json::Value,
    /// 缺省、`null` 或不是布尔时按启用处理。
    #[serde(default = "default_true", deserialize_with = "bool_or_true")]
    enabled: bool,
    /// 只有 `"type": "secret"` 算 secret；缺省、`null`、其它值或类型都按 default。
    #[serde(default, rename = "type", deserialize_with = "is_secret_type")]
    secret: bool,
}

fn string_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_str().unwrap_or_default().to_string())
}

fn bool_or_true<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(serde_json::Value::deserialize(d)?.as_bool().unwrap_or(true))
}

fn is_secret_type<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(serde_json::Value::deserialize(d)?.as_str() == Some("secret"))
}

/// 导出用：字段顺序与 Postman 自己的导出一致。
#[derive(Serialize)]
struct PostmanFileOut<'a> {
    id: String,
    name: &'a str,
    values: Vec<PostmanValueOut<'a>>,
    #[serde(rename = "_postman_variable_scope")]
    scope: &'static str,
    #[serde(rename = "_postman_exported_at")]
    exported_at: String,
    #[serde(rename = "_postman_exported_using")]
    exported_using: String,
}

#[derive(Serialize)]
struct PostmanValueOut<'a> {
    key: &'a str,
    value: &'a str,
    enabled: bool,
    #[serde(rename = "type")]
    kind: &'static str,
}

/// JSON 值转字符串：string → as-is; null/missing → ""; 其它 → JSON 文本。
fn value_to_string(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => val.to_string(),
    }
}

/// scope 值转枚举：只有 "globals" 字符串才是 Globals，其它都是 Environment。
fn scope_from_value(val: &Option<serde_json::Value>) -> PostmanScope {
    match val {
        Some(serde_json::Value::String(s)) if s == "globals" => PostmanScope::Globals,
        _ => PostmanScope::Environment,
    }
}

pub fn parse(text: &str) -> Result<PostmanEnv, PostmanEnvError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| PostmanEnvError::Json(e.to_string()))?;
    // 先验形状：必须是对象且 values 是数组
    if !value.is_object() {
        return Err(PostmanEnvError::NotEnvironment);
    }
    if !value.get("values").is_some_and(serde_json::Value::is_array) {
        return Err(PostmanEnvError::NotEnvironment);
    }

    // 验证 values 数组的每个条目都是对象且有字符串 key
    if let Some(vals) = value.get("values").and_then(serde_json::Value::as_array) {
        for val in vals {
            if !val.is_object() {
                return Err(PostmanEnvError::NotEnvironment);
            }
            // key 必须存在且为字符串
            if !val.get("key").is_some_and(serde_json::Value::is_string) {
                return Err(PostmanEnvError::NotEnvironment);
            }
        }
    }

    let file: PostmanFileIn =
        serde_json::from_value(value).map_err(|_| PostmanEnvError::NotEnvironment)?;
    Ok(PostmanEnv {
        name: file.name,
        scope: scope_from_value(&file.scope),
        variables: file
            .values
            .into_iter()
            .map(|v| Variable {
                key: v.key,
                value: value_to_string(&v.value),
                enabled: v.enabled,
                secret: v.secret,
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
    let file = PostmanFileOut {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        values: variables
            .iter()
            .map(|v| PostmanValueOut {
                key: &v.key,
                value: &v.value,
                enabled: v.enabled,
                kind: if v.secret { "secret" } else { "default" },
            })
            .collect(),
        scope: match scope {
            PostmanScope::Environment => "environment",
            PostmanScope::Globals => "globals",
        },
        exported_at: iso_utc(secs),
        exported_using: format!("GetCat/{}", env!("CARGO_PKG_VERSION")),
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

    #[test]
    fn tolerates_non_string_values_and_unknown_scope() {
        // 非字符串值：数字 → JSON 文本，布尔 → JSON 文本，null → 空字符串，对象 → JSON 文本
        let text = r#"{
          "name": "Mixed",
          "values": [
            {"key": "port", "value": 8080, "enabled": true},
            {"key": "debug", "value": true, "enabled": true},
            {"key": "empty", "value": null, "enabled": true},
            {"key": "config", "value": {"a": 1}, "enabled": true}
          ]
        }"#;
        let env = parse(text).unwrap();
        assert_eq!(env.variables[0].value, "8080");
        assert_eq!(env.variables[1].value, "true");
        assert_eq!(env.variables[2].value, "");
        assert_eq!(env.variables[3].value, r#"{"a":1}"#);

        // 未知 scope 值或非字符串 scope → Environment
        let unknown_scope =
            parse(r#"{"name":"x","values":[],"_postman_variable_scope":"workspace"}"#).unwrap();
        assert_eq!(unknown_scope.scope, PostmanScope::Environment);

        let numeric_scope =
            parse(r#"{"name":"x","values":[],"_postman_variable_scope":123}"#).unwrap();
        assert_eq!(numeric_scope.scope, PostmanScope::Environment);

        // 缺少 key 或 key 不是字符串 → NotEnvironment
        assert_eq!(
            parse(r#"{"values":[{"value":"x"}]}"#),
            Err(PostmanEnvError::NotEnvironment)
        );
    }

    /// 只读 name / values / scope 与条目的 key / value / enabled / type；
    /// 其它字段是 null 或类型不对都不影响导入，enabled / type 不对时按默认处理。
    #[test]
    fn tolerates_null_or_mistyped_fields_it_does_not_need() {
        let text = r#"{
          "id": null,
          "name": "Loose",
          "values": [
            {"key": "a", "value": "1", "enabled": null, "type": 3},
            {"key": "b", "value": "2", "enabled": "no", "type": null},
            {"key": "c", "value": "3", "enabled": false, "type": "secret", "description": null}
          ],
          "_postman_variable_scope": "environment",
          "_postman_exported_at": null,
          "_postman_exported_using": 7
        }"#;
        let env = parse(text).unwrap();
        assert_eq!(env.name, "Loose");
        assert_eq!(
            env.variables,
            vec![
                Variable::new("a", "1"),
                Variable::new("b", "2"),
                Variable {
                    enabled: false,
                    secret: true,
                    ..Variable::new("c", "3")
                },
            ]
        );
        // 名字为 null 与缺名字一样，由调用方补
        assert_eq!(parse(r#"{"name":null,"values":[]}"#).unwrap().name, "");
    }

    #[test]
    fn render_writes_a_uuid_id_and_export_metadata() {
        let text = render("Dev", PostmanScope::Environment, &[]);
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        let id = doc["id"].as_str().unwrap();
        assert_eq!(
            uuid::Uuid::parse_str(id).map(|u| u.get_version_num()),
            Ok(4),
            "{id}"
        );
        assert_ne!(
            render("Dev", PostmanScope::Environment, &[]),
            text,
            "每次导出新 id"
        );
        assert!(doc["_postman_exported_at"].as_str().unwrap().ends_with('Z'));
        assert!(
            doc["_postman_exported_using"]
                .as_str()
                .unwrap()
                .starts_with("GetCat/")
        );
        assert_eq!(doc["values"], serde_json::json!([]));
    }
}
