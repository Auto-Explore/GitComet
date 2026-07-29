use super::{Button, ButtonStyle};
use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale::UiScale;
use crate::view::icons::svg_icon;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Div, ElementId, IntoElement, Pixels, Point, ScrollHandle, Stateful,
    Window, div, point, px,
};
use std::cell::Cell;
use std::rc::Rc;

/// Fraction of the visible strip a single arrow click travels.
const ARROW_SCROLL_PAGE_FRACTION: f32 = 0.75;
/// Floor for an arrow click so a narrow strip still moves a useful amount.
const ARROW_SCROLL_MIN_PX: f32 = 120.0;
/// Sub-pixel slack: layout rounding leaves a hair of scrollable width behind
/// that must not count as overflow or light up an arrow.
const SCROLL_EPSILON: Pixels = px(0.5);

/// How the strip's scroll arrows should look, derived from the last measured
/// layout: whether they show at all, and which way they can still travel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ArrowState {
    visible: bool,
    can_scroll_left: bool,
    can_scroll_right: bool,
}

/// Scroll state of a tab strip. The view owns one of these so the offset and
/// the last measured layout survive re-renders; the strip has no visible
/// scrollbar, so the arrows are the only affordance and have to track it.
#[derive(Clone, Debug)]
pub struct TabBarScroll {
    handle: ScrollHandle,
    arrows: Rc<Cell<ArrowState>>,
}

impl Default for TabBarScroll {
    fn default() -> Self {
        Self::new()
    }
}

impl TabBarScroll {
    pub fn new() -> Self {
        Self {
            handle: ScrollHandle::new(),
            arrows: Rc::new(Cell::new(ArrowState::default())),
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

    /// Arrow state implied by the layout measured during the last prepaint.
    fn arrow_state(&self) -> ArrowState {
        let max_scroll = self.max_scroll();
        let scrolled = self.scrolled();
        ArrowState {
            visible: max_scroll > SCROLL_EPSILON,
            can_scroll_left: scrolled > SCROLL_EPSILON,
            can_scroll_right: scrolled < max_scroll - SCROLL_EPSILON,
        }
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
        let arrows = self.arrow_state();
        if direction < 0.0 {
            arrows.can_scroll_left
        } else {
            arrows.can_scroll_right
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

    fn page(&self) -> Pixels {
        let visible = self.handle.bounds().size.width;
        (visible * ARROW_SCROLL_PAGE_FRACTION).max(px(ARROW_SCROLL_MIN_PX))
    }
}

pub struct TabBar {
    id: ElementId,
    tabs: Vec<AnyElement>,
    filler: Option<AnyElement>,
    tab_end: Option<AnyElement>,
    scroll: Option<TabBarScroll>,
}

impl TabBar {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            tabs: Vec::new(),
            filler: None,
            tab_end: None,
            scroll: None,
        }
    }

    pub fn tab(mut self, tab: impl IntoElement) -> Self {
        self.tabs.push(tab.into_any_element());
        self
    }

    /// Element stretched into the empty space after the tabs (e.g. a window
    /// drag surface). It collapses to zero width once tabs fill the bar.
    pub fn filler(mut self, filler: impl IntoElement) -> Self {
        self.filler = Some(filler.into_any_element());
        self
    }

    /// Element that follows the last tab, browser-style. While the tabs fit it
    /// rides inside the strip, sitting where the next tab would appear; once
    /// they overflow it moves out beside the scroll arrows, where the tabs can
    /// never scroll it out of reach.
    pub fn tab_end(mut self, tab_end: impl IntoElement) -> Self {
        self.tab_end = Some(tab_end.into_any_element());
        self
    }

    /// Makes the strip scroll horizontally once the tabs overflow it.
    pub fn scroll(mut self, scroll: TabBarScroll) -> Self {
        self.scroll = Some(scroll);
        self
    }

    pub fn render(self, theme: AppTheme, ui_scale_percent: u32) -> Stateful<Div> {
        let ui_scale = UiScale::from_percent(ui_scale_percent);
        let Self {
            id,
            tabs,
            filler,
            tab_end,
            scroll,
        } = self;

        // Reflects the previous frame's layout; the listener below asks for a
        // redraw whenever this frame's layout disagrees with it.
        let arrows = scroll
            .as_ref()
            .map(TabBarScroll::arrow_state)
            .unwrap_or_default();

        // The strip carries the end element while everything fits, and hands it
        // to the pinned slot once the arrows appear. The swap cannot oscillate:
        // the pair of arrows is wider than the element, so a strip that
        // overflows with the element inside still overflows once it moves out.
        let (strip_end, pinned_end) = match tab_end {
            Some(tab_end) if arrows.visible => (None, Some(tab_end)),
            tab_end => (tab_end, None),
        };

        // The tabs are direct children of the scroll container on purpose:
        // GPUI measures scrollable content from its immediate children, so a
        // wrapper row would report the viewport width and the strip would clip
        // instead of scroll.
        let tabs = div()
            .id((id.clone(), "tabs"))
            .flex()
            .items_end()
            .size_full()
            .overflow_x_scroll()
            .scrollbar_width(px(0.0))
            .when_some(scroll.as_ref(), |this, scroll| {
                this.track_scroll(&scroll.handle)
            })
            .children(tabs)
            // After the tabs, so tab indices still address tabs: the scroll
            // handle addresses its children positionally.
            .children(strip_end)
            .when_some(filler, |this, filler| {
                this.child(div().flex_1().min_w(px(0.0)).h_full().child(filler))
            });

        let mut viewport = div()
            .relative()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .overflow_x_hidden();
        if let Some(scroll) = scroll.clone() {
            viewport = viewport.on_children_prepainted(move |_bounds, _window, cx| {
                let measured = scroll.arrow_state();
                if scroll.arrows.replace(measured) != measured {
                    // `Window::refresh` is a no-op mid-draw; an app effect is
                    // the way to get another frame out of prepaint.
                    cx.refresh_windows();
                }
            });
        }

        div()
            .id(id)
            .group("tab_bar")
            .flex()
            .flex_none()
            .items_center()
            .w_full()
            .h_full()
            .when(arrows.visible, |this| {
                let scroll = scroll
                    .clone()
                    .expect("visible arrows imply a scroll handle");
                this.child(scroll_arrow(
                    "tab_bar_scroll_left",
                    "icons/chevron_left.svg",
                    theme,
                    ui_scale,
                    arrows.can_scroll_left,
                    move |window| {
                        scroll.scroll_by(-scroll.page());
                        window.refresh();
                    },
                ))
            })
            .child(viewport.child(tabs))
            .when(arrows.visible, |this| {
                let scroll = scroll
                    .clone()
                    .expect("visible arrows imply a scroll handle");
                this.child(scroll_arrow(
                    "tab_bar_scroll_right",
                    "icons/chevron_right.svg",
                    theme,
                    ui_scale,
                    arrows.can_scroll_right,
                    move |window| {
                        scroll.scroll_by(scroll.page());
                        window.refresh();
                    },
                ))
            })
            .children(pinned_end)
    }
}

fn scroll_arrow(
    id: &'static str,
    icon: &'static str,
    theme: AppTheme,
    ui_scale: UiScale,
    enabled: bool,
    on_click: impl Fn(&mut Window) + 'static,
) -> impl IntoElement {
    let color = if enabled {
        theme.colors.text_muted
    } else {
        with_alpha(theme.colors.text_muted, 0.35)
    };

    div().flex_none().h_full().flex().items_center().child(
        Button::new(id, "")
            .start_slot(svg_icon(icon, color, ui_scale.px(12.0)))
            .style(ButtonStyle::Transparent)
            .borderless()
            .disabled(!enabled)
            .render(theme, ui_scale)
            .debug_selector(move || id.to_string())
            .when(enabled, |this| {
                this.on_click(move |_e, window, _cx: &mut App| on_click(window))
            }),
    )
}
