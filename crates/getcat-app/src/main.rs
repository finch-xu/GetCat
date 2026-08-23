// Windows 的 release 构建不要附带控制台窗口（debug 构建保留，方便看 tracing 输出）
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assets;
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
use gpui::*;
use gpui_component::{Root, TitleBar};

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

    gpui_platform::application()
        .with_assets(AppAssets)
        .run(|cx| {
            gpui_component::init(cx);
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
            ]);

            // 客户端自绘标题栏（spec §7.2）：TitleBar::window_options() 提供透明 titlebar、红绿灯位置与
            // app_owns_titlebar_drag；Linux 额外申请客户端装饰（与 gpui-component story 同款），
            // 得不到时 gpui 回退到服务端装饰、TitleBar 自动不画控制按钮。
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
                window_min_size: Some(size(px(800.), px(520.))),
                #[cfg(target_os = "linux")]
                window_background: WindowBackgroundAppearance::Transparent,
                #[cfg(target_os = "linux")]
                window_decorations: Some(WindowDecorations::Client),
                ..TitleBar::window_options()
            };
            cx.spawn(async move |cx| {
                // 启动读取在后台线程完成（spec §9.4）；数据目录不可写时仍以只读方式恢复已有数据，只显示横幅
                let (opened, mut loaded) = cx.background_spawn(async move { open_store() }).await;
                let opened_window = cx.update(|cx| {
                    install(cx, opened);
                    // 设置在开窗前生效：HTTP client、编辑器字号与界面语言都要在第一帧就是用户的值
                    settings::install(cx, loaded.settings.take());
                    // 更新器在开窗前安装：Workspace 构造时要订阅它
                    update::install(cx);
                    cx.open_window(options, |window, cx| {
                        // TitlebarOptions.title 为 None（标题由 TitleBar 自绘）；OS 层的窗口标题给 Dock / 任务栏 / 屏幕阅读器
                        window.set_window_title(crate::brand::APP_NAME);
                        let workspace = cx.new(|cx| Workspace::restore(loaded, window, cx));
                        // 关窗与退出都先把每个 Tab 的草稿快照投递出去，再等待写入线程清空队列（≤ 2 s）
                        window.on_window_should_close(cx, {
                            let workspace = workspace.clone();
                            move |_, cx| {
                                flush_on_exit(&workspace, cx);
                                true
                            }
                        });
                        // `flush_on_exit` 必须留在闭包的**同步**部分：gpui 的 SHUTDOWN_TIMEOUT 只有
                        // 200 ms，而写入器 flush 最多等 2 s——只有在返回 future 之前执行完，
                        // 退出前的最后一次落盘才来得及。
                        cx.on_app_quit({
                            let workspace = workspace.clone();
                            move |cx| {
                                flush_on_exit(&workspace, cx);
                                async {}
                            }
                        })
                        .detach();
                        cx.new(|cx| Root::new(workspace, window, cx))
                    })
                });
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

/// 后台线程：定位数据目录 → 读取全部文件 → 打开写入器。
/// 读取放在 `Store::open` 之前：目录不可写时也能恢复已有数据（只读模式）。
fn open_store() -> (Result<Store, StoreError>, Loaded) {
    let Some(root) = Store::default_root() else {
        return (Err(StoreError::NoDataDir), Loaded::default());
    };
    let loaded = load_all(&Layout::new(root.clone()));
    (Store::open(root), loaded)
}
