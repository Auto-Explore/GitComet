use super::prepared::ReducerInputs;
use crate::model::{AppState, BareModeConversionState, RepoId};
use crate::msg::{ControlEffect, Effect};
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::{
    BareConversionLayout, BareConversionPreview, BareModeDisabled, CommitId, RepositoryMode,
};
use gitcomet_core::error::Error;
use std::path::Path;
use std::sync::{Arc, mpsc};

fn clear_watch_leases(state: &mut AppState, repo_id: RepoId) {
    Arc::make_mut(&mut state.watch_leases).remove(&repo_id);
    Arc::make_mut(&mut state.worktree_watch_leases).retain(|(id, _, _), _| *id != repo_id);
}

pub(super) fn preview(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    if !repo.is_bare() || repo.bare_mode_conversion.is_busy() {
        return Vec::new();
    }
    repo.bare_mode_conversion = BareModeConversionState::Previewing;
    repo.bump_ops_rev();
    vec![Effect::Control(ControlEffect::PreviewDisableBareMode {
        repo_id,
    })]
}

pub(super) fn preview_loaded(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    result: Result<BareConversionPreview, Error>,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(
        repo.bare_mode_conversion,
        BareModeConversionState::Previewing
    ) {
        return Vec::new();
    }
    match result {
        Ok(preview) => {
            let preview = Arc::new(preview);
            repo.bare_mode_conversion =
                BareModeConversionState::AwaitingConfirmation(Arc::clone(&preview));
            if preview.layout == BareConversionLayout::ExistingCheckout {
                return disable(state, repo_id, preview);
            }
        }
        Err(error) => {
            super::util::push_diagnostic(
                inputs,
                repo,
                crate::model::DiagnosticKind::Error,
                error.to_string(),
            );
            repo.bare_mode_conversion = BareModeConversionState::Failed(Arc::new(error));
        }
    }
    Vec::new()
}

pub(super) fn disable(
    state: &mut AppState,
    repo_id: RepoId,
    preview: Arc<BareConversionPreview>,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    let BareModeConversionState::AwaitingConfirmation(current) = &repo.bare_mode_conversion else {
        return Vec::new();
    };
    if !Arc::ptr_eq(current, &preview) || !repo.is_bare() {
        return Vec::new();
    }
    repo.bare_mode_conversion = BareModeConversionState::Converting;
    repo.bump_ops_rev();
    vec![Effect::Control(ControlEffect::DisableBareMode {
        repo_id,
        common_dir: repo
            .common_dir
            .as_deref()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| preview.root.clone()),
        preview,
    })]
}

pub(super) fn clear(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id)
        && !repo.bare_mode_conversion.is_busy()
    {
        repo.bare_mode_conversion = BareModeConversionState::Idle;
    }
    Vec::new()
}

pub(super) fn quiesce(
    repos: &mut RepositoryRuntimes,
    state: &mut AppState,
    common_dir: &Path,
    reply: mpsc::Sender<()>,
) -> Vec<Effect> {
    let mut ids = Vec::new();
    for repo in &mut state.repos {
        if repo.common_dir.as_deref() != Some(common_dir) {
            continue;
        }
        repo.retire_repository_lifetime();
        repo.bare_mode_conversion = BareModeConversionState::Converting;
        repos.retire(&repo.id);
        ids.push(repo.id);
    }
    for id in ids {
        clear_watch_leases(state, id);
        super::auth::discard_auth_prompt_for_repo(state, id);
    }
    vec![Effect::Control(ControlEffect::RepositoryQuiesced { reply })]
}

pub(super) fn finished(
    inputs: &ReducerInputs,
    repos: &mut RepositoryRuntimes,
    state: &mut AppState,
    common_dir: &Path,
    result: Result<BareModeDisabled, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    let mut ids = Vec::new();
    for repo in &mut state.repos {
        if repo.common_dir.as_deref() != Some(common_dir)
            && !result.as_ref().is_ok_and(|result| {
                result
                    .affected_roots
                    .iter()
                    .any(|root| root == &repo.spec.workdir)
            })
        {
            continue;
        }
        repo.retire_repository_lifetime();
        repos.retire(&repo.id);
        ids.push(repo.id);
        repo.bare_mode_conversion = match &result {
            Ok(_) => BareModeConversionState::Reopening,
            Err(error) => {
                super::util::push_diagnostic(
                    inputs,
                    repo,
                    crate::model::DiagnosticKind::Error,
                    error.to_string(),
                );
                BareModeConversionState::Failed(Arc::new(error.clone()))
            }
        };
        effects.push(Effect::OpenRepo {
            repo_id: repo.id,
            path: repo.spec.workdir.clone(),
        });
    }
    for id in ids {
        clear_watch_leases(state, id);
    }
    effects
}

pub(super) fn mode_loaded(
    inputs: &ReducerInputs,
    repos: &mut RepositoryRuntimes,
    state: &mut AppState,
    repo_id: RepoId,
    result: Result<(RepositoryMode, Option<CommitId>), Error>,
) -> Vec<Effect> {
    let follow = state.file_browser_settings.follow_selected_commit;
    let sidebar_mode = state.sidebar_mode;
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    let (mode, head) = match result {
        Ok(value) => value,
        Err(error)
            if matches!(
                error.kind(),
                gitcomet_core::error::ErrorKind::RepositoryChanged
            ) =>
        {
            repo.retire_repository_lifetime();
            repos.retire(&repo_id);
            let path = repo.spec.workdir.clone();
            clear_watch_leases(state, repo_id);
            return vec![Effect::OpenRepo { repo_id, path }];
        }
        Err(error) => {
            super::util::push_diagnostic(
                inputs,
                repo,
                crate::model::DiagnosticKind::Error,
                error.to_string(),
            );
            return Vec::new();
        }
    };
    if repo.repository_mode.is_none() || repo.repository_mode == Some(mode) {
        repo.repository_mode = Some(mode);
        let changed = repo.head_commit != head;
        repo.head_commit = head;
        if changed && repo.is_bare() {
            repo.file_browser.followed_selection_rev = None;
            let source = if follow {
                repo.history_state
                    .selected_commit
                    .clone()
                    .map(gitcomet_core::domain::FileSource::Commit)
                    .or_else(|| repo.live_file_source())
            } else if !repo.file_browser.active {
                repo.live_file_source()
            } else {
                return Vec::new();
            };
            if let Some(source) = source {
                if super::effects::retarget_file_browser(repo, source)
                    && sidebar_mode == crate::model::SidebarMode::Files
                {
                    return super::effects::request_file_browser_load(repo)
                        .into_iter()
                        .collect();
                }
            } else {
                repo.file_browser.source = gitcomet_core::domain::FileSource::WorkingDirectory;
                repo.file_browser.entries = crate::model::Loadable::Ready(Arc::new(Vec::new()));
                repo.file_browser.stale = false;
                repo.file_browser.bump_rev();
            }
        }
        return Vec::new();
    }
    repo.repository_mode = Some(mode);
    repo.head_commit = head;
    repo.retire_repository_lifetime();
    repos.retire(&repo_id);
    let path = repo.spec.workdir.clone();
    clear_watch_leases(state, repo_id);
    vec![Effect::OpenRepo { repo_id, path }]
}
