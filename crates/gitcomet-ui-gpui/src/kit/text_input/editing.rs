use super::highlight::*;
use super::shaping::*;
use super::state::*;
use super::wrap::*;
use super::*;

/// The single replaced span between two texts, as `(old_range, new_range)`.
///
/// Both ranges share a start and land on character boundaries, so the pair is
/// directly usable as a `replace_utf8_range` edit. `None` means the texts are
/// identical. Shared with callers that need to describe a wholesale rewrite as
/// one minimal edit rather than a full-buffer replacement.
pub(crate) fn utf8_edit_delta_between_texts(
    old_text: &str,
    new_text: &str,
) -> Option<(Range<usize>, Range<usize>)> {
    if old_text == new_text {
        return None;
    }

    let old = old_text.as_bytes();
    let new = new_text.as_bytes();
    let mut prefix = 0usize;
    while prefix < old.len().min(new.len()) && old[prefix] == new[prefix] {
        prefix += 1;
    }
    while prefix > 0 && (!old_text.is_char_boundary(prefix) || !new_text.is_char_boundary(prefix)) {
        prefix -= 1;
    }

    let mut suffix = 0usize;
    while suffix < old.len().saturating_sub(prefix)
        && suffix < new.len().saturating_sub(prefix)
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }
    while suffix > 0
        && (!old_text.is_char_boundary(old.len().saturating_sub(suffix))
            || !new_text.is_char_boundary(new.len().saturating_sub(suffix)))
    {
        suffix -= 1;
    }

    Some((
        prefix..old.len().saturating_sub(suffix),
        prefix..new.len().saturating_sub(suffix),
    ))
}

/// The span of a highlight source that covers a live window.
///
/// A deletion since the source was installed makes it longer than the buffer
/// over the edited span, so reach that much further or the bottom of the window
/// comes back short.
fn interpolated_source_window(
    interpolation: &HighlightInterpolation,
    byte_range: &Range<usize>,
) -> Range<usize> {
    let mut source_range = interpolation.to_source_range(byte_range);
    source_range.end = source_range
        .end
        .saturating_add(interpolation.source_lookahead());
    source_range
}

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
            leading_icon: options.leading_icon,
            multiline: options.multiline,
            read_only: options.read_only,
            chromeless: options.chromeless,
            soft_wrap: options.soft_wrap,
            min_lines: options.min_lines,
            display_truncation: None,
            masked: false,
            line_ending: if cfg!(windows) { "\r\n" } else { "\n" },
            style: TextInputStyle::from_theme(AppTheme::gitcomet_dark()),
            line_height_override: None,
            vertical_padding_override: None,
            highlight: HighlightState::new(),
            layout: LayoutState::new(),
            wrap: WrapState::new(),
            content_width_cache: None,
            selection: SelectionState::new(),
            interaction: InteractionState::new(),
            protected_ranges: Arc::from([]),
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

    /// Background syntax chunks landed for the text the provider already
    /// describes. Its tokens improved; the text it was built over did not, so
    /// the interpolation anchor must survive — resetting it here would snap
    /// every highlight to coordinates the buffer left behind.
    pub(super) fn note_provider_highlights_changed(&mut self) {
        self.highlight.interpolated_cache = None;
        self.invalidate_highlights(true);
    }

    /// Record a text edit against the highlights currently on screen.
    ///
    /// The highlight source is not recomputed here — that is debounced by the
    /// owner. Instead the edit is folded into the interpolation so the stale
    /// highlights keep pointing at the tokens they describe.
    pub(super) fn note_text_edit_for_highlights(
        &mut self,
        replaced: &Range<usize>,
        inserted: &Range<usize>,
    ) {
        if self.highlight.provider.is_none()
            && self.highlight.highlights.is_empty()
            && self.highlight.superseded.is_none()
        {
            return;
        }

        self.highlight.interpolation.record_edit(replaced, inserted);
        // A source held in reserve has to keep tracking the buffer too, or it
        // would answer in coordinates that stopped being true the moment it
        // was set aside.
        if let Some(superseded) = self.highlight.superseded.as_mut() {
            superseded.interpolation.record_edit(replaced, inserted);
        }
        self.highlight.interpolated_cache = None;
        self.highlight.prepaint_runs_cache = None;
    }

    /// Drop the accumulated edits because the highlight source was replaced by
    /// one built over the buffer's current text.
    fn reset_highlight_interpolation(&mut self) {
        self.highlight.interpolation.reset();
        self.highlight.interpolated_cache = None;
    }

    /// Set the outgoing highlight source aside so it can cover for its
    /// replacement until that replacement has tokens to show. See
    /// `SupersededHighlights`.
    fn supersede_current_highlight_source(&mut self) {
        if !self.highlight.answered {
            // This source never settled either, so it is no better a fallback
            // than the incoming one. Keep whatever reserve is already held.
            return;
        }
        if self.highlight.provider.is_none() && self.highlight.highlights.is_empty() {
            self.highlight.superseded = None;
            return;
        }

        self.highlight.superseded = Some(SupersededHighlights {
            provider: self.highlight.provider.clone(),
            highlights: Arc::clone(&self.highlight.highlights),
            interpolation: std::mem::take(&mut self.highlight.interpolation),
        });
    }

    /// The highlights covering `byte_range`, in the buffer's live coordinates.
    ///
    /// Collapses the provider and static-vector paths so both interpolate: a
    /// text edit moves highlights published through `set_highlights` exactly as
    /// it moves a provider's.
    pub(super) fn effective_highlights_for_window(
        &mut self,
        byte_range: Range<usize>,
    ) -> ResolvedProviderHighlights {
        let resolved = self.resolve_current_highlight_source(&byte_range);
        if !resolved.pending {
            // The source has settled, so it is now the truth and is fit to
            // stand in for its own replacement later.
            self.highlight.answered = true;
            self.highlight.superseded = None;
            return resolved;
        }

        // Still waiting on tokens. Rather than paint the viewport in the base
        // color for a frame or two, answer from the source this one replaced,
        // carried across the edits since by its own interpolation.
        let Some(superseded) = self.highlight.superseded.take() else {
            return resolved;
        };
        let stale = self.resolve_superseded_highlight_source(&superseded, &byte_range);
        self.highlight.superseded = Some(superseded);
        ResolvedProviderHighlights {
            // Keep the caller polling: this is a stopgap, not the answer.
            pending: true,
            highlights: stale,
        }
    }

    fn resolve_current_highlight_source(
        &mut self,
        byte_range: &Range<usize>,
    ) -> ResolvedProviderHighlights {
        if self.highlight.interpolation.is_exact() {
            return if self.highlight.provider.is_some() {
                self.resolve_provider_highlights(byte_range.start, byte_range.end)
            } else {
                ResolvedProviderHighlights {
                    pending: false,
                    highlights: Arc::clone(&self.highlight.highlights),
                }
            };
        }

        if let Some(cache) = self.highlight.interpolated_cache.as_ref()
            && cache.highlight_epoch == self.highlight.epoch
            && cache.interpolation_generation == self.highlight.interpolation.generation()
            && cache.byte_start <= byte_range.start
            && cache.byte_end >= byte_range.end
        {
            return ResolvedProviderHighlights {
                pending: cache.pending,
                highlights: Arc::clone(&cache.highlights),
            };
        }

        let source_range = interpolated_source_window(&self.highlight.interpolation, byte_range);
        let source = if self.highlight.provider.is_some() {
            self.resolve_provider_highlights(source_range.start, source_range.end)
        } else {
            ResolvedProviderHighlights {
                pending: false,
                highlights: Arc::clone(&self.highlight.highlights),
            }
        };
        let highlights = Arc::new(
            self.highlight
                .interpolation
                .map_highlights(source.highlights.as_slice(), self.content.len()),
        );
        self.debug_assert_highlights_on_char_boundaries(&highlights);

        self.highlight.interpolated_cache = Some(InterpolatedHighlightCache {
            highlight_epoch: self.highlight.epoch,
            interpolation_generation: self.highlight.interpolation.generation(),
            byte_start: byte_range.start,
            byte_end: byte_range.end,
            pending: source.pending,
            highlights: Arc::clone(&highlights),
        });
        ResolvedProviderHighlights {
            pending: source.pending,
            highlights,
        }
    }

    /// Answer a window from the source that is being replaced.
    ///
    /// Deliberately uncached: `provider_cache` belongs to the source that
    /// replaced this one, and mixing two providers' answers under one key would
    /// outlive the handoff. This runs for the frame or two the replacement
    /// needs to build its tokens.
    fn resolve_superseded_highlight_source(
        &self,
        superseded: &SupersededHighlights,
        byte_range: &Range<usize>,
    ) -> Arc<Vec<(Range<usize>, gpui::HighlightStyle)>> {
        let source_range = interpolated_source_window(&superseded.interpolation, byte_range);
        let highlights = match superseded.provider.as_ref() {
            Some(provider) => {
                let mut resolved = provider.resolve(source_range).highlights;
                resolved.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
                superseded
                    .interpolation
                    .map_highlights(&resolved, self.content.len())
            }
            None => superseded
                .interpolation
                .map_highlights(superseded.highlights.as_slice(), self.content.len()),
        };
        let highlights = Arc::new(highlights);
        self.debug_assert_highlights_on_char_boundaries(&highlights);
        highlights
    }

    /// A bound landing mid-character would corrupt the byte-length runs
    /// `text_run_for_style` builds from these ranges.
    #[inline]
    fn debug_assert_highlights_on_char_boundaries(
        &self,
        highlights: &[(Range<usize>, gpui::HighlightStyle)],
    ) {
        #[cfg(debug_assertions)]
        {
            let text = self.content.as_ref();
            for (range, _) in highlights {
                debug_assert!(
                    text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
                    "interpolated highlight {range:?} must land on character boundaries"
                );
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = highlights;
    }

    #[cfg(test)]
    pub(crate) fn debug_effective_highlights_for_range(
        &mut self,
        byte_range: Range<usize>,
    ) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
        self.effective_highlights_for_window(byte_range)
            .highlights
            .as_ref()
            .clone()
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

    pub fn set_chromeless(&mut self, chromeless: bool, cx: &mut Context<Self>) {
        if self.chromeless == chromeless {
            return;
        }
        self.chromeless = chromeless;
        self.invalidate_layout_caches();
        cx.notify();
    }

    pub fn set_leading_icon(&mut self, leading_icon: Option<&'static str>, cx: &mut Context<Self>) {
        if self.leading_icon == leading_icon {
            return;
        }
        self.leading_icon = leading_icon;
        self.invalidate_layout_caches();
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn debug_text_color(&self) -> gpui::Hsla {
        self.style.text
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        if self.content.as_ref() == text.as_ref() {
            return;
        }
        // Computed before the overwrite so highlights already on screen can ride
        // along; the equality check above keeps this O(n) scan off the hot path.
        let text_edit_delta = utf8_edit_delta_between_texts(self.content.as_ref(), text.as_ref());
        self.content.set_text(text.as_ref());
        self.protected_ranges = Arc::from([]);
        self.rebuild_content_width_cache_if_present();
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
        self.selection.pending_text_edit_deltas.clear();
        if let Some((replaced, inserted)) = text_edit_delta {
            self.note_text_edit_for_highlights(&replaced, &inserted);
        }
        cx.notify();
    }

    pub fn set_highlights(
        &mut self,
        mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
        cx: &mut Context<Self>,
    ) {
        highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        if self.highlight.provider.is_none()
            && self.highlight.highlights.as_slice() == highlights.as_slice()
        {
            return;
        }
        self.supersede_current_highlight_source();
        self.highlight.highlights = Arc::new(highlights);
        self.highlight.provider = None;
        self.highlight.provider_binding_key = None;
        self.highlight.provider_poll_task.take();
        // A materialized vector answers the moment it is published.
        self.highlight.answered = true;
        self.highlight.superseded = None;
        // A fresh highlight source describes the buffer as it stands now.
        self.reset_highlight_interpolation();
        self.invalidate_highlights(false);
        cx.notify();
    }

    #[allow(dead_code)]
    pub fn set_selected_range(
        &mut self,
        range: Range<usize>,
        autoscroll: bool,
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

        self.selection.range = next;
        self.selection.reversed = false;
        self.interaction.vertical_motion_x = None;
        self.interaction.cursor_blink_visible = true;
        if autoscroll {
            self.queue_cursor_autoscroll();
        }
        cx.notify();
    }

    /// `source_len` is the byte length of the text the provider answers in.
    /// Highlight interpolation anchors edits to that text, so a provider built
    /// over a snapshot the buffer has since moved past would be mapped with the
    /// wrong origin — callers must read the buffer immediately before building.
    pub(super) fn install_highlight_provider(
        &mut self,
        provider: HighlightProvider,
        binding_key: Option<u64>,
        source_len: usize,
        cx: &mut Context<Self>,
    ) {
        debug_assert_eq!(
            source_len,
            self.content.len(),
            "a highlight provider must describe the buffer's current text"
        );

        if !should_reset_highlight_provider_binding(
            self.highlight.provider.is_some(),
            self.highlight.provider_binding_key,
            binding_key,
        ) {
            return;
        }

        // A provider over a freshly prepared document cannot answer until its
        // token chunks are built, so hold the outgoing source to cover the gap.
        self.supersede_current_highlight_source();
        self.highlight.provider = Some(provider);
        self.highlight.provider_binding_key = binding_key;
        self.highlight.provider_poll_task.take();
        self.highlight.highlights = Arc::new(Vec::new());
        self.highlight.answered = false;
        // Only past the early return: an unchanged binding key means the same
        // closure over the same text, so its anchor must survive.
        self.reset_highlight_interpolation();
        self.invalidate_highlights(false);
        cx.notify();
    }

    /// Replace the full highlight vector with a lazy provider that generates
    /// highlights on demand for only the visible byte range. Use this for large
    /// documents where materializing all highlights is wasteful.
    ///
    /// `binding_key` identifies the source the provider speaks for. Reinstalling
    /// under the same key keeps the existing highlight cache; a new key resets
    /// it, along with the edit interpolation that was tracking the old source.
    ///
    /// `source_len` is the byte length of the text the provider was built over;
    /// see `install_highlight_provider`.
    pub fn set_highlight_provider_with_key(
        &mut self,
        binding_key: u64,
        provider: HighlightProvider,
        source_len: usize,
        cx: &mut Context<Self>,
    ) {
        self.install_highlight_provider(provider, Some(binding_key), source_len, cx);
    }

    pub fn set_line_height(&mut self, line_height: Option<Pixels>, cx: &mut Context<Self>) {
        if self.line_height_override == line_height {
            return;
        }
        self.line_height_override = line_height;
        cx.notify();
    }

    pub fn set_vertical_padding(&mut self, padding: Option<Pixels>, cx: &mut Context<Self>) {
        if self.vertical_padding_override == padding {
            return;
        }
        self.vertical_padding_override = padding;
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
        self.interaction.arrow_up_pressed = false;
        self.interaction.arrow_down_pressed = false;
        self.interaction.tab_pressed = false;
        self.interaction.shift_tab_pressed = false;
    }

    pub fn take_arrow_up_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.arrow_up_pressed)
    }

    pub fn take_arrow_down_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.arrow_down_pressed)
    }

    pub fn take_tab_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.tab_pressed)
    }

    pub fn take_shift_tab_pressed(&mut self) -> bool {
        std::mem::take(&mut self.interaction.shift_tab_pressed)
    }

    pub fn set_submit_on_enter(&mut self, submit_on_enter: bool) {
        self.interaction.submit_on_enter = submit_on_enter;
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

    /// Refuse edits to these byte spans, each covering a whole line including
    /// its terminator. Spans must be sorted and disjoint. Cleared by
    /// [`Self::set_text`], since the offsets describe the buffer that was
    /// replaced; the owner re-publishes them for the new one.
    pub fn set_protected_ranges(&mut self, ranges: Arc<[Range<usize>]>) {
        self.protected_ranges = ranges;
    }

    #[cfg(test)]
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

        let content = self.content.as_ref();
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
                        && content
                            .as_bytes()
                            .get(range.start.saturating_sub(1))
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

    /// Enable content-width layout: a multiline input lays out at its widest-line
    /// width so an outer `overflow_scroll` container can scroll it horizontally
    /// and drive a real horizontal `max_offset` on the shared scroll handle.
    pub fn set_content_width_layout(&mut self, enabled: bool) {
        if enabled && self.content_width_cache.is_none() {
            self.rebuild_content_width_cache();
        }
        self.interaction.content_width_layout = enabled;
    }

    fn content_width_line_units(text: &str, line_starts: &[usize], line_ix: usize) -> usize {
        let start = line_starts.get(line_ix).copied().unwrap_or_default();
        let end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        let line = text.get(start.min(end)..end).unwrap_or_default();
        line.len().max(line_display_columns(line))
    }

    fn content_width_affected_lines(
        line_starts: &[usize],
        text_len: usize,
        byte_range: Range<usize>,
    ) -> Range<usize> {
        let line_count = line_starts.len().max(1);
        let line_for_offset = |offset: usize| {
            line_starts
                .partition_point(|&start| start <= offset.min(text_len))
                .saturating_sub(1)
                .min(line_count.saturating_sub(1))
        };
        let start = line_for_offset(byte_range.start);
        let end = line_for_offset(byte_range.end);
        start..end.saturating_add(1).min(line_count)
    }

    fn rebuild_content_width_cache(&mut self) {
        let text = self.content.as_str();
        let starts = self.content.line_starts();
        let mut cache = ContentWidthCache::default();
        cache.line_units.reserve(starts.len().max(1));
        for line_ix in 0..starts.len().max(1) {
            let units = Self::content_width_line_units(text, starts, line_ix);
            cache.line_units.push(units);
            *cache.unit_counts.entry(units).or_default() += 1;
        }
        self.content_width_cache = Some(cache);
    }

    fn rebuild_content_width_cache_if_present(&mut self) {
        if self.content_width_cache.is_some() {
            self.rebuild_content_width_cache();
        }
    }

    fn replace_content_range(&mut self, range: Range<usize>, new_text: &str) -> Range<usize> {
        let old_affected = self.content_width_cache.as_ref().map(|_| {
            Self::content_width_affected_lines(
                self.content.line_starts(),
                self.content.len(),
                range.clone(),
            )
        });
        let inserted = self.content.replace_range(range, new_text);
        let Some(old_affected) = old_affected else {
            return inserted;
        };

        let new_affected = Self::content_width_affected_lines(
            self.content.line_starts(),
            self.content.len(),
            inserted.clone(),
        );
        let text = self.content.as_str();
        let starts = self.content.line_starts();
        let replacement_units = new_affected
            .clone()
            .map(|line_ix| Self::content_width_line_units(text, starts, line_ix))
            .collect::<Vec<_>>();
        let cache = self
            .content_width_cache
            .as_mut()
            .expect("content-width cache was present before edit");
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
        debug_assert_eq!(cache.line_units.len(), starts.len().max(1));
        inserted
    }

    pub(super) fn content_width_max_units(&self) -> usize {
        self.content_width_cache
            .as_ref()
            .map(ContentWidthCache::max_units)
            .unwrap_or_default()
    }

    pub(super) fn queue_cursor_autoscroll(&mut self) {
        self.interaction.pending_cursor_autoscroll = true;
        self.interaction.cursor_autoscroll_retry_exhausted = false;
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
                // Route the poll delay through gpui's executor rather than
                // `smol::Timer`: the smol timer drives on the global async-io
                // reactor thread, which breaks the deterministic test scheduler
                // (it asserts against cross-thread activity at teardown).
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;

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
            && cache.interpolation_generation == self.highlight.interpolation.generation()
            && cache.shape_style_epoch == self.layout.shape_style_epoch
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
            interpolation_generation: self.highlight.interpolation.generation(),
            shape_style_epoch: self.layout.shape_style_epoch,
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
        let shaped =
            window
                .text_system()
                .shape_line(capped_text, shape_style.font_size, runs, None);
        self.layout.plain_line_cache.insert(key, shaped.clone());
        self.trim_shape_caches();
        shaped
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

    #[allow(dead_code)]
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
        // max_offset is one frame stale when content just grew. If the cursor isn't
        // actually in view at target_scroll, allow one retry so the next frame can use
        // the updated max_offset. After that single retry we always stop: when the cursor
        // is at end-of-document cursor_bottom equals the content height, which is always
        // outside the caret_margin zone, so without a retry cap this would loop forever.
        let cursor_will_be_visible = cursor_top >= target_scroll + caret_margin
            && cursor_bottom <= target_scroll + viewport_height - caret_margin;
        if !cursor_will_be_visible && !self.interaction.cursor_autoscroll_retry_exhausted {
            self.interaction.pending_cursor_autoscroll = true;
            self.interaction.cursor_autoscroll_retry_exhausted = true;
        } else {
            self.interaction.pending_cursor_autoscroll = false;
        }
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
        if let Some(text) = crate::clipboard::read_text(cx) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_with_source(crate::clipboard::CopySource::TextInputShortcut, cx);
    }

    fn copy_with_source(&self, source: crate::clipboard::CopySource, cx: &mut Context<Self>) {
        if !self.selection.range.is_empty() {
            crate::clipboard::write_text(
                cx,
                self.content[self.selection.range.clone()].to_string(),
                source,
            );
        }
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.cut_with_source(crate::clipboard::CopySource::TextInputShortcut, window, cx);
    }

    fn cut_with_source(
        &mut self,
        source: crate::clipboard::CopySource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selection.range.is_empty() {
            self.copy_with_source(source, cx);
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
        self.replace_utf8_range_internal_with_view(range, new_text, false, cx)
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

    fn replace_utf8_range_internal_with_view(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        preserve_view: bool,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        let undo_snapshot = self.current_undo_snapshot();
        let range = self.normalized_utf8_range(range);
        let previous_selection = self.selection.range.clone();
        let previous_reversed = self.selection.reversed;
        let inserted = self.replace_content_range(range.clone(), new_text);
        self.shift_protected_ranges_for_edit(&range, &inserted);
        self.push_undo_snapshot(undo_snapshot);
        self.selection.redo_stack.clear();
        self.selection
            .pending_text_edit_deltas
            .push((range.clone(), inserted.clone()));
        let cursor = inserted.end;
        self.mark_wrap_dirty_from_edit(range.clone(), inserted.clone());
        if preserve_view {
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
        } else {
            self.selection.range = cursor..cursor;
            self.selection.reversed = false;
        }
        self.selection.marked_range.take();
        self.interaction.vertical_motion_x = None;
        self.interaction.cursor_blink_visible = true;
        self.invalidate_layout_caches_preserving_wrap_rows();
        self.note_text_edit_for_highlights(&range, &inserted);
        if !preserve_view {
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
        self.replace_utf8_range_internal_with_view(range, &new_text, true, cx)
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
        let text_edit_delta =
            utf8_edit_delta_between_texts(self.content.as_ref(), snapshot.content.as_ref());
        self.content = snapshot.content.into();
        // The spans described the buffer this snapshot just replaced; the owner
        // republishes them for the restored one.
        self.protected_ranges = Arc::from([]);
        self.rebuild_content_width_cache_if_present();
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
        if let Some(delta) = text_edit_delta {
            self.note_text_edit_for_highlights(&delta.0, &delta.1);
            self.selection.pending_text_edit_deltas.push(delta);
        }
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

    fn valid_hotspot_range(&self, range: &Range<usize>) -> bool {
        range.start < range.end && range.end <= self.content.len()
    }

    fn offset_inside_hotspot(range: &Range<usize>, offset: usize) -> bool {
        offset >= range.start && offset < range.end
    }

    fn wrapped_line_for_offset(
        starts: &[usize],
        lines: &[WrappedLine],
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
            self.layout.bounds.as_ref(),
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
        let bounds = self.layout.bounds?;
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
        _event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
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
            let index = self.index_for_mouse_position(event.position);
            self.select_to(index, cx);
        }
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

    pub(super) fn on_mouse_down_right(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.interaction.suppress_right_click {
            return;
        }

        crate::press_gesture::claim_press(cx);
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
            can_paste: crate::clipboard::read_text(cx).is_some(),
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
                        this.cut_with_source(
                            crate::clipboard::CopySource::TextInputContextMenu,
                            window,
                            cx,
                        );
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
                cx.listener(|this, _e: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                    this.interaction.context_menu = None;
                    this.copy_with_source(crate::clipboard::CopySource::TextInputContextMenu, cx);
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
            .rounded(px(10.0))
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
        if self.edit_alters_protected_range(&range, new_text.as_str()) {
            return;
        }

        let inserted = self.replace_content_range(range.clone(), new_text.as_str());
        self.shift_protected_ranges_for_edit(&range, &inserted);
        self.selection
            .pending_text_edit_deltas
            .push((range.clone(), inserted.clone()));
        self.mark_wrap_dirty_from_edit(range.clone(), inserted.clone());
        self.push_undo_snapshot(undo_snapshot);
        self.selection.range = inserted.end..inserted.end;
        self.selection.reversed = false;
        self.selection.marked_range.take();
        self.interaction.vertical_motion_x = None;
        self.interaction.cursor_blink_visible = true;
        self.invalidate_layout_caches_preserving_wrap_rows();
        self.note_text_edit_for_highlights(&range, &inserted);
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
        if self.edit_alters_protected_range(&range, new_text.as_str()) {
            return;
        }

        let inserted = self.replace_content_range(range.clone(), new_text.as_str());
        self.shift_protected_ranges_for_edit(&range, &inserted);
        self.selection
            .pending_text_edit_deltas
            .push((range.clone(), inserted.clone()));
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
        self.note_text_edit_for_highlights(&range, &inserted);
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

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
