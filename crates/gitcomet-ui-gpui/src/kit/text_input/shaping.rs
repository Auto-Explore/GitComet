use super::highlight::runs_for_line;
use super::state::{LineShapeInput, PlainLineLayouts, TextShapeStyle};
use super::*;

#[cfg(any(test, feature = "benchmarks"))]
pub(super) const TEXT_INPUT_SHAPING_FINGERPRINT_SAMPLE_BYTES: usize = 64;
#[cfg(any(test, feature = "benchmarks"))]
pub(super) const TEXT_INPUT_SHAPING_FINGERPRINT_MID_SAMPLES_TRUNCATED: usize = 3;
#[cfg(any(test, feature = "benchmarks"))]
pub(super) const TEXT_INPUT_SHAPING_FINGERPRINT_MID_SAMPLES_UNTRUNCATED: usize = 1;

#[derive(Clone, Copy)]
pub(super) struct ShapingSliceInfo<'a> {
    prefix: &'a str,
    capped_len: usize,
    truncated: bool,
}

impl<'a> ShapingSliceInfo<'a> {
    #[inline]
    fn new(line_text: &'a str, max_bytes: usize) -> Self {
        if line_text.len() <= max_bytes {
            return Self {
                prefix: line_text,
                capped_len: line_text.len(),
                truncated: false,
            };
        }

        let suffix_len = TEXT_INPUT_TRUNCATION_SUFFIX.len();
        let mut end = max_bytes.saturating_sub(suffix_len).min(line_text.len());
        while end > 0 && !line_text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }

        Self {
            prefix: &line_text[..end],
            capped_len: end.saturating_add(suffix_len),
            truncated: true,
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    #[inline]
    fn hash(self) -> u64 {
        hash_shaping_prefix_bytes(self.prefix.as_bytes(), self.capped_len, self.truncated)
    }

    #[inline]
    fn into_shared_string(self) -> SharedString {
        if !self.truncated {
            return self.prefix.to_string().into();
        }

        let mut truncated = String::with_capacity(self.capped_len);
        truncated.push_str(self.prefix);
        truncated.push_str(TEXT_INPUT_TRUNCATION_SUFFIX);
        truncated.into()
    }

    #[inline]
    fn into_cow(self) -> Cow<'a, str> {
        if !self.truncated {
            return Cow::Borrowed(self.prefix);
        }

        let mut truncated = String::with_capacity(self.capped_len);
        truncated.push_str(self.prefix);
        truncated.push_str(TEXT_INPUT_TRUNCATION_SUFFIX);
        Cow::Owned(truncated)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[inline]
pub(super) fn hash_shaping_prefix_bytes(
    prefix_bytes: &[u8],
    capped_len: usize,
    truncated: bool,
) -> u64 {
    let mut hasher = FxHasher::default();
    hasher.write_usize(capped_len);

    if prefix_bytes.len() <= TEXT_INPUT_SHAPING_FINGERPRINT_SAMPLE_BYTES * 4 {
        hasher.write(prefix_bytes);
        return hasher.finish();
    }

    let sample_len = TEXT_INPUT_SHAPING_FINGERPRINT_SAMPLE_BYTES;
    let mid_samples = if truncated {
        TEXT_INPUT_SHAPING_FINGERPRINT_MID_SAMPLES_TRUNCATED
    } else {
        // The uncapped path only needs a cheap stable whole-line sketch for
        // benchmark/test helpers, not the denser truncated-line sampling.
        TEXT_INPUT_SHAPING_FINGERPRINT_MID_SAMPLES_UNTRUNCATED
    };
    hasher.write(&prefix_bytes[..sample_len]);

    let last_start = prefix_bytes.len().saturating_sub(sample_len);
    if mid_samples > 0 {
        let gap = last_start.saturating_sub(sample_len);
        for sample_ix in 1..=mid_samples {
            let start = sample_len + gap.saturating_mul(sample_ix) / (mid_samples + 1);
            hasher.write_usize(start);
            hasher.write(&prefix_bytes[start..start + sample_len]);
        }
    }

    hasher.write_usize(last_start);
    hasher.write(&prefix_bytes[last_start..]);
    hasher.finish()
}

#[inline]
pub(super) fn shaping_slice_info(line_text: &str, max_bytes: usize) -> ShapingSliceInfo<'_> {
    ShapingSliceInfo::new(line_text, max_bytes)
}

/// Compute a stable fingerprint and capped byte length for a line that may need truncation.
/// This does NOT allocate, and on very long lines it samples representative chunks instead of
/// rescanning the full shaping prefix.
#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn hash_shaping_slice(line_text: &str, max_bytes: usize) -> (u64, usize) {
    let info = shaping_slice_info(line_text, max_bytes);
    (info.hash(), info.capped_len)
}

/// Tabs shaped as spaces: the same byte offsets, and no missing-glyph box.
/// [`apply_tab_stops`] then widens each to its tab stop.
pub(super) fn shaping_text_without_tabs(text: SharedString) -> SharedString {
    if !text.contains('\t') {
        return text;
    }
    SharedString::from(text.replace('\t', " "))
}

/// `layout` (shaped from [`shaping_text_without_tabs`] of `text`) with every
/// tab advanced to the next multiple of `tab_size` columns. Glyph indices are
/// untouched, so every index/x mapping over the line stays byte-exact. `None`
/// when there is nothing to move.
pub(super) fn apply_tab_stops(
    layout: &gpui::LineLayout,
    text: &str,
    tab_size: usize,
) -> Option<gpui::LineLayout> {
    let bytes = text.as_bytes();
    memchr::memchr(b'\t', bytes)?;
    let glyphs: Vec<(usize, usize)> = layout
        .runs
        .iter()
        .enumerate()
        .flat_map(|(run_ix, run)| (0..run.glyphs.len()).map(move |glyph_ix| (run_ix, glyph_ix)))
        .collect();
    let glyph = |k: usize| &layout.runs[glyphs[k].0].glyphs[glyphs[k].1];
    let x_of = |k: usize| f32::from(glyph(k).position.x);
    let is_tab = |k: usize| bytes.get(glyph(k).index) == Some(&b'\t');
    let advance_of = |k: usize| {
        if k + 1 < glyphs.len() {
            x_of(k + 1) - x_of(k)
        } else {
            f32::from(layout.width) - x_of(k)
        }
    };
    // Right-to-left runs are not laid out left to right; leave them be.
    if (1..glyphs.len()).any(|k| x_of(k) < x_of(k - 1)) {
        return None;
    }
    // A tab shaped as a space is one column wide (the editor font is monospace).
    let column = advance_of((0..glyphs.len()).find(|&k| is_tab(k))?);
    if column <= 0.0 {
        return None;
    }
    let tab_size = tab_size.max(1);
    let mut runs = layout.runs.clone();
    let mut shift = 0.0f32;
    let mut logical_column = 0usize;
    let mut after_tab = 0usize;
    for k in 0..glyphs.len() {
        let x = x_of(k) + shift;
        runs[glyphs[k].0].glyphs[glyphs[k].1].position.x = px(x);
        if is_tab(k) {
            let index = glyph(k).index;
            logical_column += text[after_tab..index].chars().count();
            let spaces = tab_size - logical_column % tab_size;
            shift += spaces as f32 * column - advance_of(k);
            logical_column += spaces;
            after_tab = index + 1;
        }
    }
    Some(gpui::LineLayout {
        font_size: layout.font_size,
        width: layout.width + px(shift),
        ascent: layout.ascent,
        descent: layout.descent,
        runs,
        len: layout.len,
    })
}

/// Build the (possibly truncated) SharedString for shaping. Only call on cache miss.
pub(super) fn build_shaping_text(line_text: &str, max_bytes: usize) -> SharedString {
    shaping_slice_info(line_text, max_bytes).into_shared_string()
}

pub(super) fn build_shaping_line_slice<'a>(line_text: &'a str, max_bytes: usize) -> Cow<'a, str> {
    shaping_slice_info(line_text, max_bytes).into_cow()
}

#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn truncate_line_for_shaping(line_text: &str, max_bytes: usize) -> (SharedString, u64) {
    let info = shaping_slice_info(line_text, max_bytes);
    let hash = info.hash();
    let text = info.into_shared_string();
    (text, hash)
}

#[cfg(feature = "benchmarks")]
#[inline]
pub(crate) fn benchmark_text_input_shaping_slice(text: &str, max_bytes: usize) -> (u64, usize) {
    hash_shaping_slice(text, max_bytes)
}

#[cfg(test)]
thread_local! {
    static WRAPPED_LINES_SHAPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Soft-wrapped lines this thread shaped since the last call.
#[cfg(test)]
pub(crate) fn take_wrapped_lines_shaped_for_tests() -> usize {
    WRAPPED_LINES_SHAPED.with(|shaped| shaped.replace(0))
}

/// Shape one soft-wrapped source line.
///
/// Unlike the plain path there is no cache here: `WrappedLine` shaping is
/// driven by wrap width as well as text, and the wrapped-row cache it used to
/// write was never read back.
pub(super) fn shape_wrapped_line(
    line: LineShapeInput<'_>,
    wrap_width: Pixels,
    precomputed_runs: Option<&[TextRun]>,
    shape_style: &TextShapeStyle<'_>,
    tab_size: usize,
    window: &mut Window,
) -> WrappedLine {
    #[cfg(test)]
    WRAPPED_LINES_SHAPED.with(|shaped| shaped.set(shaped.get() + 1));
    let capped_text = build_shaping_text(line.line_text, TEXT_INPUT_MAX_LINE_SHAPE_BYTES);
    let owned_runs;
    let runs = if let Some(precomputed_runs) = precomputed_runs {
        precomputed_runs
    } else {
        owned_runs = runs_for_line(
            shape_style.base_font,
            shape_style.text_color,
            line.line_start,
            capped_text.as_ref(),
            shape_style.highlights,
        );
        owned_runs.as_slice()
    };
    let with_tabs = capped_text.clone();
    let shaped = window
        .text_system()
        .shape_text(
            shaping_text_without_tabs(capped_text),
            shape_style.font_size,
            runs,
            Some(wrap_width),
            None,
        )
        .unwrap_or_default();
    let mut wrapped: WrappedLine = shaped.into_iter().next().unwrap_or_default();
    if let Some(layout) = apply_tab_stops_wrapped(&wrapped, &with_tabs, tab_size) {
        *wrapped = Arc::new(layout);
    }
    wrapped
}

/// [`apply_tab_stops`] for a soft-wrapped line, wrapped again: gpui chose
/// its breaks measuring each tab as one space.
pub(super) fn apply_tab_stops_wrapped(
    wrapped: &gpui::WrappedLineLayout,
    text: &str,
    tab_size: usize,
) -> Option<gpui::WrappedLineLayout> {
    let unwrapped = apply_tab_stops(&wrapped.unwrapped_layout, text, tab_size)?;
    let wrap_boundaries = match wrapped.wrap_width {
        Some(wrap_width) => wrap_boundaries(&unwrapped, text, wrap_width)
            .into_iter()
            .collect(),
        None => wrapped.wrap_boundaries.clone(),
    };
    Some(gpui::WrappedLineLayout {
        unwrapped_layout: Arc::new(unwrapped),
        wrap_boundaries,
        wrap_width: wrapped.wrap_width,
    })
}

/// gpui's private `LineLayout::compute_wrap_boundaries` (no line clamp),
/// over glyph positions that include tab stops. Tabs count as the spaces
/// they were shaped as.
fn wrap_boundaries(
    layout: &gpui::LineLayout,
    text: &str,
    wrap_width: Pixels,
) -> Vec<gpui::WrapBoundary> {
    let mut boundaries = Vec::new();
    let mut seen_non_whitespace = false;
    let mut last_candidate: Option<(gpui::WrapBoundary, Pixels)> = None;
    let mut last_boundary = gpui::WrapBoundary {
        run_ix: 0,
        glyph_ix: 0,
    };
    let mut last_boundary_x = px(0.0);
    let mut prev_ch = '\0';
    let mut glyphs = layout
        .runs
        .iter()
        .enumerate()
        .flat_map(|(run_ix, run)| {
            run.glyphs.iter().enumerate().map(move |(glyph_ix, glyph)| {
                let boundary = gpui::WrapBoundary { run_ix, glyph_ix };
                (boundary, glyph.index, glyph.position.x)
            })
        })
        .peekable();
    while let Some((boundary, index, x)) = glyphs.next() {
        let ch = match text.get(index..).and_then(|rest| rest.chars().next()) {
            Some('\t') => ' ',
            Some('\n') | None => continue,
            Some(ch) => ch,
        };
        if is_word_char(ch) {
            if prev_ch == ' ' && ch != ' ' && seen_non_whitespace {
                last_candidate = Some((boundary, x));
            }
        } else if ch != ' ' && seen_non_whitespace {
            last_candidate = Some((boundary, x));
        }
        seen_non_whitespace |= ch != ' ';
        let next_x = glyphs.peek().map_or(layout.width, |&(_, _, x)| x);
        if next_x - last_boundary_x > wrap_width && boundary > last_boundary {
            (last_boundary, last_boundary_x) = last_candidate.take().unwrap_or((boundary, x));
            boundaries.push(last_boundary);
        }
        prev_ch = ch;
    }
    boundaries
}

/// gpui's `LineWrapper::is_word_char` (crate-private there): characters a
/// line is not broken before when they follow another.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '\u{00C0}'..='\u{024F}'
                | '\u{0400}'..='\u{04FF}'
                | '\u{1E00}'..='\u{1EFF}'
                | '\u{0300}'..='\u{036F}'
                | '\u{0980}'..='\u{09FF}'
        )
        || matches!(
            c,
            '-' | '_'
                | '.'
                | '\''
                | '’'
                | '‘'
                | '$'
                | '%'
                | '@'
                | '#'
                | '^'
                | '~'
                | ','
                | '='
                | ':'
                | ';'
                | '!'
                | ')'
                | ']'
                | '}'
                | '"'
                | '”'
                | '»'
                | '…'
                | '⋯'
                | '\u{202F}'
                | '\u{00A0}'
                | '\u{2011}'
        )
}

pub(super) use crate::theme::with_alpha;

#[cfg(target_os = "macos")]
pub(super) fn primary_modifier_label() -> &'static str {
    "Cmd"
}

#[cfg(not(target_os = "macos"))]
pub(super) fn primary_modifier_label() -> &'static str {
    "Ctrl"
}

pub(super) fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(8);
    starts.push(0);
    for (ix, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(ix + 1);
        }
    }
    starts
}

/// Where a frame reads its row text from.
///
/// The soft-wrap and masked arms hand over a document they already hold as one
/// `&str`; the plain arm copies out only the rows it is about to shape, so a
/// frame costs the viewport rather than the document.
pub(super) enum LineTextSource<'a> {
    Whole {
        text: &'a str,
        starts: &'a [usize],
    },
    Window {
        rows: Range<usize>,
        text: String,
        starts: Vec<usize>,
        /// The caret's row, when it sits outside `rows` and still has to be
        /// shaped to place the caret.
        stray: Option<(usize, String)>,
    },
}

impl LineTextSource<'_> {
    /// Copy out `rows` (and `extra`, if it falls outside them) from `snapshot`.
    ///
    /// Cost is proportional to the rows taken, not to the document.
    pub(super) fn window(
        snapshot: &crate::kit::text_model::TextModelSnapshot,
        rows: Range<usize>,
        extra: Option<usize>,
    ) -> Self {
        let mut text = String::new();
        let mut starts = Vec::with_capacity(rows.len() + 1);
        let line_count = snapshot.line_count();
        for row in rows.clone() {
            starts.push(text.len());
            text.push_str(snapshot.line_text(row).as_ref());
            // Re-add the terminator the model strips, so `line_text_for_index`
            // sees the same shape it does for a whole document — but only where
            // the document really has one. A final row with no trailing newline
            // that ends in `\r` keeps that `\r` in the `Whole` variant, so
            // inventing a terminator here would make the same row shape one
            // character narrower in this arm than in the other.
            if row + 1 < line_count {
                text.push('\n');
            }
        }
        starts.push(text.len());

        // The caret's row, when it is outside the window. Stored already
        // stripped: the windowed rows lose their `\r` to `line_text_for_index`
        // and the `Whole` variant strips it too, but `line_text` only excludes
        // the `\n`. Leaving it on would make the caret's row shape one column
        // wider than the identical row shapes in-viewport, so scrolling the
        // caret in and out would move it.
        let stray = extra.filter(|row| !rows.contains(row)).map(|row| {
            let text = snapshot.line_text(row);
            // Strip only where the document really terminates the row, exactly
            // as the windowed rows above do: a `\r` at the very end of a file
            // with no trailing newline survives in the `Whole` variant, so it
            // has to survive here.
            let text = match text.strip_suffix('\r') {
                Some(stripped) if row + 1 < line_count => stripped,
                _ => text.as_ref(),
            };
            (row, text.to_owned())
        });

        Self::Window {
            rows,
            text,
            starts,
            stray,
        }
    }

    /// Text of `line_ix`, excluding its terminator. Empty for rows outside the
    /// window, which a correct caller never asks for.
    pub(super) fn line_text(&self, line_ix: usize) -> &str {
        match self {
            Self::Whole { text, starts } => line_text_for_index(text, starts, line_ix),
            Self::Window {
                rows,
                text,
                starts,
                stray,
            } => {
                if let Some((stray_row, stray_text)) = stray
                    && *stray_row == line_ix
                {
                    return stray_text;
                }
                if !rows.contains(&line_ix) {
                    return "";
                }
                line_text_for_index(text, starts, line_ix - rows.start)
            }
        }
    }
}

pub(super) fn line_text_for_index<'a>(text: &'a str, starts: &[usize], line_ix: usize) -> &'a str {
    let text_len = text.len();
    let Some(start) = starts.get(line_ix).copied() else {
        return "";
    };
    if start >= text_len {
        return "";
    }

    let mut end = starts
        .get(line_ix + 1)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if end > start && text.as_bytes().get(end - 1) == Some(&b'\n') {
        end -= 1;
        if end > start && text.as_bytes().get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
    }
    text.get(start..end).unwrap_or("")
}

pub(super) fn mask_text_for_display(text: &str) -> String {
    let mut masked = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        match byte {
            b'\n' => masked.push('\n'),
            b'\r' => masked.push('\r'),
            _ => masked.push('*'),
        }
    }
    masked
}

pub(super) fn truncated_line_x_for_source_offset(
    line: &TruncatedLineLayout,
    source_offset: usize,
) -> Pixels {
    if let Some((hidden_range, display_range)) = line
        .projection
        .ellipsis_segment_for_source_offset(source_offset)
    {
        let hidden_mid =
            hidden_range.start + (hidden_range.end.saturating_sub(hidden_range.start) / 2);
        let display_offset = if source_offset <= hidden_mid {
            display_range.start
        } else {
            display_range.end
        };
        return line.shaped_line.x_for_index(display_offset);
    }

    let display_offset = line.projection.source_to_display_offset(source_offset);
    line.shaped_line.x_for_index(display_offset)
}

pub(super) fn truncated_line_source_offset_for_x(line: &TruncatedLineLayout, x: Pixels) -> usize {
    let display_offset = line.shaped_line.closest_index_for_x(x.max(px(0.0)));
    if let Some((hidden_range, display_range)) = line
        .projection
        .ellipsis_segment_at_display_offset(display_offset)
    {
        let x0 = line.shaped_line.x_for_index(display_range.start);
        let x1 = line.shaped_line.x_for_index(display_range.end);
        let midpoint = x0 + (x1 - x0) / 2.0;
        return if x <= midpoint {
            hidden_range.start
        } else {
            hidden_range.end
        };
    }

    line.projection
        .display_to_source_start_offset(display_offset)
}

/// The line index and line-local byte offset for a document offset.
///
/// The local offset is clamped to the shaped line's length when that line is
/// one of the shaped ones; for a line outside the shaped window callers pair
/// this with `PlainLineLayouts::get`, which yields `None`, so the unclamped
/// offset is never turned into geometry.
pub(super) fn line_for_offset(
    starts: &[usize],
    lines: &PlainLineLayouts,
    offset: usize,
) -> (usize, usize) {
    let mut ix = starts.partition_point(|&s| s <= offset);
    if ix == 0 {
        ix = 1;
    }
    let line_ix = (ix - 1).min(lines.line_count().saturating_sub(1));
    let start = starts.get(line_ix).copied().unwrap_or(0);
    let local = offset.saturating_sub(start);
    let local = match lines.get(line_ix) {
        Some(line) => local.min(line.len()),
        None => local,
    };
    (line_ix, local)
}

#[cfg(test)]
mod tab_stop_tests {
    use super::*;

    /// A monospace line: one glyph per byte, `column` pixels apart.
    fn mono_layout(text: &str, column: f32) -> gpui::LineLayout {
        let glyphs = (0..text.len())
            .map(|ix| gpui::ShapedGlyph {
                id: gpui::GlyphId(ix as u32),
                position: gpui::point(px(ix as f32 * column), px(0.0)),
                index: ix,
                is_emoji: false,
            })
            .collect();
        gpui::LineLayout {
            font_size: px(12.0),
            width: px(text.len() as f32 * column),
            ascent: px(10.0),
            descent: px(2.0),
            runs: vec![gpui::ShapedRun {
                font_id: gpui::FontId(0),
                glyphs,
            }],
            len: text.len(),
        }
    }

    fn xs(layout: &gpui::LineLayout) -> Vec<f32> {
        layout.runs[0]
            .glyphs
            .iter()
            .map(|glyph| f32::from(glyph.position.x))
            .collect()
    }

    #[test]
    fn review_wide_characters_use_the_same_tab_columns_as_diff() {
        let text = "日本\tx";
        let mut layout = mono_layout("abcd", 10.0);
        layout.len = text.len();
        layout.width = px(60.0);
        for (glyph, (index, x)) in
            layout.runs[0]
                .glyphs
                .iter_mut()
                .zip([(0, 0.0), (3, 20.0), (6, 40.0), (7, 50.0)])
        {
            glyph.index = index;
            glyph.position.x = px(x);
        }
        let layout = apply_tab_stops(&layout, text, 4).unwrap();
        // The diff expands two characters plus a tab to 日本<space><space>x.
        assert_eq!(xs(&layout)[3], 60.0);
        assert_eq!(layout.runs[0].glyphs[3].index, 7);
    }

    #[test]
    fn tabs_advance_to_the_next_stop() {
        let text = "a\tbc\td";
        let layout = apply_tab_stops(&mono_layout(text, 10.0), text, 4).expect("tabs moved");
        // a@0, tab@10 → stop 40, b@40, c@50, tab@60 → stop 80, d@80.
        assert_eq!(xs(&layout), vec![0.0, 10.0, 40.0, 50.0, 60.0, 80.0]);
        assert_eq!(f32::from(layout.width), 90.0);
        assert_eq!(layout.runs[0].glyphs[2].index, 2, "indices are untouched");
    }

    #[test]
    fn a_tab_on_a_stop_takes_a_whole_stop_and_width_is_configurable() {
        let text = "abcd\tx";
        let layout = apply_tab_stops(&mono_layout(text, 10.0), text, 4).unwrap();
        assert_eq!(xs(&layout)[5], 80.0);
        let layout = apply_tab_stops(&mono_layout("\tx", 10.0), "\tx", 8).unwrap();
        assert_eq!(xs(&layout)[1], 80.0);
    }

    #[test]
    fn a_wrapped_line_wraps_at_its_widened_tabs() {
        let text = "\t\t\tabc def";
        // gpui measured each tab as one column: 10 columns fit in 100 px.
        let wrapped = gpui::WrappedLineLayout {
            unwrapped_layout: Arc::new(mono_layout(text, 10.0)),
            wrap_boundaries: Default::default(),
            wrap_width: Some(px(100.0)),
        };
        let layout = apply_tab_stops_wrapped(&wrapped, text, 4).expect("tabs moved");
        // Tabs end at 40/80/120, `abc ` at 120..160, `def` at 160..190.
        let boundary = |glyph_ix| gpui::WrapBoundary {
            run_ix: 0,
            glyph_ix,
        };
        assert_eq!(
            layout.wrap_boundaries.as_slice(),
            &[boundary(2), boundary(7)]
        );
        let row_starts = [0.0, 80.0, 160.0, f32::from(layout.unwrapped_layout.width)];
        for row in row_starts.windows(2) {
            assert!(row[1] - row[0] <= 100.0, "{row:?} overflows the wrap width");
        }
    }

    #[gpui::test]
    fn the_wrap_port_breaks_where_gpui_does(cx: &mut gpui::TestAppContext) {
        let cx = cx.add_empty_window();
        let text = "fn main() { let value = foo-bar(baz, qux); // see: a/b/c?d=1 plz! Grüße 日本語テキスト }";
        let run = super::super::highlight::text_run_for_style(
            &gpui::font("Monospace"),
            gpui::black(),
            text.len(),
            None,
        );
        let mut wrapped_rows = 0;
        for width in [30.0, 55.0, 80.0, 130.0, 400.0] {
            let wrapped = cx.update(|window, _| {
                window
                    .text_system()
                    .shape_text(
                        SharedString::from(text),
                        px(12.0),
                        &[run.clone()],
                        Some(px(width)),
                        None,
                    )
                    .expect("shaped")
                    .into_iter()
                    .next()
                    .expect("one line")
            });
            assert_eq!(
                wrap_boundaries(&wrapped.unwrapped_layout, text, px(width)).as_slice(),
                wrapped.wrap_boundaries.as_slice(),
                "wrap width {width}"
            );
            wrapped_rows += wrapped.wrap_boundaries.len();
        }
        assert!(wrapped_rows > 10, "the widths must actually wrap");
    }

    #[test]
    fn lines_without_tabs_are_left_alone() {
        assert!(apply_tab_stops(&mono_layout("abc", 10.0), "abc", 4).is_none());
        assert_eq!(shaping_text_without_tabs("a\tb".into()).as_ref(), "a b");
    }
}
