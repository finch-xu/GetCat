// 品牌标识常量。模块文档挂在 `main.rs` 的 `mod brand;` 上，不在本文件里 ——
//
// 本文件同时被 `build.rs` 用 `include!` 拉进 `mod brand { .. }`（build script 不能
// `use` 所在 crate 的模块），而 `include!` 展开出来的内容里不允许出现 `//!`：写了会
// 以 E0753 打断 build script 的编译。又因为那段 include 挂在 `#[cfg(windows)]` 下、
// cfg 按**宿主**求值，macOS / Linux 上根本不展开，错误只在 Windows runner 上现形。
// 所以这里的注释一律用 `//` 或 `///`，不要用 `//!`。
//
// 同样的原因，这里只放 `const`，不要引入任何依赖或 `use`。两个编译单元用到的常量不是
// 同一批，各自都会有一部分「没人用」——因此下面用 `allow(dead_code)` 而不是 `expect`：
// `expect` 在真的被用到的那一侧会反过来报「预期未兑现」。

/// 应用显示名。OS 层窗口标题、标题栏、「关于」页共用。
pub const APP_NAME: &str = "GetCat";

/// 反向域名形式的应用标识。Linux 桌面靠它把窗口和 `.desktop` 文件对上：
/// Wayland 下作为 `xdg_toplevel` 的 app_id，X11 下作为 `WM_CLASS`，桌面环境再按
/// `<APP_ID>.desktop` 找到应用名与图标——**没有这条对应，Wayland 下窗口就没有图标**
/// （协议里根本没有「窗口图标」，图标只能来自 .desktop）。与 `scripts/ci/gen-oss-manifest.py`
/// 里镜像清单的 `app_id` 是同一个值。
#[allow(dead_code)]
pub const APP_ID: &str = "top.catonthe.getcat";

/// 「关于」页的作者署名。只在应用内渲染（gpui 全程 UTF-8），不要送进 Windows 资源。
pub const AUTHOR: &str = "虚拟世界的懒猫 (@finch-xu)";

/// 发布者，用于 Windows exe 的 VERSIONINFO 与 MSI 的「发布者」栏。只被 `build.rs` 读。
///
/// **必须是纯 ASCII**：`.rc` 由 `rc.exe` 按系统 ANSI 代码页解析、MSI 的摘要信息流同样
/// 受代码页限制，中文在那两处会变成乱码。展示用的中文署名见 [`AUTHOR`]。
#[allow(dead_code)]
pub const PUBLISHER: &str = "finch-xu";

/// 项目主页；「关于」页的链接指向这里（发布页在 `update::RELEASES_URL`）。
pub const REPO_URL: &str = "https://github.com/finch-xu/GetCat";

/// 开源协议，与仓库根的 `LICENSE` 一致。
pub const LICENSE: &str = "Apache-2.0";
