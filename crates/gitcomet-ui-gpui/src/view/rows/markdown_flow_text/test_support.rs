use gpui::{Bounds, Pixels, SharedString};
use std::{cell::RefCell, ops::Range};

/// Paint layers for flowing Markdown text, in their visual stacking order.
///
/// Run backgrounds include inline-code and quick-search washes. The selection
/// must sit above those backgrounds so neither can hide it, while the glyphs
/// stay above both layers and remain crisp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum MarkdownFlowPaintPhase {
    RunBackgrounds,
    Selection,
    Glyphs,
}

// Selection quads painted this frame, keyed by row. The highlight is a
// paint-time computation with no other observable effect, so this is the only
// way a test can see where it landed.
thread_local! {
    static SELECTION_PAINT_LOG: RefCell<Vec<(usize, Bounds<Pixels>)>> =
        const { RefCell::new(Vec::new()) };
    // `None` keeps ordinary tests from accumulating every Markdown text paint.
    // A regression test opts into one frame by installing an empty log.
    static MARKDOWN_FLOW_PAINT_PHASE_LOG:
        RefCell<Option<Vec<(usize, MarkdownFlowPaintPhase)>>> = const { RefCell::new(None) };
}

pub(in crate::view::rows) fn record_selection_paint_for_tests(
    row_ix: usize,
    rects: &[Bounds<Pixels>],
) {
    SELECTION_PAINT_LOG.with(|log| {
        let mut log = log.borrow_mut();
        log.extend(rects.iter().map(|rect| (row_ix, *rect)));
    });
}

/// Font family per byte range of one row's text.
type MarkdownFlowFontRuns = Vec<(Range<usize>, SharedString)>;

/// The family each stretch of a `len`-byte text is laid out in: `base`, except
/// where an override applies.
fn markdown_flow_font_runs(
    len: usize,
    base: &SharedString,
    overrides: &[(Range<usize>, SharedString)],
) -> MarkdownFlowFontRuns {
    let mut runs = Vec::new();
    let mut at = 0;
    for (range, family) in overrides {
        if at < range.start {
            runs.push((at..range.start, base.clone()));
        }
        runs.push((range.clone(), family.clone()));
        at = range.end;
    }
    if at < len {
        runs.push((at..len, base.clone()));
    }
    runs
}

thread_local! {
    // Font family per byte range of each row's text as last laid out. `None`
    // until a test opts in, so ordinary tests do not accumulate it.
    static MARKDOWN_FLOW_FONT_LOG: RefCell<
        Option<rustc_hash::FxHashMap<usize, MarkdownFlowFontRuns>>,
    > = const { RefCell::new(None) };
    // Row texts built since the last take: the rows a frame lays out.
    static MARKDOWN_FLOW_TEXTS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Row texts built since the last call, which resets the count.
pub(in crate::view) fn take_markdown_flow_texts_built_for_tests() -> usize {
    MARKDOWN_FLOW_TEXTS_BUILT.with(|built| built.replace(0))
}

pub(super) fn record_markdown_flow_fonts_for_tests(
    row_ix: usize,
    len: usize,
    base: &SharedString,
    overrides: &[(Range<usize>, SharedString)],
) {
    MARKDOWN_FLOW_FONT_LOG.with(|log| {
        if let Some(log) = log.borrow_mut().as_mut() {
            log.insert(row_ix, markdown_flow_font_runs(len, base, overrides));
        }
    });
}

pub(in crate::view) fn begin_markdown_flow_font_capture_for_tests() {
    MARKDOWN_FLOW_FONT_LOG.with(|log| *log.borrow_mut() = Some(Default::default()));
}

/// The font family each byte range of `row_ix` was laid out with, from the
/// frames drawn since capture began; ends the capture.
pub(in crate::view) fn markdown_flow_fonts_for_tests(row_ix: usize) -> MarkdownFlowFontRuns {
    MARKDOWN_FLOW_FONT_LOG.with(|log| {
        log.borrow_mut()
            .take()
            .and_then(|mut log| log.remove(&row_ix))
            .unwrap_or_default()
    })
}

pub(super) fn record_markdown_flow_paint_phase_for_tests(
    row_ix: usize,
    phase: MarkdownFlowPaintPhase,
) {
    MARKDOWN_FLOW_PAINT_PHASE_LOG.with(|log| {
        if let Some(log) = log.borrow_mut().as_mut() {
            log.push((row_ix, phase));
        }
    });
}

pub(in crate::view) fn begin_markdown_flow_paint_phase_capture_for_tests() {
    MARKDOWN_FLOW_PAINT_PHASE_LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
}

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

pub(in crate::view) fn clear_markdown_selection_paint_log_for_tests() {
    SELECTION_PAINT_LOG.with(|log| log.borrow_mut().clear());
}

/// Selection quads painted since the last clear, in paint order.
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

pub(super) fn record_row_built() {
    MARKDOWN_FLOW_TEXTS_BUILT.with(|built| built.set(built.get() + 1));
}
