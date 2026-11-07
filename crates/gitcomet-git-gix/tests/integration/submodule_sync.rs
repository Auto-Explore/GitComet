//! First-level submodule behaviour: parent detection, drift and inner-change
//! facts, and keeping a submodule's checkout in step with its pointer.

use super::submodules::{
    add_submodule_raw, git_command, git_stdout, init_repo_with_seed, run_git, setup_submodule_test,
};
use gitcomet_core::domain::{CommitId, SubmoduleCheckoutPolicy, SubmoduleDrift, SubmoduleStatus};
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
fn discarding_a_moved_checkout_keeps_the_staged_pointer() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let staged = commit_in(&fixture.child(), "a.txt", "a\n");
    run_git(&fixture.parent, &["add", "mods/child"]);
    commit_in(&fixture.child(), "b.txt", "b\n");
    let parent = fixture.open(&fixture.parent);

    // The discard names the path only, as for a file: the checkout goes back
    // to the index, which keeps its staged pointer (`git checkout -- <file>`).
    let outcome = discard(&parent, "mods/child", SubmoduleCheckoutPolicy::Guarded);
    assert!(!outcome.is_blocked(), "both commits stay on master");
    assert_eq!(recorded_in_index(&fixture.parent, "mods/child"), staged);
    assert_eq!(head_of(&fixture.child()), staged);
    let status = parent.status(&CancellationToken::new()).expect("status");
    assert_eq!(
        status
            .staged
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>(),
        vec![PathBuf::from("mods/child")],
        "the staged pointer stays staged"
    );
    assert!(status.unstaged.is_empty(), "{status:?}");
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
fn a_failed_checkout_after_stashing_gives_the_changes_back() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let child = fixture.child();
    commit_in(&child, "extra.txt", "theirs\n");
    commit_all(&fixture.parent, "record extra");
    run_git(&child, &["rm", "-q", "extra.txt"]);
    commit_all(&child, "drop extra");
    let moved = head_of(&child);
    // The recorded commit tracks extra.txt, which is untracked here now, so
    // the checkout refuses to overwrite it after the stash took the edit.
    fs::write(child.join("extra.txt"), "mine\n").unwrap();
    fs::write(child.join("child.txt"), "edited\n").unwrap();
    let parent = fixture.open(&fixture.parent);

    let error = parent
        .discard_worktree_changes_with_submodules(
            &[Path::new("mods/child")],
            SubmoduleCheckoutPolicy::StashAndUpdate,
            RemoteUrlPolicy::default(),
        )
        .expect_err("the checkout cannot overwrite the untracked file");
    assert!(
        !error.to_string().contains("stash@{0}"),
        "the changes came back, so the error does not point at the stash: {error}"
    );
    assert_eq!(head_of(&child), moved, "the checkout did not move");
    assert_eq!(
        fs::read_to_string(child.join("child.txt")).unwrap(),
        "edited\n",
        "the stashed edit is back"
    );
    assert_eq!(
        fs::read_to_string(child.join("extra.txt")).unwrap(),
        "mine\n"
    );
    assert_eq!(git_stdout(&child, &["stash", "list"]), "");
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

#[test]
fn recording_a_submodule_commit_leaves_other_staged_files_staged() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    run_git(&fixture.parent, &["config", "commit.gpgsign", "false"]);
    let moved = commit_in(&fixture.child(), "a.txt", "a\n");
    fs::write(fixture.parent.join("parent.txt"), "staged\n").unwrap();
    run_git(&fixture.parent, &["add", "parent.txt"]);
    let parent = fixture.open(&fixture.parent);

    let outcome = parent
        .record_submodule_commit(Path::new("mods/child"), "Update mods/child", None)
        .expect("record");
    assert_ne!(outcome.pre_head, outcome.post_head);
    assert_eq!(
        git_stdout(&fixture.parent, &["rev-parse", "HEAD:mods/child"]),
        moved
    );
    assert_eq!(
        git_stdout(&fixture.parent, &["diff", "--cached", "--name-only"]),
        "parent.txt",
        "the other staged change is neither committed nor unstaged"
    );
}

#[test]
fn re_recording_amends_only_while_head_is_the_recording_commit() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    run_git(&fixture.parent, &["config", "commit.gpgsign", "false"]);
    commit_in(&fixture.child(), "a.txt", "a\n");
    let parent = fixture.open(&fixture.parent);
    let recorded = parent
        .record_submodule_commit(Path::new("mods/child"), "Update mods/child", None)
        .expect("record")
        .post_head
        .expect("a commit");

    run_git(
        &fixture.child(),
        &["commit", "-q", "--amend", "-m", "rebased"],
    );
    let rebased = head_of(&fixture.child());
    let amended = parent
        .amend_submodule_record(Path::new("mods/child"), &recorded, None)
        .expect("amend");
    assert_eq!(
        git_stdout(&fixture.parent, &["rev-parse", "HEAD:mods/child"]),
        rebased
    );
    assert_eq!(
        git_stdout(&fixture.parent, &["log", "-1", "--format=%s"]),
        "Update mods/child"
    );

    commit_in(&fixture.parent, "later.txt", "later\n");
    assert!(
        parent
            .amend_submodule_record(Path::new("mods/child"), &amended.post_head.unwrap(), None)
            .is_err(),
        "a commit made since is never rewritten"
    );
}

#[test]
fn unpushed_submodule_pointers_lists_commits_missing_from_the_submodule_remote() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    run_git(&fixture.parent, &["config", "commit.gpgsign", "false"]);
    let parent = fixture.open(&fixture.parent);
    assert!(parent.unpushed_submodule_pointers().unwrap().is_empty());

    let local = commit_in(&fixture.child(), "a.txt", "a\n");
    parent
        .record_submodule_commit(Path::new("mods/child"), "Update mods/child", None)
        .expect("record");
    let unpushed = parent.unpushed_submodule_pointers().unwrap();
    assert_eq!(unpushed.len(), 1);
    assert_eq!(unpushed[0].path, PathBuf::from("mods/child"));
    assert_eq!(unpushed[0].commit.as_ref(), local.as_str());

    run_git(
        &fixture.child(),
        &["push", "-q", "origin", "HEAD:refs/heads/shared"],
    );
    run_git(&fixture.child(), &["fetch", "-q", "origin"]);
    assert!(parent.unpushed_submodule_pointers().unwrap().is_empty());
}

#[test]
fn a_signed_recording_commit_is_still_reported_unpushed() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    run_git(&fixture.parent, &["config", "commit.gpgsign", "false"]);
    let parent = fixture.open(&fixture.parent);
    let local = commit_in(&fixture.child(), "a.txt", "a\n");
    parent
        .record_submodule_commit(Path::new("mods/child"), "Update mods/child", None)
        .expect("record");

    // Wrap the recording commit in a signature header. Whether it verifies
    // does not matter: with `log.showSignature` Git prints what the verifier
    // says, and Git itself, run as the verifier, always prints its usage.
    let raw = git_stdout(&fixture.parent, &["cat-file", "commit", "HEAD"]);
    let (headers, message) = raw.split_once("\n\n").expect("commit headers");
    let signed_raw = format!(
        "{headers}\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n iQEzBAABCAAdFiEE\n -----END PGP SIGNATURE-----\n\n{message}\n"
    );
    let mut cmd = git_command();
    cmd.arg("-C")
        .arg(&fixture.parent)
        .args(["hash-object", "-t", "commit", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut hash = cmd.spawn().expect("spawn git hash-object");
    {
        use std::io::Write as _;
        hash.stdin
            .as_mut()
            .expect("stdin")
            .write_all(signed_raw.as_bytes())
            .expect("write commit object");
    }
    let output = hash.wait_with_output().expect("hash-object output");
    assert!(output.status.success(), "git hash-object failed");
    let signed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    run_git(&fixture.parent, &["update-ref", "HEAD", &signed]);
    run_git(&fixture.parent, &["config", "log.showSignature", "true"]);
    run_git(&fixture.parent, &["config", "gpg.program", "git"]);

    let unpushed = parent.unpushed_submodule_pointers().unwrap();
    assert_eq!(unpushed.len(), 1, "{unpushed:?}");
    assert_eq!(unpushed[0].commit.as_ref(), local.as_str());
}

#[test]
fn a_submodule_pinned_to_a_tag_is_not_reported_unpushed() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    run_git(&fixture.parent, &["config", "commit.gpgsign", "false"]);
    let parent = fixture.open(&fixture.parent);
    commit_in(&fixture.child(), "a.txt", "a\n");
    parent
        .record_submodule_commit(Path::new("mods/child"), "Update mods/child", None)
        .expect("record");
    assert_eq!(parent.unpushed_submodule_pointers().unwrap().len(), 1);

    run_git(&fixture.child(), &["tag", "v1"]);
    assert!(parent.unpushed_submodule_pointers().unwrap().is_empty());
}

#[test]
fn pushing_a_detached_submodule_pushes_the_branch_that_holds_the_commit() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let child = fixture.child();
    run_git(&child, &["checkout", "-q", "--detach"]);
    // A non-bare origin refuses pushes to its checked-out branch, so the
    // branch pushes to a branch of its own there.
    run_git(&child, &["push", "-q", "origin", "HEAD:refs/heads/topic"]);
    run_git(&child, &["fetch", "-q", "origin"]);
    run_git(&child, &["branch", "--track", "topic", "origin/topic"]);
    let local = commit_in(&child, "a.txt", "a\n");
    run_git(&child, &["update-ref", "refs/heads/topic", &local]);
    assert_eq!(
        git_stdout(&child, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "HEAD"
    );

    let parent = fixture.open(&fixture.parent);
    parent
        .push_submodule_with_output(Path::new("mods/child"), &CommitId(local.as_str().into()))
        .expect("push the branch that holds the commit");
    assert_eq!(
        git_stdout(
            &fixture.root.join("child"),
            &["rev-parse", "refs/heads/topic"]
        ),
        local
    );
}

#[test]
fn pushing_a_submodule_commit_no_branch_holds_says_what_to_do() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    run_git(&fixture.child(), &["checkout", "-q", "--detach"]);
    let local = commit_in(&fixture.child(), "a.txt", "a\n");
    let parent = fixture.open(&fixture.parent);
    let error = parent
        .push_submodule_with_output(Path::new("mods/child"), &CommitId(local.as_str().into()))
        .expect_err("no branch holds the commit");
    assert!(
        error.to_string().contains("No branch with an upstream"),
        "{error}"
    );
}

#[test]
fn pushing_a_submodule_skips_a_branch_tracking_a_local_branch() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let child = fixture.child();
    let master = head_of(&child);
    run_git(&child, &["checkout", "-q", "-b", "feature"]);
    run_git(&child, &["config", "branch.feature.remote", "."]);
    run_git(
        &child,
        &["config", "branch.feature.merge", "refs/heads/master"],
    );
    let local = commit_in(&child, "a.txt", "a\n");

    let parent = fixture.open(&fixture.parent);
    let error = parent
        .push_submodule_with_output(Path::new("mods/child"), &CommitId(local.as_str().into()))
        .expect_err("a local upstream is no remote to push to");
    assert!(
        error.to_string().contains("No branch with an upstream"),
        "{error}"
    );
    assert_eq!(
        git_stdout(&child, &["rev-parse", "refs/heads/master"]),
        master,
        "the local branch it tracks did not move"
    );
}

#[test]
fn shelving_a_submodule_stashes_tracked_changes_and_leaves_untracked_files() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let child = fixture.child();
    fs::write(child.join("child.txt"), "changed\n").expect("write tracked change");
    fs::write(child.join("local.cfg"), "mine\n").expect("write untracked file");

    let parent = fixture.open(&fixture.parent);
    parent
        .stash_submodule_changes(Path::new("mods/child"), "Shelved while committing parent")
        .expect("stash");
    assert_eq!(
        fs::read_to_string(child.join("child.txt")).unwrap(),
        "child\n"
    );
    assert!(child.join("local.cfg").exists(), "untracked files stay");
    assert!(git_stdout(&child, &["stash", "list"]).contains("Shelved while committing parent"));
}

#[test]
fn a_parent_opens_its_submodule_to_stage_and_commit_inside_it() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let parent = fixture.open(&fixture.parent);
    fs::write(fixture.child().join("a.txt"), "a\n").unwrap();
    fs::write(fixture.child().join("b.txt"), "b\n").unwrap();

    let child = parent
        .open_submodule(Path::new("mods/child"))
        .expect("open the checked-out submodule");
    assert_eq!(
        canonicalize_or_original(child.spec().workdir.clone()),
        fixture.child()
    );
    child.stage(&[Path::new("a.txt")]).expect("stage inside");
    let status = child.status(&CancellationToken::new()).expect("status");
    assert_eq!(
        status
            .staged
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>(),
        vec![PathBuf::from("a.txt")]
    );
    assert!(
        status
            .unstaged
            .iter()
            .any(|entry| entry.path == Path::new("b.txt"))
    );

    let before = head_of(&fixture.child());
    child
        .commit_with_identity("Add a", None)
        .expect("commit inside the submodule");
    assert_ne!(head_of(&fixture.child()), before);
    assert_eq!(
        git_stdout(&fixture.child(), &["log", "-1", "--format=%s"]),
        "Add a"
    );

    for outside in ["", "../child", "/tmp", "mods/../.."] {
        assert!(
            parent.open_submodule(Path::new(outside)).is_err(),
            "{outside:?} is not a submodule of this worktree"
        );
    }
    assert!(
        parent.open_submodule(Path::new("missing")).is_err(),
        "a path without a checkout is not opened"
    );
}

#[cfg(unix)]
#[test]
fn open_submodule_refuses_symlinks_and_non_submodule_repos() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    // An untracked repository inside the worktree.
    let scratch = fixture.parent.join("scratch");
    fs::create_dir_all(&scratch).unwrap();
    init_repo_with_seed(&scratch, "scratch.txt", "scratch\n", "seed scratch");
    // A gitlink whose path runs through a symlink to a repository outside
    // the worktree.
    let outside = head_of(&fixture.root.join("child"));
    run_git(
        &fixture.parent,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{outside},link/child"),
        ],
    );
    std::os::unix::fs::symlink(&fixture.root, fixture.parent.join("link")).unwrap();
    assert!(fixture.parent.join("link/child/.git").exists());

    let parent = fixture.open(&fixture.parent);
    assert!(
        parent.open_submodule(Path::new("scratch")).is_err(),
        "an untracked repository is not a submodule"
    );
    assert!(
        parent
            .open_submodule(Path::new("mods/child/nested/grand"))
            .is_err(),
        "a submodule of a submodule is not this worktree's"
    );
    assert!(
        parent.open_submodule(Path::new("link/child")).is_err(),
        "a checkout through a symlink lies outside the worktree"
    );
    assert!(parent.open_submodule(Path::new("mods/child")).is_ok());
}

#[test]
fn a_submodule_opened_from_its_parent_reports_its_own_unpushed_submodule_commits() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    let parent = fixture.open(&fixture.parent);
    let child = parent
        .open_submodule(Path::new("mods/child"))
        .expect("open the checked-out submodule");
    assert!(child.unpushed_submodule_pointers().unwrap().is_empty());

    // A commit in the nested submodule that its remote does not have,
    // recorded in the first-level submodule: pushing that submodule would
    // publish a pointer nobody can fetch.
    let local = commit_in(&fixture.grand(), "g.txt", "g\n");
    child
        .record_submodule_commit(Path::new("nested/grand"), "Update nested/grand", None)
        .expect("record in the submodule");
    let unpushed = child.unpushed_submodule_pointers().unwrap();
    assert_eq!(unpushed.len(), 1);
    assert_eq!(unpushed[0].path, PathBuf::from("nested/grand"));
    assert_eq!(unpushed[0].commit.as_ref(), local.as_str());
}

#[test]
fn re_recording_in_a_commit_with_other_files_keeps_them() {
    let _guard = setup_submodule_test();
    let fixture = fixture();
    run_git(&fixture.parent, &["config", "commit.gpgsign", "false"]);
    commit_in(&fixture.child(), "a.txt", "a\n");
    fs::write(fixture.parent.join("parent.txt"), "with the pointer\n").unwrap();
    let parent = fixture.open(&fixture.parent);
    parent
        .stage(&[Path::new("mods/child"), Path::new("parent.txt")])
        .expect("stage the pointer with another file");
    let recorded = parent
        .commit_with_identity("Use the new child", None)
        .expect("commit the parent")
        .post_head
        .expect("a commit");

    run_git(
        &fixture.child(),
        &["commit", "-q", "--amend", "-m", "rebased"],
    );
    let rebased = head_of(&fixture.child());
    parent
        .amend_submodule_record(Path::new("mods/child"), &recorded, None)
        .expect("amend");
    assert_eq!(
        git_stdout(&fixture.parent, &["rev-parse", "HEAD:mods/child"]),
        rebased
    );
    assert_eq!(
        git_stdout(&fixture.parent, &["show", "HEAD:parent.txt"]),
        "with the pointer",
        "the rest of the commit stays as it was"
    );
    assert_eq!(
        git_stdout(&fixture.parent, &["log", "-1", "--format=%s"]),
        "Use the new child"
    );
}
