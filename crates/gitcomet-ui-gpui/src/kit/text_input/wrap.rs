use super::shaping::*;
use super::state::*;
use super::*;

#[cfg(feature = "benchmarks")]
#[inline]
pub(crate) fn benchmark_text_input_wrap_rows_for_line(text: &str, wrap_columns: usize) -> usize {
    estimate_wrap_rows_for_line(text, wrap_columns)
}

pub(super) fn wrap_width_cache_key(wrap_width: Pixels) -> i32 {
    let mut key = f32::from(wrap_width.round()) as i32;
    if key == i32::MIN {
        key = i32::MIN + 1;
    }
    key
}

pub(super) fn line_index_for_offset(starts: &[usize], offset: usize, line_count: usize) -> usize {
    if line_count == 0 {
        return 0;
    }
    let mut ix = starts.partition_point(|&s| s <= offset);
    if ix == 0 {
        ix = 1;
    }
    (ix - 1).min(line_count.saturating_sub(1))
}

pub(super) fn visible_vertical_window(
    bounds: Bounds<Pixels>,
    scroll_handle: Option<&ScrollHandle>,
) -> (Pixels, Pixels) {
    let full_top = Pixels::ZERO;
    let full_bottom = bounds.size.height.max(px(0.0));
    let Some(scroll_handle) = scroll_handle else {
        return (full_top, full_bottom);
    };

    let viewport = scroll_handle.bounds();
    let top = (viewport.top() - bounds.top()).max(Pixels::ZERO);
    let bottom = (viewport.bottom() - bounds.top())
        .max(Pixels::ZERO)
        .min(full_bottom);
    if bottom <= top {
        (full_top, full_bottom)
    } else {
        (top, bottom)
    }
}

pub(super) fn visible_plain_line_range(
    line_count: usize,
    line_height: Pixels,
    visible_top: Pixels,
    visible_bottom: Pixels,
    guard_rows: usize,
) -> Range<usize> {
    if line_count == 0 {
        return 0..0;
    }
    let safe_line_height = if line_height <= px(0.0) {
        px(1.0)
    } else {
        line_height
    };
    let start_row = (f32::from(visible_top) / f32::from(safe_line_height))
        .floor()
        .max(0.0) as usize;
    let end_row = (f32::from(visible_bottom) / f32::from(safe_line_height))
        .ceil()
        .max(0.0) as usize;
    let start = start_row
        .saturating_sub(guard_rows)
        .min(line_count.saturating_sub(1));
    let mut end = end_row
        .saturating_add(guard_rows.saturating_add(1))
        .min(line_count);
    if end <= start {
        end = (start + 1).min(line_count);
    }
    start..end
}

pub(super) fn byte_range_for_line_range(
    line_starts: &[usize],
    text_len: usize,
    line_range: Range<usize>,
) -> Range<usize> {
    if line_range.is_empty() {
        return 0..0;
    }

    let start = line_starts
        .get(line_range.start)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    let end = line_starts
        .get(line_range.end)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    start.min(end)..end.max(start)
}

pub(super) fn provider_prefetch_byte_range_for_visible_window(
    line_starts: &[usize],
    text_len: usize,
    line_count: usize,
    line_height: Pixels,
    visible_top: Pixels,
    visible_bottom: Pixels,
) -> Range<usize> {
    let line_range = visible_plain_line_range(
        line_count,
        line_height,
        visible_top,
        visible_bottom,
        TEXT_INPUT_PROVIDER_PREFETCH_GUARD_ROWS,
    );
    byte_range_for_line_range(line_starts, text_len, line_range)
}

pub(super) fn wrapped_line_index_for_y(
    y_offsets: &[Pixels],
    row_counts: &[usize],
    _line_height: Pixels,
    local_y: Pixels,
) -> usize {
    let line_count = y_offsets.len().min(row_counts.len());
    if line_count == 0 {
        return 0;
    }
    y_offsets[..line_count]
        .partition_point(|&y| y <= local_y)
        .saturating_sub(1)
        .min(line_count.saturating_sub(1))
}

pub(super) fn visible_wrapped_line_range(
    y_offsets: &[Pixels],
    row_counts: &[usize],
    line_height: Pixels,
    visible_top: Pixels,
    visible_bottom: Pixels,
    guard_rows: usize,
) -> Range<usize> {
    let line_count = y_offsets.len().min(row_counts.len());
    if line_count == 0 {
        return 0..0;
    }
    let safe_line_height = if line_height <= px(0.0) {
        px(1.0)
    } else {
        line_height
    };

    let guard = safe_line_height * guard_rows as f32;
    let top = (visible_top - guard).max(Pixels::ZERO);
    let bottom = (visible_bottom + guard).max(top);
    let y_offsets = &y_offsets[..line_count];
    let row_counts = &row_counts[..line_count];
    let start = wrapped_line_index_for_y(y_offsets, row_counts, safe_line_height, top)
        .min(line_count.saturating_sub(1));
    let mut end = y_offsets.partition_point(|&y| y <= bottom).min(line_count);
    if end <= start {
        end = (start + 1).min(line_count);
    }
    start..end
}

pub(super) fn total_wrap_rows(row_counts: &[usize]) -> usize {
    row_counts
        .iter()
        .copied()
        .map(|rows| rows.max(1))
        .sum::<usize>()
        .max(1)
}

pub(super) fn wrap_columns_for_width(wrap_width: Pixels, font_size: Pixels) -> usize {
    let width_px = f32::from(wrap_width.max(px(1.0)));
    let font_px = f32::from(font_size.max(px(1.0)));
    let advance_px = (font_px * TEXT_INPUT_WRAP_CHAR_ADVANCE_FACTOR).max(1.0);
    (width_px / advance_px).floor().max(1.0) as usize
}

pub(super) fn estimate_wrap_rows_for_text(text: &str, wrap_columns: usize) -> Vec<usize> {
    let line_starts = compute_line_starts(text);
    let mut rows = Vec::with_capacity(line_starts.len().max(1));
    estimate_wrap_rows_with_line_starts(text, line_starts.as_slice(), wrap_columns, &mut rows);
    rows
}

pub(super) fn estimate_wrap_rows_with_line_starts(
    text: &str,
    line_starts: &[usize],
    wrap_columns: usize,
    rows: &mut Vec<usize>,
) {
    let line_count = line_starts.len().max(1);
    rows.resize(line_count, 1);
    for (line_ix, row_slot) in rows.iter_mut().take(line_count).enumerate() {
        if line_ix > 0 && line_ix % TEXT_INPUT_WRAP_BACKGROUND_YIELD_EVERY_ROWS == 0 {
            std::thread::yield_now();
        }
        let line_text = line_text_for_index(text, line_starts, line_ix);
        *row_slot = estimate_wrap_rows_for_line(line_text, wrap_columns);
    }
}

pub(super) fn estimate_wrap_rows_budgeted(
    text: &str,
    line_starts: &[usize],
    wrap_columns: usize,
    rows: &mut [usize],
    budget: Duration,
) {
    let line_count = line_starts.len().min(rows.len());
    if line_count == 0 {
        return;
    }

    let start = Instant::now();
    for (line_ix, row_slot) in rows.iter_mut().take(line_count).enumerate() {
        if line_ix > 0
            && line_ix % TEXT_INPUT_WRAP_BACKGROUND_YIELD_EVERY_ROWS == 0
            && start.elapsed() >= budget
        {
            break;
        }
        let line_text = line_text_for_index(text, line_starts, line_ix);
        *row_slot = estimate_wrap_rows_for_line(line_text, wrap_columns);
    }
}

#[inline]
pub(super) fn estimate_wrap_rows_for_line(line_text: &str, wrap_columns: usize) -> usize {
    if line_text.is_empty() {
        return 1;
    }
    let wrap_columns = wrap_columns.max(1);
    let bytes = line_text.as_bytes();

    // ASCII fast path: process segments between tabs in O(1) each
    // instead of iterating character by character.
    if line_text.is_ascii() {
        let tab_stop = TEXT_INPUT_WRAP_TAB_STOP_COLUMNS;
        let mut rows = 1usize;
        let mut column = 0usize;
        let mut pos = 0usize;

        if wrap_columns > tab_stop {
            for tab_pos in memchr::memchr_iter(b'\t', bytes) {
                let seg = tab_pos - pos;
                if seg > 0 {
                    advance_ascii_segment(&mut rows, &mut column, seg, wrap_columns);
                }
                advance_ascii_tab_common(&mut rows, &mut column, wrap_columns);
                pos = tab_pos + 1;
            }
        } else {
            for tab_pos in memchr::memchr_iter(b'\t', bytes) {
                let seg = tab_pos - pos;
                if seg > 0 {
                    advance_ascii_segment(&mut rows, &mut column, seg, wrap_columns);
                }
                advance_ascii_tab_general(&mut rows, &mut column, wrap_columns);
                pos = tab_pos + 1;
            }
        }

        let trailing = bytes.len() - pos;
        if trailing > 0 {
            advance_ascii_segment(&mut rows, &mut column, trailing, wrap_columns);
        }
        return rows.max(1);
    }

    // Non-ASCII fallback: character-by-character scan
    let mut rows = 1usize;
    let mut column = 0usize;
    for ch in line_text.chars() {
        let width = if ch == '\t' {
            let rem = column % TEXT_INPUT_WRAP_TAB_STOP_COLUMNS;
            if rem == 0 {
                TEXT_INPUT_WRAP_TAB_STOP_COLUMNS
            } else {
                TEXT_INPUT_WRAP_TAB_STOP_COLUMNS - rem
            }
        } else {
            1
        };

        if width >= wrap_columns {
            if column > 0 {
                rows = rows.saturating_add(1);
            }
            rows = rows.saturating_add(width / wrap_columns);
            column = width % wrap_columns;
            if column == 0 {
                column = wrap_columns;
            }
            continue;
        }

        if column + width > wrap_columns {
            rows = rows.saturating_add(1);
            column = width;
        } else {
            column += width;
        }
    }
    rows.max(1)
}

/// Advance column by `segment_len` ASCII characters (width 1 each),
/// updating rows and column for wraps. O(1) per segment.
#[inline]
pub(super) fn advance_ascii_segment(
    rows: &mut usize,
    column: &mut usize,
    segment_len: usize,
    wrap_columns: usize,
) {
    let remaining = wrap_columns - *column;
    if segment_len <= remaining {
        *column += segment_len;
    } else {
        let after = segment_len - remaining;
        *rows += 1 + after / wrap_columns;
        *column = after % wrap_columns;
    }
}

#[inline]
pub(super) fn advance_ascii_tab_common(rows: &mut usize, column: &mut usize, wrap_columns: usize) {
    debug_assert!(TEXT_INPUT_WRAP_TAB_STOP_COLUMNS.is_power_of_two());
    let tab_stop = TEXT_INPUT_WRAP_TAB_STOP_COLUMNS;
    let tab_width = tab_stop - (*column & (tab_stop - 1));
    if *column > wrap_columns - tab_width {
        *rows += 1;
        *column = tab_width;
    } else {
        *column += tab_width;
    }
}

#[inline]
pub(super) fn advance_ascii_tab_general(rows: &mut usize, column: &mut usize, wrap_columns: usize) {
    let tab_stop = TEXT_INPUT_WRAP_TAB_STOP_COLUMNS;
    let rem = *column % tab_stop;
    let tab_width = if rem == 0 { tab_stop } else { tab_stop - rem };
    if tab_width >= wrap_columns {
        if *column > 0 {
            *rows += 1;
        }
        *rows += tab_width / wrap_columns;
        *column = tab_width % wrap_columns;
        if *column == 0 {
            *column = wrap_columns;
        }
    } else if *column + tab_width > wrap_columns {
        *rows += 1;
        *column = tab_width;
    } else {
        *column += tab_width;
    }
}

pub(super) fn clamp_offset_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset = offset.saturating_sub(1);
    }
    offset
}

pub(super) fn expanded_dirty_wrap_line_range_for_edit(
    text: &str,
    line_starts: &[usize],
    old_range: &Range<usize>,
    new_range: &Range<usize>,
) -> Range<usize> {
    let line_count = line_starts.len().max(1);
    if line_count == 0 {
        return 0..0;
    }

    let mut start_offset = old_range.start.min(new_range.start).min(text.len());
    let mut end_offset = old_range.end.max(new_range.end).min(text.len());
    start_offset = clamp_offset_to_char_boundary(text, start_offset);
    end_offset = clamp_offset_to_char_boundary(text, end_offset.max(start_offset));

    let start_line = line_index_for_offset(line_starts, start_offset, line_count);
    let mut end_line = line_index_for_offset(line_starts, end_offset, line_count)
        .saturating_add(1)
        .min(line_count);
    if end_line <= start_line {
        end_line = (start_line + 1).min(line_count);
    }

    start_line.min(line_count)..end_line.min(line_count)
}

pub(super) fn apply_interpolated_wrap_patch_delta(
    rows: &mut [usize],
    patch: &InterpolatedWrapPatch,
) {
    for (ix, old_rows) in patch.old_rows.iter().copied().enumerate() {
        let Some(new_rows) = patch.new_rows.get(ix).copied() else {
            break;
        };
        let Some(slot) = rows.get_mut(patch.line_start.saturating_add(ix)) else {
            break;
        };
        let delta = new_rows as isize - old_rows as isize;
        let next = (*slot as isize + delta).max(1) as usize;
        *slot = next;
    }
}

pub(super) fn reset_interpolated_wrap_patches_on_overflow(
    interpolated_wrap_patches: &mut Vec<InterpolatedWrapPatch>,
    wrap_recompute_requested: &mut bool,
) -> bool {
    if interpolated_wrap_patches.len() < TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES {
        return false;
    }
    interpolated_wrap_patches.clear();
    *wrap_recompute_requested = true;
    true
}

pub(super) fn pending_wrap_job_accepts_interpolated_patch(
    pending_wrap_job: Option<&PendingWrapJob>,
    width_key: i32,
    line_count: usize,
    allow_interpolated_patches: bool,
) -> bool {
    allow_interpolated_patches
        && pending_wrap_job
            .map(|job| job.width_key == width_key && job.line_count == line_count)
            .unwrap_or(false)
}
