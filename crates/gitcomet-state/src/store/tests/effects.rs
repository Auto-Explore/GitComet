use super::*;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::Upstream;

static MERGETOOL_TRACE_TEST_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

pub(super) fn schedule_effect_with_state_for_test(
    executor: &super::executor::TaskExecutor,
    session_persist_executor: &super::executor::TaskExecutor,
    backend: &Arc<dyn GitBackend>,
    repos: &mut RepositoryRuntimes,
    state: AppState,
    msg_tx: std::sync::mpsc::Sender<Msg>,
    effect: Effect,
) {
    // Each invocation represents an independent effect fixture, as when its
    // task scope was a local map. Tests of shared scopes use the router directly.
    for repo in &state.repos {
        repos.take_load_scope(&repo.id);
    }
    repos.bind_test_state(&state);
    let thread_state = Arc::new(std::sync::RwLock::new(Arc::new(state)));
    let msg_tx = super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);

    let repo_load_executor = super::executor::TaskExecutor::new(1);
    let metadata_executor = super::executor::TaskExecutor::new(1);
    super::effects::schedule_effect(
        super::effects::EffectExecutors {
            executor,
            repo_load_executor: &repo_load_executor,
            worktree_scan_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
            session_persist_executor,
            metadata_executor: &metadata_executor,
            signature_executor: &metadata_executor,
            history_find_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
            filesystem_executor: &super::executor::FILESYSTEM_QUEUE,
        },
        &thread_state,
        backend,
        repos,
        msg_tx,
        effect,
    );
    // Helper-owned workers must exit before the test can finish: detached
    // workers can race process teardown on macOS.
    repo_load_executor.join();
    metadata_executor.join();
}

fn schedule_effect_for_test(
    executor: &super::executor::TaskExecutor,
    session_persist_executor: &super::executor::TaskExecutor,
    backend: &Arc<dyn GitBackend>,
    repos: &mut RepositoryRuntimes,
    msg_tx: std::sync::mpsc::Sender<Msg>,
    effect: Effect,
) {
    let mut state = AppState::test_default();
    if effect.requires_available_git() {
        state.repos = repos
            .repository_specs_for_test()
            .map(|(repo_id, spec)| {
                let mut repo = RepoState::new_opening(repo_id, spec);
                repo.open = Loadable::Ready(());
                repo
            })
            .collect();
        if let Some(repo_id) = crate::store::repo_load_trace::effect_repo_id(&effect)
            && !state.repos.iter().any(|repo| repo.id == repo_id)
        {
            state.repos.push(RepoState::new_opening(
                repo_id,
                RepoSpec {
                    workdir: "/tmp/missing-test-repository".into(),
                },
            ));
        }
    }
    schedule_effect_with_state_for_test(
        executor,
        session_persist_executor,
        backend,
        repos,
        state,
        msg_tx,
        effect,
    );
}

/// Receives the next externally meaningful effect message while treating the
/// Git-operation lifecycle as the envelope used by the real store reducer.
fn recv_effect_message(
    msg_rx: &std::sync::mpsc::Receiver<Msg>,
    timeout: Duration,
) -> std::result::Result<Msg, std::sync::mpsc::RecvTimeoutError> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match msg_rx.recv_timeout(remaining)? {
            Msg::Internal(crate::msg::InternalMsg::GitOperationStarted { .. })
            | Msg::Internal(crate::msg::InternalMsg::GitOperationEvent { .. }) => {}
            Msg::Internal(crate::msg::InternalMsg::GitOperationFinished { message, .. }) => {
                return Ok(Msg::Internal(*message));
            }
            message => return Ok(message),
        }
    }
}

fn unique_temp_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

fn unsupported_repo_result<T>() -> Result<T> {
    Err(Error::new(ErrorKind::Unsupported(
        "unsupported repo for effect scheduling coverage",
    )))
}

struct UnsupportedRepo {
    spec: RepoSpec,
    delete_branch_calls: Option<Arc<Mutex<Vec<String>>>>,
    cancel_delete_branch: Option<String>,
}

impl GitRepository for UnsupportedRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: Result<std::sync::Arc<LogPage>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unsupported_repo_result()
    }
    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unsupported_repo_result()
    }
    fn current_branch(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<String> {
        cancellation.check_cancelled()?;
        let result: Result<String> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Branch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Branch>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remotes(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Remote>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Remote>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remote_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<RemoteBranch>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn status(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<RepoStatus> {
        cancellation.check_cancelled()?;
        let result: Result<RepoStatus> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unsupported_repo_result()
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn delete_branch(&self, name: &str) -> Result<()> {
        if let Some(calls) = self.delete_branch_calls.as_ref() {
            calls
                .lock()
                .expect("delete branch recording mutex")
                .push(name.to_string());
        }
        if self.cancel_delete_branch.as_deref() == Some(name) {
            return Err(Error::new(ErrorKind::Cancelled));
        }
        unsupported_repo_result()
    }
    fn checkout_branch(&self, _name: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_list(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<StashEntry>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<StashEntry>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn stash_apply(&self, _index: usize) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_drop(&self, _index: usize) -> Result<()> {
        unsupported_repo_result()
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn commit(&self, _message: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn fetch_all(&self, _prune: bool) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { unsupported_repo_result() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git fetch --all"))
    }
    fn pull(&self, _mode: PullMode) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { unsupported_repo_result() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git pull"))
    }
    fn push(&self) -> Result<()> {
        unsupported_repo_result()
    }
    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
}

struct PanicOpenBackend;

impl GitBackend for PanicOpenBackend {
    fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
        panic!("open should not be called in effect scheduler tests")
    }
}

struct BlockingReleaseGuard {
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl Drop for BlockingReleaseGuard {
    fn drop(&mut self) {
        let (lock, condvar) = &*self.release;
        let mut released = lock.lock().expect("release mutex");
        *released = true;
        condvar.notify_all();
    }
}

fn wait_for_release_signal(release: &Arc<(Mutex<bool>, Condvar)>) {
    let (lock, condvar) = &**release;
    let mut released = lock.lock().expect("release mutex");
    while !*released {
        released = condvar.wait(released).expect("release wait");
    }
}

enum MetadataRepoMode {
    BlockingRemoteTags,
    ReadyTags,
}

struct MetadataSchedulingRepo {
    spec: RepoSpec,
    mode: MetadataRepoMode,
    started_tx: std::sync::mpsc::Sender<&'static str>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl GitRepository for MetadataSchedulingRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn lfs_locks(
        &self,
        _cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<gitcomet_core::large_files::LfsLock>> {
        let _ = self.started_tx.send("lfs_locks");
        wait_for_release_signal(&self.release);
        Ok(Vec::new())
    }

    fn annex_unused(
        &self,
        _cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<gitcomet_core::large_files::AnnexUnused> {
        let _ = self.started_tx.send("annex_unused");
        wait_for_release_signal(&self.release);
        Ok(Default::default())
    }

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: Result<std::sync::Arc<LogPage>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unsupported_repo_result()
    }
    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unsupported_repo_result()
    }
    fn current_branch(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<String> {
        cancellation.check_cancelled()?;
        let result: Result<String> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Branch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Branch>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_tags(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<gitcomet_core::domain::Tag>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<gitcomet_core::domain::Tag>> = {
            match self.mode {
                MetadataRepoMode::ReadyTags => {
                    let _ = self.started_tx.send("tags");
                    Ok(Vec::new())
                }
                MetadataRepoMode::BlockingRemoteTags => unsupported_repo_result(),
            }
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remote_tags(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<gitcomet_core::domain::RemoteTag>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<gitcomet_core::domain::RemoteTag>> = {
            match self.mode {
                MetadataRepoMode::BlockingRemoteTags => {
                    let _ = self.started_tx.send("remote_tags");
                    wait_for_release_signal(&self.release);
                    Ok(Vec::new())
                }
                MetadataRepoMode::ReadyTags => unsupported_repo_result(),
            }
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remotes(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Remote>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Remote>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remote_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<RemoteBranch>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn status(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<RepoStatus> {
        cancellation.check_cancelled()?;
        let result: Result<RepoStatus> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unsupported_repo_result()
    }
    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn delete_branch(&self, _name: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn checkout_branch(&self, _name: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_list(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<StashEntry>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<StashEntry>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn stash_apply(&self, _index: usize) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_drop(&self, _index: usize) -> Result<()> {
        unsupported_repo_result()
    }
    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn commit(&self, _message: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn fetch_all(&self, _prune: bool) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { unsupported_repo_result() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git fetch --all"))
    }
    fn pull(&self, _mode: PullMode) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { unsupported_repo_result() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git pull"))
    }
    fn push(&self) -> Result<()> {
        unsupported_repo_result()
    }
    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
}

enum SelectedDiffRepoMode {
    BlockingDiff,
    ReadyDiff,
}

struct SelectedDiffSchedulingRepo {
    spec: RepoSpec,
    mode: SelectedDiffRepoMode,
    started_tx: std::sync::mpsc::Sender<RepoId>,
    started_repo_id: RepoId,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl GitRepository for SelectedDiffSchedulingRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: Result<std::sync::Arc<LogPage>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unsupported_repo_result()
    }
    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unsupported_repo_result()
    }
    fn current_branch(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<String> {
        cancellation.check_cancelled()?;
        let result: Result<String> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Branch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Branch>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remotes(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Remote>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Remote>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remote_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<RemoteBranch>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn status(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<RepoStatus> {
        cancellation.check_cancelled()?;
        let result: Result<RepoStatus> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unsupported_repo_result()
    }
    fn uncommitted_line_stats(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<gitcomet_core::domain::UncommittedLineStats> {
        cancellation.check_cancelled()?;
        panic!("the executor must reuse the supplied status, not rescan")
    }
    fn uncommitted_line_stats_for_status(
        &self,
        status: &RepoStatus,
        cancellation: &CancellationToken,
    ) -> Result<gitcomet_core::domain::UncommittedLineStats> {
        assert_eq!(status.unstaged[0].path, PathBuf::from("snapshot-only.txt"));
        let _ = self.started_tx.send(self.started_repo_id);
        if matches!(self.mode, SelectedDiffRepoMode::BlockingDiff) {
            while !cancellation.is_cancelled() {
                let (lock, condvar) = &*self.release;
                let released = lock.lock().expect("release mutex");
                let (released, _) = condvar
                    .wait_timeout(released, Duration::from_millis(10))
                    .expect("release wait");
                if *released {
                    break;
                }
            }
        }
        cancellation.check_cancelled()?;
        Ok(Default::default())
    }
    fn diff_parsed(
        &self,
        target: &DiffTarget,
        _encoding: Option<gitcomet_core::text_format::TextEncoding>,
        cancellation: &CancellationToken,
    ) -> Result<gitcomet_core::domain::Diff> {
        let _ = self.started_tx.send(self.started_repo_id);
        if matches!(self.mode, SelectedDiffRepoMode::BlockingDiff) {
            while !cancellation.is_cancelled() {
                let (lock, condvar) = &*self.release;
                let released = lock.lock().expect("release mutex");
                let (released, _) = condvar
                    .wait_timeout(released, Duration::from_millis(10))
                    .expect("release wait");
                if *released {
                    break;
                }
            }
        }
        cancellation.check_cancelled()?;
        Ok(gitcomet_core::domain::Diff::from_unified(
            target.clone(),
            "diff --git a/tracked.txt b/tracked.txt\n",
        ))
    }
    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn delete_branch(&self, _name: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn checkout_branch(&self, _name: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_list(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<StashEntry>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<StashEntry>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn stash_apply(&self, _index: usize) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_drop(&self, _index: usize) -> Result<()> {
        unsupported_repo_result()
    }
    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn commit(&self, _message: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn fetch_all(&self, _prune: bool) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { unsupported_repo_result() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git fetch --all"))
    }
    fn pull(&self, _mode: PullMode) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { unsupported_repo_result() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git pull"))
    }
    fn push(&self) -> Result<()> {
        unsupported_repo_result()
    }
    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
}

struct RecordingLogRepo {
    spec: RepoSpec,
    calls: Arc<std::sync::Mutex<Vec<String>>>,
}

impl GitRepository for RecordingLogRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_history_mode_page_streaming(
        &self,
        mode: LogScope,
        author: Option<&str>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
        on_chunk: &mut dyn FnMut(gitcomet_core::services::LogChunk),
    ) -> Result<Arc<LogPage>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("stream {author:?}"));
        on_chunk(gitcomet_core::services::LogChunk {
            commits: Vec::new(),
            scanned: 1,
        });
        self.log_history_mode_page(mode, limit, cursor, cancellation)
    }

    fn log_history_mode_page_filtered(
        &self,
        mode: LogScope,
        author: Option<&str>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Arc<LogPage>> {
        cancellation.check_cancelled()?;
        self.calls
            .lock()
            .unwrap()
            .push(format!("filtered {author:?}"));
        self.log_history_mode_page(mode, limit, cursor, cancellation)
    }

    fn log_history_mode_page(
        &self,
        mode: LogScope,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: Result<std::sync::Arc<LogPage>> = {
            self.calls
                .lock()
                .expect("log recording mutex")
                .push(format!(
                    "history {mode:?} {limit} {}",
                    cursor
                        .map(|cursor| cursor.last_seen.as_ref())
                        .unwrap_or("none")
                ));
            Ok(std::sync::Arc::new(LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }))
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn log_head_page(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: Result<std::sync::Arc<LogPage>> = {
            self.calls
                .lock()
                .expect("log recording mutex")
                .push(format!(
                    "head {limit} {}",
                    cursor
                        .map(|cursor| cursor.last_seen.as_ref())
                        .unwrap_or("none")
                ));
            Ok(std::sync::Arc::new(LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }))
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unsupported_repo_result()
    }
    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unsupported_repo_result()
    }
    fn current_branch(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<String> {
        cancellation.check_cancelled()?;
        let result: Result<String> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Branch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Branch>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remotes(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Remote>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Remote>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remote_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<RemoteBranch>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn status(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<RepoStatus> {
        cancellation.check_cancelled()?;
        let result: Result<RepoStatus> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unsupported_repo_result()
    }
    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn delete_branch(&self, _name: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn checkout_branch(&self, _name: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_list(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<StashEntry>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<StashEntry>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn stash_apply(&self, _index: usize) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_drop(&self, _index: usize) -> Result<()> {
        unsupported_repo_result()
    }
    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn commit(&self, _message: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn fetch_all(&self, _prune: bool) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { unsupported_repo_result() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git fetch --all"))
    }
    fn pull(&self, _mode: PullMode) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { unsupported_repo_result() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git pull"))
    }
    fn push(&self) -> Result<()> {
        unsupported_repo_result()
    }
    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
}

struct RecordingCheckoutRepo {
    spec: RepoSpec,
    calls: Arc<std::sync::Mutex<Vec<String>>>,
    create_branch_already_exists: bool,
    rename_branch_already_exists: bool,
    /// Reported by `branch_checked_out_in_other_worktree` for every branch.
    other_worktree: Option<PathBuf>,
    /// Reported by `current_branch`; unsupported when `None`.
    current_branch: Option<String>,
    prepared_barrier: Option<PreparedPatchBarrier>,
    /// Holds `fetch_all` and `pull` until released.
    remote_barrier: Option<PreparedPatchBarrier>,
}

struct PreparedPatchBarrier {
    started: std::sync::mpsc::Sender<()>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl RecordingCheckoutRepo {
    fn new(spec: RepoSpec, calls: Arc<std::sync::Mutex<Vec<String>>>) -> Self {
        Self {
            spec,
            calls,
            create_branch_already_exists: false,
            rename_branch_already_exists: false,
            other_worktree: None,
            current_branch: None,
            prepared_barrier: None,
            remote_barrier: None,
        }
    }

    fn record(&self, call: String) {
        self.calls
            .lock()
            .expect("checkout recording mutex")
            .push(call);
    }

    fn remote_call(&self, name: &str) -> Result<gitcomet_core::services::CommandOutput> {
        let Some(barrier) = &self.remote_barrier else {
            return unsupported_repo_result();
        };
        barrier.started.send(()).unwrap();
        wait_for_release_signal(&barrier.release);
        self.record(name.into());
        Ok(gitcomet_core::services::CommandOutput::empty_success(
            format!("git {name}"),
        ))
    }
}

fn branch_already_exists_error(command: &str, name: &str) -> Error {
    Error::new(ErrorKind::Git(gitcomet_core::error::GitFailure::new(
        command,
        gitcomet_core::error::GitFailureId::BranchAlreadyExists,
        Some(128),
        Vec::new(),
        Vec::new(),
        Some(format!("a branch named '{name}' already exists")),
    )))
}

impl GitRepository for RecordingCheckoutRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn apply_prepared_diff_patch(
        &self,
        _prepared: &gitcomet_core::patch_action::PreparedDiffPatch,
    ) -> Result<CommandOutput> {
        let Some(barrier) = &self.prepared_barrier else {
            return unsupported_repo_result();
        };
        barrier.started.send(()).unwrap();
        wait_for_release_signal(&barrier.release);
        self.record("apply reviewed".into());
        Ok(CommandOutput::empty_success("reviewed patch"))
    }

    fn branch_checked_out_in_other_worktree(&self, _name: &str) -> Result<Option<PathBuf>> {
        Ok(self.other_worktree.clone())
    }
    fn rename_branch(&self, old_name: &str, new_name: &str) -> Result<()> {
        self.record(format!("rename {old_name} {new_name}"));
        if self.rename_branch_already_exists {
            return Err(branch_already_exists_error("git branch -m", new_name));
        }
        Ok(())
    }
    fn rename_branch_force(&self, old_name: &str, new_name: &str) -> Result<()> {
        self.record(format!("rename-force {old_name} {new_name}"));
        Ok(())
    }

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: Result<std::sync::Arc<LogPage>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unsupported_repo_result()
    }
    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unsupported_repo_result()
    }
    fn current_branch(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<String> {
        cancellation.check_cancelled()?;
        let result: Result<String> = {
            self.current_branch
                .clone()
                .map_or_else(unsupported_repo_result, Ok)
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Branch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Branch>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remotes(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Remote>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Remote>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn list_remote_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<RemoteBranch>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn status(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<RepoStatus> {
        cancellation.check_cancelled()?;
        let result: Result<RepoStatus> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unsupported_repo_result()
    }

    fn create_branch(&self, name: &str, target: &CommitId) -> Result<()> {
        self.record(format!("create {name} {}", target.as_ref()));
        if self.create_branch_already_exists {
            return Err(branch_already_exists_error("git branch", name));
        }
        Ok(())
    }
    fn create_branch_force_and_checkout(&self, name: &str, target: &CommitId) -> Result<()> {
        self.calls
            .lock()
            .expect("checkout recording mutex")
            .push(format!(
                "force-create-and-checkout {name} {}",
                target.as_ref()
            ));
        Ok(())
    }
    fn delete_branch(&self, _name: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn checkout_branch(&self, name: &str) -> Result<()> {
        self.calls
            .lock()
            .expect("checkout recording mutex")
            .push(format!("checkout {name}"));
        Ok(())
    }
    fn checkout_remote_branch(
        &self,
        remote: &str,
        branch: &str,
        local_branch: &str,
        mode: gitcomet_core::services::CheckoutRemoteBranchMode,
    ) -> Result<()> {
        self.calls
            .lock()
            .expect("checkout recording mutex")
            .push(format!(
                "checkout_remote {remote}/{branch} -> {local_branch} ({mode:?})"
            ));
        Ok(())
    }
    fn checkout_commit(&self, id: &CommitId) -> Result<()> {
        self.calls
            .lock()
            .expect("checkout recording mutex")
            .push(format!("checkout_commit {}", id.as_ref()));
        Ok(())
    }
    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_list(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<StashEntry>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<StashEntry>> = { unsupported_repo_result() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }
    fn stash_apply(&self, _index: usize) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_drop(&self, _index: usize) -> Result<()> {
        unsupported_repo_result()
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn commit(&self, _message: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn fetch_all(&self, _prune: bool) -> Result<gitcomet_core::services::CommandOutput> {
        self.remote_call("fetch")
    }
    fn pull(&self, _mode: PullMode) -> Result<gitcomet_core::services::CommandOutput> {
        self.remote_call("pull")
    }
    fn push(&self) -> Result<()> {
        unsupported_repo_result()
    }
    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
}

struct RecordingWorktreeBackend {
    repo: Arc<dyn GitRepository>,
    opened: Arc<std::sync::Mutex<Vec<PathBuf>>>,
}

impl GitBackend for RecordingWorktreeBackend {
    fn open(&self, path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
        self.opened
            .lock()
            .expect("opened mutex")
            .push(path.to_path_buf());
        Ok(Arc::clone(&self.repo))
    }
}

/// An origin repo that reports every branch as checked out in `worktree`, and
/// the recorder the backend hands out for that worktree.
struct WorktreeRedirectFixture {
    repos: RepositoryRuntimes,
    backend: Arc<dyn GitBackend>,
    origin_calls: Arc<std::sync::Mutex<Vec<String>>>,
    worktree_calls: Arc<std::sync::Mutex<Vec<String>>>,
    opened: Arc<std::sync::Mutex<Vec<PathBuf>>>,
    worktree: PathBuf,
}

fn worktree_redirect_fixture(
    repo_id: RepoId,
    label: &str,
    worktree_branch: &str,
    worktree_is_origin: bool,
) -> WorktreeRedirectFixture {
    let origin_workdir = unique_temp_path(label);
    let worktree = unique_temp_path(&format!("{label}-worktree"));
    let origin_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let worktree_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut origin = RecordingCheckoutRepo::new(
        RepoSpec {
            workdir: origin_workdir.clone(),
        },
        Arc::clone(&origin_calls),
    );
    origin.other_worktree = Some(worktree.clone());
    let mut worktree_repo = RecordingCheckoutRepo::new(
        RepoSpec {
            workdir: if worktree_is_origin {
                origin_workdir
            } else {
                worktree.clone()
            },
        },
        Arc::clone(&worktree_calls),
    );
    worktree_repo.current_branch = Some(worktree_branch.to_string());
    let opened = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(RecordingWorktreeBackend {
        repo: Arc::new(worktree_repo),
        opened: Arc::clone(&opened),
    });
    let mut repos: RepositoryRuntimes = Default::default();
    repos.attach_repository_for_test(repo_id, Arc::new(origin));
    WorktreeRedirectFixture {
        repos,
        backend,
        origin_calls,
        worktree_calls,
        opened,
        worktree,
    }
}

/// Messages before the action's finishing message, and that message.
fn recv_until_action_finished(msg_rx: &std::sync::mpsc::Receiver<Msg>) -> (Vec<Msg>, Msg) {
    let mut others = Vec::new();
    loop {
        let msg = recv_effect_message(msg_rx, Duration::from_secs(2))
            .expect("expected the action to finish");
        if matches!(
            msg,
            Msg::Internal(
                crate::msg::InternalMsg::RepoActionFinished { .. }
                    | crate::msg::InternalMsg::RepoActionFinishedInWorktree { .. }
                    | crate::msg::InternalMsg::BranchAlreadyExists { .. }
            )
        ) {
            return (others, msg);
        }
        others.push(msg);
    }
}

fn run_effect_with_fixture(
    fixture: &mut WorktreeRedirectFixture,
    effect: Effect,
) -> std::sync::mpsc::Receiver<Msg> {
    let executor = super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    schedule_effect_for_test(
        &executor,
        &executor,
        &fixture.backend,
        &mut fixture.repos,
        msg_tx,
        effect,
    );
    // The receiver is unbounded, so all replies can wait until task cleanup
    // and worker teardown have finished.
    executor.join();
    msg_rx
}

mod cancellation;
mod load_scheduling;
mod prepared_patch;
mod repository_operations;
mod retries_errors;
mod reword;
