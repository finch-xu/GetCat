//! 只读文本文档：文本 + 行索引，渲染线程只做切片。
//! 通过 `Arc<TextDoc>` 在实体与渲染闭包之间共享，每帧只 clone Arc。

use crate::body::line_index::LineIndex;

/// 单行最多显示的字符数；更长的行截断并提示剩余字符数。
pub const MAX_LINE_CHARS: usize = 2000;

#[derive(Debug)]
pub struct TextDoc {
    text: String,
    lines: LineIndex,
    longest_line: Option<usize>,
}

impl TextDoc {
    pub fn new(text: String) -> TextDoc {
        Self::new_cancellable(text, || false).expect("never cancelled")
    }

    pub fn new_cancellable(text: String, should_cancel: impl FnMut() -> bool) -> Option<TextDoc> {
        let lines = LineIndex::build_cancellable(text.as_bytes(), should_cancel)?;
        // 按字节数取最长行（用于虚拟列表的横向宽度测量）；O(行数)，在后台线程执行
        let longest_line = (0..lines.len()).max_by_key(|ix| lines.span(*ix).len());
        Some(TextDoc {
            text,
            lines,
            longest_line,
        })
    }

    /// 从字节构建：合法 UTF-8 直接接管 Vec（零拷贝），不合法则有损转换。
    pub fn from_bytes(bytes: Vec<u8>) -> TextDoc {
        Self::from_bytes_cancellable(bytes, || false).expect("never cancelled")
    }

    pub fn from_bytes_cancellable(
        bytes: Vec<u8>,
        should_cancel: impl FnMut() -> bool,
    ) -> Option<TextDoc> {
        let text = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        };
        Self::new_cancellable(text, should_cancel)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len_bytes(&self) -> usize {
        self.text.len()
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// 最长一行（按字节）的下标；空文档为 None。
    pub fn longest_line(&self) -> Option<usize> {
        self.longest_line
    }

    pub fn index(&self) -> &LineIndex {
        &self.lines
    }

    /// 文本缓冲 + 索引占用的堆内存（性能回归测试用；按 capacity 计）。
    pub fn heap_bytes(&self) -> usize {
        self.text.capacity() + self.lines.heap_bytes()
    }

    /// 第 `ix` 行文本，不含行尾 `\n` / `\r\n`。越界 panic。
    pub fn line(&self, ix: usize) -> &str {
        let s = &self.text[self.lines.span(ix)];
        let s = s.strip_suffix('\n').unwrap_or(s);
        s.strip_suffix('\r').unwrap_or(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClippedLine<'a> {
    pub text: &'a str,
    /// 被截掉的字符数；0 表示未截断。
    pub hidden_chars: usize,
}

/// 超过 MAX_LINE_CHARS 个字符的行只保留前 MAX_LINE_CHARS 个字符（按字符边界切）。
pub fn clip_line(line: &str) -> ClippedLine<'_> {
    // 字节数 ≤ 上限则字符数必然 ≤ 上限：常见短行零开销
    if line.len() <= MAX_LINE_CHARS {
        return ClippedLine {
            text: line,
            hidden_chars: 0,
        };
    }
    match line.char_indices().nth(MAX_LINE_CHARS) {
        None => ClippedLine {
            text: line,
            hidden_chars: 0,
        },
        Some((cut, _)) => ClippedLine {
            text: &line[..cut],
            hidden_chars: line[cut..].chars().count(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_trimmed_of_lf_and_crlf() {
        let doc = TextDoc::new("a\r\nbb\nccc".to_string());
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.line(0), "a");
        assert_eq!(doc.line(1), "bb");
        assert_eq!(doc.line(2), "ccc");
        assert_eq!(doc.longest_line(), Some(2));
        assert_eq!(doc.len_bytes(), 9);
    }

    #[test]
    fn empty_doc_has_no_lines() {
        let doc = TextDoc::new(String::new());
        assert_eq!(doc.line_count(), 0);
        assert_eq!(doc.longest_line(), None);
    }

    #[test]
    fn from_bytes_keeps_valid_utf8_and_repairs_invalid() {
        assert_eq!(
            TextDoc::from_bytes(b"\xe5\x90\x8d\n".to_vec()).line(0),
            "名"
        );
        let doc = TextDoc::from_bytes(vec![b'a', 0xFF, b'b']);
        assert_eq!(doc.line(0), "a\u{FFFD}b");
    }

    #[test]
    fn cancellation_propagates() {
        assert!(TextDoc::new_cancellable("x".to_string(), || true).is_none());
        assert!(TextDoc::from_bytes_cancellable(b"x".to_vec(), || true).is_none());
    }

    #[test]
    fn short_lines_are_not_clipped() {
        let line = "a".repeat(MAX_LINE_CHARS);
        let c = clip_line(&line);
        assert_eq!(c.text.len(), MAX_LINE_CHARS);
        assert_eq!(c.hidden_chars, 0);
    }

    #[test]
    fn long_lines_are_clipped_on_char_boundaries() {
        let ascii = "a".repeat(MAX_LINE_CHARS + 1);
        let c = clip_line(&ascii);
        assert_eq!(c.text.len(), MAX_LINE_CHARS);
        assert_eq!(c.hidden_chars, 1);

        let wide = "中".repeat(2500);
        let c = clip_line(&wide);
        assert_eq!(c.text.chars().count(), MAX_LINE_CHARS);
        assert_eq!(c.text.len(), MAX_LINE_CHARS * 3);
        assert_eq!(c.hidden_chars, 500);
    }
}
