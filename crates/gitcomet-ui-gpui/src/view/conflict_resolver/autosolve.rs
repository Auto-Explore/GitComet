use super::*;

pub fn format_autosolve_trace_summary(
    mode: AutosolveTraceMode,
    unresolved_before: usize,
    unresolved_after: usize,
    stats: &gitcomet_state::msg::ConflictAutosolveStats,
) -> String {
    let resolved = unresolved_before.saturating_sub(unresolved_after);
    let blocks_word = if resolved == 1 { "block" } else { "blocks" };
    match mode {
        AutosolveTraceMode::OnOpen => format!(
            "Auto-solved on open: resolved {resolved} {blocks_word}, unresolved {} -> {} (pass1 {}, split {}, regex {}).",
            unresolved_before, unresolved_after, stats.pass1, stats.pass2_split, stats.regex
        ),
        #[cfg(test)]
        AutosolveTraceMode::History => format!(
            "Last autosolve (history): resolved {resolved} {blocks_word}, unresolved {} -> {} (history {}).",
            unresolved_before, unresolved_after, stats.history
        ),
    }
}

/// KDiff3-style accounting for a merge session.
///
/// Plan-backed sessions count every block classified as a conflict or delta,
/// not just the subset that still needs a user decision. The whitespace count
/// is optional because marker-only fallback sessions do not have KDiff3's
/// exact aligned-row classification available.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConflictSummaryCounts {
    pub total: usize,
    pub auto_solved: usize,
    pub unsolved: usize,
    pub whitespace_conflicts: Option<usize>,
}

impl ConflictSummaryCounts {
    fn normalized(self) -> Self {
        let unsolved = self.unsolved.min(self.total);
        Self {
            auto_solved: self.auto_solved.min(self.total.saturating_sub(unsolved)),
            unsolved,
            whitespace_conflicts: self.whitespace_conflicts.map(|count| count.min(self.total)),
            ..self
        }
    }
}

/// Format the shared total / auto-solved / unsolved report used by the toast
/// and resolver status bar.
pub fn format_conflict_summary(counts: ConflictSummaryCounts) -> String {
    let counts = counts.normalized();
    format!(
        "Total {} / auto-solved {} / unsolved {}",
        counts.total, counts.auto_solved, counts.unsolved
    )
}

/// Build the one-shot toast message pushed when a conflict file's resolver
/// opens fresh.
pub fn format_open_summary_toast(counts: ConflictSummaryCounts) -> Option<String> {
    if counts.total == 0 {
        return None;
    }
    Some(format_conflict_summary(counts))
}

/// Count a session using KDiff3's conflict-reporting convention.
///
/// With a merge plan, `total` is every stable conflict-or-delta block,
/// `unsolved` is the currently unresolved subset, and `auto_solved` is their
/// difference. Marker-only sessions retain region-based fallback accounting.
pub fn conflict_session_summary_counts(
    session: &gitcomet_core::conflict_session::ConflictSession,
) -> ConflictSummaryCounts {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

    if let Some(plan) = session.merge_plan.as_ref() {
        let mut total = 0usize;
        let mut unsolved = 0usize;
        let mut whitespace_conflicts = 0usize;
        for block in plan
            .blocks
            .iter()
            .filter(|block| block.original_conflict || block.is_delta)
        {
            total += 1;
            unsolved += usize::from(!block.is_resolved());
            // KDiff3's status line reports the *unsolved* whitespace subset
            // (`getNumberOfUnsolvedConflicts(&wsc)`), so the count falls as the
            // user picks sides for them.
            whitespace_conflicts += usize::from(block.whitespace_conflict && !block.is_resolved());
        }
        return ConflictSummaryCounts {
            total,
            auto_solved: total.saturating_sub(unsolved),
            unsolved,
            whitespace_conflicts: Some(whitespace_conflicts),
        };
    }

    let auto_solved = session
        .regions
        .iter()
        .filter(|region| {
            matches!(
                &region.resolution,
                ConflictRegionResolution::AutoResolved { .. }
            )
        })
        .count();
    ConflictSummaryCounts {
        total: session.total_regions(),
        auto_solved,
        unsolved: session.unsolved_count(),
        whitespace_conflicts: None,
    }
}

/// Summarize the on-open autosolve pass from session region resolutions.
///
/// On a fresh open every resolved region is an [`AutoResolved`] one (user
/// picks cannot exist yet), so the confidence-tier breakdown can be
/// reconstructed from the applied rules. Returns `None` when nothing was
/// auto-resolved.
///
/// [`AutoResolved`]: gitcomet_core::conflict_session::ConflictRegionResolution::AutoResolved
pub fn on_open_autosolve_summary(
    session: &gitcomet_core::conflict_session::ConflictSession,
) -> Option<String> {
    use gitcomet_core::conflict_session::{AutosolveRule, ConflictRegionResolution};

    let mut stats = gitcomet_state::msg::ConflictAutosolveStats::default();
    for region in &session.regions {
        let ConflictRegionResolution::AutoResolved { rule, .. } = &region.resolution else {
            continue;
        };
        match rule {
            AutosolveRule::IdenticalSides
            | AutosolveRule::OnlyOursChanged
            | AutosolveRule::OnlyTheirsChanged
            | AutosolveRule::WhitespaceOnly => stats.pass1 += 1,
            AutosolveRule::SubchunkFullyMerged => stats.pass2_split += 1,
            AutosolveRule::RegexEquivalentSides
            | AutosolveRule::RegexOnlyTheirsChanged
            | AutosolveRule::RegexOnlyOursChanged => stats.regex += 1,
            AutosolveRule::HistoryMerged => stats.history += 1,
        }
    }

    let resolved = stats.total_resolved();
    if resolved == 0 {
        return None;
    }
    let unresolved_after = session.unsolved_count();
    Some(format_autosolve_trace_summary(
        AutosolveTraceMode::OnOpen,
        unresolved_after + resolved,
        unresolved_after,
        &stats,
    ))
}

/// Count conflict blocks whose backing session regions were auto-resolved when
/// the resolver opened.
/// Build a per-conflict autosolve trace label for the active conflict.
///
/// Returns `None` when the active conflict does not map to an auto-resolved
/// session region.
pub fn active_conflict_autosolve_trace_label(
    session: &gitcomet_core::conflict_session::ConflictSession,
    conflict_region_indices: &[usize],
    active_conflict: usize,
) -> Option<String> {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

    let region_index = *conflict_region_indices.get(active_conflict)?;
    let region = session.regions.get(region_index)?;
    if let ConflictRegionResolution::AutoResolved {
        rule, confidence, ..
    } = &region.resolution
    {
        Some(format!(
            "Auto: {} ({})",
            rule.description(),
            confidence.label()
        ))
    } else {
        None
    }
}

pub fn conflict_count(segments: &[ConflictSegment]) -> usize {
    segments
        .iter()
        .filter(|s| matches!(s, ConflictSegment::Block(_)))
        .count()
}

/// Count how many conflict blocks have been explicitly resolved.
pub fn resolved_conflict_count(segments: &[ConflictSegment]) -> usize {
    segments
        .iter()
        .filter(|s| matches!(s, ConflictSegment::Block(b) if b.resolved))
        .count()
}

/// Compute effective conflict counters for resolver UI state.
///
/// Marker segments are authoritative for text-based conflict flows. For
/// non-marker strategies (binary side-pick / keep-delete / decision-only),
/// callers can pass state-layer session counters as a fallback.
pub fn effective_conflict_counts(
    segments: &[ConflictSegment],
    session_counts: Option<(usize, usize)>,
) -> (usize, usize) {
    let total = conflict_count(segments);
    if total > 0 {
        return (total, resolved_conflict_count(segments));
    }
    if let Some((session_total, session_resolved)) = session_counts {
        return (session_total, session_resolved.min(session_total));
    }
    (0, 0)
}

/// Return conflict indices for currently unresolved blocks in queue order.
#[cfg(test)]
pub fn unresolved_conflict_indices(segments: &[ConflictSegment]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut conflict_ix = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if !block.resolved {
            out.push(conflict_ix);
        }
        conflict_ix += 1;
    }
    out
}

/// Apply a choice to all unresolved conflict blocks.
///
/// Already-resolved blocks are preserved. Choosing `Base` skips unresolved
/// 2-way blocks that don't have an ancestor section.
///
/// Returns the number of blocks updated.
#[cfg(test)]
pub fn apply_choice_to_unresolved_segments(
    segments: &mut [ConflictSegment],
    choice: ConflictChoice,
) -> usize {
    let mut updated = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if block.resolved {
            continue;
        }
        if matches!(choice, ConflictChoice::Base) && block.base.is_none() {
            continue;
        }
        block.choice = choice;
        block.resolved = true;
        updated += 1;
    }
    updated
}

/// Find the next unresolved conflict index after `current`.
/// Wraps around to the first unresolved conflict.
#[cfg(test)]
pub fn next_unresolved_conflict_index(
    segments: &[ConflictSegment],
    current: usize,
) -> Option<usize> {
    let unresolved = unresolved_conflict_indices(segments);
    unresolved
        .iter()
        .copied()
        .find(|&ix| ix > current)
        .or_else(|| unresolved.first().copied())
}

/// Find the previous unresolved conflict index before `current`.
/// Wraps around to the last unresolved conflict.
#[cfg(test)]
pub fn prev_unresolved_conflict_index(
    segments: &[ConflictSegment],
    current: usize,
) -> Option<usize> {
    let unresolved = unresolved_conflict_indices(segments);
    unresolved
        .iter()
        .rev()
        .copied()
        .find(|&ix| ix < current)
        .or_else(|| unresolved.last().copied())
}

/// Apply safe auto-resolve rules (Pass 1) to all unresolved conflict blocks.
///
/// Safe rules:
/// 1. `ours == theirs` — both sides made the same change → pick ours.
/// 2. `ours == base` and `theirs != base` — only theirs changed → pick theirs.
/// 3. `theirs == base` and `ours != base` — only ours changed → pick ours.
/// 4. (if `whitespace_normalize`) whitespace-only difference → pick ours.
///
/// Returns the number of blocks auto-resolved.
#[cfg(test)]
pub fn auto_resolve_segments(segments: &mut [ConflictSegment]) -> usize {
    auto_resolve_segments_with_options(segments, false)
}

/// Like [`auto_resolve_segments`] but with an optional whitespace-normalization toggle.
///
/// Segment-based autosolve is now test-only: the live on-open path uses the
/// session-based `apply_autosolve_to_session` in gitcomet-state, and the
/// manual re-trigger button was removed.
#[cfg(test)]
pub fn auto_resolve_segments_with_options(
    segments: &mut [ConflictSegment],
    whitespace_normalize: bool,
) -> usize {
    use gitcomet_core::conflict_session::{AutosolvePickSide, safe_auto_resolve_pick};

    let mut count = 0;
    for seg in segments.iter_mut() {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if block.resolved {
            continue;
        }

        let Some((_, pick)) = safe_auto_resolve_pick(
            block.base.as_deref(),
            &block.ours,
            &block.theirs,
            whitespace_normalize,
        ) else {
            continue;
        };

        block.choice = match pick {
            AutosolvePickSide::Ours => ConflictChoice::Ours,
            AutosolvePickSide::Theirs => ConflictChoice::Theirs,
        };
        block.resolved = true;
        count += 1;
    }
    count
}

/// Apply Pass 3 regex-assisted auto-resolve rules (opt-in) to unresolved blocks.
///
/// This mode uses regex normalization rules from core and only performs
/// side-picks (`Ours` / `Theirs`), never synthetic text rewrites.
#[cfg(test)]
pub fn auto_resolve_segments_regex(
    segments: &mut [ConflictSegment],
    options: &gitcomet_core::conflict_session::RegexAutosolveOptions,
) -> usize {
    use gitcomet_core::conflict_session::{AutosolvePickSide, regex_assisted_auto_resolve_pick};

    let mut count = 0;
    for seg in segments.iter_mut() {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if block.resolved {
            continue;
        }

        let Some((_, pick)) = regex_assisted_auto_resolve_pick(
            block.base.as_deref(),
            &block.ours,
            &block.theirs,
            options,
        ) else {
            continue;
        };

        block.choice = match pick {
            AutosolvePickSide::Ours => ConflictChoice::Ours,
            AutosolvePickSide::Theirs => ConflictChoice::Theirs,
        };
        block.resolved = true;
        count += 1;
    }
    count
}

/// Apply history-aware auto-resolve to unresolved conflict blocks.
///
/// Detects history/changelog sections and merges entries by deduplication.
/// When a block is resolved by history merge, it is replaced with a `Text`
/// segment containing the merged content.
///
/// Returns the number of blocks resolved.
#[cfg(test)]
pub fn auto_resolve_segments_history(
    segments: &mut Vec<ConflictSegment>,
    options: &gitcomet_core::conflict_session::HistoryAutosolveOptions,
) -> usize {
    let mut block_region_indices = sequential_conflict_region_indices(segments);
    auto_resolve_segments_history_with_region_indices(segments, options, &mut block_region_indices)
}

/// Like [`auto_resolve_segments_history`] but keeps block->region mappings in sync.
#[cfg(test)]
pub fn auto_resolve_segments_history_with_region_indices(
    segments: &mut Vec<ConflictSegment>,
    options: &gitcomet_core::conflict_session::HistoryAutosolveOptions,
    block_region_indices: &mut Vec<usize>,
) -> usize {
    use gitcomet_core::conflict_session::history_merge_region;

    let mut new_segments = Vec::with_capacity(segments.len());
    let mut new_block_region_indices = Vec::with_capacity(block_region_indices.len());
    let mut block_ix = 0usize;
    let mut count = 0;

    for seg in segments.drain(..) {
        match seg {
            ConflictSegment::Block(block) => {
                let region_ix = block_region_indices
                    .get(block_ix)
                    .copied()
                    .unwrap_or(block_ix);
                block_ix += 1;
                if !block.resolved
                    && let Some(merged) = history_merge_region(
                        block.base.as_deref(),
                        &block.ours,
                        &block.theirs,
                        options,
                    )
                {
                    // Merge adjacent Text segments for cleanliness.
                    if let Some(ConflictSegment::Text(prev)) = new_segments.last_mut() {
                        prev.push_str(&merged);
                    } else {
                        new_segments.push(ConflictSegment::Text(merged.into()));
                    }
                    count += 1;
                    continue;
                }
                new_segments.push(ConflictSegment::Block(block));
                new_block_region_indices.push(region_ix);
            }
            other => new_segments.push(other),
        }
    }

    *segments = new_segments;
    *block_region_indices = new_block_region_indices;
    count
}

/// Apply Pass 2 (heuristic subchunk splitting) to unresolved conflict blocks.
///
/// For each unresolved block that has a base, attempts to split it into
/// line-level subchunks via 3-way diff/merge. Non-conflicting subchunks
/// become `Text` segments; remaining conflicts become smaller `Block` segments.
///
/// Returns the number of original blocks that were split.
#[cfg(test)]
pub fn auto_resolve_segments_pass2(segments: &mut Vec<ConflictSegment>) -> usize {
    let mut block_region_indices = sequential_conflict_region_indices(segments);
    auto_resolve_segments_pass2_with_region_indices(segments, &mut block_region_indices)
}

/// Like [`auto_resolve_segments_pass2`] but keeps block->region mappings in sync.
#[cfg(test)]
pub fn auto_resolve_segments_pass2_with_region_indices(
    segments: &mut Vec<ConflictSegment>,
    block_region_indices: &mut Vec<usize>,
) -> usize {
    use gitcomet_core::conflict_session::{Subchunk, split_conflict_into_subchunks};

    let mut new_segments = Vec::with_capacity(segments.len());
    let mut new_block_region_indices = Vec::with_capacity(block_region_indices.len());
    let mut block_ix = 0usize;
    let mut split_count = 0;

    for seg in segments.drain(..) {
        match seg {
            ConflictSegment::Block(block) => {
                let region_ix = block_region_indices
                    .get(block_ix)
                    .copied()
                    .unwrap_or(block_ix);
                block_ix += 1;
                if !block.resolved
                    && let Some(base) = block.base.as_deref()
                    && let Some(subchunks) =
                        split_conflict_into_subchunks(base, &block.ours, &block.theirs)
                {
                    split_count += 1;
                    for subchunk in subchunks {
                        match subchunk {
                            Subchunk::Resolved(text) => {
                                // Merge adjacent Text segments for cleanliness.
                                if let Some(ConflictSegment::Text(prev)) = new_segments.last_mut() {
                                    prev.push_str(&text);
                                } else {
                                    new_segments.push(ConflictSegment::Text(text.into()));
                                }
                            }
                            Subchunk::Conflict { base, ours, theirs } => {
                                new_segments.push(ConflictSegment::Block(ConflictBlock {
                                    base: Some(base.into()),
                                    ours: ours.into(),
                                    theirs: theirs.into(),
                                    choice: ConflictChoice::empty(),
                                    resolved: false,
                                    // Every row of a whitespace-only block is
                                    // whitespace-only, so each subchunk of it
                                    // is too. kdiff3 clears the flag the same
                                    // way when a split lands a real change in
                                    // a block (MergeEditLine.h join/append).
                                    whitespace_only: block.whitespace_only,
                                }));
                                new_block_region_indices.push(region_ix);
                            }
                        }
                    }
                    // If all subchunks resolved, no Block segments remain
                    // from this split (all became Text above).
                    continue;
                }
                new_segments.push(ConflictSegment::Block(block));
                new_block_region_indices.push(region_ix);
            }
            other => new_segments.push(other),
        }
    }

    *segments = new_segments;
    *block_region_indices = new_block_region_indices;
    split_count
}
