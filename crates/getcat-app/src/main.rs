use gpui::*;
use gpui_component::{ActiveTheme, Root, v_flex};
use gpui_component_assets::Assets;

struct Hello;

impl Render for Hello {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("GetCat")
    }
}

fn main() {
    gpui_platform::application().with_assets(Assets).run(|cx| {
        gpui_component::init(cx);
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
                let view = cx.new(|_| Hello);
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
            cx.update(|cx| cx.activate(true));
        })
        .detach();
    });
}
