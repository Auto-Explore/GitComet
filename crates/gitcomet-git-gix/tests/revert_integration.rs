use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{
    GitBackend, GitRepository, REVERT_NOTHING_TO_REVERT_SENTINEL, SequencerState,
};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

#[cfg(unix)]
fn install_hook(repo: &Path, name: &str, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;

    let hook = repo.join(".git").join("hooks").join(name);
    fs::write(&hook, script).expect("write hook");
    let mut permissions = fs::metadata(&hook).expect("stat hook").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(hook, permissions).expect("make hook executable");
}

fn git_output(repo: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .output()
        .expect("git command to run")
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = git_output(repo, args);
    assert!(output.status.success(), "git {:?} failed", args);
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = git_output(repo, args);
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo directory");
    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
}

fn commit_file(repo: &Path, name: &str, content: &str, message: &str) -> String {
    fs::write(repo.join(name), content).expect("write file");
    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
    git_stdout(repo, &["rev-parse", "HEAD"])
}

fn commit_id(sha: &str) -> CommitId {
    CommitId(sha.into())
}

fn open_backend(repo: &Path) -> Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open repository")
}

fn head(repo: &Path) -> String {
    git_stdout(repo, &["rev-parse", "HEAD"])
}

fn status(repo: &Path) -> String {
    git_stdout(repo, &["status", "--porcelain"])
}

fn sequencer_state(repo: &Path) -> SequencerState {
    open_backend(repo).sequencer_state().unwrap()
}

fn assert_no_revert_state(repo: &Path) {
    assert_eq!(sequencer_state(repo), SequencerState::None);
    assert!(!repo.join(".git/REVERT_HEAD").exists(), "REVERT_HEAD left");
    assert!(!repo.join(".git/MERGE_MSG").exists(), "MERGE_MSG left");
}

/// `file.txt`: "old" → "new" in `change`; returns `change`.
fn setup_revertable_repo(repo: &Path) -> String {
    init_repo(repo);
    commit_file(repo, "file.txt", "old\n", "base");
    commit_file(repo, "file.txt", "new\n", "change")
}

/// Like [`setup_revertable_repo`], plus a later edit of the same line, so
/// reverting `change` conflicts. Returns `change`.
fn setup_conflicting_revert_repo(repo: &Path) -> String {
    let change = setup_revertable_repo(repo);
    commit_file(repo, "file.txt", "later\n", "later");
    change
}

/// A merge on `main` whose first-parent-only (`mainline.txt`) and
/// second-parent-only (`side.txt`) changes are distinguishable.
fn setup_merge_revert_repo(repo: &Path) -> String {
    init_repo(repo);
    let base = commit_file(repo, "base.txt", "base\n", "base");
    commit_file(repo, "mainline.txt", "mainline\n", "mainline change");
    run_git(repo, &["checkout", "-b", "side", &base]);
    commit_file(repo, "side.txt", "side\n", "side change");
    run_git(repo, &["checkout", "main"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--no-ff",
            "side",
            "-m",
            "merge side",
        ],
    );
    head(repo)
}

#[test]
fn revert_with_commit_creates_revert_commit() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);

    let output = open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect("revert");

    assert_eq!(output.command, format!("git revert {change}"));
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        format!("Revert \"change\"\n\nThis reverts commit {change}.")
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), change);
    assert_eq!(fs::read_to_string(repo.join("file.txt")).unwrap(), "old\n");
    assert_eq!(status(&repo), "");
    assert_no_revert_state(&repo);
}

#[test]
fn revert_without_commit_stages_inverse_and_pauses_in_revert_state() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);

    let output = open_backend(&repo)
        .revert_with_output(&commit_id(&change), false, None)
        .expect("revert --no-commit");

    assert_eq!(output.command, format!("git revert --no-commit {change}"));
    assert_eq!(head(&repo), change);
    assert_eq!(status(&repo), "M  file.txt");
    assert_eq!(sequencer_state(&repo), SequencerState::Revert);
    assert!(open_backend(&repo).rebase_in_progress().unwrap());
    assert_eq!(git_stdout(&repo, &["rev-parse", "REVERT_HEAD"]), change);

    let output = open_backend(&repo)
        .rebase_continue_with_output()
        .expect("continue commits the staged revert");

    assert_eq!(output.command, "git revert --continue");
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "Revert \"change\""
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), change);
    assert_eq!(status(&repo), "");
    assert_no_revert_state(&repo);
}

#[test]
fn revert_without_commit_abort_restores_previous_state() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    open_backend(&repo)
        .revert_with_output(&commit_id(&change), false, None)
        .expect("revert --no-commit");

    let output = open_backend(&repo)
        .rebase_abort_with_output()
        .expect("abort the paused revert");

    assert_eq!(output.command, "git revert --abort");
    assert_eq!(head(&repo), change);
    assert_eq!(fs::read_to_string(repo.join("file.txt")).unwrap(), "new\n");
    assert_eq!(status(&repo), "");
    assert_no_revert_state(&repo);
}

#[test]
fn merge_revert_uses_selected_mainline_parent() {
    for (mainline, kept, removed) in [
        (1, "mainline.txt", "side.txt"),
        (2, "side.txt", "mainline.txt"),
    ] {
        let dir = tempfile::tempdir().expect("create tempdir");
        let repo = dir.path().join("repo");
        let merge = setup_merge_revert_repo(&repo);

        let output = open_backend(&repo)
            .revert_with_output(&commit_id(&merge), true, Some(mainline))
            .expect("merge revert");

        assert_eq!(output.command, format!("git revert -m {mainline} {merge}"));
        assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), merge);
        assert!(repo.join(kept).exists(), "-m {mainline} should keep {kept}");
        assert!(
            !repo.join(removed).exists(),
            "-m {mainline} should remove {removed}"
        );
        assert_eq!(status(&repo), "");
        assert_no_revert_state(&repo);
    }
}

#[test]
fn revert_validates_mainline_before_starting() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let merge = setup_merge_revert_repo(&repo);

    for (commit, mainline, expected) in [
        (merge.clone(), None, "revert: "),
        (
            merge.clone(),
            None,
            "is a merge commit with 2 parents; choose a mainline parent",
        ),
        (
            merge.clone(),
            Some(0),
            "mainline parent 0 is invalid for merge commit",
        ),
        (
            merge.clone(),
            Some(3),
            "mainline parent 3 is invalid for merge commit",
        ),
        (
            git_stdout(&repo, &["rev-parse", "HEAD^1"]),
            Some(1),
            "is not a merge commit; a mainline parent cannot be selected",
        ),
    ] {
        let err = open_backend(&repo)
            .revert_with_output(&commit_id(&commit), true, mainline)
            .expect_err("invalid mainline should be rejected");
        assert!(
            err.to_string().contains(expected),
            "unexpected error: {err}"
        );
        assert_eq!(head(&repo), merge);
        assert_eq!(status(&repo), "");
        assert_no_revert_state(&repo);
    }
}

#[test]
fn already_reverted_revert_is_successful_noop_and_cleans_state() {
    for commit in [true, false] {
        let dir = tempfile::tempdir().expect("create tempdir");
        let repo = dir.path().join("repo");
        let change = setup_revertable_repo(&repo);
        commit_file(&repo, "file.txt", "old\n", "undo change by hand");
        let before_head = head(&repo);

        let output = open_backend(&repo)
            .revert_with_output(&commit_id(&change), commit, None)
            .expect("revert of already-undone change");

        assert_eq!(output.exit_code, Some(0));
        assert!(
            output.stdout.contains(REVERT_NOTHING_TO_REVERT_SENTINEL),
            "commit={commit}: missing sentinel in {output:?}"
        );
        assert_eq!(head(&repo), before_head);
        assert_eq!(status(&repo), "");
        assert_no_revert_state(&repo);
    }
}

#[test]
fn conflicting_revert_returns_error_and_pauses_in_revert_state() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_conflicting_revert_repo(&repo);

    let err = open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect_err("conflicting revert should fail");

    let message = err.to_string();
    assert!(
        message.contains("could not revert") || message.contains("CONFLICT"),
        "unexpected conflict error: {message}"
    );
    assert_eq!(status(&repo), "UU file.txt");
    assert_eq!(sequencer_state(&repo), SequencerState::Revert);
    assert_eq!(git_stdout(&repo, &["rev-parse", "REVERT_HEAD"]), change);
}

#[test]
fn revert_continue_commits_resolved_conflict() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_conflicting_revert_repo(&repo);
    let later = head(&repo);
    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect_err("conflict");

    let err = open_backend(&repo)
        .rebase_continue_with_output()
        .expect_err("continue with unresolved conflicts");
    assert!(
        err.to_string().contains("git revert --continue"),
        "unexpected continue error: {err}"
    );
    assert_eq!(sequencer_state(&repo), SequencerState::Revert);

    fs::write(repo.join("file.txt"), "resolved\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    let output = open_backend(&repo)
        .rebase_continue_with_output()
        .expect("continue resolved revert");

    assert_eq!(output.command, "git revert --continue");
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), later);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "Revert \"change\""
    );
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "resolved\n"
    );
    assert_eq!(status(&repo), "");
    assert_no_revert_state(&repo);
}

#[test]
fn revert_continue_skips_an_empty_resolution() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_conflicting_revert_repo(&repo);
    let later = head(&repo);
    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect_err("conflict");
    fs::write(repo.join("file.txt"), "later\n").expect("resolve to HEAD");
    run_git(&repo, &["add", "file.txt"]);

    let output = open_backend(&repo)
        .rebase_continue_with_output()
        .expect("empty resolution is skipped");

    assert_eq!(output.command, "git revert --skip");
    assert_eq!(head(&repo), later);
    assert_eq!(status(&repo), "");
    assert_no_revert_state(&repo);
}

#[test]
fn revert_abort_restores_state_after_conflict() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_conflicting_revert_repo(&repo);
    let later = head(&repo);
    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect_err("conflict");

    let output = open_backend(&repo)
        .rebase_abort_with_output()
        .expect("abort conflicted revert");

    assert_eq!(output.command, "git revert --abort");
    assert_eq!(head(&repo), later);
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "later\n"
    );
    assert_eq!(status(&repo), "");
    assert_no_revert_state(&repo);
}

#[test]
fn externally_started_revert_sequence_is_reported_and_aborted() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_conflicting_revert_repo(&repo);
    let later = head(&repo);
    let other = commit_file(&repo, "other.txt", "other\n", "other");

    let conflict = git_output(&repo, &["revert", "--no-edit", &other, &change]);
    assert!(
        !conflict.status.success(),
        "sequence should stop at a conflict"
    );
    assert!(repo.join(".git/sequencer/todo").exists());
    assert_eq!(sequencer_state(&repo), SequencerState::Revert);

    let output = open_backend(&repo)
        .rebase_abort_with_output()
        .expect("abort revert sequence");

    assert_eq!(output.command, "git revert --abort");
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), later);
    assert_eq!(status(&repo), "");
    assert_no_revert_state(&repo);
}

#[test]
fn staged_changes_reject_revert_before_git_runs() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    fs::write(repo.join("staged.txt"), "staged\n").expect("write staged file");
    run_git(&repo, &["add", "staged.txt"]);

    for commit in [true, false] {
        let err = open_backend(&repo)
            .revert_with_output(&commit_id(&change), commit, None)
            .expect_err("staged changes should reject revert");

        assert!(
            err.to_string().contains("staged changes"),
            "unexpected error: {err}"
        );
        assert_eq!(head(&repo), change);
        assert_eq!(status(&repo), "A  staged.txt");
        assert_no_revert_state(&repo);
    }
}

#[test]
fn dirty_worktree_rejects_revert_without_leaving_state() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    fs::write(repo.join("file.txt"), "dirty\n").expect("write dirty worktree");

    let err = open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect_err("dirty worktree should reject revert");

    let message = err.to_string();
    assert!(
        message.contains("local changes") || message.contains("would be overwritten"),
        "unexpected dirty-worktree error: {message}"
    );
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "dirty\n"
    );
    assert_eq!(status(&repo), "M file.txt");
    assert_no_revert_state(&repo);
}

#[test]
fn revert_is_refused_while_a_cherry_pick_is_in_progress() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "base\n", "base");
    let reverted = commit_file(&repo, "other.txt", "other\n", "other");
    run_git(&repo, &["checkout", "-b", "feature", "HEAD~1"]);
    let picked = commit_file(&repo, "file.txt", "feature\n", "feature change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "file.txt", "main\n", "main change");
    let conflict = git_output(&repo, &["cherry-pick", &picked]);
    assert!(!conflict.status.success(), "cherry-pick should conflict");

    let err = open_backend(&repo)
        .revert_with_output(&commit_id(&reverted), true, None)
        .expect_err("revert during a cherry-pick");

    assert!(
        err.to_string().contains("a cherry-pick is in progress"),
        "unexpected error: {err}"
    );
    assert_eq!(sequencer_state(&repo), SequencerState::CherryPick);
    assert!(!repo.join(".git/REVERT_HEAD").exists());
}

#[test]
fn signing_failure_leaves_a_resumable_revert() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    run_git(&repo, &["config", "commit.gpgsign", "true"]);
    run_git(&repo, &["config", "gpg.program", "false"]);

    let err = open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect_err("signing failure");

    let message = err.to_string();
    assert!(
        message.contains("sign") || message.contains("gpg"),
        "unexpected signing error: {message}"
    );
    assert_eq!(head(&repo), change);
    assert_eq!(status(&repo), "M  file.txt");
    assert_eq!(sequencer_state(&repo), SequencerState::Revert);

    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    let output = open_backend(&repo)
        .rebase_continue_with_output()
        .expect("continue after fixing the signer");

    assert_eq!(output.command, "git revert --continue");
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "Revert \"change\""
    );
    assert_eq!(status(&repo), "");
    assert_no_revert_state(&repo);
}

#[cfg(unix)]
#[test]
fn failing_pre_commit_hook_does_not_block_revert() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    install_hook(&repo, "pre-commit", "#!/bin/sh\nexit 1\n");
    install_hook(&repo, "commit-msg", "#!/bin/sh\nexit 1\n");

    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect("one-shot git revert skips pre-commit and commit-msg");

    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), change);
    assert_no_revert_state(&repo);
}

#[cfg(unix)]
#[test]
fn failing_prepare_commit_msg_hook_leaves_a_resumable_revert() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    install_hook(&repo, "prepare-commit-msg", "#!/bin/sh\nexit 1\n");

    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect_err("prepare-commit-msg failure");

    assert_eq!(head(&repo), change);
    assert_eq!(sequencer_state(&repo), SequencerState::Revert);
    fs::remove_file(repo.join(".git/hooks/prepare-commit-msg")).expect("remove hook");
    open_backend(&repo)
        .rebase_continue_with_output()
        .expect("continue after fixing the hook");
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), change);
    assert_no_revert_state(&repo);
}
