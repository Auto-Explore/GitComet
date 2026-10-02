//! Window chrome: the title bars, their controls, and the frame around them.
//!
//! A title bar sits outside the appearance system. It shares its row with the OS
//! window controls, which do not resize, so the bar, its buttons and the
//! repository tabs hold one size at every UI scale, density and font size. Bar
//! geometry is written in literal `px`; the shared components a bar borrows are
//! handed [`chrome_scale`] so they size themselves the same way.
//!
//! That covers the bar and what it draws. The window *frame* around it --
//! [`client_side_decoration_inset`] and the resize band derived from it -- is a
//! pointer target on the window edge rather than something in the bar, and
//! deliberately still follows the UI scale.
use super::*;
use crate::kit::click::PointerClickExt as _;
use crate::ui_scale;
use crate::view::components::{ControlInteractionExt, InteractionState, InteractionStyle};

use std::cell::Cell;
use std::rc::Rc;

pub(super) const CLIENT_SIDE_DECORATION_INSET_PX: f32 = 10.0;
pub(super) const TITLE_BAR_HEIGHT_PX: f32 = 38.0;
/// Empty title-bar width kept beside repository tabs so a full tab strip still
/// leaves somewhere to grab the window. Deliberately near the smallest usable
/// grab target: every pixel here is width the tab strip can never use, so it
/// narrows tabs before the bar is even full.
const REPO_TABS_TRAILING_DRAG_WIDTH_PX: f32 = 24.0;
/// AppKit's own traffic-light buttons. Their frames cannot be read before the
/// window exists, so the geometry is pinned here and the bar is laid out around
/// it.
const MACOS_TRAFFIC_LIGHT_BUTTON_HEIGHT_PX: f32 = 14.0;
const MACOS_TRAFFIC_LIGHT_BUTTON_WIDTH_PX: f32 = 14.0;
const MACOS_TRAFFIC_LIGHT_BUTTON_GAP_PX: f32 = 6.0;
/// Distance from the window's leading edge to the close button.
const MACOS_TRAFFIC_LIGHTS_LEADING_PX: f32 = 9.0;
/// Breathing room between the zoom button and the first control we draw.
const MACOS_TRAFFIC_LIGHTS_CLEARANCE_PX: f32 = 15.0;
/// Leading padding a bar gives up to the lights AppKit paints over it. Derived,
/// so the clearance is the only number anyone needs to reconsider.
pub(crate) const MACOS_TRAFFIC_LIGHTS_SAFE_INSET: Pixels = px(MACOS_TRAFFIC_LIGHTS_LEADING_PX
    + MACOS_TRAFFIC_LIGHT_BUTTON_WIDTH_PX * 3.0
    + MACOS_TRAFFIC_LIGHT_BUTTON_GAP_PX * 2.0
    + MACOS_TRAFFIC_LIGHTS_CLEARANCE_PX);
const _: () = assert!(MACOS_TRAFFIC_LIGHTS_CLEARANCE_PX > 0.0);

#[cfg(test)]
pub(super) const CLIENT_SIDE_DECORATION_INSET: Pixels = px(CLIENT_SIDE_DECORATION_INSET_PX);

/// The scale a shared component is handed when it is drawn into a bar. One
/// definition, so a component cannot end up pinned to a different baseline than
/// the bar around it.
pub(in crate::view) use crate::ui_scale::chrome_scale;

pub(super) fn client_side_decoration_inset(ui_scale_percent: u32) -> Pixels {
    ui_scale::design_px_from_percent(CLIENT_SIDE_DECORATION_INSET_PX, ui_scale_percent)
}

pub(crate) const TITLE_BAR_HEIGHT: Pixels = px(TITLE_BAR_HEIGHT_PX);

/// Fallback size for text in a bar that does not set its own, so it cannot
/// inherit the app root's rem-based size and grow with the UI scale.
const TITLE_BAR_TEXT_SIZE_PX: f32 = 14.0;

/// Geometry every control in a title bar shares -- the app menu and repository
/// switcher at the leading edge, the min/max/close caption buttons at the
/// trailing one. One set of numbers, so a 16px glyph clears the same 8px on
/// each side at both ends of the bar.
const TITLE_BAR_BUTTON_HEIGHT_PX: f32 = 26.0;
const TITLE_BAR_BUTTON_WIDTH_PX: f32 = 32.0;
const TITLE_BAR_ICON_SIZE_PX: f32 = 16.0;
/// Spacing around the trailing caption cluster.
const TITLE_BAR_CONTROL_GAP_PX: f32 = 4.0;
const TITLE_BAR_CONTROL_TRAILING_PAD_PX: f32 = 8.0;

pub(super) const TITLE_BAR_BUTTON_HEIGHT: Pixels = px(TITLE_BAR_BUTTON_HEIGHT_PX);
/// The plate has to leave the bar an inset at both edges.
const _: () = assert!(TITLE_BAR_BUTTON_HEIGHT_PX < TITLE_BAR_HEIGHT_PX);

/// The trailing min/max/close cluster. Both title bars build it here: the
/// buttons hold one size, so the spacing around them has to as well, and a
/// single builder is what stops the two bars drifting apart. The list follows
/// the configured button order and is empty on macOS, where AppKit draws them.
pub(super) fn window_controls_cluster(controls: Vec<AnyElement>) -> gpui::Div {
    let has_controls = !controls.is_empty();
    div()
        .flex()
        .items_center()
        .h_full()
        .gap(px(TITLE_BAR_CONTROL_GAP_PX))
        .children(controls)
        .when(has_controls, |cluster| {
            cluster.pr(px(TITLE_BAR_CONTROL_TRAILING_PAD_PX))
        })
}

/// Where AppKit should put the traffic lights. Every GitComet window asks here,
/// so the three of them agree.
///
/// AppKit centres the buttons in a container it sizes as
/// `button_height + 2 * position.y` and pins to the top of the window, so the y
/// inset is what puts them on our bar's midline -- and the bar is a fixed
/// height, so this is one too.
pub(crate) fn macos_traffic_light_position() -> Point<Pixels> {
    point(
        px(MACOS_TRAFFIC_LIGHTS_LEADING_PX),
        px((TITLE_BAR_HEIGHT_PX - MACOS_TRAFFIC_LIGHT_BUTTON_HEIGHT_PX) / 2.0),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::view) enum TitleBarMode {
    Standard,
    Gated,
}

pub(super) struct TitleBarView {
    mode: TitleBarMode,
    brand: Option<gpui::AnyView>,
    theme: AppTheme,
    root_view: WeakEntity<GitCometView>,
    title_drag_state: TitleBarDragState,
    app_menu_open: bool,
    app_menu_focus_handle: FocusHandle,
    repo_picker_open: bool,
    repo_tab_actions_enabled: bool,
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
    window_surface_bounds(window).origin
}

/// The shadow band the client-side frame pads around the window surface, per
/// side: `inset` on every edge the compositor has not tiled, nothing for a
/// native or suppressed frame. `window_frame` pads by it and popovers fit
/// inside it.
pub(in crate::view) fn frame_insets(
    decorations: Decorations,
    inset: Pixels,
) -> gpui::Edges<Pixels> {
    match decorations {
        Decorations::Client { tiling } if !should_suppress_window_frame(decorations) => {
            let side = |tiled: bool| if tiled { px(0.0) } else { inset };
            gpui::Edges {
                top: side(tiling.top),
                right: side(tiling.right),
                bottom: side(tiling.bottom),
                left: side(tiling.left),
            }
        }
        _ => gpui::Edges::default(),
    }
}

/// The visible window surface in window coordinates. Size and place things
/// against this, never `window_bounds()`: that is the restore size while the
/// window is maximized or fullscreen, and it includes the shadow band.
pub(in crate::view) fn window_surface_bounds(window: &Window) -> Bounds<Pixels> {
    let insets = frame_insets(
        window.window_decorations(),
        window.client_inset().unwrap_or(px(0.0)),
    );
    let viewport = window.viewport_size();
    let top_left = point(insets.left, insets.top);
    Bounds::from_corners(
        top_left,
        point(
            (viewport.width - insets.right).max(top_left.x),
            (viewport.height - insets.bottom).max(top_left.y),
        ),
    )
}

pub(super) fn titlebar_control_button(
    id: &'static str,
    icon_path: &'static str,
    idle_color: gpui::Rgba,
    hover_color: gpui::Rgba,
) -> gpui::Div {
    let hitbox_width = px(TITLE_BAR_BUTTON_WIDTH_PX);
    let visual_size = px(TITLE_BAR_BUTTON_HEIGHT_PX);
    let icon_size = px(TITLE_BAR_ICON_SIZE_PX);

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

fn main_window_control(
    button: gpui::WindowButton,
    is_maximized: bool,
    theme: AppTheme,
    cx: &mut gpui::Context<TitleBarView>,
) -> AnyElement {
    match button {
        gpui::WindowButton::Minimize => titlebar_control_button(
            "win_min_btn",
            "icons/generic_minimize.svg",
            theme.colors.foreground.secondary,
            theme.colors.foreground.primary,
        )
        .id("win_min")
        .debug_selector(|| "titlebar_win_min".to_string())
        .window_control_area(WindowControlArea::Min)
        .gitcomet_tooltip(theme, SharedString::from("Minimize window"))
        .on_activate(
            false,
            components::ControlActivation::Action,
            cx.listener(|_this, _e: &ClickEvent, window, cx| {
                cx.stop_propagation();
                window.minimize_window();
            }),
        )
        .into_any_element(),
        gpui::WindowButton::Maximize => {
            let (icon, tooltip) = if is_maximized {
                ("icons/generic_restore.svg", "Restore window")
            } else {
                ("icons/generic_maximize.svg", "Maximize window")
            };
            titlebar_control_button(
                "win_max_btn",
                icon,
                theme.colors.foreground.secondary,
                theme.colors.foreground.primary,
            )
            .id("win_max")
            .debug_selector(|| "titlebar_win_max".to_string())
            .window_control_area(WindowControlArea::Max)
            .gitcomet_tooltip(theme, SharedString::from(tooltip))
            .on_activate(
                false,
                components::ControlActivation::Action,
                cx.listener(|_this, _e: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    crate::app::toggle_window_zoom(window);
                    cx.notify();
                }),
            )
            .into_any_element()
        }
        gpui::WindowButton::Close => titlebar_control_button(
            "win_close_btn",
            "icons/generic_close.svg",
            theme.colors.foreground.secondary,
            theme.colors.status.danger.foreground,
        )
        .id("win_close")
        .debug_selector(|| "titlebar_win_close".to_string())
        .window_control_area(WindowControlArea::Close)
        .gitcomet_tooltip(theme, SharedString::from("Close window"))
        .on_activate(
            false,
            components::ControlActivation::Action,
            cx.listener(|_this, _e: &ClickEvent, window, cx| {
                cx.stop_propagation();
                crate::app::close_window_or_warn(window, cx);
            }),
        )
        .into_any_element(),
    }
}

fn mix(mut a: gpui::Rgba, b: gpui::Rgba, t: f32) -> gpui::Rgba {
    let t = t.clamp(0.0, 1.0);
    a.red = a.red + (b.red - a.red) * t;
    a.green = a.green + (b.green - a.green) * t;
    a.blue = a.blue + (b.blue - a.blue) * t;
    a.alpha = a.alpha + (b.alpha - a.alpha) * t;
    a
}

fn lighten(color: gpui::Rgba, amount: f32) -> gpui::Rgba {
    mix(color, gpui::rgba(0xFFFFFFFF), amount)
}

/// A workspace's dot and tint colour. Fixed per option, never theme-derived,
/// so a dot always shows the option that was picked; Default dots are Blue.
pub(in crate::view) fn workspace_color(
    color: Option<gitcomet_state::session::WorkspaceColor>,
) -> gpui::Rgba {
    use gitcomet_state::session::WorkspaceColor;
    match color {
        Some(WorkspaceColor::Gray) => gpui::rgba(0x8E949CFF),
        Some(WorkspaceColor::Brown) => gpui::rgba(0xA47551FF),
        Some(WorkspaceColor::Red) => gpui::rgba(0xE05252FF),
        Some(WorkspaceColor::Orange) => gpui::rgba(0xE08A3EFF),
        Some(WorkspaceColor::Yellow) => gpui::rgba(0xD2A83AFF),
        Some(WorkspaceColor::Lime) => gpui::rgba(0x90B43CFF),
        Some(WorkspaceColor::Green) => gpui::rgba(0x48A868FF),
        Some(WorkspaceColor::Teal) => gpui::rgba(0x2E9E8FFF),
        Some(WorkspaceColor::Cyan) => gpui::rgba(0x39B0D0FF),
        None | Some(WorkspaceColor::Blue) => gpui::rgba(0x4B8DDBFF),
        Some(WorkspaceColor::Indigo) => gpui::rgba(0x4E66D0FF),
        Some(WorkspaceColor::Purple) => gpui::rgba(0x956EDBFF),
        Some(WorkspaceColor::Magenta) => gpui::rgba(0xC757C7FF),
        Some(WorkspaceColor::Pink) => gpui::rgba(0xD866A4FF),
    }
}

/// The title bar's fill. The active window lifts it off the workspace surface;
/// an inactive one drops back to it. Repo tabs sit on this color, so anything
/// that has to blend into the bar (the label fade, for one) asks here.
pub(in crate::view) fn title_bar_background(
    theme: AppTheme,
    window_is_active: bool,
    group_color: Option<gitcomet_state::session::WorkspaceColor>,
) -> gpui::Rgba {
    let base = if window_is_active {
        lighten(
            theme.colors.surface.panel,
            if theme.is_dark { 0.06 } else { 0.03 },
        )
    } else {
        theme.colors.surface.panel
    };
    let Some(group_color) = group_color else {
        return base;
    };

    // Keep text, tabs and window controls in the theme's contrast system while
    // making groups visibly distinct. Active windows carry a little more tint;
    // inactive ones remain identifiable without competing for attention.
    mix(
        base,
        workspace_color(Some(group_color)),
        if window_is_active { 0.18 } else { 0.12 },
    )
}

/// A workspace's row in a popover list wears its active title bar's tint,
/// flattened onto the popover surface so hover and selection can composite over
/// it. `None` when no colour is chosen: the row stays plain.
pub(in crate::view) fn workspace_row_tint(
    color: Option<gitcomet_state::session::WorkspaceColor>,
    theme: AppTheme,
) -> Option<gpui::Rgba> {
    let color = color?;
    Some(crate::theme::composite_over(
        theme.colors.surface.raised,
        title_bar_background(theme, true, Some(color)),
    ))
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
        repo_tab_actions_enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        Self {
            theme,
            root_view,
            mode: TitleBarMode::Standard,
            brand: None,
            title_drag_state: TitleBarDragState::default(),
            app_menu_open: false,
            app_menu_focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
            repo_picker_open: false,
            repo_tab_actions_enabled,
            repo_picker_toggle_bounds: Rc::new(Cell::new(None)),
        }
    }

    pub(in crate::view) fn set_brand(
        &mut self,
        brand: Option<gpui::AnyView>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.brand = brand;
        cx.notify();
    }

    pub(in crate::view) fn set_mode(&mut self, mode: TitleBarMode, cx: &mut gpui::Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            cx.notify();
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

    pub(super) fn set_repo_tab_actions_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.repo_tab_actions_enabled == enabled {
            return;
        }
        self.repo_tab_actions_enabled = enabled;
        if !enabled {
            // The app menu stays reachable from Home; only the picker closes.
            self.repo_picker_open = false;
        }
        cx.notify();
    }

    fn open_popover_at(
        &mut self,
        kind: impl Into<PopoverRequest>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_at(kind, anchor, window, cx);
        });
    }

    fn open_popover_for_bounds(
        &mut self,
        kind: impl Into<PopoverRequest>,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_for_bounds(kind, anchor_bounds, window, cx);
        });
    }
}

impl Render for TitleBarView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let is_macos = cfg!(target_os = "macos");
        let repo_tab_actions_enabled =
            self.mode == TitleBarMode::Standard && self.repo_tab_actions_enabled;
        let root_view_mode = self.root_view.upgrade().map(|root| root.read(cx).view_mode);
        let repo_tabs_enabled =
            repo_tab_actions_enabled && root_view_mode.is_some_and(show_titlebar_repo_tabs);
        // Normal windows always offer the app menu, Home included.
        let app_menu_enabled = root_view_mode.is_some_and(renders_full_chrome);
        let app_menu_open = self.app_menu_open;
        let app_menu_open_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.30 } else { 0.24 },
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
        let workspace = crate::workspaces::with_workspace_for_window(
            cx,
            window.window_handle().window_id(),
            |workspace| (workspace.display_name(), workspace.color),
        );
        let workspace_chip_visible = self.mode == TitleBarMode::Standard && workspace.is_some();
        let bar_bg = title_bar_background(
            theme,
            window.is_window_active(),
            workspace.as_ref().and_then(|(_, color)| *color),
        );
        let app_menu_focus_handle = self.app_menu_focus_handle.clone();

        let menu_toggle = div().h_full().pl(px(2.0)).flex().items_center().child(
            components::Button::new("app_menu_btn", "")
                .start_slot(svg_icon(
                    "icons/menu.svg",
                    theme.colors.foreground.primary,
                    px(TITLE_BAR_ICON_SIZE_PX),
                ))
                .style(components::ButtonStyle::Transparent)
                .open(app_menu_open)
                .selected_bg(app_menu_open_bg)
                .focus_handle(app_menu_focus_handle)
                .unscaled()
                .on_click(theme, cx, |this, _e, window, cx| {
                    let anchor = window_top_left_corner(window);
                    this.open_popover_at(PopoverKind::AppMenu, anchor, window, cx);
                })
                // The button's intrinsic `icon_pad_x` is narrower than the
                // shared hitbox, so state the size rather than inherit it.
                .h(TITLE_BAR_BUTTON_HEIGHT)
                .w(px(TITLE_BAR_BUTTON_WIDTH_PX))
                .rounded(px(theme.radii.control))
                .block_mouse_except_scroll()
                .debug_selector(|| "app_menu".to_string())
                .gitcomet_tooltip(theme, "Application menu".into()),
        );

        // The group name and color identify this window beside the repository
        // tabs and open the repository picker.
        let repo_picker_open = self.repo_picker_open;
        let workspace_dot = workspace_color(workspace.as_ref().and_then(|(_, color)| *color));
        let workspace_label: SharedString = workspace
            .map(|(name, _)| name)
            .unwrap_or_else(|| "Workspace".to_string())
            .into();
        let workspace_tooltip: SharedString = format!("Workspace: {workspace_label}").into();
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
                    .h(TITLE_BAR_BUTTON_HEIGHT)
                    .max_w(px(190.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.0))
                    .cursor(CursorStyle::PointingHand)
                    .rounded(px(theme.radii.control))
                    // Reserved at rest so gaining the hover outline does not
                    // shift the chevron by a pixel.
                    .border_1()
                    .border_color(gpui::transparent_black())
                    // Stay lit in the pressed/open color while the picker popover
                    // is open, mirroring the app-menu button.
                    .tab_index(0)
                    .control_interaction(
                        InteractionStyle::new(theme)
                            .persistent_background(app_menu_open_bg)
                            .hover(
                                StyleRefinement::default()
                                    .bg(app_menu_hover_bg)
                                    .border_color(titlebar_hover_border),
                            )
                            .pressed(StyleRefinement::default().bg(app_menu_active_bg)),
                        InteractionState::default().open(repo_picker_open),
                    )
                    .child(
                        div()
                            .size(px(8.0))
                            .flex_none()
                            .rounded_full()
                            .bg(workspace_dot),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(workspace_label),
                    )
                    .child(svg_icon(
                        "icons/chevron_down.svg",
                        theme.colors.foreground.primary,
                        px(TITLE_BAR_ICON_SIZE_PX),
                    ))
                    .block_mouse_except_scroll()
                    .on_activate(
                        false,
                        components::ControlActivation::Action,
                        cx.listener(move |this, e: &ClickEvent, window, cx| {
                            let anchor_bounds = repo_picker_toggle_bounds_for_click
                                .get()
                                .unwrap_or_else(|| {
                                    Bounds::new(e.position(), gpui::size(px(0.0), px(0.0)))
                                });
                            this.open_popover_for_bounds(
                                PopoverKind::RepoPicker {
                                    scope: RepoPickerScope::All,
                                },
                                anchor_bounds,
                                window,
                                cx,
                            );
                        }),
                    )
                    .gitcomet_tooltip(theme, workspace_tooltip),
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
            .on_activate(
                false,
                components::ControlActivation::Composite,
                cx.listener(|this, e: &ClickEvent, window, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                    if !should_handle_titlebar_double_click(e.click_count(), e.standard_click()) {
                        return;
                    }
                    cx.stop_propagation();
                    handle_titlebar_double_click(window);
                }),
            )
            // The system menu follows the same completed-click rule as controls.
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(|_this, e: &MouseDownEvent, window, cx| {
                    show_titlebar_secondary_menu(e.position, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _w, cx| {
                    crate::press_gesture::claim_press(cx);
                    crate::text_selection_owner::preserve(cx);
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

        let is_maximized = window.is_maximized();
        let control_layout = crate::window_controls::resolve_visibility(
            crate::window_controls::current(cx).mode,
            cx.button_layout(),
            cfg!(any(target_os = "linux", target_os = "freebsd")),
            window.window_decorations(),
            is_maximized,
        );
        let (left_controls, right_controls) = if is_macos {
            (Vec::new(), Vec::new())
        } else {
            let mut render = |button| main_window_control(button, is_maximized, theme, cx);
            (
                control_layout
                    .left
                    .into_iter()
                    .flatten()
                    .map(&mut render)
                    .collect::<Vec<_>>(),
                control_layout
                    .right
                    .into_iter()
                    .flatten()
                    .map(render)
                    .collect::<Vec<_>>(),
            )
        };
        let has_left_controls = !left_controls.is_empty();

        // Leading and trailing clusters center on the full bar height; tab
        // labels compensate for their bottom fusion (see `Tab::render`) so
        // icons and tab text share the bar's true midline.
        let leading = div()
            .flex()
            .items_center()
            .h_full()
            .gap(px(2.0))
            .when(is_macos, |d| d.pl(MACOS_TRAFFIC_LIGHTS_SAFE_INSET))
            .when(has_left_controls, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .h_full()
                        .gap(px(4.0))
                        .pl(px(8.0))
                        .children(left_controls),
                )
            })
            .when(!is_macos && app_menu_enabled, |d| d.child(menu_toggle))
            .children(self.brand.clone())
            // An empty customized workspace keeps its name and colour on Home.
            .when(repo_tab_actions_enabled || workspace_chip_visible, |d| {
                d.child(repo_picker_toggle)
            });

        // Browser-style: when repositories are open, the repo tabs live in the
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
                        .w(px(REPO_TABS_TRAILING_DRAG_WIDTH_PX))
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
            .h(TITLE_BAR_HEIGHT)
            .w_full()
            .text_size(px(TITLE_BAR_TEXT_SIZE_PX))
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
            .child(window_controls_cluster(right_controls))
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
    let insets = frame_insets(decorations, window_frame_visual_inset(ui_scale_percent));
    let outer = div()
        .id("window_frame")
        .size_full()
        .bg(gpui::rgba(0x00000000))
        .pt(insets.top)
        .pb(insets.bottom)
        .pl(insets.left)
        .pr(insets.right);

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
        // First child, so its capture listeners are registered before any
        // content listener and therefore run first; see `window_root_hook`.
        .child(crate::window_root_hook::WindowRootHook::new(|window| {
            crate::press_gesture::install_reset(window);
            crate::text_selection_owner::install_reset(window);
        }))
        .child(inner)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gpui's numbered spacing shorthands (`gap_1`, `pr_2`, `text_sm`, …) are
    /// rem-based, and the rem size follows the UI scale -- so a single one of
    /// them inside the chrome silently re-scales part of a bar that is supposed
    /// to hold one size. This is how the settings header's caption cluster kept
    /// growing after the rest of the bar was pinned. Needles are assembled at
    /// runtime so this test does not match its own source.
    #[test]
    fn the_chrome_never_reaches_for_a_rem_based_size() {
        const CHROME_SOURCES: [(&str, &str); 4] = [
            ("view/chrome.rs", include_str!("chrome.rs")),
            (
                "view/panels/repo_tabs_bar.rs",
                include_str!("panels/repo_tabs_bar.rs"),
            ),
            (
                "gitcomet-ui-kit/src/components/tab.rs",
                include_str!("../../../gitcomet-ui-kit/src/components/tab.rs"),
            ),
            (
                "gitcomet-ui-kit/src/components/tab_bar.rs",
                include_str!("../../../gitcomet-ui-kit/src/components/tab_bar.rs"),
            ),
        ];
        // Length-taking builders only: `border_*` and `rounded_*` are px, and
        // `flex_1` is a grow factor, not a size.
        const LENGTH: &[&str] = &[
            "gap", "gap_x", "gap_y", "p", "px", "py", "pt", "pb", "pl", "pr", "m", "mx", "my",
            "mt", "mb", "ml", "mr", "w", "h", "size", "top", "bottom", "left", "right", "min_w",
            "min_h", "max_w", "max_h",
        ];
        // The `_0` step is px(0), so it is exempt.
        const STEPS: &[&str] = &[
            "0p5", "1", "1p5", "2", "2p5", "3", "3p5", "4", "5", "6", "8", "10", "12", "16", "20",
            "24",
        ];
        const TEXT: &[&str] = &["xs", "sm", "base", "lg", "xl", "2xl", "3xl"];

        let needles: Vec<String> = LENGTH
            .iter()
            .flat_map(|prefix| STEPS.iter().map(move |step| format!(".{prefix}_{step}(")))
            .chain(TEXT.iter().map(|size| format!(".text_{size}(")))
            .chain(std::iter::once(format!("{}(", "rems")))
            .collect();

        let mut offenders = Vec::new();
        for (name, source) in CHROME_SOURCES {
            for (ix, line) in source.lines().enumerate() {
                for needle in &needles {
                    if line.contains(needle.as_str()) {
                        offenders.push(format!("{name}:{} uses {needle}", ix + 1));
                    }
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "window chrome must size itself in fixed px: {offenders:#?}"
        );
    }

    /// AppKit centres the traffic lights inside a container it sizes as
    /// `button_height + 2 * position.y` and pins to the top of the window (gpui
    /// `move_traffic_light`). So the y inset is the only thing that decides
    /// whether the lights share a midline with our bar -- get it wrong and they
    /// sit high in the bar with no other symptom.
    #[test]
    fn the_macos_traffic_lights_share_the_title_bars_midline() {
        let position = macos_traffic_light_position();
        let container = px(MACOS_TRAFFIC_LIGHT_BUTTON_HEIGHT_PX) + position.y * 2.0;

        assert_eq!(
            container, TITLE_BAR_HEIGHT,
            "the container AppKit builds must be exactly the bar the lights sit in"
        );
    }

    /// `chrome_scale` is what pins every shared component drawn into a bar, so
    /// it has to be a genuine no-op on both axes -- if the app's defaults ever
    /// move under it, the chrome moves with them.
    #[test]
    fn the_chrome_scale_is_the_identity_on_both_axes() {
        let scale = chrome_scale();

        assert_eq!(scale.px(20.0), px(20.0), "the chrome must not zoom");
        assert_eq!(
            scale.appearance,
            crate::appearance::Appearance {
                density: crate::appearance::UiDensity::Compact,
                ..crate::appearance::Appearance::default()
            },
            "the chrome must not take density or font size"
        );
        assert_eq!(scale.row_height(24.0, 32.0), px(24.0));
        assert_eq!(scale.ui_text(15.0), px(15.0));
    }

    #[test]
    fn titlebar_buttons_do_not_double_set_hover_style() {
        let theme = AppTheme::gitcomet_dark();
        assert!(
            std::panic::catch_unwind(|| {
                let _ = titlebar_control_button(
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
                    "test_btn_2",
                    "icons/generic_close.svg",
                    theme.colors.foreground.secondary,
                    theme.colors.status.danger.foreground,
                );
            })
            .is_ok()
        );
    }

    /// Popovers fit inside the same padding the frame draws, so the two can
    /// only agree if a tiled edge loses its shadow band in both.
    #[test]
    fn frame_insets_pad_only_the_untiled_edges_of_a_client_frame() {
        let inset = px(10.0);
        assert_eq!(
            frame_insets(Decorations::Server, inset),
            gpui::Edges::default()
        );
        for bits in 0..16u8 {
            let tiling = gpui::Tiling {
                top: bits & 1 != 0,
                left: bits & 2 != 0,
                right: bits & 4 != 0,
                bottom: bits & 8 != 0,
            };
            let side = |tiled: bool| if tiled { px(0.0) } else { inset };
            assert_eq!(
                frame_insets(Decorations::Client { tiling }, inset),
                gpui::Edges {
                    top: side(tiling.top),
                    right: side(tiling.right),
                    bottom: side(tiling.bottom),
                    left: side(tiling.left),
                },
                "{tiling:?}"
            );
        }
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
    fn configured_workspace_color_tints_the_title_bar_without_replacing_the_theme() {
        let distance = |a: gpui::Rgba, b: gpui::Rgba| {
            (a.red - b.red).abs() + (a.green - b.green).abs() + (a.blue - b.blue).abs()
        };
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let base = title_bar_background(theme, true, None);
            let blue = workspace_color(Some(gitcomet_state::session::WorkspaceColor::Blue));
            let tinted = title_bar_background(
                theme,
                true,
                Some(gitcomet_state::session::WorkspaceColor::Blue),
            );

            assert_ne!(tinted, base, "a configured group color must be visible");
            assert!(
                distance(tinted, blue) < distance(base, blue),
                "the configured title bar should move toward the selected color"
            );
            assert_eq!(
                title_bar_background(theme, true, None),
                base,
                "Default must preserve the existing theme-derived title bar"
            );
        }
    }

    #[test]
    fn default_workspace_dot_is_blue() {
        // The dot takes no theme, so Default can't borrow the theme accent.
        assert_eq!(
            workspace_color(None),
            workspace_color(Some(gitcomet_state::session::WorkspaceColor::Blue))
        );
    }

    #[test]
    fn workspace_row_tint_is_the_active_title_bar_and_opaque_in_every_theme() {
        for option in crate::theme::available_themes() {
            let theme = AppTheme::from_key(&option.key).expect("bundled theme");
            assert_eq!(workspace_row_tint(None, theme), None, "{}", option.key);
            for (color, label) in crate::workspaces::WORKSPACE_COLORS {
                let Some(color) = color else { continue };
                let tint = workspace_row_tint(Some(color), theme).expect("a chosen colour tints");
                // `InteractionStyle::on_surface` asserts an opaque backing.
                assert_eq!(tint.alpha, 1.0, "{} {label}", option.key);
                let title_bar = title_bar_background(theme, true, Some(color));
                if title_bar.alpha >= 1.0 {
                    assert_eq!(tint, title_bar, "{} {label}", option.key);
                }
            }
        }
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
