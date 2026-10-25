//! A product's history ref filter, and the refs a request hides, reach every
//! all-branches reader: pages, authors, the index, and snapshots.

use super::*;
use gitcomet_core::services::{
    CancellationToken, ConfiguredBackend, GitRepository, HistoryReadRequest, HistoryReadResult,
    HistoryRefFilter, HistoryWalk, RepositoryOptions,
};

fn commit_as(repo: &Path, author: &str, file: &str) -> String {
    std::fs::write(repo.join(file), file).unwrap();
    run_git(repo, &["add", file]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--author",
            &format!("{author} <{author}@example.com>"),
            "-m",
            file,
        ],
    );
    git_stdout(repo, &["rev-parse", "HEAD"])
}

fn messages(repo: &dyn GitRepository) -> Vec<String> {
    hiding(repo, &HistoryRefFilter::default())
}

fn hiding(repo: &dyn GitRepository, excluded: &HistoryRefFilter) -> Vec<String> {
    let result = repo
        .read_history(
            &HistoryWalk::new(HistoryMode::AllBranches, excluded.clone()),
            None,
            &HistoryReadRequest::Page {
                limit: 100,
                cursor: None,
                snapshot: None,
            },
            &CancellationToken::new(),
            &mut |_| {},
        )
        .unwrap();
    let HistoryReadResult::Page { page, .. } = result else {
        panic!("a first read is a page");
    };
    page.commits
        .iter()
        .map(|commit| commit.summary.to_string())
        .collect()
}

#[test]
fn excluded_refs_leave_every_all_branches_reader() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "user.name", "Committer"]);
    run_git(repo, &["config", "user.email", "committer@example.com"]);
    commit_as(repo, "base", "base.txt");
    // A review ref with a commit nothing else reaches.
    run_git(repo, &["checkout", "-q", "-b", "review"]);
    let review_tip = commit_as(repo, "reviewer", "review.txt");
    run_git(repo, &["checkout", "-q", "main"]);
    run_git(repo, &["update-ref", "refs/pull/1/head", &review_tip]);
    run_git(repo, &["branch", "-q", "-D", "review"]);
    commit_as(repo, "maintainer", "main.txt");

    let unfiltered = GixBackend.open(repo).unwrap();
    assert!(messages(unfiltered.as_ref()).contains(&"review.txt".to_string()));

    let options = RepositoryOptions::default()
        .with_history_ref_filter(HistoryRefFilter::excluding(["refs/pull"]));
    let backend = ConfiguredBackend::new(Arc::new(GixBackend), options);
    let filtered = backend.open(repo).unwrap();
    let cancel = CancellationToken::new();

    let shown = messages(filtered.as_ref());
    assert_eq!(shown, vec!["main.txt".to_string(), "base.txt".to_string()]);

    let authors = filtered
        .history_authors(&HistoryMode::AllBranches.into(), &cancel)
        .unwrap();
    assert!(
        !authors.iter().any(|author| author.as_ref() == "reviewer"),
        "{authors:?}"
    );

    let index = filtered
        .build_history_index(&HistoryMode::AllBranches.into(), None, &cancel, &mut |_| {})
        .unwrap()
        .expect("the gix backend indexes history");
    assert_eq!(index.len(), 2);
    let range = filtered
        .read_history_range(&index, 0..index.len(), &cancel)
        .unwrap();
    assert!(
        range
            .commits
            .iter()
            .all(|commit| commit.summary.as_ref() != "review.txt")
    );

    // HEAD's own history is unaffected by the filter.
    let head = filtered
        .log_history_mode_page(
            HistoryMode::FullReachable,
            10,
            None,
            &gitcomet_core::services::CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(head.commits.len(), 2);
}

fn remote_only_commit(repo: &Path, remote_ref: &str, file: &str) {
    run_git(repo, &["checkout", "-q", "-b", "scratch"]);
    let tip = commit_as(repo, file, file);
    run_git(repo, &["checkout", "-q", "main"]);
    run_git(repo, &["update-ref", remote_ref, &tip]);
    run_git(repo, &["branch", "-q", "-D", "scratch"]);
}

fn index_messages(repo: &dyn GitRepository, excluded: &HistoryRefFilter) -> Vec<String> {
    let cancel = CancellationToken::new();
    let index = repo
        .build_history_index(
            &HistoryWalk::new(HistoryMode::AllBranches, excluded.clone()),
            None,
            &cancel,
            &mut |_| {},
        )
        .unwrap()
        .expect("the gix backend indexes history");
    repo.read_history_range(&index, 0..index.len(), &cancel)
        .unwrap()
        .commits
        .iter()
        .map(|commit| commit.summary.to_string())
        .collect()
}

#[test]
fn hidden_remotes_leave_every_all_branches_reader() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "user.name", "Committer"]);
    run_git(repo, &["config", "user.email", "committer@example.com"]);
    let base = commit_as(repo, "base", "base.txt");
    run_git(repo, &["update-ref", "refs/remotes/origin/main", &base]);
    remote_only_commit(repo, "refs/remotes/origin/feature", "origin.txt");
    remote_only_commit(repo, "refs/remotes/origin2/feature", "origin2.txt");
    remote_only_commit(repo, "refs/remotes/upstream/feature", "upstream.txt");
    remote_only_commit(repo, "refs/pull/1/head", "pull.txt");
    commit_as(repo, "local", "local.txt");

    let options = RepositoryOptions::default()
        .with_history_ref_filter(HistoryRefFilter::excluding(["refs/pull"]));
    let backend = ConfiguredBackend::new(Arc::new(GixBackend), options);
    let opened = backend.open(repo).unwrap();
    let repo = opened.as_ref();
    let cancel = CancellationToken::new();
    let sorted = |mut messages: Vec<String>| {
        messages.sort();
        messages
    };
    let origin = HistoryRefFilter::excluding_remotes(["origin"]);
    let every_remote = HistoryRefFilter::excluding_remotes(["origin", "origin2", "upstream"]);

    assert_eq!(
        sorted(hiding(repo, &HistoryRefFilter::default())),
        [
            "base.txt",
            "local.txt",
            "origin.txt",
            "origin2.txt",
            "upstream.txt"
        ]
    );
    let without_origin = ["base.txt", "local.txt", "origin2.txt", "upstream.txt"];
    assert_eq!(sorted(hiding(repo, &origin)), without_origin);
    assert_eq!(sorted(index_messages(repo, &origin)), without_origin);
    let authors = repo
        .history_authors(
            &HistoryWalk::new(HistoryMode::AllBranches, origin.clone()),
            &cancel,
        )
        .unwrap();
    assert!(
        !authors.iter().any(|author| author.as_ref() == "origin.txt"),
        "{authors:?}"
    );
    assert!(
        authors
            .iter()
            .any(|author| author.as_ref() == "upstream.txt"),
        "{authors:?}"
    );

    assert_eq!(
        hiding(repo, &every_remote),
        ["local.txt", "base.txt"],
        "with every remote hidden only local history remains"
    );
    assert_eq!(
        index_messages(repo, &every_remote),
        ["local.txt", "base.txt"]
    );
    assert_eq!(
        sorted(hiding(repo, &HistoryRefFilter::default())),
        [
            "base.txt",
            "local.txt",
            "origin.txt",
            "origin2.txt",
            "upstream.txt"
        ],
        "showing the remotes again does not reuse the filtered walk"
    );

    let head = repo
        .read_history(
            &HistoryWalk::new(HistoryMode::FullReachable, every_remote.clone()),
            None,
            &HistoryReadRequest::Page {
                limit: 100,
                cursor: None,
                snapshot: None,
            },
            &cancel,
            &mut |_| {},
        )
        .unwrap();
    let HistoryReadResult::Page { page, .. } = head else {
        panic!("a first read is a page");
    };
    assert_eq!(
        page.commits.len(),
        2,
        "HEAD's own history ignores the filter"
    );
}
