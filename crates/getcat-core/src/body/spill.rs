//! 大响应落盘：临时文件的创建、守卫（drop 即删除）与应用退出清理。

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
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

/// 临时文件守卫：drop 时删除文件。`BodyStore::Spilled` 通过 `Arc<SpillFile>` 共享，
/// 最后一个持有者释放时文件随之消失。
#[derive(Debug)]
pub struct SpillFile {
    path: PathBuf,
}

impl SpillFile {
    /// 在会话目录里创建一个新的空文件，返回守卫与可写句柄（调用方写完后 drop 句柄即可）。
    pub fn create() -> io::Result<(SpillFile, fs::File)> {
        let dir = session_dir();
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!(
            "{:06}.body",
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
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
}
