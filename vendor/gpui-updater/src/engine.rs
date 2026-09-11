//! The blocking update orchestrator that ties the pieces together.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use semver::Version;

use crate::error::{Error, Result};
use crate::install::{self, Installed};
use crate::release::Release;
use crate::source::UpdateSource;
use crate::{http, verify};

/// How strictly [`UpdateEngine`] enforces integrity and authenticity checks on
/// a release.
///
/// The default, [`BestEffort`](Verification::BestEffort), verifies whatever the
/// release provides and *skips* a check when its input is absent — so an
/// unsigned release installs with only a checksum, or with nothing. The
/// `Signature`/`Strict` modes instead fail closed: a release that cannot be
/// verified is rejected at [`check`](UpdateEngine::check) time (before it is
/// surfaced as available) and again at [`download`](UpdateEngine::download)
/// time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Verification {
    /// Verify whatever the release and config provide; skip a check when its
    /// input is missing (no checksum, no signature, or no public key). The
    /// backwards-compatible default — note it fails *open*.
    #[default]
    BestEffort,
    /// Skip all integrity and authenticity checks. For tests and local
    /// development only.
    Off,
    /// Require a matching SHA-256. Fails closed when the release publishes no
    /// checksum. A signature is still verified when one is available.
    Checksum,
    /// Require a valid minisign signature: a public key must be configured and
    /// the release must publish a signature. A checksum is still verified when
    /// available, but is not itself required.
    Signature,
    /// Require both a valid minisign signature and a matching SHA-256.
    Strict,
}

impl Verification {
    fn skips_all(self) -> bool {
        matches!(self, Self::Off)
    }

    fn requires_signature(self) -> bool {
        matches!(self, Self::Signature | Self::Strict)
    }

    fn requires_checksum(self) -> bool {
        matches!(self, Self::Checksum | Self::Strict)
    }
}

/// Configuration for an [`UpdateEngine`].
pub struct EngineConfig {
    /// The currently running version, compared against the source's latest.
    pub current_version: Version,
    /// Base64 minisign public key. When set and the release publishes a
    /// `.minisig`, the download is signature-checked in addition to SHA-256.
    pub minisign_public_key: Option<String>,
    /// How strictly downloads are verified. See [`Verification`].
    pub verification: Verification,
    /// Where to install. Defaults to [`install::current_install_root`].
    pub install_root: Option<PathBuf>,
    /// Directory for downloads. Defaults to a fresh temp directory.
    pub download_dir: Option<PathBuf>,
}

impl EngineConfig {
    /// Start a config for the given running version.
    pub fn new(current_version: Version) -> Self {
        Self {
            current_version,
            minisign_public_key: None,
            verification: Verification::default(),
            install_root: None,
            download_dir: None,
        }
    }

    /// Set the base64 minisign public key used to verify signed downloads.
    #[must_use]
    pub fn minisign_public_key(mut self, key: impl Into<String>) -> Self {
        self.minisign_public_key = Some(key.into());
        self
    }

    /// Set how strictly downloads are verified. Defaults to
    /// [`Verification::BestEffort`].
    #[must_use]
    pub fn verification(mut self, verification: Verification) -> Self {
        self.verification = verification;
        self
    }

    /// Fail closed when the policy requires a check the release *metadata*
    /// cannot satisfy (no public key, no advertised signature, no checksum).
    ///
    /// Run at check time so an obviously unverifiable release is not surfaced as
    /// available, and again before a download. This validates presence only: a
    /// release advertising a `signature_url` passes here even if that URL later
    /// fails to fetch or yields an invalid signature — those are caught when the
    /// download is verified.
    fn precheck(&self, release: &Release) -> Result<()> {
        if self.verification.requires_signature() {
            if self.minisign_public_key.is_none() {
                return Err(Error::VerificationRequired(
                    "policy requires a minisign signature, but no public key is configured".into(),
                ));
            }
            if !release_has_signature(release) {
                return Err(Error::VerificationRequired(
                    "policy requires a minisign signature, but the release publishes none".into(),
                ));
            }
        }
        if self.verification.requires_checksum() && non_empty(release.sha256.as_deref()).is_none() {
            return Err(Error::VerificationRequired(
                "policy requires a SHA-256 checksum, but the release publishes none".into(),
            ));
        }
        Ok(())
    }

    /// Override the install location (defaults to the running app/bundle).
    #[must_use]
    pub fn install_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.install_root = Some(path.into());
        self
    }

    /// Override the download directory (defaults to a temp directory).
    #[must_use]
    pub fn download_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.download_dir = Some(path.into());
        self
    }
}

/// Blocking update orchestrator: check → download → verify → install.
///
/// All methods block; the GPUI integration runs them on a background executor.
pub struct UpdateEngine<S: UpdateSource> {
    source: S,
    config: EngineConfig,
}

impl<S: UpdateSource> UpdateEngine<S> {
    /// Build an engine from a source and configuration.
    pub fn new(source: S, config: EngineConfig) -> Self {
        Self { source, config }
    }

    /// The configured current version.
    pub fn current_version(&self) -> &Version {
        &self.config.current_version
    }

    /// Fetch the latest release, returning it only if it is newer than the
    /// current version (otherwise `Ok(None)`).
    ///
    /// # Errors
    /// Propagates source/network/parse errors.
    pub fn check(&self) -> Result<Option<Release>> {
        let latest = self.source.fetch_latest()?;
        if latest.version > self.config.current_version {
            self.config.precheck(&latest)?;
            Ok(Some(latest))
        } else {
            Ok(None)
        }
    }

    /// Download `release.asset`, then verify it according to the configured
    /// [`Verification`] policy. Returns the downloaded artifact path.
    ///
    /// `progress` is called as `(downloaded_bytes, total_bytes)`.
    ///
    /// # Errors
    /// Returns an error on download failure, if verification fails, or if the
    /// policy requires a check the release cannot satisfy
    /// ([`Error::VerificationRequired`]).
    pub fn download(
        &self,
        release: &Release,
        mut progress: impl FnMut(u64, Option<u64>),
    ) -> Result<PathBuf> {
        let dir = match &self.config.download_dir {
            Some(dir) => {
                std::fs::create_dir_all(dir)?;
                dir.clone()
            }
            None => default_download_dir()?,
        };
        let artifact = dir.join(&release.asset.name);
        http::download(&release.asset.url, &[], &artifact, &mut progress)?;
        self.verify_artifact(&artifact, release)?;
        Ok(artifact)
    }

    /// Verify a downloaded `artifact` against `release` under the configured
    /// policy. Fails closed first (so a policy-required-but-missing input is
    /// rejected before the artifact is read), then runs whatever checks the
    /// release does provide.
    fn verify_artifact(&self, artifact: &Path, release: &Release) -> Result<()> {
        let policy = self.config.verification;
        if policy.skips_all() {
            return Ok(());
        }
        // `download` is public and may be called without `check`, so re-run the
        // check-time gate here rather than trusting the caller went through it.
        self.config.precheck(release)?;

        if let Some(expected) = non_empty(release.sha256.as_deref()) {
            verify::verify_sha256(artifact, expected)?;
        }
        if let Some(key) = &self.config.minisign_public_key {
            match resolve_signature(release)? {
                Some(signature) => verify::verify_minisign(artifact, key, &signature)?,
                // `precheck` already accepted a non-empty `signature_url`, but the
                // fetched body turned out blank. Fail closed instead of relying on
                // the verifier to reject an empty signature string.
                None if policy.requires_signature() => {
                    return Err(Error::VerificationRequired(
                        "policy requires a minisign signature, but the published signature is empty"
                            .into(),
                    ));
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Install a downloaded artifact over the configured install root.
    ///
    /// # Errors
    /// Propagates install failures.
    pub fn install(&self, artifact: &Path) -> Result<Installed> {
        let root = match &self.config.install_root {
            Some(root) => root.clone(),
            None => install::current_install_root()?,
        };
        install::install(artifact, &root)
    }

    /// Run the full pipeline. Returns `Ok(None)` when already up to date.
    ///
    /// # Errors
    /// Propagates any check/download/verify/install error.
    pub fn update_now(&self, progress: impl FnMut(u64, Option<u64>)) -> Result<Option<Installed>> {
        let Some(release) = self.check()? else {
            return Ok(None);
        };
        let artifact = self.download(&release, progress)?;
        Ok(Some(self.install(&artifact)?))
    }
}

fn default_download_dir() -> Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("gpui-updater-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Trim `s` and treat a blank string as absent.
fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

/// Whether the release advertises a minisign signature, inline or by URL.
fn release_has_signature(release: &Release) -> bool {
    non_empty(release.signature.as_deref()).is_some()
        || non_empty(release.signature_url.as_deref()).is_some()
}

/// The minisign signature for a release: the inline value if present, otherwise
/// fetched from `signature_url`. `None` when the release advertises neither, or
/// when a fetched signature body is blank.
fn resolve_signature(release: &Release) -> Result<Option<String>> {
    if let Some(signature) = non_empty(release.signature.as_deref()) {
        return Ok(Some(signature.to_string()));
    }
    if let Some(url) = non_empty(release.signature_url.as_deref()) {
        let body = http::get_string(url, &[])?;
        return Ok(non_empty(Some(&body)).map(str::to_string));
    }
    Ok(None)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::release::Asset;

    fn release(
        sha256: Option<&str>,
        signature: Option<&str>,
        signature_url: Option<&str>,
    ) -> Release {
        Release {
            version: Version::new(1, 0, 0),
            notes: None,
            asset: Asset {
                name: "app.dmg".to_string(),
                url: "https://example.test/app.dmg".to_string(),
                size: 0,
            },
            signature: signature.map(str::to_string),
            signature_url: signature_url.map(str::to_string),
            sha256: sha256.map(str::to_string),
        }
    }

    fn config(verification: Verification, key: Option<&str>) -> EngineConfig {
        let mut config = EngineConfig::new(Version::new(0, 1, 0)).verification(verification);
        if let Some(key) = key {
            config = config.minisign_public_key(key);
        }
        config
    }

    #[test]
    fn best_effort_accepts_unverifiable_release() {
        // No checksum, no signature, no key: the default fails open.
        assert!(
            config(Verification::BestEffort, None)
                .precheck(&release(None, None, None))
                .is_ok()
        );
    }

    #[test]
    fn checksum_policy_requires_published_checksum() {
        let cfg = config(Verification::Checksum, None);
        assert!(matches!(
            cfg.precheck(&release(None, None, None)),
            Err(Error::VerificationRequired(_))
        ));
        assert!(cfg.precheck(&release(Some("abc"), None, None)).is_ok());
    }

    #[test]
    fn signature_policy_requires_public_key() {
        // Key missing, even though the release advertises a signature.
        let cfg = config(Verification::Signature, None);
        assert!(matches!(
            cfg.precheck(&release(
                None,
                None,
                Some("https://example.test/app.dmg.minisig")
            )),
            Err(Error::VerificationRequired(_))
        ));
    }

    #[test]
    fn signature_policy_requires_advertised_signature() {
        let cfg = config(Verification::Signature, Some("pubkey"));
        assert!(matches!(
            cfg.precheck(&release(Some("abc"), None, None)),
            Err(Error::VerificationRequired(_))
        ));
        // A blank signature reference is treated as absent.
        assert!(matches!(
            cfg.precheck(&release(None, Some("  "), Some(""))),
            Err(Error::VerificationRequired(_))
        ));
        assert!(
            cfg.precheck(&release(None, Some("trusted comment: …"), None))
                .is_ok()
        );
        assert!(
            cfg.precheck(&release(
                None,
                None,
                Some("https://example.test/app.dmg.minisig")
            ))
            .is_ok()
        );
    }

    #[test]
    fn strict_policy_requires_both_signature_and_checksum() {
        let cfg = config(Verification::Strict, Some("pubkey"));
        // Signature present but checksum missing.
        assert!(matches!(
            cfg.precheck(&release(None, Some("sig"), None)),
            Err(Error::VerificationRequired(_))
        ));
        assert!(
            cfg.precheck(&release(Some("abc"), Some("sig"), None))
                .is_ok()
        );
    }

    #[test]
    fn off_policy_skips_verification_entirely() {
        // `verify_artifact` returns Ok without touching the (nonexistent) file
        // even though the release claims a checksum.
        let engine = UpdateEngine::new(PanicSource, config(Verification::Off, None));
        let path = Path::new("/nonexistent/app.dmg");
        assert!(
            engine
                .verify_artifact(path, &release(Some("abc"), None, None))
                .is_ok()
        );
    }

    #[test]
    fn blank_checksum_is_treated_as_absent_not_verified() {
        // A `"sha256": ""` (or whitespace) field reaches us as Some("") from the
        // manifest source. It must be skipped like a missing checksum, not run
        // through verify_sha256 against a blank expected hash — which would read
        // the (here nonexistent) artifact and spuriously fail every download.
        let engine = UpdateEngine::new(PanicSource, config(Verification::BestEffort, None));
        let path = Path::new("/nonexistent/app.dmg");
        for blank in ["", "   "] {
            assert!(
                engine
                    .verify_artifact(path, &release(Some(blank), None, None))
                    .is_ok(),
                "blank sha256 {blank:?} should be treated as absent"
            );
        }
    }

    /// A source whose `fetch_latest` must never be called: the policy tests
    /// exercise verification, not fetching.
    struct PanicSource;

    impl UpdateSource for PanicSource {
        fn fetch_latest(&self) -> Result<Release> {
            unreachable!("policy tests do not fetch")
        }
    }
}
