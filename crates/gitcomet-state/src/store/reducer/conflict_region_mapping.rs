use crate::msg::ConflictRegionOrigin;
use gitcomet_core::conflict_session::{
    ConflictRegion, ConflictRegionResolution, ConflictRegionSourceRanges, ConflictSession,
    reconstruct_conflict_marker_sides,
};
use gitcomet_core::merge::{MergeSource, OrderedSelection};

pub(super) enum RegionCorrespondence {
    Identity(usize),
    SourceOverlap(Vec<usize>),
}

impl RegionCorrespondence {
    pub(super) fn origins(&self) -> &[usize] {
        match self {
            Self::Identity(index) => std::slice::from_ref(index),
            Self::SourceOverlap(indices) => indices,
        }
    }
}

pub(super) fn correspondences(
    origin: &ConflictRegionOrigin,
    next: &ConflictSession,
) -> Vec<Option<RegionCorrespondence>> {
    let matches = |previous_index: usize, current: &ConflictRegion| {
        let previous = &origin.regions[previous_index];
        (previous.base == current.base || (origin.is_provisional() && previous.base.is_none()))
            && previous.ours == current.ours
            && previous.theirs == current.theirs
    };
    if origin.regions.len() == next.regions.len()
        && next
            .regions
            .iter()
            .enumerate()
            .all(|(index, region)| matches(index, region))
    {
        return (0..next.regions.len())
            .map(|index| Some(RegionCorrespondence::Identity(index)))
            .collect();
    }
    let mut cursor = 0;
    let mut result: Vec<_> = next
        .regions
        .iter()
        .map(|current| {
            let current_is_unique = next
                .regions
                .iter()
                .filter(|candidate| {
                    candidate.base == current.base
                        && candidate.ours == current.ours
                        && candidate.theirs == current.theirs
                })
                .take(2)
                .count()
                == 1;
            if !current_is_unique {
                return None;
            }
            let mut matching_origins =
                (0..origin.regions.len()).filter(|index| matches(*index, current));
            let index = matching_origins.next()?;
            if matching_origins.next().is_some() || index < cursor {
                return None;
            }
            cursor = index + 1;
            Some(RegionCorrespondence::Identity(index))
        })
        .collect();
    if origin.source_ranges.len() != origin.regions.len()
        || next.region_source_ranges.len() != next.regions.len()
    {
        return result;
    }
    let Some(marker_projection) = origin.marker_projection() else {
        return result;
    };
    let (projected_ours, projected_theirs) = reconstruct_conflict_marker_sides(marker_projection);
    if next.ours.as_text() != Some(projected_ours.as_str())
        || next.theirs.as_text() != Some(projected_theirs.as_str())
    {
        return result;
    }
    for (mapped, current) in result.iter_mut().zip(&next.region_source_ranges) {
        if mapped.is_some() {
            continue;
        }
        let origins: Vec<_> = origin
            .source_ranges
            .iter()
            .enumerate()
            .filter_map(|(index, previous)| ranges_overlap(previous, current).then_some(index))
            .collect();
        if !origins.is_empty() {
            *mapped = Some(RegionCorrespondence::SourceOverlap(origins));
        }
    }
    result
}

fn ranges_overlap(
    previous: &ConflictRegionSourceRanges,
    current: &ConflictRegionSourceRanges,
) -> bool {
    let overlaps = |left: &std::ops::Range<usize>, right: &std::ops::Range<usize>| {
        if left.is_empty() || right.is_empty() {
            return left.is_empty() && right.is_empty() && left.start == right.start;
        }
        left.start < right.end && right.start < left.end
    };
    overlaps(&previous.ours, &current.ours) || overlaps(&previous.theirs, &current.theirs)
}

pub(super) fn map_source(
    origin: &ConflictRegionOrigin,
    origin_index: usize,
    target_has_base: bool,
    source: MergeSource,
) -> MergeSource {
    if origin.is_provisional() && origin.regions[origin_index].base.is_none() && target_has_base {
        match source {
            MergeSource::A => MergeSource::B,
            MergeSource::B | MergeSource::C => MergeSource::C,
        }
    } else {
        source
    }
}

pub(super) fn map_resolution(
    origin: &ConflictRegionOrigin,
    origin_index: usize,
    target_has_base: bool,
    resolution: &ConflictRegionResolution,
) -> ConflictRegionResolution {
    match resolution {
        ConflictRegionResolution::Sources(selection) => {
            ConflictRegionResolution::Sources(OrderedSelection::from_sources(
                selection
                    .iter()
                    .map(|source| map_source(origin, origin_index, target_has_base, source)),
            ))
        }
        other => other.clone(),
    }
}

pub(super) fn source_backed_resolution(resolution: &ConflictRegionResolution) -> bool {
    matches!(
        resolution,
        ConflictRegionResolution::PickBase
            | ConflictRegionResolution::PickOurs
            | ConflictRegionResolution::PickTheirs
            | ConflictRegionResolution::PickBoth
            | ConflictRegionResolution::Sources(_)
    )
}
