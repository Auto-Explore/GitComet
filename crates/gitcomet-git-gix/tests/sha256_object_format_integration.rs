//! Git's SHA-256 object format (`git init --object-format=sha256`): the backend
//! must open such repositories and carry 256-bit ids through the ordinary
//! paths — status, log, diff, staging, commit, indexed history, reflog, file
//! history and reference resolution.

use gitcomet_core::domain::{
    CommitId, DiffArea, DiffLineKind, DiffTarget, FileStatusKind, HistoryMode,
};
use gitcomet_core::services::{CancellationToken, GitBackend, GitRepository};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let output = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn init_sha256_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();
    git(repo, &["init", "-q", "--object-format=sha256"]);
    git(repo, &["config", "user.email", "you@example.com"]);
    git(repo, &["config", "user.name", "You"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
    dir
}

fn open(repo: &Path) -> Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open SHA-256 repository")
}

fn head_hex(repo: &Path) -> String {
    String::from_utf8(git(repo, &["rev-parse", "HEAD"]))
        .expect("HEAD is utf-8")
        .trim()
        .to_owned()
}

fn commit_through_git(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", message]);
    head_hex(repo)
}

/// A SHA-256 repository with two commits touching `notes.txt`, oldest first.
fn two_commit_repo() -> (tempfile::TempDir, String, String) {
    let dir = init_sha256_repo();
    let repo = dir.path();
    fs::write(repo.join("notes.txt"), "one\n").expect("write notes");
    let first = commit_through_git(repo, "first");
    fs::write(repo.join("notes.txt"), "one\ntwo\n").expect("modify notes");
    let second = commit_through_git(repo, "second");
    (dir, first, second)
}

#[test]
fn sha256_repository_status_log_diff_staging_and_commit() {
    let dir = init_sha256_repo();
    let repo = dir.path();

    fs::write(repo.join("notes.txt"), "one\n").expect("write notes");
    let first = commit_through_git(repo, "first");
    assert_eq!(first.len(), 64, "fixture must use SHA-256 ids");

    let opened = open(repo);

    let status = opened.status().expect("status on clean SHA-256 repo");
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());

    fs::write(repo.join("notes.txt"), "one\ntwo\n").expect("modify notes");
    let status = opened.status().expect("status after modifying a file");
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, PathBuf::from("notes.txt"));
    assert_eq!(status.unstaged[0].kind, FileStatusKind::Modified);

    let diff = opened
        .diff_parsed(&DiffTarget::WorkingTree {
            path: PathBuf::from("notes.txt"),
            area: DiffArea::Unstaged,
        })
        .expect("parse working-tree diff");
    assert!(
        diff.lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Add && line.text.as_ref() == "+two"),
        "diff should contain the added line: {:?}",
        diff.lines
    );

    let page = opened.log_head_page(10, None).expect("log page");
    assert_eq!(page.commits.len(), 1);
    assert_eq!(page.commits[0].id.as_ref(), first);

    opened
        .stage(&[Path::new("notes.txt")])
        .expect("stage through the backend");
    let status = opened.status().expect("status after staging");
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, PathBuf::from("notes.txt"));
    assert!(status.unstaged.is_empty());

    opened
        .commit("second through the backend")
        .expect("commit through the backend");
    let second = opened
        .head_commit_id()
        .expect("head id")
        .expect("a head commit");
    assert_eq!(second.as_ref().len(), 64);
    assert_eq!(second.as_ref(), head_hex(repo));
    assert_eq!(
        String::from_utf8(git(repo, &["cat-file", "-t", second.as_ref()]))
            .unwrap()
            .trim(),
        "commit",
        "git must read the backend-written object as a commit"
    );

    let status = opened.status().expect("status after committing");
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());

    let commit_diff = opened
        .diff_parsed(&DiffTarget::Commit {
            commit_id: second.clone(),
            path: None,
        })
        .expect("parse commit diff");
    assert!(
        commit_diff
            .lines
            .iter()
            .any(|line| line.kind == DiffLineKind::Add && line.text.as_ref() == "+two"),
        "commit diff should contain the added line"
    );
}

#[test]
fn sha256_indexed_history_round_trips_256_bit_ids() {
    let (dir, first, second) = two_commit_repo();
    let repo = dir.path();

    let opened = open(repo);
    let cancellation = CancellationToken::new();
    let index = opened
        .build_history_index(HistoryMode::FullReachable, None, &cancellation, &mut |_| {})
        .expect("build history index")
        .expect("gix backend indexes history");
    assert_eq!(index.len(), 2);

    let range = opened
        .read_history_range(&index, 0..2, &cancellation)
        .expect("read indexed history range");
    assert_eq!(range.commits.len(), 2);
    assert_eq!(range.commits[0].id.as_ref(), second);
    assert!(
        range
            .commits
            .iter()
            .all(|commit| commit.id.as_ref().len() == 64),
        "indexed history must keep 256-bit ids"
    );
    assert_eq!(range.commits[0].parent_ids.len(), 1);
    assert_eq!(range.commits[0].parent_ids[0].as_ref(), first);
    assert!(range.commits[1].parent_ids.is_empty());
}

#[test]
fn sha256_reference_resolution_resolves_full_and_abbreviated_ids() {
    let (dir, _, second) = two_commit_repo();
    let opened = open(dir.path());

    let resolved = opened
        .resolve_commit(&CommitId(second.clone().into()))
        .expect("resolve full id");
    assert_eq!(resolved.id.as_ref(), second);
    for (label, prefix) in [
        ("even", second[..12].to_owned()),
        ("odd", second[..13].to_owned()),
        ("forty", second[..40].to_owned()),
        ("uppercase", second[..12].to_uppercase()),
    ] {
        let resolved_short = opened
            .resolve_commit(&CommitId(prefix.into()))
            .expect("resolve abbreviated id");
        assert_eq!(resolved_short.id.as_ref(), second, "{label} prefix");
    }
    assert!(
        opened
            .resolve_commit(&CommitId("f".repeat(12).into()))
            .is_err(),
        "a prefix that matches nothing must not resolve"
    );
}

#[test]
fn sha256_commit_details_reflog_and_file_history() {
    let (dir, first, second) = two_commit_repo();
    let opened = open(dir.path());

    let details = opened
        .commit_details(&CommitId(second.clone().into()))
        .expect("commit details");
    assert_eq!(details.id.as_ref(), second);
    assert!(details.message.contains("second"));
    assert_eq!(details.parent_ids.len(), 1);
    assert_eq!(details.parent_ids[0].as_ref(), first);

    let reflog = opened.reflog_head(10).expect("reflog");
    assert!(!reflog.is_empty());
    assert!(
        reflog.iter().all(|entry| entry.new_id.as_ref().len() == 64),
        "reflog ids must keep 256-bit widths"
    );
    assert_eq!(reflog[0].new_id.as_ref(), second);

    let file_page = opened
        .log_file_page(Path::new("notes.txt"), 10, None)
        .expect("file history");
    assert_eq!(file_page.commits.len(), 2);
    assert_eq!(file_page.commits[0].id.as_ref(), second);
    assert_eq!(file_page.commits[1].id.as_ref(), first);
}

#[test]
fn sha256_compound_abbreviated_specs_resolve() {
    let (dir, first, second) = two_commit_repo();
    let repo = dir.path();
    let opened = open(repo);
    let short = &second[..12];

    let parent = opened
        .resolve_commit(&CommitId(format!("{short}~1").into()))
        .expect("abbreviated id with ~1 resolves");
    assert_eq!(parent.id.as_ref(), first);
    let peeled = opened
        .resolve_commit(&CommitId(format!("{short}^{{commit}}").into()))
        .expect("abbreviated id with ^{commit} resolves");
    assert_eq!(peeled.id.as_ref(), second);

    // Loose and packed objects must behave the same.
    git(repo, &["gc", "-q"]);
    let packed_parent = opened
        .resolve_commit(&CommitId(format!("{short}~1").into()))
        .expect("packed abbreviated id with ~1 resolves");
    assert_eq!(packed_parent.id.as_ref(), first);
}
