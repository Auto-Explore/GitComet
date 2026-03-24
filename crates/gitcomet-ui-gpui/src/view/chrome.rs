use super::*;

pub(super) const CLIENT_SIDE_DECORATION_INSET: Pixels = px(10.0);
pub(super) const TITLE_BAR_HEIGHT: Pixels = px(34.0);
pub(super) const WINDOW_OUTLINE_RGBA: u32 = 0x5a5d63ff;
const MACOS_TRAFFIC_LIGHTS_SAFE_INSET: Pixels = px(78.0);

pub(super) struct TitleBarView {
    theme: AppTheme,
    root_view: WeakEntity<GitCometView>,
    title_drag_state: TitleBarDragState,
    app_menu_open: bool,
}

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub(super) struct TitleBarDragState {
    should_move: bool,
}

impl TitleBarDragState {
    pub(super) fn on_left_mouse_down(&mut self, click_count: usize) {
        self.should_move = click_count < 2;
    }

    pub(super) fn clear(&mut self) {
        self.should_move = false;
    }

    pub(super) fn take_move_request(&mut self) -> bool {
        let should_move = self.should_move;
        self.should_move = false;
        should_move
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum TitleBarDoubleClickAction {
    PlatformDefault,
    ToggleZoom,
}

pub(super) fn should_handle_titlebar_double_click(
    click_count: usize,
    standard_click: bool,
) -> bool {
    standard_click && click_count == 2
}

fn titlebar_double_click_action() -> TitleBarDoubleClickAction {
    if cfg!(target_os = "macos") {
        TitleBarDoubleClickAction::PlatformDefault
    } else {
        TitleBarDoubleClickAction::ToggleZoom
    }
}

pub(super) fn handle_titlebar_double_click(window: &mut Window) {
    match titlebar_double_click_action() {
        TitleBarDoubleClickAction::PlatformDefault => window.titlebar_double_click(),
        TitleBarDoubleClickAction::ToggleZoom => window.zoom_window(),
    }
}

pub(in crate::view) fn window_top_left_corner(window: &Window) -> Point<Pixels> {
    let inset = window.client_inset().unwrap_or(px(0.0));
    match window.window_decorations() {
        Decorations::Client { tiling } => point(
            if tiling.left { px(0.0) } else { inset },
            if tiling.top { px(0.0) } else { inset },
        ),
        Decorations::Server => point(px(0.0), px(0.0)),
    }
}

pub(super) fn titlebar_control_icon(path: &'static str, color: gpui::Rgba) -> gpui::Svg {
    svg_icon(path, color, px(16.0))
}

fn titlebar_app_icon(theme: AppTheme) -> AnyElement {
    gpui::image_cache(gpui::retain_all("titlebar_icon_cache"))
        .child(
            div().id("titlebar_app_icon").size(px(16.0)).child(
                gpui::img("gitcomet-window-icon.png")
                    .size(px(16.0))
                    .with_fallback(move || {
                        svg_icon("icons/gitcomet_mark.svg", theme.colors.accent, px(16.0))
                            .into_any_element()
                    }),
            ),
        )
        .into_any_element()
}

pub(super) fn titlebar_control_button(
    theme: AppTheme,
    id: &'static str,
    icon: gpui::Svg,
    hover_bg: gpui::Rgba,
    active_bg: gpui::Rgba,
) -> gpui::Div {
    const TITLEBAR_CONTROL_HITBOX_WIDTH: Pixels = px(32.0);
    const TITLEBAR_CONTROL_VISUAL_SIZE: Pixels = px(26.0);

    div()
        .h_full()
        .w(TITLEBAR_CONTROL_HITBOX_WIDTH)
        .flex()
        .items_center()
        .justify_center()
        .cursor(CursorStyle::PointingHand)
        .child(
            div()
                .id(id)
                .h_full()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(theme.radii.pill))
                .hover(move |s| s.bg(hover_bg))
                .active(move |s| s.bg(active_bg))
                .child(
                    div()
                        .size(TITLEBAR_CONTROL_VISUAL_SIZE)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(theme.radii.pill))
                        .child(icon),
                ),
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

fn window_frame_visual_inset() -> Pixels {
    if cfg!(target_os = "macos") {
        px(0.0)
    } else {
        CLIENT_SIDE_DECORATION_INSET
    }
}

fn window_frame_outline_color(theme: AppTheme) -> gpui::Rgba {
    if cfg!(target_os = "macos") {
        with_alpha(theme.colors.border, if theme.is_dark { 0.96 } else { 0.90 })
    } else {
        gpui::rgba(WINDOW_OUTLINE_RGBA)
    }
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
    pub(super) fn new(theme: AppTheme, root_view: WeakEntity<GitCometView>) -> Self {
        Self {
            theme,
            root_view,
            title_drag_state: TitleBarDragState::default(),
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

    fn set_tooltip_text_if_changed(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.root_view
            .update(cx, |root, cx| {
                root.tooltip_host
                    .update(cx, |host, cx| host.set_tooltip_text_if_changed(next, cx))
            })
            .unwrap_or(false)
    }

    fn clear_tooltip_if_matches(
        &mut self,
        tooltip: &SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let tooltip = tooltip.clone();
        self.root_view
            .update(cx, |root, cx| {
                root.tooltip_host
                    .update(cx, |host, cx| host.clear_tooltip_if_matches(&tooltip, cx))
            })
            .unwrap_or(false)
    }
}

impl Render for TitleBarView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let is_macos = cfg!(target_os = "macos");
        let app_menu_open = self.app_menu_open;
        let app_menu_open_bg =
            with_alpha(theme.colors.accent, if theme.is_dark { 0.30 } else { 0.24 });
        let app_menu_open_active_bg =
            with_alpha(theme.colors.accent, if theme.is_dark { 0.48 } else { 0.38 });
        let app_menu_hover_bg =
            with_alpha(theme.colors.text, if theme.is_dark { 0.10 } else { 0.08 });
        let app_menu_active_bg =
            with_alpha(theme.colors.text, if theme.is_dark { 0.16 } else { 0.12 });
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

        let menu_toggle = div()
            .id("app_menu")
            .debug_selector(|| "app_menu".to_string())
            .h_full()
            .pl_1()
            .flex()
            .items_center()
            .cursor(CursorStyle::PointingHand)
            .child(
                div()
                    .id("app_menu_btn")
                    .h(px(26.0))
                    .w(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(theme.radii.pill))
                    .when(app_menu_open, move |s| s.bg(app_menu_open_bg))
                    .hover(move |s| {
                        if app_menu_open {
                            s.bg(app_menu_open_bg)
                        } else {
                            s.bg(app_menu_hover_bg)
                        }
                    })
                    .active(move |s| {
                        if app_menu_open {
                            s.bg(app_menu_open_active_bg)
                        } else {
                            s.bg(app_menu_active_bg)
                        }
                    })
                    .child(svg_icon("icons/menu.svg", theme.colors.text, px(14.0))),
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

        let windows_brand = div()
            .id("titlebar_brand")
            .debug_selector(|| "titlebar_brand".to_string())
            .h_full()
            .flex()
            .items_center()
            .child(
                div()
                    .h(px(26.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(titlebar_app_icon(theme))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.colors.text)
                            .whitespace_nowrap()
                            .child("GITCOMET"),
                    ),
            );

        let drag_region = div()
            .id("title_drag")
            .debug_selector(|| "titlebar_drag".to_string())
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .min_w(px(0.0))
            .px_2()
            .window_control_area(WindowControlArea::Drag)
            .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                if !should_handle_titlebar_double_click(e.click_count(), e.standard_click()) {
                    return;
                }
                this.title_drag_state.clear();
                cx.stop_propagation();
                handle_titlebar_double_click(window);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _w, cx| {
                    this.title_drag_state.on_left_mouse_down(e.click_count);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, _e, window, _cx| {
                if this.title_drag_state.take_move_request() {
                    window.start_window_move();
                }
            }));

        let min_hover = with_alpha(theme.colors.text, if theme.is_dark { 0.10 } else { 0.08 });
        let min_active = with_alpha(theme.colors.text, if theme.is_dark { 0.16 } else { 0.12 });
        let min_tooltip: SharedString = "Minimize window".into();
        let min = titlebar_control_button(
            theme,
            "win_min_btn",
            titlebar_control_icon("icons/generic_minimize.svg", theme.colors.accent),
            min_hover,
            min_active,
        )
        .id("win_min")
        .debug_selector(|| "titlebar_win_min".to_string())
        .window_control_area(WindowControlArea::Min)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            window.minimize_window();
        }))
        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
            let changed = if *hovering {
                this.set_tooltip_text_if_changed(Some(min_tooltip.clone()), cx)
            } else {
                this.clear_tooltip_if_matches(&min_tooltip, cx)
            };
            if changed {
                cx.notify();
            }
        }));

        let max_icon = if window.is_maximized() {
            "icons/generic_restore.svg"
        } else {
            "icons/generic_maximize.svg"
        };
        let max_tooltip: SharedString = if window.is_maximized() {
            "Restore window".into()
        } else {
            "Maximize window".into()
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
        .debug_selector(|| "titlebar_win_max".to_string())
        .window_control_area(WindowControlArea::Max)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            window.zoom_window();
            cx.notify();
        }))
        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
            let changed = if *hovering {
                this.set_tooltip_text_if_changed(Some(max_tooltip.clone()), cx)
            } else {
                this.clear_tooltip_if_matches(&max_tooltip, cx)
            };
            if changed {
                cx.notify();
            }
        }));

        let close_hover = with_alpha(theme.colors.danger, if theme.is_dark { 0.45 } else { 0.28 });
        let close_active = with_alpha(theme.colors.danger, if theme.is_dark { 0.60 } else { 0.40 });
        let close_tooltip: SharedString = "Close window".into();
        let close = titlebar_control_button(
            theme,
            "win_close_btn",
            titlebar_control_icon("icons/generic_close.svg", theme.colors.danger),
            close_hover,
            close_active,
        )
        .id("win_close")
        .debug_selector(|| "titlebar_win_close".to_string())
        .window_control_area(WindowControlArea::Close)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            window.remove_window();
        }))
        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
            let changed = if *hovering {
                this.set_tooltip_text_if_changed(Some(close_tooltip.clone()), cx)
            } else {
                this.clear_tooltip_if_matches(&close_tooltip, cx)
            };
            if changed {
                cx.notify();
            }
        }));

        let free_badge_bg = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.22 } else { 0.16 },
        );
        let free_badge_border = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.34 } else { 0.28 },
        );
        let free_badge_text =
            with_alpha(theme.colors.text, if theme.is_dark { 0.72 } else { 0.62 });
        let free_badge = div()
            .id("free_badge")
            .h(px(18.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(2.0))
            .bg(free_badge_bg)
            .border_1()
            .border_color(free_badge_border)
            .text_xs()
            .font_weight(FontWeight::NORMAL)
            .text_color(free_badge_text)
            .child("FREE");

        let macos_brand = div()
            .id("title_bar_macos_brand")
            .h_full()
            .pl_2()
            .flex()
            .items_center()
            .child(
                div()
                    .h(px(26.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(titlebar_app_icon(theme))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.colors.text)
                            .whitespace_nowrap()
                            .child("GitComet"),
                    ),
            );

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
                    .gap_0p5()
                    .when(is_macos, |d| d.pl(MACOS_TRAFFIC_LIGHTS_SAFE_INSET))
                    .when(is_macos, |d| d.child(macos_brand))
                    .when(!is_macos, |d| d.child(menu_toggle).child(windows_brand)),
            )
            .child(drag_region)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(free_badge)
                    .when(!is_macos, |d| d.child(min).child(max).child(close))
                    .pr_2(),
            )
            .into_any_element()
    }
}

pub(crate) fn window_frame(
    theme: AppTheme,
    decorations: Decorations,
    content: AnyElement,
) -> AnyElement {
    let frame_inset = window_frame_visual_inset();
    let mut outer = div()
        .id("window_frame")
        .size_full()
        .bg(gpui::rgba(0x00000000));

    if let Decorations::Client { tiling } = decorations {
        outer = outer
            .when(!tiling.top, |d| d.pt(frame_inset))
            .when(!tiling.bottom, |d| d.pb(frame_inset))
            .when(!tiling.left, |d| d.pl(frame_inset))
            .when(!tiling.right, |d| d.pr(frame_inset));
    }

    let inner = div()
        .id("window_surface")
        .size_full()
        .bg(theme.colors.window_bg)
        .border_1()
        .border_color(window_frame_outline_color(theme))
        .overflow_hidden()
        .when(!cfg!(target_os = "macos"), |d| {
            d.rounded(px(theme.radii.panel)).shadow_lg()
        })
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

    #[test]
    fn window_frame_visual_inset_matches_platform_chrome_strategy() {
        #[cfg(target_os = "macos")]
        assert_eq!(window_frame_visual_inset(), px(0.0));
        #[cfg(not(target_os = "macos"))]
        assert_eq!(window_frame_visual_inset(), CLIENT_SIDE_DECORATION_INSET);
    }

    #[test]
    fn window_frame_outline_color_tracks_platform_and_theme() {
        let dark = AppTheme::zed_ayu_dark();
        let light = AppTheme::zed_one_light();

        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                window_frame_outline_color(dark),
                with_alpha(dark.colors.border, 0.96)
            );
            assert_eq!(
                window_frame_outline_color(light),
                with_alpha(light.colors.border, 0.90)
            );
        }

        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(
                window_frame_outline_color(dark),
                gpui::rgba(WINDOW_OUTLINE_RGBA)
            );
            assert_eq!(
                window_frame_outline_color(light),
                gpui::rgba(WINDOW_OUTLINE_RGBA)
            );
        }
    }

    #[test]
    fn titlebar_drag_state_tracks_single_clicks_and_suppresses_double_click_drags() {
        let mut state = TitleBarDragState::default();

        state.on_left_mouse_down(1);
        assert!(state.should_move, "single click should arm a window move");

        state.on_left_mouse_down(2);
        assert!(
            !state.should_move,
            "double click should suppress drag tracking so it can toggle zoom instead"
        );
    }

    #[test]
    fn titlebar_drag_state_move_request_is_consumed_once() {
        let mut state = TitleBarDragState::default();
        state.on_left_mouse_down(1);

        assert!(
            state.take_move_request(),
            "the first mouse move after pressing the title bar should start a window move"
        );
        assert!(
            !state.take_move_request(),
            "move tracking should clear after the move request is consumed"
        );
    }

    #[test]
    fn titlebar_double_click_requires_standard_double_click() {
        assert!(should_handle_titlebar_double_click(2, true));
        assert!(!should_handle_titlebar_double_click(1, true));
        assert!(!should_handle_titlebar_double_click(3, true));
        assert!(!should_handle_titlebar_double_click(2, false));
    }

    #[test]
    fn titlebar_double_click_action_matches_platform_convention() {
        let expected = if cfg!(target_os = "macos") {
            TitleBarDoubleClickAction::PlatformDefault
        } else {
            TitleBarDoubleClickAction::ToggleZoom
        };

        assert_eq!(titlebar_double_click_action(), expected);
    }
}
