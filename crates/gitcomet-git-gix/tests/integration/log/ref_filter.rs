//! A product's history ref filter, and the remotes a request hides, reach
//! every all-branches reader: pages, authors, the index, and snapshots.

use super::*;
use gitcomet_core::history_index::HistoryIndexHandle;
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
    hiding(repo, &[])
}

fn hiding(repo: &dyn GitRepository, remotes: &[&str]) -> Vec<String> {
    let result = repo
        .read_history(
            &HistoryWalk::new(HistoryMode::AllBranches, remotes.iter().copied()),
            None,
            &Default::default(),
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
        .history_authors(
            &HistoryMode::AllBranches.into(),
            &Default::default(),
            &cancel,
        )
        .unwrap();
    assert!(
        !authors.iter().any(|author| author.as_ref() == "reviewer"),
        "{authors:?}"
    );

    let index = filtered
        .build_history_index(
            &HistoryMode::AllBranches.into(),
            None,
            &Default::default(),
            &cancel,
            &mut |_| {},
        )
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

fn index_of(repo: &dyn GitRepository, remotes: &[&str]) -> HistoryIndexHandle {
    repo.build_history_index(
        &HistoryWalk::new(HistoryMode::AllBranches, remotes.iter().copied()),
        None,
        &Default::default(),
        &CancellationToken::new(),
        &mut |_| {},
    )
    .unwrap()
    .expect("the gix backend indexes history")
}

fn index_messages(repo: &dyn GitRepository, remotes: &[&str]) -> Vec<String> {
    let index = index_of(repo, remotes);
    repo.read_history_range(&index, 0..index.len(), &CancellationToken::new())
        .unwrap()
        .commits
        .iter()
        .map(|commit| commit.summary.to_string())
        .collect()
}

fn sorted(mut messages: Vec<String>) -> Vec<String> {
    messages.sort();
    messages
}

#[test]
fn hidden_remotes_leave_every_all_branches_reader() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "user.name", "Committer"]);
    run_git(repo, &["config", "user.email", "committer@example.com"]);
    for remote in ["origin", "origin2", "upstream"] {
        run_git(
            repo,
            &["remote", "add", remote, "https://example.com/r.git"],
        );
    }
    let base = commit_as(repo, "base", "base.txt");
    run_git(repo, &["update-ref", "refs/remotes/origin/main", &base]);
    remote_only_commit(repo, "refs/remotes/origin/feature", "origin.txt");
    remote_only_commit(repo, "refs/remotes/origin2/feature", "origin2.txt");
    remote_only_commit(repo, "refs/remotes/upstream/feature", "upstream.txt");
    remote_only_commit(repo, "refs/pull/1/head", "pull.txt");
    let local = commit_as(repo, "local", "local.txt");
    run_git(repo, &["update-ref", "refs/remotes/upstream/main", &local]);

    let options = RepositoryOptions::default()
        .with_history_ref_filter(HistoryRefFilter::excluding(["refs/pull"]));
    let backend = ConfiguredBackend::new(Arc::new(GixBackend), options);
    let opened = backend.open(repo).unwrap();
    let repo = opened.as_ref();
    let cancel = CancellationToken::new();
    let everything = [
        "base.txt",
        "local.txt",
        "origin.txt",
        "origin2.txt",
        "upstream.txt",
    ];

    assert_eq!(sorted(hiding(repo, &[])), everything);
    let without_origin = ["base.txt", "local.txt", "origin2.txt", "upstream.txt"];
    assert_eq!(sorted(hiding(repo, &["origin"])), without_origin);
    assert_eq!(sorted(index_messages(repo, &["origin"])), without_origin);
    let authors = repo
        .history_authors(
            &HistoryWalk::new(HistoryMode::AllBranches, ["origin"]),
            &Default::default(),
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

    let every_remote = ["origin", "origin2", "upstream"];
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
        sorted(hiding(repo, &[])),
        everything,
        "showing the remotes again does not reuse the hidden walk"
    );
    assert!(
        Arc::ptr_eq(&index_of(repo, &[]), &index_of(repo, &["nonexistent"])),
        "hiding a remote with no refs walks the same history"
    );

    let head = repo
        .read_history(
            &HistoryWalk::new(HistoryMode::FullReachable, every_remote),
            None,
            &Default::default(),
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
    assert_eq!(page.commits.len(), 2, "HEAD's own history hides nothing");
}

#[test]
fn hidden_remotes_follow_their_fetch_refspecs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "user.name", "Committer"]);
    run_git(repo, &["config", "user.email", "committer@example.com"]);
    for remote in ["team", "foo"] {
        run_git(
            repo,
            &["remote", "add", remote, "https://example.com/r.git"],
        );
    }
    // `git remote add` refuses a name nested in another; a config edit does not.
    run_git(
        repo,
        &["config", "remote.team/x.url", "https://example.com/r.git"],
    );
    run_git(
        repo,
        &[
            "config",
            "remote.team/x.fetch",
            "+refs/heads/*:refs/remotes/team/x/*",
        ],
    );
    // `foo` writes into another remote's usual namespace.
    run_git(
        repo,
        &[
            "config",
            "remote.foo.fetch",
            "+refs/heads/*:refs/remotes/bar/*",
        ],
    );
    let base = commit_as(repo, "base", "base.txt");
    run_git(repo, &["update-ref", "refs/remotes/team/main", &base]);
    remote_only_commit(repo, "refs/remotes/team/x/feature", "team-x.txt");
    remote_only_commit(repo, "refs/remotes/team/topic", "team.txt");
    remote_only_commit(repo, "refs/remotes/bar/feature", "foo.txt");
    remote_only_commit(repo, "refs/remotes/stale/feature", "stale.txt");
    let opened = GixBackend.open(repo).unwrap();
    let repo = opened.as_ref();

    assert_eq!(
        sorted(hiding(repo, &["team"])),
        ["base.txt", "foo.txt", "stale.txt", "team-x.txt"],
        "`team/x`'s refs also match `team`'s refspec, and `team/x` is shown"
    );
    assert_eq!(
        sorted(hiding(repo, &["team", "team/x"])),
        ["base.txt", "foo.txt", "stale.txt"]
    );
    assert_eq!(
        sorted(hiding(repo, &["foo"])),
        ["base.txt", "stale.txt", "team-x.txt", "team.txt"],
        "`foo`'s refspec writes refs/remotes/bar"
    );
    assert_eq!(
        sorted(hiding(repo, &["bar"])),
        ["base.txt", "foo.txt", "stale.txt", "team-x.txt", "team.txt"],
        "no remote is called bar"
    );
}

/// Hiding a remote thins out the all-branches seed; focusing names refs
/// outright. A focused ref is walked even when its remote is hidden, because
/// the user asked for exactly that ref.
#[test]
fn a_focused_ref_is_walked_even_when_its_remote_is_hidden() {
    use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet};

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "user.name", "Committer"]);
    run_git(repo, &["config", "user.email", "committer@example.com"]);
    run_git(
        repo,
        &["remote", "add", "upstream", "https://example.com/r.git"],
    );
    commit_as(repo, "base", "base.txt");
    remote_only_commit(repo, "refs/remotes/upstream/feature", "upstream.txt");

    let opened = GixBackend.open(repo).unwrap();
    let read = |focus: &HistoryFocusSet| {
        let result = opened
            .read_history(
                &HistoryWalk::new(HistoryMode::AllBranches, ["upstream"]),
                None,
                focus,
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
        sorted(
            page.commits
                .iter()
                .map(|commit| commit.summary.to_string())
                .collect(),
        )
    };

    assert_eq!(read(&HistoryFocusSet::default()), vec!["base.txt"]);
    assert_eq!(
        read(&HistoryFocusSet::from_iter([HistoryFocus::remote_branch(
            "upstream", "feature"
        )])),
        vec!["base.txt", "upstream.txt"]
    );
}
