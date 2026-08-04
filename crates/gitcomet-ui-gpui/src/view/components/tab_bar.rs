use super::tab::TAB_HEIGHT_PX;
use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Div, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Point, ScrollHandle, Stateful, Window, div, point, px,
};

/// Sub-pixel slack: layout rounding leaves a hair of scrollable width behind
/// that must not count as overflow.
const SCROLL_EPSILON: Pixels = px(0.5);

/// Scroll state of a tab strip. The view owns one of these so the offset and
/// the last measured overflow state survive re-renders. Repository tabs use
/// wheel/trackpad input and active-tab reveal instead of visible scroll arrows.
#[derive(Clone, Debug)]
pub struct TabBarScroll {
    handle: ScrollHandle,
}

impl Default for TabBarScroll {
    fn default() -> Self {
        Self::new()
    }
}

/// Keeps a trailing control outside the scroll container's input hitbox while
/// painting it at the measured end of the visible tab run. Its normal layout
/// width remains reserved at the viewport edge for overflowing tabs.
struct TabBarEnd {
    child: Option<AnyElement>,
    scroll: Option<TabBarScroll>,
    tab_count: usize,
}

impl IntoElement for TabBarEnd {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TabBarEnd {
    type RequestLayoutState = AnyElement;
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
        let mut child = self.child.take().expect("tab bar end child");
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let offset_x = self
            .scroll
            .as_ref()
            .and_then(|scroll| {
                let viewport = scroll.viewport();
                let desired_left = self
                    .tab_count
                    .checked_sub(1)
                    .and_then(|ix| scroll.tab_bounds(ix))
                    .map_or(viewport.left(), |last| last.right().min(viewport.right()));
                (viewport.size.width > px(0.0)).then(|| (desired_left - bounds.left()).min(px(0.0)))
            })
            .unwrap_or(px(0.0));

        window.with_element_offset(point(offset_x, px(0.0)), |window| {
            child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

impl TabBarScroll {
    pub fn new() -> Self {
        Self {
            handle: ScrollHandle::new(),
        }
    }

    /// Scroll the strip so the tab at `ix` is fully visible. Applied during the
    /// next prepaint, when the tab bounds for this frame are known.
    pub fn scroll_to_tab(&self, ix: usize) {
        self.handle.scroll_to_item(ix);
    }

    /// Whether the strip has been laid out at least once. GPUI resolves
    /// `scroll_to_tab` against the previous frame's viewport, so asking before
    /// the first measurement silently does nothing.
    pub fn is_measured(&self) -> bool {
        self.handle.bounds().size.width > px(0.0)
    }

    /// Whether the tab at `ix` sits fully inside the strip, judged on the last
    /// measured layout. A tab too wide to ever fit counts as shown once its
    /// left edge is in view, so callers can't wait on it forever.
    pub fn tab_is_visible(&self, ix: usize) -> bool {
        let viewport = self.handle.bounds();
        let Some(tab) = self.handle.bounds_for_item(ix) else {
            return false;
        };
        let offset_x = self.offset().x;
        let left = tab.left() + offset_x;
        let right = tab.right() + offset_x;

        if tab.size.width >= viewport.size.width {
            return (left - viewport.left()).abs() <= SCROLL_EPSILON;
        }
        left >= viewport.left() - SCROLL_EPSILON && right <= viewport.right() + SCROLL_EPSILON
    }

    fn offset(&self) -> Point<Pixels> {
        self.handle.offset()
    }

    /// Width scrolled out of view on the left; `0` at the start of the strip.
    pub fn scrolled(&self) -> Pixels {
        -self.offset().x
    }

    pub fn max_scroll(&self) -> Pixels {
        self.handle.max_offset().x
    }

    /// Bounds of the visible strip, as measured during the last prepaint.
    pub fn viewport(&self) -> Bounds<Pixels> {
        self.handle.bounds()
    }

    /// Where the tab at `ix` is actually painted: its layout box shifted by the
    /// current scroll offset.
    pub fn tab_bounds(&self, ix: usize) -> Option<Bounds<Pixels>> {
        let mut bounds = self.handle.bounds_for_item(ix)?;
        bounds.origin.x += self.offset().x;
        Some(bounds)
    }

    /// Whether the strip can still travel in `direction` (negative is left).
    pub fn can_scroll(&self, direction: f32) -> bool {
        let max_scroll = self.max_scroll();
        let scrolled = self.scrolled();
        if direction < 0.0 {
            scrolled > SCROLL_EPSILON
        } else {
            scrolled < max_scroll - SCROLL_EPSILON
        }
    }

    /// Scrolls by `delta` (positive moves the tabs left, revealing later ones).
    /// Returns whether the offset actually moved.
    pub fn scroll_by(&self, delta: Pixels) -> bool {
        let offset = self.offset();
        let x = (offset.x - delta).clamp(-self.max_scroll(), px(0.0));
        if x == offset.x {
            return false;
        }
        self.handle.set_offset(point(x, offset.y));
        true
    }
}

pub struct TabBar {
    id: ElementId,
    tabs: Vec<AnyElement>,
    tab_end: Option<AnyElement>,
    scroll: Option<TabBarScroll>,
}

impl TabBar {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            tabs: Vec::new(),
            tab_end: None,
            scroll: None,
        }
    }

    pub fn tab(mut self, tab: impl IntoElement) -> Self {
        self.tabs.push(tab.into_any_element());
        self
    }

    /// Element laid out after the tab viewport and visually attached to the
    /// final visible tab without becoming part of the scroll hitbox.
    pub fn tab_end(mut self, tab_end: impl IntoElement) -> Self {
        self.tab_end = Some(tab_end.into_any_element());
        self
    }

    /// Makes the strip scroll horizontally once the tabs overflow it.
    pub fn scroll(mut self, scroll: TabBarScroll) -> Self {
        self.scroll = Some(scroll);
        self
    }

    pub fn render(self, _theme: AppTheme, ui_scale_percent: u32) -> Stateful<Div> {
        let ui_scale = UiScale::from_percent(ui_scale_percent);
        let Self {
            id,
            tabs,
            tab_end,
            scroll,
        } = self;
        let tab_count = tabs.len();
        let tab_end = tab_end.map(|child| TabBarEnd {
            child: Some(child),
            scroll: scroll.clone(),
            tab_count,
        });

        // The tabs are direct children of the scroll container on purpose:
        // GPUI measures scrollable content from its immediate children, so a
        // wrapper row would report the viewport width and the strip would clip
        // instead of scroll.
        let tabs = div()
            .id((id.clone(), "tabs"))
            .flex()
            .items_end()
            .w_full()
            .h(ui_scale.px(TAB_HEIGHT_PX))
            .overflow_x_scroll()
            .scrollbar_width(px(0.0))
            .when_some(scroll.as_ref(), |this, scroll| {
                this.track_scroll(&scroll.handle)
            })
            .children(tabs);

        let viewport = div()
            .relative()
            .flex()
            .items_end()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .overflow_x_hidden();

        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .w_full()
            .h_full()
            .child(viewport.child(tabs))
            .children(tab_end)
    }
}
