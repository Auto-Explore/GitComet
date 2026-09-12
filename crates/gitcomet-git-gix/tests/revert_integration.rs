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
fn revert_without_commit_only_stages_the_inverse() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);

    let output = open_backend(&repo)
        .revert_with_output(&commit_id(&change), false, None)
        .expect("revert --no-commit");

    assert_eq!(output.command, format!("git revert --no-commit {change}"));
    assert_eq!(head(&repo), change);
    assert_eq!(status(&repo), "M  file.txt");
    // Like `cherry-pick -n`: no operation left in progress, but MERGE_MSG
    // stays as the next commit's template.
    assert_eq!(sequencer_state(&repo), SequencerState::None);
    assert!(!repo.join(".git/REVERT_HEAD").exists());
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "--no-edit"],
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        format!("Revert \"change\"\n\nThis reverts commit {change}.")
    );
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

    // The auth retry replays the revert, which resumes at the commit step.
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    let output = open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect("replay after fixing the signer");

    assert_eq!(output.command, format!("git revert {change}"));
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        format!("Revert \"change\"\n\nThis reverts commit {change}.")
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), change);
    assert_eq!(status(&repo), "");
    assert_no_revert_state(&repo);
}

#[cfg(unix)]
#[test]
fn replayed_revert_commits_with_the_same_hooks_skipped() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    run_git(&repo, &["config", "commit.gpgsign", "true"]);
    run_git(&repo, &["config", "gpg.program", "false"]);
    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect_err("signing failure");
    install_hook(&repo, "pre-commit", "#!/bin/sh\nexit 1\n");
    install_hook(&repo, "commit-msg", "#!/bin/sh\nexit 1\n");
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect("the replay skips pre-commit and commit-msg like the first attempt");

    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), change);
    assert_no_revert_state(&repo);
}

#[test]
fn stale_squash_msg_does_not_leak_into_the_revert() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    fs::write(
        repo.join(".git/SQUASH_MSG"),
        "Squashed commit of the following:\n",
    )
    .expect("write stale SQUASH_MSG");

    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect("revert");

    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        format!("Revert \"change\"\n\nThis reverts commit {change}.")
    );
    assert!(
        git_stdout(&repo, &["reflog", "-1", "--format=%gs"]).starts_with("revert: "),
        "the reflog should record a revert"
    );
}

#[test]
fn leftover_cherry_pick_sequence_blocks_revert_and_stays_reported() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "base\n", "base");
    let reverted = commit_file(&repo, "other.txt", "other\n", "other");
    run_git(&repo, &["checkout", "-b", "feature", "HEAD~1"]);
    let first = commit_file(&repo, "file.txt", "feature\n", "first pick");
    let second = commit_file(&repo, "second.txt", "second\n", "second pick");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "file.txt", "main\n", "main change");
    let conflict = git_output(&repo, &["cherry-pick", &first, &second]);
    assert!(!conflict.status.success(), "the first pick should conflict");
    fs::write(repo.join("file.txt"), "resolved\n").expect("resolve");
    run_git(&repo, &["add", "file.txt"]);
    // A plain commit concludes the stopped step but keeps the sequence.
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "--no-edit"],
    );
    assert!(!repo.join(".git/CHERRY_PICK_HEAD").exists());
    assert!(repo.join(".git/sequencer/todo").exists());

    assert_eq!(sequencer_state(&repo), SequencerState::CherryPick);
    let err = open_backend(&repo)
        .revert_with_output(&commit_id(&reverted), true, None)
        .expect_err("revert over a leftover sequence");
    assert!(
        err.to_string().contains("sequence is in progress"),
        "unexpected error: {err}"
    );
    assert!(
        repo.join(".git/sequencer/todo").exists(),
        "the pending pick survives"
    );
}

#[test]
fn leftover_revert_sequence_continues_its_remaining_steps() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_conflicting_revert_repo(&repo);
    let other = commit_file(&repo, "other.txt", "other\n", "other");
    let conflict = git_output(&repo, &["revert", "--no-edit", &change, &other]);
    assert!(
        !conflict.status.success(),
        "the first revert should conflict"
    );
    fs::write(repo.join("file.txt"), "resolved\n").expect("resolve");
    run_git(&repo, &["add", "file.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "--no-edit"],
    );
    assert!(!repo.join(".git/REVERT_HEAD").exists());
    assert_eq!(sequencer_state(&repo), SequencerState::Revert);

    let output = open_backend(&repo)
        .rebase_continue_with_output()
        .expect("continue the remaining revert");

    assert_eq!(output.command, "git revert --continue");
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "Revert \"other\""
    );
    assert!(!repo.join("other.txt").exists());
    assert_eq!(sequencer_state(&repo), SequencerState::None);
}

#[test]
fn unstage_all_keeps_a_stopped_revert_and_a_resolved_merge() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    run_git(&repo, &["config", "commit.gpgsign", "true"]);
    run_git(&repo, &["config", "gpg.program", "false"]);
    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect_err("signing failure stops the revert");

    open_backend(&repo).unstage(&[]).expect("unstage all");

    // Unstaged now (`status` trims the leading space of " M").
    assert_eq!(status(&repo), "M file.txt");
    assert_eq!(sequencer_state(&repo), SequencerState::Revert);
    assert!(repo.join(".git/MERGE_MSG").exists());

    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    setup_merge_revert_repo(&repo);
    run_git(&repo, &["reset", "--hard", "HEAD~1"]);
    run_git(&repo, &["merge", "--no-ff", "--no-commit", "side"]);
    assert!(repo.join(".git/MERGE_HEAD").exists());

    open_backend(&repo).unstage(&[]).expect("unstage all");

    assert!(repo.join(".git/MERGE_HEAD").exists(), "the merge survives");
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

/// base(f=a, g=0) → cG(g=1) → cF(f=b) → undo(g=0) → head(f=c).
/// Reverting cF conflicts; reverting cG is already undone.
fn setup_revert_sequence_repo(repo: &Path) -> (String, String) {
    init_repo(repo);
    fs::write(repo.join("g.txt"), "0\n").expect("write g");
    commit_file(repo, "f.txt", "a\n", "base");
    let c_g = commit_file(repo, "g.txt", "1\n", "g one");
    let c_f = commit_file(repo, "f.txt", "b\n", "f b");
    commit_file(repo, "g.txt", "0\n", "undo g by hand");
    commit_file(repo, "f.txt", "c\n", "f c");
    (c_g, c_f)
}

#[test]
fn revert_sequence_continue_pauses_at_the_next_conflict() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "f.txt", "a\n", "base");
    let first = commit_file(&repo, "f.txt", "b\n", "f b");
    let second = commit_file(&repo, "f.txt", "c\n", "f c");
    commit_file(&repo, "f.txt", "d\n", "f d");
    let conflict = git_output(&repo, &["revert", "--no-edit", &second, &first]);
    assert!(
        !conflict.status.success(),
        "the first revert should conflict"
    );
    fs::write(repo.join("f.txt"), "x\n")
        .expect("resolve to something the next revert cannot apply to");
    run_git(&repo, &["add", "f.txt"]);

    let output = open_backend(&repo)
        .rebase_continue_with_output()
        .expect("a sequence that advanced and paused again is not a failure");

    assert_eq!(output.command, "git revert --continue");
    assert_ne!(
        output.exit_code,
        Some(0),
        "the paused exit code drives the \"paused at the next conflict\" summary"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "Revert \"f c\"",
        "the resolved step is committed"
    );
    assert_eq!(sequencer_state(&repo), SequencerState::Revert);
    assert_eq!(status(&repo), "UU f.txt");
}

#[test]
fn revert_sequence_continue_skips_a_step_that_is_already_undone() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (c_g, c_f) = setup_revert_sequence_repo(&repo);
    let conflict = git_output(&repo, &["revert", "--no-edit", &c_f, &c_g]);
    assert!(!conflict.status.success(), "reverting f should conflict");
    fs::write(repo.join("f.txt"), "a\n").expect("resolve");
    run_git(&repo, &["add", "f.txt"]);

    let output = open_backend(&repo)
        .rebase_continue_with_output()
        .expect("one Continue finishes the sequence");

    assert!(
        output.command.starts_with("git revert --"),
        "unexpected command: {}",
        output.command
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "Revert \"f b\"",
        "the empty step is skipped rather than stopping the sequence"
    );
    assert_no_revert_state(&repo);
    assert_eq!(status(&repo), "");
}

#[test]
fn an_abbreviated_id_still_resumes_a_stopped_revert() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    run_git(&repo, &["config", "commit.gpgsign", "true"]);
    run_git(&repo, &["config", "gpg.program", "false"]);
    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect_err("signing failure");
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    let abbreviated = commit_id(&change[..8]);
    let output = open_backend(&repo)
        .revert_with_output(&abbreviated, true, None)
        .expect("an abbreviated id names the same stopped revert");

    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), change);
    assert_eq!(output.command, format!("git revert {}", &change[..8]));
    assert_no_revert_state(&repo);
}

#[test]
fn a_sequencer_directory_git_ignores_does_not_block_revert() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    // Git reports no operation for a todo whose first command it cannot read,
    // and refuses only to start a *sequence*; a single revert still runs.
    fs::create_dir_all(repo.join(".git/sequencer")).expect("create sequencer dir");
    fs::write(repo.join(".git/sequencer/todo"), "# nothing here\n").expect("write todo");
    assert_eq!(sequencer_state(&repo), SequencerState::None);

    open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect("a revert git itself would allow");

    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), change);

    // The no-op path cleans up after itself, and must not take a sequencer
    // directory it did not create with it.
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_revertable_repo(&repo);
    commit_file(&repo, "file.txt", "old\n", "undo change by hand");
    fs::create_dir_all(repo.join(".git/sequencer")).expect("create sequencer dir");
    fs::write(repo.join(".git/sequencer/todo"), "# nothing here\n").expect("write todo");

    let output = open_backend(&repo)
        .revert_with_output(&commit_id(&change), true, None)
        .expect("nothing to revert");

    assert!(
        output.stdout.contains(REVERT_NOTHING_TO_REVERT_SENTINEL),
        "{output:?}"
    );
    assert!(
        repo.join(".git/sequencer").exists(),
        "another operation's directory must survive"
    );
    assert!(!repo.join(".git/REVERT_HEAD").exists());
}

#[test]
fn aborting_a_leftover_sequence_reports_that_head_was_kept() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let change = setup_conflicting_revert_repo(&repo);
    let other = commit_file(&repo, "other.txt", "other\n", "other");
    let conflict = git_output(&repo, &["revert", "--no-edit", &change, &other]);
    assert!(
        !conflict.status.success(),
        "the first revert should conflict"
    );
    fs::write(repo.join("file.txt"), "resolved\n").expect("resolve");
    run_git(&repo, &["add", "file.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "--no-edit"],
    );
    let head_before = head(&repo);

    let output = open_backend(&repo)
        .rebase_abort_with_output()
        .expect("abort clears the leftover sequence");

    assert!(
        output
            .stdout
            .contains(gitcomet_core::services::REVERT_ABORT_KEPT_HEAD_SENTINEL),
        "git refused to rewind, so the summary must not claim a restore: {output:?}"
    );
    assert_eq!(
        head(&repo),
        head_before,
        "the manual commit is still on the branch"
    );
    assert_no_revert_state(&repo);
}
