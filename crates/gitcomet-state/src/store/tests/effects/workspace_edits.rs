//! Realizing a workspace edit: which Git operations it asks for, and in what
//! order.
//!
//! What is pinned here is the decision inside `apply_edit`, because that is
//! where the meaning of an edit lives: above versus below an anchor, whether an
//! edit rewrites history, and whether the workspace branch has to be rebuilt at
//! all. The plumbing — a failure coming back as a message rather than dying on
//! the worker — is covered in `workspace_switch`.
//!
//! The double records calls instead of running them. `GitRepository` has 26
//! required methods and Rust has no inheritance, so they are all spelled out;
//! each one the workspace does not use answers with an error naming itself
//! rather than panicking, so an edit that reaches for the wrong thing fails the
//! test with a sentence instead of a backtrace.

use super::*;

use crate::model::WorkspaceEdit;
use gitcomet_core::services::CommitOperationOutcome;
use gitcomet_core::workspace::{AssignmentIndex, BranchApplyState, VirtualBranch, WorkspaceState};

/// Enough `GitBackend` to satisfy the scheduling harness; the repositories are
/// supplied directly, so it is never asked to open anything.
struct Backend;

impl GitBackend for Backend {
    fn open(&self, _path: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported("test backend")))
    }
}

/// The workspace calls one edit made, in the order it made them.
#[derive(Clone, Default)]
struct Calls {
    /// `create_virtual_branch(name, base)`.
    created: Vec<(String, String)>,
    /// `rebase_virtual_branch(name, onto)`.
    rebased: Vec<(String, String)>,
    /// `delete_branch(name)` — must stay empty; removing a branch from the
    /// workspace is not a request to delete work.
    deleted: Vec<String>,
    /// `update_workspace_target(target)`.
    targets: Vec<String>,
    /// `update_workspace_branch(applied)`, one entry per rebuild.
    rebuilds: Vec<Vec<String>>,
    /// The workspace as it stood each time it was written.
    writes: Vec<WorkspaceState>,
    /// `commit_paths_to_virtual_branch(name, message, paths)`.
    committed: Vec<(String, String, Vec<PathBuf>)>,
    /// `push_virtual_branch(name)` — the branch is named because the workspace
    /// branch itself must never be what gets published.
    pushed: Vec<String>,
    /// How many times the stored workspace was read.
    reads: usize,
}

/// A workspace that remembers what it was asked to do.
struct WorkspaceRepo {
    spec: RepoSpec,
    state: Mutex<WorkspaceState>,
    assignments: Mutex<AssignmentIndex>,
    calls: Mutex<Calls>,
}

impl WorkspaceRepo {
    fn new(branches: Vec<VirtualBranch>) -> Self {
        Self {
            spec: RepoSpec {
                workdir: PathBuf::from("/workspace-repo"),
            },
            state: Mutex::new(WorkspaceState::new("main").with_branches(branches)),
            assignments: Mutex::new(AssignmentIndex::new()),
            calls: Mutex::new(Calls::default()),
        }
    }

    fn calls(&self) -> Calls {
        self.calls.lock().expect("workspace calls poisoned").clone()
    }

    fn last_write(&self) -> WorkspaceState {
        self.calls()
            .writes
            .last()
            .cloned()
            .expect("the edit wrote nothing")
    }

    fn stored(&self) -> WorkspaceState {
        self.state.lock().expect("workspace poisoned").clone()
    }
}

fn unexpected<T>(call: &str) -> Result<T> {
    Err(Error::new(ErrorKind::Backend(format!(
        "the workspace double was not expected to be asked for {call}"
    ))))
}

impl GitRepository for WorkspaceRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
    ) -> Result<Arc<LogPage>> {
        unexpected("log_head_page")
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unexpected("commit_details")
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unexpected("reflog_head")
    }

    fn current_branch(&self) -> Result<String> {
        unexpected("current_branch")
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        unexpected("list_branches")
    }

    fn list_remotes(&self) -> Result<Vec<Remote>> {
        unexpected("list_remotes")
    }

    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        unexpected("list_remote_branches")
    }

    fn status(&self) -> Result<RepoStatus> {
        unexpected("status")
    }

    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unexpected("diff_unified")
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        unexpected("create_branch")
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        self.calls
            .lock()
            .expect("workspace calls poisoned")
            .deleted
            .push(name.to_string());
        Ok(())
    }

    fn checkout_branch(&self, _name: &str) -> Result<()> {
        unexpected("checkout_branch")
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        unexpected("checkout_commit")
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        unexpected("cherry_pick")
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unexpected("stash_create")
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        unexpected("stash_list")
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        unexpected("stash_apply")
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        unexpected("stash_drop")
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        unexpected("stage")
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        unexpected("unstage")
    }

    fn commit(&self, _message: &str) -> Result<()> {
        unexpected("commit")
    }

    fn fetch_all(&self) -> Result<()> {
        unexpected("fetch_all")
    }

    fn pull(&self, _mode: PullMode) -> Result<()> {
        unexpected("pull")
    }

    fn push(&self) -> Result<()> {
        unexpected("push")
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unexpected("discard_worktree_changes")
    }

    fn read_workspace(&self) -> Result<WorkspaceState> {
        let mut calls = self.calls.lock().expect("workspace calls poisoned");
        calls.reads += 1;
        Ok(self.state.lock().expect("workspace poisoned").clone())
    }

    fn write_workspace(&self, state: &WorkspaceState) -> Result<()> {
        let mut calls = self.calls.lock().expect("workspace calls poisoned");
        calls.writes.push(state.clone());
        *self.state.lock().expect("workspace poisoned") = state.clone();
        Ok(())
    }

    fn read_workspace_assignments(&self) -> Result<AssignmentIndex> {
        Ok(self
            .assignments
            .lock()
            .expect("assignments poisoned")
            .clone())
    }

    fn write_workspace_assignments(&self, index: &AssignmentIndex) -> Result<()> {
        *self.assignments.lock().expect("assignments poisoned") = index.clone();
        Ok(())
    }

    fn create_virtual_branch(&self, name: &str, base: &str) -> Result<()> {
        self.calls
            .lock()
            .expect("workspace calls poisoned")
            .created
            .push((name.to_string(), base.to_string()));
        Ok(())
    }

    fn rebase_virtual_branch(&self, name: &str, onto: &str) -> Result<()> {
        self.calls
            .lock()
            .expect("workspace calls poisoned")
            .rebased
            .push((name.to_string(), onto.to_string()));
        Ok(())
    }

    fn update_workspace_branch(&self, applied: &[&str]) -> Result<CommitId> {
        let mut calls = self.calls.lock().expect("workspace calls poisoned");
        let rebuild = calls.rebuilds.len();
        calls
            .rebuilds
            .push(applied.iter().map(|name| name.to_string()).collect());
        Ok(CommitId(format!("workspace-{rebuild}")))
    }

    fn update_workspace_target(&self, target: &str) -> Result<()> {
        // The backend owns the move, so it writes the state itself — which is
        // why the effect reads it back rather than trusting its own copy.
        let mut state = self.state.lock().expect("workspace poisoned");
        state.set_target(target)?;
        self.calls
            .lock()
            .expect("workspace calls poisoned")
            .targets
            .push(target.to_string());
        Ok(())
    }

    fn commit_paths_to_virtual_branch(
        &self,
        name: &str,
        message: &str,
        paths: &[&Path],
    ) -> Result<CommitOperationOutcome> {
        self.calls
            .lock()
            .expect("workspace calls poisoned")
            .committed
            .push((
                name.to_string(),
                message.to_string(),
                paths.iter().map(|path| path.to_path_buf()).collect(),
            ));
        Ok(CommitOperationOutcome {
            local_branch: Some(name.to_string()),
            pre_head: None,
            post_head: Some(CommitId(format!("{name}-commit"))),
        })
    }

    fn push_virtual_branch(&self, name: &str) -> Result<CommandOutput> {
        self.calls
            .lock()
            .expect("workspace calls poisoned")
            .pushed
            .push(name.to_string());
        Ok(CommandOutput::empty_success("git push"))
    }
}

const REPO_ID: RepoId = RepoId(11);

fn repos_with(repo: Arc<dyn GitRepository>) -> FxHashMap<RepoId, Arc<dyn GitRepository>> {
    let mut repos = FxHashMap::default();
    repos.insert(REPO_ID, repo);
    repos
}

/// Schedule one workspace effect and return the message it answers with.
fn schedule(repo: &FxHashMap<RepoId, Arc<dyn GitRepository>>, effect: Effect) -> Msg {
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    schedule_effect_for_test(&executor, &executor, &backend, repo, msg_tx, effect);
    recv_effect_message(&msg_rx, std::time::Duration::from_secs(10)).expect("no message came back")
}

/// Realize one edit and return what it produced.
fn apply(repo: &Arc<WorkspaceRepo>, edit: WorkspaceEdit) -> Result<CommitOperationOutcome> {
    let msg = schedule(
        &repos_with(repo.clone()),
        Effect::ApplyWorkspaceEdit {
            repo_id: REPO_ID,
            edit,
        },
    );
    match msg {
        Msg::Internal(crate::msg::InternalMsg::WorkspaceEditFinished { result, .. }) => result,
        other => panic!("expected a finished edit, got {other:?}"),
    }
}

fn applied(name: &str) -> VirtualBranch {
    VirtualBranch::new(name).with_applied(BranchApplyState::Applied)
}

fn stacked(name: &str, parent: &str) -> VirtualBranch {
    VirtualBranch::new(name)
        .with_parent(parent)
        .with_applied(BranchApplyState::Applied)
}

#[test]
fn creating_a_branch_creates_it_at_the_target_and_puts_it_in_the_workspace() {
    // A branch nobody has committed to yet still has to show up as a branch, or
    // the working directory would not change when it is created.
    let repo = Arc::new(WorkspaceRepo::new(vec![]));
    let outcome = apply(
        &repo,
        WorkspaceEdit::Create {
            name: "api".into(),
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(calls.created, [("api".to_string(), "main".to_string())]);
    assert_eq!(
        calls.rebuilds,
        [vec!["api".to_string()]],
        "a new branch is applied by default, so the workspace has to take it"
    );
    assert!(
        repo.last_write().get("api").is_some(),
        "the state file has to claim the branch exists"
    );
    assert_eq!(
        outcome.post_head.map(|id| id.0),
        Some("workspace-0".to_string()),
        "the caller is told what the workspace branch now points at"
    );
}

#[test]
fn inserting_above_an_anchor_stacks_on_it_and_replays_nothing() {
    // Above means "on top of the anchor": the anchor's base did not move, so
    // nothing that already existed has to be replayed.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api")]));
    apply(
        &repo,
        WorkspaceEdit::InsertRelativeTo {
            name: "ui".into(),
            anchor: "api".into(),
            below: false,
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(
        calls.created,
        [("ui".to_string(), "api".to_string())],
        "above stacks on the anchor itself"
    );
    assert!(
        calls.rebased.is_empty(),
        "no branch changed base, so nothing may be rewritten"
    );
    assert_eq!(
        calls.rebuilds,
        [vec!["api".to_string(), "ui".to_string()]]
    );
}

#[test]
fn inserting_below_an_anchor_takes_its_place_and_replays_it() {
    // Below is the mirror image: the new branch starts where the anchor sat and
    // the anchor is now stacked on it, so the anchor's commits have to be
    // replayed or a pull request for it would contain none of the new branch's.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api")]));
    apply(
        &repo,
        WorkspaceEdit::InsertRelativeTo {
            name: "ui".into(),
            anchor: "api".into(),
            below: true,
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(
        calls.created,
        [("ui".to_string(), "main".to_string())],
        "below starts at the anchor's base, not on the anchor"
    );
    assert_eq!(
        calls.rebased,
        [("api".to_string(), "ui".to_string())],
        "the anchor now sits on the branch that displaced it"
    );
    let written = repo.last_write();
    assert_eq!(written.get("api").and_then(|b| b.parent.clone()).as_deref(), Some("ui"));
    assert_eq!(written.get("ui").and_then(|b| b.parent.clone()), None);
    assert_eq!(
        calls.rebuilds,
        [vec!["ui".to_string(), "api".to_string()]],
        "the new branch is applied first: the anchor is stacked on it"
    );
}

#[test]
fn inserting_below_a_stacked_anchor_keeps_the_stack_the_same_depth() {
    // The anchor had a parent, so the branch displacing it inherits that parent
    // instead of jumping to the target — otherwise the stack loses a level and
    // everything above the anchor would be pulled down with it.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("base"), stacked("api", "base")]));
    apply(
        &repo,
        WorkspaceEdit::InsertRelativeTo {
            name: "ui".into(),
            anchor: "api".into(),
            below: true,
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(calls.created, [("ui".to_string(), "base".to_string())]);
    assert_eq!(calls.rebased, [("api".to_string(), "ui".to_string())]);
    assert_eq!(
        calls.rebuilds,
        [vec![
            "base".to_string(),
            "ui".to_string(),
            "api".to_string()
        ]]
    );
}

#[test]
fn inserting_next_to_a_branch_that_is_not_there_changes_nothing() {
    // The branch was removed elsewhere, or the state was hand-edited. Either way
    // the answer has to be an error rather than a branch built on a base that
    // does not exist.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api")]));
    let result = apply(
        &repo,
        WorkspaceEdit::InsertRelativeTo {
            name: "ui".into(),
            anchor: "gone".into(),
            below: false,
        },
    );
    assert!(result.is_err(), "a missing anchor has to be refused");

    let calls = repo.calls();
    assert!(calls.created.is_empty(), "nothing may be created on a base that does not exist");
    assert!(calls.writes.is_empty(), "a refused edit must not write the state");
    assert!(calls.rebuilds.is_empty());
}

#[test]
fn unapplying_a_branch_rebuilds_the_workspace_without_it() {
    // Unapplying is what actually takes a branch's changes back out of the
    // working directory, so the rebuild is the whole point of the edit.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api"), applied("ui")]));
    apply(
        &repo,
        WorkspaceEdit::SetApplied {
            name: "ui".into(),
            applied: BranchApplyState::Unapplied,
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(calls.rebuilds, [vec!["api".to_string()]]);
    assert!(
        calls.rebased.is_empty(),
        "unapplying moves no commits; it only changes what is merged"
    );
    assert!(
        repo.last_write().get("ui").is_some(),
        "the branch still exists, it just no longer contributes"
    );
}

#[test]
fn stacking_a_branch_replays_it_onto_its_new_base() {
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api"), applied("ui")]));
    apply(
        &repo,
        WorkspaceEdit::SetParent {
            name: "ui".into(),
            parent: Some("api".into()),
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(calls.rebased, [("ui".to_string(), "api".to_string())]);
    assert_eq!(
        calls.rebuilds,
        [vec!["api".to_string(), "ui".to_string()]],
        "a parent is merged into the workspace before its children"
    );
}

#[test]
fn detaching_a_branch_replays_it_onto_the_target() {
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api"), stacked("ui", "api")]));
    apply(
        &repo,
        WorkspaceEdit::SetParent {
            name: "ui".into(),
            parent: None,
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(
        calls.rebased,
        [("ui".to_string(), "main".to_string())],
        "an independent branch is based on the target"
    );
    assert_eq!(calls.rebuilds, [vec!["api".to_string(), "ui".to_string()]]);
}

#[test]
fn moving_into_another_stack_rebases_onto_the_new_base() {
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api"), applied("ui")]));
    apply(
        &repo,
        WorkspaceEdit::MoveToStack {
            name: "ui".into(),
            stack_base: "api".into(),
            relative_to: Some("api".into()),
            below: true,
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(calls.rebased, [("ui".to_string(), "api".to_string())]);
    assert_eq!(
        calls.rebuilds,
        [vec!["api".to_string(), "ui".to_string()]]
    );
}

#[test]
fn a_move_that_leaves_the_base_alone_does_not_rewrite_anything() {
    // Moving a branch to where it already is is what a drag produces when the
    // row does not actually move. Replaying it anyway would rewrite commits
    // that did not need rewriting, for a gesture that changed nothing.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api"), stacked("ui", "api")]));
    let outcome = apply(
        &repo,
        WorkspaceEdit::MoveToStack {
            name: "ui".into(),
            stack_base: "api".into(),
            relative_to: Some("api".into()),
            below: true,
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert!(calls.rebased.is_empty(), "no base moved, so no commits move");
    assert!(calls.rebuilds.is_empty(), "nothing about the tree changed");
    assert!(
        outcome.post_head.is_none(),
        "no rebuild means there is no new commit to report"
    );
}

#[test]
fn reordering_two_siblings_writes_the_state_and_stops_there() {
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("a"), applied("b")]));
    apply(
        &repo,
        WorkspaceEdit::Reorder {
            first: "a".into(),
            second: "b".into(),
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(calls.writes.len(), 1, "the new positions have to be written");
    assert!(
        calls.rebuilds.is_empty(),
        "reordering changes which commit is applied first, not what is applied"
    );
    assert_eq!(calls.reads, 1, "and it has nothing to read back afterwards");
}

#[test]
fn removing_a_branch_takes_its_work_out_without_deleting_the_branch() {
    // The user removed a branch from the workspace, not from their repository:
    // deleting the ref would throw away commits they may still want.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api"), stacked("ui", "api")]));
    apply(&repo, WorkspaceEdit::Remove { name: "ui".into() }).expect("the edit failed");

    let calls = repo.calls();
    assert!(
        calls.deleted.is_empty(),
        "removing a branch from the workspace is not a request to delete it"
    );
    assert_eq!(calls.rebuilds, [vec!["api".to_string()]]);
    assert!(repo.last_write().get("ui").is_none());
}

#[test]
fn removing_a_branch_moves_its_children_onto_its_own_base() {
    let repo = Arc::new(WorkspaceRepo::new(vec![
        applied("base"),
        stacked("api", "base"),
        stacked("ui", "api"),
    ]));
    apply(&repo, WorkspaceEdit::Remove { name: "api".into() }).expect("the edit failed");

    assert_eq!(
        repo.last_write()
            .get("ui")
            .and_then(|branch| branch.parent.clone())
            .as_deref(),
        Some("base"),
        "the stack must not develop a hole where the removed branch was"
    );
}

#[test]
fn a_refused_edit_writes_nothing_and_rebuilds_nothing() {
    // The stored state must keep describing what Git actually has. Writing a
    // half-applied edit would leave it claiming a branch exists that does not.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api")]));
    let result = apply(&repo, WorkspaceEdit::Remove { name: "gone".into() });
    assert!(result.is_err());

    let calls = repo.calls();
    assert!(calls.writes.is_empty());
    assert!(calls.rebuilds.is_empty());
    let stored = repo.stored();
    assert_eq!(
        stored.branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        ["api"],
        "a refused edit has to leave the stored state exactly as it was"
    );
}

#[test]
fn pointing_the_workspace_at_another_target_is_the_backends_job() {
    // The backend owns the move — it has to rebase every independent branch —
    // so the effect reads the state back rather than editing its own copy, and
    // that read-back is the second of the three reads.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api")]));
    let outcome = apply(
        &repo,
        WorkspaceEdit::SetTarget {
            target: "release/2.3".into(),
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(calls.targets, ["release/2.3".to_string()]);
    assert_eq!(calls.reads, 3, "one to start, one to read the move back, one to rebuild");
    assert!(
        calls.writes.is_empty(),
        "the backend wrote the target itself; writing it again would be a guess"
    );
    assert!(
        outcome.post_head.is_some(),
        "moving the target rebuilds the workspace branch"
    );
}

#[test]
fn committing_paths_goes_to_the_branch_and_does_not_rebuild() {
    // A commit to a virtual branch is not a change to the applied set, so the
    // workspace branch has to stay exactly where it is.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api")]));
    let outcome = apply(
        &repo,
        WorkspaceEdit::CommitPaths {
            name: "api".into(),
            message: "add the client".into(),
            paths: vec![PathBuf::from("src/api.rs")],
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert_eq!(
        calls.committed,
        [(
            "api".to_string(),
            "add the client".to_string(),
            vec![PathBuf::from("src/api.rs")],
        )]
    );
    assert!(calls.rebuilds.is_empty());
    assert_eq!(
        outcome.post_head.map(|id| id.0),
        Some("api-commit".to_string()),
        "the commit is on the branch, not on the workspace"
    );
}

#[test]
fn assigning_a_file_never_reaches_git_at_all() {
    // A file assignment is bookkeeping. Rebuilding the workspace branch for it
    // would rewrite a commit for something that changes no file at all, and it
    // would move HEAD under a user who only clicked a row.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api")]));
    let outcome = apply(
        &repo,
        WorkspaceEdit::AssignFile {
            path: PathBuf::from("src/api.rs"),
            branch: Some("api".into()),
        },
    )
    .expect("the edit failed");

    let calls = repo.calls();
    assert!(calls.reads.is_empty(), "the assignment index does not need the workspace");
    assert!(calls.writes.is_empty());
    assert!(calls.rebuilds.is_empty());
    assert!(calls.created.is_empty());
    assert!(outcome.post_head.is_none());
}

#[test]
fn assigning_a_hunk_is_also_only_bookkeeping() {
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api")]));
    apply(
        &repo,
        WorkspaceEdit::AssignHunk {
            path: PathBuf::from("src/api.rs"),
            hunk: gitcomet_core::workspace::HunkFingerprint::of(&["NEW\n"], 1),
            branch: Some("api".into()),
        },
    )
    .expect("the edit failed");

    assert!(repo.calls().rebuilds.is_empty());
}

#[test]
fn assigning_a_file_writes_it_through_the_index() {
    // The drag has to survive a restart, so the effect owns the write; the
    // reducer only mirrors the result into the store.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api")]));
    let path = PathBuf::from("src/api.rs");
    let msg = schedule(
        &repos_with(repo.clone()),
        Effect::AssignWorkspaceFile {
            repo_id: REPO_ID,
            path: path.clone(),
            hunk: None,
            branch: Some("api".into()),
        },
    );
    assert!(
        matches!(
            msg,
            Msg::Internal(crate::msg::InternalMsg::WorkspaceAssignFinished {
                repo_id: REPO_ID,
                result: Ok(()),
                ..
            })
        ),
        "the drag has to report back or the row never updates, got {msg:?}"
    );

    let stored = repo
        .assignments
        .lock()
        .expect("assignments poisoned")
        .clone();
    let state = WorkspaceState::new("main").with_branches(vec![applied("api")]);
    assert_eq!(stored.resolve(&path, &state), Some("api"));
}

#[test]
fn assigning_one_hunk_splits_the_file_it_belongs_to() {
    // Assigning a hunk is a split: the file stops being on the branch that
    // owned it as a whole, and the other hunks are not silently dragged along.
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api"), applied("ui")]));
    let path = PathBuf::from("src/api.rs");
    let hunk = gitcomet_core::workspace::HunkFingerprint::of(&["NEW\n"], 1);

    for branch in ["api", "ui"] {
        let msg = schedule(
            &repos_with(repo.clone()),
            Effect::AssignWorkspaceFile {
                repo_id: REPO_ID,
                path: path.clone(),
                hunk: Some(hunk),
                branch: Some(branch.into()),
            },
        );
        assert!(
            matches!(msg, Msg::Internal(crate::msg::InternalMsg::WorkspaceAssignFinished { result: Ok(()), .. })),
            "assigning to '{branch}' failed: {msg:?}"
        );
    }

    let stored = repo
        .assignments
        .lock()
        .expect("assignments poisoned")
        .clone();
    let file = stored.file(&path).expect("the file has no entry at all");
    assert_eq!(
        file.branch(),
        None,
        "the second hunk assignment split the file; it is on no single branch"
    );
    assert_eq!(
        file.hunk_branch(hunk),
        Some("ui"),
        "the later assignment is the one that stands"
    );
}

#[test]
fn assigning_a_file_whole_replaces_the_split_it_had() {
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api"), applied("ui")]));
    let path = PathBuf::from("src/api.rs");
    let hunk = gitcomet_core::workspace::HunkFingerprint::of(&["NEW\n"], 1);

    schedule(
        &repos_with(repo.clone()),
        Effect::AssignWorkspaceFile {
            repo_id: REPO_ID,
            path: path.clone(),
            hunk: Some(hunk),
            branch: Some("api".into()),
        },
    );
    schedule(
        &repos_with(repo.clone()),
        Effect::AssignWorkspaceFile {
            repo_id: REPO_ID,
            path: path.clone(),
            hunk: None,
            branch: Some("ui".into()),
        },
    );

    let stored = repo
        .assignments
        .lock()
        .expect("assignments poisoned")
        .clone();
    let file = stored.file(&path).expect("the file has no entry at all");
    assert_eq!(file.branch(), Some("ui"));
    assert!(
        file.hunks().next().is_none(),
        "assigning the whole file has to clear the hunk assignments, or the \
         commit for it would take only part of itself"
    );
}

#[test]
fn pushing_a_virtual_branch_reports_the_branch_it_pushed() {
    let repo = Arc::new(WorkspaceRepo::new(vec![applied("api")]));
    let msg = schedule(
        &repos_with(repo.clone()),
        Effect::PushWorkspaceBranch {
            repo_id: REPO_ID,
            name: "api".into(),
        },
    );
    assert!(
        matches!(
            msg,
            Msg::Internal(crate::msg::InternalMsg::WorkspacePushFinished {
                repo_id: REPO_ID,
                name: ref name,
                result: Ok(_),
            } if name == "api"
        ),
        "the row has to know which push finished, got {msg:?}"
    );
    assert_eq!(repo.calls().pushed, ["api".to_string()]);
}

#[test]
fn an_edit_for_a_repository_that_is_gone_is_reported_not_dropped() {
    // The repository was closed between the click and the worker picking the
    // task up. The reply still has to come back, or the busy flag the reducer
    // set on the way in is never cleared.
    let msg = schedule(
        &FxHashMap::default(),
        Effect::ApplyWorkspaceEdit {
            repo_id: RepoId(404),
            edit: WorkspaceEdit::Create {
                name: "api".into(),
            },
        },
    );
    assert!(
        matches!(
            msg,
            Msg::Internal(crate::msg::InternalMsg::WorkspaceEditFinished {
                repo_id: RepoId(404),
                result: Err(_),
                ..
            })
        ),
        "a missing repository has to be reported through the same channel"
    );
}

#[test]
fn an_assignment_for_a_repository_that_is_gone_is_reported_not_dropped() {
    let msg = schedule(
        &FxHashMap::default(),
        Effect::AssignWorkspaceFile {
            repo_id: RepoId(404),
            path: PathBuf::from("src/api.rs"),
            hunk: None,
            branch: Some("api".into()),
        },
    );
    assert!(
        matches!(
            msg,
            Msg::Internal(crate::msg::InternalMsg::WorkspaceAssignFinished {
                repo_id: RepoId(404),
                result: Err(_),
                ..
            })
        ),
        "the row the user just dragged has to be told it did not stick"
    );
}