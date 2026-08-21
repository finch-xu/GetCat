mod bridge;
mod state;
mod ui;

use getcat_core::store::{Layout, Loaded, Store, StoreError, load_all};
use gpui::*;
use gpui_component::Root;
use gpui_component_assets::Assets;

use crate::state::store::{flush_on_exit, install};
use crate::state::workspace::Workspace;

actions!(
    getcat,
    [SendRequest, NewTab, CloseTab, ToggleSidebar, SaveRequest]
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

    gpui_platform::application().with_assets(Assets).run(|cx| {
        gpui_component::init(cx);
        bridge::init(cx);

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
        ]);

        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("GetCat".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(820.)), cx)),
            ..Default::default()
        };
        cx.spawn(async move |cx| {
            // 启动读取在后台线程完成（spec §9.4）；数据目录不可写时仍以只读方式恢复已有数据，只显示横幅
            let (opened, loaded) = cx.background_spawn(async move { open_store() }).await;
            let opened_window = cx.update(|cx| {
                install(cx, opened);
                cx.open_window(options, |window, cx| {
                    let workspace = cx.new(|cx| Workspace::restore(loaded, window, cx));
                    // 关窗与退出都先把每个 Tab 的草稿快照投递出去，再等待写入线程清空队列（≤ 2 s）
                    window.on_window_should_close(cx, {
                        let workspace = workspace.clone();
                        move |_, cx| {
                            flush_on_exit(&workspace, cx);
                            true
                        }
                    });
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
                Ok(_) => cx.update(|cx| cx.activate(true)),
                Err(e) => tracing::error!("failed to open window: {e}"),
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
