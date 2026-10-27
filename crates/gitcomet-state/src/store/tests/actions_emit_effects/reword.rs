use super::*;
use crate::model::{AuthRetryOperation, RewordRequestId, RewordStatus};
use crate::msg::InternalMsg;
use gitcomet_core::auth::{AuthChallenge, AuthOutcome, GitAuthKind};
use gitcomet_core::error::{GitFailure, GitFailureId};

const REPO: RepoId = RepoId(1);
const MESSAGE: &str = "Updated subject\n\nUpdated body";

fn target() -> CommitId {
    CommitId("3333333333333333333333333333333333333333".into())
}

fn rewritten() -> CommitId {
    CommitId("5555555555555555555555555555555555555555".into())
}

fn signing_error(rejected: bool) -> Error {
    let challenge = AuthChallenge {
        kind: GitAuthKind::Passphrase,
        prompt: "Signing passphrase:".into(),
        key_path: None,
        key_fingerprint: None,
    };
    Error::new(ErrorKind::Git(
        GitFailure::new(
            "git rebase -i",
            GitFailureId::CommandFailed,
            Some(1),
            Vec::new(),
            b"signing failed".to_vec(),
            None,
        )
        .with_auth_outcome(if rejected {
            AuthOutcome::Rejected(challenge)
        } else {
            AuthOutcome::NeedsInput(challenge)
        }),
    ))
}

struct Fixture {
    repos: RepositoryRuntimes,
    id_alloc: AtomicU64,
    state: AppState,
}

impl Fixture {
    fn new() -> Self {
        let mut state = AppState::test_default();
        let mut repo = RepoState::new_opening(
            REPO,
            RepoSpec {
                workdir: "/tmp/reword".into(),
            },
        );
        repo.history_state.selected_commit = Some(target());
        state.repos.push(repo);
        state.active_repo = Some(REPO);
        Self {
            repos: Default::default(),
            id_alloc: AtomicU64::new(1),
            state,
        }
    }

    fn send(&mut self, message: Msg) -> Vec<Effect> {
        reduce(&mut self.repos, &self.id_alloc, &mut self.state, message)
    }

    fn start(&mut self, request_id: RewordRequestId) -> Vec<Effect> {
        self.send(Msg::RewordCommit {
            repo_id: REPO,
            request_id,
            commit_id: target(),
            message: MESSAGE.into(),
        })
    }

    fn command(request_id: RewordRequestId, rebase_paused: bool) -> RepoCommandKind {
        RepoCommandKind::RewordCommit {
            request_id,
            commit_id: target(),
            message: MESSAGE.into(),
            rebase_paused,
            rewritten_id: None,
        }
    }

    fn finish(
        &mut self,
        request_id: RewordRequestId,
        paused: bool,
        result: Result<CommandOutput>,
    ) -> Vec<Effect> {
        let mut command = Self::command(request_id, paused);
        if result.is_ok()
            && let RepoCommandKind::RewordCommit { rewritten_id, .. } = &mut command
        {
            *rewritten_id = Some(rewritten());
        }
        self.send(Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id: REPO,
            command,
            result,
        }))
    }

    fn status(&self, request_id: RewordRequestId) -> RewordStatus {
        self.state.repos[0].reword.requests[&request_id].status
    }

    fn counts(&self, count: u32) {
        assert_eq!(self.state.repos[0].local_actions_in_flight, count);
        assert_eq!(self.state.repos[0].sequencer_actions_in_flight, count);
    }

    fn replace_with_empty_history(&mut self, indexed: bool) {
        let repo = &mut self.state.repos[0];
        let scope = repo.history_state.history_scope;
        let snapshot = gitcomet_core::services::HistorySnapshot(
            format!("empty-{}-{}", repo.log_rev, repo.history_state.indexed.rev).into(),
        );
        if indexed {
            repo.history_state.log_snapshot = Some(snapshot.clone());
            self.send(Msg::IndexedHistory(
                crate::indexed_history::IndexedHistoryMsg::Ensure { repo_id: REPO },
            ));
            let seq = self.state.repos[0].history_state.indexed.seq;
            let index = gitcomet_core::history_index::HistoryIndexBuilder::new(snapshot, scope, 20)
                .unwrap()
                .finish(&gitcomet_core::services::CancellationToken::new())
                .unwrap();
            self.send(Msg::IndexedHistory(
                crate::indexed_history::IndexedHistoryMsg::Built {
                    repo_id: REPO,
                    seq,
                    result: Ok(Some(index)),
                },
            ));
        } else {
            let seq = repo.loads_in_flight.active_log_seq().unwrap_or_else(|| {
                repo.loads_in_flight
                    .request_log(crate::model::PendingLogLoad {
                        walk: scope.into(),
                        author: None,
                        focus: Default::default(),
                        limit: 100,
                        cursor: None,
                    })
                    .unwrap()
            });
            self.send(Msg::Internal(InternalMsg::LogLoaded {
                repo_id: REPO,
                seq,
                scope,
                cursor: None,
                result: Ok(gitcomet_core::services::HistoryReadResult::Page {
                    page: Arc::new(gitcomet_core::domain::LogPage {
                        commits: vec![],
                        next_cursor: None,
                    }),
                    snapshot: Some(snapshot),
                }),
            }));
        }
    }
}

fn assert_reword_effect(effects: &[Effect], request_id: RewordRequestId, resume: bool) {
    assert!(
        matches!(effects,
            [Effect::RepoCommand(RepoCommandEffect::RewordCommit {
                repo_id: REPO, request_id: actual, commit_id, message, resume: actual_resume, ..
            })] if *actual == request_id && *commit_id == target() && message == MESSAGE && *actual_resume == resume
        ),
        "{effects:?}"
    );
}

#[test]
fn reword_admission_rejections_have_terminal_status_without_counting_an_operation() {
    for reason in ["busy", "unavailable", "blank"] {
        let mut fixture = Fixture::new();
        let request_id = RewordRequestId::new();
        if reason == "busy" {
            fixture.state.repos[0].rebase_in_progress = Loadable::Ready(true);
        } else if reason == "unavailable" {
            fixture.state.git_runtime.availability =
                gitcomet_core::process::GitExecutableAvailability::Unavailable {
                    detail: "Missing Git".into(),
                };
        }
        let effects = fixture.send(Msg::RewordCommit {
            repo_id: REPO,
            request_id,
            commit_id: target(),
            message: if reason == "blank" {
                " \n".into()
            } else {
                MESSAGE.into()
            },
        });
        assert!(effects.is_empty(), "{reason}: {effects:?}");
        assert_eq!(
            fixture.status(request_id),
            RewordStatus::Rejected,
            "{reason}"
        );
        fixture.counts(0);
        assert!(fixture.state.repos[0].feedback.command_log.is_empty());
    }

    let mut fixture = Fixture::new();
    assert!(
        fixture
            .send(Msg::RewordCommit {
                repo_id: RepoId(99),
                request_id: RewordRequestId::new(),
                commit_id: target(),
                message: MESSAGE.into(),
            })
            .is_empty()
    );
    assert!(fixture.state.repos[0].reword.requests.is_empty());
    fixture.counts(0);
}

#[test]
fn reword_duplicate_dispatch_keeps_the_running_request_and_rejects_a_new_request() {
    let mut fixture = Fixture::new();
    let running = RewordRequestId::new();
    assert_reword_effect(&fixture.start(running), running, false);
    assert!(fixture.start(running).is_empty());
    assert_eq!(fixture.status(running), RewordStatus::Running);
    fixture.counts(1);

    let rejected = RewordRequestId::new();
    assert!(fixture.start(rejected).is_empty());
    assert_eq!(fixture.status(rejected), RewordStatus::Rejected);
    assert_eq!(fixture.status(running), RewordStatus::Running);
    fixture.counts(1);
}

#[test]
fn reword_unrelated_and_duplicate_completions_cannot_release_another_operation() {
    let mut fixture = Fixture::new();
    let running = RewordRequestId::new();
    fixture.start(running);
    let unknown = RewordRequestId::new();
    assert!(
        fixture
            .finish(unknown, false, Ok(CommandOutput::empty_success("reword")))
            .is_empty()
    );
    let mut wrong_target = Fixture::command(running, false);
    if let RepoCommandKind::RewordCommit { commit_id, .. } = &mut wrong_target {
        *commit_id = rewritten();
    }
    assert!(
        fixture
            .send(Msg::Internal(InternalMsg::RepoCommandFinished {
                repo_id: REPO,
                command: wrong_target,
                result: Ok(CommandOutput::empty_success("reword")),
            }))
            .is_empty()
    );
    fixture.counts(1);
    assert_eq!(fixture.status(running), RewordStatus::Running);
    assert!(fixture.state.repos[0].feedback.command_log.is_empty());

    fixture.finish(running, false, Ok(CommandOutput::empty_success("reword")));
    fixture.counts(0);
    let next = RewordRequestId::new();
    fixture.start(next);
    assert!(
        fixture
            .finish(running, false, Ok(CommandOutput::empty_success("reword")))
            .is_empty()
    );
    fixture.counts(1);
    assert_eq!(fixture.status(next), RewordStatus::Running);
    assert_eq!(fixture.state.repos[0].feedback.command_log.len(), 1);
}

#[test]
fn reword_signing_retries_preserve_identity_and_select_the_completed_rewrite() {
    let _lock = super::super::staged_auth_test_lock();
    for paused in [false, true] {
        let mut fixture = Fixture::new();
        let request_id = RewordRequestId::new();
        fixture.start(request_id);
        for attempt in 0..2 {
            fixture.finish(request_id, paused, Err(signing_error(attempt > 0)));
            assert_eq!(
                fixture.status(request_id),
                RewordStatus::AwaitingAuthentication
            );
            fixture.counts(0);
            let prompt = fixture.state.auth_prompt.as_ref().unwrap();
            assert!(
                matches!(&prompt.operation, AuthRetryOperation::RepoCommand {
                repo_id: REPO,
                command: RepoCommandKind::RewordCommit { request_id: actual, commit_id, message, .. },
            } if *actual == request_id && *commit_id == target() && message == MESSAGE)
            );
            let generation = prompt.generation;
            let effects = fixture.send(Msg::SubmitAuthPrompt {
                generation,
                username: None,
                secret: "a passphrase".into(),
            });
            assert_reword_effect(&effects, request_id, paused);
            assert!(effects[0].git_auth().is_some());
            assert_eq!(fixture.status(request_id), RewordStatus::Running);
            fixture.counts(1);
            assert!(fixture.state.auth_prompt.is_none());
        }
        fixture.finish(
            request_id,
            false,
            Ok(CommandOutput::empty_success("reword")),
        );
        fixture.counts(0);
        assert_eq!(fixture.status(request_id), RewordStatus::Completed);
        assert_eq!(
            fixture.state.repos[0].history_state.selected_commit,
            Some(rewritten())
        );
        assert!(fixture.state.repos[0].reword.continuation.is_none());
    }
}

#[test]
fn reword_validated_auth_counts_only_the_dispatched_retry() {
    let _lock = super::super::staged_auth_test_lock();
    let mut fixture = Fixture::new();
    let request_id = RewordRequestId::new();
    fixture.start(request_id);
    fixture.finish(request_id, true, Err(signing_error(false)));
    fixture
        .state
        .auth_prompt
        .as_mut()
        .unwrap()
        .challenge
        .as_mut()
        .unwrap()
        .key_path = Some(PathBuf::from("/tmp/reword-signing-key"));

    for rejected in [true, false] {
        let generation = fixture.state.auth_prompt.as_ref().unwrap().generation;
        let effects = fixture.send(Msg::SubmitAuthPrompt {
            generation,
            username: None,
            secret: "a passphrase".into(),
        });
        fixture.counts(0);
        assert_eq!(
            fixture.status(request_id),
            RewordStatus::AwaitingAuthentication
        );
        let Effect::ValidateAuthPrompt { auth, .. } = effects.into_iter().next().unwrap() else {
            panic!("a known key must validate first");
        };
        let effects = fixture.send(Msg::Internal(InternalMsg::AuthPromptValidated {
            generation,
            result: if rejected {
                gitcomet_core::auth::PassphraseValidation::Rejected
            } else {
                gitcomet_core::auth::PassphraseValidation::Unknown
            },
            auth,
        }));
        if rejected {
            assert!(effects.is_empty());
            fixture.counts(0);
            assert_eq!(
                fixture.status(request_id),
                RewordStatus::AwaitingAuthentication
            );
        } else {
            assert_reword_effect(&effects, request_id, true);
            fixture.counts(1);
            assert_eq!(fixture.status(request_id), RewordStatus::Running);
        }
    }
    fixture.finish(
        request_id,
        false,
        Ok(CommandOutput::empty_success("reword")),
    );
    fixture.counts(0);
    assert_eq!(fixture.status(request_id), RewordStatus::Completed);
}

#[test]
fn reword_cancelled_auth_can_be_continued_manually_with_the_original_identity() {
    let _lock = super::super::staged_auth_test_lock();
    let mut fixture = Fixture::new();
    let request_id = RewordRequestId::new();
    fixture.start(request_id);
    fixture.finish(request_id, true, Err(signing_error(false)));
    assert!(fixture.send(Msg::CancelAuthPrompt).is_empty());
    assert_eq!(fixture.status(request_id), RewordStatus::Cancelled);
    assert_eq!(fixture.state.repos[0].reword.continuation, Some(request_id));
    fixture.counts(0);

    let effects = fixture.send(Msg::RebaseContinue { repo_id: REPO });
    assert_reword_effect(&effects, request_id, true);
    fixture.counts(1);
    fixture.finish(
        request_id,
        false,
        Ok(CommandOutput::empty_success("reword")),
    );
    assert_eq!(fixture.status(request_id), RewordStatus::Completed);
    assert_eq!(
        fixture.state.repos[0].history_state.selected_commit,
        Some(rewritten())
    );
    fixture.counts(0);
}

#[test]
fn reword_success_supersedes_a_paused_rebase_log_refresh_before_reselection() {
    let _lock = super::super::staged_auth_test_lock();
    let mut fixture = Fixture::new();
    let page = |id: CommitId| {
        Arc::new(gitcomet_core::domain::LogPage {
            commits: vec![gitcomet_core::domain::Commit {
                id,
                parent_ids: Default::default(),
                author: "Author".into(),
                summary: "Subject".into(),
                time: std::time::UNIX_EPOCH,
            }],
            next_cursor: None,
        })
    };
    let load = |effects: &[Effect]| {
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadLog { seq, walk, .. } => Some((*seq, walk.mode())),
                _ => None,
            })
            .expect("completion must start a fresh history read")
    };
    fixture.state.repos[0].set_log(Loadable::Ready(page(target())));
    let snapshot = gitcomet_core::services::HistorySnapshot("before-reword".into());
    fixture.state.repos[0].history_state.log_snapshot = Some(snapshot.clone());
    fixture.send(Msg::IndexedHistory(
        crate::indexed_history::IndexedHistoryMsg::Ensure { repo_id: REPO },
    ));
    let stale_index_seq = fixture.state.repos[0].history_state.indexed.seq;
    let request_id = RewordRequestId::new();
    fixture.start(request_id);
    let (stale_seq, scope) = load(&fixture.finish(request_id, true, Err(signing_error(false))));
    fixture.send(Msg::CancelAuthPrompt);
    fixture.send(Msg::RebaseContinue { repo_id: REPO });
    let (fresh_seq, _) = load(&fixture.finish(
        request_id,
        false,
        Ok(CommandOutput::empty_success("reword")),
    ));
    assert_ne!(fresh_seq, stale_seq);
    let selection_rev = fixture.state.repos[0].history_state.selected_commit_rev;
    assert!(
        fixture.state.repos[0]
            .history_state
            .multi_selection
            .anchor_log_rev
            .is_none()
    );

    // An old index build can also return before the fresh log's snapshot.
    let old_index = gitcomet_core::history_index::HistoryIndexBuilder::new(snapshot, scope, 20)
        .unwrap()
        .finish(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
    fixture.send(Msg::IndexedHistory(
        crate::indexed_history::IndexedHistoryMsg::Built {
            repo_id: REPO,
            seq: stale_index_seq,
            result: Ok(Some(old_index)),
        },
    ));
    assert!(fixture.state.repos[0].history_state.indexed.index.is_none());
    assert_eq!(
        fixture.state.repos[0].history_state.selected_commit,
        Some(rewritten())
    );

    // The first read captured history before the rewrite, but returns after
    // the successful continuation selected the replacement commit.
    let log_rev = fixture.state.repos[0].log_rev;
    assert!(
        fixture
            .send(Msg::Internal(InternalMsg::LogLoaded {
                repo_id: REPO,
                seq: stale_seq,
                scope,
                cursor: None,
                result: Ok(gitcomet_core::services::HistoryReadResult::Page {
                    page: page(target()),
                    snapshot: None,
                }),
            }))
            .is_empty()
    );
    assert_eq!(fixture.state.repos[0].log_rev, log_rev);
    assert_eq!(
        fixture.state.repos[0].history_state.selected_commit,
        Some(rewritten())
    );
    assert_eq!(
        fixture.state.repos[0].loads_in_flight.active_log_seq(),
        Some(fresh_seq)
    );

    fixture.send(Msg::Internal(InternalMsg::LogLoaded {
        repo_id: REPO,
        seq: fresh_seq,
        scope,
        cursor: None,
        result: Ok(gitcomet_core::services::HistoryReadResult::Page {
            page: page(rewritten()),
            snapshot: Some(gitcomet_core::services::HistorySnapshot(
                "after-reword".into(),
            )),
        }),
    }));
    assert_eq!(
        fixture.state.repos[0].history_state.selected_commit_rev, selection_rev,
        "fresh history must not cancel the queued rewritten-commit detail load"
    );
    fixture.send(Msg::IndexedHistory(
        crate::indexed_history::IndexedHistoryMsg::Ensure { repo_id: REPO },
    ));
    let fresh_index_seq = fixture.state.repos[0].history_state.indexed.seq;
    let mut builder = gitcomet_core::history_index::HistoryIndexBuilder::new(
        gitcomet_core::services::HistorySnapshot("after-reword".into()),
        scope,
        20,
    )
    .unwrap();
    builder
        .push(&[0x55; 20], std::iter::empty::<&[u8]>(), false)
        .unwrap();
    let fresh_index = builder
        .finish(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
    fixture.send(Msg::IndexedHistory(
        crate::indexed_history::IndexedHistoryMsg::Built {
            repo_id: REPO,
            seq: fresh_index_seq,
            result: Ok(Some(fresh_index)),
        },
    ));
    assert_eq!(
        fixture.state.repos[0].history_state.selected_commit_rev, selection_rev,
        "a fresh index must retain the detail load's selection revision too"
    );
    fixture.send(Msg::Internal(InternalMsg::CommitDetailsLoaded {
        repo_id: REPO,
        commit_id: rewritten(),
        result: Ok(gitcomet_core::domain::CommitDetails {
            id: rewritten(),
            message: MESSAGE.into(),
            author_name: "Author".into(),
            author_email: "author@example.test".into(),
            authored_at_unix: 0,
            committed_at: String::new(),
            committed_at_unix: 0,
            parent_ids: vec![],
            files: vec![],
        }),
    }));
    assert_eq!(
        fixture.state.repos[0].history_state.selected_commit,
        Some(rewritten())
    );
    assert!(matches!(
        &fixture.state.repos[0].history_state.commit_details,
        Loadable::Ready(details) if details.id == rewritten() && details.message == MESSAGE
    ));
    assert_eq!(fixture.status(request_id), RewordStatus::Completed);
    fixture.counts(0);
}

#[test]
fn reword_target_survives_partial_history_until_its_owned_operation_settles() {
    let _lock = super::super::staged_auth_test_lock();
    for indexed in [false, true] {
        let mut fixture = Fixture::new();
        fixture.send(Msg::SelectCommit {
            request_id: None,
            repo_id: REPO,
            commit_id: target(),
        });
        let request_id = RewordRequestId::new();
        fixture.start(request_id);
        for phase in ["running", "awaiting auth", "cancelled auth", "continuing"] {
            match phase {
                "awaiting auth" => {
                    fixture.finish(request_id, true, Err(signing_error(false)));
                }
                "cancelled auth" => {
                    fixture.send(Msg::CancelAuthPrompt);
                }
                "continuing" => {
                    assert_reword_effect(
                        &fixture.send(Msg::RebaseContinue { repo_id: REPO }),
                        request_id,
                        true,
                    );
                }
                _ => {}
            }
            fixture.replace_with_empty_history(indexed);
            assert_eq!(
                fixture.state.repos[0].history_state.selected_commit,
                Some(target()),
                "{phase}, indexed={indexed}"
            );
            assert_eq!(
                fixture.state.repos[0]
                    .history_state
                    .multi_selection
                    .commits
                    .as_slice(),
                &[target()],
                "{phase}, indexed={indexed}"
            );
        }
        fixture.finish(
            request_id,
            false,
            Ok(CommandOutput::empty_success("reword")),
        );
        assert_eq!(
            fixture.state.repos[0].history_state.selected_commit,
            Some(rewritten())
        );

        // A completed request does not exempt its original target from later
        // reconciliation, even if the user selects that old id again.
        fixture.send(Msg::SelectCommit {
            request_id: None,
            repo_id: REPO,
            commit_id: target(),
        });
        fixture.replace_with_empty_history(indexed);
        assert!(
            fixture.state.repos[0]
                .history_state
                .selected_commit
                .is_none()
        );
        fixture.counts(0);
    }
}

#[test]
fn reword_unowned_terminal_requests_do_not_retain_missing_history_targets() {
    let _lock = super::super::staged_auth_test_lock();
    for indexed in [false, true] {
        for terminal in [
            RewordStatus::Failed,
            RewordStatus::Rejected,
            RewordStatus::Cancelled,
        ] {
            let mut fixture = Fixture::new();
            fixture.send(Msg::SelectCommit {
                request_id: None,
                repo_id: REPO,
                commit_id: target(),
            });
            let request_id = RewordRequestId::new();
            if terminal == RewordStatus::Rejected {
                fixture.state.repos[0].local_actions_in_flight = 1;
            }
            fixture.start(request_id);
            match terminal {
                RewordStatus::Failed => {
                    fixture.finish(
                        request_id,
                        false,
                        Err(Error::new(ErrorKind::Backend("failed".into()))),
                    );
                }
                RewordStatus::Cancelled => {
                    fixture.finish(request_id, false, Err(signing_error(false)));
                    fixture.send(Msg::CancelAuthPrompt);
                }
                _ => fixture.state.repos[0].local_actions_in_flight = 0,
            }
            assert_eq!(fixture.status(request_id), terminal);
            assert!(!fixture.state.repos[0].reword.can_resume(request_id));
            fixture.replace_with_empty_history(indexed);
            assert!(
                fixture.state.repos[0]
                    .history_state
                    .selected_commit
                    .is_none(),
                "{terminal:?}, indexed={indexed}"
            );
        }
    }
}

#[test]
fn reword_continuation_completion_preserves_navigation_away_from_its_target() {
    let _lock = super::super::staged_auth_test_lock();
    let mut fixture = Fixture::new();
    let request_id = RewordRequestId::new();
    fixture.start(request_id);
    fixture.finish(request_id, true, Err(signing_error(false)));
    fixture.send(Msg::CancelAuthPrompt);
    fixture.send(Msg::RebaseContinue { repo_id: REPO });
    let other = CommitId("7777777777777777777777777777777777777777".into());
    fixture.send(Msg::SelectCommit {
        request_id: None,
        repo_id: REPO,
        commit_id: other.clone(),
    });
    let effects = fixture.finish(
        request_id,
        false,
        Ok(CommandOutput::empty_success("reword")),
    );
    assert_eq!(
        fixture.state.repos[0].history_state.selected_commit,
        Some(other)
    );
    assert!(!effects.iter().any(|effect| matches!(effect, Effect::LoadCommitDetails { commit_id, .. } if *commit_id == rewritten())));
}

#[test]
fn reword_manual_continue_with_auth_pending_replaces_the_prompt_with_running_state() {
    let _lock = super::super::staged_auth_test_lock();
    let mut fixture = Fixture::new();
    let request_id = RewordRequestId::new();
    fixture.start(request_id);
    fixture.finish(request_id, true, Err(signing_error(false)));
    let generation = fixture.state.auth_prompt.as_ref().unwrap().generation;
    assert_reword_effect(
        &fixture.send(Msg::RebaseContinue { repo_id: REPO }),
        request_id,
        true,
    );
    assert!(fixture.state.auth_prompt.is_none());
    assert_eq!(fixture.status(request_id), RewordStatus::Running);
    // An answer from the dismissed prompt cannot schedule a second continuation.
    assert!(
        fixture
            .send(Msg::SubmitAuthPrompt {
                generation,
                username: None,
                secret: "late answer".into(),
            })
            .is_empty()
    );
    fixture.counts(1);
    fixture.finish(request_id, true, Err(signing_error(true)));
    assert_eq!(
        fixture.status(request_id),
        RewordStatus::AwaitingAuthentication
    );
    assert!(fixture.state.auth_prompt.as_ref().unwrap().generation > generation);
    fixture.counts(0);
}

#[test]
fn reword_abort_clears_continuation_and_auth_only_after_success() {
    let _lock = super::super::staged_auth_test_lock();
    let mut fixture = Fixture::new();
    let request_id = RewordRequestId::new();
    fixture.start(request_id);
    fixture.finish(request_id, true, Err(signing_error(false)));
    for success in [false, true] {
        let effects = fixture.send(Msg::RebaseAbort { repo_id: REPO });
        assert!(matches!(
            effects.as_slice(),
            [Effect::RepoCommand(RepoCommandEffect::RebaseAbort {
                repo_id: REPO
            })]
        ));
        fixture.counts(1);
        fixture.send(Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id: REPO,
            command: RepoCommandKind::RebaseAbort,
            result: if success {
                Ok(CommandOutput::empty_success("git rebase --abort"))
            } else {
                Err(Error::new(ErrorKind::Backend("abort failed".into())))
            },
        }));
        fixture.counts(0);
        assert_eq!(
            fixture.state.repos[0].reword.continuation,
            (!success).then_some(request_id)
        );
        assert_eq!(fixture.state.auth_prompt.is_some(), !success);
    }
    assert_eq!(fixture.status(request_id), RewordStatus::Cancelled);
}

#[test]
fn reword_dismissal_prunes_terminal_results_without_cancelling_running_work() {
    let mut fixture = Fixture::new();
    let request_id = RewordRequestId::new();
    fixture.start(request_id);
    assert!(
        fixture
            .send(Msg::DismissRewordCommit {
                repo_id: REPO,
                request_id
            })
            .is_empty()
    );
    assert_eq!(fixture.status(request_id), RewordStatus::Running);
    fixture.counts(1);
    fixture.finish(
        request_id,
        false,
        Ok(CommandOutput::empty_success("reword")),
    );
    assert!(
        !fixture.state.repos[0]
            .reword
            .requests
            .contains_key(&request_id)
    );
    fixture.counts(0);

    let failed = RewordRequestId::new();
    fixture.start(failed);
    fixture.finish(
        failed,
        false,
        Err(Error::new(ErrorKind::Backend("hook failed".into()))),
    );
    assert_eq!(fixture.status(failed), RewordStatus::Failed);
    fixture.send(Msg::DismissRewordCommit {
        repo_id: REPO,
        request_id: failed,
    });
    assert!(!fixture.state.repos[0].reword.requests.contains_key(&failed));
}

#[test]
fn reword_dismissal_keeps_a_paused_continuation_until_manual_completion() {
    let _lock = super::super::staged_auth_test_lock();
    let mut fixture = Fixture::new();
    let request_id = RewordRequestId::new();
    fixture.start(request_id);
    fixture.finish(request_id, true, Err(signing_error(false)));
    fixture.send(Msg::DismissRewordCommit {
        repo_id: REPO,
        request_id,
    });
    fixture.send(Msg::CancelAuthPrompt);
    assert_eq!(fixture.status(request_id), RewordStatus::Cancelled);
    assert_reword_effect(
        &fixture.send(Msg::RebaseContinue { repo_id: REPO }),
        request_id,
        true,
    );
    fixture.finish(
        request_id,
        false,
        Ok(CommandOutput::empty_success("reword")),
    );
    assert!(
        !fixture.state.repos[0]
            .reword
            .requests
            .contains_key(&request_id)
    );
    assert!(fixture.state.repos[0].reword.continuation.is_none());
    fixture.counts(0);
}

#[test]
fn reword_continuation_retires_only_after_a_fresh_idle_sequencer_read() {
    let _lock = super::super::staged_auth_test_lock();
    for timing in ["fresh", "queued", "active"] {
        let mut fixture = Fixture::new();
        let request_id = RewordRequestId::new();
        fixture.start(request_id);
        fixture.finish(request_id, true, Err(signing_error(false)));
        if timing == "active" {
            assert_reword_effect(
                &fixture.send(Msg::RebaseContinue { repo_id: REPO }),
                request_id,
                true,
            );
        }
        let repo = &mut fixture.state.repos[0];
        // Isolate a read started after the paused result, with an optional
        // newer refresh already queued behind it.
        repo.loads_in_flight = Default::default();
        assert!(
            repo.loads_in_flight
                .request(crate::model::LoadKind::RebaseState)
        );
        if timing == "queued" {
            assert!(
                !repo
                    .loads_in_flight
                    .request(crate::model::LoadKind::RebaseState)
            );
        }
        let effects = fixture.send(Msg::Internal(InternalMsg::RebaseStateLoaded {
            repo_id: REPO,
            result: Ok(gitcomet_core::services::SequencerState::None),
        }));
        if timing == "fresh" {
            assert_eq!(fixture.status(request_id), RewordStatus::Cancelled);
            assert!(fixture.state.repos[0].reword.continuation.is_none());
            assert!(fixture.state.auth_prompt.is_none());
            fixture.counts(0);
        } else {
            assert_eq!(fixture.state.repos[0].reword.continuation, Some(request_id));
            assert_eq!(
                fixture.status(request_id),
                if timing == "active" {
                    RewordStatus::Running
                } else {
                    RewordStatus::AwaitingAuthentication
                }
            );
            fixture.counts(u32::from(timing == "active"));
            if timing == "queued" {
                assert!(
                    effects
                        .iter()
                        .any(|effect| matches!(effect, Effect::LoadRebaseState { repo_id: REPO }))
                );
                fixture.send(Msg::Internal(InternalMsg::RebaseStateLoaded {
                    repo_id: REPO,
                    result: Ok(gitcomet_core::services::SequencerState::None),
                }));
                assert!(fixture.state.repos[0].reword.continuation.is_none());
                assert_eq!(fixture.status(request_id), RewordStatus::Cancelled);
                assert!(fixture.state.auth_prompt.is_none());
            }
        }
    }
}
