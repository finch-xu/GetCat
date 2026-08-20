//! 三档展示策略的选档（spec §6.3）。

/// A 档（只读 Editor）允许的最大字节数与行数；超出任一进入 B 档（虚拟列表）。
pub const EDITOR_MAX_BYTES: usize = 5 * 1024 * 1024;
pub const EDITOR_MAX_LINES: usize = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewTier {
    /// A：gpui-component Editor，语法高亮、搜索
    Editor,
    /// B：uniform_list 纯文本虚拟化
    Virtual,
    /// C：落盘响应，仅对前 1 MiB 做 B 档处理并附摘要
    Preview,
}

impl ViewTier {
    /// 用户可见的一行提示；A 档无提示。
    pub fn notice(self) -> Option<&'static str> {
        match self {
            ViewTier::Editor => None,
            ViewTier::Virtual => Some("大响应：已切换为纯文本模式"),
            ViewTier::Preview => Some("响应超过 64 MB，已写入临时文件，仅预览前 1 MB"),
        }
    }
}

/// 对一份**驻留内存**的文本选档（A 或 B）。Preview 只由调用方在响应已落盘时指定。
pub fn select_tier(bytes: usize, lines: usize) -> ViewTier {
    if bytes <= EDITOR_MAX_BYTES && lines <= EDITOR_MAX_LINES {
        ViewTier::Editor
    } else {
        ViewTier::Virtual
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_tier_requires_both_limits() {
        assert_eq!(select_tier(0, 0), ViewTier::Editor);
        assert_eq!(
            select_tier(EDITOR_MAX_BYTES, EDITOR_MAX_LINES),
            ViewTier::Editor
        );
        assert_eq!(select_tier(EDITOR_MAX_BYTES + 1, 1), ViewTier::Virtual);
        assert_eq!(select_tier(1, EDITOR_MAX_LINES + 1), ViewTier::Virtual);
    }

    #[test]
    fn notices() {
        assert!(ViewTier::Editor.notice().is_none());
        assert!(ViewTier::Virtual.notice().unwrap().contains("纯文本"));
        assert!(ViewTier::Preview.notice().unwrap().contains("1 MB"));
    }
}
