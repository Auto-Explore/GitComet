#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConflictChoice {
    Base,
    Ours,
    Theirs,
    Both,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictDiffMode {
    Split,
    Inline,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConflictResolverViewMode {
    ThreeWay,
    TwoWayDiff,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub enum ConflictPickSide {
    Ours,
    Theirs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum AutosolveTraceMode {
    Safe,
    History,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictNavDirection {
    Prev,
    Next,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictBlock {
    pub base: Option<String>,
    pub ours: String,
    pub theirs: String,
    pub choice: ConflictChoice,
    /// Whether this block has been explicitly resolved (by user pick or auto-resolve).
    /// Blocks start unresolved; becomes `true` when the user picks a side or auto-resolve runs.
    pub resolved: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConflictSegment {
    Text(String),
    Block(ConflictBlock),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictInlineRow {
    pub side: ConflictPickSide,
    pub kind: gitcomet_core::domain::DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub content: String,
}

/// Source provenance for a resolved output line.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ResolvedLineSource {
    /// Line matches source A (Base in three-way, Ours in two-way).
    A,
    /// Line matches source B (Ours in three-way, Theirs in two-way).
    B,
    /// Line matches source C (Theirs in three-way; not used in two-way).
    C,
    /// Line was manually edited or does not match any source.
    Manual,
}

impl ResolvedLineSource {
    /// Compact single-character label for UI badges.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn badge_char(self) -> char {
        match self {
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
            Self::Manual => 'M',
        }
    }
}

/// Per-line provenance metadata for the resolved output outline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedLineMeta {
    /// 0-based line index in the resolved output.
    pub output_line: u32,
    /// Which source this line came from (or Manual).
    pub source: ResolvedLineSource,
    /// If source is A/B/C, the 1-based line number in that source pane.
    pub input_line: Option<u32>,
}

/// Key identifying a specific source line for dedupe gating (plus-icon visibility).
///
/// Two source lines with the same key are considered "the same row" for purposes
/// of preventing duplicate insertion into the resolved output.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceLineKey {
    pub view_mode: ConflictResolverViewMode,
    pub side: ResolvedLineSource,
    /// 1-based line number in the source pane.
    pub line_no: u32,
    /// Hash of the line's text content for fast equality checks.
    pub content_hash: u64,
}

impl SourceLineKey {
    pub fn new(
        view_mode: ConflictResolverViewMode,
        side: ResolvedLineSource,
        line_no: u32,
        content: &str,
    ) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        content.hash(&mut hasher);
        Self {
            view_mode,
            side,
            line_no,
            content_hash: hasher.finish(),
        }
    }
}

/// Per-line word-highlight ranges. `None` means no highlights for that line.
pub type WordHighlights = Vec<Option<Vec<std::ops::Range<usize>>>>;

/// Resolve conflict quick-pick keyboard shortcuts to a concrete choice.
pub fn conflict_quick_pick_choice_for_key(key: &str) -> Option<ConflictChoice> {
    match key {
        "a" => Some(ConflictChoice::Base),
        "b" => Some(ConflictChoice::Ours),
        "c" => Some(ConflictChoice::Theirs),
        "d" => Some(ConflictChoice::Both),
        _ => None,
    }
}

/// Resolve conflict navigation shortcuts (`F2`, `F3`, `F7`) to a direction.
pub fn conflict_nav_direction_for_key(key: &str, shift: bool) -> Option<ConflictNavDirection> {
    match key {
        "f2" => Some(ConflictNavDirection::Prev),
        "f3" => Some(ConflictNavDirection::Next),
        "f7" if shift => Some(ConflictNavDirection::Prev),
        "f7" => Some(ConflictNavDirection::Next),
        _ => None,
    }
}

/// Build a user-facing summary for the most recent autosolve run.
///
/// The summary is shown in the resolver UI so autosolve behavior remains
/// auditable without opening command logs.
pub fn format_autosolve_trace_summary(
    mode: AutosolveTraceMode,
    unresolved_before: usize,
    unresolved_after: usize,
    stats: &gitcomet_state::msg::ConflictAutosolveStats,
) -> String {
    let resolved = unresolved_before.saturating_sub(unresolved_after);
    let blocks_word = if resolved == 1 { "block" } else { "blocks" };
    match mode {
        AutosolveTraceMode::Safe => format!(
            "Last autosolve (safe): resolved {resolved} {blocks_word}, unresolved {} -> {} (pass1 {}, split {}, pass1-after-split {}).",
            unresolved_before,
            unresolved_after,
            stats.pass1,
            stats.pass2_split,
            stats.pass1_after_split
        ),
        AutosolveTraceMode::History => format!(
            "Last autosolve (history): resolved {resolved} {blocks_word}, unresolved {} -> {} (history {}).",
            unresolved_before, unresolved_after, stats.history
        ),
    }
}

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

pub fn parse_conflict_markers(text: &str) -> Vec<ConflictSegment> {
    gitcomet_core::conflict_session::parse_conflict_marker_segments(text)
        .into_iter()
        .map(|segment| match segment {
            gitcomet_core::conflict_session::ParsedConflictSegment::Text(text) => {
                ConflictSegment::Text(text)
            }
            gitcomet_core::conflict_session::ParsedConflictSegment::Conflict(block) => {
                ConflictSegment::Block(ConflictBlock {
                    base: block.base,
                    ours: block.ours,
                    theirs: block.theirs,
                    choice: ConflictChoice::Ours,
                    resolved: false,
                })
            }
        })
        .collect()
}

fn append_text_segment(segments: &mut Vec<ConflictSegment>, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(ConflictSegment::Text(prev)) = segments.last_mut() {
        prev.push_str(&text);
        return;
    }
    segments.push(ConflictSegment::Text(text));
}

fn choice_for_resolved_content(block: &ConflictBlock, content: &str) -> Option<ConflictChoice> {
    if content == block.ours {
        return Some(ConflictChoice::Ours);
    }
    if content == block.theirs {
        return Some(ConflictChoice::Theirs);
    }
    if block.base.as_deref().is_some_and(|base| content == base) {
        return Some(ConflictChoice::Base);
    }
    content
        .strip_prefix(block.ours.as_str())
        .is_some_and(|rest| rest == block.theirs)
        .then_some(ConflictChoice::Both)
}

fn content_matches_block_choice(block: &ConflictBlock, content: &str) -> bool {
    match block.choice {
        ConflictChoice::Base => block.base.as_deref().is_some_and(|base| content == base),
        ConflictChoice::Ours => content == block.ours,
        ConflictChoice::Theirs => content == block.theirs,
        ConflictChoice::Both => content
            .strip_prefix(block.ours.as_str())
            .is_some_and(|rest| rest == block.theirs),
    }
}

fn extract_block_contents_from_output(
    segments: &[ConflictSegment],
    output_text: &str,
) -> Option<Vec<String>> {
    let mut cursor = 0usize;
    let mut block_contents = Vec::new();

    for (seg_ix, seg) in segments.iter().enumerate() {
        match seg {
            ConflictSegment::Text(text) => {
                let tail = output_text.get(cursor..)?;
                if !tail.starts_with(text) {
                    return None;
                }
                cursor = cursor.saturating_add(text.len());
            }
            ConflictSegment::Block(_) => {
                let next_anchor = segments[seg_ix + 1..].iter().find_map(|next| match next {
                    ConflictSegment::Text(text) if !text.is_empty() => Some(text.as_str()),
                    _ => None,
                });
                let end = match next_anchor {
                    Some(anchor) => {
                        let rel = output_text.get(cursor..)?.find(anchor)?;
                        cursor.saturating_add(rel)
                    }
                    None => output_text.len(),
                };
                if end < cursor {
                    return None;
                }
                block_contents.push(output_text[cursor..end].to_string());
                cursor = end;
            }
        }
    }

    (cursor == output_text.len()).then_some(block_contents)
}

/// Derive per-region session resolution updates from the current resolved output.
///
/// This is used to persist manual resolver edits back into state without
/// requiring marker reparse in the reducer.
pub fn derive_region_resolution_updates_from_output(
    segments: &[ConflictSegment],
    block_region_indices: &[usize],
    output_text: &str,
) -> Option<
    Vec<(
        usize,
        gitcomet_core::conflict_session::ConflictRegionResolution,
    )>,
> {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let block_contents = extract_block_contents_from_output(segments, output_text)?;
    let mut updates = Vec::with_capacity(block_contents.len());

    let mut block_ix = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        let content = block_contents.get(block_ix)?;
        let region_ix = block_region_indices
            .get(block_ix)
            .copied()
            .unwrap_or(block_ix);

        let resolution = if !block.resolved && content_matches_block_choice(block, content) {
            R::Unresolved
        } else if let Some(choice) = choice_for_resolved_content(block, content) {
            match choice {
                ConflictChoice::Base => R::PickBase,
                ConflictChoice::Ours => R::PickOurs,
                ConflictChoice::Theirs => R::PickTheirs,
                ConflictChoice::Both => R::PickBoth,
            }
        } else {
            R::ManualEdit(content.clone())
        };
        updates.push((region_ix, resolution));
        block_ix += 1;
    }

    Some(updates)
}

/// Result of applying state-layer region resolutions to UI marker segments.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionRegionApplyResult {
    /// Number of source regions visited/applied.
    pub applied_regions: usize,
    /// Mapping from visible block index -> source `ConflictSession` region index.
    pub block_region_indices: Vec<usize>,
}

/// Build a default visible block -> region index mapping by position.
pub fn sequential_conflict_region_indices(segments: &[ConflictSegment]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut conflict_ix = 0usize;
    for seg in segments {
        if matches!(seg, ConflictSegment::Block(_)) {
            out.push(conflict_ix);
            conflict_ix += 1;
        }
    }
    out
}

fn apply_region_resolution_to_block(
    block: &mut ConflictBlock,
    resolution: &gitcomet_core::conflict_session::ConflictRegionResolution,
) -> Option<String> {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    match resolution {
        R::Unresolved => {
            block.resolved = false;
            None
        }
        R::PickBase => {
            if block.base.is_some() {
                block.choice = ConflictChoice::Base;
                block.resolved = true;
            } else {
                block.resolved = false;
            }
            None
        }
        R::PickOurs => {
            block.choice = ConflictChoice::Ours;
            block.resolved = true;
            None
        }
        R::PickTheirs => {
            block.choice = ConflictChoice::Theirs;
            block.resolved = true;
            None
        }
        R::PickBoth => {
            block.choice = ConflictChoice::Both;
            block.resolved = true;
            None
        }
        R::ManualEdit(text) => {
            if let Some(choice) = choice_for_resolved_content(block, text) {
                block.choice = choice;
                block.resolved = true;
                return None;
            }
            Some(text.clone())
        }
        R::AutoResolved { content, .. } => {
            if let Some(choice) = choice_for_resolved_content(block, content) {
                block.choice = choice;
                block.resolved = true;
                return None;
            }
            Some(content.clone())
        }
    }
}

/// Apply state-layer region resolutions to parsed UI marker segments.
///
/// This allows resolver rebuilds to preserve choices tracked in
/// `RepoState.conflict_state.conflict_session`, and materializes manual/auto-resolved
/// non-side-pick text into plain `Text` segments when needed.
///
/// Returns how many conflict regions were applied.
#[cfg_attr(not(test), allow(dead_code))]
pub fn apply_session_region_resolutions(
    segments: &mut Vec<ConflictSegment>,
    regions: &[gitcomet_core::conflict_session::ConflictRegion],
) -> usize {
    apply_session_region_resolutions_with_index_map(segments, regions).applied_regions
}

/// Like [`apply_session_region_resolutions`] but also returns a visible block
/// index map back to the original `ConflictSession` region indices.
pub fn apply_session_region_resolutions_with_index_map(
    segments: &mut Vec<ConflictSegment>,
    regions: &[gitcomet_core::conflict_session::ConflictRegion],
) -> SessionRegionApplyResult {
    if segments.is_empty() {
        return SessionRegionApplyResult::default();
    }
    if regions.is_empty() {
        return SessionRegionApplyResult {
            applied_regions: 0,
            block_region_indices: sequential_conflict_region_indices(segments),
        };
    }

    let mut applied = 0usize;
    let mut conflict_ix = 0usize;
    let mut block_region_indices = Vec::new();
    let mut synced: Vec<ConflictSegment> = Vec::with_capacity(segments.len());

    for seg in segments.drain(..) {
        match seg {
            ConflictSegment::Text(text) => append_text_segment(&mut synced, text),
            ConflictSegment::Block(mut block) => {
                if let Some(region) = regions.get(conflict_ix) {
                    if let Some(materialized_text) =
                        apply_region_resolution_to_block(&mut block, &region.resolution)
                    {
                        append_text_segment(&mut synced, materialized_text);
                    } else {
                        synced.push(ConflictSegment::Block(block));
                        block_region_indices.push(conflict_ix);
                    }
                    applied += 1;
                } else {
                    synced.push(ConflictSegment::Block(block));
                    block_region_indices.push(conflict_ix);
                }
                conflict_ix += 1;
            }
        }
    }

    *segments = synced;
    SessionRegionApplyResult {
        applied_regions: applied,
        block_region_indices,
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
#[cfg_attr(not(test), allow(dead_code))]
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
#[cfg_attr(not(test), allow(dead_code))]
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
#[cfg_attr(not(test), allow(dead_code))]
pub fn auto_resolve_segments(segments: &mut [ConflictSegment]) -> usize {
    auto_resolve_segments_with_options(segments, false)
}

/// Like [`auto_resolve_segments`] but with an optional whitespace-normalization toggle.
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
#[cfg_attr(not(test), allow(dead_code))]
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
#[cfg_attr(not(test), allow(dead_code))]
pub fn auto_resolve_segments_history(
    segments: &mut Vec<ConflictSegment>,
    options: &gitcomet_core::conflict_session::HistoryAutosolveOptions,
) -> usize {
    let mut block_region_indices = sequential_conflict_region_indices(segments);
    auto_resolve_segments_history_with_region_indices(segments, options, &mut block_region_indices)
}

/// Like [`auto_resolve_segments_history`] but keeps block->region mappings in sync.
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
                        new_segments.push(ConflictSegment::Text(merged));
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
#[cfg_attr(not(test), allow(dead_code))]
pub fn auto_resolve_segments_pass2(segments: &mut Vec<ConflictSegment>) -> usize {
    let mut block_region_indices = sequential_conflict_region_indices(segments);
    auto_resolve_segments_pass2_with_region_indices(segments, &mut block_region_indices)
}

/// Like [`auto_resolve_segments_pass2`] but keeps block->region mappings in sync.
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
                                    new_segments.push(ConflictSegment::Text(text));
                                }
                            }
                            Subchunk::Conflict { base, ours, theirs } => {
                                new_segments.push(ConflictSegment::Block(ConflictBlock {
                                    base: Some(base),
                                    ours,
                                    theirs,
                                    choice: ConflictChoice::Ours,
                                    resolved: false,
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

pub fn generate_resolved_text(segments: &[ConflictSegment]) -> String {
    use gitcomet_core::conflict_output::GenerateResolvedTextOptions;

    generate_resolved_text_with_options(segments, GenerateResolvedTextOptions::default())
}

pub fn generate_resolved_text_with_options(
    segments: &[ConflictSegment],
    options: gitcomet_core::conflict_output::GenerateResolvedTextOptions<'_>,
) -> String {
    use gitcomet_core::conflict_output::{
        ConflictOutputBlockRef, ConflictOutputChoice, ConflictOutputSegmentRef,
        generate_resolved_text as generate_core_resolved_text,
    };

    fn map_choice(choice: ConflictChoice) -> ConflictOutputChoice {
        match choice {
            ConflictChoice::Base => ConflictOutputChoice::Base,
            ConflictChoice::Ours => ConflictOutputChoice::Ours,
            ConflictChoice::Theirs => ConflictOutputChoice::Theirs,
            ConflictChoice::Both => ConflictOutputChoice::Both,
        }
    }

    let core_segments: Vec<ConflictOutputSegmentRef<'_>> = segments
        .iter()
        .map(|segment| match segment {
            ConflictSegment::Text(text) => ConflictOutputSegmentRef::Text(text),
            ConflictSegment::Block(block) => {
                ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                    base: block.base.as_deref(),
                    ours: &block.ours,
                    theirs: &block.theirs,
                    choice: map_choice(block.choice),
                    resolved: block.resolved,
                })
            }
        })
        .collect();

    generate_core_resolved_text(&core_segments, options)
}

pub fn build_inline_rows(rows: &[gitcomet_core::file_diff::FileDiffRow]) -> Vec<ConflictInlineRow> {
    use gitcomet_core::domain::DiffLineKind as K;
    use gitcomet_core::file_diff::FileDiffRowKind as RK;

    let extra = rows.iter().filter(|r| matches!(r.kind, RK::Modify)).count();
    let mut out: Vec<ConflictInlineRow> = Vec::with_capacity(rows.len() + extra);
    for row in rows {
        match row.kind {
            RK::Context => out.push(ConflictInlineRow {
                side: ConflictPickSide::Ours,
                kind: K::Context,
                old_line: row.old_line,
                new_line: row.new_line,
                content: row.old.as_deref().unwrap_or("").to_string(),
            }),
            RK::Add => out.push(ConflictInlineRow {
                side: ConflictPickSide::Theirs,
                kind: K::Add,
                old_line: None,
                new_line: row.new_line,
                content: row.new.as_deref().unwrap_or("").to_string(),
            }),
            RK::Remove => out.push(ConflictInlineRow {
                side: ConflictPickSide::Ours,
                kind: K::Remove,
                old_line: row.old_line,
                new_line: None,
                content: row.old.as_deref().unwrap_or("").to_string(),
            }),
            RK::Modify => {
                out.push(ConflictInlineRow {
                    side: ConflictPickSide::Ours,
                    kind: K::Remove,
                    old_line: row.old_line,
                    new_line: None,
                    content: row.old.as_deref().unwrap_or("").to_string(),
                });
                out.push(ConflictInlineRow {
                    side: ConflictPickSide::Theirs,
                    kind: K::Add,
                    old_line: None,
                    new_line: row.new_line,
                    content: row.new.as_deref().unwrap_or("").to_string(),
                });
            }
        }
    }
    out
}

fn text_line_count(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    u32::try_from(text.lines().count()).unwrap_or(u32::MAX)
}

fn build_two_way_conflict_line_ranges(
    segments: &[ConflictSegment],
) -> Vec<(std::ops::Range<u32>, std::ops::Range<u32>)> {
    let mut ranges = Vec::new();
    let mut ours_line = 1u32;
    let mut theirs_line = 1u32;

    for seg in segments {
        match seg {
            ConflictSegment::Text(text) => {
                let count = text_line_count(text);
                ours_line = ours_line.saturating_add(count);
                theirs_line = theirs_line.saturating_add(count);
            }
            ConflictSegment::Block(block) => {
                let ours_count = text_line_count(&block.ours);
                let theirs_count = text_line_count(&block.theirs);
                let ours_end = ours_line.saturating_add(ours_count);
                let theirs_end = theirs_line.saturating_add(theirs_count);
                ranges.push((ours_line..ours_end, theirs_line..theirs_end));
                ours_line = ours_end;
                theirs_line = theirs_end;
            }
        }
    }

    ranges
}

fn row_conflict_index_for_lines(
    old_line: Option<u32>,
    new_line: Option<u32>,
    ranges: &[(std::ops::Range<u32>, std::ops::Range<u32>)],
) -> Option<usize> {
    ranges.iter().position(|(ours, theirs)| {
        old_line.is_some_and(|line| ours.contains(&line))
            || new_line.is_some_and(|line| theirs.contains(&line))
    })
}

fn text_line_count_usize(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ThreeWayConflictMaps {
    pub conflict_ranges: Vec<std::ops::Range<usize>>,
    pub base_line_conflict_map: Vec<Option<usize>>,
    pub ours_line_conflict_map: Vec<Option<usize>>,
    pub theirs_line_conflict_map: Vec<Option<usize>>,
    pub conflict_has_base: Vec<bool>,
}

/// Build per-column line-to-conflict maps for three-way conflict rendering.
///
/// The returned `conflict_ranges` follow the legacy behavior and are expressed
/// in the ours-column line space. The line maps provide O(1) conflict lookup
/// for each column at render/navigation time.
pub fn build_three_way_conflict_maps(
    segments: &[ConflictSegment],
    base_line_count: usize,
    ours_line_count: usize,
    theirs_line_count: usize,
) -> ThreeWayConflictMaps {
    let block_count = segments
        .iter()
        .filter(|segment| matches!(segment, ConflictSegment::Block(_)))
        .count();
    let mut maps = ThreeWayConflictMaps {
        conflict_ranges: Vec::with_capacity(block_count),
        base_line_conflict_map: vec![None; base_line_count],
        ours_line_conflict_map: vec![None; ours_line_count],
        theirs_line_conflict_map: vec![None; theirs_line_count],
        conflict_has_base: Vec::with_capacity(block_count),
    };

    fn mark_range(map: &mut [Option<usize>], start: usize, end: usize, conflict_ix: usize) {
        if map.is_empty() {
            return;
        }
        let from = start.min(map.len());
        let to = end.min(map.len());
        for slot in &mut map[from..to] {
            *slot = Some(conflict_ix);
        }
    }

    let mut base_offset = 0usize;
    let mut ours_offset = 0usize;
    let mut theirs_offset = 0usize;
    let mut conflict_ix = 0usize;
    for segment in segments {
        match segment {
            ConflictSegment::Text(text) => {
                let line_count = text_line_count_usize(text);
                base_offset = base_offset.saturating_add(line_count);
                ours_offset = ours_offset.saturating_add(line_count);
                theirs_offset = theirs_offset.saturating_add(line_count);
            }
            ConflictSegment::Block(block) => {
                let base_count = text_line_count_usize(block.base.as_deref().unwrap_or_default());
                let ours_count = text_line_count_usize(&block.ours);
                let theirs_count = text_line_count_usize(&block.theirs);

                let base_end = base_offset.saturating_add(base_count);
                let ours_end = ours_offset.saturating_add(ours_count);
                let theirs_end = theirs_offset.saturating_add(theirs_count);

                maps.conflict_ranges.push(ours_offset..ours_end);
                maps.conflict_has_base.push(block.base.is_some());

                mark_range(
                    &mut maps.base_line_conflict_map,
                    base_offset,
                    base_end,
                    conflict_ix,
                );
                mark_range(
                    &mut maps.ours_line_conflict_map,
                    ours_offset,
                    ours_end,
                    conflict_ix,
                );
                mark_range(
                    &mut maps.theirs_line_conflict_map,
                    theirs_offset,
                    theirs_end,
                    conflict_ix,
                );

                base_offset = base_end;
                ours_offset = ours_end;
                theirs_offset = theirs_end;
                conflict_ix = conflict_ix.saturating_add(1);
            }
        }
    }

    maps
}

/// Build conflict-index maps for two-way split and inline rows.
///
/// Each output entry is `Some(conflict_index)` when the row belongs to a marker
/// conflict block, or `None` for non-conflict context rows.
pub fn map_two_way_rows_to_conflicts(
    segments: &[ConflictSegment],
    diff_rows: &[gitcomet_core::file_diff::FileDiffRow],
    inline_rows: &[ConflictInlineRow],
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let ranges = build_two_way_conflict_line_ranges(segments);
    let split = diff_rows
        .iter()
        .map(|row| row_conflict_index_for_lines(row.old_line, row.new_line, &ranges))
        .collect();
    let inline = inline_rows
        .iter()
        .map(|row| row_conflict_index_for_lines(row.old_line, row.new_line, &ranges))
        .collect();
    (split, inline)
}

/// Build visible row indices for two-way views.
///
/// When `hide_resolved` is true, rows belonging to resolved conflict blocks are
/// removed from the visible list. Non-conflict rows are always kept visible.
pub fn build_two_way_visible_indices(
    row_conflict_map: &[Option<usize>],
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> Vec<usize> {
    if !hide_resolved {
        return (0..row_conflict_map.len()).collect();
    }

    let resolved_blocks: Vec<bool> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b.resolved),
            _ => None,
        })
        .collect();

    row_conflict_map
        .iter()
        .enumerate()
        .filter_map(|(ix, conflict_ix)| match conflict_ix {
            Some(ci) if resolved_blocks.get(*ci).copied().unwrap_or(false) => None,
            _ => Some(ix),
        })
        .collect()
}

/// Find the visible list index for the first row that belongs to `conflict_ix`.
///
/// `visible_row_indices` maps visible list rows to source row indices. This helper
/// resolves conflict index -> visible row index so callers can scroll/focus a
/// specific conflict in two-way resolver modes.
pub fn visible_index_for_two_way_conflict(
    row_conflict_map: &[Option<usize>],
    visible_row_indices: &[usize],
    conflict_ix: usize,
) -> Option<usize> {
    visible_row_indices.iter().position(|&row_ix| {
        row_conflict_map
            .get(row_ix)
            .copied()
            .flatten()
            .is_some_and(|ix| ix == conflict_ix)
    })
}

/// Build unresolved-only visible navigation entries for two-way views.
///
/// Returns visible list indices (not source row indices) in unresolved queue
/// order so callers can feed them directly into shared diff navigation helpers.
pub fn unresolved_visible_nav_entries_for_two_way(
    segments: &[ConflictSegment],
    row_conflict_map: &[Option<usize>],
    visible_row_indices: &[usize],
) -> Vec<usize> {
    unresolved_conflict_indices(segments)
        .into_iter()
        .filter_map(|conflict_ix| {
            visible_index_for_two_way_conflict(row_conflict_map, visible_row_indices, conflict_ix)
        })
        .collect()
}

/// Map a two-way visible index back to its conflict index.
pub fn two_way_conflict_index_for_visible_row(
    row_conflict_map: &[Option<usize>],
    visible_row_indices: &[usize],
    visible_ix: usize,
) -> Option<usize> {
    let row_ix = *visible_row_indices.get(visible_ix)?;
    row_conflict_map.get(row_ix).copied().flatten()
}

/// Represents a visible row in the three-way view when hide-resolved is active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreeWayVisibleItem {
    /// A normal line at the given index in the three-way data.
    Line(usize),
    /// A collapsed summary row for a resolved conflict block (by conflict index).
    CollapsedBlock(usize),
}

/// Build the mapping from visible row indices to actual three-way data items.
///
/// When `hide_resolved` is false, every line maps directly.
/// When true, resolved conflict ranges are collapsed to a single summary row.
pub fn build_three_way_visible_map(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> Vec<ThreeWayVisibleItem> {
    if !hide_resolved {
        return (0..total_lines).map(ThreeWayVisibleItem::Line).collect();
    }

    let resolved_blocks: Vec<bool> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b.resolved),
            _ => None,
        })
        .collect();

    let mut visible = Vec::with_capacity(total_lines);
    let mut line = 0usize;
    while line < total_lines {
        if let Some((range_ix, range)) = conflict_ranges
            .iter()
            .enumerate()
            .find(|(_, r)| r.contains(&line))
            .filter(|(ri, _)| resolved_blocks.get(*ri).copied().unwrap_or(false))
        {
            // Emit one collapsed summary row and skip the rest of the range.
            visible.push(ThreeWayVisibleItem::CollapsedBlock(range_ix));
            line = range.end;
            continue;
        }
        visible.push(ThreeWayVisibleItem::Line(line));
        line += 1;
    }
    visible
}

/// Find the visible index for the first line of a conflict range, or the
/// collapsed block entry. Returns `None` if the range is not visible.
pub fn visible_index_for_conflict(
    visible_map: &[ThreeWayVisibleItem],
    conflict_ranges: &[std::ops::Range<usize>],
    range_ix: usize,
) -> Option<usize> {
    let range = conflict_ranges.get(range_ix)?;
    visible_map.iter().position(|item| match item {
        ThreeWayVisibleItem::Line(ix) => range.contains(ix),
        ThreeWayVisibleItem::CollapsedBlock(ci) => *ci == range_ix,
    })
}

/// Build unresolved-only visible navigation entries for three-way views.
///
/// Returns visible indices in unresolved queue order.
pub fn unresolved_visible_nav_entries_for_three_way(
    segments: &[ConflictSegment],
    visible_map: &[ThreeWayVisibleItem],
    conflict_ranges: &[std::ops::Range<usize>],
) -> Vec<usize> {
    unresolved_conflict_indices(segments)
        .into_iter()
        .filter_map(|conflict_ix| {
            visible_index_for_conflict(visible_map, conflict_ranges, conflict_ix)
        })
        .collect()
}

pub fn compute_three_way_word_highlights(
    base_lines: &[gpui::SharedString],
    ours_lines: &[gpui::SharedString],
    theirs_lines: &[gpui::SharedString],
    marker_segments: &[ConflictSegment],
) -> (WordHighlights, WordHighlights, WordHighlights) {
    let len = base_lines
        .len()
        .max(ours_lines.len())
        .max(theirs_lines.len());
    let mut wh_base: WordHighlights = vec![None; len];
    let mut wh_ours: WordHighlights = vec![None; len];
    let mut wh_theirs: WordHighlights = vec![None; len];

    fn merge_line_ranges(
        highlights: &mut WordHighlights,
        line_ix: usize,
        ranges: Vec<std::ops::Range<usize>>,
    ) {
        if ranges.is_empty() {
            return;
        }
        let Some(slot) = highlights.get_mut(line_ix) else {
            return;
        };
        match slot {
            Some(existing) => {
                *existing = merge_ranges(existing, &ranges);
            }
            None => {
                *slot = Some(ranges);
            }
        }
    }

    fn line_index(start: usize, line_no: Option<u32>) -> Option<usize> {
        let local = usize::try_from(line_no?).ok()?.checked_sub(1)?;
        start.checked_add(local)
    }

    fn full_line_range(
        lines: &[gpui::SharedString],
        line_ix: usize,
    ) -> Vec<std::ops::Range<usize>> {
        let Some(line) = lines.get(line_ix).map(|s| s.as_ref()) else {
            return Vec::new();
        };
        if line.is_empty() {
            return Vec::new();
        }
        std::iter::once(0..line.len()).collect()
    }

    struct HighlightSide<'a> {
        global_start: usize,
        lines: &'a [gpui::SharedString],
    }

    fn apply_aligned_word_highlights(
        old_text: &str,
        new_text: &str,
        old_side: HighlightSide<'_>,
        new_side: HighlightSide<'_>,
        old_highlights: &mut WordHighlights,
        new_highlights: &mut WordHighlights,
    ) {
        use gitcomet_core::file_diff::FileDiffRowKind;

        let rows = gitcomet_core::file_diff::side_by_side_rows(old_text, new_text);
        for row in rows {
            match row.kind {
                FileDiffRowKind::Modify => {
                    let old = row.old.as_deref().unwrap_or("");
                    let new = row.new.as_deref().unwrap_or("");
                    let (old_ranges, new_ranges) =
                        super::word_diff::capped_word_diff_ranges(old, new);

                    if let Some(ix) = line_index(old_side.global_start, row.old_line) {
                        merge_line_ranges(old_highlights, ix, old_ranges);
                    }
                    if let Some(ix) = line_index(new_side.global_start, row.new_line) {
                        merge_line_ranges(new_highlights, ix, new_ranges);
                    }
                }
                FileDiffRowKind::Remove => {
                    if let Some(ix) = line_index(old_side.global_start, row.old_line) {
                        merge_line_ranges(old_highlights, ix, full_line_range(old_side.lines, ix));
                    }
                }
                FileDiffRowKind::Add => {
                    if let Some(ix) = line_index(new_side.global_start, row.new_line) {
                        merge_line_ranges(new_highlights, ix, full_line_range(new_side.lines, ix));
                    }
                }
                FileDiffRowKind::Context => {}
            }
        }
    }

    let mut base_offset = 0usize;
    let mut ours_offset = 0usize;
    let mut theirs_offset = 0usize;
    for seg in marker_segments {
        match seg {
            ConflictSegment::Text(text) => {
                let n = usize::try_from(text_line_count(text)).unwrap_or(0);
                base_offset = base_offset.saturating_add(n);
                ours_offset = ours_offset.saturating_add(n);
                theirs_offset = theirs_offset.saturating_add(n);
            }
            ConflictSegment::Block(block) => {
                if let Some(base) = block.base.as_deref() {
                    apply_aligned_word_highlights(
                        base,
                        &block.ours,
                        HighlightSide {
                            global_start: base_offset,
                            lines: base_lines,
                        },
                        HighlightSide {
                            global_start: ours_offset,
                            lines: ours_lines,
                        },
                        &mut wh_base,
                        &mut wh_ours,
                    );
                    apply_aligned_word_highlights(
                        base,
                        &block.theirs,
                        HighlightSide {
                            global_start: base_offset,
                            lines: base_lines,
                        },
                        HighlightSide {
                            global_start: theirs_offset,
                            lines: theirs_lines,
                        },
                        &mut wh_base,
                        &mut wh_theirs,
                    );
                }
                // Local/Remote highlighting must align by diff rows, not absolute same-row index.
                apply_aligned_word_highlights(
                    &block.ours,
                    &block.theirs,
                    HighlightSide {
                        global_start: ours_offset,
                        lines: ours_lines,
                    },
                    HighlightSide {
                        global_start: theirs_offset,
                        lines: theirs_lines,
                    },
                    &mut wh_ours,
                    &mut wh_theirs,
                );

                let base_count =
                    usize::try_from(text_line_count(block.base.as_deref().unwrap_or_default()))
                        .unwrap_or(0);
                let ours_count = usize::try_from(text_line_count(&block.ours)).unwrap_or(0);
                let theirs_count = usize::try_from(text_line_count(&block.theirs)).unwrap_or(0);
                base_offset = base_offset.saturating_add(base_count);
                ours_offset = ours_offset.saturating_add(ours_count);
                theirs_offset = theirs_offset.saturating_add(theirs_count);
            }
        }
    }

    (wh_base, wh_ours, wh_theirs)
}

fn merge_ranges(
    a: &[std::ops::Range<usize>],
    b: &[std::ops::Range<usize>],
) -> Vec<std::ops::Range<usize>> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }
    let mut combined: Vec<std::ops::Range<usize>> = Vec::with_capacity(a.len() + b.len());
    combined.extend_from_slice(a);
    combined.extend_from_slice(b);
    combined.sort_by_key(|r| (r.start, r.end));
    let mut out: Vec<std::ops::Range<usize>> = Vec::with_capacity(combined.len());
    for r in combined {
        if let Some(last) = out.last_mut().filter(|l| r.start <= l.end) {
            last.end = last.end.max(r.end);
            continue;
        }
        out.push(r);
    }
    out
}

/// Per-line pair of (old, new) word-highlight ranges for two-way diff.
pub type TwoWayWordHighlights =
    Vec<Option<(Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>)>>;

pub fn compute_two_way_word_highlights(
    diff_rows: &[gitcomet_core::file_diff::FileDiffRow],
) -> TwoWayWordHighlights {
    diff_rows
        .iter()
        .map(|row| {
            if row.kind != gitcomet_core::file_diff::FileDiffRowKind::Modify {
                return None;
            }
            let old = row.old.as_deref().unwrap_or("");
            let new = row.new.as_deref().unwrap_or("");
            let (old_ranges, new_ranges) = super::word_diff::capped_word_diff_ranges(old, new);
            if old_ranges.is_empty() && new_ranges.is_empty() {
                None
            } else {
                Some((old_ranges, new_ranges))
            }
        })
        .collect()
}

/// When conflict markers use 2-way style (no `|||||||` base section), `block.base`
/// will be `None` even though the git ancestor content (index stage :1:) is available.
/// This function populates `block.base` by using the Text segments as anchors to
/// locate the corresponding base content in the ancestor file.
pub fn populate_block_bases_from_ancestor(segments: &mut [ConflictSegment], ancestor_text: &str) {
    if ancestor_text.is_empty() {
        return;
    }
    let any_missing = segments
        .iter()
        .any(|s| matches!(s, ConflictSegment::Block(b) if b.base.is_none()));
    if !any_missing {
        return;
    }

    // Find each Text segment's byte position in the ancestor file.
    // Text segments are the non-conflicting parts that exist in all three versions.
    let mut text_byte_ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut cursor = 0usize;
    for seg in segments.iter() {
        if let ConflictSegment::Text(text) = seg {
            if let Some(rel) = ancestor_text[cursor..].find(text.as_str()) {
                let start = cursor + rel;
                let end = start + text.len();
                text_byte_ranges.push(start..end);
                cursor = end;
            } else {
                // Text not found in ancestor – bail out.
                return;
            }
        }
    }

    // Extract base content for each block from the gaps between text positions.
    let mut text_idx = 0usize;
    let mut prev_end = 0usize;
    for seg in segments.iter_mut() {
        match seg {
            ConflictSegment::Text(_) => {
                prev_end = text_byte_ranges[text_idx].end;
                text_idx += 1;
            }
            ConflictSegment::Block(block) => {
                if block.base.is_some() {
                    continue;
                }
                let next_start = text_byte_ranges
                    .get(text_idx)
                    .map(|r| r.start)
                    .unwrap_or(ancestor_text.len());
                block.base = Some(ancestor_text[prev_end..next_start].to_string());
            }
        }
    }
}

/// Check whether the given text still contains git conflict markers.
/// Used as a safety gate before "Save & stage" to warn the user about unresolved conflicts.
pub fn text_contains_conflict_markers(text: &str) -> bool {
    gitcomet_core::services::validate_conflict_resolution_text(text).has_conflict_markers
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConflictStageSafetyCheck {
    pub has_conflict_markers: bool,
    pub unresolved_blocks: usize,
}

impl ConflictStageSafetyCheck {
    pub fn requires_confirmation(self) -> bool {
        self.has_conflict_markers || self.unresolved_blocks > 0
    }
}

/// Compute stage-safety status for the current conflict resolver output/state.
///
/// This gate is stricter than marker-only checks: unresolved conflict blocks
/// should still require explicit confirmation even if the current output text
/// no longer contains marker lines.
pub fn conflict_stage_safety_check(
    output_text: &str,
    segments: &[ConflictSegment],
) -> ConflictStageSafetyCheck {
    let total_blocks = conflict_count(segments);
    let resolved_blocks = resolved_conflict_count(segments);
    ConflictStageSafetyCheck {
        has_conflict_markers: text_contains_conflict_markers(output_text),
        unresolved_blocks: total_blocks.saturating_sub(resolved_blocks),
    }
}

/// Split resolved output into one logical row per newline for outline rendering.
///
/// Uses `split('\n')` so trailing newlines are preserved as a final empty row.
pub fn split_output_lines_for_outline(output: &str) -> Vec<String> {
    output.split('\n').map(|line| line.to_string()).collect()
}

pub fn append_lines_to_output(output: &str, lines: &[String]) -> String {
    if lines.is_empty() {
        return output.to_string();
    }

    let needs_leading_nl = !output.is_empty() && !output.ends_with('\n');
    let extra_len: usize =
        lines.iter().map(|l| l.len()).sum::<usize>() + lines.len() + usize::from(needs_leading_nl);
    let mut out = String::with_capacity(output.len() + extra_len);
    out.push_str(output);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    out.push('\n');
    out
}

// ---------------------------------------------------------------------------
// Provenance mapping: classify resolved output lines as A/B/C/Manual
// ---------------------------------------------------------------------------

/// Source lines from the three input panes, used for provenance matching.
///
/// In three-way mode: A = Base, B = Ours, C = Theirs.
/// In two-way mode: A = Ours (old), B = Theirs (new), C is empty.
pub struct SourceLines<'a> {
    pub a: &'a [gpui::SharedString],
    pub b: &'a [gpui::SharedString],
    pub c: &'a [gpui::SharedString],
}

/// Compute per-line provenance metadata for the resolved output.
///
/// Each output line is compared (exact text equality) against every source line
/// in A, B, C. The first match found (priority: A, B, C) wins; if none match
/// the line is labeled `Manual`.
pub fn compute_resolved_line_provenance(
    output_lines: &[String],
    sources: &SourceLines<'_>,
) -> Vec<ResolvedLineMeta> {
    // Build lookup tables: content -> Vec<(source, 1-based line_no)>
    // We iterate sources in priority order (A, B, C) and take the first match.
    let mut result = Vec::with_capacity(output_lines.len());

    for (out_ix, out_line) in output_lines.iter().enumerate() {
        let trimmed = out_line.as_str();
        let mut found = None;

        // Check A
        for (i, src_line) in sources.a.iter().enumerate() {
            if src_line.as_ref() == trimmed {
                found = Some((ResolvedLineSource::A, (i + 1) as u32));
                break;
            }
        }
        // Check B (only if A didn't match)
        if found.is_none() {
            for (i, src_line) in sources.b.iter().enumerate() {
                if src_line.as_ref() == trimmed {
                    found = Some((ResolvedLineSource::B, (i + 1) as u32));
                    break;
                }
            }
        }
        // Check C (only if A and B didn't match)
        if found.is_none() {
            for (i, src_line) in sources.c.iter().enumerate() {
                if src_line.as_ref() == trimmed {
                    found = Some((ResolvedLineSource::C, (i + 1) as u32));
                    break;
                }
            }
        }

        let (source, input_line) = match found {
            Some((src, line_no)) => (src, Some(line_no)),
            None => (ResolvedLineSource::Manual, None),
        };
        result.push(ResolvedLineMeta {
            output_line: out_ix as u32,
            source,
            input_line,
        });
    }

    result
}

// ---------------------------------------------------------------------------
// Dedupe key index: tracks which source lines are present in resolved output
// ---------------------------------------------------------------------------

/// Build the set of `SourceLineKey`s currently represented in the resolved output.
///
/// Used to gate the plus-icon: a source row's plus-icon is hidden when its key
/// is already in this set (preventing duplicate insertion).
pub fn build_resolved_output_line_sources_index(
    meta: &[ResolvedLineMeta],
    output_lines: &[String],
    view_mode: ConflictResolverViewMode,
) -> rustc_hash::FxHashSet<SourceLineKey> {
    let mut index = rustc_hash::FxHashSet::with_capacity_and_hasher(meta.len(), Default::default());
    for m in meta {
        if m.source == ResolvedLineSource::Manual {
            continue;
        }
        let Some(line_no) = m.input_line else {
            continue;
        };
        let content = output_lines
            .get(m.output_line as usize)
            .map(|s| s.as_str())
            .unwrap_or("");
        index.insert(SourceLineKey::new(view_mode, m.source, line_no, content));
    }
    index
}

/// Check whether a given source line is already present in the resolved output.
///
/// Returns `true` if the source line's key is in the dedupe index — meaning
/// the plus-icon for that row should be hidden.
#[allow(dead_code)]
pub fn is_source_line_in_output(
    index: &rustc_hash::FxHashSet<SourceLineKey>,
    view_mode: ConflictResolverViewMode,
    side: ResolvedLineSource,
    line_no: u32,
    content: &str,
) -> bool {
    let key = SourceLineKey::new(view_mode, side, line_no, content);
    index.contains(&key)
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests;
