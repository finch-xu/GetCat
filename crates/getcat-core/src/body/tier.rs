//! 三档展示策略的选档（spec §6.3）。

use std::sync::LazyLock;

use crate::body::spill::HEAD_BYTES;
use crate::http::MAX_BODY_BYTES;

/// A 档（只读 Editor）允许的最大字节数与行数；超出任一进入 B 档（虚拟列表）。
pub const EDITOR_MAX_BYTES: usize = 5 * 1024 * 1024;
pub const EDITOR_MAX_LINES: usize = 200_000;

/// 提示文案里的体积写法：整 MiB 不带小数（"64 MB"），否则保留一位小数（"1.5 MB"）。
pub fn mib_label(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes.is_multiple_of(MIB) {
        format!("{} MB", bytes / MIB)
    } else {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    }
}

// 文案只构造一次；数字全部来自阈值常量，改阈值时文案自动跟随。
static VIRTUAL_NOTICE: LazyLock<String> = LazyLock::new(|| {
    format!(
        "大响应：超过 {} 或 {} 行，已切换为纯文本模式",
        mib_label(EDITOR_MAX_BYTES as u64),
        EDITOR_MAX_LINES
    )
});
static PREVIEW_NOTICE: LazyLock<String> = LazyLock::new(|| {
    format!(
        "响应超过 {}，已写入临时文件，仅预览前 {}",
        mib_label(MAX_BODY_BYTES),
        mib_label(HEAD_BYTES as u64)
    )
});

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
            ViewTier::Virtual => Some(VIRTUAL_NOTICE.as_str()),
            ViewTier::Preview => Some(PREVIEW_NOTICE.as_str()),
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
    fn mib_label_formats_whole_and_fractional_mebibytes() {
        assert_eq!(mib_label(64 * 1024 * 1024), "64 MB");
        assert_eq!(mib_label(1024 * 1024), "1 MB");
        assert_eq!(mib_label(1024 * 1024 + 512 * 1024), "1.5 MB");
    }

    #[test]
    fn notices_are_derived_from_the_thresholds() {
        assert!(ViewTier::Editor.notice().is_none());
        let virtual_notice = ViewTier::Virtual.notice().unwrap();
        assert!(
            virtual_notice.contains(&mib_label(EDITOR_MAX_BYTES as u64)),
            "{virtual_notice}"
        );
        assert!(
            virtual_notice.contains(&EDITOR_MAX_LINES.to_string()),
            "{virtual_notice}"
        );
        assert!(virtual_notice.contains("纯文本"));
        let preview_notice = ViewTier::Preview.notice().unwrap();
        assert!(
            preview_notice.contains(&mib_label(MAX_BODY_BYTES)),
            "{preview_notice}"
        );
        assert!(
            preview_notice.contains(&mib_label(HEAD_BYTES as u64)),
            "{preview_notice}"
        );
    }
}
