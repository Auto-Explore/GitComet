use super::*;
use gitcomet_core::domain::LogPage;
use gitcomet_core::services::{
    CancellationToken, GitRepository, HistoryReadRequest, HistoryReadResult, HistorySnapshot,
};

fn fixture(count: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    fast_import_linear_history(dir.path(), count);
    dir
}

fn read(
    repo: &dyn GitRepository,
    mode: HistoryMode,
    author: Option<&str>,
    request: HistoryReadRequest,
) -> HistoryReadResult {
    repo.read_history(
        mode,
        author,
        &request,
        &CancellationToken::new(),
        &mut |_| {},
    )
    .unwrap()
}

fn page(result: HistoryReadResult) -> (Arc<LogPage>, Option<HistorySnapshot>) {
    match result {
        HistoryReadResult::Page { page, snapshot } => (page, snapshot),
        other => panic!("expected a page, got {other:?}"),
    }
}

fn first(
    repo: &dyn GitRepository,
    mode: HistoryMode,
    author: Option<&str>,
    limit: usize,
) -> (Arc<LogPage>, Option<HistorySnapshot>) {
    page(read(
        repo,
        mode,
        author,
        HistoryReadRequest::Page {
            limit,
            cursor: None,
            snapshot: None,
        },
    ))
}

fn assert_unchanged(
    repo: &dyn GitRepository,
    mode: HistoryMode,
    previous: Arc<LogPage>,
    snapshot: Option<HistorySnapshot>,
) {
    let _capture = gitcomet_core::git_ops_trace::capture();
    for _ in 0..5 {
        assert_eq!(
            read(
                repo,
                mode,
                None,
                HistoryReadRequest::Refresh {
                    previous: Arc::clone(&previous),
                    snapshot: snapshot.clone(),
                }
            ),
            HistoryReadResult::Unchanged
        );
    }
    assert_eq!(
        gitcomet_core::git_ops_trace::snapshot().log_walk.calls,
        0,
        "refocusing unchanged history must not enter a history walk"
    );
}

#[test]
fn snapshot_reads_and_rebuilt_refreshes_share_cached_pages() {
    let dir = fixture(20);
    let repo = GixBackend.open(dir.path()).unwrap();
    for mode in [HistoryMode::FullReachable, HistoryMode::FirstParent] {
        let (original, snapshot) = first(repo.as_ref(), mode, None, 200);
        let (cached, cached_snapshot) = first(repo.as_ref(), mode, None, 200);
        assert!(Arc::ptr_eq(&original, &cached));
        assert_eq!(snapshot, cached_snapshot);

        // Without a known snapshot the refresh reads a page, but a complete
        // cached result can still be shared without copying its commits.
        let (refreshed, refreshed_snapshot) = page(read(
            repo.as_ref(),
            mode,
            None,
            HistoryReadRequest::Refresh {
                previous: Arc::clone(&original),
                snapshot: None,
            },
        ));
        assert!(Arc::ptr_eq(&original, &refreshed));
        assert_eq!(snapshot, refreshed_snapshot);
    }
}

#[test]
fn snapshot_refresh_keeps_fifty_thousand_commits_without_walking_again() {
    let dir = fixture(50_000);
    let repo = GixBackend.open(dir.path()).unwrap();
    for mode in [HistoryMode::FullReachable, HistoryMode::AllBranches] {
        let (previous, snapshot) = first(repo.as_ref(), mode, None, 50_000);
        assert_eq!(previous.commits.len(), 50_000);
        assert!(previous.next_cursor.is_none());
        assert!(snapshot.is_some());
        assert_unchanged(repo.as_ref(), mode, previous, snapshot);
    }
}

#[test]
fn snapshot_refresh_retains_loaded_commits_after_more_than_a_page_is_added() {
    let dir = fixture(1_001);
    let tip = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    let old_tip = git_stdout(dir.path(), &["rev-parse", "HEAD~401"]);
    let repo = GixBackend.open(dir.path()).unwrap();
    for complete in [false, true] {
        run_git(dir.path(), &["update-ref", "refs/heads/master", &old_tip]);
        let (previous, snapshot) = first(
            repo.as_ref(),
            HistoryMode::AllBranches,
            None,
            if complete { 600 } else { 300 },
        );
        assert_eq!(previous.next_cursor.is_none(), complete);
        run_git(dir.path(), &["update-ref", "refs/heads/master", &tip]);
        if let Some(cursor) = previous.next_cursor.clone() {
            assert_eq!(
                read(
                    repo.as_ref(),
                    HistoryMode::AllBranches,
                    None,
                    HistoryReadRequest::Page {
                        limit: 200,
                        cursor: Some(cursor),
                        snapshot: snapshot.clone(),
                    }
                ),
                HistoryReadResult::Invalidated
            );
        }
        let (updated, token) = page(read(
            repo.as_ref(),
            HistoryMode::AllBranches,
            None,
            HistoryReadRequest::Refresh {
                previous: Arc::clone(&previous),
                snapshot,
            },
        ));
        let ids: std::collections::HashSet<_> = updated.commits.iter().map(|c| &c.id).collect();
        assert_eq!(ids.len(), updated.commits.len());
        assert!(previous.commits.iter().all(|c| ids.contains(&c.id)));
        if complete {
            assert_eq!(updated.commits.len(), 1_001);
            assert!(updated.next_cursor.is_none());
        } else {
            assert!(updated.next_cursor.is_some());
            let (next, _) = page(read(
                repo.as_ref(),
                HistoryMode::AllBranches,
                None,
                HistoryReadRequest::Page {
                    limit: 200,
                    cursor: updated.next_cursor.clone(),
                    snapshot: token.clone(),
                },
            ));
            assert!(next.commits.iter().all(|c| !ids.contains(&c.id)));
        }
        assert_unchanged(repo.as_ref(), HistoryMode::AllBranches, updated, token);
    }
}

#[test]
fn snapshot_refresh_rebuilds_for_deleted_commits_and_shallow_boundaries() {
    let dir = fixture(120);
    let repo = GixBackend.open(dir.path()).unwrap();
    let (previous, snapshot) = first(repo.as_ref(), HistoryMode::FullReachable, None, 200);
    let shorter = git_stdout(dir.path(), &["rev-parse", "HEAD~20"]);
    run_git(dir.path(), &["update-ref", "refs/heads/master", &shorter]);
    let (updated, _) = page(read(
        repo.as_ref(),
        HistoryMode::FullReachable,
        None,
        HistoryReadRequest::Refresh { previous, snapshot },
    ));
    assert_eq!(updated.commits.len(), 100);
    assert!(updated.next_cursor.is_none());

    let boundary = git_stdout(dir.path(), &["rev-parse", "HEAD~40"]);
    std::fs::write(dir.path().join(".git/shallow"), format!("{boundary}\n")).unwrap();
    let (previous, snapshot) = first(repo.as_ref(), HistoryMode::FullReachable, None, 200);
    assert_eq!(previous.commits.len(), 41);
    std::fs::remove_file(dir.path().join(".git/shallow")).unwrap();
    let (updated, _) = page(read(
        repo.as_ref(),
        HistoryMode::FullReachable,
        None,
        HistoryReadRequest::Refresh { previous, snapshot },
    ));
    assert_eq!(updated.commits.len(), 100);
    assert!(updated.next_cursor.is_none());
}

#[test]
fn snapshot_refresh_identity_includes_mode_author_and_non_head_refs() {
    let dir = fixture(80);
    let repo = GixBackend.open(dir.path()).unwrap();
    let (previous, snapshot) = first(repo.as_ref(), HistoryMode::AllBranches, Some("YOU"), 200);
    assert_eq!(
        read(
            repo.as_ref(),
            HistoryMode::AllBranches,
            Some("you"),
            HistoryReadRequest::Refresh {
                previous: Arc::clone(&previous),
                snapshot: snapshot.clone(),
            }
        ),
        HistoryReadResult::Unchanged
    );
    let (filtered, _) = page(read(
        repo.as_ref(),
        HistoryMode::AllBranches,
        Some("absent author"),
        HistoryReadRequest::Refresh {
            previous: Arc::clone(&previous),
            snapshot: snapshot.clone(),
        },
    ));
    assert!(filtered.commits.is_empty());
    assert!(matches!(
        read(
            repo.as_ref(),
            HistoryMode::MergesOnly,
            Some("you"),
            HistoryReadRequest::Refresh {
                previous: Arc::clone(&previous),
                snapshot: snapshot.clone(),
            }
        ),
        HistoryReadResult::Page { .. }
    ));
    // HEAD is unchanged, but adding another traversal tip changes the snapshot.
    run_git(dir.path(), &["update-ref", "refs/custom/older", "HEAD~10"]);
    assert!(matches!(
        read(
            repo.as_ref(),
            HistoryMode::AllBranches,
            Some("you"),
            HistoryReadRequest::Refresh { previous, snapshot }
        ),
        HistoryReadResult::Page { .. }
    ));
}

#[test]
#[ignore = "read-only check against GITCOMET_HISTORY_TEST_REPO"]
fn snapshot_refresh_real_repository() {
    let path = std::env::var_os("GITCOMET_HISTORY_TEST_REPO").expect("repository path");
    let repo = GixBackend.open(Path::new(&path)).unwrap();
    let mode = HistoryMode::AllBranches;
    let (mut loaded, mut snapshot) = first(repo.as_ref(), mode, None, 200);
    while let Some(cursor) = loaded.next_cursor.clone() {
        let (next, token) = page(read(
            repo.as_ref(),
            mode,
            None,
            HistoryReadRequest::Page {
                limit: 200,
                cursor: Some(cursor),
                snapshot: snapshot.clone(),
            },
        ));
        let loaded = Arc::make_mut(&mut loaded);
        loaded.commits.extend(next.commits.iter().cloned());
        loaded.next_cursor = next.next_cursor.clone();
        snapshot = token;
    }
    eprintln!(
        "Verified {} commits through the oldest commit",
        loaded.commits.len()
    );
    assert_unchanged(repo.as_ref(), mode, loaded, snapshot);
}
