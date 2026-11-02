//! First-level submodule behaviour: parent detection, drift and inner-change
//! facts, and keeping a submodule's checkout in step with its pointer.

use super::submodules::{
    add_submodule_raw, git_stdout, init_repo_with_seed, run_git, setup_submodule_test,
};
use gitcomet_core::domain::{SubmoduleCheckoutPolicy, SubmoduleDrift, SubmoduleStatus};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::services::{CancellationToken, GitBackend, GitRepository};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `parent` records `child` at `mods/child`; `child` records `grand` at
/// `nested/grand`. Every checkout is initialized and in sync.
pub(crate) struct Fixture {
    _dir: tempfile::TempDir,
    pub(crate) root: PathBuf,
    pub(crate) parent: PathBuf,
    /// The repository `mods/child` was cloned from: its `origin`.
    pub(crate) child_source: PathBuf,
}

impl Fixture {
    pub(crate) fn child(&self) -> PathBuf {
        self.parent.join("mods/child")
    }

    pub(crate) fn grand(&self) -> PathBuf {
        self.child().join("nested/grand")
    }

    pub(crate) fn open(&self, path: &Path) -> Arc<dyn GitRepository> {
        GixBackend.open(path).expect("open repository")
    }
}

fn commit_all(repo: &Path, message: &str) {
    run_git(repo, &["add", "-A"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", message],
    );
}

pub(crate) fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = canonicalize_or_original(dir.path().to_path_buf());
    let grand = root.join("grand");
    let child = root.join("child");
    let parent = root.join("parent");
    for repo in [&grand, &child, &parent] {
        fs::create_dir_all(repo).expect("create repository directory");
    }
    init_repo_with_seed(&grand, "grand.txt", "grand\n", "seed grand");
    init_repo_with_seed(&child, "child.txt", "child\n", "seed child");
    init_repo_with_seed(&parent, "parent.txt", "parent\n", "seed parent");

    add_submodule_raw(&child, &grand, Path::new("nested/grand"), None);
    commit_all(&child, "add grand");
    add_submodule_raw(&parent, &child, Path::new("mods/child"), None);
    commit_all(&parent, "add child");
    run_git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
            "--recursive",
        ],
    );
    for checkout in [
        parent.join("mods/child"),
        parent.join("mods/child/nested/grand"),
    ] {
        for (key, value) in [
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ] {
            run_git(&checkout, &["config", key, value]);
        }
    }
    Fixture {
        _dir: dir,
        root,
        parent,
        child_source: child,
    }
}

pub(crate) fn commit_in(repo: &Path, file: &str, contents: &str) -> String {
    fs::write(repo.join(file), contents).expect("write file");
    commit_all(repo, file);
    git_stdout(repo, &["rev-parse", "HEAD"])
}

#[test]
fn superproject_names_the_direct_parent_worktree_and_marks_deeper_levels() {
    let _guard = setup_submodule_test();
    let fixture = fixture();

    assert_eq!(fixture.open(&fixture.parent).superproject().unwrap(), None);

    let child = fixture
        .open(&fixture.child())
        .superproject()
        .unwrap()
        .expect("a first-level submodule names its worktree");
    assert_eq!(child.workdir, fixture.parent);
    assert_eq!(child.path, PathBuf::from("mods/child"));
    assert!(!child.nested);

    let grand = fixture
        .open(&fixture.grand())
        .superproject()
        .unwrap()
        .expect("a second-level submodule still names its direct parent");
    assert_eq!(grand.workdir, fixture.child());
    assert_eq!(grand.path, PathBuf::from("nested/grand"));
    assert!(grand.nested, "its parent is itself a submodule");
}

#[test]
fn superproject_follows_the_linked_worktree_that_holds_the_checkout() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let linked = fixture.root.join("parent-wt");
    run_git(
        &fixture.parent,
        &[
            "worktree",
            "add",
            "-q",
            linked.to_str().unwrap(),
            "-b",
            "side",
        ],
    );
    run_git(
        &linked,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
            "--",
            "mods/child",
        ],
    );

    let parent = fixture
        .open(&linked.join("mods/child"))
        .superproject()
        .unwrap()
        .expect("each worktree has its own checkout of the submodule");
    assert_eq!(parent.workdir, canonicalize_or_original(linked));
    assert_eq!(parent.path, PathBuf::from("mods/child"));
}

#[test]
fn a_worktree_of_a_submodule_checked_out_elsewhere_has_no_parent() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let elsewhere = fixture.root.join("child-wt");
    run_git(
        &fixture.child(),
        &[
            "worktree",
            "add",
            "-q",
            elsewhere.to_str().unwrap(),
            "-b",
            "elsewhere",
        ],
    );
    assert_eq!(fixture.open(&elsewhere).superproject().unwrap(), None);
}

#[test]
fn first_level_rows_count_drift_and_inner_changes_and_nested_rows_do_not() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let parent = fixture.open(&fixture.parent);
    let list = || {
        parent
            .list_submodules(&CancellationToken::new())
            .expect("list submodules")
    };

    let rows = list();
    assert_eq!(rows.len(), 2);
    assert!(!rows[0].nested && rows[1].nested);
    assert!(rows.iter().all(|row| !row.is_out_of_sync()));

    let recorded = rows[0].recorded_head.clone();
    commit_in(&fixture.child(), "a.txt", "a\n");
    commit_in(&fixture.child(), "b.txt", "b\n");
    let rows = list();
    assert_eq!(rows[0].status, SubmoduleStatus::HeadMismatch);
    assert_eq!(
        rows[0].drift,
        Some(SubmoduleDrift {
            ahead: 2,
            behind: 0
        })
    );
    assert!(!rows[0].has_inner_changes);

    // Recorded moves forward past the checkout: behind.
    run_git(&fixture.parent, &["add", "mods/child"]);
    run_git(
        &fixture.parent,
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "bump"],
    );
    run_git(
        &fixture.child(),
        &["checkout", "-q", "--detach", recorded.as_ref()],
    );
    let rows = list();
    assert_eq!(
        rows[0].drift,
        Some(SubmoduleDrift {
            ahead: 0,
            behind: 2
        })
    );

    fs::write(fixture.child().join("child.txt"), "edited\n").unwrap();
    fs::write(fixture.child().join("untracked.txt"), "new\n").unwrap();
    let rows = list();
    assert!(rows[0].has_inner_changes);
    fs::write(fixture.child().join("child.txt"), "child\n").unwrap();
    let rows = list();
    assert!(
        !rows[0].has_inner_changes,
        "untracked files alone are not changes a commit would take"
    );

    commit_in(&fixture.grand(), "deep.txt", "deep\n");
    let rows = list();
    let nested = rows.iter().find(|row| row.nested).unwrap();
    assert_eq!(nested.drift, None, "deeper levels keep today's facts");
    assert!(!nested.has_inner_changes);
}

fn head_of(repo: &Path) -> String {
    git_stdout(repo, &["rev-parse", "HEAD"])
}

fn recorded_in_index(parent: &Path, path: &str) -> String {
    git_stdout(parent, &["rev-parse", &format!(":{path}")])
}

fn discard(
    repo: &Arc<dyn GitRepository>,
    path: &str,
    policy: SubmoduleCheckoutPolicy,
) -> gitcomet_core::domain::DiscardOutcome {
    repo.discard_worktree_changes_with_submodules(
        &[Path::new(path)],
        policy,
        RemoteUrlPolicy::default(),
    )
    .expect("discard")
}

#[test]
fn discarding_a_moved_pointer_checks_the_submodule_out_at_the_recorded_commit() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let recorded = head_of(&fixture.child());
    commit_in(&fixture.child(), "a.txt", "a\n");
    let parent = fixture.open(&fixture.parent);

    let outcome = discard(&parent, "mods/child", SubmoduleCheckoutPolicy::Guarded);
    assert!(!outcome.is_blocked(), "the moved commit stays on master");
    assert_eq!(outcome.updated.len(), 1);
    assert_eq!(outcome.updated[0].commit.as_ref(), recorded);
    assert_eq!(head_of(&fixture.child()), recorded);
}

#[test]
fn discarding_a_staged_pointer_restores_the_index_and_the_checkout() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let recorded = head_of(&fixture.child());
    let moved = commit_in(&fixture.child(), "a.txt", "a\n");
    run_git(&fixture.parent, &["add", "mods/child"]);
    assert_eq!(recorded_in_index(&fixture.parent, "mods/child"), moved);
    let parent = fixture.open(&fixture.parent);

    let outcome = discard(&parent, "mods/child", SubmoduleCheckoutPolicy::Guarded);
    assert!(!outcome.is_blocked());
    assert_eq!(recorded_in_index(&fixture.parent, "mods/child"), recorded);
    assert_eq!(head_of(&fixture.child()), recorded);
    let status = parent.status(&CancellationToken::new()).expect("status");
    assert!(
        status.staged.is_empty() && status.unstaged.is_empty(),
        "nothing is left to discard: {status:?}"
    );
}

#[test]
fn a_dirty_submodule_blocks_the_discard_until_the_user_picks_how() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let recorded = head_of(&fixture.child());
    let moved = commit_in(&fixture.child(), "a.txt", "a\n");
    fs::write(fixture.child().join("child.txt"), "edited\n").unwrap();
    let parent = fixture.open(&fixture.parent);

    let blocked = discard(&parent, "mods/child", SubmoduleCheckoutPolicy::Guarded);
    assert_eq!(blocked.blocked.len(), 1);
    assert!(blocked.blocked[0].has_inner_changes);
    assert!(!blocked.blocked[0].unreferenced);
    assert_eq!(head_of(&fixture.child()), moved, "nothing changed");

    let kept = discard(&parent, "mods/child", SubmoduleCheckoutPolicy::KeepCheckout);
    assert!(kept.updated.is_empty());
    assert_eq!(head_of(&fixture.child()), moved);
    assert_eq!(
        fs::read_to_string(fixture.child().join("child.txt")).unwrap(),
        "edited\n"
    );

    let stashed = discard(
        &parent,
        "mods/child",
        SubmoduleCheckoutPolicy::StashAndUpdate,
    );
    assert_eq!(stashed.stashed, vec![PathBuf::from("mods/child")]);
    assert_eq!(head_of(&fixture.child()), recorded);
    assert_eq!(
        fs::read_to_string(fixture.child().join("child.txt")).unwrap(),
        "child\n"
    );
    let stash = git_stdout(&fixture.child(), &["stash", "list"]);
    assert!(
        stash.contains(&format!("Before checking out {} to match", &recorded[..7])),
        "{stash}"
    );
}

#[test]
fn a_commit_on_no_branch_is_kept_on_a_branch_before_the_checkout_moves() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let recorded = head_of(&fixture.child());
    run_git(&fixture.child(), &["checkout", "-q", "--detach"]);
    let orphan = commit_in(&fixture.child(), "a.txt", "a\n");
    let parent = fixture.open(&fixture.parent);

    let blocked = discard(&parent, "mods/child", SubmoduleCheckoutPolicy::Guarded);
    assert_eq!(blocked.blocked.len(), 1);
    assert!(blocked.blocked[0].unreferenced);

    let kept = discard(
        &parent,
        "mods/child",
        SubmoduleCheckoutPolicy::StashAndUpdate,
    );
    assert_eq!(kept.kept_on_branch.len(), 1);
    let branch = &kept.kept_on_branch[0].1;
    assert_eq!(
        git_stdout(&fixture.child(), &["rev-parse", branch.as_str()]),
        orphan
    );
    assert_eq!(head_of(&fixture.child()), recorded);
}

#[test]
fn update_to_recorded_moves_only_the_checkout() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let recorded = head_of(&fixture.child());
    commit_in(&fixture.child(), "a.txt", "a\n");
    let parent = fixture.open(&fixture.parent);

    let outcome = parent
        .update_submodule_to_recorded(Path::new("mods/child"), SubmoduleCheckoutPolicy::Guarded)
        .expect("update");
    assert_eq!(outcome.updated.len(), 1);
    assert_eq!(head_of(&fixture.child()), recorded);
}

#[test]
fn submodules_in_sync_follow_their_pointer_after_a_parent_checkout() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let original = head_of(&fixture.child());
    run_git(&fixture.parent, &["checkout", "-q", "-b", "side"]);
    let moved = commit_in(&fixture.child(), "a.txt", "a\n");
    commit_all(&fixture.parent, "bump child");
    run_git(&fixture.parent, &["checkout", "-q", "master"]);
    // Git leaves the checkout behind: it is now out of sync.
    assert_eq!(head_of(&fixture.child()), moved);
    run_git(
        &fixture.child(),
        &["checkout", "-q", "--detach", original.as_str()],
    );

    let parent = fixture.open(&fixture.parent);
    let snapshot = parent
        .submodule_pointer_snapshot()
        .unwrap()
        .expect("the submodule is in sync on master");
    run_git(&fixture.parent, &["checkout", "-q", "side"]);
    let updates = parent.follow_submodule_pointers(&snapshot).unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(head_of(&fixture.child()), moved);

    // With `submodule.recurse` Git already does it; GitComet stays out.
    run_git(&fixture.parent, &["config", "submodule.recurse", "true"]);
    assert_eq!(parent.submodule_pointer_snapshot().unwrap(), None);
}

#[test]
fn a_dirty_submodule_does_not_follow_its_pointer() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let original = head_of(&fixture.child());
    run_git(&fixture.parent, &["checkout", "-q", "-b", "side"]);
    commit_in(&fixture.child(), "a.txt", "a\n");
    commit_all(&fixture.parent, "bump child");
    run_git(&fixture.parent, &["checkout", "-q", "master"]);
    run_git(
        &fixture.child(),
        &["checkout", "-q", "--detach", original.as_str()],
    );
    let parent = fixture.open(&fixture.parent);
    let snapshot = parent.submodule_pointer_snapshot().unwrap().unwrap();
    fs::write(fixture.child().join("child.txt"), "edited\n").unwrap();
    run_git(&fixture.parent, &["checkout", "-q", "side"]);
    assert!(
        parent
            .follow_submodule_pointers(&snapshot)
            .unwrap()
            .is_empty()
    );
    assert_eq!(head_of(&fixture.child()), original);
}
