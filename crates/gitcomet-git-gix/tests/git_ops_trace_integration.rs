use gitcomet_core::domain::{DiffArea, DiffTarget};
use gitcomet_core::git_ops_trace::{self, GitOpTraceKind};
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run_git(repo: &Path, args: &[&str], empty_config: &Path) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", empty_config)
        .env("GIT_CONFIG_SYSTEM", empty_config)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

#[test]
fn git_op_trace_captures_backend_entry_points_once_per_operation() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path();

    // Use an empty file instead of NUL/​/dev/null — Git on Windows ARM64
    // cannot access the NUL device as a config path.
    // Keep the config file in a separate tempdir so it does not appear as an
    // untracked file inside the repo and skew the status assertion below.
    let config_dir = tempfile::tempdir().expect("create config tempdir");
    let empty_config = config_dir.path().join("empty.gitconfig");
    std::fs::write(&empty_config, "").expect("create empty git config");

    run_git(repo, &["init", "-b", "main"], &empty_config);
    run_git(
        repo,
        &["config", "user.email", "you@example.com"],
        &empty_config,
    );
    run_git(repo, &["config", "user.name", "You"], &empty_config);
    run_git(repo, &["config", "commit.gpgsign", "false"], &empty_config);

    std::fs::write(repo.join("story.txt"), "one\ntwo\nthree\n").expect("write base file");
    run_git(repo, &["add", "story.txt"], &empty_config);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
        &empty_config,
    );

    run_git(repo, &["checkout", "-b", "feature"], &empty_config);
    std::fs::write(repo.join("story.txt"), "one\ntwo feature\nthree\n")
        .expect("write branch commit");
    run_git(repo, &["add", "story.txt"], &empty_config);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature change",
        ],
        &empty_config,
    );

    std::fs::write(
        repo.join("story.txt"),
        "one\ntwo feature\nthree\nworking tree change\n",
    )
    .expect("write unstaged change");

    let backend = GixBackend;
    let opened = backend.open(repo).expect("open repo");

    let _capture = git_ops_trace::capture();

    let status = opened.status().expect("status");
    assert_eq!(status.unstaged.len(), 1);

    let page = opened.log_head_page(16, None).expect("log head page");
    assert!(page.commits.len() >= 2);

    let branches = opened.list_branches().expect("list branches");
    assert!(branches.iter().any(|branch| branch.name == "main"));
    assert!(branches.iter().any(|branch| branch.name == "feature"));

    let diff = opened
        .diff_parsed(&DiffTarget::WorkingTree {
            path: PathBuf::from("story.txt"),
            area: DiffArea::Unstaged,
        })
        .expect("diff parsed");
    assert!(!diff.lines.is_empty());

    let blame = opened
        .blame_file(Path::new("story.txt"), None)
        .expect("blame file");
    assert_eq!(blame.len(), 3);

    let snapshot = git_ops_trace::snapshot();
    assert_eq!(snapshot.status.calls, 1);
    assert_eq!(snapshot.log_walk.calls, 1);
    assert_eq!(snapshot.ref_enumerate.calls, 1);
    assert_eq!(snapshot.diff.calls, 1);
    assert_eq!(snapshot.blame.calls, 1);

    for kind in GitOpTraceKind::ALL {
        let stats = snapshot.stats(kind);
        assert!(stats.total_nanos > 0, "expected {kind:?} to record time");
        assert_eq!(stats.last_nanos, stats.max_nanos);
    }
}
