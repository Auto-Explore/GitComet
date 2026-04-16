use super::text_model::{TextModel, TextModelSnapshot};
use crate::theme::AppTheme;
use crate::view::components::CONTROL_HEIGHT_PX;
use gpui::prelude::*;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Div, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, IsZero, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Rgba,
    ScrollHandle, ShapedLine, SharedString, Style, TextAlign, TextRun, UTF16Selection, Window,
    WrappedLine, actions, anchored, deferred, div, fill, point, px, relative, size,
};
use rustc_hash::FxHashMap as HashMap;
#[cfg(any(test, feature = "benchmarks"))]
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::borrow::Cow;
use std::hash::Hash;
#[cfg(any(test, feature = "benchmarks"))]
use std::hash::Hasher;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation as _;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        Enter,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        Home,
        SelectHome,
        End,
        SelectEnd,
        PageUp,
        SelectPageUp,
        PageDown,
        SelectPageDown,
        Paste,
        Cut,
        Copy,
        Undo,
        Redo,
        ShowCharacterPalette,
    ]
);

const MAX_UNDO_STEPS: usize = 100;
const TEXT_INPUT_GUARD_ROWS: usize = 2;
const TEXT_INPUT_PROVIDER_PREFETCH_GUARD_ROWS: usize = 24;
const TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT: usize = 4;
const TEXT_INPUT_MAX_LINE_SHAPE_BYTES: usize = 4 * 1024;
const TEXT_INPUT_SHAPE_CACHE_LIMIT: usize = 8 * 1024;
const TEXT_INPUT_TRUNCATION_SUFFIX: &str = "…";
const TEXT_INPUT_WRAP_SYNC_LINE_THRESHOLD: usize = 256;
const TEXT_INPUT_WRAP_FOREGROUND_BUDGET_MS: u64 = 4;
const TEXT_INPUT_WRAP_BACKGROUND_YIELD_EVERY_ROWS: usize = 100;
const TEXT_INPUT_WRAP_DIRTY_SYNC_LINE_LIMIT: usize = 128;
const TEXT_INPUT_WRAP_TAB_STOP_COLUMNS: usize = 4;
const TEXT_INPUT_WRAP_CHAR_ADVANCE_FACTOR: f32 = 0.6;
const TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES: usize = 4_096;
const TEXT_INPUT_STREAMED_HIGHLIGHT_LEGACY_LINE_THRESHOLD: usize = 64;
const TEXT_INPUT_STREAMED_HIGHLIGHT_ESTIMATED_RUNS_PER_VISIBLE_LINE: usize = 2;
const TEXT_INPUT_INLINE_ACTIVE_HIGHLIGHT_CAPACITY: usize = 8;
const TEXT_INPUT_INLINE_TEXT_RUN_CAPACITY: usize = 32;

// Text or display-mode changes always clear shaped-row caches, so cache keys
// only need the line index plus wrap identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ShapedRowCacheKey {
    line_ix: usize,
    wrap_width_key: i32,
}

#[derive(Clone, Default)]
pub struct HighlightProviderResult {
    pub highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    pub pending: bool,
}

#[derive(Clone)]
pub struct HighlightProvider {
    resolve: Arc<dyn Fn(Range<usize>) -> HighlightProviderResult + Send + Sync>,
    drain_pending: Arc<dyn Fn() -> usize + Send + Sync>,
    has_pending: Arc<dyn Fn() -> bool + Send + Sync>,
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

    fn drain_pending(&self) -> usize {
        (self.drain_pending)()
    }

    fn has_pending(&self) -> bool {
        (self.has_pending)()
    }
}

#[derive(Clone)]
struct ProviderHighlightCacheEntry {
    byte_start: usize,
    byte_end: usize,
    pending: bool,
    highlights: Arc<Vec<(Range<usize>, gpui::HighlightStyle)>>,
}

impl ProviderHighlightCacheEntry {
    fn contains_range(&self, byte_range: &Range<usize>) -> bool {
        self.byte_start <= byte_range.start && self.byte_end >= byte_range.end
    }

    fn span_len(&self) -> usize {
        self.byte_end.saturating_sub(self.byte_start)
    }
}

#[derive(Clone)]
struct ProviderHighlightCache {
    highlight_epoch: u64,
    entries: Vec<ProviderHighlightCacheEntry>,
}

impl ProviderHighlightCache {
    fn new(highlight_epoch: u64) -> Self {
        Self {
            highlight_epoch,
            entries: Vec::new(),
        }
    }

    fn resolve(
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

    fn insert(
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
struct ResolvedProviderHighlights {
    pending: bool,
    highlights: Arc<Vec<(Range<usize>, gpui::HighlightStyle)>>,
}

fn should_reset_highlight_provider_binding(
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
struct PrepaintHighlightRunsCache {
    highlight_epoch: u64,
    visible_start: usize,
    visible_end: usize,
    line_runs: Arc<VisibleWindowTextRuns>,
}

#[derive(Clone, Debug, Default)]
struct VisibleWindowTextRuns {
    line_offsets: Vec<usize>,
    runs: Vec<TextRun>,
}

impl VisibleWindowTextRuns {
    fn with_line_capacity(line_count: usize) -> Self {
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

    fn finish_line(&mut self) {
        self.line_offsets.push(self.runs.len());
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn len(&self) -> usize {
        self.line_offsets.len().saturating_sub(1)
    }

    fn line(&self, local_ix: usize) -> Option<&[TextRun]> {
        let start = *self.line_offsets.get(local_ix)?;
        let end = *self.line_offsets.get(local_ix.saturating_add(1))?;
        self.runs.get(start..end)
    }
}

#[derive(Clone, Copy)]
struct TextShapeStyle<'a> {
    base_font: &'a gpui::Font,
    text_color: gpui::Hsla,
    highlights: Option<&'a [(Range<usize>, gpui::HighlightStyle)]>,
    font_size: Pixels,
}

#[derive(Clone, Copy)]
struct LineShapeInput<'a> {
    line_ix: usize,
    line_start: usize,
    line_text: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UndoSnapshot {
    content: TextModelSnapshot,
    selected_range: Range<usize>,
    selection_reversed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TextInputStyle {
    background: Rgba,
    border: Rgba,
    hover_border: Rgba,
    focus_border: Rgba,
    radius: f32,
    text: gpui::Hsla,
    placeholder: gpui::Hsla,
    cursor: Rgba,
    selection: Rgba,
}

#[derive(Clone, Copy, Debug)]
struct TextInputContextMenuState {
    can_paste: bool,
    anchor: Point<Pixels>,
}

impl TextInputStyle {
    fn from_theme(theme: AppTheme) -> Self {
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
struct WrapCache {
    width: Pixels,
    rows: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingWrapJob {
    sequence: u64,
    width_key: i32,
    line_count: usize,
    wrap_columns: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InterpolatedWrapPatch {
    width_key: i32,
    line_start: usize,
    old_rows: Vec<usize>,
    new_rows: Vec<usize>,
}

#[derive(Debug)]
enum TextInputLayout {
    Plain(Vec<ShapedLine>),
    Wrapped {
        lines: Vec<WrappedLine>,
        y_offsets: Vec<Pixels>,
        row_counts: Vec<usize>,
    },
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: TextModel,
    placeholder: SharedString,
    multiline: bool,
    read_only: bool,
    chromeless: bool,
    soft_wrap: bool,
    masked: bool,
    line_ending: &'static str,
    style: TextInputStyle,
    highlights: Arc<Vec<(Range<usize>, gpui::HighlightStyle)>>,
    highlight_provider: Option<HighlightProvider>,
    highlight_provider_binding_key: Option<u64>,
    provider_highlight_cache: Option<ProviderHighlightCache>,
    highlight_epoch: u64,
    line_height_override: Option<Pixels>,

    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,

    scroll_x: Pixels,
    last_layout: Option<TextInputLayout>,
    last_line_starts: Option<Arc<[usize]>>,
    last_bounds: Option<Bounds<Pixels>>,
    last_line_height: Pixels,
    wrap_cache: Option<WrapCache>,
    last_wrap_rows: Option<usize>,
    wrap_row_counts: Vec<usize>,
    wrap_row_counts_width: Option<Pixels>,
    wrap_recompute_sequence: u64,
    wrap_recompute_requested: bool,
    pending_wrap_job: Option<PendingWrapJob>,
    pending_wrap_dirty_ranges: Vec<Range<usize>>,
    interpolated_wrap_patches: Vec<InterpolatedWrapPatch>,
    shape_style_epoch: u64,
    prepaint_highlight_runs_cache: Option<PrepaintHighlightRunsCache>,
    plain_line_cache: HashMap<ShapedRowCacheKey, ShapedLine>,
    wrapped_line_cache: HashMap<ShapedRowCacheKey, ()>,
    is_selecting: bool,
    suppress_right_click: bool,
    context_menu: Option<TextInputContextMenuState>,
    vertical_motion_x: Option<Pixels>,
    vertical_scroll_handle: Option<ScrollHandle>,
    pending_cursor_autoscroll: bool,
    pending_text_edit_delta: Option<(Range<usize>, Range<usize>)>,

    has_focus: bool,
    cursor_blink_visible: bool,
    cursor_blink_task: Option<gpui::Task<()>>,
    highlight_provider_poll_task: Option<gpui::Task<()>>,
    undo_stack: Vec<UndoSnapshot>,
    redo_stack: Vec<UndoSnapshot>,
    enter_pressed: bool,
    escape_pressed: bool,
}

impl TextInput {
    pub fn new(options: TextInputOptions, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        Self {
            focus_handle,
            content: TextModel::new(),
            placeholder: options.placeholder,
            multiline: options.multiline,
            read_only: options.read_only,
            chromeless: options.chromeless,
            soft_wrap: options.soft_wrap,
            masked: false,
            line_ending: if cfg!(windows) { "\r\n" } else { "\n" },
            style: TextInputStyle::from_theme(AppTheme::gitcomet_dark()),
            highlights: Arc::new(Vec::new()),
            highlight_provider: None,
            highlight_provider_binding_key: None,
            provider_highlight_cache: None,
            highlight_epoch: 1,
            line_height_override: None,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            scroll_x: px(0.0),
            last_layout: None,
            last_line_starts: None,
            last_bounds: None,
            last_line_height: px(0.0),
            wrap_cache: None,
            last_wrap_rows: None,
            wrap_row_counts: Vec::new(),
            wrap_row_counts_width: None,
            wrap_recompute_sequence: 1,
            wrap_recompute_requested: false,
            pending_wrap_job: None,
            pending_wrap_dirty_ranges: Vec::new(),
            interpolated_wrap_patches: Vec::new(),
            shape_style_epoch: 1,
            prepaint_highlight_runs_cache: None,
            plain_line_cache: HashMap::default(),
            wrapped_line_cache: HashMap::default(),
            is_selecting: false,
            suppress_right_click: false,
            context_menu: None,
            vertical_motion_x: None,
            vertical_scroll_handle: None,
            pending_cursor_autoscroll: false,
            pending_text_edit_delta: None,
            has_focus: false,
            cursor_blink_visible: true,
            cursor_blink_task: None,
            highlight_provider_poll_task: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            enter_pressed: false,
            escape_pressed: false,
        }
    }

    pub fn new_inert(options: TextInputOptions, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        Self {
            focus_handle,
            content: TextModel::new(),
            placeholder: options.placeholder,
            multiline: options.multiline,
            read_only: options.read_only,
            chromeless: options.chromeless,
            soft_wrap: options.soft_wrap,
            masked: false,
            line_ending: if cfg!(windows) { "\r\n" } else { "\n" },
            style: TextInputStyle::from_theme(AppTheme::gitcomet_dark()),
            highlights: Arc::new(Vec::new()),
            highlight_provider: None,
            highlight_provider_binding_key: None,
            provider_highlight_cache: None,
            highlight_epoch: 1,
            line_height_override: None,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            scroll_x: px(0.0),
            last_layout: None,
            last_line_starts: None,
            last_bounds: None,
            last_line_height: px(0.0),
            wrap_cache: None,
            last_wrap_rows: None,
            wrap_row_counts: Vec::new(),
            wrap_row_counts_width: None,
            wrap_recompute_sequence: 1,
            wrap_recompute_requested: false,
            pending_wrap_job: None,
            pending_wrap_dirty_ranges: Vec::new(),
            interpolated_wrap_patches: Vec::new(),
            shape_style_epoch: 1,
            prepaint_highlight_runs_cache: None,
            plain_line_cache: HashMap::default(),
            wrapped_line_cache: HashMap::default(),
            is_selecting: false,
            suppress_right_click: false,
            context_menu: None,
            vertical_motion_x: None,
            vertical_scroll_handle: None,
            pending_cursor_autoscroll: false,
            pending_text_edit_delta: None,
            has_focus: false,
            cursor_blink_visible: true,
            cursor_blink_task: None,
            highlight_provider_poll_task: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            enter_pressed: false,
            escape_pressed: false,
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

    fn clear_shaped_row_caches(&mut self) {
        self.plain_line_cache.clear();
        self.wrapped_line_cache.clear();
        self.prepaint_highlight_runs_cache = None;
    }

    fn clear_wrap_recompute_state(&mut self) {
        self.pending_wrap_job = None;
        self.pending_wrap_dirty_ranges.clear();
        self.interpolated_wrap_patches.clear();
        self.wrap_recompute_requested = false;
    }

    fn invalidate_layout_caches_full(&mut self) {
        self.wrap_cache = None;
        self.last_layout = None;
        self.last_line_starts = None;
        self.wrap_row_counts.clear();
        self.wrap_row_counts_width = None;
        self.clear_wrap_recompute_state();
        self.last_wrap_rows = None;
        self.clear_shaped_row_caches();
    }

    fn invalidate_layout_caches_preserving_wrap_rows(&mut self) {
        self.wrap_cache = None;
        self.last_layout = None;
        self.last_line_starts = None;
        self.clear_shaped_row_caches();
    }

    fn invalidate_layout_caches(&mut self) {
        self.invalidate_layout_caches_full();
    }

    fn request_wrap_recompute(&mut self) {
        self.wrap_recompute_requested = true;
    }

    fn bump_shape_style_epoch(&mut self) {
        self.shape_style_epoch = self.shape_style_epoch.wrapping_add(1).max(1);
        self.invalidate_layout_caches();
    }

    fn bump_shape_style_epoch_preserving_wrap_rows(&mut self) {
        self.shape_style_epoch = self.shape_style_epoch.wrapping_add(1).max(1);
        self.invalidate_layout_caches_preserving_wrap_rows();
    }

    fn invalidate_highlights(&mut self, preserve_wrap_rows: bool) {
        self.provider_highlight_cache = None;
        self.highlight_epoch = self.highlight_epoch.wrapping_add(1).max(1);
        if preserve_wrap_rows {
            self.bump_shape_style_epoch_preserving_wrap_rows();
        } else {
            self.bump_shape_style_epoch();
        }
    }

    fn note_provider_highlights_changed(&mut self) {
        self.invalidate_highlights(true);
    }

    fn invalidate_provider_highlights_for_text_change(&mut self) {
        if self.highlight_provider.is_none() {
            return;
        }

        self.provider_highlight_cache = None;
        self.prepaint_highlight_runs_cache = None;
        self.highlight_epoch = self.highlight_epoch.wrapping_add(1).max(1);
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
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.cursor_blink_visible = true;
        self.scroll_x = px(0.0);
        self.invalidate_layout_caches();
        if self.multiline && self.soft_wrap {
            self.request_wrap_recompute();
        }
        self.pending_text_edit_delta = None;
        self.invalidate_provider_highlights_for_text_change();
        cx.notify();
    }

    pub fn set_highlights(
        &mut self,
        mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
        cx: &mut Context<Self>,
    ) {
        highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        self.highlights = Arc::new(highlights);
        self.highlight_provider = None;
        self.highlight_provider_binding_key = None;
        self.highlight_provider_poll_task.take();
        self.invalidate_highlights(false);
        cx.notify();
    }

    fn install_highlight_provider(
        &mut self,
        provider: HighlightProvider,
        binding_key: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if !should_reset_highlight_provider_binding(
            self.highlight_provider.is_some(),
            self.highlight_provider_binding_key,
            binding_key,
        ) {
            return;
        }

        self.highlight_provider = Some(provider);
        self.highlight_provider_binding_key = binding_key;
        self.highlight_provider_poll_task.take();
        self.highlights = Arc::new(Vec::new());
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

    fn effective_line_height(&self, window: &Window) -> Pixels {
        self.line_height_override
            .unwrap_or_else(|| window.line_height())
    }

    pub fn take_enter_pressed(&mut self) -> bool {
        std::mem::take(&mut self.enter_pressed)
    }

    pub fn take_escape_pressed(&mut self) -> bool {
        std::mem::take(&mut self.escape_pressed)
    }

    pub fn clear_transient_key_presses(&mut self) {
        self.enter_pressed = false;
        self.escape_pressed = false;
    }

    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        if self.read_only == read_only {
            return;
        }
        self.read_only = read_only;
        cx.notify();
    }

    pub fn set_suppress_right_click(&mut self, suppress: bool) {
        self.suppress_right_click = suppress;
    }

    pub fn set_vertical_scroll_handle(&mut self, handle: Option<ScrollHandle>) {
        self.vertical_scroll_handle = handle;
    }

    fn queue_cursor_autoscroll(&mut self) {
        self.pending_cursor_autoscroll = true;
    }

    fn resolve_provider_highlights(
        &mut self,
        byte_start: usize,
        byte_end: usize,
    ) -> ResolvedProviderHighlights {
        let requested_range = byte_start..byte_end;
        if let Some(cache) = self.provider_highlight_cache.as_mut()
            && let Some(resolved) = cache.resolve(self.highlight_epoch, &requested_range)
        {
            return resolved;
        }
        let Some(ref provider) = self.highlight_provider else {
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
        self.provider_highlight_cache
            .get_or_insert_with(|| ProviderHighlightCache::new(self.highlight_epoch))
            .insert(
                self.highlight_epoch,
                requested_range,
                pending,
                Arc::clone(&highlights),
            );
        ResolvedProviderHighlights {
            pending,
            highlights,
        }
    }

    fn ensure_highlight_provider_poll(&mut self, cx: &mut Context<Self>) {
        if self.highlight_provider_poll_task.is_some() {
            return;
        }

        let task = cx.spawn(
            async move |input: gpui::WeakEntity<TextInput>, cx: &mut gpui::AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(16)).await;

                let should_continue = input
                    .update(cx, |input, cx| {
                        let Some(provider) = input.highlight_provider.clone() else {
                            input.highlight_provider_poll_task = None;
                            return false;
                        };

                        let applied = provider.drain_pending();
                        if applied > 0 {
                            input.note_provider_highlights_changed();
                            cx.notify();
                        }

                        let pending = provider.has_pending();
                        if !pending {
                            input.highlight_provider_poll_task = None;
                        }
                        pending
                    })
                    .unwrap_or(false);

                if !should_continue {
                    break;
                }
            },
        );
        self.highlight_provider_poll_task = Some(task);
    }

    fn trim_shape_caches(&mut self) {
        if self.plain_line_cache.len() > TEXT_INPUT_SHAPE_CACHE_LIMIT {
            self.plain_line_cache.clear();
        }
        if self.wrapped_line_cache.len() > TEXT_INPUT_SHAPE_CACHE_LIMIT {
            self.wrapped_line_cache.clear();
        }
    }

    fn streamed_highlight_runs_for_visible_window(
        &mut self,
        display_text: &str,
        line_starts: &[usize],
        visible_line_range: Range<usize>,
        shape_style: &TextShapeStyle<'_>,
    ) -> Option<Arc<VisibleWindowTextRuns>> {
        let Some(highlights) = shape_style.highlights else {
            self.prepaint_highlight_runs_cache = None;
            return None;
        };
        let line_count = line_starts.len().max(1);
        if highlights.is_empty()
            || line_count <= TEXT_INPUT_STREAMED_HIGHLIGHT_LEGACY_LINE_THRESHOLD
            || visible_line_range.is_empty()
        {
            self.prepaint_highlight_runs_cache = None;
            return None;
        }

        if let Some(cache) = self.prepaint_highlight_runs_cache.as_ref()
            && cache.highlight_epoch == self.highlight_epoch
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
        self.prepaint_highlight_runs_cache = Some(PrepaintHighlightRunsCache {
            highlight_epoch: self.highlight_epoch,
            visible_start: visible_line_range.start,
            visible_end: visible_line_range.end,
            line_runs: Arc::clone(&line_runs),
        });
        Some(line_runs)
    }

    fn shape_plain_line_cached(
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
        if let Some(cached) = self.plain_line_cache.get(&key) {
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
        self.plain_line_cache.insert(key, shaped.clone());
        self.trim_shape_caches();
        shaped
    }

    fn shape_wrapped_line_cached(
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
        self.wrapped_line_cache.insert(key, ());
        self.trim_shape_caches();
        wrapped
    }

    fn mark_wrap_dirty_from_edit(&mut self, old_range: Range<usize>, new_range: Range<usize>) {
        if !(self.multiline && self.soft_wrap) {
            return;
        }

        let text = self.content.as_ref();
        let line_starts = self.content.line_starts();
        let line_count = line_starts.len().max(1);
        if self.wrap_row_counts.len() != line_count {
            self.wrap_row_counts.resize(line_count, 1);
            self.wrap_recompute_requested = true;
            self.pending_wrap_job = None;
            self.interpolated_wrap_patches.clear();
            return;
        }

        let dirty_range =
            expanded_dirty_wrap_line_range_for_edit(text, line_starts, &old_range, &new_range);
        if dirty_range.start < dirty_range.end {
            self.pending_wrap_dirty_ranges.push(dirty_range);
        }
    }

    fn take_normalized_wrap_dirty_ranges(&mut self, line_count: usize) -> Vec<Range<usize>> {
        let mut ranges = std::mem::take(&mut self.pending_wrap_dirty_ranges);
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

    fn push_interpolated_wrap_patch(
        &mut self,
        width_key: i32,
        line_ix: usize,
        old_rows: usize,
        new_rows: usize,
    ) {
        if old_rows == new_rows {
            return;
        }

        if let Some(last) = self.interpolated_wrap_patches.last_mut()
            && last.width_key == width_key
            && last.line_start + last.old_rows.len() == line_ix
        {
            last.old_rows.push(old_rows);
            last.new_rows.push(new_rows);
            return;
        }

        if reset_interpolated_wrap_patches_on_overflow(
            &mut self.interpolated_wrap_patches,
            &mut self.wrap_recompute_requested,
        ) {
            return;
        }
        self.interpolated_wrap_patches.push(InterpolatedWrapPatch {
            width_key,
            line_start: line_ix,
            old_rows: vec![old_rows],
            new_rows: vec![new_rows],
        });
    }

    fn apply_pending_dirty_wrap_updates(
        &mut self,
        display_text: &str,
        line_starts: &[usize],
        rounded_wrap_width: Pixels,
        font_size: Pixels,
        allow_interpolated_patches: bool,
    ) -> bool {
        if self.pending_wrap_dirty_ranges.is_empty() {
            return false;
        }

        let line_count = line_starts.len().max(1);
        if line_count == 0 {
            self.pending_wrap_dirty_ranges.clear();
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
            self.pending_wrap_job.as_ref(),
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
                let old_rows = self.wrap_row_counts[line_ix].max(1);
                if old_rows != new_rows {
                    self.wrap_row_counts[line_ix] = new_rows;
                    changed = true;
                    if job_accepts_interpolation {
                        self.push_interpolated_wrap_patch(width_key, line_ix, old_rows, new_rows);
                    }
                }
            }
        }
        changed
    }

    fn maybe_recompute_wrap_rows(
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
            self.pending_wrap_job = None;
            self.interpolated_wrap_patches.clear();
            estimate_wrap_rows_with_line_starts(
                display_text,
                line_starts,
                wrap_columns,
                &mut self.wrap_row_counts,
            );
            self.wrap_recompute_requested = false;
            return false;
        }

        let has_compatible_job = self
            .pending_wrap_job
            .map(|job| job.width_key == width_key && job.line_count == line_count)
            .unwrap_or(false);
        if has_compatible_job && !self.wrap_recompute_requested {
            return false;
        }
        if !self.wrap_recompute_requested {
            return false;
        }

        let mut budget_rows = std::mem::take(&mut self.wrap_row_counts);
        budget_rows.resize(line_count, 1);
        estimate_wrap_rows_budgeted(
            display_text,
            line_starts,
            wrap_columns,
            &mut budget_rows,
            Duration::from_millis(TEXT_INPUT_WRAP_FOREGROUND_BUDGET_MS),
        );
        self.wrap_row_counts = budget_rows;
        self.wrap_row_counts_width = Some(rounded_wrap_width);
        self.wrap_recompute_requested = false;

        let sequence = self.wrap_recompute_sequence.wrapping_add(1).max(1);
        self.wrap_recompute_sequence = sequence;
        self.pending_wrap_job = Some(PendingWrapJob {
            sequence,
            width_key,
            line_count,
            wrap_columns,
        });
        self.interpolated_wrap_patches.clear();

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

    fn complete_wrap_recompute_job(
        &mut self,
        sequence: u64,
        width_key: i32,
        line_count: usize,
        mut rows: Vec<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(job) = self.pending_wrap_job else {
            return;
        };
        if job.sequence != sequence || job.width_key != width_key || job.line_count != line_count {
            return;
        }

        rows.resize(line_count, 1);
        for rows_per_line in &mut rows {
            *rows_per_line = (*rows_per_line).max(1);
        }
        for patch in &self.interpolated_wrap_patches {
            if patch.width_key == width_key {
                apply_interpolated_wrap_patch_delta(rows.as_mut_slice(), patch);
            }
        }
        self.interpolated_wrap_patches.clear();
        self.wrap_row_counts = rows;
        self.pending_wrap_job = None;
        self.last_wrap_rows = Some(total_wrap_rows(self.wrap_row_counts.as_slice()));
        cx.notify();
    }

    pub fn selected_text(&self) -> Option<String> {
        if self.selected_range.is_empty() {
            None
        } else {
            Some(self.content[self.selected_range.clone()].to_string())
        }
    }

    pub fn selected_range(&self) -> Range<usize> {
        self.selected_range.clone()
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
            self.last_wrap_rows = None;
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

    fn sanitize_insert_text(&self, text: &str) -> Option<String> {
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

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
        self.queue_cursor_autoscroll();
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
        self.queue_cursor_autoscroll();
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_word_start(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
        self.queue_cursor_autoscroll();
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_word_end(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
        self.queue_cursor_autoscroll();
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_word_start(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_word_end(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.vertical_move_target(self.cursor_offset(), -1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.move_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.vertical_move_target(self.cursor_offset(), 1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.move_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.vertical_move_target(self.cursor_offset(), -1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.select_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.vertical_move_target(self.cursor_offset(), 1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.select_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_text(cx);
    }

    fn row_start(&self, offset: usize) -> usize {
        self.row_boundaries(offset).0
    }

    fn row_end(&self, offset: usize) -> usize {
        self.row_boundaries(offset).1
    }

    fn logical_row_boundaries(&self, offset: usize) -> (usize, usize) {
        let s = self.content.as_ref();
        let offset = offset.min(s.len());
        let start = s[..offset].rfind('\n').map(|ix| ix + 1).unwrap_or(0);
        let rel_end = s[offset..].find('\n').unwrap_or(s.len() - offset);
        let end = offset + rel_end;
        (start, end)
    }

    fn row_boundaries(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.content.len());
        if self.content.is_empty() {
            return (0, 0);
        }
        if !(self.multiline && self.soft_wrap) {
            return self.logical_row_boundaries(offset);
        }

        let Some(TextInputLayout::Wrapped { lines, .. }) = self.last_layout.as_ref() else {
            return self.logical_row_boundaries(offset);
        };
        let Some(starts) = self.last_line_starts.as_ref() else {
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

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_start(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_start(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_end(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_end(self.cursor_offset()), cx);
        self.queue_cursor_autoscroll();
    }

    fn caret_point_for_hit_testing(&self, cursor: usize) -> Option<Point<Pixels>> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        let starts = self.last_line_starts.as_ref()?;
        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let (line_ix, local_ix) = line_for_offset(starts, lines, cursor);
                let line = lines.get(line_ix)?;
                let x = line.x_for_index(local_ix) - self.scroll_x;
                let y = line_height * line_ix as f32 + line_height / 2.0;
                Some(point(bounds.left() + x, bounds.top() + y))
            }
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

    fn vertical_move_target(
        &self,
        cursor: usize,
        direction: f32,
        preferred_x: Option<Pixels>,
    ) -> Option<(usize, Pixels)> {
        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };
        let caret_point = self.caret_point_for_hit_testing(cursor)?;
        let preferred_x = preferred_x.unwrap_or(caret_point.x);
        let target = point(preferred_x, caret_point.y + line_height * direction);
        Some((self.index_for_position(target), preferred_x))
    }

    fn page_move_target(
        &self,
        cursor: usize,
        direction: f32,
        preferred_x: Option<Pixels>,
    ) -> Option<(usize, Pixels)> {
        let bounds = self.last_bounds?;
        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };
        let page_height = bounds.size.height.max(line_height);
        let caret_point = self.caret_point_for_hit_testing(cursor)?;
        let preferred_x = preferred_x.unwrap_or(caret_point.x);
        let target = point(preferred_x, caret_point.y + page_height * direction);
        Some((self.index_for_position(target), preferred_x))
    }

    fn cursor_vertical_span(&self, cursor: usize) -> Option<(Pixels, Pixels)> {
        let layout = self.last_layout.as_ref()?;
        let starts = self.last_line_starts.as_ref()?;
        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let (line_ix, _) = line_for_offset(starts, lines, cursor);
                let top = line_height * line_ix as f32;
                let bottom = top + line_height;
                Some((top, bottom))
            }
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

    fn ensure_cursor_visible_in_vertical_scroll(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.vertical_scroll_handle.clone() else {
            self.pending_cursor_autoscroll = false;
            return;
        };
        let Some(text_bounds) = self.last_bounds else {
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
                self.pending_cursor_autoscroll = true;
            } else if cursor_out_of_view {
                cx.notify();
                self.pending_cursor_autoscroll = true;
            } else {
                self.pending_cursor_autoscroll = false;
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
            self.pending_cursor_autoscroll = false;
            return;
        }
        .max(px(0.0))
        .min(max_offset);

        if target_scroll == scroll_y {
            self.pending_cursor_autoscroll = false;
            return;
        }

        let next_y = if negative_axis {
            -target_scroll
        } else {
            target_scroll
        };
        handle.set_offset(point(current.x, next_y));
        self.pending_cursor_autoscroll = false;
        cx.notify();
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.page_move_target(self.cursor_offset(), -1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.move_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    fn select_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.page_move_target(self.cursor_offset(), -1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.select_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.page_move_target(self.cursor_offset(), 1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.move_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    fn select_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        let Some((target, preferred_x)) =
            self.page_move_target(self.cursor_offset(), 1.0, self.vertical_motion_x)
        else {
            return;
        };
        self.select_to(target, cx);
        self.vertical_motion_x = Some(preferred_x);
        self.queue_cursor_autoscroll();
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            self.select_to(self.previous_word_start(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            self.select_to(self.next_word_end(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || !self.multiline {
            self.enter_pressed = true;
            cx.notify();
            return;
        }
        self.queue_cursor_autoscroll();
        self.replace_text_in_range(None, self.line_ending, window, cx);
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            if !self.read_only {
                self.replace_text_in_range(None, "", window, cx)
            }
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(snapshot) = self.undo_stack.pop() else {
            return;
        };
        self.push_redo_snapshot(self.current_undo_snapshot());
        self.restore_undo_snapshot(snapshot, cx);
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(snapshot) = self.redo_stack.pop() else {
            return;
        };
        self.push_undo_snapshot(self.current_undo_snapshot());
        self.restore_undo_snapshot(snapshot, cx);
    }

    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    pub fn set_cursor_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.move_to(offset, cx);
        self.queue_cursor_autoscroll();
    }

    fn normalized_utf8_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clamp_to_char_boundary(range.start.min(self.content.len()));
        let end = self.clamp_to_char_boundary(range.end.min(self.content.len()));
        if end < start { end..start } else { start..end }
    }

    fn replace_utf8_range_internal(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        let undo_snapshot = self.current_undo_snapshot();
        let range = self.normalized_utf8_range(range);
        let inserted = self.content.replace_range(range.clone(), new_text);
        self.push_undo_snapshot(undo_snapshot);
        self.redo_stack.clear();
        self.pending_text_edit_delta = Some((range.clone(), inserted.clone()));
        let cursor = inserted.end;
        self.mark_wrap_dirty_from_edit(range, inserted.clone());
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range.take();
        self.vertical_motion_x = None;
        self.cursor_blink_visible = true;
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
        self.replace_utf8_range(self.selected_range.clone(), new_text, cx)
    }

    /// Consume the latest UTF-8 edit delta as `(old_range, new_range)`.
    ///
    /// `old_range` references bytes in the pre-edit text; `new_range` references
    /// bytes in the post-edit text.
    pub fn take_recent_utf8_edit_delta(&mut self) -> Option<(Range<usize>, Range<usize>)> {
        self.pending_text_edit_delta.take()
    }

    pub fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        self.index_for_position(position)
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = self.clamp_to_char_boundary(offset);
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.vertical_motion_x = None;
        self.cursor_blink_visible = true;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = self.clamp_to_char_boundary(offset);
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.vertical_motion_x = None;
        self.cursor_blink_visible = true;
        cx.notify();
    }

    fn clamp_to_char_boundary(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.content.len());
        while offset > 0 && !self.content.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    fn current_undo_snapshot(&self) -> UndoSnapshot {
        UndoSnapshot {
            content: self.content.snapshot(),
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
        }
    }

    fn push_undo_snapshot(&mut self, snapshot: UndoSnapshot) {
        Self::push_history_snapshot(&mut self.undo_stack, snapshot);
    }

    fn push_redo_snapshot(&mut self, snapshot: UndoSnapshot) {
        Self::push_history_snapshot(&mut self.redo_stack, snapshot);
    }

    fn push_history_snapshot(stack: &mut Vec<UndoSnapshot>, snapshot: UndoSnapshot) {
        if stack.last() == Some(&snapshot) {
            return;
        }
        if stack.len() >= MAX_UNDO_STEPS {
            let _ = stack.remove(0);
        }
        stack.push(snapshot);
    }

    fn restore_undo_snapshot(&mut self, snapshot: UndoSnapshot, cx: &mut Context<Self>) {
        self.content = snapshot.content.into();
        self.selected_range = snapshot.selected_range;
        self.selection_reversed = snapshot.selection_reversed;
        self.marked_range = None;
        self.vertical_motion_x = None;
        self.cursor_blink_visible = true;
        self.is_selecting = false;
        self.invalidate_layout_caches();
        if self.multiline && self.soft_wrap {
            self.request_wrap_recompute();
        }
        self.pending_text_edit_delta = None;
        self.invalidate_provider_highlights_for_text_change();
        self.queue_cursor_autoscroll();
        cx.notify();
    }

    fn skip_left_while(
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

    fn skip_right_while(
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

    fn previous_word_start(&self, offset: usize) -> usize {
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

    fn next_word_end(&self, offset: usize) -> usize {
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

    fn token_range_for_offset(&self, offset: usize) -> Range<usize> {
        let s = self.content.as_ref();
        if s.is_empty() {
            return 0..0;
        }

        let mut probe = offset.min(s.len());
        if probe == s.len() && probe > 0 {
            probe = self.previous_boundary(probe);
        }

        let Some(ch) = s[probe..].chars().next() else {
            return probe..probe;
        };

        if ch.is_whitespace() {
            let start = Self::skip_left_while(s, probe, |ch| ch.is_whitespace());
            let end = Self::skip_right_while(s, probe, |ch| ch.is_whitespace());
            return start..end;
        }

        if Self::is_word_char(ch) {
            let start = Self::skip_left_while(s, probe, Self::is_word_char);
            let end = Self::skip_right_while(s, probe, Self::is_word_char);
            return start..end;
        }

        let start = Self::skip_left_while(s, probe, |ch| {
            !ch.is_whitespace() && !Self::is_word_char(ch)
        });
        let end = Self::skip_right_while(s, probe, |ch| {
            !ch.is_whitespace() && !Self::is_word_char(ch)
        });
        start..end
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.context_menu.take().is_some() {
            cx.notify();
        }
        cx.stop_propagation();
        window.focus(&self.focus_handle, cx);
        self.cursor_blink_visible = true;
        let index = self.index_for_mouse_position(event.position);
        self.vertical_motion_x = None;

        if event.modifiers.shift {
            self.is_selecting = true;
            self.select_to(index, cx);
            return;
        }

        if event.click_count >= 2 {
            self.is_selecting = false;
            let range = self.token_range_for_offset(index);
            if range.is_empty() {
                self.move_to(index, cx);
            } else {
                self.selected_range = range;
                self.selection_reversed = false;
                cx.notify();
            }
        } else {
            self.is_selecting = true;
            self.move_to(index, cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn on_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.modified() {
            return;
        }

        if event.keystroke.key.as_str() == "escape" {
            self.escape_pressed = true;
            cx.notify();
        }
    }

    fn on_mouse_down_right(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.suppress_right_click {
            return;
        }

        cx.stop_propagation();
        window.focus(&self.focus_handle, cx);
        self.cursor_blink_visible = true;
        self.is_selecting = false;
        self.vertical_motion_x = None;

        let index = self.index_for_mouse_position(event.position);
        let click_inside_selection = !self.selected_range.is_empty()
            && index >= self.selected_range.start
            && index <= self.selected_range.end;
        if !click_inside_selection {
            self.move_to(index, cx);
        }

        self.context_menu = Some(TextInputContextMenuState {
            can_paste: cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some(),
            anchor: event.position,
        });
        cx.notify();
    }

    fn context_menu_entry_row(
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

    fn render_context_menu(
        &mut self,
        state: TextInputContextMenuState,
        cx: &mut Context<Self>,
    ) -> Div {
        let primary = primary_modifier_label();
        let undo_disabled = self.read_only || self.undo_stack.is_empty();
        let redo_disabled = self.read_only || self.redo_stack.is_empty();
        let cut_disabled = self.read_only || self.selected_range.is_empty();
        let copy_disabled = self.selected_range.is_empty();
        let paste_disabled = self.read_only || !state.can_paste;
        let delete_disabled = self.read_only || self.selected_range.is_empty();
        let select_all_disabled = self.content.is_empty();

        let mut undo_row =
            self.context_menu_entry_row("Undo", format!("{primary}+Z").into(), undo_disabled);
        if !undo_disabled {
            undo_row = undo_row.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.context_menu = None;
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
                    this.context_menu = None;
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
                        this.context_menu = None;
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
                    this.context_menu = None;
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
                    this.context_menu = None;
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
                    this.context_menu = None;
                    if !this.selected_range.is_empty() && !this.read_only {
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
                    this.context_menu = None;
                    this.select_all(&SelectAll, window, cx);
                    cx.notify();
                }),
            );
        }

        div()
            .w(px(188.0))
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

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(layout), Some(starts)) = (
            self.last_bounds.as_ref(),
            self.last_layout.as_ref(),
            self.last_line_starts.as_ref(),
        ) else {
            return 0;
        };

        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }

        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let ratio = f32::from(position.y - bounds.top()) / f32::from(line_height);
                let mut line_ix = ratio.floor() as isize;
                line_ix = line_ix.clamp(0, lines.len().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let local_x = position.x - bounds.left() + self.scroll_x;
                let local_ix = lines[line_ix].closest_index_for_x(local_x);
                let doc_ix = starts.get(line_ix).copied().unwrap_or(0) + local_ix;
                doc_ix.min(self.content.len())
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

    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(layout), Some(starts)) = (
            self.last_bounds.as_ref(),
            self.last_layout.as_ref(),
            self.last_line_starts.as_ref(),
        ) else {
            return 0;
        };

        let line_height = if self.last_line_height.is_zero() {
            px(16.0)
        } else {
            self.last_line_height
        };

        match layout {
            TextInputLayout::Plain(lines) => {
                let ratio = f32::from(position.y - bounds.top()) / f32::from(line_height);
                let mut line_ix = ratio.floor() as isize;
                line_ix = line_ix.clamp(0, lines.len().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let local_x = position.x - bounds.left() + self.scroll_x;
                let local_ix = lines[line_ix].closest_index_for_x(local_x);
                let doc_ix = starts.get(line_ix).copied().unwrap_or(0) + local_ix;
                doc_ix.min(self.content.len())
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

    fn offset_from_utf16(&self, offset: usize) -> usize {
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

    fn offset_to_utf16(&self, offset: usize) -> usize {
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

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
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
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
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
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        let inserted = self.content.replace_range(range.clone(), new_text.as_str());
        self.pending_text_edit_delta = Some((range.clone(), inserted.clone()));
        self.mark_wrap_dirty_from_edit(range.clone(), inserted.clone());
        self.push_undo_snapshot(undo_snapshot);
        self.selected_range = inserted.end..inserted.end;
        self.selection_reversed = false;
        self.marked_range.take();
        self.vertical_motion_x = None;
        self.cursor_blink_visible = true;
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
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        let inserted = self.content.replace_range(range.clone(), new_text.as_str());
        self.pending_text_edit_delta = Some((range.clone(), inserted.clone()));
        self.mark_wrap_dirty_from_edit(range.clone(), inserted.clone());
        self.push_undo_snapshot(undo_snapshot);
        if !new_text.is_empty() {
            self.marked_range = Some(inserted.clone());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selection_reversed = false;

        self.vertical_motion_x = None;
        self.cursor_blink_visible = true;
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
        let layout = self.last_layout.as_ref()?;
        let starts = self.last_line_starts.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let offset = range.start.min(self.content.len());
        let line_height = self.effective_line_height(window);

        let (line_ix, local_ix, y_offset) = match layout {
            TextInputLayout::Plain(lines) => {
                let (line_ix, local_ix) = line_for_offset(starts, lines, offset);
                (line_ix, local_ix, line_height * line_ix as f32)
            }
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
                (line.x_for_index(local_ix) - self.scroll_x, y_offset)
            }
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
        let local = self.last_bounds?.localize(&p)?;
        let layout = self.last_layout.as_ref()?;
        let starts = self.last_line_starts.as_ref()?;
        let line_height = self.effective_line_height(window);
        match layout {
            TextInputLayout::Plain(lines) => {
                let mut line_ix = (local.y / line_height).floor() as isize;
                line_ix = line_ix.clamp(0, lines.len().saturating_sub(1) as isize);
                let line_ix = line_ix as usize;
                let line = lines.get(line_ix)?;
                let local_x = local.x + self.scroll_x;
                let idx = line.index_for_x(local_x).unwrap_or(line.len());
                let doc_offset = starts.get(line_ix).copied().unwrap_or(0) + idx;
                Some(self.offset_to_utf16(doc_offset))
            }
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

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    layout: Option<TextInputLayout>,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
    line_starts: Option<Arc<[usize]>>,
    wrap_cache: Option<WrapCache>,
    scroll_x: Pixels,
    visible_line_range: Range<usize>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        let line_height = input.effective_line_height(window);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if input.multiline {
            let line_count = input.content.line_starts().len().max(1) as f32;
            if input.soft_wrap
                && let Some(cache) = input.wrap_cache
                && cache.rows > 0
                && cache.width > px(0.0)
            {
                style.size.height = (line_height * cache.rows as f32).into();
            } else if input.soft_wrap
                && let Some(rows) = input.last_wrap_rows
                && rows > 0
            {
                // Preserve the previous wrapped row count until the next wrap pass finishes.
                style.size.height = (line_height * rows as f32).into();
            } else {
                style.size.height = (line_height * line_count).into();
            }
        } else {
            style.size.height = line_height.into();
        }
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.input.update(cx, |input, cx| {
            let content = input.content.snapshot();
            let selected_range = input.selected_range.clone();
            let cursor = input.cursor_offset();
            let style_colors = input.style;
            let soft_wrap = input.soft_wrap && input.multiline;
            let style = window.text_style();
            let has_content = !content.is_empty();

            let (display_text, text_color) = if content.is_empty() {
                (input.placeholder.clone(), style_colors.placeholder)
            } else if input.masked {
                (
                    mask_text_for_display(content.as_ref()).into(),
                    style_colors.text,
                )
            } else {
                (content.as_shared_string(), style_colors.text)
            };

            let font_size = style.font_size.to_pixels(window.rem_size());
            let line_height = input.effective_line_height(window);
            let base_font = style.font();

            let display_text_str = display_text.as_ref();
            let line_starts: Arc<[usize]> = if has_content && !input.masked {
                content.shared_line_starts()
            } else {
                compute_line_starts(display_text_str).into()
            };
            let line_count = line_starts.len().max(1);
            let (visible_top, visible_bottom) =
                visible_vertical_window(bounds, input.vertical_scroll_handle.as_ref());

            // Resolve highlights: use the provider path for large documents,
            // otherwise use the pre-materialized highlight vector.
            let highlights = if !has_content {
                None
            } else if input.highlight_provider.is_some() {
                let byte_range = provider_prefetch_byte_range_for_visible_window(
                    line_starts.as_ref(),
                    display_text_str.len(),
                    line_count,
                    line_height,
                    visible_top,
                    visible_bottom,
                );
                let resolved = input.resolve_provider_highlights(byte_range.start, byte_range.end);
                if resolved.pending {
                    input.ensure_highlight_provider_poll(cx);
                }
                Some(resolved.highlights)
            } else {
                Some(Arc::clone(&input.highlights))
            };
            let highlight_slice = highlights.as_ref().map(|h| h.as_slice());
            let shape_style = TextShapeStyle {
                base_font: &base_font,
                text_color,
                highlights: highlight_slice,
                font_size,
            };

            if !soft_wrap {
                let mut scroll_x = if input.multiline {
                    px(0.0)
                } else {
                    input.scroll_x
                };
                let mut lines = vec![ShapedLine::default(); line_count];
                let mut visible_line_range = if input.multiline {
                    visible_plain_line_range(
                        line_count,
                        line_height,
                        visible_top,
                        visible_bottom,
                        TEXT_INPUT_GUARD_ROWS,
                    )
                } else {
                    0..line_count
                };
                if visible_line_range.is_empty() {
                    visible_line_range = 0..line_count.min(1);
                }
                let streamed_line_runs = input.streamed_highlight_runs_for_visible_window(
                    display_text_str,
                    line_starts.as_ref(),
                    visible_line_range.clone(),
                    &shape_style,
                );

                for line_ix in visible_line_range.clone() {
                    let precomputed_runs = visible_window_runs_for_line_ix(
                        streamed_line_runs.as_deref(),
                        visible_line_range.start,
                        line_ix,
                    );
                    let shaped = input.shape_plain_line_cached(
                        LineShapeInput {
                            line_ix,
                            line_start: line_starts.get(line_ix).copied().unwrap_or(0),
                            line_text: line_text_for_index(
                                display_text_str,
                                line_starts.as_ref(),
                                line_ix,
                            ),
                        },
                        precomputed_runs,
                        &shape_style,
                        window,
                    );
                    if let Some(slot) = lines.get_mut(line_ix) {
                        *slot = shaped;
                    }
                }

                let cursor_line_ix =
                    line_index_for_offset(line_starts.as_ref(), cursor, line_count);
                if cursor_line_ix < line_count
                    && (cursor_line_ix < visible_line_range.start
                        || cursor_line_ix >= visible_line_range.end)
                {
                    let shaped = input.shape_plain_line_cached(
                        LineShapeInput {
                            line_ix: cursor_line_ix,
                            line_start: line_starts.get(cursor_line_ix).copied().unwrap_or(0),
                            line_text: line_text_for_index(
                                display_text_str,
                                line_starts.as_ref(),
                                cursor_line_ix,
                            ),
                        },
                        None,
                        &shape_style,
                        window,
                    );
                    if let Some(slot) = lines.get_mut(cursor_line_ix) {
                        *slot = shaped;
                    }
                }

                if !input.multiline && !lines.is_empty() {
                    let viewport_w = bounds.size.width.max(px(0.0));
                    let pad = px(8.0).min(viewport_w / 4.0);
                    let (line_ix, local_ix) = line_for_offset(line_starts.as_ref(), &lines, cursor);
                    let cursor_x = lines[line_ix].x_for_index(local_ix);
                    let max_scroll_x = (lines[line_ix].width - viewport_w).max(px(0.0));

                    let left = scroll_x;
                    let right = scroll_x + viewport_w;
                    if cursor_x < left + pad {
                        scroll_x = (cursor_x - pad).max(px(0.0));
                    } else if cursor_x > right - pad {
                        scroll_x = (cursor_x + pad - viewport_w).max(px(0.0));
                    }
                    scroll_x = scroll_x.min(max_scroll_x);
                } else {
                    scroll_x = px(0.0);
                }

                let mut selections = Vec::with_capacity(visible_line_range.len().max(1));
                let cursor_quad = if selected_range.is_empty() {
                    let (line_ix, local_ix) = line_for_offset(line_starts.as_ref(), &lines, cursor);
                    let x = lines[line_ix].x_for_index(local_ix) - scroll_x;
                    let caret_inset_y = px(3.0);
                    let caret_h = if !input.multiline && !input.chromeless {
                        // Cap caret to fit within the fixed-height container
                        // (CONTROL_HEIGHT_PX minus 2px border minus insets).
                        (px(CONTROL_HEIGHT_PX) - px(2.0) - caret_inset_y * 2.0).max(px(2.0))
                    } else {
                        (line_height - caret_inset_y * 2.0).max(px(2.0))
                    };
                    let caret_top_inset = (line_height - caret_h) / 2.0;
                    let top = bounds.top() + line_height * line_ix as f32 + caret_top_inset;
                    Some(fill(
                        Bounds::new(point(bounds.left() + x, top), size(px(1.0), caret_h)),
                        style_colors.cursor,
                    ))
                } else {
                    for ix in visible_line_range.clone() {
                        let start = line_starts.get(ix).copied().unwrap_or(0);
                        let next_start = line_starts
                            .get(ix + 1)
                            .copied()
                            .unwrap_or(display_text.len());
                        let line_len = lines[ix].len();
                        let line_end = start + line_len;

                        let seg_start = selected_range.start.max(start);
                        let seg_end = selected_range.end.min(next_start);
                        if seg_start >= seg_end {
                            continue;
                        }

                        let local_start = seg_start.min(line_end) - start;
                        let local_end = seg_end.min(line_end) - start;

                        let x0 = lines[ix].x_for_index(local_start) - scroll_x;
                        let x1 = lines[ix].x_for_index(local_end) - scroll_x;
                        let top = bounds.top() + line_height * ix as f32;
                        selections.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + x0, top),
                                point(bounds.left() + x1, top + line_height),
                            ),
                            style_colors.selection,
                        ));
                    }
                    None
                };

                return PrepaintState {
                    layout: Some(TextInputLayout::Plain(lines)),
                    cursor: cursor_quad,
                    selections,
                    line_starts: Some(line_starts),
                    wrap_cache: None,
                    scroll_x,
                    visible_line_range,
                };
            }

            let wrap_width = bounds.size.width.max(px(0.0));
            let rounded_wrap_width = wrap_width.round();
            let wrap_width_key = wrap_width_cache_key(rounded_wrap_width);
            if input.wrap_row_counts.len() != line_count {
                input.wrap_row_counts.resize(line_count, 1);
                input.request_wrap_recompute();
            }
            if input.wrap_row_counts_width != Some(rounded_wrap_width) {
                input.wrap_row_counts_width = Some(rounded_wrap_width);
                input.request_wrap_recompute();
            }
            for rows in &mut input.wrap_row_counts {
                *rows = (*rows).max(1);
            }
            let started_wrap_job = input.maybe_recompute_wrap_rows(
                display_text_str,
                line_starts.as_ref(),
                rounded_wrap_width,
                font_size,
                line_count,
                cx,
            );

            let mut row_counts_changed = input.apply_pending_dirty_wrap_updates(
                display_text_str,
                line_starts.as_ref(),
                rounded_wrap_width,
                font_size,
                !started_wrap_job,
            );

            let mut y_offsets = vec![Pixels::ZERO; line_count];
            let mut y = Pixels::ZERO;
            for (ix, rows) in input.wrap_row_counts.iter().enumerate() {
                y_offsets[ix] = y;
                y += line_height * (*rows as f32).max(1.0);
            }

            let mut visible_line_range = visible_wrapped_line_range(
                &y_offsets,
                input.wrap_row_counts.as_slice(),
                line_height,
                visible_top,
                visible_bottom,
                TEXT_INPUT_GUARD_ROWS,
            );
            let mut lines = (0..line_count)
                .map(|_| WrappedLine::default())
                .collect::<Vec<_>>();
            let mut shaped_mask = vec![false; line_count];
            let job_accepts_interpolation = pending_wrap_job_accepts_interpolated_patch(
                input.pending_wrap_job.as_ref(),
                wrap_width_key,
                line_count,
                !started_wrap_job,
            );
            let mut streamed_line_runs = input.streamed_highlight_runs_for_visible_window(
                display_text_str,
                line_starts.as_ref(),
                visible_line_range.clone(),
                &shape_style,
            );

            for line_ix in visible_line_range.clone() {
                let precomputed_runs = visible_window_runs_for_line_ix(
                    streamed_line_runs.as_deref(),
                    visible_line_range.start,
                    line_ix,
                );
                let wrapped = input.shape_wrapped_line_cached(
                    LineShapeInput {
                        line_ix,
                        line_start: line_starts.get(line_ix).copied().unwrap_or(0),
                        line_text: line_text_for_index(
                            display_text_str,
                            line_starts.as_ref(),
                            line_ix,
                        ),
                    },
                    wrap_width,
                    precomputed_runs,
                    &shape_style,
                    window,
                );
                let rows = wrapped.wrap_boundaries().len().saturating_add(1).max(1);
                let old_rows = input
                    .wrap_row_counts
                    .get(line_ix)
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                if old_rows != rows {
                    if let Some(slot) = input.wrap_row_counts.get_mut(line_ix) {
                        *slot = rows;
                    }
                    row_counts_changed = true;
                    if job_accepts_interpolation {
                        input.push_interpolated_wrap_patch(wrap_width_key, line_ix, old_rows, rows);
                    }
                }
                if let Some(slot) = lines.get_mut(line_ix) {
                    *slot = wrapped;
                }
                if let Some(mask) = shaped_mask.get_mut(line_ix) {
                    *mask = true;
                }
            }

            let cursor_line_ix = line_index_for_offset(line_starts.as_ref(), cursor, line_count);
            if cursor_line_ix < line_count
                && (cursor_line_ix < visible_line_range.start
                    || cursor_line_ix >= visible_line_range.end)
            {
                let wrapped = input.shape_wrapped_line_cached(
                    LineShapeInput {
                        line_ix: cursor_line_ix,
                        line_start: line_starts.get(cursor_line_ix).copied().unwrap_or(0),
                        line_text: line_text_for_index(
                            display_text_str,
                            line_starts.as_ref(),
                            cursor_line_ix,
                        ),
                    },
                    wrap_width,
                    None,
                    &shape_style,
                    window,
                );
                let rows = wrapped.wrap_boundaries().len().saturating_add(1).max(1);
                let old_rows = input
                    .wrap_row_counts
                    .get(cursor_line_ix)
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                if old_rows != rows {
                    if let Some(slot) = input.wrap_row_counts.get_mut(cursor_line_ix) {
                        *slot = rows;
                    }
                    row_counts_changed = true;
                    if job_accepts_interpolation {
                        input.push_interpolated_wrap_patch(
                            wrap_width_key,
                            cursor_line_ix,
                            old_rows,
                            rows,
                        );
                    }
                }
                if let Some(slot) = lines.get_mut(cursor_line_ix) {
                    *slot = wrapped;
                }
                if let Some(mask) = shaped_mask.get_mut(cursor_line_ix) {
                    *mask = true;
                }
            }

            if row_counts_changed {
                y = Pixels::ZERO;
                for (ix, rows) in input.wrap_row_counts.iter().enumerate() {
                    y_offsets[ix] = y;
                    y += line_height * (*rows as f32).max(1.0);
                }
                visible_line_range = visible_wrapped_line_range(
                    &y_offsets,
                    input.wrap_row_counts.as_slice(),
                    line_height,
                    visible_top,
                    visible_bottom,
                    TEXT_INPUT_GUARD_ROWS,
                );
                streamed_line_runs = input.streamed_highlight_runs_for_visible_window(
                    display_text_str,
                    line_starts.as_ref(),
                    visible_line_range.clone(),
                    &shape_style,
                );
                for line_ix in visible_line_range.clone() {
                    if shaped_mask.get(line_ix).copied().unwrap_or(false) {
                        continue;
                    }
                    let precomputed_runs = visible_window_runs_for_line_ix(
                        streamed_line_runs.as_deref(),
                        visible_line_range.start,
                        line_ix,
                    );
                    let wrapped = input.shape_wrapped_line_cached(
                        LineShapeInput {
                            line_ix,
                            line_start: line_starts.get(line_ix).copied().unwrap_or(0),
                            line_text: line_text_for_index(
                                display_text_str,
                                line_starts.as_ref(),
                                line_ix,
                            ),
                        },
                        wrap_width,
                        precomputed_runs,
                        &shape_style,
                        window,
                    );
                    let rows = wrapped.wrap_boundaries().len().saturating_add(1).max(1);
                    let old_rows = input
                        .wrap_row_counts
                        .get(line_ix)
                        .copied()
                        .unwrap_or(1)
                        .max(1);
                    if let Some(slot) = input.wrap_row_counts.get_mut(line_ix) {
                        *slot = rows;
                    }
                    if old_rows != rows && job_accepts_interpolation {
                        input.push_interpolated_wrap_patch(wrap_width_key, line_ix, old_rows, rows);
                    }
                    if let Some(slot) = lines.get_mut(line_ix) {
                        *slot = wrapped;
                    }
                    if let Some(mask) = shaped_mask.get_mut(line_ix) {
                        *mask = true;
                    }
                }
            }

            let total_rows = total_wrap_rows(input.wrap_row_counts.as_slice());
            let wrap_cache = Some(WrapCache {
                width: rounded_wrap_width,
                rows: total_rows,
            });

            let mut selections = Vec::with_capacity(visible_line_range.len().max(1));
            let cursor_quad = if selected_range.is_empty() {
                let line_ix = line_index_for_offset(line_starts.as_ref(), cursor, line_count);
                let start = line_starts.get(line_ix).copied().unwrap_or(0);
                let local = cursor.saturating_sub(start).min(lines[line_ix].len());
                let caret_inset_y = px(3.0);
                let caret_h = (line_height - caret_inset_y * 2.0).max(px(2.0));
                let pos = lines[line_ix]
                    .position_for_index(local, line_height)
                    .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                let top = bounds.top() + y_offsets[line_ix] + pos.y + caret_inset_y;
                Some(fill(
                    Bounds::new(point(bounds.left() + pos.x, top), size(px(1.0), caret_h)),
                    style_colors.cursor,
                ))
            } else {
                for ix in visible_line_range.clone() {
                    let start = line_starts.get(ix).copied().unwrap_or(0);
                    let next_start = line_starts
                        .get(ix + 1)
                        .copied()
                        .unwrap_or(display_text.len());
                    let line_len = lines[ix].len();
                    let line_end = start + line_len;

                    let seg_start = selected_range.start.max(start);
                    let seg_end = selected_range.end.min(next_start);
                    if seg_start >= seg_end {
                        continue;
                    }

                    let local_start = seg_start.min(line_end) - start;
                    let local_end = seg_end.min(line_end) - start;

                    let start_pos = lines[ix]
                        .position_for_index(local_start, line_height)
                        .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                    let end_pos = lines[ix]
                        .position_for_index(local_end, line_height)
                        .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));

                    let start_row = (start_pos.y / line_height).floor().max(0.0) as usize;
                    let end_row = (end_pos.y / line_height).floor().max(0.0) as usize;

                    for row in start_row..=end_row {
                        let top = bounds.top() + y_offsets[ix] + line_height * row as f32;
                        let (x0, x1) = if start_row == end_row {
                            (start_pos.x, end_pos.x)
                        } else if row == start_row {
                            (start_pos.x, bounds.size.width)
                        } else if row == end_row {
                            (Pixels::ZERO, end_pos.x)
                        } else {
                            (Pixels::ZERO, bounds.size.width)
                        };
                        selections.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + x0, top),
                                point(bounds.left() + x1, top + line_height),
                            ),
                            style_colors.selection,
                        ));
                    }
                }
                None
            };

            PrepaintState {
                layout: Some(TextInputLayout::Wrapped {
                    lines,
                    y_offsets,
                    row_counts: input.wrap_row_counts.clone(),
                }),
                cursor: cursor_quad,
                selections,
                line_starts: Some(line_starts),
                wrap_cache,
                scroll_x: px(0.0),
                visible_line_range,
            }
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        if self.input.read(cx).is_selecting {
            let input = self.input.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, _phase, _window, cx| {
                input.update(cx, |input, cx| {
                    if input.is_selecting {
                        input.select_to(input.index_for_mouse_position(event.position), cx);
                    }
                });
            });

            let input = self.input.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, _phase, _window, cx| {
                if event.button != MouseButton::Left {
                    return;
                }
                input.update(cx, |input, _cx| {
                    input.is_selecting = false;
                });
            });
        }

        for selection in prepaint.selections.drain(..) {
            window.paint_quad(selection);
        }
        let line_height = self.input.read(cx).effective_line_height(window);
        if let Some(layout) = prepaint.layout.as_ref() {
            match layout {
                TextInputLayout::Plain(lines) => {
                    for ix in prepaint.visible_line_range.clone() {
                        let Some(line) = lines.get(ix) else {
                            continue;
                        };
                        let painted = line.paint(
                            point(
                                bounds.origin.x - prepaint.scroll_x,
                                bounds.origin.y + line_height * ix as f32,
                            ),
                            line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                        debug_assert!(
                            painted.is_ok(),
                            "TextInput plain line paint failed at line index {ix}"
                        );
                    }
                }
                TextInputLayout::Wrapped {
                    lines, y_offsets, ..
                } => {
                    for ix in prepaint.visible_line_range.clone() {
                        let Some(line) = lines.get(ix) else {
                            continue;
                        };
                        let y = y_offsets.get(ix).copied().unwrap_or(Pixels::ZERO);
                        let _ = line.paint(
                            point(bounds.origin.x, bounds.origin.y + y),
                            line_height,
                            TextAlign::Left,
                            Some(bounds),
                            window,
                            cx,
                        );
                    }
                }
            }
        }

        let cursor_blink_visible = self.input.read(cx).cursor_blink_visible;
        if focus_handle.is_focused(window)
            && cursor_blink_visible
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, cx| {
            let prev_height_rows = if input.multiline && input.soft_wrap {
                input
                    .wrap_cache
                    .map(|cache| cache.rows)
                    .or(input.last_wrap_rows)
            } else {
                None
            };
            let had_pending_cursor_autoscroll = input.pending_cursor_autoscroll;
            input.last_layout = prepaint.layout.take();
            input.last_line_starts = prepaint.line_starts.clone();
            input.last_bounds = Some(bounds);
            input.last_line_height = line_height;
            input.wrap_cache = prepaint.wrap_cache;
            if input.multiline && input.soft_wrap {
                if let Some(cache) = input.wrap_cache {
                    input.last_wrap_rows = Some(cache.rows);
                }
            } else {
                input.last_wrap_rows = None;
            }
            input.scroll_x = prepaint.scroll_x;
            if had_pending_cursor_autoscroll {
                input.ensure_cursor_visible_in_vertical_scroll(cx);
            }
            let next_height_rows = if input.multiline && input.soft_wrap {
                input
                    .wrap_cache
                    .map(|cache| cache.rows)
                    .or(input.last_wrap_rows)
            } else {
                None
            };
            if prev_height_rows != next_height_rows {
                // Wrapped height changes land one frame later in the parent scroll container.
                // Keep one follow-up pass so Enter-at-EOF remains pinned to the true bottom.
                if had_pending_cursor_autoscroll && input.cursor_offset() == input.content.len() {
                    input.pending_cursor_autoscroll = true;
                }
                cx.notify();
            }
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = self.style;
        let focus = self.focus_handle.clone();
        let entity_id = cx.entity().entity_id();
        let chromeless = self.chromeless;
        let multiline = self.multiline;
        let pad_x = if chromeless { px(0.0) } else { px(8.0) };
        let pad_y = if chromeless || !multiline {
            px(0.0)
        } else {
            px(8.0)
        };
        let is_focused = focus.is_focused(window);

        if self.has_focus != is_focused {
            self.has_focus = is_focused;
            self.cursor_blink_visible = true;
            if !is_focused {
                self.cursor_blink_task.take();
                self.context_menu = None;
            }
        }

        if is_focused
            && self.cursor_blink_task.is_none()
            && crate::ui_runtime::current().uses_cursor_blink()
        {
            let task = cx.spawn(
                async move |input: gpui::WeakEntity<TextInput>, cx: &mut gpui::AsyncApp| {
                    loop {
                        smol::Timer::after(Duration::from_millis(800)).await;
                        let should_continue = input
                            .update(cx, |input, cx| {
                                if !input.has_focus {
                                    input.cursor_blink_visible = true;
                                    input.cursor_blink_task = None;
                                    cx.notify();
                                    return false;
                                }

                                if input.selected_range.is_empty() {
                                    input.cursor_blink_visible = !input.cursor_blink_visible;
                                } else {
                                    input.cursor_blink_visible = true;
                                }
                                cx.notify();
                                true
                            })
                            .unwrap_or(false);

                        if !should_continue {
                            break;
                        }
                    }
                },
            );
            self.cursor_blink_task = Some(task);
        }

        let text_surface = div()
            .w_full()
            .min_w(px(0.0))
            .px(pad_x)
            .py(pad_y)
            .overflow_hidden()
            .child(TextElement { input: cx.entity() });

        let mut input = div()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .track_focus(&focus)
            .key_context("TextInput")
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::show_character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down_right))
            .line_height(self.effective_line_height(window))
            .text_size(px(13.0))
            .when(!multiline && !chromeless, |d| d.h(px(CONTROL_HEIGHT_PX)))
            .when(!multiline, |d| d.items_center())
            .when(multiline, |d| d.items_start())
            .child(text_surface);

        if !chromeless {
            input = input
                .bg(style.background)
                .border_1()
                .rounded(px(style.radius));

            if is_focused {
                input = input.border_color(style.focus_border);
            } else {
                input = input
                    .border_color(style.border)
                    .hover(move |s| s.border_color(style.hover_border));
            }

            input = input.focus(move |s| s.border_color(style.focus_border));
        }

        let render_id = ElementId::from(("text_input_root", entity_id));
        let render_id =
            ElementId::from((render_id, if is_focused { "focused" } else { "blurred" }));
        let mut outer = div()
            // Focus changes toggle GPUI platform input handler registration during paint.
            // Key the subtree by focus state so GPUI doesn't reuse a stale unfocused paint
            // range that contains no input handlers when the field becomes focused.
            .id(render_id)
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .child(input);

        if let Some(state) = self.context_menu {
            outer = outer.child(
                deferred(
                    anchored()
                        .position(state.anchor)
                        .offset(point(px(4.0), px(4.0)))
                        .child(self.render_context_menu(state, cx)),
                )
                .priority(10_000),
            );
        }

        outer
    }
}

#[cfg(any(test, feature = "benchmarks"))]
const TEXT_INPUT_SHAPING_FINGERPRINT_SAMPLE_BYTES: usize = 64;
#[cfg(any(test, feature = "benchmarks"))]
const TEXT_INPUT_SHAPING_FINGERPRINT_MID_SAMPLES_TRUNCATED: usize = 3;
#[cfg(any(test, feature = "benchmarks"))]
const TEXT_INPUT_SHAPING_FINGERPRINT_MID_SAMPLES_UNTRUNCATED: usize = 1;

#[derive(Clone, Copy)]
struct ShapingSliceInfo<'a> {
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
fn hash_shaping_prefix_bytes(prefix_bytes: &[u8], capped_len: usize, truncated: bool) -> u64 {
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
fn shaping_slice_info(line_text: &str, max_bytes: usize) -> ShapingSliceInfo<'_> {
    ShapingSliceInfo::new(line_text, max_bytes)
}

/// Compute a stable fingerprint and capped byte length for a line that may need truncation.
/// This does NOT allocate, and on very long lines it samples representative chunks instead of
/// rescanning the full shaping prefix.
#[cfg(any(test, feature = "benchmarks"))]
fn hash_shaping_slice(line_text: &str, max_bytes: usize) -> (u64, usize) {
    let info = shaping_slice_info(line_text, max_bytes);
    (info.hash(), info.capped_len)
}

/// Build the (possibly truncated) SharedString for shaping. Only call on cache miss.
fn build_shaping_text(line_text: &str, max_bytes: usize) -> SharedString {
    shaping_slice_info(line_text, max_bytes).into_shared_string()
}

fn build_shaping_line_slice<'a>(line_text: &'a str, max_bytes: usize) -> Cow<'a, str> {
    shaping_slice_info(line_text, max_bytes).into_cow()
}

#[cfg(any(test, feature = "benchmarks"))]
fn truncate_line_for_shaping(line_text: &str, max_bytes: usize) -> (SharedString, u64) {
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

#[cfg(feature = "benchmarks")]
#[inline]
pub(crate) fn benchmark_text_input_wrap_rows_for_line(text: &str, wrap_columns: usize) -> usize {
    estimate_wrap_rows_for_line(text, wrap_columns)
}

fn wrap_width_cache_key(wrap_width: Pixels) -> i32 {
    let mut key = f32::from(wrap_width.round()) as i32;
    if key == i32::MIN {
        key = i32::MIN + 1;
    }
    key
}

fn line_index_for_offset(starts: &[usize], offset: usize, line_count: usize) -> usize {
    if line_count == 0 {
        return 0;
    }
    let mut ix = starts.partition_point(|&s| s <= offset);
    if ix == 0 {
        ix = 1;
    }
    (ix - 1).min(line_count.saturating_sub(1))
}

fn visible_vertical_window(
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

fn visible_plain_line_range(
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

fn byte_range_for_line_range(
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

fn provider_prefetch_byte_range_for_visible_window(
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

fn wrapped_line_index_for_y(
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

fn visible_wrapped_line_range(
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

fn total_wrap_rows(row_counts: &[usize]) -> usize {
    row_counts
        .iter()
        .copied()
        .map(|rows| rows.max(1))
        .sum::<usize>()
        .max(1)
}

fn wrap_columns_for_width(wrap_width: Pixels, font_size: Pixels) -> usize {
    let width_px = f32::from(wrap_width.max(px(1.0)));
    let font_px = f32::from(font_size.max(px(1.0)));
    let advance_px = (font_px * TEXT_INPUT_WRAP_CHAR_ADVANCE_FACTOR).max(1.0);
    (width_px / advance_px).floor().max(1.0) as usize
}

fn estimate_wrap_rows_for_text(text: &str, wrap_columns: usize) -> Vec<usize> {
    let line_starts = compute_line_starts(text);
    let mut rows = Vec::with_capacity(line_starts.len().max(1));
    estimate_wrap_rows_with_line_starts(text, line_starts.as_slice(), wrap_columns, &mut rows);
    rows
}

fn estimate_wrap_rows_with_line_starts(
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

fn estimate_wrap_rows_budgeted(
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
fn estimate_wrap_rows_for_line(line_text: &str, wrap_columns: usize) -> usize {
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
fn advance_ascii_segment(
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
fn advance_ascii_tab_common(rows: &mut usize, column: &mut usize, wrap_columns: usize) {
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
fn advance_ascii_tab_general(rows: &mut usize, column: &mut usize, wrap_columns: usize) {
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

fn clamp_offset_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset = offset.saturating_sub(1);
    }
    offset
}

fn expanded_dirty_wrap_line_range_for_edit(
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

fn apply_interpolated_wrap_patch_delta(rows: &mut [usize], patch: &InterpolatedWrapPatch) {
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

fn reset_interpolated_wrap_patches_on_overflow(
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

fn pending_wrap_job_accepts_interpolated_patch(
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

fn visible_window_runs_for_line_ix(
    line_runs_by_visible_line: Option<&VisibleWindowTextRuns>,
    visible_start: usize,
    line_ix: usize,
) -> Option<&[TextRun]> {
    let visible_runs = line_runs_by_visible_line?;
    let local_ix = line_ix.checked_sub(visible_start)?;
    visible_runs.line(local_ix)
}

#[derive(Clone)]
struct ActiveHighlight<'a> {
    end: usize,
    style: &'a gpui::HighlightStyle,
}

type ActiveHighlightBuffer<'a> =
    SmallVec<[ActiveHighlight<'a>; TEXT_INPUT_INLINE_ACTIVE_HIGHLIGHT_CAPACITY]>;
type LineTextRuns = SmallVec<[TextRun; TEXT_INPUT_INLINE_TEXT_RUN_CAPACITY]>;

trait TextRunSink {
    fn push_text_run(&mut self, run: TextRun);
}

impl TextRunSink for Vec<TextRun> {
    fn push_text_run(&mut self, run: TextRun) {
        self.push(run);
    }
}

impl<const N: usize> TextRunSink for SmallVec<[TextRun; N]> {
    fn push_text_run(&mut self, run: TextRun) {
        self.push(run);
    }
}

struct HighlightCursor<'a> {
    highlights: &'a [(Range<usize>, gpui::HighlightStyle)],
    next_ix: usize,
    active: ActiveHighlightBuffer<'a>,
}

impl<'a> HighlightCursor<'a> {
    fn new_at_offset(
        highlights: &'a [(Range<usize>, gpui::HighlightStyle)],
        offset: usize,
    ) -> Self {
        let next_ix = highlights.partition_point(|(range, _)| range.start < offset);
        let mut active_start = next_ix;
        let mut active = ActiveHighlightBuffer::new();
        while active_start > 0 {
            let order = active_start.saturating_sub(1);
            let Some((range, style)) = highlights.get(order) else {
                break;
            };
            if range.end <= offset {
                break;
            }
            active_start = order;
            active.push(ActiveHighlight {
                end: range.end,
                style,
            });
        }
        Self {
            highlights,
            next_ix,
            active,
        }
    }

    fn advance_to_line_start(&mut self, line_start: usize) {
        self.active.retain(|highlight| highlight.end > line_start);
        while let Some((range, style)) = self.highlights.get(self.next_ix) {
            if range.end <= line_start {
                self.next_ix = self.next_ix.saturating_add(1);
                continue;
            }
            if range.start < line_start {
                self.active.push(ActiveHighlight {
                    end: range.end,
                    style,
                });
                self.next_ix = self.next_ix.saturating_add(1);
                continue;
            }
            break;
        }
    }

    fn append_simple_runs_for_line(
        &self,
        base_font: &gpui::Font,
        base_color: gpui::Hsla,
        line_start: usize,
        line_end: usize,
        runs: &mut impl TextRunSink,
    ) -> bool {
        let next_highlight = self
            .highlights
            .get(self.next_ix)
            .filter(|(range, _)| range.start < line_end);

        match (self.active.len(), next_highlight) {
            (0, None) => {
                runs.push_text_run(text_run_for_style(
                    base_font,
                    base_color,
                    line_end.saturating_sub(line_start),
                    None,
                ));
                true
            }
            (0, Some((range, style)))
                if self
                    .highlights
                    .get(self.next_ix.saturating_add(1))
                    .map(|(next_range, _)| next_range.start >= line_end)
                    .unwrap_or(true) =>
            {
                if range.start > line_start {
                    runs.push_text_run(text_run_for_style(
                        base_font,
                        base_color,
                        range.start.saturating_sub(line_start),
                        None,
                    ));
                }
                let styled_end = range.end.min(line_end);
                if styled_end > range.start {
                    runs.push_text_run(text_run_for_style(
                        base_font,
                        base_color,
                        styled_end.saturating_sub(range.start),
                        Some(style),
                    ));
                }
                if styled_end < line_end {
                    runs.push_text_run(text_run_for_style(
                        base_font,
                        base_color,
                        line_end.saturating_sub(styled_end),
                        None,
                    ));
                }
                true
            }
            (1, None) => {
                let Some(active) = self.active.first() else {
                    return false;
                };
                let styled_end = active.end.min(line_end);
                runs.push_text_run(text_run_for_style(
                    base_font,
                    base_color,
                    styled_end.saturating_sub(line_start),
                    Some(active.style),
                ));
                if styled_end < line_end {
                    runs.push_text_run(text_run_for_style(
                        base_font,
                        base_color,
                        line_end.saturating_sub(styled_end),
                        None,
                    ));
                }
                true
            }
            _ => false,
        }
    }

    fn try_simple_runs_for_line(
        &self,
        base_font: &gpui::Font,
        base_color: gpui::Hsla,
        line_start: usize,
        line_end: usize,
    ) -> Option<LineTextRuns> {
        let mut runs = LineTextRuns::new();
        if self.append_simple_runs_for_line(base_font, base_color, line_start, line_end, &mut runs)
        {
            Some(runs)
        } else {
            None
        }
    }

    fn append_runs_for_line(
        &mut self,
        base_font: &gpui::Font,
        base_color: gpui::Hsla,
        line_start: usize,
        line_text: &str,
        runs: &mut impl TextRunSink,
    ) {
        if line_text.is_empty() {
            return;
        }
        self.advance_to_line_start(line_start);
        self.append_runs_for_current_line(base_font, base_color, line_start, line_text, runs);
    }

    fn append_runs_for_current_line(
        &mut self,
        base_font: &gpui::Font,
        base_color: gpui::Hsla,
        line_start: usize,
        line_text: &str,
        runs: &mut impl TextRunSink,
    ) {
        let line_end = line_start + line_text.len();

        let mut offset = line_start;
        while offset < line_end {
            while let Some((range, style)) = self.highlights.get(self.next_ix) {
                if range.end <= offset {
                    self.next_ix = self.next_ix.saturating_add(1);
                    continue;
                }
                if range.start > offset || range.start >= line_end {
                    break;
                }
                self.active.push(ActiveHighlight {
                    end: range.end,
                    style,
                });
                self.next_ix = self.next_ix.saturating_add(1);
            }

            let mut next_boundary = line_end;
            if let Some((next_range, _)) = self.highlights.get(self.next_ix)
                && next_range.start < line_end
            {
                next_boundary = next_boundary.min(next_range.start);
            }
            // Active highlights stay in insertion order, and later highlights
            // win precedence, so the last active entry is always the visible
            // style at the current offset.
            let top_highlight = self.active.last();
            let style = top_highlight.map(|highlight| highlight.style);
            if let Some(top_highlight) = top_highlight {
                // Only the current top highlight's end can change the visible
                // style; lower-priority highlight ends should not split runs.
                next_boundary = next_boundary.min(top_highlight.end);
            }
            if next_boundary <= offset {
                next_boundary = (offset + 1).min(line_end);
            }

            runs.push_text_run(text_run_for_style(
                base_font,
                base_color,
                next_boundary - offset,
                style,
            ));
            self.active
                .retain(|highlight| highlight.end > next_boundary);
            offset = next_boundary;
        }
    }
}

fn build_streamed_highlight_runs_for_visible_window(
    base_font: &gpui::Font,
    base_color: gpui::Hsla,
    display_text: &str,
    line_starts: &[usize],
    visible_line_range: Range<usize>,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> VisibleWindowTextRuns {
    let mut line_runs = VisibleWindowTextRuns::with_line_capacity(visible_line_range.len());
    if visible_line_range.is_empty() {
        return line_runs;
    }
    let first_line_start = line_starts
        .get(visible_line_range.start)
        .copied()
        .unwrap_or(0);
    let mut cursor = HighlightCursor::new_at_offset(highlights, first_line_start);
    for line_ix in visible_line_range {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let line_text = line_text_for_index(display_text, line_starts, line_ix);
        let capped_line_text = build_shaping_line_slice(line_text, TEXT_INPUT_MAX_LINE_SHAPE_BYTES);
        let capped_line_text = capped_line_text.as_ref();
        if !capped_line_text.is_empty() {
            let line_end = line_start.saturating_add(capped_line_text.len());
            cursor.advance_to_line_start(line_start);
            if !cursor.append_simple_runs_for_line(
                base_font,
                base_color,
                line_start,
                line_end,
                &mut line_runs.runs,
            ) {
                cursor.append_runs_for_current_line(
                    base_font,
                    base_color,
                    line_start,
                    capped_line_text,
                    &mut line_runs.runs,
                );
            }
        }
        line_runs.finish_line();
    }
    line_runs
}

fn text_run_for_style(
    base_font: &gpui::Font,
    base_color: gpui::Hsla,
    len: usize,
    style: Option<&gpui::HighlightStyle>,
) -> TextRun {
    let mut font = base_font.clone();
    let mut color = base_color;
    let mut background_color = None;
    let mut underline = None;
    let mut strikethrough = None;

    if let Some(style) = style {
        if let Some(next_color) = style.color {
            color = next_color;
        }
        if let Some(next_weight) = style.font_weight {
            font.weight = next_weight;
        }
        if let Some(next_style) = style.font_style {
            font.style = next_style;
        }
        background_color = style.background_color;
        underline = style.underline;
        strikethrough = style.strikethrough;
        if let Some(fade_out) = style.fade_out {
            color.a *= (1.0 - fade_out).clamp(0.0, 1.0);
        }
    }

    TextRun {
        len,
        font,
        color,
        background_color,
        underline,
        strikethrough,
    }
}

fn runs_for_line(
    base_font: &gpui::Font,
    base_color: gpui::Hsla,
    line_start: usize,
    line_text: &str,
    highlights: Option<&[(Range<usize>, gpui::HighlightStyle)]>,
) -> LineTextRuns {
    if line_text.is_empty() {
        return LineTextRuns::new();
    }

    let Some(highlights) = highlights else {
        let mut runs = LineTextRuns::new();
        runs.push(text_run_for_style(
            base_font,
            base_color,
            line_text.len(),
            None,
        ));
        return runs;
    };

    let line_end = line_start.saturating_add(line_text.len());
    let mut cursor = HighlightCursor::new_at_offset(highlights, line_start);
    if let Some(runs) = cursor.try_simple_runs_for_line(base_font, base_color, line_start, line_end)
    {
        return runs;
    }
    let mut runs = LineTextRuns::new();
    cursor.append_runs_for_line(base_font, base_color, line_start, line_text, &mut runs);
    runs
}

#[cfg(feature = "benchmarks")]
fn hash_text_runs_for_benchmark(runs: &[TextRun], hasher: &mut FxHasher) {
    runs.len().hash(hasher);
    let mut total = 0usize;
    for run in runs {
        total = total.saturating_add(run.len);
        run.len.hash(hasher);
        run.color.a.to_bits().hash(hasher);
    }
    total.hash(hasher);
}

#[cfg(feature = "benchmarks")]
pub(crate) fn benchmark_text_input_runs_legacy_visible_window(
    text: &str,
    line_starts: &[usize],
    visible_line_range: Range<usize>,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> u64 {
    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let mut hasher = FxHasher::default();
    for line_ix in visible_line_range {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let line_text = line_text_for_index(text, line_starts, line_ix);
        let (capped_line_text, _) =
            truncate_line_for_shaping(line_text, TEXT_INPUT_MAX_LINE_SHAPE_BYTES);
        let runs = runs_for_line(
            &base_font,
            base_color,
            line_start,
            capped_line_text.as_ref(),
            Some(highlights),
        );
        hash_text_runs_for_benchmark(runs.as_slice(), &mut hasher);
    }
    hasher.finish()
}

#[cfg(feature = "benchmarks")]
pub(crate) fn benchmark_text_input_runs_streamed_visible_window(
    text: &str,
    line_starts: &[usize],
    visible_line_range: Range<usize>,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> u64 {
    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let line_runs = build_streamed_highlight_runs_for_visible_window(
        &base_font,
        base_color,
        text,
        line_starts,
        visible_line_range,
        highlights,
    );
    let mut hasher = FxHasher::default();
    for local_ix in 0..line_runs.len() {
        if let Some(runs) = line_runs.line(local_ix) {
            hash_text_runs_for_benchmark(runs, &mut hasher);
        }
    }
    hasher.finish()
}

fn with_alpha(mut color: Rgba, alpha: f32) -> Rgba {
    color.a = alpha;
    color
}

#[cfg(target_os = "macos")]
fn primary_modifier_label() -> &'static str {
    "Cmd"
}

#[cfg(not(target_os = "macos"))]
fn primary_modifier_label() -> &'static str {
    "Ctrl"
}

fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(8);
    starts.push(0);
    for (ix, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(ix + 1);
        }
    }
    starts
}

fn line_text_for_index<'a>(text: &'a str, starts: &[usize], line_ix: usize) -> &'a str {
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
    }
    text.get(start..end).unwrap_or("")
}

fn mask_text_for_display(text: &str) -> String {
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

fn line_for_offset(starts: &[usize], lines: &[ShapedLine], offset: usize) -> (usize, usize) {
    let mut ix = starts.partition_point(|&s| s <= offset);
    if ix == 0 {
        ix = 1;
    }
    let line_ix = (ix - 1).min(lines.len().saturating_sub(1));
    let start = starts.get(line_ix).copied().unwrap_or(0);
    let local = offset.saturating_sub(start).min(lines[line_ix].len());
    (line_ix, local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_text_preserves_length_and_newlines() {
        let input = "a\nb\r\nc";
        let masked = mask_text_for_display(input);
        assert_eq!(masked.len(), input.len());
        assert_eq!(masked, "*\n*\r\n*");
    }

    #[test]
    fn mask_text_removes_original_characters() {
        let input = "secret-passphrase";
        let masked = mask_text_for_display(input);
        assert_ne!(masked, input);
        assert!(masked.chars().all(|ch| ch == '*'));
    }

    #[test]
    fn truncate_line_for_shaping_respects_utf8_boundary_and_appends_suffix() {
        let input = "éééé";
        let (truncated, hash) = truncate_line_for_shaping(input, 5);
        assert_eq!(truncated.as_ref(), "é…");
        // hash_shaping_slice must be consistent with truncate_line_for_shaping
        let (hash2, _) = hash_shaping_slice(input, 5);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn visible_plain_line_range_applies_guard_rows() {
        let range = visible_plain_line_range(100, px(20.0), px(200.0), px(260.0), 2);
        assert_eq!(range, 8..16);
    }

    #[test]
    fn provider_prefetch_byte_range_extends_visible_window_with_guard_rows() {
        let text = std::iter::repeat_n("x", 100).collect::<Vec<_>>().join("\n");
        let line_starts = compute_line_starts(text.as_str());
        let range = provider_prefetch_byte_range_for_visible_window(
            line_starts.as_slice(),
            text.len(),
            100,
            px(20.0),
            px(600.0),
            px(660.0),
        );

        assert_eq!(range, 12..116);
    }

    #[test]
    fn provider_prefetch_byte_range_clamps_to_document_bounds() {
        let text = std::iter::repeat_n("x", 10).collect::<Vec<_>>().join("\n");
        let line_starts = compute_line_starts(text.as_str());
        let range = provider_prefetch_byte_range_for_visible_window(
            line_starts.as_slice(),
            text.len(),
            10,
            px(20.0),
            px(0.0),
            px(20.0),
        );

        assert_eq!(range, 0..text.len());
    }

    #[test]
    fn wrapped_line_index_and_visible_range_use_row_counts() {
        let row_counts = vec![1, 3, 1, 2, 1];
        let y_offsets = vec![px(0.0), px(10.0), px(40.0), px(50.0), px(70.0)];
        let line_height = px(10.0);

        assert_eq!(
            wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(35.0)),
            1
        );
        let range =
            visible_wrapped_line_range(&y_offsets, &row_counts, line_height, px(42.0), px(58.0), 0);
        assert_eq!(range, 2..4);
    }

    #[test]
    fn compute_line_starts_and_line_text_handle_trailing_newline() {
        let text = "alpha\nbeta\n";
        let starts = compute_line_starts(text);
        assert_eq!(starts, vec![0, 6, 11]);
        assert_eq!(line_text_for_index(text, starts.as_slice(), 0), "alpha");
        assert_eq!(line_text_for_index(text, starts.as_slice(), 1), "beta");
        assert_eq!(line_text_for_index(text, starts.as_slice(), 2), "");
        assert_eq!(line_text_for_index(text, starts.as_slice(), 3), "");
    }

    #[test]
    fn wrapped_line_index_for_y_handles_row_boundaries() {
        let row_counts = vec![2, 1, 3];
        let y_offsets = vec![px(0.0), px(20.0), px(30.0)];
        let line_height = px(10.0);

        assert_eq!(
            wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(0.0)),
            0
        );
        assert_eq!(
            wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(19.0)),
            0
        );
        assert_eq!(
            wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(20.0)),
            1
        );
        assert_eq!(
            wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(30.0)),
            2
        );
        assert_eq!(
            wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(250.0)),
            2
        );
    }

    #[test]
    fn estimate_wrap_rows_for_line_handles_tabs_and_overflow() {
        assert_eq!(estimate_wrap_rows_for_line("abcd", 4), 1);
        assert_eq!(estimate_wrap_rows_for_line("abcde", 4), 2);
        assert_eq!(estimate_wrap_rows_for_line("a\tb", 4), 2);
    }

    #[test]
    fn estimate_wrap_rows_for_line_matches_reference_for_ascii_tabs() {
        fn reference_wrap_rows_for_line(line_text: &str, wrap_columns: usize) -> usize {
            if line_text.is_empty() {
                return 1;
            }
            let wrap_columns = wrap_columns.max(1);
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
                        rows += 1;
                    }
                    rows += width / wrap_columns;
                    column = width % wrap_columns;
                    if column == 0 {
                        column = wrap_columns;
                    }
                    continue;
                }

                if column + width > wrap_columns {
                    rows += 1;
                    column = width;
                } else {
                    column += width;
                }
            }
            rows.max(1)
        }

        let samples = [
            "",
            "\t",
            "a\tb",
            "ab\tcd\tef",
            "\tsection_00000\tvalue = token\ttoken\ttoken\ttoken\t",
            "token\ttoken\ttoken\ttoken\ttoken\t",
            "abcd",
            "abcde",
            "\t\t\t",
            "trailing-tab\t",
        ];

        for wrap_columns in (TEXT_INPUT_WRAP_TAB_STOP_COLUMNS + 1)..=12 {
            for sample in samples {
                assert_eq!(
                    estimate_wrap_rows_for_line(sample, wrap_columns),
                    reference_wrap_rows_for_line(sample, wrap_columns),
                    "sample={sample:?}, wrap_columns={wrap_columns}"
                );
            }
        }
    }

    #[test]
    fn expanded_dirty_wrap_line_range_for_edit_keeps_tab_affected_line_dirty() {
        let text = "ax\tbb\nnext";
        let starts = compute_line_starts(text);
        let dirty =
            expanded_dirty_wrap_line_range_for_edit(text, starts.as_slice(), &(1..1), &(1..2));
        assert_eq!(dirty, 0..1);
    }

    #[test]
    fn apply_interpolated_wrap_patch_delta_adjusts_rows_by_delta() {
        let mut rows = vec![6, 5, 4, 3];
        let patch = InterpolatedWrapPatch {
            width_key: 80,
            line_start: 1,
            old_rows: vec![3, 2],
            new_rows: vec![5, 1],
        };
        apply_interpolated_wrap_patch_delta(rows.as_mut_slice(), &patch);
        assert_eq!(rows, vec![6, 7, 3, 3]);
    }

    #[test]
    fn reset_interpolated_wrap_patches_on_overflow_requests_full_recompute() {
        let patch = InterpolatedWrapPatch {
            width_key: 80,
            line_start: 12,
            old_rows: vec![1],
            new_rows: vec![2],
        };

        let mut below_limit =
            vec![patch.clone(); TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES.saturating_sub(1)];
        let mut recompute_requested = false;
        assert!(!reset_interpolated_wrap_patches_on_overflow(
            &mut below_limit,
            &mut recompute_requested
        ));
        assert_eq!(
            below_limit.len(),
            TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES.saturating_sub(1)
        );
        assert!(!recompute_requested);

        let mut saturated = vec![patch; TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES];
        assert!(reset_interpolated_wrap_patches_on_overflow(
            &mut saturated,
            &mut recompute_requested
        ));
        assert!(saturated.is_empty());
        assert!(recompute_requested);
    }

    #[test]
    fn pending_wrap_job_accepts_interpolated_patch_respects_prepaint_launch_gate() {
        let job = PendingWrapJob {
            sequence: 5,
            width_key: 120,
            line_count: 64,
            wrap_columns: 80,
        };

        assert!(pending_wrap_job_accepts_interpolated_patch(
            Some(&job),
            120,
            64,
            true
        ));
        assert!(!pending_wrap_job_accepts_interpolated_patch(
            Some(&job),
            120,
            64,
            false
        ));
        assert!(!pending_wrap_job_accepts_interpolated_patch(
            Some(&job),
            121,
            64,
            true
        ));
        assert!(!pending_wrap_job_accepts_interpolated_patch(
            Some(&job),
            120,
            63,
            true
        ));
        assert!(!pending_wrap_job_accepts_interpolated_patch(
            None, 120, 64, true
        ));
    }

    fn runs_fingerprint(runs: &[TextRun]) -> Vec<String> {
        runs.iter().map(|run| format!("{run:?}")).collect()
    }

    fn run_color_at_offset(runs: &[TextRun], offset: usize) -> gpui::Hsla {
        let mut cursor = 0usize;
        for run in runs {
            let end = cursor.saturating_add(run.len);
            if offset < end {
                return run.color;
            }
            cursor = end;
        }
        panic!("offset {offset} is outside the run coverage");
    }

    #[test]
    fn highlight_runs_skip_hidden_overlap_end_boundaries() {
        let text = "abcdefghijklmnop";
        let line_starts = compute_line_starts(text);
        let style_low = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let style_mid = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.33, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let style_high = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let mut highlights = vec![(0..10, style_low), (2..8, style_mid), (4..12, style_high)];
        highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

        let base_font = gpui::font(".SystemUIFont");
        let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
        let streamed = build_streamed_highlight_runs_for_visible_window(
            &base_font,
            base_color,
            text,
            line_starts.as_slice(),
            0..1,
            highlights.as_slice(),
        );
        let legacy_runs =
            runs_for_line(&base_font, base_color, 0, text, Some(highlights.as_slice()));

        assert_eq!(streamed.line(0).unwrap_or(&[]).len(), 4);
        assert_eq!(legacy_runs.len(), 4);
        assert_eq!(
            run_color_at_offset(streamed.line(0).unwrap_or(&[]), 1),
            style_low.color.expect("style_low color should exist")
        );
        assert_eq!(
            run_color_at_offset(streamed.line(0).unwrap_or(&[]), 3),
            style_mid.color.expect("style_mid color should exist")
        );
        assert_eq!(
            run_color_at_offset(streamed.line(0).unwrap_or(&[]), 6),
            style_high.color.expect("style_high color should exist")
        );
        assert_eq!(
            run_color_at_offset(streamed.line(0).unwrap_or(&[]), 14),
            base_color
        );
    }

    #[test]
    fn streamed_highlight_runs_match_legacy_visible_window() {
        let mut text = String::new();
        for ix in 0..160usize {
            text.push_str(format!("line_{ix:03}_abcdefghijklmnopqrstuvwxyz0123456789\n").as_str());
        }
        let line_starts = compute_line_starts(text.as_str());

        let style_a = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let style_b = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.33, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let style_c = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)> = Vec::new();
        for line_ix in 0..line_starts.len() {
            let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
            let line_len =
                line_text_for_index(text.as_str(), line_starts.as_slice(), line_ix).len();
            if line_len < 24 {
                continue;
            }
            if line_ix % 2 == 0 {
                highlights.push((line_start + 1..line_start + 14, style_a));
            }
            if line_ix % 3 == 0 {
                highlights.push((line_start + 6..line_start + line_len.min(24), style_b));
            }
        }
        let wide_start = line_starts.get(18).copied().unwrap_or(0).saturating_add(2);
        let wide_end = line_starts
            .get(140)
            .copied()
            .unwrap_or(text.len())
            .saturating_add(20)
            .min(text.len());
        highlights.push((wide_start..wide_end, style_c));
        highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

        let visible_range = 47..121;
        let base_font = gpui::font(".SystemUIFont");
        let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
        let streamed = build_streamed_highlight_runs_for_visible_window(
            &base_font,
            base_color,
            text.as_str(),
            line_starts.as_slice(),
            visible_range.clone(),
            highlights.as_slice(),
        );
        assert_eq!(streamed.len(), visible_range.len());

        for local_ix in 0..streamed.len() {
            let line_ix = visible_range.start + local_ix;
            let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
            let line_text = line_text_for_index(text.as_str(), line_starts.as_slice(), line_ix);
            let (capped, _) = truncate_line_for_shaping(line_text, TEXT_INPUT_MAX_LINE_SHAPE_BYTES);
            let legacy_runs = runs_for_line(
                &base_font,
                base_color,
                line_start,
                capped.as_ref(),
                Some(highlights.as_slice()),
            );
            assert_eq!(
                runs_fingerprint(streamed.line(local_ix).unwrap_or(&[])),
                runs_fingerprint(legacy_runs.as_slice())
            );
        }
    }

    #[test]
    fn streamed_highlight_runs_preserve_latest_overlap_precedence() {
        let text = "abcdefghijklmnop";
        let line_starts = compute_line_starts(text);
        let style_low = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let style_high = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let mut highlights = vec![(2..12, style_low), (4..10, style_high)];
        highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

        let base_font = gpui::font(".SystemUIFont");
        let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
        let streamed = build_streamed_highlight_runs_for_visible_window(
            &base_font,
            base_color,
            text,
            line_starts.as_slice(),
            0..1,
            highlights.as_slice(),
        );
        let legacy_runs =
            runs_for_line(&base_font, base_color, 0, text, Some(highlights.as_slice()));
        assert_eq!(
            runs_fingerprint(streamed.line(0).unwrap_or(&[])),
            runs_fingerprint(legacy_runs.as_slice())
        );

        assert_eq!(
            run_color_at_offset(streamed.line(0).unwrap_or(&[]), 3),
            style_low.color.expect("style_low color should exist")
        );
        assert_eq!(
            run_color_at_offset(streamed.line(0).unwrap_or(&[]), 6),
            style_high.color.expect("style_high color should exist")
        );
    }

    #[test]
    fn highlight_runs_single_carry_in_highlight_matches_streamed() {
        let text = "prefix highlight continues here\nsuffix line";
        let line_starts = compute_line_starts(text);
        let style = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.12, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let mut highlights = vec![(3..30, style)];
        highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

        let base_font = gpui::font(".SystemUIFont");
        let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
        let streamed = build_streamed_highlight_runs_for_visible_window(
            &base_font,
            base_color,
            text,
            line_starts.as_slice(),
            0..2,
            highlights.as_slice(),
        );

        for line_ix in 0..2 {
            let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
            let line_text = line_text_for_index(text, line_starts.as_slice(), line_ix);
            let legacy_runs = runs_for_line(
                &base_font,
                base_color,
                line_start,
                line_text,
                Some(highlights.as_slice()),
            );
            assert_eq!(
                runs_fingerprint(streamed.line(line_ix).unwrap_or(&[])),
                runs_fingerprint(legacy_runs.as_slice())
            );
        }
    }

    #[test]
    fn resolve_provider_highlights_caches_by_epoch_and_range() {
        use std::sync::atomic::Ordering;

        let (call_count, provider) = make_counting_provider();

        // Simulate the cache behavior without needing a full GPUI context.
        let mut cache: Option<ProviderHighlightCache> = None;
        let epoch: u64 = 1;

        let h1 = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        assert!(!h1.pending);
        assert_eq!(h1.highlights.len(), 1);
        assert_eq!(h1.highlights[0].0, 0..100);

        // Same range and epoch → cached, no new call.
        let h2 = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&h1.highlights, &h2.highlights));

        // Contained range → cached, no new call.
        let h3 = test_resolve_with_cache(&mut cache, epoch, 20, 80, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&h1.highlights, &h3.highlights));

        // Wider range → new call.
        let _h4 = test_resolve_with_cache(&mut cache, epoch, 0, 120, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);

        // Different epoch → new call even for same range.
        let _h5 = test_resolve_with_cache(&mut cache, epoch + 1, 0, 120, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn resolve_provider_highlights_reuses_multiple_cached_ranges() {
        use std::sync::atomic::Ordering;

        let (call_count, provider) = make_counting_provider();

        let mut cache: Option<ProviderHighlightCache> = None;
        let epoch = 1;

        let first = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
        let second = test_resolve_with_cache(&mut cache, epoch, 200, 300, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);

        let first_subrange = test_resolve_with_cache(&mut cache, epoch, 20, 80, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
        assert!(Arc::ptr_eq(&first.highlights, &first_subrange.highlights));

        let second_subrange = test_resolve_with_cache(&mut cache, epoch, 220, 260, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
        assert!(Arc::ptr_eq(&second.highlights, &second_subrange.highlights));

        let cache = cache.expect("resolved ranges should populate the provider cache");
        assert_eq!(cache.highlight_epoch, epoch);
        assert_eq!(cache.entries.len(), 2);
    }

    #[test]
    fn resolve_provider_highlights_prefers_smallest_containing_cached_range() {
        use std::sync::atomic::Ordering;

        let (call_count, provider) = make_counting_provider();

        let mut cache: Option<ProviderHighlightCache> = None;
        let epoch = 1;

        let narrow = test_resolve_with_cache(&mut cache, epoch, 50, 150, &provider);
        let wide = test_resolve_with_cache(&mut cache, epoch, 0, 200, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);

        let resolved = test_resolve_with_cache(&mut cache, epoch, 60, 140, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
        assert!(
            Arc::ptr_eq(&resolved.highlights, &narrow.highlights),
            "the smallest cached containing slice should win even if a wider slice is newer"
        );
        assert!(
            !Arc::ptr_eq(&resolved.highlights, &wide.highlights),
            "the wider containing slice should not be reused when a tighter one exists"
        );

        let cache = cache.expect("resolved ranges should populate the provider cache");
        assert_eq!(cached_provider_ranges(&cache), vec![0..200, 50..150]);
    }

    #[test]
    fn resolve_provider_highlights_cache_is_bounded() {
        use std::sync::atomic::Ordering;

        let (call_count, provider) = make_counting_provider();

        let mut cache: Option<ProviderHighlightCache> = None;
        let epoch = 1;
        for window in 0..TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT {
            let start = window * 100;
            let end = start + 100;
            let _ = test_resolve_with_cache(&mut cache, epoch, start, end, &provider);
        }
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT
        );

        let _ = test_resolve_with_cache(
            &mut cache,
            epoch,
            TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT * 100,
            TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT * 100 + 100,
            &provider,
        );
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT + 1
        );

        let cache_ref = cache.as_ref().expect("cache should retain recent ranges");
        assert_eq!(
            cache_ref.entries.len(),
            TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT
        );

        let _ = test_resolve_with_cache(&mut cache, epoch, 0, 50, &provider);
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT + 2,
            "the oldest cached slice should be evicted once the cache reaches its bound"
        );
    }

    #[test]
    fn resolve_provider_highlights_cache_hit_promotes_entry_before_eviction() {
        use std::sync::atomic::Ordering;

        let (call_count, provider) = make_counting_provider();

        let mut cache: Option<ProviderHighlightCache> = None;
        let epoch = 1;

        let first = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
        let _second = test_resolve_with_cache(&mut cache, epoch, 100, 200, &provider);
        let _third = test_resolve_with_cache(&mut cache, epoch, 200, 300, &provider);
        let _fourth = test_resolve_with_cache(&mut cache, epoch, 300, 400, &provider);
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT
        );

        let promoted = test_resolve_with_cache(&mut cache, epoch, 20, 80, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 4);
        assert!(Arc::ptr_eq(&promoted.highlights, &first.highlights));

        let _fifth = test_resolve_with_cache(&mut cache, epoch, 400, 500, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 5);

        let cache_ref = cache
            .as_ref()
            .expect("cache should retain recent ranges after a bounded insert");
        assert_eq!(
            cached_provider_ranges(cache_ref),
            vec![200..300, 300..400, 0..100, 400..500]
        );

        let reused = test_resolve_with_cache(&mut cache, epoch, 10, 50, &provider);
        assert_eq!(call_count.load(Ordering::SeqCst), 5);
        assert!(
            Arc::ptr_eq(&reused.highlights, &first.highlights),
            "a cache hit should keep the promoted slice resident across the next eviction"
        );

        let _evicted = test_resolve_with_cache(&mut cache, epoch, 120, 180, &provider);
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            6,
            "the cold slice should be evicted instead of the recently-used one"
        );
    }

    #[test]
    fn highlight_provider_binding_key_reuses_existing_provider_when_unchanged() {
        assert!(!should_reset_highlight_provider_binding(
            true,
            Some(41),
            Some(41)
        ));
    }

    #[test]
    fn highlight_provider_binding_key_rebinds_when_missing_changed_or_unkeyed() {
        assert!(should_reset_highlight_provider_binding(
            false,
            Some(41),
            Some(41)
        ));
        assert!(should_reset_highlight_provider_binding(
            true,
            Some(41),
            Some(42)
        ));
        assert!(should_reset_highlight_provider_binding(
            true,
            Some(41),
            None
        ));
    }

    fn test_resolve_with_cache(
        cache: &mut Option<ProviderHighlightCache>,
        epoch: u64,
        byte_start: usize,
        byte_end: usize,
        provider: &HighlightProvider,
    ) -> ResolvedProviderHighlights {
        let requested_range = byte_start..byte_end;
        if let Some(resolved) = cache
            .as_mut()
            .and_then(|c| c.resolve(epoch, &requested_range))
        {
            return resolved;
        }
        let mut result = provider.resolve(requested_range.clone());
        result
            .highlights
            .sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        let pending = result.pending;
        let highlights = Arc::new(result.highlights);
        cache
            .get_or_insert_with(|| ProviderHighlightCache::new(epoch))
            .insert(epoch, requested_range, pending, Arc::clone(&highlights));
        ResolvedProviderHighlights {
            pending,
            highlights,
        }
    }

    fn make_counting_provider() -> (Arc<std::sync::atomic::AtomicUsize>, HighlightProvider) {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let call_count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&call_count);
        let provider = HighlightProvider::from_fn(move |range: Range<usize>| {
            counter.fetch_add(1, Ordering::SeqCst);
            vec![(
                range,
                gpui::HighlightStyle {
                    color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
                    ..gpui::HighlightStyle::default()
                },
            )]
        });

        (call_count, provider)
    }

    fn cached_provider_ranges(cache: &ProviderHighlightCache) -> Vec<Range<usize>> {
        cache
            .entries
            .iter()
            .map(|entry| entry.byte_start..entry.byte_end)
            .collect()
    }

    struct DualProviders {
        first_calls: Arc<std::sync::atomic::AtomicUsize>,
        second_calls: Arc<std::sync::atomic::AtomicUsize>,
        first_color: gpui::Hsla,
        second_color: gpui::Hsla,
        first: HighlightProvider,
        second: HighlightProvider,
    }

    fn make_dual_providers() -> DualProviders {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let first_color = gpui::hsla(0.0, 1.0, 0.5, 1.0);
        let second_color = gpui::hsla(0.66, 1.0, 0.5, 1.0);
        let first_calls = Arc::new(AtomicUsize::new(0));
        let second_calls = Arc::new(AtomicUsize::new(0));
        let fc = Arc::clone(&first_calls);
        let sc = Arc::clone(&second_calls);
        DualProviders {
            first_calls,
            second_calls,
            first_color,
            second_color,
            first: HighlightProvider::from_fn(move |range: Range<usize>| {
                fc.fetch_add(1, Ordering::SeqCst);
                vec![(
                    range,
                    gpui::HighlightStyle {
                        color: Some(first_color),
                        ..gpui::HighlightStyle::default()
                    },
                )]
            }),
            second: HighlightProvider::from_fn(move |range: Range<usize>| {
                sc.fetch_add(1, Ordering::SeqCst);
                vec![(
                    range,
                    gpui::HighlightStyle {
                        color: Some(second_color),
                        ..gpui::HighlightStyle::default()
                    },
                )]
            }),
        }
    }

    #[gpui::test]
    fn stable_highlight_provider_binding_key_preserves_existing_provider_and_cache(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::sync::atomic::Ordering;

        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let dp = make_dual_providers();

        cx.update(|_window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha\nbeta", cx);
                input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

                let initial_resolved = input.resolve_provider_highlights(0, 5);
                assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
                assert_eq!(initial_resolved.highlights[0].1.color, Some(dp.first_color));

                let initial_cache = input
                    .provider_highlight_cache
                    .as_ref()
                    .expect("initial resolve should populate the provider cache");
                assert_eq!(initial_cache.entries.len(), 1);
                let initial_entry = initial_cache
                    .entries
                    .last()
                    .expect("initial cache should contain one provider slice");
                assert_eq!(initial_entry.byte_start, 0);
                assert_eq!(initial_entry.byte_end, 5);

                let initial_highlight_epoch = input.highlight_epoch;
                let initial_shape_epoch = input.shape_style_epoch;
                let initial_cached_highlights = Arc::clone(&initial_entry.highlights);

                input.set_highlight_provider_with_key(41, dp.second.clone(), cx);

                assert_eq!(
                    input.highlight_epoch, initial_highlight_epoch,
                    "reinstalling the same binding key should not invalidate provider highlights"
                );
                assert_eq!(
                    input.shape_style_epoch, initial_shape_epoch,
                    "reinstalling the same binding key should not invalidate shaped rows"
                );

                let cache = input
                    .provider_highlight_cache
                    .as_ref()
                    .expect("stable binding key should preserve the cached provider range");
                let cache_entry = cache
                    .entries
                    .last()
                    .expect("stable binding key should keep the cached provider slice");
                assert!(
                    Arc::ptr_eq(&cache_entry.highlights, &initial_cached_highlights),
                    "stable binding key should preserve the existing cached highlight vector"
                );

                let resolved = input.resolve_provider_highlights(1, 4);
                assert_eq!(
                    dp.first_calls.load(Ordering::SeqCst),
                    1,
                    "stable binding key should keep using the original provider/cache"
                );
                assert_eq!(
                    dp.second_calls.load(Ordering::SeqCst),
                    0,
                    "stable binding key should not bind a replacement provider"
                );
                assert!(Arc::ptr_eq(
                    &resolved.highlights,
                    &initial_cached_highlights
                ));
                assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
            });
        });
    }

    #[gpui::test]
    fn replace_utf8_range_clears_shaped_row_caches(cx: &mut gpui::TestAppContext) {
        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: true,
                    soft_wrap: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        cx.update(|_window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha\nbeta", cx);

                input.plain_line_cache.insert(
                    ShapedRowCacheKey {
                        line_ix: 0,
                        wrap_width_key: i32::MIN,
                    },
                    ShapedLine::default(),
                );
                input.wrapped_line_cache.insert(
                    ShapedRowCacheKey {
                        line_ix: 0,
                        wrap_width_key: wrap_width_cache_key(px(320.0)),
                    },
                    (),
                );

                assert_eq!(input.plain_line_cache.len(), 1);
                assert_eq!(input.wrapped_line_cache.len(), 1);

                input.replace_utf8_range(0..5, "gamma", cx);

                assert!(
                    input.plain_line_cache.is_empty(),
                    "text edits must invalidate cached plain shaped rows"
                );
                assert!(
                    input.wrapped_line_cache.is_empty(),
                    "text edits must invalidate cached wrapped shaped rows"
                );
            });
        });
    }

    #[gpui::test]
    fn changed_highlight_provider_binding_key_rebinds_and_clears_cached_range(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::sync::atomic::Ordering;

        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let dp = make_dual_providers();

        cx.update(|_window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha\nbeta", cx);
                input.set_highlight_provider_with_key(41, dp.first.clone(), cx);
                let _ = input.resolve_provider_highlights(0, 5);
                assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
                let previous_highlight_epoch = input.highlight_epoch;
                let previous_shape_epoch = input.shape_style_epoch;

                input.set_highlight_provider_with_key(42, dp.second.clone(), cx);

                assert!(
                    input.provider_highlight_cache.is_none(),
                    "changing the binding key should drop the cached provider range"
                );
                assert!(
                    input.highlight_epoch > previous_highlight_epoch,
                    "changing the binding key should invalidate provider highlight epochs"
                );
                assert!(
                    input.shape_style_epoch > previous_shape_epoch,
                    "changing the binding key should invalidate shaped text caches"
                );

                let resolved = input.resolve_provider_highlights(0, 5);
                assert_eq!(
                    dp.first_calls.load(Ordering::SeqCst),
                    1,
                    "rebinding should stop using the previous provider"
                );
                assert_eq!(
                    dp.second_calls.load(Ordering::SeqCst),
                    1,
                    "rebinding should resolve highlights from the new provider"
                );
                assert_eq!(resolved.highlights[0].1.color, Some(dp.second_color));

                let cache = input
                    .provider_highlight_cache
                    .as_ref()
                    .expect("resolving after a rebind should repopulate the provider cache");
                assert_eq!(cache.highlight_epoch, input.highlight_epoch);
                assert_eq!(cache.entries.len(), 1);
                let cache_entry = cache
                    .entries
                    .last()
                    .expect("rebind resolve should cache the requested provider slice");
                assert_eq!(cache_entry.byte_start, 0);
                assert_eq!(cache_entry.byte_end, 5);
            });
        });
    }

    #[gpui::test]
    fn replace_utf8_range_invalidates_cached_provider_highlights(cx: &mut gpui::TestAppContext) {
        use std::sync::atomic::Ordering;

        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let dp = make_dual_providers();

        cx.update(|_window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha\nbeta", cx);
                input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

                let _ = input.resolve_provider_highlights(0, 5);
                assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
                let previous_highlight_epoch = input.highlight_epoch;
                assert!(
                    input.provider_highlight_cache.is_some(),
                    "initial resolve should populate the provider cache"
                );

                let inserted = input.replace_utf8_range(0..5, "gamma", cx);
                assert_eq!(inserted, 0..5);
                assert!(
                    input.provider_highlight_cache.is_none(),
                    "text edits should clear cached provider ranges"
                );
                assert!(
                    input.highlight_epoch > previous_highlight_epoch,
                    "text edits should invalidate provider highlight epochs"
                );

                let resolved = input.resolve_provider_highlights(0, 5);
                assert_eq!(
                    dp.first_calls.load(Ordering::SeqCst),
                    2,
                    "after an edit, the stable provider should be asked for a fresh range"
                );
                assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
            });
        });
    }

    #[gpui::test]
    fn set_text_invalidates_cached_provider_highlights(cx: &mut gpui::TestAppContext) {
        use std::sync::atomic::Ordering;

        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let dp = make_dual_providers();

        cx.update(|_window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha\nbeta", cx);
                input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

                let _ = input.resolve_provider_highlights(0, 5);
                assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
                let previous_highlight_epoch = input.highlight_epoch;
                assert!(
                    input.provider_highlight_cache.is_some(),
                    "initial resolve should populate the provider cache"
                );

                input.set_text("gamma\nbeta", cx);

                assert!(
                    input.provider_highlight_cache.is_none(),
                    "set_text should clear cached provider ranges"
                );
                assert!(
                    input.highlight_epoch > previous_highlight_epoch,
                    "set_text should invalidate provider highlight epochs"
                );

                let resolved = input.resolve_provider_highlights(0, 5);
                assert_eq!(
                    dp.first_calls.load(Ordering::SeqCst),
                    2,
                    "after set_text, the stable provider should be asked for a fresh range"
                );
                assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
            });
        });
    }

    #[gpui::test]
    fn undo_invalidates_cached_provider_highlights(cx: &mut gpui::TestAppContext) {
        use std::sync::atomic::Ordering;

        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let dp = make_dual_providers();

        cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha\nbeta", cx);
                input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

                let _ = input.resolve_provider_highlights(0, 5);
                assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);

                let inserted = input.replace_utf8_range(0..5, "gamma", cx);
                assert_eq!(inserted, 0..5);
                let _ = input.resolve_provider_highlights(0, 5);
                assert_eq!(dp.first_calls.load(Ordering::SeqCst), 2);
                let previous_highlight_epoch = input.highlight_epoch;

                input.undo(&Undo, window, cx);

                assert_eq!(input.text(), "alpha\nbeta");
                assert!(
                    input.provider_highlight_cache.is_none(),
                    "undo should clear cached provider ranges restored from the old snapshot"
                );
                assert!(
                    input.highlight_epoch > previous_highlight_epoch,
                    "undo should invalidate provider highlight epochs"
                );

                let resolved = input.resolve_provider_highlights(0, 5);
                assert_eq!(
                    dp.first_calls.load(Ordering::SeqCst),
                    3,
                    "after undo, the provider should be asked for a fresh range"
                );
                assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
            });
        });
    }

    #[gpui::test]
    fn redo_restores_text_after_undo(cx: &mut gpui::TestAppContext) {
        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: false,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha", cx);
                let inserted = input.replace_utf8_range(0..5, "beta", cx);
                assert_eq!(inserted, 0..4);
                assert_eq!(input.text(), "beta");

                input.undo(&Undo, window, cx);
                assert_eq!(input.text(), "alpha");

                input.redo(&Redo, window, cx);
                assert_eq!(input.text(), "beta");
                assert!(input.redo_stack.is_empty());
            });
        });
    }

    #[gpui::test]
    fn redo_is_cleared_by_a_new_edit_after_undo(cx: &mut gpui::TestAppContext) {
        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: false,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha", cx);
                let inserted = input.replace_utf8_range(0..5, "beta", cx);
                assert_eq!(inserted, 0..4);

                input.undo(&Undo, window, cx);
                assert_eq!(input.text(), "alpha");
                assert_eq!(input.redo_stack.len(), 1);

                let inserted = input.replace_utf8_range(0..5, "gamma", cx);
                assert_eq!(inserted, 0..5);
                assert_eq!(input.text(), "gamma");
                assert!(input.redo_stack.is_empty());

                input.redo(&Redo, window, cx);
                assert_eq!(input.text(), "gamma");
                assert!(input.redo_stack.is_empty());
            });
        });
    }

    #[gpui::test]
    fn redo_is_noop_when_input_is_read_only(cx: &mut gpui::TestAppContext) {
        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: false,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha", cx);
                let inserted = input.replace_utf8_range(0..5, "beta", cx);
                assert_eq!(inserted, 0..4);

                input.undo(&Undo, window, cx);
                assert_eq!(input.text(), "alpha");
                assert_eq!(input.redo_stack.len(), 1);

                input.set_read_only(true, cx);
                input.redo(&Redo, window, cx);
                assert_eq!(input.text(), "alpha");
                assert_eq!(input.redo_stack.len(), 1);
            });
        });
    }

    #[gpui::test]
    fn replace_text_in_range_invalidates_cached_provider_highlights(cx: &mut gpui::TestAppContext) {
        use std::sync::atomic::Ordering;

        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let dp = make_dual_providers();

        cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha\nbeta", cx);
                input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

                let _ = input.resolve_provider_highlights(0, 5);
                assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
                let previous_highlight_epoch = input.highlight_epoch;
                assert!(
                    input.provider_highlight_cache.is_some(),
                    "initial resolve should populate the provider cache"
                );

                input.replace_text_in_range(Some(0..5), "gamma", window, cx);

                assert_eq!(input.text(), "gamma\nbeta");
                assert!(
                    input.provider_highlight_cache.is_none(),
                    "IME replace_text_in_range should clear cached provider ranges"
                );
                assert!(
                    input.highlight_epoch > previous_highlight_epoch,
                    "IME replace_text_in_range should invalidate provider highlight epochs"
                );

                let resolved = input.resolve_provider_highlights(0, 5);
                assert_eq!(
                    dp.first_calls.load(Ordering::SeqCst),
                    2,
                    "after replace_text_in_range, the stable provider should be asked for a fresh range"
                );
                assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
            });
        });
    }

    #[gpui::test]
    fn replace_and_mark_text_in_range_invalidates_cached_provider_highlights(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::sync::atomic::Ordering;

        let (input, cx) = cx.add_window_view(|window, cx| {
            TextInput::new(
                TextInputOptions {
                    multiline: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let dp = make_dual_providers();

        cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha\nbeta", cx);
                input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

                let _ = input.resolve_provider_highlights(0, 5);
                assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
                let previous_highlight_epoch = input.highlight_epoch;
                assert!(
                    input.provider_highlight_cache.is_some(),
                    "initial resolve should populate the provider cache"
                );

                input.replace_and_mark_text_in_range(Some(0..5), "gamma", None, window, cx);

                assert_eq!(input.text(), "gamma\nbeta");
                assert_eq!(input.marked_range, Some(0..5));
                assert!(
                    input.provider_highlight_cache.is_none(),
                    "IME replace_and_mark_text_in_range should clear cached provider ranges"
                );
                assert!(
                    input.highlight_epoch > previous_highlight_epoch,
                    "IME replace_and_mark_text_in_range should invalidate provider highlight epochs"
                );

                let resolved = input.resolve_provider_highlights(0, 5);
                assert_eq!(
                    dp.first_calls.load(Ordering::SeqCst),
                    2,
                    "after replace_and_mark_text_in_range, the stable provider should be asked for a fresh range"
                );
                assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
            });
        });
    }

    #[test]
    fn highlight_provider_with_pending_uses_custom_callbacks() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let pending = Arc::new(AtomicBool::new(true));
        let pending_for_resolve = Arc::clone(&pending);
        let pending_for_check = Arc::clone(&pending);
        let drain_calls = Arc::new(AtomicUsize::new(0));
        let drain_calls_for_provider = Arc::clone(&drain_calls);
        let provider = HighlightProvider::with_pending(
            move |range: Range<usize>| HighlightProviderResult {
                highlights: vec![(
                    range,
                    gpui::HighlightStyle {
                        color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
                        ..gpui::HighlightStyle::default()
                    },
                )],
                pending: pending_for_resolve.load(Ordering::SeqCst),
            },
            move || {
                drain_calls_for_provider.fetch_add(1, Ordering::SeqCst);
                pending.store(false, Ordering::SeqCst);
                1
            },
            move || pending_for_check.load(Ordering::SeqCst),
        );

        let first = provider.resolve(4..12);
        assert!(first.pending);
        assert_eq!(first.highlights[0].0, 4..12);
        assert!(provider.has_pending());
        assert_eq!(provider.drain_pending(), 1);
        assert_eq!(drain_calls.load(Ordering::SeqCst), 1);
        assert!(!provider.has_pending());

        let second = provider.resolve(4..12);
        assert!(!second.pending);
    }
}
