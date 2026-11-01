//! Switching the working directory into and out of the virtual branches.
//!
//! The repository here is `UnconfiguredRepository`, which answers every call
//! with "unsupported". That is enough on purpose: what these tests pin is the
//! *plumbing* — that the effect reaches the repository, and that whatever
//! happens comes back as a `VirtualBranchesActiveFinished` message rather than being
//! dropped on the worker thread where nobody would ever hear about it.
//!
//! Asserting *which* branch `leave_virtual_branches` was called with would need a
//! recording double, and building one here would mean spelling out all 26 of
//! `GitRepository`'s required methods again — Rust has no inheritance, so a
//! double that cares about three calls still has to implement the other 23.
//! `virtual_branches_edits` already carries such a double; duplicating it to assert
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

/// Queue one effect against a repository that cannot do the work, and hand
/// back the channel its replies arrive on.
fn replies_to_unconfigured(effect: Effect) -> std::sync::mpsc::Receiver<Msg> {
    let repo_id = RepoId(7);
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos = RepositoryRuntimes::default();
    {
        repos.attach_repository_for_test(
            repo_id,
            Arc::new(UnconfiguredRepository::new(std::path::PathBuf::from(
                "/repo",
            ))),
        );
    }
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    schedule_effect_for_test(&executor, &executor, &backend, &mut repos, msg_tx, effect);
    msg_rx
}

/// Run one effect against a repository that cannot do the work, and return the
/// single message it produced.
fn run_against_unconfigured(
    effect: Effect,
) -> std::result::Result<Msg, std::sync::mpsc::RecvTimeoutError> {
    replies_to_unconfigured(effect).recv_timeout(std::time::Duration::from_secs(10))
}

/// True when this message is a finished switch whose payload matches.
fn is_switch(msg: &Msg, active: bool) -> bool {
    matches!(
        msg,
        Msg::Internal(crate::msg::InternalMsg::VirtualBranchesActiveFinished {
            repo_id: RepoId(7),
            active: reported,
            result: Err(_),
            ..
        }) if *reported == active
    )
}

#[test]
fn entering_the_virtual_branches_reports_its_outcome_instead_of_dropping_it() {
    // A switch that fails must still arrive: the reducer needs it to clear the
    // busy flag, and the view needs it to stop claiming the working directory
    // moved when it did not.
    let msg = run_against_unconfigured(Effect::EnterVirtualBranches {
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
fn entering_the_virtual_branches_carries_the_branch_to_go_back_to() {
    let msg = run_against_unconfigured(Effect::EnterVirtualBranches {
        repo_id: RepoId(7),
        checkout_base: Some("release/2.3".into()),
    })
    .expect("no message came back");
    assert!(
        matches!(
            msg,
            Msg::Internal(crate::msg::InternalMsg::VirtualBranchesActiveFinished {
                checkout_base: Some(ref base),
                ..
            }) if base == "release/2.3"
        ),
        "the branch has to survive the trip, or leaving has nowhere to go"
    );
}

#[test]
fn leaving_the_virtual_branches_reports_its_outcome() {
    let msg = run_against_unconfigured(Effect::LeaveVirtualBranches {
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
fn leaving_with_nothing_remembered_falls_back_to_reading_the_virtual_branches() {
    // With no branch remembered, the effect reads the virtual branches for its
    // target. That read failing used to be the `?` that could not compile in a
    // closure returning `()`; now it has to come back as a reported failure.
    let msg = run_against_unconfigured(Effect::LeaveVirtualBranches {
        repo_id: RepoId(7),
        checkout_base: None,
    })
    .expect("no message came back");
    assert!(
        is_switch(&msg, false),
        "an unreadable virtual branches is a failure to report, not one to swallow, got {msg:?}"
    );
}

#[test]
fn a_switch_for_a_repository_that_is_gone_still_answers() {
    // The repository was closed between the click and the worker picking the
    // task up. The reply still has to come back, or the busy flag the reducer
    // set on the way in is never cleared.
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos = RepositoryRuntimes::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &mut repos,
        msg_tx,
        Effect::EnterVirtualBranches {
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
            Msg::Internal(crate::msg::InternalMsg::VirtualBranchesActiveFinished {
                repo_id: RepoId(404),
                result: Err(_),
                ..
            })
        ),
        "a missing repository has to be reported through the same channel"
    );
}

#[test]
fn a_leave_that_carries_a_checkout_drops_the_checkout_when_the_leave_fails() {
    // The leave fails here — the repository answers "unsupported" — and the
    // checkout that asked for it goes with it. Running it against a working
    // directory the virtual branches still holds is exactly the stranding the leave
    // exists to prevent, so the second message must never arrive.
    let replies = replies_to_unconfigured(Effect::LeaveVirtualBranchesAndCheckout {
        repo_id: RepoId(7),
        checkout_base: Some("main".into()),
        then: crate::msg::CheckoutRequest::Branch {
            name: "feature".to_string(),
        },
    });

    let msg = replies
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the leave has to be reported, not dropped on the worker");
    assert!(
        is_switch(&msg, false),
        "expected a failed leave to be reported, got {msg:?}"
    );
    match replies.recv_timeout(std::time::Duration::from_millis(200)) {
        Ok(later) => panic!("the checkout ran after a leave that failed: {later:?}"),
        Err(_) => {}
    }
}

#[test]
fn a_successful_leave_runs_the_checkout_that_waited_for_it() {
    // The whole point of carrying the checkout through the leave: once the
    // working directory is back on an ordinary branch, the request that had to
    // wait arrives as the ordinary message it is, in order behind the report.
    let repo_id = RepoId(7);
    let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let repo = RecordingCheckoutRepo::new(
        RepoSpec {
            workdir: std::path::PathBuf::from("/repo"),
        },
        Arc::clone(&calls),
    );
    let mut repos = RepositoryRuntimes::default();
    repos.attach_repository_for_test(repo_id, Arc::new(repo));
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &mut repos,
        msg_tx,
        Effect::LeaveVirtualBranchesAndCheckout {
            repo_id,
            checkout_base: Some("main".into()),
            then: crate::msg::CheckoutRequest::Branch {
                name: "feature".to_string(),
            },
        },
    );

    let left = msg_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the leave has to be reported first");
    assert!(
        matches!(
            left,
            Msg::Internal(crate::msg::InternalMsg::VirtualBranchesActiveFinished {
                repo_id: RepoId(7),
                active: false,
                result: Ok(()),
                ..
            })
        ),
        "expected the leave to be reported as done, got {left:?}"
    );
    let checkout = msg_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the checkout never followed the leave");
    assert!(
        matches!(
            &checkout,
            Msg::CheckoutBranch {
                repo_id: RepoId(7),
                name,
            } if name == "feature"
        ),
        "expected the checkout that asked for the leave, got {checkout:?}"
    );
}
