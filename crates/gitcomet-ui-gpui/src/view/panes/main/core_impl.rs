use super::helpers::*;
use super::*;
use crate::kit::text_model::TextModelSnapshot;
use crate::view::branch_sidebar::BranchSection;
use gitcomet_core::domain::LogScope;
use gitcomet_core::mergetool_trace::{
    self, MergetoolTraceEvent, MergetoolTraceSideStats, MergetoolTraceStage,
};
use std::sync::Arc;
use std::time::Instant;

fn line_ranges_intersect(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

pub(in crate::view::panes::main) fn resolved_output_highlight_provider_binding_key(
    theme_epoch: u64,
    language: rows::DiffSyntaxLanguage,
    document: rows::PreparedDiffSyntaxDocument,
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = rustc_hash::FxHasher::default();
    theme_epoch.hash(&mut hasher);
    language.hash(&mut hasher);
    document.hash(&mut hasher);
    hasher.finish()
}

fn shift_resolved_output_marker(
    marker: ResolvedOutputConflictMarker,
    line_delta: isize,
) -> ResolvedOutputConflictMarker {
    ResolvedOutputConflictMarker {
        conflict_ix: marker.conflict_ix,
        range_start: shifted_line_index(marker.range_start, line_delta),
        range_end: shifted_line_index(marker.range_end, line_delta),
        is_start: marker.is_start,
        is_end: marker.is_end,
        unresolved: marker.unresolved,
    }
}

fn diff_syntax_edit_from_outline_delta(delta: ResolvedOutlineDelta) -> rows::DiffSyntaxEdit {
    rows::DiffSyntaxEdit {
        old_range: delta.old_range,
        new_range: delta.new_range,
    }
}

fn record_resolved_outline_trace(
    path: Option<&std::path::PathBuf>,
    started: Instant,
    pane: &MainPaneView,
    output_line_count: usize,
) {
    let path = path.cloned();
    let elapsed = started.elapsed();
    let (diff_row_count, inline_row_count) = pane.conflict_resolver.two_way_row_counts();
    mergetool_trace::record_with(|| {
        MergetoolTraceEvent::new(MergetoolTraceStage::ResolvedOutlineRecompute, path, elapsed)
            .with_base(MergetoolTraceSideStats::from_text(Some(
                pane.conflict_resolver.three_way_text.base.as_ref(),
            )))
            .with_ours(MergetoolTraceSideStats::from_text(Some(
                pane.conflict_resolver.three_way_text.ours.as_ref(),
            )))
            .with_theirs(MergetoolTraceSideStats::from_text(Some(
                pane.conflict_resolver.three_way_text.theirs.as_ref(),
            )))
            .with_conflict_block_count(Some(conflict_resolver::conflict_count(
                &pane.conflict_resolver.marker_segments,
            )))
            .with_diff_row_count(Some(diff_row_count))
            .with_inline_row_count(Some(inline_row_count))
            .with_resolved_output_line_count(Some(output_line_count))
    });
}

struct ResolvedOutlineComputation {
    output_line_count: usize,
    outline: ResolvedOutlineData,
}

enum ResolvedOutlineSourceView<'a> {
    ThreeWay {
        base_text: &'a str,
        base_line_starts: &'a [usize],
        ours_text: &'a str,
        ours_line_starts: &'a [usize],
        theirs_text: &'a str,
        theirs_line_starts: &'a [usize],
    },
    TwoWay {
        ours_text: &'a str,
        ours_line_starts: &'a [usize],
        theirs_text: &'a str,
        theirs_line_starts: &'a [usize],
    },
}

impl ResolvedOutlineSourceView<'_> {
    fn view_mode(&self) -> ConflictResolverViewMode {
        match self {
            Self::ThreeWay { .. } => ConflictResolverViewMode::ThreeWay,
            Self::TwoWay { .. } => ConflictResolverViewMode::TwoWayDiff,
        }
    }
}

#[derive(Clone)]
enum OwnedResolvedOutlineSourceData {
    ThreeWay {
        base_text: Arc<str>,
        base_line_starts: Arc<[usize]>,
        ours_text: Arc<str>,
        ours_line_starts: Arc<[usize]>,
        theirs_text: Arc<str>,
        theirs_line_starts: Arc<[usize]>,
    },
    TwoWay {
        ours_text: Arc<str>,
        ours_line_starts: Arc<[usize]>,
        theirs_text: Arc<str>,
        theirs_line_starts: Arc<[usize]>,
    },
}

impl OwnedResolvedOutlineSourceData {
    fn as_view(&self) -> ResolvedOutlineSourceView<'_> {
        match self {
            Self::ThreeWay {
                base_text,
                base_line_starts,
                ours_text,
                ours_line_starts,
                theirs_text,
                theirs_line_starts,
            } => ResolvedOutlineSourceView::ThreeWay {
                base_text,
                base_line_starts,
                ours_text,
                ours_line_starts,
                theirs_text,
                theirs_line_starts,
            },
            Self::TwoWay {
                ours_text,
                ours_line_starts,
                theirs_text,
                theirs_line_starts,
            } => ResolvedOutlineSourceView::TwoWay {
                ours_text,
                ours_line_starts,
                theirs_text,
                theirs_line_starts,
            },
        }
    }
}

#[derive(Clone)]
struct BackgroundResolvedOutlineRecomputeRequest {
    output_text: Arc<str>,
    output_line_count: usize,
    marker_segments: Vec<conflict_resolver::ConflictSegment>,
    sources: OwnedResolvedOutlineSourceData,
}

struct ResolvedOutlineIncrementalBase<'a> {
    text: &'a TextModelSnapshot,
    line_starts: &'a Arc<[usize]>,
    marker_segments: &'a [conflict_resolver::ConflictSegment],
    view_mode: ConflictResolverViewMode,
}

fn compute_resolved_outline_computation(
    output_text: &str,
    output_line_count: usize,
    marker_segments: &[conflict_resolver::ConflictSegment],
    sources: ResolvedOutlineSourceView<'_>,
) -> ResolvedOutlineComputation {
    let view_mode = sources.view_mode();
    let markers =
        build_resolved_output_conflict_markers(marker_segments, output_text, output_line_count);
    if should_skip_resolved_outline_provenance(view_mode, output_line_count) {
        return ResolvedOutlineComputation {
            output_line_count,
            outline: ResolvedOutlineData {
                meta: Vec::new(),
                markers,
                sources_index: HashSet::default(),
            },
        };
    }

    let mut meta = match sources {
        ResolvedOutlineSourceView::ThreeWay {
            base_text,
            base_line_starts,
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        } => conflict_resolver::compute_resolved_line_provenance_from_text_with_indexed_sources(
            output_text,
            base_text,
            base_line_starts,
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        ),
        ResolvedOutlineSourceView::TwoWay {
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        } => conflict_resolver::compute_resolved_line_provenance_from_text_two_way_indexed_sources(
            output_text,
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        ),
    };
    apply_conflict_choice_provenance_hints(&mut meta, marker_segments, output_text, view_mode);
    let sources_index = conflict_resolver::build_resolved_output_line_sources_index_from_text(
        &meta,
        output_text,
        view_mode,
    );

    ResolvedOutlineComputation {
        output_line_count,
        outline: ResolvedOutlineData {
            meta,
            markers,
            sources_index,
        },
    }
}

fn compute_resolved_outline_computation_from_projection(
    projection: &conflict_resolver::ResolvedOutputProjection,
    marker_segments: &[conflict_resolver::ConflictSegment],
    view_mode: ConflictResolverViewMode,
    sources: Option<ResolvedOutlineSourceView<'_>>,
) -> ResolvedOutlineComputation {
    let output_line_count = projection.len();
    let block_ranges = projection.conflict_line_ranges();
    let markers = build_resolved_output_conflict_markers_from_block_ranges(
        marker_segments,
        block_ranges,
        output_line_count,
    );
    if should_skip_resolved_outline_provenance(view_mode, output_line_count) {
        return ResolvedOutlineComputation {
            output_line_count,
            outline: ResolvedOutlineData {
                meta: Vec::new(),
                markers,
                sources_index: HashSet::default(),
            },
        };
    }

    let Some(sources) = sources else {
        return ResolvedOutlineComputation {
            output_line_count,
            outline: ResolvedOutlineData {
                meta: Vec::new(),
                markers,
                sources_index: HashSet::default(),
            },
        };
    };
    let mut source_lookup: HashMap<&str, (conflict_resolver::ResolvedLineSource, Option<u32>)> =
        HashMap::default();
    match sources {
        ResolvedOutlineSourceView::ThreeWay {
            base_text,
            base_line_starts,
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        } => {
            insert_lookup_from_indexed_text(
                &mut source_lookup,
                conflict_resolver::ResolvedLineSource::C,
                theirs_text,
                theirs_line_starts,
            );
            insert_lookup_from_indexed_text(
                &mut source_lookup,
                conflict_resolver::ResolvedLineSource::B,
                ours_text,
                ours_line_starts,
            );
            insert_lookup_from_indexed_text(
                &mut source_lookup,
                conflict_resolver::ResolvedLineSource::A,
                base_text,
                base_line_starts,
            );
        }
        ResolvedOutlineSourceView::TwoWay {
            ours_text,
            ours_line_starts,
            theirs_text,
            theirs_line_starts,
        } => {
            insert_lookup_from_indexed_text(
                &mut source_lookup,
                conflict_resolver::ResolvedLineSource::B,
                theirs_text,
                theirs_line_starts,
            );
            insert_lookup_from_indexed_text(
                &mut source_lookup,
                conflict_resolver::ResolvedLineSource::A,
                ours_text,
                ours_line_starts,
            );
        }
    }

    let mut meta = Vec::with_capacity(output_line_count);
    for line_ix in 0..output_line_count {
        let line = projection
            .line_text(marker_segments, line_ix)
            .unwrap_or(std::borrow::Cow::Borrowed(""));
        let (source, input_line) = source_lookup
            .get(line.as_ref())
            .copied()
            .unwrap_or((conflict_resolver::ResolvedLineSource::Manual, None));
        meta.push(conflict_resolver::ResolvedLineMeta {
            output_line: u32::try_from(line_ix).unwrap_or(u32::MAX),
            source,
            input_line,
        });
    }
    apply_conflict_choice_provenance_hints_for_ranges(
        &mut meta,
        marker_segments,
        block_ranges,
        view_mode,
    );

    let mut sources_index = HashSet::default();
    sources_index.reserve(meta.len());
    for (line_ix, line_meta) in meta.iter().enumerate() {
        if line_meta.source == conflict_resolver::ResolvedLineSource::Manual {
            continue;
        }
        let Some(line_no) = line_meta.input_line else {
            continue;
        };
        let Some(line) = projection.line_text(marker_segments, line_ix) else {
            continue;
        };
        sources_index.insert(conflict_resolver::SourceLineKey::new(
            view_mode,
            line_meta.source,
            line_no,
            line.as_ref(),
        ));
    }

    ResolvedOutlineComputation {
        output_line_count,
        outline: ResolvedOutlineData {
            meta,
            markers,
            sources_index,
        },
    }
}

fn insert_lookup_from_indexed_text<'a>(
    lookup: &mut HashMap<&'a str, (conflict_resolver::ResolvedLineSource, Option<u32>)>,
    source: conflict_resolver::ResolvedLineSource,
    text: &'a str,
    line_starts: &[usize],
) {
    let line_count = indexed_line_count(text, line_starts);
    for line_ix in (0..line_count).rev() {
        let line = rows::resolved_output_line_text(text, line_starts, line_ix);
        lookup.insert(
            line,
            (
                source,
                Some(u32::try_from(line_ix.saturating_add(1)).unwrap_or(u32::MAX)),
            ),
        );
    }
}

fn update_line_sources_index_for_range(
    index: &mut HashSet<conflict_resolver::SourceLineKey>,
    view_mode: ConflictResolverViewMode,
    meta: &[conflict_resolver::ResolvedLineMeta],
    text: &str,
    line_starts: &[usize],
    line_range: Range<usize>,
    insert: bool,
) {
    if line_range.start >= line_range.end {
        return;
    }
    for line_ix in line_range {
        let Some(line_meta) = meta.get(line_ix) else {
            break;
        };
        if line_meta.source == conflict_resolver::ResolvedLineSource::Manual {
            continue;
        }
        let Some(line_no) = line_meta.input_line else {
            continue;
        };
        let key = conflict_resolver::SourceLineKey::new(
            view_mode,
            line_meta.source,
            line_no,
            rows::resolved_output_line_text(text, line_starts, line_ix),
        );
        if insert {
            index.insert(key);
        } else {
            index.remove(&key);
        }
    }
}

fn widest_resolved_output_line_ix(text: &str, line_starts: &[usize]) -> usize {
    let mut best_ix = 0usize;
    let mut best_len = 0usize;
    let line_count = line_starts.len().max(1);
    for line_ix in 0..line_count {
        let width = rows::resolved_output_line_text(text, line_starts, line_ix).len();
        if width > best_len {
            best_len = width;
            best_ix = line_ix;
        }
    }
    best_ix
}

fn preferred_scroll_master_index<const N: usize>(max_scrolls: [Pixels; N]) -> usize {
    let mut preferred_ix = 0usize;
    for ix in 1..N {
        if max_scrolls[ix] > max_scrolls[preferred_ix] {
            preferred_ix = ix;
        }
    }
    preferred_ix
}

fn clamp_raw_scroll_y(raw_y: Pixels, max_scroll: Pixels) -> Pixels {
    let max_scroll = max_scroll.max(px(0.0));
    let magnitude = if raw_y < px(0.0) { -raw_y } else { raw_y };
    let clamped = magnitude.min(max_scroll);
    if raw_y < px(0.0) { -clamped } else { clamped }
}

fn compute_synced_scroll_offsets<const N: usize>(
    offsets: [Pixels; N],
    max_scrolls: [Pixels; N],
    last_synced: [Pixels; N],
    preferred_ix: usize,
) -> [Pixels; N] {
    if N == 0 {
        return offsets;
    }
    if offsets.iter().all(|offset| *offset == offsets[0]) {
        return offsets;
    }

    let preferred_ix = preferred_ix.min(N.saturating_sub(1));
    let mut changed_count = 0usize;
    let mut sole_changed_ix = preferred_ix;
    let mut preferred_changed = false;
    let mut largest_changed_ix = preferred_ix;

    for ix in 0..N {
        if offsets[ix] == last_synced[ix] {
            continue;
        }

        if changed_count == 0 || max_scrolls[ix] > max_scrolls[largest_changed_ix] {
            largest_changed_ix = ix;
        }
        if ix == preferred_ix {
            preferred_changed = true;
        }
        sole_changed_ix = ix;
        changed_count += 1;
    }

    let master_ix = match changed_count {
        0 => preferred_ix,
        1 => sole_changed_ix,
        _ if preferred_changed => preferred_ix,
        _ => largest_changed_ix,
    };
    let master_y = offsets[master_ix];

    std::array::from_fn(|ix| clamp_raw_scroll_y(master_y, max_scrolls[ix]))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncedScrollAxis {
    Horizontal,
    Vertical,
}

impl SyncedScrollAxis {
    const fn includes(self, mode: DiffScrollSync) -> bool {
        match self {
            Self::Horizontal => mode.includes_horizontal(),
            Self::Vertical => mode.includes_vertical(),
        }
    }

    const fn offset_component(self, offset: Point<Pixels>) -> Pixels {
        match self {
            Self::Horizontal => offset.x,
            Self::Vertical => offset.y,
        }
    }

    const fn max_scroll_component(self, max_offset: Size<Pixels>) -> Pixels {
        match self {
            Self::Horizontal => max_offset.width,
            Self::Vertical => max_offset.height,
        }
    }

    fn with_offset_component(self, offset: Point<Pixels>, value: Pixels) -> Point<Pixels> {
        match self {
            Self::Horizontal => point(value, offset.y),
            Self::Vertical => point(offset.x, value),
        }
    }
}

fn uniform_list_base_handle(handle: &UniformListScrollHandle) -> ScrollHandle {
    handle.0.borrow().base_handle.clone()
}

fn snapshot_synced_scroll_offsets<const N: usize>(
    handles: &[ScrollHandle; N],
    axis: SyncedScrollAxis,
) -> [Pixels; N] {
    std::array::from_fn(|ix| axis.offset_component(handles[ix].offset()))
}

fn sync_synced_scroll_offsets<const N: usize>(
    handles: &[ScrollHandle; N],
    last_synced: &mut [Pixels; N],
    axis: SyncedScrollAxis,
) {
    let offsets: [Point<Pixels>; N] = std::array::from_fn(|ix| handles[ix].offset());
    let max_scrolls = std::array::from_fn(|ix| {
        axis.max_scroll_component(handles[ix].max_offset().into())
            .max(px(0.0))
    });
    let targets = compute_synced_scroll_offsets(
        std::array::from_fn(|ix| axis.offset_component(offsets[ix])),
        max_scrolls,
        *last_synced,
        preferred_scroll_master_index(max_scrolls),
    );

    for ix in 0..N {
        if axis.offset_component(offsets[ix]) != targets[ix] {
            handles[ix].set_offset(axis.with_offset_component(offsets[ix], targets[ix]));
        }
    }
    *last_synced = targets;
}

fn maybe_sync_synced_scroll_offsets<const N: usize>(
    handles: &[ScrollHandle; N],
    last_synced: &mut [Pixels; N],
    axis: SyncedScrollAxis,
    mode: DiffScrollSync,
) {
    if axis.includes(mode) {
        sync_synced_scroll_offsets(handles, last_synced, axis);
    } else {
        *last_synced = snapshot_synced_scroll_offsets(handles, axis);
    }
}

impl MainPaneView {
    pub(super) fn notify_fingerprint_for(state: &AppState) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = rustc_hash::FxHasher::default();
        state.active_repo.hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            match repo.diff_state.diff_target.as_ref() {
                Some(DiffTarget::WorkingTree { path, area }) => {
                    0u8.hash(&mut hasher);
                    path.hash(&mut hasher);
                    match area {
                        DiffArea::Staged => 0u8.hash(&mut hasher),
                        DiffArea::Unstaged => 1u8.hash(&mut hasher),
                    }
                }
                Some(DiffTarget::Commit { commit_id, path }) => {
                    1u8.hash(&mut hasher);
                    commit_id.hash(&mut hasher);
                    path.hash(&mut hasher);
                }
                None => {
                    2u8.hash(&mut hasher);
                }
            }
            repo.diff_state.diff_state_rev.hash(&mut hasher);
            repo.conflict_state.conflict_rev.hash(&mut hasher);

            // Only include status changes when viewing a working tree diff.
            let status_rev = if matches!(
                repo.diff_state.diff_target,
                Some(DiffTarget::WorkingTree { .. })
            ) {
                repo.status_rev
            } else {
                0
            };
            status_rev.hash(&mut hasher);
            let commit_details_rev = if matches!(
                repo.diff_state.diff_target,
                Some(DiffTarget::Commit { path: Some(_), .. })
            ) {
                repo.history_state.commit_details_rev
            } else {
                0
            };
            commit_details_rev.hash(&mut hasher);
        }

        hasher.finish()
    }

    pub(in crate::view) fn clear_diff_selection_or_exit(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        match clear_diff_selection_action(self.view_mode) {
            ClearDiffSelectionAction::ClearSelection => {
                self.store.dispatch(Msg::ClearDiffSelection { repo_id });
            }
            ClearDiffSelectionAction::ExitFocusedMergetool => {
                self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_CANCELED);
                cx.quit();
            }
        }
    }

    pub(in crate::view) fn reveal_history_commit(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        desired_scope: LogScope,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(
            clear_diff_selection_action(self.view_mode),
            ClearDiffSelectionAction::ExitFocusedMergetool
        ) {
            self.clear_diff_selection_or_exit(repo_id, cx);
            return;
        }

        self.clear_diff_selection_or_exit(repo_id, cx);
        self.history_view.update(cx, |view, cx| {
            view.request_reveal_commit(repo_id, commit_id, desired_scope, cx);
        });
        cx.notify();
    }

    pub(in crate::view) fn reveal_history_branch_commit(
        &mut self,
        repo_id: RepoId,
        section: BranchSection,
        branch_name: &str,
        commit_id: CommitId,
        desired_scope: LogScope,
        cx: &mut gpui::Context<Self>,
    ) {
        let branch_name = branch_name.to_string();
        self.history_view.update(cx, |view, cx| {
            view.set_selected_branch(repo_id, section, &branch_name, cx);
        });
        self.reveal_history_commit(repo_id, commit_id, desired_scope, cx);
    }

    pub(super) fn set_focused_mergetool_exit_code(&self, code: i32) {
        if let Some(exit_code) = &self.focused_mergetool_exit_code {
            exit_code.store(code, Ordering::SeqCst);
        }
    }

    pub(super) fn focused_mergetool_labels_or_default(&self) -> FocusedMergetoolLabels {
        self.focused_mergetool_labels
            .clone()
            .unwrap_or(FocusedMergetoolLabels {
                local: "LOCAL".to_string(),
                remote: "REMOTE".to_string(),
                base: "BASE".to_string(),
            })
    }

    pub(in crate::view) fn focused_mergetool_save_and_exit(
        &mut self,
        repo_id: RepoId,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        use gitcomet_core::conflict_output::ConflictMarkerLabels;

        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_ERROR);
            cx.quit();
            return;
        };

        let labels = self.focused_mergetool_labels_or_default();
        let materialized_output = (!self.conflict_resolved_output_is_streamed()).then(|| {
            self.conflict_resolver_input
                .read_with(cx, |input, _| input.text().to_string())
        });
        let save_payload = build_focused_mergetool_save_payload(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            materialized_output.as_deref(),
            ConflictMarkerLabels {
                local: labels.local.as_str(),
                remote: labels.remote.as_str(),
                base: labels.base.as_str(),
            },
        );
        let output = save_payload.output;

        let full_path = repo.spec.workdir.join(&path);
        if let Some(parent) = full_path.parent().filter(|p| !p.as_os_str().is_empty())
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            eprintln!(
                "Failed to create parent directory for {}: {err}",
                full_path.display()
            );
            self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_ERROR);
            cx.quit();
            return;
        }

        if let Err(err) = std::fs::write(&full_path, output.as_bytes()) {
            eprintln!(
                "Failed to write merged output to {}: {err}",
                full_path.display()
            );
            self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_ERROR);
            cx.quit();
            return;
        }

        let exit_code = focused_mergetool_save_exit_code(
            save_payload.total_conflicts,
            save_payload.resolved_conflicts,
        );
        self.set_focused_mergetool_exit_code(exit_code);
        cx.quit();
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        show_timezone: bool,
        diff_scroll_sync: DiffScrollSync,
        history_show_author: bool,
        history_show_date: bool,
        history_show_sha: bool,
        view_mode: GitCometViewMode,
        focused_mergetool_labels: Option<FocusedMergetoolLabels>,
        focused_mergetool_exit_code: Option<Arc<AtomicI32>>,
        root_view: WeakEntity<GitCometView>,
        tooltip_host: WeakEntity<TooltipHost>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let initial_fingerprint = Self::notify_fingerprint_for(&state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint_for(&next);
            if next_fingerprint == this.notify_fingerprint {
                this.state = next;
                return;
            }

            this.notify_fingerprint = next_fingerprint;
            this.apply_state_snapshot(next, cx);
            cx.notify();
        });

        let diff_raw_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "".into(),
                    multiline: true,
                    read_only: true,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let conflict_resolver_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Resolve file contents…".into(),
                    multiline: true,
                    read_only: false,
                    chromeless: true,
                    soft_wrap: false,
                },
                window,
                cx,
            );
            input.set_suppress_right_click(true);
            input.set_line_height(Some(px(20.0)), cx);
            input
        });

        let conflict_resolver_subscription =
            cx.observe(&conflict_resolver_input, |this, input, cx| {
                let (output_snapshot, edit_delta) = input.update(cx, |input, _| {
                    (input.text_snapshot(), input.take_recent_utf8_edit_delta())
                });
                let output_hash = hash_text_bytes(output_snapshot.as_ref());
                let outline_delta = resolved_outline_delta_for_snapshot_transition(
                    &this.conflict_resolved_preview_text,
                    &output_snapshot,
                    edit_delta,
                );

                let path = this.conflict_resolver.path.clone();
                let needs_update = this.conflict_resolved_preview_path.as_ref() != path.as_ref()
                    || this.conflict_resolved_preview_source_hash != Some(output_hash);
                if !needs_update {
                    return;
                }

                this.conflict_resolved_preview_path = path.clone();
                this.conflict_resolved_preview_source_hash = Some(output_hash);
                this.schedule_conflict_resolved_outline_recompute(
                    path,
                    output_hash,
                    outline_delta,
                    cx,
                );
            });

        let diff_search_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Search diff".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });
        let diff_search_subscription = cx.observe(&diff_search_input, |this, input, cx| {
            if input.update(cx, |input, _| input.take_enter_pressed()) {
                if this.diff_search_active {
                    this.diff_search_next_match();
                    cx.notify();
                }
                return;
            }
            let next: SharedString = input.read(cx).text().to_string().into();
            if this.diff_search_query != next {
                let previous_query = this.diff_search_query.clone();
                this.diff_search_query = next.clone();
                this.invalidate_diff_text_query_overlay_cache(next.as_ref());
                this.clear_worktree_preview_segments_cache();
                this.clear_conflict_diff_query_overlay_caches();
                this.diff_search_recompute_matches_for_query_change(previous_query.as_ref());
                cx.notify();
            }
        });

        let diff_panel_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);

        let last_window_size = window.viewport_size();
        let history_view = cx.new(|cx| {
            super::HistoryView::new(
                Arc::clone(&store),
                ui_model.clone(),
                theme,
                date_time_format,
                timezone,
                show_timezone,
                history_show_author,
                history_show_date,
                history_show_sha,
                root_view.clone(),
                tooltip_host.clone(),
                last_window_size,
                window,
                cx,
            )
        });

        let mut pane = Self {
            store,
            state,
            view_mode,
            focused_mergetool_labels,
            focused_mergetool_exit_code,
            theme,
            date_time_format,
            _ui_model_subscription: subscription,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            active_context_menu_invoker: None,
            last_window_size: size(px(0.0), px(0.0)),
            layout_sidebar_render_width: px(280.0),
            layout_details_render_width: px(420.0),
            layout_sidebar_collapsed: false,
            layout_details_collapsed: false,
            show_whitespace: false,
            diff_view: DiffViewMode::Split,
            rendered_preview_modes: RenderedPreviewModes::default(),
            diff_word_wrap: false,
            diff_scroll_sync,
            diff_split_ratio: 0.5,
            diff_split_resize: None,
            diff_split_last_synced_x: [px(0.0); 2],
            diff_split_last_synced_y: [px(0.0); 2],
            diff_horizontal_min_width: px(0.0),
            diff_cache_repo_id: None,
            diff_cache_rev: 0,
            diff_cache_target: None,
            diff_cache: Vec::new(),
            diff_row_provider: None,
            diff_split_row_provider: None,
            diff_file_for_src_ix: Vec::new(),
            diff_language_for_src_ix: Vec::new(),
            diff_yaml_block_scalar_for_src_ix: Vec::new(),
            diff_click_kinds: Vec::new(),
            diff_line_kind_for_src_ix: Vec::new(),
            diff_hide_unified_header_for_src_ix: Vec::new(),
            diff_header_display_cache: HashMap::default(),
            diff_split_cache: Vec::new(),
            diff_split_cache_len: 0,
            diff_panel_focus_handle,
            diff_autoscroll_pending: false,
            diff_raw_input,
            diff_visible_indices: Vec::new(),
            diff_visible_inline_map: None,
            diff_visible_cache_len: 0,
            diff_visible_view: DiffViewMode::Split,
            diff_visible_is_file_view: false,
            diff_scrollbar_markers_cache: Vec::new(),
            diff_word_highlights: Vec::new(),
            diff_word_highlights_inflight: None,
            diff_file_stats: Vec::new(),
            diff_text_segments_cache: Vec::new(),
            diff_text_query_segments_cache: Vec::new(),
            diff_text_query_cache_query: SharedString::default(),
            diff_text_query_cache_generation: 0,
            diff_selection_anchor: None,
            diff_selection_range: None,
            diff_text_selecting: false,
            diff_text_anchor: None,
            diff_text_head: None,
            diff_text_autoscroll_seq: 0,
            diff_text_autoscroll_target: None,
            diff_text_last_mouse_pos: point(px(0.0), px(0.0)),
            diff_suppress_clicks_remaining: 0,
            diff_text_hitboxes: HashMap::default(),
            diff_text_layout_cache_epoch: 0,
            diff_text_layout_cache: HashMap::default(),
            diff_hunk_picker_search_input: None,
            diff_search_active: false,
            diff_search_query: "".into(),
            diff_search_matches: Vec::new(),
            diff_search_inline_patch_trigram_index: None,
            diff_search_match_ix: None,
            diff_search_input,
            _diff_search_subscription: diff_search_subscription,
            file_diff_cache_repo_id: None,
            file_diff_cache_rev: 0,
            file_diff_cache_content_signature: None,
            file_diff_cache_target: None,
            file_diff_cache_path: None,
            file_diff_cache_language: None,
            file_diff_cache_rows: Vec::new(),
            file_diff_row_provider: None,
            file_diff_old_text: SharedString::default(),
            file_diff_old_line_starts: Arc::default(),
            file_diff_new_text: SharedString::default(),
            file_diff_new_line_starts: Arc::default(),
            file_diff_inline_cache: Vec::new(),
            file_diff_inline_row_provider: None,
            file_diff_inline_text: SharedString::default(),
            file_diff_inline_word_highlights: Vec::new(),
            file_diff_split_word_highlights_old: Vec::new(),
            file_diff_split_word_highlights_new: Vec::new(),
            file_diff_cache_seq: 0,
            file_diff_cache_inflight: None,
            file_diff_syntax_generation: 0,
            file_diff_style_cache_epochs: FileDiffStyleCacheEpochs::default(),
            syntax_chunk_poll_task: None,
            prepared_syntax_documents: HashMap::default(),
            #[cfg(test)]
            diff_syntax_budget_override: None,
            file_markdown_preview_cache_repo_id: None,
            file_markdown_preview_cache_rev: 0,
            file_markdown_preview_cache_content_signature: None,
            file_markdown_preview_cache_target: None,
            file_markdown_preview: Loadable::NotLoaded,
            file_markdown_preview_seq: 0,
            file_markdown_preview_inflight: None,
            file_image_diff_cache_repo_id: None,
            file_image_diff_cache_rev: 0,
            file_image_diff_cache_content_signature: None,
            file_image_diff_cache_target: None,
            file_image_diff_cache_seq: 0,
            file_image_diff_cache_inflight: None,
            file_image_diff_cache_path: None,
            file_image_diff_cache_old: None,
            file_image_diff_cache_new: None,
            file_image_diff_cache_old_svg_path: None,
            file_image_diff_cache_new_svg_path: None,
            worktree_preview_path: None,
            worktree_preview_source_path: None,
            worktree_preview: Loadable::NotLoaded,
            worktree_preview_source_len: 0,
            worktree_preview_text: SharedString::default(),
            worktree_preview_line_starts: Arc::default(),
            worktree_preview_line_flags: Arc::default(),
            worktree_preview_search_trigram_index: None,
            worktree_preview_content_rev: 0,
            worktree_markdown_preview_path: None,
            worktree_markdown_preview_source_rev: 0,
            worktree_markdown_preview: Loadable::NotLoaded,
            worktree_markdown_preview_seq: 0,
            worktree_markdown_preview_inflight: None,
            worktree_preview_segments_cache_path: None,
            worktree_preview_syntax_language: None,
            worktree_preview_style_cache_epoch: 0,
            worktree_preview_cache_write_blocked_until_rev: None,
            worktree_preview_segments_cache: HashMap::default(),
            diff_preview_is_new_file: false,
            conflict_resolver_input,
            _conflict_resolver_input_subscription: conflict_resolver_subscription,
            conflict_resolver: ConflictResolverUiState::default(),
            conflict_resolver_vsplit_ratio: 0.5,
            conflict_resolver_vsplit_resize: None,
            conflict_three_way_col_ratios: [1.0 / 3.0, 2.0 / 3.0],
            conflict_three_way_col_widths: [px(0.0); 3],
            conflict_hsplit_resize: None,
            conflict_diff_split_ratio: 0.5,
            conflict_diff_split_resize: None,
            conflict_diff_split_col_widths: [px(0.0); 2],
            conflict_canvas_rows_enabled: conflict_canvas_rows_enabled_from_env(),
            conflict_diff_segments_cache_split:
                conflict_resolver::ConflictSplitStyledTextCache::default(),
            conflict_diff_query_segments_cache_split:
                conflict_resolver::ConflictSplitStyledTextCache::default(),
            conflict_diff_query_cache_query: SharedString::default(),
            conflict_three_way_segments_cache: HashMap::default(),
            conflict_three_way_prepared_syntax_documents: ThreeWaySides::default(),
            conflict_three_way_syntax_inflight: ThreeWaySides::default(),
            conflict_resolved_preview_path: None,
            conflict_resolved_preview_source_hash: None,
            conflict_resolved_output_projection: None,
            conflict_resolved_preview_text: TextModelSnapshot::default(),
            conflict_resolved_preview_syntax_language: None,
            conflict_resolved_preview_highlight_provider_theme_epoch: 1,
            conflict_resolved_preview_style_cache_epoch: 0,
            conflict_resolved_preview_prepared_syntax_document: None,
            conflict_resolved_preview_syntax_inflight: None,
            conflict_resolved_preview_line_count: 0,
            conflict_resolved_preview_line_starts: Arc::default(),
            conflict_resolved_output_measure_row: 0,
            conflict_resolved_outline_stash: None,
            conflict_resolved_preview_segments_cache: HashMap::default(),
            #[cfg(test)]
            conflict_resolved_outline_background_delay_override: None,
            history_view,
            diff_scroll: UniformListScrollHandle::default(),
            diff_split_right_scroll: UniformListScrollHandle::default(),
            conflict_resolver_diff_scroll: UniformListScrollHandle::default(),
            conflict_preview_ours_scroll: UniformListScrollHandle::default(),
            conflict_preview_theirs_scroll: UniformListScrollHandle::default(),
            conflict_preview_last_synced_x: [px(0.0); 4],
            conflict_preview_last_synced_y: [px(0.0); 4],
            conflict_resolved_preview_scroll: UniformListScrollHandle::default(),
            conflict_resolved_preview_gutter_scroll: UniformListScrollHandle::default(),
            conflict_resolved_preview_gutter_last_synced_y: [px(0.0); 2],
            worktree_preview_scroll: UniformListScrollHandle::default(),
            path_display_cache: std::cell::RefCell::new(path_display::PathDisplayCache::default()),
        };

        pane.set_theme(theme, cx);
        pane.rebuild_diff_cache(cx);
        pane
    }

    pub(in crate::view) fn sync_root_layout_snapshot(&mut self, cx: &mut gpui::Context<Self>) {
        let fallback_sidebar = self.layout_sidebar_render_width;
        let fallback_details = self.layout_details_render_width;
        let fallback_sidebar_collapsed = self.layout_sidebar_collapsed;
        let fallback_details_collapsed = self.layout_details_collapsed;

        let (sidebar_w, details_w, sidebar_collapsed, details_collapsed) = self
            .root_view
            .read_with(cx, |root, _cx| {
                (
                    root.sidebar_render_width,
                    root.details_render_width,
                    root.sidebar_collapsed,
                    root.details_collapsed,
                )
            })
            .unwrap_or((
                fallback_sidebar,
                fallback_details,
                fallback_sidebar_collapsed,
                fallback_details_collapsed,
            ));

        self.layout_sidebar_render_width = sidebar_w;
        self.layout_details_render_width = details_w;
        self.layout_sidebar_collapsed = sidebar_collapsed;
        self.layout_details_collapsed = details_collapsed;
    }

    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.conflict_resolved_preview_highlight_provider_theme_epoch = self
            .conflict_resolved_preview_highlight_provider_theme_epoch
            .wrapping_add(1)
            .max(1);
        self.clear_diff_text_style_caches();
        self.clear_worktree_preview_segments_cache();
        self.clear_conflict_diff_style_caches();
        self.conflict_three_way_segments_cache.clear();
        self.conflict_resolved_preview_segments_cache.clear();
        self.diff_raw_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.diff_search_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.conflict_resolver_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        if self.conflict_resolved_output_is_streamed() {
            self.conflict_resolved_preview_syntax_language = self
                .conflict_resolved_preview_path
                .as_ref()
                .and_then(rows::diff_syntax_language_for_path);
            self.conflict_resolved_preview_prepared_syntax_document = None;
            self.conflict_resolved_preview_syntax_inflight = None;
            self.conflict_resolved_output_measure_row = self
                .conflict_resolved_output_projection
                .as_ref()
                .map(conflict_resolver::ResolvedOutputProjection::widest_line_ix)
                .unwrap_or(0);
        } else {
            let output_snapshot = self
                .conflict_resolver_input
                .read_with(cx, |input, _| input.text_snapshot());
            self.conflict_resolved_preview_line_starts = output_snapshot.shared_line_starts();
            self.conflict_resolved_preview_line_count =
                self.conflict_resolved_preview_line_starts.len().max(1);
            self.conflict_resolved_output_measure_row = widest_resolved_output_line_ix(
                output_snapshot.as_str(),
                self.conflict_resolved_preview_line_starts.as_ref(),
            );
            self.refresh_conflict_resolved_output_syntax(&output_snapshot, None, cx);
        }
        if let Some(input) = &self.diff_hunk_picker_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        self.history_view
            .update(cx, |view, cx| view.set_theme(theme, cx));
        cx.notify();
    }

    pub(in crate::view) fn invalidate_font_metrics(&mut self, cx: &mut gpui::Context<Self>) {
        self.diff_horizontal_min_width = px(0.0);
        self.diff_text_hitboxes.clear();
        self.diff_text_layout_cache_epoch = self.diff_text_layout_cache_epoch.wrapping_add(1);
        self.diff_text_layout_cache.clear();
        cx.notify();
    }

    pub(in crate::view) fn conflict_resolved_output_is_streamed(&self) -> bool {
        self.conflict_resolved_output_projection.is_some()
    }

    fn sync_conflict_resolved_preview_projection(
        &mut self,
        projection: conflict_resolver::ResolvedOutputProjection,
        path: Option<&std::path::PathBuf>,
    ) {
        self.conflict_resolved_output_projection = Some(projection.clone());
        self.conflict_resolved_preview_path = path.cloned();
        self.conflict_resolved_preview_source_hash = Some(projection.output_hash());
        self.conflict_resolved_preview_text = TextModelSnapshot::default();
        self.conflict_resolved_preview_syntax_language =
            path.and_then(rows::diff_syntax_language_for_path);
        self.conflict_resolved_preview_prepared_syntax_document = None;
        self.conflict_resolved_preview_syntax_inflight = None;
        self.conflict_resolved_preview_line_count = projection.len();
        self.conflict_resolved_preview_line_starts = Arc::default();
        self.conflict_resolved_output_measure_row = projection.widest_line_ix();
        self.conflict_resolved_outline_stash = None;
        self.conflict_resolved_preview_segments_cache.clear();
    }

    pub(in crate::view) fn refresh_streamed_resolved_output_preview_from_projection(
        &mut self,
        projection: conflict_resolver::ResolvedOutputProjection,
        path: Option<&std::path::PathBuf>,
    ) {
        let trace_started = Instant::now();
        let output_line_count = projection.len();
        let view_mode = self.conflict_resolver.view_mode;
        let computed = compute_resolved_outline_computation_from_projection(
            &projection,
            &self.conflict_resolver.marker_segments,
            view_mode,
            (!should_skip_resolved_outline_provenance(view_mode, output_line_count))
                .then(|| self.resolved_outline_source_view()),
        );
        self.sync_conflict_resolved_preview_projection(projection, path);
        self.apply_resolved_outline_computation(path, trace_started, computed);
    }

    pub(in crate::view) fn refresh_streamed_resolved_output_preview_from_markers(
        &mut self,
        path: Option<&std::path::PathBuf>,
    ) {
        let projection = conflict_resolver::ResolvedOutputProjection::from_segments(
            &self.conflict_resolver.marker_segments,
        );
        self.refresh_streamed_resolved_output_preview_from_projection(projection, path);
    }

    pub(in crate::view) fn ensure_conflict_resolved_output_materialized(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.conflict_resolved_output_is_streamed() {
            return;
        }

        let resolved =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        let output_hash = hash_text_bytes(&resolved);
        let line_ending = crate::kit::TextInput::detect_line_ending(&resolved);
        let theme = self.theme;
        let path = self.conflict_resolver.path.clone();
        self.conflict_resolved_output_projection = None;
        self.conflict_resolved_preview_path = path.clone();
        self.conflict_resolved_preview_source_hash = Some(output_hash);
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_line_ending(line_ending);
            input.set_text(resolved.clone(), cx);
        });
        self.recompute_conflict_resolved_outline_and_provenance(path.as_ref(), cx);
    }

    pub(in crate::view) fn current_conflict_resolved_output_text(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> String {
        if self.conflict_resolved_output_is_streamed() {
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments)
        } else {
            self.conflict_resolver_input
                .read_with(cx, |input, _| input.text().to_string())
        }
    }

    pub(in crate::view) fn conflict_resolver_save_contents_from_text(
        &mut self,
        text: String,
    ) -> String {
        self.conflict_resolver_sync_session_resolutions_from_output(&text);
        text
    }

    pub(in crate::view) fn conflict_resolver_save_contents(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> String {
        let text = self.current_conflict_resolved_output_text(cx);
        self.conflict_resolver_save_contents_from_text(text)
    }

    pub(in crate::view) fn ensure_prepared_syntax_chunk_poll(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.syntax_chunk_poll_task.is_some() {
            return;
        }

        if cfg!(test) {
            while self.apply_prepared_syntax_chunk_updates(cx) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            self.syntax_chunk_poll_task = None;
            return;
        }

        let task = cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| loop {
                let should_continue = view
                    .update(cx, |this, cx| this.apply_prepared_syntax_chunk_updates(cx))
                    .unwrap_or(false);

                if !should_continue {
                    break;
                }

                smol::Timer::after(std::time::Duration::from_millis(16)).await;
            },
        );
        self.syntax_chunk_poll_task = Some(task);
    }

    fn apply_prepared_syntax_chunk_updates(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let mut applied = false;

        let split_left_applied = self
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
            .map(rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document)
            .unwrap_or(0);
        if split_left_applied > 0 {
            self.file_diff_style_cache_epochs.bump_left();
            applied = true;
        }

        let split_right_applied = self
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
            .map(rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document)
            .unwrap_or(0);
        if split_right_applied > 0 {
            self.file_diff_style_cache_epochs.bump_right();
            applied = true;
        }

        let worktree_preview_applied = self
            .worktree_preview_prepared_syntax_document()
            .map(rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document)
            .unwrap_or(0);
        if worktree_preview_applied > 0 {
            self.worktree_preview_style_cache_epoch =
                self.worktree_preview_style_cache_epoch.wrapping_add(1);
            applied = true;
        }

        let resolved_preview_applied = self
            .conflict_resolved_preview_prepared_syntax_document
            .map(rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document)
            .unwrap_or(0);
        if resolved_preview_applied > 0 {
            self.conflict_resolved_preview_style_cache_epoch = self
                .conflict_resolved_preview_style_cache_epoch
                .wrapping_add(1);
            applied = true;
        }

        if rows::drain_completed_prepared_diff_syntax_chunk_builds() > 0 {
            applied = true;
        }

        if applied {
            cx.notify();
        }

        let pending = rows::has_pending_prepared_diff_syntax_chunk_builds();
        if !pending {
            self.syntax_chunk_poll_task = None;
        }
        pending
    }

    fn refresh_conflict_resolved_output_syntax(
        &mut self,
        output_snapshot: &TextModelSnapshot,
        syntax_edit: Option<rows::DiffSyntaxEdit>,
        cx: &mut gpui::Context<Self>,
    ) {
        let old_document = self.conflict_resolved_preview_prepared_syntax_document;
        let syntax_state = build_resolved_output_syntax_state_for_snapshot_with_budget(
            self.theme,
            output_snapshot,
            self.conflict_resolved_preview_syntax_language,
            old_document,
            syntax_edit.clone(),
            self.full_document_syntax_budget(),
        );
        let background_key = if syntax_state.needs_background_prepare {
            self.conflict_resolved_preview_syntax_language
                .map(|language| {
                    let source_hash = self
                        .conflict_resolved_preview_source_hash
                        .unwrap_or_else(|| hash_text_bytes(output_snapshot.as_ref()));
                    ResolvedOutputSyntaxBackgroundKey {
                        source_hash,
                        language,
                    }
                })
        } else {
            None
        };
        let prepared_document_changed = old_document != syntax_state.prepared_document;
        self.conflict_resolved_preview_prepared_syntax_document = syntax_state.prepared_document;
        if prepared_document_changed {
            self.conflict_resolved_preview_style_cache_epoch = self
                .conflict_resolved_preview_style_cache_epoch
                .wrapping_add(1);
            cx.notify();
        }
        if background_key.is_none() {
            self.conflict_resolved_preview_syntax_inflight = None;
        }
        let provider_key = syntax_state
            .prepared_document
            .zip(self.conflict_resolved_preview_syntax_language)
            .map(|(document, language)| {
                resolved_output_highlight_provider_binding_key(
                    self.conflict_resolved_preview_highlight_provider_theme_epoch,
                    language,
                    document,
                )
            });
        self.conflict_resolver_input.update(cx, |input, cx| {
            if let Some(provider) = syntax_state.highlight_provider {
                if let Some(provider_key) = provider_key {
                    input.set_highlight_provider_with_key(provider_key, provider, cx);
                } else {
                    input.set_highlight_provider(provider, cx);
                }
            } else {
                input.set_highlights(syntax_state.highlights, cx);
            }
        });
        if let Some(background_key) = background_key {
            self.ensure_conflict_resolved_output_background_syntax_prepare(
                background_key,
                output_snapshot,
                old_document,
                syntax_edit,
                cx,
            );
        }
    }

    fn ensure_conflict_resolved_output_background_syntax_prepare(
        &mut self,
        request_key: ResolvedOutputSyntaxBackgroundKey,
        output_snapshot: &TextModelSnapshot,
        old_document: Option<rows::PreparedDiffSyntaxDocument>,
        syntax_edit: Option<rows::DiffSyntaxEdit>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolved_preview_syntax_inflight == Some(request_key) {
            return;
        }
        self.conflict_resolved_preview_syntax_inflight = Some(request_key);
        let output_text = output_snapshot.as_shared_string();
        let output_line_starts = output_snapshot.shared_line_starts();
        let old_reparse_seed = old_document.and_then(rows::prepared_diff_syntax_reparse_seed);
        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let prepare_document = move || {
                    rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                        request_key.language,
                        rows::DiffSyntaxMode::Auto,
                        output_text,
                        output_line_starts,
                        old_reparse_seed,
                        syntax_edit,
                    )
                };
                let parsed_document = if cfg!(test) {
                    prepare_document()
                } else {
                    smol::unblock(prepare_document).await
                };

                let _ = view.update(cx, |this, cx| {
                    if this.conflict_resolved_preview_syntax_inflight != Some(request_key) {
                        return;
                    }
                    if this.conflict_resolved_preview_source_hash != Some(request_key.source_hash)
                        || this.conflict_resolved_preview_syntax_language
                            != Some(request_key.language)
                    {
                        return;
                    }

                    this.conflict_resolved_preview_syntax_inflight = None;
                    if let Some(parsed_document) = parsed_document {
                        let _ =
                            rows::inject_background_prepared_diff_syntax_document(parsed_document);
                    }
                    let current_output_snapshot = this
                        .conflict_resolver_input
                        .read_with(cx, |input, _| input.text_snapshot());
                    this.refresh_conflict_resolved_output_syntax(
                        &current_output_snapshot,
                        None,
                        cx,
                    );
                });
            },
        )
        .detach();
    }

    /// Schedule a background tree-sitter parse for one merge-input side.
    ///
    /// When the parse completes, the prepared document is injected into the
    /// global cache and the three-way styled-text cache is cleared so the next
    /// render picks up document-based syntax highlighting.
    pub(in crate::view) fn ensure_conflict_three_way_background_syntax_prepare(
        &mut self,
        side: ThreeWayColumn,
        text: SharedString,
        line_starts: Arc<[usize]>,
        language: rows::DiffSyntaxLanguage,
        source_hash: Option<u64>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_three_way_syntax_inflight[side] {
            return;
        }
        self.conflict_three_way_syntax_inflight[side] = true;
        let expected_source_hash = source_hash;
        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let prepare_document = move || {
                    rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                        language,
                        rows::DiffSyntaxMode::Auto,
                        text,
                        line_starts,
                        None,
                        None,
                    )
                };
                let parsed = if cfg!(test) {
                    prepare_document()
                } else {
                    smol::unblock(prepare_document).await
                };

                let _ = view.update(cx, |this, cx| {
                    this.conflict_three_way_syntax_inflight[side] = false;

                    // Stale: source hash changed while we were parsing.
                    if this.conflict_resolver.source_hash != expected_source_hash {
                        return;
                    }

                    if let Some(parsed) = parsed {
                        let document =
                            rows::inject_background_prepared_diff_syntax_document(parsed);
                        this.conflict_three_way_prepared_syntax_documents[side] = Some(document);
                        // Invalidate cached styled text so the next render uses
                        // the prepared document across three-way and two-way
                        // conflict views instead of per-line fallback styling.
                        this.clear_conflict_diff_style_caches_preserving_query();
                        this.conflict_three_way_segments_cache.clear();
                        cx.notify();
                    }
                });
            },
        )
        .detach();
    }

    pub(in crate::view) fn clear_diff_text_query_overlay_cache(&mut self) {
        self.diff_text_query_segments_cache.clear();
        self.diff_text_query_cache_query = SharedString::default();
        self.diff_text_query_cache_generation =
            self.diff_text_query_cache_generation.wrapping_add(1);
    }

    pub(in crate::view) fn invalidate_diff_text_query_overlay_cache(&mut self, query: &str) {
        if self.diff_text_query_cache_query.as_ref() != query {
            self.diff_text_query_cache_query = query.to_string().into();
            self.diff_text_query_cache_generation =
                self.diff_text_query_cache_generation.wrapping_add(1);
        }
    }

    pub(in crate::view) fn sync_diff_text_query_overlay_cache(&mut self, query: &str) {
        self.invalidate_diff_text_query_overlay_cache(query);
    }

    pub(in crate::view) fn clear_diff_text_style_caches(&mut self) {
        self.diff_text_segments_cache.clear();
        self.clear_diff_text_query_overlay_cache();
    }

    pub(in crate::view) fn clear_worktree_preview_segments_cache(&mut self) {
        self.worktree_preview_segments_cache.clear();
        self.worktree_preview_cache_write_blocked_until_rev = None;
    }

    pub(in crate::view) fn clear_conflict_diff_query_overlay_caches(&mut self) {
        self.conflict_diff_query_segments_cache_split.clear();
        self.conflict_diff_query_cache_query = SharedString::default();
    }

    pub(in crate::view) fn clear_conflict_diff_style_caches_preserving_query(&mut self) {
        self.conflict_diff_segments_cache_split.clear();
        self.conflict_diff_query_segments_cache_split.clear();
    }

    pub(in crate::view) fn sync_conflict_diff_query_overlay_caches(&mut self, query: &str) {
        if self.conflict_diff_query_cache_query.as_ref() != query {
            self.conflict_diff_query_cache_query = query.to_string().into();
            self.conflict_diff_query_segments_cache_split.clear();
        }
    }

    pub(in crate::view) fn clear_conflict_diff_style_caches(&mut self) {
        self.clear_conflict_diff_style_caches_preserving_query();
        self.conflict_diff_query_cache_query = SharedString::default();
    }

    pub(super) fn conflict_resolver_invalidate_resolved_outline(&mut self) {
        self.conflict_resolver.resolver_pending_recompute_seq = self
            .conflict_resolver
            .resolver_pending_recompute_seq
            .wrapping_add(1);
        self.conflict_resolved_preview_path = None;
        self.conflict_resolved_preview_source_hash = None;
        self.conflict_resolved_output_projection = None;
        self.conflict_resolved_preview_text = TextModelSnapshot::default();
        self.conflict_resolved_preview_syntax_language = None;
        self.conflict_resolved_preview_prepared_syntax_document = None;
        self.conflict_resolved_preview_syntax_inflight = None;
        self.conflict_resolved_preview_line_count = 0;
        self.conflict_resolved_preview_line_starts = Arc::default();
        self.conflict_resolved_output_measure_row = 0;
        self.conflict_resolved_outline_stash = None;
        self.conflict_resolved_preview_segments_cache.clear();
        self.conflict_three_way_prepared_syntax_documents = ThreeWaySides::default();
        self.conflict_three_way_syntax_inflight = ThreeWaySides::default();
        self.conflict_three_way_segments_cache.clear();
        self.conflict_resolver.resolved_outline = ResolvedOutlineData::default();
    }

    pub(super) fn recompute_conflict_resolved_outline_and_provenance(
        &mut self,
        path: Option<&std::path::PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.recompute_conflict_resolved_outline_and_provenance_with_syntax_edit(path, None, cx);
    }

    fn resolved_outline_source_view(&self) -> ResolvedOutlineSourceView<'_> {
        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => ResolvedOutlineSourceView::ThreeWay {
                base_text: &self.conflict_resolver.three_way_text.base,
                base_line_starts: self
                    .conflict_resolver
                    .three_way_line_starts_ref(ThreeWayColumn::Base),
                ours_text: &self.conflict_resolver.three_way_text.ours,
                ours_line_starts: self
                    .conflict_resolver
                    .three_way_line_starts_ref(ThreeWayColumn::Ours),
                theirs_text: &self.conflict_resolver.three_way_text.theirs,
                theirs_line_starts: self
                    .conflict_resolver
                    .three_way_line_starts_ref(ThreeWayColumn::Theirs),
            },
            ConflictResolverViewMode::TwoWayDiff => ResolvedOutlineSourceView::TwoWay {
                ours_text: &self.conflict_resolver.three_way_text.ours,
                ours_line_starts: self
                    .conflict_resolver
                    .three_way_line_starts_ref(ThreeWayColumn::Ours),
                theirs_text: &self.conflict_resolver.three_way_text.theirs,
                theirs_line_starts: self
                    .conflict_resolver
                    .three_way_line_starts_ref(ThreeWayColumn::Theirs),
            },
        }
    }

    fn background_resolved_outline_recompute_request(
        &self,
        output_snapshot: &TextModelSnapshot,
    ) -> BackgroundResolvedOutlineRecomputeRequest {
        let output_text: Arc<str> = output_snapshot.as_shared_string().into();
        let output_line_count = output_snapshot.shared_line_starts().len().max(1);
        let sources = match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => OwnedResolvedOutlineSourceData::ThreeWay {
                base_text: self.conflict_resolver.three_way_text.base.clone().into(),
                base_line_starts: self
                    .conflict_resolver
                    .three_way_shared_line_starts(ThreeWayColumn::Base),
                ours_text: self.conflict_resolver.three_way_text.ours.clone().into(),
                ours_line_starts: self
                    .conflict_resolver
                    .three_way_shared_line_starts(ThreeWayColumn::Ours),
                theirs_text: self.conflict_resolver.three_way_text.theirs.clone().into(),
                theirs_line_starts: self
                    .conflict_resolver
                    .three_way_shared_line_starts(ThreeWayColumn::Theirs),
            },
            ConflictResolverViewMode::TwoWayDiff => OwnedResolvedOutlineSourceData::TwoWay {
                ours_text: self.conflict_resolver.three_way_text.ours.clone().into(),
                ours_line_starts: self
                    .conflict_resolver
                    .three_way_shared_line_starts(ThreeWayColumn::Ours),
                theirs_text: self.conflict_resolver.three_way_text.theirs.clone().into(),
                theirs_line_starts: self
                    .conflict_resolver
                    .three_way_shared_line_starts(ThreeWayColumn::Theirs),
            },
        };

        BackgroundResolvedOutlineRecomputeRequest {
            output_text,
            output_line_count,
            marker_segments: self.conflict_resolver.marker_segments.clone(),
            sources,
        }
    }

    fn stash_current_conflict_resolved_outline_state(&mut self) {
        let line_count = self.conflict_resolved_preview_line_count;
        if line_count == 0
            || self.conflict_resolver.resolved_outline.meta.len() != line_count
            || self.conflict_resolver.resolved_outline.markers.len() != line_count
        {
            return;
        }

        self.conflict_resolved_outline_stash = Some(StashedResolvedOutlineState {
            text: self.conflict_resolved_preview_text.clone(),
            line_starts: self.conflict_resolved_preview_line_starts.clone(),
            marker_segments: self.conflict_resolver.marker_segments.clone(),
            view_mode: self.conflict_resolver.view_mode,
            outline: self.conflict_resolver.resolved_outline.clone(),
        });
    }

    fn resolved_outline_incremental_base(&self) -> Option<ResolvedOutlineIncrementalBase<'_>> {
        if self.conflict_resolved_output_is_streamed() {
            return None;
        }
        if let Some(stash) = self.conflict_resolved_outline_stash.as_ref() {
            return Some(ResolvedOutlineIncrementalBase {
                text: &stash.text,
                line_starts: &stash.line_starts,
                marker_segments: &stash.marker_segments,
                view_mode: stash.view_mode,
            });
        }

        let line_count = self.conflict_resolved_preview_line_count;
        if line_count == 0
            || self.conflict_resolver.resolved_outline.meta.len() != line_count
            || self.conflict_resolver.resolved_outline.markers.len() != line_count
        {
            return None;
        }

        Some(ResolvedOutlineIncrementalBase {
            text: &self.conflict_resolved_preview_text,
            line_starts: &self.conflict_resolved_preview_line_starts,
            marker_segments: &self.conflict_resolver.marker_segments,
            view_mode: self.conflict_resolver.view_mode,
        })
    }

    fn sync_conflict_resolved_preview_snapshot(
        &mut self,
        output_snapshot: &TextModelSnapshot,
        path: Option<&std::path::PathBuf>,
        syntax_edit: Option<rows::DiffSyntaxEdit>,
        clear_outline: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if clear_outline {
            self.stash_current_conflict_resolved_outline_state();
        }
        self.conflict_resolved_preview_line_starts = output_snapshot.shared_line_starts();
        self.conflict_resolved_preview_syntax_language =
            path.and_then(rows::diff_syntax_language_for_path);
        self.conflict_resolved_preview_line_count =
            self.conflict_resolved_preview_line_starts.len().max(1);
        self.conflict_resolved_output_measure_row = widest_resolved_output_line_ix(
            output_snapshot.as_str(),
            self.conflict_resolved_preview_line_starts.as_ref(),
        );
        self.conflict_resolved_preview_segments_cache.clear();
        self.refresh_conflict_resolved_output_syntax(output_snapshot, syntax_edit, cx);
        self.conflict_resolved_preview_text = output_snapshot.clone();

        if clear_outline {
            self.conflict_resolver.resolved_outline = ResolvedOutlineData::default();
            self.conflict_resolver.resolved_outline_gutter_rows.clear();
        }
    }

    fn apply_resolved_outline_computation(
        &mut self,
        path: Option<&std::path::PathBuf>,
        trace_started: Instant,
        computed: ResolvedOutlineComputation,
    ) {
        self.conflict_resolved_outline_stash = None;
        self.conflict_resolver.resolved_outline = computed.outline;
        self.conflict_resolver.resolved_outline_gutter_rows.clear();
        record_resolved_outline_trace(path, trace_started, self, computed.output_line_count);
    }

    fn recompute_conflict_resolved_outline_and_provenance_with_syntax_edit(
        &mut self,
        path: Option<&std::path::PathBuf>,
        syntax_edit: Option<rows::DiffSyntaxEdit>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolved_output_is_streamed() {
            let _ = syntax_edit;
            let _ = cx;
            self.refresh_streamed_resolved_output_preview_from_markers(path);
            return;
        }
        let _perf_scope = perf::span(ViewPerfSpan::RecomputeResolvedOutline);
        let trace_started = Instant::now();
        let output_snapshot = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text_snapshot());
        let output_text = output_snapshot.as_ref();
        let output_line_count = output_snapshot.shared_line_starts().len().max(1);
        let computed = compute_resolved_outline_computation(
            output_text,
            output_line_count,
            &self.conflict_resolver.marker_segments,
            self.resolved_outline_source_view(),
        );
        self.sync_conflict_resolved_preview_snapshot(
            &output_snapshot,
            path,
            syntax_edit,
            false,
            cx,
        );
        self.apply_resolved_outline_computation(path, trace_started, computed);
    }

    fn recompute_conflict_resolved_outline_and_provenance_incremental(
        &mut self,
        path: Option<&std::path::PathBuf>,
        delta: ResolvedOutlineDelta,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.conflict_resolved_output_is_streamed() {
            let _ = path;
            let _ = delta;
            let _ = cx;
            return false;
        }
        let Some(base) = self.resolved_outline_incremental_base() else {
            return false;
        };
        let old_text_snapshot = base.text.clone();
        let old_text = old_text_snapshot.as_ref();
        let output_snapshot = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text_snapshot());
        let output_text = output_snapshot.as_ref();
        let old_line_starts = base.line_starts.clone();
        let old_line_count = old_line_starts.len().max(1);
        let new_line_starts = output_snapshot.shared_line_starts();
        let new_line_count = new_line_starts.len().max(1);
        if old_line_starts.is_empty() {
            return false;
        }
        let used_stash = self.conflict_resolved_outline_stash.is_some();
        let delta = if used_stash {
            resolved_outline_delta_between_texts(old_text, output_text)
        } else {
            Some(delta)
        };
        let Some(delta) = delta else {
            return false;
        };
        if delta.old_range.start > delta.old_range.end
            || delta.new_range.start > delta.new_range.end
            || delta.old_range.end > old_text.len()
            || delta.new_range.end > output_text.len()
        {
            return false;
        }

        let mut old_affected = dirty_byte_range_to_line_range(
            old_line_starts.as_ref(),
            old_text.len(),
            delta.old_range.clone(),
        );
        let mut new_affected = dirty_byte_range_to_line_range(
            new_line_starts.as_ref(),
            output_text.len(),
            delta.new_range.clone(),
        );
        old_affected.start = old_affected.start.saturating_sub(1);
        old_affected.end = old_affected.end.saturating_add(1).min(old_line_count);
        new_affected.start = new_affected.start.saturating_sub(1);
        new_affected.end = new_affected.end.saturating_add(1).min(new_line_count);

        let Some(old_block_ranges) =
            resolved_output_conflict_block_ranges_in_text(base.marker_segments, old_text)
        else {
            return false;
        };
        let Some(new_block_ranges) = resolved_output_conflict_block_ranges_in_text(
            &self.conflict_resolver.marker_segments,
            output_text,
        ) else {
            return false;
        };
        if old_block_ranges.len() != new_block_ranges.len() {
            return false;
        }

        let mut touched_conflicts: HashSet<usize> = HashSet::default();
        for (conflict_ix, range) in old_block_ranges.iter().enumerate() {
            if line_ranges_intersect(range, &old_affected) {
                touched_conflicts.insert(conflict_ix);
            }
        }
        for (conflict_ix, range) in new_block_ranges.iter().enumerate() {
            if line_ranges_intersect(range, &new_affected) {
                touched_conflicts.insert(conflict_ix);
            }
        }
        for conflict_ix in &touched_conflicts {
            if let Some(old_range) = old_block_ranges.get(*conflict_ix) {
                old_affected.start = old_affected.start.min(old_range.start);
                old_affected.end = old_affected.end.max(old_range.end).min(old_line_count);
            }
            if let Some(new_range) = new_block_ranges.get(*conflict_ix) {
                new_affected.start = new_affected.start.min(new_range.start);
                new_affected.end = new_affected.end.max(new_range.end).min(new_line_count);
            }
        }

        let mut recompute_conflicts = Vec::new();
        for (conflict_ix, new_range) in new_block_ranges.iter().enumerate() {
            if line_ranges_intersect(new_range, &new_affected) {
                recompute_conflicts.push(conflict_ix);
                if let Some(old_range) = old_block_ranges.get(conflict_ix) {
                    old_affected.start = old_affected.start.min(old_range.start);
                    old_affected.end = old_affected.end.max(old_range.end).min(old_line_count);
                }
                new_affected.start = new_affected.start.min(new_range.start);
                new_affected.end = new_affected.end.max(new_range.end).min(new_line_count);
            }
        }
        if old_affected.start != new_affected.start {
            return false;
        }

        let old_view_mode = base.view_mode;
        let new_view_mode = self.conflict_resolver.view_mode;
        let middle_meta = {
            let mut source_lookup: HashMap<
                &str,
                (conflict_resolver::ResolvedLineSource, Option<u32>),
            > = HashMap::default();
            match new_view_mode {
                ConflictResolverViewMode::ThreeWay => {
                    insert_lookup_from_indexed_text(
                        &mut source_lookup,
                        conflict_resolver::ResolvedLineSource::C,
                        &self.conflict_resolver.three_way_text.theirs,
                        self.conflict_resolver
                            .three_way_line_starts_ref(ThreeWayColumn::Theirs),
                    );
                    insert_lookup_from_indexed_text(
                        &mut source_lookup,
                        conflict_resolver::ResolvedLineSource::B,
                        &self.conflict_resolver.three_way_text.ours,
                        self.conflict_resolver
                            .three_way_line_starts_ref(ThreeWayColumn::Ours),
                    );
                    insert_lookup_from_indexed_text(
                        &mut source_lookup,
                        conflict_resolver::ResolvedLineSource::A,
                        &self.conflict_resolver.three_way_text.base,
                        self.conflict_resolver
                            .three_way_line_starts_ref(ThreeWayColumn::Base),
                    );
                }
                ConflictResolverViewMode::TwoWayDiff => {
                    insert_lookup_from_indexed_text(
                        &mut source_lookup,
                        conflict_resolver::ResolvedLineSource::B,
                        &self.conflict_resolver.three_way_text.theirs,
                        self.conflict_resolver
                            .three_way_line_starts_ref(ThreeWayColumn::Theirs),
                    );
                    insert_lookup_from_indexed_text(
                        &mut source_lookup,
                        conflict_resolver::ResolvedLineSource::A,
                        &self.conflict_resolver.three_way_text.ours,
                        self.conflict_resolver
                            .three_way_line_starts_ref(ThreeWayColumn::Ours),
                    );
                }
            }

            let mut middle_meta = Vec::with_capacity(new_affected.len());
            for line_ix in new_affected.clone() {
                let output_line =
                    rows::resolved_output_line_text(output_text, new_line_starts.as_ref(), line_ix);
                let (source, input_line) = source_lookup
                    .get(output_line)
                    .copied()
                    .unwrap_or((conflict_resolver::ResolvedLineSource::Manual, None));
                middle_meta.push(conflict_resolver::ResolvedLineMeta {
                    output_line: u32::try_from(line_ix).unwrap_or(u32::MAX),
                    source,
                    input_line,
                });
            }
            middle_meta
        };

        let old_outline = if used_stash {
            self.conflict_resolved_outline_stash
                .as_ref()
                .map(|stash| stash.outline.clone())
                .unwrap_or_default()
        } else {
            std::mem::take(&mut self.conflict_resolver.resolved_outline)
        };
        let old_meta = old_outline.meta;
        let old_markers = old_outline.markers;
        let mut next_sources_index = old_outline.sources_index;
        let line_delta = new_affected.len() as isize - old_affected.len() as isize;

        let mut next_meta = Vec::with_capacity(new_line_count);
        next_meta.extend(
            old_meta
                .iter()
                .take(old_affected.start.min(old_meta.len()))
                .cloned(),
        );
        next_meta.extend(middle_meta);
        for entry in old_meta.iter().skip(old_affected.end.min(old_meta.len())) {
            let mut shifted = entry.clone();
            shifted.output_line =
                u32::try_from(shifted_line_index(entry.output_line as usize, line_delta))
                    .unwrap_or(u32::MAX);
            next_meta.push(shifted);
        }
        apply_conflict_choice_provenance_hints(
            &mut next_meta,
            &self.conflict_resolver.marker_segments,
            output_text,
            new_view_mode,
        );

        let mut next_markers = vec![None; new_line_count];
        for (line_ix, marker) in old_markers
            .iter()
            .copied()
            .enumerate()
            .take(old_affected.start.min(old_markers.len()))
        {
            if line_ix < new_line_count {
                next_markers[line_ix] = marker;
            }
        }
        for (old_line_ix, marker) in old_markers
            .iter()
            .copied()
            .enumerate()
            .skip(old_affected.end.min(old_markers.len()))
        {
            let Some(marker) = marker else {
                continue;
            };
            let new_line_ix = shifted_line_index(old_line_ix, line_delta);
            if new_line_ix < new_line_count {
                next_markers[new_line_ix] = Some(shift_resolved_output_marker(marker, line_delta));
            }
        }
        let blocks: Vec<&conflict_resolver::ConflictBlock> = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|seg| match seg {
                conflict_resolver::ConflictSegment::Block(block) => Some(block),
                _ => None,
            })
            .collect();
        for conflict_ix in recompute_conflicts {
            let block = blocks[conflict_ix];
            let range = new_block_ranges[conflict_ix].clone();
            let marker_ranges = conflict_marker_ranges_for_block(block, range);
            write_conflict_markers_for_ranges(
                &mut next_markers,
                conflict_ix,
                !block.resolved,
                marker_ranges.as_slice(),
            );
        }

        update_line_sources_index_for_range(
            &mut next_sources_index,
            old_view_mode,
            old_meta.as_slice(),
            old_text,
            old_line_starts.as_ref(),
            old_affected.clone(),
            false,
        );
        update_line_sources_index_for_range(
            &mut next_sources_index,
            new_view_mode,
            next_meta.as_slice(),
            output_text,
            new_line_starts.as_ref(),
            new_affected.clone(),
            true,
        );

        self.conflict_resolved_preview_syntax_language =
            path.and_then(rows::diff_syntax_language_for_path);
        self.conflict_resolved_preview_line_count = new_line_count;
        self.conflict_resolved_preview_line_starts = new_line_starts;
        self.conflict_resolved_output_measure_row = widest_resolved_output_line_ix(
            output_text,
            self.conflict_resolved_preview_line_starts.as_ref(),
        );
        if used_stash {
            self.conflict_resolved_preview_segments_cache.clear();
        } else {
            remap_line_keyed_cache_for_delta(
                &mut self.conflict_resolved_preview_segments_cache,
                old_affected,
                new_affected,
            );
        }
        let syntax_edit = (!used_stash).then(|| diff_syntax_edit_from_outline_delta(delta));
        self.refresh_conflict_resolved_output_syntax(&output_snapshot, syntax_edit, cx);
        self.conflict_resolved_outline_stash = None;
        self.conflict_resolver.resolved_outline = ResolvedOutlineData {
            meta: next_meta,
            markers: next_markers,
            sources_index: next_sources_index,
        };
        self.conflict_resolver.resolved_outline_gutter_rows.clear();
        self.conflict_resolved_preview_text = output_snapshot;
        true
    }

    pub(super) fn conflict_resolver_scroll_resolved_output_to_line(
        &self,
        target_line_ix: usize,
        line_count: usize,
    ) {
        scroll_conflict_resolved_output_to_line(
            &self.conflict_resolved_preview_scroll,
            target_line_ix,
            line_count,
        );
    }

    pub(super) fn conflict_resolver_scroll_resolved_output_to_line_in_text(
        &self,
        target_line_ix: usize,
        output_text: &str,
    ) {
        let line_count = count_newlines(output_text).saturating_add(1);
        self.conflict_resolver_scroll_resolved_output_to_line(target_line_ix, line_count);
    }

    pub(super) fn schedule_conflict_resolved_outline_recompute(
        &mut self,
        path: Option<std::path::PathBuf>,
        output_hash: u64,
        delta: Option<ResolvedOutlineDelta>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolved_output_is_streamed() {
            let _ = output_hash;
            let _ = delta;
            self.refresh_streamed_resolved_output_preview_from_markers(path.as_ref());
            cx.notify();
            return;
        }
        self.conflict_resolver.resolver_pending_recompute_seq = self
            .conflict_resolver
            .resolver_pending_recompute_seq
            .wrapping_add(1);
        let seq = self.conflict_resolver.resolver_pending_recompute_seq;

        #[cfg(test)]
        {
            let did_incremental = delta.clone().is_some_and(|delta| {
                self.recompute_conflict_resolved_outline_and_provenance_incremental(
                    path.as_ref(),
                    delta,
                    cx,
                )
            });
            if did_incremental {
                cx.notify();
                return;
            }

            let trace_started = Instant::now();
            let output_snapshot = self
                .conflict_resolver_input
                .read_with(cx, |input, _| input.text_snapshot());
            let syntax_edit = delta.clone().map(diff_syntax_edit_from_outline_delta);
            let request = self.background_resolved_outline_recompute_request(&output_snapshot);
            let background_delay = self
                .conflict_resolved_outline_background_delay_override
                .unwrap_or_default();
            self.sync_conflict_resolved_preview_snapshot(
                &output_snapshot,
                path.as_ref(),
                syntax_edit,
                true,
                cx,
            );

            if background_delay.is_zero()
                && self.conflict_resolver.resolver_pending_recompute_seq == seq
                && self.conflict_resolved_preview_source_hash == Some(output_hash)
                && self.conflict_resolved_preview_path.as_ref() == path.as_ref()
            {
                let computed = compute_resolved_outline_computation(
                    request.output_text.as_ref(),
                    request.output_line_count,
                    &request.marker_segments,
                    request.sources.as_view(),
                );
                self.apply_resolved_outline_computation(path.as_ref(), trace_started, computed);
            }

            cx.notify();
        }

        #[cfg(not(test))]
        {
            cx.spawn(
                async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                    smol::Timer::after(Duration::from_millis(
                        CONFLICT_RESOLVED_OUTLINE_DEBOUNCE_MS,
                    ))
                    .await;
                    let request = view.update(cx, |this, cx| {
                        if this.conflict_resolver.resolver_pending_recompute_seq != seq {
                            return None;
                        }
                        if this.conflict_resolved_preview_source_hash != Some(output_hash)
                            || this.conflict_resolved_preview_path.as_ref() != path.as_ref()
                        {
                            return None;
                        }
                        let did_incremental = delta.clone().is_some_and(|delta| {
                            this.recompute_conflict_resolved_outline_and_provenance_incremental(
                                path.as_ref(),
                                delta,
                                cx,
                            )
                        });
                        if !did_incremental {
                            let trace_started = Instant::now();
                            let output_snapshot = this
                                .conflict_resolver_input
                                .read_with(cx, |input, _| input.text_snapshot());
                            let syntax_edit =
                                delta.clone().map(diff_syntax_edit_from_outline_delta);
                            let request = this
                                .background_resolved_outline_recompute_request(&output_snapshot);
                            let background_delay = Duration::default();
                            this.sync_conflict_resolved_preview_snapshot(
                                &output_snapshot,
                                path.as_ref(),
                                syntax_edit,
                                true,
                                cx,
                            );
                            cx.notify();
                            return Some((request, trace_started, background_delay));
                        }

                        cx.notify();
                        None
                    });
                    let Some((request, trace_started, background_delay)) = request.ok().flatten()
                    else {
                        return;
                    };

                    if !background_delay.is_zero() {
                        smol::Timer::after(background_delay).await;
                    }

                    let compute_outline = move || {
                        compute_resolved_outline_computation(
                            request.output_text.as_ref(),
                            request.output_line_count,
                            &request.marker_segments,
                            request.sources.as_view(),
                        )
                    };
                    let computed = smol::unblock(compute_outline).await;

                    let _ = view.update(cx, |this, cx| {
                        if this.conflict_resolver.resolver_pending_recompute_seq != seq {
                            return;
                        }
                        if this.conflict_resolved_preview_source_hash != Some(output_hash)
                            || this.conflict_resolved_preview_path.as_ref() != path.as_ref()
                        {
                            return;
                        }

                        this.apply_resolved_outline_computation(
                            path.as_ref(),
                            trace_started,
                            computed,
                        );
                        cx.notify();
                    });
                },
            )
            .detach();
        }
    }

    #[cfg(test)]
    pub(in crate::view) fn recompute_conflict_resolved_outline_for_tests(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let path = self.conflict_resolver.path.clone();
        self.recompute_conflict_resolved_outline_and_provenance_with_syntax_edit(
            path.as_ref(),
            None,
            cx,
        );
    }

    #[cfg(test)]
    pub(in crate::view) fn set_conflict_resolved_outline_background_delay_override_for_tests(
        &mut self,
        delay: Duration,
    ) {
        self.conflict_resolved_outline_background_delay_override = Some(delay);
    }

    pub(in crate::view) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next.clone();
        self.history_view.update(cx, |view, cx| {
            view.set_active_context_menu_invoker(next, cx)
        });
        cx.notify();
    }

    pub(in crate::view) fn set_date_time_format(
        &mut self,
        next: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == next {
            return;
        }
        self.date_time_format = next;
        self.history_view
            .update(cx, |view, cx| view.set_date_time_format(next, cx));
        cx.notify();
    }

    pub(in crate::view) fn set_timezone(&mut self, next: Timezone, cx: &mut gpui::Context<Self>) {
        self.history_view
            .update(cx, |view, cx| view.set_timezone(next, cx));
        cx.notify();
    }

    pub(in crate::view) fn set_show_timezone(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view
            .update(cx, |view, cx| view.set_show_timezone(enabled, cx));
        cx.notify();
    }

    pub(in crate::view) fn set_diff_scroll_sync(
        &mut self,
        next: DiffScrollSync,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_scroll_sync == next {
            return;
        }

        self.diff_scroll_sync = next;
        self.sync_diff_split_scroll();
        self.sync_conflict_preview_scroll();
        cx.notify();
    }

    pub(in crate::view) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(in crate::view) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in crate::view) fn history_visible_column_preferences(
        &self,
        cx: &gpui::App,
    ) -> (bool, bool, bool) {
        self.history_view
            .read(cx)
            .history_visible_column_preferences()
    }

    pub(in crate::view) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_at(kind, anchor, window, cx);
                });
            });
        });
    }

    pub(in crate::view) fn activate_context_menu_invoker(
        &mut self,
        invoker: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, move |root, cx| {
            root.set_active_context_menu_invoker(Some(invoker), cx);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn open_conflict_resolver_input_row_context_menu(
        &mut self,
        invoker: SharedString,
        line_label: SharedString,
        line_target: ResolverPickTarget,
        chunk_label: SharedString,
        chunk_target: ResolverPickTarget,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.activate_context_menu_invoker(invoker, cx);
        self.open_popover_at(
            PopoverKind::ConflictResolverInputRowMenu {
                line_label,
                line_target,
                chunk_label,
                chunk_target,
            },
            anchor,
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn open_conflict_resolver_chunk_context_menu(
        &mut self,
        invoker: SharedString,
        conflict_ix: usize,
        has_base: bool,
        is_three_way: bool,
        selected_choices: Vec<conflict_resolver::ConflictChoice>,
        output_line_ix: Option<usize>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.activate_context_menu_invoker(invoker, cx);
        self.open_popover_at(
            PopoverKind::ConflictResolverChunkMenu {
                conflict_ix,
                has_base,
                is_three_way,
                selected_choices,
                output_line_ix,
            },
            anchor,
            window,
            cx,
        );
    }

    pub(in crate::view) fn conflict_resolver_selected_choices_for_conflict_ix(
        &self,
        conflict_ix: usize,
    ) -> Vec<conflict_resolver::ConflictChoice> {
        conflict_group_selected_choices_for_ix(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            conflict_ix,
        )
    }

    pub(in crate::view) fn conflict_resolver_has_base_for_conflict_ix(
        &self,
        conflict_ix: usize,
    ) -> bool {
        self.conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|seg| match seg {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.base.is_some()),
                _ => None,
            })
            .nth(conflict_ix)
            .unwrap_or(false)
    }

    pub(in crate::view) fn open_conflict_resolver_output_context_menu(
        &mut self,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let (selected_text, cursor_offset, clicked_offset, content) =
            self.conflict_resolver_input.read_with(cx, |i, _| {
                (
                    i.selected_text(),
                    i.cursor_offset(),
                    i.offset_for_position(anchor),
                    i.text().to_string(),
                )
            });
        let context_line =
            conflict_resolver_output_context_line(&content, cursor_offset, Some(clicked_offset));

        self.open_conflict_resolver_output_context_menu_at_line(
            context_line,
            selected_text,
            content,
            anchor,
            window,
            cx,
        );
    }

    pub(in crate::view) fn open_conflict_resolver_output_context_menu_for_line(
        &mut self,
        line_ix: usize,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolved_output_is_streamed() {
            let context_line =
                line_ix.min(self.conflict_resolved_preview_line_count.saturating_sub(1));
            self.open_conflict_resolver_output_context_menu_at_line(
                context_line,
                None,
                String::new(),
                anchor,
                window,
                cx,
            );
            return;
        }

        let content = self
            .conflict_resolver_input
            .read_with(cx, |i, _| i.text().to_string());
        let context_line = line_ix.min(self.conflict_resolved_preview_line_count.saturating_sub(1));
        let cursor_offset = line_start_offset_for_index(
            self.conflict_resolved_preview_line_starts.as_ref(),
            content.len(),
            context_line,
        );
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_cursor_offset(cursor_offset, cx);
        });

        self.open_conflict_resolver_output_context_menu_at_line(
            context_line,
            None,
            content,
            anchor,
            window,
            cx,
        );
    }

    fn open_conflict_resolver_output_context_menu_at_line(
        &mut self,
        context_line: usize,
        selected_text: Option<String>,
        content: String,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let conflict_marker = if self.conflict_resolved_output_is_streamed() {
            self.conflict_resolver
                .resolved_outline
                .markers
                .get(context_line)
                .copied()
                .flatten()
        } else {
            resolved_output_marker_for_line(
                &self.conflict_resolver.marker_segments,
                &content,
                context_line,
            )
        };
        if let Some(marker) = conflict_marker {
            let is_three_way = self.conflict_resolver.view_mode
                == conflict_resolver::ConflictResolverViewMode::ThreeWay;
            let selected_choices =
                self.conflict_resolver_selected_choices_for_conflict_ix(marker.conflict_ix);
            let has_base = self.conflict_resolver_has_base_for_conflict_ix(marker.conflict_ix);
            let invoker: SharedString = format!(
                "resolver_output_chunk_menu_{}_{}",
                marker.conflict_ix, context_line
            )
            .into();
            self.open_conflict_resolver_chunk_context_menu(
                invoker,
                marker.conflict_ix,
                has_base,
                is_three_way,
                selected_choices,
                Some(context_line),
                anchor,
                window,
                cx,
            );
            return;
        }

        let is_three_way = self.conflict_resolver.view_mode
            == conflict_resolver::ConflictResolverViewMode::ThreeWay;

        let (has_source_a, has_source_b, has_source_c) = if is_three_way {
            (
                self.conflict_resolver
                    .three_way_has_line(ThreeWayColumn::Base, context_line),
                self.conflict_resolver
                    .three_way_has_line(ThreeWayColumn::Ours, context_line),
                self.conflict_resolver
                    .three_way_has_line(ThreeWayColumn::Theirs, context_line),
            )
        } else {
            {
                let row = self
                    .conflict_resolver
                    .two_way_split_row_by_source(context_line);
                (
                    row.as_ref().and_then(|r| r.old.as_ref()).is_some(),
                    row.as_ref().and_then(|r| r.new.as_ref()).is_some(),
                    false,
                )
            }
        };

        self.open_popover_at(
            PopoverKind::ConflictResolverOutputMenu {
                cursor_line: context_line,
                selected_text,
                has_source_a,
                has_source_b,
                has_source_c,
                is_three_way,
            },
            anchor,
            window,
            cx,
        );
    }

    pub(in crate::view) fn open_popover_at_cursor(
        &mut self,
        kind: PopoverKind,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_at(kind, root.last_mouse_pos, window, cx);
                });
            });
        });
    }

    pub(in crate::view) fn clear_status_multi_selection(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.remove(&repo_id);
                cx.notify();
            });
        });
    }

    pub(in crate::view) fn active_change_tracking_view(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> ChangeTrackingView {
        self.root_view
            .update(cx, |root, _cx| root.change_tracking_view)
            .unwrap_or(ChangeTrackingView::Combined)
    }

    pub(in crate::view) fn scroll_status_section_to_ix(
        &mut self,
        section: StatusSection,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.details_pane
                .update(cx, |pane: &mut DetailsPaneView, cx| {
                    match section {
                        StatusSection::CombinedUnstaged | StatusSection::Unstaged => pane
                            .unstaged_scroll
                            .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center),
                        StatusSection::Untracked => pane
                            .untracked_scroll
                            .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center),
                        StatusSection::Staged => pane
                            .staged_scroll
                            .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center),
                    }
                    cx.notify();
                });
        });
    }

    pub(in crate::view) fn set_tooltip_text_if_changed(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.set_tooltip_text_if_changed(next, cx));
        false
    }

    pub(in crate::view) fn clear_tooltip_if_matches(
        &mut self,
        tooltip: &SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let tooltip = tooltip.clone();
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.clear_tooltip_if_matches(&tooltip, cx));
        false
    }

    pub(super) fn apply_state_snapshot(
        &mut self,
        next: Arc<AppState>,
        cx: &mut gpui::Context<Self>,
    ) {
        let prev_active_repo_id = self.state.active_repo;
        let prev_diff_target = self
            .active_repo()
            .and_then(|r| r.diff_state.diff_target.as_ref())
            .cloned();

        let next_repo_id = next.active_repo;
        let next_repo = next_repo_id.and_then(|id| next.repos.iter().find(|r| r.id == id));
        let next_diff_target = next_repo
            .and_then(|r| r.diff_state.diff_target.as_ref())
            .cloned();
        let next_diff_rev = next_repo.map(|r| r.diff_state.diff_rev).unwrap_or(0);

        if prev_diff_target != next_diff_target {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            self.diff_autoscroll_pending = next_diff_target.is_some();
            self.worktree_preview_path = None;
            self.worktree_preview = Loadable::NotLoaded;
            self.worktree_preview_content_rev = 0;
            self.worktree_markdown_preview_path = None;
            self.worktree_markdown_preview_source_rev = 0;
            self.worktree_markdown_preview = Loadable::NotLoaded;
            self.worktree_markdown_preview_inflight = None;
            self.worktree_preview_syntax_language = None;
            self.reset_worktree_preview_source_state();
            self.diff_horizontal_min_width = px(0.0);
        }

        self.state = next;

        self.sync_conflict_resolver(cx);
        self.ensure_file_image_diff_cache(cx);

        if prev_active_repo_id != next_repo_id {
            self.history_view.update(cx, |view, _| {
                view.history_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            });
        }

        let should_rebuild_diff_cache = self.diff_cache_repo_id != next_repo_id
            || self.diff_cache_rev != next_diff_rev
            || self.diff_cache_target != next_diff_target;
        if should_rebuild_diff_cache {
            self.rebuild_diff_cache(cx);
        }

        // History caches are now managed by HistoryView.
    }

    pub(in crate::view) fn cached_path_display(&self, path: &std::path::Path) -> SharedString {
        let mut cache = self.path_display_cache.borrow_mut();
        path_display::cached_path_display(&mut cache, path)
    }

    pub(in crate::view) fn touch_diff_text_layout_cache(
        &mut self,
        key: u64,
        layout: Option<ShapedLine>,
    ) {
        let epoch = self.diff_text_layout_cache_epoch;
        match layout {
            Some(layout) => {
                self.diff_text_layout_cache.insert(
                    key,
                    DiffTextLayoutCacheEntry {
                        layout,
                        last_used_epoch: epoch,
                    },
                );
            }
            None => {
                if let Some(entry) = self.diff_text_layout_cache.get_mut(&key) {
                    entry.last_used_epoch = epoch;
                }
            }
        }
    }

    /// Prune the layout cache if it has grown past the high-water mark.
    /// Call once per render frame (after bumping the epoch), **not** from
    /// the per-row `touch_diff_text_layout_cache` hot path.
    pub(in crate::view) fn prune_diff_text_layout_cache(&mut self) {
        if self.diff_text_layout_cache.len()
            <= DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES + DIFF_TEXT_LAYOUT_CACHE_PRUNE_OVERAGE
        {
            return;
        }

        let over_by = self
            .diff_text_layout_cache
            .len()
            .saturating_sub(DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES);
        if over_by == 0 {
            return;
        }

        let mut by_age: Vec<(u64, u64)> = self
            .diff_text_layout_cache
            .iter()
            .map(|(k, v)| (*k, v.last_used_epoch))
            .collect();
        by_age.sort_by_key(|(_, last_used)| *last_used);

        for (key, _) in by_age.into_iter().take(over_by) {
            self.diff_text_layout_cache.remove(&key);
        }
    }

    pub(in crate::view) fn diff_text_segments_cache_get(
        &self,
        key: usize,
        syntax_epoch: u64,
    ) -> Option<&CachedDiffStyledText> {
        versioned_cached_diff_styled_text_is_current(
            self.diff_text_segments_cache
                .get(key)
                .and_then(Option::as_ref),
            syntax_epoch,
        )
    }

    pub(in crate::view) fn file_diff_split_cache_key(
        &self,
        row_ix: usize,
        region: DiffTextRegion,
    ) -> Option<usize> {
        let base = row_ix.checked_mul(2)?;
        match region {
            DiffTextRegion::SplitLeft => Some(base),
            DiffTextRegion::SplitRight => base.checked_add(1),
            DiffTextRegion::Inline => None,
        }
    }

    pub(in crate::view) fn diff_text_segments_cache_set(
        &mut self,
        key: usize,
        syntax_epoch: u64,
        value: CachedDiffStyledText,
    ) -> &CachedDiffStyledText {
        if self.diff_text_segments_cache.len() <= key {
            self.diff_text_segments_cache.resize_with(key + 1, || None);
        }
        self.diff_text_segments_cache[key] = Some(VersionedCachedDiffStyledText {
            syntax_epoch,
            query_generation: 0,
            styled: value,
        });
        if self.diff_text_query_segments_cache.len() > key {
            self.diff_text_query_segments_cache[key] = None;
        }
        self.diff_text_segments_cache[key]
            .as_ref()
            .map(|entry| &entry.styled)
            .expect("just set")
    }

    /// Returns the current diff search query, or an empty `SharedString` if search is inactive.
    pub(in crate::view) fn diff_search_query_or_empty(&self) -> SharedString {
        if self.diff_search_active {
            self.diff_search_query.clone()
        } else {
            SharedString::default()
        }
    }

    /// Returns the syntax mode for patch diff views (non-full-document).
    /// Uses `Auto` for small diffs and `HeuristicOnly` for large ones.
    pub(in crate::view) fn patch_diff_syntax_mode(&self) -> rows::DiffSyntaxMode {
        if self.patch_diff_row_len() <= rows::MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
            rows::DiffSyntaxMode::Auto
        } else {
            rows::DiffSyntaxMode::HeuristicOnly
        }
    }

    pub(in crate::view) fn conflict_row_styling_enabled(&self) -> bool {
        !self.conflict_resolver.is_binary_conflict
    }

    pub(in crate::view) fn conflict_row_syntax_language(&self) -> Option<rows::DiffSyntaxLanguage> {
        self.conflict_resolver.conflict_syntax_language
    }

    pub(in crate::view) fn conflict_resolved_preview_render_syntax_language(
        &self,
    ) -> Option<rows::DiffSyntaxLanguage> {
        const MAX_RENDER_LINES: usize = 20_000;

        (self.conflict_resolved_preview_line_count <= MAX_RENDER_LINES)
            .then_some(self.conflict_resolved_preview_syntax_language)
            .flatten()
    }

    pub(in crate::view) fn worktree_preview_segments_cache_get(
        &self,
        key: usize,
    ) -> Option<&CachedDiffStyledText> {
        versioned_cached_diff_styled_text_is_current(
            self.worktree_preview_segments_cache.get(&key),
            self.worktree_preview_style_cache_epoch,
        )
    }

    pub(in crate::view) fn worktree_preview_segments_cache_set(
        &mut self,
        key: usize,
        value: CachedDiffStyledText,
    ) {
        self.worktree_preview_segments_cache.insert(
            key,
            VersionedCachedDiffStyledText {
                syntax_epoch: self.worktree_preview_style_cache_epoch,
                query_generation: 0,
                styled: value,
            },
        );
    }

    pub(in crate::view) fn conflict_resolved_preview_segments_cache_get(
        &self,
        key: usize,
    ) -> Option<&CachedDiffStyledText> {
        versioned_cached_diff_styled_text_is_current(
            self.conflict_resolved_preview_segments_cache.get(&key),
            self.conflict_resolved_preview_style_cache_epoch,
        )
    }

    pub(in crate::view) fn conflict_resolved_preview_segments_cache_set(
        &mut self,
        key: usize,
        value: CachedDiffStyledText,
    ) {
        self.conflict_resolved_preview_segments_cache.insert(
            key,
            VersionedCachedDiffStyledText {
                syntax_epoch: self.conflict_resolved_preview_style_cache_epoch,
                query_generation: 0,
                styled: value,
            },
        );
    }

    pub(in crate::view) fn is_file_diff_view_active(&self) -> bool {
        let Some(repo) = self.active_repo() else {
            return false;
        };
        self.file_diff_cache_repo_id == Some(repo.id)
            && self.file_diff_cache_rev == repo.diff_state.diff_file_rev
            && self.file_diff_cache_target == repo.diff_state.diff_target
            && self.file_diff_cache_path.is_some()
    }

    pub(in crate::view) fn is_file_image_diff_view_active(&self) -> bool {
        let Some(repo) = self.active_repo() else {
            return false;
        };
        self.file_image_diff_cache_repo_id == Some(repo.id)
            && self.file_image_diff_cache_rev == repo.diff_state.diff_file_rev
            && self.file_image_diff_cache_target == repo.diff_state.diff_target
            && self.file_image_diff_cache_path.is_some()
            && (self.file_image_diff_cache_old.is_some()
                || self.file_image_diff_cache_new.is_some()
                || self.file_image_diff_cache_old_svg_path.is_some()
                || self.file_image_diff_cache_new_svg_path.is_some())
    }

    pub(in crate::view) fn consume_suppress_click_after_drag(&mut self) -> bool {
        if self.diff_suppress_clicks_remaining > 0 {
            self.diff_suppress_clicks_remaining =
                self.diff_suppress_clicks_remaining.saturating_sub(1);
            return true;
        }
        false
    }

    pub(in crate::view) fn diff_visible_len(&self) -> usize {
        self.diff_visible_inline_map
            .as_ref()
            .map(|map| map.visible_len())
            .unwrap_or_else(|| self.diff_visible_indices.len())
    }

    pub(in crate::view) fn diff_mapped_ix_for_visible_ix(
        &self,
        visible_ix: usize,
    ) -> Option<usize> {
        if let Some(map) = self.diff_visible_inline_map.as_ref() {
            return map.src_ix_for_visible_ix(visible_ix);
        }
        self.diff_visible_indices.get(visible_ix).copied()
    }

    pub(super) fn diff_src_ixs_for_visible_ix(&self, visible_ix: usize) -> Vec<usize> {
        if self.is_file_diff_view_active() {
            return Vec::new();
        }
        let Some(mapped_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
            return Vec::new();
        };

        match self.diff_view {
            DiffViewMode::Inline => vec![mapped_ix],
            DiffViewMode::Split => {
                let Some(row) = self.patch_diff_split_row(mapped_ix) else {
                    return Vec::new();
                };
                match row {
                    PatchSplitRow::Raw { src_ix, .. } => vec![src_ix],
                    PatchSplitRow::Aligned {
                        old_src_ix,
                        new_src_ix,
                        ..
                    } => {
                        let mut out = Vec::with_capacity(2);
                        if let Some(ix) = old_src_ix {
                            out.push(ix);
                        }
                        if let Some(ix) = new_src_ix
                            && out.first().copied() != Some(ix)
                        {
                            out.push(ix);
                        }
                        out
                    }
                }
            }
        }
    }

    pub(super) fn diff_enclosing_hunk_src_ix(&self, src_ix: usize) -> Option<usize> {
        let src_ix = src_ix.min(self.patch_diff_row_len().saturating_sub(1));
        for ix in (0..=src_ix).rev() {
            let line = self.patch_diff_row(ix)?;
            if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
                && line.text.starts_with("diff --git ")
            {
                break;
            }
            if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk) {
                return Some(ix);
            }
        }
        None
    }

    pub(in crate::view) fn select_all_diff_text(&mut self) {
        // Markdown preview (both file preview and diff preview) uses
        // markdown preview row counts instead of source-text line counts.
        if self.is_markdown_preview_active() {
            let Some(count) = self.markdown_preview_row_count() else {
                return;
            };
            if count == 0 {
                return;
            }
            let region = if self.is_file_preview_active() {
                DiffTextRegion::Inline
            } else {
                match self.diff_view {
                    DiffViewMode::Inline => DiffTextRegion::Inline,
                    DiffViewMode::Split => self
                        .diff_text_head
                        .or(self.diff_text_anchor)
                        .map(|p| p.region)
                        .filter(|r| {
                            matches!(r, DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight)
                        })
                        .unwrap_or(DiffTextRegion::SplitLeft),
                }
            };
            let end_visible_ix = count - 1;
            let end_offset = self.diff_text_line_len_for_region(end_visible_ix, region);

            self.diff_text_selecting = false;
            self.diff_text_anchor = Some(DiffTextPos {
                visible_ix: 0,
                region,
                offset: 0,
            });
            self.diff_text_head = Some(DiffTextPos {
                visible_ix: end_visible_ix,
                region,
                offset: end_offset,
            });
            return;
        }

        if self.is_file_preview_active() {
            let Some(count) = self.worktree_preview_line_count() else {
                return;
            };
            if count == 0 {
                return;
            }
            let end_visible_ix = count - 1;
            let end_offset =
                self.diff_text_line_len_for_region(end_visible_ix, DiffTextRegion::Inline);

            self.diff_text_selecting = false;
            self.diff_text_anchor = Some(DiffTextPos {
                visible_ix: 0,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            self.diff_text_head = Some(DiffTextPos {
                visible_ix: end_visible_ix,
                region: DiffTextRegion::Inline,
                offset: end_offset,
            });
            return;
        }

        if self.diff_visible_len() == 0 {
            return;
        }

        let start_region = match self.diff_view {
            DiffViewMode::Inline => DiffTextRegion::Inline,
            DiffViewMode::Split => self
                .diff_text_head
                .or(self.diff_text_anchor)
                .map(|p| p.region)
                .filter(|r| matches!(r, DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight))
                .unwrap_or(DiffTextRegion::SplitLeft),
        };

        let end_visible_ix = self.diff_visible_len() - 1;
        let end_region = start_region;
        let end_offset = self.diff_text_line_len_for_region(end_visible_ix, end_region);

        self.diff_text_selecting = false;
        self.diff_text_anchor = Some(DiffTextPos {
            visible_ix: 0,
            region: start_region,
            offset: 0,
        });
        self.diff_text_head = Some(DiffTextPos {
            visible_ix: end_visible_ix,
            region: end_region,
            offset: end_offset,
        });
    }

    pub(super) fn select_diff_text_rows_range(
        &mut self,
        start_visible_ix: usize,
        end_visible_ix: usize,
        region: DiffTextRegion,
    ) {
        let list_len = self.diff_visible_len();
        if list_len == 0 {
            return;
        }

        let a = start_visible_ix.min(list_len - 1);
        let b = end_visible_ix.min(list_len - 1);
        let (a, b) = if a <= b { (a, b) } else { (b, a) };

        let region = match self.diff_view {
            DiffViewMode::Inline => DiffTextRegion::Inline,
            DiffViewMode::Split => match region {
                DiffTextRegion::SplitRight => DiffTextRegion::SplitRight,
                _ => DiffTextRegion::SplitLeft,
            },
        };
        let start_region = region;
        let end_region = region;

        let end_offset = self.diff_text_line_len_for_region(b, end_region);

        self.diff_text_selecting = false;
        self.diff_text_anchor = Some(DiffTextPos {
            visible_ix: a,
            region: start_region,
            offset: 0,
        });
        self.diff_text_head = Some(DiffTextPos {
            visible_ix: b,
            region: end_region,
            offset: end_offset,
        });

        // Double-click produces two click events; suppress both.
        self.diff_suppress_clicks_remaining = 2;
    }

    pub(in crate::view) fn double_click_select_diff_text(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        kind: DiffClickKind,
    ) {
        // Markdown preview: select the full row on double-click.
        if self.is_markdown_preview_active() {
            let Some(count) = self.markdown_preview_row_count() else {
                return;
            };
            if count == 0 {
                return;
            }
            let effective_region = if self.is_file_preview_active() {
                DiffTextRegion::Inline
            } else {
                region
            };
            let visible_ix = visible_ix.min(count - 1);
            let end_offset = self.diff_text_line_len_for_region(visible_ix, effective_region);
            self.diff_text_selecting = false;
            self.diff_text_anchor = Some(DiffTextPos {
                visible_ix,
                region: effective_region,
                offset: 0,
            });
            self.diff_text_head = Some(DiffTextPos {
                visible_ix,
                region: effective_region,
                offset: end_offset,
            });
            self.diff_suppress_clicks_remaining = 2;
            return;
        }

        if self.is_file_preview_active() {
            let Some(count) = self.worktree_preview_line_count() else {
                return;
            };
            if count == 0 {
                return;
            }
            let visible_ix = visible_ix.min(count - 1);
            let end_offset = self.diff_text_line_len_for_region(visible_ix, DiffTextRegion::Inline);
            self.diff_text_selecting = false;
            self.diff_text_anchor = Some(DiffTextPos {
                visible_ix,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            self.diff_text_head = Some(DiffTextPos {
                visible_ix,
                region: DiffTextRegion::Inline,
                offset: end_offset,
            });

            // Double-click produces two click events; suppress both.
            self.diff_suppress_clicks_remaining = 2;
            return;
        }

        let list_len = self.diff_visible_len();
        if list_len == 0 {
            return;
        }
        let visible_ix = visible_ix.min(list_len - 1);

        // File-diff view doesn't have file/hunk header blocks; treat as row selection.
        if self.is_file_diff_view_active() {
            self.select_diff_text_rows_range(visible_ix, visible_ix, region);
            return;
        }

        let end = match self.diff_view {
            DiffViewMode::Inline => match kind {
                DiffClickKind::Line => visible_ix,
                DiffClickKind::HunkHeader => self
                    .diff_next_boundary_visible_ix(visible_ix, |src_ix| {
                        self.patch_diff_row(src_ix).is_some_and(|line| {
                            matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk)
                                || (matches!(
                                    line.kind,
                                    gitcomet_core::domain::DiffLineKind::Header
                                ) && line.text.starts_with("diff --git "))
                        })
                    })
                    .unwrap_or(list_len - 1),
                DiffClickKind::FileHeader => self
                    .diff_next_boundary_visible_ix(visible_ix, |src_ix| {
                        self.patch_diff_row(src_ix).is_some_and(|line| {
                            matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
                                && line.text.starts_with("diff --git ")
                        })
                    })
                    .unwrap_or(list_len - 1),
            },
            DiffViewMode::Split => match kind {
                DiffClickKind::Line => visible_ix,
                DiffClickKind::HunkHeader => self
                    .split_next_boundary_visible_ix(visible_ix, |row| {
                        matches!(
                            row,
                            PatchSplitRow::Raw {
                                click_kind: DiffClickKind::HunkHeader | DiffClickKind::FileHeader,
                                ..
                            }
                        )
                    })
                    .unwrap_or(list_len - 1),
                DiffClickKind::FileHeader => self
                    .split_next_boundary_visible_ix(visible_ix, |row| {
                        matches!(
                            row,
                            PatchSplitRow::Raw {
                                click_kind: DiffClickKind::FileHeader,
                                ..
                            }
                        )
                    })
                    .unwrap_or(list_len - 1),
            },
        };

        self.select_diff_text_rows_range(visible_ix, end, region);
    }

    pub(super) fn split_next_boundary_visible_ix(
        &self,
        from_visible_ix: usize,
        is_boundary: impl Fn(&PatchSplitRow) -> bool,
    ) -> Option<usize> {
        let visible_len = self.diff_visible_len();
        let from_visible_ix = from_visible_ix.min(visible_len.saturating_sub(1));
        for visible_ix in (from_visible_ix + 1)..visible_len {
            let row_ix = self.diff_mapped_ix_for_visible_ix(visible_ix)?;
            let row = self.patch_diff_split_row(row_ix)?;
            if is_boundary(&row) {
                return Some(visible_ix.saturating_sub(1));
            }
        }
        None
    }

    pub(super) fn diff_next_boundary_visible_ix(
        &self,
        from_visible_ix: usize,
        is_boundary: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        let visible_len = self.diff_visible_len();
        let from_visible_ix = from_visible_ix.min(visible_len.saturating_sub(1));
        for visible_ix in (from_visible_ix + 1)..visible_len {
            let src_ix = self.diff_mapped_ix_for_visible_ix(visible_ix)?;
            if is_boundary(src_ix) {
                return Some(visible_ix.saturating_sub(1));
            }
        }
        None
    }

    fn diff_split_scroll_handles(&self) -> [ScrollHandle; 2] {
        [
            uniform_list_base_handle(&self.diff_scroll),
            uniform_list_base_handle(&self.diff_split_right_scroll),
        ]
    }

    fn conflict_preview_scroll_handles(&self) -> [ScrollHandle; 4] {
        [
            uniform_list_base_handle(&self.conflict_resolver_diff_scroll),
            uniform_list_base_handle(&self.conflict_preview_ours_scroll),
            uniform_list_base_handle(&self.conflict_preview_theirs_scroll),
            uniform_list_base_handle(&self.conflict_resolved_preview_scroll),
        ]
    }

    pub(in crate::view) fn sync_diff_split_scroll(&mut self) {
        let handles = self.diff_split_scroll_handles();
        maybe_sync_synced_scroll_offsets(
            &handles,
            &mut self.diff_split_last_synced_y,
            SyncedScrollAxis::Vertical,
            self.diff_scroll_sync,
        );
        maybe_sync_synced_scroll_offsets(
            &handles,
            &mut self.diff_split_last_synced_x,
            SyncedScrollAxis::Horizontal,
            self.diff_scroll_sync,
        );
    }

    pub(in crate::view) fn sync_conflict_preview_scroll(&mut self) {
        let handles = self.conflict_preview_scroll_handles();
        maybe_sync_synced_scroll_offsets(
            &handles,
            &mut self.conflict_preview_last_synced_y,
            SyncedScrollAxis::Vertical,
            self.diff_scroll_sync,
        );
        maybe_sync_synced_scroll_offsets(
            &handles,
            &mut self.conflict_preview_last_synced_x,
            SyncedScrollAxis::Horizontal,
            self.diff_scroll_sync,
        );
    }

    pub(in crate::view) fn sync_conflict_resolved_output_gutter_scroll(&mut self) {
        let handles = [
            uniform_list_base_handle(&self.conflict_resolved_preview_gutter_scroll),
            uniform_list_base_handle(&self.conflict_resolved_preview_scroll),
        ];
        sync_synced_scroll_offsets(
            &handles,
            &mut self.conflict_resolved_preview_gutter_last_synced_y,
            SyncedScrollAxis::Vertical,
        );
    }

    pub(in crate::view) fn main_pane_content_width(&self, cx: &mut gpui::Context<Self>) -> Pixels {
        let _ = cx;

        super::pane_content_width_for_layout(
            self.last_window_size.width,
            self.layout_sidebar_render_width,
            self.layout_details_render_width,
            self.layout_sidebar_collapsed,
            self.layout_details_collapsed,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_raw_scroll_y_limits_scroll_without_flipping_direction() {
        assert_eq!(clamp_raw_scroll_y(px(-180.0), px(120.0)), px(-120.0));
        assert_eq!(clamp_raw_scroll_y(px(180.0), px(120.0)), px(120.0));
        assert_eq!(clamp_raw_scroll_y(px(-40.0), px(120.0)), px(-40.0));
    }

    #[test]
    fn synced_scroll_offsets_keep_longer_pane_as_master_after_shorter_clamps() {
        let targets = compute_synced_scroll_offsets(
            [px(-100.0), px(-500.0)],
            [px(100.0), px(500.0)],
            [px(-90.0), px(-90.0)],
            1,
        );

        assert_eq!(targets, [px(-100.0), px(-500.0)]);
    }

    #[test]
    fn synced_scroll_offsets_follow_shorter_pane_when_user_scrolled_it() {
        let targets = compute_synced_scroll_offsets(
            [px(-100.0), px(-320.0)],
            [px(100.0), px(500.0)],
            [px(-80.0), px(-320.0)],
            1,
        );

        assert_eq!(targets, [px(-100.0), px(-100.0)]);
    }

    #[test]
    fn synced_scroll_offsets_support_four_panes_when_output_is_scrolled() {
        let targets = compute_synced_scroll_offsets(
            [px(-100.0), px(-100.0), px(-100.0), px(-320.0)],
            [px(100.0), px(100.0), px(100.0), px(500.0)],
            [px(-100.0), px(-100.0), px(-100.0), px(-80.0)],
            3,
        );

        assert_eq!(targets, [px(-100.0), px(-100.0), px(-100.0), px(-320.0)]);
    }
}
