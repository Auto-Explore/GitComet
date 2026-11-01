use super::state::*;
use super::wrap::line_display_columns;
use super::*;
use gitcomet_text::rope::Rope;

pub(super) enum EditSelection {
    AfterInsertion,
    Preserve,
    Composition(Option<Range<usize>>),
}

/// The single replaced span between two texts, as `(old_range, new_range)`.
///
/// Both ranges share a start and land on character boundaries, so the pair is
/// directly usable as a `replace_utf8_range` edit. `None` means the texts are
/// identical. Shared with callers that need to describe a wholesale rewrite as
/// one minimal edit rather than a full-buffer replacement.
pub fn utf8_edit_delta_between_texts(
    old_text: &str,
    new_text: &str,
) -> Option<(Range<usize>, Range<usize>)> {
    edit_delta(old_text, new_text)
}

/// [`utf8_edit_delta_between_texts`] over two snapshots. Neither text is
/// flattened, and chunks the two ropes share are skipped without comparing
/// their bytes.
fn edit_delta_between_snapshots(
    old: &TextModelSnapshot,
    new: &TextModelSnapshot,
) -> Option<(Range<usize>, Range<usize>)> {
    edit_delta(&old.rope(), &new.rope())
}

/// Text read as a sequence of chunks: a rope's, or a string's single one.
trait ChunkedText {
    fn len(&self) -> usize;
    fn chunks(&self) -> impl Iterator<Item = &str>;
    /// The text from `start`, back to front.
    fn reversed_chunks_from(&self, start: usize) -> impl Iterator<Item = &str>;
    fn is_char_boundary(&self, offset: usize) -> bool;
}

impl ChunkedText for str {
    fn len(&self) -> usize {
        str::len(self)
    }
    fn chunks(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self)
    }
    fn reversed_chunks_from(&self, start: usize) -> impl Iterator<Item = &str> {
        std::iter::once(&self[start..])
    }
    fn is_char_boundary(&self, offset: usize) -> bool {
        str::is_char_boundary(self, offset)
    }
}

impl ChunkedText for Rope {
    fn len(&self) -> usize {
        Rope::len(self)
    }
    fn chunks(&self) -> impl Iterator<Item = &str> {
        Rope::chunks(self)
    }
    fn reversed_chunks_from(&self, start: usize) -> impl Iterator<Item = &str> {
        self.reversed_chunks_in_range(start..Rope::len(self))
    }
    fn is_char_boundary(&self, offset: usize) -> bool {
        Rope::is_char_boundary(self, offset)
    }
}

fn edit_delta<T: ChunkedText + ?Sized>(old: &T, new: &T) -> Option<(Range<usize>, Range<usize>)> {
    let (old_len, new_len) = (old.len(), new.len());
    let mut prefix = common_len(old.chunks(), new.chunks(), false);
    if prefix == old_len && old_len == new_len {
        return None;
    }
    while prefix > 0 && (!old.is_char_boundary(prefix) || !new.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let mut suffix = common_len(
        old.reversed_chunks_from(prefix),
        new.reversed_chunks_from(prefix),
        true,
    );
    while suffix > 0
        && (!old.is_char_boundary(old_len - suffix) || !new.is_char_boundary(new_len - suffix))
    {
        suffix -= 1;
    }
    Some((prefix..old_len - suffix, prefix..new_len - suffix))
}

/// The length of the common prefix of two chunk streams, or of their common
/// suffix when the chunks arrive back to front.
fn common_len<'a>(
    mut old: impl Iterator<Item = &'a str>,
    mut new: impl Iterator<Item = &'a str>,
    reversed: bool,
) -> usize {
    let (mut a, mut b): (&[u8], &[u8]) = (&[], &[]);
    let mut total = 0;
    loop {
        if a.is_empty() {
            match old.next() {
                Some(chunk) => a = chunk.as_bytes(),
                None => return total,
            }
            continue;
        }
        if b.is_empty() {
            match new.next() {
                Some(chunk) => b = chunk.as_bytes(),
                None => return total,
            }
            continue;
        }
        let n = a.len().min(b.len());
        let (a_part, b_part) = if reversed {
            (&a[a.len() - n..], &b[b.len() - n..])
        } else {
            (&a[..n], &b[..n])
        };
        if std::ptr::eq(a_part, b_part) || a_part == b_part {
            total += n;
            if reversed {
                a = &a[..a.len() - n];
                b = &b[..b.len() - n];
            } else {
                a = &a[n..];
                b = &b[n..];
            }
            continue;
        }
        let same = if reversed {
            a_part
                .iter()
                .rev()
                .zip(b_part.iter().rev())
                .take_while(|(x, y)| x == y)
                .count()
        } else {
            a_part
                .iter()
                .zip(b_part)
                .take_while(|(x, y)| x == y)
                .count()
        };
        return total + same;
    }
}

impl TextInput {
    fn emit_content_changed(&self, cx: &mut Context<Self>) {
        let snapshot = self.content.snapshot();
        crate::ui_probe::action_phase(self.probe_action, "applied", || {
            serde_json::json!({
                "model":snapshot.model_id(), "revision":snapshot.revision(), "bytes":snapshot.len()
            })
        });
        cx.emit(TextInputChanged {
            model_id: snapshot.model_id(),
            revision: snapshot.revision(),
        });
    }

    /// A live read-only log may append while a user is selecting earlier text.
    /// Replacements (including front truncation) deliberately reset selection.
    pub fn set_text_preserving_selection_on_append(
        &mut self,
        text: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        assert!(self.read_only);
        self.update_text(text.into(), true, cx);
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.update_text(text.into(), false, cx);
    }

    fn update_text(&mut self, text: SharedString, preserve_append: bool, cx: &mut Context<Self>) {
        if self.content.as_ref() == text.as_ref() {
            return;
        }
        let preserve_selection = preserve_append && text.starts_with(self.content.as_ref());
        self.probe_action = 0;
        // Computed before the overwrite so highlights already on screen can ride
        // along; the equality check above keeps this O(n) scan off the hot path.
        let text_edit_delta = utf8_edit_delta_between_texts(self.content.as_ref(), text.as_ref());
        self.content.set_text(text.as_ref());
        self.emit_content_changed(cx);
        self.protected_ranges = Arc::from([]);
        self.rebuild_content_width_cache_if_present();
        if !preserve_selection {
            self.selection.range = self.content.len()..self.content.len();
            self.selection.reversed = false;
            self.end_mouse_drag();
            self.layout.scroll_x = px(0.0);
        }
        self.selection.undo_stack.clear();
        self.selection.redo_stack.clear();
        self.show_cursor();
        self.invalidate_layout_caches();
        if self.multiline && self.soft_wrap {
            self.request_wrap_recompute();
        }
        self.selection.pending_text_edit_deltas.clear();
        if let Some((replaced, inserted)) = text_edit_delta {
            self.note_text_edit_for_highlights(&replaced, &inserted);
        }
        cx.notify();
    }

    /// Refuse edits to these byte spans, each covering a whole line including
    /// its terminator. Spans must be sorted and disjoint. Cleared by
    /// [`Self::set_text`], since the offsets describe the buffer that was
    /// replaced; the owner re-publishes them for the new one.
    pub fn set_protected_ranges(&mut self, ranges: Arc<[Range<usize>]>) {
        self.protected_ranges = ranges;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn protected_ranges(&self) -> &[Range<usize>] {
        &self.protected_ranges
    }

    /// Whether replacing `range` with `new_text` would alter a protected line.
    ///
    /// Anything overlapping a span is out, and so are the two ways to reach one
    /// from outside: inserting at its first offset lands inside the protected
    /// line, and an edit that stops there eats the newline that made the line
    /// stand on its own unless it puts a line boundary back.
    pub fn edit_alters_protected_range(&self, range: &Range<usize>, new_text: &str) -> bool {
        if self.protected_ranges.is_empty() {
            return false;
        }

        let rope = self.content.snapshot().rope();
        self.protected_ranges.iter().any(|protected| {
            if range.start >= protected.end || range.end < protected.start {
                return false;
            }
            if range.end > protected.start || range.start == protected.start {
                return true;
            }
            // Ends exactly where the span begins: safe only while whatever now
            // precedes the span still ends a line.
            match new_text.as_bytes().last() {
                Some(last) => *last != b'\n',
                None => {
                    range.start != 0
                        && rope
                            .bytes_at(range.start - 1, range.start)
                            .first()
                            .is_some_and(|byte| *byte != b'\n')
                }
            }
        })
    }

    /// Carry the protected spans across an edit that was allowed through.
    ///
    /// Typed edits never overlap a span, but a programmatic rewrite does when
    /// the owner resolves that conflict — the spans it published describe a
    /// buffer that no longer exists, so drop them and let it republish.
    fn shift_protected_ranges_for_edit(&mut self, old: &Range<usize>, new: &Range<usize>) {
        if self.protected_ranges.is_empty() {
            return;
        }
        if self
            .protected_ranges
            .iter()
            .any(|range| old.start < range.end && old.end > range.start)
        {
            self.protected_ranges = Arc::from([]);
            return;
        }
        let shift = new.len() as isize - old.len() as isize;
        if shift == 0 {
            return;
        }
        let shifted = |offset: usize| {
            if shift >= 0 {
                offset.saturating_add(shift as usize)
            } else {
                offset.saturating_sub(shift.unsigned_abs())
            }
        };
        self.protected_ranges = self
            .protected_ranges
            .iter()
            .map(|range| {
                if range.end <= old.start {
                    range.clone()
                } else {
                    shifted(range.start)..shifted(range.end)
                }
            })
            .collect();
    }

    /// Width of one row, in the max of byte length and display columns.
    ///
    /// Reads just that row out of the rope, so maintaining the cache after an
    /// edit costs O(log n) per touched row rather than a whole-document scan.
    fn content_width_line_units(
        content: &TextModelSnapshot,
        line_ix: usize,
        tab_size: usize,
    ) -> usize {
        let line = content.slice(content.line_range_with_terminator(line_ix));
        line.len().max(line_display_columns(&line, tab_size))
    }

    fn affected_lines(content: &TextModelSnapshot, byte_range: Range<usize>) -> Range<usize> {
        let line_count = content.line_count().max(1);
        let start = content.row_for_offset(byte_range.start);
        let end = content.row_for_offset(byte_range.end);
        start.min(line_count.saturating_sub(1))..end.saturating_add(1).min(line_count)
    }

    pub(super) fn rebuild_content_width_cache(&mut self) {
        let content = self.content.snapshot();
        let tab_size = self.tab_size;
        let line_count = content.line_count().max(1);
        let mut cache = ContentWidthCache::default();
        cache.line_units.reserve(line_count);
        for line_ix in 0..line_count {
            let units = Self::content_width_line_units(&content, line_ix, tab_size);
            cache.line_units.push(units);
            *cache.unit_counts.entry(units).or_default() += 1;
        }
        self.content_width_cache = Some(cache);
    }

    pub(super) fn rebuild_content_width_cache_if_present(&mut self) {
        if self.content_width_cache.is_some() {
            self.rebuild_content_width_cache();
        }
    }

    fn replace_content_range(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        if range.start <= range.end
            && range.end <= self.content.len()
            && self.content.clamp_to_char_boundary(range.start) == range.start
            && self.content.clamp_to_char_boundary(range.end) == range.end
            // Before `slice`, which copies a range spanning rope chunks.
            && range.len() == new_text.len()
            && self.content.snapshot().slice(range.clone()).as_ref() == new_text
        {
            return range;
        }
        self.probe_action = crate::ui_probe::begin_action("typing");
        // Snapshotting is an `Arc` bump, and it is the only way to read the
        // pre-edit row layout after `replace_range` has already moved on.
        let previous = self.content.snapshot();
        let inserted = self.content.replace_range(range.clone(), new_text);
        self.emit_content_changed(cx);
        self.note_lines_replaced(&previous, range, inserted.clone());
        inserted
    }

    /// Line bookkeeping after `old` in `previous` became `new` in the content:
    /// wrap rows and cached widths of the edited lines only.
    fn note_lines_replaced(
        &mut self,
        previous: &TextModelSnapshot,
        old: Range<usize>,
        new: Range<usize>,
    ) {
        if self.content_width_cache.is_none() && !(self.multiline && self.soft_wrap) {
            return;
        }
        let content = self.content.snapshot();
        let old_lines = Self::affected_lines(previous, old);
        let new_lines = Self::affected_lines(&content, new);
        self.mark_wrap_dirty_from_edit(old_lines.clone(), new_lines.clone());
        self.update_content_width_cache(&content, old_lines, new_lines);
    }

    /// Replace the cached widths of the edited lines, `old_affected` in the
    /// previous text, with those of `new_affected` in `content`.
    fn update_content_width_cache(
        &mut self,
        content: &TextModelSnapshot,
        old_affected: Range<usize>,
        new_affected: Range<usize>,
    ) {
        let tab_size = self.tab_size;
        let Some(cache) = self.content_width_cache.as_mut() else {
            return;
        };
        let replacement_units = new_affected
            .map(|line_ix| Self::content_width_line_units(content, line_ix, tab_size))
            .collect::<Vec<_>>();
        for &units in cache
            .line_units
            .get(old_affected.clone())
            .unwrap_or_default()
        {
            if let Some(count) = cache.unit_counts.get_mut(&units) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    cache.unit_counts.remove(&units);
                }
            }
        }
        cache
            .line_units
            .splice(old_affected, replacement_units.iter().copied());
        for units in replacement_units {
            *cache.unit_counts.entry(units).or_default() += 1;
        }
        debug_assert_eq!(cache.line_units.len(), content.line_count().max(1));
    }

    pub(super) fn content_width_max_units(&self) -> usize {
        self.content_width_cache
            .as_ref()
            .map(ContentWidthCache::max_units)
            .unwrap_or_default()
    }

    pub(super) fn sanitize_insert_text(&self, text: &str) -> Option<String> {
        if self.multiline {
            return Some(text.to_string());
        }

        if text == "\n" || text == "\r" || text == "\r\n" {
            return None;
        }

        Some(
            text.replace("\r\n", "\n")
                .replace('\r', "\n")
                .replace('\n', " "),
        )
    }

    pub(super) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selection.range.is_empty() {
            self.extend_selection_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    pub(super) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selection.range.is_empty() {
            self.extend_selection_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    pub(super) fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selection.range.is_empty() {
            self.extend_selection_to(self.previous_word_start(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    pub(super) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selection.range.is_empty() {
            self.extend_selection_to(self.next_word_end(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    /// Cmd-Backspace (Ctrl-Shift-Backspace on Windows/Linux): delete back to
    /// where Home would move the caret.
    /// At the start of a row it deletes the line break instead, joining the
    /// row to the one above, as native text fields do.
    pub(super) fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selection.range.is_empty() {
            let cursor = self.cursor_offset();
            let start = self.row_start(cursor);
            let target = if start < cursor {
                start
            } else {
                self.previous_boundary(cursor)
            };
            self.extend_selection_to(target, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    /// Cmd-Delete (Ctrl-Shift-Delete on Windows/Linux): delete forward to
    /// where End would move the caret, or the line break when already at the
    /// end of a row.
    pub(super) fn delete_to_line_end(
        &mut self,
        _: &DeleteToLineEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selection.range.is_empty() {
            let cursor = self.cursor_offset();
            let end = self.row_end(cursor);
            let target = if end > cursor {
                end
            } else {
                self.next_boundary(cursor)
            };
            self.extend_selection_to(target, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    pub(super) fn insert_line_break(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.queue_cursor_autoscroll();
        self.replace_text_in_range(None, self.line_ending, window, cx);
    }

    pub(super) fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || !self.multiline || self.interaction.submit_on_enter {
            self.interaction.enter_pressed = true;
            cx.notify();
            return;
        }
        self.insert_line_break(window, cx);
    }

    pub(super) fn shift_enter(
        &mut self,
        _: &ShiftEnter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only || !self.multiline {
            return;
        }
        self.insert_line_break(window, cx);
    }

    pub(super) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(snapshot) = self.selection.undo_stack.pop() else {
            return;
        };
        self.push_redo_snapshot(self.current_undo_snapshot());
        self.restore_undo_snapshot(snapshot, window, cx);
    }

    pub(super) fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(snapshot) = self.selection.redo_stack.pop() else {
            return;
        };
        self.push_undo_snapshot(self.current_undo_snapshot());
        self.restore_undo_snapshot(snapshot, window, cx);
    }

    pub(super) fn normalized_utf8_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clamp_to_char_boundary(range.start.min(self.content.len()));
        let end = self.clamp_to_char_boundary(range.end.min(self.content.len()));
        if end < start { end..start } else { start..end }
    }

    pub(super) fn replace_utf8_range_internal(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        self.apply_edit(range, new_text, EditSelection::AfterInsertion, cx)
    }

    /// Shift one caret/selection endpoint across an edit that replaced
    /// `range` with `inserted`, so it keeps pointing at the same text.
    fn shift_offset_across_edit(
        offset: usize,
        range: &Range<usize>,
        inserted: &Range<usize>,
    ) -> usize {
        if offset <= range.start {
            offset
        } else if offset >= range.end {
            // Past the edit: move by the length delta, computed without
            // signed arithmetic so a shrinking edit cannot underflow.
            offset
                .saturating_sub(range.end)
                .saturating_add(inserted.end)
        } else {
            // Inside the replaced span, which no longer exists: the end of the
            // replacement is the closest surviving position.
            inserted.end
        }
    }

    pub(super) fn apply_edit(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        selection: EditSelection,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        let undo_snapshot = self.current_undo_snapshot();
        let range = self.normalized_utf8_range(range);
        let previous_selection = self.selection.range.clone();
        let previous_reversed = self.selection.reversed;
        let inserted = self.replace_content_range(range.clone(), new_text, cx);
        self.shift_protected_ranges_for_edit(&range, &inserted);
        self.push_undo_snapshot(undo_snapshot);
        self.selection.redo_stack.clear();
        self.selection
            .pending_text_edit_deltas
            .push((range.clone(), inserted.clone()));
        self.selection.marked_range = None;
        match &selection {
            EditSelection::Preserve => {
                let start = self.clamp_to_char_boundary(
                    Self::shift_offset_across_edit(previous_selection.start, &range, &inserted)
                        .min(self.content.len()),
                );
                let end = self.clamp_to_char_boundary(
                    Self::shift_offset_across_edit(previous_selection.end, &range, &inserted)
                        .min(self.content.len()),
                );
                self.selection.range = start.min(end)..start.max(end);
                self.selection.reversed = previous_reversed;
            }
            EditSelection::AfterInsertion => {
                self.selection.range = inserted.end..inserted.end;
                self.selection.reversed = false;
            }
            EditSelection::Composition(selected_utf16) => {
                self.selection.marked_range = (!inserted.is_empty()).then(|| inserted.clone());
                self.selection.range = selected_utf16
                    .as_ref()
                    .map(|selected| {
                        let origin = self.offset_to_utf16(inserted.start);
                        self.normalized_utf8_range(
                            self.offset_from_utf16(origin.saturating_add(selected.start))
                                .min(inserted.end)
                                ..self
                                    .offset_from_utf16(origin.saturating_add(selected.end))
                                    .min(inserted.end),
                        )
                    })
                    .unwrap_or(inserted.end..inserted.end);
                self.selection.reversed = false;
            }
        }
        self.interaction.vertical_motion_x = None;
        self.show_cursor();
        if matches!(selection, EditSelection::Composition(_)) {
            self.clear_selection_on_ownership_loss(cx);
        }
        self.invalidate_layout_caches_preserving_wrap_rows();
        self.note_text_edit_for_highlights(&range, &inserted);
        if !matches!(selection, EditSelection::Preserve) {
            self.queue_cursor_autoscroll();
        }
        cx.notify();
        inserted
    }

    /// Replace a UTF-8 byte range in content.
    ///
    /// Returns the inserted byte range after replacement.
    pub fn replace_utf8_range(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        if self.read_only {
            let cursor = self.cursor_offset();
            return cursor..cursor;
        }
        let Some(new_text) = self.sanitize_insert_text(new_text) else {
            let cursor = self.cursor_offset();
            return cursor..cursor;
        };
        self.replace_utf8_range_internal(range, &new_text, cx)
    }

    /// Replace a UTF-8 byte range without moving the caret or scrolling to the
    /// edit, for rewrites the user did not type.
    ///
    /// [`replace_utf8_range`](Self::replace_utf8_range) parks the caret at the
    /// end of the replacement and queues a cursor autoscroll, which is right
    /// for an edit the user just made at that spot. It is wrong when the
    /// document is regenerated from state the user changed elsewhere — the
    /// merge tool rebuilds its whole resolved output on every pick — because
    /// the autoscroll runs during paint and therefore overrides whatever the
    /// caller scrolled to itself. Callers that do want the view to follow the
    /// edit scroll explicitly after calling this.
    ///
    /// The caret and selection are shifted across the edit so they keep
    /// pointing at the same text. Returns the inserted byte range.
    pub fn replace_utf8_range_preserving_view(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        if self.read_only {
            let cursor = self.cursor_offset();
            return cursor..cursor;
        }
        let Some(new_text) = self.sanitize_insert_text(new_text) else {
            let cursor = self.cursor_offset();
            return cursor..cursor;
        };
        self.apply_edit(range, &new_text, EditSelection::Preserve, cx)
    }

    /// Replace the current selection range with `new_text`.
    ///
    /// Returns the inserted byte range after replacement.
    pub fn replace_selection_utf8(
        &mut self,
        new_text: &str,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        self.replace_utf8_range(self.selection.range.clone(), new_text, cx)
    }

    /// Drain queued UTF-8 edit deltas in application order.
    ///
    /// Each `old_range` references bytes before its corresponding edit and
    /// each `new_range` references bytes after it. Retaining the whole queue is
    /// important when GPUI coalesces multiple notifications.
    pub fn drain_recent_utf8_edit_deltas(&mut self) -> Vec<(Range<usize>, Range<usize>)> {
        std::mem::take(&mut self.selection.pending_text_edit_deltas)
    }

    pub(super) fn current_undo_snapshot(&self) -> UndoSnapshot {
        UndoSnapshot {
            content: self.content.snapshot_without_caches(),
            selected_range: self.selection.range.clone(),
            selection_reversed: self.selection.reversed,
        }
    }

    pub(super) fn push_undo_snapshot(&mut self, snapshot: UndoSnapshot) {
        Self::push_history_snapshot(&mut self.selection.undo_stack, snapshot);
    }

    pub(super) fn push_redo_snapshot(&mut self, snapshot: UndoSnapshot) {
        Self::push_history_snapshot(&mut self.selection.redo_stack, snapshot);
    }

    pub(super) fn push_history_snapshot(stack: &mut Vec<UndoSnapshot>, snapshot: UndoSnapshot) {
        if stack.last() == Some(&snapshot) {
            return;
        }
        if stack.len() >= MAX_UNDO_STEPS {
            let _ = stack.remove(0);
        }
        stack.push(snapshot);
    }

    pub(super) fn restore_undo_snapshot(
        &mut self,
        snapshot: UndoSnapshot,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.probe_action = crate::ui_probe::begin_action("typing");
        let previous = self.content.snapshot();
        let text_edit_delta = edit_delta_between_snapshots(&previous, &snapshot.content);
        self.content = snapshot.content.into();
        if let Some((old, new)) = &text_edit_delta {
            self.emit_content_changed(cx);
            self.note_lines_replaced(&previous, old.clone(), new.clone());
        }
        // The spans described the buffer this snapshot just replaced; the owner
        // republishes them for the restored one.
        self.protected_ranges = Arc::from([]);
        self.selection.range = snapshot.selected_range;
        self.selection.reversed = snapshot.selection_reversed;
        self.selection.marked_range = None;
        if !self.selection.range.is_empty() {
            self.selection_owner.adopt(window, cx);
        }
        self.interaction.vertical_motion_x = None;
        self.show_cursor();
        self.end_mouse_drag();
        self.invalidate_layout_caches_preserving_wrap_rows();
        if let Some(delta) = text_edit_delta {
            self.note_text_edit_for_highlights(&delta.0, &delta.1);
            self.selection.pending_text_edit_deltas.push(delta);
        }
        self.queue_cursor_autoscroll();
        cx.notify();
    }
}
