//! 响应体处理：美化、分行索引、选档、落盘等纯计算。
pub mod line_index;
pub mod pretty;
pub mod spill;
pub mod text;
pub mod tier;

#[cfg(test)]
mod tests {
    use crate::body::pretty::{CHECK_INTERVAL, pretty_json, pretty_json_cancellable};

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
}
