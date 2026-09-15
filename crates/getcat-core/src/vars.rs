//! `{{name}}` 变量替换。
//!
//! 替换在 [`crate::http::prepare`] **之前**对 `RequestDraft` 的克隆做：先展开 `{{var}}`，
//! 再由 `build_url` 单遍替换 Path 参数 `{id}`——两种语法互不干扰
//! （`extract_path_params` 会跳过名字里含 `{` 的片段）。已保存请求里始终存原文。
//!
//! 优先级（高 → 低）：内置动态变量（`$` 开头）> 环境 > 分类 > 全局；同层重名取第一条启用的。
//! 值里可再引用变量，最多 [`MAX_DEPTH`] 层；循环或未定义的都原样保留并记入 `unresolved`。

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use memchr::memmem;

use crate::model::{BodyKind, FormValue, KeyValue, PostOpKind, RequestDraft, Variable};

/// 值里再引用变量时的最大展开层数。
pub const MAX_DEPTH: usize = 8;
/// 变量名长度上限。
pub const MAX_NAME_LEN: usize = 128;

/// 三层变量，低 → 高：`[全局, 分类, 环境]`。
pub struct VarContext<'a> {
    layers: [&'a [Variable]; 3],
}

impl<'a> VarContext<'a> {
    pub const EMPTY: VarContext<'static> = VarContext {
        layers: [&[], &[], &[]],
    };

    pub fn new(globals: &'a [Variable], group: &'a [Variable], env: &'a [Variable]) -> Self {
        Self {
            layers: [globals, group, env],
        }
    }

    /// 高层优先；同层取第一条启用的。
    fn lookup(&self, name: &str) -> Option<&'a str> {
        self.layers.iter().rev().find_map(|layer| {
            layer
                .iter()
                .find(|v| v.enabled && v.key == name)
                .map(|v| v.value.as_str())
        })
    }
}

/// 变量名是否合法：非空、不超长、不含花括号与换行。
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME_LEN && !name.contains(['{', '}', '\n', '\r'])
}

/// 内置动态变量的名字都以 `$` 开头，用户变量盖不住它们。
pub fn is_dynamic(name: &str) -> bool {
    name.starts_with('$')
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Unix 秒 → `YYYY-MM-DDTHH:MM:SSZ`。日历换算用 Howard Hinnant 的 civil_from_days，
/// 二十行就够，不值得为此引入 chrono。
pub fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn dynamic_value(name: &str) -> Option<String> {
    Some(match name {
        "$timestamp" => unix_secs().to_string(),
        "$isoTimestamp" => iso_utc(unix_secs()),
        "$randomUUID" | "$guid" => uuid::Uuid::new_v4().to_string(),
        // Postman 语义：0..=1000
        "$randomInt" => (uuid::Uuid::new_v4().as_u128() % 1001).to_string(),
        _ => return None,
    })
}

/// 一次替换会话：动态变量在会话内只取值一次，未解析的名字累计到 `unresolved`。
pub struct Resolver<'a> {
    ctx: &'a VarContext<'a>,
    dynamic: HashMap<String, String>,
    unresolved: BTreeSet<String>,
}

impl<'a> Resolver<'a> {
    pub fn new(ctx: &'a VarContext<'a>) -> Self {
        Self {
            ctx,
            dynamic: HashMap::new(),
            unresolved: BTreeSet::new(),
        }
    }

    /// 没有 `{{` 时零分配原样返回。
    pub fn resolve<'s>(&mut self, input: &'s str) -> Cow<'s, str> {
        if memmem::find(input.as_bytes(), b"{{").is_none() {
            return Cow::Borrowed(input);
        }
        let mut stack = Vec::new();
        Cow::Owned(self.expand(input, &mut stack))
    }

    /// 原地替换；没变化时不重新分配。
    pub fn resolve_in(&mut self, s: &mut String) {
        let replaced = match self.resolve(s.as_str()) {
            Cow::Owned(o) => Some(o),
            Cow::Borrowed(_) => None,
        };
        if let Some(o) = replaced {
            *s = o;
        }
    }

    pub fn finish(self) -> BTreeSet<String> {
        self.unresolved
    }

    /// `stack` 是正在展开的名字链，用来判断循环与深度。
    fn expand(&mut self, input: &str, stack: &mut Vec<String>) -> String {
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = memmem::find(rest.as_bytes(), b"{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(len) = memmem::find(after.as_bytes(), b"}}") else {
                // 没有闭合：剩下的全是原文
                out.push_str(&rest[start..]);
                return out;
            };
            let name = after[..len].trim();
            if !valid_name(name) {
                // 不是变量（比如 `{{a{{b}}` 的外层）：吐出这两个字符，从后面继续找
                out.push_str("{{");
                rest = after;
                continue;
            }
            let token = &rest[start..start + 2 + len + 2];
            match self.value_of(name) {
                Some(value) if stack.len() < MAX_DEPTH && !stack.iter().any(|n| n == name) => {
                    stack.push(name.to_string());
                    let expanded = self.expand(&value, stack);
                    stack.pop();
                    out.push_str(&expanded);
                }
                _ => {
                    self.unresolved.insert(name.to_string());
                    out.push_str(token);
                }
            }
            rest = &after[len + 2..];
        }
        out.push_str(rest);
        out
    }

    fn value_of(&mut self, name: &str) -> Option<String> {
        if is_dynamic(name) {
            if let Some(v) = self.dynamic.get(name) {
                return Some(v.clone());
            }
            let v = dynamic_value(name)?;
            self.dynamic.insert(name.to_string(), v.clone());
            return Some(v);
        }
        self.ctx.lookup(name).map(str::to_string)
    }
}

/// `resolve_draft` 的结果：替换后的草稿 + 未解析的名字（按名字排序去重）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub draft: RequestDraft,
    pub unresolved: BTreeSet<String>,
}

fn resolve_kvs(r: &mut Resolver, kvs: &mut [KeyValue], keys: bool) {
    for kv in kvs.iter_mut().filter(|kv| kv.enabled) {
        if keys {
            r.resolve_in(&mut kv.key);
        }
        r.resolve_in(&mut kv.value);
    }
}

fn resolve_path(r: &mut Resolver, path: &mut PathBuf) {
    if let Some(s) = path.to_str()
        && s.contains("{{")
    {
        let replaced = r.resolve(s).into_owned();
        *path = PathBuf::from(replaced);
    }
}

/// 克隆草稿并替换所有可替换字段（只处理启用的行）。
/// Path 参数只替换值——key 由 URL 里的 `{id}` 驱动。前置操作的值**不在这里**替换，
/// 它们在执行时（[`crate::ops::run_pre_ops`]）按当时的变量表替换。
pub fn resolve_draft(draft: &RequestDraft, ctx: &VarContext) -> Resolved {
    let mut r = Resolver::new(ctx);
    let mut out = draft.clone();
    r.resolve_in(&mut out.url);
    resolve_kvs(&mut r, &mut out.path_params, false);
    resolve_kvs(&mut r, &mut out.params, true);
    resolve_kvs(&mut r, &mut out.headers, true);
    match &mut out.body {
        BodyKind::None => {}
        BodyKind::Raw { text, .. } => r.resolve_in(text),
        BodyKind::FormData { fields } => {
            for f in fields.iter_mut().filter(|f| f.enabled) {
                r.resolve_in(&mut f.key);
                match &mut f.value {
                    FormValue::Text { value } => r.resolve_in(value),
                    FormValue::File { path, .. } => resolve_path(&mut r, path),
                }
            }
        }
        BodyKind::FormUrlEncoded { fields } => resolve_kvs(&mut r, fields, true),
        BodyKind::Binary { path, .. } => resolve_path(&mut r, path),
    }
    for op in out.post_ops.iter_mut().filter(|o| o.enabled) {
        if let PostOpKind::Assert { expected, .. } = &mut op.kind {
            r.resolve_in(expected);
        }
    }
    Resolved {
        draft: out,
        unresolved: r.finish(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AssertOp, BodyKind, FormField, FormValue, KeyValue, PostOp, PostOpKind, RawFormat,
        RequestDraft, ResponseSource, VarScope,
    };
    use std::path::PathBuf;

    fn vars(pairs: &[(&str, &str)]) -> Vec<Variable> {
        pairs.iter().map(|(k, v)| Variable::new(*k, *v)).collect()
    }

    #[test]
    fn plain_text_is_borrowed_and_untouched() {
        let ctx = VarContext::EMPTY;
        let mut r = Resolver::new(&ctx);
        assert!(matches!(r.resolve("no vars {single}"), Cow::Borrowed(_)));
        assert!(r.finish().is_empty());
    }

    #[test]
    fn layers_override_low_to_high_and_first_enabled_wins() {
        let globals = vars(&[("a", "g"), ("b", "g"), ("c", "g")]);
        let group = vars(&[("b", "grp"), ("c", "grp")]);
        let env = vec![
            Variable {
                enabled: false,
                ..Variable::new("c", "disabled")
            },
            Variable::new("c", "env1"),
            Variable::new("c", "env2"),
        ];
        let ctx = VarContext::new(&globals, &group, &env);
        let mut r = Resolver::new(&ctx);
        assert_eq!(r.resolve("{{a}}/{{b}}/{{c}}"), "g/grp/env1");
        assert!(r.finish().is_empty());
    }

    #[test]
    fn names_are_trimmed_and_unknown_kept_verbatim() {
        let globals = vars(&[("tok", "T")]);
        let ctx = VarContext::new(&globals, &[], &[]);
        let mut r = Resolver::new(&ctx);
        assert_eq!(r.resolve("x={{ tok }}&y={{nope}}"), "x=T&y={{nope}}");
        let unresolved = r.finish();
        assert_eq!(
            unresolved.into_iter().collect::<Vec<_>>(),
            vec!["nope".to_string()]
        );
    }

    #[test]
    fn invalid_names_and_unclosed_braces_are_left_alone() {
        let globals = vars(&[("b", "B")]);
        let ctx = VarContext::new(&globals, &[], &[]);
        let mut r = Resolver::new(&ctx);
        // 外层名字含 `{`：不是变量；内层 {{b}} 照常替换
        assert_eq!(r.resolve("{{a{{b}}"), "{{aB");
        assert_eq!(r.resolve("{{unclosed"), "{{unclosed");
        assert_eq!(r.resolve("{{}}"), "{{}}");
        assert!(r.finish().is_empty());
    }

    #[test]
    fn nested_values_expand_up_to_max_depth_and_cycles_stop() {
        let globals = vars(&[
            ("base", "https://{{host}}/v1"),
            ("host", "{{env}}.example.com"),
            ("env", "dev"),
            ("loop_a", "{{loop_b}}"),
            ("loop_b", "{{loop_a}}"),
        ]);
        let ctx = VarContext::new(&globals, &[], &[]);
        let mut r = Resolver::new(&ctx);
        assert_eq!(
            r.resolve("{{base}}/users"),
            "https://dev.example.com/v1/users"
        );
        assert_eq!(r.resolve("{{loop_a}}"), "{{loop_a}}");
        assert!(r.finish().contains("loop_a"));

        // 深度链：d0 → d1 → … → d9，超过 MAX_DEPTH 的那一段原样保留
        let chain: Vec<Variable> = (0..10)
            .map(|i| Variable::new(format!("d{i}"), format!("{{{{d{}}}}}", i + 1)))
            .chain(std::iter::once(Variable::new("d10", "end")))
            .collect();
        let ctx = VarContext::new(&chain, &[], &[]);
        let mut r = Resolver::new(&ctx);
        let out = r.resolve("{{d0}}").into_owned();
        assert!(out.starts_with("{{d"), "{out}");
        assert!(!r.finish().is_empty());
    }

    #[test]
    fn dynamic_variables_are_generated_once_per_resolver() {
        let ctx = VarContext::EMPTY;
        let mut r = Resolver::new(&ctx);
        let a = r.resolve("{{$randomUUID}}").into_owned();
        let b = r.resolve("{{$guid}}").into_owned();
        assert_eq!(a.len(), 36);
        assert_eq!(a, a.to_lowercase());
        assert_ne!(a, b, "$guid 与 $randomUUID 是两个名字，各自取值");
        let t1 = r.resolve("{{$timestamp}}").into_owned();
        let t2 = r.resolve("{{$timestamp}}").into_owned();
        assert_eq!(t1, t2);
        assert!(t1.parse::<u64>().unwrap() > 1_767_225_600);
        let n: u32 = r.resolve("{{$randomInt}}").parse().unwrap();
        assert!(n <= 1000);
        let iso = r.resolve("{{$isoTimestamp}}").into_owned();
        assert!(iso.ends_with('Z') && iso.len() == 20, "{iso}");
        // 用户变量盖不住动态变量；未知的 $ 名字算未解析
        let globals = vars(&[("$timestamp", "user")]);
        let ctx = VarContext::new(&globals, &[], &[]);
        let mut r = Resolver::new(&ctx);
        assert_ne!(r.resolve("{{$timestamp}}"), "user");
        assert_eq!(r.resolve("{{$nope}}"), "{{$nope}}");
        assert!(r.finish().contains("$nope"));
        assert!(is_dynamic("$timestamp") && !is_dynamic("timestamp"));
    }

    #[test]
    fn iso_utc_handles_epoch_leap_day_and_2026() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_utc(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_767_225_600 + 3_723), "2026-01-01T01:02:03Z");
    }

    #[test]
    fn resolve_draft_touches_every_field_and_reports_unresolved() {
        let globals = vars(&[
            ("h", "example.com"),
            ("k", "K"),
            ("v", "V"),
            ("p", "/tmp/f"),
        ]);
        let ctx = VarContext::new(&globals, &[], &[]);
        let draft = RequestDraft {
            url: "https://{{h}}/users/{id}?x={{missing}}".into(),
            path_params: vec![KeyValue::new("id", "{{v}}")],
            params: vec![
                KeyValue::new("{{k}}", "{{v}}"),
                KeyValue {
                    enabled: false,
                    ..KeyValue::new("{{k}}", "off")
                },
            ],
            headers: vec![KeyValue::new("X-{{k}}", "{{v}}")],
            body: BodyKind::FormData {
                fields: vec![
                    FormField::text("{{k}}", "{{v}}"),
                    FormField::file("f", PathBuf::from("{{p}}")),
                ],
            },
            post_ops: vec![PostOp {
                enabled: true,
                kind: PostOpKind::Assert {
                    subject: ResponseSource::Status,
                    op: AssertOp::Equals,
                    expected: "{{v}}".into(),
                },
            }],
            ..Default::default()
        };
        let out = resolve_draft(&draft, &ctx);
        assert_eq!(
            out.draft.url,
            "https://example.com/users/{id}?x={{missing}}"
        );
        assert_eq!(out.draft.path_params[0].value, "V");
        assert_eq!(
            (
                out.draft.params[0].key.as_str(),
                out.draft.params[0].value.as_str()
            ),
            ("K", "V")
        );
        // 禁用行不替换
        assert_eq!(out.draft.params[1].key, "{{k}}");
        assert_eq!(out.draft.headers[0].key, "X-K");
        match &out.draft.body {
            BodyKind::FormData { fields } => {
                assert_eq!(fields[0].key, "K");
                assert_eq!(fields[0].value, FormValue::Text { value: "V".into() });
                assert_eq!(
                    fields[1].value,
                    FormValue::File {
                        path: PathBuf::from("/tmp/f"),
                        content_type: None
                    }
                );
            }
            other => panic!("{other:?}"),
        }
        match &out.draft.post_ops[0].kind {
            PostOpKind::Assert { expected, .. } => assert_eq!(expected, "V"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            out.unresolved.into_iter().collect::<Vec<_>>(),
            vec!["missing".to_string()]
        );
        // 原草稿不动
        assert!(draft.url.contains("{{h}}"));

        let raw = RequestDraft {
            body: BodyKind::Raw {
                format: RawFormat::Json,
                text: r#"{"k":"{{v}}"}"#.into(),
            },
            ..Default::default()
        };
        let out = resolve_draft(&raw, &ctx);
        assert_eq!(
            out.draft.body,
            BodyKind::Raw {
                format: RawFormat::Json,
                text: r#"{"k":"V"}"#.into()
            }
        );
        let bin = RequestDraft {
            body: BodyKind::Binary {
                path: PathBuf::from("{{p}}"),
                content_type: None,
            },
            ..Default::default()
        };
        assert_eq!(
            resolve_draft(&bin, &ctx).draft.body,
            BodyKind::Binary {
                path: PathBuf::from("/tmp/f"),
                content_type: None
            }
        );
        // pre_ops 的值不在这里替换（由 ops::run_pre_ops 在执行时替换）
        let pre = RequestDraft {
            pre_ops: vec![crate::model::PreOp {
                enabled: true,
                scope: VarScope::Global,
                key: "a".into(),
                value: "{{v}}".into(),
            }],
            ..Default::default()
        };
        assert_eq!(resolve_draft(&pre, &ctx).draft.pre_ops[0].value, "{{v}}");
    }

    #[test]
    fn valid_name_rules() {
        assert!(valid_name("a"));
        assert!(valid_name("$timestamp"));
        assert!(!valid_name(""));
        assert!(!valid_name("a{b"));
        assert!(!valid_name("a\nb"));
        assert!(!valid_name(&"x".repeat(MAX_NAME_LEN + 1)));
    }
}
