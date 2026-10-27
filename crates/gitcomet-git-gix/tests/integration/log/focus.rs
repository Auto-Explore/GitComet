use super::*;
use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet};
use gitcomet_core::services::{
    CancellationToken, GitRepository, HistoryReadRequest, HistoryReadResult,
};

/// `master` carries `base` then `on-master`; `feature`, branched off `base`,
/// carries `on-feature`. No branch reaches the other's tip commit, which is
/// what makes a focus observable.
fn forked_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    run_git(dir.path(), &["config", "user.email", "you@example.com"]);
    run_git(dir.path(), &["config", "user.name", "You"]);
    run_git(dir.path(), &["commit", "--allow-empty", "-qm", "base"]);
    run_git(dir.path(), &["checkout", "-qb", "feature"]);
    run_git(
        dir.path(),
        &["commit", "--allow-empty", "-qm", "on-feature"],
    );
    run_git(dir.path(), &["checkout", "-q", "master"]);
    run_git(dir.path(), &["commit", "--allow-empty", "-qm", "on-master"]);
    dir
}

fn summaries(repo: &dyn GitRepository, mode: HistoryMode, focus: &HistoryFocusSet) -> Vec<String> {
    let result = repo
        .read_history(
            &mode.into(),
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
        panic!("expected a page");
    };
    page.commits
        .iter()
        .map(|commit| commit.summary.to_string())
        .collect()
}

#[test]
fn focusing_a_local_branch_walks_only_that_branch() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();

    // All branches sees both tips; the checked-out branch sees only its own.
    let everything = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistoryFocusSet::default(),
    );
    assert!(everything.contains(&"on-master".to_string()));
    assert!(everything.contains(&"on-feature".to_string()));

    let focused = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistoryFocusSet::from_iter([HistoryFocus::local_branch("feature")]),
    );
    assert_eq!(focused, vec!["on-feature".to_string(), "base".to_string()]);
}

#[test]
fn focus_survives_a_history_mode_that_would_otherwise_seed_head() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();

    // HEAD is `master`, so an unfocused first-parent walk never reaches
    // `on-feature`. Focus reseeds the walk without changing how it runs.
    assert!(
        !summaries(
            repo.as_ref(),
            HistoryMode::FirstParent,
            &HistoryFocusSet::default()
        )
        .contains(&"on-feature".to_string())
    );
    assert_eq!(
        summaries(
            repo.as_ref(),
            HistoryMode::FirstParent,
            &HistoryFocusSet::from_iter([HistoryFocus::local_branch("feature")]),
        ),
        vec!["on-feature".to_string(), "base".to_string()],
    );
}

#[test]
fn focusing_a_remote_branch_and_a_whole_remote_seed_the_matching_refs() {
    let origin = forked_repo();
    let clone_dir = tempfile::tempdir().unwrap();
    let workdir = clone_dir.path().join("clone");
    run_git(
        clone_dir.path(),
        &[
            "clone",
            "-q",
            &git_remote_url(origin.path()),
            workdir.to_str().unwrap(),
        ],
    );
    // A commit only the clone has, so "everything" and "just origin" differ.
    run_git(&workdir, &["config", "user.email", "you@example.com"]);
    run_git(&workdir, &["config", "user.name", "You"]);
    run_git(&workdir, &["commit", "--allow-empty", "-qm", "local-only"]);

    let repo = GixBackend.open(&workdir).unwrap();

    let remote_branch = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistoryFocusSet::from_iter([HistoryFocus::remote_branch("origin", "feature")]),
    );
    assert_eq!(
        remote_branch,
        vec!["on-feature".to_string(), "base".to_string()]
    );

    // A whole remote seeds every branch it tracks -- both tips here -- and
    // nothing the clone has added on top.
    let whole_remote = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistoryFocusSet::from_iter([HistoryFocus::remote("origin")]),
    );
    assert!(whole_remote.contains(&"on-master".to_string()));
    assert!(whole_remote.contains(&"on-feature".to_string()));
    assert!(!whole_remote.contains(&"local-only".to_string()));
}

#[test]
fn a_focus_on_a_missing_ref_walks_nothing() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();

    // A branch deleted after the focus was set must not fail the load: the
    // state layer clears the focus once the branch list confirms it is gone.
    assert!(
        summaries(
            repo.as_ref(),
            HistoryMode::AllBranches,
            &HistoryFocusSet::from_iter([HistoryFocus::local_branch("does-not-exist")]),
        )
        .is_empty()
    );
}

#[test]
fn focus_targets_that_share_a_tip_still_get_their_own_snapshot() {
    let dir = forked_repo();
    run_git(dir.path(), &["branch", "alias", "feature"]);
    let repo = GixBackend.open(dir.path()).unwrap();

    let snapshot_for = |focus: &HistoryFocusSet| {
        let result = repo
            .read_history(
                &HistoryMode::AllBranches.into(),
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
        let HistoryReadResult::Page { snapshot, .. } = result else {
            panic!("expected a page");
        };
        snapshot.expect("gix reports a snapshot")
    };

    // `alias` and `feature` point at the same commit, so only naming the target
    // keeps a refresh under one from being served the other's snapshot.
    assert_ne!(
        snapshot_for(&HistoryFocusSet::from_iter([HistoryFocus::local_branch(
            "feature"
        )])),
        snapshot_for(&HistoryFocusSet::from_iter([HistoryFocus::local_branch(
            "alias"
        )])),
    );
}

/// Focusing two branches seeds the walk from both, so the page is the union of
/// what they reach -- that is what makes focusing a pair a comparison.
#[test]
fn focusing_two_branches_shows_the_union_of_what_they_reach() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();

    let both = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistoryFocusSet::from_iter([
            HistoryFocus::local_branch("feature"),
            HistoryFocus::local_branch("master"),
        ]),
    );
    assert!(both.contains(&"on-feature".to_string()));
    assert!(both.contains(&"on-master".to_string()));
    assert!(both.contains(&"base".to_string()));

    // And each alone still excludes the other's tip.
    let feature_only = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistoryFocusSet::from_iter([HistoryFocus::local_branch("feature")]),
    );
    assert!(!feature_only.contains(&"on-master".to_string()));
}

/// A focus set whose refs have all been deleted walks nothing, but one live ref
/// among dead ones still seeds its own history.
#[test]
fn a_partly_missing_focus_set_still_walks_its_live_refs() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();

    let mixed = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistoryFocusSet::from_iter([
            HistoryFocus::local_branch("feature"),
            HistoryFocus::local_branch("does-not-exist"),
        ]),
    );
    assert_eq!(mixed, vec!["on-feature".to_string(), "base".to_string()]);
}

#[test]
fn focus_history_indexes_and_authors_follow_the_selected_refs() {
    let dir = forked_repo();
    run_git(dir.path(), &["checkout", "-q", "feature"]);
    run_git(
        dir.path(),
        &[
            "commit",
            "--amend",
            "--allow-empty",
            "--no-edit",
            "--author",
            "Feature Author <feature@example.com>",
        ],
    );
    run_git(dir.path(), &["checkout", "-q", "master"]);
    run_git(dir.path(), &["branch", "alias", "feature"]);
    let repo = GixBackend.open(dir.path()).unwrap();
    let cancellation = CancellationToken::new();
    let mut snapshots = Vec::new();

    for (branch, expected_summary, expected_authors) in [
        ("feature", "on-feature", vec!["Feature Author", "You"]),
        ("master", "on-master", vec!["You"]),
        ("alias", "on-feature", vec!["Feature Author", "You"]),
    ] {
        let focus = HistoryFocusSet::from_iter([HistoryFocus::local_branch(branch)]);
        let index = repo
            .build_history_index(
                &HistoryMode::FirstParent.into(),
                None,
                &focus,
                &cancellation,
                &mut |_| {},
            )
            .unwrap()
            .unwrap();
        assert_eq!(index.len(), 2);
        let range = repo
            .read_history_range(&index, 0..2, &cancellation)
            .unwrap();
        assert_eq!(range.commits[0].summary.as_ref(), expected_summary);
        let mut authors = repo
            .history_authors(&HistoryMode::FirstParent.into(), &focus, &cancellation)
            .unwrap()
            .iter()
            .map(|author| author.to_string())
            .collect::<Vec<_>>();
        authors.sort();
        assert_eq!(authors, expected_authors);
        snapshots.push(index.snapshot.clone());
    }
    assert_ne!(snapshots[0], snapshots[1]);
    assert_ne!(
        snapshots[0], snapshots[2],
        "aliases remain distinct queries"
    );
}

fn first_page(
    repo: &dyn GitRepository,
    mode: HistoryMode,
    focus: &HistoryFocusSet,
    limit: usize,
    cursor: Option<gitcomet_core::domain::LogCursor>,
    snapshot: Option<gitcomet_core::services::HistorySnapshot>,
) -> (
    Arc<gitcomet_core::domain::LogPage>,
    Option<gitcomet_core::services::HistorySnapshot>,
) {
    let result = repo
        .read_history(
            &mode.into(),
            None,
            focus,
            &HistoryReadRequest::Page {
                limit,
                cursor,
                snapshot,
            },
            &CancellationToken::new(),
            &mut |_| {},
        )
        .unwrap();
    let HistoryReadResult::Page { page, snapshot } = result else {
        panic!("expected a page");
    };
    (page, snapshot)
}

fn commit_summaries(commits: &[gitcomet_core::domain::Commit]) -> Vec<String> {
    commits
        .iter()
        .map(|commit| commit.summary.to_string())
        .collect()
}

/// `master` is `base -> main1 -> main2`, and `old` points at `base`. `base`
/// carries the newest committer date, so only a topological walk -- not commit
/// date order -- keeps it below `main1`.
fn linear_repo_with_an_ancestor_branch(dir: &Path) {
    run_git(dir, &["init", "-q", "-b", "master"]);
    run_git(dir, &["config", "user.email", "you@example.com"]);
    run_git(dir, &["config", "user.name", "You"]);
    run_git_at(dir, &["commit", "--allow-empty", "-qm", "base"], 300);
    run_git(dir, &["branch", "old"]);
    run_git_at(dir, &["commit", "--allow-empty", "-qm", "main1"], 100);
    run_git_at(dir, &["commit", "--allow-empty", "-qm", "main2"], 200);
}

/// Focusing a branch and one of its ancestors must not lift the ancestor above
/// the commits between them, in paged reads or in the history index.
#[test]
fn a_first_parent_focus_on_a_branch_and_its_ancestor_keeps_children_first() {
    let dir = tempfile::tempdir().unwrap();
    linear_repo_with_an_ancestor_branch(dir.path());
    let repo = GixBackend.open(dir.path()).unwrap();
    let focus = HistoryFocusSet::from_iter([
        HistoryFocus::local_branch("master"),
        HistoryFocus::local_branch("old"),
    ]);
    let expected = vec!["main2", "main1", "base"];

    assert_eq!(
        summaries(repo.as_ref(), HistoryMode::FirstParent, &focus),
        expected
    );

    let cancellation = CancellationToken::new();
    let index = repo
        .build_history_index(
            &HistoryMode::FirstParent.into(),
            None,
            &focus,
            &cancellation,
            &mut |_| {},
        )
        .unwrap()
        .unwrap();
    let range = repo
        .read_history_range(&index, 0..index.len(), &cancellation)
        .unwrap();
    assert_eq!(commit_summaries(&range.commits), expected);
}

/// A first-parent walk from several tips reports first parents only, so a
/// focused merge shows no edge to a side branch the walk never followed.
#[test]
fn a_multi_tip_first_parent_focus_reports_only_first_parents() {
    let dir = forked_repo();
    run_git(
        dir.path(),
        &["merge", "-q", "--no-ff", "-m", "merge-feature", "feature"],
    );
    run_git(dir.path(), &["branch", "base-tip", "master~2"]);
    let repo = GixBackend.open(dir.path()).unwrap();
    let focus = HistoryFocusSet::from_iter([
        HistoryFocus::local_branch("master"),
        HistoryFocus::local_branch("base-tip"),
    ]);

    let (page, _) = first_page(
        repo.as_ref(),
        HistoryMode::FirstParent,
        &focus,
        100,
        None,
        None,
    );
    assert_eq!(
        commit_summaries(&page.commits),
        vec!["merge-feature", "on-master", "base"]
    );
    let merge = &page.commits[0];
    assert_eq!(merge.parent_ids.len(), 1, "{:?}", merge.parent_ids);
}

/// A shallow boundary stops gix's topo walk from counting parents; the chains
/// fallback still puts every commit after its children.
#[test]
fn a_shallow_multi_tip_first_parent_focus_keeps_children_first() {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin");
    let shallow = dir.path().join("shallow");
    std::fs::create_dir_all(&origin).unwrap();
    linear_repo_with_an_ancestor_branch(&origin);
    run_git(
        dir.path(),
        &[
            "clone",
            "-q",
            "--depth",
            "2",
            git_force_file_transport_url(&origin).as_str(),
            shallow.to_str().unwrap(),
        ],
    );
    assert_eq!(
        git_stdout(&shallow, &["rev-parse", "--is-shallow-repository"]),
        "true"
    );
    // `main1` is the boundary; point a second branch at it.
    run_git(&shallow, &["branch", "boundary", "master~1"]);
    let repo = GixBackend.open(&shallow).unwrap();
    let focus = HistoryFocusSet::from_iter([
        HistoryFocus::remote_branch("origin", "master"),
        HistoryFocus::local_branch("boundary"),
    ]);

    assert_eq!(
        summaries(repo.as_ref(), HistoryMode::FirstParent, &focus),
        vec!["main2", "main1"]
    );
}

/// The sidebar lists a remote's branches through its fetch refspecs, so focus
/// has to find them the same way, wherever the refspecs put them.
#[test]
fn remote_focus_follows_custom_fetch_refspecs() {
    let origin = forked_repo();
    let clone_dir = tempfile::tempdir().unwrap();
    let workdir = clone_dir.path().join("clone");
    run_git(
        clone_dir.path(),
        &[
            "clone",
            "-q",
            &git_remote_url(origin.path()),
            workdir.to_str().unwrap(),
        ],
    );
    run_git(
        &workdir,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/custom/*",
        ],
    );
    for name in ["HEAD", "master", "feature"] {
        run_git(
            &workdir,
            &["update-ref", "-d", &format!("refs/remotes/origin/{name}")],
        );
    }
    run_git(&workdir, &["fetch", "-q", "origin"]);
    let repo = GixBackend.open(&workdir).unwrap();
    assert!(
        repo.list_remote_branches(&CancellationToken::new())
            .unwrap()
            .iter()
            .any(|branch| branch.remote == "origin" && branch.name == "feature"),
        "the sidebar lists origin/feature through the custom refspec"
    );

    assert_eq!(
        summaries(
            repo.as_ref(),
            HistoryMode::AllBranches,
            &HistoryFocusSet::from_iter([HistoryFocus::remote_branch("origin", "feature")]),
        ),
        vec!["on-feature".to_string(), "base".to_string()]
    );
    let whole_remote = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistoryFocusSet::from_iter([HistoryFocus::remote("origin")]),
    );
    assert!(whole_remote.contains(&"on-master".to_string()));
    assert!(whole_remote.contains(&"on-feature".to_string()));
}

/// With several focused tips there is no single chain to resume from, so a
/// cursor whose walk was evicted restarts from every tip and skips forward.
#[test]
fn a_multi_tip_first_parent_cursor_resumes_every_branch_after_eviction() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();
    let focus = HistoryFocusSet::from_iter([
        HistoryFocus::local_branch("master"),
        HistoryFocus::local_branch("feature"),
    ]);
    let everything = summaries(repo.as_ref(), HistoryMode::FirstParent, &focus);
    assert_eq!(everything.len(), 3, "{everything:?}");

    let (page, snapshot) = first_page(
        repo.as_ref(),
        HistoryMode::FirstParent,
        &focus,
        1,
        None,
        None,
    );
    let mut cursor = page.next_cursor.clone().expect("more history");
    assert_eq!(
        cursor.resume_from, None,
        "several tips have no single chain to resume from"
    );
    // As if the parked walk had been evicted.
    cursor.resume_token = None;

    let (rest, _) = first_page(
        repo.as_ref(),
        HistoryMode::FirstParent,
        &focus,
        100,
        Some(cursor),
        snapshot,
    );
    let mut paged = commit_summaries(&page.commits);
    paged.extend(commit_summaries(&rest.commits));
    assert_eq!(paged, everything);
}
