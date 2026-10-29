//! 数据位置与下一次启动位置分开保存；修改位置不替换正在使用的 Store。

use crate::{
    i18n::tr,
    state::{store, update},
};
use getcat_core::store::{
    Layout, Loaded, Store, StoreError, load_all,
    location::{DataLocation, LocationError},
};
use gpui_kit::{App, AppContext, Global, PathPromptOptions, SharedString};
use std::{io, path::PathBuf};

pub enum Notice {
    Changed,
    Copied(usize),
    Failed(LocationError),
}

pub struct DirectoryState {
    pub location: Option<DataLocation>,
    pub active: Option<PathBuf>,
    pub selected: Option<PathBuf>,
    pub busy: bool,
    pub migration_error: Option<LocationError>,
    pub notice: Option<Notice>,
}

impl Global for DirectoryState {}

impl DirectoryState {
    pub fn load(location: DataLocation) -> (Self, Result<Store, StoreError>, Loaded) {
        let resolved = location.resolve();
        let (active, opened, loaded, notice) = match resolved {
            Ok(root) => {
                let loaded = load_all(&Layout::new(root.clone()));
                let opened = Store::open(root.clone());
                let root = root.canonicalize().unwrap_or(root);
                (Some(root), opened, loaded, None)
            }
            Err(error) => {
                let message = error.to_string();
                (
                    None,
                    Err(StoreError::Io(io::Error::other(message))),
                    Loaded::default(),
                    Some(Notice::Failed(error)),
                )
            }
        };
        let migration_error = active
            .as_ref()
            .and_then(|root| location.migration_available(root).err());
        let state = Self {
            location: Some(location),
            selected: active.clone(),
            active,
            busy: false,
            migration_error,
            notice,
        };
        (state, opened, loaded)
    }

    pub fn pending(&self) -> bool {
        self.selected != self.active
    }
}

/// 在启动后台线程调用；MSI 与免安装的判定复用更新器，避免两套规则分叉。
pub fn open() -> (DirectoryState, Result<Store, StoreError>, Loaded) {
    let Some(system_root) = Store::default_root() else {
        return (
            DirectoryState {
                location: None,
                active: None,
                selected: None,
                busy: false,
                migration_error: None,
                notice: None,
            },
            Err(StoreError::NoDataDir),
            Loaded::default(),
        );
    };
    let portable_dir =
        if cfg!(windows) && update::windows_package() == update::WindowsPackage::Portable {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        } else {
            None
        };
    DirectoryState::load(DataLocation::new(system_root, portable_dir))
}

pub fn state(cx: &App) -> Option<&DirectoryState> {
    cx.try_global::<DirectoryState>()
}

pub fn choose(cx: &mut App) {
    if state(cx).is_none_or(|state| state.busy || state.location.is_none()) {
        return;
    }
    cx.global_mut::<DirectoryState>().busy = true;
    cx.refresh_windows();
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(tr!("common.choose")),
    });
    cx.spawn(async move |cx| {
        let path = match rx.await {
            Ok(Ok(Some(paths))) => paths.into_iter().next(),
            _ => None,
        };
        cx.update(|cx| {
            cx.global_mut::<DirectoryState>().busy = false;
            if let Some(path) = path {
                run_action(Action::Select(path), cx);
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

pub fn reset(cx: &mut App) {
    run_action(Action::Reset, cx);
}
pub fn migrate(cx: &mut App) {
    run_action(Action::Migrate, cx);
}

enum Action {
    Select(PathBuf),
    Reset,
    Migrate,
}

fn run_action(action: Action, cx: &mut App) {
    let Some(state) = state(cx) else {
        return;
    };
    if state.busy {
        return;
    }
    let Some(location) = state.location.clone() else {
        return;
    };
    let selected = state.selected.clone();
    if matches!(action, Action::Migrate) {
        if !state.pending() || state.migration_error.is_some() || selected.is_none() {
            return;
        }
        // 先投递尚未落盘的草稿；真正等待写入队列在后台完成。
        if let Some(workspace) = crate::current_workspace(cx) {
            workspace.update(cx, |workspace, cx| workspace.flush_drafts(cx));
        }
    }
    let source_store = store::store(cx).cloned();
    let state = cx.global_mut::<DirectoryState>();
    state.busy = true;
    state.notice = None;
    cx.refresh_windows();
    cx.spawn(async move |cx| {
        let result = cx
            .background_spawn(async move {
                let (target, notice) = match action {
                    Action::Select(path) => (location.select(&path)?, Notice::Changed),
                    Action::Reset => (location.reset()?, Notice::Changed),
                    Action::Migrate => {
                        let target = selected.expect("migration needs target");
                        let snapshot = if let Some(store) = source_store
                            && store.root().canonicalize()?
                                == location.system_root.canonicalize()?
                        {
                            let snapshot_location = location.clone();
                            let snapshot_target = target.clone();
                            let snapshot_store = store.clone();
                            store.read_snapshot(move || {
                                if let Some(error) = snapshot_store.last_error() {
                                    return Err(LocationError::Io(io::Error::other(error)));
                                }
                                snapshot_location.capture_migration(&snapshot_target)
                            })??
                        } else {
                            location.capture_migration(&target)?
                        };
                        let count = snapshot.copy_to(&target)?;
                        (target, Notice::Copied(count))
                    }
                };
                let migration_error = location.migration_available(&target).err();
                Ok::<_, LocationError>((target, notice, migration_error))
            })
            .await;
        cx.update(|cx| {
            let state = cx.global_mut::<DirectoryState>();
            state.busy = false;
            match result {
                Ok((target, notice, migration_error)) => {
                    state.selected = Some(target);
                    state.migration_error = migration_error;
                    state.notice = Some(notice);
                }
                Err(error) => state.notice = Some(Notice::Failed(error)),
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

pub fn error_line(error: &LocationError) -> SharedString {
    match error {
        LocationError::SourceEmpty => tr!("settings.data_source_empty"),
        LocationError::TargetNotEmpty => tr!("settings.data_target_not_empty"),
        LocationError::IncompleteMigration => tr!("settings.data_incomplete_migration"),
        LocationError::SameDirectory => tr!("settings.data_same_dir"),
        LocationError::OverlappingDirectories => tr!("settings.data_overlap"),
        LocationError::UnsupportedFile(_) => tr!("settings.data_unsupported_file"),
        _ => tr!("settings.data_dir_error", error = error),
    }
}

pub fn notice_line(notice: &Notice) -> SharedString {
    match notice {
        Notice::Changed => tr!("settings.data_dir_changed"),
        Notice::Copied(count) => tr!("settings.data_dir_copied", count = count),
        Notice::Failed(error) => error_line(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use getcat_core::{
        model::AppSettings,
        store::{Store, location::DataLocation},
    };
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn selection_and_reset_only_change_next_launch_and_never_redirect_current_writes(
        cx: &mut TestAppContext,
    ) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let location = DataLocation::new(dir.path().join("system"), None);
        let (state, opened, _) = DirectoryState::load(location.clone());
        let store = opened.unwrap();
        cx.update(|cx| {
            crate::state::store::install(cx, Ok(store.clone()));
            cx.set_global(state);
            run_action(Action::Select(dir.path().join("a")), cx);
        });
        cx.run_until_parked();
        let target = dir.path().join("a").canonicalize().unwrap();
        cx.update(|cx| {
            let state = cx.global::<DirectoryState>();
            assert_eq!(state.selected.as_ref(), Some(&target));
            assert_ne!(state.active, state.selected);
            assert_eq!(crate::state::store::store(cx).unwrap().root(), store.root());
        });
        store.write_settings(AppSettings::default());
        assert!(store.flush());
        assert!(store.root().join("settings.json").is_file());
        assert!(!target.join("settings.json").exists());
        cx.update(|cx| run_action(Action::Reset, cx));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(
                cx.global::<DirectoryState>().selected.as_deref(),
                Some(store.root().canonicalize().unwrap().as_path())
            );
            assert_eq!(crate::state::store::store(cx).unwrap().root(), store.root());
        });
        assert!(Store::open(location.resolve().unwrap()).is_ok());
    }

    #[gpui_kit::test]
    fn failed_selection_preserves_pending_directory(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let location = DataLocation::new(dir.path().join("system"), None);
        let (state, opened, _) = DirectoryState::load(location.clone());
        let _store = opened.unwrap();
        cx.update(|cx| cx.set_global(state));
        let file = dir.path().join("not-a-directory");
        std::fs::write(&file, b"x").unwrap();
        cx.update(|cx| run_action(Action::Select(file), cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let state = cx.global::<DirectoryState>();
            assert_eq!(state.selected, state.active);
            assert!(!state.busy);
            assert!(matches!(state.notice, Some(Notice::Failed(_))));
        });
        assert_eq!(location.resolve().unwrap(), location.system_root);
    }

    #[gpui_kit::test]
    fn directory_picker_selects_folders_and_cancel_keeps_saved_choice(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let location = DataLocation::new(dir.path().join("system"), None);
        let (state, opened, _) = DirectoryState::load(location.clone());
        let _store = opened.unwrap();
        cx.update(|cx| cx.set_global(state));
        cx.update(choose);
        assert!(cx.did_prompt_for_paths());
        let chosen = dir.path().join("choice");
        std::fs::create_dir(&chosen).unwrap();
        cx.simulate_path_prompt_response(move |opts| {
            assert!(!opts.files && opts.directories && !opts.multiple);
            Some(vec![chosen])
        });
        cx.run_until_parked();
        let saved = location.resolve().unwrap();
        assert_eq!(saved, dir.path().join("choice").canonicalize().unwrap());
        cx.update(choose);
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert_eq!(location.resolve().unwrap(), saved);
        cx.update(|cx| assert!(!cx.global::<DirectoryState>().busy));
    }

    #[gpui_kit::test]
    fn migration_uses_system_data_even_when_active_workspace_is_custom(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let location = DataLocation::new(dir.path().join("system"), None);
        std::fs::create_dir_all(&location.system_root).unwrap();
        std::fs::write(
            location.system_root.join("variables.json"),
            b"system variables",
        )
        .unwrap();
        let custom = location.select(&dir.path().join("custom")).unwrap();
        let (state, opened, _) = DirectoryState::load(location.clone());
        let store = opened.unwrap();
        store.write_settings(AppSettings::default());
        assert!(store.flush());
        cx.update(|cx| {
            crate::state::store::install(cx, Ok(store.clone()));
            cx.set_global(state);
            run_action(Action::Select(dir.path().join("target")), cx);
        });
        cx.run_until_parked();
        cx.update(migrate);
        cx.run_until_parked();
        let target = location.resolve().unwrap();
        assert_eq!(
            std::fs::read(target.join("variables.json")).unwrap(),
            b"system variables"
        );
        assert!(!target.join("settings.json").exists());
        assert!(custom.join("settings.json").is_file());
        cx.update(|cx| {
            let state = cx.global::<DirectoryState>();
            assert!(matches!(state.notice, Some(Notice::Copied(1))));
            assert_eq!(state.active.as_ref(), Some(&custom));
            assert_eq!(crate::state::store::store(cx).unwrap().root(), custom);
        });
    }

    #[gpui_kit::test]
    fn migration_cannot_write_into_currently_active_empty_directory(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let location = DataLocation::new(dir.path().join("system"), None);
        std::fs::create_dir_all(&location.system_root).unwrap();
        std::fs::write(location.system_root.join("settings.json"), b"system data").unwrap();
        let active = location.select(&dir.path().join("active")).unwrap();
        let (state, opened, _) = DirectoryState::load(location);
        let _store = opened.unwrap();
        cx.update(|cx| cx.set_global(state));
        cx.update(migrate);
        cx.run_until_parked();
        assert!(!active.join("settings.json").exists());
        cx.update(|cx| assert!(!cx.global::<DirectoryState>().busy));
    }

    #[test]
    fn interrupted_target_is_neither_loaded_quarantined_nor_opened_for_writing() {
        let dir = tempfile::tempdir().unwrap();
        let location = DataLocation::new(dir.path().join("system"), None);
        let target = location.select(&dir.path().join("target")).unwrap();
        std::fs::write(target.join("workspace.json"), b"partial invalid JSON").unwrap();
        std::fs::write(target.join(".getcat-migration-in-progress"), b"").unwrap();
        let (state, store, loaded) = DirectoryState::load(location);
        assert!(store.is_err());
        assert!(state.active.is_none());
        assert!(loaded.workspace.is_none());
        assert_eq!(
            std::fs::read(target.join("workspace.json")).unwrap(),
            b"partial invalid JSON"
        );
    }

    #[gpui_kit::test]
    fn migration_from_active_system_directory_includes_queued_unsaved_data(
        cx: &mut TestAppContext,
    ) {
        use getcat_core::{
            model::{RequestDraft, SavedRequest},
            store::codec::decode,
        };
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let location = DataLocation::new(dir.path().join("system"), None);
        let (state, opened, _) = DirectoryState::load(location.clone());
        let store = opened.unwrap();
        store.write_settings(AppSettings::default());
        assert!(store.flush());
        cx.update(|cx| {
            crate::state::store::install(cx, Ok(store.clone()));
            cx.set_global(state);
            run_action(Action::Select(dir.path().join("target")), cx);
        });
        cx.run_until_parked();
        let request = SavedRequest::new("queued", RequestDraft::default());
        store.write_request(request.clone()); // 仍在 500ms 合并窗口内，迁移必须先将它落盘。
        cx.update(migrate);
        cx.run_until_parked();
        let target = location.resolve().unwrap();
        let copied: SavedRequest = decode(
            &std::fs::read(target.join("requests").join(format!("{}.json", request.id))).unwrap(),
        )
        .unwrap();
        assert_eq!(copied, request);
        assert!(
            store
                .root()
                .join("requests")
                .join(format!("{}.json", request.id))
                .exists()
        );
        cx.update(|cx| {
            assert!(matches!(
                cx.global::<DirectoryState>().notice,
                Some(Notice::Copied(2))
            ))
        });
    }
}
