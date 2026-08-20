mod bridge;
mod state;
mod ui;

use gpui::*;
use gpui_component::Root;
use gpui_component_assets::Assets;

use crate::state::workspace::Workspace;

actions!(getcat, [SendRequest, NewTab, CloseTab, ToggleSidebar]);

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
            cx.open_window(options, |window, cx| {
                let workspace = cx.new(|cx| Workspace::new(window, cx));
                cx.new(|cx| Root::new(workspace, window, cx))
            })
            .expect("failed to open window");
            cx.update(|cx| cx.activate(true));
        })
        .detach();
    });
}
