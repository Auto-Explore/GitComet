use super::*;

struct BlockedStatusRepository {
    inner: UnconfiguredRepository,
    started: std::sync::mpsc::Sender<CancellationToken>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl GitRepository for BlockedStatusRepository {
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
        self.started.send(cancellation.clone()).unwrap();
        let _ = self
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        cancellation.check_cancelled()?;
        Ok(RepoStatus::default())
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

    fn pull(
        &self,
        _mode: PullMode,
        _prune: bool,
    ) -> Result<gitcomet_core::services::CommandOutput> {
        self.inner.pull(_mode, _prune)
    }

    fn push(&self) -> Result<()> {
        self.inner.push()
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        self.inner.discard_worktree_changes(_paths)
    }
}

#[test]
fn removing_an_open_worktree_cancels_its_running_loads() {
    for force in [false, true] {
        let (store, _events) = AppStore::new_test(Arc::new(FailingBackend));
        store.disable_repo_monitors_for_test();
        let removed_id = RepoId(2);
        let removed_path = std::env::temp_dir().join(format!(
            "gitcomet-removed-worktree-loads-{}-{force}",
            std::process::id()
        ));
        let (started, running) = std::sync::mpsc::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let blocked: Arc<dyn GitRepository> = Arc::new(BlockedStatusRepository {
            inner: UnconfiguredRepository::new(removed_path.clone()),
            started,
            release: Mutex::new(gate),
        });
        let owner: Arc<dyn GitRepository> = Arc::new(UnconfiguredRepository::new(
            std::env::temp_dir().join(format!(
                "gitcomet-removing-worktree-owner-{}-{force}",
                std::process::id()
            )),
        ));
        let mut state = AppState::test_default();
        for (repo_id, repository) in [(RepoId(1), &owner), (removed_id, &blocked)] {
            let mut repo = RepoState::new_opening(repo_id, repository.spec().clone());
            repo.open = Loadable::Ready(());
            state.repos.push(repo);
        }
        state.active_repo = Some(RepoId(1));
        store.replace_snapshot_for_test(Arc::new(state));
        store.insert_repo_for_test(RepoId(1), owner);
        store.insert_repo_for_test(removed_id, blocked);
        store.dispatch(Msg::ReloadRepo {
            repo_id: removed_id,
        });
        let cancellation = running
            .recv_timeout(Duration::from_secs(5))
            .expect("status load entered backend");
        assert!(!cancellation.is_cancelled(), "the load starts live");

        let command = if force {
            RepoCommandKind::ForceRemoveWorktree { path: removed_path }
        } else {
            RepoCommandKind::RemoveWorktree { path: removed_path }
        };
        store.dispatch(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: RepoId(1),
                command,
                result: Ok(CommandOutput::empty_success("git worktree remove")),
            },
        ));
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let closed = !store
            .snapshot()
            .repos
            .iter()
            .any(|repo| repo.id == removed_id);
        let cancelled_while_store_alive = cancellation.is_cancelled();
        drop(release);
        store
            .shutdown_for_test()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(
            closed,
            "the successful command closes the removed worktree tab"
        );
        assert!(
            cancelled_while_store_alive,
            "closing via worktree removal must cancel its running load before shutdown (force={force})"
        );
    }
}
