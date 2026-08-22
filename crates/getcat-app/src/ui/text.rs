//! core 类型的界面文案：core 的 `Display` 是英文技术文案，界面上按变体翻译。
//!
//! 规则（与用户确认过）：错误的**种类**翻译，**技术细节**（reqwest 原话、路径、字段名）
//! 两种语言都保留英文原文。

use getcat_core::body::spill::HEAD_BYTES;
use getcat_core::body::tier::{EDITOR_MAX_BYTES, EDITOR_MAX_LINES, ViewTier, mib_label};
use getcat_core::detect::ContentKind;
use getcat_core::http::{MAX_BODY_BYTES, RequestError};
use getcat_core::model::{LanguagePref, ThemePref};
use gpui::SharedString;

use crate::i18n::tr;

/// 错误种类的短标签（状态行、失败页标题）。
pub fn error_kind(error: &RequestError) -> SharedString {
    match error {
        RequestError::InvalidUrl(_) => tr!("error.kind.invalid_url"),
        RequestError::InvalidHeader(_) => tr!("error.kind.invalid_header"),
        RequestError::Unsupported(_) => tr!("error.kind.unsupported"),
        RequestError::Dns(_) => tr!("error.kind.dns"),
        RequestError::ConnectionRefused(_) => tr!("error.kind.connection_refused"),
        RequestError::Tls(_) => tr!("error.kind.tls"),
        RequestError::Timeout => tr!("error.kind.timeout"),
        RequestError::Spill(_) => tr!("error.kind.spill"),
        RequestError::FileBody(_) => tr!("error.kind.file_body"),
        RequestError::Cancelled => tr!("error.kind.cancelled"),
        RequestError::Other(_) => tr!("error.kind.other"),
    }
}

/// 错误的说明：带载荷的变体直接给载荷（技术细节保留原文），其余给一句翻译。
pub fn error_detail(error: &RequestError) -> SharedString {
    match error {
        RequestError::InvalidUrl(s)
        | RequestError::InvalidHeader(s)
        | RequestError::Unsupported(s)
        | RequestError::Dns(s)
        | RequestError::ConnectionRefused(s)
        | RequestError::Tls(s)
        | RequestError::Spill(s)
        | RequestError::FileBody(s)
        | RequestError::Other(s) => s.clone().into(),
        RequestError::Timeout => tr!("error.detail.timeout"),
        RequestError::Cancelled => tr!("error.kind.cancelled"),
    }
}

/// 发送前校验失败的一行提示：种类 + 细节。
pub fn prepare_error_line(error: &RequestError) -> SharedString {
    let detail = error_detail(error);
    if detail.is_empty() {
        error_kind(error)
    } else {
        format!("{}: {}", error_kind(error), detail).into()
    }
}

/// 内容类型标签：JSON / XML / HTML 是专名不翻译，文本 / 二进制按语言显示。
pub fn content_kind_label(kind: ContentKind) -> SharedString {
    match kind {
        ContentKind::Text => tr!("content.text"),
        ContentKind::Binary => tr!("content.binary"),
        other => other.label().into(),
    }
}

/// 响应档位的横幅提示；A 档没有。数字全部来自 core 的阈值常量。
pub fn tier_notice(tier: ViewTier) -> Option<SharedString> {
    match tier {
        ViewTier::Editor => None,
        ViewTier::Virtual => Some(tr!(
            "response.tier.virtual",
            size = mib_label(EDITOR_MAX_BYTES as u64),
            lines = EDITOR_MAX_LINES
        )),
        ViewTier::Preview => Some(tr!(
            "response.tier.preview",
            size = mib_label(MAX_BODY_BYTES),
            head = mib_label(HEAD_BYTES as u64)
        )),
    }
}

pub fn theme_label(pref: ThemePref) -> SharedString {
    match pref {
        ThemePref::System => tr!("theme.system"),
        ThemePref::Light => tr!("theme.light"),
        ThemePref::Dark => tr!("theme.dark"),
    }
}

pub fn language_label(pref: LanguagePref) -> SharedString {
    match pref {
        LanguagePref::System => tr!("language.system"),
        LanguagePref::English => tr!("language.en"),
        LanguagePref::Chinese => tr!("language.zh"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试进程的 locale 是 rust-i18n 的默认值 en（没有人调用 set_locale）。
    #[test]
    fn error_kind_and_detail_split_label_from_payload() {
        let e = RequestError::Dns("lookup failed for example.invalid".into());
        assert_eq!(error_kind(&e).as_ref(), "DNS lookup failed");
        assert_eq!(
            error_detail(&e).as_ref(),
            "lookup failed for example.invalid"
        );
        assert_eq!(
            error_detail(&RequestError::Timeout).as_ref(),
            "The connection timed out"
        );
        assert_eq!(
            prepare_error_line(&RequestError::InvalidUrl("x".into())).as_ref(),
            "Invalid URL: x"
        );
    }

    #[test]
    fn content_kind_keeps_proper_nouns() {
        assert_eq!(content_kind_label(ContentKind::Json).as_ref(), "JSON");
        assert_eq!(content_kind_label(ContentKind::Text).as_ref(), "Text");
    }

    #[test]
    fn tier_notices_embed_the_thresholds() {
        let virt = tier_notice(ViewTier::Virtual).unwrap();
        assert!(virt.contains("5 MB"), "{virt}");
        assert!(virt.contains("200000"), "{virt}");
        assert!(tier_notice(ViewTier::Editor).is_none());
    }

    #[test]
    fn chinese_locale_is_wired_up() {
        assert_eq!(
            rust_i18n::t!("theme.system", locale = "zh-CN").as_ref(),
            "跟随系统"
        );
        assert_eq!(
            rust_i18n::t!("theme.system", locale = "en").as_ref(),
            "System"
        );
    }
}
