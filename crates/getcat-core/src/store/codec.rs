//! 版本化 JSON 文档：每个文件顶层带 `"version": N`；读取时按版本迁移，未知版本视为损坏。

use std::{collections::BTreeMap, io, path::PathBuf};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;

/// 当前文件格式版本。字段增删靠 serde 默认值；结构性改动递增此值并在 `migrate` 中写迁移分支。
pub const FORMAT_VERSION: u64 = 1;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("无法确定数据目录（找不到用户主目录）")]
    NoDataDir,
    #[error("数据目录不可写：{path}（{source}）")]
    Unwritable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("IO 错误：{0}")]
    Io(#[from] io::Error),
    #[error("JSON 错误：{0}")]
    Json(#[from] serde_json::Error),
    #[error("缺少 version 字段")]
    MissingVersion,
    #[error("不支持的文件版本 {0}（当前 {FORMAT_VERSION}）")]
    UnsupportedVersion(u64),
}

/// 序列化为美化 JSON（键按字母序，末尾换行），顶层注入 `"version"`。文档必须是 JSON 对象。
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    let mut doc = serde_json::to_value(value)?;
    match &mut doc {
        Value::Object(map) => {
            map.insert("version".into(), Value::from(FORMAT_VERSION));
        }
        _ => {
            return Err(StoreError::Json(
                <serde_json::Error as serde::ser::Error>::custom("document must be a JSON object"),
            ));
        }
    }
    let doc = sort_keys(doc);
    let mut bytes = serde_json::to_vec_pretty(&doc)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// 递归按键的字典序重建每个对象，使输出与 `serde_json` 是否启用 `preserve_order`
/// feature（例如通过 gpui → schemars 的 feature unification）无关，始终按字母序落盘。
fn sort_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> =
                map.into_iter().map(|(k, v)| (k, sort_keys(v))).collect();
            let mut out = serde_json::Map::new();
            for (k, v) in sorted {
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

/// 读取：先取 `version`，交给 `migrate` 改写到当前结构，再反序列化。未知字段忽略、缺失字段取默认值。
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    let value: Value = serde_json::from_slice(bytes)?;
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or(StoreError::MissingVersion)?;
    let value = migrate(version, value)?;
    Ok(serde_json::from_value(value)?)
}

/// 显式迁移函数：新增版本时在此添加 `v => { 改写 value }` 分支。未知版本视为损坏（调用方会隔离文件）。
fn migrate(version: u64, value: Value) -> Result<Value, StoreError> {
    match version {
        FORMAT_VERSION => Ok(value),
        other => Err(StoreError::UnsupportedVersion(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyValue, Method, RequestDraft, SavedRequest, WorkspaceState};

    #[test]
    fn encode_stamps_version_and_decode_roundtrips() {
        let req = SavedRequest::new(
            "x",
            RequestDraft {
                method: Method::Post,
                ..Default::default()
            },
        );
        let bytes = encode(&req).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
        assert!(text.ends_with('\n'));
        let back: SavedRequest = decode(&bytes).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn decode_rejects_missing_version() {
        let err = decode::<WorkspaceState>(b"{}").unwrap_err();
        assert!(matches!(err, StoreError::MissingVersion), "{err:?}");
    }

    #[test]
    fn decode_rejects_unknown_version() {
        let err = decode::<WorkspaceState>(br#"{"version": 2}"#).unwrap_err();
        assert!(matches!(err, StoreError::UnsupportedVersion(2)), "{err:?}");
    }

    #[test]
    fn decode_ignores_unknown_fields_and_fills_defaults() {
        let ws: WorkspaceState =
            decode(br#"{"version": 1, "sidebar_collapsed": false, "future_field": [1, 2]}"#)
                .unwrap();
        assert!(!ws.sidebar_collapsed);
        assert!(ws.tab_order.is_empty());
    }

    #[test]
    fn decode_reports_invalid_json() {
        let err = decode::<WorkspaceState>(b"{not json").unwrap_err();
        assert!(matches!(err, StoreError::Json(_)), "{err:?}");
    }

    #[test]
    fn encode_rejects_non_object_documents() {
        assert!(matches!(encode(&5u32), Err(StoreError::Json(_))));
    }

    /// 键必须按字母序落盘，与 `serde_json` 是否启用 `preserve_order` feature 无关
    /// （真实 app 构建中 gpui → schemars 的 feature unification 会打开它）。
    #[test]
    fn encode_sorts_keys_deterministically_regardless_of_serde_json_features() {
        let bytes = encode(&WorkspaceState::default()).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let expected = "{\n  \"active\": null,\n  \"sidebar_collapsed\": true,\n  \"sidebar_width\": null,\n  \"split\": \"horizontal\",\n  \"tab_order\": [],\n  \"theme\": \"system\",\n  \"version\": 1\n}\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn encode_sorts_nested_object_keys() {
        let req = SavedRequest::new(
            "x",
            RequestDraft {
                headers: vec![KeyValue::new("X-Test", "1")],
                ..Default::default()
            },
        );
        let bytes = encode(&req).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let enabled_pos = text.find("\"enabled\"").expect("enabled key present");
        let key_pos = text.find("\"key\"").expect("key key present");
        let value_pos = text.find("\"value\"").expect("value key present");
        assert!(enabled_pos < key_pos, "{text}");
        assert!(key_pos < value_pos, "{text}");
    }
}
