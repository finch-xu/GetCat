//! 版本化 JSON 文档：每个文件顶层带 `"version": N`；读取时按版本迁移，未知版本视为损坏。

use std::{io, path::PathBuf};

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
    let mut bytes = serde_json::to_vec_pretty(&doc)?;
    bytes.push(b'\n');
    Ok(bytes)
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
    use crate::model::{Method, RequestDraft, SavedRequest, WorkspaceState};

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
            decode(br#"{"version": 1, "sidebar_collapsed": true, "future_field": [1, 2]}"#)
                .unwrap();
        assert!(ws.sidebar_collapsed);
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
}
