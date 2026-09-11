//! Integrity and authenticity checks for downloaded artifacts.

use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Compute the lowercase hex SHA-256 of a file, streaming it in chunks.
///
/// # Errors
/// Returns [`Error::Io`] if the file cannot be read.
pub fn sha256_hex(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let mut out = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

/// Verify a file against an expected hex SHA-256 (case-insensitive).
///
/// # Errors
/// Returns [`Error::ChecksumMismatch`] if the digests differ.
pub fn verify_sha256(path: &Path, expected_hex: &str) -> Result<()> {
    let actual = sha256_hex(path)?;
    if actual.eq_ignore_ascii_case(expected_hex.trim()) {
        Ok(())
    } else {
        Err(Error::ChecksumMismatch {
            expected: expected_hex.trim().to_ascii_lowercase(),
            actual,
        })
    }
}

/// Verify a file against a detached [minisign] signature.
///
/// - `public_key`: the base64 minisign public key (the second line of a
///   `minisign.pub`, or the whole single-line key).
/// - `signature`: the contents of the `.minisig` file.
///
/// [minisign]: https://jedisct1.github.io/minisign/
///
/// # Errors
/// Returns [`Error::Signature`] if the key/signature cannot be decoded or the
/// signature does not match the file's contents.
pub fn verify_minisign(path: &Path, public_key: &str, signature: &str) -> Result<()> {
    let pk = minisign_verify::PublicKey::from_base64(public_key.trim())
        .map_err(|e| Error::Signature(format!("invalid public key: {e}")))?;
    let sig = minisign_verify::Signature::decode(signature)
        .map_err(|e| Error::Signature(format!("invalid signature: {e}")))?;
    let data = std::fs::read(path)?;
    pk.verify(&data, &sig, false)
        .map_err(|e| Error::Signature(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(contents: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("gpui-updater-test-{}.bin", std::process::id()));
        let mut f = File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        path
    }

    #[test]
    fn sha256_matches_known_vector() {
        // SHA-256("abc")
        let path = temp_file(b"abc");
        let got = sha256_hex(&path).unwrap();
        assert_eq!(
            got,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(verify_sha256(&path, &got.to_uppercase()).is_ok());
        assert!(verify_sha256(&path, "deadbeef").is_err());
        let _ = std::fs::remove_file(&path);
    }
}
