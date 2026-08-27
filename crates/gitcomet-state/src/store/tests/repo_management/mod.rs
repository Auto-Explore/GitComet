use super::*;
use crate::model::{
    AppNotificationKind, CommitMultiSelection, ForeignDiffOrigin, GitLogSettings,
    GitLogTagFetchMode, InlineSubmoduleDiffState, RangeSelection, RepoLoadsInFlight,
    SidebarDataRequest, SidebarMode, ViewNavDir,
};
use gitcomet_core::domain::{CommitFileChange, FileStatusKind};
use rustc_hash::{FxHashMap, FxHashSet};

fn mark_repo_switch_secondary_metadata_ready(repo: &mut RepoState) {
    repo.branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.remotes = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.rebase_in_progress = Loadable::Ready(false);
    repo.merge_commit_message = Loadable::Ready(None);
}

fn has_full_refresh_only_effects(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadRemotes { repo_id: candidate }
                | Effect::LoadRemoteBranches { repo_id: candidate }
                if *candidate == repo_id
        )
    })
}

fn has_worktree_refresh_effect(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadWorktrees { repo_id: candidate } if *candidate == repo_id
        )
    })
}

fn has_cancel_repo_loads_effect(effects: &[Effect], repo_id: RepoId, load_epoch: u64) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::CancelRepoLoads {
                repo_id: candidate,
                load_epoch: candidate_epoch,
            } if *candidate == repo_id && *candidate_epoch == load_epoch
        )
    })
}

fn has_submodule_load_effect(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadSubmodules { repo_id: candidate } if *candidate == repo_id
        )
    })
}

fn has_stash_load_effect(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadStashes {
                repo_id: candidate,
                limit: 50
            } if *candidate == repo_id
        )
    })
}

fn has_effect_for_repo(
    effects: &[Effect],
    repo_id: RepoId,
    matches_effect: impl Fn(&Effect, RepoId) -> bool,
) -> bool {
    effects.iter().any(|effect| matches_effect(effect, repo_id))
}

fn mark_repo_open_ready(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) {
    let workdir = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .expect("repo exists")
        .spec
        .workdir
        .to_string_lossy()
        .into_owned();
    repos.insert(repo_id, Arc::new(DummyRepo::new(&workdir)));

    let repo_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id)
        .expect("repo exists");
    repo_state.set_open(Loadable::Ready(()));
    repo_state.missing_on_disk = false;
}

fn open_repo_ready(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    path: impl Into<PathBuf>,
) -> RepoId {
    reduce(repos, id_alloc, state, Msg::OpenRepo(path.into()));
    let repo_id = state.active_repo.expect("open repo should become active");
    mark_repo_open_ready(repos, state, repo_id);
    repo_id
}

fn assert_open_repo_history_mode_resolution(
    seed_session: impl FnOnce(&Path, &Path),
    expected: LogScope,
) {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");
    let session_file = dir.path().join("session.json");
    std::fs::create_dir_all(&repo_path).expect("create repo path");
    let normalized_repo_path = super::reducer::normalize_repo_path(repo_path.clone());

    let _session_file_override =
        crate::session::push_test_session_file_path_override(Some(session_file.clone()));
    seed_session(&normalized_repo_path, &session_file);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_path.clone()),
    );

    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert_eq!(state.repos[0].history_state.history_scope, expected);

    let spec = state.repos[0].spec.clone();
    let workdir = spec.workdir.to_string_lossy().into_owned();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec,
            repo: Arc::new(DummyRepo::new(&workdir)),
        }),
    );

    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog {
                repo_id,
                scope,
                ..
            } if *repo_id == RepoId(1) && *scope == expected
        )),
        "expected RepoOpenedOk to request LoadLog({expected:?}), got {effects:?}"
    );
}

fn file_browser_tree_entries() -> Vec<gitcomet_core::domain::FileEntry> {
    use gitcomet_core::domain::{FileEntry, FileEntryKind};

    let entry = |path: &str, kind, depth| FileEntry {
        name: PathBuf::from(path)
            .file_name()
            .expect("named entry")
            .to_string_lossy()
            .into_owned(),
        path: Arc::new(PathBuf::from(path)),
        kind,
        depth,
    };

    vec![
        entry("other", FileEntryKind::Directory, 0),
        entry("other/c.rs", FileEntryKind::File, 1),
        entry("src", FileEntryKind::Directory, 0),
        entry("src/nested", FileEntryKind::Directory, 1),
        entry("src/nested/b.rs", FileEntryKind::File, 2),
        entry("src/a.rs", FileEntryKind::File, 1),
    ]
}

fn state_with_file_browser_tree() -> (
    FxHashMap<RepoId, Arc<dyn GitRepository>>,
    AtomicU64,
    AppState,
    RepoId,
) {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo_id = RepoId(1);
    mark_repo_open_ready(&mut repos, &mut state, repo_id);
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(file_browser_tree_entries()));

    (repos, id_alloc, state, repo_id)
}

mod active_repo;
mod clone;
mod close_reorder;
mod file_browser_and_misc;
mod open_drop;
mod repo_opened;
mod restore_session;
mod worker_commands;
