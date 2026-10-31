//! Extension views in a hosted diff. The list keeps its uniform rows: a
//! block view reserves as many as its measured height needs and is drawn
//! once, from the first of them on screen, over the rest. A view at a line's
//! end takes no rows: its line's row draws it where its canvas finds the
//! text to end. A view over the text takes none either: a layer over its
//! column's rows places it beside its line. A view is measured when it first
//! shows and again only when it notifies, a block or over view's column
//! changes width, a start-aligned over view's text start moves, or the text
//! style or the row height does.
use super::*;
use crate::view::rows::horizontal_column;
use gitcomet_extension_api::{DiffAlign, DiffHostedView, DiffLineSide, DiffPlacement};
use gpui::{Div, EntityId, TextStyle};
use smallvec::SmallVec;
use std::cell::Cell;
use std::rc::Rc;

/// What the pane knows of one view.
struct HostedViewState {
    /// What it was last measured against, and the size it measured at,
    /// which it draws at; none until it is measured.
    measured: Option<(Measure, Size<Pixels>)>,
    /// It notified since it was measured, so it is measured again before it
    /// is next drawn; its rows until then are the ones it measured at.
    notified: bool,
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
    /// At its own width, up to `max`: lined up with the start, the room from
    /// the text start to the column's end; with the end, the column's width.
    Over { max: Pixels, row_height: Pixels },
}

impl Measure {
    /// What a view placed so is measured against, in a column `column` wide
    /// whose text starts at `start`.
    fn of(placement: DiffPlacement, column: Pixels, start: Pixels, row_height: Pixels) -> Self {
        match placement {
            DiffPlacement::Block => Self::Block { column, row_height },
            DiffPlacement::EndOfLine => Self::EndOfLine { row_height },
            DiffPlacement::Over { align } => Self::Over {
                max: match align {
                    DiffAlign::Start => (column - start).max(px(0.0)),
                    DiffAlign::End => column,
                },
                row_height,
            },
        }
    }

    /// The view laid out to be measured, with the width it has for it.
    fn frame(self, content: AnyView, text: &TextStyle) -> (Div, gpui::AvailableSpace) {
        match self {
            Self::Block { column, .. } => (
                hosted_view_frame(content, Some(column), text),
                gpui::AvailableSpace::Definite(column),
            ),
            Self::EndOfLine { .. } => (
                hosted_view_frame(content, None, text),
                gpui::AvailableSpace::MaxContent,
            ),
            Self::Over { max, .. } => (
                hosted_view_frame(content, None, text).max_w(max),
                gpui::AvailableSpace::MaxContent,
            ),
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
        Some((Measure::EndOfLine { .. } | Measure::Over { .. }, _)) | None => 1,
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
    /// The views over the text, in list order, with the edge each lines up
    /// with.
    over_views: Vec<(usize, DiffAlign)>,
    /// The rows built this frame with views after their text, by row and
    /// column.
    line_ends: FxHashMap<(usize, DiffTextRegion), Rc<HostedLineEnd>>,
    /// How wide the text of each row built with views after it is, by the
    /// row's lines. Its column's scroll range holds each with its views as
    /// they now measure, so a view that goes or shrinks gives its room back.
    line_end_text: FxHashMap<LineEndLines, Pixels>,
}

/// The lines of a row with views after its text, in its column: the views
/// anchored to them are the ones it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::view) struct LineEndLines {
    region: DiffTextRegion,
    old: Option<u32>,
    new: Option<u32>,
}

/// Where a row's text ends, which its canvas works out as it prepaints; the
/// views after the text are then placed there.
pub(in crate::view) struct HostedLineEnd {
    /// The lines of the row whose text it follows.
    pub(in crate::view) lines: LineEndLines,
    /// How wide the views are together.
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
        self.over_views.clear();
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
                DiffPlacement::Over { align } => self.over_views.push((ix, align)),
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
            notified: false,
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

    /// The views after the text of the row on `lines`, by their place in the
    /// list.
    fn line_end_views_after(&self, lines: LineEndLines) -> impl Iterator<Item = usize> + '_ {
        [
            (DiffLineSide::Old, lines.old),
            (DiffLineSide::New, lines.new),
        ]
        .into_iter()
        .filter_map(|(side, line)| self.line_end_views.get(&(side, line?)))
        .flatten()
        .copied()
    }

    /// How wide the views after the text of the row on `lines` now are
    /// together.
    fn line_end_width(&self, lines: LineEndLines) -> Pixels {
        self.line_end_views_after(lines)
            .map(|ix| self.width(self.views[ix].view().entity_id()))
            .fold(px(0.0), |total, width| total + width)
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
    let views = renderer.update(cx, |pane, cx| pane.hosted_views_to_measure(&text, cx));
    if views.is_empty() {
        return;
    }
    let sizes = views
        .into_iter()
        .map(|(id, view, measure)| {
            let (frame, width) = measure.frame(view, &text);
            let size = frame.into_any_element().layout_as_root(
                gpui::size(width, gpui::AvailableSpace::MinContent),
                window,
                cx,
            );
            (id, measure, size)
        })
        .collect();
    renderer.update(cx, |pane, _| pane.set_hosted_view_sizes(sizes));
}

impl DiffRenderer {
    /// Notes the row height the hosted rows are placed, counted and drawn
    /// in, which follows the UI scale and the editor's metrics: the pitch the
    /// list laid its rows out at when it last asked for this height at this
    /// scale; otherwise the height as gpui will snap it, and the next frame
    /// takes the pitch the list then lays them out at.
    pub(in crate::view) fn sync_hosted_row_height(&mut self, window: &mut Window, cx: &mut App) {
        let asked = self
            .theme
            .editor_row_height(ui_scale::UiScale::current(cx).percent());
        let scale_factor = window.scale_factor();
        let built = self.diff_list_built;
        let current = built.row_height == asked && built.scale_factor == scale_factor;
        let pitch = list_row_pitch(&self.diff_scroll, built.rows).filter(|_| current);
        if built.rows > 0 && !current {
            // The views are cached: nothing else draws the frame that would.
            window.on_next_frame(|window, _| window.refresh());
        }
        let row_height = pitch.unwrap_or_else(|| device_snapped(asked, scale_factor));
        if let Some(decor) = self.hosted_decor.as_mut() {
            decor.row_height = row_height;
        }
    }

    /// The views to measure, each with what it is measured against: those
    /// whose size does not hold for their placement as it now is, which is
    /// those not measured since they showed or notified, block and over views
    /// whose column changed width, start-aligned over views whose text start
    /// moved, and every one when the row height did, or when the text style
    /// changed.
    fn hosted_views_to_measure(
        &mut self,
        text: &TextStyle,
        cx: &mut App,
    ) -> Vec<(EntityId, AnyView, Measure)> {
        let Some(decor) = &self.hosted_decor else {
            return Vec::new();
        };
        if decor.views.is_empty() {
            return Vec::new();
        }
        // How wide views draw in each column, old side then new.
        let widths = self.diff_columns().row_widths();
        let starts = [DiffLineSide::Old, DiffLineSide::New].map(|side| {
            let region = match self.diff_view {
                DiffViewMode::Inline => DiffTextRegion::Inline,
                DiffViewMode::Split => hosted_region(Some(side)),
            };
            self.diff_text_start(region, LineNumberCells::of(region), cx)
        });
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
                let side = usize::from(view.anchor().side == DiffLineSide::New);
                let measure = Measure::of(view.placement(), widths[side], starts[side], row_height);
                let state = views.state(view.view().entity_id(), cx);
                if !restyled
                    && !state.notified
                    && state.measured.is_some_and(|(at, _)| at == measure)
                {
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
            state.notified = false;
            if measured_rows(state.measured) != rows {
                decor.key = None;
            }
        }
    }

    /// Measures the view again before it is next drawn: it notified.
    pub(in crate::view) fn hosted_view_notified(&mut self, id: EntityId) {
        if let Some(state) = self
            .hosted_decor
            .as_mut()
            .and_then(|decor| decor.views.states.get_mut(&id))
        {
            state.notified = true;
        }
    }

    /// The row height the pane places its rows at.
    #[cfg(test)]
    pub(in crate::view) fn hosted_row_height(&self) -> Pixels {
        self.hosted_decor
            .as_ref()
            .map_or(px(0.0), |decor| decor.row_height)
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
        let scroll_x = self.column_scroll(side).0.borrow().base_handle.offset().x;
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

    /// Notes how wide the text of the row on `lines` is, its gutter and
    /// padding included, which its column's scroll range holds with the views
    /// after it. The rows are laid out again only when that range changes.
    pub(in crate::view) fn record_hosted_line_end_text(
        &mut self,
        lines: LineEndLines,
        width: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_word_wrap
            || self
                .hosted_decor
                .as_ref()
                .is_none_or(|decor| decor.views.line_end_text.get(&lines) == Some(&width))
        {
            return;
        }
        let column = horizontal_column(lines.region);
        let range = self.diff_horizontal_content_width_for_column(column);
        if let Some(decor) = self.hosted_decor.as_mut() {
            decor.views.line_end_text.insert(lines, width);
        }
        if self.diff_horizontal_content_width_for_column(column) != range {
            cx.notify();
        }
    }

    /// The widest of `column`'s rows with views after their text, the views
    /// as they now measure; zero without any.
    pub(in crate::view) fn hosted_line_end_rows_width(
        &self,
        column: DiffHorizontalScrollColumn,
    ) -> Pixels {
        let Some(decor) = &self.hosted_decor else {
            return px(0.0);
        };
        decor
            .views
            .line_end_text
            .iter()
            .filter(|(lines, _)| horizontal_column(lines.region) == column)
            .map(|(lines, text)| (*text + decor.views.line_end_width(*lines)).round())
            .fold(px(0.0), Pixels::max)
    }

    /// Forgets how wide the rows with views after their text are, with the
    /// rest of the horizontal scroll range.
    pub(in crate::view) fn forget_hosted_line_end_text(&mut self) {
        if let Some(decor) = self.hosted_decor.as_mut() {
            decor.views.line_end_text.clear();
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
        if decor.views.line_end_views.is_empty() {
            return None;
        }
        let shown = |at: DiffLineSide, line: Option<u32>| {
            line.filter(|_| side.is_none_or(|side| side == at))
        };
        let lines = LineEndLines {
            region: hosted_region(side),
            old: shown(DiffLineSide::Old, old),
            new: shown(DiffLineSide::New, new),
        };
        let mut ends: SmallVec<[usize; 2]> = decor.views.line_end_views_after(lines).collect();
        let region = lines.region;
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
        let width = views.line_end_width(lines);
        let frames: Vec<_> = ends
            .into_iter()
            .map(|(id, content, view_width)| {
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
            lines,
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

impl DiffRenderer {
    /// The scroll handle of `side`'s column, or of an inline diff's.
    fn column_scroll(&self, side: Option<DiffLineSide>) -> &UniformListScrollHandle {
        match side {
            Some(DiffLineSide::New) => &self.diff_split_right_scroll,
            _ => &self.diff_scroll,
        }
    }

    /// The views over the text of `side`'s column, or of an inline diff's:
    /// a layer over its rows that clips them, `right` and `bottom` short of
    /// its edges for its scrollbars. Each is at the size it was measured at,
    /// below its line or above it, lined up with its column's text start or
    /// end; the layer places them as it prepaints, from where the rows have
    /// then scrolled, and builds only those it shows.
    pub(in crate::view) fn render_hosted_over_layer(
        &mut self,
        side: Option<DiffLineSide>,
        right: Pixels,
        bottom: Pixels,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let decor = self.hosted_decor.as_ref()?;
        if decor.views.over_views.is_empty() {
            return None;
        }
        let views: Vec<_> = decor
            .views
            .over_views
            .iter()
            .filter_map(|(ix, align)| {
                let view = &decor.views.views()[*ix];
                let anchor = view.anchor();
                if side.is_some_and(|side| side != anchor.side) {
                    return None;
                }
                let (first, last) = decor.line_rows.get(&(anchor.side, anchor.line))?;
                let rows = (
                    *decor.document.get(*first)?,
                    *decor.document.get(*last)? + 1,
                );
                let id = view.view().entity_id();
                let state = decor.views.states.get(&id)?;
                let (_, size) = state.measured?;
                Some(OverView {
                    id,
                    content: view.view().clone(),
                    focus: state.focus.clone(),
                    rows,
                    size,
                    align: *align,
                    element: None,
                })
            })
            .collect();
        if views.is_empty() {
            return None;
        }
        let pane_id = decor.view_id;
        let row_height = decor.row_height;
        // The pane measures its views before its rows draw.
        let text = decor.views.text.clone().expect("measured before drawn");
        let region = hosted_region(side);
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right(right)
                .bottom(bottom)
                .debug_selector(move || format!("hosted_diff_{pane_id}_over_{region:?}"))
                .child(HostedOverLayer {
                    pane_id,
                    scroll: self.column_scroll(side).clone(),
                    list_rows: self.diff_list_built.rows,
                    row_height,
                    start: self.diff_text_start(region, LineNumberCells::of(region), cx),
                    text,
                    views,
                })
                .into_any_element(),
        )
    }
}

/// A view over the text, which its layer places.
struct OverView {
    id: EntityId,
    content: AnyView,
    focus: gpui::FocusHandle,
    /// Its line's first row and the row after its last, in the list.
    rows: (usize, usize),
    size: Size<Pixels>,
    align: DiffAlign,
    /// Built when it is placed in the layer's bounds this frame.
    element: Option<AnyElement>,
}

/// A column's views over its text, filling it and clipped to it. A view
/// goes below its line, or above it when it would run past the bottom and
/// fits there, held against sideways scrolling; one wholly outside is not
/// built.
struct HostedOverLayer {
    pane_id: u64,
    scroll: UniformListScrollHandle,
    /// The rows the column's list was built with this frame.
    list_rows: usize,
    /// The pitch until the list has laid out.
    row_height: Pixels,
    /// Where the column's text starts.
    start: Pixels,
    text: TextStyle,
    views: Vec<OverView>,
}

impl IntoElement for HostedOverLayer {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for HostedOverLayer {
    type RequestLayoutState = ();
    type PrepaintState = ();

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
        style.inset = gpui::Edges::all(px(0.0).into());
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
        // Where the rows are this frame: the list prepainted first.
        let origin = bounds.top() + self.scroll.0.borrow().base_handle.offset().y;
        // The pitch the list laid its rows out at this frame.
        let pitch = list_row_pitch(&self.scroll, self.list_rows).unwrap_or(self.row_height);
        let (pane_id, start) = (self.pane_id, self.start);
        let mask = gpui::ContentMask {
            bounds,
            ..Default::default()
        };
        window.with_content_mask(Some(mask), |window| {
            for view in &mut self.views {
                let line_top = origin + pitch * view.rows.0 as f32;
                let line_bottom = origin + pitch * view.rows.1 as f32;
                let above = line_top - view.size.height;
                let top =
                    if line_bottom + view.size.height > bounds.bottom() && above >= bounds.top() {
                        above
                    } else {
                        line_bottom
                    };
                view.element = None;
                if top >= bounds.bottom() || top + view.size.height <= bounds.top() {
                    continue;
                }
                let left = match view.align {
                    DiffAlign::Start => bounds.left() + start,
                    DiffAlign::End => bounds.right() - view.size.width,
                };
                let id = view.id;
                let mut element = keeps_its_input(
                    hosted_view_frame(view.content.clone(), Some(view.size.width), &self.text)
                        .id(("hosted_view", id))
                        .debug_selector(move || {
                            format!("hosted_diff_{pane_id}_view_{}", id.as_u64())
                        })
                        .h(view.size.height)
                        // The rows under it see no hover, though they still
                        // scroll under the wheel.
                        .block_mouse_except_scroll(),
                    &view.focus,
                )
                .into_any_element();
                element.layout_as_root(
                    gpui::size(
                        gpui::AvailableSpace::Definite(view.size.width),
                        gpui::AvailableSpace::Definite(view.size.height),
                    ),
                    window,
                    cx,
                );
                element.prepaint_at(gpui::point(left, top), window, cx);
                view.element = Some(element);
            }
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mask = gpui::ContentMask {
            bounds,
            ..Default::default()
        };
        window.with_content_mask(Some(mask), |window| {
            for element in self
                .views
                .iter_mut()
                .filter_map(|view| view.element.as_mut())
            {
                element.paint(window, cx);
            }
        });
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
