use super::*;
use gitcomet_core::domain::{CommitId, FileStatus, FileStatusKind};

fn status(path: &str, kind: FileStatusKind) -> FileStatus {
    FileStatus {
        path: PathBuf::from(path),
        kind,
        conflict: None,
    }
}

fn worktree() -> Worktree {
    Worktree {
        path: PathBuf::from("/wt/side"),
        head: Some(CommitId("abc123".into())),
        branch: Some("side".into()),
        detached: false,
    }
}

#[test]
fn a_summary_sums_staged_and_unstaged_into_the_three_buckets() {
    let repo_status = RepoStatus {
        staged: std::sync::Arc::new(vec![status("gone.txt", FileStatusKind::Deleted)]),
        unstaged: std::sync::Arc::new(vec![
            status("edited.txt", FileStatusKind::Modified),
            status("new.txt", FileStatusKind::Untracked),
        ]),
    };

    let summary = worktree_dirty_summary(worktree(), repo_status, true, Default::default());
    assert_eq!(
        (summary.added, summary.modified, summary.deleted),
        (1, 1, 1)
    );
    assert!(summary.is_dirty());
    // The rows list these files, so the scan has to hand them over rather
    // than reducing them to counts and dropping them.
    assert_eq!(summary.unstaged.len(), 2);
    assert_eq!(summary.staged.len(), 1);
    assert_eq!(summary.staged[0].path, PathBuf::from("gone.txt"));
}

/// The row is anchored by HEAD and labelled by branch, so the scan has to
/// carry both through; the repo's own worktree list is loaded lazily and
/// cannot be relied on.
#[test]
fn a_summary_carries_the_worktrees_identity() {
    let summary =
        worktree_dirty_summary(worktree(), RepoStatus::default(), true, Default::default());
    assert_eq!(summary.path, PathBuf::from("/wt/side"));
    assert_eq!(summary.head.as_ref().map(|id| id.as_ref()), Some("abc123"));
    assert_eq!(summary.branch.as_deref(), Some("side"));
    assert!(!summary.detached);
    assert!(!summary.is_dirty(), "a clean tree must not be reported");
}

/// The scan's own workdir arrives canonicalized while git reports the path it
/// recorded, so the two spell the same directory differently whenever the repo
/// root is reached through a symlink. Missing that match scans this tab's own
/// worktree and renders its changes twice.
#[cfg(unix)]
#[test]
fn a_symlinked_repo_root_is_still_recognised_as_our_own_worktree() {
    let dir = tempfile::tempdir().expect("temp dir");
    let real = dir.path().join("real");
    std::fs::create_dir(&real).expect("create real worktree dir");
    let linked = dir.path().join("linked");
    std::os::unix::fs::symlink(&real, &linked).expect("symlink");

    let own_workdir = canonicalize_or_original(linked.clone());
    assert_ne!(
        own_workdir, linked,
        "fixture must actually produce two spellings of one directory"
    );

    assert!(
        is_own_worktree(&linked, &own_workdir),
        "the path git reports must still match our canonicalized workdir"
    );
    assert!(
        is_own_worktree(&real, &own_workdir),
        "and so must the resolved one"
    );
    assert!(
        !is_own_worktree(&dir.path().join("other"), &own_workdir),
        "a genuinely different worktree is still scanned"
    );
}
