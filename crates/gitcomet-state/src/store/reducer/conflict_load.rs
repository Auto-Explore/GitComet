use super::prepared::ReducerInputs;
use super::util::{EffectAccumulator, push_diagnostic};
use crate::model::{AppState, ConflictFileLoadMode, DiagnosticKind, Loadable, RepoId, RepoState};
use crate::msg::{ConflictAutosolveMode, Effect};
use gitcomet_core::conflict_session::{
    ConflictPayload, ConflictRegionResolution, ConflictResolverStrategy, ConflictSession,
};
use gitcomet_core::domain::FileStatusKind;
use gitcomet_core::error::Error;
use std::path::{Path, PathBuf};

const CONFLICT_RELOAD_EFFECT_COUNT: usize = 1;

pub(super) fn conflict_file_loaded(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    request: crate::model::ConflictLoadRequestId,
    path: PathBuf,
    result: std::result::Result<Option<crate::model::ConflictFile>, Error>,
    conflict_session: Option<ConflictSession>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.conflict_state.accepts_load(request, &path)
    {
        // A same-path reload stashes the previous session (see
        // `RepoState::begin_conflict_load`); it counts as the existing
        // session for resolution restore. Ordinary reloads suppress on-open
        // autosolve, while the provisional CurrentOnly -> Full upgrade does not.
        let stashed_session = repo_state.conflict_state.session_pending_restore.take();
        let existing_session = repo_state
            .conflict_state
            .conflict_session
            .as_ref()
            .or(stashed_session.as_ref());
        // CurrentOnly sessions are provisional: they preserve marker-backed
        // picks during the fast first paint, but the subsequent Full load is
        // still the first stage-backed open and must run on-open autosolve.
        let fresh_open =
            existing_session.is_none_or(conflict_session_uses_provisional_stage_inputs);
        let session = conflict_session.or_else(|| match &result {
            Ok(Some(file)) => build_conflict_session(repo_state, file),
            _ => None,
        });
        let session = session.map(|mut session| {
            if let Some(existing_session) = existing_session {
                restore_conflict_session_resolutions(existing_session, &mut session);
            }
            session
        });
        let session_is_provisional = session
            .as_ref()
            .is_some_and(conflict_session_uses_provisional_stage_inputs);
        let value = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(inputs, repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        let keep_stashed_session = session.is_none() && stashed_session.is_some();
        repo_state.set_conflict_file(value);
        repo_state.set_conflict_session(session);
        if keep_stashed_session {
            repo_state.conflict_state.session_pending_restore = stashed_session;
        }
        if fresh_open
            && !session_is_provisional
            && repo_state.conflict_state.conflict_session.is_some()
        {
            auto_resolve_session_on_open(inputs, repo_state, &path);
        }
    }
    Vec::new()
}

/// Auto-solve policy: only the always-safe rules
/// (identical sides, one-side-changed) and the subchunk split apply
/// automatically when a conflicted file first opens in the resolver.
///
/// Whitespace-only conflicts and regex normalization are deliberately left
/// alone, matching KDiff3: its `WhiteSpace2FileMergeDefault` /
/// `WhiteSpace3FileMergeDefault` both default to "Manual Choice"
/// (`e_SrcSelector::None`), so `MergeResultWindow::merge` skips
/// `updateDefaults` for whitespace blocks, and `RunRegExpAutoMergeOnMergeStart`
/// defaults to false. Both still run behind the explicit Auto-solve action, as
/// does the Low tier (history merge).
///
/// Reloads of an already stage-backed file keep user resolutions via
/// [`restore_conflict_session_resolutions`] and are never re-autosolved, so a
/// region the user deliberately un-resolved stays unresolved. A provisional
/// CurrentOnly session waits to run this policy until its Full upgrade.
fn auto_resolve_session_on_open(inputs: &ReducerInputs, repo_state: &mut RepoState, path: &Path) {
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return;
    };
    if session.strategy != ConflictResolverStrategy::FullTextResolver {
        return;
    }
    let total_before = session.total_regions();
    let unresolved_before = session.unsolved_count();
    if unresolved_before == 0 {
        return;
    }

    let stats = super::conflict_interactions::apply_autosolve_to_session(
        session,
        ConflictAutosolveMode::Safe,
        false,
    );
    if stats.total_resolved() == 0 {
        return;
    }
    session.sync_merge_plan_from_regions();
    let unresolved_after = session.unsolved_count();
    let total_after = session.total_regions();

    super::util::push_action_log(
        inputs,
        repo_state,
        true,
        format!("telemetry.conflict_autosolve.on_open {}", path.display()),
        super::util::conflict_autosolve_telemetry_summary(
            ConflictAutosolveMode::Safe,
            Some(path),
            total_before,
            total_after,
            unresolved_before,
            unresolved_after,
            stats,
        ),
        None,
        false,
    );
}

fn restore_conflict_session_resolutions(existing: &ConflictSession, next: &mut ConflictSession) {
    if existing.path != next.path {
        return;
    }

    // Split/join rewrites the in-memory marker projection without touching the
    // worktree until Save. If Git stages are unchanged, keep that complete
    // structural projection across same-path watcher or explicit reloads.
    if existing.has_pending_structural_edits
        && existing.conflict_kind == next.conflict_kind
        && existing.strategy == next.strategy
        && existing.base == next.base
        && existing.ours == next.ours
        && existing.theirs == next.theirs
    {
        next.marker_projection = existing.marker_projection.clone();
        next.regions = existing.regions.clone();
        next.region_source_ranges = existing.region_source_ranges.clone();
        next.merge_plan = existing.merge_plan.clone();
        next.merge_plan_fallback = existing.merge_plan_fallback;
        next.region_plan_blocks = existing.region_plan_blocks.clone();
        next.has_pending_structural_edits = true;
        return;
    }

    let origin = crate::msg::ConflictRegionOrigin::from_session(existing);
    let correspondences = super::conflict_region_mapping::correspondences(&origin, next);
    let same_ordered_regions = existing.regions.len() == next.regions.len()
        && correspondences.iter().enumerate().all(|(index, correspondence)| {
            matches!(correspondence, Some(super::conflict_region_mapping::RegionCorrespondence::Identity(previous)) if *previous == index)
        });
    if !same_ordered_regions {
        next.restore_plan_decisions_from(existing);
    } else if let (Some(previous), Some(current)) =
        (existing.merge_plan.as_ref(), next.merge_plan.as_mut())
        && crate::msg::ConflictPlanOrigin::from_plan(previous).matches_plan(current)
    {
        current.restore_decisions_from(previous);
    }
    let next_has_base = !next.base.is_absent();
    for (current, correspondence) in next.regions.iter_mut().zip(correspondences) {
        use super::conflict_region_mapping::{
            RegionCorrespondence, map_resolution, source_backed_resolution,
        };
        match correspondence {
            Some(RegionCorrespondence::Identity(index)) => {
                current.resolution = map_resolution(
                    &origin,
                    index,
                    next_has_base,
                    &existing.regions[index].resolution,
                );
            }
            Some(RegionCorrespondence::SourceOverlap(indices))
                if matches!(current.resolution, ConflictRegionResolution::Unresolved) =>
            {
                let Some((&first, remaining)) = indices.split_first() else {
                    continue;
                };
                if !source_backed_resolution(&existing.regions[first].resolution) {
                    continue;
                }
                let resolution = map_resolution(
                    &origin,
                    first,
                    next_has_base,
                    &existing.regions[first].resolution,
                );
                if remaining.iter().all(|&index| {
                    source_backed_resolution(&existing.regions[index].resolution)
                        && map_resolution(
                            &origin,
                            index,
                            next_has_base,
                            &existing.regions[index].resolution,
                        ) == resolution
                }) {
                    current.resolution = resolution;
                }
            }
            Some(RegionCorrespondence::SourceOverlap(_)) | None => {}
        }
    }
    next.sync_merge_plan_from_regions();
}

fn conflict_session_uses_provisional_stage_inputs(session: &ConflictSession) -> bool {
    session.strategy == gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver
        && session.base.is_absent()
        && session.ours.is_absent()
        && session.theirs.is_absent()
}

/// Build a `ConflictSession` from a loaded `ConflictFile` and the current repo status.
///
/// Looks up the `FileConflictKind` from the status entries. Full loads derive
/// text boundaries from immutable Git stages; CurrentOnly loads use a
/// provisional marker-backed session until those stages arrive.
fn build_conflict_session(
    repo_state: &crate::model::RepoState,
    file: &crate::model::ConflictFile,
) -> Option<ConflictSession> {
    // Look up the conflict kind from the repo's status entries.
    let conflict_kind = repo_state
        .worktree_status_entries()?
        .iter()
        .find(|e| e.path == file.path && e.kind == FileStatusKind::Conflicted)
        .and_then(|e| e.conflict)?;

    let base = ConflictPayload::from_stage_parts(file.base_bytes.clone(), file.base.clone());
    let ours = ConflictPayload::from_stage_parts(file.ours_bytes.clone(), file.ours.clone());
    let theirs = ConflictPayload::from_stage_parts(file.theirs_bytes.clone(), file.theirs.clone());

    let is_binary = base.is_binary() || ours.is_binary() || theirs.is_binary();
    let strategy = gitcomet_core::conflict_session::ConflictResolverStrategy::for_conflict(
        conflict_kind,
        is_binary,
    );

    if strategy == gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver
        && base.is_absent()
        && ours.is_absent()
        && theirs.is_absent()
    {
        // CurrentOnly intentionally omits the immutable stages. Build a
        // provisional session from the worktree markers so first-paint picks
        // have real regions; the Full upgrade replaces its inputs and retains
        // matching choices.
        file.current.as_ref().map(|current| {
            ConflictSession::from_merged_shared_text(
                file.path.to_path_buf(),
                conflict_kind,
                base,
                ours,
                theirs,
                current.clone(),
            )
        })
    } else if strategy
        == gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver
    {
        let current = file
            .current
            .as_ref()
            .map(|text| ConflictPayload::Text(text.clone()))
            .or_else(|| {
                file.current_bytes
                    .as_ref()
                    .map(|bytes| ConflictPayload::Binary(bytes.clone()))
            });
        Some(ConflictSession::from_stage_inputs_with_current(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
            current,
        ))
    } else if let Some(current) = file.current.as_ref() {
        Some(ConflictSession::from_merged_shared_text(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
            current.clone(),
        ))
    } else if let Some(current) = file.current_bytes.as_ref() {
        Some(ConflictSession::new_with_current(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
            ConflictPayload::Binary(current.clone()),
        ))
    } else {
        Some(ConflictSession::new(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
        ))
    }
}

pub(super) fn load_conflict_file(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    mode: ConflictFileLoadMode,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let same_path = repo_state.conflict_state.conflict_file_path.as_ref() == Some(&path);
    repo_state.set_conflict_file_path(Some(path.clone()));
    let request = repo_state.begin_conflict_load(mode, same_path);
    vec![Effect::LoadConflictFile {
        request,
        repo_id,
        path,
        mode,
    }]
}
pub(super) fn current_conflict_load_mode(repo_state: &RepoState) -> ConflictFileLoadMode {
    repo_state.conflict_state.conflict_file_load_mode
}

pub(super) fn start_current_conflict_target_reload(repo_state: &mut RepoState) -> Vec<Effect> {
    let mode = current_conflict_load_mode(repo_state);
    let mut effects = Vec::with_capacity(CONFLICT_RELOAD_EFFECT_COUNT);
    append_start_current_conflict_target_reload_with_mode(&mut effects, repo_state, mode);
    effects
}

pub(super) fn append_start_current_conflict_target_reload(
    effects: &mut impl EffectAccumulator,
    repo_state: &mut RepoState,
) {
    let mode = current_conflict_load_mode(repo_state);
    append_start_current_conflict_target_reload_with_mode(effects, repo_state, mode);
}

pub(super) fn start_conflict_target_reload(repo_state: &mut RepoState, path: &Path) -> Vec<Effect> {
    let mode = current_conflict_load_mode(repo_state);
    start_conflict_target_reload_with_mode(repo_state, path, mode)
}

pub(super) fn append_start_conflict_target_reload(
    effects: &mut impl EffectAccumulator,
    repo_state: &mut RepoState,
    path: &Path,
) {
    let mode = current_conflict_load_mode(repo_state);
    append_start_conflict_target_reload_with_mode(effects, repo_state, path, mode);
}

pub(super) fn start_conflict_target_reload_with_mode(
    repo_state: &mut RepoState,
    path: &Path,
    mode: ConflictFileLoadMode,
) -> Vec<Effect> {
    let mut effects = Vec::with_capacity(CONFLICT_RELOAD_EFFECT_COUNT);
    append_start_conflict_target_reload_with_mode(&mut effects, repo_state, path, mode);
    effects
}

fn append_start_current_conflict_target_reload_with_mode(
    effects: &mut impl EffectAccumulator,
    repo_state: &mut RepoState,
    mode: ConflictFileLoadMode,
) {
    debug_assert!(repo_state.conflict_state.conflict_file_path.is_some());
    let request = repo_state.begin_conflict_load(mode, true);
    effects.push_effect(Effect::LoadSelectedConflictFile {
        request,
        repo_id: repo_state.id,
        mode,
    });
}

fn append_start_conflict_target_reload_with_mode(
    effects: &mut impl EffectAccumulator,
    repo_state: &mut RepoState,
    path: &Path,
    mode: ConflictFileLoadMode,
) {
    if repo_state.conflict_state.conflict_file_path.as_deref() == Some(path) {
        append_start_current_conflict_target_reload_with_mode(effects, repo_state, mode);
        return;
    }

    repo_state.set_conflict_file_path(Some(path.to_path_buf()));
    let request = repo_state.begin_conflict_load(mode, false);
    effects.push_effect(Effect::LoadSelectedConflictFile {
        request,
        repo_id: repo_state.id,
        mode,
    });
}
