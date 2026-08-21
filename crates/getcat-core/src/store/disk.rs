//! 磁盘布局、原子写与启动读取。全部是阻塞 IO，只能在后台线程 / 写入线程调用。

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::de::DeserializeOwned;
use tempfile::NamedTempFile;
use tracing::warn;

use crate::model::{SavedRequest, TabDraft, TabId, Ulid, WorkspaceState, now_ms};
use crate::store::codec::{StoreError, decode};

pub const WORKSPACE_FILE: &str = "workspace.json";
pub const REQUESTS_DIR: &str = "requests";
pub const DRAFTS_DIR: &str = "drafts";
/// 可写性探测文件：创建后立即删除。
const PROBE_FILE: &str = ".write-probe";

/// 数据目录布局（spec §9.2）：`workspace.json`、`requests/<ulid>.json`、`drafts/<tab-id>.json`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: PathBuf) -> Layout {
        Layout { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn workspace_path(&self) -> PathBuf {
        self.root.join(WORKSPACE_FILE)
    }

    pub fn requests_dir(&self) -> PathBuf {
        self.root.join(REQUESTS_DIR)
    }

    pub fn drafts_dir(&self) -> PathBuf {
        self.root.join(DRAFTS_DIR)
    }

    pub fn request_path(&self, id: Ulid) -> PathBuf {
        self.requests_dir().join(format!("{id}.json"))
    }

    pub fn draft_path(&self, id: TabId) -> PathBuf {
        self.drafts_dir().join(format!("{id}.json"))
    }

    /// 创建三级目录并做一次真实写入探测；任何失败都归为"数据目录不可写"（spec §9.4）。
    pub fn ensure(&self) -> Result<(), StoreError> {
        for dir in [self.root.clone(), self.requests_dir(), self.drafts_dir()] {
            fs::create_dir_all(&dir).map_err(|e| self.unwritable(e))?;
        }
        let probe = self.root.join(PROBE_FILE);
        write_atomic(&probe, b"").map_err(|e| self.unwritable(e))?;
        remove_if_exists(&probe).map_err(|e| self.unwritable(e))?;
        Ok(())
    }

    fn unwritable(&self, source: io::Error) -> StoreError {
        StoreError::Unwritable {
            path: self.root.clone(),
            source,
        }
    }
}

/// 原子写：同目录临时文件 → 写入 → fsync → rename 覆盖。
/// 崩溃最多留下一个 `.tmp*` 临时文件；目标文件要么是完整的旧版，要么是完整的新版。
/// Unix 上显式把权限收紧到 0600（仅当前用户可读写），不依赖调用方的 umask：
/// 请求 Body / Header 里的 `Authorization` 等以明文落盘（spec §9.4）。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    fs::create_dir_all(dir)?;
    let mut tmp = NamedTempFile::new_in(dir)?;
    tmp.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// 启动读取时被跳过的文件及原因（已写入日志；UI 目前只计数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    pub path: PathBuf,
    pub message: String,
}

/// 一次启动读取的全部结果。
#[derive(Debug, Default)]
pub struct Loaded {
    pub workspace: Option<WorkspaceState>,
    pub drafts: Vec<TabDraft>,
    pub requests: Vec<SavedRequest>,
    pub errors: Vec<LoadError>,
}

/// 一次性读取三类文件（spec §9.4）。不存在的目录 / 文件视为空；损坏文件改名隔离并继续。
pub fn load_all(layout: &Layout) -> Loaded {
    let mut errors = Vec::new();
    let workspace = load_file(&layout.workspace_path(), &mut errors);
    let mut requests = Vec::new();
    load_dir(&layout.requests_dir(), &mut requests, &mut errors);
    let mut drafts = Vec::new();
    load_dir(&layout.drafts_dir(), &mut drafts, &mut errors);
    Loaded {
        workspace,
        drafts,
        requests,
        errors,
    }
}

/// 读取目录内所有 `*.json` 普通文件（按文件名排序）；临时文件与 `.corrupt-*` 因扩展名不同被自然忽略。
fn load_dir<T: DeserializeOwned>(dir: &Path, out: &mut Vec<T>, errors: &mut Vec<LoadError>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => {
            record(dir, format!("无法读取目录：{e}"), errors);
            return;
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "json") && p.is_file())
        .collect();
    paths.sort();
    for path in paths {
        if let Some(value) = load_file(&path, errors) {
            out.push(value);
        }
    }
}

/// 单个文件：不存在 → None；读不出来 → 记录（不改名，文件可能只是暂时不可读）；解析失败 → 隔离。
fn load_file<T: DeserializeOwned>(path: &Path, errors: &mut Vec<LoadError>) -> Option<T> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            record(path, format!("无法读取文件：{e}"), errors);
            return None;
        }
    };
    match decode::<T>(&bytes) {
        Ok(value) => Some(value),
        Err(err) => {
            quarantine(path, &err, errors);
            None
        }
    }
}

fn record(path: &Path, message: String, errors: &mut Vec<LoadError>) {
    warn!(path = %path.display(), "{message}");
    errors.push(LoadError {
        path: path.to_path_buf(),
        message,
    });
}

/// 把损坏文件改名为 `<原名>.corrupt-<unix毫秒>`；之后的 `load_dir` 因扩展名不是 json 而忽略它。
fn quarantine(path: &Path, err: &StoreError, errors: &mut Vec<LoadError>) {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".corrupt-{}", now_ms()));
    let dest = path.with_file_name(name);
    let message = match fs::rename(path, &dest) {
        Ok(()) => format!("文件损坏，已重命名为 {}：{err}", dest.display()),
        Err(rename_err) => format!("文件损坏且无法重命名（{rename_err}）：{err}"),
    };
    record(path, message, errors);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RequestDraft, SavedRequest, ThemePref};
    use crate::store::codec::encode;

    fn layout() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path().to_path_buf());
        layout.ensure().unwrap();
        (dir, layout)
    }

    /// 目录内全部条目名（含子目录），排序后返回。
    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn paths_follow_spec_layout() {
        let layout = Layout::new(PathBuf::from("/data/GetCat"));
        let id = Ulid::generate();
        assert_eq!(
            layout.workspace_path(),
            PathBuf::from("/data/GetCat/workspace.json")
        );
        assert_eq!(
            layout.request_path(id),
            PathBuf::from(format!("/data/GetCat/requests/{id}.json"))
        );
        assert_eq!(
            layout.draft_path(id),
            PathBuf::from(format!("/data/GetCat/drafts/{id}.json"))
        );
    }

    #[test]
    fn ensure_creates_layout_and_removes_probe() {
        let (_dir, layout) = layout();
        assert!(layout.requests_dir().is_dir());
        assert!(layout.drafts_dir().is_dir());
        assert_eq!(entries(layout.root()), vec!["drafts", "requests"]);
    }

    #[test]
    fn write_atomic_replaces_content_and_leaves_no_temp_files() {
        let (_dir, layout) = layout();
        let path = layout.workspace_path();
        write_atomic(&path, b"v1").unwrap();
        write_atomic(&path, b"v2-longer").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"v2-longer");
        assert_eq!(
            entries(layout.root()),
            vec!["drafts", "requests", "workspace.json"]
        );
        // 父目录不存在时自动创建
        let nested = layout.root().join("deep").join("x.json");
        write_atomic(&nested, b"{}").unwrap();
        assert_eq!(std::fs::read(&nested).unwrap(), b"{}");
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_fixes_permissions_to_0600() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, layout) = layout();
        let path = layout.workspace_path();
        write_atomic(&path, b"{}").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    }

    #[test]
    fn remove_if_exists_tolerates_missing_file() {
        let (_dir, layout) = layout();
        let path = layout.workspace_path();
        remove_if_exists(&path).unwrap();
        write_atomic(&path, b"{}").unwrap();
        remove_if_exists(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn load_all_on_empty_or_missing_root_is_empty() {
        let (dir, layout) = layout();
        let loaded = load_all(&layout);
        assert!(loaded.workspace.is_none());
        assert!(loaded.drafts.is_empty() && loaded.requests.is_empty() && loaded.errors.is_empty());
        // 目录整个不存在也不算错误
        let missing = Layout::new(dir.path().join("nope"));
        let loaded = load_all(&missing);
        assert!(loaded.errors.is_empty());
    }

    #[test]
    fn load_all_reads_three_kinds_and_ignores_leftovers() {
        let (_dir, layout) = layout();
        let req = SavedRequest::new(
            "a",
            RequestDraft {
                url: "https://a".into(),
                ..Default::default()
            },
        );
        let draft = TabDraft {
            id: Ulid::generate(),
            draft: RequestDraft::default(),
            saved_id: Some(req.id),
            dirty: true,
        };
        let ws = WorkspaceState {
            theme: ThemePref::Dark,
            ..Default::default()
        };
        write_atomic(&layout.request_path(req.id), &encode(&req).unwrap()).unwrap();
        write_atomic(&layout.draft_path(draft.id), &encode(&draft).unwrap()).unwrap();
        write_atomic(&layout.workspace_path(), &encode(&ws).unwrap()).unwrap();
        // 崩溃残留的临时文件与早先隔离的损坏文件都不是 .json，必须被忽略
        std::fs::write(layout.requests_dir().join(".tmpAbC123"), b"garbage").unwrap();
        std::fs::write(layout.requests_dir().join("old.json.corrupt-1"), b"garbage").unwrap();
        std::fs::create_dir(layout.drafts_dir().join("folder.json")).unwrap();

        let loaded = load_all(&layout);
        assert_eq!(loaded.workspace, Some(ws));
        assert_eq!(loaded.requests, vec![req]);
        assert_eq!(loaded.drafts, vec![draft]);
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    }

    #[test]
    fn corrupt_json_is_quarantined_and_reported() {
        let (_dir, layout) = layout();
        let bad = layout.requests_dir().join("bad.json");
        std::fs::write(&bad, b"{not json").unwrap();
        let loaded = load_all(&layout);
        assert!(loaded.requests.is_empty());
        assert_eq!(loaded.errors.len(), 1);
        assert_eq!(loaded.errors[0].path, bad);
        assert!(!bad.exists(), "original must be renamed away");
        let names = entries(&layout.requests_dir());
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("bad.json.corrupt-"), "{names:?}");
        // 第二次启动：隔离文件被忽略，不再报错
        let again = load_all(&layout);
        assert!(again.errors.is_empty());
    }

    #[test]
    fn unknown_version_is_quarantined() {
        let (_dir, layout) = layout();
        std::fs::write(
            layout.workspace_path(),
            br#"{"version": 99, "sidebar_collapsed": true}"#,
        )
        .unwrap();
        let loaded = load_all(&layout);
        assert!(loaded.workspace.is_none());
        assert_eq!(loaded.errors.len(), 1);
        assert!(
            loaded.errors[0].message.contains("99"),
            "{}",
            loaded.errors[0].message
        );
        assert!(!layout.workspace_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn unwritable_root_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ro");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555)).unwrap();
        let layout = Layout::new(root.clone());
        let result = layout.ensure();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        match result {
            Err(StoreError::Unwritable { path, .. }) => assert_eq!(path, root),
            // root 用户无视权限位：此时无法构造不可写目录，跳过
            Ok(()) => eprintln!("running with elevated permissions; skipping unwritable check"),
            Err(other) => panic!("expected Unwritable, got {other:?}"),
        }
    }
}
