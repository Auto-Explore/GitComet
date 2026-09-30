//! Which input line each resolved-output line came from, and when that is
//! too expensive to compute.

use super::*;

/// Full resolved-output provenance is much more expensive in three-way mode,
/// because it builds source-line lookups across all three full documents.
pub(in crate::view::panes::main) const LARGE_RESOLVED_OUTLINE_THREE_WAY_PROVENANCE_MAX_LINES:
    usize = 50_000;
/// Two-way mode still needs a cap, because the source-index alone scales with
/// output-line count even when the diff-row lookup is small.
pub(in crate::view::panes::main) const LARGE_RESOLVED_OUTLINE_TWO_WAY_PROVENANCE_MAX_LINES: usize =
    200_000;

pub(in crate::view::panes::main) fn should_skip_resolved_outline_provenance(
    view_mode: ConflictResolverViewMode,
    output_line_count: usize,
) -> bool {
    match view_mode {
        ConflictResolverViewMode::ThreeWay => {
            output_line_count > LARGE_RESOLVED_OUTLINE_THREE_WAY_PROVENANCE_MAX_LINES
        }
        ConflictResolverViewMode::TwoWayDiff => {
            output_line_count > LARGE_RESOLVED_OUTLINE_TWO_WAY_PROVENANCE_MAX_LINES
        }
    }
}

#[cfg(test)]
pub(in crate::view::panes::main) fn apply_three_way_empty_base_provenance_hints(
    meta: &mut [conflict_resolver::ResolvedLineMeta],
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
) {
    let generated = conflict_resolver::generate_resolved_text(marker_segments);
    if generated != output_text || meta.is_empty() {
        return;
    }

    let mut block_ix = 0usize;
    let mut a_line = 1u32;
    let mut b_line = 1u32;
    let mut c_line = 1u32;

    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                let n = u32::try_from(source_line_count(text)).unwrap_or(0);
                a_line = a_line.saturating_add(n);
                b_line = b_line.saturating_add(n);
                c_line = c_line.saturating_add(n);
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                let a_count =
                    u32::try_from(source_line_count(block.base.as_deref().unwrap_or_default()))
                        .unwrap_or(0);
                let b_count = u32::try_from(source_line_count(&block.ours)).unwrap_or(0);
                let c_count = u32::try_from(source_line_count(&block.theirs)).unwrap_or(0);

                let base_empty = block.base.as_ref().is_none_or(|s| s.is_empty());
                if base_empty
                    && let Some(range) = output_line_range_for_conflict_block_in_text(
                        marker_segments,
                        output_text,
                        block_ix,
                    )
                {
                    let mut output_offset = 0usize;
                    for source in block.choice.iter() {
                        let (source_count, resolved_source, input_line) = match source {
                            gitcomet_core::conflict_output::ConflictOutputSource::Base => {
                                (a_count, conflict_resolver::ResolvedLineSource::A, a_line)
                            }
                            gitcomet_core::conflict_output::ConflictOutputSource::Ours => {
                                (b_count, conflict_resolver::ResolvedLineSource::B, b_line)
                            }
                            gitcomet_core::conflict_output::ConflictOutputSource::Theirs => {
                                (c_count, conflict_resolver::ResolvedLineSource::C, c_line)
                            }
                        };
                        let remaining = range
                            .end
                            .saturating_sub(range.start.saturating_add(output_offset));
                        let take =
                            usize::min(remaining, usize::try_from(source_count).unwrap_or(0));
                        for off in 0..take {
                            if let Some(m) = meta.get_mut(range.start + output_offset + off)
                                && matches!(
                                    m.source,
                                    conflict_resolver::ResolvedLineSource::A
                                        | conflict_resolver::ResolvedLineSource::Manual
                                )
                            {
                                m.source = resolved_source;
                                m.input_line = Some(
                                    input_line.saturating_add(u32::try_from(off).unwrap_or(0)),
                                );
                            }
                        }
                        output_offset = output_offset.saturating_add(take);
                    }
                }

                a_line = a_line.saturating_add(a_count);
                b_line = b_line.saturating_add(b_count);
                c_line = c_line.saturating_add(c_count);
                block_ix = block_ix.saturating_add(1);
            }
        }
    }
}

pub(in crate::view::panes::main) fn apply_conflict_choice_provenance_hints_for_ranges(
    meta: &mut [conflict_resolver::ResolvedLineMeta],
    marker_segments: &[conflict_resolver::ConflictSegment],
    block_ranges: &[Range<usize>],
    view_mode: ConflictResolverViewMode,
) {
    if meta.is_empty() {
        return;
    }

    let assign_range = |meta: &mut [conflict_resolver::ResolvedLineMeta],
                        range: Range<usize>,
                        source: conflict_resolver::ResolvedLineSource,
                        start_line: u32,
                        line_count: u32| {
        let len = range.end.saturating_sub(range.start);
        for off in 0..len {
            if let Some(m) = meta.get_mut(range.start + off) {
                m.source = source;
                let off_u32 = u32::try_from(off).unwrap_or(u32::MAX);
                m.input_line = (off_u32 < line_count).then_some(start_line.saturating_add(off_u32));
            }
        }
    };

    let assign_both_range = |meta: &mut [conflict_resolver::ResolvedLineMeta],
                             range: Range<usize>,
                             first_source: conflict_resolver::ResolvedLineSource,
                             first_start: u32,
                             first_count: u32,
                             second_source: conflict_resolver::ResolvedLineSource,
                             second_start: u32,
                             second_count: u32| {
        let len = range.end.saturating_sub(range.start);
        let first_count_usize = usize::try_from(first_count).unwrap_or(0);
        let first_take = len.min(first_count_usize);
        assign_range(
            meta,
            range.start..range.start.saturating_add(first_take),
            first_source,
            first_start,
            first_count,
        );
        assign_range(
            meta,
            range.start.saturating_add(first_take)..range.end,
            second_source,
            second_start,
            second_count,
        );
    };

    let mut block_ix = 0usize;
    let mut a_line = 1u32;
    let mut b_line = 1u32;
    let mut c_line = 1u32;

    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                let n = u32::try_from(source_line_count(text)).unwrap_or(0);
                a_line = a_line.saturating_add(n);
                b_line = b_line.saturating_add(n);
                if view_mode == ConflictResolverViewMode::ThreeWay {
                    c_line = c_line.saturating_add(n);
                }
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                let (a_count, b_count, c_count) = match view_mode {
                    ConflictResolverViewMode::ThreeWay => (
                        u32::try_from(source_line_count(block.base.as_deref().unwrap_or_default()))
                            .unwrap_or(0),
                        u32::try_from(source_line_count(&block.ours)).unwrap_or(0),
                        u32::try_from(source_line_count(&block.theirs)).unwrap_or(0),
                    ),
                    ConflictResolverViewMode::TwoWayDiff => (
                        u32::try_from(source_line_count(&block.ours)).unwrap_or(0),
                        u32::try_from(source_line_count(&block.theirs)).unwrap_or(0),
                        0,
                    ),
                };

                if let Some(range) = block_ranges.get(block_ix).cloned() {
                    if !block.resolved && block.choice.is_empty() {
                        assign_range(
                            meta,
                            range,
                            conflict_resolver::ResolvedLineSource::Manual,
                            0,
                            0,
                        );
                    } else {
                        match (view_mode, block.choice) {
                            (
                                ConflictResolverViewMode::ThreeWay,
                                conflict_resolver::ConflictChoice::Base,
                            ) => {
                                assign_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::A,
                                    a_line,
                                    a_count,
                                );
                            }
                            (
                                ConflictResolverViewMode::ThreeWay,
                                conflict_resolver::ConflictChoice::Ours,
                            ) => {
                                assign_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::B,
                                    b_line,
                                    b_count,
                                );
                            }
                            (
                                ConflictResolverViewMode::ThreeWay,
                                conflict_resolver::ConflictChoice::Theirs,
                            ) => {
                                assign_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::C,
                                    c_line,
                                    c_count,
                                );
                            }
                            (
                                ConflictResolverViewMode::ThreeWay,
                                conflict_resolver::ConflictChoice::Both,
                            ) => {
                                assign_both_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::B,
                                    b_line,
                                    b_count,
                                    conflict_resolver::ResolvedLineSource::C,
                                    c_line,
                                    c_count,
                                );
                            }
                            (
                                ConflictResolverViewMode::TwoWayDiff,
                                conflict_resolver::ConflictChoice::Theirs,
                            ) => {
                                assign_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::B,
                                    b_line,
                                    b_count,
                                );
                            }
                            (
                                ConflictResolverViewMode::TwoWayDiff,
                                conflict_resolver::ConflictChoice::Both,
                            ) => {
                                assign_both_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::A,
                                    a_line,
                                    a_count,
                                    conflict_resolver::ResolvedLineSource::B,
                                    b_line,
                                    b_count,
                                );
                            }
                            // In two-way mode, Base falls back to local-side semantics.
                            (
                                ConflictResolverViewMode::TwoWayDiff,
                                conflict_resolver::ConflictChoice::Base,
                            )
                            | (
                                ConflictResolverViewMode::TwoWayDiff,
                                conflict_resolver::ConflictChoice::Ours,
                            ) => {
                                assign_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::A,
                                    a_line,
                                    a_count,
                                );
                            }
                            _ => {
                                // Arbitrary ordered combinations are rendered
                                // correctly; this compact hint table treats their
                                // mixed provenance as manual.
                            }
                        }
                    }
                }

                a_line = a_line.saturating_add(a_count);
                b_line = b_line.saturating_add(b_count);
                c_line = c_line.saturating_add(c_count);
                block_ix = block_ix.saturating_add(1);
            }
        }
    }
}

pub(in crate::view::panes::main) fn apply_conflict_choice_provenance_hints(
    meta: &mut [conflict_resolver::ResolvedLineMeta],
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    view_mode: ConflictResolverViewMode,
) {
    let generated = conflict_resolver::generate_resolved_text(marker_segments);
    if generated != output_text {
        return;
    }

    let Some(block_ranges) =
        resolved_output_conflict_block_ranges_in_text(marker_segments, output_text)
    else {
        return;
    };

    apply_conflict_choice_provenance_hints_for_ranges(
        meta,
        marker_segments,
        block_ranges.as_slice(),
        view_mode,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_outline_provenance_skip_thresholds_match_view_mode() {
        assert!(!should_skip_resolved_outline_provenance(
            ConflictResolverViewMode::ThreeWay,
            LARGE_RESOLVED_OUTLINE_THREE_WAY_PROVENANCE_MAX_LINES,
        ));
        assert!(should_skip_resolved_outline_provenance(
            ConflictResolverViewMode::ThreeWay,
            LARGE_RESOLVED_OUTLINE_THREE_WAY_PROVENANCE_MAX_LINES + 1,
        ));
        assert!(!should_skip_resolved_outline_provenance(
            ConflictResolverViewMode::TwoWayDiff,
            LARGE_RESOLVED_OUTLINE_TWO_WAY_PROVENANCE_MAX_LINES,
        ));
        assert!(should_skip_resolved_outline_provenance(
            ConflictResolverViewMode::TwoWayDiff,
            LARGE_RESOLVED_OUTLINE_TWO_WAY_PROVENANCE_MAX_LINES + 1,
        ));
    }
}
