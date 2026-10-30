//! Extension views in a hosted diff. The list keeps its uniform rows: a
//! block view reserves as many as its measured height needs and is drawn
//! once, from the first of them on screen, over the rest. A view at a line's
//! end takes no rows: its line's row draws it where its canvas finds the
//! text to end. A view is measured when it first shows and again only when
//! it notifies, a block view's column changes width, or the text style or
//! the row height does.
use super::*;
use gitcomet_extension_api::{DiffHostedView, DiffLineSide, DiffPlacement};
use gpui::{Div, EntityId, TextStyle};
use smallvec::SmallVec;
use std::cell::Cell;
use std::rc::Rc;

/// What the pane knows of one view.
struct HostedViewState {
    /// What it was last measured against, and the size it measured at,
    /// which it draws at; none until it is measured, and none again after it
    /// notified.
    measured: Option<(Measure, Size<Pixels>)>,
    /// Its wrapper's: the diff's keys stand aside while focus is inside.
    focus: gpui::FocusHandle,
}

/// What a view's measured size holds for: its placement, and what its size
/// depends on there. The row height follows the UI scale and the editor's
/// metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Measure {
    /// At its column's width, its height then counted in rows.
    Block { column: Pixels, row_height: Pixels },
    /// At its own width.
    EndOfLine { row_height: Pixels },
}

impl Measure {
    fn of(placement: DiffPlacement, column: Pixels, row_height: Pixels) -> Self {
        match placement {
            DiffPlacement::Block => Self::Block { column, row_height },
            DiffPlacement::EndOfLine => Self::EndOfLine { row_height },
        }
    }

    /// The width to lay the view out at; none for its own.
    fn width(self) -> Option<Pixels> {
        match self {
            Self::Block { column, .. } => Some(column),
            Self::EndOfLine { .. } => None,
        }
    }
}

/// The rows a measured block view takes; one for any other, or before it is
/// measured.
fn measured_rows(measured: Option<(Measure, Size<Pixels>)>) -> usize {
    match measured {
        Some((Measure::Block { row_height, .. }, size)) => {
            ((f32::from(size.height) / f32::from(row_height).max(1.0)).ceil() as usize).max(1)
        }
        Some((Measure::EndOfLine { .. }, _)) | None => 1,
    }
}

/// A hosted diff's extension views, by their entities.
#[derive(Default)]
pub(in crate::view) struct HostedViews {
    views: Arc<[DiffHostedView]>,
    states: FxHashMap<EntityId, HostedViewState>,
    /// The text style they were measured in, and are drawn in rather than
    /// the diff's editor font.
    text: Option<TextStyle>,
    /// The views as last projected, so a row of one can be found again in a
    /// replacement list.
    projected: Vec<EntityId>,
    /// The views after a line's text, by their line, in list order.
    line_end_views: FxHashMap<(DiffLineSide, u32), SmallVec<[usize; 1]>>,
    /// The rows built this frame with views after their text, by row and
    /// column.
    line_ends: FxHashMap<(usize, DiffTextRegion), Rc<HostedLineEnd>>,
}

/// Where a row's text ends, which its canvas works out as it prepaints; the
/// views after the text are then placed there.
pub(in crate::view) struct HostedLineEnd {
    /// How wide the views are together, which the row's scroll range makes
    /// room for.
    pub(in crate::view) width: Pixels,
    /// The text's end in the window, once the canvas has found it.
    pub(in crate::view) x: Cell<Option<Pixels>>,
}

impl HostedViews {
    pub(super) fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    pub(super) fn views(&self) -> &[DiffHostedView] {
        &self.views
    }

    /// Replaces the views, keeping what is known of those that stay.
    /// Whether the list is a new one.
    pub(super) fn set(&mut self, views: Arc<[DiffHostedView]>) -> bool {
        if Arc::ptr_eq(&self.views, &views) {
            return false;
        }
        self.states
            .retain(|id, _| views.iter().any(|view| view.view().entity_id() == *id));
        self.line_end_views.clear();
        for (ix, view) in views.iter().enumerate() {
            match view.placement() {
                DiffPlacement::Block => {}
                DiffPlacement::EndOfLine => {
                    let anchor = view.anchor();
                    self.line_end_views
                        .entry((anchor.side, anchor.line))
                        .or_default()
                        .push(ix);
                }
            }
        }
        self.views = views;
        true
    }

    /// The rows `view` takes.
    pub(super) fn rows(&self, view: &DiffHostedView) -> usize {
        measured_rows(
            self.states
                .get(&view.view().entity_id())
                .and_then(|state| state.measured),
        )
    }

    /// Notes the views' order as projected.
    pub(super) fn project(&mut self) {
        self.projected = self
            .views
            .iter()
            .map(|view| view.view().entity_id())
            .collect();
    }

    /// Where the view projected at `index` now is.
    pub(super) fn reprojected(&self, index: usize) -> Option<usize> {
        let id = self.projected.get(index)?;
        self.views
            .iter()
            .position(|view| view.view().entity_id() == *id)
    }

    fn state(&mut self, id: EntityId, cx: &mut App) -> &mut HostedViewState {
        self.states.entry(id).or_insert_with(|| HostedViewState {
            measured: None,
            focus: cx.focus_handle(),
        })
    }

    /// The width `id` draws at: the one it measured at.
    fn width(&self, id: EntityId) -> Pixels {
        self.states
            .get(&id)
            .and_then(|state| state.measured)
            .map_or(px(0.0), |(_, size)| size.width)
    }
}

/// A view laid out in the text style it is measured in, the same for
/// measuring it and for drawing it: at `width`, or at its own without one.
fn hosted_view_frame(content: AnyView, width: Option<Pixels>, text: &TextStyle) -> Div {
    let mut frame = div();
    if let Some(width) = width {
        frame = frame.w(width);
    }
    *frame.text_style() = text.subtract(&gpui::TextStyleRefinement::default());
    frame.child(content)
}

/// A drawn view's input is its own: the diff's keys stand aside while focus
/// is inside, and its presses neither focus the diff nor start a selection
/// in it.
fn keeps_its_input(frame: gpui::Stateful<Div>, focus: &gpui::FocusHandle) -> gpui::Stateful<Div> {
    frame
        .track_focus(focus)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
}

/// Measures the views that need it, as gpui's `list` measures its items, in
/// the text style the pane's rows are drawn under. The renderer's rows follow
/// in the same frame.
pub(in crate::view) fn measure_hosted_views(
    renderer: &gpui::Entity<DiffRenderer>,
    window: &mut Window,
    cx: &mut App,
) {
    let text = window.text_style();
    let views = renderer.update(cx, |pane, cx| {
        pane.hosted_views_to_measure(&text, window, cx)
    });
    if views.is_empty() {
        return;
    }
    let sizes = views
        .into_iter()
        .map(|(id, view, measure)| {
            let width = measure.width();
            let mut frame = hosted_view_frame(view, width, &text).into_any_element();
            let size = frame.layout_as_root(
                gpui::size(
                    width.map_or(
                        gpui::AvailableSpace::MaxContent,
                        gpui::AvailableSpace::Definite,
                    ),
                    gpui::AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            (id, measure, size)
        })
        .collect();
    renderer.update(cx, |pane, _| pane.set_hosted_view_sizes(sizes));
}

impl DiffRenderer {
    /// How wide views draw in each column, old side then new: the column
    /// less its scrollbar, and never below nothing in a narrow split.
    fn hosted_view_widths(&self, window: &Window) -> [Pixels; 2] {
        let width = self
            .hosted_content_width
            .unwrap_or(window.viewport_size().width);
        self.hosted_card_widths(width)
            .map(|width| width.max(px(0.0)))
    }

    /// The views to measure, each with what it is measured against: those
    /// whose size does not hold for their placement as it now is, which is
    /// those not measured since they showed or notified, block views whose
    /// column changed width, and every one when the row height did, or when
    /// the text style changed.
    fn hosted_views_to_measure(
        &mut self,
        text: &TextStyle,
        window: &Window,
        cx: &mut App,
    ) -> Vec<(EntityId, AnyView, Measure)> {
        let Some(decor) = &self.hosted_decor else {
            return Vec::new();
        };
        if decor.views.is_empty() {
            return Vec::new();
        }
        let widths = self.hosted_view_widths(window);
        let decor = self.hosted_decor.as_mut().expect("hosted renderer");
        let row_height = decor.row_height;
        let views = &mut decor.views;
        let restyled = views.text.as_ref() != Some(text);
        if restyled {
            views.text = Some(text.clone());
        }
        let list = views.views.clone();
        list.iter()
            .filter_map(|view| {
                let column = widths[usize::from(view.anchor().side == DiffLineSide::New)];
                let measure = Measure::of(view.placement(), column, row_height);
                let state = views.state(view.view().entity_id(), cx);
                if !restyled && state.measured.is_some_and(|(at, _)| at == measure) {
                    return None;
                }
                Some((view.view().entity_id(), view.view().clone(), measure))
            })
            .collect()
    }

    /// Takes measured sizes. A view whose rows changed moves the lines after
    /// it, so the rows are projected again.
    fn set_hosted_view_sizes(&mut self, sizes: Vec<(EntityId, Measure, Size<Pixels>)>) {
        let Some(decor) = self.hosted_decor.as_mut() else {
            return;
        };
        for (id, measure, size) in sizes {
            let Some(state) = decor.views.states.get_mut(&id) else {
                continue;
            };
            let rows = measured_rows(state.measured);
            state.measured = Some((measure, size));
            if measured_rows(state.measured) != rows {
                decor.key = None;
            }
        }
    }

    /// Measures the view again before it is next drawn: it notified.
    pub(in crate::view) fn forget_hosted_view_size(&mut self, id: EntityId) {
        if let Some(state) = self
            .hosted_decor
            .as_mut()
            .and_then(|decor| decor.views.states.get_mut(&id))
        {
            state.measured = None;
        }
    }

    /// How many views the pane keeps state for.
    #[cfg(test)]
    pub(in crate::view) fn hosted_view_states(&self) -> usize {
        self.hosted_decor
            .as_ref()
            .map_or(0, |decor| decor.views.states.len())
    }

    /// Whether focus is inside a hosted view: its keys are then its own.
    pub(in crate::view) fn hosted_view_focused(&self, window: &Window, cx: &App) -> bool {
        self.hosted_decor.as_ref().is_some_and(|decor| {
            decor
                .views
                .states
                .values()
                .any(|state| state.focus.contains_focused(window, cx))
        })
    }

    /// One of a view's rows. The first on screen draws the whole view over
    /// the run, in the anchor's column only, held against horizontal scroll;
    /// the others are empty, so one entity is drawn once a frame.
    pub(super) fn render_hosted_view_row(
        &mut self,
        index: usize,
        row: usize,
        side: Option<DiffLineSide>,
        first_on_screen: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let decor = self.hosted_decor.as_ref().expect("hosted renderer");
        let row_height = decor.row_height;
        let placeholder = div().h(row_height).w_full();
        let view = &decor.views.views()[index];
        let own_side = side.is_none_or(|side| side == view.anchor().side);
        if !own_side || !(row == 0 || first_on_screen) {
            return placeholder.into_any_element();
        }
        let pane_id = decor.view_id;
        let id = view.view().entity_id();
        let content = view.view().clone();
        let rows = decor.views.rows(view);
        // The pane measures its views before its rows draw.
        let text = decor.views.text.clone().expect("measured before drawn");
        // Drawn at the width it was measured at, this frame's column's.
        let width = decor.views.width(id);
        let scroll = match side {
            Some(DiffLineSide::New) => &self.diff_split_right_scroll,
            _ => &self.diff_scroll,
        };
        let scroll_x = scroll.0.borrow().base_handle.offset().x;
        let focus = self
            .hosted_decor
            .as_mut()
            .expect("hosted renderer")
            .views
            .state(id, cx)
            .focus
            .clone();
        // Ids from the view's entity, so its element state follows it from
        // row to row as it scrolls.
        placeholder
            .id(("hosted_view_row", id))
            .relative()
            .child(keeps_its_input(
                hosted_view_frame(content, Some(width), &text)
                    .id("hosted_view")
                    .debug_selector(move || format!("hosted_diff_{pane_id}_view_{}", id.as_u64()))
                    .absolute()
                    .top(-row_height * row as f32)
                    .left(-scroll_x)
                    .h(row_height * rows as f32),
                &focus,
            ))
            .into_any_element()
    }

    /// Whether any hosted view goes after a line's text.
    pub(in crate::view) fn has_hosted_line_end_views(&self) -> bool {
        self.hosted_decor
            .as_ref()
            .is_some_and(|decor| !decor.views.line_end_views.is_empty())
    }

    /// Forgets the line ends of `side`'s rows, or an inline diff's, before
    /// they are built again.
    pub(super) fn forget_hosted_line_ends(&mut self, side: Option<DiffLineSide>) {
        let region = hosted_region(side);
        if let Some(decor) = self.hosted_decor.as_mut() {
            decor.views.line_ends.retain(|(_, at), _| *at != region);
        }
    }

    /// Where the views after the text of row `visible_ix` in `region` go, if
    /// it was built with any; its canvas finds the place.
    pub(in crate::view) fn hosted_line_end(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> Option<Rc<HostedLineEnd>> {
        self.hosted_decor
            .as_ref()?
            .views
            .line_ends
            .get(&(visible_ix, region))
            .cloned()
    }

    /// Whether row `visible_ix` is the one its line's text ends on in
    /// `region`: the last of a wrapped line's rows that holds any of it.
    fn is_hosted_line_end_row(&self, visible_ix: usize, region: DiffTextRegion) -> bool {
        if self.diff_text_wrap_for_visible_ix(visible_ix).is_none() {
            return true;
        }
        let rows = &self.diff_wrap_visible_rows;
        let source = rows[visible_ix].source_visible_ix;
        // The wrap projection keeps a line's rows together, in order.
        let after = rows.partition_point(|row| row.source_visible_ix <= source);
        let holds_text = |row: &super::helpers::state::DiffWrapVisualRow| {
            let range = match region {
                DiffTextRegion::Inline | DiffTextRegion::SplitLeft => row.primary_range,
                DiffTextRegion::SplitRight => row.secondary_range,
            };
            row.wrap_ix == 0 || range.start < range.end
        };
        (visible_ix..after).rev().find(|ix| holds_text(&rows[*ix])) == Some(visible_ix)
    }

    /// The views after the text of document row `row`, whose line numbers
    /// are `lines`, in `side`'s column or in an inline diff's one: those
    /// anchored to its line on that side, side by side in list order, one
    /// row high, centred on it and at their measured widths, on the row its
    /// text ends on. They go where the row's canvas finds the text to end, so
    /// they scroll sideways with it.
    pub(super) fn render_hosted_line_end(
        &mut self,
        row: usize,
        side: Option<DiffLineSide>,
        (old, new): (Option<u32>, Option<u32>),
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let decor = self.hosted_decor.as_ref()?;
        let index = &decor.views.line_end_views;
        if index.is_empty() {
            return None;
        }
        let on = |at: DiffLineSide, line: Option<u32>| {
            line.filter(|_| side.is_none_or(|side| side == at))
                .and_then(|line| index.get(&(at, line)))
        };
        let mut ends: SmallVec<[usize; 2]> =
            [on(DiffLineSide::Old, old), on(DiffLineSide::New, new)]
                .into_iter()
                .flatten()
                .flatten()
                .copied()
                .collect();
        let region = hosted_region(side);
        if ends.is_empty() || !self.is_hosted_line_end_row(row, region) {
            return None;
        }
        ends.sort_unstable();
        let pane_id = decor.view_id;
        let row_height = decor.row_height;
        // The pane measures its views before its rows draw.
        let text = decor.views.text.clone().expect("measured before drawn");
        let ends: SmallVec<[(EntityId, AnyView, Pixels); 2]> = ends
            .into_iter()
            .map(|ix| {
                let view = decor.views.views()[ix].view();
                let id = view.entity_id();
                (id, view.clone(), decor.views.width(id))
            })
            .collect();
        let views = &mut self.hosted_decor.as_mut().expect("hosted renderer").views;
        // Ids from the first view's entity, so their element state stays
        // with them when rows come and go above.
        let key = ends[0].0;
        let mut width = px(0.0);
        let frames: Vec<_> = ends
            .into_iter()
            .map(|(id, content, view_width)| {
                width += view_width;
                keeps_its_input(
                    hosted_view_frame(content, Some(view_width), &text)
                        .id(("hosted_view", id))
                        .debug_selector(move || {
                            format!("hosted_diff_{pane_id}_view_{}", id.as_u64())
                        })
                        .flex_none()
                        .h_full()
                        // Centred on the row, a taller view clipped both ways.
                        .flex()
                        .flex_col()
                        .justify_center(),
                    &views.state(id, cx).focus,
                )
            })
            .collect();
        let end = Rc::new(HostedLineEnd {
            width,
            x: Cell::new(None),
        });
        views.line_ends.insert((row, region), end.clone());
        Some(
            HostedLineEndSlot {
                end,
                views: div()
                    .id(("hosted_line_end", key))
                    .flex()
                    .h(row_height)
                    .overflow_hidden()
                    .children(frames)
                    .into_any_element(),
            }
            .into_any_element(),
        )
    }
}

/// The text region of `side`'s column, or of an inline diff's.
fn hosted_region(side: Option<DiffLineSide>) -> DiffTextRegion {
    match side {
        None => DiffTextRegion::Inline,
        Some(DiffLineSide::Old) => DiffTextRegion::SplitLeft,
        Some(DiffLineSide::New) => DiffTextRegion::SplitRight,
    }
}

/// The views after a row's text: it takes no room in the row, and places
/// them where the row's canvas, prepainted before it, found the text to end.
/// Nothing is drawn if the canvas found no end.
struct HostedLineEndSlot {
    end: Rc<HostedLineEnd>,
    views: AnyElement,
}

impl IntoElement for HostedLineEndSlot {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for HostedLineEndSlot {
    type RequestLayoutState = ();
    type PrepaintState = bool;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style {
            position: gpui::Position::Absolute,
            ..Default::default()
        };
        style.inset.top = px(0.0).into();
        style.inset.left = px(0.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let Some(x) = self.end.x.get() else {
            return false;
        };
        self.views.layout_as_root(
            gpui::size(
                gpui::AvailableSpace::Definite(self.end.width),
                gpui::AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        self.views
            .prepaint_at(gpui::point(x, bounds.top()), window, cx);
        true
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        placed: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if *placed {
            self.views.paint(window, cx);
        }
    }
}
