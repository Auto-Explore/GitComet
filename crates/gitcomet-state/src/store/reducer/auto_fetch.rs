use super::prepared::ReducerInputs;
use super::util::{push_command_log, refresh_log_request};
use crate::model::{
    AppState, CommandSource, LoadKind, Loadable, RepoAutoFetchState, RepoId, RepoLifetime,
};
use crate::msg::{Effect, ReferenceEffect, RepoCommandKind};
use gitcomet_core::services::{CommandOutput, Result};
use std::sync::Arc;

pub(super) fn updated(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    lifetime: RepoLifetime,
    revision: u64,
    snapshot: RepoAutoFetchState,
    result: Option<Arc<Result<CommandOutput>>>,
) -> Vec<Effect> {
    let Some(repo) = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id && repo.lifetime_id() == lifetime)
    else {
        return Vec::new();
    };
    if revision <= repo.auto_fetch_revision {
        return Vec::new();
    }
    repo.auto_fetch_revision = revision;
    repo.auto_fetch = snapshot;
    let unchecked = match &repo.incoming {
        crate::model::RepoIncomingState::Current(incoming) => {
            skipped_upstream(repo, &incoming.tracking)
        }
        _ => false,
    };
    if unchecked
        || matches!(
            repo.auto_fetch.status,
            crate::model::AutoFetchStatus::Failed { .. }
        )
    {
        repo.incoming.mark_stale();
    }
    if let Some(result) = result {
        match result.as_ref() {
            Ok(output) => push_command_log(
                inputs,
                repo,
                true,
                &RepoCommandKind::FetchAll,
                output,
                None,
                false,
            ),
            Err(error) => push_command_log(
                inputs,
                repo,
                false,
                &RepoCommandKind::FetchAll,
                &CommandOutput::default(),
                Some(error),
                false,
            ),
        }
        if let Some(entry) = repo.feedback_mut().command_log.last_mut() {
            entry.source = CommandSource::Background;
            entry.announce_success = false;
            entry.hook_operation_id = None;
        }
    }
    Vec::new()
}

/// The last automatic fetch left out the remote `tracking` follows, so a
/// count read since says nothing new about it.
pub(super) fn skipped_upstream(
    repo: &crate::model::RepoState,
    tracking: &gitcomet_core::domain::HeadTracking,
) -> bool {
    let gitcomet_core::domain::HeadTracking::Tracking { upstream, .. } = tracking else {
        return false;
    };
    let fetch = &repo.auto_fetch;
    fetch.skipped_remotes.contains(&upstream.remote)
        || fetch.paused_remotes.contains(&upstream.remote)
}

pub(super) fn refresh(
    state: &mut AppState,
    repo_id: RepoId,
    lifetime: RepoLifetime,
) -> Vec<Effect> {
    let Some(repo) = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id && repo.lifetime_id() == lifetime)
    else {
        return Vec::new();
    };
    let mut effects = Vec::new();
    repo.refresh_read_failed = false;
    repo.incoming.mark_stale();
    if repo
        .loads_in_flight
        .request_background(LoadKind::UpstreamDivergence)
    {
        effects.push(Effect::References(
            ReferenceEffect::LoadUpstreamDivergence { repo_id },
        ));
    }
    if repo.backend_parked || !matches!(repo.open, Loadable::Ready(())) {
        return effects;
    }
    for (kind, effect) in [
        (
            LoadKind::Branches,
            ReferenceEffect::LoadBranches { repo_id },
        ),
        (
            LoadKind::RemoteBranches,
            ReferenceEffect::LoadRemoteBranches { repo_id },
        ),
        (LoadKind::Remotes, ReferenceEffect::LoadRemotes { repo_id }),
    ] {
        if repo.loads_in_flight.request_background(kind) {
            effects.push(Effect::References(effect));
        }
    }
    if !matches!(repo.tags, Loadable::NotLoaded)
        && repo.loads_in_flight.request_background(LoadKind::Tags)
    {
        effects.push(Effect::References(ReferenceEffect::LoadTags { repo_id }));
    }
    if !matches!(repo.history_state.log, Loadable::NotLoaded) {
        let request = refresh_log_request(repo);
        if let Some(seq) = repo.loads_in_flight.request_background_log(request.clone()) {
            effects.push(Effect::LoadLog {
                repo_id,
                seq,
                walk: request.walk,
                author: request.author,
                focus: request.focus,
                limit: request.limit,
                cursor: request.cursor,
            });
        }
    }
    effects
}

pub(super) fn refresh_failed(
    inputs: &ReducerInputs,
    repo: &mut crate::model::RepoState,
    error: gitcomet_core::error::Error,
) {
    repo.refresh_read_failed = true;
    if !matches!(error.kind(), gitcomet_core::error::ErrorKind::Cancelled) {
        repo.auto_fetch.status = crate::model::AutoFetchStatus::Failed {
            at: inputs.now(),
            error,
        };
    }
}

pub(super) fn has_quiet_failure<T>(
    inputs: &ReducerInputs,
    repo: &mut crate::model::RepoState,
    background: bool,
    result: &Result<T>,
) -> bool {
    if background && let Err(error) = result {
        refresh_failed(inputs, repo, error.clone());
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests;
