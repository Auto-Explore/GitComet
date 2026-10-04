//! Switching the working directory into and out of the workspace.
//!
//! The repository here is `UnconfiguredRepository`, which answers every call
//! with "unsupported". That is enough on purpose: what these tests pin is the
//! *plumbing* — that the effect reaches the repository, and that whatever
//! happens comes back as a `WorkspaceActiveFinished` message rather than being
//! dropped on the worker thread where nobody would ever hear about it.
//!
//! Asserting *which* branch `leave_workspace` was called with would need a
//! recording double, and building one here would mean spelling out all 26 of
//! `GitRepository`'s required methods again — Rust has no inheritance, so a
//! double that cares about three calls still has to implement the other 23.
//! `workspace_edits` already carries such a double; duplicating it to assert
//! one more field would cost more than the assertion is worth.

use super::*;

/// Enough `GitBackend` to satisfy the scheduling harness; it is never asked to
/// open anything, because the repositories are supplied directly.
struct Backend;

impl GitBackend for Backend {
    fn open(&self, _path: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported("test backend")))
    }
}

/// Run one effect against a repository that cannot do the work, and return the
/// single message it produced.
fn run_against_unconfigured(
    effect: Effect,
) -> std::result::Result<Msg, std::sync::mpsc::RecvError> {
    let repo_id = RepoId(7);
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(
            repo_id,
            Arc::new(UnconfiguredRepository::new(std::path::PathBuf::from("/repo"))),
        );
        repos
    };
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    schedule_effect_for_test(&executor, &executor, &backend, &repos, msg_tx, effect);
    msg_rx.recv_timeout(std::time::Duration::from_secs(10))
}

/// True when this message is a finished switch whose payload matches.
fn is_switch(msg: &Msg, active: bool) -> bool {
    matches!(
        msg,
        Msg::Internal(crate::msg::InternalMsg::WorkspaceActiveFinished {
            repo_id: RepoId(7),
            active: reported,
            result: Err(_),
            ..
        }) if *reported == active
    )
}

#[test]
fn entering_the_workspace_reports_its_outcome_instead_of_dropping_it() {
    // A switch that fails must still arrive: the reducer needs it to clear the
    // busy flag, and the view needs it to stop claiming the working directory
    // moved when it did not.
    let msg = run_against_unconfigured(Effect::EnterWorkspace {
        repo_id: RepoId(7),
        checkout_base: Some("main".into()),
    })
    .expect("no message came back");
    assert!(
        is_switch(&msg, true),
        "expected a failed enter to be reported, got {msg:?}"
    );
}

#[test]
fn entering_the_workspace_carries_the_branch_to_go_back_to() {
    let msg = run_against_unconfigured(Effect::EnterWorkspace {
        repo_id: RepoId(7),
        checkout_base: Some("release/2.3".into()),
    })
    .expect("no message came back");
    assert!(
        matches!(
            msg,
            Msg::Internal(crate::msg::InternalMsg::WorkspaceActiveFinished {
                checkout_base: Some(ref base),
                ..
            }) if base == "release/2.3"
        ),
        "the branch has to survive the trip, or leaving has nowhere to go"
    );
}

#[test]
fn leaving_the_workspace_reports_its_outcome() {
    let msg = run_against_unconfigured(Effect::LeaveWorkspace {
        repo_id: RepoId(7),
        checkout_base: Some("main".into()),
    })
    .expect("no message came back");
    assert!(
        is_switch(&msg, false),
        "expected a failed leave to be reported, got {msg:?}"
    );
}

#[test]
fn leaving_with_nothing_remembered_falls_back_to_reading_the_workspace() {
    // With no branch remembered, the effect reads the workspace for its
    // target. That read failing used to be the `?` that could not compile in a
    // closure returning `()`; now it has to come back as a reported failure.
    let msg = run_against_unconfigured(Effect::LeaveWorkspace {
        repo_id: RepoId(7),
        checkout_base: None,
    })
    .expect("no message came back");
    assert!(
        is_switch(&msg, false),
        "an unreadable workspace is a failure to report, not one to swallow, got {msg:?}"
    );
}

#[test]
fn a_switch_for_a_repository_that_is_gone_still_answers() {
    // The repository was closed between the click and the worker picking the
    // task up. The reply still has to come back, or the busy flag the reducer
    // set on the way in is never cleared.
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::EnterWorkspace {
            repo_id: RepoId(404),
            checkout_base: Some("main".into()),
        },
    );
    let msg = msg_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("no message came back");
    assert!(
        matches!(
            msg,
            Msg::Internal(crate::msg::InternalMsg::WorkspaceActiveFinished {
                repo_id: RepoId(404),
                result: Err(_),
                ..
            })
        ),
        "a missing repository has to be reported through the same channel"
    );
}
