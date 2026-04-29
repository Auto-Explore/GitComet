use super::shaping::*;
use super::state::*;
use super::*;

pub(super) fn visible_window_runs_for_line_ix(
    line_runs_by_visible_line: Option<&VisibleWindowTextRuns>,
    visible_start: usize,
    line_ix: usize,
) -> Option<&[TextRun]> {
    let visible_runs = line_runs_by_visible_line?;
    let local_ix = line_ix.checked_sub(visible_start)?;
    visible_runs.line(local_ix)
}

#[derive(Clone)]
pub(super) struct ActiveHighlight<'a> {
    end: usize,
    style: &'a gpui::HighlightStyle,
}

pub(super) type ActiveHighlightBuffer<'a> =
    SmallVec<[ActiveHighlight<'a>; TEXT_INPUT_INLINE_ACTIVE_HIGHLIGHT_CAPACITY]>;
pub(super) type LineTextRuns = SmallVec<[TextRun; TEXT_INPUT_INLINE_TEXT_RUN_CAPACITY]>;

pub(super) trait TextRunSink {
    fn push_text_run(&mut self, run: TextRun);
}

impl TextRunSink for Vec<TextRun> {
    fn push_text_run(&mut self, run: TextRun) {
        self.push(run);
    }
}

impl<const N: usize> TextRunSink for SmallVec<[TextRun; N]> {
    fn push_text_run(&mut self, run: TextRun) {
        self.push(run);
    }
}

pub(super) struct HighlightCursor<'a> {
    highlights: &'a [(Range<usize>, gpui::HighlightStyle)],
    next_ix: usize,
    active: ActiveHighlightBuffer<'a>,
}

impl<'a> HighlightCursor<'a> {
    fn new_at_offset(
        highlights: &'a [(Range<usize>, gpui::HighlightStyle)],
        offset: usize,
    ) -> Self {
        let next_ix = highlights.partition_point(|(range, _)| range.start < offset);
        let mut active_start = next_ix;
        let mut active = ActiveHighlightBuffer::new();
        while active_start > 0 {
            let order = active_start.saturating_sub(1);
            let Some((range, style)) = highlights.get(order) else {
                break;
            };
            if range.end <= offset {
                break;
            }
            active_start = order;
            active.push(ActiveHighlight {
                end: range.end,
                style,
            });
        }
        Self {
            highlights,
            next_ix,
            active,
        }
    }

    fn advance_to_line_start(&mut self, line_start: usize) {
        self.active.retain(|highlight| highlight.end > line_start);
        while let Some((range, style)) = self.highlights.get(self.next_ix) {
            if range.end <= line_start {
                self.next_ix = self.next_ix.saturating_add(1);
                continue;
            }
            if range.start < line_start {
                self.active.push(ActiveHighlight {
                    end: range.end,
                    style,
                });
                self.next_ix = self.next_ix.saturating_add(1);
                continue;
            }
            break;
        }
    }

    fn append_simple_runs_for_line(
        &self,
        base_font: &gpui::Font,
        base_color: gpui::Hsla,
        line_start: usize,
        line_end: usize,
        runs: &mut impl TextRunSink,
    ) -> bool {
        let next_highlight = self
            .highlights
            .get(self.next_ix)
            .filter(|(range, _)| range.start < line_end);

        match (self.active.len(), next_highlight) {
            (0, None) => {
                runs.push_text_run(text_run_for_style(
                    base_font,
                    base_color,
                    line_end.saturating_sub(line_start),
                    None,
                ));
                true
            }
            (0, Some((range, style)))
                if self
                    .highlights
                    .get(self.next_ix.saturating_add(1))
                    .map(|(next_range, _)| next_range.start >= line_end)
                    .unwrap_or(true) =>
            {
                if range.start > line_start {
                    runs.push_text_run(text_run_for_style(
                        base_font,
                        base_color,
                        range.start.saturating_sub(line_start),
                        None,
                    ));
                }
                let styled_end = range.end.min(line_end);
                if styled_end > range.start {
                    runs.push_text_run(text_run_for_style(
                        base_font,
                        base_color,
                        styled_end.saturating_sub(range.start),
                        Some(style),
                    ));
                }
                if styled_end < line_end {
                    runs.push_text_run(text_run_for_style(
                        base_font,
                        base_color,
                        line_end.saturating_sub(styled_end),
                        None,
                    ));
                }
                true
            }
            (1, None) => {
                let Some(active) = self.active.first() else {
                    return false;
                };
                let styled_end = active.end.min(line_end);
                runs.push_text_run(text_run_for_style(
                    base_font,
                    base_color,
                    styled_end.saturating_sub(line_start),
                    Some(active.style),
                ));
                if styled_end < line_end {
                    runs.push_text_run(text_run_for_style(
                        base_font,
                        base_color,
                        line_end.saturating_sub(styled_end),
                        None,
                    ));
                }
                true
            }
            _ => false,
        }
    }

    fn try_simple_runs_for_line(
        &self,
        base_font: &gpui::Font,
        base_color: gpui::Hsla,
        line_start: usize,
        line_end: usize,
    ) -> Option<LineTextRuns> {
        let mut runs = LineTextRuns::new();
        if self.append_simple_runs_for_line(base_font, base_color, line_start, line_end, &mut runs)
        {
            Some(runs)
        } else {
            None
        }
    }

    fn append_runs_for_line(
        &mut self,
        base_font: &gpui::Font,
        base_color: gpui::Hsla,
        line_start: usize,
        line_text: &str,
        runs: &mut impl TextRunSink,
    ) {
        if line_text.is_empty() {
            return;
        }
        self.advance_to_line_start(line_start);
        self.append_runs_for_current_line(base_font, base_color, line_start, line_text, runs);
    }

    fn append_runs_for_current_line(
        &mut self,
        base_font: &gpui::Font,
        base_color: gpui::Hsla,
        line_start: usize,
        line_text: &str,
        runs: &mut impl TextRunSink,
    ) {
        let line_end = line_start + line_text.len();

        let mut offset = line_start;
        while offset < line_end {
            while let Some((range, style)) = self.highlights.get(self.next_ix) {
                if range.end <= offset {
                    self.next_ix = self.next_ix.saturating_add(1);
                    continue;
                }
                if range.start > offset || range.start >= line_end {
                    break;
                }
                self.active.push(ActiveHighlight {
                    end: range.end,
                    style,
                });
                self.next_ix = self.next_ix.saturating_add(1);
            }

            let mut next_boundary = line_end;
            if let Some((next_range, _)) = self.highlights.get(self.next_ix)
                && next_range.start < line_end
            {
                next_boundary = next_boundary.min(next_range.start);
            }
            // Active highlights stay in insertion order, and later highlights
            // win precedence, so the last active entry is always the visible
            // style at the current offset.
            let top_highlight = self.active.last();
            let style = top_highlight.map(|highlight| highlight.style);
            if let Some(top_highlight) = top_highlight {
                // Only the current top highlight's end can change the visible
                // style; lower-priority highlight ends should not split runs.
                next_boundary = next_boundary.min(top_highlight.end);
            }
            if next_boundary <= offset {
                next_boundary = (offset + 1).min(line_end);
            }

            runs.push_text_run(text_run_for_style(
                base_font,
                base_color,
                next_boundary - offset,
                style,
            ));
            self.active
                .retain(|highlight| highlight.end > next_boundary);
            offset = next_boundary;
        }
    }
}

pub(super) fn build_streamed_highlight_runs_for_visible_window(
    base_font: &gpui::Font,
    base_color: gpui::Hsla,
    display_text: &str,
    line_starts: &[usize],
    visible_line_range: Range<usize>,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> VisibleWindowTextRuns {
    let mut line_runs = VisibleWindowTextRuns::with_line_capacity(visible_line_range.len());
    if visible_line_range.is_empty() {
        return line_runs;
    }
    let first_line_start = line_starts
        .get(visible_line_range.start)
        .copied()
        .unwrap_or(0);
    let mut cursor = HighlightCursor::new_at_offset(highlights, first_line_start);
    for line_ix in visible_line_range {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let line_text = line_text_for_index(display_text, line_starts, line_ix);
        let capped_line_text = build_shaping_line_slice(line_text, TEXT_INPUT_MAX_LINE_SHAPE_BYTES);
        let capped_line_text = capped_line_text.as_ref();
        if !capped_line_text.is_empty() {
            let line_end = line_start.saturating_add(capped_line_text.len());
            cursor.advance_to_line_start(line_start);
            if !cursor.append_simple_runs_for_line(
                base_font,
                base_color,
                line_start,
                line_end,
                &mut line_runs.runs,
            ) {
                cursor.append_runs_for_current_line(
                    base_font,
                    base_color,
                    line_start,
                    capped_line_text,
                    &mut line_runs.runs,
                );
            }
        }
        line_runs.finish_line();
    }
    line_runs
}

pub(super) fn text_run_for_style(
    base_font: &gpui::Font,
    base_color: gpui::Hsla,
    len: usize,
    style: Option<&gpui::HighlightStyle>,
) -> TextRun {
    let mut font = base_font.clone();
    let mut color = base_color;
    let mut background_color = None;
    let mut underline = None;
    let mut strikethrough = None;

    if let Some(style) = style {
        if let Some(next_color) = style.color {
            color = next_color;
        }
        if let Some(next_weight) = style.font_weight {
            font.weight = next_weight;
        }
        if let Some(next_style) = style.font_style {
            font.style = next_style;
        }
        background_color = style.background_color;
        underline = style.underline;
        strikethrough = style.strikethrough;
        if let Some(fade_out) = style.fade_out {
            color.a *= (1.0 - fade_out).clamp(0.0, 1.0);
        }
    }

    TextRun {
        len,
        font,
        color,
        background_color,
        underline,
        strikethrough,
    }
}

pub(super) fn runs_for_line(
    base_font: &gpui::Font,
    base_color: gpui::Hsla,
    line_start: usize,
    line_text: &str,
    highlights: Option<&[(Range<usize>, gpui::HighlightStyle)]>,
) -> LineTextRuns {
    if line_text.is_empty() {
        return LineTextRuns::new();
    }

    let Some(highlights) = highlights else {
        let mut runs = LineTextRuns::new();
        runs.push(text_run_for_style(
            base_font,
            base_color,
            line_text.len(),
            None,
        ));
        return runs;
    };

    let line_end = line_start.saturating_add(line_text.len());
    let mut cursor = HighlightCursor::new_at_offset(highlights, line_start);
    if let Some(runs) = cursor.try_simple_runs_for_line(base_font, base_color, line_start, line_end)
    {
        return runs;
    }
    let mut runs = LineTextRuns::new();
    cursor.append_runs_for_line(base_font, base_color, line_start, line_text, &mut runs);
    runs
}

#[cfg(feature = "benchmarks")]
pub(super) fn hash_text_runs_for_benchmark(runs: &[TextRun], hasher: &mut FxHasher) {
    runs.len().hash(hasher);
    let mut total = 0usize;
    for run in runs {
        total = total.saturating_add(run.len);
        run.len.hash(hasher);
        run.color.a.to_bits().hash(hasher);
    }
    total.hash(hasher);
}

#[cfg(feature = "benchmarks")]
pub(crate) fn benchmark_text_input_runs_legacy_visible_window(
    text: &str,
    line_starts: &[usize],
    visible_line_range: Range<usize>,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> u64 {
    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let mut hasher = FxHasher::default();
    for line_ix in visible_line_range {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let line_text = line_text_for_index(text, line_starts, line_ix);
        let (capped_line_text, _) =
            truncate_line_for_shaping(line_text, TEXT_INPUT_MAX_LINE_SHAPE_BYTES);
        let runs = runs_for_line(
            &base_font,
            base_color,
            line_start,
            capped_line_text.as_ref(),
            Some(highlights),
        );
        hash_text_runs_for_benchmark(runs.as_slice(), &mut hasher);
    }
    hasher.finish()
}

#[cfg(feature = "benchmarks")]
pub(crate) fn benchmark_text_input_runs_streamed_visible_window(
    text: &str,
    line_starts: &[usize],
    visible_line_range: Range<usize>,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> u64 {
    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let line_runs = build_streamed_highlight_runs_for_visible_window(
        &base_font,
        base_color,
        text,
        line_starts,
        visible_line_range,
        highlights,
    );
    let mut hasher = FxHasher::default();
    for local_ix in 0..line_runs.len() {
        if let Some(runs) = line_runs.line(local_ix) {
            hash_text_runs_for_benchmark(runs, &mut hasher);
        }
    }
    hasher.finish()
}
