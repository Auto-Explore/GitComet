//! `MainPaneView` and the state, cache-entry, and key types its fields hold.

use super::*;
use crate::kit::text_model::TextModelSnapshot;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;

#[derive(Clone, Debug)]
pub(in crate::view) struct VersionedCachedDiffStyledText {
    pub(in crate::view) syntax_epoch: u64,
    pub(in crate::view) query_generation: u64,
    pub(in crate::view) styled: CachedDiffStyledText,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct StashedResolvedOutlineState {
    pub(in crate::view) text: TextModelSnapshot,
    pub(in crate::view) line_starts: Arc<[usize]>,
    pub(in crate::view) marker_segments: Vec<conflict_resolver::ConflictSegment>,
    pub(in crate::view) view_mode: ConflictResolverViewMode,
    pub(in crate::view) outline: ResolvedOutlineData,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct FileDiffStyleCacheEpochs {
    pub(in crate::view) split_left: u64,
    pub(in crate::view) split_right: u64,
}

impl FileDiffStyleCacheEpochs {
    pub(in crate::view) fn bump_left(&mut self) {
        self.split_left = self.split_left.wrapping_add(1);
    }

    pub(in crate::view) fn bump_right(&mut self) {
        self.split_right = self.split_right.wrapping_add(1);
    }

    pub(in crate::view) fn bump_both(&mut self) {
        self.bump_left();
        self.bump_right();
    }

    pub(in crate::view) fn split_epoch(self, region: crate::view::DiffTextRegion) -> u64 {
        match region {
            crate::view::DiffTextRegion::SplitLeft => self.split_left,
            crate::view::DiffTextRegion::SplitRight => self.split_right,
            crate::view::DiffTextRegion::Inline => 0,
        }
    }

    pub(in crate::view) fn inline_epoch(self, kind: gitcomet_core::domain::DiffLineKind) -> u64 {
        match kind {
            gitcomet_core::domain::DiffLineKind::Remove => self.split_left,
            gitcomet_core::domain::DiffLineKind::Add
            | gitcomet_core::domain::DiffLineKind::Context => self.split_right,
            gitcomet_core::domain::DiffLineKind::Header
            | gitcomet_core::domain::DiffLineKind::Hunk => 0,
        }
    }
}

pub(in crate::view) const FILE_DIFF_WORD_HIGHLIGHT_CACHE_MAX_ENTRIES: usize = 4_096;

#[derive(Clone, Debug, Default)]
pub(in crate::view) struct FileDiffSplitWordHighlights {
    pub(in crate::view) old: Arc<[Range<usize>]>,
    pub(in crate::view) new: Arc<[Range<usize>]>,
}

pub(in crate::view) fn versioned_cached_diff_styled_text_is_current(
    entry: Option<&VersionedCachedDiffStyledText>,
    syntax_epoch: u64,
) -> Option<&CachedDiffStyledText> {
    let entry = entry?;
    (entry.syntax_epoch == syntax_epoch).then_some(&entry.styled)
}

pub(in crate::view) fn versioned_query_cached_diff_styled_text_is_current(
    entry: Option<&VersionedCachedDiffStyledText>,
    syntax_epoch: u64,
    query_generation: u64,
) -> Option<&CachedDiffStyledText> {
    let entry = entry?;
    (entry.syntax_epoch == syntax_epoch && entry.query_generation == query_generation)
        .then_some(&entry.styled)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum PreparedSyntaxViewMode {
    FileDiffSplitLeft,
    FileDiffSplitRight,
    WorktreePreview,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) struct PreparedSyntaxDocumentKey {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) target_rev: u64,
    pub(in crate::view) file_path: std::path::PathBuf,
    pub(in crate::view) view_mode: PreparedSyntaxViewMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) enum CollapsedDiffExpansionKind {
    #[default]
    None,
    Up,
    Down,
    Both,
    Short,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct CollapsedDiffHunk {
    pub(in crate::view) src_ix: usize,
    pub(in crate::view) base_row_start: usize,
    pub(in crate::view) base_row_end_exclusive: usize,
    pub(in crate::view) has_additions: bool,
    pub(in crate::view) has_removals: bool,
    pub(in crate::view) reveal_up_lines: usize,
    pub(in crate::view) reveal_down_lines: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct CollapsedDiffReveal {
    pub(in crate::view) up_lines: usize,
    pub(in crate::view) down_lines: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct CollapsedDiffProjectionIdentity {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) diff_target: DiffTarget,
    pub(in crate::view) file_path: std::path::PathBuf,
    pub(in crate::view) diff_whitespace_mode: DiffWhitespaceMode,
    pub(in crate::view) patch_content_signature: Option<u64>,
    pub(in crate::view) file_content_signature: Option<u64>,
}

/// The `ensure_diff_visible_indices` cache key: changes whenever the visible
/// rows are laid out afresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct DiffVisibleLayoutKey {
    pub(in crate::view) len: usize,
    pub(in crate::view) view: DiffViewMode,
    pub(in crate::view) is_file_view: bool,
    pub(in crate::view) projection_rev: u64,
}

/// Which sides of a diff a row, or a whole block, changes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct DiffChangeSides {
    pub(in crate::view) removed: bool,
    pub(in crate::view) added: bool,
}

impl DiffChangeSides {
    pub(in crate::view) fn union(self, other: Self) -> Self {
        Self {
            removed: self.removed || other.removed,
            added: self.added || other.added,
        }
    }
}

/// The change block F2/F3 last landed on, for the accent bar and outline
/// that mark it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct DiffFocusedChangeBlock {
    /// Visual row navigation selected; the marks hide once the selection moves.
    pub(in crate::view) anchor: usize,
    /// Source-visible rows, so word-wrap continuations are covered too.
    pub(in crate::view) rows: std::ops::Range<usize>,
    /// Split views outline the old column only if the block removes something
    /// and the new column only if it adds something.
    pub(in crate::view) sides: DiffChangeSides,
    pub(in crate::view) layout: DiffVisibleLayoutKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum DiffChangeSide {
    Removed,
    Added,
}

/// How one visual row paints its part of the focused block's marks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct FocusedChangeBlockRow {
    /// First and last visual rows close the outline.
    pub(in crate::view) top: bool,
    pub(in crate::view) bottom: bool,
    /// What this row itself changes; inline rows outline in their own colour.
    pub(in crate::view) row_sides: DiffChangeSides,
    pub(in crate::view) block_sides: DiffChangeSides,
}

impl FocusedChangeBlockRow {
    /// Inline rows outline in their own colour; a `\ No newline` marker row,
    /// which changes nothing itself, borrows the block's.
    pub(in crate::view) fn inline_outline(self) -> DiffChangeSide {
        if self.row_sides.removed {
            DiffChangeSide::Removed
        } else if self.row_sides.added || !self.block_sides.removed {
            DiffChangeSide::Added
        } else {
            DiffChangeSide::Removed
        }
    }

    /// A split column is outlined only if the block changes that side.
    pub(in crate::view) fn column_outline(self, old_side: bool) -> Option<DiffChangeSide> {
        if old_side {
            self.block_sides.removed.then_some(DiffChangeSide::Removed)
        } else {
            self.block_sides.added.then_some(DiffChangeSide::Added)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum CollapsedDiffVisibleRow {
    HunkHeader {
        src_ix: usize,
        expansion_kind: CollapsedDiffExpansionKind,
        display_src_ix: Option<usize>,
        hidden_rows: usize,
    },
    FileRow {
        row_ix: usize,
    },
}

impl CollapsedDiffVisibleRow {
    pub(in crate::view) const fn row_ix(self) -> Option<usize> {
        match self {
            Self::FileRow { row_ix } => Some(row_ix),
            Self::HunkHeader { .. } => None,
        }
    }

    pub(in crate::view) const fn header_display_src_ix(self) -> Option<usize> {
        match self {
            Self::HunkHeader { display_src_ix, .. } => display_src_ix,
            Self::FileRow { .. } => None,
        }
    }

    pub(in crate::view) const fn header_action_src_ix(self) -> Option<usize> {
        match self {
            Self::HunkHeader { src_ix, .. } => Some(src_ix),
            Self::FileRow { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum DiffHorizontalScrollColumn {
    Primary,
    SplitRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct DiffWrapVisualRow {
    pub(in crate::view) source_visible_ix: usize,
    pub(in crate::view) wrap_ix: usize,
    pub(in crate::view) primary_range: rows::DiffWrapByteRange,
    pub(in crate::view) secondary_range: rows::DiffWrapByteRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct DiffWrapVisibleCacheKey {
    pub(in crate::view) source_len: usize,
    pub(in crate::view) diff_view: DiffViewMode,
    pub(in crate::view) is_file_view: bool,
    pub(in crate::view) collapsed_projection_active: bool,
    pub(in crate::view) projection_rev: u64,
    pub(in crate::view) diff_cache_rev: u64,
    pub(in crate::view) file_diff_cache_seq: u64,
    pub(in crate::view) inline_columns: usize,
    pub(in crate::view) split_columns: usize,
    /// Columns a file preview row wraps at. The preview is a single column
    /// with its own gutter, so neither of the diff's two widths describes it.
    pub(in crate::view) preview_columns: usize,
    /// Bumped when the previewed file's content changes, so the rows are
    /// rebuilt for the new text rather than kept from the old.
    pub(in crate::view) preview_content_rev: u64,
    pub(in crate::view) reveal_whitespace_chars: bool,
}

impl DiffHorizontalScrollColumn {
    pub(in crate::view) const fn index(self) -> usize {
        match self {
            Self::Primary => 0,
            Self::SplitRight => 1,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::view) struct DiffHorizontalScrollState {
    pub(in crate::view) content_widths: [Pixels; 2],
}

/// Memoized blame author-time range, keyed by a clone of the blame `Arc`. See
/// [`MainPaneView::blame_time_range_cache`].
pub(in crate::view) type BlameTimeRangeCache = Option<(
    std::sync::Arc<Vec<gitcomet_core::services::BlameLine>>,
    Option<(i64, i64)>,
)>;

/// Test hook that observes the pane before a click worker completes.
#[cfg(test)]
pub(in crate::view) type ClickSyntaxCompleteHook = Arc<dyn Fn(&MainPaneView) + Send + Sync>;

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::view) struct FileImagePreviewAnimationSide {
    pub(in crate::view) image_id: Option<gpui::ImageId>,
    pub(in crate::view) frame_index: usize,
    pub(in crate::view) frame_started_at: Option<std::time::Instant>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::view) struct FileImagePreviewAnimation {
    pub(in crate::view) old: FileImagePreviewAnimationSide,
    pub(in crate::view) new: FileImagePreviewAnimationSide,
    pub(in crate::view) scheduled_deadline: Option<std::time::Instant>,
    pub(in crate::view) generation: u64,
}

impl DiffHorizontalScrollState {
    pub(in crate::view) fn new() -> Self {
        Self {
            content_widths: [px(0.0); 2],
        }
    }

    pub(in crate::view) fn reset(&mut self) {
        self.content_widths = [px(0.0); 2];
    }

    pub(in crate::view) fn record_content_width(
        &mut self,
        column: DiffHorizontalScrollColumn,
        width: Pixels,
    ) -> bool {
        let ix = column.index();
        if width > self.content_widths[ix] {
            self.content_widths[ix] = width;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Default)]
pub(in crate::view::panes::main) enum RemoteMarkdownImageDocumentSet {
    #[default]
    None,
    Worktree(Arc<crate::view::markdown_preview::MarkdownPreviewDocument>),
    Diff(Arc<crate::view::markdown_preview::MarkdownPreviewDiff>),
    Conflict([Option<Arc<crate::view::markdown_preview::MarkdownPreviewDocument>>; 3]),
}

impl RemoteMarkdownImageDocumentSet {
    pub(in crate::view::panes::main) fn has_same_identity(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) => true,
            (Self::Worktree(left), Self::Worktree(right)) => Arc::ptr_eq(left, right),
            (Self::Diff(left), Self::Diff(right)) => Arc::ptr_eq(left, right),
            (Self::Conflict(left), Self::Conflict(right)) => {
                left.iter().zip(right).all(|(left, right)| {
                    matches!((left, right), (None, None))
                        || matches!((left, right), (Some(left), Some(right)) if Arc::ptr_eq(left, right))
                })
            }
            _ => false,
        }
    }
}

#[derive(Default)]
pub(in crate::view::panes::main) struct RemoteMarkdownImageSummaryCache {
    pub(in crate::view::panes::main) documents: RemoteMarkdownImageDocumentSet,
    pub(in crate::view::panes::main) approval_revision: u64,
    pub(in crate::view::panes::main) urls: Arc<FxHashSet<SharedString>>,
    pub(in crate::view::panes::main) has_blocked: bool,
}

pub(crate) struct MainPaneView {
    // Store, theme, and window wiring.
    pub(in crate::view) store: crate::view::pane_store::PaneStore,
    pub(in crate::view::panes::main) state: Arc<AppState>,
    pub(in crate::view) view_mode: GitCometViewMode,
    pub(in crate::view) focused_mergetool_labels: Option<FocusedMergetoolLabels>,
    pub(in crate::view) focused_mergetool_exit_code: Option<Arc<AtomicI32>>,
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) date_time_format: DateTimeFormat,
    pub(in crate::view::panes::main) _ui_model_subscription: gpui::Subscription,
    pub(in crate::view::panes::main) _text_selection_owner_subscription: gpui::Subscription,
    pub(in crate::view) root_view: WeakEntity<GitCometView>,
    pub(in crate::view) tooltip_host: WeakEntity<TooltipHost>,
    pub(in crate::view::panes::main) notify_fingerprint: u64,
    pub(in crate::view) active_context_menu_invoker: Option<SharedString>,

    pub(in crate::view) hosted_decor: Option<super::super::hosted_binding::HostedDiffDecor>,
    pub(in crate::view) hosted_content_width: Option<Pixels>,

    // Surrounding pane layout, as last rendered.
    pub(in crate::view) last_window_size: Size<Pixels>,
    pub(in crate::view) layout_sidebar_render_width: Pixels,
    pub(in crate::view) layout_details_render_width: Pixels,
    pub(in crate::view) layout_sidebar_collapsed: bool,
    pub(in crate::view) layout_details_collapsed: bool,

    // Display settings: whitespace, merge tool, diff view, blame, tabs, split.
    pub(in crate::view) reveal_whitespace_chars: bool,
    /// section 30 merge tool: auto-advance to the next unresolved conflict after a
    /// source pick. Persisted UI setting (cog menu).
    pub(in crate::view) mergetool_auto_advance: bool,
    /// section 30 merge tool: default for the collapse-unchanged-context mode when a
    /// conflicted file opens. Persisted UI setting (cog menu).
    pub(in crate::view) mergetool_collapse_unchanged: bool,
    /// section 30 merge tool: sync the resolved output pane's scroll with the source
    /// columns (in modes where they share a row space). Persisted UI setting
    /// (cog menu). Merge-tool-specific rather than a general diff setting
    /// because the resolver ships as a standalone tool.
    pub(in crate::view) mergetool_output_scroll_sync: bool,
    /// section 30 merge tool: show per-column and resolved-output line number
    /// gutters. Persisted UI setting (cog menu).
    pub(in crate::view) mergetool_show_line_numbers: bool,
    /// section 30 merge tool: last-used view mode (true = 3-way). Fresh opens of
    /// base-present conflicts default to this; toolbar toggle persists it.
    pub(in crate::view) mergetool_view_three_way: bool,
    pub(in crate::view) diff_view: DiffViewMode,
    pub(in crate::view) annotate_enabled: bool,
    /// Width (design px) of the annotate column; user-resizable, session-local.
    pub(in crate::view) annotate_column_width: f32,
    /// Active annotate-column resize drag, if any.
    pub(in crate::view) annotate_resize: Option<AnnotateResizeState>,
    /// Blame annotation sub-area currently hovered (row index + area). Drives the
    /// accent highlight and tooltip for the annotation column on the next paint.
    pub(in crate::view) blame_annot_hover: Option<(usize, crate::view::rows::AnnotArea)>,
    /// Diff row whose stage/unstage gutter button is currently hovered, as the
    /// row index plus which column's gutter it sits in. Drives painting the
    /// button and its tooltip on the next paint; `None` means none is showing.
    pub(in crate::view) diff_stage_gutter_hover: Option<crate::view::rows::DiffStageHover>,
    /// Painted bounds of each row's stage-gutter cell, recorded during paint so
    /// tests can drive the button without duplicating its geometry.
    pub(in crate::view) diff_stage_gutter_cells:
        FxHashMap<(usize, crate::view::rows::DiffStageSlot), gpui::Bounds<Pixels>>,
    /// Memoized `(min, max)` author-time range for the currently loaded blame,
    /// keyed by a clone of the blame `Arc`. The range never changes after load,
    /// so this avoids rescanning all blame lines on every render frame. Holding
    /// the `Arc` (rather than a bare pointer) keeps the allocation alive while
    /// cached, so a reloaded blame can never alias the same address and return a
    /// stale range.
    pub(in crate::view) blame_time_range_cache: BlameTimeRangeCache,
    pub(in crate::view) rendered_preview_modes: RenderedPreviewModes,
    pub(in crate::view) remote_markdown_images: RemoteMarkdownImages,
    pub(in crate::view) diff_word_wrap: bool,
    /// The settings' tab size; a file's attributes or the user's choice win.
    pub(in crate::view) default_tab_size: u8,
    /// Width used by this pane's current layout and selection geometry.
    pub(in crate::view) display_tab_width: usize,
    pub(in crate::view) diff_show_line_numbers: bool,
    pub(in crate::view) diff_scroll_sync: DiffScrollSync,
    pub(in crate::view) diff_content_mode: DiffContentMode,
    pub(in crate::view) diff_whitespace_mode: DiffWhitespaceMode,
    pub(in crate::view) diff_split_ratio: f32,
    pub(in crate::view) diff_split_resize: Option<DiffSplitResizeState>,
    pub(in crate::view) diff_split_last_synced_x: [Pixels; 2],
    pub(in crate::view) diff_split_last_synced_y: [Pixels; 2],
    pub(in crate::view) diff_horizontal_scroll: DiffHorizontalScrollState,

    // Patch diff rows and their per-row indexes.
    pub(in crate::view) diff_cache_repo_id: Option<RepoId>,
    pub(in crate::view) diff_cache_rev: u64,
    pub(in crate::view) diff_cache_content_signature: Option<u64>,
    /// The last patch `patch_diff_content_signature` hashed and its result.
    /// Holding the `Arc` keeps its address from being reused by another diff.
    pub(in crate::view) patch_signature_memo: Option<(Arc<gitcomet_core::domain::Diff>, u64)>,
    pub(in crate::view) diff_cache_target: Option<DiffTarget>,
    pub(in crate::view) diff_cache: Arc<[AnnotatedDiffLine]>,
    pub(in crate::view) diff_row_provider:
        Option<Arc<super::super::diff_cache::PagedPatchDiffRows>>,
    pub(in crate::view) diff_split_row_provider:
        Option<Arc<super::super::diff_cache::PagedPatchSplitRows>>,
    pub(in crate::view) diff_file_for_src_ix: Vec<Option<Arc<str>>>,
    pub(in crate::view) diff_language_for_src_ix: Vec<Option<rows::DiffSyntaxLanguage>>,
    pub(in crate::view) diff_yaml_block_scalar_for_src_ix: Vec<bool>,
    pub(in crate::view) diff_click_kinds: Vec<DiffClickKind>,
    pub(in crate::view) diff_line_kind_for_src_ix: Vec<gitcomet_core::domain::DiffLineKind>,
    pub(in crate::view) diff_visual_line_kind_for_src_ix: Vec<gitcomet_core::domain::DiffLineKind>,
    pub(in crate::view) diff_hide_unified_header_for_src_ix: Vec<bool>,
    pub(in crate::view) diff_header_display_cache: FxHashMap<usize, SharedString>,
    pub(in crate::view) diff_split_cache: Arc<[PatchSplitRow]>,
    pub(in crate::view) diff_split_cache_len: usize,

    // Diff panel focus, raw input, and submodule summary.
    pub(in crate::view) diff_panel_focus_handle: FocusHandle,
    pub(in crate::view) diff_autoscroll_pending: bool,
    pub(in crate::view) diff_raw_input: Entity<components::TextInput>,
    /// The error-text containers' scroll, so a drag selection can scroll them.
    pub(in crate::view) diff_raw_scroll: ScrollHandle,
    pub(in crate::view) submodule_summary_cache:
        Option<super::super::submodule_summary::SubmoduleSummaryCache>,
    pub(in crate::view) submodule_hash_inputs: Vec<Entity<components::TextInput>>,

    // Visible-row projection: filtered rows, word wrap, collapsed hunks.
    pub(in crate::view) diff_visible_indices: Arc<[usize]>,
    pub(in crate::view) diff_visible_inline_map:
        Option<super::super::diff_cache::PatchInlineVisibleMap>,
    pub(in crate::view) diff_wrap_visible_rows: Arc<[DiffWrapVisualRow]>,
    pub(in crate::view) diff_wrap_visible_cache_key: Option<DiffWrapVisibleCacheKey>,
    pub(in crate::view) collapsed_diff_hunks: Vec<CollapsedDiffHunk>,
    pub(in crate::view) collapsed_diff_hunk_ix_by_src_ix: FxHashMap<usize, usize>,
    pub(in crate::view) collapsed_diff_reveals: FxHashMap<usize, CollapsedDiffReveal>,
    pub(in crate::view) collapsed_diff_visible_rows: Arc<[CollapsedDiffVisibleRow]>,
    pub(in crate::view) collapsed_diff_hunk_visible_indices: Vec<usize>,
    pub(in crate::view) collapsed_diff_header_display_cache: FxHashMap<usize, SharedString>,
    pub(in crate::view) collapsed_diff_projection_identity: Option<CollapsedDiffProjectionIdentity>,
    pub(in crate::view) diff_visible_cache_len: usize,
    pub(in crate::view) diff_visible_view: DiffViewMode,
    pub(in crate::view) diff_visible_is_file_view: bool,
    pub(in crate::view) diff_visible_projection_rev: u64,
    pub(in crate::view) diff_visible_cache_projection_rev: u64,

    // Per-row render caches: scrollbar markers, word highlights, styled text.
    pub(in crate::view) diff_scrollbar_markers_cache: Vec<components::ScrollbarMarker>,
    pub(in crate::view) diff_word_highlights: Vec<Option<Vec<Range<usize>>>>,
    pub(in crate::view) diff_word_highlights_inflight: Option<u64>,
    pub(in crate::view) diff_file_stats: Vec<Option<(usize, usize)>>,
    pub(in crate::view) diff_text_segments_cache: Vec<Option<VersionedCachedDiffStyledText>>,
    pub(in crate::view) diff_text_query_segments_cache: Vec<Option<VersionedCachedDiffStyledText>>,
    pub(in crate::view) diff_text_query_cache_query: SharedString,
    pub(in crate::view) diff_text_query_cache_options: super::super::diff_search::DiffSearchOptions,
    pub(in crate::view) diff_text_query_cache_matcher_shared:
        Option<Arc<super::super::diff_search::DiffSearchMatcher>>,
    pub(in crate::view) diff_text_query_cache_generation: u64,

    // Row and text selection, click highlights, and per-frame hit targets.
    pub(in crate::view) diff_selection_anchor: Option<usize>,
    pub(in crate::view) diff_selection_range: Option<(usize, usize)>,
    pub(in crate::view) diff_focused_change_block: Option<DiffFocusedChangeBlock>,
    pub(in crate::view) diff_text_selecting: bool,
    pub(in crate::view) diff_text_anchor: Option<DiffTextPos>,
    pub(in crate::view) diff_text_head: Option<DiffTextPos>,
    /// Which window's text selection the diff/preview character selection owns.
    /// See [`crate::text_selection_owner`].
    pub(in crate::view) diff_text_selection_owner: crate::text_selection_owner::SelectionOwnerToken,
    pub(in crate::view::panes::main) diff_text_autoscroll_seq: u64,
    pub(in crate::view::panes::main) diff_text_autoscroll_target: Option<DiffTextAutoscrollTarget>,
    pub(in crate::view::panes::main) diff_text_last_mouse_pos: Point<Pixels>,
    pub(in crate::view) diff_suppress_clicks_remaining: u8,
    /// The delimiter pair the last click selected, already projected onto rows.
    ///
    /// Not folded into any cache key: unlike the file editor's highlight runs,
    /// this is painted as a quad outside every cached artifact, and `KeyedCanvas`
    /// re-runs prepaint and paint every frame regardless of its revision key.
    pub(in crate::view) diff_text_pair_match: Option<DiffTextPairMatch>,
    /// Every place the clicked name appears, already projected onto rows and
    /// bucketed by the row that paints it.
    ///
    /// Separate from `diff_text_pair_match` because a click produces both: the
    /// name's other uses, and the construct enclosing it. Bucketed because the
    /// paint path asks per row per region per frame, and scanning a flat list of
    /// up to `MAX_OCCURRENCES` for each of them is work proportional to rows
    /// times matches, repeated at frame rate for as long as the highlight is up.
    pub(in crate::view) diff_text_occurrences:
        FxHashMap<(usize, DiffTextRegion), smallvec::SmallVec<[Range<usize>; 4]>>,
    /// A click waiting for a cold full-document syntax parse. The second region
    /// is the real old/new side to prepare (inline rows still belong to one of
    /// those documents). Projection resets clear this before its worker can
    /// replay stale coordinates.
    pub(in crate::view) diff_text_pending_syntax_click: Option<(DiffTextPos, DiffTextRegion)>,
    pub(in crate::view) diff_text_hitboxes: FxHashMap<(usize, DiffTextRegion), DiffTextHitbox>,
    /// Non-text Markdown blocks that still carry logical copy text. Rebuilt
    /// every frame alongside `diff_text_hitboxes`.
    pub(in crate::view) diff_text_motion_targets: Vec<DiffTextMotionTarget>,
    /// A search match whose row still has to be brought into view sideways, and
    /// how many more frames to keep trying for.
    ///
    /// The vertical jump is deferred to the list's own prepaint and the row is
    /// only measurable once it paints at its new position, which is not always
    /// the very next frame — the frame that applies the scroll can still be
    /// painting the rows it was showing before. The budget is what stops a row
    /// that never paints from leaving the request live for good.
    pub(in crate::view) diff_search_horizontal_reveal: Option<(usize, u8)>,
    /// Where the merge tool's column rows painted their text this frame, for the
    /// sideways half of a search reveal. Rebuilt every frame like
    /// [`Self::diff_text_hitboxes`].
    pub(in crate::view) conflict_text_hitboxes:
        FxHashMap<(usize, ThreeWayColumn), crate::view::mod_helpers::ConflictTextHitbox>,

    // Shaped text layout cache.
    pub(in crate::view) diff_text_layout_cache_epoch: u64,
    pub(in crate::view) diff_text_layout_cache: FxHashMap<u64, DiffTextLayoutCacheEntry>,

    // Quick search.
    pub(in crate::view) diff_search_active: bool,
    pub(in crate::view) diff_search_query: SharedString,
    pub(in crate::view) diff_search_options: super::super::diff_search::DiffSearchOptions,
    pub(in crate::view) diff_search_regex_error: Option<SharedString>,
    pub(in crate::view) diff_search_matches: Vec<usize>,
    pub(in crate::view) diff_search_inline_patch_trigram_index:
        Option<super::super::diff_search::DiffSearchVisibleTrigramIndex>,
    pub(in crate::view) diff_search_match_ix: Option<usize>,
    pub(in crate::view) diff_search_debounce_seq: u64,
    pub(in crate::view) diff_search_pending_previous_query: Option<SharedString>,
    pub(in crate::view) diff_search_worker_running: bool,
    /// `diff_search_debounce_seq` the running worker may publish under.
    pub(in crate::view) diff_search_worker_seq: u64,
    pub(in crate::view) diff_search_pending_finalize:
        super::super::diff_search::DiffSearchFinalizeMode,
    pub(in crate::view) diff_search_cancellation:
        Option<gitcomet_core::services::CancellationToken>,
    pub(in crate::view) diff_search_document: Option<(
        super::super::diff_search::SearchDocumentKey,
        Arc<super::super::diff_search::SearchDocument>,
    )>,
    pub(in crate::view) diff_search_pending_navigation: isize,
    pub(in crate::view) diff_search_probe_action: u64,
    pub(in crate::view) diff_search_probe_render: u64,
    pub(in crate::view) diff_search_scroll: ScrollHandle,
    pub(in crate::view) diff_search_input: Entity<components::TextInput>,
    pub(in crate::view::panes::main) _diff_search_subscription: gpui::Subscription,

    // File-to-file diff rows, source-backed sides, and their syntax.
    pub(in crate::view) file_diff_cache_repo_id: Option<RepoId>,
    pub(in crate::view) file_diff_cache_rev: u64,
    pub(in crate::view) file_diff_cache_content_signature: Option<u64>,
    pub(in crate::view) file_diff_cache_whitespace_mode: DiffWhitespaceMode,
    pub(in crate::view) file_diff_cache_target: Option<DiffTarget>,
    pub(in crate::view) file_diff_cache_error:
        Option<crate::view::panes::main::diff_cache::FileDiffCacheError>,
    pub(in crate::view) file_diff_cache_path: Option<std::path::PathBuf>,
    pub(in crate::view) file_diff_cache_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) file_diff_cache_rows: Arc<[FileDiffRow]>,
    pub(in crate::view) file_diff_row_provider:
        Option<Arc<super::super::diff_cache::PagedFileDiffRows>>,
    /// Text read back from a source-backed side for a click, kept alive.
    ///
    /// Not just a cache: the prepared-document identity is keyed partly on the
    /// text's *address*, so handing it a `SharedString` that is dropped when the
    /// call returns leaves an identity pointing at freed memory, which a later
    /// allocation of the same length can alias. Retaining it also means a second
    /// click resolves by identity instead of re-reading and re-parsing the file.
    ///
    /// The read happens inline only for small files and otherwise on the click
    /// syntax worker. Cleared whenever the cache rebuilds, so what it holds is
    /// always the body the current generation's line index was built from.
    pub(in crate::view) file_diff_pair_syntax_text: FxHashMap<DiffTextRegion, SharedString>,
    /// Source sides currently being read and parsed for an interactive click.
    /// Prevents repeated clicks from launching duplicate full-document work.
    /// The value identifies the syntax generation that owns the marker, so a
    /// superseded worker cannot remove a newer generation's marker.
    pub(in crate::view) file_diff_click_syntax_inflight: FxHashMap<DiffTextRegion, u64>,
    /// Test-only switch for the eager source-backed prepare. Off, a source-backed
    /// side gets a document only when clicked, which is what the click-path tests
    /// are there to cover and what they would otherwise stop exercising.
    #[cfg(test)]
    pub(in crate::view) eager_source_backed_syntax_prepare: bool,
    /// Test-only mutation point after a click worker has parsed but before its
    /// result is returned to the UI thread.
    #[cfg(test)]
    pub(in crate::view) file_diff_click_syntax_after_prepare_hook:
        Option<Arc<dyn Fn() + Send + Sync>>,
    /// Test-only observation point immediately before a click worker updates
    /// its in-flight marker and installs or rejects its prepared document.
    #[cfg(test)]
    pub(in crate::view) file_diff_click_syntax_before_complete_hook:
        Option<ClickSyntaxCompleteHook>,
    /// Where each side's content lives when it is a file rather than text in
    /// memory. A source-backed side keeps its text off the heap so a huge diff
    /// can render from per-line slices; the click path reads it back from here
    /// when it needs a whole-document parse.
    pub(in crate::view) file_diff_old_source_path: Option<Arc<std::path::PathBuf>>,
    pub(in crate::view) file_diff_new_source_path: Option<Arc<std::path::PathBuf>>,
    /// Filesystem identity of each source-backed side as this generation indexed
    /// it. A click re-reads the file, so it needs to know whether the file is
    /// still the one the rows are showing.
    pub(in crate::view) file_diff_old_source_identity: Option<Arc<str>>,
    pub(in crate::view) file_diff_new_source_identity: Option<Arc<str>>,
    /// Real old-side file text used for split and inline syntax projection.
    /// Empty when the side is source-backed; `file_diff_old_source_path` says so.
    pub(in crate::view) file_diff_old_text: SharedString,
    pub(in crate::view) file_diff_old_line_starts: Arc<[usize]>,
    pub(in crate::view) file_diff_old_line_to_row: Arc<[Option<usize>]>,
    pub(in crate::view) file_diff_old_line_to_inline_row: Arc<[Option<usize>]>,
    /// Real new-side file text used for split and inline syntax projection.
    /// Empty when the side is source-backed; `file_diff_new_source_path` says so.
    pub(in crate::view) file_diff_new_text: SharedString,
    pub(in crate::view) file_diff_new_line_starts: Arc<[usize]>,
    pub(in crate::view) file_diff_new_line_to_row: Arc<[Option<usize>]>,
    pub(in crate::view) file_diff_new_line_to_inline_row: Arc<[Option<usize>]>,
    pub(in crate::view) file_diff_inline_cache: Arc<[AnnotatedDiffLine]>,
    pub(in crate::view) file_diff_inline_row_provider:
        Option<Arc<super::super::diff_cache::PagedFileDiffInlineRows>>,
    pub(in crate::view) file_diff_inline_text: SharedString,
    pub(in crate::view) blame_label_cache:
        std::rc::Rc<std::cell::RefCell<crate::view::rows::BlameLabelCache>>,
    pub(in crate::view) file_diff_inline_word_highlights:
        rows::LruCache<usize, Arc<[Range<usize>]>>,
    pub(in crate::view) file_diff_split_word_highlights:
        rows::LruCache<usize, FileDiffSplitWordHighlights>,
    pub(in crate::view) file_diff_cache_seq: u64,
    pub(in crate::view) file_diff_cache_inflight: Option<u64>,
    /// Identity of the row/source generation currently visible to clicks.
    /// Kept stable while a same-target replacement builds, then advanced at the
    /// atomic row swap.
    pub(in crate::view) file_diff_syntax_generation: u64,
    pub(in crate::view) file_diff_style_cache_epochs: FileDiffStyleCacheEpochs,
    pub(in crate::view) syntax_chunk_poll_task: Option<gpui::Task<()>>,
    pub(in crate::view) prepared_syntax_documents:
        FxHashMap<PreparedSyntaxDocumentKey, rows::PreparedDiffSyntaxDocument>,
    #[cfg(test)]
    pub(in crate::view) diff_syntax_budget_override: Option<rows::DiffSyntaxBudget>,

    // Markdown preview and the resolved preview surface.
    pub(in crate::view) diff_markdown: DiffMarkdownPreview,
    pub(in crate::view) markdown_interaction: MarkdownPreviewInteraction,
    /// Frames drawn, which is how often the preview surface re-reads the disk.
    pub(in crate::view) main_pane_surface_frame: u64,
    /// The preview surface this frame, once resolved; see
    /// [`MainPaneView::main_pane_surface`].
    pub(in crate::view) main_pane_surface_memo: RefCell<Option<MainPaneSurfaceMemo>>,

    // Image diff decoding and animation.
    pub(in crate::view) file_image_diff_cache_repo_id: Option<RepoId>,
    pub(in crate::view) file_image_diff_cache_rev: u64,
    pub(in crate::view) file_image_diff_cache_content_signature: Option<u64>,
    pub(in crate::view) file_image_diff_cache_target: Option<DiffTarget>,
    pub(in crate::view) file_image_diff_cache_seq: u64,
    pub(in crate::view) file_image_diff_cache_inflight: Option<u64>,
    pub(in crate::view) file_image_diff_cache_complete: bool,
    pub(in crate::view) file_image_diff_cache_failed: bool,
    pub(in crate::view) file_image_diff_cache_path: Option<std::path::PathBuf>,
    pub(in crate::view) file_image_diff_cache_old: Option<Arc<gpui::RenderImage>>,
    pub(in crate::view) file_image_diff_cache_new: Option<Arc<gpui::RenderImage>>,
    pub(in crate::view) file_image_diff_cache_old_svg_path: Option<std::path::PathBuf>,
    pub(in crate::view) file_image_diff_cache_new_svg_path: Option<std::path::PathBuf>,
    pub(in crate::view) file_image_preview_animation: FileImagePreviewAnimation,
    pub(in crate::view) file_image_preview_animation_task: Option<gpui::Task<()>>,

    // Conflict image preview loading.
    pub(in crate::view) conflict_image_preview_seq: u64,
    pub(in crate::view) conflict_image_preview_inflight: Option<u64>,
    pub(in crate::view) conflict_image_preview_task: Option<gpui::Task<()>>,
    pub(in crate::view) conflict_image_preview_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,

    // Read-only worktree file preview.
    pub(in crate::view) worktree_preview_path: Option<std::path::PathBuf>,
    pub(in crate::view) worktree_preview_source_path: Option<std::path::PathBuf>,
    /// What the preview was decoded with; a change re-reads it.
    pub(in crate::view) worktree_preview_decode_key: Option<super::super::preview::TextDecodeKey>,
    /// How the previewed bytes were read, for the status strip.
    pub(in crate::view) worktree_preview_text_format:
        Option<gitcomet_core::text_format::SideTextFormat>,
    pub(in crate::view) worktree_preview: Loadable<usize>,
    pub(in crate::view) worktree_preview_source_len: usize,
    pub(in crate::view) worktree_preview_text: SharedString,
    pub(in crate::view) worktree_preview_line_starts: Arc<[usize]>,
    pub(in crate::view) worktree_preview_line_flags: Arc<[u8]>,
    pub(in crate::view) worktree_preview_search_trigram_index:
        Option<super::super::diff_search::DiffSearchVisibleTrigramIndex>,
    pub(in crate::view) worktree_preview_content_rev: u64,
    pub(in crate::view) worktree_markdown: WorktreeMarkdownPreview,
    pub(in crate::view) worktree_preview_segments_cache_path: Option<std::path::PathBuf>,
    pub(in crate::view) worktree_preview_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) worktree_preview_style_cache_epoch: u64,
    pub(in crate::view) worktree_preview_cache_write_blocked_until_rev: Option<u64>,
    pub(in crate::view) worktree_preview_segments_cache:
        FxHashMap<usize, VersionedCachedDiffStyledText>,
    pub(in crate::view) diff_preview_is_new_file: bool,
    /// What the read-only preview was read from. See `super::super::file_disk`.
    pub(in crate::view) worktree_preview_disk: DiskIdentity,
    /// Scroll offset to restore after a reload that must not jump to the top.
    pub(in crate::view) worktree_preview_restore_scroll_offset: Option<gpui::Point<Pixels>>,

    // Editable working-tree buffer: disk sync, saves, syntax, search.
    /// The editable working-tree buffer. See `super::super::file_editor`.
    pub(in crate::view) file_editor_input: Entity<components::TextInput>,
    pub(in crate::view::panes::main) _file_editor_input_subscription: gpui::Subscription,
    /// Which repo/path the input currently holds, so a target change is one
    /// comparison rather than a reload every frame.
    pub(in crate::view) file_editor_key: Option<gitcomet_core::filesystem::DocumentIdentity>,
    pub(crate) filesystem_pauses:
        std::collections::BTreeSet<gitcomet_core::filesystem::OperationId>,
    pub(in crate::view) file_editor_disk_versions: FxHashMap<
        gitcomet_core::filesystem::DocumentIdentity,
        gitcomet_core::filesystem::DiskVersion,
    >,
    pub(in crate::view) file_editor_saves: std::collections::BTreeMap<
        gitcomet_core::filesystem::OperationId,
        (gitcomet_core::filesystem::DocumentIdentity, u64),
    >,
    pub(in crate::view) file_editor_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) file_editor_loading: bool,
    /// Generation of the last disk read, so a superseded read is dropped.
    pub(in crate::view) file_editor_reread_seq: u64,
    /// What the buffer was read from (or last wrote). See `super::super::file_disk`.
    pub(in crate::view) file_editor_disk: DiskIdentity,
    pub(in crate::view) file_editor_error: Option<SharedString>,
    /// How the buffer's file was read, and so how it is written back.
    pub(in crate::view) file_editor_text_format: Option<gitcomet_core::text_format::SideTextFormat>,
    pub(in crate::view) file_editor_source_text_format:
        Option<gitcomet_core::text_format::SideTextFormat>,
    /// What the buffer was decoded with; a new choice re-reads it.
    pub(in crate::view) file_editor_decode_key: Option<super::super::preview::TextDecodeKey>,
    /// A read is waiting for the file's attributes.
    pub(in crate::view) file_editor_waiting_for_attributes: bool,
    pub(in crate::view) file_editor_dirty: bool,
    /// The topmost 0-based line an unsaved edit has touched, or `None` while the
    /// buffer matches disk.
    ///
    /// Blame is indexed by committed line number, so an insertion or deletion
    /// shifts the attribution of everything under it — but only under it. This
    /// watermark is what lets the gutter keep showing blame for the untouched
    /// head of the file instead of blanking the whole column on the first
    /// keystroke. Deliberately pessimistic: an edit that changed no line count
    /// still moves it, because tracking that precisely costs more than the
    /// attribution below it is worth.
    pub(in crate::view) file_editor_first_dirty_line: Option<u32>,
    /// Why the last save wrote nothing (the text has no bytes in the file's
    /// encoding). Shows Save/Discard under auto-save and stops auto-save
    /// re-reporting the same failure every pause.
    pub(in crate::view) file_editor_save_error: Option<SharedString>,
    /// Fingerprint of the text last known to be on disk. `None` before the
    /// first read lands, which reads as "everything is unsaved".
    pub(in crate::view) file_editor_saved_fingerprint: Option<u64>,
    /// "File changed on disk", for the surface it names. See `super::super::file_disk`.
    pub(in crate::view) file_disk_notice: Option<FileDiskNotice>,
    /// Generation of the last disk check, so a superseded check is dropped.
    pub(in crate::view) file_disk_check_seq: u64,
    /// Why the check in flight was started, if one is.
    pub(in crate::view) file_disk_check_in_flight: Option<DiskCheckCause>,
    /// The surface on screen and the repo revisions it was last read or
    /// checked at. `None` until a read lands, so bumps that predate the read
    /// never fire a check.
    pub(in crate::view) file_disk_seen: Option<FileDiskSeen>,
    /// Unsaved buffers the user navigated away from, keyed by path. This is what
    /// makes leaving a file and coming back non-destructive with auto-save off.
    /// Keyed by repo *and* path: two repo tabs can hold the same relative path,
    /// and one must not restore over the other's buffer.
    pub(in crate::view) file_editor_stash: FxHashMap<
        gitcomet_core::filesystem::DocumentIdentity,
        super::super::file_editor::StashedFileEdit,
    >,
    /// Bumped whenever the set of files with unsaved edits changes.
    ///
    /// That set lives here rather than in the store, so nothing outside this
    /// pane can notice it moving on its own — the sidebar keys its file-row
    /// cache off this counter and repaints on the notify that bumps it.
    pub(in crate::view) unsaved_file_edits_rev: u64,
    /// The pending quiet-period timer for auto-save. Dropping it cancels it, so
    /// every keystroke simply replaces it.
    pub(in crate::view) file_editor_autosave: Option<gpui::Task<()>>,
    /// The editor's tree-sitter document. Owned here for the same reason the
    /// resolved output's is: it must survive every keystroke, which is exactly
    /// what a content-hash-keyed cache cannot do.
    pub(in crate::view) file_editor_live_syntax: Option<rows::LiveSyntaxDocument>,
    /// `(model_id, revision)` the live tree was last built or synced for.
    pub(in crate::view) file_editor_live_syntax_source: Option<(u64, u64)>,
    pub(in crate::view) file_editor_live_syntax_building: Option<(u64, u64)>,
    /// In-flight *first* parse. Kept apart from the reparse slot, which is
    /// cleared whenever there is no document to reparse — the state a first
    /// parse runs in.
    pub(in crate::view) file_editor_live_syntax_build: Option<gpui::Task<()>>,
    pub(in crate::view) file_editor_live_syntax_reparse: Option<gpui::Task<()>>,
    /// The delimiters currently washed as the caret's bracket pair.
    pub(in crate::view) file_editor_syntax_pair: Option<rows::SyntaxPair>,
    /// Everywhere the editor's buffer names the token under the caret.
    pub(in crate::view) file_editor_occurrences: Vec<Range<usize>>,
    /// The document version `file_editor_occurrences` was computed against, so
    /// a caret moving *within* the same name does not rescan the document.
    pub(in crate::view) file_editor_occurrences_version: Option<u64>,
    /// Byte ranges of every search match in the editor buffer, one per
    /// occurrence and parallel to `diff_search_matches`, which carries the line
    /// each of them sits on. Keeping the two parallel is what lets the shared
    /// `n/N` label and match cursor work over the editor unchanged.
    pub(in crate::view) file_editor_search_matches: Vec<Range<usize>>,
    /// The buffer the scan reads. It runs without a `cx` and so cannot reach the
    /// input; a snapshot is an `Arc` bump and immutable under later edits, which
    /// makes caching one here the cheap way to hand it the live text.
    pub(in crate::view) file_editor_search_source: Option<TextModelSnapshot>,
    /// Bumped whenever the *painted* match set moves — a rescan, a cursor step,
    /// the search closing. `render_file_editor` rebinds the highlight provider
    /// when it differs from `file_editor_search_applied_rev`.
    pub(in crate::view) file_editor_search_rev: u64,
    pub(in crate::view) file_editor_search_applied_rev: u64,
    /// Bumped only when the match *cursor* moves. Separate from the rev above
    /// because it drives the selection, and a rescan alone must not re-select:
    /// the buffer is rescanned on every keystroke while the search box is open,
    /// which would drag the caret off what the user is typing.
    pub(in crate::view) file_editor_search_reveal_rev: u64,
    pub(in crate::view) file_editor_search_reveal_applied_rev: u64,
    /// Set once a search reveal has moved the caret, cleared once the editor
    /// has been scrolled sideways to it.
    ///
    /// The caret's x can only be read from the layout of a frame that already
    /// painted it, so the horizontal half of the reveal lands one frame after
    /// the selection does.
    pub(in crate::view) file_editor_search_reveal_x_pending: bool,
    /// Bumped on every theme change: the syntax palette is baked into the
    /// snapshot the provider closes over, so a new theme needs a new binding key.
    pub(in crate::view) file_editor_provider_theme_epoch: u64,
    /// Mirrors the settings window's toggle; the pane never writes it back.
    pub(in crate::view) auto_save_file_edits: bool,

    // Merge conflict resolver: input, layout, caches, resolved output.
    pub(in crate::view) conflict_resolver_input: Entity<components::TextInput>,
    pub(in crate::view::panes::main) _conflict_resolver_input_subscription: gpui::Subscription,
    pub(in crate::view) conflict_resolver: ConflictResolverUiState,
    pub(in crate::view) conflict_open_summary_toasted_files:
        FxHashSet<(RepoId, std::path::PathBuf)>,
    pub(in crate::view) conflict_resolver_vsplit_ratio: f32,
    pub(in crate::view) conflict_resolver_vsplit_resize: Option<ConflictVSplitResizeState>,
    pub(in crate::view) conflict_three_way_col_ratios: [f32; 2],
    pub(in crate::view) conflict_three_way_col_widths: [Pixels; 3],
    pub(in crate::view) conflict_hsplit_resize: Option<ConflictHSplitResizeState>,
    pub(in crate::view) conflict_diff_split_ratio: f32,
    pub(in crate::view) conflict_diff_split_resize: Option<ConflictDiffSplitResizeState>,
    pub(in crate::view) conflict_diff_split_col_widths: [Pixels; 2],
    pub(in crate::view) conflict_canvas_rows_enabled: bool,
    pub(in crate::view) conflict_diff_segments_cache_split:
        crate::view::conflict_resolver::ConflictSplitStyledTextCache,
    pub(in crate::view) conflict_diff_query_segments_cache_split:
        crate::view::conflict_resolver::ConflictSplitStyledTextCache,
    pub(in crate::view) conflict_diff_query_cache_query: SharedString,
    pub(in crate::view) conflict_diff_query_cache_options:
        super::super::diff_search::DiffSearchOptions,
    pub(in crate::view) conflict_three_way_segments_cache:
        FxHashMap<(usize, ThreeWayColumn), CachedDiffStyledText>,
    /// Quick-search overlay layered on top of `conflict_three_way_segments_cache`.
    ///
    /// Separate so a query change throws away only the wash and leaves the
    /// syntax/word-highlight work standing, the way the two-way columns split
    /// `conflict_diff_segments_cache_split` from its query twin. Holds only
    /// non-current matches — the current one moves with the search cursor and
    /// is built per frame.
    pub(in crate::view) conflict_three_way_query_segments_cache:
        FxHashMap<(usize, ThreeWayColumn), CachedDiffStyledText>,
    /// Prepared full-document syntax trees for each merge-input side (base, ours, theirs).
    /// When present, three-way rendering uses document-based syntax instead of per-line heuristics.
    pub(in crate::view) conflict_three_way_prepared_syntax_documents:
        ThreeWaySides<Option<rows::PreparedDiffSyntaxDocument>>,
    /// Per-side flag tracking whether a background syntax parse is in-flight.
    pub(in crate::view) conflict_three_way_syntax_inflight: ThreeWaySides<bool>,
    pub(in crate::view) conflict_resolved_preview_path: Option<std::path::PathBuf>,
    /// Latest editable-output revision observed by the input subscription. This
    /// is intentionally independent of the content hash so a keypress can
    /// supersede debounce work without materializing and scanning the document.
    pub(in crate::view) conflict_resolved_preview_source_revision:
        Option<ResolvedOutputSourceRevision>,
    /// Editable-output snapshot at the last file load/save refresh. Snapshot
    /// equality is O(1), and undo restores the matching snapshot, so this can
    /// drive the user-facing Modified state without hashing the whole output.
    pub(in crate::view) conflict_resolved_output_saved_snapshot: Option<TextModelSnapshot>,
    pub(in crate::view) conflict_resolved_output_modified: bool,
    pub(in crate::view) conflict_resolved_output_projection:
        Option<conflict_resolver::ResolvedOutputProjection>,
    /// Byte ownership for displayed conflict blocks in the live output.
    pub(in crate::view) conflict_resolved_output_block_map:
        conflict_resolver::ResolvedOutputBlockMap,
    pub(in crate::view) conflict_resolved_preview_text: TextModelSnapshot,
    pub(in crate::view) conflict_resolved_preview_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) conflict_resolved_preview_line_count: usize,
    pub(in crate::view) conflict_resolved_preview_line_starts: Arc<[usize]>,
    /// The editable resolved output's tree-sitter document. Owned here rather
    /// than in the shared thread-local cache because there is exactly one of
    /// them at a time and it must survive every keystroke — which is precisely
    /// what a content-hash-keyed cache cannot do.
    pub(in crate::view) conflict_resolved_output_live_syntax: Option<rows::LiveSyntaxDocument>,
    /// In-flight reparse for an edit that outran the foreground budget.
    pub(in crate::view) conflict_resolved_output_live_syntax_reparse: Option<gpui::Task<()>>,
    /// What the live tree was last built for: the buffer revision and the
    /// placeholder mask. Both must be unchanged for a refresh to be a no-op.
    ///
    /// Deliberately not pointer identity on the text. `SharedString` can be
    /// `Borrowed`, and `Arc<str>::from(&str)` then allocates afresh on every
    /// call, so a pointer check would never match — turning every refresh into a
    /// reparse and, because installing a provider notifies the input that
    /// triggered the refresh, into an unbreakable loop.
    pub(in crate::view) conflict_resolved_output_live_syntax_source:
        Option<(ResolvedOutputSourceRevision, Arc<[Range<usize>]>)>,
    /// Bumped on every theme change. The syntax palette is baked into
    /// `LiveSyntaxSnapshot`, so a new theme needs a new provider -- and the
    /// binding key is the only thing that makes `TextInput` adopt one.
    /// Hashing theme *colours* into the key instead is not enough: two dark
    /// themes can agree on the few colours sampled and still differ on the
    /// syntax palette, leaving stale colours installed.
    pub(in crate::view) conflict_resolved_output_provider_theme_epoch: u64,
    /// Which conflict the installed output highlights wash yellow. Conflict
    /// navigation moves no text and touches no tree, so none of the refresh
    /// paths fire on it; this is what tells the render pass the active row moved
    /// and the provider has to be rebuilt.
    pub(in crate::view) conflict_resolved_output_highlighted_conflict: Option<usize>,
    /// Unresolved output rows and their conflict, cached for the buffer
    /// revision they were computed from.
    ///
    /// Conflict navigation moves the yellow wash but changes no text, so
    /// recomputing these would rescan the whole document on every jump — which
    /// is exactly what made F3 cost tens of milliseconds on a large file.
    pub(in crate::view) conflict_resolved_output_unresolved_rows: Option<CachedUnresolvedRows>,
    /// How many times the resolved output's syntax refresh has gone past its
    /// early-out and rescanned the document. Only an edit should do that;
    /// navigation must not.
    #[cfg(test)]
    pub(in crate::view) conflict_resolved_output_full_scans: usize,
    /// Revision an off-thread first parse is currently running for, so repeated
    /// refreshes over the same text do not pile up duplicate builds.
    pub(in crate::view) conflict_resolved_output_live_syntax_building:
        Option<ResolvedOutputSourceRevision>,
    /// In-flight *first* parse. Kept apart from the reparse slot: that one is
    /// cleared whenever there is no document to reparse, which is exactly the
    /// state a first parse runs in -- sharing the slot would cancel it.
    pub(in crate::view) conflict_resolved_output_live_syntax_build: Option<gpui::Task<()>>,
    pub(in crate::view) conflict_resolved_output_measure_row: usize,
    pub(in crate::view) conflict_resolved_outline_stash: Option<StashedResolvedOutlineState>,
    #[cfg(test)]
    pub(in crate::view) conflict_resolved_outline_background_delay_override:
        Option<std::time::Duration>,

    // History view, scroll handles, and gutter geometry.
    pub(in crate::view) history_view: Entity<super::super::HistoryView>,
    pub(in crate::view) diff_scroll: UniformListScrollHandle,
    pub(in crate::view) diff_split_right_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_resolver_diff_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_preview_ours_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_preview_theirs_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_preview_last_synced_x: [Pixels; 4],
    pub(in crate::view) conflict_preview_last_synced_y: [Pixels; 4],
    /// Source/output handle index that received the latest vertical wheel
    /// gesture: base/left=0, ours=1, theirs/right=2, output=3.
    pub(in crate::view) conflict_preview_vertical_wheel_master: Option<usize>,
    /// The next output/gutter sync belongs to that wheel gesture, so output
    /// must drive the pair instead of a stale gutter baseline.
    pub(in crate::view) conflict_output_gutter_wheel_sync_pending: bool,
    pub(in crate::view) conflict_resolved_preview_scroll: UniformListScrollHandle,
    /// Scroll handle for the editable resolved-output `TextInput`. The input lays
    /// out at full content height inside an `overflow_y_scroll` container that
    /// tracks this handle, and the input reads the same handle to window its line
    /// shaping. It is also the output member (index 3) of the conflict-preview
    /// scroll-sync group, so it stands in for `conflict_resolved_preview_scroll`
    /// (which now only backs the read-only projection paths).
    pub(in crate::view) conflict_resolved_output_editor_scroll: ScrollHandle,
    pub(in crate::view) conflict_resolved_preview_gutter_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_resolved_preview_gutter_last_synced_y: [Pixels; 2],
    pub(in crate::view) worktree_preview_scroll: UniformListScrollHandle,
    /// Scroll handle for the editor's `TextInput`: the input lays out at full
    /// content size inside an `overflow_scroll` container tracking this handle,
    /// and reads the same handle to window its line shaping.
    pub(in crate::view) file_editor_scroll: ScrollHandle,
    /// Gutter list, mirrored to `file_editor_scroll`'s vertical offset.
    pub(in crate::view) file_editor_gutter_scroll: UniformListScrollHandle,
    /// UI-scaled row height the gutter list paints at, computed by the render
    /// pass so the virtualized row processor can read it without a scale lookup.
    pub(in crate::view) file_editor_gutter_row_height: Pixels,
    /// The same, for the merge tool's resolved-output gutter. Navigation centres
    /// the editable output on a row from `&self`, where there is no `cx` to look
    /// the scale up through, so the render pass leaves it here.
    pub(in crate::view) conflict_resolved_gutter_row_height: Pixels,
    /// Blame for the edited file, resolved by the render pass so the virtualized
    /// gutter rows can read it without rebuilding the context per row.
    pub(in crate::view) file_editor_blame: Option<rows::BlameRenderCtx>,
    pub(in crate::view) file_editor_blame_width: Pixels,
    /// First gutter row owned by each logical line, so a wrapped line's number
    /// sits on the first of the rows it spans. Empty when the buffer is not
    /// wrapping. Retained to keep its allocation across frames.
    pub(in crate::view) file_editor_wrap_row_starts: Vec<usize>,

    // Path display cache and per-repo interactive rebase state.
    pub(in crate::view::panes::main) path_display_cache:
        std::cell::RefCell<path_display::PathDisplayCache>,

    /// Per-repo interactive rebase editing state, keyed by repo id so that
    /// setups open in several repo tabs at once stay independent. Entries are
    /// populated when a repo's setup becomes Ready and dropped when its setup
    /// goes away (see `apply_state`).
    pub(in crate::view) interactive_rebase_states: FxHashMap<RepoId, IRebaseViewState>,
}

/// View-local editing state for one repo's interactive rebase setup.
#[derive(Default)]
pub(in crate::view) struct IRebaseViewState {
    pub(in crate::view) mode: ICommitEditorMode,
    pub(in crate::view) entries: Vec<gitcomet_core::services::InteractiveRebaseEntry>,
    pub(in crate::view) original_entries: Vec<gitcomet_core::services::InteractiveRebaseEntry>,
    pub(in crate::view) source_colors: FxHashMap<String, u8>,
    /// Active auto-squash strategy, or None when auto-squash is off.
    pub(in crate::view) autosquash_mode: Option<AutosquashMode>,
    /// Commits folded away by auto-squash, keyed by the surviving commit id.
    /// Each survivor's `entries` row displays these ids; they are re-expanded
    /// into `fixup` todo entries when the rebase starts.
    pub(in crate::view) folded:
        FxHashMap<String, Vec<gitcomet_core::services::InteractiveRebaseEntry>>,
    pub(in crate::view) drag_state: Option<IRebaseDragState>,
    /// Variable-height virtualized list state, lazily created on first render
    /// (`ListState` has no `Default`). Kept in sync with `entries`/`folded` via
    /// `list_sig` (remeasure on same-count content change, reset on count change).
    pub(in crate::view) scroll: Option<gpui::ListState>,
    /// (content-hash, item-count) the `scroll` ListState was last synced to.
    pub(in crate::view) list_sig: (u64, usize),
    /// (ix_a, ix_b, version) — the two data-indices swapped by ▲/▼; drives fade-in animation.
    pub(in crate::view) reorder_anim: Option<(usize, usize, u32)>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) enum ICommitEditorMode {
    #[default]
    Rebase,
    CherryPick,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::view) struct IRebaseDragState {
    pub(in crate::view) from_ix: usize,
    pub(in crate::view) to_ix: usize,
    /// Drop-target position in display order (0..=entry_count).
    pub(in crate::view) display_pos: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view::panes::main) enum DiffTextAutoscrollTarget {
    DiffLeftOrInline,
    DiffSplitRight,
    WorktreePreview,
    ConflictResolvedPreview,
}
