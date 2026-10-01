//! Edits to the conflict segment list: splitting a block into subchunks,
//! grouping picks, and resetting, removing, or appending blocks.

use super::*;

pub(in crate::view::panes::main) fn push_conflict_text_segment(
    segments: &mut Vec<conflict_resolver::ConflictSegment>,
    text: impl Into<conflict_resolver::ConflictText>,
) {
    let text = text.into();
    if text.is_empty() {
        return;
    }
    if let Some(conflict_resolver::ConflictSegment::Text(prev)) = segments.last_mut() {
        prev.push_str(text.as_str());
        return;
    }
    segments.push(conflict_resolver::ConflictSegment::Text(text));
}

pub(in crate::view::panes::main) fn split_target_conflict_block_into_subchunks(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    target_conflict_ix: usize,
) -> bool {
    use gitcomet_core::conflict_session::{Subchunk, split_conflict_into_subchunks};

    let Some(target_block) = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .nth(target_conflict_ix)
        .cloned()
    else {
        return false;
    };
    if target_block.resolved {
        return false;
    }

    enum SplitMode {
        Subchunks(Vec<Subchunk>),
        DecisionRanges {
            regions: Vec<UnresolvedDecisionRegion>,
            choice_is_ours: bool,
        },
    }
    let split_mode = if let Some(base) = target_block.base.as_deref() {
        split_conflict_into_subchunks(base, &target_block.ours, &target_block.theirs).and_then(
            |subchunks| {
                let split_conflict_count = subchunks
                    .iter()
                    .filter(|subchunk| matches!(subchunk, Subchunk::Conflict { .. }))
                    .count();
                (split_conflict_count > 1).then_some(SplitMode::Subchunks(subchunks))
            },
        )
    } else {
        None
    }
    .or_else(|| {
        let (analysis_block, choice_is_ours) =
            if target_block.choice == conflict_resolver::ConflictChoice::Ours {
                (target_block.clone(), true)
            } else if target_block.choice == conflict_resolver::ConflictChoice::Theirs {
                (target_block.clone(), false)
            } else if target_block.choice.is_empty() {
                let mut analysis_block = target_block.clone();
                analysis_block.choice = conflict_resolver::ConflictChoice::Ours;
                (analysis_block, true)
            } else {
                return None;
            };
        unresolved_decision_regions_for_block(&analysis_block).and_then(|regions| {
            (regions.len() > 1).then_some(SplitMode::DecisionRanges {
                regions,
                choice_is_ours,
            })
        })
    });
    let Some(split_mode) = split_mode else {
        return false;
    };

    let mut next_segments = Vec::with_capacity(marker_segments.len().saturating_add(4));
    let mut next_region_indices =
        Vec::with_capacity(conflict_region_indices.len().saturating_add(4));
    let mut seen_conflict_ix = 0usize;
    for seg in marker_segments.drain(..) {
        match seg {
            conflict_resolver::ConflictSegment::Block(block) => {
                let region_ix = conflict_region_indices
                    .get(seen_conflict_ix)
                    .copied()
                    .unwrap_or(seen_conflict_ix);
                if seen_conflict_ix == target_conflict_ix {
                    match &split_mode {
                        SplitMode::Subchunks(subchunks) => {
                            for subchunk in subchunks {
                                match subchunk {
                                    Subchunk::Resolved(text) => {
                                        push_conflict_text_segment(
                                            &mut next_segments,
                                            text.clone(),
                                        );
                                    }
                                    Subchunk::Conflict { base, ours, theirs } => {
                                        next_segments.push(
                                            conflict_resolver::ConflictSegment::Block(
                                                conflict_resolver::ConflictBlock {
                                                    base: Some(base.clone().into()),
                                                    ours: ours.clone().into(),
                                                    theirs: theirs.clone().into(),
                                                    choice: target_block.choice,
                                                    resolved: false,
                                                    // Subchunks of a whitespace-only
                                                    // block are whitespace-only too.
                                                    whitespace_only: target_block.whitespace_only,
                                                },
                                            ),
                                        );
                                        next_region_indices.push(region_ix);
                                    }
                                }
                            }
                        }
                        SplitMode::DecisionRanges {
                            regions,
                            choice_is_ours,
                        } => {
                            let (selected_text, alternate_text) = if *choice_is_ours {
                                (&target_block.ours, &target_block.theirs)
                            } else {
                                (&target_block.theirs, &target_block.ours)
                            };
                            let selected_total_lines = source_line_count(selected_text);
                            let mut selected_cursor = 0usize;
                            for region in regions {
                                let prefix = slice_text_by_line_range(
                                    selected_text,
                                    selected_cursor..region.selected_line_range.start,
                                );
                                push_conflict_text_segment(&mut next_segments, prefix);

                                let selected_fragment = slice_text_by_line_range(
                                    selected_text,
                                    region.selected_line_range.clone(),
                                );
                                let alternate_fragment = slice_text_by_line_range(
                                    alternate_text,
                                    region.alternate_line_range.clone(),
                                );
                                let (ours, theirs) = if *choice_is_ours {
                                    (selected_fragment, alternate_fragment)
                                } else {
                                    (alternate_fragment, selected_fragment)
                                };
                                next_segments.push(conflict_resolver::ConflictSegment::Block(
                                    conflict_resolver::ConflictBlock {
                                        base: None,
                                        ours: ours.into(),
                                        theirs: theirs.into(),
                                        choice: target_block.choice,
                                        resolved: false,
                                        whitespace_only: target_block.whitespace_only,
                                    },
                                ));
                                next_region_indices.push(region_ix);
                                selected_cursor = region.selected_line_range.end;
                            }
                            let suffix = slice_text_by_line_range(
                                selected_text,
                                selected_cursor..selected_total_lines,
                            );
                            push_conflict_text_segment(&mut next_segments, suffix);
                        }
                    }
                } else {
                    next_segments.push(conflict_resolver::ConflictSegment::Block(block));
                    next_region_indices.push(region_ix);
                }
                seen_conflict_ix = seen_conflict_ix.saturating_add(1);
            }
            conflict_resolver::ConflictSegment::Text(text) => {
                push_conflict_text_segment(&mut next_segments, text);
            }
        }
    }

    *marker_segments = next_segments;
    *conflict_region_indices = next_region_indices;
    true
}

pub(in crate::view::panes::main) fn conflict_region_index_is_unique(
    conflict_region_indices: &[usize],
    region_ix: usize,
) -> bool {
    conflict_region_indices
        .iter()
        .filter(|&&ix| ix == region_ix)
        .take(2)
        .count()
        <= 1
}

pub(in crate::view::panes::main) fn conflict_block_matches_group(
    block: &conflict_resolver::ConflictBlock,
    region_ix: usize,
    target_block: &conflict_resolver::ConflictBlock,
    target_region_ix: usize,
) -> bool {
    region_ix == target_region_ix
        && block.base == target_block.base
        && block.ours == target_block.ours
        && block.theirs == target_block.theirs
}

pub(in crate::view::panes::main) fn conflict_group_member_indices_for_ix(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
) -> Vec<usize> {
    let mut blocks: Vec<&conflict_resolver::ConflictBlock> = Vec::new();
    // True when a block has non-empty text between it and the previous block.
    let mut separated_before: Vec<bool> = Vec::new();
    let mut saw_text_since_prev_block = false;
    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                if !text.is_empty() {
                    saw_text_since_prev_block = true;
                }
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                separated_before.push(saw_text_since_prev_block);
                blocks.push(block);
                saw_text_since_prev_block = false;
            }
        }
    }
    let Some(target_block) = blocks.get(conflict_ix).copied() else {
        return Vec::new();
    };
    let target_region_ix = conflict_region_indices
        .get(conflict_ix)
        .copied()
        .unwrap_or(conflict_ix);

    let mut start = conflict_ix;
    while start > 0 {
        if separated_before[start] {
            break;
        }
        let prev_ix = start - 1;
        let prev_block = blocks[prev_ix];
        let prev_region_ix = conflict_region_indices
            .get(prev_ix)
            .copied()
            .unwrap_or(prev_ix);
        if conflict_block_matches_group(prev_block, prev_region_ix, target_block, target_region_ix)
        {
            start = prev_ix;
        } else {
            break;
        }
    }

    let mut end_exclusive = conflict_ix + 1;
    while end_exclusive < blocks.len() {
        let next_ix = end_exclusive;
        if separated_before[next_ix] {
            break;
        }
        let next_block = blocks[next_ix];
        let next_region_ix = conflict_region_indices
            .get(next_ix)
            .copied()
            .unwrap_or(next_ix);
        if conflict_block_matches_group(next_block, next_region_ix, target_block, target_region_ix)
        {
            end_exclusive += 1;
        } else {
            break;
        }
    }

    (start..end_exclusive).collect()
}

pub(in crate::view::panes::main) fn conflict_group_selected_choices_for_ix(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
) -> Vec<conflict_resolver::ConflictChoice> {
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    if group_indices.is_empty() {
        return Vec::new();
    }
    let blocks: Vec<&conflict_resolver::ConflictBlock> = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .collect();

    let mut has_base = false;
    let mut has_ours = false;
    let mut has_theirs = false;
    for ix in group_indices {
        let Some(block) = blocks.get(ix).copied() else {
            continue;
        };
        if !block.resolved {
            continue;
        }
        use gitcomet_core::conflict_output::ConflictOutputSource;
        has_base |= block.choice.contains(ConflictOutputSource::Base);
        has_ours |= block.choice.contains(ConflictOutputSource::Ours);
        has_theirs |= block.choice.contains(ConflictOutputSource::Theirs);
    }

    let mut selected = Vec::with_capacity(3);
    if has_base {
        selected.push(conflict_resolver::ConflictChoice::Base);
    }
    if has_ours {
        selected.push(conflict_resolver::ConflictChoice::Ours);
    }
    if has_theirs {
        selected.push(conflict_resolver::ConflictChoice::Theirs);
    }
    selected
}

pub(in crate::view::panes::main) fn conflict_group_indices_for_choice(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
    choice: conflict_resolver::ConflictChoice,
) -> Vec<usize> {
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    if group_indices.is_empty() {
        return Vec::new();
    }
    let blocks: Vec<&conflict_resolver::ConflictBlock> = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .collect();

    group_indices
        .into_iter()
        .filter(|&ix| {
            let Some(block) = blocks.get(ix).copied() else {
                return false;
            };
            if !block.resolved {
                return false;
            }
            match choice {
                conflict_resolver::ConflictChoice::Base => block
                    .choice
                    .contains(gitcomet_core::conflict_output::ConflictOutputSource::Base),
                conflict_resolver::ConflictChoice::Ours => block
                    .choice
                    .contains(gitcomet_core::conflict_output::ConflictOutputSource::Ours),
                conflict_resolver::ConflictChoice::Theirs => block
                    .choice
                    .contains(gitcomet_core::conflict_output::ConflictOutputSource::Theirs),
                conflict_resolver::ConflictChoice::Both => {
                    block.choice == conflict_resolver::ConflictChoice::Both
                }
                _ => block.choice == choice,
            }
        })
        .collect()
}

pub(in crate::view::panes::main) fn should_remove_conflict_block_on_reset(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
) -> bool {
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    group_indices.len() > 1
}

pub(in crate::view::panes::main) fn remove_conflict_block_at(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    conflict_ix: usize,
) -> bool {
    let mut next_segments = Vec::with_capacity(marker_segments.len());
    let mut seen_conflict_ix = 0usize;
    let mut removed = false;
    for seg in marker_segments.drain(..) {
        match seg {
            conflict_resolver::ConflictSegment::Block(block) => {
                if seen_conflict_ix == conflict_ix {
                    removed = true;
                } else {
                    next_segments.push(conflict_resolver::ConflictSegment::Block(block));
                }
                seen_conflict_ix = seen_conflict_ix.saturating_add(1);
            }
            conflict_resolver::ConflictSegment::Text(text) => {
                push_conflict_text_segment(&mut next_segments, text);
            }
        }
    }
    *marker_segments = next_segments;
    if removed && conflict_ix < conflict_region_indices.len() {
        conflict_region_indices.remove(conflict_ix);
    }
    removed
}

pub(in crate::view::panes::main) fn reset_conflict_block_selection(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    conflict_ix: usize,
) -> bool {
    if should_remove_conflict_block_on_reset(marker_segments, conflict_region_indices, conflict_ix)
    {
        return remove_conflict_block_at(marker_segments, conflict_region_indices, conflict_ix);
    }

    let mut seen_conflict_ix = 0usize;
    for seg in marker_segments.iter_mut() {
        let conflict_resolver::ConflictSegment::Block(block) = seg else {
            continue;
        };
        if seen_conflict_ix == conflict_ix {
            if !block.resolved {
                return false;
            }
            block.resolved = false;
            // A genuinely unpicked block has no implicit source. The output
            // projection renders its dedicated merge-conflict placeholder.
            block.choice = conflict_resolver::ConflictChoice::empty();
            return true;
        }
        seen_conflict_ix = seen_conflict_ix.saturating_add(1);
    }
    false
}

pub(in crate::view::panes::main) fn append_choice_after_conflict_block(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    conflict_ix: usize,
    choice: conflict_resolver::ConflictChoice,
) -> Option<usize> {
    let target_block = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .nth(conflict_ix)?
        .clone();
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    let &group_end_ix = group_indices.last()?;
    let target_region_ix = conflict_region_indices
        .get(conflict_ix)
        .copied()
        .unwrap_or(conflict_ix);
    if !target_block.resolved {
        return None;
    }
    if matches!(choice, conflict_resolver::ConflictChoice::Base) && target_block.base.is_none() {
        return None;
    }
    if conflict_group_selected_choices_for_ix(marker_segments, conflict_region_indices, conflict_ix)
        .contains(&choice)
    {
        return None;
    }

    let mut next_segments = Vec::with_capacity(marker_segments.len().saturating_add(1));
    let mut next_region_indices =
        Vec::with_capacity(conflict_region_indices.len().saturating_add(1));
    let mut seen_conflict_ix = 0usize;
    let mut next_conflict_ix = 0usize;
    let mut inserted_conflict_ix = None;

    let push_appended = |next_segments: &mut Vec<conflict_resolver::ConflictSegment>,
                         next_region_indices: &mut Vec<usize>,
                         next_conflict_ix: &mut usize,
                         inserted_conflict_ix: &mut Option<usize>| {
        if inserted_conflict_ix.is_some() {
            return;
        }
        let mut appended = target_block.clone();
        appended.choice = choice;
        appended.resolved = true;
        next_segments.push(conflict_resolver::ConflictSegment::Block(appended));
        next_region_indices.push(target_region_ix);
        *inserted_conflict_ix = Some(*next_conflict_ix);
        *next_conflict_ix = next_conflict_ix.saturating_add(1);
    };

    for seg in marker_segments.drain(..) {
        if seen_conflict_ix == group_end_ix.saturating_add(1) {
            push_appended(
                &mut next_segments,
                &mut next_region_indices,
                &mut next_conflict_ix,
                &mut inserted_conflict_ix,
            );
        }
        match seg {
            conflict_resolver::ConflictSegment::Block(block) => {
                let region_ix = conflict_region_indices
                    .get(seen_conflict_ix)
                    .copied()
                    .unwrap_or(seen_conflict_ix);
                next_segments.push(conflict_resolver::ConflictSegment::Block(block));
                next_region_indices.push(region_ix);
                next_conflict_ix = next_conflict_ix.saturating_add(1);
                seen_conflict_ix = seen_conflict_ix.saturating_add(1);
            }
            conflict_resolver::ConflictSegment::Text(text) => {
                push_conflict_text_segment(&mut next_segments, text);
            }
        }
    }
    push_appended(
        &mut next_segments,
        &mut next_region_indices,
        &mut next_conflict_ix,
        &mut inserted_conflict_ix,
    );

    *marker_segments = next_segments;
    *conflict_region_indices = next_region_indices;
    inserted_conflict_ix
}
