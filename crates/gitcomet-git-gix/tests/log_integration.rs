use gitcomet_core::domain::{CommitId, FileStatusKind};
use gitcomet_core::error::{ErrorKind, GitFailureId};
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::path::Path;
use std::process::Command;
#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_SYSTEM", NULL_DEVICE)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_SYSTEM", NULL_DEVICE)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .output()
        .expect("git command to run");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_integration_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let output = match Command::new("git")
            .args(["difftool", "--tool-help"])
            .output()
        {
            Ok(output) => output,
            Err(_) => return true,
        };
        if output.status.success() {
            return true;
        }
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        !is_git_shell_startup_failure(&text)
    })
}

fn require_git_shell_for_remote_tracking_test() -> bool {
    #[cfg(windows)]
    {
        if !git_shell_available_for_integration_tests() {
            eprintln!(
                "skipping remote-tracking integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

fn git_remote_url(path: &Path) -> String {
    if cfg!(windows) {
        // Use a file:// URL so drive-letter paths are never treated as
        // scp-style host:path remotes.
        let normalized = path.to_string_lossy().replace('\\', "/");
        format!("file:///{normalized}")
    } else {
        path.to_string_lossy().into_owned()
    }
}

#[test]
fn log_all_branches_includes_remote_tracking_branches() {
    if !require_git_shell_for_remote_tracking_test() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");

    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(&repo, &["add", "a.txt"]);
    run_git(&repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    run_git(&repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("b.txt"), "two\n").unwrap();
    run_git(&repo, &["add", "b.txt"]);
    run_git(&repo, &["-c", "commit.gpgsign=false", "commit", "-m", "C"]);
    let feature_tip = git_stdout(&repo, &["rev-parse", "HEAD"]);

    run_git(
        dir.path(),
        &["init", "--bare", "-b", "main", origin.to_str().unwrap()],
    );
    let origin_url = git_remote_url(&origin);
    run_git(&repo, &["remote", "add", "origin", origin_url.as_str()]);
    run_git(&repo, &["push", "-u", "origin", "feature"]);

    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["branch", "-D", "feature"]);
    run_git(&repo, &["fetch", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();

    let head = opened.log_head_page(200, None).unwrap();
    assert!(
        !head.commits.iter().any(|c| c.id.0 == feature_tip),
        "head log unexpectedly contains feature commit"
    );

    let all = opened.log_all_branches_page(200, None).unwrap();
    assert!(
        all.commits.iter().any(|c| c.id.0 == feature_tip),
        "all-branches log should include remote-tracking branch commit"
    );
}

#[test]
fn log_all_branches_includes_nonstandard_ref_namespaces() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    run_git(repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("b.txt"), "two\n").unwrap();
    run_git(repo, &["add", "b.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "C"]);
    let feature_tip = git_stdout(repo, &["rev-parse", "HEAD"]);

    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["branch", "-D", "feature"]);
    run_git(
        repo,
        &[
            "update-ref",
            "refs/branch-heads/feature",
            feature_tip.as_str(),
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let all = opened.log_all_branches_page(200, None).unwrap();
    assert!(
        all.commits.iter().any(|c| c.id.0 == feature_tip),
        "all-branches log should include commits reachable from refs outside refs/heads and refs/remotes"
    );
}

#[test]
fn log_all_branches_does_not_include_tag_only_tips() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    run_git(repo, &["checkout", "-b", "tag-only"]);
    std::fs::write(repo.join("b.txt"), "two\n").unwrap();
    run_git(repo, &["add", "b.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);
    let tag_only_tip = git_stdout(repo, &["rev-parse", "HEAD"]);

    run_git(
        repo,
        &[
            "-c",
            "tag.gpgSign=false",
            "tag",
            "-a",
            "-m",
            "tag",
            "v0.0",
            tag_only_tip.as_str(),
        ],
    );
    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["branch", "-D", "tag-only"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let all = opened.log_all_branches_page(200, None).unwrap();
    assert!(
        !all.commits.iter().any(|c| c.id.0 == tag_only_tip),
        "all-branches log should not be expanded by tag-only tips"
    );
}

#[test]
fn log_all_branches_ignores_non_commit_refs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);
    let head = git_stdout(repo, &["rev-parse", "HEAD"]);

    let blob = git_stdout(repo, &["hash-object", "-w", "a.txt"]);
    run_git(repo, &["update-ref", "refs/blob-test", blob.as_str()]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let all = opened.log_all_branches_page(200, None).unwrap();

    assert_eq!(all.commits.len(), 1);
    assert_eq!(all.commits[0].id.0, head);
    assert_eq!(all.commits[0].summary, "A");
}

#[test]
fn empty_repo_log_and_head_branch_do_not_error() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert_eq!(opened.current_branch().unwrap(), "main");
    assert!(opened.log_head_page(200, None).unwrap().commits.is_empty());
    assert!(
        opened
            .log_all_branches_page(200, None)
            .unwrap()
            .commits
            .is_empty()
    );
}

#[test]
fn detached_head_reports_head_as_current_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);
    run_git(repo, &["checkout", "--detach", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    assert_eq!(opened.current_branch().unwrap(), "HEAD");
}

#[test]
fn log_head_page_limit_sets_next_cursor_and_supports_pagination() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let first = opened.log_head_page(1, None).unwrap();
    assert_eq!(first.commits.len(), 1);
    let first_id = first.commits[0].id.0.clone();
    let cursor = first.next_cursor.as_ref().expect("next cursor");

    let second = opened.log_head_page(10, Some(cursor)).unwrap();
    assert!(!second.commits.is_empty());
    assert!(
        second.commits.iter().all(|c| c.id.0 != first_id),
        "paginated page should skip last-seen commit"
    );
}

#[test]
fn log_head_page_exact_limit_has_no_next_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let page = opened.log_head_page(2, None).unwrap();
    assert_eq!(page.commits.len(), 2);
    assert!(page.next_cursor.is_none());
}

#[test]
fn zero_limit_log_pages_return_empty_without_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let head = opened.log_head_page(0, None).unwrap();
    assert!(head.commits.is_empty());
    assert!(head.next_cursor.is_none());

    let all = opened.log_all_branches_page(0, None).unwrap();
    assert!(all.commits.is_empty());
    assert!(all.next_cursor.is_none());

    let file = opened.log_file_page(Path::new("a.txt"), 0, None).unwrap();
    assert!(file.commits.is_empty());
    assert!(file.next_cursor.is_none());
}

#[test]
fn log_file_page_follows_renames() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("docs/old name.txt"), "line 1\n").unwrap();
    run_git(repo, &["add", "docs/old name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add history file",
        ],
    );

    run_git(repo, &["mv", "docs/old name.txt", "docs/new name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "rename history file",
        ],
    );

    std::fs::write(repo.join("docs/new name.txt"), "line 1\nline 2\n").unwrap();
    run_git(repo, &["add", "docs/new name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "update history file",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let page = opened
        .log_file_page(Path::new("docs/new name.txt"), 10, None)
        .unwrap();
    let summaries: Vec<&str> = page.commits.iter().map(|c| c.summary.as_str()).collect();

    assert_eq!(
        summaries,
        vec![
            "update history file",
            "rename history file",
            "add history file"
        ]
    );
}

#[test]
fn log_file_page_cursor_paginates_rename_follow_history() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("docs/old name.txt"), "line 1\n").unwrap();
    run_git(repo, &["add", "docs/old name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add history file",
        ],
    );

    run_git(repo, &["mv", "docs/old name.txt", "docs/new name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "rename history file",
        ],
    );

    std::fs::write(repo.join("docs/new name.txt"), "line 1\nline 2\n").unwrap();
    run_git(repo, &["add", "docs/new name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "update history file once",
        ],
    );

    std::fs::write(repo.join("docs/new name.txt"), "line 1\nline 2\nline 3\n").unwrap();
    run_git(repo, &["add", "docs/new name.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "update history file twice",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let first = opened
        .log_file_page(Path::new("docs/new name.txt"), 2, None)
        .unwrap();
    let first_summaries: Vec<&str> = first.commits.iter().map(|c| c.summary.as_str()).collect();
    assert_eq!(
        first_summaries,
        vec!["update history file twice", "update history file once"]
    );

    let cursor = first.next_cursor.as_ref().expect("next cursor");
    let second = opened
        .log_file_page(Path::new("docs/new name.txt"), 2, Some(cursor))
        .unwrap();
    let second_summaries: Vec<&str> = second.commits.iter().map(|c| c.summary.as_str()).collect();
    assert_eq!(
        second_summaries,
        vec!["rename history file", "add history file"]
    );
    assert!(second.next_cursor.is_none());
}

#[test]
fn log_file_page_exact_limit_has_no_next_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let page = opened.log_file_page(Path::new("a.txt"), 2, None).unwrap();
    assert_eq!(page.commits.len(), 2);
    assert!(page.next_cursor.is_none());
}

#[test]
fn commit_details_reports_merge_parents_and_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("base.txt"), "base\n").unwrap();
    run_git(repo, &["add", "base.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("feature.txt"), "feature\n").unwrap();
    run_git(repo, &["add", "feature.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "main"]);
    std::fs::write(repo.join("main.txt"), "main\n").unwrap();
    run_git(repo, &["add", "main.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );
    run_git(
        repo,
        &["merge", "--no-ff", "feature", "-m", "merge feature branch"],
    );

    let merge_id = git_stdout(repo, &["rev-parse", "HEAD"]);
    let feature_id = git_stdout(repo, &["rev-parse", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let merge_details = opened
        .commit_details(&CommitId(merge_id.clone()))
        .expect("commit details");
    let feature_details = opened
        .commit_details(&CommitId(feature_id))
        .expect("feature commit details");

    assert_eq!(merge_details.id, CommitId(merge_id));
    assert_eq!(merge_details.message, "merge feature branch");
    assert!(
        !merge_details.committed_at.is_empty(),
        "expected committed_at to be set"
    );
    assert_eq!(merge_details.parent_ids.len(), 2);
    assert!(
        merge_details.files.is_empty(),
        "merge commit details should match `git show` and omit file rows without `-m`"
    );
    assert!(
        feature_details.files.iter().any(|f| {
            f.path == std::path::PathBuf::from("feature.txt") && f.kind == FileStatusKind::Added
        }),
        "expected feature commit details to include feature file"
    );
}

#[test]
fn commit_details_reports_root_and_rename_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "diff.renames", "true"]);

    std::fs::write(repo.join("old name.txt"), "hello\n").unwrap();
    run_git(repo, &["add", "old name.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "root commit"],
    );
    let root_id = git_stdout(repo, &["rev-parse", "HEAD"]);

    run_git(repo, &["mv", "old name.txt", "new name.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "rename file"],
    );
    let rename_id = git_stdout(repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let root_details = opened
        .commit_details(&CommitId(root_id.clone()))
        .expect("root commit details");
    let rename_details = opened
        .commit_details(&CommitId(rename_id.clone()))
        .expect("rename commit details");

    assert_eq!(root_details.id, CommitId(root_id));
    assert_eq!(root_details.message, "root commit");
    assert_eq!(root_details.parent_ids, Vec::<CommitId>::new());
    assert_eq!(
        root_details.files,
        vec![gitcomet_core::domain::CommitFileChange {
            path: Path::new("old name.txt").to_path_buf(),
            kind: FileStatusKind::Added,
        }]
    );

    assert_eq!(rename_details.id, CommitId(rename_id));
    assert_eq!(rename_details.message, "rename file");
    assert_eq!(rename_details.parent_ids.len(), 1);
    assert_eq!(
        rename_details.files,
        vec![gitcomet_core::domain::CommitFileChange {
            path: Path::new("new name.txt").to_path_buf(),
            kind: FileStatusKind::Renamed,
        }]
    );
}

#[test]
fn reflog_head_returns_recent_entries_with_indices() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let reflog = opened.reflog_head(2).unwrap();

    assert_eq!(reflog.len(), 2);
    assert_eq!(reflog[0].selector, "HEAD@{0}");
    assert_eq!(reflog[1].selector, "HEAD@{1}");
    assert_eq!(reflog[0].index, 0);
    assert_eq!(reflog[1].index, 1);
    assert!(reflog.iter().all(|entry| !entry.new_id.0.is_empty()));
    assert!(reflog.iter().all(|entry| entry.time.is_some()));
}

#[test]
fn reflog_head_returns_error_for_unborn_head() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .reflog_head(5)
        .expect_err("unborn HEAD should not have a reflog");

    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), "git reflog");
            assert_eq!(failure.id(), GitFailureId::CommandFailed);
            assert_eq!(failure.exit_code(), Some(128));
            assert_eq!(
                failure.detail(),
                Some("fatal: your current branch 'main' does not have any commits yet")
            );
            assert_eq!(failure.stdout(), b"");
            assert_eq!(
                failure.stderr(),
                b"fatal: your current branch 'main' does not have any commits yet\n"
            );
        }
        other => panic!("expected structured git failure, got {other:?}"),
    }
}

#[test]
fn log_all_branches_includes_older_stash_reflog_entries() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "base\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    std::fs::write(repo.join("stash.txt"), "first\n").unwrap();
    run_git(repo, &["add", "stash.txt"]);
    run_git(repo, &["stash", "push", "-m", "stash-one"]);

    std::fs::write(repo.join("stash.txt"), "second\n").unwrap();
    run_git(repo, &["add", "stash.txt"]);
    run_git(repo, &["stash", "push", "-m", "stash-two"]);

    let stash_ids = git_stdout(
        repo,
        &["reflog", "show", "-n2", "--format=%H", "refs/stash"],
    );
    let stash_ids: Vec<&str> = stash_ids.lines().collect();
    assert_eq!(stash_ids.len(), 2, "expected two stash reflog entries");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let all = opened.log_all_branches_page(200, None).unwrap();

    assert!(
        all.commits.iter().any(|c| c.id.0 == stash_ids[0]),
        "expected all-branches log to include stash tip"
    );
    assert!(
        all.commits.iter().any(|c| c.id.0 == stash_ids[1]),
        "expected all-branches log to include older stash reflog commit"
    );
}
