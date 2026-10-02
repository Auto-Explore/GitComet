//! Windows extensions open for their own content, such as a popped-out
//! pane. They take native decorations like the focused difftool, share the
//! main window's app id, and close with the window that opened them.

use super::*;
use gitcomet_extension_api::{
    HostError, OnWindowClosed, PopOutWindow, WindowContent, WindowHost, host::PopOutImpl,
};
use gpui::{TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions};
use std::rc::Rc;

const POP_OUT_DEFAULT_WIDTH_PX: f32 = 960.0;
const POP_OUT_DEFAULT_HEIGHT_PX: f32 = 680.0;
const POP_OUT_MIN_WIDTH_PX: f32 = 360.0;
const POP_OUT_MIN_HEIGHT_PX: f32 = 240.0;

pub(crate) struct PopOutView {
    host: WindowHost,
    content: gpui::AnyView,
    on_closed: Option<OnWindowClosed>,
    _main_closed: gpui::Subscription,
    _released: gpui::Subscription,
}

impl Render for PopOutView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.host.theme(cx);
        div()
            .id("extension_pop_out")
            .debug_selector(|| "extension_pop_out".to_string())
            .size_full()
            .bg(theme.colors.surface.canvas)
            .text_color(theme.colors.foreground.primary)
            .child(self.content.clone())
    }
}

/// Opens `content` in a window of its own for the extension host of window
/// `main`.
pub(in crate::view) fn open(
    host: WindowHost,
    main: gpui::WindowId,
    title: SharedString,
    content: WindowContent,
    on_closed: OnWindowClosed,
    cx: &mut App,
) -> Result<PopOutWindow, HostError> {
    let percent = crate::ui_scale::percent_for_window(cx, main);
    let bounds = Bounds::centered(
        None,
        crate::ui_scale::design_size_from_percent(
            POP_OUT_DEFAULT_WIDTH_PX,
            POP_OUT_DEFAULT_HEIGHT_PX,
            percent,
        ),
        cx,
    );
    let identity = gitcomet_core::identity::current();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(crate::ui_scale::design_size_from_percent(
            POP_OUT_MIN_WIDTH_PX,
            POP_OUT_MIN_HEIGHT_PX,
            percent,
        )),
        titlebar: Some(TitlebarOptions {
            title: Some(format!("{} — {title}", identity.display_name()).into()),
            appears_transparent: false,
            traffic_light_position: Some(chrome::macos_traffic_light_position()),
        }),
        app_id: Some(identity.window_app_id(gitcomet_core::identity::WindowKind::Main)),
        window_decorations: Some(WindowDecorations::Server),
        icon: crate::assets::window_icon(),
        is_movable: true,
        is_resizable: true,
        ..Default::default()
    };
    let handle = cx
        .open_window(options, move |window, cx| {
            // Renders at its main window's zoom, and changes with it.
            crate::ui_scale::follow_window(cx, window.window_handle().window_id(), main);
            crate::ui_scale::apply_to_window(window, percent);
            let popped = window.window_handle();
            let content = content(window, cx);
            cx.new(|cx| PopOutView {
                host,
                content,
                on_closed: Some(on_closed),
                _main_closed: cx.on_window_closed(move |cx, closed| {
                    if closed == main {
                        let _ = popped.update(cx, |_, window, _| window.remove_window());
                    }
                }),
                _released: cx.on_release(|this: &mut PopOutView, cx| {
                    if let Some(on_closed) = this.on_closed.take() {
                        on_closed(cx);
                    }
                }),
            })
        })
        .map_err(|_| HostError::Unsupported)?;
    Ok(PopOutWindow::new(Rc::new(HostedPopOut {
        handle: handle.into(),
    })))
}

struct HostedPopOut {
    handle: gpui::AnyWindowHandle,
}

impl PopOutImpl for HostedPopOut {
    fn close(&self, cx: &mut App) {
        let _ = self
            .handle
            .update(cx, |_, window, _| window.remove_window());
    }

    fn is_open(&self, cx: &App) -> bool {
        cx.windows().contains(&self.handle)
    }
}
