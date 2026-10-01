//! A product's history ref filter reaches every all-branches reader: pages,
//! authors, the index, and snapshots.

use super::*;
use gitcomet_core::services::{
    CancellationToken, ConfiguredBackend, GitRepository, HistoryReadRequest, HistoryReadResult,
    HistoryRefFilter, RepositoryOptions,
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
    let result = repo
        .read_history(
            HistoryMode::AllBranches,
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
        .history_authors(HistoryMode::AllBranches, &cancel)
        .unwrap();
    assert!(
        !authors.iter().any(|author| author.as_ref() == "reviewer"),
        "{authors:?}"
    );

    let index = filtered
        .build_history_index(HistoryMode::AllBranches, None, &cancel, &mut |_| {})
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
        .log_history_mode_page(HistoryMode::FullReachable, 10, None)
        .unwrap();
    assert_eq!(head.commits.len(), 2);
}
