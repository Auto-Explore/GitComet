//! Where a menu goes. Native menus, GTK's anchor hints and Floating UI all use
//! the same order: open on the preferred side, flip to the other side, slide
//! inside the window, and only then shrink and scroll. The shrinking is the
//! host's height cap ([`MenuPlacementState::height_cap`]); everything else is
//! decided here from the menu's measured size.

use gpui::{
    AnyElement, App, Bounds, Display, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Point, Position, Size, Style, Window, point, px,
};
use std::cell::RefCell;
use std::rc::Rc;

/// What the menu opens from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAnchor {
    /// A pointer position (a right-click). The menu may slide over it.
    Point(Point<Pixels>),
    /// The control that opened the menu. The menu never covers it vertically.
    Trigger(Bounds<Pixels>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuRequest {
    pub anchor: MenuAnchor,
    /// Opens downward unless only the space above fits it.
    pub prefer_below: bool,
    /// Opens rightward (left edge at the anchor) unless only leftward fits.
    pub prefer_right: bool,
    /// Vertical space between the anchor and the menu.
    pub gap: Pixels,
}

/// A resolved menu position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuPlacement {
    pub origin: Point<Pixels>,
    /// Whether the menu ended up below its anchor (including a slide that
    /// started from below).
    pub below: bool,
}

/// One axis. A forward menu starts at `after` and grows toward `hi`; a
/// backward one ends at `before` and grows toward `lo`.
struct Axis {
    len: Pixels,
    after: Pixels,
    before: Pixels,
    lo: Pixels,
    hi: Pixels,
    prefer_forward: bool,
    /// Whether the menu may slide over the anchor when neither side fits.
    may_cover_anchor: bool,
}

impl Axis {
    fn room_forward(&self) -> Pixels {
        self.hi - self.after
    }

    fn room_backward(&self) -> Pixels {
        self.before - self.lo
    }

    /// Returns the start coordinate and whether the menu opened forward.
    fn place(mut self) -> (Pixels, bool) {
        // An anchor outside the limits (the window shrank under an open menu)
        // opens from the nearest edge.
        self.after = self.after.max(self.lo);
        self.before = self.before.min(self.hi);
        let forward = (self.after, true);
        let backward = (self.before - self.len, false);
        let fits_forward = self.len <= self.room_forward();
        let fits_backward = self.len <= self.room_backward();
        let (first, second, first_fits, second_fits) = if self.prefer_forward {
            (forward, backward, fits_forward, fits_backward)
        } else {
            (backward, forward, fits_backward, fits_forward)
        };
        if first_fits {
            return first;
        }
        if second_fits {
            return second;
        }

        let roomier_forward = self.room_forward() >= self.room_backward();
        let start = if self.may_cover_anchor {
            // Slide from the roomier side's placement just far enough to fit.
            if roomier_forward {
                self.hi - self.len
            } else {
                self.lo
            }
        } else if roomier_forward {
            // The host caps the height to this side, so this fits; if it
            // does not, staying visible beats keeping the anchor clear.
            self.after.min(self.hi - self.len)
        } else {
            self.before - self.len
        };
        // Taller than the limits: pin the start so the top stays reachable.
        (start.max(self.lo), roomier_forward)
    }
}

/// Places a menu of `size` inside `limits` (the usable window, margins
/// already removed).
pub fn place_menu(
    request: &MenuRequest,
    size: Size<Pixels>,
    limits: Bounds<Pixels>,
) -> MenuPlacement {
    let (vertical, horizontal) = match request.anchor {
        MenuAnchor::Point(at) => (
            Axis {
                len: size.height,
                after: at.y + request.gap,
                before: at.y - request.gap,
                lo: limits.top(),
                hi: limits.bottom(),
                prefer_forward: request.prefer_below,
                may_cover_anchor: true,
            },
            Axis {
                len: size.width,
                after: at.x,
                before: at.x,
                lo: limits.left(),
                hi: limits.right(),
                prefer_forward: request.prefer_right,
                may_cover_anchor: true,
            },
        ),
        MenuAnchor::Trigger(trigger) => (
            Axis {
                len: size.height,
                after: trigger.bottom() + request.gap,
                before: trigger.top() - request.gap,
                lo: limits.top(),
                hi: limits.bottom(),
                prefer_forward: request.prefer_below,
                may_cover_anchor: false,
            },
            Axis {
                len: size.width,
                after: trigger.left(),
                before: trigger.right(),
                lo: limits.left(),
                hi: limits.right(),
                prefer_forward: request.prefer_right,
                may_cover_anchor: true,
            },
        ),
    };
    let (y, below) = vertical.place();
    let (x, _) = horizontal.place();
    MenuPlacement {
        origin: point(x, y),
        below,
    }
}

/// The placement of one open menu across frames. Until the user interacts
/// with the menu it is re-placed every frame, so content that loads late can
/// still pick the side it fits. After [`Self::latch`] it stays put: the top
/// edge is pinned and growth extends downward, then scrolls, so rows never
/// jump under the pointer.
#[derive(Clone, Debug, Default)]
pub struct MenuPlacementState {
    latched: bool,
    last: Option<Resolved>,
}

#[derive(Clone, Copy, Debug)]
struct Resolved {
    placement: MenuPlacement,
    size: Size<Pixels>,
    limits: Bounds<Pixels>,
}

impl MenuPlacementState {
    /// Forgets the previous menu. Call whenever a menu opens or closes.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Freezes the placement from the next frame on.
    pub fn latch(&mut self) {
        self.latched = true;
    }

    pub fn is_latched(&self) -> bool {
        self.latched
    }

    /// The tallest the menu (outer box) may be this frame.
    pub fn height_cap(&self, request: &MenuRequest, limits: Bounds<Pixels>) -> Pixels {
        let below_trigger_only = |below: bool| match request.anchor {
            MenuAnchor::Trigger(trigger) if !below => trigger.top() - request.gap,
            _ => limits.bottom(),
        };
        if self.latched
            && let Some(last) = self.last
        {
            let top = last.placement.origin.y.max(limits.top());
            return (below_trigger_only(last.placement.below).min(limits.bottom()) - top)
                .max(px(0.0));
        }
        match request.anchor {
            MenuAnchor::Point(_) => limits.size.height.max(px(0.0)),
            MenuAnchor::Trigger(trigger) => {
                let below = limits.bottom() - (trigger.bottom() + request.gap);
                let above = (trigger.top() - request.gap) - limits.top();
                below.max(above).max(px(0.0))
            }
        }
    }

    /// Where a menu of `size` goes this frame.
    pub fn resolve(
        &mut self,
        request: &MenuRequest,
        size: Size<Pixels>,
        limits: Bounds<Pixels>,
    ) -> Point<Pixels> {
        if self.latched
            && let Some(last) = self.last
            && (last.limits == limits || fits_within(last.placement.origin, size, limits))
        {
            // Pinned top-left; a menu that grew wider than the room to its
            // right still slides back inside rather than being cut off.
            let origin = point(
                last.placement
                    .origin
                    .x
                    .min(limits.right() - size.width)
                    .max(limits.left()),
                last.placement.origin.y,
            );
            let placement = MenuPlacement {
                origin,
                ..last.placement
            };
            self.last = Some(Resolved {
                placement,
                size,
                limits,
            });
            return origin;
        }

        let placement = place_menu(request, size, limits);
        self.last = Some(Resolved {
            placement,
            size,
            limits,
        });
        placement.origin
    }

    /// The size measured in the last frame, if any.
    pub fn last_size(&self) -> Option<Size<Pixels>> {
        self.last.map(|last| last.size)
    }
}

/// Positions `child` where `state` resolves it, from the child's size measured
/// in this same frame, so a menu never flashes in the wrong spot. The host
/// caps the child's height with [`MenuPlacementState::height_cap`]. Like
/// gpui's `anchored`, the child must not carry a margin.
pub fn menu_placement(
    state: Rc<RefCell<MenuPlacementState>>,
    request: MenuRequest,
    limits: Bounds<Pixels>,
    child: impl IntoElement,
) -> MenuPlacementElement {
    MenuPlacementElement {
        state,
        request,
        limits,
        child: Some(child.into_any_element()),
    }
}

pub struct MenuPlacementElement {
    state: Rc<RefCell<MenuPlacementState>>,
    request: MenuRequest,
    limits: Bounds<Pixels>,
    child: Option<AnyElement>,
}

pub struct MenuPlacementLayout {
    child: AnyElement,
    child_layout_id: LayoutId,
}

impl Element for MenuPlacementElement {
    type RequestLayoutState = MenuPlacementLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child = self.child.take().expect("menu placement child");
        let child_layout_id = child.request_layout(window, cx);
        let style = Style {
            position: Position::Absolute,
            display: Display::Flex,
            ..Style::default()
        };
        let layout_id = window.request_layout(style, [child_layout_id], cx);
        (
            layout_id,
            MenuPlacementLayout {
                child,
                child_layout_id,
            },
        )
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let size = window.layout_bounds(layout.child_layout_id).size;
        let origin = self
            .state
            .borrow_mut()
            .resolve(&self.request, size, self.limits);
        let offset = origin - bounds.origin;
        let offset = point(offset.x.round(), offset.y.round());
        window.with_element_offset(offset, |window| layout.child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        layout.child.paint(window, cx);
    }
}

impl IntoElement for MenuPlacementElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The scroll distance (0 at the top, `max` at the bottom) that fully reveals
/// the row `top..bottom` of a viewport `viewport_h` tall, moving as little as
/// possible. `edge` is the height of the scroll arrows, which cover content at
/// an edge whenever more lies beyond it.
pub fn reveal_scroll(
    top: Pixels,
    bottom: Pixels,
    viewport_h: Pixels,
    max: Pixels,
    current: Pixels,
    edge: Pixels,
) -> Pixels {
    let max = max.max(px(0.0));
    let covered_top = if current > px(0.0) { edge } else { px(0.0) };
    let covered_bottom = if current < max { edge } else { px(0.0) };
    if top < current + covered_top {
        // Scrolled anywhere but the very top, the up arrow covers `edge`.
        let target = top - edge;
        return if target <= px(0.0) {
            px(0.0)
        } else {
            target.min(max)
        };
    }
    if bottom > current + viewport_h - covered_bottom {
        let target = bottom + edge - viewport_h;
        return if target >= max {
            max
        } else {
            target.max(px(0.0))
        };
    }
    current
}

fn fits_within(origin: Point<Pixels>, size: Size<Pixels>, limits: Bounds<Pixels>) -> bool {
    origin.x >= limits.left()
        && origin.y >= limits.top()
        && origin.x + size.width <= limits.right()
        && origin.y + size.height <= limits.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    fn limits() -> Bounds<Pixels> {
        // A 1000×800 surface with 16px margins.
        Bounds {
            origin: point(px(16.0), px(16.0)),
            size: size(px(968.0), px(768.0)),
        }
    }

    fn at(x: f32, y: f32) -> MenuRequest {
        MenuRequest {
            anchor: MenuAnchor::Point(point(px(x), px(y))),
            prefer_below: true,
            prefer_right: true,
            gap: px(8.0),
        }
    }

    fn trigger(x: f32, y: f32, w: f32, h: f32) -> MenuRequest {
        MenuRequest {
            anchor: MenuAnchor::Trigger(Bounds {
                origin: point(px(x), px(y)),
                size: size(px(w), px(h)),
            }),
            prefer_below: true,
            prefer_right: true,
            gap: px(1.0),
        }
    }

    fn menu(w: f32, h: f32) -> Size<Pixels> {
        size(px(w), px(h))
    }

    #[test]
    fn a_menu_that_fits_below_opens_below_right_of_the_pointer() {
        let placed = place_menu(&at(100.0, 100.0), menu(200.0, 300.0), limits());
        assert_eq!(placed.origin, point(px(100.0), px(108.0)));
        assert!(placed.below);
    }

    #[test]
    fn a_menu_that_only_fits_above_flips_above_the_pointer() {
        // 300px below the pointer, 480px above it.
        let placed = place_menu(&at(100.0, 500.0), menu(200.0, 400.0), limits());
        assert_eq!(placed.origin.y, px(500.0 - 8.0 - 400.0));
        assert!(!placed.below);
    }

    #[test]
    fn a_menu_that_prefers_above_stays_above_when_it_fits() {
        let request = MenuRequest {
            prefer_below: false,
            ..at(100.0, 500.0)
        };
        let placed = place_menu(&request, menu(200.0, 100.0), limits());
        assert_eq!(placed.origin.y, px(392.0));
        assert!(!placed.below);
    }

    #[test]
    fn a_menu_that_fits_neither_side_slides_toward_the_roomier_side() {
        // 600px menu, pointer at 450: 328 below, 418 above -> slide down from
        // the top limit, covering the pointer.
        let placed = place_menu(&at(100.0, 450.0), menu(200.0, 600.0), limits());
        assert_eq!(placed.origin.y, px(16.0));
        // Pointer at 350: 418 below, 318 above -> bottom at the bottom limit.
        let placed = place_menu(&at(100.0, 350.0), menu(200.0, 600.0), limits());
        assert_eq!(placed.origin.y, px(784.0 - 600.0));
        assert!(placed.below);
    }

    #[test]
    fn a_menu_taller_than_the_limits_pins_its_top() {
        let placed = place_menu(&at(100.0, 400.0), menu(200.0, 2000.0), limits());
        assert_eq!(placed.origin.y, px(16.0));
    }

    #[test]
    fn a_menu_flips_and_slides_horizontally() {
        // Fits only to the left of the pointer.
        let placed = place_menu(&at(900.0, 100.0), menu(200.0, 100.0), limits());
        assert_eq!(placed.origin.x, px(700.0));
        // Wider than either side: slides inside.
        let placed = place_menu(&at(500.0, 100.0), menu(700.0, 100.0), limits());
        assert_eq!(placed.origin.x, px(984.0 - 700.0));
        // Wider than the window: pinned to the left limit.
        let placed = place_menu(&at(500.0, 100.0), menu(2000.0, 100.0), limits());
        assert_eq!(placed.origin.x, px(16.0));
    }

    #[test]
    fn a_right_aligned_trigger_menu_ends_at_the_trigger_right_edge() {
        let request = MenuRequest {
            prefer_right: false,
            ..trigger(700.0, 10.0, 30.0, 20.0)
        };
        let placed = place_menu(&request, menu(200.0, 100.0), limits());
        assert_eq!(placed.origin.x, px(730.0 - 200.0));
        assert_eq!(placed.origin.y, px(31.0));
    }

    #[test]
    fn a_trigger_menu_never_covers_its_trigger() {
        // Trigger at y 400..420: 363 below, 383 above. A 600px menu fits
        // neither side, so it opens on the larger side (above), flush to it.
        let request = trigger(100.0, 400.0, 40.0, 20.0);
        let placed = place_menu(&request, menu(200.0, 383.0), limits());
        assert_eq!(placed.origin.y + px(383.0), px(399.0));
        assert!(!placed.below);

        let state = MenuPlacementState::default();
        assert_eq!(state.height_cap(&request, limits()), px(383.0));
    }

    #[test]
    fn a_trigger_menu_flips_above_a_trigger_at_the_bottom() {
        let request = trigger(100.0, 760.0, 40.0, 20.0);
        let placed = place_menu(&request, menu(200.0, 300.0), limits());
        assert_eq!(placed.origin.y, px(759.0 - 300.0));
    }

    #[test]
    fn the_unlatched_cap_is_the_whole_window_for_pointer_menus() {
        let state = MenuPlacementState::default();
        assert_eq!(state.height_cap(&at(100.0, 700.0), limits()), px(768.0));
    }

    #[test]
    fn an_unlatched_menu_is_re_placed_as_it_grows() {
        let mut state = MenuPlacementState::default();
        let request = at(100.0, 500.0);
        assert_eq!(
            state.resolve(&request, menu(200.0, 100.0), limits()).y,
            px(508.0)
        );
        // Loaded rows no longer fit below: it flips.
        assert_eq!(
            state.resolve(&request, menu(200.0, 400.0), limits()).y,
            px(92.0)
        );
    }

    #[test]
    fn a_latched_menu_keeps_its_top_and_grows_downward_to_the_margin() {
        let mut state = MenuPlacementState::default();
        let request = at(100.0, 500.0);
        let first = state.resolve(&request, menu(200.0, 400.0), limits());
        assert_eq!(first.y, px(92.0));
        state.latch();
        assert_eq!(state.height_cap(&request, limits()), px(784.0 - 92.0));

        let grown = state.resolve(&request, menu(200.0, 600.0), limits());
        assert_eq!(grown, first);
    }

    #[test]
    fn a_latched_trigger_menu_above_its_trigger_does_not_grow_over_it() {
        let mut state = MenuPlacementState::default();
        let request = trigger(100.0, 760.0, 40.0, 20.0);
        let first = state.resolve(&request, menu(200.0, 300.0), limits());
        state.latch();
        assert_eq!(state.height_cap(&request, limits()), px(759.0) - first.y);
    }

    #[test]
    fn a_latched_menu_keeps_its_place_when_the_window_changes_but_it_still_fits() {
        let mut state = MenuPlacementState::default();
        let request = at(100.0, 100.0);
        let first = state.resolve(&request, menu(200.0, 300.0), limits());
        state.latch();
        let taller = Bounds {
            size: size(px(968.0), px(1200.0)),
            ..limits()
        };
        assert_eq!(state.resolve(&request, menu(200.0, 300.0), taller), first);
    }

    #[test]
    fn a_latched_menu_is_re_placed_when_the_window_shrinks_under_it() {
        let mut state = MenuPlacementState::default();
        let request = at(100.0, 500.0);
        state.resolve(&request, menu(200.0, 250.0), limits());
        state.latch();
        let short = Bounds {
            size: size(px(968.0), px(400.0)),
            ..limits()
        };
        let moved = state.resolve(&request, menu(200.0, 250.0), short);
        assert!(moved.y + px(250.0) <= short.bottom());
        assert!(state.is_latched());
    }

    #[test]
    fn reveal_leaves_a_visible_row_alone() {
        let at = reveal_scroll(
            px(100.0),
            px(130.0),
            px(300.0),
            px(500.0),
            px(50.0),
            px(0.0),
        );
        assert_eq!(at, px(50.0));
    }

    #[test]
    fn reveal_scrolls_a_row_below_the_fold_just_into_view() {
        let at = reveal_scroll(px(400.0), px(430.0), px(300.0), px(500.0), px(0.0), px(0.0));
        assert_eq!(at, px(130.0));
        // With arrows, the down arrow still covers the bottom edge there.
        let at = reveal_scroll(
            px(400.0),
            px(430.0),
            px(300.0),
            px(500.0),
            px(0.0),
            px(16.0),
        );
        assert_eq!(at, px(146.0));
    }

    #[test]
    fn reveal_scrolls_a_row_above_the_fold_just_into_view() {
        let at = reveal_scroll(
            px(200.0),
            px(230.0),
            px(300.0),
            px(500.0),
            px(260.0),
            px(16.0),
        );
        assert_eq!(at, px(184.0));
    }

    #[test]
    fn reveal_snaps_to_the_ends_where_an_arrow_disappears() {
        // The first row: scrolling to 0 hides the up arrow, so no margin.
        let at = reveal_scroll(px(0.0), px(30.0), px(300.0), px(500.0), px(260.0), px(16.0));
        assert_eq!(at, px(0.0));
        // The last row: at max the down arrow is gone.
        let at = reveal_scroll(
            px(770.0),
            px(800.0),
            px(300.0),
            px(500.0),
            px(0.0),
            px(16.0),
        );
        assert_eq!(at, px(500.0));
        // A row the up arrow covers at offset 10 counts as hidden.
        let at = reveal_scroll(px(12.0), px(40.0), px(300.0), px(500.0), px(10.0), px(16.0));
        assert_eq!(at, px(0.0));
    }

    #[test]
    fn reset_forgets_the_latch() {
        let mut state = MenuPlacementState::default();
        state.resolve(&at(100.0, 100.0), menu(200.0, 100.0), limits());
        state.latch();
        state.reset();
        assert!(!state.is_latched());
        assert_eq!(state.last_size(), None);
    }
}
