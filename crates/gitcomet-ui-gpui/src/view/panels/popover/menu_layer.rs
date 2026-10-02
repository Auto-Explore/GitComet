//! Where `popover_view` puts a context menu and how tall it may be. The kit's
//! `menu_placement` decides flip and slide from the measured menu. This module
//! turns a popover anchor into that request and caps the height so the menu
//! only scrolls when the window cannot show all of it.

use super::*;
use crate::kit::menu_placement::{MenuAnchor, MenuRequest};

/// The context menu's placement inputs for one frame.
pub(super) struct ContextMenuLayout {
    pub(super) request: MenuRequest,
    /// The usable window: the visible surface less the popover margin.
    pub(super) limits: Bounds<Pixels>,
    /// The tallest the scrolling content may be inside the surface chrome.
    pub(super) content_max_h: Pixels,
}

impl PopoverHost {
    pub(super) fn context_menu_layout(
        &self,
        kind: &PopoverKind,
        window: &Window,
        ui_scale: ui_scale::UiScale,
    ) -> ContextMenuLayout {
        let scaled_px = crate::ui_scale::scaler(ui_scale);
        let corner = popover_anchor_corner(kind);
        let prefer_below = !matches!(corner, Anchor::BottomLeft | Anchor::BottomRight);
        let prefer_right = !matches!(corner, Anchor::TopRight | Anchor::BottomRight);

        let (anchor, gap) = match (&self.popover_anchor, kind) {
            // The app menu drops from the title bar, not from the corner it
            // is anchored to.
            (Some(PopoverAnchor::Point(corner)), PopoverKind::AppMenu) => (
                MenuAnchor::Trigger(Bounds {
                    origin: *corner,
                    size: size(px(0.0), crate::view::chrome::TITLE_BAR_HEIGHT),
                }),
                px(0.0),
            ),
            (Some(PopoverAnchor::Point(point)), _) => (
                MenuAnchor::Point(*point),
                scaled_px(if prefer_right { 8.0 } else { 10.0 }),
            ),
            (Some(PopoverAnchor::Bounds(bounds)), _) => (MenuAnchor::Trigger(*bounds), px(1.0)),
            (Some(PopoverAnchor::Centered) | None, _) => {
                (MenuAnchor::Point(point(px(64.0), px(64.0))), scaled_px(8.0))
            }
        };
        let request = MenuRequest {
            anchor,
            prefer_below,
            prefer_right,
            gap,
        };

        let limits = menu_limits(window, scaled_px(16.0));

        let outer_max_h = self
            .context_menu_placement
            .borrow()
            .height_cap(&request, limits);
        // The surface's `p_1` padding and 1px border on both sides.
        let chrome = window.rem_size() * 0.25 * 2.0 + px(2.0);
        ContextMenuLayout {
            request,
            limits,
            content_max_h: (outer_max_h - chrome).max(px(0.0)),
        }
    }

    /// Freezes the open menu where it is: from now on growth extends it
    /// downward and then scrolls, so rows never jump under the pointer.
    pub(super) fn latch_context_menu_placement(&self) {
        self.context_menu_placement.borrow_mut().latch();
    }
}

/// How fast a hovered scroll arrow scrolls, in design px per second.
const ARROW_SCROLL_PX_PER_SEC: f32 = 480.0;
/// The longest frame step an arrow scrolls by, so a stalled frame does not
/// jump the menu.
const ARROW_SCROLL_MAX_STEP: std::time::Duration = std::time::Duration::from_millis(50);

/// The usable window for a floating menu: the visible surface less `margin`.
pub(super) fn menu_limits(window: &Window, margin: Pixels) -> Bounds<Pixels> {
    let surface = crate::view::chrome::window_surface_bounds(window);
    Bounds::from_corners(
        point(surface.left() + margin, surface.top() + margin),
        point(
            (surface.right() - margin).max(surface.left() + margin),
            (surface.bottom() - margin).max(surface.top() + margin),
        ),
    )
}

/// Whether rows are hidden past the menu's `direction` edge.
fn menu_can_scroll(scroll: &ScrollHandle, direction: components::MenuScrollDirection) -> bool {
    let offset = scroll.offset().y;
    match direction {
        components::MenuScrollDirection::Up => offset < px(-0.5),
        components::MenuScrollDirection::Down => offset > -scroll.max_offset().y + px(0.5),
    }
}

/// One floating menu's scroll arrows: which one the pointer rests on, and when
/// it last scrolled. Shared cells, so the arrows' own listeners update them.
#[derive(Clone, Default)]
pub(super) struct MenuArrows {
    hovered: Rc<std::cell::Cell<Option<components::MenuScrollDirection>>>,
    tick: Rc<std::cell::Cell<Option<std::time::Instant>>>,
}

impl MenuArrows {
    pub(super) fn reset(&self) {
        self.hovered.set(None);
        self.tick.set(None);
    }

    /// The top or bottom arrow over `scroll`. It is laid out every frame but
    /// painted only while rows are hidden past its edge, judged after the
    /// scroll area has measured itself this frame. Hovering it latches the
    /// menu's placement; pressing it neither dismisses the menu nor moves
    /// focus or a selection.
    pub(super) fn arrow(
        &self,
        direction: components::MenuScrollDirection,
        scroll: &ScrollHandle,
        placement: &Rc<std::cell::RefCell<crate::kit::menu_placement::MenuPlacementState>>,
        selector_prefix: &'static str,
        theme: AppTheme,
        ui_scale: ui_scale::UiScale,
        cx: &mut gpui::Context<PopoverHost>,
    ) -> impl IntoElement + use<> {
        let gate_scroll = scroll.clone();
        let (id, suffix) = match direction {
            components::MenuScrollDirection::Up => (0_usize, "up"),
            components::MenuScrollDirection::Down => (1_usize, "down"),
        };
        let selector = format!("{selector_prefix}_{suffix}");
        let hovered = self.hovered.get() == Some(direction);
        let hover_state = self.clone();
        let placement = placement.clone();
        let host = cx.entity_id();
        components::painted_when(
            move || menu_can_scroll(&gate_scroll, direction),
            components::context_menu_scroll_arrow(theme, ui_scale, direction, hovered)
                .id((selector_prefix, id))
                .debug_selector(move || selector.clone())
                // Rows underneath take no hover or click; the wheel still scrolls.
                .block_mouse_except_scroll()
                .on_hover(move |hovered: &bool, _window, cx| {
                    if *hovered {
                        placement.borrow_mut().latch();
                        hover_state.hovered.set(Some(direction));
                        hover_state.tick.set(None);
                        cx.notify(host);
                    } else if hover_state.hovered.get() == Some(direction) {
                        hover_state.hovered.set(None);
                        cx.notify(host);
                    }
                })
                .on_any_mouse_down(|_e, window, cx| {
                    crate::text_selection_owner::preserve(cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }),
        )
    }

    /// Scrolls `scroll` while the pointer rests on one of its arrows: one
    /// time-scaled step per frame, asking for the next frame until the edge.
    /// Called while rendering the menu, before its layout.
    pub(super) fn drive(
        &self,
        scroll: &ScrollHandle,
        window: &Window,
        ui_scale: ui_scale::UiScale,
        now: std::time::Instant,
    ) {
        let Some(direction) = self.hovered.get() else {
            self.tick.set(None);
            return;
        };
        let viewport = scroll.bounds();
        let strip_h = ui_scale.px(components::MENU_SCROLL_ARROW_HEIGHT_PX);
        let band_top = match direction {
            components::MenuScrollDirection::Up => viewport.top(),
            components::MenuScrollDirection::Down => viewport.bottom() - strip_h,
        };
        let band = Bounds {
            origin: point(viewport.left(), band_top),
            size: size(viewport.size.width, strip_h),
        };
        if !band.contains(&window.mouse_position()) {
            // An arrow that stopped painting never reports the hover ending.
            self.reset();
            return;
        }
        if !menu_can_scroll(scroll, direction) {
            self.tick.set(None);
            return;
        }

        let step = self
            .tick
            .get()
            .map_or(std::time::Duration::ZERO, |tick| {
                now.saturating_duration_since(tick)
            })
            .min(ARROW_SCROLL_MAX_STEP);
        self.tick.set(Some(now));
        let delta = ui_scale.px(ARROW_SCROLL_PX_PER_SEC * step.as_secs_f32());
        let offset = scroll.offset();
        let y = match direction {
            components::MenuScrollDirection::Up => offset.y + delta,
            components::MenuScrollDirection::Down => offset.y - delta,
        }
        .clamp(-scroll.max_offset().y, px(0.0));
        scroll.set_offset(point(offset.x, y));
        window.request_animation_frame();
    }
}

/// The scroll area of a floating menu with its two arrows laid over it. The
/// arrows come after the area so they paint over it and judge its overflow
/// from this frame's measurement.
pub(super) fn scroll_area_with_arrows(
    area: impl IntoElement,
    up: impl IntoElement,
    down: impl IntoElement,
) -> gpui::Div {
    div().relative().child(area).child(up).child(down)
}
