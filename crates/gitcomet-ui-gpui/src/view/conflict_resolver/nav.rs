use super::*;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictNavDirection {
    Prev,
    Next,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum ConflictNavTargetId {
    PlanBlock(gitcomet_core::merge::MergeBlockId),
    Region(usize),
    DisplayBlock(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct ConflictNavTarget {
    pub(in crate::view) id: ConflictNavTargetId,
    pub(in crate::view) order: usize,
    pub(in crate::view) aligned_rows: Option<Range<usize>>,
    pub(in crate::view) region_index: Option<usize>,
    pub(in crate::view) display_conflict_index: Option<usize>,
    pub(in crate::view) is_delta: bool,
    pub(in crate::view) original_conflict: bool,
    pub(in crate::view) unresolved: bool,
}

impl ConflictNavTarget {
    pub(in crate::view) fn anchor(&self) -> ConflictNavAnchor {
        ConflictNavAnchor {
            id: self.id,
            order_hint: self.order,
            aligned_row_hint: self.aligned_rows.as_ref().map(|range| range.start),
        }
    }

    fn contains_aligned_row(&self, row: usize) -> bool {
        self.aligned_rows
            .as_ref()
            .is_some_and(|range| range.contains(&row) || (range.is_empty() && range.start == row))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct ConflictNavAnchor {
    pub(in crate::view) id: ConflictNavTargetId,
    pub(in crate::view) order_hint: usize,
    pub(in crate::view) aligned_row_hint: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum ConflictNavTargetFilter {
    Delta,
    Conflict,
    Unresolved,
}

impl ConflictNavTargetFilter {
    pub(in crate::view) fn matches(self, target: &ConflictNavTarget) -> bool {
        match self {
            Self::Delta => target.is_delta,
            Self::Conflict => conflict_nav_target_is_conflict(target),
            Self::Unresolved => target.unresolved,
        }
    }
}

/// Whether "next/previous conflict" should stop here.
///
/// The plan's `original_conflict` flag alone is too narrow. It answers "did the
/// merge algorithm call this a conflict", and the resolved output can draw a
/// `<Merge Conflict>` row for a block the plan did not — a region the worktree
/// carries markers for, a block left open after an unresolve, a block whose plan
/// identity was lost to a split or an edit. Skipping those made conflict
/// navigation step over rows that are visibly awaiting a decision, while the
/// unresolved-only navigation reached them.
///
/// So a target counts as a conflict when *any* of the three agree: the plan
/// called it one, the output renders it as a decision block, or it is still
/// unresolved. The last clause is what makes this a superset of
/// [`ConflictNavTargetFilter::Unresolved`] — conflict navigation can never skip
/// a row that "next unresolved" would land on.
pub(in crate::view) fn conflict_nav_target_is_conflict(target: &ConflictNavTarget) -> bool {
    target.original_conflict || target.display_conflict_index.is_some() || target.unresolved
}

pub(in crate::view) fn fresh_conflict_nav_target_index(
    targets: &[ConflictNavTarget],
) -> Option<usize> {
    targets
        .iter()
        .position(|target| target.unresolved)
        .or_else(|| targets.iter().position(|target| target.original_conflict))
        .or_else(|| targets.iter().position(|target| target.is_delta))
}

pub(in crate::view) fn reconcile_conflict_nav_target_index(
    anchor: Option<ConflictNavAnchor>,
    previous_targets: &[ConflictNavTarget],
    targets: &[ConflictNavTarget],
) -> Option<usize> {
    let Some(anchor) = anchor else {
        return fresh_conflict_nav_target_index(targets);
    };
    if targets.is_empty() {
        return None;
    }

    // Stable plan identities and unchanged legacy identities win.
    if let Some(index) = targets.iter().position(|target| target.id == anchor.id) {
        return Some(index);
    }

    // Bridge plan-backed and region-backed projections in either direction.
    let previous_region = match anchor.id {
        ConflictNavTargetId::Region(region_index) => Some(region_index),
        ConflictNavTargetId::PlanBlock(_) | ConflictNavTargetId::DisplayBlock(_) => {
            previous_targets
                .iter()
                .find(|target| target.id == anchor.id)
                .and_then(|target| target.region_index)
        }
    };
    if let Some(region_index) = previous_region
        && let Some(index) = targets
            .iter()
            .position(|target| target.region_index == Some(region_index))
    {
        return Some(index);
    }

    // Structural edits can replace target identities while retaining row
    // geometry. Prefer the target that still owns the remembered row.
    if let Some(row) = anchor.aligned_row_hint
        && let Some(index) = targets
            .iter()
            .position(|target| target.contains_aligned_row(row))
    {
        return Some(index);
    }

    // Finally retain the nearest ordered position. Orders are monotonic but
    // need not be dense, so compare explicitly after clamping the hint.
    let max_order = targets.iter().map(|target| target.order).max().unwrap_or(0);
    let clamped_order = anchor.order_hint.min(max_order);
    targets
        .iter()
        .enumerate()
        .min_by_key(|(_, target)| (target.order.abs_diff(clamped_order), target.order))
        .map(|(index, _)| index)
        .or_else(|| fresh_conflict_nav_target_index(targets))
}

pub(in crate::view) fn conflict_nav_anchor_order(
    targets: &[ConflictNavTarget],
    anchor: ConflictNavAnchor,
) -> usize {
    targets
        .iter()
        .find(|target| target.id == anchor.id)
        .map(|target| target.order)
        .unwrap_or(anchor.order_hint)
}

pub(in crate::view) fn previous_conflict_nav_target_index(
    targets: &[ConflictNavTarget],
    anchor: Option<ConflictNavAnchor>,
    filter: ConflictNavTargetFilter,
) -> Option<usize> {
    let anchor = anchor?;
    let current_order = conflict_nav_anchor_order(targets, anchor);
    targets
        .iter()
        .enumerate()
        .rev()
        .find(|(_, target)| target.order < current_order && filter.matches(target))
        .map(|(index, _)| index)
}

pub(in crate::view) fn next_conflict_nav_target_index(
    targets: &[ConflictNavTarget],
    anchor: Option<ConflictNavAnchor>,
    filter: ConflictNavTargetFilter,
) -> Option<usize> {
    let anchor = anchor?;
    let current_order = conflict_nav_anchor_order(targets, anchor);
    targets
        .iter()
        .enumerate()
        .find(|(_, target)| target.order > current_order && filter.matches(target))
        .map(|(index, _)| index)
}

/// The anchored target itself, when it is the only one the filter matches.
///
/// With one conflict left to decide — the anchored one — nothing lies strictly
/// past it in either direction, so both `next` and `previous` went dead and both
/// toolbar arrows greyed out while that conflict could be far off screen. Naming
/// it as the target is what lets the keys and the buttons bring it back.
///
/// Deliberately narrower than wrapping: with several matches, standing on the
/// last one still reports "nothing further this way", which is what tells the
/// user they have reached the end.
pub(in crate::view) fn sole_matching_anchor_index(
    targets: &[ConflictNavTarget],
    anchor: ConflictNavAnchor,
    filter: ConflictNavTargetFilter,
) -> Option<usize> {
    let index = targets.iter().position(|target| target.id == anchor.id)?;
    let mut matches = targets
        .iter()
        .enumerate()
        .filter(|(_, target)| filter.matches(target));
    (matches.next().map(|(index, _)| index) == Some(index) && matches.next().is_none())
        .then_some(index)
}

pub(in crate::view) fn previous_conflict_nav_target_index_or_sole_anchor(
    targets: &[ConflictNavTarget],
    anchor: Option<ConflictNavAnchor>,
    filter: ConflictNavTargetFilter,
) -> Option<usize> {
    let anchor = anchor?;
    previous_conflict_nav_target_index(targets, Some(anchor), filter)
        .or_else(|| sole_matching_anchor_index(targets, anchor, filter))
}

pub(in crate::view) fn next_conflict_nav_target_index_or_sole_anchor(
    targets: &[ConflictNavTarget],
    anchor: Option<ConflictNavAnchor>,
    filter: ConflictNavTargetFilter,
) -> Option<usize> {
    let anchor = anchor?;
    next_conflict_nav_target_index(targets, Some(anchor), filter)
        .or_else(|| sole_matching_anchor_index(targets, anchor, filter))
}

pub(in crate::view) fn conflict_pick_choices(
    view_mode: ConflictResolverViewMode,
) -> &'static [ConflictChoice] {
    match view_mode {
        ConflictResolverViewMode::ThreeWay => &[
            ConflictChoice::Base,
            ConflictChoice::Ours,
            ConflictChoice::Theirs,
            ConflictChoice::Both,
        ],
        ConflictResolverViewMode::TwoWayDiff => &[
            ConflictChoice::Ours,
            ConflictChoice::Theirs,
            ConflictChoice::Both,
        ],
    }
}

/// Resolve conflict quick-pick keyboard shortcuts to a concrete choice.
pub fn conflict_quick_pick_choice_for_key(
    key: &str,
    view_mode: ConflictResolverViewMode,
) -> Option<ConflictChoice> {
    let index = match key {
        "a" => 0,
        "b" => 1,
        "c" => 2,
        "d" => 3,
        _ => return None,
    };
    conflict_pick_choices(view_mode).get(index).copied()
}

/// Resolve kdiff3-compatible `Ctrl+1/2/3` pick aliases (section 30 keyboard model).
///
/// Unlike the single-letter picks these also work while the output editor is
/// focused, since they cannot collide with text input.
pub fn conflict_ctrl_pick_choice_for_key(
    key: &str,
    view_mode: ConflictResolverViewMode,
) -> Option<ConflictChoice> {
    let index = match key {
        "1" => 0,
        "2" => 1,
        "3" => 2,
        _ => return None,
    };
    conflict_pick_choices(view_mode).get(index).copied()
}

/// Resolve conflict navigation shortcuts (`F2`, `F3`, `F7`) to a direction.
#[cfg(test)]
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
pub(in crate::view) fn conflict_nav_region_aligned_ranges(
    session: &gitcomet_core::conflict_session::ConflictSession,
    fallback_ranges: &[Range<usize>],
) -> Vec<Option<Range<usize>>> {
    if let Some(plan) = session.merge_plan.as_ref() {
        return (0..session.regions.len())
            .map(|region_index| {
                let block_index = *session.region_plan_blocks.get(region_index)?;
                plan.blocks.get(block_index).map(|block| block.rows.clone())
            })
            .collect();
    }

    (0..session.regions.len())
        .map(|region_index| fallback_ranges.get(region_index).cloned())
        .collect()
}

pub(in crate::view) fn build_conflict_nav_targets(
    session: Option<&gitcomet_core::conflict_session::ConflictSession>,
    region_aligned_ranges: &[Option<Range<usize>>],
    display_region_indices: &[usize],
    display_aligned_ranges: &[Option<Range<usize>>],
    segments: &[ConflictSegment],
) -> Vec<ConflictNavTarget> {
    let display_resolved: Vec<bool> = segments
        .iter()
        .filter_map(|segment| match segment {
            ConflictSegment::Block(block) => Some(block.resolved),
            ConflictSegment::Text(_) => None,
        })
        .collect();
    let display_for_region = |region_index: usize| {
        display_region_indices
            .iter()
            .position(|candidate| *candidate == region_index)
    };

    if let Some(session) = session {
        if let Some(plan) = session.merge_plan.as_ref() {
            let mut targets = Vec::new();
            for (block_index, block) in plan.blocks.iter().enumerate() {
                if !block.is_delta && !block.original_conflict {
                    continue;
                }
                let region_index = session
                    .region_plan_blocks
                    .iter()
                    .position(|candidate| *candidate == block_index);
                // `display_conflict_index` addresses a *rendered marker block*
                // — it becomes `active_conflict`, which the rest of the UI uses
                // as `conflict_ix`. Only the region mapping can produce it: a
                // plan block's position among the plan's blocks is a different
                // index space, and blocks that render no marker (automatically
                // selected deltas) are absent from the displayed space entirely.
                let display_conflict_index = region_index.and_then(display_for_region);
                // Prefer the displayed block's own verdict where the block is
                // rendered, since an in-progress edit can resolve a marker
                // ahead of the plan; otherwise the plan block is the authority,
                // which is what makes a plan-only delta navigable.
                let unresolved = display_conflict_index
                    .and_then(|index| display_resolved.get(index))
                    .map_or_else(|| !block.is_resolved(), |resolved| !resolved);
                targets.push(ConflictNavTarget {
                    id: ConflictNavTargetId::PlanBlock(block.id),
                    order: targets.len(),
                    aligned_rows: Some(block.rows.clone()),
                    region_index,
                    display_conflict_index,
                    is_delta: block.is_delta,
                    original_conflict: block.original_conflict,
                    unresolved,
                });
            }
            return targets;
        }

        return session
            .regions
            .iter()
            .enumerate()
            .map(|(region_index, region)| {
                let display_conflict_index = display_for_region(region_index);
                let unresolved = display_conflict_index
                    .and_then(|index| display_resolved.get(index))
                    .map_or_else(|| !region.resolution.is_resolved(), |resolved| !resolved);
                ConflictNavTarget {
                    id: ConflictNavTargetId::Region(region_index),
                    order: region_index,
                    aligned_rows: region_aligned_ranges.get(region_index).cloned().flatten(),
                    region_index: Some(region_index),
                    display_conflict_index,
                    is_delta: true,
                    original_conflict: true,
                    unresolved,
                }
            })
            .collect();
    }

    display_resolved
        .into_iter()
        .enumerate()
        .map(|(display_conflict_index, resolved)| ConflictNavTarget {
            id: ConflictNavTargetId::DisplayBlock(display_conflict_index),
            order: display_conflict_index,
            aligned_rows: display_aligned_ranges
                .get(display_conflict_index)
                .cloned()
                .flatten(),
            region_index: display_region_indices.get(display_conflict_index).copied(),
            display_conflict_index: Some(display_conflict_index),
            is_delta: true,
            original_conflict: true,
            unresolved: !resolved,
        })
        .collect()
}
