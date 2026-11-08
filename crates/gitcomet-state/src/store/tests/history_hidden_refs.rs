use super::*;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::HistoryMode;
use gitcomet_core::services::{HistoryRefFilter, HistoryWalk};

fn opened_repo(id: u64, scope: HistoryMode) -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(id),
        RepoSpec {
            workdir: PathBuf::from(format!("/repo-{id}")),
        },
    );
    repo.open = Loadable::Ready(());
    repo.set_log_scope(scope);
    repo.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: Vec::new(),
        next_cursor: None,
    })));
    repo
}

fn load_log_walks(effects: &[Effect]) -> Vec<(RepoId, HistoryWalk)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LoadLog { repo_id, walk, .. } => Some((*repo_id, walk.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn hiding_refs_reloads_the_active_walk_and_unloads_other_all_branches_walks() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = std::sync::atomic::AtomicU64::new(4);
    let mut state = AppState::test_default();
    state.repos.push(opened_repo(1, HistoryMode::AllBranches));
    state.repos.push(opened_repo(2, HistoryMode::AllBranches));
    state.repos.push(opened_repo(3, HistoryMode::FirstParent));
    state.active_repo = Some(RepoId(1));
    let filter = HistoryRefFilter::excluding(["refs/t3", "refs/claude"]);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryHiddenRefs(filter.clone()),
    );

    assert_eq!(
        load_log_walks(&effects),
        [(
            RepoId(1),
            HistoryWalk::from(HistoryMode::AllBranches).with_hidden_refs(filter.clone())
        )],
        "only the active repository walks now"
    );
    assert!(matches!(state.repos[0].log, Loadable::Loading));
    assert!(
        matches!(state.repos[1].log, Loadable::NotLoaded),
        "an inactive all-branches walk reloads on activation"
    );
    assert!(
        matches!(state.repos[2].log, Loadable::Ready(_)),
        "a first-parent walk has no refs to hide"
    );
    for repo in &state.repos {
        assert_eq!(repo.history_state.hidden_refs, filter);
    }

    let repeated = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryHiddenRefs(filter),
    );
    assert!(repeated.is_empty(), "{repeated:?}");
}

#[test]
fn an_opened_repository_walks_without_hidden_refs_from_the_first_load() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = std::sync::atomic::AtomicU64::new(1);
    let mut state = AppState::test_default();
    let filter = HistoryRefFilter::excluding(["refs/t3"]);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryHiddenRefs(filter.clone()),
    );
    let path = PathBuf::from("/tmp/hidden-refs-repo");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepoFromExternalDrop(path.clone()),
    );
    let spec = state.repos[0].spec.clone();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repository_mode: gitcomet_core::domain::RepositoryMode::WorkingTree,
            head_commit: None,
            preferences: Some(crate::model::RepositoryPreferencesSnapshot {
                key: crate::model::RepositoryKey::Worktree(path.clone()),
                revision: 1,
                preferences: Arc::new(crate::model::SharedRepositoryPreferences {
                    history_mode: HistoryMode::AllBranches,
                    ..Default::default()
                }),
            }),
            repo_id: RepoId(1),
            spec,
            repo: Arc::new(DummyRepo::new(path.to_string_lossy().as_ref())),
        }),
    );

    assert_eq!(
        load_log_walks(&effects),
        [(
            RepoId(1),
            HistoryWalk::from(HistoryMode::AllBranches).with_hidden_refs(filter)
        )]
    );
}

#[test]
fn restored_repositories_carry_the_hidden_refs() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = std::sync::atomic::AtomicU64::new(1);
    let mut state = AppState::test_default();
    let filter = HistoryRefFilter::excluding(["refs/claude"]);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryHiddenRefs(filter.clone()),
    );
    let saved = std::env::temp_dir().join("hidden-refs-restored");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![saved.clone()],
            active_repo: Some(saved),
        },
    );
    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.repos[0].history_state.hidden_refs, filter);
}
