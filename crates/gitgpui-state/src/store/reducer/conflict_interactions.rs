use crate::model::{AppState, RepoId};
use crate::msg::{ConflictAutosolveMode, ConflictBulkChoice, ConflictRegionChoice, Effect};
use gitgpui_core::conflict_session::{
    ConflictRegionResolution, HistoryAutosolveOptions, RegexAutosolveOptions,
};
use std::path::Path;
use std::path::PathBuf;

pub(super) fn set_hide_resolved(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    hide_resolved: bool,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, &path) {
        return Vec::new();
    }
    repo_state.set_conflict_hide_resolved(hide_resolved);
    Vec::new()
}

pub(super) fn apply_bulk_choice(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    choice: ConflictBulkChoice,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, &path) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path {
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
    path: PathBuf,
    region_index: usize,
    choice: ConflictRegionChoice,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, &path) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path {
        return Vec::new();
    }

    let Some(region) = session.regions.get_mut(region_index) else {
        return Vec::new();
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
        return Vec::new();
    };

    if region.resolution != next_resolution {
        region.resolution = next_resolution;
        repo_state.bump_conflict_rev();
    }
    Vec::new()
}

pub(super) fn apply_autosolve(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    mode: ConflictAutosolveMode,
    whitespace_normalize: bool,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, &path) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path {
        return Vec::new();
    }

    let resolved = apply_autosolve_to_session(session, mode, whitespace_normalize);
    if resolved > 0 {
        repo_state.bump_conflict_rev();
    }
    Vec::new()
}

fn matches_current_conflict_path(repo_state: &crate::model::RepoState, path: &Path) -> bool {
    repo_state.conflict_file_path.as_deref() == Some(path)
        || repo_state
            .conflict_session
            .as_ref()
            .is_some_and(|session| session.path.as_path() == path)
}

fn apply_bulk_choice_to_session(
    session: &mut gitgpui_core::conflict_session::ConflictSession,
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

fn apply_autosolve_to_session(
    session: &mut gitgpui_core::conflict_session::ConflictSession,
    mode: ConflictAutosolveMode,
    whitespace_normalize: bool,
) -> usize {
    match mode {
        ConflictAutosolveMode::Safe => {
            let pass1 = session.auto_resolve_safe_with_options(whitespace_normalize);
            let pass2 = session.auto_resolve_pass2();
            let pass1_after_split = if pass2 > 0 {
                session.auto_resolve_safe_with_options(whitespace_normalize)
            } else {
                0
            };
            pass1 + pass2 + pass1_after_split
        }
        ConflictAutosolveMode::Regex => {
            let pass1 = session.auto_resolve_safe_with_options(whitespace_normalize);
            let pass2 = session.auto_resolve_pass2();
            let pass1_after_split = if pass2 > 0 {
                session.auto_resolve_safe_with_options(whitespace_normalize)
            } else {
                0
            };
            let regex =
                session.auto_resolve_regex(&RegexAutosolveOptions::whitespace_insensitive());
            pass1 + pass2 + pass1_after_split + regex
        }
        ConflictAutosolveMode::History => {
            session.auto_resolve_history(&HistoryAutosolveOptions::bullet_list())
        }
    }
}
