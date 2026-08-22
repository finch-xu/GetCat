//! 纯文件持久化：版本化 JSON、路径布局、原子写、合并写队列、启动读取与损坏隔离（spec §9）。
//! 不存历史、不存响应。

pub mod codec;
pub mod disk;
pub mod writer;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub use codec::{FORMAT_VERSION, StoreError};
pub use disk::{
    DRAFTS_DIR, Layout, LoadError, Loaded, REQUESTS_DIR, SETTINGS_FILE, WORKSPACE_FILE,
    copy_atomic, copy_atomic_user, load_all, write_atomic, write_atomic_user,
};
pub use writer::{COALESCE_WINDOW, FLUSH_TIMEOUT, StoreWriter};

use crate::model::{AppSettings, SavedRequest, TabDraft, TabId, Ulid, WorkspaceState};

/// 持久化门面：路径布局 + 写入线程。可 Clone（共享同一线程）。
#[derive(Clone)]
pub struct Store {
    layout: Layout,
    writer: StoreWriter,
}

impl Store {
    /// 平台数据目录（spec §9.2）：macOS `~/Library/Application Support/GetCat`，
    /// Linux `$XDG_DATA_HOME/getcat`（默认 `~/.local/share/getcat`），Windows `%APPDATA%\GetCat\data`。
    pub fn default_root() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "GetCat").map(|dirs| dirs.data_dir().to_path_buf())
    }

    pub fn open(root: PathBuf) -> Result<Store, StoreError> {
        Self::open_with_delay(root, COALESCE_WINDOW)
    }

    /// 创建目录、探测可写性、启动写入线程。`delay` 是合并窗口（测试用 `Duration::ZERO`）。
    pub fn open_with_delay(root: PathBuf, delay: Duration) -> Result<Store, StoreError> {
        let layout = Layout::new(root);
        layout.ensure()?;
        Ok(Store {
            layout,
            writer: StoreWriter::spawn(delay),
        })
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn root(&self) -> &Path {
        self.layout.root()
    }

    /// 阻塞读取全部文件；只能在后台线程调用。
    pub fn load_all(&self) -> Loaded {
        disk::load_all(&self.layout)
    }

    pub fn write_request(&self, request: SavedRequest) {
        let path = self.layout.request_path(request.id);
        self.writer.write(path, move || codec::encode(&request));
    }

    pub fn write_draft(&self, draft: TabDraft) {
        let path = self.layout.draft_path(draft.id);
        self.writer.write(path, move || codec::encode(&draft));
    }

    pub fn write_workspace(&self, state: WorkspaceState) {
        let path = self.layout.workspace_path();
        self.writer.write(path, move || codec::encode(&state));
    }

    pub fn write_settings(&self, settings: AppSettings) {
        let path = self.layout.settings_path();
        self.writer.write(path, move || codec::encode(&settings));
    }

    pub fn delete_request(&self, id: Ulid) {
        self.writer.delete(self.layout.request_path(id));
    }

    pub fn delete_draft(&self, id: TabId) {
        self.writer.delete(self.layout.draft_path(id));
    }

    /// 等待队列清空，最多 `FLUSH_TIMEOUT`（2 s）。
    pub fn flush(&self) -> bool {
        self.writer.flush(FLUSH_TIMEOUT)
    }

    pub fn flush_within(&self, timeout: Duration) -> bool {
        self.writer.flush(timeout)
    }

    /// 最近一次写入失败（供顶部横幅；O(1)）。
    pub fn last_error(&self) -> Option<String> {
        self.writer.last_error()
    }

    pub fn write_count(&self) -> u64 {
        self.writer.write_count()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::model::{RequestDraft, ThemePref};

    fn open(delay: Duration) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_with_delay(dir.path().to_path_buf(), delay).unwrap();
        (dir, store)
    }

    fn draft(url: &str) -> TabDraft {
        TabDraft {
            id: Ulid::generate(),
            draft: RequestDraft {
                url: url.into(),
                ..Default::default()
            },
            saved_id: None,
            dirty: false,
        }
    }

    #[cfg(not(windows))] // Windows 下是 %APPDATA%\GetCat\data，末段为 data
    #[test]
    fn default_root_ends_with_getcat() {
        let root = Store::default_root().expect("home dir available");
        let name = root.file_name().unwrap().to_str().unwrap();
        assert!(name.eq_ignore_ascii_case("getcat"), "{name}");
    }

    #[test]
    fn open_creates_layout_and_roundtrips_all_three_kinds() {
        let (_dir, store) = open(Duration::ZERO);
        assert!(store.layout().requests_dir().is_dir());
        let req = SavedRequest::new("a", RequestDraft::default());
        let d = draft("https://d");
        let ws = WorkspaceState {
            tab_order: vec![d.id],
            active: Some(d.id),
            theme: ThemePref::Light,
            ..Default::default()
        };
        store.write_request(req.clone());
        store.write_draft(d.clone());
        store.write_workspace(ws.clone());
        assert!(store.flush());
        let loaded = store.load_all();
        assert_eq!(loaded.requests, vec![req]);
        assert_eq!(loaded.drafts, vec![d]);
        assert_eq!(loaded.workspace, Some(ws));
        assert!(loaded.errors.is_empty());
    }

    #[test]
    fn repeated_draft_writes_are_coalesced_per_tab() {
        let (_dir, store) = open(Duration::from_millis(300));
        let mut d = draft("u0");
        for i in 1..=9 {
            d.draft.url = format!("u{i}");
            store.write_draft(d.clone());
        }
        let other = draft("other");
        store.write_draft(other.clone());
        assert!(store.flush());
        assert_eq!(store.write_count(), 2);
        let loaded = store.load_all();
        let urls: Vec<&str> = loaded.drafts.iter().map(|d| d.draft.url.as_str()).collect();
        assert!(urls.contains(&"u9") && urls.contains(&"other"), "{urls:?}");
    }

    #[test]
    fn delete_removes_files() {
        let (_dir, store) = open(Duration::ZERO);
        let req = SavedRequest::new("a", RequestDraft::default());
        let d = draft("x");
        store.write_request(req.clone());
        store.write_draft(d.clone());
        assert!(store.flush());
        store.delete_request(req.id);
        store.delete_draft(d.id);
        assert!(store.flush());
        let loaded = store.load_all();
        assert!(loaded.requests.is_empty() && loaded.drafts.is_empty());
        assert!(!store.layout().request_path(req.id).exists());
    }

    #[cfg(unix)]
    #[test]
    fn unwritable_root_fails_to_open() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ro");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = Store::open_with_delay(root.clone(), Duration::ZERO);
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        match result {
            Err(StoreError::Unwritable { .. }) => {}
            Ok(_) => eprintln!("running with elevated permissions; skipping"),
            Err(other) => panic!("expected Unwritable, got {other:?}"),
        }
    }
}
