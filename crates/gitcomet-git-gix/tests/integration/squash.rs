use crate::test_git_env;
use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{GitBackend, GitRepository, InteractiveRebaseAction};
use gitcomet_core::test_support::git_fixture::append_config;
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn run_git(repo: &Path, args: &[&str]) {
    run_git_with_env(repo, args, &[]);
}

fn run_git_with_env(repo: &Path, args: &[&str], envs: &[(&str, &str)]) {
    let mut cmd = gitcomet_core::test_support::git_fixture::git_command();
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
    let mut cmd = gitcomet_core::test_support::git_fixture::git_command();
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
    append_config(
        repo,
        &[
            ("user.email", "you@example.com"),
            ("user.name", "You"),
            ("commit.gpgsign", "false"),
        ],
    );
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
fn interactive_rebase_flattens_merges_like_git() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "main-a.txt", "a\n", "Mainline A");
    run_git(&repo, &["checkout", "-b", "side"]);
    commit_file(&repo, "side.txt", "s\n", "Side B");
    run_git(&repo, &["checkout", "-"]);
    commit_file(&repo, "main-c.txt", "c\n", "Mainline C");
    run_git(&repo, &["merge", "--no-ff", "side", "-m", "Merge side"]);
    commit_file(&repo, "top.txt", "t\n", "Top D");

    let backend = open_backend(&repo);
    let entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");

    // The listing matches git's flattened todo: the merge commit is
    // excluded (a `pick <merge>` todo is rejected by git) and the side
    // branch is linearized in, parents always before children.
    let summaries: Vec<&str> = entries.iter().map(|e| e.summary.as_str()).collect();
    assert!(
        !summaries.contains(&"Merge side"),
        "merge must be excluded, got {summaries:?}"
    );
    assert_eq!(entries.len(), 4, "got {summaries:?}");
    assert_eq!(summaries[0], "Mainline A");
    assert_eq!(summaries[3], "Top D");
    assert!(summaries.contains(&"Side B") && summaries.contains(&"Mainline C"));

    // The generated todo must be executable end to end.
    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("interactive rebase over a flattened merge completes");

    let range = format!("{root}..HEAD");
    assert_eq!(
        git_stdout(&repo, &["rev-list", "--merges", "--count", &range]).trim(),
        "0",
        "history is linearized"
    );
    assert_eq!(
        git_stdout(&repo, &["rev-list", "--count", &range]).trim(),
        "4"
    );
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
fn interactive_rebase_setup_survives_separator_bytes_in_messages() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    // Record/unit separator bytes are legal in commit messages and must not
    // corrupt record framing.
    commit_file(
        &repo,
        "file.txt",
        "a\n",
        "Weird subject\n\nBody with \x1e record and \x1f unit bytes",
    );
    let a = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "b\n", "Plain commit");
    let b = rev_parse(&repo, "HEAD");

    let backend = open_backend(&repo);
    let entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].commit_id, a);
    assert_eq!(entries[0].summary, "Weird subject");
    assert_eq!(
        entries[0].message,
        "Weird subject\n\nBody with \x1e record and \x1f unit bytes"
    );
    assert_eq!(entries[1].commit_id, b);
    assert_eq!(entries[1].summary, "Plain commit");
}

#[test]
fn interactive_rebase_drops_picks_that_become_empty() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "orig\n", "Root commit");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "x\n", "Commit X");
    commit_file(&repo, "file.txt", "orig\n", "Back to orig");
    commit_file(&repo, "file.txt", "x\n", "Commit X again");

    let backend = open_backend(&repo);
    let entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    // Replaying "Commit X again" directly after "Commit X" makes it empty;
    // the rebase must drop it and continue instead of stopping on a step
    // the UI has no skip control for.
    let reordered = vec![entries[0].clone(), entries[2].clone(), entries[1].clone()];

    let output = backend
        .interactive_rebase_with_output(&root, &reordered)
        .expect("become-empty pick should be dropped, not strand the rebase");

    assert_eq!(output.exit_code, Some(0));
    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );
    assert_eq!(git_stdout(&repo, &["rev-list", "--count", "HEAD"]), "3");
    assert_eq!(
        git_stdout(&repo, &["log", "-2", "--format=%s"]),
        "Back to orig\nCommit X"
    );
    assert_eq!(fs::read_to_string(repo.join("file.txt")).unwrap(), "orig\n");
}

#[test]
fn interactive_rebase_aborts_when_branch_changes_after_setup() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, _a, _b, _c, _d) = linear_repo(&repo);

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[0].action = InteractiveRebaseAction::Reword;
    entries[0].new_message = Some("Stale plan".to_string());

    // A commit lands after the setup snapshot: executing the stale plan
    // would install a todo that silently drops it.
    commit_file(&repo, "file.txt", "e\n", "Late commit");
    let head_before = rev_parse(&repo, "HEAD");
    let count_before = git_stdout(&repo, &["rev-list", "--count", "HEAD"]);

    let err = backend
        .interactive_rebase_with_output(&root, &entries)
        .expect_err("stale plan must be rejected");
    assert!(
        err.to_string().contains("changed since"),
        "unexpected error: {err}"
    );

    // Nothing was rewritten and no rebase is in progress.
    assert_eq!(rev_parse(&repo, "HEAD"), head_before);
    assert_eq!(
        git_stdout(&repo, &["rev-list", "--count", "HEAD"]),
        count_before
    );
    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );
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
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    let continue_output = backend
        .rebase_continue_with_output()
        .expect("continue interactive rebase");

    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state"),
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
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    let continue_output = backend
        .rebase_continue_with_output()
        .expect("continue interactive rebase");

    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state"),
        "rebase still in progress after continue: {continue_output:?}"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Reworded in spaced path"
    );
}

/// Git runs the reword editor with `--cleanup=strip`, which deletes every line
/// starting with the comment character, so a planned `#123 Fix crash` subject
/// and a `# Details` heading were dropped from the rewritten commit.
#[test]
fn interactive_rebase_reword_keeps_lines_starting_with_hash() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "a\n", "Reword me");

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[0].action = InteractiveRebaseAction::Reword;
    entries[0].new_message = Some("#123 Fix crash\n\n# Details\nbody".to_string());

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("interactive rebase");

    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "#123 Fix crash\n\n# Details\nbody"
    );
}

/// The continue path installs the persisted editor, so it has to keep the
/// planned lines that start with `#` as well.
#[test]
fn interactive_rebase_reword_keeps_lines_starting_with_hash_across_conflict() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, _p, _t) = conflict_repo(&repo);

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[0].action = InteractiveRebaseAction::Drop;
    entries[1].action = InteractiveRebaseAction::Reword;
    entries[1].new_message = Some("#77 Reworded\n\n# Notes\nkept".to_string());

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("start interactive rebase");
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    let continue_output = backend
        .rebase_continue_with_output()
        .expect("continue interactive rebase");

    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state"),
        "rebase still in progress after continue: {continue_output:?}"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "#77 Reworded\n\n# Notes\nkept"
    );
}

/// A squash with no planned message keeps git's own combined message, whose
/// instruction lines git comments out and must still strip.
#[test]
fn interactive_rebase_squash_without_planned_message_strips_git_instructions() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, _a, _b, _c, _d) = linear_repo(&repo);

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[1].action = InteractiveRebaseAction::Squash;
    entries[2].action = InteractiveRebaseAction::Reword;
    entries[2].new_message = Some("#9 Later reword".to_string());

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("interactive rebase");

    let exclude_base = format!("^{root}");
    let rewritten = git_stdout(&repo, &["rev-list", "--reverse", "HEAD", &exclude_base]);
    let rewritten: Vec<&str> = rewritten.lines().collect();
    assert_eq!(rewritten.len(), 3, "A+B squashed, C reworded, D replayed");
    let squashed = git_stdout(&repo, &["log", "-1", "--format=%B", rewritten[0]]);
    assert!(
        !squashed.contains("This is a combination"),
        "git's instruction lines must be stripped:\n{squashed}"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", rewritten[1]]),
        "#9 Later reword"
    );
}

/// A configured `core.commentChar=;` keeps a squashed commit's `#123` line.
/// Overriding it for every plan with a planned message (to `#`, which no
/// planned line started with) stripped that line from an unplanned squash.
#[test]
fn interactive_rebase_keeps_the_configured_comment_char_when_no_planned_line_collides() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    run_git(&repo, &["config", "core.commentChar", ";"]);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "a\n", "Target");
    commit_file(&repo, "file.txt", "b\n", "Squashed\n\n#123 keep me");
    commit_file(&repo, "file.txt", "c\n", "Other");

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[1].action = InteractiveRebaseAction::Squash;
    entries[2].action = InteractiveRebaseAction::Reword;
    entries[2].new_message = Some("Reworded".to_string());

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("interactive rebase");

    let exclude_base = format!("^{root}");
    let rewritten = git_stdout(&repo, &["rev-list", "--reverse", "HEAD", &exclude_base]);
    let rewritten: Vec<&str> = rewritten.lines().collect();
    assert_eq!(rewritten.len(), 2, "Target+Squashed folded, Other reworded");
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", rewritten[0]]),
        "Target\n\nSquashed\n\n#123 keep me"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", rewritten[1]]),
        "Reworded"
    );
}

/// Captures an expected Git failure while preparing a paused rebase.
#[cfg(unix)]
fn run_git_allow_fail(repo: &Path, args: &[&str], envs: &[(&str, &str)]) {
    let mut cmd = gitcomet_core::test_support::git_fixture::git_command();
    test_git_env::apply(&mut cmd);
    let cmd = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let output = cmd.output().expect("git command to run");
    assert!(
        !output.status.success(),
        "git {:?} unexpectedly succeeded:\n{}\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Writes a GIT_SEQUENCE_EDITOR script that installs `todo` verbatim,
/// simulating an interactive rebase planned outside GitComet.
#[cfg(unix)]
fn write_external_seq_editor(dir: &Path, todo: &str) -> PathBuf {
    let todo_file = dir.join("external-todo");
    fs::write(&todo_file, todo).expect("write external todo");
    let script = dir.join("external-seq-editor.sh");
    gitcomet_core::test_support::script::write_executable(
        &script,
        &format!("#!/bin/sh\ncp \"{}\" \"$1\"\n", todo_file.display()),
    );
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
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

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
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state"),
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
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    backend
        .rebase_continue_with_output()
        .expect("continue a pick-only external rebase");

    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );
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
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

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
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state"),
        "rebase must stay paused and recoverable"
    );
}

#[test]
fn rebase_continue_with_unresolved_conflict_keeps_git_error() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, _p, _t) = conflict_repo(&repo);

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[0].action = InteractiveRebaseAction::Drop;

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("start interactive rebase (pauses at conflict)");
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

    // Continue without resolving: git fails without advancing, and that
    // failure must reach the caller instead of masquerading as a pause.
    let err = backend
        .rebase_continue_with_output()
        .expect_err("continue with unresolved conflicts must fail");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("conflict") || msg.contains("unmerged") || msg.contains("fix them"),
        "error should carry git's message: {err}"
    );
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );
}

#[cfg(unix)]
#[test]
fn rebase_continue_with_signing_failure_keeps_git_error() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (root, _p, _t) = conflict_repo(&repo);

    // Signing options are captured when the rebase starts, so a broken
    // signing setup must be in place before then. The dropped first entry
    // means no commit (and no signing) happens until continue.
    run_git(&repo, &["config", "commit.gpgsign", "true"]);
    run_git(&repo, &["config", "gpg.program", "false"]);

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    entries[0].action = InteractiveRebaseAction::Drop;

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("start interactive rebase (pauses at conflict)");

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);

    // The commit fails to sign: continue makes no progress and the failure
    // must not be reported as a successful pause.
    let err = backend
        .rebase_continue_with_output()
        .expect_err("continue must surface the signing failure");
    assert!(
        err.to_string().contains("commit"),
        "error should carry git's commit failure: {err}"
    );
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );
}

#[test]
fn rebase_continue_pausing_at_next_conflict_is_success() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "a.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "a.txt", "a1\n", "Prerequisite A");
    commit_file(&repo, "b.txt", "b1\n", "Prerequisite B");
    commit_file(&repo, "a.txt", "a2\n", "Conflicts A");
    commit_file(&repo, "b.txt", "b2\n", "Conflicts B");

    let backend = open_backend(&repo);
    let mut entries = backend
        .list_commits_for_interactive_rebase(&root)
        .expect("list commits for interactive rebase");
    // Dropping both prerequisites makes each remaining pick conflict in turn.
    entries[0].action = InteractiveRebaseAction::Drop;
    entries[1].action = InteractiveRebaseAction::Drop;

    backend
        .interactive_rebase_with_output(&root, &entries)
        .expect("start interactive rebase (pauses at first conflict)");
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

    fs::write(repo.join("a.txt"), "a2\n").expect("resolve first conflict");
    run_git(&repo, &["add", "a.txt"]);
    // Advancing to the second conflict is a pause, not a failure.
    backend
        .rebase_continue_with_output()
        .expect("advancing to the next conflict is a successful pause");
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

    fs::write(repo.join("b.txt"), "b2\n").expect("resolve second conflict");
    run_git(&repo, &["add", "b.txt"]);
    backend
        .rebase_continue_with_output()
        .expect("finish the rebase");
    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
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
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

    let persisted = git_path(&repo, "rebase-merge/gitcomet-reword");
    assert!(persisted.is_dir(), "reword state must survive the pause");

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    let continue_output = backend
        .rebase_continue_with_output()
        .expect("continue interactive rebase");

    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state"),
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
    assert!(
        backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );

    fs::write(repo.join("file.txt"), "b\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);
    let continue_output = backend
        .rebase_continue_with_output()
        .expect("continue interactive rebase");

    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state"),
        "rebase still in progress after continue: {continue_output:?}; status: {}",
        git_stdout(&repo, &["status", "--porcelain=v2", "--branch"])
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Later message preserved"
    );
}

#[test]
fn reword_head_keeps_staged_changes_out_of_the_commit() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, _b, c, _d) = linear_repo(&repo);
    let tree_before = rev_parse(&repo, "HEAD^{tree}");

    fs::write(repo.join("staged.txt"), "staged\n").expect("write staged file");
    run_git(&repo, &["add", "staged.txt"]);
    fs::write(repo.join("file.txt"), "dirty\n").expect("dirty the worktree");

    let backend = open_backend(&repo);
    let head = rev_parse(&repo, "HEAD");
    let outcome = backend
        .reword_commit_with_output(&commit_id(&head), "New D message\n\nWith a body")
        .expect("reword HEAD");

    let new_head = rev_parse(&repo, "HEAD");
    assert_ne!(new_head, head, "HEAD must move to the reworded commit");
    assert_eq!(outcome.rewritten_id, Some(commit_id(&new_head)));
    assert_eq!(rev_parse(&repo, "HEAD^"), c);
    assert_eq!(rev_parse(&repo, "HEAD^{tree}"), tree_before);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "New D message\n\nWith a body"
    );
    let status = git_stdout(&repo, &["status", "--porcelain"]);
    assert!(
        status.contains("A  staged.txt"),
        "staged file must stay staged: {status}"
    );
    assert!(status.contains("M file.txt"), "dirty file kept: {status}");
}

#[test]
fn reword_head_allows_an_empty_root_commit() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "Empty root",
        ],
    );
    let root = rev_parse(&repo, "HEAD");

    let backend = open_backend(&repo);
    backend
        .reword_commit_with_output(&commit_id(&root), "Reworded root")
        .expect("reword an empty root HEAD");

    assert_ne!(rev_parse(&repo, "HEAD"), root);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Reworded root"
    );
    assert_eq!(git_stdout(&repo, &["rev-list", "--count", "HEAD"]), "1");
}

#[test]
fn reword_older_commit_replays_its_descendants_unchanged() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, a, b, _c, _d) = linear_repo(&repo);
    let trees_before: Vec<String> = ["HEAD", "HEAD~1", "HEAD~2"]
        .iter()
        .map(|spec| rev_parse(&repo, &format!("{spec}^{{tree}}")))
        .collect();

    let backend = open_backend(&repo);
    let outcome = backend
        .reword_commit_with_output(&commit_id(&b), "Reworded B\n\n# not a comment")
        .expect("reword an older commit");

    let trees_after: Vec<String> = ["HEAD", "HEAD~1", "HEAD~2"]
        .iter()
        .map(|spec| rev_parse(&repo, &format!("{spec}^{{tree}}")))
        .collect();
    assert_eq!(trees_after, trees_before, "rewording must not change trees");
    assert_eq!(rev_parse(&repo, "HEAD~3"), a, "the parent is untouched");
    let reworded = rev_parse(&repo, "HEAD~2");
    assert_ne!(reworded, b);
    assert_eq!(outcome.rewritten_id, Some(commit_id(&reworded)));
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", "HEAD~2"]),
        "Reworded B\n\n# not a comment"
    );
    assert_eq!(
        git_stdout(
            &repo,
            &["log", "-1", "--format=%an|%ad", "--date=raw", "HEAD~2"]
        ),
        "Oldest Author|1600000000 +0230",
        "the author is kept"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "--format=%s", "-2"]),
        "Commit D\nCommit C"
    );
    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state")
    );
}

#[test]
fn reword_refuses_older_commits_it_cannot_replay_linearly() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "Base");
    let base = rev_parse(&repo, "HEAD");
    run_git(&repo, &["checkout", "-b", "feature"]);
    commit_file(&repo, "feature.txt", "feature\n", "Feature");
    let feature = rev_parse(&repo, "HEAD");
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
    commit_file(&repo, "after.txt", "after\n", "After merge");
    let head = rev_parse(&repo, "HEAD");

    let backend = open_backend(&repo);
    for (target, expected) in [
        (&main_work, "merge commit lies between"),
        (&feature, "merge commit lies between"),
        (&base, "merge commit lies between"),
    ] {
        let err = backend
            .reword_commit_with_output(&commit_id(target), "Nope")
            .expect_err("a merge in the replayed range must fail");
        assert!(
            err.to_string().contains(expected),
            "unexpected error: {err}"
        );
    }
    let merge = rev_parse(&repo, "HEAD^");
    let err = backend
        .reword_commit_with_output(&commit_id(&merge), "Nope")
        .expect_err("rewording a merge must fail");
    assert!(
        err.to_string().contains("is a merge commit"),
        "unexpected error: {err}"
    );
    let err = backend
        .reword_commit_with_output(&commit_id(&head), "   \n")
        .expect_err("an empty message must fail");
    assert!(err.to_string().contains("empty"), "unexpected error: {err}");
    assert_eq!(rev_parse(&repo, "HEAD"), head);

    // A root commit below HEAD, and a commit off the branch.
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    run_git(&repo, &["checkout", "-b", "side"]);
    commit_file(&repo, "side.txt", "side\n", "Side");
    let side = rev_parse(&repo, "HEAD");
    run_git(&repo, &["checkout", "-"]);
    commit_file(&repo, "file.txt", "next\n", "Next");
    let next = rev_parse(&repo, "HEAD");

    let backend = open_backend(&repo);
    let err = backend
        .reword_commit_with_output(&commit_id(&root), "Nope")
        .expect_err("an older root commit must fail");
    assert!(
        err.to_string().contains("root commit"),
        "unexpected error: {err}"
    );
    let err = backend
        .reword_commit_with_output(&commit_id(&side), "Nope")
        .expect_err("a commit off the branch must fail");
    assert!(
        err.to_string().contains("not part of the current branch"),
        "unexpected error: {err}"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), next);
}

#[test]
fn reword_head_keeps_comment_lines_like_the_rebase_path() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, b, _c, d) = linear_repo(&repo);
    append_config(&repo, &[("commit.cleanup", "strip")]);

    let backend = open_backend(&repo);
    let message = "Reworded\n\n# kept line";
    backend
        .reword_commit_with_output(&commit_id(&d), message)
        .expect("reword HEAD");
    backend
        .reword_commit_with_output(&commit_id(&b), message)
        .expect("reword an older commit");

    assert_eq!(git_stdout(&repo, &["log", "-1", "--format=%B"]), message);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", "HEAD~2"]),
        message
    );
}

#[test]
fn reword_head_keeps_a_merge_commits_parents() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "Base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    commit_file(&repo, "feature.txt", "feature\n", "Feature");
    run_git(&repo, &["checkout", "-"]);
    commit_file(&repo, "main.txt", "main\n", "Main work");
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
    let parents = git_stdout(&repo, &["log", "-1", "--format=%P"]);

    let backend = open_backend(&repo);
    backend
        .reword_commit_with_output(&commit_id(&merge), "Merge the feature branch")
        .expect("reword a merge HEAD");

    assert_ne!(rev_parse(&repo, "HEAD"), merge);
    assert_eq!(git_stdout(&repo, &["log", "-1", "--format=%P"]), parents);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Merge the feature branch"
    );
}

#[test]
fn reword_rolls_back_when_a_hook_rejects_an_older_commits_message() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, b, _c, d) = linear_repo(&repo);
    let hook = git_path(&repo, "hooks/commit-msg");
    fs::create_dir_all(hook.parent().unwrap()).expect("create hooks dir");
    fs::write(
        &hook,
        "#!/bin/sh\nif grep -q Rejected \"$1\"; then echo 'hook says no' >&2; exit 1; fi\n",
    )
    .expect("write hook");
    #[cfg(unix)]
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("make hook executable");

    let backend = open_backend(&repo);
    let err = backend
        .reword_commit_with_output(&commit_id(&b), "Rejected message")
        .expect_err("the hook rejects the message");
    assert!(
        err.to_string().contains("hook says no"),
        "unexpected error: {err}"
    );
    assert!(
        !backend
            .rebase_in_progress(&gitcomet_core::services::CancellationToken::new())
            .expect("read rebase state"),
        "the rejected reword must not leave a rebase behind"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), d);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", "HEAD~2"]),
        "Commit B\n\nBody of B"
    );
}

fn write_reword_hook(repo: &Path, name: &str, script: &str) {
    let path = git_path(repo, &format!("hooks/{name}"));
    fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn reword_preserves_empty_commits_and_detached_head() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "a", "base\n", "Base");
    run_git(&repo, &["commit", "--allow-empty", "-m", "Empty target"]);
    let target = rev_parse(&repo, "HEAD");
    commit_file(&repo, "a", "later\n", "Later");
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "Empty descendant"],
    );
    let branch = git_stdout(&repo, &["branch", "--show-current"]);
    let tip = rev_parse(&repo, "HEAD");
    let before = git_stdout(&repo, &["log", "--format=%T|%an|%ae|%aI"]);
    run_git(&repo, &["checkout", "--detach"]);

    let outcome = open_backend(&repo)
        .reword_commit_with_output(&commit_id(&target), "New empty message")
        .unwrap();
    assert_eq!(
        outcome.rewritten_id,
        Some(commit_id(&rev_parse(&repo, "HEAD~2")))
    );
    assert_eq!(
        git_stdout(&repo, &["log", "--format=%T|%an|%ae|%aI"]),
        before
    );
    assert_eq!(git_stdout(&repo, &["rev-list", "--count", "HEAD"]), "4");
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", "HEAD~2"]),
        "New empty message"
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Empty descendant"
    );
    assert_eq!(git_stdout(&repo, &["branch", "--show-current"]), "");
    assert_eq!(
        rev_parse(&repo, &branch),
        tip,
        "detached reword must not move the branch"
    );
}

#[test]
fn reword_continue_refuses_an_unrelated_interactive_rebase() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let (root, _prerequisite, target) = conflict_repo(&repo);
    let backend = open_backend(&repo);
    let mut entries = backend.list_commits_for_interactive_rebase(&root).unwrap();
    entries[0].action = InteractiveRebaseAction::Drop;
    entries[1].action = InteractiveRebaseAction::Reword;
    entries[1].new_message = Some("New message".into());
    backend
        .interactive_rebase_with_output(&root, &entries)
        .unwrap();
    let before = git_stdout(&repo, &["status", "--porcelain=v2", "--branch"]);
    assert!(
        !backend
            .reword_commit_in_progress(&commit_id(&target), "New message")
            .unwrap()
    );
    let error = backend
        .continue_reword_commit_with_output(&commit_id(&target), "New message")
        .unwrap_err();
    assert!(error.to_string().contains("does not belong"), "{error}");
    assert_eq!(
        git_stdout(&repo, &["status", "--porcelain=v2", "--branch"]),
        before
    );
    run_git(&repo, &["rebase", "--abort"]);
}

#[test]
fn reword_success_without_a_matching_head_keeps_its_success() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let (root, _a, _b, _c, head) = linear_repo(&repo);
    // A post-rewrite hook models another process moving HEAD after the
    // commit was written but before the optional selection lookup.
    write_reword_hook(
        &repo,
        "post-rewrite",
        &format!("#!/bin/sh\ngit update-ref HEAD {root}\n"),
    );
    let outcome = open_backend(&repo)
        .reword_commit_with_output(&commit_id(&head), "New message")
        .expect("optional result lookup must not turn the rewrite into a failure");
    assert_eq!(outcome.rewritten_id, None);
    assert_eq!(rev_parse(&repo, "HEAD"), root);
    assert!(git_stdout(&repo, &["reflog", "--format=%gs"]).contains("commit (amend): New message"));
}

#[test]
fn reword_success_does_not_identify_an_unrelated_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let (root, _a, _b, parent, head) = linear_repo(&repo);
    let sibling = git_stdout(
        &repo,
        &[
            "commit-tree",
            &format!("{root}^{{tree}}"),
            "-p",
            &parent,
            "-m",
            "Different tree",
        ],
    );
    // Matching parents alone cannot identify the rewritten commit.
    write_reword_hook(
        &repo,
        "post-rewrite",
        &format!("#!/bin/sh\ngit update-ref HEAD {sibling}\n"),
    );
    let outcome = open_backend(&repo)
        .reword_commit_with_output(&commit_id(&head), "New message")
        .unwrap();
    assert_eq!(rev_parse(&repo, "HEAD"), sibling);
    assert_eq!(outcome.rewritten_id, None);
}

#[test]
fn reword_rebase_success_without_a_resolvable_target_keeps_its_success() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let (root, _a, target, _c, _head) = linear_repo(&repo);
    write_reword_hook(
        &repo,
        "post-rewrite",
        &format!("#!/bin/sh\nif [ \"$1\" = rebase ]; then git update-ref HEAD {root}; fi\n"),
    );
    let outcome = open_backend(&repo)
        .reword_commit_with_output(&commit_id(&target), "New message")
        .expect("an unresolvable HEAD~distance is optional metadata");
    assert_eq!(outcome.rewritten_id, None);
    assert_eq!(rev_parse(&repo, "HEAD"), root);
}

#[test]
fn reword_does_not_identify_a_same_tree_sibling_with_another_message() {
    for older in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let (_root, a, b, c, head) = linear_repo(&repo);
        let (target, parent, hook_kind) = if older {
            (&b, &a, "rebase")
        } else {
            (&head, &c, "amend")
        };
        let sibling = git_stdout(
            &repo,
            &[
                "commit-tree",
                &format!("{target}^{{tree}}"),
                "-p",
                parent,
                "-m",
                "Unrelated sibling message",
            ],
        );
        let moved_head = if older {
            let next = git_stdout(
                &repo,
                &[
                    "commit-tree",
                    &format!("{c}^{{tree}}"),
                    "-p",
                    &sibling,
                    "-m",
                    "Commit C",
                ],
            );
            git_stdout(
                &repo,
                &[
                    "commit-tree",
                    &format!("{head}^{{tree}}"),
                    "-p",
                    &next,
                    "-m",
                    "Commit D",
                ],
            )
        } else {
            sibling
        };
        write_reword_hook(
            &repo,
            "post-rewrite",
            &format!(
                "#!/bin/sh\nif [ \"$1\" = {hook_kind} ]; then git update-ref HEAD {moved_head}; fi\n"
            ),
        );
        let outcome = open_backend(&repo)
            .reword_commit_with_output(&commit_id(target), "Requested new message")
            .expect("Git completed the rewrite before the hook moved HEAD");
        assert_eq!(rev_parse(&repo, "HEAD"), moved_head);
        assert_eq!(outcome.rewritten_id, None, "older target: {older}");
    }
}

#[test]
fn reword_identification_accepts_git_message_whitespace_cleanup() {
    for cleanup in ["strip", "verbatim"] {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let (_root, _a, _b, _c, _head) = linear_repo(&repo);
        append_config(&repo, &[("commit.cleanup", cleanup)]);
        let message = " \t\r\nNew subject \t\r\n\n\n# Kept body\t\r\n\n";
        let backend = open_backend(&repo);
        for target_spec in ["HEAD", "HEAD~2"] {
            let target = commit_id(&rev_parse(&repo, target_spec));
            let outcome = backend.reword_commit_with_output(&target, message).unwrap();
            assert_eq!(
                outcome.rewritten_id,
                Some(commit_id(&rev_parse(&repo, target_spec))),
                "cleanup={cleanup}, target={target_spec}"
            );
        }
    }
}

#[test]
fn reword_retains_the_hook_error_when_rollback_also_fails() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let (_root, _a, target, _c, _head) = linear_repo(&repo);
    write_reword_hook(
        &repo,
        "commit-msg",
        "#!/bin/sh\necho 'original hook rejection' >&2\nprintf hold >\"$(git rev-parse --git-path HEAD.lock)\"\nexit 1\n",
    );
    let error = open_backend(&repo)
        .reword_commit_with_output(&commit_id(&target), "Rejected message")
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("original hook rejection"), "{message}");
    assert!(message.contains("Rollback also failed"), "{message}");
    assert!(message.contains("HEAD.lock"), "{message}");
    fs::remove_file(git_path(&repo, "HEAD.lock")).unwrap();
    run_git(&repo, &["rebase", "--abort"]);
}
