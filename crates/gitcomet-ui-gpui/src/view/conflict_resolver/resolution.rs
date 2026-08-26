use super::*;

pub(in crate::view) fn content_matches_block_choice(block: &ConflictBlock, content: &str) -> bool {
    use gitcomet_core::conflict_output::ConflictOutputSource;

    let mut selected = String::new();
    for source in block.choice.iter() {
        match source {
            ConflictOutputSource::Base => {
                let Some(base) = block.base.as_deref() else {
                    return false;
                };
                selected.push_str(base);
            }
            ConflictOutputSource::Ours => selected.push_str(&block.ours),
            ConflictOutputSource::Theirs => selected.push_str(&block.theirs),
        }
    }
    selected == content
}

pub(in crate::view) fn resolution_for_choice(
    choice: ConflictChoice,
    has_base: bool,
) -> gitcomet_core::conflict_session::ConflictRegionResolution {
    use gitcomet_core::conflict_output::ConflictOutputSource;
    use gitcomet_core::conflict_session::ConflictRegionResolution;
    use gitcomet_core::merge::{MergeSource, OrderedSelection};

    let selection = OrderedSelection::from_sources(choice.iter().filter_map(|source| {
        match (has_base, source) {
            (true, ConflictOutputSource::Base) => Some(MergeSource::A),
            (true, ConflictOutputSource::Ours) => Some(MergeSource::B),
            (true, ConflictOutputSource::Theirs) => Some(MergeSource::C),
            (false, ConflictOutputSource::Base) => None,
            (false, ConflictOutputSource::Ours) => Some(MergeSource::A),
            (false, ConflictOutputSource::Theirs) => Some(MergeSource::B),
        }
    }));
    if selection.is_empty() {
        ConflictRegionResolution::Unresolved
    } else {
        ConflictRegionResolution::Sources(selection)
    }
}

pub(in crate::view) fn choice_for_selection(
    selection: &gitcomet_core::merge::OrderedSelection,
    has_base: bool,
) -> Option<ConflictChoice> {
    use gitcomet_core::conflict_output::{ConflictOutputChoice, ConflictOutputSource};
    use gitcomet_core::merge::MergeSource;

    let mut choice = ConflictOutputChoice::empty();
    for source in selection.iter() {
        let output_source = match (has_base, source) {
            (true, MergeSource::A) => ConflictOutputSource::Base,
            (true, MergeSource::B) => ConflictOutputSource::Ours,
            (true, MergeSource::C) => ConflictOutputSource::Theirs,
            (false, MergeSource::A) => ConflictOutputSource::Ours,
            (false, MergeSource::B) => ConflictOutputSource::Theirs,
            (false, MergeSource::C) => return None,
        };
        choice.append(output_source);
    }
    Some(choice)
}

/// Byte ownership for the editable output of each displayed conflict block.
///
/// The editor text is authoritative; these ranges only decide which bytes
/// belong to which conflict when deriving session resolutions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResolvedOutputBlockMap {
    pub(super) ranges: Vec<Range<usize>>,
    pub(super) text_len: usize,
}

impl ResolvedOutputBlockMap {
    pub(in crate::view) fn from_segments(segments: &[ConflictSegment]) -> Self {
        let mut ranges = Vec::with_capacity(conflict_count(segments));
        let mut cursor = 0usize;
        for segment in segments {
            match segment {
                ConflictSegment::Text(text) => {
                    cursor = cursor.saturating_add(text.len());
                }
                ConflictSegment::Block(block) => {
                    let start = cursor;
                    cursor = cursor.saturating_add(editable_conflict_block_len(block));
                    ranges.push(start..cursor);
                }
            }
        }
        Self {
            ranges,
            text_len: cursor,
        }
    }

    pub(in crate::view) fn ranges(&self) -> &[Range<usize>] {
        &self.ranges
    }

    pub(in crate::view) fn is_valid_for(
        &self,
        segments: &[ConflictSegment],
        output_text: &(impl ResolvedOutputSource + ?Sized),
    ) -> bool {
        self.text_len == output_text.len()
            && self.ranges.len() == conflict_count(segments)
            && self.ranges.iter().all(|range| {
                range.start <= range.end
                    && range.end <= output_text.len()
                    && output_text.is_char_boundary(range.start)
                    && output_text.is_char_boundary(range.end)
            })
            && self
                .ranges
                .windows(2)
                .all(|ranges| ranges[0].end <= ranges[1].start)
    }

    pub(in crate::view) fn block_slice<'a>(
        &self,
        segments: &[ConflictSegment],
        output_text: &'a str,
        block_index: usize,
    ) -> Option<&'a str> {
        self.is_valid_for(segments, output_text).then_some(())?;
        output_text.get(self.ranges.get(block_index)?.clone())
    }

    fn owner_for_edit_start(&self, start: usize) -> Option<usize> {
        self.ranges
            .iter()
            .position(|range| range.start <= start && start < range.end)
            .or_else(|| {
                self.ranges
                    .iter()
                    .position(|range| range.is_empty() && range.start == start)
            })
    }

    pub(in crate::view) fn apply_edit_delta(
        &mut self,
        old_range: Range<usize>,
        new_range: Range<usize>,
    ) -> bool {
        if old_range.start > old_range.end
            || old_range.end > self.text_len
            || new_range.start != old_range.start
            || new_range.start > new_range.end
        {
            return false;
        }

        let owner = self.owner_for_edit_start(old_range.start);
        let shift = new_range.len() as isize - old_range.len() as isize;
        let shift_offset = |offset: usize| {
            if shift >= 0 {
                offset.saturating_add(shift as usize)
            } else {
                offset.saturating_sub((-shift) as usize)
            }
        };

        for (block_index, range) in self.ranges.iter_mut().enumerate() {
            let previous = range.clone();
            if owner == Some(block_index) {
                let start = if previous.start < old_range.start {
                    previous.start
                } else {
                    old_range.start
                };
                let end = if previous.end > old_range.end {
                    shift_offset(previous.end)
                } else {
                    new_range.end
                };
                *range = start..end.max(start);
                continue;
            }

            if previous.end <= old_range.start {
                continue;
            }
            if previous.start >= old_range.end {
                *range = shift_offset(previous.start)..shift_offset(previous.end);
                continue;
            }

            let keeps_left = previous.start < old_range.start;
            let keeps_right = previous.end > old_range.end;
            *range = match (keeps_left, keeps_right) {
                (true, true) => previous.start..shift_offset(previous.end),
                (true, false) => previous.start..old_range.start,
                (false, true) => new_range.end..shift_offset(previous.end),
                // A later block deleted by an edit owned by an earlier block
                // collapses after the replacement, never into the owner's text.
                (false, false) => new_range.end..new_range.end,
            };
        }

        self.text_len = shift_offset(self.text_len);
        self.ranges
            .windows(2)
            .all(|ranges| ranges[0].end <= ranges[1].start)
            && self
                .ranges
                .iter()
                .all(|range| range.start <= range.end && range.end <= self.text_len)
    }

    pub(in crate::view) fn apply_edit_deltas(
        &mut self,
        deltas: impl IntoIterator<Item = (Range<usize>, Range<usize>)>,
    ) -> bool {
        for (old_range, new_range) in deltas {
            if !self.apply_edit_delta(old_range, new_range) {
                return false;
            }
        }
        true
    }
}

/// Derive per-region session resolution updates from the current resolved output.
///
/// This is used to persist manual resolver edits back into state without
/// requiring marker reparse in the reducer.
pub fn derive_region_resolution_updates_from_output(
    segments: &[ConflictSegment],
    block_region_indices: &[usize],
    block_map: &ResolvedOutputBlockMap,
    output_text: &str,
) -> Option<
    Vec<(
        usize,
        gitcomet_core::conflict_session::ConflictRegionResolution,
    )>,
> {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    if !block_map.is_valid_for(segments, output_text) {
        return None;
    }
    let mut updates = Vec::with_capacity(block_map.ranges().len());

    let mut block_ix = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        let content = block_map.block_slice(segments, output_text, block_ix)?;
        let region_ix = block_region_indices
            .get(block_ix)
            .copied()
            .unwrap_or(block_ix);

        let resolution = if !block.resolved
            && (content.contains(UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER)
                || (!block.choice.is_empty() && content_matches_block_choice(block, content)))
        {
            R::Unresolved
        } else if content.is_empty() && block.choice.is_empty() {
            // Empty bytes alone cannot prove that an empty source was chosen.
            // Only the explicit block choice below carries that intent.
            R::ManualEdit(String::new())
        } else if let Some(choice) = choice_for_resolved_content(block, content) {
            resolution_for_choice(choice, block.base.is_some())
        } else {
            R::ManualEdit(content.to_string())
        };
        updates.push((region_ix, resolution));
        block_ix += 1;
    }

    Some(updates)
}

/// Derive per-region session resolution updates directly from marker segments.
///
/// Streamed resolved-output mode is read-only until explicit materialization,
/// so the block choice state is the source of truth and no full output string
/// needs to be assembled.
pub fn derive_region_resolution_updates_from_segments(
    segments: &[ConflictSegment],
    block_region_indices: &[usize],
) -> Vec<(
    usize,
    gitcomet_core::conflict_session::ConflictRegionResolution,
)> {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let mut updates = Vec::with_capacity(conflict_count(segments));
    let mut block_ix = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        let region_ix = block_region_indices
            .get(block_ix)
            .copied()
            .unwrap_or(block_ix);
        let resolution = if !block.resolved {
            R::Unresolved
        } else {
            resolution_for_choice(block.choice, block.base.is_some())
        };
        updates.push((region_ix, resolution));
        block_ix += 1;
    }
    updates
}

/// Result of applying state-layer region resolutions to UI marker segments.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionRegionApplyResult {
    /// Number of source regions visited/applied.
    pub applied_regions: usize,
    /// Mapping from visible block index -> source `ConflictSession` region index.
    pub block_region_indices: Vec<usize>,
}

/// Result of applying ConflictRegion choices to a plan projection while
/// retaining exact semantic block identities for every visible marker.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlanSessionRegionApplyResult {
    pub block_region_indices: Vec<usize>,
    pub block_plan_indices: Vec<usize>,
}

/// Apply marker-backed region choices to a projection whose marker blocks are
/// identified by merge-plan block index.
///
/// A plan-only automatic delta can become unresolved after a source toggle.
/// Such a block has no ConflictRegion, so it is retained untouched and given a
/// synthetic, out-of-range grouping key while its plan identity remains exact.
pub fn apply_plan_session_region_resolutions_with_index_map(
    segments: &mut Vec<ConflictSegment>,
    session: &gitcomet_core::conflict_session::ConflictSession,
    projected_plan_blocks: &[usize],
) -> Option<PlanSessionRegionApplyResult> {
    if conflict_count(segments) != projected_plan_blocks.len() {
        return None;
    }

    let mut marker_index = 0usize;
    let mut block_region_indices = Vec::new();
    let mut block_plan_indices = Vec::new();
    let mut synced = Vec::with_capacity(segments.len());
    for segment in segments.drain(..) {
        match segment {
            ConflictSegment::Text(text) => append_text_segment(&mut synced, text),
            ConflictSegment::Block(mut block) => {
                let block_index = projected_plan_blocks[marker_index];
                marker_index += 1;
                // Only the plan applies kdiff3's per-row whitespace rule, so
                // carry its verdict onto the display block here rather than
                // re-deriving a weaker one from the block text.
                block.whitespace_only = session
                    .merge_plan
                    .as_ref()
                    .and_then(|plan| plan.blocks.get(block_index))
                    .is_some_and(|plan_block| plan_block.whitespace_conflict);
                let region_index = session
                    .region_plan_blocks
                    .iter()
                    .position(|candidate| *candidate == block_index);
                if let Some(region_index) = region_index
                    && let Some(region) = session.regions.get(region_index)
                    && let Some(materialized) =
                        apply_region_resolution_to_block(&mut block, &region.resolution)
                {
                    append_text_segment(&mut synced, materialized);
                    continue;
                }

                synced.push(ConflictSegment::Block(block));
                block_plan_indices.push(block_index);
                block_region_indices.push(region_index.unwrap_or_else(|| {
                    // Real regions occupy 0..len. Plan-only keys start at len
                    // and stay unique by semantic block index.
                    session.regions.len().saturating_add(block_index)
                }));
            }
        }
    }
    *segments = synced;
    Some(PlanSessionRegionApplyResult {
        block_region_indices,
        block_plan_indices,
    })
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

pub(in crate::view) fn apply_region_resolution_to_block(
    block: &mut ConflictBlock,
    resolution: &gitcomet_core::conflict_session::ConflictRegionResolution,
) -> Option<String> {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    match resolution {
        R::Unresolved => {
            block.choice = ConflictChoice::empty();
            block.resolved = false;
            None
        }
        R::PickBase => {
            if block.base.is_some() {
                block.choice = ConflictChoice::Base;
                block.resolved = true;
            } else {
                block.choice = ConflictChoice::empty();
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
        R::Sources(selection) => {
            if let Some(choice) = choice_for_selection(selection, block.base.is_some()) {
                block.choice = choice;
                block.resolved = !choice.is_empty();
            } else {
                block.choice = ConflictChoice::empty();
                block.resolved = false;
            }
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

/// Apply ordered per-block resolutions to parsed UI marker segments.
///
/// This is used by save/export paths that derive resolutions from the current
/// resolved-output buffer and need to keep manual edits as plain text while
/// preserving untouched unresolved blocks.
pub(in crate::view) fn apply_ordered_region_resolutions(
    segments: &mut Vec<ConflictSegment>,
    resolutions: &[gitcomet_core::conflict_session::ConflictRegionResolution],
) -> usize {
    if segments.is_empty() || resolutions.is_empty() {
        return 0;
    }

    let mut applied = 0usize;
    let mut block_ix = 0usize;
    let mut synced: Vec<ConflictSegment> = Vec::with_capacity(segments.len());

    for seg in segments.drain(..) {
        match seg {
            ConflictSegment::Text(text) => append_text_segment(&mut synced, text),
            ConflictSegment::Block(mut block) => {
                if let Some(resolution) = resolutions.get(block_ix) {
                    if let Some(materialized_text) =
                        apply_region_resolution_to_block(&mut block, resolution)
                    {
                        append_text_segment(&mut synced, materialized_text);
                    } else {
                        synced.push(ConflictSegment::Block(block));
                    }
                    applied += 1;
                } else {
                    synced.push(ConflictSegment::Block(block));
                }
                block_ix += 1;
            }
        }
    }

    *segments = synced;
    applied
}

/// Apply state-layer region resolutions to parsed UI marker segments.
///
/// This allows resolver rebuilds to preserve choices tracked in
/// `RepoState.conflict_state.conflict_session`, and materializes manual/auto-resolved
/// non-side-pick text into plain `Text` segments when needed.
///
/// Returns how many conflict regions were applied.
#[cfg(test)]
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
