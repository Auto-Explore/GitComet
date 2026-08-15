use super::*;
use crate::ui_scale;
use std::cell::Cell;
use std::rc::Rc;

pub(super) const CLIENT_SIDE_DECORATION_INSET_PX: f32 = 10.0;
pub(super) const TITLE_BAR_HEIGHT_PX: f32 = 38.0;
/// Empty title-bar width kept beside repository tabs so a full tab strip still
/// leaves somewhere to grab the window. Deliberately near the smallest usable
/// grab target: every pixel here is width the tab strip can never use, so it
/// narrows tabs before the bar is even full.
const REPO_TABS_TRAILING_DRAG_WIDTH_PX: f32 = 24.0;
const MACOS_TRAFFIC_LIGHTS_SAFE_INSET_PX: f32 = 78.0;
#[cfg(test)]
pub(super) const CLIENT_SIDE_DECORATION_INSET: Pixels = px(CLIENT_SIDE_DECORATION_INSET_PX);

pub(super) fn client_side_decoration_inset(ui_scale_percent: u32) -> Pixels {
    ui_scale::design_px_from_percent(CLIENT_SIDE_DECORATION_INSET_PX, ui_scale_percent)
}

pub(super) fn title_bar_height(ui_scale_percent: u32) -> Pixels {
    ui_scale::design_px_from_percent(TITLE_BAR_HEIGHT_PX, ui_scale_percent)
}

fn macos_traffic_lights_safe_inset(_ui_scale_percent: u32) -> Pixels {
    px(MACOS_TRAFFIC_LIGHTS_SAFE_INSET_PX)
}

pub(super) struct TitleBarView {
    theme: AppTheme,
    root_view: WeakEntity<GitCometView>,
    title_drag_state: TitleBarDragState,
    app_menu_open: bool,
    app_menu_focus_handle: FocusHandle,
    repo_picker_open: bool,
    workspace_actions_enabled: bool,
    /// Painted bounds of the repository switcher chevron, so opening the picker
    /// from the keyboard can anchor to the same control the mouse uses.
    repo_picker_toggle_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub(in crate::view) struct TitleBarDragState {
    should_move: bool,
}

impl TitleBarDragState {
    pub(in crate::view) fn on_left_mouse_down(&mut self, click_count: usize) {
        self.should_move = click_count < 2;
    }

    pub(in crate::view) fn clear(&mut self) {
        self.should_move = false;
    }

    pub(in crate::view) fn take_move_request(&mut self) -> bool {
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

pub(in crate::view) fn should_handle_titlebar_double_click(
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

pub(in crate::view) fn handle_titlebar_double_click(window: &mut Window) {
    match titlebar_double_click_action() {
        TitleBarDoubleClickAction::PlatformDefault => window.titlebar_double_click(),
        TitleBarDoubleClickAction::ToggleZoom => crate::app::toggle_window_zoom(window),
    }
}

pub(in crate::view) fn show_titlebar_secondary_menu<T: 'static>(
    position: Point<Pixels>,
    window: &Window,
    cx: &mut gpui::Context<T>,
) {
    cx.stop_propagation();

    #[cfg(target_os = "windows")]
    if let Some(request) = crate::app::window_system_menu_request(window, position) {
        // Run the native menu loop after the current GPUI event dispatch has fully unwound,
        // and without holding an App borrow while Windows processes system commands.
        cx.spawn(async move |_this, _cx: &mut gpui::AsyncApp| {
            gitcomet_win32_window_utils::show_window_system_menu(
                request.hwnd,
                request.x,
                request.y,
            );
        })
        .detach();
        return;
    }

    crate::app::show_window_system_menu(window, position);
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

pub(super) fn titlebar_control_button(
    ui_scale_percent: u32,
    id: &'static str,
    icon_path: &'static str,
    idle_color: gpui::Rgba,
    hover_color: gpui::Rgba,
) -> gpui::Div {
    let hitbox_width = ui_scale::design_px_from_percent(32.0, ui_scale_percent);
    let visual_size = ui_scale::design_px_from_percent(26.0, ui_scale_percent);
    let icon_size = ui_scale::design_px_from_percent(16.0, ui_scale_percent);

    div()
        .h_full()
        .w(hitbox_width)
        .flex()
        .items_center()
        .justify_center()
        .cursor(CursorStyle::PointingHand)
        // `occlude`, not `block_mouse_except_scroll`. gpui answers Windows'
        // WM_NCHITTEST with the first hovered window-control area in paint
        // order, testing membership against the whole hit-test list rather than
        // its hovered prefix — so any window-control area painted underneath
        // one of these buttons would answer for it, and Windows would run that
        // area's behaviour instead of delivering the click. Occluding ends the
        // hit test here, leaving the button's own Min/Max/Close area as the
        // only candidate. Nothing scrollable sits under the title bar, so the
        // stricter blocking costs nothing.
        .occlude()
        .child(
            div()
                .id(id)
                .group(id)
                .h_full()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(visual_size)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            // Hovering anywhere in the hitbox recolors the
                            // glyph itself — deliberately no background plate.
                            gpui::svg()
                                .path(icon_path)
                                .w(icon_size)
                                .h(icon_size)
                                .flex_shrink_0()
                                .text_color(idle_color)
                                .group_hover(id, move |s| s.text_color(hover_color)),
                        ),
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

/// The title bar's fill. The active window lifts it off the workspace surface;
/// an inactive one drops back to it. Repo tabs sit on this color, so anything
/// that has to blend into the bar (the label fade, for one) asks here.
pub(in crate::view) fn title_bar_background(theme: AppTheme, window_is_active: bool) -> gpui::Rgba {
    if window_is_active {
        lighten(
            theme.colors.surface.panel,
            if theme.is_dark { 0.06 } else { 0.03 },
        )
    } else {
        theme.colors.surface.panel
    }
}

fn window_frame_visual_inset(ui_scale_percent: u32) -> Pixels {
    if cfg!(target_os = "macos") {
        px(0.0)
    } else {
        client_side_decoration_inset(ui_scale_percent)
    }
}

fn should_suppress_window_frame(decorations: Decorations) -> bool {
    crate::linux_gui_env::LinuxGuiEnvironment::should_suppress_custom_window_frame(decorations)
}

/// Corner radii the window's edge bars (title bar, bottom bar) must adopt so
/// their square backgrounds don't poke past the rounded client frame. `None`
/// when the frame is native, suppressed, or the window is maximized.
pub(in crate::view) struct FrameCornerRounding {
    pub(in crate::view) top_left: bool,
    pub(in crate::view) top_right: bool,
    pub(in crate::view) bottom_left: bool,
    pub(in crate::view) bottom_right: bool,
    pub(in crate::view) radius: Pixels,
}

pub(in crate::view) fn client_frame_corner_rounding(
    theme: AppTheme,
    window: &Window,
) -> Option<FrameCornerRounding> {
    if cfg!(target_os = "macos") {
        return None;
    }
    let decorations = window.window_decorations();
    if should_suppress_window_frame(decorations) {
        return None;
    }
    let Decorations::Client { tiling } = decorations else {
        return None;
    };
    // Children sit inside the frame's 1px border, so their arcs must be one
    // pixel tighter to stay flush with the frame's inner edge.
    let radius = px((theme.radii.window - 1.0).max(0.0));
    Some(FrameCornerRounding {
        top_left: !tiling.top && !tiling.left,
        top_right: !tiling.top && !tiling.right,
        bottom_left: !tiling.bottom && !tiling.left,
        bottom_right: !tiling.bottom && !tiling.right,
        radius,
    })
}

fn window_frame_outline_color(theme: AppTheme) -> gpui::Rgba {
    if cfg!(target_os = "macos") {
        with_alpha(
            theme.colors.stroke.default,
            if theme.is_dark { 0.96 } else { 0.90 },
        )
    } else {
        theme.colors.stroke.default
    }
}

fn should_draw_window_frame_outline() -> bool {
    !cfg!(target_os = "windows")
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
    pub(super) fn new(
        theme: AppTheme,
        root_view: WeakEntity<GitCometView>,
        workspace_actions_enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        Self {
            theme,
            root_view,
            title_drag_state: TitleBarDragState::default(),
            app_menu_open: false,
            app_menu_focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
            repo_picker_open: false,
            workspace_actions_enabled,
            repo_picker_toggle_bounds: Rc::new(Cell::new(None)),
        }
    }

    pub(super) fn repo_picker_toggle_bounds(&self) -> Option<Bounds<Pixels>> {
        self.repo_picker_toggle_bounds.get()
    }

    #[cfg(test)]
    pub(in crate::view) fn app_menu_focus_handle_for_test(&self) -> FocusHandle {
        self.app_menu_focus_handle.clone()
    }

    #[cfg(test)]
    pub(in crate::view) fn title_drag_armed_for_test(&self) -> bool {
        self.title_drag_state.should_move
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

    pub(super) fn set_repo_picker_open(&mut self, open: bool, cx: &mut gpui::Context<Self>) {
        if self.repo_picker_open == open {
            return;
        }
        self.repo_picker_open = open;
        cx.notify();
    }

    pub(super) fn set_workspace_actions_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.workspace_actions_enabled == enabled {
            return;
        }
        self.workspace_actions_enabled = enabled;
        if !enabled {
            self.app_menu_open = false;
            self.repo_picker_open = false;
        }
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

    fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_for_bounds(kind, anchor_bounds, window, cx);
        });
    }
}

impl Render for TitleBarView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value: f32| ui_scale::design_px_from_percent(value, ui_scale_percent);
        let is_macos = cfg!(target_os = "macos");
        let workspace_actions_enabled = self.workspace_actions_enabled;
        let repo_tabs_enabled = workspace_actions_enabled
            && self
                .root_view
                .upgrade()
                .is_some_and(|root| show_titlebar_repo_tabs(root.read(cx).view_mode));
        let app_menu_open = self.app_menu_open;
        let app_menu_open_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.30 } else { 0.24 },
        );
        let app_menu_open_active_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.48 } else { 0.38 },
        );
        let app_menu_hover_bg = theme.titlebar_hover_overlay();
        let app_menu_active_bg = theme.titlebar_active_overlay();
        // Matches `ButtonStyle::Transparent`'s hover border exactly, so the
        // hand-rolled repo-picker div and the `Button`s either side of it grow
        // the same outline under the cursor.
        let titlebar_hover_border = with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.40 } else { 0.30 },
        );
        let bar_bg = title_bar_background(theme, window.is_window_active());
        let app_menu_focus_handle = self.app_menu_focus_handle.clone();

        let menu_toggle = div()
            .h_full()
            .pl(scaled_px(2.0))
            .flex()
            .items_center()
            .child(
                components::Button::new("app_menu_btn", "")
                    .start_slot(svg_icon(
                        "icons/menu.svg",
                        theme.colors.foreground.primary,
                        scaled_px(16.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .selected(app_menu_open)
                    .selected_bg(app_menu_open_bg)
                    .focus_handle(app_menu_focus_handle)
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.set_app_menu_open(true, cx);
                        let anchor = window_top_left_corner(window);
                        this.open_popover_at(PopoverKind::AppMenu, anchor, window, cx);
                    })
                    // Sized to the window controls' 32px hitbox so the 16px glyph
                    // clears the same 8px on each side. The button's intrinsic
                    // `icon_pad_x` is narrower; a fixed width plus the centered
                    // content is what actually matches the two ends of the bar.
                    .h(scaled_px(26.0))
                    .w(scaled_px(32.0))
                    .rounded(px(theme.radii.control))
                    .block_mouse_except_scroll()
                    .debug_selector(|| "app_menu".to_string())
                    .gitcomet_tooltip(theme, "Application menu".into()),
            );

        // Browser-style repository switcher: a bare chevron beside the app menu
        // (and the repo tabs) that opens the repository picker. Replaces the old
        // labelled "Repositories" button that used to sit in the action bar.
        let repo_picker_open = self.repo_picker_open;
        let repo_picker_toggle_bounds_for_prepaint = Rc::clone(&self.repo_picker_toggle_bounds);
        let repo_picker_toggle_bounds_for_click = Rc::clone(&self.repo_picker_toggle_bounds);
        let repo_picker_toggle = div()
            .h_full()
            .flex()
            .items_center()
            .on_children_prepainted(move |children_bounds, _window, _cx| {
                repo_picker_toggle_bounds_for_prepaint.set(children_bounds.first().copied());
            })
            .child(
                div()
                    .id("repo_picker_btn")
                    .debug_selector(|| "repo_picker_toggle".to_string())
                    .h(scaled_px(26.0))
                    .w(scaled_px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor(CursorStyle::PointingHand)
                    .rounded(px(theme.radii.control))
                    // Reserved at rest so gaining the hover outline does not
                    // shift the chevron by a pixel.
                    .border_1()
                    .border_color(gpui::transparent_black())
                    // Stay lit in the pressed/open color while the picker popover
                    // is open, mirroring the app-menu button.
                    .when(repo_picker_open, move |s| s.bg(app_menu_open_bg))
                    .hover(move |s| {
                        let s = s.border_color(titlebar_hover_border);
                        if repo_picker_open {
                            s.bg(app_menu_open_bg)
                        } else {
                            s.bg(app_menu_hover_bg)
                        }
                    })
                    .active(move |s| {
                        if repo_picker_open {
                            s.bg(app_menu_open_active_bg)
                        } else {
                            s.bg(app_menu_active_bg)
                        }
                    })
                    .child(svg_icon(
                        "icons/chevron_down.svg",
                        theme.colors.foreground.primary,
                        scaled_px(16.0),
                    ))
                    .block_mouse_except_scroll()
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        let anchor_bounds = repo_picker_toggle_bounds_for_click
                            .get()
                            .unwrap_or_else(|| {
                                Bounds::new(e.position(), gpui::size(px(0.0), px(0.0)))
                            });
                        this.open_popover_for_bounds(
                            PopoverKind::RepoPicker,
                            anchor_bounds,
                            window,
                            cx,
                        );
                    }))
                    .gitcomet_tooltip(theme, "Switch repository".into()),
            );

        // One drag surface spans the title bar underneath its controls. Each
        // visible control occludes only its painted bounds, so the uncovered
        // title above and below it behaves like ordinary window chrome.
        //
        // Deliberately *not* a `WindowControlArea::Drag`. gpui answers Windows'
        // WM_NCHITTEST with the first hovered window-control area in paint
        // order, testing membership against the whole hit-test list rather than
        // its hovered prefix — so a full-bleed drag area painted under the bar
        // claims HTCAPTION for every control on top of it, and Windows runs its
        // own SC_MOVE loop instead of delivering the click. That froze the
        // repo tabs, the app menu, and the repo picker. Marking each control
        // `occlude()` would fix the lookup but would also cut the tab strip off
        // from wheel events, which is exactly what `block_mouse_except_scroll`
        // is there to preserve. Dragging instead goes through the handlers
        // below, which is already the only path on Linux (window control areas
        // are a no-op there) and reaches the same native SC_MOVE on Windows.
        let drag_surface = div()
            .id("title_drag")
            .debug_selector(|| "titlebar_drag".to_string())
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                if !should_handle_titlebar_double_click(e.click_count(), e.standard_click()) {
                    return;
                }
                this.title_drag_state.clear();
                cx.stop_propagation();
                handle_titlebar_double_click(window);
                cx.notify();
            }))
            // GPUI synthesizes ClickEvent only from the left mouse button, so use mouse-up
            // directly for the Windows title bar system menu.
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|_this, e: &MouseUpEvent, window, cx| {
                    if crate::press_gesture::is_press_claimed(cx) {
                        return;
                    }
                    show_titlebar_secondary_menu(e.position, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _w, cx| {
                    crate::press_gesture::claim_press(cx);
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
                    crate::app::begin_window_move(window);
                }
            }));

        let min_tooltip: SharedString = "Minimize window".into();
        let min = titlebar_control_button(
            ui_scale_percent,
            "win_min_btn",
            "icons/generic_minimize.svg",
            theme.colors.foreground.secondary,
            theme.colors.foreground.primary,
        )
        .id("win_min")
        .debug_selector(|| "titlebar_win_min".to_string())
        .window_control_area(WindowControlArea::Min)
        .gitcomet_tooltip(theme, min_tooltip)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            window.minimize_window();
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
        let max = titlebar_control_button(
            ui_scale_percent,
            "win_max_btn",
            max_icon,
            theme.colors.foreground.secondary,
            theme.colors.foreground.primary,
        )
        .id("win_max")
        .debug_selector(|| "titlebar_win_max".to_string())
        .window_control_area(WindowControlArea::Max)
        .gitcomet_tooltip(theme, max_tooltip)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            crate::app::toggle_window_zoom(window);
            cx.notify();
        }));

        let close_tooltip: SharedString = "Close window".into();
        let close = titlebar_control_button(
            ui_scale_percent,
            "win_close_btn",
            "icons/generic_close.svg",
            theme.colors.foreground.secondary,
            theme.colors.status.danger.foreground,
        )
        .id("win_close")
        .debug_selector(|| "titlebar_win_close".to_string())
        .window_control_area(WindowControlArea::Close)
        .gitcomet_tooltip(theme, close_tooltip)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            crate::app::close_window_or_warn(window, cx);
        }));

        // Leading and trailing clusters center on the full bar height; tab
        // labels compensate for their bottom fusion (see `Tab::render`) so
        // icons and tab text share the bar's true midline.
        let leading = div()
            .flex()
            .items_center()
            .h_full()
            .gap(scaled_px(2.0))
            .when(is_macos, |d| {
                d.pl(macos_traffic_lights_safe_inset(ui_scale_percent))
            })
            .when(!is_macos && workspace_actions_enabled, |d| {
                d.child(menu_toggle)
            })
            .when(workspace_actions_enabled, |d| d.child(repo_picker_toggle));

        // Browser-style: when a workspace is open, the repo tabs live in the
        // title bar's middle. Keep a fixed draggable strip beside them so the
        // window can still be moved by the empty title-bar area.
        let repo_tabs = if repo_tabs_enabled {
            self.root_view
                .upgrade()
                .map(|root| root.read(cx).repo_tabs_bar.clone())
        } else {
            None
        };
        let middle: AnyElement = if let Some(repo_tabs) = repo_tabs {
            div()
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .overflow_hidden()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .h_full()
                        .overflow_hidden()
                        .child(repo_tabs),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .w(scaled_px(REPO_TABS_TRAILING_DRAG_WIDTH_PX))
                        .h_full(),
                )
                .into_any_element()
        } else {
            div().flex_1().h_full().into_any_element()
        };

        let frame_rounding = client_frame_corner_rounding(theme, window);
        div()
            .id("title_bar")
            .relative()
            .flex()
            .items_center()
            .h(title_bar_height(ui_scale_percent))
            .w_full()
            .bg(bar_bg)
            .when_some(frame_rounding, |d, rounding| {
                d.when(rounding.top_left, |d| d.rounded_tl(rounding.radius))
                    .when(rounding.top_right, |d| d.rounded_tr(rounding.radius))
            })
            // The bar/content boundary line. Painted before the tabs so the
            // active tab (flush with the bar bottom, filled with the content
            // strip color) covers its segment and fuses into the action bar.
            .when(repo_tabs_enabled, |d| {
                d.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(1.0))
                        .bg(components::Tab::outline_color(theme)),
                )
            })
            .child(drag_surface)
            .child(leading)
            .child(middle)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .gap(scaled_px(4.0))
                    .when(!is_macos, |d| d.child(min).child(max).child(close))
                    .pr(scaled_px(8.0)),
            )
            .into_any_element()
    }
}

pub(crate) fn window_frame(
    theme: AppTheme,
    decorations: Decorations,
    content: AnyElement,
    overlay: Option<AnyElement>,
    ui_scale_percent: u32,
) -> AnyElement {
    let suppress_frame = should_suppress_window_frame(decorations);
    let frame_inset = window_frame_visual_inset(ui_scale_percent);
    let mut outer = div()
        .id("window_frame")
        .size_full()
        .bg(gpui::rgba(0x00000000));

    if !suppress_frame && let Decorations::Client { tiling } = decorations {
        outer = outer
            .when(!tiling.top, |d| d.pt(frame_inset))
            .when(!tiling.bottom, |d| d.pb(frame_inset))
            .when(!tiling.left, |d| d.pl(frame_inset))
            .when(!tiling.right, |d| d.pr(frame_inset));
    }

    let mut inner = div()
        .id("window_surface")
        .size_full()
        .relative()
        .bg(theme.colors.surface.canvas);

    if !suppress_frame {
        let draw_outline = should_draw_window_frame_outline();
        inner = inner
            .when(draw_outline, |d| {
                d.border_1().border_color(window_frame_outline_color(theme))
            })
            .when(!cfg!(target_os = "macos"), |d| {
                d.rounded(px(theme.radii.window)).shadow_lg()
            });
    }

    inner = inner.child(content);
    if let Some(overlay) = overlay {
        inner = inner.child(overlay);
    }

    // Every window built on the frame resets the press claim; see the
    // `press_gesture` module docs.
    outer
        .child(crate::press_gesture::PressGestureReset)
        .child(inner)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titlebar_buttons_do_not_double_set_hover_style() {
        let theme = AppTheme::gitcomet_dark();
        assert!(
            std::panic::catch_unwind(|| {
                let _ = titlebar_control_button(
                    ui_scale::DEFAULT_UI_SCALE_PERCENT,
                    "test_btn_1",
                    "icons/generic_minimize.svg",
                    theme.colors.foreground.secondary,
                    theme.colors.foreground.primary,
                );
            })
            .is_ok()
        );
        assert!(
            std::panic::catch_unwind(|| {
                let _ = titlebar_control_button(
                    ui_scale::DEFAULT_UI_SCALE_PERCENT,
                    "test_btn_2",
                    "icons/generic_close.svg",
                    theme.colors.foreground.secondary,
                    theme.colors.status.danger.foreground,
                );
            })
            .is_ok()
        );
    }

    #[test]
    fn window_frame_visual_inset_matches_platform_chrome_strategy() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            window_frame_visual_inset(ui_scale::DEFAULT_UI_SCALE_PERCENT),
            px(0.0)
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            window_frame_visual_inset(ui_scale::DEFAULT_UI_SCALE_PERCENT),
            CLIENT_SIDE_DECORATION_INSET
        );
    }

    #[test]
    fn window_frame_outline_color_tracks_platform_and_theme() {
        let dark = AppTheme::gitcomet_dark();
        let light = AppTheme::gitcomet_light();

        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                window_frame_outline_color(dark),
                with_alpha(dark.colors.stroke.default, 0.96)
            );
            assert_eq!(
                window_frame_outline_color(light),
                with_alpha(light.colors.stroke.default, 0.90)
            );
        }

        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(window_frame_outline_color(dark), dark.colors.stroke.default);
            assert_eq!(
                window_frame_outline_color(light),
                light.colors.stroke.default
            );
        }
    }

    #[test]
    fn window_frame_outline_is_omitted_on_windows() {
        #[cfg(target_os = "windows")]
        assert!(!should_draw_window_frame_outline());
        #[cfg(not(target_os = "windows"))]
        assert!(should_draw_window_frame_outline());
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
