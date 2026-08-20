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
    for _ in 0..depth {
        out.extend_from_slice(INDENT);
    }
}
