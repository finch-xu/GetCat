//! [`UpdateSource`] backed by a static JSON release manifest.
//!
//! This source works with any HTTPS-hosted manifest: Cloudflare R2, AWS S3,
//! `MinIO`, Backblaze B2, GitHub Pages, GitHub Releases, or a normal static web
//! server. It does not speak the S3 API; the updater only needs a static JSON
//! document plus direct artifact URLs.

use serde::Deserialize;

use super::UpdateSource;
use crate::error::{Error, Result};
use crate::http;
use crate::release::{Asset, Release, parse_tag};

/// Reads release metadata from a static JSON manifest.
///
/// The manifest describes one latest release and a flat list of downloadable
/// assets. Unknown fields are ignored, so producers can include additional
/// metadata such as `app_id`, `channel`, `release_url`, or platform-specific
/// compatibility fields.
///
/// Required manifest fields are `version` and `assets`; required asset fields
/// are `name` and `url`.
///
/// ```no_run
/// use gpui_updater::StaticManifestSource;
///
/// let source = StaticManifestSource::new("https://dl.example.com/channels/stable/latest.json")
///     .os("macos")
///     .arch("arm64")
///     .format("dmg");
/// ```
pub struct StaticManifestSource {
    url: String,
    /// Case-insensitive substrings; an asset matches when its name contains all
    /// of them. Defaults to a single extension guessed from the running OS.
    asset_patterns: Vec<String>,
    /// Optional exact match against the asset's `os` field.
    os: Option<String>,
    /// Optional exact match against the asset's `arch` field.
    arch: Option<String>,
    /// Optional exact match against the asset's `format` field.
    format: Option<String>,
}

impl StaticManifestSource {
    /// Create a static-manifest source, matching assets by an extension guessed
    /// from the current OS (`.dmg` / `.exe` / `.tar.gz`).
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            asset_patterns: vec![default_extension().to_string()],
            os: None,
            arch: None,
            format: None,
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

    /// Require the selected asset's `os` field to equal `os` (case-insensitive).
    ///
    /// Common values are `macos`, `windows`, and `linux`.
    #[must_use]
    pub fn os(mut self, os: impl Into<String>) -> Self {
        self.os = Some(os.into());
        self
    }

    /// Require the selected asset's `arch` field to equal `arch` (case-insensitive).
    ///
    /// Common values are `arm64`, `x86_64`, and `universal`.
    #[must_use]
    pub fn arch(mut self, arch: impl Into<String>) -> Self {
        self.arch = Some(arch.into());
        self
    }

    /// Require the selected asset's `format` field to equal `format`
    /// (case-insensitive).
    ///
    /// Common values are `dmg`, `zip`, `msi`, `exe`, `appimage`, and `tar.gz`.
    #[must_use]
    pub fn format(mut self, format: impl Into<String>) -> Self {
        if self.asset_patterns.len() == 1
            && self.asset_patterns[0].eq_ignore_ascii_case(default_extension())
        {
            self.asset_patterns.clear();
        }
        self.format = Some(format.into());
        self
    }

    /// The manifest URL this source fetches.
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl UpdateSource for StaticManifestSource {
    fn fetch_latest(&self) -> Result<Release> {
        let manifest: Manifest = http::get_json(&self.url, &[])?;
        manifest.validate_schema_version()?;
        let version = parse_tag(&manifest.version)?;
        let selector = AssetSelector {
            name_patterns: &self.asset_patterns,
            os: self.os.as_deref(),
            arch: self.arch.as_deref(),
            format: self.format.as_deref(),
        };
        let main = select_asset(&manifest.assets, selector).ok_or(Error::NoMatchingAsset {
            target_os: std::env::consts::OS,
            target_arch: std::env::consts::ARCH,
        })?;

        Ok(Release {
            version,
            notes: manifest
                .notes
                .filter(|s| !s.trim().is_empty())
                .or(manifest.notes_url),
            asset: Asset {
                name: main.name.clone(),
                url: main.url.clone(),
                size: main.size.unwrap_or(0),
            },
            signature: main.signature.clone().filter(|s| !s.trim().is_empty()),
            signature_url: main.signature_url.clone().filter(|s| !s.trim().is_empty()),
            sha256: main.sha256.as_ref().map(|s| s.trim().to_ascii_lowercase()),
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

#[derive(Clone, Copy)]
struct AssetSelector<'a> {
    name_patterns: &'a [String],
    os: Option<&'a str>,
    arch: Option<&'a str>,
    format: Option<&'a str>,
}

fn asset_matches(asset: &ManifestAsset, selector: AssetSelector<'_>) -> bool {
    let lower_name = asset.name.to_ascii_lowercase();
    let name_matches = selector
        .name_patterns
        .iter()
        .all(|p| lower_name.contains(&p.to_ascii_lowercase()));
    name_matches
        && field_matches(asset.os.as_deref(), selector.os)
        && field_matches(asset.arch.as_deref(), selector.arch)
        && field_matches(asset.format.as_deref(), selector.format)
}

fn field_matches(actual: Option<&str>, want: Option<&str>) -> bool {
    want.is_none_or(|want| actual.is_some_and(|actual| actual.eq_ignore_ascii_case(want)))
}

fn select_asset<'a>(
    assets: &'a [ManifestAsset],
    selector: AssetSelector<'_>,
) -> Option<&'a ManifestAsset> {
    assets.iter().find(|a| asset_matches(a, selector))
}

#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    schema_version: Option<u32>,
    version: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    notes_url: Option<String>,
    #[serde(default)]
    assets: Vec<ManifestAsset>,
}

impl Manifest {
    fn validate_schema_version(&self) -> Result<()> {
        match self.schema_version.unwrap_or(1) {
            1 => Ok(()),
            other => Err(Error::Parse(format!(
                "unsupported static manifest schema_version `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ManifestAsset {
    name: String,
    url: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    signature_url: Option<String>,
    #[serde(default)]
    os: Option<String>,
    #[serde(default)]
    arch: Option<String>,
    #[serde(default)]
    format: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pats(p: &[&str]) -> Vec<String> {
        p.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn asset_matching_requires_all_substrings_case_insensitively() {
        let asset = ManifestAsset {
            name: "OpenLogi-v0.2.1-macOS-arm64.DMG".to_string(),
            url: "https://dl.example.com/app.dmg".to_string(),
            size: None,
            sha256: None,
            signature: None,
            signature_url: None,
            os: Some("macos".to_string()),
            arch: Some("arm64".to_string()),
            format: Some("dmg".to_string()),
        };
        assert!(asset_matches(
            &asset,
            AssetSelector {
                name_patterns: &pats(&["macos", "arm64", ".dmg"]),
                os: Some("macos"),
                arch: Some("ARM64"),
                format: Some("dmg"),
            }
        ));
        assert!(!asset_matches(
            &asset,
            AssetSelector {
                name_patterns: &pats(&["macos", "arm64", ".dmg"]),
                os: Some("windows"),
                arch: Some("arm64"),
                format: Some("dmg"),
            }
        ));
    }

    #[test]
    fn deserializes_manifest_with_flat_assets_and_unknown_fields() {
        let json = r#"{
            "schema_version": 1,
            "app_id": "org.openlogi.OpenLogi",
            "version": "v0.2.1",
            "channel": "stable",
            "published_at": "2026-06-01T12:00:00Z",
            "notes": "notes here",
            "assets": [
                {
                    "name": "OpenLogi-v0.2.1-macos-arm64.dmg",
                    "url": "https://dl.example.com/OpenLogi-v0.2.1-macos-arm64.dmg",
                    "os": "macos",
                    "arch": "arm64",
                    "format": "dmg",
                    "size": 1234,
                    "sha256": "ABCDEF",
                    "signature": "trusted comment: sig",
                    "minimum_os_version": "13.0"
                }
            ]
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        manifest.validate_schema_version().unwrap();
        assert_eq!(parse_tag(&manifest.version).unwrap().to_string(), "0.2.1");
        let picked = select_asset(
            &manifest.assets,
            AssetSelector {
                name_patterns: &pats(&["macos", "arm64"]),
                os: Some("macos"),
                arch: Some("arm64"),
                format: Some("dmg"),
            },
        )
        .unwrap();
        assert_eq!(picked.size, Some(1234));
        assert_eq!(picked.sha256.as_deref(), Some("ABCDEF"));
        assert!(
            select_asset(
                &manifest.assets,
                AssetSelector {
                    name_patterns: &pats(&[]),
                    os: Some("windows"),
                    arch: None,
                    format: None,
                },
            )
            .is_none()
        );
    }

    #[test]
    fn explicit_format_replaces_default_extension_matcher() {
        let source = StaticManifestSource::new("https://dl.example.com/latest.json").format("dmg");
        assert!(source.asset_patterns.is_empty());
        assert_eq!(source.format.as_deref(), Some("dmg"));
    }

    #[test]
    fn rejects_unknown_schema_version() {
        let manifest = Manifest {
            schema_version: Some(2),
            version: "1.0.0".to_string(),
            notes: None,
            notes_url: None,
            assets: Vec::new(),
        };
        assert!(manifest.validate_schema_version().is_err());
    }
}
