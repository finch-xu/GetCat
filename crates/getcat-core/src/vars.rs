//! `{{name}}` 变量替换。
//!
//! 替换在 [`crate::http::prepare`] **之前**对 `RequestDraft` 的副本做：先展开 `{{var}}`，
//! 再由 `build_url` 单遍替换 Path 参数 `{id}`——两种语法互不干扰
//! （`extract_path_params` 会跳过名字里含 `{` 的片段）。已保存请求里始终存原文。
//!
//! 优先级（高 → 低）：内置动态变量（`$` 开头）> 环境 > 分类 > 全局；同层重名取第一条启用的。
//! 值里可再引用变量，最多 [`MAX_DEPTH`] 层；循环或未定义的都原样保留并记入 `unresolved`。

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use memchr::memmem;

use crate::model::{AssertOp, BodyKind, FormValue, KeyValue, PostOpKind, RequestDraft, Variable};

/// 值里再引用变量时的最大展开层数。
pub const MAX_DEPTH: usize = 8;
/// 变量名长度上限（按 UTF-8 字节计）。
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

/// 变量名是否合法：非空、不超过 [`MAX_NAME_LEN`]（按 UTF-8 字节计）、不含花括号与换行。
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

/// 一次替换会话：动态变量在会话内只取值一次，未解析的名字累计到 `unresolved`。
pub struct Resolver<'a> {
    ctx: &'a VarContext<'a>,
    dynamic: HashMap<String, String>,
    unresolved: BTreeSet<String>,
    /// 会话内第一次需要时读一次时钟（Unix 秒），`$timestamp` 与 `$isoTimestamp` 共用。
    now: Option<u64>,
}

impl<'a> Resolver<'a> {
    pub fn new(ctx: &'a VarContext<'a>) -> Self {
        Self {
            ctx,
            dynamic: HashMap::new(),
            unresolved: BTreeSet::new(),
            now: None,
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
                // 不是变量（比如 `{{a{{b}}` 的外层）：吐出这两个字符，从后面继续找。
                // 段内还有 `{{` 时直接跳到其中最后一个（逐 2 字节前进会对每个 `{{` 重找一遍
                // `}}`，`"{{ x " * n + "}}"` 这类输入就成了 O(n²)）。
                out.push_str("{{");
                match last_open_in(&after.as_bytes()[..len]) {
                    Some(ix) => {
                        out.push_str(&after[..ix]);
                        rest = &after[ix..];
                    }
                    None => rest = after,
                }
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
            let v = self.dynamic_value(name)?;
            self.dynamic.insert(name.to_string(), v.clone());
            return Some(v);
        }
        self.ctx.lookup(name).map(str::to_string)
    }

    fn dynamic_value(&mut self, name: &str) -> Option<String> {
        Some(match name {
            "$timestamp" => self.now().to_string(),
            "$isoTimestamp" => iso_utc(self.now()),
            "$randomUUID" | "$guid" => uuid::Uuid::new_v4().to_string(),
            // Postman 语义：0..=1000
            "$randomInt" => (uuid::Uuid::new_v4().as_u128() % 1001).to_string(),
            _ => return None,
        })
    }

    fn now(&mut self) -> u64 {
        *self.now.get_or_insert_with(unix_secs)
    }
}

/// `seg`（`{{` 与其后第一个 `}}` 之间的内容）里还有 `{{` 时，返回逐个前进会停在的**最后一个**
/// `{{` 的起点；没有则 None。
///
/// 逐个前进是从左到右、不重叠地匹配：一串连续的 `{` 两两配对，奇数个时最后一个落单。
/// 所以不能直接用 `rfind` 的位置（它可能落在重叠处，比如 `{{{` 里的第 1 个字节），
/// 要按这串 `{` 的起点对齐到偶数偏移。`seg` 紧跟在一个已配对的 `{{` 之后，从它开头对齐即可。
fn last_open_in(seg: &[u8]) -> Option<usize> {
    let last = memmem::rfind(seg, b"{{")?;
    // `last + 1` 是最后一串 `{` 的最后一个字节（否则 rfind 会找到更靠后的位置）
    let run_start = seg[..last]
        .iter()
        .rposition(|&b| b != b'{')
        .map_or(0, |i| i + 1);
    let run_len = last + 2 - run_start;
    Some(run_start + (run_len / 2 - 1) * 2)
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

/// 原地替换草稿的所有可替换字段（只处理启用的行），放进 [`Resolved`]。
///
/// 按值接收：调用方手里都是刚取出、用完即丢的草稿，不必再为多 MB 的 body 克隆一次。
/// Path 参数只替换值——key 由 URL 里的 `{id}` 驱动。前置操作的值**不在这里**替换，
/// 它们在执行时（[`crate::ops::run_pre_ops`]）按当时的变量表替换。
pub fn resolve_draft(draft: RequestDraft, ctx: &VarContext) -> Resolved {
    let mut r = Resolver::new(ctx);
    let mut out = draft;
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
    for post in out.post_ops.iter_mut().filter(|o| o.enabled) {
        // Exists 不看期望值：不替换，也就不会因残留的 `{{x}}` 报未解析
        if let PostOpKind::Assert { op, expected, .. } = &mut post.kind
            && *op != AssertOp::Exists
        {
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
        AssertOp, BodyKind, FormField, FormValue, KeyValue, PostOp, PostOpKind, PreOpKind,
        RawFormat, RequestDraft, ResponseSource, VarScope,
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
        let globals = vars(&[("b", "B"), ("c", "C"), ("x", "X")]);
        let ctx = VarContext::new(&globals, &[], &[]);
        let mut r = Resolver::new(&ctx);
        for (input, want) in [
            // 外层名字含 `{`：不是变量；内层 {{b}} 照常替换
            ("{{a{{b}}", "{{aB"),
            ("{{{c}}", "{{{c}}"),
            ("{{{{c}}", "{{C"),
            ("{{a{{{}}", "{{a{{{}}"),
            ("{{unclosed", "{{unclosed"),
            ("{{}}", "{{}}"),
            // 连续的 `{` 从左到右不重叠地两两配对：奇数个时落单的 `{` 让里面的名字不成立
            ("{{{{{x}}", "{{{{{x}}"),
            ("{{{{{{x}}", "{{{{X"),
            ("{{{ {{x}}", "{{{ X"),
            ("{{ {{{x}} }}", "{{ {{{x}} }}"),
            ("{{a}b}}{{x}}", "{{a}b}}X"),
        ] {
            assert_eq!(r.resolve(input), want, "{input:?}");
        }
        assert!(r.finish().is_empty());
    }

    /// 名字非法时若只前进 2 字节再重找 `}}`，`"{{ x " * n + "}}"` 是 O(n²)；
    /// 替换会在 UI 主线程上对多 MB 的 body 执行，必须线性。
    #[test]
    fn malformed_braces_resolve_in_linear_time() {
        let input = "{{ x ".repeat(200_000) + "}}";
        assert!(input.len() >= 1_000_000);
        let ctx = VarContext::EMPTY;
        let mut r = Resolver::new(&ctx);
        let started = std::time::Instant::now();
        let out = r.resolve(&input);
        let elapsed = started.elapsed();
        assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
        assert_eq!(out, input.as_str());
        assert_eq!(r.finish(), BTreeSet::from(["x".to_string()]));
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
        // d0…d7 共 MAX_DEPTH 层展开，第 9 层的 {{d8}} 原样保留
        assert_eq!(r.resolve("{{d0}}"), "{{d8}}");
        assert_eq!(r.finish(), BTreeSet::from(["d8".to_string()]));
    }

    #[test]
    fn timestamp_and_iso_timestamp_share_one_clock_reading() {
        let ctx = VarContext::EMPTY;
        let mut r = Resolver::new(&ctx);
        let ts: u64 = r.resolve("{{$timestamp}}").parse().unwrap();
        assert_eq!(r.resolve("{{$isoTimestamp}}"), iso_utc(ts));
        // 同一个 Resolver 只读一次时钟：先取 iso 再取秒数，也基于同一个读数
        let mut r = Resolver::new(&ctx);
        r.now = Some(951_782_400);
        assert_eq!(
            r.resolve("{{$isoTimestamp}} {{$timestamp}}"),
            "2000-02-29T00:00:00Z 951782400"
        );
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
        let input = draft.clone();
        let out = resolve_draft(draft, &ctx);
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
        assert_eq!(out.draft.params[1], input.params[1]);
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
        // 按值接收、原地替换：与克隆的输入对比，确实改过
        assert_ne!(out.draft, input);

        let raw = RequestDraft {
            body: BodyKind::Raw {
                format: RawFormat::Json,
                text: r#"{"k":"{{v}}"}"#.into(),
            },
            ..Default::default()
        };
        let out = resolve_draft(raw, &ctx);
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
            resolve_draft(bin, &ctx).draft.body,
            BodyKind::Binary {
                path: PathBuf::from("/tmp/f"),
                content_type: None
            }
        );
        // pre_ops 的值不在这里替换（由 ops::run_pre_ops 在执行时替换）
        let pre = RequestDraft {
            pre_ops: vec![crate::model::PreOp {
                enabled: true,
                kind: PreOpKind::SetVariable {
                    scope: VarScope::Global,
                    key: "a".into(),
                    value: "{{v}}".into(),
                },
            }],
            ..Default::default()
        };
        assert_eq!(resolve_draft(pre.clone(), &ctx).draft.pre_ops, pre.pre_ops);
        // urlencoded：启用行的 key / value 都替换，禁用行不动
        let form = RequestDraft {
            body: BodyKind::FormUrlEncoded {
                fields: vec![
                    KeyValue::new("{{k}}", "{{v}}"),
                    KeyValue {
                        enabled: false,
                        ..KeyValue::new("{{k}}", "{{v}}")
                    },
                ],
            },
            ..Default::default()
        };
        assert_eq!(
            resolve_draft(form, &ctx).draft.body,
            BodyKind::FormUrlEncoded {
                fields: vec![
                    KeyValue::new("K", "V"),
                    KeyValue {
                        enabled: false,
                        ..KeyValue::new("{{k}}", "{{v}}")
                    },
                ],
            }
        );
    }

    fn assertion(op: AssertOp, expected: &str) -> PostOp {
        PostOp {
            enabled: true,
            kind: PostOpKind::Assert {
                subject: ResponseSource::Status,
                op,
                expected: expected.into(),
            },
        }
    }

    /// Exists 不看期望值，所以不替换，也就不会因残留的 `{{x}}` 报未解析。
    #[test]
    fn exists_assertion_expected_is_not_resolved() {
        let globals = vars(&[("v", "V")]);
        let ctx = VarContext::new(&globals, &[], &[]);
        let draft = RequestDraft {
            post_ops: vec![
                assertion(AssertOp::Exists, "{{nope}}"),
                assertion(AssertOp::Equals, "{{v}}"),
                assertion(AssertOp::Exists, "{{v}}"),
            ],
            ..Default::default()
        };
        let out = resolve_draft(draft, &ctx);
        let expected: Vec<&str> = out
            .draft
            .post_ops
            .iter()
            .map(|op| match &op.kind {
                PostOpKind::Assert { expected, .. } => expected.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(expected, ["{{nope}}", "V", "{{v}}"]);
        assert!(out.unresolved.is_empty(), "{:?}", out.unresolved);
    }

    /// 先展开 `{{}}` 再由 `build_url` 替换 `{id}`：两步合起来的最终 URL。
    #[test]
    fn variables_expand_before_path_params_are_substituted() {
        let globals = vars(&[("base", "https://api.example.com"), ("v", "42")]);
        let ctx = VarContext::new(&globals, &[], &[]);
        let draft = RequestDraft {
            url: "{{base}}/users/{id}".into(),
            path_params: vec![KeyValue::new("id", "{{v}}")],
            ..Default::default()
        };
        let resolved = resolve_draft(draft, &ctx);
        assert!(resolved.unresolved.is_empty());
        let req = crate::http::prepare(&resolved.draft).unwrap();
        assert_eq!(req.url.path(), "/users/42");
        assert_eq!(req.url.as_str(), "https://api.example.com/users/42");
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
