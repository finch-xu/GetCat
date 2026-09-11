//! [`UpdateSource`] backed by the GitHub Releases API.

use serde::Deserialize;

use super::UpdateSource;
use crate::error::{Error, Result};
use crate::http;
use crate::release::{Asset, Release, parse_tag};

const API: &str = "https://api.github.com";

/// Reads releases from a GitHub repository's Releases.
///
/// ```no_run
/// use gpui_updater::GitHubSource;
///
/// let source = GitHubSource::new("AprilNEA", "OpenLogi")
///     .asset_contains("macos")
///     .asset_contains(".dmg")
///     .with_checksums("SHA256SUMS")
///     .with_minisig();
/// ```
pub struct GitHubSource {
    owner: String,
    repo: String,
    /// Case-insensitive substrings; an asset matches when its name contains all
    /// of them. Defaults to a single extension guessed from the running OS.
    asset_patterns: Vec<String>,
    /// Name of a checksums file asset (e.g. `SHA256SUMS`) used to resolve the
    /// expected SHA-256 of the selected asset.
    sums_asset: Option<String>,
    /// Suffix of the per-asset detached minisign signature (e.g. `.minisig`).
    signature_suffix: Option<String>,
    /// Whether to consider pre-releases as well as full releases.
    include_prereleases: bool,
    /// Optional token for private repos / higher rate limits.
    token: Option<String>,
}

impl GitHubSource {
    /// Create a source for `owner/repo`, matching assets by an extension
    /// guessed from the current OS (`.dmg` / `.exe` / `.tar.gz`).
    pub fn new(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            repo: repo.into(),
            asset_patterns: vec![default_extension().to_string()],
            sums_asset: None,
            signature_suffix: None,
            include_prereleases: false,
            token: None,
        }
    }

    /// Require the selected asset's name to contain `substr` (case-insensitive).
    /// Call multiple times to require several substrings (e.g. `macos` + `arm64`).
    #[must_use]
    pub fn asset_contains(mut self, substr: impl Into<String>) -> Self {
        self.asset_patterns.push(substr.into());
        self
    }

    /// Replace the asset-matching substrings entirely.
    #[must_use]
    pub fn asset_patterns(mut self, patterns: Vec<String>) -> Self {
        self.asset_patterns = patterns;
        self
    }

    /// Resolve the expected SHA-256 from a checksums file asset (one
    /// `<hex>  <name>` line per asset, as produced by `shasum -a 256`).
    #[must_use]
    pub fn with_checksums(mut self, sums_asset: impl Into<String>) -> Self {
        self.sums_asset = Some(sums_asset.into());
        self
    }

    /// Look for a detached minisign signature named `<asset>.minisig`.
    #[must_use]
    pub fn with_minisig(mut self) -> Self {
        self.signature_suffix = Some(".minisig".to_string());
        self
    }

    /// Use a custom signature suffix instead of `.minisig`.
    #[must_use]
    pub fn signature_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.signature_suffix = Some(suffix.into());
        self
    }

    /// Consider pre-releases in addition to full releases.
    #[must_use]
    pub fn include_prereleases(mut self, yes: bool) -> Self {
        self.include_prereleases = yes;
        self
    }

    /// Authenticate requests with a token (private repos / rate limits).
    #[must_use]
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    fn fetch_release(&self, headers: &[(&str, &str)]) -> Result<GhRelease> {
        if self.include_prereleases {
            let url = format!(
                "{API}/repos/{}/{}/releases?per_page=30",
                self.owner, self.repo
            );
            let list: Vec<GhRelease> = http::get_json(&url, headers)?;
            list.into_iter()
                .find(|r| !r.draft)
                .ok_or_else(|| Error::Parse("repository has no releases".to_string()))
        } else {
            let url = format!("{API}/repos/{}/{}/releases/latest", self.owner, self.repo);
            http::get_json(&url, headers)
        }
    }
}

impl UpdateSource for GitHubSource {
    fn fetch_latest(&self) -> Result<Release> {
        let auth = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let mut headers: Vec<(&str, &str)> = vec![
            ("accept", "application/vnd.github+json"),
            ("x-github-api-version", "2022-11-28"),
        ];
        if let Some(auth) = &auth {
            headers.push(("authorization", auth));
        }

        let release = self.fetch_release(&headers)?;
        let version = parse_tag(&release.tag_name)?;

        let main =
            select_asset(&release.assets, &self.asset_patterns).ok_or(Error::NoMatchingAsset {
                target_os: std::env::consts::OS,
                target_arch: std::env::consts::ARCH,
            })?;
        let asset = Asset {
            name: main.name.clone(),
            url: main.browser_download_url.clone(),
            size: main.size,
        };

        let signature_url = self.signature_suffix.as_ref().and_then(|suffix| {
            let want = format!("{}{suffix}", main.name);
            release
                .assets
                .iter()
                .find(|a| a.name == want)
                .map(|a| a.browser_download_url.clone())
        });

        let sha256 = match &self.sums_asset {
            Some(sums_name) => match release.assets.iter().find(|a| &a.name == sums_name) {
                Some(sums) => {
                    let text = http::get_string(&sums.browser_download_url, &headers)?;
                    parse_sha256_for(&text, &main.name)
                }
                None => None,
            },
            None => None,
        };

        Ok(Release {
            version,
            notes: release
                .body
                .filter(|s| !s.trim().is_empty())
                .or(release.name),
            asset,
            signature: None,
            signature_url,
            sha256,
        })
    }
}

fn default_extension() -> &'static str {
    match std::env::consts::OS {
        "macos" => ".dmg",
        "windows" => ".exe",
        _ => ".tar.gz",
    }
}

/// True when `name` contains every pattern (case-insensitive).
fn asset_matches(name: &str, patterns: &[String]) -> bool {
    let lower = name.to_ascii_lowercase();
    patterns
        .iter()
        .all(|p| lower.contains(&p.to_ascii_lowercase()))
}

fn select_asset<'a>(assets: &'a [GhAsset], patterns: &[String]) -> Option<&'a GhAsset> {
    assets.iter().find(|a| asset_matches(&a.name, patterns))
}

/// Find the SHA-256 for `asset_name` in the contents of a `shasum`-style file.
///
/// Lines look like `<hex>  <path>` (an optional `*` marks binary mode); the
/// `<path>` may carry a directory prefix, so matching is by basename.
fn parse_sha256_for(sums: &str, asset_name: &str) -> Option<String> {
    for line in sums.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(path)) = (parts.next(), parts.next()) else {
            continue;
        };
        let path = path.strip_prefix('*').unwrap_or(path);
        let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
        if base == asset_name {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pats(p: &[&str]) -> Vec<String> {
        p.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn asset_matching_requires_all_substrings_case_insensitively() {
        assert!(asset_matches(
            "OpenLogi-v0.2.0-macOS.DMG",
            &pats(&["macos", ".dmg"])
        ));
        assert!(!asset_matches(
            "OpenLogi-v0.2.0-linux.tar.gz",
            &pats(&["macos"])
        ));
        assert!(asset_matches(
            "app-arm64-macos.dmg",
            &pats(&["arm64", "macos"])
        ));
        assert!(!asset_matches(
            "app-x86_64-macos.dmg",
            &pats(&["arm64", "macos"])
        ));
    }

    #[test]
    fn select_picks_first_matching_asset() {
        let assets = vec![
            GhAsset {
                name: "SHA256SUMS".into(),
                browser_download_url: "u0".into(),
                size: 0,
            },
            GhAsset {
                name: "app-linux.tar.gz".into(),
                browser_download_url: "u1".into(),
                size: 1,
            },
            GhAsset {
                name: "app-macos.dmg".into(),
                browser_download_url: "u2".into(),
                size: 2,
            },
        ];
        let picked = select_asset(&assets, &pats(&[".dmg"])).unwrap();
        assert_eq!(picked.name, "app-macos.dmg");
        assert!(select_asset(&assets, &pats(&[".exe"])).is_none());
    }

    #[test]
    fn parses_checksum_by_basename_ignoring_dir_prefix() {
        let sums = "\
abc123  dist/app-macos.dmg
DEF456  dist/app-linux.tar.gz
";
        assert_eq!(
            parse_sha256_for(sums, "app-macos.dmg").as_deref(),
            Some("abc123")
        );
        // case-normalised to lowercase
        assert_eq!(
            parse_sha256_for(sums, "app-linux.tar.gz").as_deref(),
            Some("def456")
        );
        assert!(parse_sha256_for(sums, "app-windows.exe").is_none());
    }

    #[test]
    fn deserializes_github_release_json() {
        let json = r#"{
            "tag_name": "v0.2.0",
            "name": "Release 0.2.0",
            "body": "notes here",
            "draft": false,
            "assets": [
                {"name": "app-macos.dmg", "browser_download_url": "https://x/app-macos.dmg", "size": 1234}
            ]
        }"#;
        let r: GhRelease = serde_json::from_str(json).unwrap();
        assert_eq!(r.tag_name, "v0.2.0");
        assert_eq!(r.assets.len(), 1);
        assert_eq!(r.assets[0].size, 1234);
        assert_eq!(parse_tag(&r.tag_name).unwrap().minor, 2);
    }
}
