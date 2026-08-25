//! 应用资源：GetCat 自己的 logo 与补充图标叠在 gpui-component 的图标集之上。
//!
//! gpui 的 `img()` / `svg()` 都经由全局 `AssetSource` 取字节；gpui-component-assets
//! 只嵌入了它自带的那份 `icons/**`，应用私有资源需要自己的来源。这里不引入 rust-embed，
//! 只用 `include_bytes!` 逐个嵌入，其余路径原样交给上游。

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component_assets::Assets;

/// Logo 的资源路径（`img(LOGO_PATH)`）。位图走 `img()`：整张栅格化、保留原色；
/// `svg()` 是单色蒙版，只会被 `text_color` 染成一种颜色，多色 logo 用不了。
///
/// 这份 PNG 由 `scripts/gen-logo.py` 从 `assets/logo/cat.png` 合成，改 logo 要重跑脚本。
pub const LOGO_PATH: &str = "logo/getcat.png";

/// 「自动换行」切换按钮的图标。
pub const ICON_WRAP_TEXT: &str = "icons/wrap-text.svg";
/// 「导入 cURL」的图标。
pub const ICON_FILE_INPUT: &str = "icons/file-input.svg";
/// 标签栏「多行展示」切换按钮的图标。
pub const ICON_ROWS_3: &str = "icons/rows-3.svg";

/// 应用自带的资源表。上游 `gpui-component-assets` 没有的图标补在这里——
/// 三个都取自 Lucide（ISC），与上游图标集同源，风格与 24px 网格自然一致。
///
/// 补图标而不是拿现成的凑：`wrap-text` / `file-input` / `rows-3` 各自都是一眼能认出
/// 语义的标准图形，用意思相近的替代只会让按钮更难懂。
const ASSETS: &[(&str, &[u8])] = &[
    (LOGO_PATH, include_bytes!("../assets/logo/getcat.png")),
    (
        ICON_WRAP_TEXT,
        include_bytes!("../assets/icons/wrap-text.svg"),
    ),
    (
        ICON_FILE_INPUT,
        include_bytes!("../assets/icons/file-input.svg"),
    ),
    (ICON_ROWS_3, include_bytes!("../assets/icons/rows-3.svg")),
];

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = ASSETS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut entries = Assets.list(path)?;
        entries.extend(
            ASSETS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_is_served_from_the_embedded_bytes() {
        let bytes = AppAssets.load(LOGO_PATH).unwrap().expect("logo present");
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    /// 补的三个图标要真的能取到，而且是 gpui 的 `svg()` 认得的 SVG。
    #[test]
    fn app_icons_are_served_as_svg() {
        for path in [ICON_WRAP_TEXT, ICON_FILE_INPUT, ICON_ROWS_3] {
            let bytes = AppAssets
                .load(path)
                .unwrap()
                .unwrap_or_else(|| panic!("{path} missing"));
            let text = std::str::from_utf8(&bytes).expect("SVG 是 UTF-8");
            assert!(text.starts_with("<svg"), "{path} 不是 SVG");
            // 图标靠 text_color 染色，写死 stroke 会让它在深色主题里消失
            assert!(
                text.contains(r#"stroke="currentColor""#),
                "{path} 没有用 currentColor，换主题时会瞎"
            );
        }
    }

    #[test]
    fn icon_paths_still_come_from_gpui_component_assets() {
        let bytes = AppAssets
            .load("icons/moon.svg")
            .unwrap()
            .expect("upstream icon present");
        assert!(bytes.starts_with(b"<svg"));
        assert!(AppAssets.load("icons/does-not-exist.svg").is_err());
    }

    #[test]
    fn listing_includes_both_app_assets_and_upstream_icons() {
        let all = AppAssets.list("").unwrap();
        assert!(all.iter().any(|p| p.as_ref() == LOGO_PATH));
        assert!(all.iter().any(|p| p.as_ref() == ICON_WRAP_TEXT));
        assert!(all.iter().any(|p| p.as_ref() == "icons/moon.svg"));
        assert_eq!(
            AppAssets.list("logo").unwrap(),
            vec![SharedString::from(LOGO_PATH)]
        );
        // 前缀过滤要把应用图标和上游图标一起列出来
        let icons = AppAssets.list("icons").unwrap();
        assert!(icons.iter().any(|p| p.as_ref() == ICON_ROWS_3));
        assert!(icons.iter().any(|p| p.as_ref() == "icons/moon.svg"));
    }
}
