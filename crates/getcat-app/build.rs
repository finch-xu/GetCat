//! Windows 资源嵌入：应用图标与版本信息。其它平台上什么都不做。
//!
//! 两层 `cfg` 缺一不可，它们判断的不是同一件事：
//!
//! - `#[cfg(windows)]` 说的是**宿主**平台。build script 在宿主机上编译并运行，而
//!   `[target.'cfg(target_os = "windows")'.build-dependencies]` 里的 cfg 同样按宿主解析，
//!   所以在 macOS 上 `embed_resource` 压根不存在，引用它的代码必须被编译期剔除。
//! - `CARGO_CFG_TARGET_OS` 说的是**目标**平台。Windows 宿主交叉编译到 Linux 时不该嵌资源。
//!
//! 代价是「从 macOS 交叉编译出的 .exe 没有图标」。CI 的 Windows 产物在 windows runner 上
//! 原生编译，不受影响；Zed 的取舍也是如此。

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/brand.rs");

    #[cfg(windows)]
    win_resources::embed();
}

#[cfg(windows)]
mod win_resources {
    #[allow(dead_code)]
    mod brand {
        include!("src/brand.rs");
    }

    /// **图标的资源 ID 必须是 1。** gpui 的 Windows 窗口用
    /// `LoadImageW(module, PCWSTR(1 as _), IMAGE_ICON, ..)` 取 exe 里 ID 为 1 的图标，
    /// 设成窗口类的 `hIcon`；资源管理器同样把 ID 最小的图标当作文件图标。
    /// 一条 `1 ICON` 因此同时覆盖：文件图标、窗口左上角图标、任务栏图标。
    ///
    /// **不嵌 manifest。** gpui 已经通过它的 `windows-manifest` feature 嵌了
    /// `1 RT_MANIFEST`（gpui_platform 在 Windows target 上默认开启），所以用
    /// `manifest_optional()` 而不是 `manifest_required()`，避免两边打架。
    ///
    /// VERSIONINFO 让任务管理器与文件属性面板显示「GetCat」而不是「getcat.exe」。
    pub fn embed() {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
            return;
        }

        let icon =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/windows/getcat.ico");
        println!("cargo:rerun-if-changed={}", icon.display());
        assert!(
            icon.exists(),
            "缺少 {}；运行 scripts/gen-logo.py 生成后提交",
            icon.display()
        );
        // .rc 里是 C 风格字符串字面量，反斜杠要转义，否则 Windows 路径会被当成转义序列
        let icon = icon.to_string_lossy().replace('\\', "\\\\");

        let pkg_version = env!("CARGO_PKG_VERSION");
        // FILEVERSION 只收四段数字；预发布后缀（0.3.2-rc.1 的 rc、1）解析不出数字，按 0 补齐
        let mut parts = pkg_version
            .split(['.', '-'])
            .map(|p| p.parse::<u16>().unwrap_or(0))
            .chain(std::iter::repeat(0));
        let file_version = format!(
            "{},{},{},{}",
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
        );

        let app_name = brand::APP_NAME;
        let publisher = brand::PUBLISHER;
        let rc = format!(
            r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {file_version}
PRODUCTVERSION {file_version}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "{publisher}\0"
            VALUE "FileDescription", "{app_name}\0"
            VALUE "FileVersion", "{pkg_version}\0"
            VALUE "InternalName", "getcat\0"
            VALUE "LegalCopyright", "Copyright (c) 2026 {publisher}\0"
            VALUE "OriginalFilename", "getcat.exe\0"
            VALUE "ProductName", "{app_name}\0"
            VALUE "ProductVersion", "{pkg_version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#
        );

        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
        let rc_path = std::path::Path::new(&out_dir).join("getcat.rc");
        std::fs::write(&rc_path, rc).expect("could not write the generated .rc");

        embed_resource::compile(&rc_path, embed_resource::NONE)
            .manifest_optional()
            .expect("could not compile the Windows resources");
    }
}
