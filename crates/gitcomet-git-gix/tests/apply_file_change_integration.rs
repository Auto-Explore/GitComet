use gitcomet_core::domain::{CommitId, DiffTarget};
use gitcomet_core::error::{ErrorKind, GitFailureId};
use gitcomet_core::services::{
    APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL, APPLY_FILE_CHANGE_ALREADY_STAGED_SENTINEL,
    GitBackend, GitRepository,
};
use gitcomet_core::test_support::git_fixture::append_config;
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn run_git(repo: &Path, args: &[&str]) {
    let output = git_output(repo, args);
    assert!(
        output.status.success(),
        "git {:?} failed:\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repo: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .output()
        .expect("git command to run")
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = git_output(repo, args);
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo directory");
    run_git(repo, &["init", "-b", "main"]);
    append_config(
        repo,
        &[("user.email", "you@example.com"), ("user.name", "You")],
    );
}

fn commit_all(repo: &Path, message: &str) -> String {
    run_git(repo, &["add", "-A"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
    git_stdout(repo, &["rev-parse", "HEAD"])
}

fn write(repo: &Path, name: &str, content: &str) {
    fs::write(repo.join(name), content).expect("write file");
}

fn open_backend(repo: &Path) -> Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open repository")
}

fn commit_target(sha: &str, path: &str) -> DiffTarget {
    DiffTarget::Commit {
        commit_id: CommitId(sha.into()),
        path: PathBuf::from(path),
    }
}

fn status(repo: &Path) -> String {
    git_stdout(repo, &["status", "--porcelain", "--untracked-files=no"])
}

/// `main` at a base with `a.txt` and `b.txt`; `feature` edits both in one
/// commit. Returns that commit, with `main` checked out.
fn setup_two_file_commit(repo: &Path) -> String {
    init_repo(repo);
    write(repo, "a.txt", "one\ntwo\nthree\n");
    write(repo, "b.txt", "alpha\n");
    commit_all(repo, "base");
    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "one\nTWO\nthree\n");
    write(repo, "b.txt", "beta\n");
    run_git(repo, &["add", "-A"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Source Author",
            "-c",
            "user.email=source@example.com",
            "commit",
            "-m",
            "feature change\n\nbody line",
        ],
    );
    let picked = git_stdout(repo, &["rev-parse", "HEAD"]);
    run_git(repo, &["checkout", "main"]);
    picked
}

#[test]
fn staged_apply_takes_only_the_selected_file() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    let head_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "a.txt"), false)
        .expect("apply change");

    assert_eq!(status(&repo), "M  a.txt");
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "one\nTWO\nthree\n"
    );
    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "alpha\n");
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), head_before);
}

#[test]
fn committed_apply_reuses_source_message_and_author_and_leaves_other_staged_work() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    write(&repo, "c.txt", "unrelated\n");
    run_git(&repo, &["add", "c.txt"]);

    let output = open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "a.txt"), true)
        .expect("apply and commit change");

    assert!(output.command.contains("git commit"), "{output:?}");
    assert_eq!(
        git_stdout(&repo, &["show", "--name-only", "--format=", "HEAD"]),
        "a.txt"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%an <%ae>|%B"]),
        "Source Author <source@example.com>|feature change\n\nbody line"
    );
    assert_eq!(status(&repo), "A  c.txt");
}

#[test]
fn range_target_applies_the_combined_change_with_a_generated_message() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    write(&repo, "a.txt", "1\n2\n3\n4\n5\n");
    let base = commit_all(&repo, "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    write(&repo, "a.txt", "1\nX\n3\n4\n5\n");
    commit_all(&repo, "first");
    write(&repo, "a.txt", "1\nX\n3\n4\nY\n");
    let tip = commit_all(&repo, "second");
    run_git(&repo, &["checkout", "main"]);

    let target = DiffTarget::CommitRange {
        from_commit_id: CommitId(base.as_str().into()),
        to_commit_id: Some(CommitId(tip.as_str().into())),
        path: Some(PathBuf::from("a.txt")),
    };
    open_backend(&repo)
        .apply_file_change_with_output(&target, true)
        .expect("apply range change");

    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "1\nX\n3\n4\nY\n"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        format!("Apply a.txt from {}..{}", &base[..7], &tip[..7])
    );
    assert_eq!(git_stdout(&repo, &["log", "-1", "--format=%an"]), "You");
}

#[test]
fn a_comparison_to_the_working_tree_is_rejected() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    let backend = open_backend(&repo);

    let worktree_range = DiffTarget::CommitRange {
        from_commit_id: CommitId(picked.as_str().into()),
        to_commit_id: None,
        path: Some(PathBuf::from("a.txt")),
    };
    assert!(
        backend
            .apply_file_change_with_output(&worktree_range, false)
            .is_err()
    );
    assert_eq!(status(&repo), "");
}

#[test]
fn root_commit_additions_and_later_deletions_apply() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    write(&repo, "root.txt", "from root\n");
    let root = commit_all(&repo, "root");
    run_git(&repo, &["rm", "-q", "root.txt"]);
    write(&repo, "other.txt", "other\n");
    commit_all(&repo, "remove root file");
    run_git(&repo, &["checkout", "-q", "-b", "side", &root]);
    write(&repo, "gone.txt", "to delete\n");
    commit_all(&repo, "add gone");
    run_git(&repo, &["rm", "-q", "gone.txt"]);
    let deletion = commit_all(&repo, "delete gone");
    run_git(&repo, &["checkout", "-q", "main"]);

    // Re-adding a file from the root commit, which has no parent.
    open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&root, "root.txt"), false)
        .expect("apply root addition");
    assert_eq!(status(&repo), "A  root.txt");

    // A deletion of a file this branch has with the same content.
    write(&repo, "gone.txt", "to delete\n");
    commit_all(&repo, "add gone on main");
    open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&deletion, "gone.txt"), false)
        .expect("apply deletion");
    assert!(!repo.join("gone.txt").exists());
    assert!(status(&repo).contains("D  gone.txt"));
}

#[test]
fn conflicting_apply_leaves_the_file_unmerged() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    write(&repo, "a.txt", "one\nMAIN\nthree\n");
    commit_all(&repo, "main edit");
    let head_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let err = open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "a.txt"), true)
        .expect_err("conflicting apply should fail");

    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.id(), GitFailureId::ApplyChangeConflict);
            assert!(
                failure
                    .detail()
                    .unwrap_or_default()
                    .contains("with conflicts"),
                "{failure:?}"
            );
        }
        other => panic!("expected a git conflict failure, got {other:?}"),
    }
    assert_eq!(status(&repo), "UU a.txt");
    let content = fs::read_to_string(repo.join("a.txt")).unwrap();
    assert!(content.contains("<<<<<<<"), "{content}");
    // A conflict is never committed.
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), head_before);
}

#[test]
fn unstaged_edits_to_the_file_are_refused_and_kept() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    write(&repo, "a.txt", "one\ntwo\nthree\nlocal\n");

    let err = open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "a.txt"), false)
        .expect_err("dirty file should be refused");

    assert!(err.to_string().contains("unstaged changes"), "{err}");
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "one\ntwo\nthree\nlocal\n"
    );
    assert_eq!(git_stdout(&repo, &["diff", "--name-only"]), "a.txt");
    assert_eq!(git_stdout(&repo, &["diff", "--cached", "--name-only"]), "");
}

#[test]
fn a_change_the_branch_already_has_reports_the_sentinel() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    write(&repo, "a.txt", "one\nTWO\nthree\n");
    let head = commit_all(&repo, "same edit on main");

    let output = open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "a.txt"), true)
        .expect("already applied is not an error");

    assert!(
        output
            .stdout
            .contains(APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL)
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(status(&repo), "");
}

#[test]
fn a_rerun_after_the_apply_step_only_commits() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    let backend = open_backend(&repo);
    let target = commit_target(&picked, "a.txt");

    backend
        .apply_file_change_with_output(&target, false)
        .expect("stage change");
    let staged_again = backend
        .apply_file_change_with_output(&target, false)
        .expect("second staged run");
    assert!(
        staged_again
            .stdout
            .contains(APPLY_FILE_CHANGE_ALREADY_STAGED_SENTINEL)
    );

    // What an auth retry of a failed commit step does.
    backend
        .apply_file_change_with_output(&target, true)
        .expect("commit staged change");
    assert_eq!(
        git_stdout(&repo, &["show", "--name-only", "--format=", "HEAD"]),
        "a.txt"
    );
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "one\nTWO\nthree\n"
    );
    assert_eq!(status(&repo), "");
}

#[test]
fn an_operation_in_progress_is_refused() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    write(&repo, "b.txt", "main\n");
    commit_all(&repo, "main b");
    let merge = git_output(&repo, &["merge", "--no-edit", "feature"]);
    assert!(
        !merge.status.success(),
        "the merge should stop at a conflict"
    );

    let err = open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "a.txt"), false)
        .expect_err("apply during a merge should be refused");

    assert!(err.to_string().contains("a merge is in progress"), "{err}");
}

// `*` is not a valid Windows file name character.
#[cfg(unix)]
#[test]
fn glob_characters_in_the_path_are_literal() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    write(&repo, "a*.txt", "star\n");
    write(&repo, "ab.txt", "plain\n");
    commit_all(&repo, "base");
    run_git(&repo, &["checkout", "-q", "-b", "feature"]);
    write(&repo, "a*.txt", "star changed\n");
    write(&repo, "ab.txt", "plain changed\n");
    let picked = commit_all(&repo, "both");
    run_git(&repo, &["checkout", "-q", "main"]);

    open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "a*.txt"), true)
        .expect("apply literal path");

    assert_eq!(
        git_stdout(&repo, &["show", "--name-only", "--format=", "HEAD"]),
        "a*.txt"
    );
    assert_eq!(fs::read_to_string(repo.join("ab.txt")).unwrap(), "plain\n");
}

#[test]
fn staged_edits_to_the_file_are_refused_not_committed() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    write(&repo, "a.txt", "one\ntwo\nthree\nmine\n");
    run_git(&repo, &["add", "a.txt"]);
    let head_before = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let err = open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "a.txt"), true)
        .expect_err("a file with staged edits should be refused");

    assert!(err.to_string().contains("staged changes"), "{err}");
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        git_stdout(&repo, &["show", ":a.txt"]),
        "one\ntwo\nthree\nmine"
    );
}

#[test]
fn a_change_the_branch_has_is_not_confused_with_other_staged_edits() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    write(&repo, "a.txt", "one\nTWO\nthree\n");
    let head = commit_all(&repo, "same edit on main");
    write(&repo, "a.txt", "one\nTWO\nthree\nmine\n");
    run_git(&repo, &["add", "a.txt"]);

    for commit in [false, true] {
        let err = open_backend(&repo)
            .apply_file_change_with_output(&commit_target(&picked, "a.txt"), commit)
            .expect_err("a file with staged edits should be refused");
        assert!(err.to_string().contains("staged changes"), "{err}");
    }
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), head);
}

#[test]
fn a_post_image_elsewhere_in_the_file_is_not_taken_for_already_applied() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    // Two blocks with identical surroundings: the second one's added line,
    // with its context, also matches the first block.
    let block = |enabled: bool| {
        let mut text = String::from("a\nb\nc\n- item:\n    name: x\n");
        if enabled {
            text.push_str("    enabled: true\n");
        }
        text.push_str("    size: 1\nd\ne\nf\n");
        text
    };
    write(
        &repo,
        "list.yml",
        &format!("{}{}", block(true), block(false)),
    );
    commit_all(&repo, "base");
    run_git(&repo, &["checkout", "-q", "-b", "feature"]);
    let changed = format!("{}{}", block(true), block(true));
    write(&repo, "list.yml", &changed);
    let picked = commit_all(&repo, "enable the second item");
    run_git(&repo, &["checkout", "-q", "main"]);

    let output = open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "list.yml"), false)
        .expect("apply change");

    assert!(
        !output
            .stdout
            .contains(APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL),
        "{output:?}"
    );
    assert_eq!(fs::read_to_string(repo.join("list.yml")).unwrap(), changed);
}

#[test]
fn a_change_already_in_head_beside_later_edits_reports_already_applied() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    write(&repo, "a.txt", "1\n2\n3\n4\n5\n6\n");
    commit_all(&repo, "base");
    run_git(&repo, &["checkout", "-q", "-b", "feature"]);
    write(&repo, "a.txt", "1\n2\n3\nFOUR\n5\n6\n");
    let picked = commit_all(&repo, "four");
    run_git(&repo, &["checkout", "-q", "main"]);
    write(&repo, "a.txt", "1\nTWO\n3\nFOUR\n5\n6\n");
    let head = commit_all(&repo, "four and two on main");

    for commit in [false, true] {
        let output = open_backend(&repo)
            .apply_file_change_with_output(&commit_target(&picked, "a.txt"), commit)
            .expect("already applied is not an error");
        assert!(
            output
                .stdout
                .contains(APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL),
            "commit={commit}: {output:?}"
        );
    }
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(status(&repo), "");
}

#[test]
fn user_diff_config_does_not_reshape_the_patch() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_two_file_commit(&repo);
    append_config(
        &repo,
        &[
            ("diff.context", "0"),
            ("color.diff", "always"),
            ("color.ui", "always"),
            ("diff.noprefix", "true"),
            ("diff.mnemonicPrefix", "true"),
            ("diff.renames", "false"),
            ("log.showSignature", "true"),
            ("apply.whitespace", "error"),
        ],
    );

    open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "a.txt"), false)
        .expect("apply change under hostile diff config");

    assert_eq!(status(&repo), "M  a.txt");
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "one\nTWO\nthree\n"
    );
}

#[test]
fn a_renamed_file_is_applied_as_a_rename() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let body: String = (1..=8).map(|n| format!("line {n}\n")).collect();
    write(&repo, "old.txt", &body);
    write(&repo, "other.txt", "other\n");
    commit_all(&repo, "base");
    run_git(&repo, &["checkout", "-q", "-b", "feature"]);
    run_git(&repo, &["mv", "old.txt", "new.txt"]);
    let edited = body.replace("line 8", "line eight");
    write(&repo, "new.txt", &edited);
    write(&repo, "other.txt", "other changed\n");
    let picked = commit_all(&repo, "rename and edit");
    run_git(&repo, &["checkout", "-q", "main"]);

    open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "new.txt"), true)
        .expect("apply renamed file");

    assert!(!repo.join("old.txt").exists());
    assert_eq!(fs::read_to_string(repo.join("new.txt")).unwrap(), edited);
    assert_eq!(
        fs::read_to_string(repo.join("other.txt")).unwrap(),
        "other\n"
    );
    let committed = git_stdout(&repo, &["show", "-M", "--name-status", "--format=", "HEAD"]);
    let fields: Vec<_> = committed.split_whitespace().collect();
    assert!(
        matches!(fields.as_slice(), [status, "old.txt", "new.txt"] if status.starts_with('R')),
        "the commit should be one rename: {committed}"
    );
    assert_eq!(status(&repo), "");
}

/// A commit step that fails (here: a signer that always fails) leaves the
/// merged change staged and says so; the rerun an auth retry makes commits it
/// without applying the change again, even though the merge made the staged
/// file differ from the source commit's version.
#[cfg(unix)]
#[test]
fn a_failed_commit_step_keeps_the_merged_change_for_the_rerun() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    write(&repo, "a.txt", "1\n2\n3\n4\n5\n6\n7\n8\n");
    commit_all(&repo, "base");
    run_git(&repo, &["checkout", "-q", "-b", "feature"]);
    write(&repo, "a.txt", "1\n2\n3\n4\n5\n6\n7\nEIGHT\n");
    let picked = commit_all(&repo, "eight");
    run_git(&repo, &["checkout", "-q", "main"]);
    write(&repo, "a.txt", "ONE\n2\n3\n4\n5\n6\n7\n8\n");
    let head = commit_all(&repo, "one");
    append_config(
        &repo,
        &[("commit.gpgsign", "true"), ("gpg.program", "false")],
    );
    let backend = open_backend(&repo);
    let target = commit_target(&picked, "a.txt");

    let err = backend
        .apply_file_change_with_output(&target, true)
        .expect_err("the signer fails");
    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.id(), GitFailureId::ApplyChangeCommitFailed)
        }
        other => panic!("expected a git failure, got {other:?}"),
    }
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(status(&repo), "M  a.txt");

    append_config(&repo, &[("commit.gpgsign", "false")]);
    backend
        .apply_file_change_with_output(&target, true)
        .expect("the rerun commits the staged change");
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "ONE\n2\n3\n4\n5\n6\n7\nEIGHT\n"
    );
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "HEAD~1"]),
        head,
        "exactly one new commit"
    );
    assert_eq!(status(&repo), "");
}

/// File history follows renames and passes the file's current name for every
/// row, so a change from before the rename lands on the file as named now.
#[test]
fn a_change_from_before_a_rename_applies_to_the_current_name() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let body: String = (1..=8).map(|n| format!("line {n}\n")).collect();
    write(&repo, "a.txt", &body);
    commit_all(&repo, "base");
    let edited = body.replace("line 8", "line eight");
    write(&repo, "a.txt", &edited);
    let picked = commit_all(&repo, "edit before the rename");
    run_git(&repo, &["mv", "a.txt", "b.txt"]);
    commit_all(&repo, "rename");
    write(&repo, "b.txt", &body);
    commit_all(&repo, "undo the edit");

    open_backend(&repo)
        .apply_file_change_with_output(&commit_target(&picked, "b.txt"), false)
        .expect("apply a change made under the old name");

    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), edited);
    assert!(!repo.join("a.txt").exists());
    assert_eq!(status(&repo), "M  b.txt");
}
