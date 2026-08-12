use super::shaping::with_alpha;
use super::*;

// Text or display-mode changes always clear shaped-row caches, so cache keys
// only need the line index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct ShapedRowCacheKey {
    pub(super) line_ix: usize,
    /// Rounded font size: zoom changes reshape instead of replaying stale
    /// lines (most visible on never-edited placeholder text).
    pub(super) font_size_key: i32,
}

#[derive(Clone, Default)]
pub struct HighlightProviderResult {
    pub highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    pub pending: bool,
}

#[derive(Clone)]
pub struct HighlightProvider {
    pub(super) resolve: Arc<dyn Fn(Range<usize>) -> HighlightProviderResult + Send + Sync>,
    pub(super) drain_pending: Arc<dyn Fn() -> usize + Send + Sync>,
    pub(super) has_pending: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl HighlightProvider {
    #[cfg(test)]
    pub fn from_fn<F>(resolve: F) -> Self
    where
        F: Fn(Range<usize>) -> Vec<(Range<usize>, gpui::HighlightStyle)> + Send + Sync + 'static,
    {
        Self {
            resolve: Arc::new(move |range| HighlightProviderResult {
                highlights: resolve(range),
                pending: false,
            }),
            drain_pending: Arc::new(|| 0),
            has_pending: Arc::new(|| false),
        }
    }

    pub fn with_pending<R, D, H>(resolve: R, drain_pending: D, has_pending: H) -> Self
    where
        R: Fn(Range<usize>) -> HighlightProviderResult + Send + Sync + 'static,
        D: Fn() -> usize + Send + Sync + 'static,
        H: Fn() -> bool + Send + Sync + 'static,
    {
        Self {
            resolve: Arc::new(resolve),
            drain_pending: Arc::new(drain_pending),
            has_pending: Arc::new(has_pending),
        }
    }

    pub fn resolve(&self, range: Range<usize>) -> HighlightProviderResult {
        (self.resolve)(range)
    }

    pub(super) fn drain_pending(&self) -> usize {
        (self.drain_pending)()
    }

    pub(super) fn has_pending(&self) -> bool {
        (self.has_pending)()
    }
}

#[derive(Clone)]
pub(super) struct ProviderHighlightCacheEntry {
    pub(super) byte_start: usize,
    pub(super) byte_end: usize,
    pub(super) pending: bool,
    pub(super) highlights: Arc<Vec<(Range<usize>, gpui::HighlightStyle)>>,
}

impl ProviderHighlightCacheEntry {
    pub(super) fn contains_range(&self, byte_range: &Range<usize>) -> bool {
        self.byte_start <= byte_range.start && self.byte_end >= byte_range.end
    }

    pub(super) fn span_len(&self) -> usize {
        self.byte_end.saturating_sub(self.byte_start)
    }
}

/// Windows already fetched from the highlight provider, keyed by byte range.
///
/// Those ranges are in the provider's own *source* coordinates, not the
/// buffer's live ones — `HighlightInterpolation` maps between the two — so this
/// cache survives edits that have not yet reached the provider.
#[derive(Clone)]
pub(super) struct ProviderHighlightCache {
    pub(super) highlight_epoch: u64,
    pub(super) entries: Vec<ProviderHighlightCacheEntry>,
}

impl ProviderHighlightCache {
    pub(super) fn new(highlight_epoch: u64) -> Self {
        Self {
            highlight_epoch,
            entries: Vec::new(),
        }
    }

    pub(super) fn resolve(
        &mut self,
        highlight_epoch: u64,
        byte_range: &Range<usize>,
    ) -> Option<ResolvedProviderHighlights> {
        if self.highlight_epoch != highlight_epoch {
            self.highlight_epoch = highlight_epoch;
            self.entries.clear();
            return None;
        }

        let best_idx = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.contains_range(byte_range))
            .min_by_key(|(_, entry)| entry.span_len())
            .map(|(idx, _)| idx)?;

        if best_idx + 1 != self.entries.len() {
            let entry = self.entries.remove(best_idx);
            self.entries.push(entry);
        }

        let entry = self
            .entries
            .last()
            .expect("provider highlight cache should contain the requested entry");
        Some(ResolvedProviderHighlights {
            pending: entry.pending,
            highlights: Arc::clone(&entry.highlights),
        })
    }

    pub(super) fn insert(
        &mut self,
        highlight_epoch: u64,
        byte_range: Range<usize>,
        pending: bool,
        highlights: Arc<Vec<(Range<usize>, gpui::HighlightStyle)>>,
    ) {
        if self.highlight_epoch != highlight_epoch {
            self.highlight_epoch = highlight_epoch;
            self.entries.clear();
        }

        self.entries.retain(|entry| {
            entry.byte_start != byte_range.start || entry.byte_end != byte_range.end
        });
        self.entries.push(ProviderHighlightCacheEntry {
            byte_start: byte_range.start,
            byte_end: byte_range.end,
            pending,
            highlights,
        });
        if self.entries.len() > TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT {
            let overflow = self.entries.len() - TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT;
            self.entries.drain(0..overflow);
        }
    }
}

/// The single byte span separating "text the current highlight source still
/// describes" from "text edited since it was installed". A replace leaves the
/// prefix identical in both spaces, so one `start` suffices.
///
/// `old_len` is the span's length in source coordinates, `new_len` its length
/// in the buffer's live coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct HighlightEditPatch {
    pub(super) start: usize,
    pub(super) old_len: usize,
    pub(super) new_len: usize,
}

/// Keeps stale highlights pinned to their tokens between the edit that moved
/// them and the debounced recompute that catches up.
///
/// Mirrors `WrapState::interpolated_patches`: cheap, synchronous, applied on
/// every edit so rendering gets stale-but-positionally-correct highlights.
///
/// Deliberately one coalesced interval rather than a sorted disjoint list. A
/// run of single-character inserts at a fixed caret collapses into it exactly
/// (`old_len` stays 0 while `new_len` grows), which is the dominant typing
/// case. Two edits far apart widen the interval to their union, so the
/// untouched text between them renders in the base color instead of keeping
/// its highlights — still strictly better than the smear it replaces, and the
/// debounced recompute restores it. Upgrading to a disjoint list later is a
/// local change behind this same API.
#[derive(Debug, Default)]
pub(super) struct HighlightInterpolation {
    patch: Option<HighlightEditPatch>,
    generation: u64,
}

impl HighlightInterpolation {
    /// True while the highlight source still describes the buffer verbatim, so
    /// callers can hand its ranges straight through.
    pub(super) fn is_exact(&self) -> bool {
        self.patch.is_none()
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(super) fn debug_patch(&self) -> Option<HighlightEditPatch> {
        self.patch
    }

    /// Drop the accumulated edits. Only correct where the highlight source is
    /// itself replaced by one describing the buffer's current text.
    pub(super) fn reset(&mut self) {
        if self.patch.is_none() {
            return;
        }
        self.patch = None;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Fold one edit into the patch. `replaced` is in the pre-edit buffer's
    /// coordinates and `inserted` in the post-edit buffer's; they always share
    /// a start.
    pub(super) fn record_edit(&mut self, replaced: &Range<usize>, inserted: &Range<usize>) {
        debug_assert_eq!(
            replaced.start, inserted.start,
            "an edit replaces a span with text beginning at the same offset"
        );

        self.patch = Some(match self.patch {
            None => HighlightEditPatch {
                start: replaced.start,
                old_len: replaced.end.saturating_sub(replaced.start),
                new_len: inserted.end.saturating_sub(inserted.start),
            },
            Some(patch) => {
                // Widen to the union of the two edited spans, in the pre-edit
                // coordinates both are expressed in.
                let union_start = patch.start.min(replaced.start);
                let union_right = patch.start.saturating_add(patch.new_len).max(replaced.end);

                // `union_right >= patch.start + patch.new_len >= patch.new_len`
                // and `union_right >= replaced.end >= union_start + replaced.len()`,
                // so both subtractions below stay non-negative in this order.
                let source_right = union_right - patch.new_len + patch.old_len;
                let live_right =
                    union_right - (replaced.end - replaced.start) + (inserted.end - inserted.start);
                HighlightEditPatch {
                    start: union_start,
                    old_len: source_right.saturating_sub(union_start),
                    new_len: live_right.saturating_sub(union_start),
                }
            }
        });
        // Edits that cancel out — typing then undoing it — leave the source
        // describing the buffer verbatim again, so hand the fast path back.
        if self
            .patch
            .is_some_and(|patch| patch.old_len == 0 && patch.new_len == 0)
        {
            self.patch = None;
        }
        self.generation = self.generation.wrapping_add(1);
    }

    /// Translate a live buffer offset into the coordinates the highlight source
    /// still speaks. Monotone, and the identity outside the patch.
    pub(super) fn to_source_offset(&self, offset: usize) -> usize {
        let Some(patch) = self.patch else {
            return offset;
        };
        if offset <= patch.start {
            offset
        } else if offset >= patch.start.saturating_add(patch.new_len) {
            offset - patch.new_len + patch.old_len
        } else {
            // Inside the edited span, which the source describes with different
            // text: clamp to the span so the map stays monotone.
            patch
                .start
                .saturating_add((offset - patch.start).min(patch.old_len))
        }
    }

    pub(super) fn to_source_range(&self, range: &Range<usize>) -> Range<usize> {
        self.to_source_offset(range.start)..self.to_source_offset(range.end)
    }

    /// How much further into the source a live window may reach, so a deletion
    /// since install does not under-fetch the bottom of the visible window.
    pub(super) fn source_lookahead(&self) -> usize {
        self.patch
            .map(|patch| patch.old_len.saturating_sub(patch.new_len))
            .unwrap_or(0)
    }

    /// Rewrite source-coordinate highlights into live coordinates, clamped to
    /// `clamp_len`.
    ///
    /// A range straddling the edit is split rather than dropped. The stale
    /// window here lasts a full recompute debounce, so collapsing the range
    /// would make a block comment or string around the caret vanish for that
    /// whole time. The edited bytes themselves are left to the base color
    /// until the recompute catches up.
    pub(super) fn map_highlights(
        &self,
        source: &[(Range<usize>, gpui::HighlightStyle)],
        clamp_len: usize,
    ) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
        let Some(patch) = self.patch else {
            let mut mapped = source.to_vec();
            mapped.retain(|(range, _)| range.start < clamp_len && !range.is_empty());
            for (range, _) in mapped.iter_mut() {
                range.end = range.end.min(clamp_len);
            }
            return mapped;
        };

        let source_end = patch.start.saturating_add(patch.old_len);
        let mut mapped = Vec::with_capacity(source.len());
        let mut push = |range: Range<usize>, style: &gpui::HighlightStyle| {
            let range = range.start.min(clamp_len)..range.end.min(clamp_len);
            if range.is_empty() {
                return;
            }
            debug_assert!(
                range.start <= range.end,
                "interpolated highlight ranges must stay ordered"
            );
            mapped.push((range, *style));
        };

        for (range, style) in source {
            // Before the edit: identical in both spaces.
            if range.start < patch.start {
                push(range.start..range.end.min(patch.start), style);
            }
            // After the edit: shifted by the length delta.
            if range.end > source_end {
                let start = range.start.max(source_end) - patch.old_len + patch.new_len;
                let end = range.end - patch.old_len + patch.new_len;
                push(start..end, style);
            }
        }

        // Splitting breaks source order — a range's right piece can land after
        // a later range's left piece — and `HighlightCursor` binary-searches.
        mapped.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        mapped
    }
}

/// The highlight source a rebind replaced, kept until its replacement can
/// actually answer.
///
/// The rule is that the live highlight source is swapped from old tokens
/// straight to new ones, never through an empty state, so a query landing
/// mid-handoff gets stale colors rather than none.
///
/// A provider over a freshly prepared document needs that cover: its
/// token chunks are built in the background, so until they land it can only
/// answer with silence — which paints the viewport in the base color for a
/// frame or two. This keeps the outgoing source available to cover that gap.
pub(super) struct SupersededHighlights {
    pub(super) provider: Option<HighlightProvider>,
    pub(super) highlights: Arc<Vec<(Range<usize>, gpui::HighlightStyle)>>,
    /// Edits between the text this source describes and the buffer now. It goes
    /// on accumulating them while the source is held in reserve.
    pub(super) interpolation: HighlightInterpolation,
}

/// Highlights already mapped into live coordinates for one visible window.
///
/// Keyed in live coordinates, unlike `ProviderHighlightCache`, which is keyed
/// in the source's.
pub(super) struct InterpolatedHighlightCache {
    pub(super) highlight_epoch: u64,
    pub(super) interpolation_generation: u64,
    pub(super) byte_start: usize,
    pub(super) byte_end: usize,
    pub(super) pending: bool,
    pub(super) highlights: Arc<Vec<(Range<usize>, gpui::HighlightStyle)>>,
}

#[derive(Clone)]
pub(super) struct ResolvedProviderHighlights {
    pub(super) pending: bool,
    pub(super) highlights: Arc<Vec<(Range<usize>, gpui::HighlightStyle)>>,
}

pub(super) fn should_reset_highlight_provider_binding(
    has_existing_provider: bool,
    current_binding_key: Option<u64>,
    next_binding_key: Option<u64>,
) -> bool {
    match next_binding_key {
        Some(next_key) => !has_existing_provider || current_binding_key != Some(next_key),
        None => true,
    }
}

/// Text runs built for one visible window.
///
/// A text edit no longer bumps `highlight_epoch` — the highlights survive it by
/// interpolation — so the run identity has to name what actually changed:
/// which edits have been folded in, and the styling they were built with.
#[derive(Clone, Debug)]
pub(super) struct PrepaintHighlightRunsCache {
    pub(super) highlight_epoch: u64,
    pub(super) interpolation_generation: u64,
    pub(super) shape_style_epoch: u64,
    pub(super) visible_start: usize,
    pub(super) visible_end: usize,
    pub(super) line_runs: Arc<VisibleWindowTextRuns>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct VisibleWindowTextRuns {
    pub(super) line_offsets: Vec<usize>,
    pub(super) runs: Vec<TextRun>,
}

impl VisibleWindowTextRuns {
    pub(super) fn with_line_capacity(line_count: usize) -> Self {
        let mut line_offsets = Vec::with_capacity(line_count.saturating_add(1));
        line_offsets.push(0);
        Self {
            line_offsets,
            runs: Vec::with_capacity(
                line_count
                    .saturating_mul(TEXT_INPUT_STREAMED_HIGHLIGHT_ESTIMATED_RUNS_PER_VISIBLE_LINE),
            ),
        }
    }

    pub(super) fn finish_line(&mut self) {
        self.line_offsets.push(self.runs.len());
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(super) fn len(&self) -> usize {
        self.line_offsets.len().saturating_sub(1)
    }

    pub(super) fn line(&self, local_ix: usize) -> Option<&[TextRun]> {
        let start = *self.line_offsets.get(local_ix)?;
        let end = *self.line_offsets.get(local_ix.saturating_add(1))?;
        self.runs.get(start..end)
    }
}

#[derive(Clone, Copy)]
pub(super) struct TextShapeStyle<'a> {
    pub(super) base_font: &'a gpui::Font,
    pub(super) text_color: gpui::Hsla,
    pub(super) highlights: Option<&'a [(Range<usize>, gpui::HighlightStyle)]>,
    pub(super) font_size: Pixels,
}

#[derive(Clone, Copy)]
pub(super) struct LineShapeInput<'a> {
    pub(super) line_ix: usize,
    pub(super) line_start: usize,
    pub(super) line_text: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UndoSnapshot {
    pub(super) content: TextModelSnapshot,
    pub(super) selected_range: Range<usize>,
    pub(super) selection_reversed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TextInputStyle {
    pub(super) background: Rgba,
    pub(super) border: Rgba,
    pub(super) hover_border: Rgba,
    pub(super) focus_border: Rgba,
    pub(super) radius: f32,
    pub(super) text: gpui::Hsla,
    pub(super) placeholder: gpui::Hsla,
    pub(super) cursor: Rgba,
    pub(super) selection: Rgba,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TextInputContextMenuState {
    pub(super) can_paste: bool,
    pub(super) anchor: Point<Pixels>,
}

impl TextInputStyle {
    pub(super) fn from_theme(theme: AppTheme) -> Self {
        fn mix(mut a: Rgba, b: Rgba, t: f32) -> Rgba {
            let t = t.clamp(0.0, 1.0);
            a.r = a.r + (b.r - a.r) * t;
            a.g = a.g + (b.g - a.g) * t;
            a.b = a.b + (b.b - a.b) * t;
            a.a = a.a + (b.a - a.a) * t;
            a
        }

        // Ensure inputs look like inputs even in themes where `surface_bg` and `surface_bg_elevated`
        // are equal (Ayu/One).
        let background = if theme.is_dark {
            mix(
                theme.colors.surface_bg_elevated,
                gpui::rgba(0xFFFFFFFF),
                0.03,
            )
        } else {
            mix(
                theme.colors.surface_bg_elevated,
                gpui::rgba(0x000000FF),
                0.03,
            )
        };

        let base_border = theme.colors.border;
        let hover_border = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.55 } else { 0.40 },
        );
        let focus_border = with_alpha(theme.colors.accent, if theme.is_dark { 0.98 } else { 0.92 });
        Self {
            background,
            border: base_border,
            hover_border,
            focus_border,
            radius: theme.radii.control,
            text: theme.colors.text.into(),
            placeholder: theme.colors.input_placeholder.into(),
            cursor: with_alpha(theme.colors.text, if theme.is_dark { 0.78 } else { 0.62 }),
            selection: with_alpha(theme.colors.accent, if theme.is_dark { 0.28 } else { 0.18 }),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TextInputOptions {
    pub placeholder: SharedString,
    /// Optional icon rendered before the editable text using the input's
    /// placeholder color.
    pub leading_icon: Option<&'static str>,
    pub multiline: bool,
    pub read_only: bool,
    pub chromeless: bool,
    pub soft_wrap: bool,
    /// Minimum number of visible text rows. Only effective when `multiline: true`.
    pub min_lines: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct WrapCache {
    pub(super) width: Pixels,
    pub(super) rows: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PendingWrapJob {
    pub(super) sequence: u64,
    pub(super) width_key: i32,
    pub(super) line_count: usize,
    pub(super) wrap_columns: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct InterpolatedWrapPatch {
    pub(super) width_key: i32,
    pub(super) line_start: usize,
    pub(super) old_rows: Vec<usize>,
    pub(super) new_rows: Vec<usize>,
}

/// The shaped lines a frame actually touches, addressed by absolute line index.
///
/// A plain multiline input only shapes its visible window, plus the caret's own
/// line when that has been scrolled out of view. `ShapedLine` carries an inline
/// `SmallVec` of decoration runs and is ~3 KB, so a document-length vector costs
/// megabytes of zeroing per frame for rows that are never painted — which made
/// the per-keystroke frame scale with the file instead of the viewport.
#[derive(Debug, Default)]
pub(super) struct PlainLineLayouts {
    line_count: usize,
    window_start: usize,
    window: Vec<ShapedLine>,
    /// The caret's line when it sits outside the visible window. Boxed so one
    /// stray line does not widen every `TextInputLayout` by ~3 KB.
    stray: Option<(usize, Box<ShapedLine>)>,
}

impl PlainLineLayouts {
    pub(super) fn new(line_count: usize, window_start: usize, window_len: usize) -> Self {
        Self {
            line_count,
            window_start,
            window: Vec::with_capacity(window_len),
            stray: None,
        }
    }

    /// Append the next shaped line of the visible window. Callers shape the
    /// window in ascending line order, so position follows from `window_start`.
    pub(super) fn push(&mut self, line: ShapedLine) {
        self.window.push(line);
    }

    pub(super) fn set_stray(&mut self, line_ix: usize, line: ShapedLine) {
        self.stray = Some((line_ix, Box::new(line)));
    }

    pub(super) fn get(&self, line_ix: usize) -> Option<&ShapedLine> {
        if let Some(offset) = line_ix.checked_sub(self.window_start)
            && let Some(line) = self.window.get(offset)
        {
            return Some(line);
        }
        self.stray
            .as_ref()
            .filter(|(ix, _)| *ix == line_ix)
            .map(|(_, line)| line.as_ref())
    }

    /// The document's line count — not the number of shaped lines.
    pub(super) fn line_count(&self) -> usize {
        self.line_count
    }

    /// How many lines this frame actually shaped. Only the viewport (plus the
    /// caret's line) should ever be shaped, whatever the document's size.
    #[cfg(test)]
    pub(super) fn shaped_line_count(&self) -> usize {
        self.window.len() + usize::from(self.stray.is_some())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.line_count == 0
    }
}

#[derive(Debug)]
pub(super) enum TextInputLayout {
    Plain(PlainLineLayouts),
    TruncatedSingleLine(Arc<TruncatedLineLayout>),
    Wrapped {
        lines: Vec<WrappedLine>,
        y_offsets: Vec<Pixels>,
        row_counts: Vec<usize>,
    },
}

pub(super) struct HighlightState {
    pub(super) highlights: Arc<Vec<(Range<usize>, gpui::HighlightStyle)>>,
    pub(super) provider: Option<HighlightProvider>,
    pub(super) provider_binding_key: Option<u64>,
    pub(super) provider_cache: Option<ProviderHighlightCache>,
    pub(super) epoch: u64,
    pub(super) prepaint_runs_cache: Option<PrepaintHighlightRunsCache>,
    pub(super) provider_poll_task: Option<gpui::Task<()>>,
    /// Edits applied since `highlights`/`provider` were installed.
    pub(super) interpolation: HighlightInterpolation,
    pub(super) interpolated_cache: Option<InterpolatedHighlightCache>,
    /// Whether the current source has ever returned a settled (non-pending)
    /// answer. Until it has, it is not fit to become anyone's fallback.
    pub(super) answered: bool,
    pub(super) superseded: Option<SupersededHighlights>,
}

impl HighlightState {
    pub(super) fn new() -> Self {
        Self {
            highlights: Arc::new(Vec::new()),
            provider: None,
            provider_binding_key: None,
            provider_cache: None,
            epoch: 1,
            prepaint_runs_cache: None,
            provider_poll_task: None,
            interpolation: HighlightInterpolation::default(),
            interpolated_cache: None,
            answered: false,
            superseded: None,
        }
    }
}

pub(super) struct LayoutState {
    pub(super) scroll_x: Pixels,
    pub(super) last: Option<TextInputLayout>,
    pub(super) line_starts: Option<Arc<[usize]>>,
    pub(super) bounds: Option<Bounds<Pixels>>,
    pub(super) line_height: Pixels,
    pub(super) shape_style_epoch: u64,
    pub(super) plain_line_cache: HashMap<ShapedRowCacheKey, ShapedLine>,
}

impl LayoutState {
    pub(super) fn new() -> Self {
        Self {
            scroll_x: px(0.0),
            last: None,
            line_starts: None,
            bounds: None,
            line_height: px(0.0),
            shape_style_epoch: 1,
            plain_line_cache: HashMap::default(),
        }
    }
}

pub(super) struct WrapState {
    pub(super) cache: Option<WrapCache>,
    pub(super) last_rows: Option<usize>,
    pub(super) row_counts: Vec<usize>,
    pub(super) row_counts_width: Option<Pixels>,
    pub(super) recompute_sequence: u64,
    pub(super) recompute_requested: bool,
    pub(super) pending_job: Option<PendingWrapJob>,
    pub(super) dirty_ranges: Vec<Range<usize>>,
    pub(super) interpolated_patches: Vec<InterpolatedWrapPatch>,
}

impl WrapState {
    pub(super) fn new() -> Self {
        Self {
            cache: None,
            last_rows: None,
            row_counts: Vec::new(),
            row_counts_width: None,
            recompute_sequence: 1,
            recompute_requested: false,
            pending_job: None,
            dirty_ranges: Vec::new(),
            interpolated_patches: Vec::new(),
        }
    }
}

pub(super) struct SelectionState {
    pub(super) range: Range<usize>,
    pub(super) reversed: bool,
    pub(super) marked_range: Option<Range<usize>>,
    pub(super) pending_text_edit_deltas: Vec<(Range<usize>, Range<usize>)>,
    pub(super) undo_stack: Vec<UndoSnapshot>,
    pub(super) redo_stack: Vec<UndoSnapshot>,
}

impl SelectionState {
    pub(super) fn new() -> Self {
        Self {
            range: 0..0,
            reversed: false,
            marked_range: None,
            pending_text_edit_deltas: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

pub(super) struct InteractionState {
    pub(super) is_selecting: bool,
    pub(super) suppress_right_click: bool,
    pub(super) context_menu: Option<TextInputContextMenuState>,
    pub(super) vertical_motion_x: Option<Pixels>,
    pub(super) vertical_scroll_handle: Option<ScrollHandle>,
    /// When set, a multiline input lays out at its content (widest-line) width
    /// instead of filling its container, so an outer `overflow_scroll` container
    /// can scroll it horizontally and expose a real horizontal `max_offset` on
    /// the shared scroll handle (used for column↔output scroll sync).
    pub(super) content_width_layout: bool,
    pub(super) pending_cursor_autoscroll: bool,
    /// Set after a stale-max_offset retry so the next attempt always clears the flag,
    /// preventing an infinite notify loop when cursor_bottom sits at the viewport edge.
    pub(super) cursor_autoscroll_retry_exhausted: bool,
    pub(super) has_focus: bool,
    pub(super) cursor_blink_visible: bool,
    pub(super) cursor_blink_task: Option<gpui::Task<()>>,
    pub(super) enter_pressed: bool,
    pub(super) escape_pressed: bool,
    pub(super) arrow_up_pressed: bool,
    pub(super) arrow_down_pressed: bool,
    pub(super) tab_pressed: bool,
    pub(super) shift_tab_pressed: bool,
    pub(super) submit_on_enter: bool,
}

impl InteractionState {
    pub(super) fn new() -> Self {
        Self {
            is_selecting: false,
            suppress_right_click: false,
            context_menu: None,
            vertical_motion_x: None,
            vertical_scroll_handle: None,
            content_width_layout: false,
            pending_cursor_autoscroll: false,
            cursor_autoscroll_retry_exhausted: false,
            has_focus: false,
            cursor_blink_visible: true,
            cursor_blink_task: None,
            enter_pressed: false,
            escape_pressed: false,
            arrow_up_pressed: false,
            arrow_down_pressed: false,
            tab_pressed: false,
            shift_tab_pressed: false,
            submit_on_enter: false,
        }
    }
}

#[derive(Default)]
pub(super) struct ContentWidthCache {
    /// Mirrors the text model's line index so an edit can replace just its
    /// affected line range. The multiset keeps maximum lookup logarithmic
    /// without rescanning every line during layout.
    pub(super) line_units: Vec<usize>,
    pub(super) unit_counts: BTreeMap<usize, usize>,
}

impl ContentWidthCache {
    pub(super) fn max_units(&self) -> usize {
        self.unit_counts
            .last_key_value()
            .map(|(&units, _)| units)
            .unwrap_or_default()
    }
}

pub struct TextInput {
    pub(super) focus_handle: FocusHandle,
    pub(super) content: TextModel,
    pub(super) placeholder: SharedString,
    pub(super) leading_icon: Option<&'static str>,
    pub(super) multiline: bool,
    pub(super) read_only: bool,
    pub(super) chromeless: bool,
    pub(super) soft_wrap: bool,
    pub(super) min_lines: u32,
    pub(super) display_truncation: Option<TextTruncationProfile>,
    pub(super) masked: bool,
    pub(super) line_ending: &'static str,
    pub(super) style: TextInputStyle,
    pub(super) line_height_override: Option<Pixels>,
    pub(super) vertical_padding_override: Option<Pixels>,
    pub(super) highlight: HighlightState,
    pub(super) layout: LayoutState,
    pub(super) wrap: WrapState,
    pub(super) content_width_cache: Option<ContentWidthCache>,
    pub(super) selection: SelectionState,
    pub(super) interaction: InteractionState,
    /// Byte spans the buffer refuses to edit, each covering a whole line
    /// including its terminator. Edits that would alter one are dropped; the
    /// spans ride along with edits elsewhere so they stay accurate between
    /// refreshes by the owner.
    pub(super) protected_ranges: Arc<[Range<usize>]>,
}
