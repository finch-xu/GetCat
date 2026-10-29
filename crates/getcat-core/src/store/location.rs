//! 数据目录的独立引导配置与显式复制；不会把位置设置写进可切换的 settings.json。

use super::{
    DRAFTS_DIR, Layout, REQUESTS_DIR, SETTINGS_FILE, StoreError, VARIABLES_FILE, WORKSPACE_FILE,
    write_atomic,
};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

const CONFIG_FILE: &str = "storage-location.json";
const PORTABLE_CONFIG_FILE: &str = "getcat-storage.json";
const PORTABLE_DATA_DIR: &str = "GetCatData";
const MIGRATION_MARKER: &str = ".getcat-migration-in-progress";
const ROOT_FILES: [&str; 3] = [WORKSPACE_FILE, SETTINGS_FILE, VARIABLES_FILE];

#[derive(Debug, thiserror::Error)]
pub enum LocationError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Invalid storage location configuration: {0}")]
    Config(String),
    #[error("The system data directory contains no data")]
    SourceEmpty,
    #[error("The target already contains GetCat data")]
    TargetNotEmpty,
    #[error(
        "This directory contains an incomplete migration; keep its files and select another directory"
    )]
    IncompleteMigration,
    #[error("The source and target directories are the same")]
    SameDirectory,
    #[error("The source and target directories must not contain each other")]
    OverlappingDirectories,
    #[error("Cannot copy a symbolic link or a special file: {0}")]
    UnsupportedFile(PathBuf),
}

#[derive(Serialize, Deserialize)]
struct LocationConfig {
    version: u8,
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DataLocation {
    pub system_root: PathBuf,
    /// 仅 Windows 免安装版传入 exe 的父目录；其它版本为 None。
    pub portable_dir: Option<PathBuf>,
}

impl DataLocation {
    pub fn new(system_root: PathBuf, portable_dir: Option<PathBuf>) -> Self {
        Self {
            system_root,
            portable_dir,
        }
    }

    fn config_path(&self) -> PathBuf {
        self.portable_dir.as_ref().map_or_else(
            || self.system_root.join(CONFIG_FILE),
            |dir| dir.join(PORTABLE_CONFIG_FILE),
        )
    }

    pub fn default_root(&self) -> PathBuf {
        self.portable_dir.as_ref().map_or_else(
            || self.system_root.clone(),
            |dir| dir.join(PORTABLE_DATA_DIR),
        )
    }

    /// 不在失败时悄悄换目录：损坏的引导配置也会返回错误，由应用显示存储不可用。
    pub fn resolve(&self) -> Result<PathBuf, LocationError> {
        match fs::read(self.config_path()) {
            Ok(bytes) => {
                let config: LocationConfig = serde_json::from_slice(&bytes)
                    .map_err(|e| LocationError::Config(e.to_string()))?;
                if config.version != 1 {
                    return Err(LocationError::Config(format!(
                        "unsupported version {}",
                        config.version
                    )));
                }
                if config.root.is_absolute() {
                    return existing_root(config.root);
                }
                // 便携默认目录只存相对名字，移动 exe 文件夹后仍可找到。
                if config.root == Path::new(PORTABLE_DATA_DIR)
                    && let Some(dir) = &self.portable_dir
                {
                    return existing_root(dir.join(config.root));
                }
                Err(LocationError::Config(
                    "expected an absolute data path".into(),
                ))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let default = self.default_root();
                ensure_complete(&default)?;
                if self.portable_dir.is_some()
                    && !has_data(&default)?
                    && has_data(&self.system_root)?
                {
                    ensure_complete(&self.system_root)?;
                    ensure_layout_directories(&self.system_root)?;
                    Ok(self.system_root.clone())
                } else {
                    ensure_layout_directories(&default)?;
                    Ok(default)
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    /// 只保存下一次启动使用的位置；调用方保留当前 Store，不热切换。
    pub fn select(&self, root: &Path) -> Result<PathBuf, LocationError> {
        if !root.is_absolute() {
            return Err(LocationError::Config(
                "expected an absolute data path".into(),
            ));
        }
        ensure_complete(root)?;
        ensure_layout_directories(root)?;
        Layout::new(root.to_path_buf()).ensure()?;
        let root = fs::canonicalize(root)?;
        let configured_root = if self.portable_dir.is_some()
            && fs::canonicalize(self.default_root()).is_ok_and(|default| default == root)
        {
            PathBuf::from(PORTABLE_DATA_DIR)
        } else {
            root.clone()
        };
        let config = LocationConfig {
            version: 1,
            root: configured_root,
        };
        let mut bytes =
            serde_json::to_vec_pretty(&config).map_err(|e| LocationError::Config(e.to_string()))?;
        bytes.push(b'\n');
        let path = self.config_path();
        fs::create_dir_all(path.parent().expect("configuration has parent"))?;
        write_atomic(&path, &bytes)?;
        Ok(root)
    }

    /// 显式记住默认位置，避免便携版重置后又被旧系统数据吸引回去。
    pub fn reset(&self) -> Result<PathBuf, LocationError> {
        self.select(&self.default_root())
    }

    pub fn migration_available(&self, target: &Path) -> Result<(), LocationError> {
        self.migration_files(target).map(|_| ())
    }

    fn migration_files(&self, target: &Path) -> Result<Vec<PathBuf>, LocationError> {
        ensure_complete(&self.system_root)?;
        ensure_complete(target)?;
        let files = data_files(&self.system_root)?;
        if files.is_empty() {
            return Err(LocationError::SourceEmpty);
        }
        let source = fs::canonicalize(&self.system_root)?;
        validate_target(&source, target)?;
        Ok(files)
    }

    /// 固定从系统目录复制。先全部暂存，提交不覆盖；失败仅撤回本次创建的文件。
    /// 引导配置、未知文件和子目录不会被复制，来源始终保留。
    pub fn migrate_from_system(&self, target: &Path) -> Result<usize, LocationError> {
        self.capture_migration(target)?.copy_to(target)
    }

    /// 在系统目录的写入队列内调用：只打开当时的文件，不在写入线程上复制大文件。
    pub fn capture_migration(&self, target: &Path) -> Result<MigrationSnapshot, LocationError> {
        let paths = self.migration_files(target)?;
        let source_root = fs::canonicalize(&self.system_root)?;
        let mut files = Vec::new();
        for relative in paths {
            let path = self.system_root.join(&relative);
            ensure_regular(&path, false)?;
            files.push((relative, fs::File::open(path)?));
        }
        Ok(MigrationSnapshot { source_root, files })
    }
}

/// GetCat 的保存使用原子替换，打开的旧文件在后续保存/删除后仍保持捕获时的内容。
/// 复制在写入队列外执行，避免长时间阻塞用户保存与退出时的 flush。
pub struct MigrationSnapshot {
    source_root: PathBuf,
    files: Vec<(PathBuf, fs::File)>,
}

impl MigrationSnapshot {
    pub fn copy_to(self, target: &Path) -> Result<usize, LocationError> {
        validate_target(&self.source_root, target)?;
        Layout::new(target.to_path_buf()).ensure()?;
        // 标记先于任何数据提交；异常退出后启动必须拒绝把半成品当成完整工作区。
        let marker_path = target.join(MIGRATION_MARKER);
        let marker = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker_path)?;
        marker.sync_all()?;
        let mut created = Vec::new();
        let result = (|| {
            let staging = tempfile::tempdir_in(target)?;
            let mut staged = Vec::new();
            for (relative, mut source) in self.files {
                let mut tmp = tempfile::NamedTempFile::new_in(staging.path())?;
                io::copy(&mut source, &mut tmp)?;
                tmp.as_file().sync_all()?;
                staged.push((tmp, target.join(relative)));
            }
            for (tmp, path) in staged {
                tmp.persist_noclobber(&path).map_err(|e| e.error)?;
                created.push(path);
            }
            Ok::<_, LocationError>(created.len())
        })();
        if result.is_err() {
            let mut rollback_complete = true;
            for path in &created {
                if fs::remove_file(path).is_err() {
                    rollback_complete = false;
                }
            }
            if !rollback_complete {
                // 不能确认回滚完成时保留标记，阻止下一次启动写入不完整数据。
                return result;
            }
        }
        drop(marker); // Windows 删除前释放句柄。
        fs::remove_file(marker_path)?;
        result
    }
}

fn validate_target(source: &Path, target: &Path) -> Result<(), LocationError> {
    ensure_complete(target)?;
    let target = fs::canonicalize(target)?;
    if source == target {
        return Err(LocationError::SameDirectory);
    }
    if source.starts_with(&target) || target.starts_with(source) {
        return Err(LocationError::OverlappingDirectories);
    }
    ensure_layout_directories(&target)?;
    if has_data(&target)? {
        return Err(LocationError::TargetNotEmpty);
    }
    Ok(())
}

/// 显式选中的目录在选择时已经创建；之后消失代表原工作区不可用，不能重建为空工作区。
fn existing_root(root: PathBuf) -> Result<PathBuf, LocationError> {
    if !fs::metadata(&root)?.is_dir() {
        return Err(
            io::Error::new(io::ErrorKind::NotADirectory, root.display().to_string()).into(),
        );
    }
    ensure_complete(&root)?;
    ensure_layout_directories(&root)?;
    Ok(root)
}

fn ensure_complete(root: &Path) -> Result<(), LocationError> {
    match fs::symlink_metadata(root.join(MIGRATION_MARKER)) {
        Ok(_) => Err(LocationError::IncompleteMigration),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn ensure_layout_directories(root: &Path) -> Result<(), LocationError> {
    for name in [REQUESTS_DIR, DRAFTS_DIR] {
        ensure_regular(&root.join(name), true)?;
    }
    Ok(())
}

/// 旧用户检测保守处理：已知文件（包括损坏文件）或非空请求/草稿目录都算数据。
fn has_data(root: &Path) -> io::Result<bool> {
    for name in ROOT_FILES {
        match fs::symlink_metadata(root.join(name)) {
            Ok(_) => return Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    for name in [REQUESTS_DIR, DRAFTS_DIR] {
        let path = root.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.is_dir() => return Ok(true),
            Ok(_) => {
                if fs::read_dir(path)?.next().transpose()?.is_some() {
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}

fn ensure_regular(path: &Path, directory: bool) -> Result<(), LocationError> {
    match fs::symlink_metadata(path) {
        Ok(meta)
            if if directory {
                meta.is_dir()
            } else {
                meta.is_file()
            } =>
        {
            Ok(())
        }
        Ok(_) => Err(LocationError::UnsupportedFile(path.into())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn data_files(root: &Path) -> Result<Vec<PathBuf>, LocationError> {
    let mut files = Vec::new();
    for name in ROOT_FILES {
        let path = root.join(name);
        ensure_regular(&path, false)?;
        if path.is_file() {
            files.push(PathBuf::from(name));
        }
    }
    for name in [REQUESTS_DIR, DRAFTS_DIR] {
        let path = root.join(name);
        ensure_regular(&path, true)?;
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            if entry.path().extension().is_some_and(|ext| ext == "json") {
                ensure_regular(&entry.path(), false)?;
                files.push(Path::new(name).join(entry.file_name()));
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn location(dir: &Path, portable: bool) -> DataLocation {
        let exe_dir = dir.join("app");
        fs::create_dir_all(&exe_dir).unwrap();
        DataLocation::new(dir.join("system"), portable.then_some(exe_dir))
    }

    fn data(root: &Path, file: &str, bytes: &[u8]) {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn portable_new_users_get_exe_sibling_and_old_users_keep_system_data() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), true);
        assert_eq!(loc.resolve().unwrap(), dir.path().join("app/GetCatData"));
        data(&loc.system_root, "requests/old.json", b"old");
        assert_eq!(loc.resolve().unwrap(), loc.system_root);
        data(
            &dir.path().join("app/GetCatData"),
            "workspace.json",
            b"portable",
        );
        assert_eq!(loc.resolve().unwrap(), dir.path().join("app/GetCatData"));
    }

    #[test]
    fn installed_users_stay_in_system_directory() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        assert_eq!(loc.resolve().unwrap(), loc.system_root);
    }

    #[test]
    fn selection_survives_restart_and_reset_preserves_all_data() {
        let dir = tempfile::tempdir().unwrap();
        for portable in [false, true] {
            let loc = location(&dir.path().join(portable.to_string()), portable);
            data(&loc.system_root, "workspace.json", b"old");
            let target = dir.path().join(format!("custom-{portable}"));
            let selected = loc.select(&target).unwrap();
            assert_eq!(loc.resolve().unwrap(), selected);
            assert!(!target.join("workspace.json").exists());
            let reset = loc.reset().unwrap();
            assert_eq!(reset, fs::canonicalize(loc.default_root()).unwrap());
            assert_eq!(fs::canonicalize(loc.resolve().unwrap()).unwrap(), reset);
            assert_eq!(
                fs::read(loc.system_root.join("workspace.json")).unwrap(),
                b"old"
            );
        }
    }

    #[test]
    fn portable_default_moves_with_executable_folder() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), true);
        loc.reset().unwrap();
        data(&loc.default_root(), "settings.json", b"settings");
        fs::rename(dir.path().join("app"), dir.path().join("moved")).unwrap();
        let moved = DataLocation::new(loc.system_root.clone(), Some(dir.path().join("moved")));
        assert_eq!(
            moved.resolve().unwrap(),
            dir.path().join("moved/GetCatData")
        );
    }

    #[test]
    fn invalid_configuration_and_invalid_target_do_not_silently_change_location() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        let target = dir.path().join("target");
        loc.select(&target).unwrap();
        let not_dir = dir.path().join("file");
        fs::write(&not_dir, b"x").unwrap();
        assert!(loc.select(&not_dir).is_err());
        assert_eq!(loc.resolve().unwrap(), fs::canonicalize(&target).unwrap());
        fs::write(loc.system_root.join("storage-location.json"), b"{bad").unwrap();
        assert!(loc.resolve().is_err());
    }

    #[test]
    fn migration_always_copies_system_data_after_repeated_selections() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        data(&loc.system_root, "requests/old.json", b"original");
        data(&loc.system_root, "variables.json", b"secrets");
        data(&loc.system_root, "notes.txt", b"unrelated");
        loc.select(&dir.path().join("empty-a")).unwrap();
        let target = loc.select(&dir.path().join("empty-b")).unwrap();
        assert_eq!(loc.migrate_from_system(&target).unwrap(), 2);
        assert_eq!(
            fs::read(target.join("requests/old.json")).unwrap(),
            b"original"
        );
        assert_eq!(fs::read(target.join("variables.json")).unwrap(), b"secrets");
        assert!(!target.join("storage-location.json").exists());
        assert!(!target.join("notes.txt").exists());
        assert!(loc.system_root.join("variables.json").is_file());
        assert_eq!(loc.resolve().unwrap(), target);
    }

    #[test]
    fn migration_refuses_existing_data_same_directory_and_nested_directories() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        data(&loc.system_root, "workspace.json", b"source");
        assert!(loc.migrate_from_system(&loc.system_root).is_err());
        assert!(
            loc.migrate_from_system(&loc.system_root.join("nested"))
                .is_err()
        );
        assert!(loc.migrate_from_system(dir.path()).is_err());
        let target = dir.path().join("target");
        data(&target, "workspace.json", b"keep");
        assert!(loc.migrate_from_system(&target).is_err());
        assert_eq!(fs::read(target.join("workspace.json")).unwrap(), b"keep");
    }

    #[test]
    fn missing_or_empty_source_is_not_a_successful_migration() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        assert!(loc.migrate_from_system(&dir.path().join("target")).is_err());
        fs::create_dir_all(loc.system_root.join("requests")).unwrap();
        assert!(loc.migrate_from_system(&dir.path().join("target")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn migration_rejects_symlinked_source_and_destination_entries() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        data(&loc.system_root, "workspace.json", b"source");
        let outside = dir.path().join("outside");
        data(&outside, "private.json", b"private");
        symlink(
            outside.join("private.json"),
            loc.system_root.join("variables.json"),
        )
        .unwrap();
        let target = dir.path().join("target");
        assert!(loc.migrate_from_system(&target).is_err());
        assert!(!target.join("workspace.json").exists());
        fs::remove_file(loc.system_root.join("variables.json")).unwrap();
        fs::create_dir_all(&target).unwrap();
        symlink(&outside, target.join("requests")).unwrap();
        assert!(loc.migrate_from_system(&target).is_err());
        assert!(!outside.join("workspace.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn failed_staging_copies_nothing_and_retry_keeps_private_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        data(&loc.system_root, "workspace.json", b"workspace");
        data(&loc.system_root, "settings.json", b"settings");
        let settings = loc.system_root.join("settings.json");
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::File::open(&settings).is_ok() {
            fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
            return; // 提权运行无法模拟不可读文件。
        }
        let target = loc.select(&dir.path().join("target")).unwrap();
        let failed = loc.migrate_from_system(&target);
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(failed.is_err());
        assert!(!target.join("workspace.json").exists());
        assert!(!target.join("settings.json").exists());
        assert_eq!(loc.migrate_from_system(&target).unwrap(), 2);
        assert_eq!(
            fs::metadata(target.join("settings.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(fs::read(settings).unwrap(), b"settings");
    }

    #[test]
    fn invalid_bootstrap_version_and_relative_paths_are_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        fs::create_dir_all(&loc.system_root).unwrap();
        for text in [
            r#"{"version":2,"root":"/future"}"#,
            r#"{"version":1,"root":"../elsewhere"}"#,
        ] {
            fs::write(loc.system_root.join("storage-location.json"), text).unwrap();
            assert!(matches!(loc.resolve(), Err(LocationError::Config(_))));
        }
    }

    #[test]
    fn missing_explicit_directory_is_reported_without_recreating_an_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        let target = loc.select(&dir.path().join("removable")).unwrap();
        data(&target, "requests/saved.json", b"saved request");
        let moved = dir.path().join("moved");
        fs::rename(&target, &moved).unwrap();
        assert!(loc.resolve().is_err());
        assert!(!target.exists());
        assert_eq!(
            fs::read(moved.join("requests/saved.json")).unwrap(),
            b"saved request"
        );
    }

    #[test]
    fn choosing_directory_preserves_an_existing_write_probe_file() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        let target = dir.path().join("target");
        data(&target, ".write-probe", b"user file");
        loc.select(&target).unwrap();
        assert_eq!(fs::read(target.join(".write-probe")).unwrap(), b"user file");
    }

    #[test]
    fn interrupted_migration_cannot_be_opened_as_a_complete_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        data(&loc.system_root, "requests/old.json", b"original");
        let target = loc.select(&dir.path().join("target")).unwrap();
        // 模拟复制提交期间进程退出：只落了一部分数据，完成标记尚未清理。
        data(&target, ".getcat-migration-in-progress", b"incomplete");
        data(&target, "workspace.json", b"partial");
        assert!(loc.resolve().is_err());
        assert!(loc.select(&target).is_err());
        assert!(loc.migrate_from_system(&target).is_err());
        assert_eq!(
            fs::read(loc.system_root.join("requests/old.json")).unwrap(),
            b"original"
        );
        assert_eq!(fs::read(target.join("workspace.json")).unwrap(), b"partial");
    }

    #[cfg(unix)]
    #[test]
    fn selecting_or_reopening_a_linked_requests_directory_is_rejected() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        data(&loc.system_root, "requests/old.json", b"original");
        let target = loc.select(&dir.path().join("target")).unwrap();
        fs::remove_dir(target.join("requests")).unwrap();
        symlink(loc.system_root.join("requests"), target.join("requests")).unwrap();
        assert!(loc.select(&target).is_err());
        assert!(loc.resolve().is_err());
        assert_eq!(
            fs::read(loc.system_root.join("requests/old.json")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn captured_source_files_survive_later_saves_and_deletes_without_blocking_writes() {
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        data(&loc.system_root, "workspace.json", b"old workspace");
        data(&loc.system_root, "requests/old.json", b"old request");
        let target = loc.select(&dir.path().join("target")).unwrap();
        let snapshot = loc.capture_migration(&target).unwrap();
        write_atomic(&loc.system_root.join("requests/old.json"), b"new request").unwrap();
        fs::remove_file(loc.system_root.join("workspace.json")).unwrap();
        data(&loc.system_root, "requests/new.json", b"new file");
        assert_eq!(snapshot.copy_to(&target).unwrap(), 2);
        assert_eq!(
            fs::read(target.join("workspace.json")).unwrap(),
            b"old workspace"
        );
        assert_eq!(
            fs::read(target.join("requests/old.json")).unwrap(),
            b"old request"
        );
        assert!(!target.join("requests/new.json").exists());
        assert_eq!(
            fs::read(loc.system_root.join("requests/old.json")).unwrap(),
            b"new request"
        );
        assert!(loc.system_root.join("requests/new.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn failed_commit_rolls_back_created_files_and_allows_retry() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let loc = location(dir.path(), false);
        data(&loc.system_root, "workspace.json", b"workspace");
        data(&loc.system_root, "requests/old.json", b"request");
        let target = loc.select(&dir.path().join("target")).unwrap();
        let requests = target.join("requests");
        fs::set_permissions(&requests, fs::Permissions::from_mode(0o555)).unwrap();
        if tempfile::NamedTempFile::new_in(&requests).is_ok() {
            fs::set_permissions(&requests, fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let result = loc.migrate_from_system(&target);
        fs::set_permissions(&requests, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err());
        assert!(!target.join("workspace.json").exists());
        assert!(!target.join(MIGRATION_MARKER).exists());
        assert_eq!(
            fs::read(loc.system_root.join("workspace.json")).unwrap(),
            b"workspace"
        );
        assert_eq!(loc.migrate_from_system(&target).unwrap(), 2);
        assert!(!target.join(MIGRATION_MARKER).exists());
        assert_eq!(
            fs::read(target.join("requests/old.json")).unwrap(),
            b"request"
        );
    }
}
