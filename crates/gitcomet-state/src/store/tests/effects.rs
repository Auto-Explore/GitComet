use super::*;

static MERGETOOL_TRACE_TEST_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

fn write_deterministic_blob(path: &Path, total_bytes: usize) {
    use std::io::Write as _;

    let mut file = std::fs::File::create(path).expect("blob file should be creatable");
    let mut remaining = total_bytes;
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut buf = [0u8; 8192];

    while remaining > 0 {
        for byte in &mut buf {
            state ^= state << 7;
            state ^= state >> 9;
            state = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
            *byte = (state >> 24) as u8;
        }

        let chunk_len = remaining.min(buf.len());
        file.write_all(&buf[..chunk_len])
            .expect("blob chunk should be writable");
        remaining -= chunk_len;
    }
}

fn local_file_url(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.starts_with('/') {
        format!("file://{normalized}")
    } else {
        format!("file:///{normalized}")
    }
}

fn schedule_effect_with_state_for_test(
    executor: &super::executor::TaskExecutor,
    session_persist_executor: &super::executor::TaskExecutor,
    backend: &Arc<dyn GitBackend>,
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: AppState,
    msg_tx: std::sync::mpsc::Sender<Msg>,
    effect: Effect,
) {
    let thread_state = Arc::new(std::sync::RwLock::new(Arc::new(state)));
    super::effects::schedule_effect(
        executor,
        session_persist_executor,
        &thread_state,
        backend,
        repos,
        msg_tx,
        effect,
    );
}

fn schedule_effect_for_test(
    executor: &super::executor::TaskExecutor,
    session_persist_executor: &super::executor::TaskExecutor,
    backend: &Arc<dyn GitBackend>,
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    msg_tx: std::sync::mpsc::Sender<Msg>,
    effect: Effect,
) {
    schedule_effect_with_state_for_test(
        executor,
        session_persist_executor,
        backend,
        repos,
        AppState::default(),
        msg_tx,
        effect,
    );
}

#[test]
fn clone_repo_effect_clones_local_repo_and_emits_finished_and_open_repo() {
    if !super::require_git_shell_for_store_tests() {
        return;
    }
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let base = std::env::temp_dir().join(format!(
        "gitcomet-clone-effect-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let src = base.join("src");
    let dest = base.join("dest");
    let _ = std::fs::create_dir_all(&src);

    run_git(&src, &["init"]);
    run_git(&src, &["config", "user.email", "you@example.com"]);
    run_git(&src, &["config", "user.name", "You"]);
    run_git(&src, &["config", "commit.gpgsign", "false"]);
    std::fs::write(src.join("a.txt"), "one\n").unwrap();
    run_git(&src, &["add", "a.txt"]);
    run_git(
        &src,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CloneRepo {
            url: src.display().to_string(),
            dest: dest.clone(),
            auth: None,
        },
    );

    let start = Instant::now();
    let mut saw_finished_ok = false;
    let mut saw_open_repo = false;
    while start.elapsed() < Duration::from_secs(15) {
        let msg = match msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(m) => m,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                dest: finished_dest,
                result,
                ..
            }) if finished_dest == dest => {
                assert!(result.is_ok(), "clone failed: {result:?}");
                saw_finished_ok = true;
            }
            Msg::OpenRepo(path) if path == dest => {
                saw_open_repo = true;
            }
            _ => {}
        }

        if saw_finished_ok && saw_open_repo {
            break;
        }
    }

    assert!(saw_finished_ok, "did not observe CloneRepoFinished");
    assert!(saw_open_repo, "did not observe OpenRepo after clone");
    assert!(dest.join(".git").exists(), "expected .git at cloned dest");
}

#[test]
fn clone_repo_effect_abort_removes_partially_created_destination() {
    if !super::require_git_shell_for_store_tests() {
        return;
    }
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    const LARGE_BLOB_BYTES: usize = 64 * 1024 * 1024;

    let temp = tempfile::tempdir().expect("tempdir");
    let src = temp.path().join("src");
    let dest = temp.path().join("dest");
    std::fs::create_dir_all(&src).expect("source dir");

    run_git(&src, &["init"]);
    run_git(&src, &["config", "user.email", "you@example.com"]);
    run_git(&src, &["config", "user.name", "You"]);
    run_git(&src, &["config", "commit.gpgsign", "false"]);
    write_deterministic_blob(&src.join("payload.bin"), LARGE_BLOB_BYTES);
    run_git(&src, &["add", "payload.bin"]);
    run_git(
        &src,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx.clone(),
        Effect::CloneRepo {
            url: local_file_url(&src),
            dest: dest.clone(),
            auth: None,
        },
    );

    let start = Instant::now();
    let mut abort_sent = false;
    let mut saw_finished_err = false;
    let mut saw_open_repo = false;

    while start.elapsed() < Duration::from_secs(30) {
        if !abort_sent && dest.exists() {
            schedule_effect_for_test(
                &executor,
                &executor,
                &backend,
                &repos,
                msg_tx.clone(),
                Effect::AbortCloneRepo { dest: dest.clone() },
            );
            abort_sent = true;
        }

        let msg = match msg_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(m) => m,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                dest: finished_dest,
                result,
                ..
            }) if finished_dest == dest => {
                assert!(abort_sent, "clone finished before abort could be sent");
                let err = result.expect_err("aborted clone should not succeed");
                let err_text = err.to_string();
                assert!(
                    err_text.contains("clone aborted"),
                    "unexpected abort error: {err_text}"
                );
                saw_finished_err = true;
                break;
            }
            Msg::OpenRepo(path) if path == dest => {
                saw_open_repo = true;
            }
            _ => {}
        }
    }

    assert!(abort_sent, "did not send abort request");
    assert!(saw_finished_err, "did not observe CloneRepoFinished error");
    assert!(
        !saw_open_repo,
        "aborted clone should not open the repository"
    );
    assert!(
        !dest.exists(),
        "aborted clone should clean up the destination directory"
    );
}

#[test]
fn load_conflict_file_effect_reads_worktree_and_emits_loaded() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        diff: gitcomet_core::domain::FileDiffText,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn diff_file_text(
            &self,
            _target: &DiffTarget,
        ) -> Result<Option<gitcomet_core::domain::FileDiffText>> {
            Ok(Some(self.diff.clone()))
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    let base = std::env::temp_dir().join(format!(
        "gitcomet-conflict-load-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let rel = PathBuf::from("conflict.txt");
    let current = "a\n<<<<<<<\nours\n=======\ntheirs\n>>>>>>>\nb\n";
    std::fs::write(base.join(&rel), current.as_bytes()).unwrap();

    let repo_id = RepoId(1);
    let repo: Arc<dyn GitRepository> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: base.clone(),
        },
        diff: gitcomet_core::domain::FileDiffText::new(
            rel.clone(),
            Some("ours\n".to_string()),
            Some("theirs\n".to_string()),
        ),
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadConflictFile {
            repo_id,
            path: rel.clone(),
            mode: crate::model::ConflictFileLoadMode::CurrentOnly,
        },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = msg_rx.recv_timeout(Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                repo_id: rid,
                path,
                result,
                conflict_session,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert_eq!(path, rel);
            assert!(conflict_session.is_none());
            let file = result.unwrap().unwrap();
            assert_eq!(file.path, PathBuf::from("conflict.txt"));
            assert_eq!(file.base_bytes, None);
            assert_eq!(file.ours_bytes, None);
            assert_eq!(file.theirs_bytes, None);
            assert_eq!(file.current_bytes, None);
            assert_eq!(file.base, None);
            assert_eq!(file.ours, None);
            assert_eq!(file.theirs, None);
            assert_eq!(file.current.as_deref(), Some(current));
            return;
        };
    }
    panic!("timed out waiting for ConflictFileLoaded");
}

#[test]
fn load_conflict_file_effect_reuses_conflict_session_payloads_without_stage_fetch() {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
    use gitcomet_core::domain::FileConflictKind;
    use gitcomet_core::services::ConflictFileStages;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        session: ConflictSession,
        stage_calls: Arc<AtomicUsize>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn conflict_file_stages(&self, _path: &Path) -> Result<Option<ConflictFileStages>> {
            self.stage_calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        fn conflict_session(&self, _path: &Path) -> Result<Option<ConflictSession>> {
            Ok(Some(self.session.clone()))
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    let base = std::env::temp_dir().join(format!(
        "gitcomet-conflict-load-session-reuse-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let rel = PathBuf::from("session_reuse.txt");
    let base_text = "base\n";
    let ours_text = "ours\n";
    let theirs_text = "theirs\n";
    let current_text = "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n";
    let stage_calls = Arc::new(AtomicUsize::new(0));
    let repo_id = RepoId(8);
    let repo: Arc<dyn GitRepository> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: base.clone(),
        },
        session: ConflictSession::from_merged_text(
            rel.clone(),
            FileConflictKind::BothModified,
            ConflictPayload::Text(base_text.to_string().into()),
            ConflictPayload::Text(ours_text.to_string().into()),
            ConflictPayload::Text(theirs_text.to_string().into()),
            current_text,
        ),
        stage_calls: stage_calls.clone(),
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadConflictFile {
            repo_id,
            path: rel.clone(),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = msg_rx.recv_timeout(Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                repo_id: rid,
                path,
                result,
                conflict_session,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert_eq!(path, rel);
            let session = conflict_session.expect("session should be forwarded from backend");
            let file = result.unwrap().unwrap();
            assert_eq!(file.path, rel);
            assert_eq!(file.base.as_deref(), Some(base_text));
            assert_eq!(file.ours.as_deref(), Some(ours_text));
            assert_eq!(file.theirs.as_deref(), Some(theirs_text));
            assert_eq!(file.current.as_deref(), Some(current_text));
            assert_eq!(file.base_bytes, None);
            assert_eq!(file.ours_bytes, None);
            assert_eq!(file.theirs_bytes, None);
            assert_eq!(file.current_bytes, None);
            assert_eq!(stage_calls.load(Ordering::SeqCst), 0);
            assert_eq!(session.current_text(), Some(current_text));
            assert!(
                matches!(&session.base, ConflictPayload::Text(text) if std::sync::Arc::ptr_eq(file.base.as_ref().expect("base text"), text))
            );
            assert!(
                matches!(&session.ours, ConflictPayload::Text(text) if std::sync::Arc::ptr_eq(file.ours.as_ref().expect("ours text"), text))
            );
            assert!(
                matches!(&session.theirs, ConflictPayload::Text(text) if std::sync::Arc::ptr_eq(file.theirs.as_ref().expect("theirs text"), text))
            );
            assert!(
                matches!(
                    session.current.as_ref(),
                    Some(ConflictPayload::Text(text))
                        if std::sync::Arc::ptr_eq(file.current.as_ref().expect("current text"), text)
                ),
                "current text should be forwarded from the session without rereading the worktree"
            );
            return;
        }
    }

    panic!("timed out waiting for ConflictFileLoaded");
}

#[test]
fn load_conflict_file_effect_preserves_binary_payloads_when_reusing_session() {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
    use gitcomet_core::domain::FileConflictKind;
    use gitcomet_core::services::ConflictFileStages;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        session: ConflictSession,
        stage_calls: Arc<AtomicUsize>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn conflict_file_stages(&self, _path: &Path) -> Result<Option<ConflictFileStages>> {
            self.stage_calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        fn conflict_session(&self, _path: &Path) -> Result<Option<ConflictSession>> {
            Ok(Some(self.session.clone()))
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    let base = std::env::temp_dir().join(format!(
        "gitcomet-conflict-load-session-reuse-binary-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let rel = PathBuf::from("session_reuse.bin");
    let base_bytes = vec![0xff, 0x00, 0x01];
    let ours_bytes = vec![0xfe, 0x10, 0x11];
    let theirs_bytes = vec![0xfd, 0x20, 0x21];
    let current_bytes = vec![0xfc, 0x30, 0x31];
    let base_payload: Arc<[u8]> = base_bytes.clone().into();
    let ours_payload: Arc<[u8]> = ours_bytes.clone().into();
    let theirs_payload: Arc<[u8]> = theirs_bytes.clone().into();
    let stage_calls = Arc::new(AtomicUsize::new(0));
    let repo_id = RepoId(9);
    let repo: Arc<dyn GitRepository> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: base.clone(),
        },
        session: ConflictSession::new_with_current(
            rel.clone(),
            FileConflictKind::BothModified,
            ConflictPayload::Binary(base_payload.clone()),
            ConflictPayload::Binary(ours_payload.clone()),
            ConflictPayload::Binary(theirs_payload.clone()),
            ConflictPayload::Binary(current_bytes.clone().into()),
        ),
        stage_calls: stage_calls.clone(),
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadConflictFile {
            repo_id,
            path: rel.clone(),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = msg_rx.recv_timeout(Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                repo_id: rid,
                path,
                result,
                conflict_session,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert_eq!(path, rel);
            let session = conflict_session.expect("session should be forwarded from backend");
            let file = result.unwrap().unwrap();
            assert_eq!(file.path, rel);
            assert_eq!(file.base_bytes.as_deref(), Some(base_bytes.as_slice()));
            assert_eq!(file.ours_bytes.as_deref(), Some(ours_bytes.as_slice()));
            assert_eq!(file.theirs_bytes.as_deref(), Some(theirs_bytes.as_slice()));
            assert_eq!(
                file.current_bytes.as_deref(),
                Some(current_bytes.as_slice())
            );
            assert_eq!(file.base, None);
            assert_eq!(file.ours, None);
            assert_eq!(file.theirs, None);
            assert_eq!(file.current, None);
            assert!(
                Arc::ptr_eq(file.base_bytes.as_ref().expect("base bytes"), &base_payload,),
                "base binary bytes should be forwarded from the session without cloning",
            );
            assert!(
                Arc::ptr_eq(file.ours_bytes.as_ref().expect("ours bytes"), &ours_payload,),
                "ours binary bytes should be forwarded from the session without cloning",
            );
            assert!(
                Arc::ptr_eq(
                    file.theirs_bytes.as_ref().expect("theirs bytes"),
                    &theirs_payload,
                ),
                "theirs binary bytes should be forwarded from the session without cloning",
            );
            assert!(
                matches!(
                    session.current.as_ref(),
                    Some(ConflictPayload::Binary(bytes))
                        if Arc::ptr_eq(file.current_bytes.as_ref().expect("current bytes"), bytes)
                ),
                "current binary bytes should be forwarded from the session without rereading the worktree",
            );
            assert_eq!(stage_calls.load(Ordering::SeqCst), 0);
            return;
        }
    }

    panic!("timed out waiting for ConflictFileLoaded");
}

#[test]
fn load_conflict_file_effect_reuses_absent_current_payload_without_rereading_worktree() {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
    use gitcomet_core::domain::FileConflictKind;
    use gitcomet_core::mergetool_trace::{self, MergetoolTraceStage};
    use gitcomet_core::services::ConflictFileStages;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        session: ConflictSession,
        stage_calls: Arc<AtomicUsize>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn conflict_file_stages(&self, _path: &Path) -> Result<Option<ConflictFileStages>> {
            self.stage_calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        fn conflict_session(&self, _path: &Path) -> Result<Option<ConflictSession>> {
            Ok(Some(self.session.clone()))
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    let _trace_lock = MERGETOOL_TRACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _trace = mergetool_trace::capture();

    let base = std::env::temp_dir().join(format!(
        "gitcomet-conflict-load-session-reuse-absent-current-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let rel = PathBuf::from("removed.txt");
    let base_text = "base\n";
    let stage_calls = Arc::new(AtomicUsize::new(0));
    let repo_id = RepoId(10);
    let repo: Arc<dyn GitRepository> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: base.clone(),
        },
        session: ConflictSession::new_with_current(
            rel.clone(),
            FileConflictKind::BothDeleted,
            ConflictPayload::Text(base_text.into()),
            ConflictPayload::Absent,
            ConflictPayload::Absent,
            ConflictPayload::Absent,
        ),
        stage_calls: stage_calls.clone(),
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadConflictFile {
            repo_id,
            path: rel.clone(),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = msg_rx.recv_timeout(Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                repo_id: rid,
                path,
                result,
                conflict_session,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert_eq!(path, rel);
            let session = conflict_session.expect("session should be forwarded from backend");
            let file = result.unwrap().unwrap();
            assert_eq!(file.path, rel);
            assert_eq!(file.base.as_deref(), Some(base_text));
            assert_eq!(file.ours, None);
            assert_eq!(file.theirs, None);
            assert_eq!(file.current, None);
            assert_eq!(file.base_bytes, None);
            assert_eq!(file.ours_bytes, None);
            assert_eq!(file.theirs_bytes, None);
            assert_eq!(file.current_bytes, None);
            assert_eq!(stage_calls.load(Ordering::SeqCst), 0);
            assert!(matches!(
                session.current.as_ref(),
                Some(ConflictPayload::Absent)
            ));

            let trace = mergetool_trace::snapshot();
            let path_events: Vec<_> = trace
                .events
                .iter()
                .filter(|event| event.path.as_deref() == Some(rel.as_path()))
                .collect();
            assert!(
                path_events
                    .iter()
                    .any(|event| event.stage == MergetoolTraceStage::LoadCurrentReuse),
                "known-absent current payload should reuse the session value instead of rereading the worktree",
            );
            assert!(
                !path_events
                    .iter()
                    .any(|event| event.stage == MergetoolTraceStage::LoadCurrentRead),
                "known-absent current payload should not fall back to a worktree read",
            );
            return;
        }
    }

    panic!("timed out waiting for ConflictFileLoaded");
}

#[test]
fn load_conflict_file_effect_records_trace_stages_and_sizes() {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
    use gitcomet_core::domain::FileConflictKind;
    use gitcomet_core::mergetool_trace::{self, MergetoolTraceStage};
    use gitcomet_core::services::ConflictFileStages;

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        stages: ConflictFileStages,
        session: ConflictSession,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn conflict_file_stages(&self, _path: &Path) -> Result<Option<ConflictFileStages>> {
            Ok(Some(self.stages.clone()))
        }
        fn conflict_session(&self, _path: &Path) -> Result<Option<ConflictSession>> {
            Ok(Some(self.session.clone()))
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    fn trace_line_count(text: &str) -> usize {
        if text.is_empty() {
            0
        } else {
            text.as_bytes()
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count()
                + 1
        }
    }

    let _trace_lock = MERGETOOL_TRACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _trace = mergetool_trace::capture();
    let base = std::env::temp_dir().join(format!(
        "gitcomet-conflict-load-trace-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let rel = PathBuf::from("trace_conflict.html");
    let base_text = "<div>base</div>\n<section>common</section>\n<footer>end</footer>\n";
    let ours_text = "<div>ours</div>\n<section>common</section>\n<footer>end</footer>\n";
    let theirs_text = "<div>theirs</div>\n<section>common</section>\n<footer>end</footer>\n";
    let current_text = [
        "<<<<<<< ours",
        "<div>ours</div>",
        "=======",
        "<div>theirs</div>",
        ">>>>>>> theirs",
        "<section>common</section>",
        "<footer>end</footer>",
        "",
    ]
    .join("\n");
    let repo_id = RepoId(7);
    let repo: Arc<dyn GitRepository> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: base.clone(),
        },
        stages: ConflictFileStages {
            path: rel.clone(),
            base_bytes: Some(base_text.as_bytes().to_vec().into()),
            ours_bytes: Some(ours_text.as_bytes().to_vec().into()),
            theirs_bytes: Some(theirs_text.as_bytes().to_vec().into()),
            base: Some(base_text.to_string().into()),
            ours: Some(ours_text.to_string().into()),
            theirs: Some(theirs_text.to_string().into()),
        },
        session: ConflictSession::from_merged_text(
            rel.clone(),
            FileConflictKind::BothModified,
            ConflictPayload::Text(base_text.to_string().into()),
            ConflictPayload::Text(ours_text.to_string().into()),
            ConflictPayload::Text(theirs_text.to_string().into()),
            &current_text,
        ),
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadConflictFile {
            repo_id,
            path: rel.clone(),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );

    let loaded_file = {
        let start = Instant::now();
        loop {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "timed out waiting for ConflictFileLoaded"
            );
            match msg_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                    repo_id: rid,
                    path,
                    result,
                    conflict_session,
                })) if rid == repo_id && path == rel => {
                    let session = conflict_session.expect("trace test should receive a session");
                    assert_eq!(session.regions.len(), 1);
                    break result.unwrap().unwrap();
                }
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(err) => panic!("channel closed while waiting for conflict load: {err:?}"),
            }
        }
    };

    assert_eq!(loaded_file.path, rel);
    assert_eq!(loaded_file.base.as_deref(), Some(base_text));
    assert_eq!(loaded_file.ours.as_deref(), Some(ours_text));
    assert_eq!(loaded_file.theirs.as_deref(), Some(theirs_text));
    assert_eq!(loaded_file.current.as_deref(), Some(current_text.as_str()));

    let trace = mergetool_trace::snapshot();
    let path_events: Vec<_> = trace
        .events
        .iter()
        .filter(|event| event.path.as_deref() == Some(rel.as_path()))
        .collect();
    assert_eq!(
        path_events.len(),
        3,
        "expected exactly the three load-stage trace events for the synthetic conflict path"
    );

    let session_event = path_events
        .iter()
        .find(|event| event.stage == MergetoolTraceStage::LoadConflictSession)
        .copied()
        .expect("missing conflict-session trace event");
    assert_eq!(session_event.base.bytes, Some(base_text.len()));
    assert_eq!(session_event.ours.lines, Some(trace_line_count(ours_text)));
    assert_eq!(
        session_event.conflict_block_count,
        Some(1),
        "session trace should report the parsed conflict block count"
    );

    let stages_event = path_events
        .iter()
        .find(|event| event.stage == MergetoolTraceStage::LoadConflictFileStages)
        .copied()
        .expect("missing conflict-file-stages trace event");
    assert_eq!(stages_event.base.lines, Some(trace_line_count(base_text)));
    assert_eq!(stages_event.ours.bytes, Some(ours_text.len()));
    assert_eq!(stages_event.theirs.bytes, Some(theirs_text.len()));

    let current_event = path_events
        .iter()
        .find(|event| event.stage == MergetoolTraceStage::LoadCurrentReuse)
        .copied()
        .expect("missing current-reuse trace event");
    assert_eq!(current_event.current.bytes, Some(current_text.len()));
    assert_eq!(
        current_event.current.lines,
        Some(trace_line_count(&current_text))
    );
}

#[test]
fn save_worktree_file_effect_writes_and_can_stage() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        staged: std::sync::Mutex<Vec<PathBuf>>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, paths: &[&Path]) -> Result<()> {
            let mut staged = self.staged.lock().unwrap();
            for p in paths {
                staged.push(p.to_path_buf());
            }
            Ok(())
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    let base = std::env::temp_dir().join(format!(
        "gitcomet-save-worktree-file-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let rel = PathBuf::from("dir/out.txt");
    let contents = "hello\nworld\n";

    let repo_id = RepoId(1);
    let repo: Arc<Repo> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: base.clone(),
        },
        staged: std::sync::Mutex::new(Vec::new()),
    });
    let repo_trait: Arc<dyn GitRepository> = repo.clone();
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo_trait);
        repos
    };

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx.clone(),
        Effect::SaveWorktreeFile {
            repo_id,
            path: rel.clone(),
            contents: contents.to_string(),
            stage: true,
        },
    );

    let mut saw_write_and_stage = false;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = msg_rx.recv_timeout(Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: rid,
                command,
                result,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert!(matches!(
                command,
                crate::msg::RepoCommandKind::SaveWorktreeFile { .. }
            ));
            assert!(result.is_ok());
            let on_disk = std::fs::read_to_string(base.join(&rel)).unwrap();
            assert_eq!(on_disk, contents);
            let staged = repo.staged.lock().unwrap().clone();
            assert_eq!(staged, vec![rel.clone()]);
            saw_write_and_stage = true;
            break;
        };
    }
    assert!(
        saw_write_and_stage,
        "timed out waiting for RepoCommandFinished"
    );

    let escaped_name = format!(
        "gitcomet-save-worktree-file-escape-{}-{}.txt",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let escaped_path = PathBuf::from("..").join(&escaped_name);
    let escaped_dest = base
        .parent()
        .expect("temp dir should have a parent")
        .join(&escaped_name);
    let _ = std::fs::remove_file(&escaped_dest);

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::SaveWorktreeFile {
            repo_id,
            path: escaped_path,
            contents: "escape".to_string(),
            stage: false,
        },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = msg_rx.recv_timeout(Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: rid,
                command,
                result,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert!(matches!(
                command,
                crate::msg::RepoCommandKind::SaveWorktreeFile { .. }
            ));
            let err = result.expect_err("expected traversal write to fail");
            match err.kind() {
                ErrorKind::Backend(message) => {
                    assert!(
                        message.contains("outside repository workdir"),
                        "unexpected error message: {message}"
                    );
                }
                other => panic!("unexpected error kind: {other:?}"),
            }
            assert!(
                !escaped_dest.exists(),
                "unexpected file written outside workdir: {}",
                escaped_dest.display()
            );
            return;
        };
    }
    panic!("timed out waiting for RepoCommandFinished");
}

#[test]
fn checkout_conflict_base_effect_calls_repo_and_emits_finished() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        checkout_base_calls: std::sync::Mutex<Vec<PathBuf>>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }

        fn checkout_conflict_base(&self, path: &Path) -> Result<CommandOutput> {
            self.checkout_base_calls
                .lock()
                .unwrap()
                .push(path.to_path_buf());
            Ok(CommandOutput::empty_success(format!(
                "git checkout :1:{}",
                path.display()
            )))
        }
    }

    let repo_id = RepoId(1);
    let rel = PathBuf::from("conflicted.txt");
    let repo: Arc<Repo> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: std::env::temp_dir(),
        },
        checkout_base_calls: std::sync::Mutex::new(Vec::new()),
    });
    let repo_trait: Arc<dyn GitRepository> = repo.clone();
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo_trait);
        repos
    };

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CheckoutConflictBase {
            repo_id,
            path: rel.clone(),
        },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = msg_rx.recv_timeout(Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: rid,
                command,
                result,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert!(matches!(
                command,
                crate::msg::RepoCommandKind::CheckoutConflictBase { path } if path == rel
            ));
            assert!(result.is_ok());
            assert_eq!(repo.checkout_base_calls.lock().unwrap().as_slice(), [rel]);
            return;
        };
    }
    panic!("timed out waiting for RepoCommandFinished");
}

#[test]
fn accept_conflict_deletion_effect_calls_repo_and_emits_finished() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        accepted_deletion_calls: std::sync::Mutex<Vec<PathBuf>>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }

        fn accept_conflict_deletion(&self, path: &Path) -> Result<CommandOutput> {
            self.accepted_deletion_calls
                .lock()
                .unwrap()
                .push(path.to_path_buf());
            Ok(CommandOutput::empty_success(format!(
                "git rm -- {}",
                path.display()
            )))
        }
    }

    let repo_id = RepoId(1);
    let rel = PathBuf::from("conflicted.txt");
    let repo: Arc<Repo> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: std::env::temp_dir(),
        },
        accepted_deletion_calls: std::sync::Mutex::new(Vec::new()),
    });
    let repo_trait: Arc<dyn GitRepository> = repo.clone();
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo_trait);
        repos
    };

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::AcceptConflictDeletion {
            repo_id,
            path: rel.clone(),
        },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = msg_rx.recv_timeout(Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: rid,
                command,
                result,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert!(matches!(
                command,
                crate::msg::RepoCommandKind::AcceptConflictDeletion { path } if path == rel
            ));
            assert!(result.is_ok());
            assert_eq!(
                repo.accepted_deletion_calls.lock().unwrap().as_slice(),
                [rel]
            );
            return;
        };
    }
    panic!("timed out waiting for RepoCommandFinished");
}

#[test]
fn load_stashes_effect_truncates_results_to_limit() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        stashes: Vec<StashEntry>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            Ok(self.stashes.clone())
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    let base = std::env::temp_dir().join(format!(
        "gitcomet-stash-load-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let stashes = (0..5)
        .map(|i| StashEntry {
            index: i,
            id: CommitId(format!("stash-{i}").into()),
            message: format!("stash message {i}").into(),
            created_at: None,
        })
        .collect::<Vec<_>>();

    let repo_id = RepoId(1);
    let repo: Arc<dyn GitRepository> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: base.clone(),
        },
        stashes,
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadStashes { repo_id, limit: 2 },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(m) => m,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::Internal(crate::msg::InternalMsg::StashesLoaded {
                repo_id: got_repo_id,
                result,
            }) if got_repo_id == repo_id => {
                let entries = result.expect("expected stash list Ok");
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].index, 0);
                assert_eq!(entries[1].index, 1);
                return;
            }
            _ => {}
        }
    }

    panic!("did not observe StashesLoaded");
}

#[test]
fn stash_effect_requests_stash_reload_on_success() {
    use std::sync::Mutex;

    struct RecordingRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitRepository for RecordingRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }

        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }

        fn stash_create(&self, message: &str, include_untracked: bool) -> Result<()> {
            self.calls.lock().unwrap().push(format!(
                "stash {message} include_untracked={include_untracked}"
            ));
            Ok(())
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }

        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<RecordingRepo> = Arc::new(RecordingRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
        calls: Arc::clone(&calls),
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    repos.insert(RepoId(1), repo);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::Stash {
            repo_id: RepoId(1),
            message: "wip".to_string(),
            include_untracked: true,
        },
    );

    let start = Instant::now();
    let mut saw_load_stashes = false;
    let mut saw_finished = false;
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::LoadStashes { repo_id: RepoId(1) } => saw_load_stashes = true,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                result: Ok(()),
            }) => saw_finished = true,
            _ => {}
        }

        if saw_load_stashes && saw_finished {
            break;
        }
    }

    assert!(
        saw_load_stashes,
        "expected stash effect to request stash reload"
    );
    assert!(saw_finished, "expected stash effect to complete");
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["stash wip include_untracked=true".to_string()]
    );
}

#[test]
fn pop_stash_effect_applies_and_drops_then_requests_stash_reload() {
    use std::sync::Mutex;

    struct RecordingRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitRepository for RecordingRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }

        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }

        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("apply {index}"));
            Ok(())
        }
        fn stash_drop(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("drop {index}"));
            Ok(())
        }

        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<RecordingRepo> = Arc::new(RecordingRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
        calls: Arc::clone(&calls),
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    repos.insert(RepoId(1), repo);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::PopStash {
            repo_id: RepoId(1),
            index: 3,
        },
    );

    let start = Instant::now();
    let mut saw_load_stashes = false;
    let mut saw_finished = false;
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::LoadStashes { repo_id: RepoId(1) } => saw_load_stashes = true,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                result: Ok(()),
            }) => saw_finished = true,
            _ => {}
        }

        if saw_load_stashes && saw_finished {
            break;
        }
    }

    assert!(
        saw_load_stashes,
        "expected pop stash effect to request stash reload"
    );
    assert!(saw_finished, "expected pop stash effect to complete");
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["apply 3".to_string(), "drop 3".to_string()]
    );
}

#[test]
fn pop_stash_effect_propagates_apply_error_without_drop_or_reload() {
    use std::sync::Mutex;

    struct FailingApplyRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitRepository for FailingApplyRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }

        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }

        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("apply {index}"));
            Err(Error::new(ErrorKind::Backend("apply failed".to_string())))
        }
        fn stash_drop(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("drop {index}"));
            Ok(())
        }

        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<FailingApplyRepo> = Arc::new(FailingApplyRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
        calls: Arc::clone(&calls),
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    repos.insert(RepoId(1), repo);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::PopStash {
            repo_id: RepoId(1),
            index: 7,
        },
    );

    let start = Instant::now();
    let mut saw_load_stashes = false;
    let mut saw_finished_err = false;
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::LoadStashes { repo_id: RepoId(1) } => saw_load_stashes = true,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                result: Err(_),
            }) => {
                saw_finished_err = true;
                break;
            }
            _ => {}
        }
    }

    assert!(
        !saw_load_stashes,
        "pop stash apply failure should not request stash reload"
    );
    assert!(
        saw_finished_err,
        "expected pop stash effect to emit apply error completion"
    );
    assert_eq!(*calls.lock().unwrap(), vec!["apply 7".to_string()]);
}

#[test]
fn drop_stash_effect_requests_stash_reload_on_success() {
    use std::sync::Mutex;

    struct RecordingRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitRepository for RecordingRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }

        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }

        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("drop {index}"));
            Ok(())
        }

        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<RecordingRepo> = Arc::new(RecordingRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
        calls: Arc::clone(&calls),
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    repos.insert(RepoId(1), repo);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::DropStash {
            repo_id: RepoId(1),
            index: 3,
        },
    );

    let start = Instant::now();
    let mut saw_load_stashes = false;
    let mut saw_finished = false;
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::LoadStashes { repo_id: RepoId(1) } => saw_load_stashes = true,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                result: Ok(()),
            }) => saw_finished = true,
            _ => {}
        }

        if saw_load_stashes && saw_finished {
            break;
        }
    }

    assert!(
        saw_load_stashes,
        "expected drop stash effect to request stash reload"
    );
    assert!(saw_finished, "expected drop stash effect to complete");
    assert_eq!(*calls.lock().unwrap(), vec!["drop 3".to_string()]);
}

#[test]
fn drop_stash_effect_requests_stash_reload_on_error() {
    use std::sync::Mutex;

    struct FailingRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitRepository for FailingRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }

        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn revert(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }

        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("drop {index}"));
            Err(Error::new(ErrorKind::Backend("drop failed".to_string())))
        }

        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<FailingRepo> = Arc::new(FailingRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
        calls: Arc::clone(&calls),
    });

    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    repos.insert(RepoId(1), repo);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::DropStash {
            repo_id: RepoId(1),
            index: 4,
        },
    );

    let start = Instant::now();
    let mut saw_load_stashes = false;
    let mut saw_finished_err = false;
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::LoadStashes { repo_id: RepoId(1) } => saw_load_stashes = true,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                result: Err(_),
            }) => {
                saw_finished_err = true;
                break;
            }
            _ => {}
        }
    }

    assert!(
        saw_load_stashes,
        "drop stash failure should still request stash reload"
    );
    assert!(
        saw_finished_err,
        "expected drop stash effect to emit error completion"
    );
    assert_eq!(*calls.lock().unwrap(), vec!["drop 4".to_string()]);
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
}

impl GitRepository for UnsupportedRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
        unsupported_repo_result()
    }
    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unsupported_repo_result()
    }
    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unsupported_repo_result()
    }
    fn current_branch(&self) -> Result<String> {
        unsupported_repo_result()
    }
    fn list_branches(&self) -> Result<Vec<Branch>> {
        unsupported_repo_result()
    }
    fn list_remotes(&self) -> Result<Vec<Remote>> {
        unsupported_repo_result()
    }
    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        unsupported_repo_result()
    }
    fn status(&self) -> Result<RepoStatus> {
        unsupported_repo_result()
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
    fn revert(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        unsupported_repo_result()
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
    fn fetch_all(&self) -> Result<()> {
        unsupported_repo_result()
    }
    fn pull(&self, _mode: PullMode) -> Result<()> {
        unsupported_repo_result()
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

struct RecordingCheckoutRepo {
    spec: RepoSpec,
    calls: Arc<std::sync::Mutex<Vec<String>>>,
}

impl GitRepository for RecordingCheckoutRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
        unsupported_repo_result()
    }
    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unsupported_repo_result()
    }
    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unsupported_repo_result()
    }
    fn current_branch(&self) -> Result<String> {
        unsupported_repo_result()
    }
    fn list_branches(&self) -> Result<Vec<Branch>> {
        unsupported_repo_result()
    }
    fn list_remotes(&self) -> Result<Vec<Remote>> {
        unsupported_repo_result()
    }
    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        unsupported_repo_result()
    }
    fn status(&self) -> Result<RepoStatus> {
        unsupported_repo_result()
    }
    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unsupported_repo_result()
    }

    fn create_branch(&self, name: &str, target: &CommitId) -> Result<()> {
        self.calls
            .lock()
            .expect("checkout recording mutex")
            .push(format!("create {name} {}", target.as_ref()));
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
    fn checkout_remote_branch(&self, remote: &str, branch: &str, local_branch: &str) -> Result<()> {
        self.calls
            .lock()
            .expect("checkout recording mutex")
            .push(format!(
                "checkout_remote {remote}/{branch} -> {local_branch}"
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
    fn revert(&self, _id: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        unsupported_repo_result()
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
    fn fetch_all(&self) -> Result<()> {
        unsupported_repo_result()
    }
    fn pull(&self, _mode: PullMode) -> Result<()> {
        unsupported_repo_result()
    }
    fn push(&self) -> Result<()> {
        unsupported_repo_result()
    }
    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
}

fn wait_for_checkout_refresh_messages(
    msg_rx: &std::sync::mpsc::Receiver<Msg>,
    repo_id: RepoId,
    expect_refresh_branches: bool,
    expect_load_worktrees: bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_refresh_branches = false;
    let mut saw_load_worktrees = false;
    let mut saw_finished = false;

    while Instant::now() < deadline {
        let msg = match msg_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(err) => panic!("channel closed: {err:?}"),
        };

        match msg {
            Msg::RefreshBranches { repo_id: rid } if rid == repo_id => {
                saw_refresh_branches = true;
            }
            Msg::LoadWorktrees { repo_id: rid } if rid == repo_id => {
                saw_load_worktrees = true;
            }
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: rid,
                result: Ok(()),
            }) if rid == repo_id => {
                saw_finished = true;
            }
            _ => {}
        }

        if saw_finished
            && saw_refresh_branches == expect_refresh_branches
            && saw_load_worktrees == expect_load_worktrees
        {
            return;
        }
    }

    assert_eq!(
        saw_refresh_branches, expect_refresh_branches,
        "unexpected RefreshBranches emission for repo {repo_id:?}"
    );
    assert_eq!(
        saw_load_worktrees, expect_load_worktrees,
        "unexpected LoadWorktrees emission for repo {repo_id:?}"
    );
    assert!(
        saw_finished,
        "expected RepoActionFinished for repo {repo_id:?}"
    );
}

#[test]
fn checkout_branch_effect_requests_branch_and_worktree_reload_on_success() {
    let repo_id = RepoId(700);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingCheckoutRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-checkout-branch-effect"),
        },
        calls: Arc::clone(&calls),
    });
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CheckoutBranch {
            repo_id,
            name: "feature".to_string(),
        },
    );

    wait_for_checkout_refresh_messages(&msg_rx, repo_id, true, true);
    assert_eq!(
        *calls.lock().expect("checkout recording mutex"),
        vec!["checkout feature".to_string()]
    );
}

#[test]
fn checkout_remote_branch_effect_requests_branch_and_worktree_reload_on_success() {
    let repo_id = RepoId(701);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingCheckoutRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-checkout-remote-branch-effect"),
        },
        calls: Arc::clone(&calls),
    });
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CheckoutRemoteBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature".to_string(),
            local_branch: "feature".to_string(),
        },
    );

    wait_for_checkout_refresh_messages(&msg_rx, repo_id, true, true);
    assert_eq!(
        *calls.lock().expect("checkout recording mutex"),
        vec!["checkout_remote origin/feature -> feature".to_string()]
    );
}

#[test]
fn checkout_commit_effect_requests_worktree_reload_on_success() {
    let repo_id = RepoId(702);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingCheckoutRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-checkout-commit-effect"),
        },
        calls: Arc::clone(&calls),
    });
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    let commit_id = CommitId("deadbeef".into());

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CheckoutCommit {
            repo_id,
            commit_id: commit_id.clone(),
        },
    );

    wait_for_checkout_refresh_messages(&msg_rx, repo_id, false, true);
    assert_eq!(
        *calls.lock().expect("checkout recording mutex"),
        vec![format!("checkout_commit {}", commit_id.as_ref())]
    );
}

#[test]
fn create_branch_and_checkout_effect_requests_branch_and_worktree_reload_on_success() {
    let repo_id = RepoId(703);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingCheckoutRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-create-branch-and-checkout-effect"),
        },
        calls: Arc::clone(&calls),
    });
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CreateBranchAndCheckout {
            repo_id,
            name: "feature".to_string(),
            target: "HEAD".to_string(),
        },
    );

    wait_for_checkout_refresh_messages(&msg_rx, repo_id, true, true);
    assert_eq!(
        *calls.lock().expect("checkout recording mutex"),
        vec![
            "create feature HEAD".to_string(),
            "checkout feature".to_string()
        ]
    );
}

fn recv_n_msgs(msg_rx: &std::sync::mpsc::Receiver<Msg>, n: usize) {
    for _ in 0..n {
        msg_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("expected effect scheduler message");
    }
}

#[test]
fn open_repo_effect_emits_repo_opened_ok() {
    struct Backend {
        repo: Arc<dyn GitRepository>,
    }
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Ok(Arc::clone(&self.repo))
        }
    }

    let repo_id = RepoId(42);
    let workdir = unique_temp_path("gitcomet-open-repo-ok");
    let repo: Arc<dyn GitRepository> = Arc::new(UnsupportedRepo {
        spec: RepoSpec {
            workdir: workdir.clone(),
        },
    });
    let backend: Arc<dyn GitBackend> = Arc::new(Backend { repo });

    let executor = super::executor::TaskExecutor::new(1);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::OpenRepo {
            repo_id,
            path: workdir.clone(),
        },
    );

    let msg = msg_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("expected RepoOpenedOk");
    match msg {
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: got_repo_id,
            spec,
            repo,
        }) => {
            assert_eq!(got_repo_id, repo_id);
            assert_eq!(spec.workdir, workdir);
            assert_eq!(repo.spec().workdir, workdir);
        }
        _ => panic!("expected RepoOpenedOk"),
    }
}

#[test]
fn open_repo_effect_emits_repo_opened_err() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Backend(
                "backend open failed".to_string(),
            )))
        }
    }

    let repo_id = RepoId(43);
    let workdir = unique_temp_path("gitcomet-open-repo-err");
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let executor = super::executor::TaskExecutor::new(1);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::OpenRepo {
            repo_id,
            path: workdir.clone(),
        },
    );

    let msg = msg_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("expected RepoOpenedErr");
    match msg {
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: got_repo_id,
            spec,
            error,
        }) => {
            assert_eq!(got_repo_id, repo_id);
            assert_eq!(spec.workdir, workdir);
            assert!(matches!(error.kind(), ErrorKind::Backend(_)));
        }
        _ => panic!("expected RepoOpenedErr"),
    }
}

#[test]
fn worktree_and_submodule_effects_report_missing_repo_handle() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            panic!("open should not be called in this test")
        }
    }

    let repo_id = RepoId(77);
    let executor = super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx.clone(),
        Effect::LoadWorktrees { repo_id },
    );
    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadSubmodules { repo_id },
    );

    let first = msg_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected WorktreesLoaded");
    let second = msg_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected SubmodulesLoaded");

    match first {
        Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
            repo_id: got_repo_id,
            result: Err(error),
        }) => {
            assert_eq!(got_repo_id, repo_id);
            assert!(
                matches!(error.kind(), ErrorKind::Backend(message) if message.contains("Repository handle not found"))
            );
        }
        _ => panic!("expected WorktreesLoaded missing-handle error"),
    }

    match second {
        Msg::Internal(crate::msg::InternalMsg::SubmodulesLoaded {
            repo_id: got_repo_id,
            result: Err(error),
        }) => {
            assert_eq!(got_repo_id, repo_id);
            assert!(
                matches!(error.kind(), ErrorKind::Backend(message) if message.contains("Repository handle not found"))
            );
        }
        _ => panic!("expected SubmodulesLoaded missing-handle error"),
    }
}

#[test]
fn schedule_effect_dispatches_many_variants_with_repo_present() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            panic!("open should not be called in this test")
        }
    }

    let repo_id = RepoId(500);
    let workdir = unique_temp_path("gitcomet-effects-dispatch");
    std::fs::create_dir_all(&workdir).expect("create workdir");

    let repo: Arc<dyn GitRepository> = Arc::new(UnsupportedRepo {
        spec: RepoSpec {
            workdir: workdir.clone(),
        },
    });
    let repos: HashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = HashMap::default();
        repos.insert(repo_id, repo);
        repos
    };

    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let executor = super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("tracked.txt"),
        area: DiffArea::Unstaged,
    };
    let mut state = AppState::default();
    let mut repo_state = crate::model::RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: workdir.clone(),
        },
    );
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.conflict_state.conflict_file_path = Some(PathBuf::from("conflicted.txt"));
    state.active_repo = Some(repo_id);
    state.repos.push(repo_state);
    let commit_id = CommitId("deadbeef".into());
    let effect_specs: Vec<(Effect, usize)> = vec![
        (Effect::LoadBranches { repo_id }, 1),
        (Effect::LoadRemotes { repo_id }, 1),
        (Effect::LoadRemoteBranches { repo_id }, 1),
        (Effect::LoadStatus { repo_id }, 1),
        (Effect::LoadHeadBranch { repo_id }, 1),
        (Effect::LoadUpstreamDivergence { repo_id }, 1),
        (
            Effect::LoadLog {
                repo_id,
                scope: LogScope::CurrentBranch,
                limit: 20,
                cursor: None,
            },
            1,
        ),
        (
            Effect::LoadLog {
                repo_id,
                scope: LogScope::AllBranches,
                limit: 20,
                cursor: Some(LogCursor {
                    last_seen: CommitId("cursor".into()),
                    resume_from: None,
                }),
            },
            1,
        ),
        (Effect::LoadTags { repo_id }, 1),
        (Effect::LoadRemoteTags { repo_id }, 1),
        (Effect::LoadStashes { repo_id, limit: 3 }, 1),
        (Effect::LoadReflog { repo_id, limit: 5 }, 1),
        (
            Effect::LoadFileHistory {
                repo_id,
                path: PathBuf::from("tracked.txt"),
                limit: 10,
            },
            1,
        ),
        (
            Effect::LoadBlame {
                repo_id,
                path: PathBuf::from("tracked.txt"),
                rev: Some("HEAD".to_string()),
            },
            1,
        ),
        (Effect::LoadWorktrees { repo_id }, 1),
        (Effect::LoadSubmodules { repo_id }, 1),
        (Effect::LoadRebaseAndMergeState { repo_id }, 2),
        (Effect::LoadRebaseState { repo_id }, 1),
        (Effect::LoadMergeCommitMessage { repo_id }, 1),
        (
            Effect::LoadCommitDetails {
                repo_id,
                commit_id: commit_id.clone(),
            },
            1,
        ),
        (
            Effect::LoadDiff {
                repo_id,
                target: target.clone(),
            },
            1,
        ),
        (
            Effect::LoadDiffFile {
                repo_id,
                target: target.clone(),
            },
            1,
        ),
        (
            Effect::LoadDiffFileImage {
                repo_id,
                target: target.clone(),
            },
            1,
        ),
        (
            Effect::LoadSelectedDiff {
                repo_id,
                load_patch_diff: true,
                load_file_text: true,
                load_file_image: false,
                preview_text_side: None,
            },
            2,
        ),
        (
            Effect::LoadConflictFile {
                repo_id,
                path: PathBuf::from("conflicted.txt"),
                mode: crate::model::ConflictFileLoadMode::CurrentOnly,
            },
            1,
        ),
        (
            Effect::LoadSelectedConflictFile {
                repo_id,
                mode: crate::model::ConflictFileLoadMode::CurrentOnly,
            },
            1,
        ),
        (
            Effect::SaveWorktreeFile {
                repo_id,
                path: PathBuf::from("nested/new.txt"),
                contents: "content".to_string(),
                stage: true,
            },
            1,
        ),
        (
            Effect::CheckoutBranch {
                repo_id,
                name: "main".to_string(),
            },
            1,
        ),
        (
            Effect::CheckoutRemoteBranch {
                repo_id,
                remote: "origin".to_string(),
                branch: "main".to_string(),
                local_branch: "main".to_string(),
            },
            1,
        ),
        (
            Effect::CheckoutCommit {
                repo_id,
                commit_id: commit_id.clone(),
            },
            1,
        ),
        (
            Effect::CherryPickCommit {
                repo_id,
                commit_id: commit_id.clone(),
            },
            1,
        ),
        (
            Effect::RevertCommit {
                repo_id,
                commit_id: commit_id.clone(),
            },
            1,
        ),
        (
            Effect::CreateBranch {
                repo_id,
                name: "topic".to_string(),
                target: "HEAD".to_string(),
            },
            1,
        ),
        (
            Effect::CreateBranchAndCheckout {
                repo_id,
                name: "topic2".to_string(),
                target: "HEAD".to_string(),
            },
            1,
        ),
        (
            Effect::DeleteBranch {
                repo_id,
                name: "topic".to_string(),
            },
            1,
        ),
        (
            Effect::ForceDeleteBranch {
                repo_id,
                name: "topic".to_string(),
            },
            1,
        ),
        (
            Effect::ExportPatch {
                repo_id,
                commit_id: commit_id.clone(),
                dest: PathBuf::from("out.patch"),
            },
            1,
        ),
        (
            Effect::ApplyPatch {
                repo_id,
                patch: PathBuf::from("change.patch"),
            },
            1,
        ),
        (
            Effect::AddWorktree {
                repo_id,
                path: PathBuf::from("wt"),
                reference: Some("main".to_string()),
            },
            1,
        ),
        (
            Effect::RemoveWorktree {
                repo_id,
                path: PathBuf::from("wt"),
            },
            1,
        ),
        (
            Effect::AddSubmodule {
                repo_id,
                url: "https://example.com/repo.git".to_string(),
                path: PathBuf::from("sub"),
                branch: None,
                name: None,
                force: false,
                approved_sources: Vec::new(),
                auth: None,
            },
            1,
        ),
        (
            Effect::UpdateSubmodules {
                approved_sources: Vec::new(),
                repo_id,
                auth: None,
            },
            1,
        ),
        (
            Effect::RemoveSubmodule {
                repo_id,
                path: PathBuf::from("sub"),
            },
            1,
        ),
        (
            Effect::StageHunk {
                repo_id,
                patch: "@@ -1 +1 @@".to_string(),
            },
            1,
        ),
        (
            Effect::UnstageHunk {
                repo_id,
                patch: "@@ -1 +1 @@".to_string(),
            },
            1,
        ),
        (
            Effect::ApplyWorktreePatch {
                repo_id,
                patch: "@@ -1 +1 @@".to_string(),
                reverse: true,
            },
            1,
        ),
        (
            Effect::StagePath {
                repo_id,
                path: PathBuf::from("tracked.txt"),
            },
            1,
        ),
        (
            Effect::StagePaths {
                repo_id,
                paths: vec![PathBuf::from("b.txt"), PathBuf::from("a.txt")].into(),
            },
            1,
        ),
        (
            Effect::UnstagePath {
                repo_id,
                path: PathBuf::from("tracked.txt"),
            },
            1,
        ),
        (
            Effect::UnstagePaths {
                repo_id,
                paths: vec![PathBuf::from("b.txt"), PathBuf::from("a.txt")].into(),
            },
            1,
        ),
        (
            Effect::DiscardWorktreeChangesPath {
                repo_id,
                path: PathBuf::from("tracked.txt"),
            },
            1,
        ),
        (
            Effect::DiscardWorktreeChangesPaths {
                repo_id,
                paths: vec![PathBuf::from("b.txt"), PathBuf::from("a.txt")],
            },
            1,
        ),
        (
            Effect::Commit {
                repo_id,
                message: "msg".to_string(),
                auth: None,
            },
            1,
        ),
        (
            Effect::CommitAmend {
                repo_id,
                message: "msg".to_string(),
                auth: None,
            },
            1,
        ),
        (
            Effect::FetchAll {
                repo_id,
                prune: true,
                auth: None,
            },
            1,
        ),
        (Effect::PruneMergedBranches { repo_id }, 1),
        (Effect::PruneLocalTags { repo_id }, 1),
        (
            Effect::Pull {
                repo_id,
                mode: PullMode::FastForwardOnly,
                auth: None,
            },
            1,
        ),
        (
            Effect::PullBranch {
                repo_id,
                remote: "origin".to_string(),
                branch: "main".to_string(),
                auth: None,
            },
            1,
        ),
        (
            Effect::MergeRef {
                repo_id,
                reference: "origin/main".to_string(),
            },
            1,
        ),
        (
            Effect::SquashRef {
                repo_id,
                reference: "origin/main".to_string(),
            },
            1,
        ),
        (
            Effect::Push {
                repo_id,
                auth: None,
            },
            1,
        ),
        (
            Effect::ForcePush {
                repo_id,
                auth: None,
            },
            1,
        ),
        (
            Effect::PushSetUpstream {
                repo_id,
                remote: "origin".to_string(),
                branch: "main".to_string(),
                auth: None,
            },
            1,
        ),
        (
            Effect::SetUpstreamBranch {
                repo_id,
                branch: "main".to_string(),
                upstream: "origin/main".to_string(),
            },
            1,
        ),
        (
            Effect::UnsetUpstreamBranch {
                repo_id,
                branch: "main".to_string(),
            },
            1,
        ),
        (
            Effect::DeleteRemoteBranch {
                repo_id,
                remote: "origin".to_string(),
                branch: "main".to_string(),
                auth: None,
            },
            1,
        ),
        (
            Effect::Reset {
                repo_id,
                target: "HEAD~1".to_string(),
                mode: gitcomet_core::services::ResetMode::Mixed,
            },
            1,
        ),
        (
            Effect::Rebase {
                repo_id,
                onto: "main".to_string(),
            },
            1,
        ),
        (Effect::RebaseContinue { repo_id }, 1),
        (Effect::RebaseAbort { repo_id }, 1),
        (Effect::MergeAbort { repo_id }, 1),
        (
            Effect::CreateTag {
                repo_id,
                name: "v1.0.0".to_string(),
                target: "HEAD".to_string(),
            },
            1,
        ),
        (
            Effect::DeleteTag {
                repo_id,
                name: "v1.0.0".to_string(),
            },
            1,
        ),
        (
            Effect::PushTag {
                repo_id,
                remote: "origin".to_string(),
                name: "v1.0.0".to_string(),
                auth: None,
            },
            1,
        ),
        (
            Effect::DeleteRemoteTag {
                repo_id,
                remote: "origin".to_string(),
                name: "v1.0.0".to_string(),
                auth: None,
            },
            1,
        ),
        (
            Effect::AddRemote {
                repo_id,
                name: "origin".to_string(),
                url: "https://example.com/repo.git".to_string(),
            },
            1,
        ),
        (
            Effect::RemoveRemote {
                repo_id,
                name: "origin".to_string(),
            },
            1,
        ),
        (
            Effect::SetRemoteUrl {
                repo_id,
                name: "origin".to_string(),
                url: "https://example.com/repo.git".to_string(),
                kind: gitcomet_core::services::RemoteUrlKind::Fetch,
            },
            1,
        ),
        (
            Effect::CheckoutConflictSide {
                repo_id,
                path: PathBuf::from("conflicted.txt"),
                side: gitcomet_core::services::ConflictSide::Ours,
            },
            1,
        ),
        (
            Effect::AcceptConflictDeletion {
                repo_id,
                path: PathBuf::from("conflicted.txt"),
            },
            1,
        ),
        (
            Effect::CheckoutConflictBase {
                repo_id,
                path: PathBuf::from("conflicted.txt"),
            },
            1,
        ),
        (
            Effect::LaunchMergetool {
                repo_id,
                path: PathBuf::from("conflicted.txt"),
            },
            1,
        ),
        (
            Effect::Stash {
                repo_id,
                message: "wip".to_string(),
                include_untracked: false,
            },
            1,
        ),
        (Effect::ApplyStash { repo_id, index: 0 }, 1),
        (Effect::PopStash { repo_id, index: 0 }, 1),
        (Effect::DropStash { repo_id, index: 0 }, 2),
    ];

    for (effect, expected_messages) in effect_specs {
        schedule_effect_with_state_for_test(
            &executor,
            &executor,
            &backend,
            &repos,
            state.clone(),
            msg_tx.clone(),
            effect,
        );
        recv_n_msgs(&msg_rx, expected_messages);
    }
}
