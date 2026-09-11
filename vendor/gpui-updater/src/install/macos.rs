//! macOS install strategy: mount a `.dmg` and swap the `.app` bundle in place.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::Installed;
use crate::error::{Error, Result};

/// Walk up from `exe` to the enclosing `*.app` bundle directory.
pub(crate) fn bundle_root(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
}

pub(crate) fn install(dmg: &Path, app_root: &Path) -> Result<Installed> {
    let parent = app_root
        .parent()
        .ok_or_else(|| Error::Install("install root has no parent directory".to_string()))?;
    let name = app_root
        .file_name()
        .ok_or_else(|| Error::Install("install root has no file name".to_string()))?;

    let mount = Mount::attach(dmg)?;
    let new_app = find_app(&mount.dir)?;

    // Stage a copy on the *same* volume as the target so the final move is a
    // cheap, atomic rename rather than a cross-device copy.
    let staging = parent.join(format!(".{}.new", name.to_string_lossy()));
    let _ = fs::remove_dir_all(&staging);
    ditto(&new_app, &staging)?;

    // Swap: move the old bundle aside, move the new one in, then delete the old.
    // Roll back if the rename-in fails so we never leave the app missing.
    let backup = parent.join(format!(".{}.old", name.to_string_lossy()));
    let _ = fs::remove_dir_all(&backup);
    if app_root.exists() {
        fs::rename(app_root, &backup)
            .map_err(|e| Error::Install(format!("could not move current app aside: {e}")))?;
    }
    if let Err(e) = fs::rename(&staging, app_root) {
        if backup.exists() {
            let _ = fs::rename(&backup, app_root);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(Error::Install(format!(
            "could not move new app into place: {e}"
        )));
    }
    let _ = fs::remove_dir_all(&backup);

    // Restart into the `.app` bundle, not the executable inside it. GPUI's macOS
    // `restart` relaunches via `open <path>`; pointed at a bare Mach-O binary,
    // `open` has no GUI handler for it and falls back to running it inside
    // Terminal. Handing `open` the bundle launches it detached, as a GUI app.
    Ok(Installed {
        restart_path: Some(app_root.to_path_buf()),
    })
}

/// A mounted DMG that detaches itself (and removes its mount point) on drop.
struct Mount {
    dir: PathBuf,
}

impl Mount {
    fn attach(dmg: &Path) -> Result<Self> {
        let dir = unique_temp_dir("gpui-updater-dmg")?;
        let out = Command::new("hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&dir)
            .arg(dmg)
            .output()?;
        if !out.status.success() {
            let _ = fs::remove_dir_all(&dir);
            return Err(Error::Install(format!(
                "hdiutil attach failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(Self { dir })
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .args(["detach", "-force"])
            .arg(&self.dir)
            .output();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Find the single `*.app` at the root of a mounted volume.
fn find_app(mount: &Path) -> Result<PathBuf> {
    for entry in fs::read_dir(mount)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "app") {
            return Ok(path);
        }
    }
    Err(Error::Install(
        "no .app found in the mounted disk image".to_string(),
    ))
}

/// Copy a directory tree preserving metadata and signatures via `ditto`.
fn ditto(src: &Path, dst: &Path) -> Result<()> {
    let out = Command::new("ditto").arg(src).arg(dst).output()?;
    if !out.status.success() {
        return Err(Error::Install(format!(
            "ditto failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::{bundle_root, install, unique_temp_dir};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[test]
    fn finds_enclosing_app_bundle() {
        let exe = Path::new("/Applications/OpenLogi.app/Contents/MacOS/openlogi-gui");
        assert_eq!(
            bundle_root(exe),
            Some(PathBuf::from("/Applications/OpenLogi.app"))
        );
    }

    #[test]
    fn returns_none_when_not_in_a_bundle() {
        assert_eq!(bundle_root(Path::new("/usr/local/bin/tool")), None);
    }

    #[test]
    fn install_restart_path_is_the_app_bundle_root() {
        let work = unique_temp_dir("gpui-updater-test").unwrap();
        let payload = work.join("payload");
        fs::create_dir_all(payload.join("Demo.app/Contents/MacOS")).unwrap();
        fs::write(payload.join("Demo.app/Contents/MacOS/demo"), "new").unwrap();

        let dmg = work.join("update.dmg");
        let status = Command::new("hdiutil")
            .args(["create", "-quiet", "-srcfolder"])
            .arg(&payload)
            .arg(&dmg)
            .status()
            .unwrap();
        assert!(status.success(), "hdiutil create failed");

        let app_root = work.join("Applications/Demo.app");
        fs::create_dir_all(app_root.join("Contents/MacOS")).unwrap();
        fs::write(app_root.join("Contents/MacOS/demo"), "old").unwrap();

        let installed = install(&dmg, &app_root).unwrap();
        assert_eq!(installed.restart_path, Some(app_root.clone()));
        let swapped = fs::read_to_string(app_root.join("Contents/MacOS/demo")).unwrap();
        assert_eq!(swapped, "new");

        let _ = fs::remove_dir_all(&work);
    }
}
