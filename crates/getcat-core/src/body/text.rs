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
    /// 被截掉的字节数（O(1) 计算）；0 表示未截断。
    pub hidden_bytes: usize,
}

/// 裁掉字节串**末尾被截断的多字节字符**（落盘响应只保留前 HEAD_BYTES，切口可能落在一个字的中间）。
/// 只处理"末尾不完整"这一种情况；合法输入与中间含非法字节的输入原样返回，交给调用方做有损转换。
pub fn trim_partial_utf8(bytes: &[u8]) -> &[u8] {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes,
        // error_len() == None：错误是"输入在一个字符中间结束"，valid_up_to 之前全部合法
        Err(e) if e.error_len().is_none() => &bytes[..e.valid_up_to()],
        Err(_) => bytes,
    }
}

/// 超过 MAX_LINE_CHARS 个字符的行只保留前 MAX_LINE_CHARS 个字符（按字符边界切）。
/// `hidden_bytes` 用剩余字节数（`line.len() - cut`）而非重新数字符，保持 O(MAX_LINE_CHARS)：
/// 单行几 MB 时若数剩余字符数会在每帧渲染中做一次 O(整行) 扫描。
pub fn clip_line(line: &str) -> ClippedLine<'_> {
    // 字节数 ≤ 上限则字符数必然 ≤ 上限：常见短行零开销
    if line.len() <= MAX_LINE_CHARS {
        return ClippedLine {
            text: line,
            hidden_bytes: 0,
        };
    }
    let cut = line
        .char_indices()
        .nth(MAX_LINE_CHARS)
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    ClippedLine {
        text: &line[..cut],
        hidden_bytes: line.len() - cut,
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
        assert_eq!(c.hidden_bytes, 0);
    }

    #[test]
    fn long_lines_are_clipped_on_char_boundaries() {
        let ascii = "a".repeat(MAX_LINE_CHARS + 1);
        let c = clip_line(&ascii);
        assert_eq!(c.text.len(), MAX_LINE_CHARS);
        assert_eq!(c.hidden_bytes, 1);

        // 每个 CJK 字符占 3 字节：截掉 500 个字符 == 1500 字节。
        let wide = "中".repeat(2500);
        let c = clip_line(&wide);
        assert_eq!(c.text.chars().count(), MAX_LINE_CHARS);
        assert_eq!(c.text.len(), MAX_LINE_CHARS * 3);
        assert_eq!(c.hidden_bytes, 500 * 3);
    }

    #[test]
    fn clip_line_is_fast_on_a_multi_megabyte_single_line() {
        // 证明 hidden_bytes 是 O(1) 算术而非重新扫描剩余字节：3 MB 单行不会超时/panic，
        // 且数值等于「总字节数 - 截断点字节数」。
        let line = "a".repeat(3_000_000);
        let c = clip_line(&line);
        assert_eq!(c.text.len(), MAX_LINE_CHARS);
        assert_eq!(c.hidden_bytes, 3_000_000 - MAX_LINE_CHARS);
    }

    #[test]
    fn trim_partial_utf8_only_drops_a_truncated_tail() {
        let full = "名名".as_bytes();
        assert_eq!(trim_partial_utf8(full), full);
        // 截掉最后一个字的最后 1 字节：尾部是不完整的多字节序列 → 裁掉整个不完整字符
        let cut = &full[..full.len() - 1];
        assert_eq!(trim_partial_utf8(cut), "名".as_bytes());
        // 只剩 1 字节的不完整前导字节 → 空
        assert_eq!(trim_partial_utf8(&full[..1]), b"");
        // 中间有非法字节：不是"末尾截断"，原样返回（由调用方做有损转换）
        let bad = [b'a', 0xFF, b'b'];
        assert_eq!(trim_partial_utf8(&bad), &bad[..]);
        assert_eq!(trim_partial_utf8(b""), b"");
    }
}
