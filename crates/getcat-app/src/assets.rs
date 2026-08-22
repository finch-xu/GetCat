//! 应用资源：GetCat 自己的 logo 叠在 gpui-component 的图标集之上。
//!
//! gpui 的 `img()` / `svg()` 都经由全局 `AssetSource` 取字节；gpui-component-assets
//! 只嵌入了 `icons/**`，logo 这种应用私有资源需要自己的来源。这里不引入 rust-embed，
//! 只用 `include_bytes!` 嵌入单个文件，其余路径原样交给上游。

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component_assets::Assets;

/// Logo 的资源路径（`img(LOGO_PATH)`）。位图走 `img()`：整张栅格化、保留原色；
/// `svg()` 是单色蒙版，只会被 `text_color` 染成一种颜色，多色 logo 用不了。
///
/// 这份 PNG 由 `scripts/gen-logo.py` 从 `assets/logo/cat.png` 合成，改 logo 要重跑脚本。
pub const LOGO_PATH: &str = "logo/getcat.png";

const LOGO: &[u8] = include_bytes!("../assets/logo/getcat.png");

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == LOGO_PATH {
            return Ok(Some(Cow::Borrowed(LOGO)));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut entries = Assets.list(path)?;
        if LOGO_PATH.starts_with(path) {
            entries.push(LOGO_PATH.into());
        }
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
    fn listing_the_root_includes_the_logo_and_the_icons() {
        let all = AppAssets.list("").unwrap();
        assert!(all.iter().any(|p| p.as_ref() == LOGO_PATH));
        assert!(all.iter().any(|p| p.as_ref() == "icons/moon.svg"));
        assert_eq!(
            AppAssets.list("logo").unwrap(),
            vec![SharedString::from(LOGO_PATH)]
        );
    }
}
