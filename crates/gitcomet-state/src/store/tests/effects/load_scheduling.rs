//! Load scheduling: executors, streaming, and conflict-file loads.

use super::*;

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
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
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

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
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
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
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

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
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
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
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

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
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
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
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

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
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
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
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

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
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
        delete_branch_calls: None,
        cancel_delete_branch: None,
    });
    let backend: Arc<dyn GitBackend> = Arc::new(Backend { repo });

    let executor = super::super::executor::TaskExecutor::new(1);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
            preferences: Some(preferences),
            repo_id: got_repo_id,
            spec,
            repo,
        }) => {
            assert_eq!(
                preferences.key,
                crate::model::RepositoryKey::Worktree(workdir.clone())
            );
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
    let executor = super::super::executor::TaskExecutor::new(1);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
fn open_repo_effects_are_bounded_by_repo_load_executor() {
    struct Backend {
        started_tx: std::sync::mpsc::Sender<PathBuf>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl GitBackend for Backend {
        fn open(&self, path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            let workdir = path.to_path_buf();
            let _ = self.started_tx.send(workdir);
            wait_for_release_signal(&self.release);
            Err(Error::new(ErrorKind::Backend(
                "open released by test".to_string(),
            )))
        }
    }

    let repo_a = RepoId(45);
    let repo_b = RepoId(46);
    let workdir_a = unique_temp_path("gitcomet-open-repo-bounded-a");
    let workdir_b = unique_temp_path("gitcomet-open-repo-bounded-b");
    let (started_tx, started_rx) = std::sync::mpsc::channel::<PathBuf>();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let _release_guard = BlockingReleaseGuard {
        release: Arc::clone(&release),
    };
    let backend: Arc<dyn GitBackend> = Arc::new(Backend {
        started_tx,
        release: Arc::clone(&release),
    });

    let executor = super::super::executor::TaskExecutor::new(1);
    let repo_load_executor = super::super::executor::TaskExecutor::new(1);
    let metadata_executor = super::super::executor::TaskExecutor::new(1);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();
    let msg_tx = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);
    let thread_state = Arc::new(std::sync::RwLock::new(Arc::new(AppState::test_default())));
    let mut repo_task_tokens = FxHashMap::default();
    let executors = super::super::effects::EffectExecutors {
        executor: &executor,
        repo_load_executor: &repo_load_executor,
        worktree_scan_executor: &std::sync::LazyLock::new(|| {
            super::super::executor::TaskExecutor::new(1)
        }),
        session_persist_executor: &executor,
        metadata_executor: &metadata_executor,
        signature_executor: &metadata_executor,
        history_find_executor: &std::sync::LazyLock::new(|| {
            super::super::executor::TaskExecutor::new(1)
        }),
    };

    super::super::effects::schedule_effect(
        executors,
        &thread_state,
        &backend,
        &repos,
        &mut repo_task_tokens,
        msg_tx.clone(),
        Effect::OpenRepo {
            repo_id: repo_a,
            path: workdir_a.clone(),
        },
    );
    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first open did not start"),
        workdir_a
    );

    super::super::effects::schedule_effect(
        executors,
        &thread_state,
        &backend,
        &repos,
        &mut repo_task_tokens,
        msg_tx,
        Effect::OpenRepo {
            repo_id: repo_b,
            path: workdir_b.clone(),
        },
    );
    assert!(
        started_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "second open should wait for the single repo-load worker"
    );

    {
        let (lock, condvar) = &*release;
        let mut released = lock.lock().expect("release mutex");
        *released = true;
        condvar.notify_all();
    }
    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second open did not start after worker was released"),
        workdir_b
    );
}

#[test]
fn pr530_slow_repository_loads_do_not_block_another_window() {
    struct Backend {
        started: std::sync::mpsc::Sender<PathBuf>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }
    impl GitBackend for Backend {
        fn open(&self, path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            self.started.send(path.to_path_buf()).unwrap();
            wait_for_release_signal(&self.release);
            Err(Error::new(ErrorKind::NotARepository))
        }
    }
    let (started, received) = std::sync::mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let _release = BlockingReleaseGuard {
        release: Arc::clone(&release),
    };
    let backend: Arc<dyn GitBackend> = Arc::new(Backend { started, release });
    let (first, _events) = AppStore::new_test(Arc::clone(&backend));
    for index in 0..super::super::executor::repo_load_worker_threads() {
        first.dispatch(Msg::OpenRepo(unique_temp_path(&format!(
            "pr530-busy-{index}"
        ))));
        received
            .recv_timeout(Duration::from_secs(3))
            .expect("first window load started");
    }
    let (second, _events) = AppStore::new_test(backend);
    let path = unique_temp_path("pr530-independent-window");
    second.dispatch(Msg::OpenRepo(path.clone()));
    assert_eq!(
        received
            .recv_timeout(Duration::from_secs(3))
            .expect("the second window must load while the first window's workers are busy"),
        path
    );
}

#[test]
fn load_log_effect_uses_history_mode_api() {
    let repo_id = RepoId(498);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let cursor = LogCursor {
        last_seen: CommitId("cursor".into()),
        resume_from: None,
        resume_token: None,
    };
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingLogRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-load-log-history-mode-effect"),
        },
        calls: Arc::clone(&calls),
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadLog {
            repo_id,
            seq: 1,
            scope: LogScope::NoMerges,
            author: None,
            limit: 20,
            cursor: Some(cursor.clone()),
        },
    );

    let msg = msg_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("expected LogLoaded");
    match msg {
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: got_repo_id,
            seq,
            scope,
            cursor: got_cursor,
            result: Ok(gitcomet_core::services::HistoryReadResult::Page { page, .. }),
        }) => {
            assert_eq!(got_repo_id, repo_id);
            assert_eq!(seq, 1, "the reply carries the sequence of its request");
            assert_eq!(scope, LogScope::NoMerges);
            assert_eq!(got_cursor, Some(cursor));
            assert!(page.commits.is_empty());
            assert!(page.next_cursor.is_none());
        }
        _ => panic!("expected LogLoaded"),
    }

    assert_eq!(
        *calls.lock().expect("log recording mutex"),
        vec![
            "filtered None".to_string(),
            "history NoMerges 20 cursor".to_string()
        ]
    );
}

#[test]
fn log_effect_streams_only_while_replacing_a_loading_page() {
    for loading in [false, true] {
        let repo_id = RepoId(498);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
        let spec = RepoSpec {
            workdir: unique_temp_path("gitcomet-log-streaming"),
        };
        let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
        repos.insert(
            repo_id,
            Arc::new(RecordingLogRepo {
                spec: spec.clone(),
                calls: Arc::clone(&calls),
            }),
        );
        let mut state = AppState::test_default();
        let mut repo = RepoState::new_opening(repo_id, spec);
        repo.set_log(if loading {
            Loadable::Loading
        } else {
            Loadable::Ready(Arc::new(LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }))
        });
        state.repos.push(repo);
        let executor = super::super::executor::TaskExecutor::new(1);
        let (tx, rx) = std::sync::mpsc::channel();
        schedule_effect_with_state_for_test(
            &executor,
            &executor,
            &backend,
            &repos,
            state,
            tx,
            Effect::LoadLog {
                repo_id,
                seq: 1,
                scope: LogScope::NoMerges,
                author: Some("alice".into()),
                limit: 800,
                cursor: None,
            },
        );
        let mut chunks = 0;
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Msg::Internal(crate::msg::InternalMsg::LogChunkLoaded { .. }) => chunks += 1,
                Msg::Internal(crate::msg::InternalMsg::LogLoaded { result, .. }) => {
                    result.unwrap();
                    break;
                }
                other => panic!("unexpected message {other:?}"),
            }
        }
        assert_eq!(chunks, usize::from(loading));
        assert!(calls.lock().unwrap().contains(&format!(
            "{} Some(\"alice\")",
            if loading { "stream" } else { "filtered" }
        )));
    }
}

#[test]
fn activation_load_effect_is_not_blocked_by_main_executor_queue() {
    let repo_id = RepoId(499);
    let repo: Arc<dyn GitRepository> = Arc::new(UnsupportedRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-foreground-load-effect"),
        },
        delete_branch_calls: None,
        cancel_delete_branch: None,
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let executor = super::super::executor::TaskExecutor::new(1);
    let (block_started_tx, block_started_rx) = std::sync::mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let release_task = Arc::clone(&release);
    executor.spawn(move || {
        block_started_tx.send(()).expect("send block started");
        let (lock, condvar) = &*release_task;
        let mut released = lock.lock().expect("release mutex");
        while !*released {
            released = condvar.wait(released).expect("release wait");
        }
    });
    block_started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("executor blocker started");

    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadStatus { repo_id },
    );

    let msg = msg_rx.recv_timeout(Duration::from_secs(1));
    {
        let (lock, condvar) = &*release;
        let mut released = lock.lock().expect("release mutex");
        *released = true;
        condvar.notify_all();
    }

    match msg.expect("foreground load should not wait behind queued executor work") {
        Msg::Internal(crate::msg::InternalMsg::StatusLoaded {
            repo_id: got_repo_id,
            ..
        }) => assert_eq!(got_repo_id, repo_id),
        other => panic!("expected status load result, got {other:?}"),
    }
}

#[test]
fn slow_large_file_and_remote_tag_loads_do_not_block_other_repo_metadata() {
    for (load, expected) in [
        (
            Effect::LoadRemoteTags {
                repo_id: RepoId(510),
            },
            "remote_tags",
        ),
        (
            Effect::LoadLfsLocks {
                repo_id: RepoId(510),
            },
            "lfs_locks",
        ),
        (
            Effect::LoadAnnexUnused {
                repo_id: RepoId(510),
            },
            "annex_unused",
        ),
    ] {
        let repo_a = RepoId(510);
        let repo_b = RepoId(511);
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let _release_guard = BlockingReleaseGuard {
            release: Arc::clone(&release),
        };
        let (started_tx, started_rx) = std::sync::mpsc::channel::<&'static str>();
        let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
            let mut repos = FxHashMap::default();
            repos.insert(
                repo_a,
                Arc::new(MetadataSchedulingRepo {
                    spec: RepoSpec {
                        workdir: unique_temp_path("gitcomet-metadata-blocking-remote-tags"),
                    },
                    mode: MetadataRepoMode::BlockingRemoteTags,
                    started_tx: started_tx.clone(),
                    release: Arc::clone(&release),
                }) as Arc<dyn GitRepository>,
            );
            repos.insert(
                repo_b,
                Arc::new(MetadataSchedulingRepo {
                    spec: RepoSpec {
                        workdir: unique_temp_path("gitcomet-metadata-ready-tags"),
                    },
                    mode: MetadataRepoMode::ReadyTags,
                    started_tx,
                    release: Arc::clone(&release),
                }) as Arc<dyn GitRepository>,
            );
            repos
        };
        let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
        let executor = super::executor::TaskExecutor::new(1);
        let repo_load_executor = super::executor::TaskExecutor::new(1);
        let metadata_executor = super::executor::TaskExecutor::new(if expected == "remote_tags" {
            super::executor::metadata_worker_threads()
        } else {
            1
        });
        let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();
        let msg_tx = super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);
        let mut state = AppState::test_default();
        state.repos.push(RepoState::new_opening(
            repo_a,
            RepoSpec {
                workdir: unique_temp_path("gitcomet-metadata-state-a"),
            },
        ));
        state.repos.push(RepoState::new_opening(
            repo_b,
            RepoSpec {
                workdir: unique_temp_path("gitcomet-metadata-state-b"),
            },
        ));
        let thread_state = Arc::new(std::sync::RwLock::new(Arc::new(state)));
        let mut repo_task_tokens = FxHashMap::default();
        let executors = super::effects::EffectExecutors {
            executor: &executor,
            repo_load_executor: &repo_load_executor,
            worktree_scan_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
            session_persist_executor: &executor,
            metadata_executor: &metadata_executor,
            signature_executor: &metadata_executor,
            history_find_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
        };

        super::effects::schedule_effect(
            executors,
            &thread_state,
            &backend,
            &repos,
            &mut repo_task_tokens,
            msg_tx.clone(),
            load,
        );
        assert_eq!(
            started_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("slow task did not start"),
            expected
        );

        super::effects::schedule_effect(
            executors,
            &thread_state,
            &backend,
            &repos,
            &mut repo_task_tokens,
            msg_tx,
            Effect::LoadTags { repo_id: repo_b },
        );

        assert_eq!(
            started_rx
                .recv_timeout(Duration::from_millis(200))
                .unwrap_or_else(|e| panic!("metadata refresh waited behind {expected}: {e}")),
            "tags"
        );
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
        delete_branch_calls: None,
        cancel_delete_branch: None,
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };

    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let executor = super::super::executor::TaskExecutor::new(1);

    let target = DiffTarget::working_tree(PathBuf::from("tracked.txt"), DiffArea::Unstaged);
    let mut state = AppState::test_default();
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
                seq: 1,
                scope: LogScope::CurrentBranch,
                author: None,
                limit: 20,
                cursor: None,
            },
            1,
        ),
        (
            Effect::LoadLog {
                repo_id,
                seq: 2,
                scope: LogScope::AllBranches,
                author: None,
                limit: 20,
                cursor: Some(LogCursor {
                    last_seen: CommitId("cursor".into()),
                    resume_from: None,
                    resume_token: None,
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
                cursor: None,
            },
            1,
        ),
        (
            Effect::LoadBlame {
                repo_id,
                path: PathBuf::from("tracked.txt"),
                source: gitcomet_core::domain::BlameSource::Revision(Some("HEAD".to_string())),
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
                load_submodule_summary: false,
                preview_text_side: None,
            },
            3,
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
                expected_contents: None,
                contents: "content".to_string().into(),
                stage: true,
                completion: None,
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
                mode: gitcomet_core::services::CheckoutRemoteBranchMode::Create,
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
                commit: true,
                mainline: None,
                summary: "pick me".into(),
                auth: None,
            },
            1,
        ),
        (
            Effect::RevertCommit {
                repo_id,
                commit_id: commit_id.clone(),
                commit: true,
                mainline: None,
                summary: "revert me".into(),
                auth: None,
            },
            1,
        ),
        (
            Effect::ApplyFileChange {
                commit_retry: None,
                repo_id,
                target: gitcomet_core::domain::ApplyChangeTarget::commit(
                    commit_id.clone(),
                    PathBuf::from("tracked.txt"),
                ),
                commit: false,
                auth: None,
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
                force: false,
            },
            1,
        ),
        (
            Effect::RenameBranch {
                repo_id,
                old_name: "topic2".to_string(),
                new_name: "renamed-topic".to_string(),
                force: false,
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
                remote_url_policy: Default::default(),
                auth: None,
            },
            1,
        ),
        (
            Effect::UpdateSubmodules {
                approved_sources: Vec::new(),
                repo_id,
                remote_url_policy: Default::default(),
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
                patch: "@@ -1 +1 @@".to_string().into(),
            },
            1,
        ),
        (
            Effect::UnstageHunk {
                repo_id,
                patch: "@@ -1 +1 @@".to_string().into(),
            },
            1,
        ),
        (
            Effect::ApplyWorktreePatch {
                repo_id,
                patch: "@@ -1 +1 @@".to_string().into(),
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
            Effect::SafePushAfterCommit {
                repo_id,
                context: gitcomet_core::services::SafePushAfterCommitContext {
                    amend: false,
                    local_branch: None,
                    pre_head: None,
                    post_head: None,
                },
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
                prune: true,
                auth: None,
            },
            1,
        ),
        (
            Effect::PullBranch {
                repo_id,
                remote: "origin".to_string(),
                branch: "main".to_string(),
                prune: true,
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
            Effect::PushAfterCommit {
                repo_id,
                target: gitcomet_core::services::SafePushAfterCommitTarget {
                    remote: "origin".to_string(),
                    branch: "main".to_string(),
                    local_branch: "main".to_string(),
                    local_head: CommitId("2222222222222222222222222222222222222222".into()),
                },
                set_upstream: false,
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
            Effect::ForcePushWithLease {
                repo_id,
                lease: gitcomet_core::services::ForcePushLease {
                    remote: "origin".to_string(),
                    branch: "main".to_string(),
                    expected: CommitId("1111111111111111111111111111111111111111".into()),
                    local_branch: "main".to_string(),
                    local_head: CommitId("2222222222222222222222222222222222222222".into()),
                },
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
                upstream: Upstream {
                    remote: "origin".to_string(),
                    branch: "main".to_string(),
                },
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
        (
            Effect::RebaseContinue {
                repo_id,
                auth: None,
            },
            1,
        ),
        (Effect::RebaseAbort { repo_id }, 1),
        (Effect::MergeAbort { repo_id }, 1),
        (
            Effect::CreateTag {
                repo_id,
                name: "v1.0.0".to_string(),
                target: "HEAD".to_string(),
                message: None,
                annotated: false,
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
                remote_url_policy: Default::default(),
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
                remote_url_policy: Default::default(),
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

    let repo_load_executor = super::super::executor::TaskExecutor::new(1);
    let metadata_executor = super::super::executor::TaskExecutor::new(1);
    let executors = super::super::effects::EffectExecutors {
        executor: &executor,
        repo_load_executor: &repo_load_executor,
        worktree_scan_executor: &std::sync::LazyLock::new(|| {
            super::super::executor::TaskExecutor::new(1)
        }),
        session_persist_executor: &executor,
        metadata_executor: &metadata_executor,
        signature_executor: &metadata_executor,
        // No find effect below, so this never starts a worker.
        history_find_executor: &std::sync::LazyLock::new(|| {
            super::super::executor::TaskExecutor::new(1)
        }),
    };
    for (effect, expected_messages) in effect_specs {
        let kind: &'static str = (&effect).into();
        let mut effect_state = state.clone();
        if let Effect::LoadCommitDetails { commit_id, .. } = &effect {
            // A detail load is useful only while its commit is selected.
            effect_state.repos[0].set_selected_commit(Some(commit_id.clone()));
        }
        let thread_state = Arc::new(std::sync::RwLock::new(Arc::new(effect_state)));
        let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
        super::super::effects::schedule_effect(
            executors,
            &thread_state,
            &backend,
            &repos,
            &mut FxHashMap::default(),
            super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx),
            effect,
        );
        // Every task owns a sender clone, so disconnection means this effect's
        // work is done and no message can leak into the next effect's count.
        let mut received = 0;
        loop {
            match msg_rx.recv_timeout(Duration::from_secs(10)) {
                // The Git-operation envelope, skipped as in `recv_effect_message`.
                Ok(Msg::Internal(
                    crate::msg::InternalMsg::GitOperationStarted { .. }
                    | crate::msg::InternalMsg::GitOperationEvent { .. },
                )) => {}
                Ok(_) => received += 1,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    panic!("{kind}: work still running after 10s")
                }
            }
        }
        assert_eq!(received, expected_messages, "{kind}");
    }
    // Workers still running while the process exits crashed the macOS runner
    // with SIGSEGV after this test had passed.
    executor.join();
    repo_load_executor.join();
    metadata_executor.join();
}

/// A queued scan must wait on its own pool while foreground reads complete.
/// Drive the real effect router and backend, not just two executor instances.
#[test]
fn linked_worktree_scan_does_not_block_foreground_repository_loads() {
    use crate::store::{effects::EffectExecutors, executor::TaskExecutor};
    let (_dir, workdir, _linked) =
        crate::store::tests::worktree_redirect::repo_with_linked_worktree();
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let repo_id = RepoId(1);
    let repo = backend.open(&workdir).unwrap();
    let mut state = AppState::test_default();
    let mut repo_state = crate::model::RepoState::new_opening(repo_id, repo.spec().clone());
    repo_state.open = Loadable::Ready(());
    state.repos.push(repo_state);
    state.active_repo = Some(repo_id);
    let thread_state = Arc::new(std::sync::RwLock::new(Arc::new(state)));
    let repos = FxHashMap::from_iter([(repo_id, repo)]);
    let foreground = TaskExecutor::new(1);
    let scans: std::sync::LazyLock<TaskExecutor> =
        std::sync::LazyLock::new(|| TaskExecutor::new(1));
    let other = TaskExecutor::new(1);
    let find: std::sync::LazyLock<TaskExecutor> = std::sync::LazyLock::new(|| TaskExecutor::new(1));
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    scans.spawn(move || {
        started_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(5));
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let tx = crate::store::worker_channel::StoreWorkerSender::for_test_msg_sender(tx);
    let mut tokens = FxHashMap::default();
    let executors = EffectExecutors {
        executor: &other,
        repo_load_executor: &foreground,
        worktree_scan_executor: &scans,
        session_persist_executor: &other,
        metadata_executor: &other,
        signature_executor: &other,
        history_find_executor: &find,
    };
    for effect in [
        Effect::LoadWorktreeDirty {
            repo_id,
            scope: crate::model::WorktreeDirtyScope::All,
            workdir,
            files_for: None,
        },
        Effect::LoadHeadBranch { repo_id },
    ] {
        crate::store::effects::schedule_effect(
            executors,
            &thread_state,
            &backend,
            &repos,
            &mut tokens,
            tx.clone(),
            effect,
        );
    }
    let first = recv_effect_message(&rx, Duration::from_secs(3));
    let premature = rx.try_recv();
    release_tx.send(()).unwrap();
    assert!(
        matches!(
            first,
            Ok(Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
                result: Ok(_),
                ..
            }))
        ),
        "foreground must finish while scans are blocked: {first:?}"
    );
    assert!(
        matches!(premature, Err(std::sync::mpsc::TryRecvError::Empty)),
        "scan must stay on its own pool: {premature:?}"
    );
    assert!(matches!(
        recv_effect_message(&rx, Duration::from_secs(5)),
        Ok(Msg::Internal(
            crate::msg::InternalMsg::WorktreeDirtyLoaded { result: Ok(_), .. }
        ))
    ));
    // Cancellation must also suppress a scan still queued on the new pool.
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    scans.spawn(move || {
        let _ = release_rx.recv_timeout(Duration::from_secs(5));
    });
    crate::store::effects::schedule_effect(
        executors,
        &thread_state,
        &backend,
        &repos,
        &mut tokens,
        tx.clone(),
        Effect::LoadWorktreeDirty {
            repo_id,
            scope: crate::model::WorktreeDirtyScope::All,
            workdir: repos[&repo_id].spec().workdir.clone(),
            files_for: None,
        },
    );
    tokens[&repo_id].cancel();
    release_tx.send(()).unwrap();
    let (drained_tx, drained_rx) = std::sync::mpsc::channel();
    scans.spawn(move || {
        drained_tx.send(()).unwrap();
    });
    drained_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let cancelled_reply = rx.try_recv();
    assert!(
        matches!(cancelled_reply, Err(std::sync::mpsc::TryRecvError::Empty)),
        "a cancelled queued scan cannot publish stale results: {cancelled_reply:?}"
    );
}
