use super::*;

fn preview_image_element_id(container_id: &'static str) -> gpui::ElementId {
    (gpui::ElementId::from(container_id), "image").into()
}

/// Builds an image with stable per-element state, which GPUI requires to
/// advance animated `RenderImage` frames across renders.
pub(crate) fn preview_image_element(
    source: impl Into<gpui::ImageSource>,
    container_id: &'static str,
) -> gpui::Stateful<gpui::Img> {
    gpui::img(source).id(preview_image_element_id(container_id))
}

#[derive(Clone, Debug)]
pub(crate) enum ConflictPreviewImage {
    Encoded(Arc<gpui::Image>),
    Rendered(Arc<gpui::RenderImage>),
}

impl ConflictPreviewImage {
    pub(crate) fn element(&self, container_id: &'static str) -> gpui::Stateful<gpui::Img> {
        match self {
            Self::Encoded(image) => preview_image_element(Arc::clone(image), container_id),
            Self::Rendered(image) => preview_image_element(Arc::clone(image), container_id),
        }
    }
}

pub(crate) type LoadableImagePreview = Loadable<Option<ConflictPreviewImage>>;

#[derive(Clone, Debug)]
pub(crate) struct ConflictResolverMarkdownPreviewState {
    pub(crate) source_hash: Option<u64>,
    pub(crate) documents: ThreeWaySides<LoadableMarkdownDoc>,
}

impl Default for ConflictResolverMarkdownPreviewState {
    fn default() -> Self {
        Self {
            source_hash: None,
            documents: ThreeWaySides {
                base: Loadable::NotLoaded,
                ours: Loadable::NotLoaded,
                theirs: Loadable::NotLoaded,
            },
        }
    }
}

impl ConflictResolverMarkdownPreviewState {
    pub(crate) fn document(&self, side: ThreeWayColumn) -> &LoadableMarkdownDoc {
        &self.documents[side]
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ConflictResolverImagePreviewState {
    pub(crate) source_hash: Option<u64>,
    pub(crate) path: Option<std::path::PathBuf>,
    pub(crate) images: ThreeWaySides<LoadableImagePreview>,
}

impl Default for ConflictResolverImagePreviewState {
    fn default() -> Self {
        Self {
            source_hash: None,
            path: None,
            images: ThreeWaySides {
                base: Loadable::NotLoaded,
                ours: Loadable::NotLoaded,
                theirs: Loadable::NotLoaded,
            },
        }
    }
}

impl ConflictResolverImagePreviewState {
    pub(crate) fn image(&self, side: ThreeWayColumn) -> &LoadableImagePreview {
        &self.images[side]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedOutputConflictMarker {
    pub(crate) conflict_ix: usize,
    pub(crate) range_start: usize,
    pub(crate) range_end: usize,
    pub(crate) is_start: bool,
    pub(crate) is_end: bool,
    pub(crate) unresolved: bool,
}

/// Resolved-output outline metadata: per-line provenance, conflict markers, and source index.
/// Shared between visible state (`ConflictResolverUiState`) and incremental-recompute stash.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedOutlineData {
    /// Per-line provenance metadata.
    pub(crate) meta: Vec<conflict_resolver::ResolvedLineMeta>,
    /// Per-line conflict marker metadata for gutter markers.
    pub(crate) markers: Vec<Option<ResolvedOutputConflictMarker>>,
    /// Source line keys currently represented in resolved output (for dedupe/plus-icon).
    pub(crate) sources_index: FxHashSet<conflict_resolver::SourceLineKey>,
}

/// Mode-specific state for streamed (giant-file) conflict resolution.
///
/// Uses lazy paged access and span-based projections instead of
/// eagerly materializing all rows.
#[derive(Clone, Debug, Default)]
pub(crate) struct StreamedConflictState {
    pub(crate) three_way_visible_projection: conflict_resolver::ThreeWayVisibleProjection,
    pub(crate) split_row_index: conflict_resolver::ConflictSplitRowIndex,
    pub(crate) two_way_split_projection: conflict_resolver::TwoWaySplitProjection,
}

#[derive(Clone, Debug)]
pub(crate) enum ConflictModeState {
    Streamed(StreamedConflictState),
}

impl Default for ConflictModeState {
    fn default() -> Self {
        Self::Streamed(StreamedConflictState::default())
    }
}

/// section 30 split: a drag selection of aligned rows within one conflict block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConflictRowSelection {
    /// Visible conflict block the selection is anchored in.
    pub(crate) conflict_ix: usize,
    /// Aligned row where the drag started.
    pub(crate) anchor_row: usize,
    /// Aligned row under the cursor (clamped to the block).
    pub(crate) head_row: usize,
    /// True while the drag is in progress.
    pub(crate) selecting: bool,
}

impl ConflictRowSelection {
    /// Inclusive aligned-row range covered, normalized so start <= end.
    pub(crate) fn row_range(&self) -> std::ops::RangeInclusive<usize> {
        let lo = self.anchor_row.min(self.head_row);
        let hi = self.anchor_row.max(self.head_row);
        lo..=hi
    }
}

/// KDiff3 manual diff help: lines marked in one source column, pending the
/// Ctrl+Y that pins them against the other columns' marks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AlignmentLineSelection {
    /// Line where the mark started.
    pub(crate) anchor: usize,
    /// Line last marked.
    pub(crate) head: usize,
}

impl AlignmentLineSelection {
    /// Half-open line range covered, normalized so start <= end.
    pub(crate) fn line_range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head) + 1
    }

    pub(crate) fn contains(self, line: usize) -> bool {
        self.line_range().contains(&line)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ConflictResolverUiState {
    pub(crate) repo_id: Option<RepoId>,
    pub(crate) path: Option<std::path::PathBuf>,
    pub(crate) shared_path: Option<gitcomet_state::msg::RepoPath>,
    pub(crate) loaded_file: Option<gitcomet_state::model::ConflictFile>,
    pub(crate) conflict_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(crate) source_hash: Option<u64>,
    /// The editable output contains preserved worktree text whose conflict
    /// spans could not be mapped safely onto the stage projection.
    pub(crate) output_is_protected: bool,
    /// The user asked for the stage projection anyway, via *Reset conflict
    /// markers*, so protection stays off for this conflict however the
    /// worktree payload reads.
    ///
    /// Without this the reset lasts until the next store round-trip: the resync
    /// recomputes protection from the same unchanged worktree payload and turns
    /// it straight back on, which is what made the button look like it did
    /// nothing. A re-bootstrap drops the waiver, so it lasts exactly as long as
    /// the conflict and the file content it was granted for.
    pub(crate) output_protection_waived: bool,
    /// Marker-backed geometry used for reset and source-region rendering.
    pub(crate) current: Option<std::sync::Arc<str>>,
    pub(crate) marker_segments: Vec<conflict_resolver::ConflictSegment>,
    /// section 30 collapsed context mode: fold unchanged runs in the source columns.
    pub(crate) collapse_context: bool,
    /// Per-fold reveal state for collapsed context mode, keyed by fold id.
    pub(crate) context_fold_reveals: FxHashMap<usize, conflict_resolver::ConflictFoldReveal>,
    /// section 30 collapsed context mode for the resolved output pane: fold
    /// projection in output line space. `None` ⇒ pass-through (one row per
    /// line). Rebuilt lazily after its inputs change.
    pub(crate) resolved_output_visible: Option<conflict_resolver::ThreeWayVisibleProjection>,
    pub(crate) resolved_output_visible_dirty: bool,
    /// Per-fold reveal state for resolved-output folds (output-line fold ids).
    pub(crate) output_context_fold_reveals: FxHashMap<usize, conflict_resolver::ConflictFoldReveal>,
    /// Mapping from visible block index to `ConflictSession` region index.
    pub(crate) conflict_region_indices: Vec<usize>,
    /// Mapping from visible marker block index to its semantic merge-plan
    /// block. Empty for marker-only/fallback sessions.
    pub(crate) display_plan_block_indices: Vec<usize>,
    /// Whether each raw session region includes a diff3 base marker. This is
    /// kept separate from display blocks, whose base may be populated from
    /// the shared ancestor for picking.
    pub(crate) conflict_region_marker_has_base: Vec<bool>,
    /// Actionable conflict block currently selected in the displayed marker
    /// projection. Semantic targets without a displayed block leave this unset.
    pub(crate) active_conflict: Option<usize>,
    /// Ordered semantic resolver navigation targets.
    pub(crate) nav_targets: Vec<conflict_resolver::ConflictNavTarget>,
    /// Aligned source rows retained for every original session region before
    /// manual/automatic resolutions are materialized into plain display text.
    pub(crate) original_region_aligned_ranges: Vec<Option<Range<usize>>>,
    pub(crate) hovered_conflict: Option<(usize, ThreeWayColumn)>,
    /// section 30 split: in-progress or completed drag selection of aligned rows
    /// inside one conflict block, used to split that block at the selection
    /// boundary. Cleared whenever the conflict source rebuilds.
    pub(crate) row_selection: Option<ConflictRowSelection>,
    /// KDiff3 manual diff help: lines marked per source column, independent of
    /// the block-scoped `row_selection` because a manual alignment exists
    /// precisely to pin lines the automatic alignment put in different blocks.
    pub(crate) alignment_selection: ThreeWaySides<Option<AlignmentLineSelection>>,
    /// Streamed conflict state for the single conflict rendering/runtime path.
    pub(crate) mode_state: ConflictModeState,
    pub(crate) view_mode: ConflictResolverViewMode,
    /// Backing text for each three-way source side.
    pub(crate) three_way_text: ThreeWaySides<SharedString>,
    /// Per-side line start offsets into `three_way_text`, materialized lazily.
    pub(crate) three_way_line_starts: ThreeWaySides<DeferredLineStarts>,
    pub(crate) three_way_len: usize,
    /// section 30 aligned row space: maps visual rows to per-side lines. Identity
    /// (row == line) when alignment is unavailable.
    pub(crate) three_way_aligned: conflict_resolver::ThreeWayAlignedMap,
    /// kdiff3-style minimap column bands, in visible-row space. Empty when no
    /// alignment is available, which hides the column.
    pub(crate) minimap_bands: Arc<[gitcomet_core::merge::MinimapRowKind]>,
    /// Exact merge-plan row ranges for the currently visible marker blocks.
    ///
    /// `None` is the legacy/current-only fallback where ranges must be
    /// estimated from marker text.
    pub(crate) merge_plan_aligned_conflict_ranges: Option<Vec<Range<usize>>>,
    /// Whether the three-way visible projection/ranges have been built at
    /// least once for the current conflict source.
    pub(crate) three_way_visible_state_ready: bool,
    /// Per-side conflict ranges for O(log n) binary-search lookups and
    /// conflict-to-visible mapping. The ours ranges remain the anchor space for
    /// legacy three-way visible projections.
    pub(crate) three_way_conflict_ranges: ThreeWaySides<Vec<Range<usize>>>,
    /// Visible-row indices used to measure horizontal width for each three-way input column.
    pub(crate) three_way_horizontal_measure_rows: [usize; 3],
    pub(crate) conflict_has_base: Vec<bool>,
    /// Current choice for each conflict block, cached to avoid rebuilding it
    /// from `marker_segments` on every render.
    pub(crate) conflict_choices: Vec<conflict_resolver::ConflictChoice>,
    /// Ignore-whitespace visual row kinds by two-way split source row.
    pub(crate) two_way_split_visual_kind_cache:
        FxHashMap<usize, gitcomet_core::file_diff::FileDiffRowKind>,
    /// Visible-row indices used to measure horizontal width for the two-way split inputs.
    pub(crate) two_way_horizontal_measure_rows: [usize; 2],
    pub(crate) three_way_word_highlights: ThreeWaySides<conflict_resolver::WordHighlights>,
    /// Aligned two-way (ours↔theirs) word highlights keyed by aligned row,
    /// precomputed once per rebuild and shared by both diff columns.
    pub(crate) two_way_aligned_word_highlights:
        FxHashMap<usize, conflict_resolver::TwoWayWordHighlightPair>,
    /// Bounded on-demand word highlights for giant block-local two-way rows.
    pub(crate) two_way_split_word_highlight_cache:
        conflict_resolver::ConflictSplitWordHighlightCache,
    pub(crate) nav_anchor: Option<conflict_resolver::ConflictNavAnchor>,
    pub(crate) hide_resolved: bool,
    /// True when any conflict side contains non-UTF8 binary data.
    pub(crate) is_binary_conflict: bool,
    /// Byte sizes of the three conflict sides (for binary UI display).
    pub(crate) binary_side_sizes: [Option<usize>; 3],
    /// The resolver strategy for the current conflict (set during sync).
    pub(crate) strategy: Option<gitcomet_core::conflict_session::ConflictResolverStrategy>,
    /// The conflict kind for the current file (set during sync).
    pub(crate) conflict_kind: Option<gitcomet_core::domain::FileConflictKind>,
    /// Last autosolve trace summary shown in resolver UI.
    pub(crate) last_autosolve_summary: Option<SharedString>,
    /// KDiff3-style report captured when this resolver file opened.
    ///
    /// This stays fixed while the user makes manual picks, so the toast
    /// describes the open-time state rather than a later live state.
    pub(crate) open_summary_counts: Option<conflict_resolver::ConflictSummaryCounts>,
    /// True once the one-shot open-summary toast (total / auto-solved /
    /// unsolved, kdiff3-style) has been pushed for this resolver open.
    pub(crate) open_summary_announced: bool,
    /// Tracks the last-seen `conflict_rev` from state so we can detect
    /// state-side session changes (e.g. hide-resolved, bulk picks, autosolve)
    /// that don't change the underlying file content.
    pub(crate) conflict_rev: u64,
    /// Sequence token for debounced resolved-output outline recompute tasks.
    pub(crate) resolver_pending_recompute_seq: u64,
    /// Resolved-output outline metadata (provenance, conflict markers, source index).
    pub(crate) resolved_outline: ResolvedOutlineData,
    /// Cached per-line gutter render state for resolved-output preview rows.
    pub(crate) resolved_outline_gutter_rows: Vec<conflict_resolver::ResolvedOutputGutterRow>,
    /// Cached rendered markdown previews for the merge-input sides.
    pub(crate) markdown_preview: ConflictResolverMarkdownPreviewState,
    /// Cached image previews for the merge-input sides.
    pub(crate) image_preview: ConflictResolverImagePreviewState,
    /// Preview mode for the merge-input pane (Text vs rendered Preview).
    pub(crate) resolver_preview_mode: ConflictResolverPreviewMode,
}

impl Default for ConflictResolverUiState {
    fn default() -> Self {
        Self {
            repo_id: None,
            path: None,
            shared_path: None,
            loaded_file: None,
            collapse_context: false,
            context_fold_reveals: FxHashMap::default(),
            conflict_syntax_language: None,
            source_hash: None,
            output_is_protected: false,
            output_protection_waived: false,
            current: None,
            marker_segments: Vec::new(),
            conflict_region_indices: Vec::new(),
            display_plan_block_indices: Vec::new(),
            conflict_region_marker_has_base: Vec::new(),
            active_conflict: None,
            nav_targets: Vec::new(),
            original_region_aligned_ranges: Vec::new(),
            hovered_conflict: None,
            row_selection: None,
            alignment_selection: ThreeWaySides::default(),
            mode_state: ConflictModeState::default(),
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            three_way_text: ThreeWaySides::default(),
            three_way_line_starts: ThreeWaySides::default(),
            three_way_len: 0,
            three_way_aligned: conflict_resolver::ThreeWayAlignedMap::default(),
            minimap_bands: Arc::from([]),
            merge_plan_aligned_conflict_ranges: None,
            three_way_visible_state_ready: false,
            three_way_conflict_ranges: ThreeWaySides::default(),
            three_way_horizontal_measure_rows: [0; 3],
            conflict_has_base: Vec::new(),
            conflict_choices: Vec::new(),
            two_way_split_visual_kind_cache: FxHashMap::default(),
            two_way_horizontal_measure_rows: [0; 2],
            three_way_word_highlights: ThreeWaySides::default(),
            two_way_aligned_word_highlights: FxHashMap::default(),
            two_way_split_word_highlight_cache: Default::default(),
            nav_anchor: None,
            hide_resolved: false,
            is_binary_conflict: false,
            binary_side_sizes: [None; 3],
            strategy: None,
            conflict_kind: None,
            last_autosolve_summary: None,
            open_summary_counts: None,
            open_summary_announced: false,
            conflict_rev: 0,
            resolver_pending_recompute_seq: 0,
            resolved_outline: ResolvedOutlineData::default(),
            resolved_outline_gutter_rows: Vec::new(),
            resolved_output_visible: None,
            resolved_output_visible_dirty: true,
            output_context_fold_reveals: FxHashMap::default(),
            markdown_preview: ConflictResolverMarkdownPreviewState::default(),
            image_preview: ConflictResolverImagePreviewState::default(),
            resolver_preview_mode: ConflictResolverPreviewMode::default(),
        }
    }
}

pub(crate) fn indexed_line_text<'a>(
    text: &'a str,
    line_starts: &[usize],
    line_ix: usize,
) -> Option<&'a str> {
    if text.is_empty() {
        return None;
    }
    let text_len = text.len();
    let start = line_starts.get(line_ix).copied().unwrap_or(text_len);
    if start >= text_len {
        return None;
    }
    let mut end = line_starts
        .get(line_ix.saturating_add(1))
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
        end = end.saturating_sub(1);
    }
    Some(text.get(start..end).unwrap_or(""))
}

pub(crate) fn append_conflict_row_without_whitespace(
    row: &gitcomet_core::file_diff::FileDiffRow,
    old_out: &mut String,
    new_out: &mut String,
) {
    use gitcomet_core::file_diff::FileDiffRowKind as RK;

    match row.kind {
        RK::Context => {}
        RK::Remove => {
            if let Some(text) = row.old.as_ref() {
                old_out.extend(text.as_ref().chars().filter(|ch| !ch.is_whitespace()));
            }
        }
        RK::Add => {
            if let Some(text) = row.new.as_ref() {
                new_out.extend(text.as_ref().chars().filter(|ch| !ch.is_whitespace()));
            }
        }
        RK::Modify => {
            if let Some(text) = row.old.as_ref() {
                old_out.extend(text.as_ref().chars().filter(|ch| !ch.is_whitespace()));
            }
            if let Some(text) = row.new.as_ref() {
                new_out.extend(text.as_ref().chars().filter(|ch| !ch.is_whitespace()));
            }
        }
    }
}

impl ConflictResolverUiState {
    pub(crate) fn matches_target(&self, repo_id: RepoId, path: &std::path::Path) -> bool {
        self.repo_id == Some(repo_id) && self.path.as_deref() == Some(path)
    }

    pub(crate) fn dispatch_path(&self) -> Option<gitcomet_state::msg::RepoPath> {
        self.shared_path.clone()
    }

    pub(crate) fn selected_nav_target_index(&self) -> Option<usize> {
        let anchor = self.nav_anchor?;
        self.nav_targets
            .iter()
            .position(|target| target.id == anchor.id)
    }

    pub(crate) fn nav_target_index_for_aligned_row(&self, row: usize) -> Option<usize> {
        self.nav_targets.iter().position(|target| {
            target
                .aligned_rows
                .as_ref()
                .is_some_and(|range| range.contains(&row))
        })
    }

    pub(crate) fn selected_nav_target_contains_aligned_row(&self, row: usize) -> bool {
        self.selected_nav_target_index()
            .and_then(|index| self.nav_targets.get(index))
            .and_then(|target| target.aligned_rows.as_ref())
            .is_some_and(|range| range.contains(&row))
    }

    /// Whether the conflict a row belongs to is the selected one.
    ///
    /// `conflict_ix` is `None` for a row in no conflict at all, and
    /// `active_conflict` is `None` whenever nothing is selected — for instance
    /// right after a pick moves the anchor onto a block that renders no marker.
    /// Comparing the two options directly made those two `None`s match, which
    /// painted the active-conflict marker on every row *outside* a conflict.
    pub(crate) fn conflict_is_active(&self, conflict_ix: Option<usize>) -> bool {
        conflict_ix.is_some() && conflict_ix == self.active_conflict
    }

    pub(crate) fn nav_target_matches_display(
        &self,
        target: &conflict_resolver::ConflictNavTarget,
        display_conflict_index: usize,
    ) -> bool {
        target.display_conflict_index == Some(display_conflict_index)
            || target.region_index.is_some_and(|region_index| {
                self.conflict_region_indices
                    .get(display_conflict_index)
                    .copied()
                    == Some(region_index)
            })
            || matches!(
                target.id,
                conflict_resolver::ConflictNavTargetId::DisplayBlock(index)
                    if index == display_conflict_index
            )
    }

    pub(crate) fn select_nav_target(&mut self, target_index: usize) -> bool {
        let Some(target) = self.nav_targets.get(target_index) else {
            return false;
        };
        self.nav_anchor = Some(target.anchor());
        self.active_conflict = target.display_conflict_index;
        true
    }

    pub(crate) fn select_display_conflict(&mut self, display_conflict_index: usize) -> bool {
        let Some(target_index) = self
            .nav_targets
            .iter()
            .position(|target| self.nav_target_matches_display(target, display_conflict_index))
        else {
            return false;
        };
        self.nav_anchor = Some(self.nav_targets[target_index].anchor());
        self.active_conflict = Some(display_conflict_index);
        true
    }

    pub(crate) fn reconcile_nav_targets(
        &mut self,
        targets: Vec<conflict_resolver::ConflictNavTarget>,
    ) {
        let previous_targets = std::mem::replace(&mut self.nav_targets, targets);
        let previous_active = self.active_conflict;
        let selected = conflict_resolver::reconcile_conflict_nav_target_index(
            self.nav_anchor,
            &previous_targets,
            &self.nav_targets,
        );
        let Some(selected) = selected else {
            self.nav_anchor = None;
            self.active_conflict = None;
            return;
        };
        let target = &self.nav_targets[selected];
        self.nav_anchor = Some(target.anchor());
        self.active_conflict = previous_active
            .filter(|display| self.nav_target_matches_display(target, *display))
            .or(target.display_conflict_index);
    }

    pub(crate) fn output_line_for_nav_target_provenance(
        &self,
        target: &conflict_resolver::ConflictNavTarget,
    ) -> Option<usize> {
        let aligned_rows = target.aligned_rows.as_ref()?;
        self.resolved_outline.meta.iter().find_map(|meta| {
            let side = match (self.view_mode, meta.source) {
                (ConflictResolverViewMode::ThreeWay, conflict_resolver::ResolvedLineSource::A) => {
                    ThreeWayColumn::Base
                }
                (ConflictResolverViewMode::ThreeWay, conflict_resolver::ResolvedLineSource::B) => {
                    ThreeWayColumn::Ours
                }
                (ConflictResolverViewMode::ThreeWay, conflict_resolver::ResolvedLineSource::C) => {
                    ThreeWayColumn::Theirs
                }
                (
                    ConflictResolverViewMode::TwoWayDiff,
                    conflict_resolver::ResolvedLineSource::A,
                ) => ThreeWayColumn::Ours,
                (
                    ConflictResolverViewMode::TwoWayDiff,
                    conflict_resolver::ResolvedLineSource::B,
                ) => ThreeWayColumn::Theirs,
                (
                    ConflictResolverViewMode::TwoWayDiff,
                    conflict_resolver::ResolvedLineSource::C,
                )
                | (_, conflict_resolver::ResolvedLineSource::Manual) => return None,
            };
            let source_line = usize::try_from(meta.input_line?).ok()?.checked_sub(1)?;
            let aligned_row = self.three_way_row_for_side_line(side, source_line);
            (aligned_rows.contains(&aligned_row)
                || (aligned_rows.is_empty() && aligned_rows.start == aligned_row))
                .then_some(meta.output_line as usize)
        })
    }

    /// Map a visible input-column row to the resolved-output line it produced.
    ///
    /// Quick search walks the *input* columns, so a hit arrives as a visible
    /// row rather than a nav target and
    /// [`Self::output_line_for_nav_target_provenance`] cannot be reused. This
    /// reads the same provenance table, keyed on the row's own side lines: an
    /// output line belongs to this row when it names one of them as its origin.
    /// `meta` is ordered by output line, so the first hit is the earliest line
    /// the row contributed.
    ///
    /// Returns `None` when the outline carries no provenance — large outputs
    /// skip building it (`should_skip_resolved_outline_provenance`), exactly as
    /// conflict navigation's output reveal already degrades there.
    pub(crate) fn output_line_for_visible_row(&self, visible_ix: usize) -> Option<usize> {
        // Indexed by `ResolvedLineSource` A/B/C, which names different columns
        // per view mode — see `output_line_for_nav_target_provenance`.
        let source_lines: [Option<usize>; 3] = match self.view_mode {
            ConflictResolverViewMode::ThreeWay => {
                let aligned_row = self.three_way_aligned_row_for_visible_row(visible_ix)?;
                [
                    self.three_way_aligned
                        .side_line_for_row(ThreeWayColumn::Base.side_index(), aligned_row),
                    self.three_way_aligned
                        .side_line_for_row(ThreeWayColumn::Ours.side_index(), aligned_row),
                    self.three_way_aligned
                        .side_line_for_row(ThreeWayColumn::Theirs.side_index(), aligned_row),
                ]
            }
            ConflictResolverViewMode::TwoWayDiff => {
                // Split rows carry 1-based line numbers: `old` is Ours (source
                // A here), `new` is Theirs (source B). There is no C.
                //
                // Deliberately *not* dispatched on `two_way_uses_aligned_rows`
                // the way `two_way_visible_len` and friends are: the two-way
                // scan that produces these indices resolves them through
                // `two_way_split_projection` unconditionally, so this has to
                // read the same space to agree with it. Both are wrong together
                // whenever the aligned rows are in use — a pre-existing gap
                // between what two-way search indexes and what it renders, which
                // needs fixing on both sides at once.
                let row = self.two_way_split_visible_row(visible_ix)?.row;
                [
                    row.old_line.and_then(|line| (line as usize).checked_sub(1)),
                    row.new_line.and_then(|line| (line as usize).checked_sub(1)),
                    None,
                ]
            }
        };

        if source_lines.iter().all(Option::is_none) {
            return None;
        }

        self.resolved_outline.meta.iter().find_map(|meta| {
            let side_ix = match meta.source {
                conflict_resolver::ResolvedLineSource::A => 0,
                conflict_resolver::ResolvedLineSource::B => 1,
                conflict_resolver::ResolvedLineSource::C => 2,
                conflict_resolver::ResolvedLineSource::Manual => return None,
            };
            let source_line = usize::try_from(meta.input_line?).ok()?.checked_sub(1)?;
            (source_lines[side_ix] == Some(source_line)).then_some(meta.output_line as usize)
        })
    }

    /// The aligned merge-plan row a visible three-way row stands for.
    ///
    /// Fold summary rows answer with the first row they cover, so a match
    /// inside a fold still reveals the right neighbourhood of the output.
    pub(crate) fn three_way_aligned_row_for_visible_row(&self, visible_ix: usize) -> Option<usize> {
        match self.three_way_visible_item(visible_ix)? {
            conflict_resolver::ThreeWayVisibleItem::Line(row) => Some(row),
            conflict_resolver::ThreeWayVisibleItem::CollapsedContext {
                source_line_start, ..
            } => Some(source_line_start),
            conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(conflict_ix) => {
                let range =
                    self.three_way_conflict_ranges[ThreeWayColumn::Ours].get(conflict_ix)?;
                Some(self.three_way_row_for_side_line(ThreeWayColumn::Ours, range.start))
            }
        }
    }

    pub(crate) fn cached_loaded_file_for_target(
        &self,
        repo_id: RepoId,
        path: &std::path::Path,
    ) -> Option<&gitcomet_state::model::ConflictFile> {
        self.matches_target(repo_id, path)
            .then_some(self.loaded_file.as_ref())
            .flatten()
    }

    // ----- Mode accessors -----

    /// Return the rendering mode enum (for tracing / external APIs that expect it).
    #[cfg(test)]
    pub(crate) fn rendering_mode(&self) -> conflict_resolver::ConflictRenderingMode {
        conflict_resolver::ConflictRenderingMode::StreamedLargeFile
    }

    /// Access the streamed conflict state.
    #[cfg(test)]
    #[track_caller]
    pub(crate) fn streamed(&self) -> &StreamedConflictState {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s,
        }
    }

    /// Mutably access the streamed conflict state.
    #[cfg(test)]
    #[track_caller]
    pub(crate) fn streamed_mut(&mut self) -> &mut StreamedConflictState {
        match &mut self.mode_state {
            ConflictModeState::Streamed(s) => s,
        }
    }

    pub(crate) fn split_row_index(&self) -> Option<&conflict_resolver::ConflictSplitRowIndex> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => Some(&s.split_row_index),
        }
    }

    pub(crate) fn two_way_split_projection(
        &self,
    ) -> Option<&conflict_resolver::TwoWaySplitProjection> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => Some(&s.two_way_split_projection),
        }
    }

    pub(crate) fn three_way_visible_projection(
        &self,
    ) -> &conflict_resolver::ThreeWayVisibleProjection {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => &s.three_way_visible_projection,
        }
    }

    #[track_caller]
    #[allow(unused_variables)]
    pub(crate) fn debug_assert_rendering_mode_invariants(&self) {}

    pub(crate) fn three_way_line_count(&self, side: ThreeWayColumn) -> usize {
        self.three_way_line_starts[side].line_count()
    }

    pub(crate) fn three_way_line_starts_ref(&self, side: ThreeWayColumn) -> &[usize] {
        self.three_way_line_starts[side].starts(self.three_way_text[side].as_ref())
    }

    pub(crate) fn three_way_shared_line_starts(&self, side: ThreeWayColumn) -> Arc<[usize]> {
        self.three_way_line_starts[side].shared_starts(self.three_way_text[side].as_ref())
    }

    pub(crate) fn three_way_line_text(&self, side: ThreeWayColumn, line_ix: usize) -> Option<&str> {
        indexed_line_text(
            &self.three_way_text[side],
            self.three_way_line_starts_ref(side),
            line_ix,
        )
    }

    /// The side line rendered at an aligned visual row (section 30 aligned row
    /// space), or `None` for padding rows.
    pub(crate) fn three_way_side_line_for_row(
        &self,
        side: ThreeWayColumn,
        row: usize,
    ) -> Option<usize> {
        self.three_way_aligned
            .side_line_for_row(side.side_index(), row)
    }

    /// Text of the side line rendered at an aligned visual row; `None` for
    /// padding rows and rows past the side's end.
    pub(crate) fn three_way_row_text(&self, side: ThreeWayColumn, row: usize) -> Option<&str> {
        let line_ix = self.three_way_side_line_for_row(side, row)?;
        self.three_way_line_text(side, line_ix)
    }

    /// section 30 R11 (kdiff3 change colours): whether the side columns can tint
    /// rows by their own change vs base — needs a real base and a
    /// non-identity alignment (both-added and unaligned files keep the
    /// marker-region tint).
    pub(crate) fn three_way_per_side_change_rows(&self) -> bool {
        !self.three_way_aligned.is_identity() && !self.three_way_text.base.is_empty()
    }

    /// section 30 R11: whether `column`'s line at aligned `row` differs from the
    /// base line paired at the same row. A line on one side of a padding row
    /// counts as a change; the base column itself is never "changed".
    pub(crate) fn three_way_row_differs_from_base(
        &self,
        column: ThreeWayColumn,
        row: usize,
    ) -> bool {
        if matches!(column, ThreeWayColumn::Base) {
            return false;
        }
        self.three_way_row_text(column, row) != self.three_way_row_text(ThreeWayColumn::Base, row)
    }

    /// The aligned visual row at which a side line renders.
    pub(crate) fn three_way_row_for_side_line(&self, side: ThreeWayColumn, line: usize) -> usize {
        self.three_way_aligned
            .row_for_side_line(side.side_index(), line)
    }

    /// section 30 split: whether row selection / split is available for the current
    /// conflict. Requires a real aligned row space (so rows map consistently
    /// across columns) and a full-text resolver strategy on non-binary data.
    pub(crate) fn conflict_row_selection_enabled(&self) -> bool {
        !self.three_way_aligned.is_identity()
            && !self.is_binary_conflict
            && self.strategy
                == Some(gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver)
    }

    /// section 30 split: whether aligned `row` is inside the current row selection
    /// (highlighted in every source column since rows are shared).
    pub(crate) fn conflict_row_is_selected(&self, row: usize) -> bool {
        self.row_selection
            .is_some_and(|sel| sel.row_range().contains(&row))
    }

    /// KDiff3 manual diff help: whether the resolver can pin alignments at all.
    ///
    /// Shares the row-selection preconditions: a real aligned row space and a
    /// full-text resolver on non-binary data.
    pub(crate) fn manual_alignment_enabled(&self) -> bool {
        self.conflict_row_selection_enabled()
    }

    /// Mark `line` in `column` for a manual alignment.
    ///
    /// `extend` grows the column's existing mark from its anchor; otherwise it
    /// starts a fresh single-line mark. Each column is marked independently —
    /// that is the whole point, since a manual alignment pins lines the
    /// automatic alignment placed on different rows.
    pub(crate) fn set_alignment_selection(
        &mut self,
        column: ThreeWayColumn,
        line: usize,
        extend: bool,
    ) {
        let anchor = match self.alignment_selection[column] {
            Some(selection) if extend => selection.anchor,
            _ => line,
        };
        self.alignment_selection[column] = Some(AlignmentLineSelection { anchor, head: line });
    }

    /// Drop every pending alignment mark. Returns whether anything was marked.
    pub(crate) fn clear_alignment_selections(&mut self) -> bool {
        let had_any = self.has_alignment_selection();
        self.alignment_selection = ThreeWaySides::default();
        had_any
    }

    pub(crate) fn has_alignment_selection(&self) -> bool {
        ThreeWayColumn::ALL
            .iter()
            .any(|column| self.alignment_selection[*column].is_some())
    }

    /// Whether `line` of `column` carries a pending alignment mark.
    pub(crate) fn alignment_line_is_selected(&self, column: ThreeWayColumn, line: usize) -> bool {
        self.alignment_selection[column].is_some_and(|selection| selection.contains(line))
    }

    /// Build the entry a Ctrl+Y would pin from the current marks.
    ///
    /// A column the user left unmarked still needs a position, or the entry
    /// could not be ordered against the others. The aligned row where the
    /// marked columns begin gives it one, and it pins an empty range there —
    /// "the marked lines align against nothing on this side", which is how a
    /// one-sided block gets forced.
    ///
    /// Returns `None` when nothing is marked or the plan cannot be pinned.
    pub(crate) fn manual_alignment_from_selections(
        &self,
        has_base: bool,
    ) -> Option<gitcomet_core::merge::ManualAlignment> {
        if !self.manual_alignment_enabled() || !self.has_alignment_selection() {
            return None;
        }
        let anchor_row = ThreeWayColumn::ALL
            .iter()
            .filter_map(|column| {
                let selection = self.alignment_selection[*column]?;
                Some(
                    self.three_way_aligned
                        .aligned_range_for_side_range(column.side_index(), selection.line_range())
                        .start,
                )
            })
            .min()?;
        let range_for = |column: ThreeWayColumn| match self.alignment_selection[column] {
            Some(selection) => selection.line_range(),
            None => {
                let line = self
                    .three_way_aligned
                    .side_line_lower_bound(column.side_index(), anchor_row);
                line..line
            }
        };
        let base = if has_base {
            range_for(ThreeWayColumn::Base)
        } else {
            0..0
        };
        Some(gitcomet_core::merge::ManualAlignment::new(
            base,
            range_for(ThreeWayColumn::Ours),
            range_for(ThreeWayColumn::Theirs),
        ))
    }

    /// section 30 split: the shared aligned-row range of conflict block `conflict_ix`
    /// (all source columns share it after `rebuild_three_way_visible_state`).
    pub(crate) fn three_way_block_aligned_range(
        &self,
        conflict_ix: usize,
    ) -> Option<std::ops::Range<usize>> {
        self.three_way_conflict_ranges[ThreeWayColumn::Ours]
            .get(conflict_ix)
            .cloned()
    }

    /// section 30 split: clamp aligned `row` into conflict block `conflict_ix`.
    pub(crate) fn clamp_row_to_conflict_block(&self, conflict_ix: usize, row: usize) -> usize {
        match self.three_way_block_aligned_range(conflict_ix) {
            Some(range) if !range.is_empty() => row.clamp(range.start, range.end - 1),
            _ => row,
        }
    }

    /// section 30 split: convert a normalized row selection inside a conflict block
    /// into block-local per-side split boundaries and the target region index.
    /// Returns `None` when selection/split is unavailable, the selection is
    /// degenerate (covers the whole block or nothing), or the block maps to a
    /// non-unique session region.
    pub(crate) fn split_boundaries_for_selection(
        &self,
    ) -> Option<(
        usize,
        gitcomet_core::conflict_session::ConflictRegionSplitBoundaries,
    )> {
        let selection = self.row_selection?;
        if !self.conflict_row_selection_enabled() {
            return None;
        }
        // Custom/manual resolutions can replace a raw region with display
        // text, shifting every later display-side range away from the
        // immutable source alignment. Only split while display blocks retain
        // a one-to-one, in-order mapping to raw session regions.
        if self.conflict_region_indices.len() != self.conflict_region_marker_has_base.len()
            || self
                .conflict_region_indices
                .iter()
                .enumerate()
                .any(|(block_index, &region_index)| block_index != region_index)
        {
            return None;
        }
        let conflict_ix = selection.conflict_ix;
        let block = self.three_way_block_aligned_range(conflict_ix)?;
        if block.is_empty() {
            return None;
        }
        let row_range = selection.row_range();
        let sel_start = (*row_range.start()).max(block.start);
        let sel_end_inclusive = (*row_range.end()).min(block.end - 1);
        if sel_start > sel_end_inclusive {
            return None;
        }
        // A selection covering the whole block cannot split it.
        if sel_start <= block.start && sel_end_inclusive >= block.end - 1 {
            return None;
        }

        let marker_block = self
            .marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .nth(conflict_ix)?;
        let line_count = |text: &str| {
            if text.is_empty() {
                0
            } else {
                text.as_bytes()
                    .iter()
                    .filter(|&&byte| byte == b'\n')
                    .count()
                    + usize::from(!text.ends_with('\n'))
            }
        };

        let side_bounds = |side: usize| -> Option<([usize; 2], usize)> {
            // The aligned map is built from the actual staged sides. Its position
            // at the block's first aligned row remains correct when clean context
            // before this block exists on only one side; marker Text segments do
            // not retain enough information to reconstruct that position.
            let base = self
                .three_way_aligned
                .side_line_lower_bound(side, block.start);
            let b0 = self
                .three_way_aligned
                .side_line_lower_bound(side, sel_start)
                .saturating_sub(base);
            let b1 = self
                .three_way_aligned
                .side_line_lower_bound(side, sel_end_inclusive + 1)
                .saturating_sub(base);
            let len = match side {
                0 => line_count(marker_block.base.as_deref().unwrap_or_default()),
                1 => line_count(&marker_block.ours),
                2 => line_count(&marker_block.theirs),
                _ => return None,
            };
            let b0 = b0.min(len);
            let b1 = b1.clamp(b0, len);
            Some(([b0, b1], len))
        };

        let region_index = self.conflict_region_indices.get(conflict_ix).copied()?;
        if self
            .conflict_region_indices
            .iter()
            .filter(|&&index| index == region_index)
            .take(2)
            .count()
            != 1
        {
            return None;
        }
        let has_base = self
            .conflict_region_marker_has_base
            .get(region_index)
            .copied()?;
        let (ours, ours_len) = side_bounds(ThreeWayColumn::Ours.side_index())?;
        let (theirs, theirs_len) = side_bounds(ThreeWayColumn::Theirs.side_index())?;
        let base = if has_base {
            Some(side_bounds(ThreeWayColumn::Base.side_index())?)
        } else {
            None
        };

        // Alignment can contain padding/base-only rows that have no content in
        // the serialized marker block. Do not advertise a split unless the
        // selection owns at least one serialized line and leaves at least one
        // serialized line outside the new region.
        let selected_has_content = ours[0] < ours[1]
            || theirs[0] < theirs[1]
            || base.is_some_and(|(bounds, _)| bounds[0] < bounds[1]);
        let has_content_outside = ours[0] > 0
            || ours[1] < ours_len
            || theirs[0] > 0
            || theirs[1] < theirs_len
            || base.is_some_and(|(bounds, len)| bounds[0] > 0 || bounds[1] < len);
        if !selected_has_content || !has_content_outside {
            return None;
        }

        let boundaries = gitcomet_core::conflict_session::ConflictRegionSplitBoundaries {
            ours,
            theirs,
            base: base.map(|(bounds, _)| bounds),
        };
        Some((region_index, boundaries))
    }

    /// Whether two consecutive displayed marker blocks can be joined without
    /// crossing malformed marker-looking context. This mirrors the core
    /// surgery guard so an enabled menu item does not silently no-op.
    pub(crate) fn conflict_blocks_have_joinable_context(
        &self,
        first_conflict_ix: usize,
        second_conflict_ix: usize,
    ) -> bool {
        if first_conflict_ix.checked_add(1) != Some(second_conflict_ix) {
            return false;
        }
        let markerish = |text: &str| {
            text.lines().any(|line| {
                line.starts_with("<<<<<<<")
                    || line.starts_with("=======")
                    || line.starts_with(">>>>>>>")
                    || line.starts_with("|||||||")
            })
        };
        let mut conflict_ix = 0usize;
        let mut between = false;
        for segment in &self.marker_segments {
            match segment {
                conflict_resolver::ConflictSegment::Block(_) => {
                    if conflict_ix == second_conflict_ix {
                        return between;
                    }
                    between = conflict_ix == first_conflict_ix;
                    conflict_ix = conflict_ix.saturating_add(1);
                }
                conflict_resolver::ConflictSegment::Text(text) if between => {
                    if markerish(text.as_str()) {
                        return false;
                    }
                }
                conflict_resolver::ConflictSegment::Text(_) => {}
            }
        }
        false
    }

    pub(crate) fn three_way_has_line(&self, side: ThreeWayColumn, line_ix: usize) -> bool {
        self.three_way_line_text(side, line_ix).is_some()
    }

    /// Return source-pane text for a conflict pick choice at a global line index.
    ///
    /// This reads from the indexed merge-input texts directly so callers do not
    /// depend on eager diff rows or streamed page generation.
    pub(crate) fn source_line_text_for_choice(
        &self,
        choice: conflict_resolver::ConflictChoice,
        line_ix: usize,
    ) -> Option<&str> {
        match choice {
            conflict_resolver::ConflictChoice::Base
                if self.view_mode == ConflictResolverViewMode::ThreeWay =>
            {
                self.three_way_line_text(ThreeWayColumn::Base, line_ix)
            }
            conflict_resolver::ConflictChoice::Ours => {
                self.three_way_line_text(ThreeWayColumn::Ours, line_ix)
            }
            conflict_resolver::ConflictChoice::Theirs => {
                self.three_way_line_text(ThreeWayColumn::Theirs, line_ix)
            }
            conflict_resolver::ConflictChoice::Base | conflict_resolver::ConflictChoice::Both => {
                None
            }
            _ => None,
        }
    }

    /// Look up the visible item at `visible_ix`, dispatching between the eager
    /// map (small files) and the span-based projection (giant files).
    pub(crate) fn three_way_visible_item(
        &self,
        visible_ix: usize,
    ) -> Option<conflict_resolver::ThreeWayVisibleItem> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s.three_way_visible_projection.get(visible_ix),
        }
    }

    /// Number of visible rows in the three-way view.
    pub(crate) fn three_way_visible_len(&self) -> usize {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s.three_way_visible_projection.len(),
        }
    }

    /// Look up the conflict index for a given line on a given side.
    /// Uses binary search on per-side ranges in giant mode, O(1) array lookup otherwise.
    pub(crate) fn conflict_index_for_side_line(
        &self,
        side: ThreeWayColumn,
        line_ix: usize,
    ) -> Option<usize> {
        let ranges = &self.three_way_conflict_ranges[side];
        conflict_resolver::conflict_index_for_line(ranges, line_ix)
    }

    /// Find the visible index for a conflict range, using the projection in giant mode.
    pub(crate) fn visible_index_for_conflict(&self, range_ix: usize) -> Option<usize> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => {
                s.three_way_visible_projection.visible_index_for_conflict(
                    &self.three_way_conflict_ranges[ThreeWayColumn::Ours],
                    range_ix,
                )
            }
        }
    }

    /// Find the visible row for an aligned merge-plan row. Context hidden by a
    /// fold maps to the fold summary row.
    pub(crate) fn visible_index_for_aligned_row(&self, row: usize) -> Option<usize> {
        self.three_way_visible_projection()
            .visible_index_for_source_line(row)
    }

    // ----- Two-way split dispatch (giant vs eager) -----

    /// section 30 aligned row space: whether the two-way view renders the shared
    /// aligned whole-file rows (full mode) instead of the block-local
    /// `ConflictSplitRowIndex` rows (giant files / sides not loaded).
    pub(crate) fn two_way_uses_aligned_rows(&self) -> bool {
        !self.three_way_aligned.is_identity()
    }

    /// Number of visible rows in the two-way view (aligned or block-local).
    pub(crate) fn two_way_visible_len(&self) -> usize {
        if self.two_way_uses_aligned_rows() {
            self.three_way_visible_len()
        } else {
            self.two_way_split_visible_len()
        }
    }

    /// Number of visible rows in the two-way split view.
    pub(crate) fn two_way_split_visible_len(&self) -> usize {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s.two_way_split_projection.visible_len(),
        }
    }

    /// Retrieve a materialized split row for the given visible index,
    /// dispatching between the paged index (giant) and the eager `diff_rows`
    /// array (small).
    pub(crate) fn two_way_split_visible_row(
        &self,
        visible_ix: usize,
    ) -> Option<conflict_resolver::TwoWaySplitVisibleRow> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => {
                let (source_row_ix, conflict_ix) = s.two_way_split_projection.get(visible_ix)?;
                let row = s
                    .split_row_index
                    .row_at(&self.marker_segments, source_row_ix)?;
                Some(conflict_resolver::TwoWaySplitVisibleRow {
                    source_row_ix,
                    row,
                    conflict_ix,
                })
            }
        }
    }

    /// Retrieve a split row by source row index (not visible index).
    pub(crate) fn two_way_split_row_by_source(
        &self,
        row_ix: usize,
    ) -> Option<gitcomet_core::file_diff::FileDiffRow> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => {
                s.split_row_index.row_at(&self.marker_segments, row_ix)
            }
        }
    }

    pub(crate) fn two_way_split_visual_kind_at(
        &mut self,
        row_ix: usize,
        row: &gitcomet_core::file_diff::FileDiffRow,
        whitespace_mode: DiffWhitespaceMode,
    ) -> gitcomet_core::file_diff::FileDiffRowKind {
        use gitcomet_core::file_diff::FileDiffRowKind as RK;

        if whitespace_mode == DiffWhitespaceMode::Show || matches!(row.kind, RK::Context) {
            return row.kind;
        }

        if let Some(kind) = self.two_way_split_visual_kind_cache.get(&row_ix).copied() {
            return kind;
        }

        self.cache_two_way_split_visual_kind_run(row_ix);
        self.two_way_split_visual_kind_cache
            .get(&row_ix)
            .copied()
            .unwrap_or(row.kind)
    }

    pub(crate) fn cache_two_way_split_visual_kind_run(&mut self, row_ix: usize) {
        use gitcomet_core::file_diff::FileDiffRowKind as RK;

        let mut start = row_ix;
        while start > 0 {
            let Some(prev) = self.two_way_split_row_by_source(start - 1) else {
                break;
            };
            if matches!(prev.kind, RK::Context) {
                break;
            }
            start -= 1;
        }

        let mut old_stripped = String::new();
        let mut new_stripped = String::new();
        let mut end = start;
        while let Some(next) = self.two_way_split_row_by_source(end) {
            if matches!(next.kind, RK::Context) {
                break;
            }
            append_conflict_row_without_whitespace(&next, &mut old_stripped, &mut new_stripped);
            end += 1;
        }

        if start == end {
            return;
        }

        if old_stripped == new_stripped {
            for ix in start..end {
                self.two_way_split_visual_kind_cache.insert(ix, RK::Context);
            }
            return;
        }

        for ix in start..end {
            if let Some(row) = self.two_way_split_row_by_source(ix) {
                self.two_way_split_visual_kind_cache.insert(ix, row.kind);
            }
        }
    }

    /// Find the first visible index for a conflict in two-way split view.
    pub(crate) fn two_way_split_visible_ix_for_conflict(
        &self,
        conflict_ix: usize,
    ) -> Option<usize> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s
                .two_way_split_projection
                .visible_index_for_conflict(conflict_ix),
        }
    }

    /// Map a two-way split visible index back to its conflict index.
    #[cfg(test)]
    pub(crate) fn two_way_split_conflict_ix_for_visible(&self, visible_ix: usize) -> Option<usize> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s
                .two_way_split_projection
                .get(visible_ix)
                .and_then(|(_, ci)| ci),
        }
    }

    /// Build unresolved conflict navigation entries for two-way split view.
    #[cfg(test)]
    pub(crate) fn two_way_split_nav_entries(&self) -> Vec<usize> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => {
                conflict_resolver::unresolved_conflict_indices(&self.marker_segments)
                    .into_iter()
                    .filter_map(|ci| s.two_way_split_projection.visible_index_for_conflict(ci))
                    .collect()
            }
        }
    }

    // ----- Unified two-way dispatch (aligned vs block-local) -----

    /// Build unresolved conflict navigation entries for the current two-way
    /// conflict diff view.
    #[cfg(test)]
    pub(crate) fn two_way_nav_entries(&self) -> Vec<usize> {
        if self.two_way_uses_aligned_rows() {
            return conflict_resolver::unresolved_conflict_indices(&self.marker_segments)
                .into_iter()
                .filter_map(|ci| self.visible_index_for_conflict(ci))
                .collect();
        }
        self.two_way_split_nav_entries()
    }

    /// Map a two-way visible index to its conflict index.
    #[cfg(test)]
    pub(crate) fn two_way_conflict_ix_for_visible(&self, visible_ix: usize) -> Option<usize> {
        if self.two_way_uses_aligned_rows() {
            return match self.three_way_visible_item(visible_ix)? {
                conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(ri) => Some(ri),
                conflict_resolver::ThreeWayVisibleItem::Line(row) => {
                    // Conflict ranges are aligned-row ranges shared by all
                    // columns, so any side works for the lookup.
                    self.conflict_index_for_side_line(ThreeWayColumn::Ours, row)
                }
                conflict_resolver::ThreeWayVisibleItem::CollapsedContext { .. } => None,
            };
        }
        self.two_way_split_conflict_ix_for_visible(visible_ix)
    }

    /// Find the first visible index for a conflict in the current two-way diff
    /// view.
    pub(crate) fn two_way_visible_ix_for_conflict(&self, conflict_ix: usize) -> Option<usize> {
        if self.two_way_uses_aligned_rows() {
            return self.visible_index_for_conflict(conflict_ix);
        }
        self.two_way_split_visible_ix_for_conflict(conflict_ix)
    }

    /// Return (diff_row_count, inline_row_count) for trace recording.
    pub(crate) fn two_way_row_counts(&self) -> (usize, usize) {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => (s.split_row_index.total_rows(), 0),
        }
    }

    pub(crate) fn three_way_horizontal_measure_row(&self, side: ThreeWayColumn) -> usize {
        match side {
            ThreeWayColumn::Base => self.three_way_horizontal_measure_rows[0],
            ThreeWayColumn::Ours => self.three_way_horizontal_measure_rows[1],
            ThreeWayColumn::Theirs => self.three_way_horizontal_measure_rows[2],
        }
    }

    pub(crate) fn two_way_horizontal_measure_row(
        &self,
        side: conflict_resolver::ConflictPickSide,
    ) -> usize {
        // Aligned two-way rows share the three-way row space, so the
        // three-way per-column measurements apply directly.
        if self.two_way_uses_aligned_rows() {
            return match side {
                conflict_resolver::ConflictPickSide::Ours => {
                    self.three_way_horizontal_measure_row(ThreeWayColumn::Ours)
                }
                conflict_resolver::ConflictPickSide::Theirs => {
                    self.three_way_horizontal_measure_row(ThreeWayColumn::Theirs)
                }
            };
        }
        match side {
            conflict_resolver::ConflictPickSide::Ours => self.two_way_horizontal_measure_rows[0],
            conflict_resolver::ConflictPickSide::Theirs => self.two_way_horizontal_measure_rows[1],
        }
    }

    pub(crate) fn refresh_three_way_horizontal_measure_rows(&mut self) {
        self.three_way_horizontal_measure_rows = self.compute_three_way_horizontal_measure_rows();
    }

    pub(crate) fn refresh_two_way_horizontal_measure_rows(&mut self) {
        self.two_way_horizontal_measure_rows = self.compute_two_way_horizontal_measure_rows();
    }

    pub(crate) fn compute_three_way_horizontal_measure_rows(&self) -> [usize; 3] {
        let has_hidden_resolved_blocks = self.hide_resolved
            && self.marker_segments.iter().any(|segment| {
                matches!(
                    segment,
                    conflict_resolver::ConflictSegment::Block(block) if block.resolved
                )
            });
        if self.collapse_context || has_hidden_resolved_blocks {
            // This helper already returns indices in the compact visible
            // projection. Mapping them as side-line indices would apply the
            // alignment a second time and can select an unrelated row. Context
            // folding likewise changes visible indices even without a hidden
            // resolved block.
            return self.compute_three_way_horizontal_measure_rows_from_visible_projection();
        }

        let rows = self.compute_three_way_horizontal_measure_side_lines();
        // The scan yields indices in each stage's own text; width measurement
        // wants their corresponding aligned rows.
        [
            self.three_way_row_for_side_line(ThreeWayColumn::Base, rows[0]),
            self.three_way_row_for_side_line(ThreeWayColumn::Ours, rows[1]),
            self.three_way_row_for_side_line(ThreeWayColumn::Theirs, rows[2]),
        ]
    }

    pub(crate) fn compute_three_way_horizontal_measure_side_lines(&self) -> [usize; 3] {
        // Marker text is the merge result, not any one index stage. Clean
        // changes outside conflict markers can therefore add or remove lines
        // on only one side. Walking marker segments and advancing all three
        // counters together produces invalid stage coordinates (and can make
        // a column measure a short row instead of its widest row). Scan each
        // actual stage text independently instead.
        [
            conflict_resolver::scan_text_line_stats(self.three_way_text.base.as_ref())
                .widest_line()
                .map_or(0, |(line_ix, _)| line_ix),
            conflict_resolver::scan_text_line_stats(self.three_way_text.ours.as_ref())
                .widest_line()
                .map_or(0, |(line_ix, _)| line_ix),
            conflict_resolver::scan_text_line_stats(self.three_way_text.theirs.as_ref())
                .widest_line()
                .map_or(0, |(line_ix, _)| line_ix),
        ]
    }

    pub(crate) fn compute_three_way_horizontal_measure_rows_from_visible_projection(
        &self,
    ) -> [usize; 3] {
        let mut best_rows = [0usize; 3];
        let mut best_lens = [0usize; 3];

        for span in self.three_way_visible_projection().spans() {
            let conflict_resolver::ThreeWayVisibleSpan::Lines {
                visible_start,
                source_line_start,
                len,
            } = *span
            else {
                continue;
            };

            for offset in 0..len {
                let visible_ix = visible_start + offset;
                let line_ix = source_line_start + offset;

                for (slot, side) in [
                    ThreeWayColumn::Base,
                    ThreeWayColumn::Ours,
                    ThreeWayColumn::Theirs,
                ]
                .into_iter()
                .enumerate()
                {
                    let width = self.three_way_row_text(side, line_ix).map_or(0, str::len);
                    if width > best_lens[slot] {
                        best_lens[slot] = width;
                        best_rows[slot] = visible_ix;
                    }
                }
            }
        }

        best_rows
    }

    pub(crate) fn compute_two_way_horizontal_measure_rows(&self) -> [usize; 2] {
        let Some(split_row_index) = self.split_row_index() else {
            return [0; 2];
        };
        let Some(projection) = self.two_way_split_projection() else {
            return [0; 2];
        };

        let [ours_source_row, theirs_source_row] = split_row_index
            .widest_source_rows_by_text_len(&self.marker_segments, self.hide_resolved);

        [
            ours_source_row
                .and_then(|row_ix| projection.source_to_visible(row_ix))
                .unwrap_or(0),
            theirs_source_row
                .and_then(|row_ix| projection.source_to_visible(row_ix))
                .unwrap_or(0),
        ]
    }

    /// Pre-computed word highlights for a source row in the two-way split view.
    /// Return an already-computed giant-mode word highlight pair.
    pub(crate) fn two_way_split_word_highlight(
        &self,
        row_ix: usize,
    ) -> Option<Arc<conflict_resolver::TwoWayWordHighlightPair>> {
        self.two_way_split_word_highlight_cache.get(row_ix)
    }

    /// Cache a giant-mode word highlight pair so the other split column and
    /// later frames reuse the same word diff.
    pub(crate) fn cache_two_way_split_word_highlight(
        &mut self,
        row_ix: usize,
        highlights: conflict_resolver::TwoWayWordHighlightPair,
    ) -> Arc<conflict_resolver::TwoWayWordHighlightPair> {
        self.two_way_split_word_highlight_cache
            .insert(row_ix, highlights)
    }

    pub(crate) fn two_way_split_word_highlight_for_row(
        &mut self,
        row_ix: usize,
        row: &gitcomet_core::file_diff::FileDiffRow,
    ) -> Option<Arc<conflict_resolver::TwoWayWordHighlightPair>> {
        self.two_way_split_word_highlight(row_ix).or_else(|| {
            conflict_resolver::compute_word_highlights_for_row(row)
                .map(|highlights| self.cache_two_way_split_word_highlight(row_ix, highlights))
        })
    }

    /// Rebuild three-way visible state (conflict maps + visible map/projection)
    /// from current marker segments and line counts.
    pub(crate) fn rebuild_three_way_visible_state(&mut self) {
        let maps = conflict_resolver::build_three_way_conflict_maps_without_line_maps(
            &self.marker_segments,
            self.three_way_line_count(ThreeWayColumn::Base),
            self.three_way_line_count(ThreeWayColumn::Ours),
            self.three_way_line_count(ThreeWayColumn::Theirs),
        );
        let block_count = maps.conflict_ranges[1].len();
        let exact_plan_ranges = self
            .merge_plan_aligned_conflict_ranges
            .as_ref()
            .filter(|ranges| {
                ranges.len() == block_count
                    && ranges
                        .iter()
                        .all(|range| range.start <= range.end && range.end <= self.three_way_len)
                    && ranges.windows(2).all(|pair| pair[0].end <= pair[1].start)
            })
            .cloned();
        let aligned_ranges = exact_plan_ranges.unwrap_or_else(|| {
            // Legacy/current-only fallback: project marker-text offsets back
            // through the side alignment. Marker text is output space rather
            // than source space, so this is necessarily an estimate.
            conflict_resolver::project_conflict_ranges_to_aligned_rows(
                &self.marker_segments,
                &self.three_way_aligned,
                [
                    self.three_way_line_count(ThreeWayColumn::Base),
                    self.three_way_line_count(ThreeWayColumn::Ours),
                    self.three_way_line_count(ThreeWayColumn::Theirs),
                ],
            )
        });
        let three_way_visible_projection =
            conflict_resolver::build_three_way_visible_projection_with_options(
                self.three_way_len,
                &aligned_ranges,
                &maps.conflict_resolved,
                conflict_resolver::ThreeWayVisibleOptions {
                    hide_resolved: self.hide_resolved,
                    collapse_context: self.collapse_context,
                    context_fold_reveals: Some(&self.context_fold_reveals),
                },
            );
        self.apply_three_way_conflict_maps(maps);
        // All columns share the aligned conflict ranges.
        self.three_way_conflict_ranges = ThreeWaySides {
            base: aligned_ranges.clone(),
            ours: aligned_ranges.clone(),
            theirs: aligned_ranges,
        };
        match &mut self.mode_state {
            ConflictModeState::Streamed(s) => {
                s.three_way_visible_projection = three_way_visible_projection;
            }
        }
        self.three_way_visible_state_ready = true;
        self.refresh_three_way_horizontal_measure_rows();
        self.rebuild_minimap_bands();
    }

    /// Recompute the minimap column's bands for the current projection.
    ///
    /// Runs from `rebuild_three_way_visible_state`, after the aligned conflict
    /// ranges are in place, so a pick recolors the band it settles.
    pub(crate) fn rebuild_minimap_bands(&mut self) {
        let projection = match &self.mode_state {
            ConflictModeState::Streamed(s) => &s.three_way_visible_projection,
        };
        let resolved =
            conflict_resolver::resolved_conflict_flags_from_segments(&self.marker_segments);
        self.minimap_bands = conflict_resolver::build_minimap_bands(
            &self.three_way_aligned,
            projection,
            &self.three_way_conflict_ranges[ThreeWayColumn::Ours],
            &resolved,
            conflict_resolver::CONFLICT_BOTTOM_OVERSCROLL_ROWS,
        )
        .into();
    }

    /// Whether the minimap column has anything to show.
    pub(crate) fn has_minimap(&self) -> bool {
        !self.minimap_bands.is_empty()
    }

    /// Rebuild two-way visible state from current marker segments.
    /// Rebuilds the streamed split row index and projection.
    pub(crate) fn rebuild_two_way_visible_state(&mut self) {
        self.two_way_split_visual_kind_cache.clear();
        self.two_way_split_word_highlight_cache.clear();
        let ConflictModeState::Streamed(s) = &mut self.mode_state;
        s.split_row_index = conflict_resolver::ConflictSplitRowIndex::new(
            &self.marker_segments,
            conflict_resolver::BLOCK_LOCAL_DIFF_CONTEXT_LINES,
        );
        self.rebuild_two_way_visible_projections();
    }

    /// Rebuild streamed two-way visible projections from the current split-row index.
    pub(crate) fn rebuild_two_way_visible_projections(&mut self) {
        match &mut self.mode_state {
            ConflictModeState::Streamed(s) => {
                s.two_way_split_projection = conflict_resolver::TwoWaySplitProjection::new(
                    &s.split_row_index,
                    &self.marker_segments,
                    self.hide_resolved,
                );
            }
        }
        self.debug_assert_rendering_mode_invariants();
        self.refresh_two_way_horizontal_measure_rows();
    }

    /// Apply three-way conflict maps to state fields.
    pub(crate) fn apply_three_way_conflict_maps(
        &mut self,
        maps: conflict_resolver::ThreeWayConflictMaps,
    ) {
        let [base_ranges, ours_ranges, theirs_ranges] = maps.conflict_ranges;
        self.three_way_conflict_ranges = ThreeWaySides {
            base: base_ranges,
            ours: ours_ranges,
            theirs: theirs_ranges,
        };
        self.conflict_has_base = maps.conflict_has_base;
        self.refresh_conflict_choices_from_segments();
    }

    pub(crate) fn refresh_conflict_has_base_from_segments(&mut self) {
        self.conflict_has_base = self
            .marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.base.is_some()),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .collect();
        self.refresh_conflict_choices_from_segments();
    }

    pub(crate) fn refresh_conflict_choices_from_segments(&mut self) {
        self.conflict_choices = self
            .marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.choice),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .collect();
    }

    pub(crate) fn has_three_way_visible_state_ready(&self) -> bool {
        self.three_way_visible_state_ready
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default, clippy::single_range_in_vec_init)]
mod conflict_resolver_ui_state_tests {
    use super::{
        ConflictPreviewImage, ConflictResolverUiState, ConflictRowSelection, DeferredLineStarts,
        DiffWhitespaceMode, Loadable, ThreeWayColumn, ThreeWaySides, preview_image_element_id,
    };
    use crate::view::conflict_resolver::{
        self, ConflictBlock, ConflictChoice, ConflictNavTarget, ConflictNavTargetId,
        ConflictResolverViewMode, ConflictSegment, ConflictSplitRowIndex, ResolvedLineMeta,
        ResolvedLineSource, ThreeWayVisibleItem, TwoWaySplitProjection,
    };

    #[test]
    fn animated_preview_image_elements_have_stable_distinct_ids() {
        let frame = image::Frame::new(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([0, 0, 0, 0]),
        ));
        let preview =
            ConflictPreviewImage::Rendered(std::sync::Arc::new(gpui::RenderImage::new(vec![
                frame.clone(),
                frame,
            ])));

        let base = preview.element("conflict_preview_base");
        let ours = preview.element("conflict_preview_ours");

        assert_eq!(
            gpui::Element::id(&base),
            Some(preview_image_element_id("conflict_preview_base"))
        );
        assert_eq!(
            gpui::Element::id(&ours),
            Some(preview_image_element_id("conflict_preview_ours"))
        );
        assert_ne!(gpui::Element::id(&base), gpui::Element::id(&ours));
    }

    #[test]
    pub(crate) fn default_groups_three_way_side_fields() {
        let state = ConflictResolverUiState::default();

        assert!(state.three_way_text.base.is_empty());
        assert!(state.three_way_text.ours.is_empty());
        assert!(state.three_way_text.theirs.is_empty());
        assert!(state.rendering_mode().is_streamed_large_file());
        assert!(state.three_way_line_starts.base.is_empty());
        assert!(state.three_way_line_starts.ours.is_empty());
        assert!(state.three_way_line_starts.theirs.is_empty());
        assert!(state.three_way_conflict_ranges.base.is_empty());
        assert!(state.three_way_word_highlights.base.is_empty());
        assert!(state.split_row_index().is_some());
        assert!(state.two_way_split_projection().is_some());
        assert!(matches!(
            state.markdown_preview.documents.base,
            Loadable::NotLoaded
        ));
    }

    #[test]
    pub(crate) fn three_way_sides_keep_each_column_separate() {
        let mut sides = ThreeWaySides {
            base: vec![1],
            ours: vec![2],
            theirs: vec![3],
        };

        sides.base.push(10);
        sides.ours.push(20);
        sides.theirs.push(30);

        assert_eq!(sides.base, vec![1, 10]);
        assert_eq!(sides.ours, vec![2, 20]);
        assert_eq!(sides.theirs, vec![3, 30]);
    }

    #[test]
    pub(crate) fn three_way_sides_index_by_column() {
        let mut sides = ThreeWaySides {
            base: 10,
            ours: 20,
            theirs: 30,
        };

        assert_eq!(sides[ThreeWayColumn::Base], 10);
        assert_eq!(sides[ThreeWayColumn::Ours], 20);
        assert_eq!(sides[ThreeWayColumn::Theirs], 30);

        sides[ThreeWayColumn::Ours] = 42;
        assert_eq!(sides.ours, 42);
    }

    #[test]
    pub(crate) fn source_line_text_for_choice_reads_two_way_inputs_from_indexed_text() {
        let mut state = ConflictResolverUiState {
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            ..Default::default()
        };
        state.three_way_text.ours = "o0\no1\n".into();
        state.three_way_text.theirs = "t0\nt1\n".into();
        state.three_way_line_starts.ours = vec![0, 3].into();
        state.three_way_line_starts.theirs = vec![0, 3].into();

        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Ours, 1),
            Some("o1")
        );
        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Theirs, 0),
            Some("t0")
        );
        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Base, 0),
            None
        );
        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Both, 0),
            None
        );
    }

    #[test]
    pub(crate) fn source_line_text_for_choice_reads_base_only_in_three_way_mode() {
        let mut state = ConflictResolverUiState {
            view_mode: ConflictResolverViewMode::ThreeWay,
            ..Default::default()
        };
        state.three_way_text.base = "b0\nb1\n".into();
        state.three_way_text.ours = "o0\no1\n".into();
        state.three_way_text.theirs = "t0\nt1\n".into();
        state.three_way_line_starts.base = vec![0, 3].into();
        state.three_way_line_starts.ours = vec![0, 3].into();
        state.three_way_line_starts.theirs = vec![0, 3].into();

        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Base, 1),
            Some("b1")
        );
        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Ours, 0),
            Some("o0")
        );
        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Theirs, 1),
            Some("t1")
        );
    }

    #[test]
    pub(crate) fn apply_three_way_conflict_maps_distributes_ranges_and_flags() {
        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".into()),
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
            whitespace_only: false,
        })];
        let maps = conflict_resolver::ThreeWayConflictMaps {
            conflict_ranges: [vec![0..3], vec![0..5], vec![0..4]],
            line_conflict_maps: [vec![Some(0); 3], vec![Some(0); 5], vec![Some(0); 4]],
            conflict_has_base: vec![true],
            conflict_resolved: vec![true],
        };
        state.apply_three_way_conflict_maps(maps.clone());

        assert_eq!(
            state.three_way_conflict_ranges.base,
            maps.conflict_ranges[0]
        );
        assert_eq!(
            state.three_way_conflict_ranges.ours,
            maps.conflict_ranges[1]
        );
        assert_eq!(
            state.three_way_conflict_ranges.theirs,
            maps.conflict_ranges[2]
        );
        assert_eq!(state.conflict_has_base, maps.conflict_has_base);
        assert_eq!(state.conflict_choices, vec![ConflictChoice::Theirs]);
    }

    #[test]
    pub(crate) fn merge_plan_ranges_override_marker_output_offset_estimates() {
        let block = |ours: &str, theirs: &str| {
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: ours.into(),
                theirs: theirs.into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            })
        };
        let exact_ranges = vec![1..3, 4..5, 6..8];
        let mut state = ConflictResolverUiState {
            // These text segments are merged-output projections whose line
            // counts do not represent positions in both immutable sources.
            marker_segments: vec![
                ConflictSegment::Text("one-sided resolved output\n".into()),
                block("local-a\nlocal-b\n", "remote-a\n"),
                ConflictSegment::Text("another selected-side line\n".into()),
                block("local-c\n", "remote-c\nremote-extra\n"),
                ConflictSegment::Text("selected output before final block\n".into()),
                block("local-d\nlocal-e\n", "remote-d\n"),
            ],
            three_way_len: 9,
            merge_plan_aligned_conflict_ranges: Some(exact_ranges.clone()),
            ..Default::default()
        };

        state.rebuild_three_way_visible_state();

        assert_eq!(state.three_way_conflict_ranges.base, exact_ranges);
        assert_eq!(
            state.three_way_conflict_ranges.ours,
            state.three_way_conflict_ranges.base
        );
        assert_eq!(
            state.three_way_conflict_ranges.theirs,
            state.three_way_conflict_ranges.base
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Ours, 1),
            Some(0)
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Ours, 3),
            None
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Ours, 4),
            Some(1)
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Ours, 7),
            Some(2)
        );
    }

    #[test]
    pub(crate) fn refresh_conflict_has_base_from_segments_refreshes_choice_cache() {
        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "ours\n".into(),
                theirs: "theirs\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".into()),
                ours: "ours2\n".into(),
                theirs: "theirs2\n".into(),
                choice: ConflictChoice::Both,
                resolved: true,
                whitespace_only: false,
            }),
        ];

        state.refresh_conflict_has_base_from_segments();

        assert_eq!(state.conflict_has_base, vec![false, true]);
        assert_eq!(
            state.conflict_choices,
            vec![ConflictChoice::Ours, ConflictChoice::Both]
        );
    }

    #[test]
    pub(crate) fn ignored_whitespace_visual_kind_caches_entire_change_run() {
        use gitcomet_core::file_diff::FileDiffRowKind as RK;

        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "let x = 1\nabc  \n".into(),
            theirs: "let x=1\nabc\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        })];
        state.rebuild_two_way_visible_state();

        let first_row = state.two_way_split_row_by_source(0).unwrap();
        assert_eq!(
            state.two_way_split_visual_kind_at(0, &first_row, DiffWhitespaceMode::Ignore),
            RK::Context
        );

        assert_eq!(state.two_way_split_visual_kind_cache.len(), 2);
        assert_eq!(
            state.two_way_split_visual_kind_cache.get(&1).copied(),
            Some(RK::Context)
        );
    }

    #[test]
    pub(crate) fn giant_two_way_word_highlights_are_shared_between_column_renders() {
        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "let local_name = value;\n".into(),
            theirs: "let remote_name = value;\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        })];
        state.rebuild_two_way_visible_state();
        let row = state.two_way_split_row_by_source(0).unwrap();

        let left = state
            .two_way_split_word_highlight_for_row(0, &row)
            .expect("modified row should have word highlights");
        let right = state
            .two_way_split_word_highlight_for_row(0, &row)
            .expect("second column should reuse word highlights");

        assert!(std::sync::Arc::ptr_eq(&left, &right));
    }

    #[test]
    pub(crate) fn rebuild_three_way_visible_state_streamed_mode() {
        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\nb\n".into(),
            theirs: "c\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        })];
        state.three_way_text.ours = "a\nb\n".into();
        state.three_way_text.theirs = "c\n".into();
        state.three_way_line_starts.ours = vec![0, 2].into();
        state.three_way_line_starts.theirs = vec![0].into();
        state.three_way_len = 2;

        state.rebuild_three_way_visible_state();

        assert!(state.streamed().three_way_visible_projection.len() > 0);
        assert_eq!(
            state.three_way_visible_len(),
            state.streamed().three_way_visible_projection.len()
        );
        assert!(!state.three_way_conflict_ranges.ours.is_empty());
    }

    #[test]
    pub(crate) fn three_way_measure_rows_do_not_materialize_deferred_line_starts() {
        let mut state = ConflictResolverUiState::default();
        let base_text = "ctx\nbase 1234567890\nend\n";
        let ours_text = "ctx\nours abcdefghij\nend\n";
        let theirs_text = "ctx\ntheirs klmnopqrstuv\nend\n";

        state.marker_segments = vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base 1234567890\n".into()),
                ours: "ours abcdefghij\n".into(),
                theirs: "theirs klmnopqrstuv\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
            ConflictSegment::Text("end\n".into()),
        ];
        state.three_way_text = ThreeWaySides {
            base: base_text.into(),
            ours: ours_text.into(),
            theirs: theirs_text.into(),
        };
        state.three_way_line_starts = ThreeWaySides {
            base: DeferredLineStarts::with_line_count(3),
            ours: DeferredLineStarts::with_line_count(3),
            theirs: DeferredLineStarts::with_line_count(3),
        };
        state.three_way_len = 3;

        state.rebuild_three_way_visible_state();

        assert_eq!(
            state.three_way_horizontal_measure_row(ThreeWayColumn::Base),
            1
        );
        assert_eq!(
            state.three_way_horizontal_measure_row(ThreeWayColumn::Ours),
            1
        );
        assert_eq!(
            state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs),
            1
        );
        assert!(
            !state.three_way_line_starts.base.is_materialized(),
            "base line starts should stay deferred when selecting measure rows"
        );
        assert!(
            !state.three_way_line_starts.ours.is_materialized(),
            "ours line starts should stay deferred when selecting measure rows"
        );
        assert!(
            !state.three_way_line_starts.theirs.is_materialized(),
            "theirs line starts should stay deferred when selecting measure rows"
        );
    }

    #[test]
    pub(crate) fn three_way_measure_rows_use_each_stage_coordinates_when_clean_context_diverges() {
        let base = "ctx\nbase conflict\ntail\n";
        let ours = "ctx\nclean ours insertion\nours conflict\ntail\n";
        let long_theirs = "theirs conflict line that must drive the remote column width";
        let theirs = format!("ctx\n{long_theirs}\ntail\n");
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                base,
                ours,
                &theirs,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        );
        let mut state = ConflictResolverUiState {
            // The clean insertion is present in the merge result's context,
            // but not in the base or remote index stages.
            marker_segments: vec![
                ConflictSegment::Text("ctx\nclean ours insertion\n".into()),
                ConflictSegment::Block(ConflictBlock {
                    base: Some("base conflict\n".into()),
                    ours: "ours conflict\n".into(),
                    theirs: format!("{long_theirs}\n").into(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                    whitespace_only: false,
                }),
                ConflictSegment::Text("tail\n".into()),
            ],
            three_way_text: ThreeWaySides {
                base: base.into(),
                ours: ours.into(),
                theirs: theirs.clone().into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: super::deferred_line_starts_for_text(base).into(),
                ours: super::deferred_line_starts_for_text(ours).into(),
                theirs: super::deferred_line_starts_for_text(&theirs).into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };

        state.rebuild_three_way_visible_state();

        let measure_row = state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs);
        assert_eq!(
            state.three_way_row_text(ThreeWayColumn::Theirs, measure_row),
            Some(long_theirs),
            "remote width measurement must select the widest line in stage :3"
        );
    }

    #[test]
    pub(crate) fn hidden_resolved_measure_row_is_not_remapped_as_a_side_line() {
        let base = "head\nb1\nb2\ntail\nbase widest visible line\n";
        let ours = "head\no1\nours insertion\no2\ntail\nours widest visible line\n";
        let long_theirs = "theirs widest visible line after a collapsed conflict";
        let theirs = format!("head\nt1\nt2\ntail\n{long_theirs}\n");
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                base,
                ours,
                &theirs,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        );
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                ConflictSegment::Text("head\n".into()),
                ConflictSegment::Block(ConflictBlock {
                    base: Some("b1\nb2\n".into()),
                    ours: "o1\nours insertion\no2\n".into(),
                    theirs: "t1\nt2\n".into(),
                    choice: ConflictChoice::Ours,
                    resolved: true,
                    whitespace_only: false,
                }),
                ConflictSegment::Text(format!("tail\n{long_theirs}\n").into()),
            ],
            hide_resolved: true,
            three_way_text: ThreeWaySides {
                base: base.into(),
                ours: ours.into(),
                theirs: theirs.clone().into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: super::deferred_line_starts_for_text(base).into(),
                ours: super::deferred_line_starts_for_text(ours).into(),
                theirs: super::deferred_line_starts_for_text(&theirs).into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };

        state.rebuild_three_way_visible_state();

        let measure_visible_ix = state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs);
        let Some(ThreeWayVisibleItem::Line(aligned_row)) =
            state.three_way_visible_item(measure_visible_ix)
        else {
            panic!("remote measure row should be a visible source line");
        };
        assert_eq!(
            state.three_way_row_text(ThreeWayColumn::Theirs, aligned_row),
            Some(long_theirs),
        );
    }

    #[test]
    pub(crate) fn collapsed_context_measure_row_uses_the_compact_visible_index() {
        let prefix = (0..20)
            .map(|ix| format!("context {ix}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let base = format!("{prefix}base conflict\ntail\n");
        let ours = format!("{prefix}ours conflict\ntail\n");
        let long_theirs = "theirs conflict line wide enough to be the measurement row";
        let theirs = format!("{prefix}{long_theirs}\ntail\n");
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                &base,
                &ours,
                &theirs,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        );
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                ConflictSegment::Text(prefix.into()),
                ConflictSegment::Block(ConflictBlock {
                    base: Some("base conflict\n".into()),
                    ours: "ours conflict\n".into(),
                    theirs: format!("{long_theirs}\n").into(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                    whitespace_only: false,
                }),
                ConflictSegment::Text("tail\n".into()),
            ],
            collapse_context: true,
            three_way_text: ThreeWaySides {
                base: base.clone().into(),
                ours: ours.clone().into(),
                theirs: theirs.clone().into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: super::deferred_line_starts_for_text(&base).into(),
                ours: super::deferred_line_starts_for_text(&ours).into(),
                theirs: super::deferred_line_starts_for_text(&theirs).into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };

        state.rebuild_three_way_visible_state();

        let measure_visible_ix = state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs);
        let Some(ThreeWayVisibleItem::Line(aligned_row)) =
            state.three_way_visible_item(measure_visible_ix)
        else {
            panic!("remote measure row should survive context folding");
        };
        assert_eq!(
            state.three_way_row_text(ThreeWayColumn::Theirs, aligned_row),
            Some(long_theirs),
        );
        assert!(
            measure_visible_ix < aligned_row,
            "folded projection should compact the source row index"
        );
    }

    #[test]
    pub(crate) fn streamed_conflict_index_for_side_line_uses_grouped_side_ranges() {
        let mut state = ConflictResolverUiState::default();
        state.three_way_conflict_ranges = ThreeWaySides {
            base: vec![0..1, 4..6],
            ours: vec![2..5, 8..9],
            theirs: vec![1..3, 7..10],
        };

        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Base, 4),
            Some(1)
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Ours, 3),
            Some(0)
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Theirs, 8),
            Some(1)
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Base, 2),
            None
        );
    }

    #[test]
    pub(crate) fn streamed_mode_dispatch_uses_projection() {
        let mut state = ConflictResolverUiState::default();
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\nb\nc\nd\ne\n".into(),
            theirs: "a\nb\nc\nd\ne\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        })];
        let ranges = vec![0..5];
        state.streamed_mut().three_way_visible_projection =
            conflict_resolver::build_three_way_visible_projection(5, &ranges, &segments, false);

        assert_eq!(state.three_way_visible_len(), 5);
        assert_eq!(
            state.three_way_visible_item(2),
            Some(ThreeWayVisibleItem::Line(2))
        );
    }

    pub(crate) fn streamed_state_with_one_conflict() -> ConflictResolverUiState {
        let segments = vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "a\nb\n".into(),
                theirs: "c\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
        ];
        let index = ConflictSplitRowIndex::new(&segments, 3);
        let projection = TwoWaySplitProjection::new(&index, &segments, false);

        let mut state = ConflictResolverUiState::default();
        state.marker_segments = segments;
        state.mode_state = super::ConflictModeState::Streamed(super::StreamedConflictState {
            split_row_index: index,
            two_way_split_projection: projection,
            ..super::StreamedConflictState::default()
        });
        state
    }

    #[test]
    pub(crate) fn two_way_row_counts_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        let (diff_count, inline_count) = streamed.two_way_row_counts();
        assert!(diff_count > 0);
        assert_eq!(inline_count, 0);
    }

    #[test]
    pub(crate) fn two_way_split_conflict_ix_for_visible_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        let vis_len = streamed.two_way_split_visible_len();
        let mut found_conflict = false;
        for ix in 0..vis_len {
            if streamed.two_way_split_conflict_ix_for_visible(ix) == Some(0) {
                found_conflict = true;
                break;
            }
        }
        assert!(found_conflict);
    }

    #[test]
    pub(crate) fn two_way_split_visible_row_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        let visible_ix = streamed
            .two_way_visible_ix_for_conflict(0)
            .expect("streamed visible row should exist for the unresolved conflict");
        let visible_row = streamed
            .two_way_split_visible_row(visible_ix)
            .expect("streamed visible row should resolve through the projection");
        assert_eq!(visible_row.conflict_ix, Some(0));
        assert!(visible_row.row.old.is_some() || visible_row.row.new.is_some());
        assert!(visible_row.source_row_ix < streamed.two_way_row_counts().0);
    }

    #[test]
    pub(crate) fn two_way_split_nav_entries_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        assert_eq!(streamed.two_way_split_nav_entries().len(), 1);
    }

    #[test]
    pub(crate) fn two_way_nav_entries_uses_split_projection() {
        let streamed = streamed_state_with_one_conflict();
        assert_eq!(streamed.two_way_nav_entries().len(), 1);
    }

    #[test]
    pub(crate) fn two_way_conflict_ix_for_visible_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        let vis_len = streamed.two_way_split_visible_len();
        let mut found = false;
        for ix in 0..vis_len {
            if streamed.two_way_conflict_ix_for_visible(ix) == Some(0) {
                found = true;
                break;
            }
        }
        assert!(found);
    }

    #[test]
    pub(crate) fn two_way_visible_ix_for_conflict_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        assert!(streamed.two_way_visible_ix_for_conflict(0).is_some());
        assert_eq!(streamed.two_way_visible_ix_for_conflict(99), None);
    }

    #[test]
    pub(crate) fn default_mode_state_is_streamed() {
        let state = ConflictResolverUiState::default();
        assert!(state.rendering_mode().is_streamed_large_file());
        assert!(state.split_row_index().is_some());
    }

    pub(crate) fn split_ready_state() -> ConflictResolverUiState {
        let base = "ctx\nb1\nb2\nb3\ntail\n";
        let ours = "ctx\no1\no2\no3\ntail\n";
        let theirs = "ctx\nt1\nt2\nt3\ntail\n";
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                base,
                ours,
                theirs,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        );
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                ConflictSegment::Text("ctx\n".into()),
                // Display blocks may have a base populated from the ancestor
                // even though the raw marker block is two-sided.
                ConflictSegment::Block(ConflictBlock {
                    base: Some("b1\nb2\nb3\n".into()),
                    ours: "o1\no2\no3\n".into(),
                    theirs: "t1\nt2\nt3\n".into(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                    whitespace_only: false,
                }),
                ConflictSegment::Text("tail\n".into()),
            ],
            conflict_region_indices: vec![0],
            conflict_region_marker_has_base: vec![false],
            strategy: Some(
                gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver,
            ),
            three_way_text: ThreeWaySides {
                base: base.into(),
                ours: ours.into(),
                theirs: theirs.into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: vec![0, 4, 7, 10, 13].into(),
                ours: vec![0, 4, 7, 10, 13].into(),
                theirs: vec![0, 4, 7, 10, 13].into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };
        state.rebuild_three_way_visible_state();
        assert_eq!(state.three_way_block_aligned_range(0), Some(1..4));
        state
    }

    pub(crate) fn split_ready_state_with_synthetic_base(
        base: &str,
        block_base: &str,
    ) -> ConflictResolverUiState {
        let ours = "ctx\nshared1\nshared2\ntail\n";
        let theirs = ours;
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                base,
                ours,
                theirs,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        );
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                ConflictSegment::Text("ctx\n".into()),
                ConflictSegment::Block(ConflictBlock {
                    // Synthetic display base populated from the ancestor; the
                    // serialized marker remains the ordinary two-marker form.
                    base: Some(block_base.to_string().into()),
                    ours: "shared1\nshared2\n".into(),
                    theirs: "shared1\nshared2\n".into(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                    whitespace_only: false,
                }),
                ConflictSegment::Text("tail\n".into()),
            ],
            conflict_region_indices: vec![0],
            conflict_region_marker_has_base: vec![false],
            strategy: Some(
                gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver,
            ),
            three_way_text: ThreeWaySides {
                base: base.to_string().into(),
                ours: ours.into(),
                theirs: theirs.into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: super::deferred_line_starts_for_text(base).into(),
                ours: super::deferred_line_starts_for_text(ours).into(),
                theirs: super::deferred_line_starts_for_text(theirs).into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };
        state.rebuild_three_way_visible_state();
        state
    }

    #[test]
    pub(crate) fn conflict_row_selection_normalizes_and_clamps_to_its_block() {
        let state = split_ready_state();
        let reverse = ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: 3,
            head_row: 1,
            selecting: true,
        };
        assert_eq!(reverse.row_range(), 1..=3);
        assert_eq!(state.clamp_row_to_conflict_block(0, 0), 1);
        assert_eq!(state.clamp_row_to_conflict_block(0, usize::MAX), 3);
    }

    #[test]
    pub(crate) fn alignment_marks_are_independent_per_column_and_extend_from_their_anchor() {
        let mut state = split_ready_state();
        assert!(state.manual_alignment_enabled());
        assert!(!state.has_alignment_selection());

        state.set_alignment_selection(ThreeWayColumn::Ours, 2, false);
        state.set_alignment_selection(ThreeWayColumn::Theirs, 1, false);
        assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 2));
        assert!(state.alignment_line_is_selected(ThreeWayColumn::Theirs, 1));
        assert!(
            !state.alignment_line_is_selected(ThreeWayColumn::Ours, 1),
            "marking one column must not mark the same line in another"
        );

        // Extending backwards from the anchor normalizes the range.
        state.set_alignment_selection(ThreeWayColumn::Ours, 1, true);
        assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 1));
        assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 2));
        assert!(!state.alignment_line_is_selected(ThreeWayColumn::Ours, 3));

        // Without extend the mark restarts at the clicked line.
        state.set_alignment_selection(ThreeWayColumn::Ours, 3, false);
        assert!(!state.alignment_line_is_selected(ThreeWayColumn::Ours, 1));
        assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 3));

        assert!(state.clear_alignment_selections());
        assert!(!state.has_alignment_selection());
        assert!(!state.clear_alignment_selections());
    }

    #[test]
    pub(crate) fn an_unmarked_column_pins_an_empty_range_at_its_aligned_position() {
        let mut state = split_ready_state();
        state.set_alignment_selection(ThreeWayColumn::Ours, 2, false);
        state.set_alignment_selection(ThreeWayColumn::Theirs, 1, false);

        let entry = state
            .manual_alignment_from_selections(true)
            .expect("two marked columns are enough to pin");
        assert_eq!(entry.local, 2..3);
        assert_eq!(entry.remote, 1..2);
        assert!(
            entry.base.is_empty(),
            "the unmarked base column pins nothing, not a guessed range"
        );
        assert_eq!(
            entry.base.start,
            state
                .three_way_aligned
                .side_line_lower_bound(ThreeWayColumn::Base.side_index(), 1),
            "its empty range still sits where the marked columns start"
        );
    }

    #[test]
    pub(crate) fn a_two_input_pin_leaves_the_base_range_at_the_origin() {
        let mut state = split_ready_state();
        state.set_alignment_selection(ThreeWayColumn::Base, 2, false);
        state.set_alignment_selection(ThreeWayColumn::Ours, 2, false);

        let entry = state
            .manual_alignment_from_selections(false)
            .expect("marked columns are enough to pin");
        assert_eq!(
            entry.base,
            0..0,
            "without a base the plan maps ours/theirs onto A/B, so the base range must stay inert"
        );
        assert_eq!(entry.local, 2..3);
    }

    #[test]
    pub(crate) fn nothing_marked_pins_nothing() {
        let state = split_ready_state();
        assert!(state.manual_alignment_from_selections(true).is_none());
    }

    #[test]
    pub(crate) fn a_conflict_without_aligned_rows_cannot_be_pinned() {
        let mut state = ConflictResolverUiState {
            strategy: Some(
                gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver,
            ),
            ..Default::default()
        };
        assert!(
            !state.manual_alignment_enabled(),
            "the identity map has no shared row space to express a pin in"
        );
        state.set_alignment_selection(ThreeWayColumn::Ours, 0, false);
        assert!(state.manual_alignment_from_selections(true).is_none());
    }

    #[test]
    pub(crate) fn split_boundaries_support_forward_reverse_and_single_row_selections() {
        let mut state = split_ready_state();

        state.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: 1,
            head_row: 2,
            selecting: false,
        });
        let (region_index, forward) = state.split_boundaries_for_selection().expect("forward");
        assert_eq!(region_index, 0);
        assert_eq!(forward.ours, [0, 2]);
        assert_eq!(forward.theirs, [0, 2]);
        assert_eq!(
            forward.base, None,
            "raw two-sided markers need no base cuts"
        );

        state.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: 2,
            head_row: 1,
            selecting: false,
        });
        assert_eq!(
            state.split_boundaries_for_selection().unwrap().1,
            forward,
            "reverse drags normalize to the same boundaries",
        );

        state.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: 3,
            head_row: 3,
            selecting: false,
        });
        let single = state
            .split_boundaries_for_selection()
            .expect("single row")
            .1;
        assert_eq!(single.ours, [2, 3]);
        assert_eq!(single.theirs, [2, 3]);
    }

    #[test]
    pub(crate) fn split_boundaries_use_staged_positions_after_one_sided_clean_context() {
        let base = "ctx\nb1\nb2\ntail\n";
        let ours = "ctx\nours clean insertion\no1\no2\ntail\n";
        let theirs = "ctx\nt1\nt2\ntail\n";
        use gitcomet_core::merge::{AlignedRun, AlignedRunKind};
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(&[
            AlignedRun {
                base: 0..1,
                ours: 0..1,
                theirs: 0..1,
                kind: AlignedRunKind::Unchanged,
            },
            AlignedRun {
                base: 1..1,
                ours: 1..2,
                theirs: 1..1,
                kind: AlignedRunKind::OursChanged,
            },
            AlignedRun {
                base: 1..3,
                ours: 2..4,
                theirs: 1..3,
                kind: AlignedRunKind::Conflict,
            },
            AlignedRun {
                base: 3..4,
                ours: 4..5,
                theirs: 3..4,
                kind: AlignedRunKind::Unchanged,
            },
        ]);
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                ConflictSegment::Text("ctx\nours clean insertion\n".into()),
                ConflictSegment::Block(ConflictBlock {
                    base: Some("b1\nb2\n".into()),
                    ours: "o1\no2\n".into(),
                    theirs: "t1\nt2\n".into(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                    whitespace_only: false,
                }),
                ConflictSegment::Text("tail\n".into()),
            ],
            conflict_region_indices: vec![0],
            conflict_region_marker_has_base: vec![true],
            strategy: Some(
                gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver,
            ),
            three_way_text: ThreeWaySides {
                base: base.into(),
                ours: ours.into(),
                theirs: theirs.into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: super::deferred_line_starts_for_text(base).into(),
                ours: super::deferred_line_starts_for_text(ours).into(),
                theirs: super::deferred_line_starts_for_text(theirs).into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };
        state.rebuild_three_way_visible_state();
        let first_conflict_row = state
            .three_way_block_aligned_range(0)
            .unwrap()
            .find(|&row| {
                state.three_way_row_text(ThreeWayColumn::Base, row) == Some("b1")
                    && state.three_way_row_text(ThreeWayColumn::Ours, row) == Some("o1")
                    && state.three_way_row_text(ThreeWayColumn::Theirs, row) == Some("t1")
            })
            .expect("first conflict row");
        state.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: first_conflict_row,
            head_row: first_conflict_row,
            selecting: false,
        });

        let boundaries = state.split_boundaries_for_selection().unwrap().1;
        assert_eq!(boundaries.base, Some([0, 1]));
        assert_eq!(boundaries.ours, [0, 1]);
        assert_eq!(boundaries.theirs, [0, 1]);
    }

    #[test]
    pub(crate) fn split_boundaries_reject_whole_block_and_ambiguous_region_maps() {
        let mut state = split_ready_state();
        state.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: 1,
            head_row: 3,
            selecting: false,
        });
        assert!(state.split_boundaries_for_selection().is_none());

        state.row_selection.as_mut().unwrap().head_row = 2;
        state.conflict_region_indices = vec![0, 0];
        assert!(state.split_boundaries_for_selection().is_none());

        state.conflict_region_indices.clear();
        assert!(state.split_boundaries_for_selection().is_none());

        state.conflict_region_indices = vec![1];
        assert!(state.split_boundaries_for_selection().is_none());
    }

    #[test]
    pub(crate) fn split_boundaries_reject_synthetic_base_only_and_serialized_whole_block_selections()
     {
        let mut interior = split_ready_state_with_synthetic_base(
            "ctx\nshared1\nbase-only\nshared2\ntail\n",
            "shared1\nbase-only\nshared2\n",
        );
        let interior_range = interior.three_way_block_aligned_range(0).unwrap();
        let interior_padding = interior_range
            .clone()
            .find(|&row| {
                interior
                    .three_way_side_line_for_row(ThreeWayColumn::Base, row)
                    .is_some()
                    && interior
                        .three_way_side_line_for_row(ThreeWayColumn::Ours, row)
                        .is_none()
                    && interior
                        .three_way_side_line_for_row(ThreeWayColumn::Theirs, row)
                        .is_none()
            })
            .expect("base-only aligned row");
        interior.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: interior_padding,
            head_row: interior_padding,
            selecting: false,
        });
        assert!(
            interior.split_boundaries_for_selection().is_none(),
            "a row absent from every serialized marker side cannot become its own conflict",
        );

        let mut edge = split_ready_state_with_synthetic_base(
            "ctx\nbase-only\nshared1\nshared2\ntail\n",
            "base-only\nshared1\nshared2\n",
        );
        let edge_range = edge.three_way_block_aligned_range(0).unwrap();
        let serialized_rows: Vec<usize> = edge_range
            .clone()
            .filter(|&row| {
                edge.three_way_side_line_for_row(ThreeWayColumn::Ours, row)
                    .is_some()
                    || edge
                        .three_way_side_line_for_row(ThreeWayColumn::Theirs, row)
                        .is_some()
            })
            .collect();
        edge.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: *serialized_rows.first().expect("serialized row"),
            head_row: *serialized_rows.last().expect("serialized row"),
            selecting: false,
        });
        assert!(
            edge.split_boundaries_for_selection().is_none(),
            "selecting every serialized line remains a degenerate whole-block split",
        );
    }

    #[test]
    pub(crate) fn joinable_context_rejects_marker_looking_text_between_blocks() {
        let block = || {
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "ours\n".into(),
                theirs: "theirs\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            })
        };
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                block(),
                ConflictSegment::Text("clean context\n".into()),
                block(),
            ],
            ..Default::default()
        };
        assert!(state.conflict_blocks_have_joinable_context(0, 1));
        state.marker_segments[1] = ConflictSegment::Text("<<<<<<< malformed\n".into());
        assert!(!state.conflict_blocks_have_joinable_context(0, 1));
        assert!(!state.conflict_blocks_have_joinable_context(0, 2));
    }

    #[test]
    pub(crate) fn semantic_selection_retains_automatic_target_when_no_marker_block_exists() {
        let automatic_id = ConflictNavTargetId::PlanBlock(gitcomet_core::merge::MergeBlockId {
            fingerprint: 1,
            occurrence: 0,
        });
        let conflict_id = ConflictNavTargetId::PlanBlock(gitcomet_core::merge::MergeBlockId {
            fingerprint: 2,
            occurrence: 0,
        });
        let mut state = ConflictResolverUiState {
            conflict_region_indices: vec![0],
            nav_targets: vec![
                ConflictNavTarget {
                    id: automatic_id,
                    order: 0,
                    aligned_rows: Some(1..2),
                    region_index: None,
                    display_conflict_index: None,
                    is_delta: true,
                    original_conflict: false,
                    unresolved: false,
                },
                ConflictNavTarget {
                    id: conflict_id,
                    order: 1,
                    aligned_rows: Some(3..4),
                    region_index: Some(0),
                    display_conflict_index: Some(0),
                    is_delta: true,
                    original_conflict: true,
                    unresolved: true,
                },
            ],
            ..Default::default()
        };

        assert!(state.select_nav_target(0));
        assert_eq!(state.nav_anchor.unwrap().id, automatic_id);
        assert_eq!(state.selected_nav_target_index(), Some(0));
        assert_eq!(state.active_conflict, None);

        assert!(state.select_display_conflict(0));
        assert_eq!(state.nav_anchor.unwrap().id, conflict_id);
        assert_eq!(state.active_conflict, Some(0));
    }

    #[test]
    pub(crate) fn exact_provenance_projects_target_rows_after_output_line_shifts() {
        let target = ConflictNavTarget {
            id: ConflictNavTargetId::DisplayBlock(0),
            order: 0,
            aligned_rows: Some(2..4),
            region_index: None,
            display_conflict_index: None,
            is_delta: true,
            original_conflict: false,
            unresolved: false,
        };
        let anchor = target.anchor();
        let mut state = ConflictResolverUiState {
            view_mode: ConflictResolverViewMode::ThreeWay,
            resolved_outline: super::ResolvedOutlineData {
                meta: vec![ResolvedLineMeta {
                    output_line: 5,
                    source: ResolvedLineSource::B,
                    input_line: Some(3),
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(
            state.output_line_for_nav_target_provenance(&target),
            Some(5)
        );
        state.resolved_outline.meta[0].output_line = 11;
        assert_eq!(
            state.output_line_for_nav_target_provenance(&target),
            Some(11),
            "surrounding output insertions shift only the projection"
        );
        assert_eq!(target.anchor(), anchor, "the semantic anchor is unchanged");
    }

    #[test]
    pub(crate) fn deletion_and_untraceable_manual_output_have_no_output_projection() {
        let deletion = ConflictNavTarget {
            id: ConflictNavTargetId::DisplayBlock(0),
            order: 0,
            aligned_rows: Some(8..9),
            region_index: None,
            display_conflict_index: None,
            is_delta: true,
            original_conflict: false,
            unresolved: false,
        };
        let state = ConflictResolverUiState {
            resolved_outline: super::ResolvedOutlineData {
                meta: vec![ResolvedLineMeta {
                    output_line: 3,
                    source: ResolvedLineSource::Manual,
                    input_line: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(state.output_line_for_nav_target_provenance(&deletion), None);
    }
}
