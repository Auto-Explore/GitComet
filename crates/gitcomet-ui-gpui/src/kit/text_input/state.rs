use super::shaping::with_alpha;
use super::*;

// Text or display-mode changes always clear shaped-row caches, so cache keys
// only need the line index plus wrap identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct ShapedRowCacheKey {
    pub(super) line_ix: usize,
    pub(super) wrap_width_key: i32,
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

#[derive(Clone, Debug)]
pub(super) struct PrepaintHighlightRunsCache {
    pub(super) highlight_epoch: u64,
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
            radius: theme.radii.row,
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
    pub multiline: bool,
    pub read_only: bool,
    pub chromeless: bool,
    pub soft_wrap: bool,
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

#[derive(Debug)]
pub(super) enum TextInputLayout {
    Plain(Vec<ShapedLine>),
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
    pub(super) wrapped_line_cache: HashMap<ShapedRowCacheKey, ()>,
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
            wrapped_line_cache: HashMap::default(),
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
    pub(super) pending_text_edit_delta: Option<(Range<usize>, Range<usize>)>,
    pub(super) undo_stack: Vec<UndoSnapshot>,
    pub(super) redo_stack: Vec<UndoSnapshot>,
}

impl SelectionState {
    pub(super) fn new() -> Self {
        Self {
            range: 0..0,
            reversed: false,
            marked_range: None,
            pending_text_edit_delta: None,
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
    pub(super) pending_cursor_autoscroll: bool,
    pub(super) has_focus: bool,
    pub(super) cursor_blink_visible: bool,
    pub(super) cursor_blink_task: Option<gpui::Task<()>>,
    pub(super) enter_pressed: bool,
    pub(super) escape_pressed: bool,
}

impl InteractionState {
    pub(super) fn new() -> Self {
        Self {
            is_selecting: false,
            suppress_right_click: false,
            context_menu: None,
            vertical_motion_x: None,
            vertical_scroll_handle: None,
            pending_cursor_autoscroll: false,
            has_focus: false,
            cursor_blink_visible: true,
            cursor_blink_task: None,
            enter_pressed: false,
            escape_pressed: false,
        }
    }
}

pub struct TextInput {
    pub(super) focus_handle: FocusHandle,
    pub(super) content: TextModel,
    pub(super) placeholder: SharedString,
    pub(super) multiline: bool,
    pub(super) read_only: bool,
    pub(super) chromeless: bool,
    pub(super) soft_wrap: bool,
    pub(super) display_truncation: Option<TextTruncationProfile>,
    pub(super) masked: bool,
    pub(super) line_ending: &'static str,
    pub(super) style: TextInputStyle,
    pub(super) line_height_override: Option<Pixels>,
    pub(super) highlight: HighlightState,
    pub(super) layout: LayoutState,
    pub(super) wrap: WrapState,
    pub(super) selection: SelectionState,
    pub(super) interaction: InteractionState,
}
