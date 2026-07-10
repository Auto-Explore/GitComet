use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{GitBackend, GitRepository, InteractiveRebaseAction};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn run_git(repo: &Path, args: &[&str]) {
    run_git_with_env(repo, args, &[]);
}

fn run_git_with_env(repo: &Path, args: &[&str], envs: &[(&str, &str)]) {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let cmd = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let status = cmd.status().expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let output = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git command to run");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo directory");
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
}

fn commit_file(repo: &Path, name: &str, content: &str, message: &str) {
    commit_file_with_env(repo, name, content, message, &[]);
}

fn commit_file_with_env(
    repo: &Path,
    name: &str,
    content: &str,
    message: &str,
    envs: &[(&str, &str)],
) {
    fs::write(repo.join(name), content).expect("write file");
    run_git(repo, &["add", "."]);
    run_git_with_env(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
        envs,
    );
}

fn rev_parse(repo: &Path, spec: &str) -> String {
    git_stdout(repo, &["rev-parse", spec])
}

fn git_path(repo: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(git_stdout(repo, &["rev-parse", "--git-path", path]));
    if path.is_absolute() {
        path
    } else {
        repo.join(path)
    }
}

fn commit_id(sha: &str) -> CommitId {
    CommitId(sha.into())
}

fn open_backend(repo: &Path) -> Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open repository")
}

/// root <- a <- b <- c <- d(HEAD); returns (root, a, b, c, d) shas.
fn linear_repo(repo: &Path) -> (String, String, String, String, String) {
    init_repo(repo);
    commit_file(repo, "file.txt", "root\n", "Root commit");
    let root = rev_parse(repo, "HEAD");
    commit_file(repo, "file.txt", "a\n", "Commit A");
    let a = rev_parse(repo, "HEAD");
    commit_file_with_env(
        repo,
        "file.txt",
        "b\n",
        "Commit B\n\nBody of B",
        &[
            ("GIT_AUTHOR_NAME", "Oldest Author"),
            ("GIT_AUTHOR_EMAIL", "oldest@example.com"),
            ("GIT_AUTHOR_DATE", "1600000000 +0230"),
        ],
    );
    let b = rev_parse(repo, "HEAD");
    commit_file(repo, "file.txt", "c\n", "Commit C");
    let c = rev_parse(repo, "HEAD");
    commit_file(repo, "file.txt", "d\n", "Commit D");
    let d = rev_parse(repo, "HEAD");
    (root, a, b, c, d)
}

#[test]
fn squash_replaces_linear_range_with_single_commit() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, a, b, _c, d) = linear_repo(&repo);
    let tree_before = rev_parse(&repo, "HEAD^{tree}");

    let backend = open_backend(&repo);
    backend
        .squash_commits_with_output(
            &commit_id(&b),
            &commit_id(&d),
            "Squashed message\n\nDetails",
        )
        .expect("squash commits");

    let head = rev_parse(&repo, "HEAD");
    assert_ne!(head, d, "HEAD must move to the squash commit");
    assert_eq!(rev_parse(&repo, "HEAD^{tree}"), tree_before);
    assert_eq!(rev_parse(&repo, "HEAD^"), a);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Squashed message\n\nDetails"
    );
    // Author is preserved from the oldest squashed commit; committer is fresh.
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%an|%ae|%ad", "--date=raw"]),
        "Oldest Author|oldest@example.com|1600000000 +0230"
    );
    assert_eq!(git_stdout(&repo, &["log", "-1", "--format=%cn"]), "You");
    // Exactly root, a, squash remain.
    assert_eq!(
        git_stdout(&repo, &["rev-list", "--count", "HEAD"]),
        "3".to_string()
    );
    // The reflog records the squash.
    assert!(
        git_stdout(&repo, &["reflog", "-1"]).contains("squash: 3 commits"),
        "reflog should mention the squash"
    );
}

#[test]
fn squash_leaves_dirty_worktree_and_index_untouched() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, b, _c, d) = linear_repo(&repo);

    fs::write(repo.join("staged.txt"), "staged\n").expect("write staged file");
    run_git(&repo, &["add", "staged.txt"]);
    fs::write(repo.join("file.txt"), "dirty\n").expect("dirty the worktree");

    let backend = open_backend(&repo);
    backend
        .squash_commits_with_output(&commit_id(&b), &commit_id(&d), "Squash")
        .expect("squash commits");

    // Note: git_stdout trims the output, which strips porcelain's leading
    // space from the first line; match without the column prefix.
    let status = git_stdout(&repo, &["status", "--porcelain"]);
    assert!(
        status.contains("A  staged.txt"),
        "staged file kept: {status}"
    );
    assert!(status.contains("M file.txt"), "dirty file kept: {status}");
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "dirty\n"
    );
}

#[test]
fn squash_works_on_detached_head_and_moves_branch_when_attached() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, b, _c, d) = linear_repo(&repo);
    let branch = git_stdout(&repo, &["branch", "--show-current"]);

    // Attached: the branch ref moves.
    let backend = open_backend(&repo);
    backend
        .squash_commits_with_output(&commit_id(&b), &commit_id(&d), "Attached squash")
        .expect("squash commits");
    let squashed_head = rev_parse(&repo, "HEAD");
    assert_eq!(
        rev_parse(&repo, &format!("refs/heads/{branch}")),
        squashed_head
    );

    // Detached: HEAD itself moves, the branch stays.
    commit_file(&repo, "file.txt", "e\n", "Commit E");
    let e = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "f\n", "Commit F");
    let f = rev_parse(&repo, "HEAD");
    let branch_tip = rev_parse(&repo, &format!("refs/heads/{branch}"));
    run_git(&repo, &["checkout", "--detach"]);
    backend
        .squash_commits_with_output(&commit_id(&e), &commit_id(&f), "Detached squash")
        .expect("squash on detached HEAD");
    assert_eq!(
        rev_parse(&repo, "HEAD^"),
        squashed_head,
        "squash parent is the pre-range tip"
    );
    assert_eq!(
        rev_parse(&repo, &format!("refs/heads/{branch}")),
        branch_tip,
        "branch must not move while detached"
    );
}

#[test]
fn squash_with_stale_expected_head_fails_without_changing_refs() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, b, c, d) = linear_repo(&repo);

    let backend = open_backend(&repo);
    // Pretend the squash was prepared when `c` was HEAD.
    let err = backend
        .squash_commits_with_output(&commit_id(&b), &commit_id(&c), "Stale")
        .expect_err("stale expected head must fail");
    assert!(
        err.to_string().contains("HEAD moved"),
        "unexpected error: {err}"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), d, "refs must be unchanged");
}

#[test]
fn squash_rejects_merge_commit_in_range() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "Base");
    let base = rev_parse(&repo, "HEAD");
    run_git(&repo, &["checkout", "-b", "feature"]);
    commit_file(&repo, "feature.txt", "feature\n", "Feature");
    run_git(&repo, &["checkout", "-"]);
    commit_file(&repo, "main.txt", "main\n", "Main work");
    let main_work = rev_parse(&repo, "HEAD");
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--no-ff",
            "-m",
            "Merge feature",
            "feature",
        ],
    );
    let merge = rev_parse(&repo, "HEAD");

    let backend = open_backend(&repo);
    let err = backend
        .squash_commits_with_output(&commit_id(&main_work), &commit_id(&merge), "Nope")
        .expect_err("merge commit in range must fail");
    assert!(
        err.to_string().contains("exactly one parent"),
        "unexpected error: {err}"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), merge);
    let _ = base;
}

#[test]
fn squash_rejects_root_commit_as_oldest() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "next\n", "Next");
    let next = rev_parse(&repo, "HEAD");

    let backend = open_backend(&repo);
    let err = backend
        .squash_commits_with_output(&commit_id(&root), &commit_id(&next), "Nope")
        .expect_err("root commit as oldest must fail");
    assert!(
        err.to_string().contains("exactly one parent"),
        "unexpected error: {err}"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), next);
}

#[test]
fn squash_rejects_non_ancestor_oldest() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    run_git(&repo, &["checkout", "-b", "side"]);
    commit_file(&repo, "side.txt", "side\n", "Side");
    let side = rev_parse(&repo, "HEAD");
    run_git(&repo, &["checkout", "-"]);
    commit_file(&repo, "file.txt", "main\n", "Main");
    let head = rev_parse(&repo, "HEAD");

    let backend = open_backend(&repo);
    let err = backend
        .squash_commits_with_output(&commit_id(&side), &commit_id(&head), "Nope")
        .expect_err("non-ancestor oldest must fail");
    assert!(
        err.to_string().contains("exactly one parent"),
        "unexpected error: {err}"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), head);
}

#[test]
fn squash_message_preview_combines_messages_oldest_first() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, b, _c, d) = linear_repo(&repo);

    let backend = open_backend(&repo);
    let preview = backend
        .squash_message_preview(&commit_id(&b), &commit_id(&d))
        .expect("build message preview");
    assert_eq!(preview, "Commit B\n\nBody of B\n\nCommit C\n\nCommit D");
}

#[test]
fn interactive_rebase_setup_captures_summary_and_full_message() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, a, b, _c, _d) = linear_repo(&repo);

    let backend = open_backend(&repo);
    let entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");

    // Oldest-first, and root itself is excluded (it is the base).
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].commit_id, a);
    assert_eq!(entries[0].summary, "Commit A");
    assert_eq!(entries[0].message, "Commit A");

    // Commit B has a multi-line body: summary is the subject, message is full.
    assert_eq!(entries[1].commit_id, b);
    assert_eq!(entries[1].summary, "Commit B");
    assert_eq!(entries[1].message, "Commit B\n\nBody of B");
}

#[test]
fn interactive_rebase_accepts_branch_sha_and_head_relative_bases() {
    for case in ["branch", "sha", "head-relative"] {
        let dir = tempfile::tempdir().expect("create tempdir");
        let repo = dir.path().join("repo");
        let (root, _a, _b, _c, _d) = linear_repo(&repo);
        run_git(&repo, &["branch", "base-branch", &root]);

        let base = match case {
            "branch" => "base-branch".to_string(),
            "sha" => root.clone(),
            "head-relative" => "HEAD~4".to_string(),
            _ => unreachable!(),
        };
        let expected_base = rev_parse(&repo, &base);

        let backend = open_backend(&repo);
        let mut entries = backend
            .list_commits_for_interactive_rebase(&base)
            .expect("list commits for interactive rebase");
        assert_eq!(entries.len(), 4, "case {case}");
        entries[0].action = InteractiveRebaseAction::Reword;
        entries[0].new_message = Some(format!("Reworded via {case} base"));

        backend
            .interactive_rebase_with_output(&base, &entries)
            .expect("interactive rebase");

        let exclude_base = format!("^{expected_base}");
        let first_rewritten = git_stdout(&repo, &["rev-list", "--reverse", "HEAD", &exclude_base])
            .lines()
            .next()
            .expect("first rewritten commit")
            .to_string();
        assert_eq!(
            rev_parse(&repo, &format!("{first_rewritten}^")),
            expected_base
        );
        assert_eq!(
            git_stdout(&repo, &["log", "-1", "--format=%B", &first_rewritten]),
            format!("Reworded via {case} base")
        );
    }
}

#[test]
fn interactive_rebase_edited_squash_message_is_applied_once() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, _a, _b, c, d) = linear_repo(&repo);

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    // Squash B into A with a fully edited message: the user's text must be
    // the final message verbatim — git must not re-append B's message.
    entries[0].action = InteractiveRebaseAction::Reword;
    entries[0].new_message = Some("Combined subject\n\nCombined body".to_string());
    entries[1].action = InteractiveRebaseAction::Squash;

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("interactive rebase");

    let exclude_base = format!("^{root}");
    let rewritten = git_stdout(&repo, &["rev-list", "--reverse", "HEAD", &exclude_base]);
    let rewritten: Vec<&str> = rewritten.lines().collect();
    assert_eq!(rewritten.len(), 3, "A+B squashed, C and D replayed");
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", rewritten[0]]),
        "Combined subject\n\nCombined body"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s", rewritten[1]]),
        git_stdout(&repo, &["log", "-1", "--format=%s", &c]),
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s", rewritten[2]]),
        git_stdout(&repo, &["log", "-1", "--format=%s", &d]),
    );
}

#[test]
fn interactive_rebase_edited_message_applies_at_trailing_fixup() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, _a, _b, _c, _d) = linear_repo(&repo);

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    // Run is squash B then fixup C: git opens the message editor at the fixup
    // step (the run's last fold), so the edited message must be applied there.
    entries[0].action = InteractiveRebaseAction::Reword;
    entries[0].new_message = Some("Edited run message".to_string());
    entries[1].action = InteractiveRebaseAction::Squash;
    entries[2].action = InteractiveRebaseAction::Fixup;

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("interactive rebase");

    let exclude_base = format!("^{root}");
    let rewritten = git_stdout(&repo, &["rev-list", "--reverse", "HEAD", &exclude_base]);
    let rewritten: Vec<&str> = rewritten.lines().collect();
    assert_eq!(rewritten.len(), 2, "A+B+C folded, D replayed");
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", rewritten[0]]),
        "Edited run message"
    );
}

#[test]
fn interactive_rebase_edited_squash_message_survives_conflict() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "a\n", "Prerequisite");
    commit_file(&repo, "file.txt", "b\n", "Target");
    commit_file(&repo, "later.txt", "later\n", "Squash into target");

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    // Dropping the prerequisite makes replaying the target conflict; the
    // edited squash-run message must still be applied after continue.
    entries[0].action = InteractiveRebaseAction::Drop;
    entries[1].action = InteractiveRebaseAction::Reword;
    entries[1].new_message = Some("Squashed after conflict\n\nKept body".to_string());
    entries[2].action = InteractiveRebaseAction::Squash;

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("start interactive rebase");
    assert!(backend.rebase_in_progress().expect("read rebase state"));

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    let continue_output = backend
        .rebase_continue_with_output()
        .expect("continue interactive rebase");

    assert!(
        !backend.rebase_in_progress().expect("read rebase state"),
        "rebase still in progress after continue: {continue_output:?}"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Squashed after conflict\n\nKept body"
    );
}

#[test]
fn interactive_rebase_reword_works_in_repo_path_with_spaces() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo with spaces");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "a\n", "Prerequisite");
    commit_file(&repo, "file.txt", "b\n", "Reword me");

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[0].action = InteractiveRebaseAction::Drop;
    entries[1].action = InteractiveRebaseAction::Reword;
    entries[1].new_message = Some("Reworded in spaced path".to_string());

    // The conflict pause forces the continue path, whose persisted editor
    // lives under the space-containing repo's .git dir — the case that
    // word-splits when GIT_EDITOR is not shell-quoted.
    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("start interactive rebase");
    assert!(backend.rebase_in_progress().expect("read rebase state"));

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    let continue_output = backend
        .rebase_continue_with_output()
        .expect("continue interactive rebase");

    assert!(
        !backend.rebase_in_progress().expect("read rebase state"),
        "rebase still in progress after continue: {continue_output:?}"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Reworded in spaced path"
    );
}

/// Runs git allowing a non-zero exit (a rebase pausing at a conflict).
fn run_git_allow_fail(repo: &Path, args: &[&str], envs: &[(&str, &str)]) {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let cmd = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.status().expect("git command to run");
}

/// Writes a GIT_SEQUENCE_EDITOR script that installs `todo` verbatim,
/// simulating an interactive rebase planned outside GitComet.
#[cfg(unix)]
fn write_external_seq_editor(dir: &Path, todo: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let todo_file = dir.join("external-todo");
    fs::write(&todo_file, todo).expect("write external todo");
    let script = dir.join("external-seq-editor.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\ncp \"{}\" \"$1\"\n", todo_file.display()),
    )
    .expect("write seq editor script");
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    script
}

/// root <- P("Prerequisite") <- T("Reword me"); dropping P makes replaying T
/// conflict. Returns (root, p, t).
fn conflict_repo(repo: &Path) -> (String, String, String) {
    init_repo(repo);
    commit_file(repo, "file.txt", "root\n", "Root");
    let root = rev_parse(repo, "HEAD");
    commit_file(repo, "file.txt", "a\n", "Prerequisite");
    let p = rev_parse(repo, "HEAD");
    commit_file(repo, "file.txt", "b\n", "Reword me");
    let t = rev_parse(repo, "HEAD");
    (root, p, t)
}

#[cfg(unix)]
#[test]
fn rebase_continue_blocks_external_rebase_with_pending_reword() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, p, t) = conflict_repo(&repo);

    let seq_editor = write_external_seq_editor(dir.path(), &format!("drop {p}\nreword {t}\n"));
    run_git_allow_fail(
        &repo,
        &["rebase", "-i", &root],
        &[(
            "GIT_SEQUENCE_EDITOR",
            seq_editor.to_str().expect("utf-8 path"),
        )],
    );

    let backend = open_backend(&repo);
    assert!(backend.rebase_in_progress().expect("read rebase state"));

    // Resolve the conflict so only the guard stands between continue and
    // silently finalizing the reword with its unedited message.
    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);

    let err = backend
        .rebase_continue_with_output()
        .expect_err("continue must refuse to finalize an unplanned reword");
    assert!(
        err.to_string().contains("pending reword/squash"),
        "unexpected error: {err}"
    );
    assert!(
        backend.rebase_in_progress().expect("read rebase state"),
        "rebase must stay paused and recoverable"
    );
}

#[cfg(unix)]
#[test]
fn rebase_continue_allows_external_rebase_without_message_steps() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, p, t) = conflict_repo(&repo);

    let seq_editor = write_external_seq_editor(dir.path(), &format!("drop {p}\npick {t}\n"));
    run_git_allow_fail(
        &repo,
        &["rebase", "-i", &root],
        &[(
            "GIT_SEQUENCE_EDITOR",
            seq_editor.to_str().expect("utf-8 path"),
        )],
    );

    let backend = open_backend(&repo);
    assert!(backend.rebase_in_progress().expect("read rebase state"));

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    backend
        .rebase_continue_with_output()
        .expect("continue a pick-only external rebase");

    assert!(!backend.rebase_in_progress().expect("read rebase state"));
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "Reword me"
    );
}

#[test]
fn rebase_continue_rejects_damaged_persisted_reword_state() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, _p, _t) = conflict_repo(&repo);

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[0].action = InteractiveRebaseAction::Drop;
    entries[1].action = InteractiveRebaseAction::Reword;
    entries[1].new_message = Some("Edited".to_string());

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("start interactive rebase");
    assert!(backend.rebase_in_progress().expect("read rebase state"));

    // Damage the persisted plan: the state dir survives but the editor is gone.
    let persisted = git_path(&repo, "rebase-merge/gitcomet-reword");
    let editor = fs::read_dir(&persisted)
        .expect("read persisted state dir")
        .map(|e| e.expect("dir entry").path())
        .find(|p| p.is_file())
        .expect("persisted editor script");
    fs::remove_file(editor).expect("remove persisted editor");

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);

    let err = backend
        .rebase_continue_with_output()
        .expect_err("continue must reject incomplete reword state");
    assert!(
        err.to_string().contains("incomplete"),
        "unexpected error: {err}"
    );
    assert!(
        backend.rebase_in_progress().expect("read rebase state"),
        "rebase must stay paused and recoverable"
    );
}

#[test]
fn interactive_rebase_preserves_current_reword_message_across_conflict() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "a\n", "Prerequisite");
    commit_file(&repo, "file.txt", "b\n", "Reword me");

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[0].action = InteractiveRebaseAction::Drop;
    entries[1].action = InteractiveRebaseAction::Reword;
    entries[1].new_message = Some("Reworded after conflict\n\nPreserved body".to_string());

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("start interactive rebase");
    assert!(backend.rebase_in_progress().expect("read rebase state"));

    let persisted = git_path(&repo, "rebase-merge/gitcomet-reword");
    assert!(persisted.is_dir(), "reword state must survive the pause");

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    let continue_output = backend
        .rebase_continue_with_output()
        .expect("continue interactive rebase");

    assert!(
        !backend.rebase_in_progress().expect("read rebase state"),
        "rebase still in progress after continue: {continue_output:?}; status: {}",
        git_stdout(&repo, &["status", "--porcelain=v2", "--branch"])
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Reworded after conflict\n\nPreserved body"
    );
    assert!(
        !persisted.exists(),
        "Git must clean up completed rebase state"
    );
}

#[test]
fn interactive_rebase_preserves_later_reword_message_across_conflict() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "a\n", "Prerequisite");
    commit_file(&repo, "file.txt", "b\n", "Conflicting commit");
    commit_file(&repo, "later.txt", "later\n", "Later reword");

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[0].action = InteractiveRebaseAction::Drop;
    entries[2].action = InteractiveRebaseAction::Reword;
    entries[2].new_message = Some("Later message preserved".to_string());

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("start interactive rebase");
    assert!(backend.rebase_in_progress().expect("read rebase state"));

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    let continue_output = backend
        .rebase_continue_with_output()
        .expect("continue interactive rebase");

    assert!(
        !backend.rebase_in_progress().expect("read rebase state"),
        "rebase still in progress after continue: {continue_output:?}; status: {}",
        git_stdout(&repo, &["status", "--porcelain=v2", "--branch"])
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Later message preserved"
    );
}
