//! Replacing the installed application with a freshly downloaded artifact.
//!
//! The strategy is platform-specific:
//! - **macOS**: mount the `.dmg`, copy the new `.app` onto the same volume as
//!   the current bundle with `ditto` (preserving its existing notarized
//!   signature), then swap it into place. The new bundle carries its own
//!   signature, so nothing is re-signed at runtime.
//! - **Linux**: extract the `.tar.gz` and atomically replace the binary.
//! - **Windows**: see [`windows`] — rename-in-place for a bare `.exe`, or a
//!   staged msiexec handoff for an `.msi` (applied after the app exits, via
//!   the restart path).

use std::path::{Path, PathBuf};

use crate::error::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Outcome of installing an update.
#[derive(Debug, Clone)]
pub struct Installed {
    /// Path to relaunch into the new version, fed to `App::set_restart_path` by
    /// the GPUI integration. This is what GPUI's platform `restart` consumes, so
    /// it is platform-shaped: on macOS the `.app` bundle (relaunched via `open`,
    /// which needs the bundle, not the inner Mach-O — that would open in
    /// Terminal); on Linux the executable itself; on Windows the executable —
    /// or, for a staged MSI, the generated apply script that installs the
    /// package and relaunches the app (GPUI's Windows `restart` spawns the path
    /// only after this process exits, which is the ordering msiexec needs).
    /// `None` means the running location was updated in place with no new path
    /// to restart into.
    pub restart_path: Option<PathBuf>,
}

/// Install a downloaded update artifact over `install_root`.
///
/// - `artifact`: the downloaded file (`.dmg` on macOS, `.tar.gz` on Linux,
///   `.exe` or `.msi` on Windows).
/// - `install_root`: what to replace — on macOS the `.app` bundle directory,
///   on Linux the executable file. Use [`current_install_root`] to derive it.
///
/// # Errors
/// Returns [`Error::Install`](crate::Error::Install) on any failure during
/// mounting, copying, or swapping, and
/// [`Error::UnsupportedPlatform`](crate::Error::UnsupportedPlatform) on
/// platforms without an install strategy.
pub fn install(artifact: &Path, install_root: &Path) -> Result<Installed> {
    #[cfg(target_os = "macos")]
    {
        macos::install(artifact, install_root)
    }
    #[cfg(target_os = "linux")]
    {
        linux::install(artifact, install_root)
    }
    #[cfg(target_os = "windows")]
    {
        windows::install(artifact, install_root)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (artifact, install_root);
        Err(crate::Error::UnsupportedPlatform(std::env::consts::OS))
    }
}

/// Best-effort path of the currently installed application to replace.
///
/// On macOS this walks up from the running executable to the enclosing `.app`
/// bundle; elsewhere it is the running executable itself.
///
/// # Errors
/// Returns [`Error::Io`](crate::Error::Io) if the current executable path
/// cannot be determined.
pub fn current_install_root() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    #[cfg(target_os = "macos")]
    {
        Ok(macos::bundle_root(&exe).unwrap_or(exe))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(exe)
    }
}
