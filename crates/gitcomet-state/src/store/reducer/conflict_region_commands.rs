use super::conflict_region_mapping::{self as mapping, RegionCorrespondence};
use crate::model::{AppState, RepoId};
use crate::msg::{ConflictRegionOrigin, ConflictRegionResolutionUpdate, Effect, RepoPath};
use gitcomet_core::conflict_session::{ConflictRegionResolution, ConflictSession};
use gitcomet_core::merge::{MergeSource, OrderedSelection};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
enum RegionEdit {
    Toggle(MergeSource),
    Replace(OrderedSelection),
    Resolve(ConflictRegionResolution),
}

pub(super) fn toggle_region_source(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    origin: Arc<ConflictRegionOrigin>,
    region_index: usize,
    source: MergeSource,
) -> Vec<Effect> {
    apply_edits(
        state,
        repo_id,
        path,
        &origin,
        [(region_index, RegionEdit::Toggle(source))]
            .into_iter()
            .collect(),
    )
}

pub(super) fn replace_region_selection(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    origin: Arc<ConflictRegionOrigin>,
    region_index: usize,
    selection: OrderedSelection,
) -> Vec<Effect> {
    apply_edits(
        state,
        repo_id,
        path,
        &origin,
        [(region_index, RegionEdit::Replace(selection))]
            .into_iter()
            .collect(),
    )
}

pub(super) fn sync_region_resolutions(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    origin: Arc<ConflictRegionOrigin>,
    updates: Vec<ConflictRegionResolutionUpdate>,
) -> Vec<Effect> {
    apply_edits(
        state,
        repo_id,
        path,
        &origin,
        updates
            .into_iter()
            .map(|update| (update.region_index, RegionEdit::Resolve(update.resolution)))
            .collect(),
    )
}

fn apply_edits(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    origin: &ConflictRegionOrigin,
    edits: FxHashMap<usize, RegionEdit>,
) -> Vec<Effect> {
    if edits.is_empty() {
        return Vec::new();
    }
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    if repo.conflict_state.conflict_file_path.as_deref() != Some(path.as_path()) {
        return Vec::new();
    }
    let conflict = &mut repo.conflict_state;
    let Some(session) = conflict
        .conflict_session
        .as_mut()
        .or(conflict.session_pending_restore.as_mut())
        .filter(|session| session.path == path.as_path())
    else {
        return Vec::new();
    };
    let Ok(mapped) = map_edits(origin, session, &edits) else {
        repo.feedback_mut().last_error = Some(
            "The conflict changed while this choice was being made. Review the refreshed conflict and choose again."
                .into(),
        );
        repo.bump_conflict_rev();
        return Vec::new();
    };
    let mut changed = false;
    for (index, edit) in mapped {
        changed |= match edit {
            RegionEdit::Toggle(source) => session.toggle_region_source(index, source),
            RegionEdit::Replace(selection) => session.replace_region_selection(index, selection),
            RegionEdit::Resolve(resolution) => {
                let region = &mut session.regions[index];
                if region.resolution == resolution {
                    false
                } else {
                    region.resolution = resolution;
                    true
                }
            }
        };
    }
    if changed {
        session.sync_merge_plan_from_regions();
        repo.bump_conflict_rev();
    }
    Vec::new()
}

#[derive(Debug)]
enum RegionMappingError {
    Changed,
}

fn map_edits(
    origin: &ConflictRegionOrigin,
    session: &ConflictSession,
    edits: &FxHashMap<usize, RegionEdit>,
) -> Result<Vec<(usize, RegionEdit)>, RegionMappingError> {
    let mut matched = FxHashSet::default();
    let mut mapped = Vec::new();
    for (target_index, correspondence) in mapping::correspondences(origin, session)
        .into_iter()
        .enumerate()
    {
        let Some(correspondence) = correspondence else {
            continue;
        };
        let origins = correspondence.origins();
        if !origins.iter().any(|index| edits.contains_key(index)) {
            continue;
        }
        let mut next = None;
        for &origin_index in origins {
            let edit = edits
                .get(&origin_index)
                .ok_or(RegionMappingError::Changed)?;
            let target_has_base = !session.base.is_absent();
            let mapped_edit = match edit {
                RegionEdit::Toggle(source) => RegionEdit::Toggle(mapping::map_source(
                    origin,
                    origin_index,
                    target_has_base,
                    *source,
                )),
                RegionEdit::Replace(selection) => {
                    RegionEdit::Replace(OrderedSelection::from_sources(selection.iter().map(
                        |source| mapping::map_source(origin, origin_index, target_has_base, source),
                    )))
                }
                RegionEdit::Resolve(resolution) => {
                    if matches!(correspondence, RegionCorrespondence::SourceOverlap(_))
                        && !mapping::source_backed_resolution(resolution)
                        && !matches!(resolution, ConflictRegionResolution::Unresolved)
                    {
                        return Err(RegionMappingError::Changed);
                    }
                    RegionEdit::Resolve(mapping::map_resolution(
                        origin,
                        origin_index,
                        target_has_base,
                        resolution,
                    ))
                }
            };
            if next
                .as_ref()
                .is_some_and(|previous| previous != &mapped_edit)
                || !edit_is_available(session, target_index, &mapped_edit)
            {
                return Err(RegionMappingError::Changed);
            }
            next = Some(mapped_edit);
            matched.insert(origin_index);
        }
        if let Some(edit) = next {
            mapped.push((target_index, edit));
        }
    }
    if matched.len() != edits.len() {
        return Err(RegionMappingError::Changed);
    }
    Ok(mapped)
}

fn edit_is_available(session: &ConflictSession, target_index: usize, edit: &RegionEdit) -> bool {
    let source_available = |source| {
        session
            .merge_plan
            .as_ref()
            .is_none_or(|plan| plan.source_text(source).is_some())
    };
    match edit {
        RegionEdit::Toggle(source) => source_available(*source),
        RegionEdit::Replace(selection) => selection.iter().all(source_available),
        RegionEdit::Resolve(ConflictRegionResolution::Sources(selection)) => {
            selection.iter().all(source_available)
        }
        RegionEdit::Resolve(ConflictRegionResolution::PickBase) => {
            session.regions[target_index].base.is_some()
        }
        RegionEdit::Resolve(
            ConflictRegionResolution::Unresolved
            | ConflictRegionResolution::PickOurs
            | ConflictRegionResolution::PickTheirs
            | ConflictRegionResolution::PickBoth
            | ConflictRegionResolution::ManualEdit(_)
            | ConflictRegionResolution::AutoResolved { .. },
        ) => true,
    }
}
