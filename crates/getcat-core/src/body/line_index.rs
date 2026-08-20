//! 行索引：记录每行起始字节偏移，用 memchr 单遍构建；渲染时 O(1) 取任意一行。

use std::ops::Range;

use crate::body::pretty::CHECK_INTERVAL;

/// 每行起始偏移用 u32 存储（4 字节/行）：B 档原始体 ≤ 64 MiB，美化后也远小于 4 GiB。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    starts: Vec<u32>,
    total_len: usize,
}

impl LineIndex {
    pub fn build(bytes: &[u8]) -> LineIndex {
        Self::build_cancellable(bytes, || false).expect("never cancelled")
    }

    /// 开头检查一次 `should_cancel`，之后每 CHECK_INTERVAL 字节至少再检查一次；返回 None 表示已取消。
    /// 检查点跟随换行符位置推进：整段无换行的输入只在开头检查一次（memchr 扫 64 MiB 仅需几毫秒）。
    pub fn build_cancellable(
        bytes: &[u8],
        mut should_cancel: impl FnMut() -> bool,
    ) -> Option<LineIndex> {
        assert!(
            bytes.len() <= u32::MAX as usize,
            "LineIndex supports at most 4 GiB of text"
        );
        if should_cancel() {
            return None;
        }
        // 经验值：美化后的 JSON 约每 16 字节一行；估少了会倍增，估多了浪费 4 字节/16 字节
        let mut starts: Vec<u32> = Vec::with_capacity(bytes.len() / 16 + 1);
        if bytes.is_empty() {
            return Some(LineIndex {
                starts,
                total_len: 0,
            });
        }
        starts.push(0);
        let mut next_check = CHECK_INTERVAL;
        for pos in memchr::memchr_iter(b'\n', bytes) {
            if pos >= next_check {
                if should_cancel() {
                    return None;
                }
                next_check = pos + CHECK_INTERVAL;
            }
            if pos + 1 < bytes.len() {
                starts.push((pos + 1) as u32);
            }
        }
        Some(LineIndex {
            starts,
            total_len: bytes.len(),
        })
    }

    /// 行数：空输入为 0；末尾的换行不会多算一行。
    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    /// 第 `ix` 行的原始字节区间（**包含**行尾的 `\n` / `\r\n`，如果有）。越界 panic。
    pub fn span(&self, ix: usize) -> Range<usize> {
        let start = self.starts[ix] as usize;
        let end = self
            .starts
            .get(ix + 1)
            .map_or(self.total_len, |s| *s as usize);
        start..end
    }

    /// 索引自身占用的堆内存（性能回归测试用）。
    pub fn heap_bytes(&self) -> usize {
        self.starts.capacity() * std::mem::size_of::<u32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::pretty::CHECK_INTERVAL;

    #[test]
    fn empty_input_has_no_lines() {
        let ix = LineIndex::build(b"");
        assert_eq!(ix.len(), 0);
        assert!(ix.is_empty());
    }

    #[test]
    fn single_line_without_newline() {
        let ix = LineIndex::build(b"abc");
        assert_eq!(ix.len(), 1);
        assert_eq!(ix.span(0), 0..3);
    }

    #[test]
    fn trailing_newline_does_not_add_a_line() {
        let ix = LineIndex::build(b"a\nb\n");
        assert_eq!(ix.len(), 2);
        assert_eq!(ix.span(0), 0..2);
        assert_eq!(ix.span(1), 2..4);
    }

    #[test]
    fn crlf_spans_include_terminator() {
        let ix = LineIndex::build(b"a\r\nb");
        assert_eq!(ix.len(), 2);
        assert_eq!(ix.span(0), 0..3);
        assert_eq!(ix.span(1), 3..4);
    }

    #[test]
    fn lone_newline_is_one_empty_line() {
        let ix = LineIndex::build(b"\n");
        assert_eq!(ix.len(), 1);
        assert_eq!(ix.span(0), 0..1);
    }

    #[test]
    fn cancellation_checked_at_start_and_every_interval() {
        assert_eq!(LineIndex::build_cancellable(b"a\nb", || true), None);
        // 2·CHECK_INTERVAL + 1 个换行：检查点在开头、CHECK_INTERVAL、2·CHECK_INTERVAL
        let big = vec![b'\n'; 2 * CHECK_INTERVAL + 1];
        let mut calls = 0;
        let out = LineIndex::build_cancellable(&big, || {
            calls += 1;
            calls > 2
        });
        assert_eq!(out, None);
        assert_eq!(calls, 3);
        assert_eq!(
            LineIndex::build_cancellable(&big, || false).unwrap().len(),
            2 * CHECK_INTERVAL + 1
        );
    }
}
