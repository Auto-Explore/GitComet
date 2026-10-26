use super::*;
use gitcomet_core::domain::{HeadTracking, Upstream};
use gitcomet_core::services::GitRepository;

fn tracking_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let remote = root.path().join("remote");
    let local = root.path().join("local");
    fs::create_dir(&remote).unwrap();
    init_test_repo(&remote);
    git_success(&remote, &["checkout", "-b", "main"]);
    commit_file(&remote, "file", "first", "first");
    let output = git_command()
        .args(["clone", "-q"])
        .arg(&remote)
        .arg(&local)
        .output()
        .unwrap();
    assert!(output.status.success());
    (root, remote, local)
}

#[test]
fn head_tracking_pairs_current_branch_upstream_and_divergence() {
    let (_root, remote, local) = tracking_fixture();
    for value in ["second", "third", "fourth"] {
        commit_file(&remote, "file", value, value);
    }
    git_success(&local, &["fetch", "origin"]);
    let repo = open_repo(&local);
    assert_eq!(
        repo.head_tracking(&CancellationToken::new()).unwrap(),
        HeadTracking::Tracking {
            branch: "main".into(),
            upstream: Upstream {
                remote: "origin".into(),
                branch: "main".into(),
            },
            divergence: Some(UpstreamDivergence {
                ahead: 0,
                behind: 3,
            }),
        }
    );
    git_success(&local, &["checkout", "-b", "other"]);
    assert_eq!(
        repo.head_tracking(&CancellationToken::new()).unwrap(),
        HeadTracking::Untracked {
            branch: "other".into(),
        }
    );
    git_success(&local, &["checkout", "--detach"]);
    assert_eq!(
        repo.head_tracking(&CancellationToken::new()).unwrap(),
        HeadTracking::Detached
    );
}

#[test]
fn head_tracking_preserves_configured_upstream_when_tracking_target_is_missing() {
    let (_root, _remote, local) = tracking_fixture();
    git_success(&local, &["update-ref", "-d", "refs/remotes/origin/main"]);
    assert_eq!(
        open_repo(&local)
            .head_tracking(&CancellationToken::new())
            .unwrap(),
        HeadTracking::Tracking {
            branch: "main".into(),
            upstream: Upstream {
                remote: "origin".into(),
                branch: "main".into(),
            },
            divergence: None,
        }
    );
}

#[test]
fn head_tracking_checks_cancellation_before_reopening_a_missing_repository() {
    let (_root, _remote, local) = tracking_fixture();
    let repo = open_repo(&local);
    fs::remove_dir_all(&local).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = repo.head_tracking(&cancellation).unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::Cancelled), "{error:?}");
}

#[test]
fn head_tracking_preserves_configured_upstream_for_an_unborn_branch() {
    let (_root, _remote, local) = tracking_fixture();
    git_success(&local, &["checkout", "--orphan", "unborn"]);
    git_success(&local, &["config", "branch.unborn.remote", "origin"]);
    git_success(
        &local,
        &["config", "branch.unborn.merge", "refs/heads/main"],
    );
    assert_eq!(
        open_repo(&local)
            .head_tracking(&CancellationToken::new())
            .unwrap(),
        HeadTracking::Tracking {
            branch: "unborn".into(),
            upstream: Upstream {
                remote: "origin".into(),
                branch: "main".into()
            },
            divergence: None,
        }
    );
}

#[test]
fn head_tracking_reads_each_linked_worktrees_own_head() {
    let (root, remote, local) = tracking_fixture();
    commit_file(&remote, "file", "second", "second");
    git_success(&local, &["fetch", "origin"]);
    git_success(&local, &["branch", "--track", "linked", "origin/main"]);
    let linked = root.path().join("linked");
    git_success(
        &local,
        &["worktree", "add", linked.to_str().unwrap(), "linked"],
    );
    for (path, branch, behind) in [(&local, "main", 1), (&linked, "linked", 0)] {
        assert_eq!(
            open_repo(path)
                .head_tracking(&CancellationToken::new())
                .unwrap(),
            HeadTracking::Tracking {
                branch: branch.into(),
                upstream: Upstream {
                    remote: "origin".into(),
                    branch: "main".into()
                },
                divergence: Some(UpstreamDivergence { ahead: 0, behind }),
            }
        );
    }
}
