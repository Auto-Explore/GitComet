use super::highlight::runs_for_line;
use super::shaping::*;
use super::state::*;
use super::wrap::*;
use super::*;

impl TextInput {
    pub(super) fn clear_shaped_row_caches(&mut self) {
        self.layout.plain_line_cache.clear();
        self.highlight.prepaint_runs_cache = None;
    }

    pub(super) fn clear_wrap_recompute_state(&mut self) {
        self.wrap.pending_job = None;
        self.wrap.dirty_ranges.clear();
        self.wrap.recompute_requested = false;
    }

    pub(super) fn invalidate_layout_caches_full(&mut self) {
        self.reset_elastic_scroll();
        self.wrap.cache = None;
        self.layout.last = None;
        self.layout.line_starts = None;
        self.wrap.row_counts.clear();
        self.wrap.row_counts_current.clear();
        self.wrap.row_counts_width = None;
        self.wrap.row_counts_font = None;
        self.clear_wrap_recompute_state();
        self.wrap.last_rows = None;
        self.clear_shaped_row_caches();
    }

    pub(super) fn invalidate_layout_caches_preserving_wrap_rows(&mut self) {
        self.wrap.cache = None;
        self.layout.last = None;
        self.layout.line_starts = None;
        self.clear_shaped_row_caches();
    }

    pub(super) fn invalidate_layout_caches(&mut self) {
        self.invalidate_layout_caches_full();
    }

    pub(super) fn request_wrap_recompute(&mut self) {
        self.wrap.recompute_requested = true;
    }

    pub(super) fn bump_shape_style_epoch(&mut self) {
        self.layout.shape_style_epoch = self.layout.shape_style_epoch.wrapping_add(1).max(1);
        self.invalidate_layout_caches();
    }

    pub(super) fn bump_shape_style_epoch_preserving_wrap_rows(&mut self) {
        self.layout.shape_style_epoch = self.layout.shape_style_epoch.wrapping_add(1).max(1);
        self.invalidate_layout_caches_preserving_wrap_rows();
    }

    pub(super) fn queue_cursor_autoscroll(&mut self) {
        self.reset_elastic_scroll();
        self.interaction.pending_cursor_autoscroll = true;
        self.interaction.cursor_autoscroll_retries_remaining = TEXT_INPUT_CURSOR_AUTOSCROLL_RETRIES;
        self.interaction.cursor_autoscroll_layout_waits_remaining =
            TEXT_INPUT_CURSOR_AUTOSCROLL_RETRIES;
    }

    pub(super) fn trim_shape_caches(&mut self) {
        if self.layout.plain_line_cache.len() > TEXT_INPUT_SHAPE_CACHE_LIMIT {
            self.layout.plain_line_cache.clear();
        }
    }

    pub(super) fn shape_plain_line_cached(
        &mut self,
        line: LineShapeInput<'_>,
        precomputed_runs: Option<&[TextRun]>,
        shape_style: &TextShapeStyle<'_>,
        window: &mut Window,
    ) -> EditorLine {
        let key = ShapedRowCacheKey {
            line_ix: line.line_ix,
            font_size_key: f32::from(shape_style.font_size).round() as i32,
        };
        if let Some(cached) = self.layout.plain_line_cache.get(&key) {
            return cached.clone();
        }

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
        let shaped = EditorLine::shape(
            capped_text,
            shape_style.font_size,
            runs,
            None,
            self.tab_size,
            window,
        );
        self.layout.plain_line_cache.insert(key, shaped.clone());
        self.trim_shape_caches();
        shaped
    }

    pub(super) fn mark_wrap_dirty_from_edit(
        &mut self,
        old_lines: Range<usize>,
        new_lines: Range<usize>,
    ) {
        if !(self.multiline && self.soft_wrap) || self.wrap.row_counts.is_empty() {
            return;
        }
        debug_assert_eq!(old_lines.start, new_lines.start);
        // Keep the previous height provisionally for the edited lines, and
        // splice at the edit so all unchanged lines retain their measurements.
        let previous = &self.wrap.row_counts[old_lines.clone()];
        let replacement = (0..new_lines.len())
            .map(|ix| previous.get(ix).copied().unwrap_or(1))
            .collect::<Vec<_>>();
        self.wrap.row_counts.splice(old_lines.clone(), replacement);
        self.wrap.row_counts_current.splice(
            old_lines.clone(),
            std::iter::repeat_n(true, new_lines.len()),
        );
        if old_lines.len() != new_lines.len() {
            // A background snapshot still addresses the old line indices.
            if self.wrap.pending_job.take().is_some() {
                self.request_wrap_recompute();
            }
            for dirty in &mut self.wrap.dirty_ranges {
                dirty.start = if dirty.start >= old_lines.end {
                    dirty.start - old_lines.end + new_lines.end
                } else {
                    dirty.start.min(old_lines.start)
                };
                dirty.end = if dirty.end >= old_lines.end {
                    dirty.end - old_lines.end + new_lines.end
                } else if dirty.end > old_lines.start {
                    new_lines.end
                } else {
                    dirty.end
                };
            }
        }
        self.wrap.dirty_ranges.push(new_lines);
        self.wrap.last_rows = Some(total_wrap_rows(&self.wrap.row_counts));
    }

    pub(super) fn take_normalized_wrap_dirty_ranges(
        &mut self,
        line_count: usize,
    ) -> Vec<Range<usize>> {
        let mut ranges = std::mem::take(&mut self.wrap.dirty_ranges);
        ranges.retain_mut(|range| {
            range.start = range.start.min(line_count);
            range.end = range.end.min(line_count);
            range.start < range.end
        });
        if ranges.is_empty() {
            return ranges;
        }

        ranges.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(last) = merged.last_mut()
                && range.start <= last.end
            {
                last.end = last.end.max(range.end);
                continue;
            }
            merged.push(range);
        }
        merged
    }

    pub(super) fn substitute_display_text(
        &self,
        content: &TextModelSnapshot,
    ) -> Option<SharedString> {
        if content.is_empty() {
            Some(self.placeholder.clone())
        } else if self.masked {
            Some(mask_text_for_display(content.as_ref()).into())
        } else {
            None
        }
    }

    /// Wraps edited lines before layout, at the last frame's width and font, so
    /// this frame is laid out at their height. Prepaint corrects a resize.
    pub(super) fn measure_edited_wrap_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !(self.multiline && self.soft_wrap) || self.wrap.dirty_ranges.is_empty() {
            return;
        }
        let Some((base_font, font_size)) = self.wrap.row_counts_font.clone() else {
            return;
        };
        let Some(wrap_width) = self
            .layout
            .bounds
            .map(|bounds| bounds.size.width.max(px(0.0)))
            .filter(|width| self.wrap.row_counts_width == Some(width.round()))
        else {
            return;
        };
        let content = self.content.snapshot();
        let substitute_text = self.substitute_display_text(&content);
        let line_starts: Arc<[usize]> = match substitute_text.as_ref() {
            Some(text) => compute_line_starts(text).into(),
            None => content.shared_line_starts(),
        };
        let line_count = line_starts.len().max(1);
        if self.wrap.row_counts.len() != line_count {
            return;
        }
        let dirty_ranges = self.take_normalized_wrap_dirty_ranges(line_count);
        // Left for prepaint to confirm at the final width.
        self.wrap.dirty_ranges.clone_from(&dirty_ranges);
        let dirty_line_count = dirty_ranges.iter().map(Range::len).sum::<usize>();
        if dirty_line_count > TEXT_INPUT_WRAP_DIRTY_SYNC_LINE_LIMIT {
            return;
        }

        let has_content = !content.is_empty();
        let display_text = substitute_text.unwrap_or_else(|| content.as_shared_string());
        let mut rows_changed = false;
        for line_ix in dirty_ranges.into_iter().flatten() {
            let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
            let line_text = line_text_for_index(display_text.as_ref(), &line_starts, line_ix);
            // Bold or italic highlights move wrap points.
            let highlights = has_content.then(|| {
                self.highlights_polling_provider(line_start..line_start + line_text.len(), cx)
            });
            let line_style = TextShapeStyle {
                base_font: &base_font,
                text_color: self.style.text,
                highlights: highlights.as_ref().map(|highlights| highlights.as_slice()),
                font_size,
            };
            let wrapped = shape_wrapped_line(
                LineShapeInput {
                    line_ix,
                    line_start,
                    line_text,
                },
                wrap_width,
                None,
                &line_style,
                self.tab_size,
                window,
            );
            rows_changed |= self.set_measured_wrap_rows(line_ix, wrapped.line_count());
        }
        if !rows_changed {
            return;
        }
        let rows = total_wrap_rows(&self.wrap.row_counts);
        self.wrap.last_rows = Some(rows);
        if let Some(cache) = self.wrap.cache.as_mut() {
            cache.rows = rows;
        }
        // cache.rows already holds the new total, so paint's comparison cannot see this.
        self.wrap.ancestor_rows_stale = true;
    }

    pub(super) fn set_measured_wrap_rows(&mut self, line_ix: usize, rows: usize) -> bool {
        let rows = rows.max(1);
        let changed = self.wrap.row_counts[line_ix] != rows;
        self.wrap.row_counts[line_ix] = rows;
        // Protect even an unchanged count: a late estimator may disagree with
        // shaping, including on the very frame that launched its job.
        self.wrap.row_counts_current[line_ix] = true;
        changed
    }

    pub(super) fn maybe_recompute_wrap_rows(
        &mut self,
        display_text: &str,
        line_starts: &[usize],
        rounded_wrap_width: Pixels,
        font_size: Pixels,
        line_count: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.wrap.recompute_requested {
            return;
        }
        let width_key = wrap_width_cache_key(rounded_wrap_width);
        let wrap_columns = wrap_columns_for_width(rounded_wrap_width, font_size);
        let synchronous = line_count <= TEXT_INPUT_WRAP_SYNC_LINE_THRESHOLD;
        let tab_size = self.tab_size;
        estimate_wrap_rows_budgeted(
            display_text,
            line_starts,
            wrap_columns,
            tab_size,
            &mut self.wrap.row_counts,
            &self.wrap.row_counts_current,
            if synchronous {
                Duration::MAX
            } else {
                Duration::from_millis(TEXT_INPUT_WRAP_FOREGROUND_BUDGET_MS)
            },
        );
        self.wrap.row_counts_width = Some(rounded_wrap_width);
        self.wrap.recompute_requested = false;
        self.wrap.pending_job = None;
        if synchronous {
            return;
        }

        let sequence = self.wrap.recompute_sequence.wrapping_add(1).max(1);
        self.wrap.recompute_sequence = sequence;
        self.wrap.pending_job = Some(PendingWrapJob {
            sequence,
            width_key,
            line_count,
            wrap_columns,
        });

        let snapshot = display_text.to_string();
        let estimate = cx
            .background_executor()
            .spawn(async move { estimate_wrap_rows_for_text(&snapshot, wrap_columns, tab_size) });
        cx.spawn(
            async move |input: gpui::WeakEntity<TextInput>, cx: &mut gpui::AsyncApp| {
                let rows = estimate.await;
                let _ = input.update(cx, |input, cx| {
                    input.complete_wrap_recompute_job(sequence, width_key, line_count, rows, cx);
                });
            },
        )
        .detach();
    }

    pub(super) fn complete_wrap_recompute_job(
        &mut self,
        sequence: u64,
        width_key: i32,
        line_count: usize,
        mut rows: Vec<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(job) = self.wrap.pending_job else {
            return;
        };
        if job.sequence != sequence || job.width_key != width_key || job.line_count != line_count {
            return;
        }

        rows.resize(line_count, 1);
        for rows_per_line in &mut rows {
            *rows_per_line = (*rows_per_line).max(1);
        }
        for (ix, rows) in rows.into_iter().enumerate() {
            if !self.wrap.row_counts_current[ix] {
                self.wrap.row_counts[ix] = rows;
            }
        }
        self.wrap.pending_job = None;
        self.wrap.last_rows = Some(total_wrap_rows(self.wrap.row_counts.as_slice()));
        self.wrap.cache = None;
        cx.notify();
    }
}
