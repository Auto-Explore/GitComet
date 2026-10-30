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
use std::cell::Cell;
use std::rc::Rc;

/// What the pane knows of one view.
struct HostedViewState {
    /// A block view's rows from its last measured height; one until it is
    /// measured.
    rows: usize,
    /// Whether what was measured still holds: not after it notified.
    measured: bool,
    /// The width it draws at: its column's for a block view, the one it
    /// measured at for a view at a line's end.
    width: Pixels,
    /// The row height its rows were counted in, which follows the UI scale
    /// and the editor's metrics.
    row_height: Pixels,
    /// Its wrapper's: the diff's keys stand aside while focus is inside.
    focus: gpui::FocusHandle,
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
        self.views = views;
        true
    }

    /// The rows `view` takes.
    pub(super) fn rows(&self, view: &DiffHostedView) -> usize {
        self.states
            .get(&view.view().entity_id())
            .map_or(1, |state| state.rows)
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
            rows: 1,
            measured: false,
            width: px(0.0),
            row_height: px(0.0),
            focus: cx.focus_handle(),
        })
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
        .map(|(id, view, placement, column)| {
            let width = match placement {
                DiffPlacement::Block => Some(column),
                DiffPlacement::EndOfLine => None,
            };
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
            (id, placement, size)
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

    /// The views to measure, each with its column's width: those not
    /// measured since they showed or notified, block views whose column
    /// changed width, and every one when the text style or the row height
    /// changed.
    fn hosted_views_to_measure(
        &mut self,
        text: &TextStyle,
        window: &Window,
        cx: &mut App,
    ) -> Vec<(EntityId, AnyView, DiffPlacement, Pixels)> {
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
                let state = views.state(view.view().entity_id(), cx);
                let resized = match view.placement() {
                    DiffPlacement::Block => std::mem::replace(&mut state.width, column) != column,
                    DiffPlacement::EndOfLine => false,
                };
                if state.measured && !resized && state.row_height == row_height && !restyled {
                    return None;
                }
                state.row_height = row_height;
                Some((
                    view.view().entity_id(),
                    view.view().clone(),
                    view.placement(),
                    column,
                ))
            })
            .collect()
    }

    /// Takes measured sizes: a block view's height as whole rows, a line
    /// end view's width as the one it draws at. A block view whose rows
    /// changed moves the lines after it, so the rows are projected again.
    fn set_hosted_view_sizes(&mut self, sizes: Vec<(EntityId, DiffPlacement, Size<Pixels>)>) {
        let Some(decor) = self.hosted_decor.as_mut() else {
            return;
        };
        let row_height = f32::from(decor.row_height).max(1.0);
        for (id, placement, size) in sizes {
            let Some(state) = decor.views.states.get_mut(&id) else {
                continue;
            };
            state.measured = true;
            match placement {
                DiffPlacement::Block => {
                    let rows = ((f32::from(size.height) / row_height).ceil() as usize).max(1);
                    if state.rows != rows {
                        state.rows = rows;
                        decor.key = None;
                    }
                }
                DiffPlacement::EndOfLine => state.width = size.width,
            }
        }
    }

    /// Measures the view again before it is next drawn: it notified.
    pub(in crate::view) fn forget_hosted_view_height(&mut self, id: EntityId) {
        if let Some(state) = self
            .hosted_decor
            .as_mut()
            .and_then(|decor| decor.views.states.get_mut(&id))
        {
            state.measured = false;
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
        let scroll = match side {
            Some(DiffLineSide::New) => &self.diff_split_right_scroll,
            _ => &self.diff_scroll,
        };
        let scroll_x = scroll.0.borrow().base_handle.offset().x;
        // Drawn at the width it was measured at, this frame's column's.
        let state = self
            .hosted_decor
            .as_mut()
            .expect("hosted renderer")
            .views
            .state(id, cx);
        let (width, focus) = (state.width, state.focus.clone());
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
        let Some(wrap) = self.diff_text_wrap_for_visible_ix(visible_ix) else {
            return true;
        };
        let holds_text = |wrap: rows::DiffTextWrapSlice| {
            wrap.wrap_ix == 0 || !wrap.range_for_region(region).is_empty()
        };
        let source = self.diff_source_visible_ix_for_visible_ix(visible_ix);
        holds_text(wrap)
            && !(visible_ix + 1..self.diff_visible_len())
                .take_while(|next| self.diff_source_visible_ix_for_visible_ix(*next) == source)
                .filter_map(|next| self.diff_text_wrap_for_visible_ix(next))
                .any(holds_text)
    }

    /// The views after the text of document row `row`, in `side`'s column or
    /// in an inline diff's one: those anchored to its line on that side, side
    /// by side in order, one row high, centred on it and at their measured
    /// widths, on the row its text ends on. They go where the row's canvas finds the text
    /// to end, so they scroll sideways with it.
    pub(super) fn render_hosted_line_end(
        &mut self,
        row: usize,
        side: Option<DiffLineSide>,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let region = hosted_region(side);
        let (old, new) = self.hosted_line_numbers(row);
        let decor = self.hosted_decor.as_ref()?;
        let ends: Vec<_> = decor
            .views
            .views()
            .iter()
            .filter(|view| match view.placement() {
                DiffPlacement::Block => false,
                DiffPlacement::EndOfLine => {
                    let anchor = view.anchor();
                    let line = match anchor.side {
                        DiffLineSide::Old => old,
                        DiffLineSide::New => new,
                    };
                    side.is_none_or(|side| side == anchor.side) && line == Some(anchor.line)
                }
            })
            .map(|view| (view.view().entity_id(), view.view().clone()))
            .collect();
        if ends.is_empty() || !self.is_hosted_line_end_row(row, region) {
            return None;
        }
        let pane_id = decor.view_id;
        let row_height = decor.row_height;
        // The pane measures its views before its rows draw.
        let text = decor.views.text.clone().expect("measured before drawn");
        let views = &mut self.hosted_decor.as_mut().expect("hosted renderer").views;
        let mut width = px(0.0);
        let frames: Vec<_> = ends
            .into_iter()
            .map(|(id, content)| {
                let state = views.state(id, cx);
                width += state.width;
                keeps_its_input(
                    hosted_view_frame(content, Some(state.width), &text)
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
                    &state.focus,
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
