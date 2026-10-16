use super::shaping::*;
use super::state::*;
use super::wrap::*;
use super::*;
use crate::text_selection::is_word_char;
use gitcomet_text::rope::Rope;
use gpui::sum_tree::Bias;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

impl TextInput {
    /// Collapses the selection once another surface has taken the window's.
    /// Caret, focus, scroll, content and undo stacks are left alone.
    pub(super) fn clear_selection_on_ownership_loss(&mut self, cx: &mut Context<Self>) {
        if !self.selection_owner.is_stale(cx) || self.selection.range.is_empty() {
            return;
        }
        // Mid-composition the marked range *is* the highlight.
        if self.selection.marked_range.is_some() {
            return;
        }
        // Not `move_to`: it forces the caret visible, blinking a background input.
        let cursor = self.cursor_offset();
        self.selection.range = cursor..cursor;
        self.selection.reversed = false;
        self.end_mouse_drag();
        cx.notify();
    }

    pub fn has_selection_or_drag(&self) -> bool {
        !self.selection.range.is_empty() || self.interaction.is_selecting
    }

    /// Collapses the selection to a caret at `offset`.
    ///
    /// Needs no window, unlike [`Self::set_selected_range`]: a caret paints no
    /// highlight, so it never takes the window's selection.
    pub fn set_caret(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.move_to(offset, cx);
    }

    /// Installs a selection programmatically. Takes the window because a
    /// non-empty range is a real highlight and must own the window's selection.
    pub fn set_selected_range(
        &mut self,
        range: Range<usize>,
        autoscroll: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let start = self.clamp_to_char_boundary(range.start.min(range.end));
        let end = self.clamp_to_char_boundary(range.start.max(range.end));
        let next = start..end;
        if self.selection.range == next && !self.selection.reversed {
            if autoscroll {
                self.queue_cursor_autoscroll();
            }
            return;
        }
        // A programmatic highlight is still a highlight.
        if !next.is_empty() {
            self.selection_owner.adopt(window, cx);
        }

        self.selection.range = next;
        self.selection.reversed = false;
        self.interaction.vertical_motion_x = None;
        self.show_cursor();
        if autoscroll {
            self.queue_cursor_autoscroll();
        }
        cx.notify();
    }

    pub fn selected_text(&self) -> Option<String> {
        if self.selection.range.is_empty() {
            None
        } else {
            Some(self.content[self.selection.range.clone()].to_string())
        }
    }

    pub fn selected_range(&self) -> Range<usize> {
        self.selection.range.clone()
    }

    pub fn select_all_text(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), window, cx);
    }

    pub(super) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.range.is_empty() && self.cursor_offset() == 0 {
            self.interaction.arrow_left_at_start_pressed = true;
        }
        if self.selection.range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selection.range.start, cx)
        }
        self.queue_cursor_autoscroll();
    }

    pub(super) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.range.is_empty() && self.cursor_offset() >= self.content.len() {
            self.interaction.arrow_right_at_end_pressed = true;
        }
        if self.selection.range.is_empty() {
            self.move_to(self.next_boundary(self.selection.range.end), cx);
        } else {
            self.move_to(self.selection.range.end, cx)
        }
        self.queue_cursor_autoscroll();
    }

    pub(super) fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.range.is_empty() {
            self.move_to(self.previous_word_start(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selection.range.start, cx)
        }
        self.queue_cursor_autoscroll();
    }

    pub(super) fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.range.is_empty() {
            self.move_to(self.next_word_end(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selection.range.end, cx)
        }
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_left(
        &mut self,
        _: &SelectLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.previous_boundary(self.cursor_offset()), window, cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_right(
        &mut self,
        _: &SelectRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.next_boundary(self.cursor_offset()), window, cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.previous_word_start(self.cursor_offset()), window, cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.next_word_end(self.cursor_offset()), window, cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.interaction.arrow_up_pressed = true;
        if let Some((target, preferred_x)) = self.vertical_move_target(
            self.cursor_offset(),
            -1.0,
            self.interaction.vertical_motion_x,
        ) {
            self.move_to(target, cx);
            self.interaction.vertical_motion_x = Some(preferred_x);
            self.queue_cursor_autoscroll();
        } else {
            cx.notify();
        }
    }

    pub(super) fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.interaction.arrow_down_pressed = true;
        if let Some((target, preferred_x)) = self.vertical_move_target(
            self.cursor_offset(),
            1.0,
            self.interaction.vertical_motion_x,
        ) {
            self.move_to(target, cx);
            self.interaction.vertical_motion_x = Some(preferred_x);
            self.queue_cursor_autoscroll();
        } else {
            cx.notify();
        }
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) = self.vertical_move_target(
            self.cursor_offset(),
            -1.0,
            self.interaction.vertical_motion_x,
        ) else {
            return;
        };
        self.select_to(target, window, cx);
        self.interaction.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_down(
        &mut self,
        _: &SelectDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, preferred_x)) = self.vertical_move_target(
            self.cursor_offset(),
            1.0,
            self.interaction.vertical_motion_x,
        ) else {
            return;
        };
        self.select_to(target, window, cx);
        self.interaction.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_all(
        &mut self,
        _: &SelectAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_all_text(window, cx);
    }

    pub(super) fn row_start(&self, offset: usize) -> usize {
        self.row_boundaries(offset).0
    }

    pub(super) fn row_end(&self, offset: usize) -> usize {
        self.row_boundaries(offset).1
    }

    pub(super) fn logical_row_boundaries(&self, offset: usize) -> (usize, usize) {
        let content = self.content.snapshot();
        let row = content.line_range(content.row_for_offset(offset));
        (row.start, row.end)
    }

    pub(super) fn row_boundaries(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.content.len());
        if self.content.is_empty() {
            return (0, 0);
        }
        if !(self.multiline && self.soft_wrap) {
            return self.logical_row_boundaries(offset);
        }

        let Some(TextInputLayout::Wrapped { lines, .. }) = self.layout.last.as_ref() else {
            return self.logical_row_boundaries(offset);
        };
        let Some(starts) = self.layout.line_starts.as_ref() else {
            return self.logical_row_boundaries(offset);
        };
        let Some(line) = lines
            .get(starts.partition_point(|&s| s <= offset).saturating_sub(1))
            .or_else(|| lines.first())
        else {
            return self.logical_row_boundaries(offset);
        };

        let mut ix = starts.partition_point(|&s| s <= offset);
        if ix == 0 {
            ix = 1;
        }
        let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
        let line_start = starts.get(line_ix).copied().unwrap_or(0);
        let line = lines.get(line_ix).unwrap_or(line);
        let next_start = starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(self.content.len());
        if line.len() == 0 && next_start > line_start {
            return self.logical_row_boundaries(offset);
        }
        let local = offset.saturating_sub(line_start).min(line.len());

        let mut row_end_indices = line.row_end_indices();
        row_end_indices.sort_unstable();
        row_end_indices.dedup();
        row_end_indices.push(line.len());

        let row_ix = row_end_indices
            .iter()
            .position(|&end| local <= end)
            .unwrap_or_else(|| row_end_indices.len().saturating_sub(1));
        let row_start_local = if row_ix == 0 {
            0
        } else {
            row_end_indices[row_ix - 1]
        };
        let row_end_local = row_end_indices[row_ix];
        (
            (line_start + row_start_local).min(self.content.len()),
            (line_start + row_end_local).min(self.content.len()),
        )
    }

    pub(super) fn document_home(
        &mut self,
        _: &DocumentHome,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.interaction.document_home_pressed = true;
        self.move_to(0, cx);
        self.queue_cursor_autoscroll();
        cx.notify();
    }

    pub(super) fn document_end(&mut self, _: &DocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.interaction.document_end_pressed = true;
        self.move_to(self.content.len(), cx);
        self.queue_cursor_autoscroll();
        cx.notify();
    }

    pub(super) fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_start(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_home(
        &mut self,
        _: &SelectHome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.row_start(self.cursor_offset()), window, cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_end(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_end(
        &mut self,
        _: &SelectEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.row_end(self.cursor_offset()), window, cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn caret_point_for_hit_testing(&self, cursor: usize) -> Option<Point<Pixels>> {
        let bounds = self.current_text_bounds()?;
        let layout = self.layout.last.as_ref()?;
        let starts = self.layout.line_starts.as_ref()?;
        let line_height = if self.layout.line_height.is_zero() {
            px(16.0)
        } else {
            self.layout.line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let (line_ix, local_ix) = line_for_offset(starts, lines, cursor);
                let line = lines.get(line_ix)?;
                let x = line.x_for_index(local_ix) - self.layout.scroll_x;
                let y = line_height * line_ix as f32 + line_height / 2.0;
                Some(point(bounds.left() + x, bounds.top() + y))
            }
            TextInputLayout::TruncatedSingleLine(line) => Some(point(
                bounds.left() + truncated_line_x_for_source_offset(line, cursor),
                bounds.top() + line_height / 2.0,
            )),
            TextInputLayout::Wrapped {
                lines, y_offsets, ..
            } => {
                let mut ix = starts.partition_point(|&s| s <= cursor);
                if ix == 0 {
                    ix = 1;
                }
                let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
                let line = lines.get(line_ix)?;
                let start = starts.get(line_ix).copied().unwrap_or(0);
                let local = cursor.saturating_sub(start).min(line.len());
                let pos = line
                    .position_for_index(local, line_height)
                    .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                let y = y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO)
                    + pos.y
                    + line_height / 2.0;
                Some(point(bounds.left() + pos.x, bounds.top() + y))
            }
        }
    }

    pub(super) fn vertical_move_target(
        &self,
        cursor: usize,
        direction: f32,
        preferred_x: Option<Pixels>,
    ) -> Option<(usize, Pixels)> {
        let line_height = if self.layout.line_height.is_zero() {
            px(16.0)
        } else {
            self.layout.line_height
        };
        let caret_point = self.caret_point_for_hit_testing(cursor)?;
        let preferred_x = preferred_x.unwrap_or(caret_point.x);
        let target = point(preferred_x, caret_point.y + line_height * direction);
        Some((self.index_for_position(target), preferred_x))
    }

    pub(super) fn page_move_target(
        &self,
        cursor: usize,
        direction: f32,
        preferred_x: Option<Pixels>,
    ) -> Option<(usize, Pixels)> {
        let bounds = self.current_text_bounds()?;
        let line_height = if self.layout.line_height.is_zero() {
            px(16.0)
        } else {
            self.layout.line_height
        };
        let page_height = bounds.size.height.max(line_height);
        let caret_point = self.caret_point_for_hit_testing(cursor)?;
        let preferred_x = preferred_x.unwrap_or(caret_point.x);
        let target = point(preferred_x, caret_point.y + page_height * direction);
        Some((self.index_for_position(target), preferred_x))
    }

    /// The caret's x inside the input's own content, from the layout of the
    /// frame that last painted it.
    ///
    /// A multiline input does not scroll itself sideways — the container it sits
    /// in does — so the caret's place along its line is the only thing that
    /// container can steer by. `None` while soft wrap is on, where there is no
    /// horizontal overflow to reveal into, and before the first layout.
    pub fn cursor_content_x(&self, cursor: usize) -> Option<Pixels> {
        let layout = self.layout.last.as_ref()?;
        let starts = self.layout.line_starts.as_ref()?;
        match layout {
            TextInputLayout::Plain(lines) => {
                let (line_ix, local_ix) = line_for_offset(starts, lines, cursor);
                lines.get(line_ix).map(|line| line.x_for_index(local_ix))
            }
            TextInputLayout::TruncatedSingleLine(line) => {
                Some(truncated_line_x_for_source_offset(line, cursor))
            }
            TextInputLayout::Wrapped { .. } => None,
        }
    }

    pub(super) fn cursor_vertical_span(&self, cursor: usize) -> Option<(Pixels, Pixels)> {
        let layout = self.layout.last.as_ref()?;
        let starts = self.layout.line_starts.as_ref()?;
        let line_height = if self.layout.line_height.is_zero() {
            px(16.0)
        } else {
            self.layout.line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let (line_ix, _) = line_for_offset(starts, lines, cursor);
                let top = line_height * line_ix as f32;
                let bottom = top + line_height;
                Some((top, bottom))
            }
            TextInputLayout::TruncatedSingleLine(_) => Some((Pixels::ZERO, line_height)),
            TextInputLayout::Wrapped {
                lines, y_offsets, ..
            } => {
                let mut ix = starts.partition_point(|&s| s <= cursor);
                if ix == 0 {
                    ix = 1;
                }
                let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
                let line = lines.get(line_ix)?;
                let start = starts.get(line_ix).copied().unwrap_or(0);
                let local = cursor.saturating_sub(start).min(line.len());
                let pos = line
                    .position_for_index(local, line_height)
                    .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                let top = y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO) + pos.y;
                let bottom = top + line_height;
                Some((top, bottom))
            }
        }
    }

    pub(super) fn ensure_cursor_visible_in_vertical_scroll(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.interaction.vertical_scroll_handle.clone() else {
            self.interaction.pending_cursor_autoscroll = false;
            return;
        };
        let Some(text_bounds) = self.current_text_bounds() else {
            return;
        };
        let viewport_height = handle.bounds().size.height.max(px(0.0));
        if viewport_height <= px(0.0) {
            return;
        }
        // Wrapping is measured during prepaint, after the parent's scroll
        // extent was laid out. Wait for that height instead of scrolling
        // against a temporary, shorter document and correcting it next frame.
        let waiting_for_layout = self.multiline
            && self.soft_wrap
            && self.wrap.cache.is_some_and(|cache| {
                (self.layout.line_height * cache.rows as f32 - text_bounds.size.height).abs()
                    > px(1.0)
            })
            && self.interaction.cursor_autoscroll_layout_waits_remaining > 0;
        let caret_margin =
            px(10.0).min(((viewport_height - self.layout.line_height) / 2.0).max(px(0.0)));

        let Some((cursor_top, cursor_bottom)) = self.cursor_vertical_span(self.cursor_offset())
        else {
            return;
        };

        let current = handle.offset();
        let viewport_top = handle.bounds().top();
        let child_top = viewport_top + current.y;
        let text_origin_in_child = text_bounds.top() - child_top;
        let cursor_top = text_origin_in_child + cursor_top;
        let cursor_bottom = text_origin_in_child + cursor_bottom;
        let max_offset = handle.max_offset().y.max(px(0.0));
        let scroll_y = (-current.y).clamp(px(0.0), max_offset);
        let target_scroll = if waiting_for_layout {
            scroll_y
        } else if cursor_top < scroll_y + caret_margin {
            cursor_top - caret_margin
        } else if cursor_bottom > scroll_y + viewport_height - caret_margin {
            cursor_bottom - viewport_height + caret_margin
        } else {
            scroll_y
        }
        .max(px(0.0))
        .min(max_offset);

        let moved = current.y != -target_scroll;
        if moved {
            handle.set_offset(point(current.x, -target_scroll));
            self.interaction.cursor_autoscroll_layout_waits_remaining =
                TEXT_INPUT_CURSOR_AUTOSCROLL_RETRIES;
        }
        // Scrolling reveals lines that may still have estimated wrap counts.
        // Keep the reveal alive through their shaping and the following layout,
        // even if the estimated caret position fits now (notably Ctrl+End).
        // A fixed retry budget prevents notifications from continuing forever.
        let cursor_will_be_visible =
            cursor_top >= target_scroll && cursor_bottom <= target_scroll + viewport_height;
        if waiting_for_layout {
            // Waiting for the parent consumes a separate allowance: otherwise
            // a slow layout can use every attempt before we reach the target.
            self.interaction.cursor_autoscroll_layout_waits_remaining -= 1;
            self.interaction.pending_cursor_autoscroll = true;
        } else if ((self.soft_wrap && moved) || !cursor_will_be_visible)
            && self.interaction.cursor_autoscroll_retries_remaining > 0
        {
            self.interaction.pending_cursor_autoscroll = true;
            self.interaction.cursor_autoscroll_retries_remaining -= 1;
        } else {
            self.interaction.pending_cursor_autoscroll = false;
        }
        if moved || self.interaction.pending_cursor_autoscroll {
            cx.notify();
        }
    }

    pub(super) fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.interaction.page_up_pressed = true;
        cx.notify();
        let Some((target, preferred_x)) = self.page_move_target(
            self.cursor_offset(),
            -1.0,
            self.interaction.vertical_motion_x,
        ) else {
            return;
        };
        self.move_to(target, cx);
        self.interaction.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_page_up(
        &mut self,
        _: &SelectPageUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, preferred_x)) = self.page_move_target(
            self.cursor_offset(),
            -1.0,
            self.interaction.vertical_motion_x,
        ) else {
            return;
        };
        self.select_to(target, window, cx);
        self.interaction.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.interaction.page_down_pressed = true;
        cx.notify();
        let Some((target, preferred_x)) = self.page_move_target(
            self.cursor_offset(),
            1.0,
            self.interaction.vertical_motion_x,
        ) else {
            return;
        };
        self.move_to(target, cx);
        self.interaction.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_page_down(
        &mut self,
        _: &SelectPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, preferred_x)) = self.page_move_target(
            self.cursor_offset(),
            1.0,
            self.interaction.vertical_motion_x,
        ) else {
            return;
        };
        self.select_to(target, window, cx);
        self.interaction.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    pub fn cursor_offset(&self) -> usize {
        if self.selection.reversed {
            self.selection.range.start
        } else {
            self.selection.range.end
        }
    }

    pub fn set_cursor_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.move_to(offset, cx);
        self.queue_cursor_autoscroll();
    }

    pub fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        self.index_for_position(position)
    }

    pub fn hotspot_range_index_at_position(
        &self,
        position: Point<Pixels>,
        hotspot_ranges: &[Range<usize>],
    ) -> Option<usize> {
        let offset = self.index_for_mouse_position(position);
        hotspot_ranges.iter().enumerate().find_map(|(ix, range)| {
            (self.valid_hotspot_range(range)
                && self.position_inside_hotspot(range, position, offset))
            .then_some(ix)
        })
    }

    /// The box a hotspot occupies, in window coordinates, for anchoring a menu
    /// to the run of text it acts on.
    ///
    /// A hotspot that wraps has its end on a later visual line, where the end x
    /// says nothing about the run's extent and can even sit left of its start.
    /// Those report the first visual line only, running to the right edge of the
    /// input, so the box always describes where the hotspot begins.
    pub fn hotspot_bounds(&self, range: &Range<usize>) -> Option<Bounds<Pixels>> {
        if !self.valid_hotspot_range(range) {
            return None;
        }

        let line_height = if self.layout.line_height.is_zero() {
            px(16.0)
        } else {
            self.layout.line_height
        };
        let start = self.hotspot_position(range.start)?;
        let end = self.hotspot_position(range.end)?;
        let end = if end.y > start.y {
            point(self.current_text_bounds()?.right(), start.y)
        } else {
            end
        };
        Some(Bounds::from_corners(
            start,
            point(end.x, end.y + line_height),
        ))
    }

    pub(super) fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = self.clamp_to_char_boundary(offset);
        self.selection.range = offset..offset;
        self.selection.reversed = false;
        self.interaction.vertical_motion_x = None;
        self.show_cursor();
        cx.notify();
    }

    /// Extends the selection and takes the window's, for gestures that leave a
    /// highlight behind. See [`Self::extend_selection_to`] for the bare move.
    pub(super) fn select_to(&mut self, offset: usize, window: &Window, cx: &mut Context<Self>) {
        self.extend_selection_to(offset, cx);
        // Only a real highlight takes the window's selection: Ctrl+F select-alls
        // an empty search box, and that must not wipe a diff-text selection.
        if !self.selection.range.is_empty() {
            self.selection_owner.adopt(window, cx);
        }
    }

    /// Moves the selection head without taking the window's selection. What the
    /// delete helpers want: they build a range only to feed
    /// `replace_text_in_range`, and paint no highlight.
    pub(super) fn extend_selection_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = self.clamp_to_char_boundary(offset);
        if self.selection.reversed {
            self.selection.range.start = offset;
        } else {
            self.selection.range.end = offset;
        }
        if self.selection.range.end < self.selection.range.start {
            self.selection.reversed = !self.selection.reversed;
            self.selection.range = self.selection.range.end..self.selection.range.start;
        }
        self.interaction.vertical_motion_x = None;
        self.show_cursor();
        cx.notify();
    }

    pub(super) fn clamp_to_char_boundary(&self, offset: usize) -> usize {
        self.content.clamp_to_char_boundary(offset)
    }

    /// The grapheme boundary before `offset`. The segmenter reads the rope's
    /// chunks back from the caret only as far as the cluster needs, so a step
    /// costs the same however long the row is.
    pub(super) fn previous_boundary(&self, offset: usize) -> usize {
        let rope = self.content.snapshot().rope();
        // Inside a character, step from its end: no boundary lies between.
        let offset = rope.clip_offset(offset, Bias::Right);
        let mut cursor = GraphemeCursor::new(offset, rope.len(), true);
        let mut chunk_end = offset;
        for chunk in rope.reversed_chunks_in_range(0..offset) {
            let chunk_start = chunk_end - chunk.len();
            loop {
                match cursor.prev_boundary(chunk, chunk_start) {
                    Ok(boundary) => return boundary.unwrap_or(0),
                    Err(GraphemeIncomplete::PreContext(end)) => {
                        provide_grapheme_context(&mut cursor, &rope, end)
                    }
                    Err(GraphemeIncomplete::PrevChunk) => break,
                    Err(GraphemeIncomplete::NextChunk | GraphemeIncomplete::InvalidOffset) => {
                        unreachable!("the chunk ends at or after the grapheme cursor")
                    }
                }
            }
            chunk_end = chunk_start;
        }
        0
    }

    /// The grapheme boundary after `offset`, read forward the same way.
    pub(super) fn next_boundary(&self, offset: usize) -> usize {
        let rope = self.content.snapshot().rope();
        let len = rope.len();
        // Inside a character, step from its start: no boundary lies between.
        let offset = rope.clip_offset(offset, Bias::Left);
        let mut cursor = GraphemeCursor::new(offset, len, true);
        let mut chunk_start = offset;
        for chunk in rope.chunks_in_range(offset..len) {
            loop {
                match cursor.next_boundary(chunk, chunk_start) {
                    Ok(boundary) => return boundary.unwrap_or(len),
                    Err(GraphemeIncomplete::PreContext(end)) => {
                        provide_grapheme_context(&mut cursor, &rope, end)
                    }
                    Err(GraphemeIncomplete::NextChunk) => break,
                    Err(GraphemeIncomplete::PrevChunk | GraphemeIncomplete::InvalidOffset) => {
                        unreachable!("the chunk starts at or before the grapheme cursor")
                    }
                }
            }
            chunk_start += chunk.len();
        }
        len
    }

    /// Walks left over characters matching `predicate`, reading only the ones
    /// it passes; a whitespace run crosses rows, since a newline is whitespace.
    fn skip_left_while(
        content: &TextModelSnapshot,
        offset: usize,
        mut predicate: impl FnMut(char) -> bool,
    ) -> usize {
        let skipped: usize = content
            .reversed_chars_at(offset)
            .take_while(|ch| predicate(*ch))
            .map(char::len_utf8)
            .sum();
        offset - skipped
    }

    /// Walks right over characters matching `predicate`, the same way.
    fn skip_right_while(
        content: &TextModelSnapshot,
        offset: usize,
        mut predicate: impl FnMut(char) -> bool,
    ) -> usize {
        let skipped: usize = content
            .chars_at(offset)
            .take_while(|ch| predicate(*ch))
            .map(char::len_utf8)
            .sum();
        offset + skipped
    }

    pub(super) fn previous_word_start(&self, offset: usize) -> usize {
        let content = self.content.snapshot();
        let mut offset = self.clamp_to_char_boundary(offset);

        // Skip any whitespace to the left of the cursor.
        offset = Self::skip_left_while(&content, offset, |ch| ch.is_whitespace());

        // Skip punctuation/symbols (e.g. '.' '/' '-') so word navigation doesn't get stuck on them.
        offset = Self::skip_left_while(&content, offset, |ch| {
            !ch.is_whitespace() && !is_word_char(ch)
        });

        // Skip any whitespace again, then skip the word itself.
        offset = Self::skip_left_while(&content, offset, |ch| ch.is_whitespace());
        Self::skip_left_while(&content, offset, is_word_char)
    }

    pub(super) fn next_word_end(&self, offset: usize) -> usize {
        let content = self.content.snapshot();
        let offset = self.clamp_to_char_boundary(offset);
        let Some(ch) = content.chars_at(offset).next() else {
            return content.len();
        };

        if ch.is_whitespace() {
            return Self::skip_right_while(&content, offset, |ch| ch.is_whitespace());
        }
        if is_word_char(ch) {
            return Self::skip_right_while(&content, offset, is_word_char);
        }

        Self::skip_right_while(&content, offset, |ch| {
            !ch.is_whitespace() && !is_word_char(ch)
        })
    }

    pub(super) fn token_range_for_offset(&self, offset: usize) -> Range<usize> {
        crate::text_selection::token_range_for_offset(self.content.as_ref(), offset)
    }

    fn valid_hotspot_range(&self, range: &Range<usize>) -> bool {
        range.start < range.end && range.end <= self.content.len()
    }

    fn offset_inside_hotspot(range: &Range<usize>, offset: usize) -> bool {
        offset >= range.start && offset < range.end
    }

    fn wrapped_line_for_offset(
        starts: &[usize],
        lines: &[EditorLine],
        offset: usize,
    ) -> (usize, usize) {
        let mut ix = starts.partition_point(|&s| s <= offset);
        if ix == 0 {
            ix = 1;
        }
        let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
        let start = starts.get(line_ix).copied().unwrap_or(0);
        let local = offset.saturating_sub(start).min(lines[line_ix].len());
        (line_ix, local)
    }

    fn position_inside_hotspot(
        &self,
        range: &Range<usize>,
        position: Point<Pixels>,
        offset: usize,
    ) -> bool {
        if !self
            .layout
            .bounds
            .as_ref()
            .is_some_and(|bounds| bounds.contains(&position))
        {
            return false;
        }

        if Self::offset_inside_hotspot(range, offset) {
            return true;
        }

        offset == range.end && self.position_inside_hotspot_final_glyph(range, position)
    }

    fn position_inside_hotspot_final_glyph(
        &self,
        range: &Range<usize>,
        position: Point<Pixels>,
    ) -> bool {
        let (Some(bounds), Some(layout), Some(starts)) = (
            self.current_text_bounds(),
            self.layout.last.as_ref(),
            self.layout.line_starts.as_ref(),
        ) else {
            return false;
        };
        if !bounds.contains(&position) {
            return false;
        }

        let final_glyph_start = self.previous_boundary(range.end);
        if final_glyph_start < range.start {
            return false;
        }

        let line_height = if self.layout.line_height.is_zero() {
            px(16.0)
        } else {
            self.layout.line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let (start_line_ix, start_local_ix) =
                    line_for_offset(starts.as_ref(), lines, final_glyph_start);
                let (end_line_ix, end_local_ix) =
                    line_for_offset(starts.as_ref(), lines, range.end);
                if start_line_ix != end_line_ix {
                    return false;
                }

                let row_top = bounds.top() + line_height * end_line_ix as f32;
                if position.y < row_top || position.y > row_top + line_height {
                    return false;
                }

                let Some(line) = lines.get(end_line_ix) else {
                    return false;
                };
                let x0 = bounds.left() + line.x_for_index(start_local_ix) - self.layout.scroll_x;
                let x1 = bounds.left() + line.x_for_index(end_local_ix) - self.layout.scroll_x;
                position.x >= x0.min(x1) && position.x <= x0.max(x1)
            }
            TextInputLayout::TruncatedSingleLine(line) => {
                let row_bottom = bounds.top() + line_height;
                if position.y < bounds.top() || position.y > row_bottom {
                    return false;
                }

                let x0 =
                    bounds.left() + truncated_line_x_for_source_offset(line, final_glyph_start);
                let x1 = bounds.left() + truncated_line_x_for_source_offset(line, range.end);
                position.x >= x0.min(x1) && position.x <= x0.max(x1)
            }
            TextInputLayout::Wrapped {
                lines, y_offsets, ..
            } => {
                let (start_line_ix, start_local_ix) =
                    Self::wrapped_line_for_offset(starts.as_ref(), lines, final_glyph_start);
                let (end_line_ix, end_local_ix) =
                    Self::wrapped_line_for_offset(starts.as_ref(), lines, range.end);
                if start_line_ix != end_line_ix {
                    return false;
                }

                let Some(line) = lines.get(end_line_ix) else {
                    return false;
                };
                let Some(start_pos) = line.position_for_index(start_local_ix, line_height) else {
                    return false;
                };
                let Some(end_pos) = line.position_for_index(end_local_ix, line_height) else {
                    return false;
                };
                if start_pos.y != end_pos.y {
                    return false;
                }

                let row_top = bounds.top()
                    + y_offsets.get(end_line_ix).copied().unwrap_or(Pixels::ZERO)
                    + end_pos.y;
                if position.y < row_top || position.y > row_top + line_height {
                    return false;
                }

                let x0 = bounds.left() + start_pos.x;
                let x1 = bounds.left() + end_pos.x;
                position.x >= x0.min(x1) && position.x <= x0.max(x1)
            }
        }
    }

    fn hotspot_position(&self, offset: usize) -> Option<Point<Pixels>> {
        let bounds = self.current_text_bounds()?;
        let layout = self.layout.last.as_ref()?;
        let starts = self.layout.line_starts.as_ref()?;
        let offset = self.clamp_to_char_boundary(offset.min(self.content.len()));
        let line_height = if self.layout.line_height.is_zero() {
            px(16.0)
        } else {
            self.layout.line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let (line_ix, local_ix) = line_for_offset(starts.as_ref(), lines, offset);
                let line = lines.get(line_ix)?;
                Some(point(
                    bounds.left() + line.x_for_index(local_ix) - self.layout.scroll_x,
                    bounds.top() + line_height * line_ix as f32,
                ))
            }
            TextInputLayout::TruncatedSingleLine(line) => Some(point(
                bounds.left() + truncated_line_x_for_source_offset(line, offset),
                bounds.top(),
            )),
            TextInputLayout::Wrapped {
                lines, y_offsets, ..
            } => {
                let (line_ix, local_ix) =
                    Self::wrapped_line_for_offset(starts.as_ref(), lines, offset);
                let line = lines.get(line_ix)?;
                let pos = line.position_for_index(local_ix, line_height)?;
                Some(point(
                    bounds.left() + pos.x,
                    bounds.top() + y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO) + pos.y,
                ))
            }
        }
    }

    pub(super) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.interaction.context_menu.take().is_some() {
            cx.notify();
        }
        // The press belongs to this input for as long as the button is held,
        // even once the pointer leaves it — nobody else may read its release
        // as a click. Claimed unconditionally, because a double-click that
        // turns into a drag never sets `is_selecting`.
        crate::press_gesture::claim_press(cx);
        // Unconditional, for the same reason as the claim above.
        self.selection_owner.adopt(window, cx);
        self.interaction.took_press = true;
        cx.stop_propagation();
        window.focus(&self.focus_handle, cx);
        self.show_cursor();
        let index = self.try_index_for_mouse_position(event.position);
        self.interaction.vertical_motion_x = None;

        if event.modifiers.shift {
            self.interaction.is_selecting = true;
            self.interaction.mouse_selection_anchor = Some(if self.selection.reversed {
                self.selection.range.end
            } else {
                self.selection.range.start
            });
            self.interaction.pending_mouse_selection_anchor = None;
            self.interaction.drag_pointer = Some(event.position);
            self.interaction.drag_resolve_pending = index.is_none();
            self.start_drag_autoscroll(cx);
            if let Some(index) = index {
                self.select_mouse_to_index(index, cx);
            }
            return;
        }

        if event.click_count >= 2 {
            self.end_mouse_drag();
            let index = index.unwrap_or_else(|| self.cursor_offset());
            let range = self.token_range_for_offset(index);
            if range.is_empty() {
                self.move_to(index, cx);
            } else {
                self.selection.range = range;
                self.selection.reversed = false;
                cx.notify();
            }
        } else {
            self.interaction.is_selecting = true;
            self.interaction.mouse_selection_anchor = index;
            self.interaction.pending_mouse_selection_anchor =
                index.is_none().then_some(event.position);
            self.interaction.drag_pointer = Some(event.position);
            self.interaction.drag_resolve_pending = index.is_none();
            self.start_drag_autoscroll(cx);
            match index {
                Some(index) => self.move_to(index, cx),
                // Clear any old highlight without inventing byte zero as the
                // new anchor. The initiating pointer position is resolved by
                // the first move after the next layout has painted.
                None => self.move_to(self.cursor_offset(), cx),
            }
        }
    }

    pub(super) fn on_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.end_mouse_drag();
    }

    pub(super) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag_mouse_moved(event, cx);
    }

    pub(super) fn on_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();

        if event.keystroke.modifiers.modified() {
            return;
        }

        if key == "escape" {
            self.interaction.escape_pressed = true;
            cx.notify();
            return;
        }

        let shift = event.keystroke.modifiers.shift;

        if key == "up" {
            // Intentionally duplicated with the Up action handler (up()).
            // This key_down path is a fallback: on some platforms (e.g.
            // IME composition on Wayland) action dispatch may be suppressed.
            self.interaction.arrow_up_pressed = true;
            cx.notify();
            return;
        }

        if key == "down" {
            // Intentionally duplicated with the Down action handler (down()).
            self.interaction.arrow_down_pressed = true;
            cx.notify();
            return;
        }

        if key == "tab" {
            if shift {
                self.interaction.shift_tab_pressed = true;
            } else {
                self.interaction.tab_pressed = true;
            }
            cx.notify();
        }
    }

    pub(super) fn try_index_for_mouse_position(&self, position: Point<Pixels>) -> Option<usize> {
        self.index_for_mouse_position_in(position, false)
    }

    /// With `require_shaped`, a row the last frame did not shape answers `None`
    /// instead of its start offset.
    pub(super) fn index_for_mouse_position_in(
        &self,
        position: Point<Pixels>,
        require_shaped: bool,
    ) -> Option<usize> {
        if self.content.is_empty() {
            return Some(0);
        }

        let (Some(bounds), Some(layout), Some(starts)) = (
            self.current_text_bounds(),
            self.layout.last.as_ref(),
            self.layout.line_starts.as_ref(),
        ) else {
            return None;
        };

        if position.y < bounds.top() {
            return Some(0);
        }
        if position.y > bounds.bottom() {
            return Some(self.content.len());
        }

        let line_height = if self.layout.line_height.is_zero() {
            px(16.0)
        } else {
            self.layout.line_height
        };

        Some(match layout {
            TextInputLayout::Plain(lines) => {
                let ratio = f32::from(position.y - bounds.top()) / f32::from(line_height);
                let mut line_ix = ratio.floor() as isize;
                line_ix = line_ix.clamp(0, lines.line_count().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let local_x = position.x - bounds.left() + self.layout.scroll_x;
                // A row that was never shaped was never on screen to be hit;
                // fall back to its start offset.
                let line = lines.get(line_ix);
                if require_shaped && line.is_none() {
                    return None;
                }
                let local_ix = line
                    .map(|line| line.closest_index_for_x(local_x))
                    .unwrap_or(0);
                let doc_ix = starts.get(line_ix).copied().unwrap_or(0) + local_ix;
                doc_ix.min(self.content.len())
            }
            TextInputLayout::TruncatedSingleLine(line) => {
                let local_x = position.x - bounds.left();
                truncated_line_source_offset_for_x(line, local_x).min(self.content.len())
            }
            TextInputLayout::Wrapped {
                lines,
                y_offsets,
                row_counts,
            } => {
                let local_y = position.y - bounds.top();
                let line_ix = wrapped_line_index_for_y(y_offsets, row_counts, line_height, local_y);
                let line_ix = line_ix.min(lines.len().saturating_sub(1));
                if require_shaped && !self.layout.painted_line_range.contains(&line_ix) {
                    return None;
                }
                let local_x = position.x - bounds.left();
                let local_y_in_line =
                    local_y - y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO);
                let line = &lines[line_ix];
                let local = line
                    .closest_index_for_position(point(local_x, local_y_in_line), line_height)
                    .unwrap_or_else(|ix| ix);
                let doc_ix = starts.get(line_ix).copied().unwrap_or(0) + local;
                doc_ix.min(self.content.len())
            }
        })
    }

    pub(super) fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        // Public hit-test callers need a total answer. Mouse drag initiation
        // uses the optional form above so a briefly invalid layout never turns
        // into a bogus byte-zero anchor.
        self.try_index_for_mouse_position(position)
            .unwrap_or_else(|| self.cursor_offset())
    }

    pub(super) fn select_mouse_to_index(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(anchor) = self.interaction.mouse_selection_anchor else {
            return;
        };
        let anchor = self.clamp_to_char_boundary(anchor);
        let index = self.clamp_to_char_boundary(index);
        let next = anchor.min(index)..anchor.max(index);
        let reversed = index < anchor;
        if self.selection.range == next && self.selection.reversed == reversed {
            return;
        }
        self.selection.range = next;
        self.selection.reversed = reversed;
        self.interaction.vertical_motion_x = None;
        self.show_cursor();
        cx.notify();
    }

    pub(super) fn index_for_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(layout), Some(starts)) = (
            self.current_text_bounds(),
            self.layout.last.as_ref(),
            self.layout.line_starts.as_ref(),
        ) else {
            return 0;
        };

        let line_height = if self.layout.line_height.is_zero() {
            px(16.0)
        } else {
            self.layout.line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let ratio = f32::from(position.y - bounds.top()) / f32::from(line_height);
                let mut line_ix = ratio.floor() as isize;
                line_ix = line_ix.clamp(0, lines.line_count().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let local_x = position.x - bounds.left() + self.layout.scroll_x;
                // A row that was never shaped was never on screen to be hit;
                // fall back to its start offset.
                let local_ix = lines
                    .get(line_ix)
                    .map(|line| line.closest_index_for_x(local_x))
                    .unwrap_or(0);
                let doc_ix = starts.get(line_ix).copied().unwrap_or(0) + local_ix;
                doc_ix.min(self.content.len())
            }
            TextInputLayout::TruncatedSingleLine(line) => {
                let local_x = position.x - bounds.left();
                truncated_line_source_offset_for_x(line, local_x).min(self.content.len())
            }
            TextInputLayout::Wrapped {
                lines,
                y_offsets,
                row_counts,
            } => {
                let local_y = position.y - bounds.top();
                let line_ix = wrapped_line_index_for_y(y_offsets, row_counts, line_height, local_y);
                let line_ix = line_ix.min(lines.len().saturating_sub(1));
                let local_x = position.x - bounds.left();
                let local_y_in_line =
                    local_y - y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO);
                let line = &lines[line_ix];
                let local = line
                    .closest_index_for_position(point(local_x, local_y_in_line), line_height)
                    .unwrap_or_else(|ix| ix);
                let doc_ix = starts.get(line_ix).copied().unwrap_or(0) + local;
                doc_ix.min(self.content.len())
            }
        }
    }
}

/// Hands `cursor` the text ending at `end`, which it asks for when its own
/// chunk cannot settle a boundary: a flag pair, ZWJ sequence or Indic conjunct
/// that started in an earlier chunk.
fn provide_grapheme_context(cursor: &mut GraphemeCursor, rope: &Rope, end: usize) {
    let Some(context) = rope.reversed_chunks_in_range(0..end).next() else {
        unreachable!("pre-context is only requested after a nonzero chunk start");
    };
    cursor.provide_context(context, end - context.len());
}
