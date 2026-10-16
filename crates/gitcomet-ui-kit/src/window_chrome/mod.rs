//! Window chrome every window of a product shares: the client-side frame and
//! its resize band, title-bar geometry and caption buttons, and the platform
//! glue behind zoom, the system menu and native move/resize.
//!
//! A title bar sits outside the appearance system. It shares its row with the
//! OS window controls, which do not resize, so the bar and its buttons hold one
//! size at every UI scale, density and font size: bar geometry is literal `px`,
//! and shared components drawn into a bar take [`crate::ui_scale::chrome_scale`].
//! The frame around it ([`client_side_decoration_inset`] and the resize band)
//! is a pointer target on the window edge and deliberately follows the UI scale.

use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale;
use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, CursorStyle, Decorations, Pixels, Point, ResizeEdge, Size, Tiling, Window,
    div, point, px, size,
};

pub mod controls;
mod platform;
mod root_hook;

pub use platform::*;
pub use root_hook::WindowRootHook;

pub const CLIENT_SIDE_DECORATION_INSET_PX: f32 = 10.0;
pub const TITLE_BAR_HEIGHT_PX: f32 = 38.0;

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
pub const MACOS_TRAFFIC_LIGHTS_SAFE_INSET: Pixels = px(MACOS_TRAFFIC_LIGHTS_LEADING_PX
    + MACOS_TRAFFIC_LIGHT_BUTTON_WIDTH_PX * 3.0
    + MACOS_TRAFFIC_LIGHT_BUTTON_GAP_PX * 2.0
    + MACOS_TRAFFIC_LIGHTS_CLEARANCE_PX);
const _: () = assert!(MACOS_TRAFFIC_LIGHTS_CLEARANCE_PX > 0.0);

pub const CLIENT_SIDE_DECORATION_INSET: Pixels = px(CLIENT_SIDE_DECORATION_INSET_PX);

pub fn client_side_decoration_inset(ui_scale_percent: u32) -> Pixels {
    ui_scale::design_px_from_percent(CLIENT_SIDE_DECORATION_INSET_PX, ui_scale_percent)
}

pub const TITLE_BAR_HEIGHT: Pixels = px(TITLE_BAR_HEIGHT_PX);

/// Geometry every control in a title bar shares -- the app menu and repository
/// switcher at the leading edge, the min/max/close caption buttons at the
/// trailing one. One set of numbers, so a 16px glyph clears the same 8px on
/// each side at both ends of the bar.
const TITLE_BAR_BUTTON_HEIGHT_PX: f32 = 26.0;
pub const TITLE_BAR_BUTTON_WIDTH_PX: f32 = 32.0;
pub const TITLE_BAR_ICON_SIZE_PX: f32 = 16.0;
/// Spacing around the trailing caption cluster.
const TITLE_BAR_CONTROL_GAP_PX: f32 = 4.0;
const TITLE_BAR_CONTROL_TRAILING_PAD_PX: f32 = 8.0;

pub const TITLE_BAR_BUTTON_HEIGHT: Pixels = px(TITLE_BAR_BUTTON_HEIGHT_PX);
/// The plate has to leave the bar an inset at both edges.
const _: () = assert!(TITLE_BAR_BUTTON_HEIGHT_PX < TITLE_BAR_HEIGHT_PX);

/// The trailing min/max/close cluster. Both title bars build it here: the
/// buttons hold one size, so the spacing around them has to as well, and a
/// single builder is what stops the two bars drifting apart. The list follows
/// the configured button order and is empty on macOS, where AppKit draws them.
pub fn window_controls_cluster(controls: Vec<AnyElement>) -> gpui::Div {
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
pub fn macos_traffic_light_position() -> Point<Pixels> {
    point(
        px(MACOS_TRAFFIC_LIGHTS_LEADING_PX),
        px((TITLE_BAR_HEIGHT_PX - MACOS_TRAFFIC_LIGHT_BUTTON_HEIGHT_PX) / 2.0),
    )
}

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub struct TitleBarDragState {
    should_move: bool,
}

impl TitleBarDragState {
    pub fn on_left_mouse_down(&mut self, click_count: usize) {
        self.should_move = click_count < 2;
    }

    pub fn clear(&mut self) {
        self.should_move = false;
    }

    /// Whether the next drag would move the window.
    pub fn is_move_armed(&self) -> bool {
        self.should_move
    }

    pub fn take_move_request(&mut self) -> bool {
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

pub fn should_handle_titlebar_double_click(click_count: usize, standard_click: bool) -> bool {
    standard_click && click_count == 2
}

fn titlebar_double_click_action() -> TitleBarDoubleClickAction {
    if cfg!(target_os = "macos") {
        TitleBarDoubleClickAction::PlatformDefault
    } else {
        TitleBarDoubleClickAction::ToggleZoom
    }
}

pub fn handle_titlebar_double_click(window: &mut Window) {
    match titlebar_double_click_action() {
        TitleBarDoubleClickAction::PlatformDefault => window.titlebar_double_click(),
        TitleBarDoubleClickAction::ToggleZoom => toggle_window_zoom(window),
    }
}

pub fn show_titlebar_secondary_menu<T: 'static>(
    position: Point<Pixels>,
    window: &Window,
    cx: &mut gpui::Context<T>,
) {
    cx.stop_propagation();

    #[cfg(target_os = "windows")]
    if let Some(request) = window_system_menu_request(window, position) {
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

    show_window_system_menu(window, position);
}

pub fn window_top_left_corner(window: &Window) -> Point<Pixels> {
    window_surface_bounds(window).origin
}

/// The decoration inset the client-side frame pads around the window surface, per
/// side: `inset` on every edge the compositor has not tiled, nothing for a
/// native or suppressed frame. `window_frame` pads by it and popovers fit
/// inside it.
pub fn frame_insets(decorations: Decorations, inset: Pixels) -> gpui::Edges<Pixels> {
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
/// window is maximized or fullscreen, and it includes the decoration inset.
pub fn window_surface_bounds(window: &Window) -> Bounds<Pixels> {
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

pub fn titlebar_control_button(
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
pub struct FrameCornerRounding {
    pub top_left: bool,
    pub top_right: bool,
    pub bottom_left: bool,
    pub bottom_right: bool,
    pub radius: Pixels,
}

pub fn client_frame_corner_rounding(
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

pub fn cursor_style_for_resize_edge(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

pub fn resize_edge(
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

pub fn window_frame(
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
                d.rounded(px(theme.radii.window))
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
        .child(WindowRootHook::new(|window| {
            crate::press_gesture::install_reset(window);
            crate::text_selection_owner::install_reset(window);
        }))
        .child(inner)
        .into_any_element()
}

#[cfg(test)]
mod tests;
