//! Extension views between diff lines. The list keeps its uniform rows: a
//! view reserves as many as its measured height needs and is drawn once,
//! from the first of them on screen, over the rest. A view is measured when
//! it first shows and again only when it notifies, its column's width
//! changes, or the text style or the row height does.
use super::*;
use gitcomet_extension_api::{DiffInlineView, DiffLineSide};
use gpui::{Div, EntityId, TextStyle};

/// What the pane knows of one view.
struct InlineViewState {
    /// Rows from its last measured height; one until it is measured.
    rows: usize,
    /// Whether `rows` still holds: not after it notified.
    measured: bool,
    /// The column width it was measured at.
    width: Pixels,
    /// The row height its rows were counted in, which follows the UI scale
    /// and the editor's metrics.
    row_height: Pixels,
    /// Its wrapper's: the diff's keys stand aside while focus is inside.
    focus: gpui::FocusHandle,
}

/// A hosted diff's views between lines, by their entities.
#[derive(Default)]
pub(in crate::view) struct InlineViews {
    views: Arc<[DiffInlineView]>,
    states: FxHashMap<EntityId, InlineViewState>,
    /// The text style they were measured in, and are drawn in rather than
    /// the diff's editor font.
    text: Option<TextStyle>,
    /// The views as last projected, so a row of one can be found again in a
    /// replacement list.
    projected: Vec<EntityId>,
}

impl InlineViews {
    pub(super) fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    pub(super) fn views(&self) -> &[DiffInlineView] {
        &self.views
    }

    /// Replaces the views, keeping what is known of those that stay.
    /// Whether the list is a new one.
    pub(super) fn set(&mut self, views: Arc<[DiffInlineView]>) -> bool {
        if Arc::ptr_eq(&self.views, &views) {
            return false;
        }
        self.states
            .retain(|id, _| views.iter().any(|view| view.view().entity_id() == *id));
        self.views = views;
        true
    }

    /// The rows `view` takes.
    pub(super) fn rows(&self, view: &DiffInlineView) -> usize {
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

    fn state(&mut self, id: EntityId, cx: &mut App) -> &mut InlineViewState {
        self.states.entry(id).or_insert_with(|| InlineViewState {
            rows: 1,
            measured: false,
            width: px(0.0),
            row_height: px(0.0),
            focus: cx.focus_handle(),
        })
    }
}

/// A view laid out at its column's width in the text style it is measured
/// in, the same for measuring it and for drawing it.
fn inline_view_frame(content: AnyView, width: Pixels, text: &TextStyle) -> Div {
    let mut frame = div().w(width);
    *frame.text_style() = text.subtract(&gpui::TextStyleRefinement::default());
    frame.child(content)
}

/// Measures the views that need it, as gpui's `list` measures its items, in
/// the text style the pane's rows are drawn under. The renderer's rows follow
/// in the same frame.
pub(in crate::view) fn measure_inline_views(
    renderer: &gpui::Entity<DiffRenderer>,
    window: &mut Window,
    cx: &mut App,
) {
    let text = window.text_style();
    let views = renderer.update(cx, |pane, cx| {
        pane.inline_views_to_measure(&text, window, cx)
    });
    if views.is_empty() {
        return;
    }
    let heights = views
        .into_iter()
        .map(|(id, view, width)| {
            let mut frame = inline_view_frame(view, width, &text).into_any_element();
            let size = frame.layout_as_root(
                gpui::size(
                    gpui::AvailableSpace::Definite(width),
                    gpui::AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            (id, size.height)
        })
        .collect();
    renderer.update(cx, |pane, _| pane.set_inline_view_heights(heights));
}

impl DiffRenderer {
    /// How wide a view on `side` draws: its column, less the scrollbar.
    fn hosted_view_width(&self, side: DiffLineSide, window: &Window) -> Pixels {
        let width = self
            .hosted_content_width
            .unwrap_or(window.viewport_size().width);
        self.hosted_card_widths(width)[usize::from(side == DiffLineSide::New)]
    }

    /// The views to measure, each with the width it draws at: those not
    /// measured since they showed or notified, those whose column changed
    /// width, and every one when the text style or the row height changed.
    fn inline_views_to_measure(
        &mut self,
        text: &TextStyle,
        window: &Window,
        cx: &mut App,
    ) -> Vec<(EntityId, AnyView, Pixels)> {
        let Some(decor) = &self.hosted_decor else {
            return Vec::new();
        };
        let widths: Vec<_> = decor
            .views
            .views()
            .iter()
            .map(|view| self.hosted_view_width(view.anchor().side, window))
            .collect();
        let decor = self.hosted_decor.as_mut().expect("hosted renderer");
        let row_height = decor.row_height;
        let views = &mut decor.views;
        let restyled = views.text.as_ref() != Some(text);
        if restyled {
            views.text = Some(text.clone());
        }
        let list = views.views.clone();
        list.iter()
            .zip(widths)
            .filter_map(|(view, width)| {
                let state = views.state(view.view().entity_id(), cx);
                if state.measured
                    && state.width == width
                    && state.row_height == row_height
                    && !restyled
                {
                    return None;
                }
                state.width = width;
                state.row_height = row_height;
                Some((view.view().entity_id(), view.view().clone(), width))
            })
            .collect()
    }

    /// Takes measured heights as whole rows. A view whose rows changed moves
    /// the lines after it, so the rows are projected again.
    fn set_inline_view_heights(&mut self, heights: Vec<(EntityId, Pixels)>) {
        let Some(decor) = self.hosted_decor.as_mut() else {
            return;
        };
        let row_height = f32::from(decor.row_height).max(1.0);
        for (id, height) in heights {
            let Some(state) = decor.views.states.get_mut(&id) else {
                continue;
            };
            let rows = ((f32::from(height) / row_height).ceil() as usize).max(1);
            state.measured = true;
            if state.rows != rows {
                state.rows = rows;
                decor.key = None;
            }
        }
    }

    /// Measures the view again before it is next drawn: it notified.
    pub(in crate::view) fn forget_inline_view_height(&mut self, id: EntityId) {
        if let Some(state) = self
            .hosted_decor
            .as_mut()
            .and_then(|decor| decor.views.states.get_mut(&id))
        {
            state.measured = false;
        }
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
        window: &mut Window,
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
        let width = self.hosted_view_width(view.anchor().side, window);
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
            .child(
                inline_view_frame(content, width, &text)
                    .id("hosted_view")
                    .debug_selector(move || format!("hosted_diff_{pane_id}_view_{}", id.as_u64()))
                    .absolute()
                    .top(-row_height * row as f32)
                    .left(-scroll_x)
                    .h(row_height * rows as f32)
                    .track_focus(&focus)
                    // Its presses are its own: they neither focus the diff
                    // nor start a selection in it.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation()),
            )
            .into_any_element()
    }
}
