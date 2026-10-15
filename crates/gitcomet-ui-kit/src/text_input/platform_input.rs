use super::editing::EditSelection;
use super::shaping::*;
use super::state::*;
use super::wrap::*;
use super::*;

impl TextInput {
    /// The platform input handler addresses the buffer in UTF-16, so these two
    /// run on essentially every caret query. They used to walk the document
    /// char by char from offset zero — and reading `self.content` as a `&str`
    /// materialized the whole buffer to do it, so a single arrow key in a large
    /// document cost two full passes over it. The model answers from its own
    /// structure instead.
    pub(super) fn offset_from_utf16(&self, offset: usize) -> usize {
        self.content.snapshot().offset_from_utf16(offset)
    }

    pub(super) fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content.snapshot().offset_to_utf16(offset)
    }

    pub(super) fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub(super) fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        // Normalize at the platform boundary, before row caches, highlight
        // deltas and the text model can interpret the same IME edit differently.
        self.normalized_utf8_range(
            self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end),
        )
    }
}

impl EntityInputHandler for TextInput {
    fn accepts_text_input(&self, window: &mut Window, _cx: &mut Context<Self>) -> bool {
        crate::window_focus::is_active(&self.focus_handle, window) && !self.read_only
    }

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection.range),
            reversed: self.selection.reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.selection
            .marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.selection.marked_range = None;
        // The observer skips composing inputs and only re-runs on the next
        // global write, so a selection that went stale mid-IME would stay lit.
        self.clear_selection_on_ownership_loss(cx);
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_text_input(window, cx) {
            return;
        }
        let Some(new_text) = self.sanitize_insert_text(new_text) else {
            return;
        };
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.selection.marked_range.clone())
            .unwrap_or(self.selection.range.clone());
        if self.edit_alters_protected_range(&range, new_text.as_str()) {
            return;
        }

        self.apply_edit(range, &new_text, EditSelection::AfterInsertion, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_text_input(window, cx) {
            return;
        }
        let Some(new_text) = self.sanitize_insert_text(new_text) else {
            return;
        };
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.selection.marked_range.clone())
            .unwrap_or(self.selection.range.clone());
        if self.edit_alters_protected_range(&range, new_text.as_str()) {
            return;
        }

        self.apply_edit(
            range,
            &new_text,
            EditSelection::Composition(new_selected_range_utf16),
            cx,
        );
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.last.as_ref()?;
        let starts = self.layout.line_starts.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let offset = range.start.min(self.content.len());
        let line_height = self.effective_line_height(window);

        let (line_ix, local_ix, y_offset) = match layout {
            TextInputLayout::Plain(lines) => {
                let (line_ix, local_ix) = line_for_offset(starts, lines, offset);
                (line_ix, local_ix, line_height * line_ix as f32)
            }
            TextInputLayout::TruncatedSingleLine(_) => (0, 0, Pixels::ZERO),
            TextInputLayout::Wrapped {
                lines, y_offsets, ..
            } => {
                let mut ix = starts.partition_point(|&s| s <= offset);
                if ix == 0 {
                    ix = 1;
                }
                let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
                let start = starts.get(line_ix).copied().unwrap_or(0);
                let local = offset.saturating_sub(start).min(lines[line_ix].len());
                (
                    line_ix,
                    local,
                    y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO),
                )
            }
        };

        let (x, y) = match layout {
            TextInputLayout::Plain(lines) => {
                let line = lines.get(line_ix)?;
                (line.x_for_index(local_ix) - self.layout.scroll_x, y_offset)
            }
            TextInputLayout::TruncatedSingleLine(line) => (
                truncated_line_x_for_source_offset(line, offset),
                Pixels::ZERO,
            ),
            TextInputLayout::Wrapped { lines, .. } => {
                let line = lines.get(line_ix)?;
                let p = line
                    .position_for_index(local_ix, line_height)
                    .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                (p.x, y_offset + p.y)
            }
        };

        let top = bounds.top() + y;
        Some(Bounds::from_corners(
            point(bounds.left() + x, top),
            point(bounds.left() + x + px(2.0), top + px(16.0)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.current_text_bounds()?.localize(&p)?;
        let layout = self.layout.last.as_ref()?;
        let starts = self.layout.line_starts.as_ref()?;
        let line_height = self.effective_line_height(window);
        match layout {
            TextInputLayout::Plain(lines) => {
                let mut line_ix = (local.y / line_height).floor() as isize;
                line_ix = line_ix.clamp(0, lines.line_count().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let line = lines.get(line_ix)?;
                let local_x = local.x + self.layout.scroll_x;
                let idx = line.index_for_x(local_x).unwrap_or(line.len());
                let doc_offset = starts.get(line_ix).copied().unwrap_or(0) + idx;
                Some(self.offset_to_utf16(doc_offset))
            }
            TextInputLayout::TruncatedSingleLine(line) => Some(self.offset_to_utf16(
                truncated_line_source_offset_for_x(line, local.x).min(self.content.len()),
            )),
            TextInputLayout::Wrapped {
                lines,
                y_offsets,
                row_counts,
            } => {
                let line_ix = wrapped_line_index_for_y(y_offsets, row_counts, line_height, local.y);
                let line_ix = line_ix.min(lines.len().saturating_sub(1));
                let line = lines.get(line_ix)?;
                let local_y = local.y - y_offsets.get(line_ix).copied().unwrap_or(Pixels::ZERO);
                let idx = line
                    .closest_index_for_position(point(local.x, local_y), line_height)
                    .unwrap_or_else(|ix| ix);
                let doc_offset = starts.get(line_ix).copied().unwrap_or(0) + idx;
                Some(self.offset_to_utf16(doc_offset))
            }
        }
    }
}
