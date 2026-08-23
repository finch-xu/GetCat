//! 单遍、无 DOM 的 JSON 美化器。
//!
//! 只追踪"是否在字符串内 / 是否在转义中 / 嵌套深度"，O(n) 时间、O(1) 额外状态，
//! 不校验合法性（非法输入尽力缩进、绝不 panic）。

const INDENT: &[u8] = b"  ";
pub const CHECK_INTERVAL: usize = 1 << 20;
/// 缩进深度上限：更深的层级不再增加缩进。没有上限时，10 万层嵌套的 `[[[[…` 会让输出按深度平方膨胀
/// （每层一行、每行 2·深度 个空格），远超 1.5× 的预分配与 3× 的内存预算。
pub const MAX_INDENT_DEPTH: usize = 64;

/// 输入是否是合法 JSON。走 serde 的解析器，但用 `IgnoredAny` 丢弃所有值：不建 DOM、不分配。
///
/// [`pretty_json`] 对非法输入只做尽力缩进、从不报错，所以「格式化请求体」这类由用户触发、
/// 需要给出失败反馈的场景，得先过这道把关。
pub fn is_valid_json(input: &[u8]) -> bool {
    serde_json::from_slice::<serde::de::IgnoredAny>(input).is_ok()
}

pub fn pretty_json(input: &[u8]) -> Vec<u8> {
    pretty_json_cancellable(input, || false).expect("never cancelled")
}

pub fn pretty_json_cancellable(
    input: &[u8],
    mut should_cancel: impl FnMut() -> bool,
) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(input.len() + input.len() / 2);
    let mut depth: usize = 0;
    let mut in_str = false;
    let mut escaped = false;
    // 刚读到 `{` / `[`，尚未决定它是空容器（紧凑输出）还是需要换行缩进
    let mut pending_open: Option<u8> = None;
    // 单调递增的取消检查点：无论索引如何推进，每 CHECK_INTERVAL 字节至少检查一次（含 i = 0）
    let mut next_check = 0usize;

    for (i, &b) in input.iter().enumerate() {
        if i >= next_check {
            if should_cancel() {
                return None;
            }
            next_check = i + CHECK_INTERVAL;
        }

        if in_str {
            out.push(b);
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }

        if b.is_ascii_whitespace() {
            continue;
        }

        if let Some(closer) = pending_open.take() {
            if b == closer {
                out.push(b);
                continue;
            }
            depth += 1;
            newline(&mut out, depth);
        }

        match b {
            b'"' => {
                in_str = true;
                out.push(b);
            }
            b'{' => {
                out.push(b);
                pending_open = Some(b'}');
            }
            b'[' => {
                out.push(b);
                pending_open = Some(b']');
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(b);
            }
            b',' => {
                out.push(b);
                newline(&mut out, depth);
            }
            b':' => out.extend_from_slice(b": "),
            _ => out.push(b),
        }
    }
    Some(out)
}

#[inline]
fn newline(out: &mut Vec<u8>, depth: usize) {
    out.push(b'\n');
    for _ in 0..depth.min(MAX_INDENT_DEPTH) {
        out.extend_from_slice(INDENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pretty(s: &str) -> String {
        String::from_utf8(pretty_json(s.as_bytes())).unwrap()
    }

    #[test]
    fn formats_nested_structures() {
        let input = r#"{"a":1,"b":[1,2,{"c":"x}y"}],"d":{},"e":[ ]}"#;
        let expected = "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2,\n    {\n      \"c\": \"x}y\"\n    }\n  ],\n  \"d\": {},\n  \"e\": []\n}";
        assert_eq!(pretty(input), expected);
    }

    #[test]
    fn validity_check_accepts_only_real_json() {
        assert!(is_valid_json(br#"{"a":1,"b":[1,2,null]}"#));
        assert!(is_valid_json(b"[]"));
        assert!(is_valid_json(b"  {\"a\": 1}  "));
        // 裸标量也是合法 JSON 文档
        assert!(is_valid_json(b"42"));

        assert!(!is_valid_json(b""), "空输入不是合法 JSON");
        assert!(!is_valid_json(b"   "));
        assert!(!is_valid_json(br#"{"a":1,}"#), "尾随逗号");
        assert!(!is_valid_json(br#"{"a":"#), "截断");
        assert!(!is_valid_json(b"{'a':1}"), "单引号");
        assert!(!is_valid_json(&[0xFF, 0xFE, b'{']), "非 UTF-8 字节");
        // pretty_json 对这些只会尽力缩进，不会报错——正是需要这道把关的原因
        assert!(!pretty_json(br#"{"a":1,}"#).is_empty());
    }

    #[test]
    fn is_idempotent() {
        let once = pretty(r#"{"a":[1,{"b":null}],"c":"d"}"#);
        assert_eq!(pretty(&once), once);
    }

    #[test]
    fn preserves_escapes_and_string_contents() {
        assert_eq!(
            pretty(r#"{"q":"a\"b","bs":"\\","sp":"x  y","colon":"a:b"}"#),
            "{\n  \"q\": \"a\\\"b\",\n  \"bs\": \"\\\\\",\n  \"sp\": \"x  y\",\n  \"colon\": \"a:b\"\n}"
        );
    }

    #[test]
    fn passes_through_unicode_bytes() {
        assert_eq!(pretty(r#"{"名":"值"}"#), "{\n  \"名\": \"值\"\n}");
    }

    #[test]
    fn scalar_top_level_unchanged() {
        assert_eq!(pretty("123"), "123");
        assert_eq!(pretty("\"s\""), "\"s\"");
    }

    #[test]
    fn malformed_input_does_not_panic() {
        let _ = pretty_json(b"{{{");
        let _ = pretty_json(b"]]]");
        let _ = pretty_json(b"{\"a\":");
        let _ = pretty_json(&[0xFF, 0xFE, b'{']);
    }

    #[test]
    fn cancellation_checked_at_start() {
        assert_eq!(pretty_json_cancellable(b"{\"a\":1}", || true), None);
    }

    #[test]
    fn cancellation_checked_inside_long_whitespace_runs() {
        let big = format!("[{}]", " ".repeat(2 * CHECK_INTERVAL));
        let mut calls = 0;
        let out = pretty_json_cancellable(big.as_bytes(), || {
            calls += 1;
            calls > 2
        });
        assert_eq!(out, None);
        assert_eq!(calls, 3);
    }

    #[test]
    fn cancellation_checked_every_interval() {
        let big = format!("[{}]", "1,".repeat(CHECK_INTERVAL)); // > 2 MiB
        let mut calls = 0;
        let out = pretty_json_cancellable(big.as_bytes(), || {
            calls += 1;
            calls > 2
        });
        assert_eq!(out, None);
        assert_eq!(calls, 3);
    }

    #[test]
    fn indent_depth_is_capped() {
        let depth = 4 * MAX_INDENT_DEPTH;
        let input = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        let out = pretty(&input);
        let max_indent = out
            .lines()
            .map(|l| l.len() - l.trim_start().len())
            .max()
            .unwrap();
        assert_eq!(max_indent, 2 * MAX_INDENT_DEPTH);
        // 输出大小受缩进上限约束：每行最多 2·MAX_INDENT_DEPTH 个空格 + 1 个符号 + 换行
        assert!(
            out.len() <= input.len() * (2 * MAX_INDENT_DEPTH + 2),
            "{}",
            out.len()
        );
        // 结构不变：括号数与输入相同，且仍然幂等
        assert_eq!(out.matches('[').count(), depth);
        assert_eq!(out.matches(']').count(), depth);
        assert_eq!(pretty(&out), out);
    }
}
