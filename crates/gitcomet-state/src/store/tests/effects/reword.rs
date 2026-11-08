use super::*;
use crate::model::RewordRequestId;
use crate::msg::{InternalMsg, RepoCommandEffect};
use gitcomet_core::services::RewordCommitOutcome;

#[derive(Clone, Debug, Eq, PartialEq)]
struct RewordCall {
    operation: &'static str,
    commit_id: CommitId,
    message: String,
}

struct RecordingRewordRepo {
    spec: RepoSpec,
    calls: Arc<Mutex<Vec<RewordCall>>>,
    fail: bool,
    owned_pause: bool,
}

impl RecordingRewordRepo {
    fn record(&self, operation: &'static str, commit_id: &CommitId, message: &str) {
        self.calls.lock().unwrap().push(RewordCall {
            operation,
            commit_id: commit_id.clone(),
            message: message.into(),
        });
    }

    fn outcome(&self) -> Result<RewordCommitOutcome> {
        if self.fail {
            Err(Error::new(ErrorKind::Backend("signing failed".into())))
        } else {
            Ok(RewordCommitOutcome {
                output: CommandOutput::empty_success("reword fixture"),
                rewritten_id: Some(CommitId("5555555555555555555555555555555555555555".into())),
            })
        }
    }
}

impl GitRepository for RecordingRewordRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }
    fn reword_commit_with_output(
        &self,
        commit_id: &CommitId,
        message: &str,
    ) -> Result<RewordCommitOutcome> {
        self.record("start", commit_id, message);
        self.outcome()
    }
    fn continue_reword_commit_with_output(
        &self,
        commit_id: &CommitId,
        message: &str,
    ) -> Result<RewordCommitOutcome> {
        self.record("continue", commit_id, message);
        self.outcome()
    }
    fn reword_commit_in_progress(&self, commit_id: &CommitId, message: &str) -> Result<bool> {
        self.record("owned pause", commit_id, message);
        Ok(self.owned_pause)
    }
    fn log_head_page(
        &self,
        _: usize,
        _: Option<&LogCursor>,
        _: &CancellationToken,
    ) -> Result<Arc<LogPage>> {
        unsupported_repo_result()
    }
    fn commit_details(&self, _: &CommitId) -> Result<CommitDetails> {
        unsupported_repo_result()
    }
    fn reflog_head(&self, _: usize) -> Result<Vec<ReflogEntry>> {
        unsupported_repo_result()
    }
    fn current_branch(&self, _: &CancellationToken) -> Result<String> {
        unsupported_repo_result()
    }
    fn list_branches(&self, _: &CancellationToken) -> Result<Vec<Branch>> {
        unsupported_repo_result()
    }
    fn list_remotes(&self, _: &CancellationToken) -> Result<Vec<Remote>> {
        unsupported_repo_result()
    }
    fn list_remote_branches(&self, _: &CancellationToken) -> Result<Vec<RemoteBranch>> {
        unsupported_repo_result()
    }
    fn status(&self, _: &CancellationToken) -> Result<RepoStatus> {
        unsupported_repo_result()
    }
    fn diff_unified(&self, _: &DiffTarget) -> Result<String> {
        unsupported_repo_result()
    }
    fn create_branch(&self, _: &str, _: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn delete_branch(&self, _: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn checkout_branch(&self, _: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn checkout_commit(&self, _: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn cherry_pick(&self, _: &CommitId) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_create(&self, _: &str, _: bool) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_list(&self, _: &CancellationToken) -> Result<Vec<StashEntry>> {
        unsupported_repo_result()
    }
    fn stash_apply(&self, _: usize) -> Result<()> {
        unsupported_repo_result()
    }
    fn stash_drop(&self, _: usize) -> Result<()> {
        unsupported_repo_result()
    }
    fn stage(&self, _: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn unstage(&self, _: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
    fn commit(&self, _: &str) -> Result<()> {
        unsupported_repo_result()
    }
    fn fetch_all(&self, _: bool) -> Result<CommandOutput> {
        unsupported_repo_result()
    }
    fn pull(&self, _: PullMode) -> Result<CommandOutput> {
        unsupported_repo_result()
    }
    fn push(&self) -> Result<()> {
        unsupported_repo_result()
    }
    fn discard_worktree_changes(&self, _: &[&Path]) -> Result<()> {
        unsupported_repo_result()
    }
}

fn run_reword_effect(
    resume: bool,
    failure: Option<bool>,
) -> (
    RewordRequestId,
    RepoCommandKind,
    Result<CommandOutput>,
    Vec<RewordCall>,
) {
    let request_id = RewordRequestId::new();
    let commit_id = CommitId("3333333333333333333333333333333333333333".into());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
    let mut repos = RepositoryRuntimes::default();
    repos.attach_repository_for_test(
        RepoId(1),
        Arc::new(RecordingRewordRepo {
            spec: RepoSpec {
                workdir: "/tmp/reword-effect".into(),
            },
            calls: calls.clone(),
            fail: failure.is_some(),
            owned_pause: failure.unwrap_or(false),
        }),
    );
    let (send, receive) = std::sync::mpsc::channel();
    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &mut repos,
        send,
        Effect::RepoCommand(RepoCommandEffect::RewordCommit {
            repo_id: RepoId(1),
            request_id,
            commit_id,
            message: "New subject\n\n# Body preserved".into(),
            resume,
            auth: None,
        }),
    );
    executor.join();
    let Msg::Internal(InternalMsg::RepoCommandFinished {
        repo_id: RepoId(1),
        command,
        result,
    }) = recv_effect_message(&receive, Duration::from_secs(5)).unwrap()
    else {
        panic!("expected a reword result");
    };
    let calls = calls.lock().unwrap().clone();
    (request_id, command, result, calls)
}

#[test]
fn reword_effect_calls_the_correct_backend_entry_and_keeps_request_and_result_identity() {
    for resume in [false, true] {
        let (request_id, command, result, calls) = run_reword_effect(resume, None);
        let RepoCommandKind::RewordCommit {
            request_id: actual,
            commit_id,
            message,
            rewritten_id,
            ..
        } = command
        else {
            panic!("wrong command")
        };
        assert_eq!(actual, request_id);
        assert_eq!(
            calls,
            vec![RewordCall {
                operation: if resume { "continue" } else { "start" },
                commit_id,
                message,
            }]
        );
        assert_eq!(
            calls[0].commit_id.as_ref(),
            "3333333333333333333333333333333333333333"
        );
        assert_eq!(calls[0].message, "New subject\n\n# Body preserved");
        assert_eq!(
            rewritten_id.as_ref().map(CommitId::as_ref),
            Some("5555555555555555555555555555555555555555")
        );
        assert_eq!(result.unwrap().command, "reword fixture");
    }
}

#[test]
fn reword_failure_checks_ownership_with_the_original_target_and_message() {
    for resume in [false, true] {
        for owned in [false, true] {
            let (request_id, command, result, calls) = run_reword_effect(resume, Some(owned));
            assert!(result.is_err());
            let RepoCommandKind::RewordCommit {
                request_id: actual,
                commit_id,
                message,
                rebase_paused,
                rewritten_id,
            } = command
            else {
                panic!("wrong command")
            };
            assert_eq!(actual, request_id);
            assert_eq!(rebase_paused, owned);
            assert!(rewritten_id.is_none());
            assert_eq!(
                calls,
                vec![
                    RewordCall {
                        operation: if resume { "continue" } else { "start" },
                        commit_id: commit_id.clone(),
                        message: message.clone()
                    },
                    RewordCall {
                        operation: "owned pause",
                        commit_id,
                        message
                    },
                ]
            );
        }
    }
}

#[test]
fn reword_effect_missing_repository_still_finishes_the_original_request() {
    let request_id = RewordRequestId::new();
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: "/tmp/missing-reword".into(),
        },
    ));
    let mut effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RewordCommit {
            repo_id: RepoId(1),
            request_id,
            commit_id: CommitId("3333333333333333333333333333333333333333".into()),
            message: "New message".into(),
        },
    );
    assert_eq!(effects.len(), 1);
    assert_eq!(state.repos[0].local_actions_in_flight, 1);
    assert_eq!(state.repos[0].sequencer_actions_in_flight, 1);
    let (send, receive) = std::sync::mpsc::channel();
    schedule_effect_with_state_for_test(
        &executor,
        &executor,
        &backend,
        &mut repos,
        state.clone(),
        send,
        effects.remove(0),
    );
    executor.join();
    let reply = recv_effect_message(&receive, Duration::from_secs(5)).unwrap();
    assert!(matches!(&reply,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: RepoCommandKind::RewordCommit { request_id: actual, .. },
            result: Err(_),
        }) if *actual == request_id));
    reduce(&mut repos, &id_alloc, &mut state, reply);
    assert_eq!(
        state.repos[0].reword.requests[&request_id].status,
        crate::model::RewordStatus::Failed
    );
    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert_eq!(state.repos[0].sequencer_actions_in_flight, 0);
}
