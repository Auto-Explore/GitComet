//! A branch and a tag (or another ref) can share a name. Git resolves a short
//! name by trying `<name>`, `refs/<name>`, `refs/tags/<name>` and only then
//! `refs/heads/<name>`, so a short name the UI shows as a branch can make Git
//! act on the tag instead. A server controls the tags and branch names it
//! offers, so these are not only self-inflicted.

use crate::git_cli::{git_cmd, git_output, run_checked, stdout_text};
use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(repo: &Path) -> Command {
    let mut cmd = git_cmd(repo);
    // Local bare remotes need the file protocol.
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd
}

fn run(repo: &Path, args: &[&str]) {
    run_checked(&mut git(repo), args);
}

fn capture(repo: &Path, args: &[&str]) -> String {
    stdout_text(run_checked(&mut git(repo), args))
        .trim()
        .to_string()
}

fn init(repo: &Path) {
    fs::create_dir_all(repo).expect("create repository directory");
    run(repo, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.email", "you@example.com"),
        ("user.name", "You"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        ("core.autocrlf", "false"),
        ("core.eol", "lf"),
    ] {
        run(repo, &["config", key, value]);
    }
}

fn commit_file(repo: &Path, name: &str, message: &str) -> String {
    fs::write(repo.join(name), format!("{message}\n")).expect("write file");
    run(repo, &["add", "--", name]);
    run(repo, &["commit", "-q", "-m", message]);
    capture(repo, &["rev-parse", "HEAD"])
}

fn ref_exists(repo: &Path, name: &str) -> bool {
    git_output(repo, &["show-ref", "--verify", "--quiet", name])
        .status
        .success()
}

struct BranchAndTag {
    branch_tip: String,
    tag_target: String,
}

/// `main` holds `base.txt`. Branch `topic` adds `topic.txt`; tag `topic`
/// points at an unrelated commit on `decoy` that adds `decoy.txt`.
fn branch_and_tag_named_topic(repo: &Path) -> BranchAndTag {
    init(repo);
    commit_file(repo, "base.txt", "base");
    run(repo, &["checkout", "-q", "-b", "decoy"]);
    let tag_target = commit_file(repo, "decoy.txt", "decoy");
    run(repo, &["tag", "topic"]);
    run(repo, &["checkout", "-q", "main"]);
    run(repo, &["checkout", "-q", "-b", "topic"]);
    let branch_tip = commit_file(repo, "topic.txt", "topic");
    run(repo, &["checkout", "-q", "main"]);
    BranchAndTag {
        branch_tip,
        tag_target,
    }
}

fn is_ancestor(repo: &Path, ancestor: &str, descendant: &str) -> bool {
    git_output(repo, &["merge-base", "--is-ancestor", ancestor, descendant])
        .status
        .success()
}

#[test]
fn merge_of_a_local_branch_merges_the_branch_not_a_same_named_tag() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let refs = branch_and_tag_named_topic(&repo);

    let opened = GixBackend.open(&repo).expect("open repository");
    opened
        .merge_ref_with_output("topic")
        .expect("merge local branch topic");

    let head = capture(&repo, &["rev-parse", "HEAD"]);
    assert!(
        is_ancestor(&repo, &refs.branch_tip, &head),
        "the branch `topic` was not merged"
    );
    assert!(
        !is_ancestor(&repo, &refs.tag_target, &head),
        "the tag `topic` was merged instead of the branch"
    );
}

#[test]
fn merge_of_an_unambiguous_branch_keeps_the_short_name_in_the_message() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init(&repo);
    commit_file(&repo, "base.txt", "base");
    run(&repo, &["checkout", "-q", "-b", "topic"]);
    commit_file(&repo, "topic.txt", "topic");
    run(&repo, &["checkout", "-q", "main"]);
    commit_file(&repo, "main.txt", "main moved");

    let opened = GixBackend.open(&repo).expect("open repository");
    opened
        .merge_ref_with_output("topic")
        .expect("merge local branch topic");

    assert_eq!(
        capture(&repo, &["log", "-1", "--format=%s"]),
        "Merge branch 'topic'"
    );
}

#[test]
fn squash_of_a_local_branch_squashes_the_branch_not_a_same_named_tag() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    branch_and_tag_named_topic(&repo);

    let opened = GixBackend.open(&repo).expect("open repository");
    opened
        .squash_ref_with_output("topic")
        .expect("squash local branch topic");

    assert_eq!(
        capture(&repo, &["diff", "--cached", "--name-only"]),
        "topic.txt",
        "the squash staged the tag's changes instead of the branch's"
    );
}

#[test]
fn rebase_onto_a_local_branch_uses_the_branch_not_a_same_named_tag() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let refs = branch_and_tag_named_topic(&repo);
    run(&repo, &["checkout", "-q", "-b", "feature"]);
    commit_file(&repo, "feature.txt", "feature");

    let opened = GixBackend.open(&repo).expect("open repository");
    opened
        .rebase_with_output("topic")
        .expect("rebase onto local branch topic");

    let head = capture(&repo, &["rev-parse", "HEAD"]);
    assert!(
        is_ancestor(&repo, &refs.branch_tip, &head),
        "feature was not rebased onto the branch `topic`"
    );
    assert!(
        !is_ancestor(&repo, &refs.tag_target, &head),
        "feature was rebased onto the tag `topic`"
    );
}

#[test]
fn checkout_of_a_missing_branch_does_not_revert_a_file_with_that_name() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init(&repo);
    commit_file(&repo, "notes", "base");
    fs::write(repo.join("notes"), "unsaved local edit\n").expect("edit file");

    let opened = GixBackend.open(&repo).expect("open repository");
    let result = opened.checkout_branch("notes");

    assert_eq!(
        fs::read_to_string(repo.join("notes")).expect("read file"),
        "unsaved local edit\n",
        "checking out a branch that does not exist discarded edits to a file of that name"
    );
    assert!(
        result.is_err(),
        "checking out a branch that does not exist must fail"
    );
}

#[test]
fn checkout_still_creates_a_local_branch_from_a_remote_tracking_branch() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let server = dir.path().join("server.git");
    let work = dir.path().join("work");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&server).expect("create server directory");
    run(&server, &["init", "-q", "--bare"]);
    init(&work);
    commit_file(&work, "base.txt", "base");
    run(
        &work,
        &["remote", "add", "origin", &server.to_string_lossy()],
    );
    run(&work, &["push", "-q", "origin", "main", "main:remote-only"]);
    run(
        dir.path(),
        &[
            "clone",
            "-q",
            &server.to_string_lossy(),
            &clone.to_string_lossy(),
        ],
    );

    let opened = GixBackend.open(&clone).expect("open clone");
    opened
        .checkout_branch("remote-only")
        .expect("check out a branch that exists only on the remote");

    assert_eq!(
        capture(&clone, &["symbolic-ref", "--short", "HEAD"]),
        "remote-only"
    );
}

struct ServerAndClone {
    server: std::path::PathBuf,
    work: std::path::PathBuf,
}

/// A bare server with `main`, and a working repository that has fetched it.
fn server_and_work(root: &Path) -> ServerAndClone {
    let server = root.join("server.git");
    let work = root.join("work");
    fs::create_dir_all(&server).expect("create server directory");
    run(&server, &["init", "-q", "--bare"]);
    init(&work);
    commit_file(&work, "base.txt", "base");
    run(
        &work,
        &["remote", "add", "origin", &server.to_string_lossy()],
    );
    run(&work, &["push", "-q", "-u", "origin", "main"]);
    ServerAndClone { server, work }
}

#[test]
fn deleting_a_stale_remote_branch_does_not_delete_a_same_named_remote_tag() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let ServerAndClone { server, work } = server_and_work(dir.path());
    run(&work, &["push", "-q", "origin", "main:refs/heads/topic"]);
    run(&work, &["tag", "topic"]);
    run(&work, &["push", "-q", "origin", "refs/tags/topic"]);
    run(&work, &["fetch", "-q", "origin"]);
    // The branch is deleted on the server; this clone has not pruned yet.
    run(&server, &["update-ref", "-d", "refs/heads/topic"]);
    assert!(ref_exists(&work, "refs/remotes/origin/topic"));

    let opened = GixBackend.open(&work).expect("open repository");
    let _ = opened.delete_remote_branch_with_output("origin", "topic");

    assert!(
        ref_exists(&server, "refs/tags/topic"),
        "deleting the stale remote branch `topic` deleted the server's tag `topic`"
    );
}

#[test]
fn deleting_a_stale_remote_branch_succeeds_and_prunes_its_tracking_ref() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let ServerAndClone { server, work } = server_and_work(dir.path());
    run(&work, &["push", "-q", "origin", "main:refs/heads/stale"]);
    run(&work, &["fetch", "-q", "origin"]);
    run(&server, &["update-ref", "-d", "refs/heads/stale"]);

    let opened = GixBackend.open(&work).expect("open repository");
    opened
        .delete_remote_branch_with_output("origin", "stale")
        .expect("a branch that is already gone counts as deleted");

    assert!(
        !ref_exists(&work, "refs/remotes/origin/stale"),
        "the stale tracking ref should be pruned"
    );
}

#[test]
fn deleting_stale_remote_branches_in_a_batch_does_not_delete_a_same_named_remote_tag() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let ServerAndClone { server, work } = server_and_work(dir.path());
    run(&work, &["push", "-q", "origin", "main:refs/heads/topic"]);
    run(&work, &["tag", "topic"]);
    run(&work, &["push", "-q", "origin", "refs/tags/topic"]);
    run(&work, &["fetch", "-q", "origin"]);
    run(&server, &["update-ref", "-d", "refs/heads/topic"]);

    let opened = GixBackend.open(&work).expect("open repository");
    let _ = opened.delete_remote_branches_with_output("origin", &["topic".to_string()]);

    assert!(
        ref_exists(&server, "refs/tags/topic"),
        "deleting the stale remote branch `topic` deleted the server's tag `topic`"
    );
}

#[test]
fn deleting_a_tag_shaped_remote_branch_does_not_delete_the_release_tag() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let ServerAndClone { server, work } = server_and_work(dir.path());
    run(&work, &["tag", "-a", "-m", "release", "v1.0"]);
    run(&work, &["push", "-q", "origin", "refs/tags/v1.0"]);
    // Anyone who can push a branch can name it `refs/heads/refs/tags/v1.0`.
    run(
        &work,
        &["push", "-q", "origin", "main:refs/heads/refs/tags/v1.0"],
    );
    run(&work, &["fetch", "-q", "origin"]);
    run(&server, &["update-ref", "-d", "refs/heads/refs/tags/v1.0"]);

    let opened = GixBackend.open(&work).expect("open repository");
    let remote_branches = opened
        .list_remote_branches(&CancellationToken::new())
        .expect("list remote branches");
    assert!(
        remote_branches
            .iter()
            .any(|branch| branch.remote == "origin" && branch.name == "refs/tags/v1.0"),
        "the stale tag-shaped branch should be listed as a remote branch: {remote_branches:?}"
    );

    let _ = opened.delete_remote_branch_with_output("origin", "refs/tags/v1.0");

    assert!(
        ref_exists(&server, "refs/tags/v1.0"),
        "deleting the remote branch `refs/tags/v1.0` deleted the server's release tag"
    );
}

#[test]
fn branch_listing_names_a_branch_by_its_name_when_a_tag_shares_it() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    branch_and_tag_named_topic(&repo);
    // A `[branch]` section makes the listing use `git for-each-ref`.
    run(
        &repo,
        &["config", "branch.topic.description", "work in progress"],
    );

    let opened = GixBackend.open(&repo).expect("open repository");
    let mut names: Vec<String> = opened
        .list_branches(&CancellationToken::new())
        .expect("list branches")
        .into_iter()
        .map(|branch| branch.name)
        .collect();
    names.sort();

    assert_eq!(names, ["decoy", "main", "topic"]);
}

#[test]
fn ref_metadata_is_keyed_by_the_branch_name_when_a_tag_shares_it() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    branch_and_tag_named_topic(&repo);

    let opened = GixBackend.open(&repo).expect("open repository");
    let metadata = opened
        .list_ref_metadata(&CancellationToken::new())
        .expect("list ref metadata");

    let topic = metadata
        .get("topic")
        .unwrap_or_else(|| panic!("no metadata for branch `topic`: {:?}", metadata.keys()));
    assert_eq!(topic.summary, "topic");
}

#[test]
fn prune_merged_deletes_a_merged_branch_with_a_gone_upstream_when_a_tag_shares_its_name() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let ServerAndClone { server, work } = server_and_work(dir.path());
    run(&work, &["branch", "topic"]);
    run(&work, &["push", "-q", "-u", "origin", "topic"]);
    run(&work, &["tag", "topic"]);
    run(&server, &["update-ref", "-d", "refs/heads/topic"]);

    let opened = GixBackend.open(&work).expect("open repository");
    opened
        .prune_merged_branches_with_output()
        .expect("prune merged branches");

    assert!(
        !ref_exists(&work, "refs/heads/topic"),
        "the merged branch `topic` whose upstream is gone was not pruned"
    );
    assert!(ref_exists(&work, "refs/tags/topic"), "the tag must stay");
}

#[test]
fn prune_merged_keeps_a_branch_whose_upstream_exists_when_a_tag_shares_the_upstream_name() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let ServerAndClone { work, .. } = server_and_work(dir.path());
    run(&work, &["branch", "keep"]);
    run(&work, &["push", "-q", "-u", "origin", "keep"]);
    run(&work, &["tag", "origin/keep"]);

    let opened = GixBackend.open(&work).expect("open repository");
    opened
        .prune_merged_branches_with_output()
        .expect("prune merged branches");

    assert!(
        ref_exists(&work, "refs/heads/keep"),
        "the branch `keep` was pruned although `origin/keep` still exists"
    );
}

/// The branch picker's "Create branch ... Based off topic" passes the current
/// branch qualified, because the start point is resolved as a revision and a
/// short name would find the tag first.
#[test]
fn create_branch_from_a_qualified_branch_ignores_a_same_named_tag() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let refs = branch_and_tag_named_topic(&repo);

    let opened = GixBackend.open(&repo).expect("open repository");
    opened
        .create_branch("from-topic", &CommitId("refs/heads/topic".into()))
        .expect("create branch");

    assert_eq!(
        capture(&repo, &["rev-parse", "refs/heads/from-topic"]),
        refs.branch_tip
    );
}
