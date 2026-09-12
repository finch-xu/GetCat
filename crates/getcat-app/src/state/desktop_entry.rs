//! Linux「加入应用菜单」：按 XDG 规范把 `.desktop` 与图标写进用户目录。
//!
//! 这是设置里的一个开关，**默认关闭**；开关的真值就是 `.desktop` 文件存不存在，不记进
//! `settings.json`——用户手动删了文件，开关自然显示关闭，不会出现「设置说开着、菜单里没有」。
//!
//! 它同时决定 Wayland 下窗口有没有图标：Wayland 协议没有「窗口图标」，合成器只认
//! 窗口的 app_id（[`crate::brand::APP_ID`]），去 `applications/<app_id>.desktop` 读 `Icon=`，
//! 再按名字到 `icons/hicolor/<尺寸>/apps/` 找图。X11 下窗口图标由 `_NET_WM_ICON` 直接给
//! （`main.rs` 的 `WindowOptions::icon`），但应用列表里的条目同样要靠这份 .desktop。
//!
//! 纯函数部分（路径、文件内容、Exec 转义）在所有平台都编译，单测能在 macOS 上跑；
//! 只有 gpui 侧的全局与设置页开关是 Linux 专属。
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::io;
use std::path::{Path, PathBuf};

use crate::brand;

/// hicolor 主题里的图标名（`Icon=` 的值，不带扩展名）。
pub const ICON_NAME: &str = "getcat";
/// 写进 hicolor 的图标尺寸档；内嵌的 logo 位图本身就是 512 px 见方。
const ICON_SIZE: u32 = 512;

/// `$XDG_DATA_HOME`（默认 `~/.local/share`）下的两个落点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdgDirs {
    data_home: PathBuf,
}

impl XdgDirs {
    pub fn new(data_home: impl Into<PathBuf>) -> Self {
        Self {
            data_home: data_home.into(),
        }
    }

    /// 按 XDG Base Directory 规范解析：`$XDG_DATA_HOME` 非空且为绝对路径就用它，否则 `$HOME/.local/share`。
    pub fn from_env() -> Option<Self> {
        Self::resolve(
            std::env::var("XDG_DATA_HOME").ok(),
            std::env::var("HOME").ok(),
        )
    }

    /// [`from_env`](Self::from_env) 的纯函数形态，便于测试。规范要求相对路径的 `$XDG_DATA_HOME` 视同未设置。
    fn resolve(xdg_data_home: Option<String>, home: Option<String>) -> Option<Self> {
        if let Some(data_home) = xdg_data_home.filter(|v| Path::new(v).is_absolute()) {
            return Some(Self::new(data_home));
        }
        home.map(|h| Self::new(Path::new(&h).join(".local/share")))
    }

    pub fn desktop_file(&self) -> PathBuf {
        self.data_home
            .join("applications")
            .join(format!("{}.desktop", brand::APP_ID))
    }

    pub fn icon_file(&self) -> PathBuf {
        self.data_home
            .join("icons/hicolor")
            .join(format!("{ICON_SIZE}x{ICON_SIZE}"))
            .join("apps")
            .join(format!("{ICON_NAME}.png"))
    }
}

/// `.desktop` 文件内容；`exe` 是要启动的可执行文件绝对路径。
///
/// `Comment` 的翻译直接写在文件里（`Comment[zh_CN]=` 这种是规范自带的本地化语法），
/// 由桌面环境按系统语言挑，跟应用内的界面语言无关，所以不走 i18n。
pub fn desktop_entry(exe: &Path) -> String {
    let mut out = String::new();
    out.push_str("[Desktop Entry]\n");
    out.push_str("Type=Application\n");
    out.push_str("Version=1.5\n");
    out.push_str(&format!("Name={}\n", brand::APP_NAME));
    out.push_str("Comment=Native HTTP API client\n");
    out.push_str("Comment[zh_CN]=原生 HTTP 接口调试客户端\n");
    out.push_str("Comment[ja]=ネイティブ HTTP API クライアント\n");
    out.push_str(&format!("Exec={}\n", exec_quote(exe)));
    out.push_str(&format!("Icon={ICON_NAME}\n"));
    out.push_str("Terminal=false\n");
    out.push_str("Categories=Development;Network;\n");
    out.push_str("Keywords=http;api;rest;request;\n");
    out.push_str(&format!("StartupWMClass={}\n", brand::APP_ID));
    out
}

/// 把路径转成 `Exec=` 字段里的一个参数（Desktop Entry 规范 "The Exec key"）：
/// 含保留字符时整体用双引号包住，引号内的 `\` `"` `` ` `` `$` 加反斜杠；
/// 最后把 `%` 写成 `%%`——`%f` / `%u` 之类是字段码，单个 `%` 会被启动器解释。
pub fn exec_quote(path: &Path) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    let raw = path.to_string_lossy();
    let quoted = if raw.contains(RESERVED) {
        let mut q = String::with_capacity(raw.len() + 2);
        q.push('"');
        for c in raw.chars() {
            if matches!(c, '\\' | '"' | '`' | '$') {
                q.push('\\');
            }
            q.push(c);
        }
        q.push('"');
        q
    } else {
        raw.into_owned()
    };
    quoted.replace('%', "%%")
}

pub fn is_installed(dirs: &XdgDirs) -> bool {
    dirs.desktop_file().is_file()
}

/// 写入 .desktop 与图标；内容没变的文件不重写（图标每次启动都对比，少写 170 KB）。
pub fn install(dirs: &XdgDirs, exe: &Path, icon_png: &[u8]) -> io::Result<()> {
    write_if_changed(&dirs.icon_file(), icon_png)?;
    write_if_changed(&dirs.desktop_file(), desktop_entry(exe).as_bytes())
}

/// 删除 .desktop 与图标；本来就不存在不算错。
pub fn uninstall(dirs: &XdgDirs) -> io::Result<()> {
    remove_if_exists(&dirs.desktop_file())?;
    remove_if_exists(&dirs.icon_file())
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if std::fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

// ---------------------------------------------------------------------------
// gpui 侧：只在 Linux 编译；设置页的开关读写这里的全局
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub use app_side::{init, installed, set_installed};

#[cfg(target_os = "linux")]
mod app_side {
    use gpui_kit::{App, Global};

    use super::{XdgDirs, install, is_installed, uninstall};
    use crate::assets::LOGO_PNG;

    /// 开关状态的缓存：真值是文件存在与否，但设置页每帧都读，不能每帧 stat。
    struct DesktopEntryHandle {
        installed: bool,
    }

    impl Global for DesktopEntryHandle {}

    /// 启动时装好：已经加入过菜单的，用当前可执行文件路径重写一遍——二进制挪过位置、
    /// 或应用内更新后路径没变但以防万一，都在这里自愈；文件内容没变时什么都不写。
    pub fn init(cx: &mut App) {
        let installed = match XdgDirs::from_env() {
            Some(dirs) if is_installed(&dirs) => {
                if let Err(e) =
                    std::env::current_exe().and_then(|exe| install(&dirs, &exe, LOGO_PNG))
                {
                    tracing::warn!("刷新应用菜单项失败: {e}");
                }
                true
            }
            _ => false,
        };
        cx.set_global(DesktopEntryHandle { installed });
    }

    pub fn installed(cx: &App) -> bool {
        cx.try_global::<DesktopEntryHandle>()
            .is_some_and(|h| h.installed)
    }

    /// 打开就写 .desktop + 图标，关掉就删。两个文件加起来不到 200 KB，同步做完再更新缓存；
    /// 出错只记日志，缓存按磁盘上的真实状态回填，开关不会「显示开着其实没写进去」。
    pub fn set_installed(cx: &mut App, on: bool) {
        let Some(dirs) = XdgDirs::from_env() else {
            tracing::warn!("找不到 $HOME，无法写入应用菜单项");
            return;
        };
        let result = if on {
            std::env::current_exe().and_then(|exe| install(&dirs, &exe, LOGO_PNG))
        } else {
            uninstall(&dirs)
        };
        if let Err(e) = result {
            tracing::warn!("更新应用菜单项失败: {e}");
        }
        cx.set_global(DesktopEntryHandle {
            installed: is_installed(&dirs),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_the_xdg_layout() {
        let dirs = XdgDirs::new("/home/u/.local/share");
        assert_eq!(
            dirs.desktop_file(),
            PathBuf::from("/home/u/.local/share/applications/top.catonthe.getcat.desktop")
        );
        assert_eq!(
            dirs.icon_file(),
            PathBuf::from("/home/u/.local/share/icons/hicolor/512x512/apps/getcat.png")
        );
    }

    #[test]
    fn plain_paths_are_not_quoted() {
        assert_eq!(
            exec_quote(Path::new("/opt/getcat/getcat")),
            "/opt/getcat/getcat"
        );
    }

    #[test]
    fn reserved_characters_get_quoted_and_escaped() {
        assert_eq!(
            exec_quote(Path::new("/home/my user/apps/getcat")),
            r#""/home/my user/apps/getcat""#
        );
        assert_eq!(
            exec_quote(Path::new(r#"/x/a"b$c`d\e/getcat"#)),
            r#""/x/a\"b\$c\`d\\e/getcat""#
        );
    }

    #[test]
    fn percent_is_a_field_code_and_must_be_doubled() {
        assert_eq!(
            exec_quote(Path::new("/tmp/100%/getcat")),
            "/tmp/100%%/getcat"
        );
    }

    #[test]
    fn entry_names_the_app_icon_and_window_class() {
        let entry = desktop_entry(Path::new("/home/u/.local/bin/getcat"));
        let lines: Vec<&str> = entry.lines().collect();
        assert_eq!(lines[0], "[Desktop Entry]");
        assert!(lines.contains(&"Type=Application"));
        assert!(lines.contains(&"Name=GetCat"));
        assert!(lines.contains(&"Exec=/home/u/.local/bin/getcat"));
        assert!(lines.contains(&"Icon=getcat"));
        assert!(lines.contains(&"Terminal=false"));
        assert!(lines.contains(&"StartupWMClass=top.catonthe.getcat"));
        assert!(lines.iter().any(|l| l.starts_with("Categories=")));
        assert!(lines.contains(&"Comment[zh_CN]=原生 HTTP 接口调试客户端"));
        assert!(entry.ends_with('\n'));
    }

    #[test]
    fn install_then_uninstall_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = XdgDirs::new(tmp.path());
        assert!(!is_installed(&dirs));

        install(&dirs, Path::new("/opt/getcat"), b"png-bytes").unwrap();
        assert!(is_installed(&dirs));
        assert_eq!(std::fs::read(dirs.icon_file()).unwrap(), b"png-bytes");
        assert_eq!(
            std::fs::read_to_string(dirs.desktop_file()).unwrap(),
            desktop_entry(Path::new("/opt/getcat"))
        );

        uninstall(&dirs).unwrap();
        assert!(!is_installed(&dirs));
        assert!(!dirs.icon_file().exists());
        // 再删一次不报错
        uninstall(&dirs).unwrap();
    }

    #[test]
    fn reinstall_updates_a_moved_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = XdgDirs::new(tmp.path());
        install(&dirs, Path::new("/old/getcat"), b"png").unwrap();
        install(&dirs, Path::new("/new/getcat"), b"png").unwrap();
        assert!(
            std::fs::read_to_string(dirs.desktop_file())
                .unwrap()
                .contains("Exec=/new/getcat")
        );
    }

    #[test]
    fn xdg_data_home_must_be_absolute_to_count() {
        assert_eq!(
            XdgDirs::resolve(Some("/data".into()), Some("/home/u".into())),
            Some(XdgDirs::new("/data"))
        );
        assert_eq!(
            XdgDirs::resolve(Some("relative".into()), Some("/home/u".into())),
            Some(XdgDirs::new("/home/u/.local/share"))
        );
        assert_eq!(
            XdgDirs::resolve(Some(String::new()), Some("/home/u".into())),
            Some(XdgDirs::new("/home/u/.local/share"))
        );
        assert_eq!(XdgDirs::resolve(None, None), None);
    }
}
