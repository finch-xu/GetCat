//! 可视化前后置操作的执行器（纯函数，不碰持久化）。
//!
//! - 前置：发送前依次执行，把值写进 [`VariableSets`]；值先做 `{{}}` 替换，所以能引用
//!   上一条刚设的变量和动态变量。
//! - 后置：响应完成后在**后台线程**执行（解析 JSON 是 O(n)）；结果与提取出的变量由
//!   app 层在 generation 校验通过后用 [`apply_extracted`] 写回。断言的期望值在发送时已替换完。

use std::collections::BTreeSet;

use serde_json::Value;

use crate::model::{
    AssertOp, PostOp, PostOpKind, PreOp, PreOpKind, ResponseMeta, ResponseSource, VarScope,
    VariableSets,
};
use crate::vars::{self, Resolver};

/// JsonPath 操作愿意解析的响应体上限：再大就解析出一棵占内存的 `Value` 树，不值得。
pub const OPS_JSON_MAX_BYTES: usize = 8 * 1024 * 1024;

/// 条件不满足、没有执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpSkip {
    NoActiveEnvironment,
    NoGroup,
    /// 请求没拿到响应（网络错误、后台处理异常），后置操作没有执行。见 [`skip_all`]。
    RequestFailed,
}

/// 执行了但没成功。载荷是原文（路径、头名、实际值），界面按变体翻译种类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpFailure {
    EmptyKey,
    /// 变量名非法（含花括号 / 换行、超长），或是内置动态变量名（`$` 开头，写了也读不到）。载荷是 trim 后的 key。
    InvalidKey(String),
    EmptyPath,
    /// 响应头名 trim 后为空。
    EmptyHeader,
    /// 响应体已落盘，内存里没有完整内容。
    BodyUnavailable,
    /// 超过 [`OPS_JSON_MAX_BYTES`]。
    BodyTooLarge,
    NotJson,
    /// 载荷是 trim 后的路径。
    PathNotFound(String),
    /// 载荷是 trim 后的头名。
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

/// 一条后置提取的结果，等 app 层用 [`apply_extracted`] 写回变量表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// 在 [`PostReport::results`] 里对应的行号（同名 key 的多条提取靠它区分）。
    pub index: usize,
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

/// 作用域不可用时的跳过原因。全局作用域总是可写，走不到这里。
fn skip_reason(scope: VarScope) -> OpSkip {
    match scope {
        VarScope::Environment => OpSkip::NoActiveEnvironment,
        VarScope::Group => OpSkip::NoGroup,
        VarScope::Global => unreachable!("全局作用域总是可写"),
    }
}

/// 操作要写入的变量名：trim 后非空、是合法变量名、且不是内置动态变量名。
fn check_key(key: &str) -> Result<&str, OpFailure> {
    let key = key.trim();
    if key.is_empty() {
        Err(OpFailure::EmptyKey)
    } else if !vars::valid_name(key) || vars::is_dynamic(key) {
        Err(OpFailure::InvalidKey(key.to_string()))
    } else {
        Ok(key)
    }
}

/// 前置操作的执行结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreReport {
    /// 每条启用的操作一行（停用的不出现）。
    pub results: Vec<(PreOp, OpOutcome)>,
    /// 各条操作的值里没能解析的变量名（按名字排序去重）；app 层把它并进 URL 栏的「未定义变量」提示。
    pub unresolved: BTreeSet<String>,
}

/// 前置操作：逐条替换值并写入；每条都按**当时**的变量表替换，所以后一条能引用前一条。
pub fn run_pre_ops(ops: &[PreOp], sets: &mut VariableSets, group: Option<&str>) -> PreReport {
    let mut report = PreReport::default();
    for op in ops.iter().filter(|o| o.enabled) {
        let outcome = match &op.kind {
            PreOpKind::SetVariable { scope, key, value } => match check_key(key) {
                Err(f) => OpOutcome::Failed(f),
                Ok(key) => {
                    let value = {
                        let ctx = sets.context(group);
                        let mut resolver = Resolver::new(&ctx);
                        let value = resolver.resolve(value).into_owned();
                        report.unresolved.extend(resolver.finish());
                        value
                    };
                    if sets.set_var(*scope, group, key, &value) {
                        OpOutcome::Passed
                    } else {
                        OpOutcome::Skipped(skip_reason(*scope))
                    }
                }
            },
        };
        report.results.push((op.clone(), outcome));
    }
    report
}

/// 请求失败（没拿到响应）时的后置报告：每条启用的后置操作记为 `Skipped(RequestFailed)`，
/// 没有提取。没有启用的后置操作时 `results` 为空。
pub fn skip_all(ops: &[PostOp]) -> PostReport {
    PostReport {
        results: ops
            .iter()
            .filter(|o| o.enabled)
            .map(|op| (op.clone(), OpOutcome::Skipped(OpSkip::RequestFailed)))
            .collect(),
        extracted: Vec::new(),
    }
}

/// 把后置提取写回变量表（app 层在 generation 校验通过后调用）。
///
/// [`run_post_ops`] 在后台线程执行、看不到变量表，提取一律先记为 Passed。这里逐条
/// `set_var`：作用域不可用（没激活环境 / 请求未分类）的那条，把 `results` 里对应的行
/// 改写为 `Skipped`，并从 `extracted` 移除——调用后 `extracted` 只剩真正写入的。
pub fn apply_extracted(report: &mut PostReport, sets: &mut VariableSets, group: Option<&str>) {
    let results = &mut report.results;
    report.extracted.retain(|e| {
        if sets.set_var(e.scope, group, &e.key, &e.value) {
            return true;
        }
        if let Some((_, outcome)) = results.get_mut(e.index) {
            *outcome = OpOutcome::Skipped(skip_reason(e.scope));
        }
        false
    });
}

fn parse_body(body: Option<&[u8]>) -> Result<Value, OpFailure> {
    let bytes = body.ok_or(OpFailure::BodyUnavailable)?;
    if bytes.len() > OPS_JSON_MAX_BYTES {
        return Err(OpFailure::BodyTooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| OpFailure::NotJson)
}

/// `json` 是懒解析缓存：第一次读 JsonPath 来源时才解析 `body`，结果（含失败原因）留给后面的操作复用。
fn read_source(
    source: &ResponseSource,
    meta: &ResponseMeta,
    body: Option<&[u8]>,
    json: &mut Option<Result<Value, OpFailure>>,
) -> Result<String, OpFailure> {
    match source {
        ResponseSource::Status => Ok(meta.status.to_string()),
        ResponseSource::Header { name } => {
            let name = name.trim();
            if name.is_empty() {
                return Err(OpFailure::EmptyHeader);
            }
            meta.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
                .ok_or_else(|| OpFailure::HeaderNotFound(name.to_string()))
        }
        ResponseSource::JsonPath { path } => {
            let path = path.trim();
            if path.is_empty() {
                return Err(OpFailure::EmptyPath);
            }
            match json.get_or_insert_with(|| parse_body(body)) {
                Err(f) => Err(f.clone()),
                Ok(root) => json_path::get(root, path)
                    .map(json_path::value_to_string)
                    .ok_or_else(|| OpFailure::PathNotFound(path.to_string())),
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

/// 后置操作。`body` 为 None 表示响应体已落盘。JSON 在第一次遇到启用的 JsonPath 操作时
/// 才解析（只解析一次）；没有 JsonPath 操作时不碰 body。
///
/// 提取一律记为 Passed——这里看不到变量表，作用域是否可用由 [`apply_extracted`] 写回时判定。
pub fn run_post_ops(ops: &[PostOp], meta: &ResponseMeta, body: Option<&[u8]>) -> PostReport {
    let mut json: Option<Result<Value, OpFailure>> = None;
    let mut report = PostReport::default();
    for op in ops.iter().filter(|o| o.enabled) {
        let outcome = match &op.kind {
            PostOpKind::Extract { scope, key, source } => match check_key(key) {
                Err(f) => OpOutcome::Failed(f),
                Ok(key) => match read_source(source, meta, body, &mut json) {
                    Ok(value) => {
                        report.extracted.push(Extracted {
                            index: report.results.len(),
                            scope: *scope,
                            key: key.to_string(),
                            value,
                        });
                        OpOutcome::Passed
                    }
                    Err(f) => OpOutcome::Failed(f),
                },
            },
            PostOpKind::Assert {
                subject,
                op: aop,
                expected,
            } => match read_source(subject, meta, body, &mut json) {
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

/// JSON 路径子集：`a.b[0].c`、`a["k.v"]`、`a['k']`，可带前导 `$`（`$`、`$.a`、`$[0]`）。
/// 不支持通配、过滤、递归下降——那是脚本的事。
pub mod json_path {
    use serde_json::Value;

    pub fn get<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
        let mut rest = path.trim();
        // `$` 只在后面是 `.`、`[` 或结尾时才是根；`$ref` 查的是键 `$ref`
        if let Some(after) = rest.strip_prefix('$')
            && (after.is_empty() || after.starts_with(['.', '[']))
        {
            rest = after;
        }
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

    /// `$` 只在后面是 `.`、`[` 或结尾时才是根；`$ref` 这类键名照常按键查。
    #[test]
    fn json_path_dollar_is_root_only_before_dot_bracket_or_end() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"$ref":"R","ref":"wrong","a":{"$id":7}}"#).unwrap();
        let s = |p: &str| json_path::get(&v, p).map(json_path::value_to_string);
        assert_eq!(s("$ref").as_deref(), Some("R"));
        assert_eq!(s("$.$ref").as_deref(), Some("R"));
        assert_eq!(s("$['$ref']").as_deref(), Some("R"));
        assert_eq!(s("a.$id").as_deref(), Some("7"));
        assert_eq!(s("$.a.$id").as_deref(), Some("7"));
        assert_eq!(s("$").as_deref().map(|x| x.starts_with('{')), Some(true));
        assert_eq!(s(" $ ").as_deref().map(|x| x.starts_with('{')), Some(true));
        let arr: serde_json::Value = serde_json::from_str("[10, 20]").unwrap();
        assert_eq!(
            json_path::get(&arr, "$[1]").map(json_path::value_to_string),
            Some("20".into())
        );
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
        let report = run_pre_ops(&ops, &mut sets, None);
        assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
        let results = report.results;
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
        let results = run_pre_ops(&ops[3..5], &mut sets, Some("grp")).results;
        assert!(results.iter().all(|(_, o)| *o == OpOutcome::Passed));
        assert_eq!(sets.active_env().unwrap().variables[0].key, "e");
        assert_eq!(sets.group_vars(Some("grp"))[0].key, "g");
    }

    /// 前置值里的未定义变量要报出来；停用的、key 非法的不参与替换。
    #[test]
    fn pre_ops_report_unresolved_names_in_values() {
        let mut sets = VariableSets::default();
        let ops = vec![
            set_variable(VarScope::Global, "a", "{{missing}}-{{$timestamp}}"),
            // 引用上一条刚设的值：已定义，不算未解析
            set_variable(VarScope::Global, "b", "{{a}}"),
            PreOp {
                enabled: false,
                ..set_variable(VarScope::Global, "off", "{{disabled_ref}}")
            },
            set_variable(VarScope::Global, "$bad", "{{bad_key_ref}}"),
            set_variable(VarScope::Global, "c", "{{ other }}"),
        ];
        let report = run_pre_ops(&ops, &mut sets, None);
        assert_eq!(
            report.unresolved.into_iter().collect::<Vec<_>>(),
            vec!["missing".to_string(), "other".to_string()]
        );
        assert_eq!(report.results.len(), 4);
        // 未解析的占位符原样写入
        assert!(sets.globals[0].value.starts_with("{{missing}}-"));
    }

    /// 请求失败：每条启用的后置操作记为「请求失败，未执行」，没有提取。
    #[test]
    fn skip_all_marks_enabled_post_ops_request_failed() {
        let ops = vec![
            extract(VarScope::Global, "token", json("$.data.token")),
            PostOp {
                enabled: false,
                ..assert_op(ResponseSource::Status, AssertOp::Equals, "200")
            },
            assert_op(header("x-a"), AssertOp::Exists, ""),
        ];
        let report = skip_all(&ops);
        assert_eq!(
            report.results,
            vec![
                (ops[0].clone(), OpOutcome::Skipped(OpSkip::RequestFailed)),
                (ops[2].clone(), OpOutcome::Skipped(OpSkip::RequestFailed)),
            ]
        );
        assert!(report.extracted.is_empty());
        assert_eq!(report.passed(), 0);
        // 没有启用的后置操作：results 为空
        assert_eq!(skip_all(&ops[1..2]), PostReport::default());
        assert_eq!(skip_all(&[]), PostReport::default());
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
                    index: 0,
                    scope: VarScope::Global,
                    key: "token".into(),
                    value: "T".into()
                },
                Extracted {
                    index: 1,
                    scope: VarScope::Environment,
                    key: "req".into(),
                    value: "abc".into()
                },
                Extracted {
                    index: 2,
                    scope: VarScope::Group,
                    key: "code".into(),
                    value: "200".into()
                },
            ]
        );
    }

    #[test]
    fn apply_extracted_marks_unavailable_scopes_skipped_and_keeps_rows_aligned() {
        let m = meta(200, &[("X-A", "1")]);
        let ops = vec![
            extract(VarScope::Environment, "token", ResponseSource::Status),
            assert_op(ResponseSource::Status, AssertOp::Equals, "200"),
            extract(VarScope::Global, "k", ResponseSource::Status),
            extract(VarScope::Global, "k", header("x-a")),
            extract(VarScope::Group, "g", ResponseSource::Status),
        ];
        let mut report = run_post_ops(&ops, &m, None);
        // 同名 key 的两条提取各自指向自己的结果行
        let indices: Vec<usize> = report.extracted.iter().map(|e| e.index).collect();
        assert_eq!(indices, [0, 2, 3, 4]);
        for e in &report.extracted {
            match &report.results[e.index].0.kind {
                PostOpKind::Extract { key, .. } => assert_eq!(*key, e.key),
                other => panic!("index 指向了非提取行：{other:?}"),
            }
        }

        // 没激活环境、请求未分类：这两条改写为跳过，并从 extracted 移除；全局照常写入
        let mut sets = VariableSets::default();
        apply_extracted(&mut report, &mut sets, None);
        assert_eq!(
            report.results[0].1,
            OpOutcome::Skipped(OpSkip::NoActiveEnvironment)
        );
        assert_eq!(report.results[1].1, OpOutcome::Passed);
        assert_eq!(report.results[2].1, OpOutcome::Passed);
        assert_eq!(report.results[3].1, OpOutcome::Passed);
        assert_eq!(report.results[4].1, OpOutcome::Skipped(OpSkip::NoGroup));
        assert_eq!(report.passed(), 3);
        assert_eq!(
            report.extracted,
            vec![
                Extracted {
                    index: 2,
                    scope: VarScope::Global,
                    key: "k".into(),
                    value: "200".into()
                },
                Extracted {
                    index: 3,
                    scope: VarScope::Global,
                    key: "k".into(),
                    value: "1".into()
                },
            ]
        );
        // 按顺序写入，后一条覆盖前一条
        assert_eq!(sets.globals, vec![Variable::new("k", "1")]);
        assert!(sets.environments.is_empty() && sets.groups.is_empty());

        // 作用域都可用时全部写入，结果不变
        let env = Environment::new("dev");
        sets.active_environment = Some(env.id);
        sets.environments.push(env);
        let mut report = run_post_ops(&ops, &m, None);
        apply_extracted(&mut report, &mut sets, Some("grp"));
        assert_eq!(report.passed(), 5);
        assert_eq!(report.extracted.len(), 4);
        assert_eq!(sets.active_env().unwrap().variables[0].key, "token");
        assert_eq!(sets.group_vars(Some("grp"))[0].value, "200");
    }

    #[test]
    fn post_ops_trim_names_in_failures_and_reject_empty_header() {
        let m = meta(200, &[("X-Req", "abc")]);
        let ops = vec![
            extract(VarScope::Global, "a", header("  X-Missing ")),
            extract(VarScope::Global, "b", json(" $.nope ")),
            extract(VarScope::Global, "c", header("  ")),
            assert_op(header(" "), AssertOp::Exists, ""),
            extract(VarScope::Global, "d", header(" x-req ")),
        ];
        let r = run_post_ops(&ops, &m, Some(b"{}"));
        let outcomes: Vec<&OpOutcome> = r.results.iter().map(|(_, o)| o).collect();
        assert_eq!(
            outcomes,
            [
                &OpOutcome::Failed(OpFailure::HeaderNotFound("X-Missing".into())),
                &OpOutcome::Failed(OpFailure::PathNotFound("$.nope".into())),
                &OpOutcome::Failed(OpFailure::EmptyHeader),
                &OpOutcome::Failed(OpFailure::EmptyHeader),
                &OpOutcome::Passed,
            ]
        );
        assert_eq!(r.extracted.len(), 1);
        assert_eq!(r.extracted[0].value, "abc");
    }

    #[test]
    fn op_keys_must_be_valid_non_dynamic_names() {
        let long = "x".repeat(crate::vars::MAX_NAME_LEN + 1);
        let mut sets = VariableSets::default();
        let results = run_pre_ops(
            &[
                set_variable(VarScope::Global, "$timestamp", "x"),
                set_variable(VarScope::Global, " a{b ", "x"),
                set_variable(VarScope::Global, &long, "x"),
                set_variable(VarScope::Global, "ok", "x"),
            ],
            &mut sets,
            None,
        )
        .results;
        let outcomes: Vec<&OpOutcome> = results.iter().map(|(_, o)| o).collect();
        assert_eq!(
            outcomes,
            [
                &OpOutcome::Failed(OpFailure::InvalidKey("$timestamp".into())),
                &OpOutcome::Failed(OpFailure::InvalidKey("a{b".into())),
                &OpOutcome::Failed(OpFailure::InvalidKey(long.clone())),
                &OpOutcome::Passed,
            ]
        );
        assert_eq!(sets.globals, vec![Variable::new("ok", "x")]);

        let m = meta(200, &[]);
        let r = run_post_ops(
            &[
                extract(VarScope::Global, "$timestamp", ResponseSource::Status),
                extract(VarScope::Global, "a{b", ResponseSource::Status),
                extract(VarScope::Global, "  ", ResponseSource::Status),
            ],
            &m,
            None,
        );
        let outcomes: Vec<&OpOutcome> = r.results.iter().map(|(_, o)| o).collect();
        assert_eq!(
            outcomes,
            [
                &OpOutcome::Failed(OpFailure::InvalidKey("$timestamp".into())),
                &OpOutcome::Failed(OpFailure::InvalidKey("a{b".into())),
                &OpOutcome::Failed(OpFailure::EmptyKey),
            ]
        );
        assert!(r.extracted.is_empty());
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
        // 不是 JSON：解析结果（失败）缓存下来，之后的 JsonPath 操作报同一个原因
        let r = run_post_ops(
            &[
                ops[0].clone(),
                ops[1].clone(),
                assert_op(json("$.b"), AssertOp::Exists, ""),
            ],
            &m,
            Some(b"<html>"),
        );
        assert_eq!(r.results[0].1, OpOutcome::Failed(OpFailure::NotJson));
        assert_eq!(r.results[1].1, OpOutcome::Passed);
        assert_eq!(r.results[2].1, OpOutcome::Failed(OpFailure::NotJson));
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
