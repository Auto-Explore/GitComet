use super::*;
use gitcomet_core::domain::{FileStatus, FileStatusKind};

struct RecordedGitlinkRepository {
    inner: UnconfiguredRepository,
    started: std::sync::mpsc::Sender<PathBuf>,
    release: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
    head_result: Result<bool>,
    summaries: Option<std::sync::mpsc::Sender<DiffTarget>>,
    status: Option<RepoStatus>,
}

impl GitRepository for RecordedGitlinkRepository {
    fn spec(&self) -> &RepoSpec {
        self.inner.spec()
    }

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        self.inner.log_head_page(_limit, _cursor, cancellation)
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        self.inner.commit_details(_id)
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        self.inner.reflog_head(_limit)
    }

    fn current_branch(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<String> {
        self.inner.current_branch(cancellation)
    }

    fn list_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Branch>> {
        self.inner.list_branches(cancellation)
    }

    fn list_remotes(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Remote>> {
        self.inner.list_remotes(cancellation)
    }

    fn list_remote_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        self.inner.list_remote_branches(cancellation)
    }

    fn status(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<RepoStatus> {
        if let Some(status) = &self.status {
            cancellation.check_cancelled()?;
            return Ok(status.clone());
        }
        self.inner.status(cancellation)
    }

    fn head_path_is_gitlink(&self, path: &Path) -> Result<bool> {
        self.started.send(path.to_path_buf()).unwrap();
        if let Some(release) = &self.release {
            release.lock().unwrap().recv().unwrap();
        }
        self.head_result.clone()
    }

    fn submodule_diff_summary(
        &self,
        target: &DiffTarget,
        cancellation: &CancellationToken,
    ) -> Result<gitcomet_core::domain::SubmoduleDiffSummary> {
        if let Some(summaries) = &self.summaries {
            summaries.send(target.clone()).unwrap();
        }
        self.inner.submodule_diff_summary(target, cancellation)
    }

    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        self.inner.diff_unified(_target)
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        self.inner.create_branch(_name, _target)
    }

    fn delete_branch(&self, _name: &str) -> Result<()> {
        self.inner.delete_branch(_name)
    }

    fn checkout_branch(&self, _name: &str) -> Result<()> {
        self.inner.checkout_branch(_name)
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        self.inner.checkout_commit(_id)
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        self.inner.cherry_pick(_id)
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        self.inner.stash_create(_message, _include_untracked)
    }

    fn stash_list(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<StashEntry>> {
        self.inner.stash_list(cancellation)
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        self.inner.stash_apply(_index)
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        self.inner.stash_drop(_index)
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        self.inner.stage(_paths)
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        self.inner.unstage(_paths)
    }

    fn commit(&self, _message: &str) -> Result<()> {
        self.inner.commit(_message)
    }

    fn fetch_all(&self, _prune: bool) -> Result<gitcomet_core::services::CommandOutput> {
        self.inner.fetch_all(_prune)
    }

    fn pull(&self, _mode: PullMode) -> Result<gitcomet_core::services::CommandOutput> {
        self.inner.pull(_mode)
    }

    fn push(&self) -> Result<()> {
        self.inner.push()
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        self.inner.discard_worktree_changes(_paths)
    }
}

#[test]
fn snapshot_remains_readable_while_selected_gitlink_query_is_blocked() {
    let directory = tempfile::tempdir().unwrap();
    let (store, _events) = AppStore::new_test(Arc::new(FailingBackend));
    store.disable_repo_monitors_for_test();
    let repo_id = RepoId(1);
    let path = PathBuf::from("vendor/submodule");
    let target = DiffTarget::working_tree(path.clone(), DiffArea::Staged);
    let (started, entered) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let repository = Arc::new(RecordedGitlinkRepository {
        inner: UnconfiguredRepository::new(directory.path().to_path_buf()),
        started,
        release: Some(Mutex::new(gate)),
        head_result: Ok(true),
        summaries: None,
        status: None,
    });
    let mut repo = RepoState::new_opening(repo_id, repository.spec().clone());
    repo.set_open(Loadable::Ready(()));
    repo.set_status(Loadable::Ready(Arc::new(RepoStatus {
        staged: Arc::new(vec![FileStatus {
            path: path.clone(),
            kind: FileStatusKind::Deleted,
            conflict: None,
        }]),
        unstaged: Arc::default(),
    })));
    let mut state = AppState::test_default();
    state.repos.push(repo);
    state.active_repo = Some(repo_id);
    store.replace_snapshot_for_test(Arc::new(state));
    store.insert_repo_for_test(repo_id, repository);
    let publication_before = store
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(5))
        .unwrap();

    store.dispatch(Msg::SelectDiff {
        repo_id,
        target: target.clone(),
    });
    let queried = entered.recv_timeout(Duration::from_secs(5)).unwrap();
    store.dispatch(Msg::ReportError {
        repo_id: None,
        message: "after selection".into(),
    });
    let while_blocked = store.try_snapshot_with_publication();
    let write_lock_available = store.state.try_write().is_ok();
    let snapshot_handles = while_blocked
        .as_ref()
        .map(|(state, _)| Arc::strong_count(state));
    release.send(()).unwrap();
    store
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let after = store.snapshot();
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(5))
        .unwrap();

    assert_eq!(queried, path);
    let (before, publication) = while_blocked
        .expect("snapshot reads must remain available while the selected HEAD query is blocked");
    assert!(
        write_lock_available,
        "query execution must release the capture read guard"
    );
    assert_eq!(
        snapshot_handles,
        Some(2),
        "preparation must retain no snapshot Arc into mutation"
    );
    assert_eq!(publication, publication_before);
    assert!(before.repos[0].diff_state.diff_target.is_none());
    assert!(
        before.notifications.is_empty(),
        "later messages stay queued during preparation"
    );
    assert_eq!(after.repos[0].diff_state.diff_target, Some(target));
    assert!(after.repos[0].head_gitlink_paths.contains(&path));
    assert!(
        after
            .notifications
            .iter()
            .any(|notice| notice.message == "after selection")
    );
}

fn selected_fixture(
    root: &Path,
    status: Option<Vec<FileStatus>>,
    result: Result<bool>,
) -> (
    AppState,
    RepositoryRuntimes,
    DiffTarget,
    std::sync::mpsc::Receiver<PathBuf>,
) {
    let repo_id = RepoId(1);
    let target = DiffTarget::working_tree(PathBuf::from("vendor/submodule"), DiffArea::Unstaged);
    let (started, queries) = std::sync::mpsc::channel();
    let repository = Arc::new(RecordedGitlinkRepository {
        inner: UnconfiguredRepository::new(root.to_path_buf()),
        started,
        release: None,
        head_result: result,
        summaries: None,
        status: None,
    });
    let mut repo = RepoState::new_opening(repo_id, repository.spec().clone());
    repo.set_open(Loadable::Ready(()));
    if let Some(entries) = status {
        repo.set_status(Loadable::Ready(Arc::new(RepoStatus {
            staged: Arc::default(),
            unstaged: Arc::new(entries),
        })));
    }
    let mut repos = RepositoryRuntimes::default();
    repos.opened(repo_id, repo.lifetime_id(), repository);
    let mut state = AppState::test_default();
    state.repos.push(repo);
    state.active_repo = Some(repo_id);
    (state, repos, target, queries)
}

fn selected_status(kind: FileStatusKind) -> Vec<FileStatus> {
    vec![FileStatus {
        path: PathBuf::from("vendor/submodule"),
        kind,
        conflict: None,
    }]
}

fn prepare_retained_selection(state: &mut AppState, target: &DiffTarget) {
    let repo = &mut state.repos[0];
    repo.head_branch = Loadable::Ready("main".into());
    repo.upstream_divergence = Loadable::Ready(None);
    repo.branches = Loadable::Ready(Arc::default());
    repo.tags = Loadable::Ready(Arc::default());
    repo.remotes = Loadable::Ready(Arc::default());
    repo.remote_branches = Loadable::Ready(Arc::default());
    repo.stashes = Loadable::Ready(Arc::default());
    repo.rebase_in_progress = Loadable::Ready(false);
    repo.merge_commit_message = Loadable::Ready(None);
    repo.worktrees = Loadable::Ready(Arc::default());
    repo.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: Vec::new(),
        next_cursor: None,
    })));
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff = Loadable::Ready(Arc::new(gitcomet_core::domain::Diff {
        target: target.clone(),
        lines: Vec::new(),
    }));
    repo.diff_state.diff_file = Loadable::Ready(None);
    let mut other = RepoState::new_opening(RepoId(2), repo.spec.clone());
    other.set_open(Loadable::Ready(()));
    state.repos.push(other);
    state.active_repo = Some(RepoId(2));
}

#[test]
fn unchanged_tab_switch_skips_retained_selection_inputs() {
    use crate::msg::{PersistenceEffect, RepoCommandEffect};
    use crate::store::reducer::prepared::PreparedReduction;

    let directory = tempfile::tempdir().unwrap();
    for kind in [FileStatusKind::Deleted, FileStatusKind::Modified] {
        let (mut state, repos, target, queries) =
            selected_fixture(directory.path(), Some(selected_status(kind)), Ok(true));
        prepare_retained_selection(&mut state, &target);
        state.maintenance_settings.recommend = true;
        state.repos[0].common_dir = Some(Arc::from(directory.path().join(".git")));
        let retained = state.repos[0].diff_state.diff.clone();
        let shared = std::sync::RwLock::new(Arc::new(state));

        let prepared = PreparedReduction::for_repo_switch(RepoId(1), &shared, &repos, true);
        assert_eq!(prepared.selected_input_counts(), (0, 0));
        assert!(queries.try_iter().next().is_none());
        let (_, inputs) = prepared.into_parts();
        let mut state = shared.into_inner().unwrap();
        let state = Arc::make_mut(&mut state);
        let mut effects = reducer::SetActiveRepoEffects::new();
        reducer::fill_set_active_repo_inline(&inputs, state, RepoId(1), true, &mut effects);

        assert_eq!(state.active_repo, Some(RepoId(1)));
        assert_eq!(
            state.repos[0].diff_state.diff_target.as_ref(),
            Some(&target)
        );
        assert_eq!(state.repos[0].diff_state.diff, retained);
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadSelectedDiff { .. }))
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadWorktreeDirty { .. }))
        );
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::Persistence(PersistenceEffect::PersistSession { .. })
        )));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::RepoCommand(RepoCommandEffect::CheckRepoMaintenance { repo_id: RepoId(1) })
        )));
    }
}

#[test]
fn changed_or_incomplete_tab_switch_prepares_retained_selection_inputs() {
    use crate::model::LoadKind;
    use crate::store::reducer::prepared::PreparedReduction;

    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("vendor/submodule")).unwrap();
    fs::write(
        directory.path().join("vendor/submodule/.git"),
        b"gitdir: ../../.git/modules/submodule\n",
    )
    .unwrap();
    type Invalidate = fn(&mut RepoState);
    let cases: [(&str, bool, Invalidate); 8] = [
        ("changed", false, |_| {}),
        ("missing status", true, |repo| {
            repo.set_status(Loadable::NotLoaded)
        }),
        ("loading history", true, |repo| {
            repo.set_log(Loadable::Loading)
        }),
        ("missing diff", true, |repo| {
            repo.diff_state.diff = Loadable::NotLoaded
        }),
        ("loading file", true, |repo| {
            repo.diff_state.diff_file = Loadable::Loading
        }),
        ("cancelled refresh", true, |repo| {
            repo.left_mid_refresh = true
        }),
        ("refresh in flight", true, |repo| {
            repo.loads_in_flight.request(LoadKind::WorktreeStatus);
        }),
        ("conflict", true, |repo| {
            repo.conflict_state.conflict_file_path = Some(PathBuf::from("vendor/submodule"));
        }),
    ];
    for (case, unchanged, invalidate) in cases {
        for kind in [FileStatusKind::Deleted, FileStatusKind::Modified] {
            let (mut state, repos, target, queries) =
                selected_fixture(directory.path(), Some(selected_status(kind)), Ok(true));
            prepare_retained_selection(&mut state, &target);
            invalidate(&mut state.repos[0]);
            let expected_head =
                usize::from(kind == FileStatusKind::Deleted || case == "missing status");
            let shared = std::sync::RwLock::new(Arc::new(state));
            let prepared =
                PreparedReduction::for_repo_switch(RepoId(1), &shared, &repos, unchanged);
            assert_eq!(
                prepared.selected_input_counts(),
                (
                    expected_head,
                    usize::from(
                        kind == FileStatusKind::Modified
                            && case != "missing status"
                            && case != "conflict"
                    )
                ),
                "{case}"
            );
            assert_eq!(queries.try_iter().count(), expected_head, "{case}");
            let (_, inputs) = prepared.into_parts();
            let mut state = shared.into_inner().unwrap();
            let state = Arc::make_mut(&mut state);
            let mut effects = reducer::SetActiveRepoEffects::new();
            reducer::fill_set_active_repo_inline(
                &inputs,
                state,
                RepoId(1),
                unchanged,
                &mut effects,
            );

            assert_eq!(state.active_repo, Some(RepoId(1)));
            if case == "conflict" {
                assert!(
                    effects.iter().any(|effect| matches!(
                        effect,
                        Effect::LoadSelectedConflictFile {
                            repo_id: RepoId(1),
                            ..
                        }
                    )),
                    "{case}: {effects:?}"
                );
            } else {
                assert!(
                    effects.iter().any(|effect| matches!(effect,
                    Effect::LoadSelectedDiff { load_submodule_summary, .. }
                        if *load_submodule_summary == (case != "missing status"))),
                    "{case}: {effects:?}"
                );
            }
        }
    }
}

#[test]
fn checked_activation_refresh_keeps_selected_path_inputs() {
    use crate::msg::RepoExternalChange;
    use crate::store::reducer::prepared::PreparedReduction;

    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("vendor/submodule")).unwrap();
    fs::write(
        directory.path().join("vendor/submodule/.git"),
        b"gitdir: ../../.git/modules/submodule\n",
    )
    .unwrap();
    for (change, kind, expected_head_queries) in [
        (RepoExternalChange::all(), FileStatusKind::Deleted, 1),
        (RepoExternalChange::all(), FileStatusKind::Modified, 0),
        (RepoExternalChange::Index, FileStatusKind::Modified, 0),
        (RepoExternalChange::Worktree, FileStatusKind::Modified, 0),
    ] {
        let (mut state, mut repos, target, queries) =
            selected_fixture(directory.path(), Some(selected_status(kind)), Ok(true));
        state.repos[0].diff_state.diff_target = Some(target.clone());
        let shared = std::sync::RwLock::new(Arc::new(state));
        // The checked monitor reply reaches preparation as an external change.
        let (message, inputs) = PreparedReduction::for_worker(
            Msg::RepoExternallyChanged {
                repo_id: RepoId(1),
                change,
            },
            &shared,
            &repos,
        )
        .into_parts();
        let mut state = shared.into_inner().unwrap();
        let state = Arc::make_mut(&mut state);
        let effects =
            reducer::reduce_prepared(&inputs, &mut repos, &AtomicU64::new(2), state, message);

        assert_eq!(
            queries.try_iter().collect::<Vec<_>>(),
            vec![target.file_path().unwrap().to_path_buf(); expected_head_queries]
        );
        assert_eq!(state.repos[0].diff_state.diff_target, Some(target.clone()));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadSubmoduleSummary { repo_id: RepoId(1), target: requested }
                if *requested == target
        )));
    }
}

#[test]
fn selected_head_lookup_queries_only_deleted_and_unknown_status() {
    let directory = tempfile::tempdir().unwrap();
    for (status, expected_queries) in [
        (None, 1),
        (Some(selected_status(FileStatusKind::Deleted)), 1),
        (Some(selected_status(FileStatusKind::Modified)), 0),
        (Some(selected_status(FileStatusKind::Untracked)), 0),
        (Some(Vec::new()), 0),
    ] {
        let (mut state, mut repos, target, queries) =
            selected_fixture(directory.path(), status, Ok(true));
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(2),
            &mut state,
            Msg::SelectDiff {
                repo_id: RepoId(1),
                target: target.clone(),
            },
        );
        assert_eq!(
            queries.try_iter().collect::<Vec<_>>(),
            vec![target.file_path().unwrap().to_path_buf(); expected_queries]
        );
        assert_eq!(state.repos[0].diff_state.diff_target, Some(target));
        assert!(!effects.is_empty());
    }
}

#[test]
fn activation_head_lookup_distinguishes_unavailable_absent_and_first_matching_status() {
    let directory = tempfile::tempdir().unwrap();
    for (status, expected_queries) in [
        (None, 1),
        (Some(Vec::new()), 0),
        (Some(selected_status(FileStatusKind::Deleted)), 1),
        (Some(selected_status(FileStatusKind::Modified)), 0),
        (Some(selected_status(FileStatusKind::Untracked)), 0),
        (
            Some(
                [
                    selected_status(FileStatusKind::Modified),
                    selected_status(FileStatusKind::Deleted),
                ]
                .concat(),
            ),
            0,
        ),
        (
            Some(
                [
                    selected_status(FileStatusKind::Deleted),
                    selected_status(FileStatusKind::Modified),
                ]
                .concat(),
            ),
            1,
        ),
    ] {
        let (mut state, mut repos, target, queries) =
            selected_fixture(directory.path(), status, Ok(true));
        state.active_repo = None;
        state.repos[0].diff_state.diff_target = Some(target.clone());
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(2),
            &mut state,
            Msg::SetActiveRepo { repo_id: RepoId(1) },
        );
        assert_eq!(
            queries.try_iter().collect::<Vec<_>>(),
            vec![target.file_path().unwrap().to_path_buf(); expected_queries]
        );
        assert_eq!(state.active_repo, Some(RepoId(1)));
        assert_eq!(state.repos[0].diff_state.diff_target, Some(target));
        assert!(!effects.is_empty());
    }
}

#[test]
fn activation_applies_head_answer_before_planning_a_retained_deletion() {
    let directory = tempfile::tempdir().unwrap();
    for (result, cached, expected_submodule) in [
        (
            Err(Error::new(ErrorKind::Backend("HEAD unavailable".into()))),
            true,
            true,
        ),
        (Ok(false), true, false),
        (Ok(true), false, true),
    ] {
        let (mut state, mut repos, target, queries) = selected_fixture(
            directory.path(),
            Some(selected_status(FileStatusKind::Deleted)),
            result,
        );
        let path = target.file_path().unwrap().to_path_buf();
        state.active_repo = None;
        state.repos[0].diff_state.diff_target = Some(target);
        if cached {
            state.repos[0].head_gitlink_paths.insert(path.clone());
        }
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(2),
            &mut state,
            Msg::SetActiveRepo { repo_id: RepoId(1) },
        );
        assert_eq!(queries.try_iter().collect::<Vec<_>>(), vec![path.clone()]);
        assert_eq!(
            state.repos[0].head_gitlink_paths.contains(&path),
            expected_submodule
        );
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadSelectedDiff { load_submodule_summary, .. }
                if *load_submodule_summary == expected_submodule
        )));
    }
}

#[test]
fn reload_prepares_head_using_the_post_reset_status() {
    let directory = tempfile::tempdir().unwrap();
    let (mut state, mut repos, target, queries) = selected_fixture(
        directory.path(),
        Some(selected_status(FileStatusKind::Modified)),
        Ok(true),
    );
    state.repos[0].diff_state.diff_target = Some(target.clone());
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(2),
        &mut state,
        Msg::ReloadRepo { repo_id: RepoId(1) },
    );
    assert_eq!(
        queries.try_iter().collect::<Vec<_>>(),
        vec![target.file_path().unwrap().to_path_buf()]
    );
    assert!(
        state.repos[0]
            .head_gitlink_paths
            .contains(target.file_path().unwrap())
    );
    assert_eq!(state.repos[0].diff_state.diff_target, Some(target));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadLog { .. }))
    );
}

#[test]
fn failed_head_lookup_keeps_the_cache_but_a_negative_answer_removes_it() {
    let directory = tempfile::tempdir().unwrap();
    for (result, retained) in [
        (
            Err(Error::new(ErrorKind::Backend("HEAD unavailable".into()))),
            true,
        ),
        (Ok(false), false),
        (Ok(true), true),
    ] {
        let (mut state, mut repos, target, queries) = selected_fixture(
            directory.path(),
            Some(selected_status(FileStatusKind::Deleted)),
            result,
        );
        let path = target.file_path().unwrap().to_path_buf();
        state.repos[0].head_gitlink_paths.insert(path.clone());
        reduce(
            &mut repos,
            &AtomicU64::new(2),
            &mut state,
            Msg::SelectDiff {
                repo_id: RepoId(1),
                target,
            },
        );
        assert_eq!(queries.try_iter().collect::<Vec<_>>(), vec![path.clone()]);
        assert_eq!(state.repos[0].head_gitlink_paths.contains(&path), retained);
    }
}

#[test]
fn stale_load_completion_does_not_prepare_a_head_query() {
    let directory = tempfile::tempdir().unwrap();
    let (mut state, mut repos, target, queries) = selected_fixture(
        directory.path(),
        Some(selected_status(FileStatusKind::Deleted)),
        Ok(true),
    );
    state.repos[0].diff_state.diff_target = Some(target.clone());
    let stale = state.repos[0].load_epoch;
    state.repos[0].load_epoch = stale.next();
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(2),
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
            repo_id: RepoId(1),
            load_epoch: stale,
            message: Box::new(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                action: RepoActionKind::CheckoutBranch,
                result: Ok(()),
            }),
        }),
    );
    assert!(effects.is_empty());
    assert!(queries.try_iter().next().is_none());
    assert!(state.repos[0].head_gitlink_paths.is_empty());
    assert_eq!(state.repos[0].diff_state.diff_target, Some(target));
}

#[test]
fn auth_worktree_retry_prepares_the_path_only_when_git_is_available() {
    use crate::model::{AuthPromptKind, AuthPromptState, AuthRetryOperation};
    use crate::msg::RepoCommandEffect;
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("linked")).unwrap();
    for available in [false, true] {
        let (mut state, mut repos, _, queries) =
            selected_fixture(directory.path(), None, Ok(false));
        if !available {
            state.git_runtime.availability =
                gitcomet_core::process::GitExecutableAvailability::Unavailable {
                    detail: "test Git unavailable".into(),
                };
        }
        state.auth_prompt = Some(AuthPromptState {
            generation: 1,
            challenge: None,
            status: crate::model::AuthPromptStatus::AwaitingInput,
            kind: AuthPromptKind::Passphrase,
            reason: "test signing prompt".into(),
            operation: AuthRetryOperation::RepoCommand {
                repo_id: RepoId(1),
                command: RepoCommandKind::RemoveWorktree {
                    path: "linked".into(),
                    delete_branch: None,
                },
            },
        });
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(2),
            &mut state,
            Msg::SubmitAuthPrompt {
                generation: 1,
                username: None,
                secret: "test-passphrase".into(),
            },
        );
        assert!(queries.try_iter().next().is_none());
        if available {
            let [Effect::RepoCommand(RepoCommandEffect::RemoveWorktree { path, .. })] =
                effects.as_slice()
            else {
                panic!("expected one worktree removal: {effects:?}");
            };
            assert_eq!(
                *path,
                canonicalize_or_original(directory.path().join("linked"))
            );
        } else {
            assert!(effects.is_empty());
        }
    }
}

fn cancelled_operation(message: crate::msg::InternalMsg) -> Msg {
    Msg::Internal(crate::msg::InternalMsg::GitOperationFinished {
        repo_id: RepoId(1),
        operation_id: gitcomet_core::git_operation::GitOperationId(1),
        outer_outcome: crate::model::GitOperationOuterOutcome::Cancelled,
        duration: Duration::ZERO,
        message: Box::new(message),
    })
}

fn cancelled_maintenance_refresh(status: Option<FileStatusKind>) {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("vendor/submodule")).unwrap();
    fs::write(
        directory.path().join("vendor/submodule/.git"),
        b"gitdir: test",
    )
    .unwrap();
    for another_maintenance_running in [true, false] {
        let (mut state, mut repos, target, queries) =
            selected_fixture(directory.path(), status.map(selected_status), Ok(true));
        let repo = &mut state.repos[0];
        repo.common_dir = Some(Arc::from(directory.path().join(".git")));
        repo.maintenance.running = true;
        repo.diff_state.diff_target = Some(target.clone());
        assert!(repo.maintenance.deferred_change.is_none());
        assert!(
            repo.loads_in_flight
                .request_log(crate::model::PendingLogLoad {
                    focus: Default::default(),
                    walk: repo.history_state.history_scope.into(),
                    author: repo.history_state.history_author_filter.clone(),
                    limit: 200,
                    cursor: None,
                })
                .is_some()
        );
        let mut other = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: directory.path().join("other"),
            },
        );
        other.common_dir = repo.common_dir.clone();
        other.maintenance.running = another_maintenance_running;
        state.repos.push(other);

        let effects = reduce(
            &mut repos,
            &AtomicU64::new(3),
            &mut state,
            cancelled_operation(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: RepoId(1),
                command: RepoCommandKind::RunMaintenance,
                result: Err(Error::new(ErrorKind::Cancelled)),
            }),
        );
        assert!(!state.repos[0].maintenance.running);
        assert_eq!(
            state.repos[1].maintenance.running,
            another_maintenance_running
        );
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadLog { .. }))
        );
        let queried = queries.try_iter().collect::<Vec<_>>();
        if another_maintenance_running {
            assert!(queried.is_empty());
            assert_eq!(
                state.repos[0].maintenance.deferred_change,
                Some(RepoExternalChange::all())
            );
            assert!(!effects.iter().any(|effect| matches!(
                effect,
                Effect::LoadSubmoduleSummary { .. } | Effect::LoadDiff { .. }
            )));
        } else {
            let expects_head = status != Some(FileStatusKind::Modified);
            assert_eq!(
                queried,
                if expects_head {
                    vec![target.file_path().unwrap().to_path_buf()]
                } else {
                    Vec::new()
                }
            );
            assert!(state.repos[0].maintenance.deferred_change.is_none());
            assert_eq!(effects.iter().filter(|effect| matches!(effect,
                Effect::LoadSubmoduleSummary { repo_id: RepoId(1), target: loaded } if *loaded == target
            )).count(), usize::from(status.is_some()));
            assert_eq!(
                effects
                    .iter()
                    .filter(|effect| matches!(effect,
                        Effect::LoadDiff { repo_id: RepoId(1), target: loaded } if *loaded == target
                    ))
                    .count(),
                usize::from(status.is_none())
            );
        }
        assert_eq!(state.repos[0].diff_state.diff_target, Some(target));
    }
}

#[test]
fn cancelled_maintenance_completion_replans_a_modified_submodule() {
    cancelled_maintenance_refresh(Some(FileStatusKind::Modified));
}

#[test]
fn cancelled_maintenance_completion_reclassifies_a_deleted_submodule() {
    cancelled_maintenance_refresh(Some(FileStatusKind::Deleted));
}

#[test]
fn cancelled_maintenance_completion_reclassifies_unknown_status() {
    cancelled_maintenance_refresh(None);
}

#[test]
fn cancelled_action_uses_its_inner_log_refresh_without_an_extra_head_query() {
    use crate::msg::InternalMsg;
    let directory = tempfile::tempdir().unwrap();
    let cancelled = || Error::new(ErrorKind::Cancelled);
    for (label, active_scope, clears_selection, emits_log, message) in [
        (
            "action replaces an active log",
            Some(LogScope::AllBranches),
            false,
            true,
            InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                action: RepoActionKind::StagePath,
                result: Err(cancelled()),
            },
        ),
        (
            "paths action clears the selection",
            Some(LogScope::AllBranches),
            true,
            true,
            InternalMsg::RepoPathsActionFinished {
                repo_id: RepoId(1),
                action: RepoActionKind::StagePaths,
                paths: vec![PathBuf::from("vendor/submodule")].into(),
                result: Ok(()),
            },
        ),
        (
            "command starts an idle log",
            None,
            false,
            true,
            InternalMsg::RepoCommandFinished {
                repo_id: RepoId(1),
                command: RepoCommandKind::FetchAll,
                result: Err(cancelled()),
            },
        ),
        (
            "command changes the log scope",
            Some(LogScope::CurrentBranch),
            false,
            true,
            InternalMsg::RepoCommandFinished {
                repo_id: RepoId(1),
                command: RepoCommandKind::FetchAll,
                result: Err(cancelled()),
            },
        ),
        (
            "successful reset clears the selection",
            Some(LogScope::AllBranches),
            true,
            false,
            InternalMsg::RepoCommandFinished {
                repo_id: RepoId(1),
                command: RepoCommandKind::Reset {
                    mode: gitcomet_core::services::ResetMode::Mixed,
                    target: "HEAD".into(),
                },
                result: Ok(CommandOutput::empty_success("git reset --mixed HEAD")),
            },
        ),
        (
            "commit failure starts an idle log",
            None,
            false,
            true,
            InternalMsg::CommitFinished {
                repo_id: RepoId(1),
                result: Err(cancelled()),
            },
        ),
    ] {
        let (mut state, mut repos, target, queries) = selected_fixture(
            directory.path(),
            Some(selected_status(FileStatusKind::Deleted)),
            Ok(true),
        );
        let repo = &mut state.repos[0];
        repo.diff_state.diff_target = Some(target.clone());
        repo.history_state.history_scope = LogScope::AllBranches;
        if let Some(scope) = active_scope {
            assert!(
                repo.loads_in_flight
                    .request_log(crate::model::PendingLogLoad {
                        focus: Default::default(),
                        walk: scope.into(),
                        author: None,
                        limit: 200,
                        cursor: None,
                    })
                    .is_some()
            );
        }
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(2),
            &mut state,
            cancelled_operation(message),
        );
        assert_eq!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::LoadLog {
                    repo_id: RepoId(1),
                    ..
                }
            )),
            emits_log,
            "{label}"
        );
        assert!(queries.try_iter().next().is_none(), "{label}");
        assert!(state.repos[0].head_gitlink_paths.is_empty(), "{label}");
        assert_eq!(
            state.repos[0].diff_state.diff_target,
            (!clears_selection).then_some(target),
            "{label}"
        );
    }
}

#[test]
fn deferred_worktree_and_index_changes_replan_the_selected_submodule_marker() {
    for change in [
        crate::msg::RepoExternalChange::Worktree,
        crate::msg::RepoExternalChange::Index,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir_all(root.join("vendor/submodule")).unwrap();
        fs::write(
            root.join("vendor/submodule/.git"),
            b"gitdir: ../../.git/modules/submodule\n",
        )
        .unwrap();
        let (mut state, _, target, _) = selected_fixture(
            root,
            Some(selected_status(FileStatusKind::Modified)),
            Ok(false),
        );
        let linked = state.repos[0].id;
        state.repos[0].diff_state.diff_target = Some(target.clone());
        state.repos[0].common_dir = Some(Arc::from(root.join(".git")));
        let main = RepoId(2);
        let mut main_repo = RepoState::new_opening(
            main,
            RepoSpec {
                workdir: root.join("main"),
            },
        );
        main_repo.set_open(Loadable::Ready(()));
        main_repo.common_dir = state.repos[0].common_dir.clone();
        main_repo.maintenance.running = true;
        state.repos.push(main_repo);
        let (started, queries) = std::sync::mpsc::channel();
        let (summaries, summary_requested) = std::sync::mpsc::channel();
        let repository = Arc::new(RecordedGitlinkRepository {
            inner: UnconfiguredRepository::new(root.to_path_buf()),
            started,
            release: None,
            head_result: Ok(false),
            summaries: Some(summaries),
            status: Some(RepoStatus {
                staged: Arc::default(),
                unstaged: Arc::new(selected_status(FileStatusKind::Modified)),
            }),
        });
        let (store, _events) = AppStore::new_test(Arc::new(FailingBackend));
        store.disable_repo_monitors_for_test();
        store.replace_snapshot_for_test(Arc::new(state));
        store.insert_repo_for_test(linked, repository);
        store.dispatch(Msg::RepoExternallyChanged {
            repo_id: linked,
            change: change.clone(),
        });
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        {
            let snapshot = store.snapshot();
            assert_eq!(snapshot.repos[0].maintenance.deferred_change, Some(change));
            assert!(matches!(
                snapshot.repos[0].diff_state.submodule_summary,
                Loadable::NotLoaded
            ));
        }
        assert!(queries.try_iter().next().is_none());
        assert!(summary_requested.try_iter().next().is_none());
        store.dispatch(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: main,
                command: RepoCommandKind::RunMaintenance,
                result: Ok(CommandOutput::empty_success("git maintenance run --auto")),
            },
        ));
        let requested = summary_requested.recv_timeout(Duration::from_secs(5));
        // The fake records the request before its reply is queued, so a
        // barrier alone can overtake the reply.
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(
            store.snapshot().repos[0].diff_state.submodule_summary,
            Loadable::NotLoaded
        ) && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let snapshot = store.snapshot();
        store
            .shutdown_for_test()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(requested.unwrap(), target);
        assert!(
            queries.try_iter().next().is_none(),
            "worktree/index replay must not query HEAD"
        );
        assert!(snapshot.repos[0].maintenance.deferred_change.is_none());
        assert!(!snapshot.repos[1].maintenance.running);
        assert_eq!(snapshot.repos[0].diff_state.diff_target, Some(target));
        assert!(!matches!(
            snapshot.repos[0].diff_state.submodule_summary,
            Loadable::NotLoaded
        ));
    }
}
