//! 应用内更新：用 `gpui-updater` 从 GitHub Releases 检查、下载、校验（SHA-256 + minisign）并就地安装新版本。
//!
//! 全局只有一个 [`Updater`] 实体（[`UpdaterHandle`]），启动时由 `main` 安装；`Workspace` 观察它来刷新
//! 状态栏提示与「关于」页。所有网络与文件操作都在 gpui 的后台执行器上跑，这里只做状态读写与纯函数。
//!
//! 发布产物的命名约定在 `.github/workflows/release.yml`：`GetCat-<os>-<arch>.<ext>` + `SHA256SUMS` +
//! 每个文件的 `.minisig`。[`asset_pattern`] 按运行平台挑出对应子串，改命名时两边同步。

// 显式导入而非 `use gpui::*`：本文件含 `#[cfg(test)] mod tests`，通配符会引入 gpui 的 `test` 属性宏
// 与标准库 `#[test]` 冲突（见 workspace.rs 顶部说明）。
use std::path::PathBuf;
use std::time::Duration;

use getcat_core::model::AppSettings;
use gpui::{App, AppContext, Entity, Global, SharedString};
use gpui_updater::{
    EngineConfig, GitHubSource, UpdateSource, UpdateStatus, Updater, Verification, Version,
};

use crate::i18n::tr;
use crate::state::settings;

pub const REPO_OWNER: &str = "finch-xu";
pub const REPO_NAME: &str = "GetCat";
/// 发布页：自动更新不可用（开发构建、未支持的平台、安装失败）时给用户的退路。
pub const RELEASES_URL: &str = "https://github.com/finch-xu/GetCat/releases";
/// 启动后多久做第一次检查：先让首帧与数据加载完成，再去碰网络。
pub const LAUNCH_CHECK_DELAY: Duration = Duration::from_secs(5);

/// 仓库根目录的 `minisign.pub`（编译期嵌入）。CI 的 sign job 用对应私钥给每个资产出 `.minisig`，
/// 这里的公钥是客户端唯一信任的根——换密钥要同时换这个文件与 `MINISIGN_SECRET_KEY` secret。
const MINISIGN_PUB_FILE: &str = include_str!("../../../../minisign.pub");

/// 本次运行能不能就地安装更新。检查在任何情况下都允许，安装只有 `Installed` 可以。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// 正常安装：release 构建，macOS 上在 `.app` 内运行。
    Installed,
    /// `cargo run` / debug 构建 / macOS 上不在 `.app` 内：替换会指向错误的路径。
    DevBuild,
    /// macOS App Translocation：直接从 dmg 或 ~/Downloads 运行，系统把它挂在只读的随机路径下。
    Translocated,
}

pub struct UpdaterHandle {
    updater: Option<Entity<Updater>>,
    kind: InstallKind,
}

impl Global for UpdaterHandle {}

// ---------------------------------------------------------------------------
// 平台与安装态
// ---------------------------------------------------------------------------

/// 当前平台对应的发布资产子串（叠加在 gpui-updater 按 OS 猜的扩展名之上）；`None` = 没有为该平台发布产物。
pub fn asset_pattern() -> Option<&'static str> {
    asset_pattern_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn asset_pattern_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("macos-arm64.dmg"),
        ("macos", "x86_64") => Some("macos-x64.dmg"),
        ("linux", "x86_64") => Some("linux-x64.tar.gz"),
        ("windows", "x86_64") => Some("windows-x64.exe"),
        _ => None,
    }
}

pub fn install_kind() -> InstallKind {
    if cfg!(debug_assertions) {
        return InstallKind::DevBuild;
    }
    #[cfg(target_os = "macos")]
    {
        let Ok(root) = gpui_updater::current_install_root() else {
            return InstallKind::DevBuild;
        };
        if root.to_string_lossy().contains("/AppTranslocation/") {
            return InstallKind::Translocated;
        }
        if root.extension().is_none_or(|ext| ext != "app") {
            return InstallKind::DevBuild;
        }
    }
    InstallKind::Installed
}

/// minisign 公钥的 base64 正文（`minisign.pub` 里非注释的那一行）。
pub fn minisign_public_key() -> &'static str {
    MINISIGN_PUB_FILE
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .unwrap_or("")
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is valid semver")
}

fn download_dir() -> PathBuf {
    std::env::temp_dir().join("getcat-updates")
}

/// 生产配置：Strict —— 没有签名或校验和的 release 在检查阶段就被拒绝，不会显示为"可更新"。
pub fn engine_config() -> EngineConfig {
    EngineConfig::new(current_version())
        .minisign_public_key(minisign_public_key())
        .verification(Verification::Strict)
        .download_dir(download_dir())
}

// ---------------------------------------------------------------------------
// 安装与读取
// ---------------------------------------------------------------------------

/// 启动时安装全局更新器（必须在 `Workspace::restore` 之前，工作区构造时会订阅它）。
pub fn install(cx: &mut App) {
    let kind = install_kind();
    let Some(pattern) = asset_pattern() else {
        cx.set_global(UpdaterHandle {
            updater: None,
            kind,
        });
        return;
    };
    let source = GitHubSource::new(REPO_OWNER, REPO_NAME)
        .asset_contains(pattern)
        .with_checksums("SHA256SUMS")
        .with_minisig()
        // 演练用：含 "-" 的 tag 发成 prerelease，正式用户看不到；开发者设这个变量才会收到
        .include_prereleases(std::env::var_os("GETCAT_UPDATE_PRERELEASE").is_some());
    install_with_source(cx, source, engine_config(), kind);
}

/// 用指定的源安装更新器；测试用它注入假源。
pub fn install_with_source(
    cx: &mut App,
    source: impl UpdateSource,
    config: EngineConfig,
    kind: InstallKind,
) {
    let updater = cx.new(|cx| Updater::new(source, config, cx));
    cx.set_global(UpdaterHandle {
        updater: Some(updater),
        kind,
    });
}

pub fn updater(cx: &App) -> Option<Entity<Updater>> {
    cx.try_global::<UpdaterHandle>()
        .and_then(|h| h.updater.clone())
}

/// 当前状态；未安装更新器（不支持的平台 / 测试）时为 `Idle`。
pub fn status(cx: &App) -> UpdateStatus {
    updater(cx)
        .map(|u| u.read(cx).status().clone())
        .unwrap_or_default()
}

pub fn install_kind_of(cx: &App) -> InstallKind {
    cx.try_global::<UpdaterHandle>()
        .map(|h| h.kind)
        .unwrap_or(InstallKind::DevBuild)
}

/// 是否支持自动更新（平台有产物且更新器已安装）。
pub fn supported(cx: &App) -> bool {
    updater(cx).is_some()
}

/// 有新版本且本次运行允许就地安装。
pub fn can_install(cx: &App) -> bool {
    install_kind_of(cx) == InstallKind::Installed
        && matches!(status(cx), UpdateStatus::Available(_))
}

// ---------------------------------------------------------------------------
// 动作
// ---------------------------------------------------------------------------

pub fn check(cx: &mut App) {
    if let Some(u) = updater(cx) {
        u.update(cx, |u, cx| u.check(cx));
    }
}

pub fn download_and_install(cx: &mut App) {
    if !can_install(cx) {
        return;
    }
    if let Some(u) = updater(cx) {
        u.update(cx, |u, cx| u.download_and_install(cx));
    }
}

/// 重启进已安装的新版本（gpui 的 `restart` 会先走 `on_app_quit`，草稿照常落盘）。
pub fn restart(cx: &mut App) {
    if let Some(u) = updater(cx) {
        u.update(cx, |u, cx| u.restart(cx));
    }
}

/// 启动自动检查的开关：用户设置 + 只在正常安装下检查（开发构建每次 `cargo run` 都去打 GitHub API 没有意义），
/// `force`（环境变量 `GETCAT_UPDATE_CHECK`）让开发者也能在 `cargo run` 下试。
pub fn launch_check_enabled(settings: &AppSettings, kind: InstallKind, force: bool) -> bool {
    settings.check_updates_on_launch && (kind == InstallKind::Installed || force)
}

/// 启动后延迟 [`LAUNCH_CHECK_DELAY`] 检查一次；不轮询。
pub fn schedule_launch_check(cx: &mut App) {
    if !supported(cx) {
        return;
    }
    let force = std::env::var_os("GETCAT_UPDATE_CHECK").is_some();
    if !launch_check_enabled(&settings::settings(cx), install_kind_of(cx), force) {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(LAUNCH_CHECK_DELAY).await;
        cx.update(check);
    })
    .detach();
}

/// 清理上次更新的残留：Windows 上 gpui-updater 把旧 exe 改名为 `<exe>.old.exe` 留给下次启动删；
/// 下载目录里是已安装过的安装包。放在后台线程跑。
pub fn cleanup_leftovers() {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old.exe"));
    }
    let _ = std::fs::remove_dir_all(download_dir());
}

// ---------------------------------------------------------------------------
// 给 UI 的纯函数
// ---------------------------------------------------------------------------

/// 状态栏要不要提示：`Available` / `Staged` 给出版本号，bool 表示是否已装好只差重启。
pub fn hint_version(status: &UpdateStatus) -> Option<(&Version, bool)> {
    match status {
        UpdateStatus::Available(v) => Some((v, false)),
        UpdateStatus::Staged(v) => Some((v, true)),
        _ => None,
    }
}

/// 「关于」页的状态一行字。
pub fn status_line(status: &UpdateStatus, current: &str) -> SharedString {
    match status {
        UpdateStatus::Idle => tr!("update.idle", current = current),
        UpdateStatus::Checking => tr!("update.checking"),
        UpdateStatus::UpToDate => tr!("update.up_to_date", current = current),
        UpdateStatus::Available(v) => tr!("update.available", version = v),
        UpdateStatus::Downloading {
            downloaded,
            total: Some(total),
        } => tr!(
            "update.downloading",
            downloaded = format_mb(*downloaded),
            total = format_mb(*total)
        ),
        UpdateStatus::Downloading {
            downloaded,
            total: None,
        } => tr!("update.downloaded", downloaded = format_mb(*downloaded)),
        UpdateStatus::Installing => tr!("update.installing"),
        UpdateStatus::Staged(v) => tr!("update.staged", version = v),
        UpdateStatus::Errored(msg) => tr!("update.failed", error = msg),
    }
}

/// 下载进度百分比（0–100）；总大小未知时 `None`。
pub fn progress_percent(status: &UpdateStatus) -> Option<f32> {
    match status {
        UpdateStatus::Downloading {
            downloaded,
            total: Some(total),
        } if *total > 0 => {
            Some((*downloaded as f64 / *total as f64 * 100.0).clamp(0.0, 100.0) as f32)
        }
        _ => None,
    }
}

pub fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_pattern_matches_release_naming() {
        assert_eq!(
            asset_pattern_for("macos", "aarch64"),
            Some("macos-arm64.dmg")
        );
        assert_eq!(asset_pattern_for("macos", "x86_64"), Some("macos-x64.dmg"));
        assert_eq!(
            asset_pattern_for("linux", "x86_64"),
            Some("linux-x64.tar.gz")
        );
        assert_eq!(
            asset_pattern_for("windows", "x86_64"),
            Some("windows-x64.exe")
        );
        assert_eq!(asset_pattern_for("linux", "aarch64"), None);
        assert_eq!(asset_pattern_for("freebsd", "x86_64"), None);
        // 当前构建平台必须有产物（CI 三平台都是 x86_64 / aarch64）
        if cfg!(any(
            all(
                target_os = "macos",
                any(target_arch = "aarch64", target_arch = "x86_64")
            ),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "windows", target_arch = "x86_64"),
        )) {
            assert!(asset_pattern().is_some());
        }
    }

    #[test]
    fn public_key_is_embedded() {
        let key = minisign_public_key();
        assert!(key.starts_with("RW"), "minisign 公钥应以 RW 开头：{key:?}");
        assert!(!key.contains(' '));
    }

    #[test]
    fn current_version_parses() {
        assert_eq!(current_version().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn hint_only_for_available_and_staged() {
        let v = Version::new(1, 2, 3);
        assert_eq!(
            hint_version(&UpdateStatus::Available(v.clone())),
            Some((&v, false))
        );
        assert_eq!(
            hint_version(&UpdateStatus::Staged(v.clone())),
            Some((&v, true))
        );
        for s in [
            UpdateStatus::Idle,
            UpdateStatus::Checking,
            UpdateStatus::UpToDate,
            UpdateStatus::Downloading {
                downloaded: 1,
                total: None,
            },
            UpdateStatus::Installing,
            UpdateStatus::Errored("x".into()),
        ] {
            assert_eq!(hint_version(&s), None, "{s:?}");
        }
    }

    #[test]
    fn status_lines() {
        // 测试进程的 locale 是 rust-i18n 的默认值 en
        let v = Version::new(2, 0, 0);
        assert_eq!(
            status_line(&UpdateStatus::Idle, "0.1.0").as_ref(),
            "Not checked yet (current v0.1.0)"
        );
        assert_eq!(
            status_line(&UpdateStatus::Checking, "0.1.0").as_ref(),
            "Checking for updates…"
        );
        assert_eq!(
            status_line(&UpdateStatus::UpToDate, "0.1.0").as_ref(),
            "Up to date (v0.1.0)"
        );
        assert_eq!(
            status_line(&UpdateStatus::Available(v.clone()), "0.1.0").as_ref(),
            "v2.0.0 is available"
        );
        assert_eq!(
            status_line(
                &UpdateStatus::Downloading {
                    downloaded: 1_572_864,
                    total: Some(3_145_728),
                },
                "0.1.0"
            )
            .as_ref(),
            "Downloading 1.5 MB / 3.0 MB"
        );
        assert_eq!(
            status_line(
                &UpdateStatus::Downloading {
                    downloaded: 1_048_576,
                    total: None,
                },
                "0.1.0"
            )
            .as_ref(),
            "Downloaded 1.0 MB"
        );
        assert_eq!(
            status_line(&UpdateStatus::Installing, "0.1.0").as_ref(),
            "Installing…"
        );
        assert_eq!(
            status_line(&UpdateStatus::Staged(v), "0.1.0").as_ref(),
            "v2.0.0 installed; restart to apply"
        );
        assert_eq!(
            status_line(
                &UpdateStatus::Errored("http error: offline".into()),
                "0.1.0"
            )
            .as_ref(),
            "Update failed: http error: offline"
        );
    }

    #[test]
    fn progress_percent_only_with_known_total() {
        assert_eq!(
            progress_percent(&UpdateStatus::Downloading {
                downloaded: 25,
                total: Some(100)
            }),
            Some(25.0)
        );
        assert_eq!(
            progress_percent(&UpdateStatus::Downloading {
                downloaded: 25,
                total: None
            }),
            None
        );
        assert_eq!(
            progress_percent(&UpdateStatus::Downloading {
                downloaded: 5,
                total: Some(0)
            }),
            None
        );
        assert_eq!(progress_percent(&UpdateStatus::Installing), None);
    }

    #[test]
    fn launch_check_requires_setting_and_real_install() {
        let on = AppSettings {
            check_updates_on_launch: true,
            ..Default::default()
        };
        let off = AppSettings {
            check_updates_on_launch: false,
            ..Default::default()
        };
        assert!(launch_check_enabled(&on, InstallKind::Installed, false));
        assert!(!launch_check_enabled(&on, InstallKind::DevBuild, false));
        assert!(!launch_check_enabled(&on, InstallKind::Translocated, false));
        assert!(launch_check_enabled(&on, InstallKind::DevBuild, true));
        assert!(!launch_check_enabled(&off, InstallKind::Installed, false));
        assert!(!launch_check_enabled(&off, InstallKind::DevBuild, true));
    }

    #[test]
    fn debug_builds_are_dev_builds() {
        // 测试总是 debug 构建
        assert_eq!(install_kind(), InstallKind::DevBuild);
    }
}
