//! Error types for the updater.

/// Errors produced while checking for, downloading, verifying, or installing an
/// update.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A network request failed or returned a non-success status.
    #[error("http error: {0}")]
    Http(String),

    /// Release metadata could not be parsed (bad JSON, missing fields, or an
    /// unparseable version tag).
    #[error("failed to parse release metadata: {0}")]
    Parse(String),

    /// No release asset matched the running platform.
    #[error("no release asset matched the current platform ({target_os}/{target_arch})")]
    NoMatchingAsset {
        /// `std::env::consts::OS` of the running build.
        target_os: &'static str,
        /// `std::env::consts::ARCH` of the running build.
        target_arch: &'static str,
    },

    /// The downloaded file's SHA-256 did not match the expected checksum.
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// The checksum published alongside the release.
        expected: String,
        /// The checksum computed from the downloaded bytes.
        actual: String,
    },

    /// Minisign signature verification failed.
    #[error("signature verification failed: {0}")]
    Signature(String),

    /// The configured [`Verification`](crate::Verification) policy required a
    /// check that could not run because its input was missing — no public key,
    /// no published signature, or no published checksum.
    #[error("verification policy not satisfied: {0}")]
    VerificationRequired(String),

    /// Installing the downloaded update failed.
    #[error("install failed: {0}")]
    Install(String),

    /// The current platform has no install strategy yet.
    #[error("update install is not supported on this platform ({0})")]
    UnsupportedPlatform(&'static str),

    /// An underlying I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Convenience alias for results returned by this crate.
pub type Result<T> = std::result::Result<T, Error>;
