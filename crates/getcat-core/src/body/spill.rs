//! 大响应落盘：临时文件的创建、守卫（drop 即删除）与应用退出清理。

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

/// 落盘响应保留在内存中的前缀长度（用于 C 档预览与内容类型嗅探）。
pub const HEAD_BYTES: usize = 1024 * 1024;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// 本进程专用的临时目录：`<系统临时目录>/getcat-<pid>`。按进程隔离，多个实例互不影响。
pub fn session_dir() -> PathBuf {
    std::env::temp_dir().join(format!("getcat-{}", std::process::id()))
}

/// 应用退出时调用：整目录删除。正常情况下目录已空（守卫逐个删过），这里兜底异常退出前未 drop 的文件。
pub fn cleanup_session_dir() {
    cleanup_dir(&session_dir());
}

fn cleanup_dir(dir: &Path) {
    if let Err(e) = fs::remove_dir_all(dir)
        && e.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(dir = %dir.display(), error = %e, "failed to remove spill directory");
    }
}

/// 启动清扫时，其它进程遗留的会话目录被视为过期的年龄。
pub const STALE_SESSION_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// 删除系统临时目录下其它进程遗留的 `getcat-<pid>` 目录（上次崩溃 / 被 kill 时守卫与 `on_app_quit` 都没机会跑）。
/// 只删 mtime 早于 `max_age` 的目录：刚启动的另一个实例目录很新，不会被误删。不做 pid 存活探测
/// （没有无依赖的跨平台做法，且 pid 会被复用）。返回删除的目录数。
pub fn sweep_stale_session_dirs(max_age: Duration) -> usize {
    sweep_dir(&std::env::temp_dir(), max_age)
}

/// `sweep_stale_session_dirs` 的可测版本：对给定根目录清扫。
pub(crate) fn sweep_dir(temp: &Path, max_age: Duration) -> usize {
    let own = format!("getcat-{}", std::process::id());
    let entries = match fs::read_dir(temp) {
        Ok(entries) => entries,
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                tracing::warn!(dir = %temp.display(), error = %e, "cannot scan temp dir for stale spill dirs");
            }
            return 0;
        }
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // 只认 getcat-<纯数字 pid>；本进程的目录永远不碰
        let is_session_dir = name
            .strip_prefix("getcat-")
            .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()));
        if !is_session_dir || name == own {
            continue;
        }
        // 读不到 metadata（条目刚被别的进程删掉、或权限不足）：跳过，不是错误
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(e) => {
                tracing::debug!(dir = %entry.path().display(), error = %e, "cannot stat candidate spill directory");
                continue;
            }
        };
        if !meta.is_dir() {
            continue;
        }
        let stale = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age >= max_age);
        if !stale {
            continue;
        }
        let path = entry.path();
        match fs::remove_dir_all(&path) {
            Ok(()) => {
                removed += 1;
                tracing::info!(dir = %path.display(), "removed stale spill directory");
            }
            // 共享 /tmp 上别的用户留下的 getcat-<pid> 我们本来就删不掉：降为 debug，
            // 其余错误（磁盘故障、目录被占用）仍然值得 warn
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                tracing::debug!(dir = %path.display(), error = %e, "no permission to remove stale spill directory")
            }
            Err(e) => {
                tracing::warn!(dir = %path.display(), error = %e, "failed to remove stale spill directory")
            }
        }
    }
    removed
}

/// 会话目录创建/复用时的权限。Unix 下限制为仅属主可读写执行，避免同机其他用户
/// 读到落盘的响应体。
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;
/// 落盘文件创建时的权限：仅属主可读写。
#[cfg(unix)]
const FILE_MODE: u32 = 0o600;

/// 确保会话目录存在且（Unix 下）权限收紧到 0o700。
///
/// 取舍：理想情况下应校验目录属主是否为当前用户（防止其他用户抢先在共享临时目录下
/// 创建同名目录做符号链接攻击），但 `MetadataExt::uid()` 需要与 `libc::getuid()` 比较，
/// 而本项目不引入 `libc` 依赖，`std::env::var("UID")` 又不可靠（很多 shell 不导出该变量、
/// 也可能被伪造）。因此简化为：目录已存在时只收紧权限位到 0o700，不做属主检查——
/// 会话目录名带有本进程 pid，被抢占的窗口极小，且收紧权限后即便目录非本进程所建，
/// 其他用户也无法继续读取新写入的文件。
fn ensure_session_dir() -> io::Result<PathBuf> {
    let dir = session_dir();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        match fs::DirBuilder::new()
            .recursive(true)
            .mode(DIR_MODE)
            .create(&dir)
        {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let meta = fs::metadata(&dir)?;
        if meta.permissions().mode() & 0o777 != DIR_MODE {
            fs::set_permissions(&dir, fs::Permissions::from_mode(DIR_MODE))?;
        }
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

/// 临时文件守卫：drop 时删除文件。`BodyStore::Spilled` 通过 `Arc<SpillFile>` 共享，
/// 最后一个持有者释放时文件随之消失。
#[derive(Debug)]
pub struct SpillFile {
    path: PathBuf,
}

impl SpillFile {
    /// 在会话目录里创建一个新的空文件，返回守卫与可写句柄（调用方写完后 drop 句柄即可）。
    pub fn create() -> io::Result<(SpillFile, fs::File)> {
        let dir = ensure_session_dir()?;
        let path = dir.join(format!(
            "{:06}.body",
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut open_options = fs::OpenOptions::new();
        open_options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open_options.mode(FILE_MODE);
        }
        let file = open_options.open(&path)?;
        Ok((SpillFile { path }, file))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SpillFile {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(&self.path)
            && e.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), error = %e, "failed to remove spill file");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn spill_file_is_removed_on_drop() {
        let (guard, mut file) = SpillFile::create().unwrap();
        file.write_all(b"abc").unwrap();
        let path = guard.path().to_path_buf();
        assert!(path.starts_with(session_dir()));
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        drop(file);
        drop(guard);
        assert!(!path.exists());
    }

    #[test]
    fn session_dir_is_per_process() {
        assert!(session_dir().ends_with(format!("getcat-{}", std::process::id())));
    }

    #[test]
    fn cleanup_removes_a_whole_directory() {
        // 不直接对 session_dir 做 cleanup：并行运行的 http 测试可能正在往里写
        let dir = std::env::temp_dir().join(format!("getcat-cleanup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.body"), b"x").unwrap();
        cleanup_dir(&dir);
        assert!(!dir.exists());
        cleanup_dir(&dir); // 不存在也不报错
    }

    #[cfg(unix)]
    #[test]
    fn session_dir_and_spill_file_have_restrictive_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (guard, _file) = SpillFile::create().unwrap();
        let dir_mode = std::fs::metadata(session_dir())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o777, 0o700, "session dir must be 0700");
        let file_mode = std::fs::metadata(guard.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(file_mode & 0o777, 0o600, "spill file must be 0600");
    }

    fn fake_temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("getcat-sweep-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn sweep_removes_only_stale_foreign_session_dirs() {
        let root = fake_temp_root("stale");
        let foreign = root.join("getcat-424242");
        fs::create_dir(&foreign).unwrap();
        fs::write(foreign.join("000001.body"), b"x").unwrap();
        let own = root.join(format!("getcat-{}", std::process::id()));
        fs::create_dir(&own).unwrap();
        let unrelated = root.join("getcat-filebody-424242");
        fs::create_dir(&unrelated).unwrap();
        let file_not_dir = root.join("getcat-777");
        fs::write(&file_not_dir, b"not a dir").unwrap();

        // 年龄阈值 0：任何外来目录都算过期；本进程的、名字不合格的、不是目录的都不碰
        assert_eq!(sweep_dir(&root, Duration::ZERO), 1);
        assert!(!foreign.exists());
        assert!(own.exists());
        assert!(unrelated.exists());
        assert!(file_not_dir.exists());
        // 第二次没有可删的
        assert_eq!(sweep_dir(&root, Duration::ZERO), 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sweep_keeps_recent_dirs_and_tolerates_missing_root() {
        let root = fake_temp_root("recent");
        let fresh = root.join("getcat-515151");
        fs::create_dir(&fresh).unwrap();
        assert_eq!(sweep_dir(&root, STALE_SESSION_AGE), 0);
        assert!(fresh.exists());
        assert_eq!(sweep_dir(&root.join("does-not-exist"), Duration::ZERO), 0);
        let _ = fs::remove_dir_all(&root);
    }
}
