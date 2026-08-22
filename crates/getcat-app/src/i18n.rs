//! 界面语言：把设置里的偏好解析成 rust-i18n 的 locale，并在运行时切换。
//!
//! - 文案放在 `crates/getcat-app/locales/app.yml`（rust-i18n v2 格式，一个 key 下并列 `en` / `zh-CN`）；
//!   `rust_i18n::i18n!` 在 `main.rs` 注册，`fallback = "en"`。
//! - `rust_i18n::set_locale` 是跨 crate 的全局：gpui-component 自带的「确定 / 取消 / 搜索设置…」
//!   用的是同一个 rust-i18n，切换后一起生效，不需要我们补它的翻译。
//! - 渲染期取文案（`tr!`）的地方切换后随 `refresh_windows` 自动更新；驻留在实体里的字符串
//!   （输入框占位符等）通过 `cx.observe_global_in::<Locale>` 自己刷新。

use getcat_core::model::LanguagePref;
use gpui::{App, Global};

/// 英文（也是兜底语言）。
pub const EN: &str = "en";
/// 中文；简体与繁体系统语言都落到这里。
pub const ZH_CN: &str = "zh-CN";

/// 当前生效的 locale；`set_global` 后所有 `observe_global::<Locale>` 的实体会被通知。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale(pub &'static str);

impl Global for Locale {}

/// 把偏好解析成 locale。`system` 是系统首选语言列表（按优先级排列）。
///
/// 跟随系统时按顺序找第一个能支持的：`zh*` → 中文，`en*` → 英文；一个都不匹配用英文。
pub fn resolve<I, S>(pref: LanguagePref, system: I) -> &'static str
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    match pref {
        LanguagePref::English => EN,
        LanguagePref::Chinese => ZH_CN,
        LanguagePref::System => system
            .into_iter()
            .find_map(|tag| supported(tag.as_ref()))
            .unwrap_or(EN),
    }
}

/// 一个 BCP 47 标签能否映射到我们支持的语言。
fn supported(tag: &str) -> Option<&'static str> {
    let lang = tag
        .split(['-', '_', '.', '@'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match lang.as_str() {
        "zh" => Some(ZH_CN),
        "en" => Some(EN),
        _ => None,
    }
}

/// 系统首选语言列表（macOS 读「语言与地区」的顺序；Linux 读 LANG / LC_ALL）。
///
/// 测试里返回空列表：`settings::install` 会按它解析语言，测试断言不能依赖宿主机的系统语言
/// （rust-i18n 的 locale 是进程级全局，测试线程之间会互相影响）。
pub fn system_locales() -> Vec<String> {
    if cfg!(test) {
        return Vec::new();
    }
    sys_locale::get_locales().collect()
}

/// 按偏好切换语言并通知所有窗口重绘；locale 没变时什么都不做。
pub fn apply(pref: LanguagePref, cx: &mut App) {
    let code = resolve(pref, system_locales());
    let unchanged = cx.try_global::<Locale>().is_some_and(|l| l.0 == code);
    if unchanged {
        return;
    }
    if &*rust_i18n::locale() != code {
        rust_i18n::set_locale(code);
    }
    cx.set_global(Locale(code));
    cx.refresh_windows();
}

/// 测试用：rust-i18n 的 locale 是进程级全局，测试线程并行会互相影响。
/// 凡是断言翻译文本、或会切换语言（`settings::install` / `update`）的测试都先拿这把锁。
#[cfg(test)]
pub(crate) fn locale_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `t!` 的 [`gpui::SharedString`] 版本：绝大多数 gpui-component 的 API 都接受它。
macro_rules! tr {
    ($($args:tt)*) => {
        ::gpui::SharedString::from(::rust_i18n::t!($($args)*).into_owned())
    };
}
pub(crate) use tr;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_preferences_ignore_the_system() {
        assert_eq!(resolve(LanguagePref::English, ["zh-Hans-CN"]), EN);
        assert_eq!(resolve(LanguagePref::Chinese, ["en-US"]), ZH_CN);
    }

    #[test]
    fn system_picks_the_first_supported_language_in_order() {
        assert_eq!(
            resolve(LanguagePref::System, ["ja-JP", "zh-Hant-TW", "en-US"]),
            ZH_CN
        );
        assert_eq!(resolve(LanguagePref::System, ["en-GB", "zh-CN"]), EN);
    }

    #[test]
    fn simplified_and_traditional_chinese_both_map_to_zh_cn() {
        for tag in [
            "zh",
            "zh-CN",
            "zh-Hans-CN",
            "zh-TW",
            "zh-Hant-TW",
            "zh-HK",
            "zh_CN.UTF-8",
        ] {
            assert_eq!(resolve(LanguagePref::System, [tag]), ZH_CN, "{tag}");
        }
    }

    #[test]
    fn unsupported_or_empty_system_list_falls_back_to_english() {
        assert_eq!(resolve(LanguagePref::System, ["ja-JP", "fr-FR"]), EN);
        assert_eq!(resolve(LanguagePref::System, Vec::<String>::new()), EN);
    }
}
