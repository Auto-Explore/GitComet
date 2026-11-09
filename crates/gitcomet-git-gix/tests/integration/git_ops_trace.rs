use crate::git_cli::run_git;
use gitcomet_core::domain::{DiffArea, DiffTarget};
use gitcomet_core::git_ops_trace::{self, GitOpTraceKind};
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::path::{Path, PathBuf};

#[test]
fn git_op_trace_captures_backend_entry_points_once_per_operation() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("story.txt"), "one\ntwo\nthree\n").expect("write base file");
    run_git(repo, &["add", "story.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("story.txt"), "one\ntwo feature\nthree\n")
        .expect("write branch commit");
    run_git(repo, &["add", "story.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature change",
        ],
    );

    std::fs::write(
        repo.join("story.txt"),
        "one\ntwo feature\nthree\nworking tree change\n",
    )
    .expect("write unstaged change");

    let backend = GixBackend;
    let opened = backend.open(repo).expect("open repo");

    let _capture = git_ops_trace::capture();

    let status = opened
        .status(&gitcomet_core::services::CancellationToken::new())
        .expect("status");
    assert_eq!(status.unstaged.len(), 1);

    let page = opened
        .log_head_page(16, None, &gitcomet_core::services::CancellationToken::new())
        .expect("log head page");
    assert!(page.commits.len() >= 2);

    let branches = opened
        .list_branches(&gitcomet_core::services::CancellationToken::new())
        .expect("list branches");
    assert!(branches.iter().any(|branch| branch.name == "main"));
    assert!(branches.iter().any(|branch| branch.name == "feature"));

    let diff = opened
        .diff_parsed(
            &DiffTarget::working_tree(PathBuf::from("story.txt"), DiffArea::Unstaged),
            None,
            &gitcomet_core::services::CancellationToken::new(),
        )
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
