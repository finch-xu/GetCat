//! 单遍、无 DOM 的 JSON 美化器。
//!
//! 只追踪"是否在字符串内 / 是否在转义中 / 嵌套深度"，O(n) 时间、O(1) 额外状态，
//! 不校验合法性（非法输入尽力缩进、绝不 panic）。

const INDENT: &[u8] = b"  ";
pub const CHECK_INTERVAL: usize = 1 << 20;

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
    let mut i = 0;

    while i < input.len() {
        if i % CHECK_INTERVAL == 0 && should_cancel() {
            return None;
        }
        let b = input[i];

        if in_str {
            out.push(b);
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }

        match b {
            b'"' => {
                in_str = true;
                out.push(b);
            }
            b'{' | b'[' => {
                out.push(b);
                let mut j = i + 1;
                while j < input.len() && input[j].is_ascii_whitespace() {
                    j += 1;
                }
                let closer = if b == b'{' { b'}' } else { b']' };
                if j < input.len() && input[j] == closer {
                    out.push(closer);
                    i = j + 1;
                    continue;
                }
                depth += 1;
                newline(&mut out, depth);
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
            b' ' | b'\t' | b'\n' | b'\r' => {}
            _ => out.push(b),
        }
        i += 1;
    }
    Some(out)
}

#[inline]
fn newline(out: &mut Vec<u8>, depth: usize) {
    out.push(b'\n');
    for _ in 0..depth {
        out.extend_from_slice(INDENT);
    }
}
