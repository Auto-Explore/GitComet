//! Selectable, wrapping text for the flowing markdown preview.
//!
//! The diff preview paints one visual row at a time, so a single shaped line is
//! enough to place the selection highlight and to turn a click into an offset.
//! The flowing preview lets one row wrap over several visual lines, so both
//! operations become two-dimensional. Both read the same `TextLayout` the text
//! was painted with, which the row's hitbox keeps so hit testing sees exactly
//! the geometry the user clicked on.

use super::*;

/// One markdown row's text, painted with wrapping and wired to the shared
/// selection machinery.
pub(in crate::view) struct MarkdownFlowText {
    view: Entity<MainPaneView>,
    row_ix: usize,
    region: DiffTextRegion,
    /// The row's raw text when it differs from what is painted, i.e. when tabs
    /// were expanded. Selection and copy address this text.
    untabbed: Option<SharedString>,
    /// Text as painted.
    text: SharedString,
    /// Styled-run backgrounds painted separately so selection can sit above
    /// them without also washing over the glyphs.
    run_backgrounds: Arc<[(Range<usize>, gpui::Hsla)]>,
    highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
    inner: Option<gpui::StyledText>,
    layout: Option<gpui::TextLayout>,
}

/// Paint layers for flowing Markdown text, in their visual stacking order.
///
/// Run backgrounds include inline-code and quick-search washes. The selection
/// must sit above those backgrounds so neither can hide it, while the glyphs
/// stay above both layers and remain crisp.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum MarkdownFlowPaintPhase {
    RunBackgrounds,
    Selection,
    Glyphs,
}

impl MarkdownFlowText {
    pub(in crate::view) fn new(
        view: Entity<MainPaneView>,
        row_ix: usize,
        region: DiffTextRegion,
        row_text: SharedString,
        text: SharedString,
        highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
    ) -> Self {
        let (highlights, run_backgrounds) = split_markdown_flow_highlight_layers(highlights);
        Self {
            view,
            row_ix,
            region,
            untabbed: (row_text != text).then_some(row_text),
            text,
            run_backgrounds,
            highlights,
            inner: None,
            layout: None,
        }
    }

    /// Paint the selection behind the glyphs, one quad per visual line.
    fn paint_selection(&self, layout: &gpui::TextLayout, window: &mut Window, cx: &mut App) {
        let Some(selected) = self
            .view
            .read(cx)
            .diff_text_local_selection_range(self.row_ix, self.region)
        else {
            return;
        };
        let start = self.painted_offset(selected.start);
        let end = self.painted_offset(selected.end);
        if end <= start {
            return;
        }

        let color = self.view.read(cx).diff_text_selection_color();
        let rects = markdown_flow_range_rects(layout, start, end);
        if rects.is_empty() {
            return;
        }
        record_selection_paint_for_tests(self.row_ix, &rects);
        #[cfg(test)]
        record_markdown_flow_paint_phase_for_tests(self.row_ix, MarkdownFlowPaintPhase::Selection);
        for rect in rects {
            window.paint_quad(fill(rect, color));
        }
    }

    /// Paint styled-run backgrounds below the selection layer.
    fn paint_run_backgrounds(&self, layout: &gpui::TextLayout, window: &mut Window) {
        #[cfg(test)]
        record_markdown_flow_paint_phase_for_tests(
            self.row_ix,
            MarkdownFlowPaintPhase::RunBackgrounds,
        );
        for (range, color) in self.run_backgrounds.iter() {
            for rect in markdown_flow_range_rects(layout, range.start, range.end) {
                window.paint_quad(fill(rect, *color));
            }
        }
    }

    /// Offset in the painted text for an offset in row coordinates.
    fn painted_offset(&self, row_offset: usize) -> usize {
        match &self.untabbed {
            Some(raw) => markdown_flow_painted_offset(raw, row_offset),
            None => row_offset.min(self.text.len()),
        }
    }
}

/// Pull background colours out of GPUI highlights so they can be composited
/// independently of their glyph styles.
fn split_markdown_flow_highlight_layers(
    highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
) -> (
    Arc<[(Range<usize>, gpui::HighlightStyle)]>,
    Arc<[(Range<usize>, gpui::Hsla)]>,
) {
    if !highlights
        .iter()
        .any(|(_, style)| style.background_color.is_some())
    {
        return (highlights, Arc::from(Vec::new()));
    }

    let mut foregrounds = Vec::with_capacity(highlights.len());
    let mut backgrounds = Vec::new();
    for (range, mut style) in highlights.iter().cloned() {
        if let Some(background) = style.background_color.take() {
            backgrounds.push((range.clone(), background));
        }
        // Keep even a now-default style: the original background run formed a
        // shaping boundary, and preserving it keeps ligatures and byte-to-x
        // geometry identical to the background GPUI would have painted.
        foregrounds.push((range, style));
    }
    (Arc::from(foregrounds), Arc::from(backgrounds))
}

/// The rectangles a byte range covers, in window coordinates.
///
/// A wrapped row's selection is not one box: each visual line contributes the
/// slice of the range that falls inside it, measured against the unwrapped
/// layout the wrap boundaries index into.
fn markdown_flow_range_rects(
    layout: &gpui::TextLayout,
    start: usize,
    end: usize,
) -> Vec<Bounds<Pixels>> {
    let Some(line) = layout.line_layout_for_index(start) else {
        return Vec::new();
    };
    let bounds = layout.bounds();
    let line_height = layout.line_height();

    let mut edges = Vec::with_capacity(line.wrap_boundaries().len() + 2);
    edges.push(0usize);
    for boundary in line.wrap_boundaries() {
        let Some(glyph) = line
            .unwrapped_layout
            .runs
            .get(boundary.run_ix)
            .and_then(|run| run.glyphs.get(boundary.glyph_ix))
        else {
            // Every later line's position is counted from the boundaries before
            // it, so skipping one would paint the rest of the highlight a line
            // too high. Painting nothing is the honest failure.
            return Vec::new();
        };
        edges.push(glyph.index);
    }
    edges.push(line.unwrapped_layout.len);

    let mut rects = Vec::new();
    for (visual_ix, edge) in edges.windows(2).enumerate() {
        let (line_start, line_end) = (edge[0], edge[1]);
        let from = start.max(line_start);
        let to = end.min(line_end);
        if from >= to {
            continue;
        }
        // Wrapped lines are painted flush left, so every x is relative to where
        // that line starts inside the unwrapped layout.
        let line_origin_x = line.unwrapped_layout.x_for_index(line_start);
        let x0 = line.unwrapped_layout.x_for_index(from) - line_origin_x;
        let x1 = line.unwrapped_layout.x_for_index(to) - line_origin_x;
        let top = bounds.top() + line_height * (visual_ix as f32);
        rects.push(Bounds::from_corners(
            point(bounds.left() + x0, top),
            point(bounds.left() + x1, top + line_height),
        ));
    }
    rects
}

/// Offset in tab-expanded text for an offset in the raw text.
pub(in crate::view) fn markdown_flow_painted_offset(raw: &str, row_offset: usize) -> usize {
    let row_offset = row_offset.min(raw.len());
    let tabs = raw.as_bytes()[..row_offset]
        .iter()
        .filter(|byte| **byte == b'\t')
        .count();
    row_offset + tabs * (MARKDOWN_FLOW_TAB_COLUMNS - 1)
}

/// Offset in the raw text for an offset in the tab-expanded text.
pub(in crate::view) fn markdown_flow_row_offset(raw: &str, painted_offset: usize) -> usize {
    let mut painted = 0usize;
    for (ix, byte) in raw.bytes().enumerate() {
        let width = if byte == b'\t' {
            MARKDOWN_FLOW_TAB_COLUMNS
        } else {
            1
        };
        // A column that falls within this character's own columns belongs to
        // it rather than to the next one, so a click inside an expanded tab
        // resolves to the tab and a selection started there keeps the indent.
        if painted + width > painted_offset {
            return ix;
        }
        painted += width;
    }
    raw.len()
}

/// Tabs are painted as this many spaces; `maybe_expand_tabs` is the producer.
const MARKDOWN_FLOW_TAB_COLUMNS: usize = 4;

/// How far past the window a row still counts as reachable, so a drag that
/// runs off the edge and the rows a flick is about to bring in keep their
/// hitboxes.
const MARKDOWN_FLOW_HITBOX_MARGIN: f32 = 2.0;

/// Whether a row is close enough to the window to be worth hit testing.
fn markdown_flow_row_is_near_viewport(bounds: Bounds<Pixels>, window: &Window) -> bool {
    let height = window.viewport_size().height;
    let margin = height * MARKDOWN_FLOW_HITBOX_MARGIN;
    bounds.bottom() >= -margin && bounds.top() <= height + margin
}

// Selection quads painted this frame, keyed by row. The highlight is a
// paint-time computation with no other observable effect, so this is the only
// way a test can see where it landed.
#[cfg(test)]
thread_local! {
    static SELECTION_PAINT_LOG: RefCell<Vec<(usize, Bounds<Pixels>)>> =
        const { RefCell::new(Vec::new()) };
    // `None` keeps ordinary tests from accumulating every Markdown text paint.
    // A regression test opts into one frame by installing an empty log.
    static MARKDOWN_FLOW_PAINT_PHASE_LOG:
        RefCell<Option<Vec<(usize, MarkdownFlowPaintPhase)>>> = const { RefCell::new(None) };
}

#[cfg(test)]
fn record_selection_paint_for_tests(row_ix: usize, rects: &[Bounds<Pixels>]) {
    SELECTION_PAINT_LOG.with(|log| {
        let mut log = log.borrow_mut();
        log.extend(rects.iter().map(|rect| (row_ix, *rect)));
    });
}

#[cfg(not(test))]
fn record_selection_paint_for_tests(_row_ix: usize, _rects: &[Bounds<Pixels>]) {}

#[cfg(test)]
fn record_markdown_flow_paint_phase_for_tests(row_ix: usize, phase: MarkdownFlowPaintPhase) {
    MARKDOWN_FLOW_PAINT_PHASE_LOG.with(|log| {
        if let Some(log) = log.borrow_mut().as_mut() {
            log.push((row_ix, phase));
        }
    });
}

#[cfg(test)]
pub(in crate::view) fn begin_markdown_flow_paint_phase_capture_for_tests() {
    MARKDOWN_FLOW_PAINT_PHASE_LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
}

#[cfg(test)]
pub(in crate::view) fn markdown_flow_paint_phases_for_tests(
    row_ix: usize,
) -> Vec<MarkdownFlowPaintPhase> {
    MARKDOWN_FLOW_PAINT_PHASE_LOG.with(|log| {
        log.borrow_mut()
            .take()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(logged_ix, phase)| (logged_ix == row_ix).then_some(phase))
            .collect()
    })
}

#[cfg(test)]
pub(in crate::view) fn clear_markdown_selection_paint_log_for_tests() {
    SELECTION_PAINT_LOG.with(|log| log.borrow_mut().clear());
}

/// Selection quads painted since the last clear, in paint order.
#[cfg(test)]
pub(in crate::view) fn markdown_selection_paint_log_for_tests(
    row_ix: usize,
) -> Vec<Bounds<Pixels>> {
    // Draining on read is what keeps the log safe to use without knowing it
    // exists: gpui tests share worker threads, so anything left behind would
    // surface in whichever test ran next on this one.
    SELECTION_PAINT_LOG.with(|log| {
        let mut log = log.borrow_mut();
        let (mine, rest) = std::mem::take(&mut *log)
            .into_iter()
            .partition(|(logged_ix, _)| *logged_ix == row_ix);
        *log = rest;
        mine.into_iter().map(|(_, rect)| rect).collect()
    })
}

impl gpui::IntoElement for MarkdownFlowText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for MarkdownFlowText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut inner = gpui::StyledText::new(self.text.clone())
            .with_default_highlights(&window.text_style(), self.highlights.iter().cloned());
        self.layout = Some(inner.layout().clone());
        let layout = inner.request_layout(id, inspector_id, window, cx);
        self.inner = Some(inner);
        layout
    }

    fn prepaint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .as_mut()
            .expect("markdown flow text should be laid out before prepaint")
            .prepaint(id, inspector_id, bounds, request_layout, window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // A row outside the window cannot be clicked and its quads would be
        // clipped, so neither its selection geometry nor its hitbox is worth
        // computing. The virtualized list this renderer replaced built nothing
        // for such a row at all.
        let on_screen = markdown_flow_row_is_near_viewport(bounds, window);
        let layout = self
            .layout
            .clone()
            .expect("markdown flow text should be laid out before paint");

        // Background colours were removed from `inner` when the element was
        // built. Painting them here gives us the intended stacking order:
        // code/search background -> selection wash -> crisp text.
        self.paint_run_backgrounds(&layout, window);
        if on_screen {
            self.paint_selection(&layout, window, cx);
        }
        #[cfg(test)]
        record_markdown_flow_paint_phase_for_tests(self.row_ix, MarkdownFlowPaintPhase::Glyphs);
        self.inner
            .as_mut()
            .expect("markdown flow text should be laid out before paint")
            .paint(
                id,
                inspector_id,
                bounds,
                request_layout,
                prepaint,
                window,
                cx,
            );

        if !on_screen {
            return;
        }

        let row_ix = self.row_ix;
        let region = self.region;
        let untabbed = self.untabbed.clone();
        let text_len = untabbed
            .as_ref()
            .map_or_else(|| self.text.len(), |raw| raw.len());
        self.view.clone().update(cx, |this, _cx| {
            let (source_visible_ix, visual_range) =
                this.diff_text_visual_source_range_for_region(row_ix, region);
            this.set_diff_text_hitbox(
                row_ix,
                region,
                DiffTextHitbox {
                    bounds,
                    layout_key: 0,
                    source_visible_ix,
                    text_start_offset: visual_range.start,
                    text_len,
                    offset_map: None,
                    painted_text: self.text.clone(),
                    streamed_ascii_monospace_cell_width: None,
                    wrapped: Some(DiffTextWrappedHit { layout, untabbed }),
                },
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_colors_are_removed_from_the_glyph_highlights() {
        let code_background = gpui::hsla(0.10, 0.50, 0.25, 0.75);
        let search_background = gpui::hsla(0.55, 0.60, 0.40, 0.45);
        let search_foreground = gpui::hsla(0.55, 0.80, 0.80, 1.0);
        let highlights = Arc::from(vec![
            (
                1..5,
                gpui::HighlightStyle {
                    background_color: Some(code_background),
                    ..gpui::HighlightStyle::default()
                },
            ),
            (
                7..12,
                gpui::HighlightStyle {
                    color: Some(search_foreground),
                    background_color: Some(search_background),
                    ..gpui::HighlightStyle::default()
                },
            ),
        ]);

        let (foregrounds, backgrounds) = split_markdown_flow_highlight_layers(highlights);

        assert_eq!(
            backgrounds.as_ref(),
            &[(1..5, code_background), (7..12, search_background)]
        );
        assert_eq!(foregrounds.len(), 2);
        assert_eq!(foregrounds[0].0, 1..5);
        assert_eq!(foregrounds[0].1, gpui::HighlightStyle::default());
        assert_eq!(foregrounds[1].0, 7..12);
        assert_eq!(foregrounds[1].1.color, Some(search_foreground));
        assert_eq!(foregrounds[1].1.background_color, None);
    }

    #[test]
    fn tab_offsets_round_trip_between_row_and_painted_text() {
        let raw = "\tlet x = 1;";

        assert_eq!(markdown_flow_painted_offset(raw, 0), 0);
        // The tab itself paints as four columns, so everything after it shifts.
        assert_eq!(markdown_flow_painted_offset(raw, 1), 4);
        assert_eq!(markdown_flow_painted_offset(raw, 5), 8);
        assert_eq!(markdown_flow_painted_offset(raw, raw.len()), raw.len() + 3);

        assert_eq!(markdown_flow_row_offset(raw, 0), 0);
        // Any column inside the expanded tab resolves to the tab itself.
        assert_eq!(markdown_flow_row_offset(raw, 2), 0);
        assert_eq!(markdown_flow_row_offset(raw, 4), 1);
        assert_eq!(markdown_flow_row_offset(raw, 8), 5);
        assert_eq!(markdown_flow_row_offset(raw, 1_000), raw.len());
    }

    #[test]
    fn text_without_tabs_maps_offsets_unchanged() {
        let raw = "plain text";
        for offset in 0..=raw.len() {
            assert_eq!(markdown_flow_painted_offset(raw, offset), offset);
            assert_eq!(markdown_flow_row_offset(raw, offset), offset);
        }
    }
}
