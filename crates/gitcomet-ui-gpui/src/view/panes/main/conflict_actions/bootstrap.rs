//! Pure phases of the conflict resolver bootstrap, its mergetool trace
//! context, and the conflict source fingerprint.

use super::*;

/// Render the current semantic plan decisions into the marker/text projection
/// consumed by the resolver UI. This differs from `marker_projection`, which
/// intentionally remains the immutable structural baseline used to detect
/// protected worktree edits.
pub(super) fn conflict_session_plan_projection(
    session: &gitcomet_core::conflict_session::ConflictSession,
) -> Option<(Arc<str>, Vec<usize>)> {
    let mut projection = session.merge_plan.clone()?;
    // ConflictRegion remains the compatibility/autosolve model for original
    // marker blocks. Keep those blocks present in this structural projection;
    // their live choices are applied to parsed blocks below. Plan-only deltas
    // retain their current selection, which is the structural gap this path
    // closes.
    for block_index in &session.region_plan_blocks {
        projection.replace_selection(*block_index, gitcomet_core::merge::OrderedSelection::new());
    }
    let projected_plan_blocks = projection.unresolved_blocks.clone();
    let options = gitcomet_core::merge::MergeOptions {
        style: if projection.has_base() {
            gitcomet_core::merge::ConflictStyle::Diff3
        } else {
            gitcomet_core::merge::ConflictStyle::Merge
        },
        ..Default::default()
    };
    Some((
        Arc::from(gitcomet_core::merge::render_merge_plan(&projection, &options).output),
        projected_plan_blocks,
    ))
}

/// Pre-computed side stats for mergetool trace events.  Computing these once
/// avoids redundant full-text newline counts across the ~10 trace events per
/// bootstrap.  When tracing is disabled, stats are left at `Default` so the
/// newline counting never runs.
pub(super) struct MergetoolTraceContext {
    path: PathBuf,
    base: MergetoolTraceSideStats,
    ours: MergetoolTraceSideStats,
    theirs: MergetoolTraceSideStats,
    current: MergetoolTraceSideStats,
}

impl MergetoolTraceContext {
    fn new(
        path: PathBuf,
        base_text: &str,
        ours_text: &str,
        theirs_text: &str,
        current_text: Option<&str>,
    ) -> Self {
        if !mergetool_trace::is_enabled() {
            return Self {
                path,
                base: MergetoolTraceSideStats::default(),
                ours: MergetoolTraceSideStats::default(),
                theirs: MergetoolTraceSideStats::default(),
                current: MergetoolTraceSideStats::default(),
            };
        }
        Self {
            path,
            base: MergetoolTraceSideStats::from_text(Some(base_text)),
            ours: MergetoolTraceSideStats::from_text(Some(ours_text)),
            theirs: MergetoolTraceSideStats::from_text(Some(theirs_text)),
            current: MergetoolTraceSideStats::from_text(current_text),
        }
    }

    fn event(&self, stage: MergetoolTraceStage, started: Instant) -> MergetoolTraceEvent {
        MergetoolTraceEvent::new(stage, Some(self.path.clone()), started.elapsed())
            .with_base(self.base)
            .with_ours(self.ours)
            .with_theirs(self.theirs)
            .with_current(self.current)
    }

    pub(super) fn bootstrap_event(
        &self,
        stage: MergetoolTraceStage,
        started: Instant,
        decisions: MergetoolBootstrapTraceDecisions,
    ) -> MergetoolTraceEvent {
        decisions.apply_to_event(self.event(stage, started))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct MergetoolBootstrapTraceDecisions {
    rendering_mode: Option<MergetoolTraceRenderingMode>,
    whole_block_diff_ran: Option<bool>,
    full_output_generated: Option<bool>,
    full_syntax_parse_requested: Option<bool>,
}

impl MergetoolBootstrapTraceDecisions {
    fn apply_to_event(self, event: MergetoolTraceEvent) -> MergetoolTraceEvent {
        event
            .with_rendering_mode(self.rendering_mode)
            .with_whole_block_diff_ran(self.whole_block_diff_ran)
            .with_full_output_generated(self.full_output_generated)
            .with_full_syntax_parse_requested(self.full_syntax_parse_requested)
    }
}

fn trace_rendering_mode(
    mode: conflict_resolver::ConflictRenderingMode,
) -> MergetoolTraceRenderingMode {
    match mode {
        conflict_resolver::ConflictRenderingMode::EagerSmallFile => {
            MergetoolTraceRenderingMode::EagerSmallFile
        }
        conflict_resolver::ConflictRenderingMode::StreamedLargeFile => {
            MergetoolTraceRenderingMode::StreamedLargeFile
        }
    }
}

const CONFLICT_SOURCE_FINGERPRINT_SAMPLE_COUNT: usize = 8;
const CONFLICT_SOURCE_FINGERPRINT_WINDOW_BYTES: usize = 256;

// This is a lightweight UI cache key, not a cryptographic hash. Domain labels
// keep the text/bytes/none cases distinct without opaque numeric seeds.
fn sampled_content_fingerprint(bytes: &[u8], domain: &str) -> u64 {
    use std::hash::Hasher;

    let mut hasher = FxHasher::default();
    hasher.write_usize(domain.len());
    hasher.write(domain.as_bytes());
    hasher.write_usize(bytes.len());
    if bytes.is_empty() {
        return hasher.finish();
    }

    let window_len = CONFLICT_SOURCE_FINGERPRINT_WINDOW_BYTES.min(bytes.len());
    let sample_count = if bytes.len() <= window_len {
        1
    } else {
        CONFLICT_SOURCE_FINGERPRINT_SAMPLE_COUNT
    };
    let max_start = bytes.len().saturating_sub(window_len);
    let denominator = sample_count.saturating_sub(1).max(1);
    for sample_ix in 0..sample_count {
        let start = if sample_count == 1 {
            0
        } else {
            sample_ix.saturating_mul(max_start) / denominator
        };
        hasher.write_usize(start);
        hasher.write(&bytes[start..start.saturating_add(window_len)]);
    }
    hasher.finish()
}

fn shared_text_fingerprint(text: &Option<std::sync::Arc<str>>) -> u64 {
    let Some(text) = text.as_ref() else {
        return sampled_content_fingerprint(&[], "conflict-source:text:none");
    };
    sampled_content_fingerprint(text.as_bytes(), "conflict-source:text")
}

fn shared_bytes_fingerprint(bytes: &Option<std::sync::Arc<[u8]>>) -> u64 {
    let Some(bytes) = bytes.as_ref() else {
        return sampled_content_fingerprint(&[], "conflict-source:bytes:none");
    };
    sampled_content_fingerprint(bytes.as_ref(), "conflict-source:bytes")
}

pub(super) fn conflict_file_source_fingerprint(file: &gitcomet_state::model::ConflictFile) -> u64 {
    let side_fingerprint = |text: &Option<std::sync::Arc<str>>,
                            bytes: &Option<std::sync::Arc<[u8]>>,
                            side_domain: &str| {
        let value = if text.is_some() {
            shared_text_fingerprint(text)
        } else {
            shared_bytes_fingerprint(bytes)
        };
        sampled_content_fingerprint(&value.to_le_bytes(), side_domain)
    };

    let mut acc = sampled_content_fingerprint(&[], "conflict-source:file");
    for (side_domain, text, bytes) in [
        ("conflict-source:side:base", &file.base, &file.base_bytes),
        ("conflict-source:side:ours", &file.ours, &file.ours_bytes),
        (
            "conflict-source:side:theirs",
            &file.theirs,
            &file.theirs_bytes,
        ),
        (
            "conflict-source:side:current",
            &file.current,
            &file.current_bytes,
        ),
    ] {
        acc = acc.rotate_left(13) ^ side_fingerprint(text, bytes, side_domain);
    }
    acc
}

/// Texts, snapshots, and side sizes a text-conflict bootstrap reads from the
/// loaded file and its session.
pub(super) struct ConflictBootstrapSources<'a> {
    pub(super) current_text: Option<Arc<str>>,
    pub(super) plan_projection: Option<(Arc<str>, Vec<usize>)>,
    pub(super) marker_snapshot: Option<Arc<str>>,
    pub(super) output_is_protected: bool,
    pub(super) base_text: &'a str,
    pub(super) ours_text: &'a str,
    pub(super) theirs_text: &'a str,
    pub(super) trace_ctx: MergetoolTraceContext,
    pub(super) needs_full_side_texts: bool,
    pub(super) full_text_plan_upgrade_expected: bool,
    pub(super) three_way_base_len: usize,
    pub(super) three_way_ours_len: usize,
    pub(super) three_way_theirs_len: usize,
    pub(super) three_way_side_max_len: usize,
}

impl<'a> ConflictBootstrapSources<'a> {
    pub(super) fn gather(
        file: &'a gitcomet_state::model::ConflictFile,
        session: Option<&gitcomet_core::conflict_session::ConflictSession>,
        trace_path: PathBuf,
        conflict_strategy: Option<gitcomet_core::conflict_session::ConflictResolverStrategy>,
    ) -> Self {
        let current_text = session
            .and_then(|session| session.current.as_ref()?.as_shared_text().cloned())
            .or_else(|| file.current.clone());
        let structural_marker_snapshot = session
            .and_then(|session| session.marker_projection.clone())
            .or_else(|| current_text.clone());
        let plan_projection = session.and_then(conflict_session_plan_projection);
        let marker_snapshot = plan_projection
            .as_ref()
            .map(|(text, _)| Arc::clone(text))
            .or_else(|| structural_marker_snapshot.clone());
        let output_is_protected = worktree_output_requires_protection(
            current_text.as_deref(),
            structural_marker_snapshot.as_deref(),
            file.base.as_deref(),
            file.ours.as_deref(),
            file.theirs.as_deref(),
        );
        let current_text_ref = current_text.as_deref();
        let base_text = file.base.as_deref().unwrap_or("");
        let ours_text = file.ours.as_deref().unwrap_or("");
        let theirs_text = file.theirs.as_deref().unwrap_or("");
        let trace_ctx = MergetoolTraceContext::new(
            trace_path,
            base_text,
            ours_text,
            theirs_text,
            current_text_ref,
        );
        // True when the fast CurrentOnly first paint is showing: no side text
        // has been loaded yet (a Full load provides at least one stage for
        // real conflicts).
        let needs_full_side_texts =
            file.base.is_none() && file.ours.is_none() && file.theirs.is_none();
        const FULL_LOAD_UPGRADE_MAX_CURRENT_LINES: usize = 100_000;
        let full_text_plan_upgrade_expected = needs_full_side_texts
            && matches!(
                conflict_strategy,
                Some(gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver)
            )
            && current_text
                .as_deref()
                .is_some_and(|text| count_newlines(text) < FULL_LOAD_UPGRADE_MAX_CURRENT_LINES);
        let three_way_base_len = if base_text.is_empty() {
            0
        } else {
            count_newlines(base_text).saturating_add(1)
        };
        let three_way_ours_len = if ours_text.is_empty() {
            0
        } else {
            count_newlines(ours_text).saturating_add(1)
        };
        let three_way_theirs_len = if theirs_text.is_empty() {
            0
        } else {
            count_newlines(theirs_text).saturating_add(1)
        };
        let three_way_side_max_len = three_way_base_len
            .max(three_way_ours_len)
            .max(three_way_theirs_len);
        Self {
            current_text,
            plan_projection,
            marker_snapshot,
            output_is_protected,
            base_text,
            ours_text,
            theirs_text,
            trace_ctx,
            needs_full_side_texts,
            full_text_plan_upgrade_expected,
            three_way_base_len,
            three_way_ours_len,
            three_way_theirs_len,
            three_way_side_max_len,
        }
    }
}

/// Parsed markers, rendering mode, and aligned row space of a bootstrap.
pub(super) struct ConflictMarkerLayout {
    pub(super) marker_segments: Vec<conflict_resolver::ConflictSegment>,
    pub(super) conflict_region_marker_has_base: Vec<bool>,
    pub(super) rendering_mode: conflict_resolver::ConflictRenderingMode,
    pub(super) three_way_aligned: conflict_resolver::ThreeWayAlignedMap,
    pub(super) three_way_len: usize,
    pub(super) trace_decisions: MergetoolBootstrapTraceDecisions,
}

/// Parse the marker snapshot, choose the rendering mode, and align the sides
/// into one row space.
pub(super) fn bootstrap_marker_layout(
    marker_snapshot: &Option<Arc<str>>,
    session: Option<&gitcomet_core::conflict_session::ConflictSession>,
    base_text: &str,
    ours_text: &str,
    theirs_text: &str,
    three_way_side_max_len: usize,
    conflict_syntax_language: Option<rows::DiffSyntaxLanguage>,
    trace_ctx: &MergetoolTraceContext,
) -> ConflictMarkerLayout {
    let marker_parse_started = Instant::now();
    let marker_segments = if let Some(cur) = marker_snapshot.clone() {
        conflict_resolver::parse_conflict_markers_shared_nonempty(cur)
    } else {
        Vec::new()
    };
    let conflict_region_marker_has_base = marker_segments
        .iter()
        .filter_map(|segment| match segment {
            conflict_resolver::ConflictSegment::Block(block) => Some(block.base.is_some()),
            conflict_resolver::ConflictSegment::Text(_) => None,
        })
        .collect();
    let rendering_mode =
        conflict_resolver::select_conflict_rendering_mode(&marker_segments, three_way_side_max_len);
    // section 30 aligned row space: compute the kdiff3-style alignment once per
    // bootstrap (side texts are immutable for the session). Files without
    // a base version (e.g. both-added conflicts) align ours↔theirs
    // directly with empty base ranges, so the two-way view gets the same
    // whole-file row space. Fall back to the identity map when side texts
    // are unavailable (CurrentOnly load) or when the alignment diff would
    // be impractical (large files whose sides no longer share most of
    // their lines — whole-file conflicts make Myers effectively
    // quadratic).
    let three_way_aligned = if let Some(plan) =
        session.and_then(|session| session.merge_plan.as_ref())
    {
        conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_merge_plan(plan),
        )
    } else if !base_text.is_empty()
        && !ours_text.is_empty()
        && !theirs_text.is_empty()
        && conflict_resolver::three_way_alignment_is_practical(base_text, ours_text, theirs_text)
    {
        conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                base_text,
                ours_text,
                theirs_text,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        )
    } else if base_text.is_empty()
        && !ours_text.is_empty()
        && !theirs_text.is_empty()
        && conflict_resolver::two_way_alignment_is_practical(ours_text, theirs_text)
    {
        conflict_resolver::ThreeWayAlignedMap::from_alignment(&gitcomet_core::merge::align_two_way(
            ours_text,
            theirs_text,
            gitcomet_core::merge::DiffAlgorithm::Myers,
        ))
    } else {
        conflict_resolver::ThreeWayAlignedMap::default()
    };
    let three_way_len = if three_way_aligned.is_identity() {
        three_way_side_max_len
    } else {
        three_way_aligned.aligned_len()
    };
    let full_syntax_parse_requested = conflict_syntax_language.is_some()
        && [base_text, ours_text, theirs_text]
            .into_iter()
            .any(|text| !text.is_empty());
    let trace_decisions = MergetoolBootstrapTraceDecisions {
        rendering_mode: Some(trace_rendering_mode(rendering_mode)),
        full_syntax_parse_requested: Some(full_syntax_parse_requested),
        ..Default::default()
    };
    mergetool_trace::record_with(|| {
        trace_ctx
            .bootstrap_event(
                MergetoolTraceStage::ParseConflictMarkers,
                marker_parse_started,
                trace_decisions,
            )
            .with_conflict_block_count(Some(conflict_resolver::conflict_count(&marker_segments)))
    });
    ConflictMarkerLayout {
        marker_segments,
        conflict_region_marker_has_base,
        rendering_mode,
        three_way_aligned,
        three_way_len,
        trace_decisions,
    }
}

/// Display-block to session-region mapping and the aligned ranges built on it.
pub(super) struct ConflictSessionMapping {
    pub(super) original_region_aligned_ranges: Vec<Option<std::ops::Range<usize>>>,
    pub(super) conflict_region_indices: Vec<usize>,
    pub(super) display_plan_block_indices: Vec<usize>,
    pub(super) merge_plan_aligned_conflict_ranges: Option<Vec<std::ops::Range<usize>>>,
}

/// Fill 2-way blocks' bases from the ancestor and apply the session's region
/// resolutions to the parsed markers.
pub(super) fn apply_bootstrap_session_resolutions(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    file: &gitcomet_state::model::ConflictFile,
    session: Option<&gitcomet_core::conflict_session::ConflictSession>,
    plan_projection: &Option<(Arc<str>, Vec<usize>)>,
    three_way_aligned: &conflict_resolver::ThreeWayAlignedMap,
    three_way_base_len: usize,
    three_way_ours_len: usize,
    three_way_theirs_len: usize,
) -> ConflictSessionMapping {
    // When conflict markers are 2-way (no base section), populate block.base
    // from the git ancestor file so "A (base)" picks work.
    if let Some(base_text) = file.base.clone() {
        conflict_resolver::populate_block_bases_from_shared_ancestor(marker_segments, base_text);
    }
    let original_display_aligned_ranges =
        conflict_resolver::project_conflict_ranges_to_aligned_rows(
            marker_segments,
            three_way_aligned,
            [three_way_base_len, three_way_ours_len, three_way_theirs_len],
        );
    let original_region_aligned_ranges = session
        .map(|session| {
            conflict_resolver::conflict_nav_region_aligned_ranges(
                session,
                &original_display_aligned_ranges,
            )
        })
        .unwrap_or_else(|| {
            original_display_aligned_ranges
                .iter()
                .cloned()
                .map(Some)
                .collect()
        });
    let mut conflict_region_indices =
        conflict_resolver::sequential_conflict_region_indices(marker_segments);
    let mut display_plan_block_indices = Vec::new();
    if let Some(session) = session {
        if let Some((_, projected_plan_blocks)) = plan_projection.as_ref()
            && let Some(applied) =
                conflict_resolver::apply_plan_session_region_resolutions_with_index_map(
                    marker_segments,
                    session,
                    projected_plan_blocks,
                )
        {
            conflict_region_indices = applied.block_region_indices;
            display_plan_block_indices = applied.block_plan_indices;
        } else {
            let applied = conflict_resolver::apply_session_region_resolutions_with_index_map(
                marker_segments,
                &session.regions,
            );
            conflict_region_indices = applied.block_region_indices;
        }
    }
    let merge_plan_aligned_conflict_ranges = session.and_then(|session| {
        conflict_resolver::merge_plan_aligned_conflict_ranges(
            session,
            &conflict_region_indices,
            &display_plan_block_indices,
        )
    });
    ConflictSessionMapping {
        original_region_aligned_ranges,
        conflict_region_indices,
        display_plan_block_indices,
        merge_plan_aligned_conflict_ranges,
    }
}

/// The bootstrap's resolved output: the protected worktree text, a streamed
/// projection for large files, or the fully generated text.
pub(super) fn generate_bootstrap_resolved_output(
    output_is_protected: bool,
    current_text: &Option<Arc<str>>,
    rendering_mode: conflict_resolver::ConflictRenderingMode,
    marker_segments: &[conflict_resolver::ConflictSegment],
    marker_snapshot: &Option<Arc<str>>,
    file: &gitcomet_state::model::ConflictFile,
    trace_ctx: &MergetoolTraceContext,
    trace_decisions: &mut MergetoolBootstrapTraceDecisions,
    conflict_block_count: usize,
) -> (
    Option<conflict_resolver::ResolvedOutputText>,
    Option<conflict_resolver::ResolvedOutputProjection>,
    Option<usize>,
) {
    let resolved_started = Instant::now();
    let (resolved_output_text, streamed_output_projection) = if output_is_protected {
        trace_decisions.full_output_generated = Some(false);
        (
            current_text
                .clone()
                .map(conflict_resolver::ResolvedOutputText::Shared),
            None,
        )
    } else if rendering_mode.is_streamed_large_file() && !marker_segments.is_empty() {
        trace_decisions.full_output_generated = Some(false);
        (
            None,
            Some(conflict_resolver::ResolvedOutputProjection::from_segments(
                marker_segments,
            )),
        )
    } else {
        trace_decisions.full_output_generated = Some(true);
        (
            Some(conflict_resolver::bootstrap_resolved_output_text(
                marker_segments,
                marker_snapshot.as_ref(),
                file.ours.as_ref(),
                file.theirs.as_ref(),
            )),
            None,
        )
    };
    let resolved_line_count = if mergetool_trace::is_enabled() {
        streamed_output_projection
            .as_ref()
            .map(conflict_resolver::ResolvedOutputProjection::len)
            .or_else(|| {
                resolved_output_text
                    .as_ref()
                    .map(|resolved| resolved.line_count())
            })
    } else {
        None
    };
    mergetool_trace::record_with(|| {
        trace_ctx
            .bootstrap_event(
                MergetoolTraceStage::GenerateResolvedText,
                resolved_started,
                *trace_decisions,
            )
            .with_conflict_block_count(Some(conflict_block_count))
            .with_resolved_output_line_count(resolved_line_count)
    });
    (
        resolved_output_text,
        streamed_output_projection,
        resolved_line_count,
    )
}

/// Side texts as `SharedString`s plus their lazily built line starts.
pub(super) fn bootstrap_three_way_sides(
    file: &gitcomet_state::model::ConflictFile,
    three_way_base_len: usize,
    three_way_ours_len: usize,
    three_way_theirs_len: usize,
) -> (
    ThreeWaySides<SharedString>,
    ThreeWaySides<DeferredLineStarts>,
) {
    // Use `SharedString::from` (not `SharedString::new`) so the existing
    // `Arc<str>` is passed through to the `SmolStr` backing without a fresh
    // allocation. `SharedString::new` always copies via `SmolStr::new`,
    // whereas `From<Arc<str>>` reuses the heap allocation for non-inline
    // strings.
    let three_way_text = ThreeWaySides {
        base: file
            .base
            .clone()
            .map(SharedString::from)
            .unwrap_or_default(),
        ours: file
            .ours
            .clone()
            .map(SharedString::from)
            .unwrap_or_default(),
        theirs: file
            .theirs
            .clone()
            .map(SharedString::from)
            .unwrap_or_default(),
    };
    let three_way_line_starts: ThreeWaySides<DeferredLineStarts> = ThreeWaySides {
        base: DeferredLineStarts::with_line_count(three_way_base_len),
        ours: DeferredLineStarts::with_line_count(three_way_ours_len),
        theirs: DeferredLineStarts::with_line_count(three_way_theirs_len),
    };
    (three_way_text, three_way_line_starts)
}

/// The streamed split row index as the resolver's mode state, with its row
/// count.
pub(super) fn bootstrap_split_rows(
    marker_segments: &[conflict_resolver::ConflictSegment],
    trace_ctx: &MergetoolTraceContext,
    trace_decisions: &mut MergetoolBootstrapTraceDecisions,
    conflict_block_count: usize,
) -> (ConflictModeState, usize) {
    // Conflicts now always use the streamed split index. Bootstrap only
    // records the lazy row count here; visible projections are rebuilt
    // after state construction.
    let diff_rows_started = Instant::now();
    let index = conflict_resolver::ConflictSplitRowIndex::new(
        marker_segments,
        conflict_resolver::BLOCK_LOCAL_DIFF_CONTEXT_LINES,
    );
    trace_decisions.whole_block_diff_ran = Some(false);
    let diff_row_count = index.total_rows();
    mergetool_trace::record_with(|| {
        trace_ctx
            .bootstrap_event(
                MergetoolTraceStage::SideBySideRows,
                diff_rows_started,
                *trace_decisions,
            )
            .with_conflict_block_count(Some(conflict_block_count))
            .with_diff_row_count(Some(diff_row_count))
    });
    let mode_state = ConflictModeState::Streamed(StreamedConflictState {
        split_row_index: index,
        ..StreamedConflictState::default()
    });
    (mode_state, diff_row_count)
}

/// Three-way and two-way word highlights over the aligned row space.
pub(super) fn bootstrap_word_highlights(
    three_way_aligned: &conflict_resolver::ThreeWayAlignedMap,
    base_text: &str,
    ours_text: &str,
    theirs_text: &str,
    three_way_line_starts: &ThreeWaySides<DeferredLineStarts>,
    trace_ctx: &MergetoolTraceContext,
    trace_decisions: MergetoolBootstrapTraceDecisions,
    conflict_block_count: usize,
    diff_row_count: usize,
) -> (
    ThreeWaySides<conflict_resolver::WordHighlights>,
    FxHashMap<usize, conflict_resolver::TwoWayWordHighlightPair>,
) {
    // section 30 R11: the aligned row space gives exact base↔side line pairs, so
    // word highlights come from a row-capped per-line word diff instead of
    // the old whole-file side_by_side/myers pass (which the streamed path
    // had to skip). Identity maps (no side texts / impractical alignment)
    // and both-added conflicts (two-way highlight path) stay empty.
    let three_way_word_highlights_started = Instant::now();
    let three_way_word_highlights = if !three_way_aligned.is_identity() && !base_text.is_empty() {
        let (wh_base, wh_ours, wh_theirs) =
            conflict_resolver::compute_aligned_three_way_word_highlights(
                three_way_aligned,
                base_text,
                three_way_line_starts.base.starts(base_text),
                ours_text,
                three_way_line_starts.ours.starts(ours_text),
                theirs_text,
                three_way_line_starts.theirs.starts(theirs_text),
            );
        ThreeWaySides {
            base: wh_base,
            ours: wh_ours,
            theirs: wh_theirs,
        }
    } else {
        ThreeWaySides::default()
    };
    mergetool_trace::record_with(|| {
        trace_ctx
            .bootstrap_event(
                MergetoolTraceStage::ComputeThreeWayWordHighlights,
                three_way_word_highlights_started,
                trace_decisions,
            )
            .with_conflict_block_count(Some(conflict_block_count))
    });

    // section 30 R11: aligned two-way (ours↔theirs) word highlights, computed
    // once here and shared by both diff columns, replacing the old
    // per-render/per-column inline word diff. Independent of base, so this
    // runs even for both-added conflicts where the three-way pass stays empty.
    let two_way_word_highlights_started = Instant::now();
    let two_way_aligned_word_highlights = if three_way_aligned.is_identity() {
        FxHashMap::default()
    } else {
        conflict_resolver::compute_aligned_two_way_word_highlights(
            three_way_aligned,
            ours_text,
            three_way_line_starts.ours.starts(ours_text),
            theirs_text,
            three_way_line_starts.theirs.starts(theirs_text),
        )
    };
    mergetool_trace::record_with(|| {
        trace_ctx
            .bootstrap_event(
                MergetoolTraceStage::ComputeTwoWayWordHighlights,
                two_way_word_highlights_started,
                trace_decisions,
            )
            .with_conflict_block_count(Some(conflict_block_count))
            .with_diff_row_count(Some(diff_row_count))
    });
    (three_way_word_highlights, two_way_aligned_word_highlights)
}

/// Budgeted foreground syntax parse of each merge-input side; sides that
/// time out are flagged for a background parse.
pub(super) fn prepare_three_way_syntax_documents(
    conflict_syntax_language: Option<rows::DiffSyntaxLanguage>,
    three_way_text: &ThreeWaySides<SharedString>,
    three_way_line_starts: &ThreeWaySides<DeferredLineStarts>,
    budget: rows::DiffSyntaxBudget,
) -> (
    ThreeWaySides<Option<rows::PreparedDiffSyntaxDocument>>,
    ThreeWaySides<bool>,
) {
    let mut three_way_prepared_docs =
        ThreeWaySides::<Option<rows::PreparedDiffSyntaxDocument>>::default();
    let mut three_way_needs_background = ThreeWaySides::<bool>::default();
    if let Some(language) = conflict_syntax_language {
        for side in ThreeWayColumn::ALL {
            let text = &three_way_text[side];
            let doc_slot = &mut three_way_prepared_docs[side];
            let bg_slot = &mut three_way_needs_background[side];
            if text.is_empty() {
                continue;
            }
            let line_starts = three_way_line_starts[side].shared_starts(text.as_ref());
            match rows::prepare_diff_syntax_document_with_budget_reuse_text(
                language,
                rows::DiffSyntaxMode::Auto,
                text.clone(),
                line_starts.clone(),
                budget,
                None,
                None,
            ) {
                rows::PrepareDiffSyntaxDocumentResult::Ready(doc) => {
                    *doc_slot = Some(doc);
                }
                rows::PrepareDiffSyntaxDocumentResult::TimedOut => {
                    *bg_slot = true;
                }
                rows::PrepareDiffSyntaxDocumentResult::Unsupported => {}
            }
        }
    }
    (three_way_prepared_docs, three_way_needs_background)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_automatic_delta() -> gitcomet_core::conflict_session::ConflictSession {
        use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
        use gitcomet_core::domain::FileConflictKind;

        ConflictSession::from_stage_inputs(
            std::path::PathBuf::from("file.txt"),
            FileConflictKind::BothModified,
            ConflictPayload::Text("start\nold-local\nmiddle\nold-conflict\nend\n".into()),
            ConflictPayload::Text("start\nnew-local\nmiddle\nours-conflict\nend\n".into()),
            ConflictPayload::Text("start\nold-local\nmiddle\ntheirs-conflict\nend\n".into()),
        )
    }

    #[test]
    fn live_plan_projection_renders_an_automatic_delta_override() {
        use gitcomet_core::merge::MergeSource;

        let mut session = session_with_automatic_delta();
        let automatic_id = session
            .merge_plan
            .as_ref()
            .unwrap()
            .blocks
            .iter()
            .find(|block| block.is_delta && !block.original_conflict)
            .unwrap()
            .id;
        let (automatic, _) = conflict_session_plan_projection(&session).unwrap();
        assert!(automatic.contains("new-local\n"));

        assert!(session.replace_plan_block_selection(automatic_id, MergeSource::C.into()));
        let (overridden, _) = conflict_session_plan_projection(&session).unwrap();
        assert!(overridden.contains("old-local\n"));
        assert!(!overridden.contains("new-local\n"));
    }

    #[test]
    fn an_unresolved_automatic_delta_gets_a_visible_plan_block_mapping() {
        use gitcomet_core::merge::MergeSource;

        let mut session = session_with_automatic_delta();
        let (automatic_index, automatic_id) = session
            .merge_plan
            .as_ref()
            .unwrap()
            .blocks
            .iter()
            .enumerate()
            .find(|(_, block)| block.is_delta && !block.original_conflict)
            .map(|(index, block)| (index, block.id))
            .unwrap();
        assert!(session.toggle_plan_block_source(automatic_id, MergeSource::B));
        let (projection, projected_plan_blocks) =
            conflict_session_plan_projection(&session).unwrap();
        let mut segments = conflict_resolver::parse_conflict_markers(projection.as_ref());
        let applied = conflict_resolver::apply_plan_session_region_resolutions_with_index_map(
            &mut segments,
            &session,
            &projected_plan_blocks,
        )
        .expect("exact mapping");
        let plan_blocks = applied.block_plan_indices;
        assert!(plan_blocks.contains(&automatic_index));
        assert_eq!(
            plan_blocks,
            session.merge_plan.as_ref().unwrap().unresolved_blocks
        );
    }

    #[test]
    fn plan_whitespace_classification_reaches_the_display_blocks() {
        use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
        use gitcomet_core::domain::FileConflictKind;

        // Both sides only respaced the same line, so kdiff3's per-row rule
        // marks the block whitespace-only.
        let session = ConflictSession::from_stage_inputs(
            std::path::PathBuf::from("file.txt"),
            FileConflictKind::BothModified,
            ConflictPayload::Text("value = 1\n".into()),
            ConflictPayload::Text("value=1\n".into()),
            ConflictPayload::Text("value  =  1\n".into()),
        );
        assert!(
            session
                .merge_plan
                .as_ref()
                .expect("plan-backed session")
                .blocks
                .iter()
                .any(|block| block.whitespace_conflict),
            "fixture should produce a whitespace conflict"
        );

        let (projection, projected_plan_blocks) =
            conflict_session_plan_projection(&session).unwrap();
        let mut segments = conflict_resolver::parse_conflict_markers(projection.as_ref());
        conflict_resolver::apply_plan_session_region_resolutions_with_index_map(
            &mut segments,
            &session,
            &projected_plan_blocks,
        )
        .expect("exact mapping");

        assert!(
            segments.iter().any(|segment| matches!(
                segment,
                conflict_resolver::ConflictSegment::Block(block) if block.whitespace_only
            )),
            "the plan's whitespace verdict should land on the display block"
        );
    }

    #[test]
    fn conflict_file_source_fingerprint_is_stable_across_fresh_allocations() {
        let make_file = || gitcomet_state::model::ConflictFile {
            path: std::path::PathBuf::from("index.html").into(),
            base_bytes: Some(std::sync::Arc::<[u8]>::from(b"base\nbytes\n".as_slice())),
            ours_bytes: None,
            theirs_bytes: Some(std::sync::Arc::<[u8]>::from(b"theirs\nbytes\n".as_slice())),
            current_bytes: None,
            base: Some(std::sync::Arc::<str>::from("base\ntext\n")),
            ours: Some(std::sync::Arc::<str>::from("ours\ntext\n")),
            theirs: Some(std::sync::Arc::<str>::from("theirs\ntext\n")),
            current: Some(std::sync::Arc::<str>::from(
                "<<<<<<< ours\nbody\n=======\nbody\n>>>>>>> theirs\n",
            )),
        };

        let left = make_file();
        let right = make_file();

        assert_eq!(
            conflict_file_source_fingerprint(&left),
            conflict_file_source_fingerprint(&right),
            "content-identical conflict files should keep the lightweight resync path even when backing Arcs are freshly allocated",
        );
    }

    #[test]
    fn shared_content_fingerprints_keep_domains_distinct() {
        let none_text = None;
        let empty_text = Some(std::sync::Arc::<str>::from(""));
        let text = Some(std::sync::Arc::<str>::from("shared payload"));

        let none_bytes = None;
        let empty_bytes = Some(std::sync::Arc::<[u8]>::from(b"".as_slice()));
        let bytes = Some(std::sync::Arc::<[u8]>::from(b"shared payload".as_slice()));

        assert_ne!(
            shared_text_fingerprint(&none_text),
            shared_text_fingerprint(&empty_text),
            "missing text should not collide with an empty text payload",
        );
        assert_ne!(
            shared_bytes_fingerprint(&none_bytes),
            shared_bytes_fingerprint(&empty_bytes),
            "missing bytes should not collide with an empty byte payload",
        );
        assert_ne!(
            shared_text_fingerprint(&text),
            shared_bytes_fingerprint(&bytes),
            "text and byte payloads use separate fingerprint domains",
        );
    }
}
