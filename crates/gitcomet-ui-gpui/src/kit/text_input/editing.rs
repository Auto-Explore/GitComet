use super::highlight::*;
use super::shaping::*;
use super::state::*;
use super::wrap::*;
use super::*;

impl TextInput {
    pub fn new(options: TextInputOptions, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::from_options(options, cx)
    }

    pub fn new_inert(options: TextInputOptions, cx: &mut Context<Self>) -> Self {
        Self::from_options(options, cx)
    }

    pub(super) fn from_options(options: TextInputOptions, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        Self {
            focus_handle,
            content: TextModel::new(),
            placeholder: options.placeholder,
            multiline: options.multiline,
            read_only: options.read_only,
            chromeless: options.chromeless,
            soft_wrap: options.soft_wrap,
            display_truncation: None,
            masked: false,
            line_ending: if cfg!(windows) { "\r\n" } else { "\n" },
            style: TextInputStyle::from_theme(AppTheme::gitcomet_dark()),
            line_height_override: None,
            highlight: HighlightState::new(),
            layout: LayoutState::new(),
            wrap: WrapState::new(),
            selection: SelectionState::new(),
            interaction: InteractionState::new(),
        }
    }

    pub fn text(&self) -> &str {
        self.content.as_ref()
    }

    pub fn text_snapshot(&self) -> TextModelSnapshot {
        self.content.snapshot()
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub(super) fn clear_shaped_row_caches(&mut self) {
        self.layout.plain_line_cache.clear();
        self.layout.wrapped_line_cache.clear();
        self.highlight.prepaint_runs_cache = None;
    }

    pub(super) fn clear_wrap_recompute_state(&mut self) {
        self.wrap.pending_job = None;
        self.wrap.dirty_ranges.clear();
        self.wrap.interpolated_patches.clear();
        self.wrap.recompute_requested = false;
    }

    pub(super) fn invalidate_layout_caches_full(&mut self) {
        self.wrap.cache = None;
        self.layout.last = None;
        self.layout.line_starts = None;
        self.wrap.row_counts.clear();
        self.wrap.row_counts_width = None;
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

    pub(super) fn invalidate_highlights(&mut self, preserve_wrap_rows: bool) {
        self.highlight.provider_cache = None;
        self.highlight.epoch = self.highlight.epoch.wrapping_add(1).max(1);
        if preserve_wrap_rows {
            self.bump_shape_style_epoch_preserving_wrap_rows();
        } else {
            self.bump_shape_style_epoch();
        }
    }

    pub(super) fn note_provider_highlights_changed(&mut self) {
        self.invalidate_highlights(true);
    }

    pub(super) fn invalidate_provider_highlights_for_text_change(&mut self) {
        if self.highlight.provider.is_none() {
            return;
        }

        self.highlight.provider_cache = None;
        self.highlight.prepaint_runs_cache = None;
        self.highlight.epoch = self.highlight.epoch.wrapping_add(1).max(1);
    }

    pub fn set_theme(&mut self, theme: AppTheme, cx: &mut Context<Self>) {
        let style = TextInputStyle::from_theme(theme);
        if self.style == style {
            return;
        }
        self.style = style;
        self.bump_shape_style_epoch();
        cx.notify();
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        if self.content.as_ref() == text.as_ref() {
            return;
        }
        self.content.set_text(text.as_ref());
        self.selection.range = self.content.len()..self.content.len();
        self.selection.reversed = false;
        self.selection.undo_stack.clear();
        self.selection.redo_stack.clear();
        self.interaction.cursor_blink_visible = true;
        self.layout.scroll_x = px(0.0);
        self.invalidate_layout_caches();
        if self.multiline && self.soft_wrap {
            self.request_wrap_recompute();
        }
        self.selection.pending_text_edit_delta = None;
        self.invalidate_provider_highlights_for_text_change();
        cx.notify();
    }

    pub fn set_highlights(
        &mut self,
        mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
        cx: &mut Context<Self>,
    ) {
        highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        self.highlight.highlights = Arc::new(highlights);
        self.highlight.provider = None;
        self.highlight.provider_binding_key = None;
        self.highlight.provider_poll_task.take();
        self.invalidate_highlights(false);
        cx.notify();
    }

    pub(super) fn install_highlight_provider(
        &mut self,
        provider: HighlightProvider,
        binding_key: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if !should_reset_highlight_provider_binding(
            self.highlight.provider.is_some(),
            self.highlight.provider_binding_key,
            binding_key,
        ) {
            return;
        }

        self.highlight.provider = Some(provider);
        self.highlight.provider_binding_key = binding_key;
        self.highlight.provider_poll_task.take();
        self.highlight.highlights = Arc::new(Vec::new());
        self.invalidate_highlights(false);
        cx.notify();
    }

    /// Replace the full highlight vector with a lazy provider that generates highlights
    /// on demand for only the visible byte range. Use this for large documents where
    /// materializing all highlights is wasteful.
    pub fn set_highlight_provider(&mut self, provider: HighlightProvider, cx: &mut Context<Self>) {
        self.install_highlight_provider(provider, None, cx);
    }

    /// Like `set_highlight_provider`, but lets callers provide a stable binding key so
    /// repeated reapplication of the same provider can keep the existing highlight cache.
    pub fn set_highlight_provider_with_key(
        &mut self,
        binding_key: u64,
        provider: HighlightProvider,
        cx: &mut Context<Self>,
    ) {
        self.install_highlight_provider(provider, Some(binding_key), cx);
    }

    pub fn set_line_height(&mut self, line_height: Option<Pixels>, cx: &mut Context<Self>) {
        if self.line_height_override == line_height {
            return;
        }
        self.line_height_override = line_height;
        cx.notify();
    }

    pub(super) fn effective_line_height(&self, window: &Window) -> Pixels {
        self.line_height_override
            .unwrap_or_else(|| window.line_height())
    }

    pub fn take_enter_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.enter_pressed)
    }

    pub fn take_escape_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.escape_pressed)
    }

    pub fn clear_transient_key_presses(&mut self) {
        self.interaction.enter_pressed = false;
        self.interaction.escape_pressed = false;
    }

    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        if self.read_only == read_only {
            return;
        }
        self.read_only = read_only;
        if !self.read_only && self.display_truncation.is_some() {
            self.display_truncation = None;
            self.invalidate_layout_caches();
        }
        cx.notify();
    }

    pub fn set_display_truncation(
        &mut self,
        display_truncation: Option<TextTruncationProfile>,
        cx: &mut Context<Self>,
    ) {
        debug_assert!(
            display_truncation.is_none() || (self.read_only && !self.multiline),
            "display truncation is only supported for single-line read-only text inputs"
        );
        let next = display_truncation.filter(|_| self.read_only && !self.multiline);
        if self.display_truncation == next {
            return;
        }
        self.display_truncation = next;
        self.layout.scroll_x = px(0.0);
        self.invalidate_layout_caches();
        cx.notify();
    }

    pub fn set_suppress_right_click(&mut self, suppress: bool) {
        self.interaction.suppress_right_click = suppress;
    }

    pub fn set_vertical_scroll_handle(&mut self, handle: Option<ScrollHandle>) {
        self.interaction.vertical_scroll_handle = handle;
    }

    pub(super) fn queue_cursor_autoscroll(&mut self) {
        self.interaction.pending_cursor_autoscroll = true;
    }

    pub(super) fn resolve_provider_highlights(
        &mut self,
        byte_start: usize,
        byte_end: usize,
    ) -> ResolvedProviderHighlights {
        let requested_range = byte_start..byte_end;
        if let Some(cache) = self.highlight.provider_cache.as_mut()
            && let Some(resolved) = cache.resolve(self.highlight.epoch, &requested_range)
        {
            return resolved;
        }
        let Some(ref provider) = self.highlight.provider else {
            return ResolvedProviderHighlights {
                pending: false,
                highlights: Arc::new(Vec::new()),
            };
        };
        let mut result = provider.resolve(requested_range.clone());
        result
            .highlights
            .sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        let pending = result.pending;
        let highlights = Arc::new(result.highlights);
        self.highlight
            .provider_cache
            .get_or_insert_with(|| ProviderHighlightCache::new(self.highlight.epoch))
            .insert(
                self.highlight.epoch,
                requested_range,
                pending,
                Arc::clone(&highlights),
            );
        ResolvedProviderHighlights {
            pending,
            highlights,
        }
    }

    pub(super) fn ensure_highlight_provider_poll(&mut self, cx: &mut Context<Self>) {
        if self.highlight.provider_poll_task.is_some() {
            return;
        }

        let task = cx.spawn(
            async move |input: gpui::WeakEntity<TextInput>, cx: &mut gpui::AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(16)).await;

                let should_continue = input
                    .update(cx, |input, cx| {
                        let Some(provider) = input.highlight.provider.clone() else {
                            input.highlight.provider_poll_task = None;
                            return false;
                        };

                        let applied = provider.drain_pending();
                        if applied > 0 {
                            input.note_provider_highlights_changed();
                            cx.notify();
                        }

                        let pending = provider.has_pending();
                        if !pending {
                            input.highlight.provider_poll_task = None;
                        }
                        pending
                    })
                    .unwrap_or(false);

                if !should_continue {
                    break;
                }
            },
        );
        self.highlight.provider_poll_task = Some(task);
    }

    pub(super) fn trim_shape_caches(&mut self) {
        if self.layout.plain_line_cache.len() > TEXT_INPUT_SHAPE_CACHE_LIMIT {
            self.layout.plain_line_cache.clear();
        }
        if self.layout.wrapped_line_cache.len() > TEXT_INPUT_SHAPE_CACHE_LIMIT {
            self.layout.wrapped_line_cache.clear();
        }
    }

    pub(super) fn streamed_highlight_runs_for_visible_window(
        &mut self,
        display_text: &str,
        line_starts: &[usize],
        visible_line_range: Range<usize>,
        shape_style: &TextShapeStyle<'_>,
    ) -> Option<Arc<VisibleWindowTextRuns>> {
        let Some(highlights) = shape_style.highlights else {
            self.highlight.prepaint_runs_cache = None;
            return None;
        };
        let line_count = line_starts.len().max(1);
        if highlights.is_empty()
            || line_count <= TEXT_INPUT_STREAMED_HIGHLIGHT_LEGACY_LINE_THRESHOLD
            || visible_line_range.is_empty()
        {
            self.highlight.prepaint_runs_cache = None;
            return None;
        }

        if let Some(cache) = self.highlight.prepaint_runs_cache.as_ref()
            && cache.highlight_epoch == self.highlight.epoch
            && cache.visible_start == visible_line_range.start
            && cache.visible_end == visible_line_range.end
        {
            return Some(Arc::clone(&cache.line_runs));
        }

        let line_runs = Arc::new(build_streamed_highlight_runs_for_visible_window(
            shape_style.base_font,
            shape_style.text_color,
            display_text,
            line_starts,
            visible_line_range.clone(),
            highlights,
        ));
        self.highlight.prepaint_runs_cache = Some(PrepaintHighlightRunsCache {
            highlight_epoch: self.highlight.epoch,
            visible_start: visible_line_range.start,
            visible_end: visible_line_range.end,
            line_runs: Arc::clone(&line_runs),
        });
        Some(line_runs)
    }

    pub(super) fn shape_plain_line_cached(
        &mut self,
        line: LineShapeInput<'_>,
        precomputed_runs: Option<&[TextRun]>,
        shape_style: &TextShapeStyle<'_>,
        window: &mut Window,
    ) -> ShapedLine {
        let key = ShapedRowCacheKey {
            line_ix: line.line_ix,
            wrap_width_key: i32::MIN,
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
        let shaped =
            window
                .text_system()
                .shape_line(capped_text, shape_style.font_size, runs, None);
        self.layout.plain_line_cache.insert(key, shaped.clone());
        self.trim_shape_caches();
        shaped
    }

    pub(super) fn shape_wrapped_line_cached(
        &mut self,
        line: LineShapeInput<'_>,
        wrap_width: Pixels,
        precomputed_runs: Option<&[TextRun]>,
        shape_style: &TextShapeStyle<'_>,
        window: &mut Window,
    ) -> WrappedLine {
        let key = ShapedRowCacheKey {
            line_ix: line.line_ix,
            wrap_width_key: wrap_width_cache_key(wrap_width),
        };
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
        let shaped = window
            .text_system()
            .shape_text(
                capped_text,
                shape_style.font_size,
                runs,
                Some(wrap_width),
                None,
            )
            .unwrap_or_default();
        let wrapped = shaped.into_iter().next().unwrap_or_default();
        self.layout.wrapped_line_cache.insert(key, ());
        self.trim_shape_caches();
        wrapped
    }

    pub(super) fn mark_wrap_dirty_from_edit(
        &mut self,
        old_range: Range<usize>,
        new_range: Range<usize>,
    ) {
        if !(self.multiline && self.soft_wrap) {
            return;
        }

        let text = self.content.as_ref();
        let line_starts = self.content.line_starts();
        let line_count = line_starts.len().max(1);
        if self.wrap.row_counts.len() != line_count {
            self.wrap.row_counts.resize(line_count, 1);
            self.wrap.recompute_requested = true;
            self.wrap.pending_job = None;
            self.wrap.interpolated_patches.clear();
            return;
        }

        let dirty_range =
            expanded_dirty_wrap_line_range_for_edit(text, line_starts, &old_range, &new_range);
        if dirty_range.start < dirty_range.end {
            self.wrap.dirty_ranges.push(dirty_range);
        }
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

    pub(super) fn push_interpolated_wrap_patch(
        &mut self,
        width_key: i32,
        line_ix: usize,
        old_rows: usize,
        new_rows: usize,
    ) {
        if old_rows == new_rows {
            return;
        }

        if let Some(last) = self.wrap.interpolated_patches.last_mut()
            && last.width_key == width_key
            && last.line_start + last.old_rows.len() == line_ix
        {
            last.old_rows.push(old_rows);
            last.new_rows.push(new_rows);
            return;
        }

        if reset_interpolated_wrap_patches_on_overflow(
            &mut self.wrap.interpolated_patches,
            &mut self.wrap.recompute_requested,
        ) {
            return;
        }
        self.wrap.interpolated_patches.push(InterpolatedWrapPatch {
            width_key,
            line_start: line_ix,
            old_rows: vec![old_rows],
            new_rows: vec![new_rows],
        });
    }

    pub(super) fn apply_pending_dirty_wrap_updates(
        &mut self,
        display_text: &str,
        line_starts: &[usize],
        rounded_wrap_width: Pixels,
        font_size: Pixels,
        allow_interpolated_patches: bool,
    ) -> bool {
        if self.wrap.dirty_ranges.is_empty() {
            return false;
        }

        let line_count = line_starts.len().max(1);
        if line_count == 0 {
            self.wrap.dirty_ranges.clear();
            return false;
        }

        let mut ranges = self.take_normalized_wrap_dirty_ranges(line_count);
        let dirty_line_count = ranges
            .iter()
            .map(|range| range.end.saturating_sub(range.start))
            .sum::<usize>();
        if dirty_line_count > TEXT_INPUT_WRAP_DIRTY_SYNC_LINE_LIMIT {
            self.request_wrap_recompute();
            return false;
        }

        let width_key = wrap_width_cache_key(rounded_wrap_width);
        let wrap_columns = wrap_columns_for_width(rounded_wrap_width, font_size);
        let job_accepts_interpolation = pending_wrap_job_accepts_interpolated_patch(
            self.wrap.pending_job.as_ref(),
            width_key,
            line_count,
            allow_interpolated_patches,
        );
        let mut changed = false;
        for range in ranges.drain(..) {
            for line_ix in range {
                // Dirty wrap patches only need updated row counts here; the
                // visible-row pass below shapes whichever lines enter view.
                let new_rows = estimate_wrap_rows_for_line(
                    line_text_for_index(display_text, line_starts, line_ix),
                    wrap_columns,
                )
                .max(1);
                let old_rows = self.wrap.row_counts[line_ix].max(1);
                if old_rows != new_rows {
                    self.wrap.row_counts[line_ix] = new_rows;
                    changed = true;
                    if job_accepts_interpolation {
                        self.push_interpolated_wrap_patch(width_key, line_ix, old_rows, new_rows);
                    }
                }
            }
        }
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
    ) -> bool {
        let width_key = wrap_width_cache_key(rounded_wrap_width);
        let wrap_columns = wrap_columns_for_width(rounded_wrap_width, font_size);
        if line_count <= TEXT_INPUT_WRAP_SYNC_LINE_THRESHOLD {
            self.wrap.pending_job = None;
            self.wrap.interpolated_patches.clear();
            estimate_wrap_rows_with_line_starts(
                display_text,
                line_starts,
                wrap_columns,
                &mut self.wrap.row_counts,
            );
            self.wrap.recompute_requested = false;
            return false;
        }

        let has_compatible_job = self
            .wrap
            .pending_job
            .map(|job| job.width_key == width_key && job.line_count == line_count)
            .unwrap_or(false);
        if has_compatible_job && !self.wrap.recompute_requested {
            return false;
        }
        if !self.wrap.recompute_requested {
            return false;
        }

        let mut budget_rows = std::mem::take(&mut self.wrap.row_counts);
        budget_rows.resize(line_count, 1);
        estimate_wrap_rows_budgeted(
            display_text,
            line_starts,
            wrap_columns,
            &mut budget_rows,
            Duration::from_millis(TEXT_INPUT_WRAP_FOREGROUND_BUDGET_MS),
        );
        self.wrap.row_counts = budget_rows;
        self.wrap.row_counts_width = Some(rounded_wrap_width);
        self.wrap.recompute_requested = false;

        let sequence = self.wrap.recompute_sequence.wrapping_add(1).max(1);
        self.wrap.recompute_sequence = sequence;
        self.wrap.pending_job = Some(PendingWrapJob {
            sequence,
            width_key,
            line_count,
            wrap_columns,
        });
        self.wrap.interpolated_patches.clear();

        let snapshot = display_text.to_string();
        cx.spawn(
            async move |input: gpui::WeakEntity<TextInput>, cx: &mut gpui::AsyncApp| {
                let rows =
                    smol::unblock(move || estimate_wrap_rows_for_text(&snapshot, wrap_columns))
                        .await;
                let _ = input.update(cx, |input, cx| {
                    input.complete_wrap_recompute_job(sequence, width_key, line_count, rows, cx);
                });
            },
        )
        .detach();
        true
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
        for patch in &self.wrap.interpolated_patches {
            if patch.width_key == width_key {
                apply_interpolated_wrap_patch_delta(rows.as_mut_slice(), patch);
            }
        }
        self.wrap.interpolated_patches.clear();
        self.wrap.row_counts = rows;
        self.wrap.pending_job = None;
        self.wrap.last_rows = Some(total_wrap_rows(self.wrap.row_counts.as_slice()));
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

    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    pub fn set_soft_wrap(&mut self, soft_wrap: bool, cx: &mut Context<Self>) {
        if self.soft_wrap == soft_wrap {
            return;
        }
        self.soft_wrap = soft_wrap;
        self.invalidate_layout_caches();
        if soft_wrap {
            self.request_wrap_recompute();
        }
        if !soft_wrap {
            self.wrap.last_rows = None;
        }
        cx.notify();
    }

    pub fn set_masked(&mut self, masked: bool, cx: &mut Context<Self>) {
        if self.masked == masked {
            return;
        }
        self.masked = masked;
        self.invalidate_layout_caches();
        if self.multiline && self.soft_wrap {
            self.request_wrap_recompute();
        }
        cx.notify();
    }

    pub fn set_line_ending(&mut self, line_ending: &'static str) {
        self.line_ending = line_ending;
    }

    /// Detect line ending from file content. Returns `\r\n` if CRLF is found,
    /// otherwise falls back to the OS default (`\n` on Unix, `\r\n` on Windows).
    pub fn detect_line_ending(content: &str) -> &'static str {
        if content.contains("\r\n") || cfg!(windows) {
            "\r\n"
        } else {
            "\n"
        }
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

    pub(super) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selection.range.start, cx)
        }
        self.queue_cursor_autoscroll();
    }

    pub(super) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.previous_word_start(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.next_word_end(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) = self.vertical_move_target(
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

    pub(super) fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) = self.vertical_move_target(
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

    pub(super) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) = self.vertical_move_target(
            self.cursor_offset(),
            -1.0,
            self.interaction.vertical_motion_x,
        ) else {
            return;
        };
        self.select_to(target, cx);
        self.interaction.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) = self.vertical_move_target(
            self.cursor_offset(),
            1.0,
            self.interaction.vertical_motion_x,
        ) else {
            return;
        };
        self.select_to(target, cx);
        self.interaction.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text(cx);
    }

    pub(super) fn row_start(&self, offset: usize) -> usize {
        self.row_boundaries(offset).0
    }

    pub(super) fn row_end(&self, offset: usize) -> usize {
        self.row_boundaries(offset).1
    }

    pub(super) fn logical_row_boundaries(&self, offset: usize) -> (usize, usize) {
        let s = self.content.as_ref();
        let offset = offset.min(s.len());
        let start = s[..offset].rfind('\n').map(|ix| ix + 1).unwrap_or(0);
        let rel_end = s[offset..].find('\n').unwrap_or(s.len() - offset);
        let end = offset + rel_end;
        (start, end)
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

        let mut row_end_indices: Vec<usize> = Vec::with_capacity(line.wrap_boundaries().len() + 1);
        for boundary in line.wrap_boundaries() {
            let Some(run) = line.unwrapped_layout.runs.get(boundary.run_ix) else {
                continue;
            };
            let Some(glyph) = run.glyphs.get(boundary.glyph_ix) else {
                continue;
            };
            row_end_indices.push(glyph.index);
        }
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

    pub(super) fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_start(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_start(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_end(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_end(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn caret_point_for_hit_testing(&self, cursor: usize) -> Option<Point<Pixels>> {
        let bounds = self.layout.bounds?;
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
        let bounds = self.layout.bounds?;
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
        let Some(text_bounds) = self.layout.bounds else {
            return;
        };
        let viewport_height = handle.bounds().size.height.max(px(0.0));
        if viewport_height <= px(0.0) {
            return;
        }
        let caret_margin = px(10.0);

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
        let negative_axis = current.y < px(0.0);
        let mut scroll_y = if negative_axis { -current.y } else { current.y };

        let max_offset = handle.max_offset().y.max(px(0.0));
        if max_offset <= px(0.0) {
            let cursor_out_of_view = cursor_top < scroll_y + caret_margin
                || cursor_bottom > scroll_y + viewport_height - caret_margin;
            if self.cursor_offset() == self.content.len() {
                handle.scroll_to_bottom();
                cx.notify();
                self.interaction.pending_cursor_autoscroll = true;
            } else if cursor_out_of_view {
                cx.notify();
                self.interaction.pending_cursor_autoscroll = true;
            } else {
                self.interaction.pending_cursor_autoscroll = false;
            }
            return;
        }

        scroll_y = scroll_y.max(px(0.0)).min(max_offset);

        let target_scroll = if self.cursor_offset() == self.content.len() {
            max_offset
        } else if cursor_top < scroll_y + caret_margin {
            cursor_top - caret_margin
        } else if cursor_bottom > scroll_y + viewport_height - caret_margin {
            cursor_bottom - viewport_height + caret_margin
        } else {
            self.interaction.pending_cursor_autoscroll = false;
            return;
        }
        .max(px(0.0))
        .min(max_offset);

        if target_scroll == scroll_y {
            self.interaction.pending_cursor_autoscroll = false;
            return;
        }

        let next_y = if negative_axis {
            -target_scroll
        } else {
            target_scroll
        };
        handle.set_offset(point(current.x, next_y));
        self.interaction.pending_cursor_autoscroll = false;
        cx.notify();
    }

    pub(super) fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
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
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, preferred_x)) = self.page_move_target(
            self.cursor_offset(),
            -1.0,
            self.interaction.vertical_motion_x,
        ) else {
            return;
        };
        self.select_to(target, cx);
        self.interaction.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
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
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, preferred_x)) = self.page_move_target(
            self.cursor_offset(),
            1.0,
            self.interaction.vertical_motion_x,
        ) else {
            return;
        };
        self.select_to(target, cx);
        self.interaction.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    pub(super) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selection.range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    pub(super) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selection.range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
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
            self.select_to(self.previous_word_start(self.cursor_offset()), cx)
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
            self.select_to(self.next_word_end(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    pub(super) fn insert_line_break(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.queue_cursor_autoscroll();
        self.replace_text_in_range(None, self.line_ending, window, cx);
    }

    pub(super) fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || !self.multiline {
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

    pub(super) fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    pub(super) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selection.range.clone()].to_string(),
            ));
        }
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selection.range.clone()].to_string(),
            ));
            if !self.read_only {
                self.replace_text_in_range(None, "", window, cx)
            }
        }
    }

    pub(super) fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(snapshot) = self.selection.undo_stack.pop() else {
            return;
        };
        self.push_redo_snapshot(self.current_undo_snapshot());
        self.restore_undo_snapshot(snapshot, cx);
    }

    pub(super) fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(snapshot) = self.selection.redo_stack.pop() else {
            return;
        };
        self.push_undo_snapshot(self.current_undo_snapshot());
        self.restore_undo_snapshot(snapshot, cx);
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
        let undo_snapshot = self.current_undo_snapshot();
        let range = self.normalized_utf8_range(range);
        let inserted = self.content.replace_range(range.clone(), new_text);
        self.push_undo_snapshot(undo_snapshot);
        self.selection.redo_stack.clear();
        self.selection.pending_text_edit_delta = Some((range.clone(), inserted.clone()));
        let cursor = inserted.end;
        self.mark_wrap_dirty_from_edit(range, inserted.clone());
        self.selection.range = cursor..cursor;
        self.selection.reversed = false;
        self.selection.marked_range.take();
        self.interaction.vertical_motion_x = None;
        self.interaction.cursor_blink_visible = true;
        self.invalidate_layout_caches_preserving_wrap_rows();
        self.invalidate_provider_highlights_for_text_change();
        self.queue_cursor_autoscroll();
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

    /// Consume the latest UTF-8 edit delta as `(old_range, new_range)`.
    ///
    /// `old_range` references bytes in the pre-edit text; `new_range` references
    /// bytes in the post-edit text.
    pub fn take_recent_utf8_edit_delta(&mut self) -> Option<(Range<usize>, Range<usize>)> {
        self.selection.pending_text_edit_delta.take()
    }

    pub fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        self.index_for_position(position)
    }

    pub(super) fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = self.clamp_to_char_boundary(offset);
        self.selection.range = offset..offset;
        self.selection.reversed = false;
        self.interaction.vertical_motion_x = None;
        self.interaction.cursor_blink_visible = true;
        cx.notify();
    }

    pub(super) fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
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
        self.interaction.cursor_blink_visible = true;
        cx.notify();
    }

    pub(super) fn clamp_to_char_boundary(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.content.len());
        while offset > 0 && !self.content.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    pub(super) fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    pub(super) fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    pub(super) fn is_word_char(ch: char) -> bool {
        crate::text_selection::is_word_char(ch)
    }

    pub(super) fn current_undo_snapshot(&self) -> UndoSnapshot {
        UndoSnapshot {
            content: self.content.snapshot(),
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

    pub(super) fn restore_undo_snapshot(&mut self, snapshot: UndoSnapshot, cx: &mut Context<Self>) {
        self.content = snapshot.content.into();
        self.selection.range = snapshot.selected_range;
        self.selection.reversed = snapshot.selection_reversed;
        self.selection.marked_range = None;
        self.interaction.vertical_motion_x = None;
        self.interaction.cursor_blink_visible = true;
        self.interaction.is_selecting = false;
        self.invalidate_layout_caches();
        if self.multiline && self.soft_wrap {
            self.request_wrap_recompute();
        }
        self.selection.pending_text_edit_delta = None;
        self.invalidate_provider_highlights_for_text_change();
        self.queue_cursor_autoscroll();
        cx.notify();
    }

    pub(super) fn skip_left_while(
        s: &str,
        mut offset: usize,
        mut predicate: impl FnMut(char) -> bool,
    ) -> usize {
        offset = offset.min(s.len());
        while offset > 0 {
            let Some((idx, ch)) = s[..offset].char_indices().next_back() else {
                return 0;
            };
            if !predicate(ch) {
                break;
            }
            offset = idx;
        }
        offset
    }

    pub(super) fn skip_right_while(
        s: &str,
        mut offset: usize,
        mut predicate: impl FnMut(char) -> bool,
    ) -> usize {
        offset = offset.min(s.len());
        while offset < s.len() {
            let Some(ch) = s[offset..].chars().next() else {
                break;
            };
            if !predicate(ch) {
                break;
            }
            offset += ch.len_utf8();
        }
        offset
    }

    pub(super) fn previous_word_start(&self, offset: usize) -> usize {
        let s = self.content.as_ref();
        let mut offset = offset.min(s.len());

        // Skip any whitespace to the left of the cursor.
        offset = Self::skip_left_while(s, offset, |ch| ch.is_whitespace());

        // Skip punctuation/symbols (e.g. '.' '/' '-') so word navigation doesn't get stuck on them.
        offset = Self::skip_left_while(s, offset, |ch| {
            !ch.is_whitespace() && !Self::is_word_char(ch)
        });

        // Skip any whitespace again, then skip the word itself.
        offset = Self::skip_left_while(s, offset, |ch| ch.is_whitespace());
        Self::skip_left_while(s, offset, Self::is_word_char)
    }

    pub(super) fn next_word_end(&self, offset: usize) -> usize {
        let s = self.content.as_ref();
        let offset = offset.min(s.len());
        if offset >= s.len() {
            return s.len();
        }

        let Some(ch) = s[offset..].chars().next() else {
            return s.len();
        };

        if ch.is_whitespace() {
            return Self::skip_right_while(s, offset, |ch| ch.is_whitespace());
        }
        if Self::is_word_char(ch) {
            return Self::skip_right_while(s, offset, Self::is_word_char);
        }

        Self::skip_right_while(s, offset, |ch| {
            !ch.is_whitespace() && !Self::is_word_char(ch)
        })
    }

    pub(super) fn token_range_for_offset(&self, offset: usize) -> Range<usize> {
        crate::text_selection::token_range_for_offset(self.content.as_ref(), offset)
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
        cx.stop_propagation();
        window.focus(&self.focus_handle, cx);
        self.interaction.cursor_blink_visible = true;
        let index = self.index_for_mouse_position(event.position);
        self.interaction.vertical_motion_x = None;

        if event.modifiers.shift {
            self.interaction.is_selecting = true;
            self.select_to(index, cx);
            return;
        }

        if event.click_count >= 2 {
            self.interaction.is_selecting = false;
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
            self.move_to(index, cx)
        }
    }

    pub(super) fn on_mouse_up(
        &mut self,
        _: &MouseUpEvent,
        _window: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.interaction.is_selecting = false;
    }

    pub(super) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.interaction.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    pub(super) fn on_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.modified() {
            return;
        }

        if event.keystroke.key.as_str() == "escape" {
            self.interaction.escape_pressed = true;
            cx.notify();
        }
    }

    pub(super) fn on_mouse_down_right(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.interaction.suppress_right_click {
            return;
        }

        cx.stop_propagation();
        window.focus(&self.focus_handle, cx);
        self.interaction.cursor_blink_visible = true;
        self.interaction.is_selecting = false;
        self.interaction.vertical_motion_x = None;

        let index = self.index_for_mouse_position(event.position);
        let click_inside_selection = !self.selection.range.is_empty()
            && index >= self.selection.range.start
            && index <= self.selection.range.end;
        if !click_inside_selection {
            self.move_to(index, cx);
        }

        self.interaction.context_menu = Some(TextInputContextMenuState {
            can_paste: cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some(),
            anchor: event.position,
        });
        cx.notify();
    }

    pub(super) fn context_menu_entry_row(
        &self,
        label: &'static str,
        shortcut: SharedString,
        disabled: bool,
    ) -> Div {
        let mut row = div()
            .h(px(24.0))
            .w_full()
            .px_2()
            .rounded(px(2.0))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .text_sm()
            .child(label)
            .child(
                div()
                    .text_xs()
                    .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                    .text_color(self.style.placeholder)
                    .child(shortcut),
            );

        if disabled {
            row = row
                .text_color(self.style.placeholder)
                .cursor(CursorStyle::Arrow);
        } else {
            let hover = self.style.selection;
            row = row
                .cursor(CursorStyle::PointingHand)
                .hover(move |s| s.bg(hover));
        }

        row
    }

    pub(super) fn render_context_menu(
        &mut self,
        state: TextInputContextMenuState,
        cx: &mut Context<Self>,
    ) -> Div {
        let menu_ui_scale_percent = crate::ui_scale::current(cx).percent;
        let primary = primary_modifier_label();
        let undo_disabled = self.read_only || self.selection.undo_stack.is_empty();
        let redo_disabled = self.read_only || self.selection.redo_stack.is_empty();
        let cut_disabled = self.read_only || self.selection.range.is_empty();
        let copy_disabled = self.selection.range.is_empty();
        let paste_disabled = self.read_only || !state.can_paste;
        let delete_disabled = self.read_only || self.selection.range.is_empty();
        let select_all_disabled = self.content.is_empty();

        let mut undo_row =
            self.context_menu_entry_row("Undo", format!("{primary}+Z").into(), undo_disabled);
        if !undo_disabled {
            undo_row = undo_row.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.interaction.context_menu = None;
                    this.undo(&Undo, window, cx);
                    cx.notify();
                }),
            );
        }

        let mut redo_row =
            self.context_menu_entry_row("Redo", format!("{primary}+Shift+Z").into(), redo_disabled);
        if !redo_disabled {
            redo_row = redo_row.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.interaction.context_menu = None;
                    this.redo(&Redo, window, cx);
                    cx.notify();
                }),
            );
        }

        let mut cut_row =
            self.context_menu_entry_row("Cut", format!("{primary}+X").into(), cut_disabled);
        if !cut_disabled {
            cut_row = cut_row
                .debug_selector(|| "text_input_context_cut".to_string())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.interaction.context_menu = None;
                        this.cut(&Cut, window, cx);
                        cx.notify();
                    }),
                );
        } else {
            cut_row = cut_row.debug_selector(|| "text_input_context_cut".to_string());
        }

        let mut copy_row = self
            .context_menu_entry_row("Copy", format!("{primary}+C").into(), copy_disabled)
            .debug_selector(|| "text_input_context_copy".to_string());
        if !copy_disabled {
            copy_row = copy_row.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.interaction.context_menu = None;
                    this.copy(&Copy, window, cx);
                    cx.notify();
                }),
            );
        }

        let mut paste_row = self
            .context_menu_entry_row("Paste", format!("{primary}+V").into(), paste_disabled)
            .debug_selector(|| "text_input_context_paste".to_string());
        if !paste_disabled {
            paste_row = paste_row.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.interaction.context_menu = None;
                    this.paste(&Paste, window, cx);
                    cx.notify();
                }),
            );
        }

        let mut delete_row = self.context_menu_entry_row("Delete", "Del".into(), delete_disabled);
        if !delete_disabled {
            delete_row = delete_row.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.interaction.context_menu = None;
                    if !this.selection.range.is_empty() && !this.read_only {
                        this.replace_text_in_range(None, "", window, cx);
                    }
                    cx.notify();
                }),
            );
        }

        let mut select_all_row = self
            .context_menu_entry_row(
                "Select all",
                format!("{primary}+A").into(),
                select_all_disabled,
            )
            .debug_selector(|| "text_input_context_select_all".to_string());
        if !select_all_disabled {
            select_all_row = select_all_row.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.interaction.context_menu = None;
                    this.select_all(&SelectAll, window, cx);
                    cx.notify();
                }),
            );
        }

        div()
            .w(crate::ui_scale::design_px_from_percent(
                188.0,
                menu_ui_scale_percent,
            ))
            .p_1()
            .flex()
            .flex_col()
            .gap_0p5()
            .bg(with_alpha(self.style.background, 0.98))
            .border_1()
            .border_color(self.style.hover_border)
            .rounded(px(2.0))
            .shadow_lg()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _e: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|_this, _e: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .child(undo_row)
            .child(redo_row)
            .child(
                div()
                    .h(px(1.0))
                    .w_full()
                    .bg(with_alpha(self.style.border, 0.6)),
            )
            .child(cut_row)
            .child(copy_row)
            .child(paste_row)
            .child(delete_row)
            .child(
                div()
                    .h(px(1.0))
                    .w_full()
                    .bg(with_alpha(self.style.border, 0.6)),
            )
            .child(select_all_row)
    }

    pub(super) fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(layout), Some(starts)) = (
            self.layout.bounds.as_ref(),
            self.layout.last.as_ref(),
            self.layout.line_starts.as_ref(),
        ) else {
            return 0;
        };

        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }

        let line_height = if self.layout.line_height.is_zero() {
            px(16.0)
        } else {
            self.layout.line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let ratio = f32::from(position.y - bounds.top()) / f32::from(line_height);
                let mut line_ix = ratio.floor() as isize;
                line_ix = line_ix.clamp(0, lines.len().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let local_x = position.x - bounds.left() + self.layout.scroll_x;
                let local_ix = lines[line_ix].closest_index_for_x(local_x);
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

    pub(super) fn index_for_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(layout), Some(starts)) = (
            self.layout.bounds.as_ref(),
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
                line_ix = line_ix.clamp(0, lines.len().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let local_x = position.x - bounds.left() + self.layout.scroll_x;
                let local_ix = lines[line_ix].closest_index_for_x(local_x);
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

    pub(super) fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    pub(super) fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    pub(super) fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub(super) fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

impl EntityInputHandler for TextInput {
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

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.selection.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let Some(new_text) = self.sanitize_insert_text(new_text) else {
            return;
        };
        let undo_snapshot = self.current_undo_snapshot();

        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.selection.marked_range.clone())
            .unwrap_or(self.selection.range.clone());

        let inserted = self.content.replace_range(range.clone(), new_text.as_str());
        self.selection.pending_text_edit_delta = Some((range.clone(), inserted.clone()));
        self.mark_wrap_dirty_from_edit(range.clone(), inserted.clone());
        self.push_undo_snapshot(undo_snapshot);
        self.selection.range = inserted.end..inserted.end;
        self.selection.reversed = false;
        self.selection.marked_range.take();
        self.interaction.vertical_motion_x = None;
        self.interaction.cursor_blink_visible = true;
        self.invalidate_layout_caches_preserving_wrap_rows();
        self.invalidate_provider_highlights_for_text_change();
        self.queue_cursor_autoscroll();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let Some(new_text) = self.sanitize_insert_text(new_text) else {
            return;
        };
        let undo_snapshot = self.current_undo_snapshot();

        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.selection.marked_range.clone())
            .unwrap_or(self.selection.range.clone());

        let inserted = self.content.replace_range(range.clone(), new_text.as_str());
        self.selection.pending_text_edit_delta = Some((range.clone(), inserted.clone()));
        self.mark_wrap_dirty_from_edit(range.clone(), inserted.clone());
        self.push_undo_snapshot(undo_snapshot);
        if !new_text.is_empty() {
            self.selection.marked_range = Some(inserted.clone());
        } else {
            self.selection.marked_range = None;
        }
        self.selection.range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selection.reversed = false;

        self.interaction.vertical_motion_x = None;
        self.interaction.cursor_blink_visible = true;
        self.invalidate_layout_caches_preserving_wrap_rows();
        self.invalidate_provider_highlights_for_text_change();
        self.queue_cursor_autoscroll();
        cx.notify();
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
        let local = self.layout.bounds?.localize(&p)?;
        let layout = self.layout.last.as_ref()?;
        let starts = self.layout.line_starts.as_ref()?;
        let line_height = self.effective_line_height(window);
        match layout {
            TextInputLayout::Plain(lines) => {
                let mut line_ix = (local.y / line_height).floor() as isize;
                line_ix = line_ix.clamp(0, lines.len().saturating_sub(1) as isize);
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

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
