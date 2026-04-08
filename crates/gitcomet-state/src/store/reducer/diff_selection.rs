use super::util::{
    SelectedConflictTarget, apply_selected_diff_load_plan_state, selected_conflict_target,
    selected_diff_load_plan, start_conflict_target_reload, start_conflict_target_reload_with_mode,
    start_current_conflict_target_reload,
};
use crate::model::{AppState, ConflictFileLoadMode, DiagnosticKind, Loadable, RepoId};
use crate::msg::Effect;
use gitcomet_core::domain::{
    Diff, DiffArea, DiffPreviewTextFile, DiffPreviewTextSide, DiffTarget, FileDiffImage,
    FileDiffText,
};
use gitcomet_core::error::Error;
use smallvec::SmallVec;
use std::sync::Arc;

pub(crate) const SELECT_DIFF_INLINE_EFFECT_CAPACITY: usize = 1;
pub(crate) type SelectDiffEffects = SmallVec<[Effect; SELECT_DIFF_INLINE_EFFECT_CAPACITY]>;

pub(super) fn select_diff(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
) -> Vec<Effect> {
    let mut effects = SelectDiffEffects::new();
    fill_select_diff_inline(state, repo_id, target, &mut effects);
    effects.into_vec()
}

pub(super) fn fill_select_diff_inline(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    effects: &mut SelectDiffEffects,
) {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return;
    };

    if let Some(conflict_target) = selected_conflict_target(repo_state, &target) {
        repo_state.diff_state.diff_target = Some(target.clone());
        repo_state.diff_state.diff = Loadable::NotLoaded;
        repo_state.diff_state.diff_file = Loadable::NotLoaded;
        repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
        repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
        repo_state.bump_diff_state_rev();
        let conflict_effects = match conflict_target {
            SelectedConflictTarget::Current => start_current_conflict_target_reload(repo_state),
            SelectedConflictTarget::Path(path) => start_conflict_target_reload(repo_state, path),
        };
        debug_assert!(conflict_effects.len() <= SELECT_DIFF_INLINE_EFFECT_CAPACITY);
        effects.extend(conflict_effects);
        return;
    }

    repo_state.diff_state.diff_target = Some(target);
    let load_plan = {
        let target = repo_state
            .diff_state
            .diff_target
            .as_ref()
            .expect("diff target set before load planning");
        selected_diff_load_plan(repo_state, target)
    };
    apply_selected_diff_load_plan_state(repo_state, load_plan);
    repo_state.bump_diff_state_rev();

    effects.push(Effect::LoadSelectedDiff {
        repo_id,
        load_patch_diff: load_plan.load_patch_diff,
        load_file_text: load_plan.load_file_text,
        preview_text_side: load_plan.preview_text_side,
        load_file_image: load_plan.load_file_image,
    });
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
    repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
    repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
    repo_state.bump_diff_state_rev();

    start_conflict_target_reload_with_mode(repo_state, &path, ConflictFileLoadMode::CurrentOnly)
}

pub(super) fn clear_diff_selection(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    repo_state.diff_state.diff_target = None;
    repo_state.diff_state.diff = Loadable::NotLoaded;
    repo_state.diff_state.diff_file = Loadable::NotLoaded;
    repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
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
        if selected_conflict_target(repo_state, &target).is_some() {
            return Vec::new();
        }
        let current_plan = selected_diff_load_plan(repo_state, &target);
        if !current_plan.load_patch_diff {
            return Vec::new();
        }
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
        if selected_conflict_target(repo_state, &target).is_some() {
            return Vec::new();
        }
        let current_plan = selected_diff_load_plan(repo_state, &target);
        if !current_plan.load_file_text {
            return Vec::new();
        }
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
        if selected_conflict_target(repo_state, &target).is_some() {
            return Vec::new();
        }
        let current_plan = selected_diff_load_plan(repo_state, &target);
        if !current_plan.load_file_image {
            return Vec::new();
        }
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

pub(super) fn diff_preview_text_file_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    side: DiffPreviewTextSide,
    result: std::result::Result<Option<std::path::PathBuf>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.diff_state.diff_target.as_ref() == Some(&target)
    {
        let current_plan = selected_diff_load_plan(repo_state, &target);
        if current_plan.preview_text_side != Some(side) {
            return Vec::new();
        }

        repo_state.diff_state.diff_preview_text_file_rev = repo_state
            .diff_state
            .diff_preview_text_file_rev
            .wrapping_add(1);
        repo_state.diff_state.diff_preview_text_file = match result {
            Ok(path) => {
                Loadable::Ready(path.map(|path| Arc::new(DiffPreviewTextFile { path, side })))
            }
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.bump_diff_state_rev();
    }
    Vec::new()
}
