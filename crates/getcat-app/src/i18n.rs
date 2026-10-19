//! 界面语言：把设置里的偏好解析成 rust-i18n 的 locale，并在运行时切换。
//!
//! - 文案放在 `crates/getcat-app/locales/app.yml`（rust-i18n v2 格式，一个 key 下并列
//!   `en` / `zh-CN` / `zh-TW` / `ja`）；`rust_i18n::i18n!` 在 `main.rs` 注册，`fallback = "en"`。
//! - `rust_i18n::set_locale` 是跨 crate 的全局：gpui-component 自带的「确定 / 取消 / 搜索设置…」
//!   用的是同一个 rust-i18n，切换后一起生效，不需要我们补它的翻译。但它自带的 `ui.yml` 只有
//!   `en` / `zh-CN` / `zh-HK` / `zh-TW` / `it`——界面切到日文时那几条组件内置文案会按
//!   fallback 回落成英文，这是上游的覆盖范围，`app.yml` 无法替它补。繁体选 `zh-TW` 而不是
//!   `zh-Hant`，就是为了命中组件库的 `zh-TW` 文案（它缺的只有 GetCat 没用到的 Diff / DatePicker）。
//! - 渲染期取文案（`tr!`）的地方切换后随 `refresh_windows` 自动更新；驻留在实体里的字符串
//!   （输入框占位符等）通过 `cx.observe_global_in::<Locale>` 自己刷新。

use getcat_core::model::LanguagePref;
use gpui_kit::{App, Global};

/// 英文（也是兜底语言）。
pub const EN: &str = "en";
/// 简体中文。
pub const ZH_CN: &str = "zh-CN";
/// 繁体中文（台湾用语）；香港、澳门的系统语言也落到这里。
pub const ZH_TW: &str = "zh-TW";
/// 日文。
pub const JA: &str = "ja";

/// 当前生效的 locale；`set_global` 后所有 `observe_global::<Locale>` 的实体会被通知。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale(pub &'static str);

impl Global for Locale {}

/// 把偏好解析成 locale。`system` 是系统首选语言列表（按优先级排列）。
///
/// 跟随系统时按顺序找第一个能支持的：`zh*` → 简体或繁体中文（见 [`chinese_variant`]），
/// `ja*` → 日文，`en*` → 英文；一个都不匹配用英文。
pub fn resolve<I, S>(pref: LanguagePref, system: I) -> &'static str
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    match pref {
        LanguagePref::English => EN,
        LanguagePref::Chinese => ZH_CN,
        LanguagePref::TraditionalChinese => ZH_TW,
        LanguagePref::Japanese => JA,
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
        "zh" => Some(chinese_variant(tag)),
        "ja" => Some(JA),
        "en" => Some(EN),
        _ => None,
    }
}

/// 一个 `zh*` 系统语言标签该用简体还是繁体。
///
/// 标签的形态五花八门：macOS 给 `zh-Hant-TW` / `zh-Hans-CN`，Linux 的 LANG 是 `zh_TW.UTF-8`，
/// Windows 可能只给 `zh-HK`，也可能只有一个光秃秃的 `zh`。script 子标签（`Hans` / `Hant`）
/// 比地区更可靠——`zh-Hant-CN`（在大陆用繁体）与 `zh-Hans-HK` 都是真实存在的组合。
fn chinese_variant(tag: &str) -> &'static str {
    // `.` 之后是 Linux 的编码（`UTF-8` / `Big5-HKSCS`），`@` 之后是修饰符，都不参与判断
    let subtags: Vec<String> = tag
        .split(['.', '@'])
        .next()
        .unwrap_or("")
        .split(['-', '_'])
        .skip(1)
        .map(str::to_ascii_lowercase)
        .collect();
    // script 优先：显式写了 Hans / Hant 就以它为准，不看地区
    if subtags.iter().any(|s| s == "hant") {
        return ZH_TW;
    }
    if subtags.iter().any(|s| s == "hans") {
        return ZH_CN;
    }
    // 没有 script 时按地区推断：台港澳惯用繁体，其余（含光秃秃的 `zh`）按简体
    if subtags
        .iter()
        .any(|s| matches!(s.as_str(), "tw" | "hk" | "mo"))
    {
        ZH_TW
    } else {
        ZH_CN
    }
}

/// 当前生效的界面 locale；[`Locale`] 全局未安装（测试）时按英文处理。
pub fn current(cx: &App) -> &'static str {
    cx.try_global::<Locale>().map(|l| l.0).unwrap_or(EN)
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

/// `t!` 的 [`gpui_kit::SharedString`] 版本：绝大多数 gpui-component 的 API 都接受它。
macro_rules! tr {
    ($($args:tt)*) => {
        ::gpui_kit::SharedString::from(::rust_i18n::t!($($args)*).into_owned())
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
        assert_eq!(resolve(LanguagePref::TraditionalChinese, ["zh-CN"]), ZH_TW);
        assert_eq!(resolve(LanguagePref::Japanese, ["zh-CN"]), JA);
    }

    #[test]
    fn system_picks_the_first_supported_language_in_order() {
        assert_eq!(
            resolve(LanguagePref::System, ["fr-FR", "zh-Hans-CN", "en-US"]),
            ZH_CN
        );
        assert_eq!(
            resolve(LanguagePref::System, ["fr-FR", "zh-Hant-TW", "en-US"]),
            ZH_TW
        );
        assert_eq!(resolve(LanguagePref::System, ["en-GB", "zh-CN"]), EN);
        assert_eq!(resolve(LanguagePref::System, ["ja-JP", "zh-CN"]), JA);
    }

    #[test]
    fn japanese_system_tags_map_to_ja() {
        for tag in ["ja", "ja-JP", "ja_JP.UTF-8", "ja-Jpan-JP"] {
            assert_eq!(resolve(LanguagePref::System, [tag]), JA, "{tag}");
        }
    }

    #[test]
    fn simplified_chinese_system_tags_map_to_zh_cn() {
        for tag in [
            "zh",
            "zh-CN",
            "zh-Hans",
            "zh-Hans-CN",
            "zh-SG",
            "zh-Hans-HK",
            "zh_CN.UTF-8",
            "zh_SG.GB2312",
        ] {
            assert_eq!(resolve(LanguagePref::System, [tag]), ZH_CN, "{tag}");
        }
    }

    #[test]
    fn traditional_chinese_system_tags_map_to_zh_tw() {
        for tag in [
            "zh-TW",
            "zh-HK",
            "zh-MO",
            "zh-Hant",
            "zh-Hant-TW",
            "zh-Hant-HK",
            "zh-Hant-CN",
            "zh_TW.UTF-8",
            "zh_HK.Big5-HKSCS",
            "ZH-hant-tw",
        ] {
            assert_eq!(resolve(LanguagePref::System, [tag]), ZH_TW, "{tag}");
        }
    }

    #[test]
    fn unsupported_or_empty_system_list_falls_back_to_english() {
        assert_eq!(resolve(LanguagePref::System, ["ko-KR", "fr-FR"]), EN);
        assert_eq!(resolve(LanguagePref::System, Vec::<String>::new()), EN);
    }
}
