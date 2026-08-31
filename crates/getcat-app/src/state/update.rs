//! 应用内更新：用 `gpui-updater` 检查、下载、校验（SHA-256 + minisign）并就地安装新版本。
//!
//! 更新源有两个，安全保证一致（同一把 minisign 私钥签名、客户端同一个内置公钥验签）：
//! - 全球：GitHub Releases（`GitHubSource`）；
//! - 中国大陆：阿里云 OSS 镜像（`StaticManifestSource` 读 [`MIRROR_MANIFEST_URL`]），
//!   由 release.yml 的 mirror-to-oss job 从 GitHub Release 原样搬运并生成 manifest。
//!
//! 选哪个由设置里的 [`UpdateSourcePref`] 决定，默认自动（界面语言中文 → 大陆镜像）；
//! 运行时切换靠 [`SwitchableSource`] 的共享状态，见 [`sync_source`]。
//!
//! 全局只有一个 [`Updater`] 实体（[`UpdaterHandle`]），启动时由 `main` 安装；`Workspace` 观察它来刷新
//! 状态栏提示与「关于」页。所有网络与文件操作都在 gpui 的后台执行器上跑，这里只做状态读写与纯函数。
//!
//! 发布产物的命名约定在 `.github/workflows/release.yml`：`GetCat-<os>-<arch>.<ext>` + `SHA256SUMS` +
//! 每个文件的 `.minisig`。[`asset_pattern`] 按运行平台挑出对应子串，改命名时两边同步。
//!
//! Windows 有两份产物：免安装的 `.exe` 与 per-user 安装包 `.msi`。装了 MSI 的用户必须更新到
//! MSI，否则裸 exe 被原地替换，而「应用和功能」里记的版本停在初装那一版，之后 MSI 的修复或
//! 重装会把旧二进制还原回去。区分靠安装目录里的 `install-source.txt`，见 [`windows_package`]。

// 显式导入而非 `use gpui::*`：本文件含 `#[cfg(test)] mod tests`，通配符会引入 gpui 的 `test` 属性宏
// 与标准库 `#[test]` 冲突（见 workspace.rs 顶部说明）。
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use getcat_core::model::{AppSettings, UpdateSourcePref};
use gpui::{App, AppContext, Entity, Global, SharedString};
use gpui_updater::{
    EngineConfig, GitHubSource, StaticManifestSource, UpdateSource, UpdateStatus, Updater,
    Verification, Version,
};

use crate::i18n::tr;
use crate::state::settings;

pub const REPO_OWNER: &str = "finch-xu";
pub const REPO_NAME: &str = "GetCat";
/// 发布页：自动更新不可用（开发构建、未支持的平台、安装失败）时给用户的退路。
pub const RELEASES_URL: &str = "https://github.com/finch-xu/GetCat/releases";
/// 中国大陆镜像的静态 manifest（阿里云 OSS，release.yml 的 mirror-to-oss job 生成并上传）。
/// 桶里是扁平覆盖布局：产物直接躺在 `GetCat/` 下，每次发版原地覆盖，只保留最新版。
pub const MIRROR_MANIFEST_URL: &str = "https://d.mirror.catonthe.top/GetCat/latest.json";
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
    /// [`SwitchableSource`] 的共享状态；设置里改更新源只改这里，不重建 Updater 实体
    /// （`Workspace` 构造时订阅了实体，重建会让状态栏的更新提示失联）。
    source_switch: Option<Arc<Mutex<ResolvedSource>>>,
}

impl Global for UpdaterHandle {}

// ---------------------------------------------------------------------------
// 更新源的解析与切换
// ---------------------------------------------------------------------------

/// [`UpdateSourcePref::Auto`] 落定之后只剩两种真实源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedSource {
    /// GitHub Releases（全球默认）。
    Global,
    /// 阿里云 OSS 镜像（中国大陆），走 [`MIRROR_MANIFEST_URL`]。
    ChinaMirror,
}

/// 把更新源偏好解析成真实源：自动模式看界面语言（中文 → 大陆镜像），显式选择直接生效。
pub fn resolve_source(pref: UpdateSourcePref, ui_locale: &str) -> ResolvedSource {
    match pref {
        UpdateSourcePref::Global => ResolvedSource::Global,
        UpdateSourcePref::ChinaMirror => ResolvedSource::ChinaMirror,
        UpdateSourcePref::Auto => {
            if ui_locale == crate::i18n::ZH_CN {
                ResolvedSource::ChinaMirror
            } else {
                ResolvedSource::Global
            }
        }
    }
}

/// 运行时可切换的更新源：`fetch_latest` 每次现读共享状态，设置变更立即对下一次检查生效。
/// 泛型只为了测试能注入假源；生产装配在 [`install`]。
struct SwitchableSource<G, M> {
    current: Arc<Mutex<ResolvedSource>>,
    global: G,
    mirror: M,
}

impl<G: UpdateSource, M: UpdateSource> UpdateSource for SwitchableSource<G, M> {
    fn fetch_latest(&self) -> gpui_updater::Result<gpui_updater::Release> {
        // 锁中毒说明写入方（主线程的 sync_source）panic 过；值是 Copy 的、不会写坏，取出继续用
        let current = *self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match current {
            ResolvedSource::Global => self.global.fetch_latest(),
            ResolvedSource::ChinaMirror => self.mirror.fetch_latest(),
        }
    }
}

/// 当前生效的真实更新源；更新器未安装（不支持的平台 / 注入假源的测试）时为 `None`。
/// 目前只有测试断言用；界面要展示生效源时去掉 cfg 即可。
#[cfg(test)]
pub fn resolved_source(cx: &App) -> Option<ResolvedSource> {
    let switch = cx.try_global::<UpdaterHandle>()?.source_switch.as_ref()?;
    Some(
        *switch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// 按当前设置与界面语言重算更新源。`settings::update` 在 `update_source` 或 `language`
/// （影响自动模式）变化后调用；下一次检查即走新源。
pub fn sync_source(cx: &mut App) {
    let Some(switch) = cx
        .try_global::<UpdaterHandle>()
        .and_then(|h| h.source_switch.clone())
    else {
        return;
    };
    let resolved = resolve_source(
        settings::settings(cx).update_source,
        crate::i18n::current(cx),
    );
    *switch
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = resolved;
}

// ---------------------------------------------------------------------------
// 平台与安装态
// ---------------------------------------------------------------------------

/// Windows 上这份程序是怎么装进来的；决定自动更新去拿哪一种产物。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPackage {
    /// 免安装单文件：更新器原地重命名替换。非 Windows 平台也报这个（它们只有一种产物）。
    Portable,
    /// per-user MSI：更新器下载 `.msi`，等应用退出后交给 msiexec。
    Msi,
}

/// MSI 装进安装目录的标记文件（内容是 `msi`）；免安装版的同目录下没有它。
/// 写入方在 `scripts/package-windows.ps1`，打包进 MSI 的声明在 `resources/windows/GetCat.wxs`。
const INSTALL_SOURCE_FILE: &str = "install-source.txt";

/// 读当前可执行文件同目录的标记文件。拿不到 exe 路径、读不到文件、内容对不上——一律当免安装。
/// 这个方向的猜错代价小：MSI 用户被裸 exe 替换只是版本记录滞后；反过来免安装用户会下到一个
/// 装不进去的安装包。
pub fn windows_package() -> WindowsPackage {
    let Ok(exe) = std::env::current_exe() else {
        return WindowsPackage::Portable;
    };
    let Some(dir) = exe.parent() else {
        return WindowsPackage::Portable;
    };
    windows_package_in(dir)
}

/// [`windows_package`] 去掉「找到自己在哪」那一步后的纯函数部分，方便测试。
fn windows_package_in(dir: &Path) -> WindowsPackage {
    match std::fs::read_to_string(dir.join(INSTALL_SOURCE_FILE)) {
        Ok(marker) if marker.trim().eq_ignore_ascii_case("msi") => WindowsPackage::Msi,
        _ => WindowsPackage::Portable,
    }
}

/// 当前平台对应的发布资产子串；`None` = 没有为该平台发布产物。
pub fn asset_pattern() -> Option<&'static str> {
    asset_pattern_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        windows_package(),
    )
}

pub fn asset_pattern_for(os: &str, arch: &str, win: WindowsPackage) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("macos-arm64.dmg"),
        ("macos", "x86_64") => Some("macos-x64.dmg"),
        ("linux", "x86_64") => Some("linux-x64.tar.gz"),
        ("linux", "aarch64") => Some("linux-arm64.tar.gz"),
        ("windows", "x86_64") => Some(match win {
            WindowsPackage::Portable => "windows-x64.exe",
            WindowsPackage::Msi => "windows-x64.msi",
        }),
        ("windows", "aarch64") => Some(match win {
            WindowsPackage::Portable => "windows-arm64.exe",
            WindowsPackage::Msi => "windows-arm64.msi",
        }),
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
            source_switch: None,
        });
        return;
    };
    let github = GitHubSource::new(REPO_OWNER, REPO_NAME)
        // 必须是 asset_patterns（整体替换）而不是 asset_contains（追加）：GitHubSource::new
        // 预置了一个按 OS 猜的扩展名，Windows 上是 ".exe"。追加的话 MSI 用户的匹配条件会变成
        // 「同时含 .exe 和 windows-x64.msi」，永远匹配不上，客户端只会显示「已是最新版」。
        // 我们的 pattern 本来就自带扩展名，不需要那个预置项。
        .asset_patterns(vec![pattern.to_string()])
        .with_checksums("SHA256SUMS")
        .with_minisig()
        // 演练用：含 "-" 的 tag 发成 prerelease，正式用户看不到；开发者设这个变量才会收到。
        // 只对 GitHub 源有意义：mirror-to-oss job 会跳过 prerelease，镜像上永远只有正式版。
        .include_prereleases(std::env::var_os("GETCAT_UPDATE_PRERELEASE").is_some());
    // 镜像 manifest 的 sha256 / signature_url 内嵌在资产条目里，engine_config 的 Strict
    // 校验与 minisign 公钥两个源共用，安全保证一致。StaticManifestSource::new 同样预置了
    // 按 OS 猜的扩展名，理由同上用 asset_patterns 整体替换。
    let mirror =
        StaticManifestSource::new(MIRROR_MANIFEST_URL).asset_patterns(vec![pattern.to_string()]);
    let resolved = resolve_source(
        settings::settings(cx).update_source,
        crate::i18n::current(cx),
    );
    let switch = Arc::new(Mutex::new(resolved));
    let source = SwitchableSource {
        current: switch.clone(),
        global: github,
        mirror,
    };
    install_with_source(cx, source, engine_config(), kind);
    cx.global_mut::<UpdaterHandle>().source_switch = Some(switch);
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
        source_switch: None,
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
        use WindowsPackage::Portable;

        assert_eq!(
            asset_pattern_for("macos", "aarch64", Portable),
            Some("macos-arm64.dmg")
        );
        assert_eq!(
            asset_pattern_for("macos", "x86_64", Portable),
            Some("macos-x64.dmg")
        );
        assert_eq!(
            asset_pattern_for("linux", "x86_64", Portable),
            Some("linux-x64.tar.gz")
        );
        assert_eq!(
            asset_pattern_for("linux", "aarch64", Portable),
            Some("linux-arm64.tar.gz")
        );
        assert_eq!(asset_pattern_for("freebsd", "x86_64", Portable), None);
        // 非 Windows 平台只有一种产物，安装方式不影响选择
        assert_eq!(
            asset_pattern_for("macos", "aarch64", WindowsPackage::Msi),
            Some("macos-arm64.dmg")
        );
        // 当前构建平台必须有产物（CI 三平台都发 x86_64 与 aarch64 两种架构）
        if cfg!(all(
            any(
                target_os = "macos",
                target_os = "linux",
                target_os = "windows"
            ),
            any(target_arch = "aarch64", target_arch = "x86_64")
        )) {
            assert!(asset_pattern().is_some());
        }
    }

    /// Windows 的两份产物各走各的：免安装拿 .exe，MSI 拿 .msi。
    /// 名字必须和 release.yml 的上传步骤、sign job 的资产白名单一致。
    #[test]
    fn windows_picks_the_asset_matching_how_it_was_installed() {
        assert_eq!(
            asset_pattern_for("windows", "x86_64", WindowsPackage::Portable),
            Some("windows-x64.exe")
        );
        assert_eq!(
            asset_pattern_for("windows", "x86_64", WindowsPackage::Msi),
            Some("windows-x64.msi")
        );
        assert_eq!(
            asset_pattern_for("windows", "aarch64", WindowsPackage::Portable),
            Some("windows-arm64.exe")
        );
        assert_eq!(
            asset_pattern_for("windows", "aarch64", WindowsPackage::Msi),
            Some("windows-arm64.msi")
        );
    }

    /// 标记文件的判定：只有内容确实是 `msi` 才算 MSI 安装，其余一律退回免安装。
    /// 猜成 Portable 的代价只是版本记录滞后；反过来会让免安装用户下到装不进去的包。
    #[test]
    fn install_source_marker_decides_the_windows_package() {
        let dir = tempfile::tempdir().expect("临时目录");
        let marker = dir.path().join(INSTALL_SOURCE_FILE);

        // 没有标记文件 = 免安装
        assert_eq!(windows_package_in(dir.path()), WindowsPackage::Portable);

        // 打包脚本写的是带换行的 "msi\n"；大小写也不该影响判定
        for content in ["msi", "msi\n", "msi\r\n", "  MSI  "] {
            std::fs::write(&marker, content).expect("写标记文件");
            assert_eq!(
                windows_package_in(dir.path()),
                WindowsPackage::Msi,
                "内容 {content:?} 应判定为 MSI"
            );
        }

        // 内容对不上就不认——宁可退回免安装，也不要拿一个装不进去的包
        for content in ["", "exe", "portable", "msix"] {
            std::fs::write(&marker, content).expect("写标记文件");
            assert_eq!(
                windows_package_in(dir.path()),
                WindowsPackage::Portable,
                "内容 {content:?} 不该判定为 MSI"
            );
        }
    }

    /// 自动模式跟着界面语言走：中文界面用大陆镜像，其余用 GitHub；显式选择无视语言。
    #[test]
    fn auto_source_follows_ui_language_and_explicit_choice_wins() {
        use getcat_core::model::UpdateSourcePref::{Auto, ChinaMirror, Global};

        assert_eq!(
            resolve_source(Auto, crate::i18n::ZH_CN),
            ResolvedSource::ChinaMirror
        );
        assert_eq!(
            resolve_source(Auto, crate::i18n::EN),
            ResolvedSource::Global
        );
        assert_eq!(
            resolve_source(Global, crate::i18n::ZH_CN),
            ResolvedSource::Global
        );
        assert_eq!(
            resolve_source(ChinaMirror, crate::i18n::EN),
            ResolvedSource::ChinaMirror
        );
    }

    /// 切换共享状态后，同一个源实例的下一次 fetch 就走另一边——不需要重建 Updater 实体。
    #[test]
    fn switchable_source_delegates_to_the_currently_selected_side() {
        use std::sync::{Arc, Mutex};

        struct Fake(&'static str);
        impl UpdateSource for Fake {
            fn fetch_latest(&self) -> gpui_updater::Result<gpui_updater::Release> {
                Ok(gpui_updater::Release {
                    version: Version::new(9, 9, 9),
                    notes: Some(self.0.to_string()),
                    asset: gpui_updater::Asset {
                        name: "GetCat-test".into(),
                        url: format!("https://{}/GetCat-test", self.0),
                        size: 0,
                    },
                    signature: None,
                    signature_url: None,
                    sha256: None,
                })
            }
        }

        let current = Arc::new(Mutex::new(ResolvedSource::Global));
        let source = SwitchableSource {
            current: current.clone(),
            global: Fake("github"),
            mirror: Fake("mirror"),
        };
        assert_eq!(
            source.fetch_latest().unwrap().notes.as_deref(),
            Some("github")
        );
        *current.lock().unwrap() = ResolvedSource::ChinaMirror;
        assert_eq!(
            source.fetch_latest().unwrap().notes.as_deref(),
            Some("mirror")
        );
    }

    /// 镜像 manifest 的地址与 CI（release.yml 的 mirror-to-oss job）约定一致。
    #[test]
    fn mirror_manifest_url_is_pinned_to_the_oss_domain() {
        assert_eq!(
            MIRROR_MANIFEST_URL,
            "https://d.mirror.catonthe.top/GetCat/latest.json"
        );
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
        // 测试进程的 locale 是 en（见 i18n::locale_test_lock）
        let _locale = crate::i18n::locale_test_lock();
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
