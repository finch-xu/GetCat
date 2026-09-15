//! 可视化前后置操作的执行器（纯函数，不碰持久化）。
//!
//! - 前置：发送前依次执行，把值写进 [`VariableSets`]；值先做 `{{}}` 替换，所以能引用
//!   上一条刚设的变量和动态变量。
//! - 后置：响应完成后在**后台线程**执行（解析 JSON 是 O(n)）；结果与提取出的变量由
//!   app 层在 generation 校验通过后写回。断言的期望值在发送时已替换完。

use serde_json::Value;

use crate::model::{
    AssertOp, PostOp, PostOpKind, PreOp, PreOpKind, ResponseMeta, ResponseSource, VarScope,
    VariableSets,
};
use crate::vars::Resolver;

/// JsonPath 操作愿意解析的响应体上限：再大就解析出一棵占内存的 `Value` 树，不值得。
pub const OPS_JSON_MAX_BYTES: usize = 8 * 1024 * 1024;

/// 条件不满足、没有执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpSkip {
    NoActiveEnvironment,
    NoGroup,
}

/// 执行了但没成功。载荷是原文（路径、头名、实际值），界面按变体翻译种类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpFailure {
    EmptyKey,
    EmptyPath,
    /// 响应体已落盘，内存里没有完整内容。
    BodyUnavailable,
    /// 超过 [`OPS_JSON_MAX_BYTES`]。
    BodyTooLarge,
    NotJson,
    PathNotFound(String),
    HeaderNotFound(String),
    /// Equals / Contains 不成立。
    Mismatch {
        actual: String,
        expected: String,
    },
    /// NotEquals 不成立。
    Unexpected {
        actual: String,
    },
    /// Exists 不成立。
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpOutcome {
    Passed,
    Failed(OpFailure),
    Skipped(OpSkip),
}

/// 一条后置提取的结果，等 app 层写回变量表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub scope: VarScope,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PostReport {
    pub results: Vec<(PostOp, OpOutcome)>,
    pub extracted: Vec<Extracted>,
}

impl PostReport {
    pub fn passed(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, o)| *o == OpOutcome::Passed)
            .count()
    }
}

/// 前置操作：逐条替换值并写入；每条都按**当时**的变量表替换，所以后一条能引用前一条。
pub fn run_pre_ops(
    ops: &[PreOp],
    sets: &mut VariableSets,
    group: Option<&str>,
) -> Vec<(PreOp, OpOutcome)> {
    let mut results = Vec::new();
    for op in ops.iter().filter(|o| o.enabled) {
        let outcome = match &op.kind {
            PreOpKind::SetVariable { scope, key, value } => {
                let key = key.trim();
                if key.is_empty() {
                    OpOutcome::Failed(OpFailure::EmptyKey)
                } else {
                    let value = {
                        let ctx = sets.context(group);
                        Resolver::new(&ctx).resolve(value).into_owned()
                    };
                    if sets.set_var(*scope, group, key, &value) {
                        OpOutcome::Passed
                    } else {
                        OpOutcome::Skipped(match scope {
                            VarScope::Environment => OpSkip::NoActiveEnvironment,
                            _ => OpSkip::NoGroup,
                        })
                    }
                }
            }
        };
        results.push((op.clone(), outcome));
    }
    results
}

fn parse_body(body: Option<&[u8]>) -> Result<Value, OpFailure> {
    let bytes = body.ok_or(OpFailure::BodyUnavailable)?;
    if bytes.len() > OPS_JSON_MAX_BYTES {
        return Err(OpFailure::BodyTooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| OpFailure::NotJson)
}

fn read_source(
    source: &ResponseSource,
    meta: &ResponseMeta,
    json: Option<&Result<Value, OpFailure>>,
) -> Result<String, OpFailure> {
    match source {
        ResponseSource::Status => Ok(meta.status.to_string()),
        ResponseSource::Header { name } => {
            let wanted = name.trim();
            meta.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(wanted))
                .map(|(_, v)| v.clone())
                .ok_or_else(|| OpFailure::HeaderNotFound(name.clone()))
        }
        ResponseSource::JsonPath { path } => {
            if path.trim().is_empty() {
                return Err(OpFailure::EmptyPath);
            }
            match json.expect("json parsed when any JsonPath op is enabled") {
                Err(f) => Err(f.clone()),
                Ok(root) => json_path::get(root, path)
                    .map(json_path::value_to_string)
                    .ok_or_else(|| OpFailure::PathNotFound(path.clone())),
            }
        }
    }
}

fn compare(op: AssertOp, actual: String, expected: &str) -> OpOutcome {
    let ok = match op {
        AssertOp::Equals => actual == expected,
        AssertOp::NotEquals => actual != expected,
        AssertOp::Contains => actual.contains(expected),
        AssertOp::Exists => true,
    };
    if ok {
        return OpOutcome::Passed;
    }
    OpOutcome::Failed(match op {
        AssertOp::NotEquals => OpFailure::Unexpected { actual },
        _ => OpFailure::Mismatch {
            actual,
            expected: expected.to_string(),
        },
    })
}

/// 后置操作。`body` 为 None 表示响应体已落盘。JSON 只在有启用的 JsonPath 操作时解析一次。
pub fn run_post_ops(ops: &[PostOp], meta: &ResponseMeta, body: Option<&[u8]>) -> PostReport {
    let enabled: Vec<&PostOp> = ops.iter().filter(|o| o.enabled).collect();
    let needs_json = enabled.iter().any(|o| {
        matches!(
            &o.kind,
            PostOpKind::Extract {
                source: ResponseSource::JsonPath { .. },
                ..
            } | PostOpKind::Assert {
                subject: ResponseSource::JsonPath { .. },
                ..
            }
        )
    });
    let json = needs_json.then(|| parse_body(body));
    let mut report = PostReport::default();
    for op in enabled {
        let outcome = match &op.kind {
            PostOpKind::Extract { scope, key, source } => {
                let key = key.trim();
                if key.is_empty() {
                    OpOutcome::Failed(OpFailure::EmptyKey)
                } else {
                    match read_source(source, meta, json.as_ref()) {
                        Ok(value) => {
                            report.extracted.push(Extracted {
                                scope: *scope,
                                key: key.to_string(),
                                value,
                            });
                            OpOutcome::Passed
                        }
                        Err(f) => OpOutcome::Failed(f),
                    }
                }
            }
            PostOpKind::Assert {
                subject,
                op: aop,
                expected,
            } => match read_source(subject, meta, json.as_ref()) {
                Ok(actual) => compare(*aop, actual, expected),
                Err(OpFailure::PathNotFound(_) | OpFailure::HeaderNotFound(_))
                    if *aop == AssertOp::Exists =>
                {
                    OpOutcome::Failed(OpFailure::Missing)
                }
                Err(f) => OpOutcome::Failed(f),
            },
        };
        report.results.push((op.clone(), outcome));
    }
    report
}

/// JSON 路径子集：`a.b[0].c`、`a["k.v"]`、`a['k']`，可带前导 `$`。
/// 不支持通配、过滤、递归下降——那是脚本的事。
pub mod json_path {
    use serde_json::Value;

    pub fn get<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
        let mut rest = path.trim();
        rest = rest.strip_prefix('$').unwrap_or(rest);
        let mut cur = root;
        loop {
            if let Some(after) = rest.strip_prefix('.') {
                // `..` 与末尾的 `.` 都不合法
                if after.is_empty() || after.starts_with('.') {
                    return None;
                }
                rest = after;
            }
            if rest.is_empty() {
                return Some(cur);
            }
            if let Some(after) = rest.strip_prefix('[') {
                // 引号内的 key 可能含 `]`，所以先按引号定界，不能先找 `]` 再判断是不是引号
                // （那样会把引号内的 `]` 当成收尾，切断 key）。
                let quote = match after.as_bytes().first() {
                    Some(b'"') => Some('"'),
                    Some(b'\'') => Some('\''),
                    _ => None,
                };
                if let Some(q) = quote {
                    let body = &after[1..];
                    let close = body.find(q)?;
                    // 闭合引号后必须紧跟 `]`；不是就是畸形输入（未闭合、引号不匹配），返回 None。
                    if body.as_bytes().get(close + 1) != Some(&b']') {
                        return None;
                    }
                    cur = cur.get(&body[..close])?;
                    rest = &body[close + 2..];
                } else {
                    let end = after.find(']')?;
                    let inner = after[..end].trim();
                    cur = cur.get(inner.parse::<usize>().ok()?)?;
                    rest = &after[end + 1..];
                }
            } else {
                let end = rest.find(['.', '[']).unwrap_or(rest.len());
                let name = &rest[..end];
                if name.is_empty() {
                    return None;
                }
                cur = cur.get(name)?;
                rest = &rest[end..];
            }
        }
    }

    /// 字符串原样，其它按 JSON 文本（数字 / 布尔 / null / 紧凑的对象与数组）。
    pub fn value_to_string(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Environment, PostOpKind, ResponseSource, Variable};
    use std::time::Duration;

    fn meta(status: u16, headers: &[(&str, &str)]) -> ResponseMeta {
        ResponseMeta {
            status,
            status_text: "OK".into(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            duration: Duration::from_millis(1),
            ttfb: None,
            body_len: 0,
            content_type: None,
            http_version: None,
            certificate: None,
        }
    }

    fn extract(scope: VarScope, key: &str, source: ResponseSource) -> PostOp {
        PostOp {
            enabled: true,
            kind: PostOpKind::Extract {
                scope,
                key: key.into(),
                source,
            },
        }
    }

    fn assert_op(subject: ResponseSource, op: AssertOp, expected: &str) -> PostOp {
        PostOp {
            enabled: true,
            kind: PostOpKind::Assert {
                subject,
                op,
                expected: expected.into(),
            },
        }
    }

    fn set_variable(scope: VarScope, key: &str, value: &str) -> PreOp {
        PreOp {
            enabled: true,
            kind: PreOpKind::SetVariable {
                scope,
                key: key.into(),
                value: value.into(),
            },
        }
    }

    fn json(path: &str) -> ResponseSource {
        ResponseSource::JsonPath { path: path.into() }
    }

    fn header(name: &str) -> ResponseSource {
        ResponseSource::Header { name: name.into() }
    }

    #[test]
    fn json_path_subset() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"data":{"token":"T","items":[{"id":1},{"id":"two"}],"n":null,"ok":true,"k.v":3,"x]y":4,"a'b":5}}"#,
        )
        .unwrap();
        let s = |p: &str| json_path::get(&v, p).map(json_path::value_to_string);
        assert_eq!(s("$.data.token").as_deref(), Some("T"));
        assert_eq!(s("data.token").as_deref(), Some("T"));
        assert_eq!(s("$.data.items[0].id").as_deref(), Some("1"));
        assert_eq!(s("data.items[1].id").as_deref(), Some("two"));
        assert_eq!(s("data.n").as_deref(), Some("null"));
        assert_eq!(s("data.ok").as_deref(), Some("true"));
        assert_eq!(s(r#"data["k.v"]"#).as_deref(), Some("3"));
        assert_eq!(s("data['k.v']").as_deref(), Some("3"));
        assert_eq!(s("data.items[0]").as_deref(), Some(r#"{"id":1}"#));
        assert_eq!(s("$").as_deref().map(|x| x.starts_with('{')), Some(true));
        // 引号内的 key 本身可以含 `]`：不能先找 `]` 再判断引号，否则会把 key 切断
        assert_eq!(s(r#"data["x]y"]"#).as_deref(), Some("4"));
        assert_eq!(s("data['x]y']").as_deref(), Some("4"));
        assert_eq!(s(r#"data["a'b"]"#).as_deref(), Some("5"));
        // 畸形输入：未闭合的引号 / 引号不匹配，都应返回 None 而不是 panic
        assert_eq!(s(r#"data["x]y"#), None);
        assert_eq!(s(r#"data["x]y']"#), None);
        assert_eq!(s("data.missing"), None);
        assert_eq!(s("data.items[9]"), None);
        assert_eq!(s("data.items[x]"), None);
        assert_eq!(s("data..token"), None);
    }

    #[test]
    fn pre_ops_resolve_values_in_order_and_skip_unavailable_scopes() {
        let mut sets = VariableSets::default();
        sets.globals.push(Variable::new("base", "B"));
        let ops = vec![
            set_variable(VarScope::Global, "a", "{{base}}-1"),
            set_variable(VarScope::Global, "b", "{{a}}-2"),
            PreOp {
                enabled: false,
                ..set_variable(VarScope::Global, "never", "x")
            },
            set_variable(VarScope::Environment, "e", "x"),
            set_variable(VarScope::Group, "g", "x"),
            set_variable(VarScope::Global, "  ", "x"),
        ];
        let results = run_pre_ops(&ops, &mut sets, None);
        assert_eq!(results.len(), 5, "禁用的不出现在结果里");
        assert_eq!(results[0].1, OpOutcome::Passed);
        assert_eq!(results[1].1, OpOutcome::Passed);
        assert_eq!(
            results[2].1,
            OpOutcome::Skipped(OpSkip::NoActiveEnvironment)
        );
        assert_eq!(results[3].1, OpOutcome::Skipped(OpSkip::NoGroup));
        assert_eq!(results[4].1, OpOutcome::Failed(OpFailure::EmptyKey));
        let get = |k: &str| {
            sets.globals
                .iter()
                .find(|v| v.key == k)
                .map(|v| v.value.clone())
        };
        assert_eq!(get("a").as_deref(), Some("B-1"));
        assert_eq!(get("b").as_deref(), Some("B-1-2"));
        assert_eq!(get("never"), None);

        // 有激活环境 + 有分类时两种作用域都能写
        let env = Environment::new("dev");
        sets.active_environment = Some(env.id);
        sets.environments.push(env);
        let results = run_pre_ops(&ops[3..5], &mut sets, Some("grp"));
        assert!(results.iter().all(|(_, o)| *o == OpOutcome::Passed));
        assert_eq!(sets.active_env().unwrap().variables[0].key, "e");
        assert_eq!(sets.group_vars(Some("grp"))[0].key, "g");
    }

    #[test]
    fn post_ops_extract_and_assert_against_json_body() {
        let body = br#"{"data":{"token":"T","n":5}}"#;
        let m = meta(
            200,
            &[("Content-Type", "application/json"), ("X-Req", "abc")],
        );
        let ops = vec![
            extract(VarScope::Global, "token", json("$.data.token")),
            extract(VarScope::Environment, "req", header("x-req")),
            extract(VarScope::Group, "code", ResponseSource::Status),
            extract(VarScope::Global, "", json("$.data.token")),
            extract(VarScope::Global, "nope", json("$.data.nope")),
            extract(VarScope::Global, "h", header("X-Missing")),
            assert_op(ResponseSource::Status, AssertOp::Equals, "200"),
            assert_op(ResponseSource::Status, AssertOp::Equals, "201"),
            assert_op(ResponseSource::Status, AssertOp::NotEquals, "200"),
            assert_op(json("data.n"), AssertOp::Contains, "5"),
            assert_op(json("data.token"), AssertOp::Contains, "zzz"),
            assert_op(json("data.token"), AssertOp::Exists, ""),
            assert_op(json("data.missing"), AssertOp::Exists, ""),
            assert_op(header("x-missing"), AssertOp::Exists, ""),
            PostOp {
                enabled: false,
                ..assert_op(ResponseSource::Status, AssertOp::Equals, "500")
            },
        ];
        let report = run_post_ops(&ops, &m, Some(body));
        let outcomes: Vec<&OpOutcome> = report.results.iter().map(|(_, o)| o).collect();
        assert_eq!(outcomes.len(), 14, "禁用的不参与");
        assert_eq!(*outcomes[0], OpOutcome::Passed);
        assert_eq!(*outcomes[1], OpOutcome::Passed);
        assert_eq!(*outcomes[2], OpOutcome::Passed);
        assert_eq!(*outcomes[3], OpOutcome::Failed(OpFailure::EmptyKey));
        assert_eq!(
            *outcomes[4],
            OpOutcome::Failed(OpFailure::PathNotFound("$.data.nope".into()))
        );
        assert_eq!(
            *outcomes[5],
            OpOutcome::Failed(OpFailure::HeaderNotFound("X-Missing".into()))
        );
        assert_eq!(*outcomes[6], OpOutcome::Passed);
        assert_eq!(
            *outcomes[7],
            OpOutcome::Failed(OpFailure::Mismatch {
                actual: "200".into(),
                expected: "201".into()
            })
        );
        assert_eq!(
            *outcomes[8],
            OpOutcome::Failed(OpFailure::Unexpected {
                actual: "200".into()
            })
        );
        assert_eq!(*outcomes[9], OpOutcome::Passed);
        assert!(matches!(
            outcomes[10],
            OpOutcome::Failed(OpFailure::Mismatch { .. })
        ));
        assert_eq!(*outcomes[11], OpOutcome::Passed);
        assert_eq!(*outcomes[12], OpOutcome::Failed(OpFailure::Missing));
        assert_eq!(*outcomes[13], OpOutcome::Failed(OpFailure::Missing));
        assert_eq!(
            report.extracted,
            vec![
                Extracted {
                    scope: VarScope::Global,
                    key: "token".into(),
                    value: "T".into()
                },
                Extracted {
                    scope: VarScope::Environment,
                    key: "req".into(),
                    value: "abc".into()
                },
                Extracted {
                    scope: VarScope::Group,
                    key: "code".into(),
                    value: "200".into()
                },
            ]
        );
    }

    #[test]
    fn post_ops_report_body_problems_only_for_json_sources() {
        let m = meta(204, &[]);
        let ops = vec![
            extract(VarScope::Global, "a", json("$.a")),
            assert_op(ResponseSource::Status, AssertOp::Equals, "204"),
        ];
        // 已落盘：没有内存里的 body
        let r = run_post_ops(&ops, &m, None);
        assert_eq!(
            r.results[0].1,
            OpOutcome::Failed(OpFailure::BodyUnavailable)
        );
        assert_eq!(r.results[1].1, OpOutcome::Passed);
        // 不是 JSON
        let r = run_post_ops(&ops, &m, Some(b"<html>"));
        assert_eq!(r.results[0].1, OpOutcome::Failed(OpFailure::NotJson));
        // 超限：不解析
        let big = vec![b' '; OPS_JSON_MAX_BYTES + 1];
        let r = run_post_ops(&ops, &m, Some(&big));
        assert_eq!(r.results[0].1, OpOutcome::Failed(OpFailure::BodyTooLarge));
        // 空路径
        let r = run_post_ops(
            &[extract(VarScope::Global, "a", json("  "))],
            &m,
            Some(b"{}"),
        );
        assert_eq!(r.results[0].1, OpOutcome::Failed(OpFailure::EmptyPath));
        // 没有 JSON 操作时不碰 body（大 body 也不报错）
        let r = run_post_ops(&ops[1..], &m, Some(&big));
        assert_eq!(r.results[0].1, OpOutcome::Passed);
    }
}
