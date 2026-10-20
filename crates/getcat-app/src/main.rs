// Windows 的 release 构建不要附带控制台窗口（debug 构建保留，方便看 tracing 输出）
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assets;
/// 品牌标识：应用名、作者、仓库地址、协议。
///
/// 这些字符串会出现在标题栏、侧栏 tooltip、「关于」页与 Windows 的 exe 版本资源里，
/// 集中在一处才不会改一个漏三个。GitHub API 的参数（owner / repo）留在
/// [`crate::state::update`]，那是更新器的配置而不是展示文案。
mod brand;
mod bridge;
mod i18n;
mod state;
mod templates;
mod theme;
mod ui;

// 界面文案：crates/getcat-app/locales/app.yml；找不到当前语言的 key 时退回英文
rust_i18n::i18n!("locales", fallback = "en");

use getcat_core::store::{Layout, Loaded, Store, StoreError, load_all};
use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::assets::AppAssets;
use crate::state::settings;
use crate::state::store::{flush_on_exit, install};
use crate::state::update;
use crate::state::workspace::Workspace;

actions!(
    getcat,
    [
        SendRequest,
        NewTab,
        CloseTab,
        ToggleSidebar,
        SaveRequest,
        DuplicateTab,
        FindInResponse,
        OpenSettings
    ]
);

/// X11 的窗口图标：解码内嵌 logo。解码失败（不该发生，资源是编译期嵌入的）就没图标，不影响启动。
#[cfg(target_os = "linux")]
fn linux_window_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    match image::load_from_memory(crate::assets::LOGO_PNG) {
        Ok(img) => Some(std::sync::Arc::new(img.into_rgba8())),
        Err(e) => {
            tracing::warn!("窗口图标解码失败: {e}");
            None
        }
    }
}

fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let app = gpui_kit::platform::application().with_assets(AppAssets);
    // macOS 关掉最后一个窗口进程照常活着（gpui 默认 QuitMode::Explicit）；之后点 Dock 图标
    // 由系统回调到这里，重新开一个主窗口。其它平台关完窗口就退出，不会走到这条路
    app.on_reopen(reopen_main_window);
    app.run(|cx| {
        gpui_kit::init(cx);
        theme::install(cx);
        bridge::init(cx);

        // 上次崩溃 / 被 kill 时来不及清理的落盘目录：后台清扫 24 h 以上的 getcat-<pid>（不碰本进程的）；
        // 顺便清掉上次更新留下的安装包与旧可执行文件
        cx.background_spawn(async {
            let removed = getcat_core::body::spill::sweep_stale_session_dirs(
                getcat_core::body::spill::STALE_SESSION_AGE,
            );
            if removed > 0 {
                tracing::info!(removed, "stale spill directories removed");
            }
            update::cleanup_leftovers();
        })
        .detach();

        // 落盘响应的临时目录随进程退出一起清理（守卫已逐个删除，这里兜底异常路径）
        cx.on_app_quit(|_cx| async {
            getcat_core::body::spill::cleanup_session_dir();
        })
        .detach();

        cx.bind_keys([
            KeyBinding::new(&primary("enter"), SendRequest, None),
            KeyBinding::new(&primary("t"), NewTab, None),
            KeyBinding::new(&primary("w"), CloseTab, None),
            KeyBinding::new(&primary("b"), ToggleSidebar, None),
            KeyBinding::new(&primary("s"), SaveRequest, None),
            KeyBinding::new(&primary("f"), FindInResponse, None),
            KeyBinding::new(&primary(","), OpenSettings, None),
            // B/C 档行视图：⌘A 全选响应体（⌘C 由 gpui-component 的 Root 统一处理窗口选区）
            KeyBinding::new(
                &primary("a"),
                gpui_kit::component::input::SelectAll,
                Some(ui::body_view::LINES_KEY_CONTEXT),
            ),
        ]);

        // 退出前把当前主窗口的草稿落盘。只注册这一次、只认 MainWindow 里登记的那个工作区：
        // 若每次开窗各注册一个并强持有 Workspace，关掉再重开后，旧窗口的钩子会在退出时
        // 把过期草稿再写一遍，盖掉新窗口里的改动。
        // `flush_on_exit` 必须留在闭包的**同步**部分：gpui 的 SHUTDOWN_TIMEOUT 只有
        // 200 ms，而写入器 flush 最多等 2 s——只有在返回 future 之前执行完，
        // 退出前的最后一次落盘才来得及。
        cx.on_app_quit(|cx| {
            if let Some(workspace) = current_workspace(cx) {
                flush_on_exit(&workspace, cx);
            }
            async {}
        })
        .detach();

        // 首个窗口还在读盘时就点了 Dock：别让 reopen 再开一个
        cx.set_global(MainWindow {
            workspace: None,
            opening: true,
        });
        cx.spawn(async move |cx| {
            // 启动读取在后台线程完成（spec §9.4）；数据目录不可写时仍以只读方式恢复已有数据，只显示横幅
            let (opened, mut loaded) = cx.background_spawn(async move { open_store() }).await;
            let opened_window = cx.update(|cx| {
                install(cx, opened);
                // 设置在开窗前生效：HTTP client、编辑器字号与界面语言都要在第一帧就是用户的值
                settings::install(cx, loaded.settings.take());
                // 变量在开窗前装好：首帧的环境切换器与第一次发送都要读它
                // variables.json 在但读不出来（例如权限问题）时只改内存，免得覆盖用户的环境与 secret
                let persist = state::store::store(cx).is_none_or(|store| {
                    state::variables::should_persist(
                        &store.layout().variables_path(),
                        &loaded.errors,
                    )
                });
                state::variables::install(cx, loaded.variables.take(), persist);
                // 更新器在开窗前安装：Workspace 构造时要订阅它
                update::install(cx);
                #[cfg(target_os = "linux")]
                state::desktop_entry::init(cx);
                open_main_window(loaded, cx)
            });
            cx.update(|cx| cx.global_mut::<MainWindow>().opening = false);
            // AsyncApp::update 在本版本直接返回闭包结果（不再包一层 Result）
            match opened_window {
                Ok(_) => cx.update(|cx| {
                    cx.activate(true);
                    // 启动后延迟几秒检查一次新版本（设置可关；开发构建默认不查）
                    update::schedule_launch_check(cx);
                }),
                // 开窗失败没有任何可交互的界面，只记日志会留下一个无窗僵尸进程：显式退出
                Err(e) => {
                    tracing::error!("failed to open window: {e}");
                    cx.update(|cx| cx.quit());
                }
            }
        })
        .detach();
    });
}

/// 主窗口登记处（全局）。macOS 上窗口可以关掉再从 Dock 重开，所以退出钩子与重开逻辑
/// 都要知道「现在开着的是哪个工作区」，而不是捕获某一次开窗时的实体。
#[derive(Default)]
struct MainWindow {
    /// 当前主窗口的工作区；关窗时清空（关窗那一刻已经落过盘，退出时不该再写旧窗口的草稿）
    workspace: Option<WeakEntity<Workspace>>,
    /// 正在后台读盘、窗口还没开出来：这期间再点 Dock 不重复开窗
    opening: bool,
}

impl Global for MainWindow {}

fn current_workspace(cx: &App) -> Option<Entity<Workspace>> {
    cx.try_global::<MainWindow>()?.workspace.as_ref()?.upgrade()
}

/// 客户端自绘标题栏（spec §7.2）：TitleBar::window_options() 提供透明 titlebar、红绿灯位置与
/// app_owns_titlebar_drag；Linux 额外申请客户端装饰（与 gpui-component story 同款），
/// 得不到时 gpui 回退到服务端装饰、TitleBar 自动不画控制按钮。
fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
        window_min_size: Some(size(px(800.), px(520.))),
        #[cfg(target_os = "linux")]
        window_background: WindowBackgroundAppearance::Transparent,
        #[cfg(target_os = "linux")]
        window_decorations: Some(WindowDecorations::Client),
        // Linux 桌面靠 app_id 把窗口和 .desktop 对上（Wayland 的图标只能来自那里，见
        // state/desktop_entry.rs）；X11 没有这条路，图标由 _NET_WM_ICON 直接给
        #[cfg(target_os = "linux")]
        app_id: Some(crate::brand::APP_ID.to_string()),
        #[cfg(target_os = "linux")]
        icon: linux_window_icon(),
        ..TitleBar::window_options()
    }
}

/// 开主窗口：恢复工作区、登记到 [`MainWindow`]、关窗前落盘。启动与 Dock 重开共用。
fn open_main_window(loaded: Loaded, cx: &mut App) -> anyhow::Result<AnyWindowHandle> {
    let options = window_options(cx);
    // gpui_kit::open_window 替我们把内容包进窗口根 Root（Dialog / Sheet / 通知层由它托管）
    let (window, workspace) = gpui_kit::open_window(options, cx, |window, cx| {
        // TitlebarOptions.title 为 None（标题由 TitleBar 自绘）；OS 层的窗口标题给 Dock / 任务栏 / 屏幕阅读器
        window.set_window_title(crate::brand::APP_NAME);
        let workspace = cx.new(|cx| Workspace::restore(loaded, window, cx));
        // 关窗先把每个 Tab 的草稿快照投递出去，再等待写入线程清空队列（≤ 2 s）；
        // 之后注销登记，Dock 重开时从磁盘恢复出来的就是刚落盘的这份
        window.on_window_should_close(cx, {
            let workspace = workspace.clone();
            move |_, cx| {
                flush_on_exit(&workspace, cx);
                cx.default_global::<MainWindow>().workspace = None;
                true
            }
        });
        workspace
    })?;
    cx.default_global::<MainWindow>().workspace = Some(workspace.downgrade());
    Ok(window)
}

/// Dock 图标被点击且没有可见窗口时（macOS `applicationShouldHandleReopen`）。
fn reopen_main_window(cx: &mut App) {
    // 窗口还在（例如最小化了）：拉回前台即可，不开第二个
    if let Some(window) = cx.windows().into_iter().next() {
        window
            .update(cx, |_, window, _| window.activate_window())
            .ok();
        return;
    }
    let main = cx.default_global::<MainWindow>();
    if main.opening {
        return;
    }
    main.opening = true;
    // 与冷启动一样从磁盘恢复：关窗时草稿与布局已经落盘，重开的效果等同于退出后再启动
    cx.spawn(async move |cx| {
        let loaded = cx
            .background_spawn(async move {
                Store::default_root()
                    .map(|root| load_all(&Layout::new(root)))
                    .unwrap_or_default()
            })
            .await;
        cx.update(|cx| {
            cx.global_mut::<MainWindow>().opening = false;
            match open_main_window(loaded, cx) {
                Ok(_) => cx.activate(true),
                Err(e) => tracing::error!("failed to reopen window: {e}"),
            }
        });
    })
    .detach();
}

/// 后台线程：定位数据目录 → 读取全部文件 → 打开写入器。
/// 读取放在 `Store::open` 之前：目录不可写时也能恢复已有数据（只读模式）。
fn open_store() -> (Result<Store, StoreError>, Loaded) {
    let Some(root) = Store::default_root() else {
        return (Err(StoreError::NoDataDir), Loaded::default());
    };
    let loaded = load_all(&Layout::new(root.clone()));
    (Store::open(root), loaded)
}
