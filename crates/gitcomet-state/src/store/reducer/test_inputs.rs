use super::prepared::{PreparedReduction, ReducerInputs};
use crate::model::{AppState, RepoState};
use crate::msg::Msg;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::DiffTarget;

pub(in crate::store) fn clock() -> ReducerInputs {
    for_message(
        &AppState::test_default(),
        &Default::default(),
        Msg::CancelAuthPrompt,
    )
}

pub(in crate::store) fn for_message(
    state: &AppState,
    repos: &RepositoryRuntimes,
    message: Msg,
) -> ReducerInputs {
    PreparedReduction::new(message, state, repos).into_parts().1
}

pub(in crate::store) fn for_target(repo: &RepoState, target: &DiffTarget) -> ReducerInputs {
    let mut state = AppState::test_default();
    state.repos.push(repo.clone());
    for_message(
        &state,
        &Default::default(),
        Msg::SelectDiff {
            repo_id: repo.id,
            target: target.clone(),
        },
    )
}
