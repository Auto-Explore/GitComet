//! Exercise the message editor through the store and a controllable repository.

use super::*;
use gitcomet_core::domain::*;
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::{
    CancellationToken, CommandOutput, GitRepository, PullMode, Result, RewordCommitOutcome,
};
use gitcomet_state::model::{AppState, RepoId, RewordRequestId, RewordStatus};
use gitcomet_state::msg::Msg;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

const REWRITTEN_SHA: &str = "dddddddddddddddddddddddddddddddddddddddd";
const REPO_ID: RepoId = RepoId(71);

enum Reply {
    Success,
    Failure,
    NeedsAuthentication,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Call {
    continuing: bool,
    commit_id: CommitId,
    message: String,
}

struct ControlledRewordRepo {
    fallback: gitcomet_core::test_support::UnconfiguredRepository,
    original: RepoState,
    rewritten: Mutex<Option<Call>>,
    calls: Mutex<Vec<Call>>,
    replies: Mutex<mpsc::Receiver<Reply>>,
    paused: std::sync::atomic::AtomicBool,
}

impl ControlledRewordRepo {
    fn run(
        &self,
        continuing: bool,
        commit_id: &CommitId,
        message: &str,
    ) -> Result<RewordCommitOutcome> {
        let call = Call {
            continuing,
            commit_id: commit_id.clone(),
            message: message.into(),
        };
        self.calls.lock().unwrap().push(call.clone());
        match self
            .replies
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .expect("test must finish the pending reword")
        {
            Reply::Success => {
                *self.rewritten.lock().unwrap() = Some(call);
                self.paused.store(false, Ordering::SeqCst);
                Ok(RewordCommitOutcome {
                    output: CommandOutput::empty_success("controlled reword"),
                    rewritten_id: Some(reword_commit_id(REWRITTEN_SHA)),
                })
            }
            Reply::Failure => Err(Error::new(ErrorKind::Backend("message rejected".into()))),
            Reply::NeedsAuthentication => {
                self.paused.store(true, Ordering::SeqCst);
                Err(Error::new(ErrorKind::Git(
                    GitFailure::new(
                        "controlled reword",
                        GitFailureId::CommandFailed,
                        Some(1),
                        vec![],
                        b"signing passphrase required".to_vec(),
                        None,
                    )
                    .with_auth_outcome(
                        gitcomet_core::auth::AuthOutcome::NeedsInput(
                            gitcomet_core::auth::AuthChallenge {
                                kind: gitcomet_core::auth::GitAuthKind::Passphrase,
                                prompt: "Enter signing passphrase".into(),
                                key_path: None,
                                key_fingerprint: None,
                            },
                        ),
                    ),
                )))
            }
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl GitRepository for ControlledRewordRepo {
    fn spec(&self) -> &RepoSpec {
        self.fallback.spec()
    }
    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let Loadable::Ready(page) = &self.original.log else {
            unreachable!()
        };
        let mut page = (**page).clone();
        if let Some(call) = self.rewritten.lock().unwrap().as_ref() {
            for commit in &mut page.commits {
                if commit.id == call.commit_id {
                    commit.id = reword_commit_id(REWRITTEN_SHA);
                    commit.summary = call.message.clone().into();
                }
                for parent in &mut commit.parent_ids {
                    if *parent == call.commit_id {
                        *parent = reword_commit_id(REWRITTEN_SHA);
                    }
                }
            }
        }
        Ok(Arc::new(page))
    }
    fn commit_details(&self, id: &CommitId) -> Result<CommitDetails> {
        let Loadable::Ready(details) = &self.original.history_state.commit_details else {
            unreachable!()
        };
        let mut details = (**details).clone();
        details.id = id.clone();
        if let Some(call) = self.rewritten.lock().unwrap().as_ref()
            && id.as_ref() == REWRITTEN_SHA
        {
            details.message = call.message.clone();
        }
        Ok(details)
    }
    fn current_branch(&self, _: &CancellationToken) -> Result<String> {
        Ok("HEAD".into())
    }
    fn head_commit_id(&self) -> Result<Option<CommitId>> {
        Ok(Some(
            if self
                .rewritten
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|call| call.commit_id.as_ref() == REWORD_HEAD_SHA)
            {
                reword_commit_id(REWRITTEN_SHA)
            } else {
                reword_commit_id(REWORD_HEAD_SHA)
            },
        ))
    }
    fn list_branches(&self, _: &CancellationToken) -> Result<Vec<Branch>> {
        Ok(vec![])
    }
    fn list_remotes(&self, _: &CancellationToken) -> Result<Vec<Remote>> {
        Ok(vec![])
    }
    fn list_remote_branches(&self, _: &CancellationToken) -> Result<Vec<RemoteBranch>> {
        Ok(vec![])
    }
    fn list_submodules(&self, _: &CancellationToken) -> Result<Vec<Submodule>> {
        Ok(vec![])
    }
    fn reflog_head(&self, _: usize) -> Result<Vec<ReflogEntry>> {
        Ok(vec![])
    }
    fn status(&self, _: &CancellationToken) -> Result<RepoStatus> {
        Ok(RepoStatus::default())
    }
    fn stash_list(&self, _: &CancellationToken) -> Result<Vec<StashEntry>> {
        Ok(vec![])
    }
    fn rebase_in_progress(&self, _: &CancellationToken) -> Result<bool> {
        Ok(self.paused.load(Ordering::SeqCst))
    }
    fn rebase_abort_with_output(&self) -> Result<CommandOutput> {
        self.paused.store(false, Ordering::SeqCst);
        Ok(CommandOutput::empty_success("controlled rebase abort"))
    }
    fn reword_commit_in_progress(&self, commit_id: &CommitId, message: &str) -> Result<bool> {
        Ok(self.paused.load(Ordering::SeqCst)
            && self
                .calls
                .lock()
                .unwrap()
                .last()
                .is_some_and(|call| &call.commit_id == commit_id && call.message == message))
    }
    fn reword_commit_with_output(
        &self,
        commit_id: &CommitId,
        message: &str,
    ) -> Result<RewordCommitOutcome> {
        self.run(false, commit_id, message)
    }
    fn continue_reword_commit_with_output(
        &self,
        commit_id: &CommitId,
        message: &str,
    ) -> Result<RewordCommitOutcome> {
        self.run(true, commit_id, message)
    }
    fn diff_unified(&self, target: &DiffTarget) -> Result<String> {
        self.fallback.diff_unified(target)
    }
    fn create_branch(&self, name: &str, target: &CommitId) -> Result<()> {
        self.fallback.create_branch(name, target)
    }
    fn delete_branch(&self, name: &str) -> Result<()> {
        self.fallback.delete_branch(name)
    }
    fn checkout_branch(&self, name: &str) -> Result<()> {
        self.fallback.checkout_branch(name)
    }
    fn checkout_commit(&self, id: &CommitId) -> Result<()> {
        self.fallback.checkout_commit(id)
    }
    fn cherry_pick(&self, id: &CommitId) -> Result<()> {
        self.fallback.cherry_pick(id)
    }
    fn stash_create(&self, message: &str, untracked: bool) -> Result<()> {
        self.fallback.stash_create(message, untracked)
    }
    fn stash_apply(&self, index: usize) -> Result<()> {
        self.fallback.stash_apply(index)
    }
    fn stash_drop(&self, index: usize) -> Result<()> {
        self.fallback.stash_drop(index)
    }
    fn stage(&self, paths: &[&Path]) -> Result<()> {
        self.fallback.stage(paths)
    }
    fn unstage(&self, paths: &[&Path]) -> Result<()> {
        self.fallback.unstage(paths)
    }
    fn commit(&self, message: &str) -> Result<()> {
        self.fallback.commit(message)
    }
    fn fetch_all(&self, prune: bool) -> Result<CommandOutput> {
        self.fallback.fetch_all(prune)
    }
    fn pull(&self, mode: PullMode, prune: bool) -> Result<CommandOutput> {
        self.fallback.pull(mode, prune)
    }
    fn push(&self) -> Result<()> {
        self.fallback.push()
    }
    fn discard_worktree_changes(&self, paths: &[&Path]) -> Result<()> {
        self.fallback.discard_worktree_changes(paths)
    }
}

fn fixture<'a>(
    cx: &'a mut gpui::TestAppContext,
    selected: &str,
) -> (
    gpui::Entity<crate::view::GitCometView>,
    &'a mut gpui::VisualTestContext,
    Arc<ControlledRewordRepo>,
    mpsc::Sender<Reply>,
) {
    let original = rewordable_repo(REPO_ID, selected, "Original message", false);
    let (send, receive) = mpsc::channel();
    let repository = Arc::new(ControlledRewordRepo {
        fallback: gitcomet_core::test_support::UnconfiguredRepository::new(&original.spec.workdir),
        original: original.clone(),
        rewritten: Mutex::new(None),
        calls: Mutex::new(vec![]),
        replies: Mutex::new(receive),
        paused: std::sync::atomic::AtomicBool::new(false),
    });
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });
    show_rewordable_commit(cx, &view, original);
    cx.update(|_, app| {
        view.read(app)
            .store
            .insert_repo_for_test(REPO_ID, repository.clone());
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
    });
    click_selector(cx, "commit_details_edit_message");
    cx.update(|window, app| {
        view.read(app).details_pane.clone().update(app, |pane, cx| {
            pane.commit_reword_input
                .update(cx, |input, cx| input.set_text("Updated message", cx));
        });
        let _ = window.draw(app);
    });
    (view, cx, repository, send)
}

#[track_caller]
fn wait_for(
    view: &gpui::Entity<crate::view::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    ready: impl Fn(&AppState) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let done = cx.update(|_, app| {
            let state = view.read(app).store.snapshot();
            view.update(app, |this, cx| {
                // The panels fixture helper also replaces the store snapshot;
                // doing that here could erase a newer worker result.
                crate::view::test_support::push_test_state(this, Arc::clone(&state), cx)
            });
            ready(&state)
        });
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
        if done {
            return;
        }
        if Instant::now() >= deadline {
            let snapshot = cx.update(|_, app| view.read(app).store.snapshot());
            let repo = &snapshot.repos[0];
            panic!(
                "reword state did not settle: requests {:?}, local actions {}, sequencer actions {}, selected {:?}, details {:?}, error {:?}",
                repo.reword,
                repo.local_actions_in_flight,
                repo.sequencer_actions_in_flight,
                repo.history_state.selected_commit,
                repo.history_state.commit_details,
                repo.feedback.last_error
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn submission(
    view: &gpui::Entity<crate::view::GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Option<RewordRequestId> {
    cx.update(|_, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_reword
            .as_ref()
            .and_then(|draft| draft.submitting.map(|submission| submission.request_id))
    })
}

fn assert_read_only(
    view: &gpui::Entity<crate::view::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    expected: bool,
) {
    assert_eq!(
        cx.update(|_, app| view
            .read(app)
            .details_pane
            .read(app)
            .commit_reword_input
            .read(app)
            .is_read_only()),
        expected
    );
}

#[gpui::test]
fn reword_success_keeps_pending_text_read_only_and_selects_updated_details(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx, repository, reply) = fixture(cx, REWORD_HEAD_SHA);
    cx.simulate_keystrokes("secondary-enter");
    let request_id = submission(&view, cx).expect("submitted request");
    wait_for(&view, cx, |_| repository.calls().len() == 1);
    assert_read_only(&view, cx, true);
    cx.simulate_input("must not overwrite submitted text");
    assert_eq!(
        reword_draft_text(cx, &view).as_deref(),
        Some("Updated message")
    );
    cx.update(|window, app| {
        let input = view
            .read(app)
            .details_pane
            .read(app)
            .commit_reword_input
            .clone();
        input.update(app, |input, cx| input.select_all_text(window, cx));
        assert_eq!(
            input.read(app).selected_text().as_deref(),
            Some("Updated message")
        );
    });
    cx.simulate_keystrokes("secondary-enter");
    assert_eq!(submission(&view, cx), Some(request_id));
    reply.send(Reply::Success).unwrap();
    wait_for(
        &view,
        cx,
        |state| matches!(&state.repos[0].history_state.commit_details, Loadable::Ready(details) if details.id.as_ref() == REWRITTEN_SHA && details.message == "Updated message"),
    );
    assert_eq!(
        repository.calls(),
        vec![Call {
            continuing: false,
            commit_id: reword_commit_id(REWORD_HEAD_SHA),
            message: "Updated message".into()
        }]
    );
    assert_eq!(reword_draft_text(cx, &view), None);
    cx.update(|_, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.state.repos[0].history_state.selected_commit,
            Some(reword_commit_id(REWRITTEN_SHA))
        );
        assert_eq!(
            pane.commit_details_message_input.read(app).text(),
            "Updated message"
        );
    });
}

#[gpui::test]
fn reword_rejected_after_ui_snapshot_unlocks_the_draft(cx: &mut gpui::TestAppContext) {
    let (view, cx, repository, _reply) = fixture(cx, REWORD_HEAD_SHA);
    // The pane still sees idle state; another operation was admitted by the store.
    cx.update(|_, app| {
        let store = &view.read(app).store;
        let mut state = (*store.snapshot()).clone();
        state.repos[0].local_actions_in_flight = 1;
        store.replace_snapshot_for_test(Arc::new(state));
        view.read(app)
            .details_pane
            .clone()
            .update(app, |pane, cx| assert!(pane.submit_commit_reword(cx)));
    });
    let request_id = submission(&view, cx).unwrap();
    crate::view::test_support::drain_store_worker(&view, cx);
    wait_for(&view, cx, |state| {
        state.repos[0]
            .reword
            .requests
            .get(&request_id)
            .is_none_or(|request| request.status == RewordStatus::Rejected)
    });
    assert!(repository.calls().is_empty());
    assert_eq!(submission(&view, cx), None);
    assert_read_only(&view, cx, false);
    assert_eq!(
        reword_draft_text(cx, &view).as_deref(),
        Some("Updated message")
    );
}

#[gpui::test]
fn reword_failure_unlocks_and_can_be_submitted_again(cx: &mut gpui::TestAppContext) {
    let (view, cx, repository, reply) = fixture(cx, REWORD_HEAD_SHA);
    cx.simulate_keystrokes("secondary-enter");
    let first = submission(&view, cx).unwrap();
    wait_for(&view, cx, |_| repository.calls().len() == 1);
    reply.send(Reply::Failure).unwrap();
    wait_for(&view, cx, |state| {
        state.repos[0].local_actions_in_flight == 0
    });
    assert_eq!(submission(&view, cx), None);
    assert_read_only(&view, cx, false);
    cx.update(|window, app| {
        let input = view
            .read(app)
            .details_pane
            .read(app)
            .commit_reword_input
            .clone();
        window.focus(&input.read(app).focus_handle(), app);
    });
    cx.simulate_keystrokes("secondary-enter");
    assert_ne!(submission(&view, cx), Some(first));
    wait_for(&view, cx, |_| repository.calls().len() == 2);
    reply.send(Reply::Success).unwrap();
    wait_for(&view, cx, |state| {
        state.repos[0].local_actions_in_flight == 0
    });
}

#[gpui::test]
fn reword_signing_retries_keep_request_identity_and_follow_the_rewritten_commit(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx, repository, reply) = fixture(cx, REWORD_OLDER_SHA);
    cx.simulate_keystrokes("secondary-enter");
    let request_id = submission(&view, cx).unwrap();
    for attempt in 1..=2 {
        wait_for(&view, cx, |_| repository.calls().len() == attempt);
        reply.send(Reply::NeedsAuthentication).unwrap();
        wait_for(&view, cx, |state| state.auth_prompt.is_some());
        assert_eq!(submission(&view, cx), Some(request_id));
        assert_read_only(&view, cx, true);
        cx.update(|_, app| {
            let store = &view.read(app).store;
            let generation = store.snapshot().auth_prompt.as_ref().unwrap().generation;
            store.dispatch(Msg::SubmitAuthPrompt {
                generation,
                username: None,
                secret: "test passphrase".into(),
            });
        });
    }
    wait_for(&view, cx, |_| repository.calls().len() == 3);
    reply.send(Reply::Success).unwrap();
    wait_for(
        &view,
        cx,
        |state| matches!(&state.repos[0].history_state.commit_details, Loadable::Ready(details) if details.id.as_ref() == REWRITTEN_SHA && details.message == "Updated message"),
    );
    let calls = repository.calls();
    assert_eq!(
        calls.iter().map(|call| call.continuing).collect::<Vec<_>>(),
        vec![false, true, true]
    );
    assert!(calls.iter().all(
        |call| call.commit_id.as_ref() == REWORD_OLDER_SHA && call.message == "Updated message"
    ));
    assert_eq!(reword_draft_text(cx, &view), None);
}

#[gpui::test]
fn reword_completion_does_not_replace_a_new_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx, repository, reply) = fixture(cx, REWORD_HEAD_SHA);
    cx.simulate_keystrokes("secondary-enter");
    wait_for(&view, cx, |_| repository.calls().len() == 1);
    cx.update(|_, app| {
        view.read(app).store.dispatch(Msg::SelectCommit {
            request_id: None,
            repo_id: REPO_ID,
            commit_id: reword_commit_id(REWORD_OLDER_SHA),
        })
    });
    wait_for(&view, cx, |state| {
        state.repos[0].history_state.selected_commit == Some(reword_commit_id(REWORD_OLDER_SHA))
    });
    assert_eq!(reword_draft_text(cx, &view), None);
    reply.send(Reply::Success).unwrap();
    wait_for(&view, cx, |state| {
        state.repos[0].local_actions_in_flight == 0
    });
    cx.update(|_, app| {
        assert_eq!(
            view.read(app).store.snapshot().repos[0]
                .history_state
                .selected_commit,
            Some(reword_commit_id(REWORD_OLDER_SHA))
        )
    });
}

#[gpui::test]
fn closing_a_pending_reword_editor_does_not_cancel_the_git_operation(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx, repository, reply) = fixture(cx, REWORD_HEAD_SHA);
    cx.simulate_keystrokes("secondary-enter");
    wait_for(&view, cx, |_| repository.calls().len() == 1);
    click_selector(cx, "commit_reword_cancel");
    assert_eq!(reword_draft_text(cx, &view), None);
    assert_read_only(&view, cx, false);
    reply.send(Reply::Success).unwrap();
    wait_for(
        &view,
        cx,
        |state| matches!(&state.repos[0].history_state.commit_details, Loadable::Ready(details) if details.id.as_ref() == REWRITTEN_SHA && details.message == "Updated message"),
    );
    assert_eq!(repository.calls().len(), 1);
    cx.update(|_, app| {
        assert_eq!(
            view.read(app)
                .details_pane
                .read(app)
                .commit_details_message_input
                .read(app)
                .text(),
            "Updated message"
        );
    });
}

fn pause_and_cancel_auth(
    view: &gpui::Entity<crate::view::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repository: &ControlledRewordRepo,
    reply: &mpsc::Sender<Reply>,
) -> RewordRequestId {
    cx.simulate_keystrokes("secondary-enter");
    let request_id = submission(view, cx).unwrap();
    wait_for(view, cx, |_| repository.calls().len() == 1);
    reply.send(Reply::NeedsAuthentication).unwrap();
    wait_for(view, cx, |state| state.auth_prompt.is_some());
    cx.update(|_, app| view.read(app).store.dispatch(Msg::CancelAuthPrompt));
    wait_for(view, cx, |state| state.auth_prompt.is_none());
    assert_eq!(submission(view, cx), None);
    assert_read_only(view, cx, true);
    assert_eq!(
        reword_draft_text(cx, view).as_deref(),
        Some("Updated message")
    );
    request_id
}

#[gpui::test]
fn reword_manual_continue_after_auth_cancellation_keeps_the_editor_locked(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx, repository, reply) = fixture(cx, REWORD_OLDER_SHA);
    let request_id = pause_and_cancel_auth(&view, cx, &repository, &reply);
    cx.update(|_, app| {
        view.read(app)
            .store
            .dispatch(Msg::RebaseContinue { repo_id: REPO_ID })
    });
    wait_for(&view, cx, |_| repository.calls().len() == 2);
    assert_eq!(submission(&view, cx), Some(request_id));
    assert_read_only(&view, cx, true);
    reply.send(Reply::Success).unwrap();
    wait_for(
        &view,
        cx,
        |state| matches!(&state.repos[0].history_state.commit_details, Loadable::Ready(details) if details.id.as_ref() == REWRITTEN_SHA),
    );
    assert_eq!(reword_draft_text(cx, &view), None);
    assert!(repository.calls()[1].continuing);
}

#[gpui::test]
fn reword_aborting_the_paused_update_restores_editing(cx: &mut gpui::TestAppContext) {
    let (view, cx, repository, reply) = fixture(cx, REWORD_OLDER_SHA);
    let request_id = pause_and_cancel_auth(&view, cx, &repository, &reply);
    cx.update(|_, app| {
        view.read(app)
            .store
            .dispatch(Msg::RebaseAbort { repo_id: REPO_ID })
    });
    wait_for(&view, cx, |state| {
        !state.repos[0].reword.can_resume(request_id)
    });
    assert_read_only(&view, cx, false);
    assert_eq!(
        reword_draft_text(cx, &view).as_deref(),
        Some("Updated message")
    );
    assert_eq!(repository.calls().len(), 1);
}
