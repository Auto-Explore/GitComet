use super::util::{
    diff_target_is_svg, diff_target_wants_image_preview, selected_conflict_target_path,
    start_conflict_target_reload, start_conflict_target_reload_with_mode,
};
use crate::model::{AppState, ConflictFileLoadMode, DiagnosticKind, Loadable, RepoId};
use crate::msg::Effect;
use gitcomet_core::domain::{Diff, DiffArea, DiffTarget, FileDiffImage, FileDiffText};
use gitcomet_core::error::Error;
use std::sync::Arc;

pub(super) fn select_diff(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    repo_state.diff_state.diff_target = Some(target.clone());
    if let Some(conflict_path) = selected_conflict_target_path(repo_state, &target) {
        repo_state.diff_state.diff = Loadable::NotLoaded;
        repo_state.diff_state.diff_file = Loadable::NotLoaded;
        repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
        repo_state.bump_diff_state_rev();
        return start_conflict_target_reload(repo_state, conflict_path);
    }

    repo_state.diff_state.diff = Loadable::Loading;
    let supports_file = matches!(
        &target,
        DiffTarget::WorkingTree { .. } | DiffTarget::Commit { path: Some(_), .. }
    );
    let wants_image = diff_target_wants_image_preview(&target);
    let is_svg = diff_target_is_svg(&target);
    repo_state.diff_state.diff_file = if supports_file && (!wants_image || is_svg) {
        Loadable::Loading
    } else {
        Loadable::NotLoaded
    };
    repo_state.diff_state.diff_file_image = if supports_file && wants_image {
        Loadable::Loading
    } else {
        Loadable::NotLoaded
    };
    repo_state.bump_diff_state_rev();

    // Build effects inline with precomputed flags to avoid redundant extension
    // checks that `diff_reload_effects` would redo. File payload loads are
    // ordered before the main diff load to keep selection-path ordering stable.
    let mut effects = Vec::with_capacity(2);
    if supports_file {
        if wants_image {
            effects.push(Effect::LoadDiffFileImage {
                repo_id,
                target: target.clone(),
            });
        }
        if !wants_image || is_svg {
            effects.push(Effect::LoadDiffFile {
                repo_id,
                target: target.clone(),
            });
        }
    }
    effects.push(Effect::LoadDiff { repo_id, target });
    effects
}

pub(super) fn select_conflict_diff(
    state: &mut AppState,
    repo_id: RepoId,
    path: std::path::PathBuf,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    let target = DiffTarget::WorkingTree {
        path: path.clone(),
        area: DiffArea::Unstaged,
    };
    repo_state.diff_state.diff_target = Some(target);
    repo_state.diff_state.diff = Loadable::NotLoaded;
    repo_state.diff_state.diff_file = Loadable::NotLoaded;
    repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
    repo_state.bump_diff_state_rev();

    start_conflict_target_reload_with_mode(repo_state, path, ConflictFileLoadMode::CurrentOnly)
}

pub(super) fn clear_diff_selection(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    repo_state.diff_state.diff_target = None;
    repo_state.diff_state.diff = Loadable::NotLoaded;
    repo_state.diff_state.diff_file = Loadable::NotLoaded;
    repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
    repo_state.bump_diff_state_rev();
    Vec::new()
}

pub(super) fn stage_hunk(repo_id: RepoId, patch: String) -> Vec<Effect> {
    vec![Effect::StageHunk { repo_id, patch }]
}

pub(super) fn unstage_hunk(repo_id: RepoId, patch: String) -> Vec<Effect> {
    vec![Effect::UnstageHunk { repo_id, patch }]
}

pub(super) fn apply_worktree_patch(repo_id: RepoId, patch: String, reverse: bool) -> Vec<Effect> {
    vec![Effect::ApplyWorktreePatch {
        repo_id,
        patch,
        reverse,
    }]
}

pub(super) fn diff_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    result: std::result::Result<Diff, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.diff_state.diff_target.as_ref() == Some(&target)
    {
        repo_state.diff_state.diff_rev = repo_state.diff_state.diff_rev.wrapping_add(1);
        repo_state.diff_state.diff = match result {
            Ok(v) => Loadable::Ready(Arc::new(v)),
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.bump_diff_state_rev();
    }
    Vec::new()
}

pub(super) fn diff_file_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    result: std::result::Result<Option<FileDiffText>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.diff_state.diff_target.as_ref() == Some(&target)
    {
        repo_state.diff_state.diff_file_rev = repo_state.diff_state.diff_file_rev.wrapping_add(1);
        repo_state.diff_state.diff_file = match result {
            Ok(v) => Loadable::Ready(v.map(Arc::new)),
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.bump_diff_state_rev();
    }
    Vec::new()
}

pub(super) fn diff_file_image_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    result: std::result::Result<Option<FileDiffImage>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.diff_state.diff_target.as_ref() == Some(&target)
    {
        repo_state.diff_state.diff_file_rev = repo_state.diff_state.diff_file_rev.wrapping_add(1);
        repo_state.diff_state.diff_file_image = match result {
            Ok(v) => Loadable::Ready(v.map(Arc::new)),
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.bump_diff_state_rev();
    }
    Vec::new()
}
