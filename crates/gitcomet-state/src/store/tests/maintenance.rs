use super::*;
use crate::msg::RepoExternalChange;
use gitcomet_core::services::{GitRepository, Result};
use rustc_hash::FxHashMap;
use std::sync::atomic::AtomicU64;

/// A repository whose only real answer is its shared git directory.
struct CommonDirRepo {
    spec: RepoSpec,
    common_dir: PathBuf,
}

fn unsupported<T>() -> Result<T> {
    Err(Error::new(ErrorKind::Unsupported("maintenance test repo")))
}

impl GitRepository for CommonDirRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }
    fn common_dir(&self) -> Option<PathBuf> {
        Some(self.common_dir.clone())
    }
    fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<Arc<LogPage>> {
        unsupported()
    }
    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unsupported()
    }
    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unsupported()
    }
    fn current_branch(&self) -> Result<String> {
        unsupported()
    }
    fn list_branches(&self) -> Result<Vec<Branch>> {
        unsupported()
    }
    fn list_remotes(&self) -> Result<Vec<Remote>> {
        unsupported()
    }
    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        unsupported()
    }
    fn status(&self) -> Result<RepoStatus> {
        unsupported()
    }
    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unsupported()
    }
    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        unsupported()
    }
    fn delete_branch(&self, _name: &str) -> Result<()> {
        unsupported()
    }
    fn checkout_branch(&self, _name: &str) -> Result<()> {
        unsupported()
    }
    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        unsupported()
    }
    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        unsupported()
    }
    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unsupported()
    }
    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        unsupported()
    }
    fn stash_apply(&self, _index: usize) -> Result<()> {
        unsupported()
    }
    fn stash_drop(&self, _index: usize) -> Result<()> {
        unsupported()
    }
    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported()
    }
    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported()
    }
    fn commit(&self, _message: &str) -> Result<()> {
        unsupported()
    }
    fn fetch_all(&self) -> Result<()> {
        unsupported()
    }
    fn pull(&self, _mode: PullMode) -> Result<()> {
        unsupported()
    }
    fn push(&self) -> Result<()> {
        unsupported()
    }
    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unsupported()
    }
}

fn open_repo_tab(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    workdir: &str,
    common_dir: &str,
) -> (RepoId, Vec<Effect>) {
    let workdir = PathBuf::from(workdir);
    reduce(repos, id_alloc, state, Msg::OpenRepo(workdir.clone()));
    let repo_id = state.active_repo.expect("opened tab is active");
    let spec = RepoSpec {
        workdir: workdir.clone(),
    };
    let effects = reduce(
        repos,
        id_alloc,
        state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            preferences: None,
            repo_id,
            spec: spec.clone(),
            repo: Arc::new(CommonDirRepo {
                spec,
                common_dir: PathBuf::from(common_dir),
            }),
        }),
    );
    (repo_id, effects)
}

fn repo(state: &AppState, repo_id: RepoId) -> &RepoState {
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .expect("repo")
}

#[test]
fn opening_the_active_tab_requests_one_maintenance_check_an_hour() {
    let mut repos = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let (repo_id, effects) = open_repo_tab(
        &mut repos,
        &id_alloc,
        &mut state,
        "/work/app",
        "/work/app/.git",
    );

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::CheckRepoMaintenance { repo_id: id } if *id == repo_id)),
        "{effects:?}"
    );
    assert_eq!(
        repo(&state, repo_id).common_dir.as_deref(),
        Some(Path::new("/work/app/.git"))
    );
    assert!(
        super::reducer::maintenance::request_check(&mut state, repo_id).is_none(),
        "focus right after opening does not ask again"
    );
}

#[test]
fn recommendation_start_and_snooze_cover_every_tab_of_the_repository() {
    let mut repos = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let (main, _) = open_repo_tab(
        &mut repos,
        &id_alloc,
        &mut state,
        "/work/app",
        "/work/app/.git",
    );
    let (linked, _) = open_repo_tab(
        &mut repos,
        &id_alloc,
        &mut state,
        "/work/app-linked",
        "/work/app/.git",
    );
    let checked = |needed| {
        Msg::Internal(crate::msg::InternalMsg::RepoMaintenanceChecked {
            repo_id: linked,
            needed,
        })
    };

    reduce(&mut repos, &id_alloc, &mut state, checked(false));
    assert!(!repo(&state, linked).maintenance.recommended);
    reduce(&mut repos, &id_alloc, &mut state, checked(true));
    assert!(repo(&state, linked).maintenance.recommended);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SnoozeRepoMaintenance { repo_id: linked },
    );
    assert!(!repo(&state, linked).maintenance.recommended);
    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistRepoMaintenanceSnooze { common_dir }] if common_dir == Path::new("/work/app/.git")
    ));

    reduce(&mut repos, &id_alloc, &mut state, checked(true));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::StartRepoMaintenance { repo_id: main },
    );
    assert!(matches!(effects.as_slice(), [Effect::RunMaintenance { repo_id }] if *repo_id == main));
    assert!(repo(&state, main).maintenance.running);
    assert!(!repo(&state, linked).maintenance.recommended);
    // One run per repository, whichever tab asks.
    let again = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::StartRepoMaintenance { repo_id: linked },
    );
    assert!(again.is_empty(), "{again:?}");
    reduce(&mut repos, &id_alloc, &mut state, checked(true));
    assert!(
        !repo(&state, linked).maintenance.recommended,
        "no recommendation while it runs"
    );
}

#[test]
fn external_changes_wait_for_maintenance_and_replay_when_it_ends() {
    let mut repos = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let (main, _) = open_repo_tab(
        &mut repos,
        &id_alloc,
        &mut state,
        "/work/app",
        "/work/app/.git",
    );
    let (linked, _) = open_repo_tab(
        &mut repos,
        &id_alloc,
        &mut state,
        "/work/app-linked",
        "/work/app/.git",
    );
    let (other, _) = open_repo_tab(
        &mut repos,
        &id_alloc,
        &mut state,
        "/work/other",
        "/work/other/.git",
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::StartRepoMaintenance { repo_id: main },
    );

    let held = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: linked,
            change: RepoExternalChange::Worktree,
        },
    );
    assert!(held.is_empty(), "{held:?}");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: linked,
            change: RepoExternalChange::GitState,
        },
    );
    assert_eq!(
        repo(&state, linked).maintenance.deferred_change,
        Some(RepoExternalChange::Worktree.union(RepoExternalChange::GitState))
    );
    assert!(
        repo(&state, other).maintenance.deferred_change.is_none(),
        "another repository refreshes as usual"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: main,
            command: RepoCommandKind::RunMaintenance,
            result: Ok(CommandOutput::empty_success("git maintenance run --auto")),
        }),
    );
    assert!(!repo(&state, main).maintenance.running);
    assert!(repo(&state, linked).maintenance.deferred_change.is_none());
    assert!(
        effects
            .iter()
            .any(|effect| format!("{effect:?}").contains(&format!("{linked:?}"))),
        "the held-back refresh runs: {effects:?}"
    );
}

#[test]
fn recommendations_turned_off_ask_nothing_and_withdraw_cards() {
    let mut repos = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let (main, _) = open_repo_tab(
        &mut repos,
        &id_alloc,
        &mut state,
        "/work/app",
        "/work/app/.git",
    );
    let (other, _) = open_repo_tab(
        &mut repos,
        &id_alloc,
        &mut state,
        "/work/other",
        "/work/other/.git",
    );
    for repo_id in [main, other] {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoMaintenanceChecked {
                repo_id,
                needed: true,
            }),
        );
    }

    let off = crate::model::MaintenanceSettings { recommend: false };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetMaintenanceSettings(off),
    );
    assert!(!repo(&state, main).maintenance.recommended);
    assert!(!repo(&state, other).maintenance.recommended);

    // A check that was already running when the user turned it off.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoMaintenanceChecked {
            repo_id: main,
            needed: true,
        }),
    );
    assert!(!repo(&state, main).maintenance.recommended);

    let (opened, effects) = open_repo_tab(
        &mut repos,
        &id_alloc,
        &mut state,
        "/work/third",
        "/work/third/.git",
    );
    let switched = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: main },
    );
    let checks = effects
        .iter()
        .chain(&switched)
        .filter(|effect| matches!(effect, Effect::CheckRepoMaintenance { .. }))
        .count();
    assert_eq!(checks, 0, "open: {effects:?}, switch: {switched:?}");
    assert!(super::reducer::maintenance::request_check(&mut state, opened).is_none());
}
