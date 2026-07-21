use crate::model::{AppState, RepoId};
use crate::msg::{
    ConflictAutosolveMode, ConflictAutosolveStats, ConflictBulkChoice, ConflictRegionChoice,
    ConflictRegionResolutionUpdate, Effect, RepoPath,
};
use gitcomet_core::conflict_session::{
    ConflictPayload, ConflictRegionEditOutcome, ConflictRegionResolution,
    ConflictRegionSplitBoundaries, ConflictResolverStrategy, HistoryAutosolveOptions,
    RegexAutosolveOptions, join_conflict_regions_text, split_conflict_region_text,
};
use std::collections::BTreeMap;
use std::path::Path;

pub(super) fn set_hide_resolved(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    hide_resolved: bool,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    repo_state.set_conflict_hide_resolved(hide_resolved);
    Vec::new()
}

pub(super) fn apply_bulk_choice(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    choice: ConflictBulkChoice,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path.as_path() {
        return Vec::new();
    }

    let applied = apply_bulk_choice_to_session(session, choice);
    if applied > 0 {
        repo_state.bump_conflict_rev();
    }
    Vec::new()
}

pub(super) fn set_region_choice(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    choice: ConflictRegionChoice,
) -> Vec<Effect> {
    set_region_choice_inline(state, repo_id, path, region_index, choice);
    Vec::new()
}

#[inline]
pub(super) fn set_region_choice_inline(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    choice: ConflictRegionChoice,
) {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return;
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return;
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return;
    };
    if session.path != path.as_path() {
        return;
    }

    let Some(region) = session.regions.get_mut(region_index) else {
        return;
    };
    let Some(next_resolution) = (match choice {
        ConflictRegionChoice::Base => region
            .base
            .as_ref()
            .map(|_| ConflictRegionResolution::PickBase),
        ConflictRegionChoice::Ours => Some(ConflictRegionResolution::PickOurs),
        ConflictRegionChoice::Theirs => Some(ConflictRegionResolution::PickTheirs),
        ConflictRegionChoice::Both => Some(ConflictRegionResolution::PickBoth),
    }) else {
        return;
    };

    if region.resolution != next_resolution {
        region.resolution = next_resolution;
        repo_state.bump_conflict_rev();
    }
}

#[derive(Clone, Copy)]
enum ConflictRegionEdit {
    Split(ConflictRegionSplitBoundaries),
    Join,
}

/// section 30 split: rewrite conflict block `region_index` into 2–3 blocks at the
/// given block-local boundaries; the split parts open Unresolved and every
/// other region keeps its resolution (indices shift past the split).
pub(super) fn split_region(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    boundaries: ConflictRegionSplitBoundaries,
) -> Vec<Effect> {
    edit_regions(
        state,
        repo_id,
        path,
        region_index,
        ConflictRegionEdit::Split(boundaries),
    )
}

/// section 30 join: merge conflict blocks `region_index` and `region_index + 1`
/// (context between them is absorbed into every side); the joined region
/// opens Unresolved and later regions shift down by one.
pub(super) fn join_regions(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    expected_conflict_rev: u64,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    if repo_state.conflict_state.conflict_rev != expected_conflict_rev {
        return Vec::new();
    }
    edit_regions(state, repo_id, path, region_index, ConflictRegionEdit::Join)
}

fn edit_regions(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    edit: ConflictRegionEdit,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path.as_path() {
        return Vec::new();
    }
    if session.strategy != ConflictResolverStrategy::FullTextResolver {
        return Vec::new();
    }
    let Some(ConflictPayload::Text(current)) = session.current.as_ref() else {
        return Vec::new();
    };

    let Some(ConflictRegionEditOutcome { new_text, parts }) = (match edit {
        ConflictRegionEdit::Split(boundaries) => {
            split_conflict_region_text(current, region_index, boundaries)
        }
        ConflictRegionEdit::Join => join_conflict_regions_text(current, region_index),
    }) else {
        return Vec::new();
    };

    // Explicit resolution carry-over: the edited region(s) open Unresolved,
    // everything else keeps its resolution at its shifted index. The reload
    // restore path is content-based and would drop nothing here either, but
    // doing it inline keeps the reducer deterministic and reload-free.
    let old: Vec<ConflictRegionResolution> = session
        .regions
        .iter()
        .map(|region| region.resolution.clone())
        .collect();
    let shared: std::sync::Arc<str> = std::sync::Arc::from(new_text.as_str());
    session.current = Some(ConflictPayload::Text(std::sync::Arc::clone(&shared)));
    session.parse_regions_from_shared_text(std::sync::Arc::clone(&shared));
    for (ix, region) in session.regions.iter_mut().enumerate() {
        region.resolution = if ix < region_index {
            old.get(ix)
                .cloned()
                .unwrap_or(ConflictRegionResolution::Unresolved)
        } else if ix < region_index + parts {
            ConflictRegionResolution::Unresolved
        } else {
            // Split consumed 1 old region for `parts` new ones; join consumed
            // 2 old regions for 1 new one.
            let consumed = match edit {
                ConflictRegionEdit::Split(_) => 1,
                ConflictRegionEdit::Join => 2,
            };
            old.get(ix - parts + consumed)
                .cloned()
                .unwrap_or(ConflictRegionResolution::Unresolved)
        };
    }

    // Keep the loaded conflict file's current text in step so the view's
    // source hash changes and it rebuilds from the new segmentation. Mutate
    // the file in place — rebuilding from the session would clobber side
    // texts under CurrentOnly load mode.
    let loaded_file_updated = if let crate::model::Loadable::Ready(Some(file)) =
        &repo_state.conflict_state.conflict_file
        && file.path.as_path() == path.as_path()
    {
        let mut file = file.clone();
        file.current = Some(std::sync::Arc::clone(&shared));
        file.current_bytes = None;
        repo_state.set_conflict_file(crate::model::Loadable::Ready(Some(file)));
        true
    } else {
        false
    };
    // `set_conflict_file` already publishes the session edit. If there was no
    // matching loaded file to update, publish the session mutation directly.
    if !loaded_file_updated {
        repo_state.bump_conflict_rev();
    }

    // Persist the rewritten marker text; resolved regions stay markers on
    // disk — Save remains the materialization point for resolutions.
    vec![Effect::SaveWorktreeFile {
        repo_id,
        path: path.to_path_buf(),
        contents: new_text,
        stage: false,
    }]
}

pub(super) fn sync_region_resolutions(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    updates: Vec<ConflictRegionResolutionUpdate>,
) -> Vec<Effect> {
    if updates.is_empty() {
        return Vec::new();
    }
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path.as_path() {
        return Vec::new();
    }

    let mut latest_by_region: BTreeMap<usize, ConflictRegionResolution> = BTreeMap::new();
    for update in updates {
        latest_by_region.insert(update.region_index, update.resolution);
    }

    let mut changed = 0usize;
    for (region_index, resolution) in latest_by_region {
        let Some(region) = session.regions.get_mut(region_index) else {
            continue;
        };
        if matches!(resolution, ConflictRegionResolution::PickBase) && region.base.is_none() {
            continue;
        }
        if region.resolution != resolution {
            region.resolution = resolution;
            changed += 1;
        }
    }

    if changed > 0 {
        repo_state.bump_conflict_rev();
    }
    Vec::new()
}

pub(super) fn apply_autosolve(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    mode: ConflictAutosolveMode,
    whitespace_normalize: bool,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path.as_path() {
        return Vec::new();
    }

    let stats = apply_autosolve_to_session(session, mode, whitespace_normalize);
    if stats.total_resolved() > 0 {
        repo_state.bump_conflict_rev();
    }
    Vec::new()
}

pub(super) fn reset_resolutions(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
) -> Vec<Effect> {
    reset_resolutions_inline(state, repo_id, path);
    Vec::new()
}

#[inline]
pub(super) fn reset_resolutions_inline(state: &mut AppState, repo_id: RepoId, path: RepoPath) {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return;
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return;
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return;
    };
    if session.path != path.as_path() {
        return;
    }

    let reset_count = reset_session_resolutions(session);
    if reset_count > 0 {
        repo_state.bump_conflict_rev();
    }
}

fn matches_current_conflict_path(repo_state: &crate::model::RepoState, path: &Path) -> bool {
    repo_state.conflict_state.conflict_file_path.as_deref() == Some(path)
        || repo_state
            .conflict_state
            .conflict_session
            .as_ref()
            .is_some_and(|session| session.path.as_path() == path)
}

fn apply_bulk_choice_to_session(
    session: &mut gitcomet_core::conflict_session::ConflictSession,
    choice: ConflictBulkChoice,
) -> usize {
    let mut applied = 0usize;

    for region in &mut session.regions {
        if region.resolution.is_resolved() {
            continue;
        }
        let Some(next) = (match choice {
            ConflictBulkChoice::Base => region
                .base
                .as_ref()
                .map(|_| ConflictRegionResolution::PickBase),
            ConflictBulkChoice::Ours => Some(ConflictRegionResolution::PickOurs),
            ConflictBulkChoice::Theirs => Some(ConflictRegionResolution::PickTheirs),
            ConflictBulkChoice::Both => Some(ConflictRegionResolution::PickBoth),
        }) else {
            continue;
        };
        region.resolution = next;
        applied += 1;
    }

    applied
}

pub(super) fn apply_autosolve_to_session(
    session: &mut gitcomet_core::conflict_session::ConflictSession,
    mode: ConflictAutosolveMode,
    whitespace_normalize: bool,
) -> ConflictAutosolveStats {
    let mut stats = ConflictAutosolveStats::default();
    match mode {
        ConflictAutosolveMode::Safe | ConflictAutosolveMode::Regex => {
            stats.pass1 = session.auto_resolve_safe_with_options(whitespace_normalize);
            stats.pass2_split = session.auto_resolve_pass2();
            if stats.pass2_split > 0 {
                stats.pass1_after_split =
                    session.auto_resolve_safe_with_options(whitespace_normalize);
            }
            if mode == ConflictAutosolveMode::Regex {
                stats.regex =
                    session.auto_resolve_regex(&RegexAutosolveOptions::whitespace_insensitive());
            }
        }
        ConflictAutosolveMode::History => {
            stats.history = session.auto_resolve_history(&HistoryAutosolveOptions::bullet_list());
        }
    }
    stats
}

fn reset_session_resolutions(
    session: &mut gitcomet_core::conflict_session::ConflictSession,
) -> usize {
    let mut reset = 0usize;
    for region in &mut session.regions {
        if matches!(region.resolution, ConflictRegionResolution::Unresolved) {
            continue;
        }
        region.resolution = ConflictRegionResolution::Unresolved;
        reset += 1;
    }
    reset
}
