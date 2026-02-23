use super::*;
use gpui::ObjectFit;

pub(super) const CLIENT_SIDE_DECORATION_INSET: Pixels = px(10.0);
pub(super) const TITLE_BAR_HEIGHT: Pixels = px(34.0);
pub(super) const WINDOW_OUTLINE_RGBA: u32 = 0x5a5d63ff;

pub(super) struct TitleBarView {
    theme: AppTheme,
    root_view: WeakEntity<GitGpuiView>,
    title_should_move: bool,
    app_menu_open: bool,
}

fn window_top_left_corner(window: &Window) -> Point<Pixels> {
    let inset = window.client_inset().unwrap_or(px(0.0));
    match window.window_decorations() {
        Decorations::Client { tiling } => point(
            if tiling.left { px(0.0) } else { inset },
            if tiling.top { px(0.0) } else { inset },
        ),
        Decorations::Server => point(px(0.0), px(0.0)),
    }
}

fn titlebar_control_icon(path: &'static str, color: gpui::Rgba) -> gpui::Svg {
    gpui::svg()
        .path(path)
        .w(px(16.0))
        .h(px(16.0))
        .text_color(color)
}

fn titlebar_app_icon(theme: AppTheme) -> AnyElement {
    gpui::image_cache(gpui::retain_all("titlebar_icon_cache"))
        .child(
            div()
                .id("titlebar_app_icon")
                .size(px(16.0))
                .rounded(px(4.0))
                .bg(with_alpha(
                    theme.colors.text,
                    if theme.is_dark { 0.12 } else { 0.08 },
                ))
                .overflow_hidden()
                .child(
                    gpui::img("gitgpui_logo_window.svg")
                        .size(px(16.0))
                        .object_fit(ObjectFit::Contain)
                        .with_fallback(move || {
                            gpui::svg()
                                .path("icons/gitgpui_mark.svg")
                                .w(px(16.0))
                                .h(px(16.0))
                                .text_color(theme.colors.accent)
                                .into_any_element()
                        }),
                ),
        )
        .into_any_element()
}

fn titlebar_control_button(
    theme: AppTheme,
    id: &'static str,
    icon: gpui::Svg,
    hover_bg: gpui::Rgba,
    active_bg: gpui::Rgba,
) -> gpui::Div {
    div()
        .h_full()
        .w(px(46.0))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .id(id)
                .size(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(theme.radii.pill))
                .cursor(CursorStyle::PointingHand)
                .hover(move |s| s.bg(hover_bg))
                .active(move |s| s.bg(active_bg))
                .child(icon),
        )
}

fn mix(mut a: gpui::Rgba, b: gpui::Rgba, t: f32) -> gpui::Rgba {
    let t = t.clamp(0.0, 1.0);
    a.r = a.r + (b.r - a.r) * t;
    a.g = a.g + (b.g - a.g) * t;
    a.b = a.b + (b.b - a.b) * t;
    a.a = a.a + (b.a - a.a) * t;
    a
}

fn lighten(color: gpui::Rgba, amount: f32) -> gpui::Rgba {
    mix(color, gpui::rgba(0xFFFFFFFF), amount)
}

pub(super) fn cursor_style_for_resize_edge(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

pub(super) fn resize_edge(
    pos: Point<Pixels>,
    inset: Pixels,
    window_size: Size<Pixels>,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    let bounds = Bounds::new(Point::default(), window_size).inset(inset * 1.5);
    if bounds.contains(&pos) {
        return None;
    }

    let corner_size = size(inset * 1.5, inset * 1.5);
    let top_left_bounds = Bounds::new(Point::new(px(0.0), px(0.0)), corner_size);
    if !tiling.top && top_left_bounds.contains(&pos) {
        return Some(ResizeEdge::TopLeft);
    }

    let top_right_bounds = Bounds::new(
        Point::new(window_size.width - corner_size.width, px(0.0)),
        corner_size,
    );
    if !tiling.top && top_right_bounds.contains(&pos) {
        return Some(ResizeEdge::TopRight);
    }

    let bottom_left_bounds = Bounds::new(
        Point::new(px(0.0), window_size.height - corner_size.height),
        corner_size,
    );
    if !tiling.bottom && bottom_left_bounds.contains(&pos) {
        return Some(ResizeEdge::BottomLeft);
    }

    let bottom_right_bounds = Bounds::new(
        Point::new(
            window_size.width - corner_size.width,
            window_size.height - corner_size.height,
        ),
        corner_size,
    );
    if !tiling.bottom && bottom_right_bounds.contains(&pos) {
        return Some(ResizeEdge::BottomRight);
    }

    if !tiling.top && pos.y < inset {
        Some(ResizeEdge::Top)
    } else if !tiling.bottom && pos.y > window_size.height - inset {
        Some(ResizeEdge::Bottom)
    } else if !tiling.left && pos.x < inset {
        Some(ResizeEdge::Left)
    } else if !tiling.right && pos.x > window_size.width - inset {
        Some(ResizeEdge::Right)
    } else {
        None
    }
}

impl TitleBarView {
    pub(super) fn new(theme: AppTheme, root_view: WeakEntity<GitGpuiView>) -> Self {
        Self {
            theme,
            root_view,
            title_should_move: false,
            app_menu_open: false,
        }
    }

    pub(super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(super) fn set_app_menu_open(&mut self, open: bool, cx: &mut gpui::Context<Self>) {
        if self.app_menu_open == open {
            return;
        }
        self.app_menu_open = open;
        cx.notify();
    }

    fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_at(kind, anchor, window, cx);
        });
    }
}

impl Render for TitleBarView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let app_menu_open = self.app_menu_open;
        let app_menu_open_bg =
            with_alpha(theme.colors.accent, if theme.is_dark { 0.30 } else { 0.24 });
        let app_menu_open_hover_bg =
            with_alpha(theme.colors.accent, if theme.is_dark { 0.40 } else { 0.32 });
        let bar_bg = if window.is_window_active() {
            lighten(
                theme.colors.surface_bg,
                if theme.is_dark { 0.06 } else { 0.03 },
            )
        } else {
            theme.colors.surface_bg
        };
        let bar_border = if window.is_window_active() {
            theme.colors.border
        } else {
            with_alpha(theme.colors.border, 0.7)
        };

        let app_icon = div()
            .id("app_icon")
            .h_full()
            .pl_2()
            .pr_1()
            .flex()
            .items_center()
            .child(titlebar_app_icon(theme));

        let hamburger = div()
            .id("app_menu")
            .debug_selector(|| "app_menu".to_string())
            .h_full()
            .w(px(44.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::PointingHand)
            .child(
                div()
                    .id("app_menu_btn")
                    .size(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(theme.radii.pill))
                    .when(app_menu_open, move |s| s.bg(app_menu_open_bg))
                    .hover(move |s| {
                        if app_menu_open {
                            s.bg(app_menu_open_hover_bg)
                        } else {
                            s.bg(theme.colors.hover)
                        }
                    })
                    .active(move |s| {
                        if app_menu_open {
                            s.bg(app_menu_open_hover_bg)
                        } else {
                            s.bg(theme.colors.active)
                        }
                    })
                    .child(titlebar_control_icon("icons/menu.svg", theme.colors.accent)),
            )
            .on_click(cx.listener(|this, _e: &ClickEvent, window, cx| {
                this.set_app_menu_open(true, cx);
                let anchor = window_top_left_corner(window);
                this.open_popover_at(PopoverKind::AppMenu, anchor, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|_this, _e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.show_window_menu(window_top_left_corner(window));
                }),
            );

        let drag_region = div()
            .id("title_drag")
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .min_w(px(0.0))
            .px_2()
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.title_should_move = true;
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.title_should_move = false;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.title_should_move = false;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, _e, window, _cx| {
                if this.title_should_move {
                    this.title_should_move = false;
                    window.start_window_move();
                }
            }))
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .whitespace_nowrap()
                    .child("GitGpui"),
            );

        let min_hover = with_alpha(theme.colors.text, if theme.is_dark { 0.10 } else { 0.08 });
        let min_active = with_alpha(theme.colors.text, if theme.is_dark { 0.16 } else { 0.12 });
        let min = titlebar_control_button(
            theme,
            "win_min_btn",
            titlebar_control_icon("icons/generic_minimize.svg", theme.colors.accent),
            min_hover,
            min_active,
        )
        .id("win_min")
        .window_control_area(WindowControlArea::Min)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            window.minimize_window();
        }));

        let max_icon = if window.is_maximized() {
            "icons/generic_restore.svg"
        } else {
            "icons/generic_maximize.svg"
        };
        let max_hover = with_alpha(theme.colors.text, if theme.is_dark { 0.10 } else { 0.08 });
        let max_active = with_alpha(theme.colors.text, if theme.is_dark { 0.16 } else { 0.12 });
        let max = titlebar_control_button(
            theme,
            "win_max_btn",
            titlebar_control_icon(max_icon, theme.colors.accent),
            max_hover,
            max_active,
        )
        .id("win_max")
        .window_control_area(WindowControlArea::Max)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            window.zoom_window();
        }));

        let close_hover = with_alpha(theme.colors.danger, if theme.is_dark { 0.45 } else { 0.28 });
        let close_active = with_alpha(theme.colors.danger, if theme.is_dark { 0.60 } else { 0.40 });
        let close = titlebar_control_button(
            theme,
            "win_close_btn",
            titlebar_control_icon("icons/generic_close.svg", theme.colors.danger),
            close_hover,
            close_active,
        )
        .id("win_close")
        .window_control_area(WindowControlArea::Close)
        .on_click(cx.listener(|_this, _e: &ClickEvent, _window, cx| {
            cx.stop_propagation();
            cx.quit();
        }));

        div()
            .id("title_bar")
            .flex()
            .items_center()
            .h(TITLE_BAR_HEIGHT)
            .w_full()
            .bg(bar_bg)
            .border_b_1()
            .border_color(bar_border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .gap_1()
                    .child(app_icon)
                    .child(hamburger),
            )
            .child(drag_region)
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(min)
                    .child(max)
                    .child(close),
            )
            .into_any_element()
    }
}

pub(crate) fn window_frame(
    theme: AppTheme,
    decorations: Decorations,
    content: AnyElement,
) -> AnyElement {
    let mut outer = div()
        .id("window_frame")
        .size_full()
        .bg(gpui::rgba(0x00000000));

    if let Decorations::Client { tiling } = decorations {
        outer = outer
            .when(!tiling.top, |d| d.pt(CLIENT_SIDE_DECORATION_INSET))
            .when(!tiling.bottom, |d| d.pb(CLIENT_SIDE_DECORATION_INSET))
            .when(!tiling.left, |d| d.pl(CLIENT_SIDE_DECORATION_INSET))
            .when(!tiling.right, |d| d.pr(CLIENT_SIDE_DECORATION_INSET));
    }

    let inner = div()
        .id("window_surface")
        .size_full()
        .bg(theme.colors.window_bg)
        .border_1()
        .border_color(gpui::rgba(WINDOW_OUTLINE_RGBA))
        .rounded(px(theme.radii.panel))
        .shadow_lg()
        .overflow_hidden()
        .child(content);

    outer.child(inner).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titlebar_buttons_do_not_double_set_hover_style() {
        let theme = AppTheme::zed_ayu_dark();
        assert!(
            std::panic::catch_unwind(|| {
                let _ = titlebar_control_button(
                    theme,
                    "test_btn_1",
                    titlebar_control_icon("icons/generic_minimize.svg", theme.colors.accent),
                    theme.colors.hover,
                    theme.colors.active,
                );
            })
            .is_ok()
        );
        assert!(
            std::panic::catch_unwind(|| {
                let _ = titlebar_control_button(
                    theme,
                    "test_btn_2",
                    titlebar_control_icon("icons/generic_close.svg", theme.colors.danger),
                    with_alpha(theme.colors.danger, 0.25),
                    with_alpha(theme.colors.danger, 0.35),
                );
            })
            .is_ok()
        );
    }
}
