use super::*;
use gitcomet_core::domain::{HistorySolo, HistorySoloSet};
use gitcomet_core::services::{
    CancellationToken, GitRepository, HistoryReadRequest, HistoryReadResult,
};

/// `master` carries `base` then `on-master`; `feature`, branched off `base`,
/// carries `on-feature`. No branch reaches the other's tip commit, which is
/// what makes a solo observable.
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

fn summaries(repo: &dyn GitRepository, mode: HistoryMode, solo: &HistorySoloSet) -> Vec<String> {
    let result = repo
        .read_history(
            mode,
            None,
            solo,
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
fn soloing_a_local_branch_walks_only_that_branch() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();

    // All branches sees both tips; the checked-out branch sees only its own.
    let everything = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistorySoloSet::default(),
    );
    assert!(everything.contains(&"on-master".to_string()));
    assert!(everything.contains(&"on-feature".to_string()));

    let soloed = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistorySoloSet::from_iter([HistorySolo::local_branch("feature")]),
    );
    assert_eq!(soloed, vec!["on-feature".to_string(), "base".to_string()]);
}

#[test]
fn solo_survives_a_history_mode_that_would_otherwise_seed_head() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();

    // HEAD is `master`, so an unsoloed first-parent walk never reaches
    // `on-feature`. Solo reseeds the walk without changing how it runs.
    assert!(
        !summaries(
            repo.as_ref(),
            HistoryMode::FirstParent,
            &HistorySoloSet::default()
        )
        .contains(&"on-feature".to_string())
    );
    assert_eq!(
        summaries(
            repo.as_ref(),
            HistoryMode::FirstParent,
            &HistorySoloSet::from_iter([HistorySolo::local_branch("feature")]),
        ),
        vec!["on-feature".to_string(), "base".to_string()],
    );
}

#[test]
fn soloing_a_remote_branch_and_a_whole_remote_seed_the_matching_refs() {
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
        &HistorySoloSet::from_iter([HistorySolo::remote_branch("origin", "feature")]),
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
        &HistorySoloSet::from_iter([HistorySolo::remote("origin")]),
    );
    assert!(whole_remote.contains(&"on-master".to_string()));
    assert!(whole_remote.contains(&"on-feature".to_string()));
    assert!(!whole_remote.contains(&"local-only".to_string()));
}

#[test]
fn a_solo_on_a_missing_ref_walks_nothing() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();

    // A branch deleted after the solo was set must not fail the load: the
    // state layer clears the solo once the branch list confirms it is gone.
    assert!(
        summaries(
            repo.as_ref(),
            HistoryMode::AllBranches,
            &HistorySoloSet::from_iter([HistorySolo::local_branch("does-not-exist")]),
        )
        .is_empty()
    );
}

#[test]
fn solo_targets_that_share_a_tip_still_get_their_own_snapshot() {
    let dir = forked_repo();
    run_git(dir.path(), &["branch", "alias", "feature"]);
    let repo = GixBackend.open(dir.path()).unwrap();

    let snapshot_for = |solo: &HistorySoloSet| {
        let result = repo
            .read_history(
                HistoryMode::AllBranches,
                None,
                solo,
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
        snapshot_for(&HistorySoloSet::from_iter([HistorySolo::local_branch(
            "feature"
        )])),
        snapshot_for(&HistorySoloSet::from_iter([HistorySolo::local_branch(
            "alias"
        )])),
    );
}

/// Soloing two branches seeds the walk from both, so the page is the union of
/// what they reach -- that is what makes soloing a pair a comparison.
#[test]
fn soloing_two_branches_shows_the_union_of_what_they_reach() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();

    let both = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistorySoloSet::from_iter([
            HistorySolo::local_branch("feature"),
            HistorySolo::local_branch("master"),
        ]),
    );
    assert!(both.contains(&"on-feature".to_string()));
    assert!(both.contains(&"on-master".to_string()));
    assert!(both.contains(&"base".to_string()));

    // And each alone still excludes the other's tip.
    let feature_only = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistorySoloSet::from_iter([HistorySolo::local_branch("feature")]),
    );
    assert!(!feature_only.contains(&"on-master".to_string()));
}

/// A solo set whose refs have all been deleted walks nothing, but one live ref
/// among dead ones still seeds its own history.
#[test]
fn a_partly_missing_solo_set_still_walks_its_live_refs() {
    let dir = forked_repo();
    let repo = GixBackend.open(dir.path()).unwrap();

    let mixed = summaries(
        repo.as_ref(),
        HistoryMode::AllBranches,
        &HistorySoloSet::from_iter([
            HistorySolo::local_branch("feature"),
            HistorySolo::local_branch("does-not-exist"),
        ]),
    );
    assert_eq!(mixed, vec!["on-feature".to_string(), "base".to_string()]);
}
