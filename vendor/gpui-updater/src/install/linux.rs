//! Linux install strategy: extract a `.tar.gz` and replace the binary.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::Installed;
use crate::error::{Error, Result};

pub(crate) fn install(tarball: &Path, install_root: &Path) -> Result<Installed> {
    let parent = install_root
        .parent()
        .ok_or_else(|| Error::Install("install root has no parent directory".to_string()))?;
    let name = install_root
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| Error::Install("install root has no file name".to_string()))?;

    // Stage on the same volume as the target so the final rename is atomic.
    let staging = unique_dir(parent, "gpui-updater-stage")?;
    let extract = extract_tar_gz(tarball, &staging);
    let new_bin = extract.and_then(|()| find_binary(&staging, name));
    let new_bin = match new_bin {
        Ok(p) => p,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };

    // Replacing a running executable by rename is permitted on Linux (the old
    // inode stays live for the running process).
    let tmp = parent.join(format!(".{name}.new"));
    let result = (|| {
        fs::copy(&new_bin, &tmp)?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
        fs::rename(&tmp, install_root)
            .map_err(|e| Error::Install(format!("could not replace binary: {e}")))
    })();
    let _ = fs::remove_dir_all(&staging);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;

    Ok(Installed {
        restart_path: Some(install_root.to_path_buf()),
    })
}

fn extract_tar_gz(tarball: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let out = Command::new("tar")
        .arg("-xzf")
        .arg(tarball)
        .arg("-C")
        .arg(dest)
        .output()?;
    if !out.status.success() {
        return Err(Error::Install(format!(
            "tar extract failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

/// Find the new executable: prefer one named `wanted`, else the first regular
/// file with an executable bit set (searched recursively).
fn find_binary(dir: &Path, wanted: &str) -> Result<PathBuf> {
    let mut fallback = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !path.is_file() {
                continue;
            }
            if path.file_name().and_then(OsStr::to_str) == Some(wanted) {
                return Ok(path);
            }
            let is_exec = fs::metadata(&path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
            if is_exec && fallback.is_none() {
                fallback = Some(path);
            }
        }
    }
    fallback.ok_or_else(|| Error::Install("no executable found in archive".to_string()))
}

fn unique_dir(parent: &Path, prefix: &str) -> Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = parent.join(format!(".{prefix}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}
