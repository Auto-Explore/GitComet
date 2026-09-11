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

fn assert_index_matches(repo: &dyn GitRepository, mode: HistoryMode, author: Option<&str>) {
    let cancellation = CancellationToken::new();
    let index = repo
        .build_history_index(mode, author, &cancellation, &mut |_| {})
        .unwrap()
        .unwrap();
    let (page, snapshot) = first(repo, mode, author, 10_000);
    assert_eq!(Some(&index.snapshot), snapshot.as_ref());
    assert_eq!(index.len(), page.commits.len(), "{mode:?} {author:?}");
    // Exercise direct reads in reverse block order, including a tail shorter
    // than a block. No parked traversal cursor participates in these reads.
    for start in (0..index.len()).step_by(256).rev() {
        let end = (start + 256).min(index.len());
        let range = repo
            .read_history_range(&index, start..end, &cancellation)
            .unwrap();
        assert_eq!(
            range.commits,
            page.commits[start..end],
            "{mode:?} {author:?} at {start}"
        );
    }
    assert!(
        repo.read_history_range(&index, index.len()..index.len() + 1, &cancellation)
            .is_err()
    );
}

#[test]
fn indexed_history_matches_queries_and_retains_immutable_ranges() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    fast_import_linear_history_with_authors(dir.path(), 1100, |ix| {
        if ix % 3 == 0 {
            "Alice <alice@example.com>"
        } else {
            "Bob <bob@example.com>"
        }
    });
    run_git(dir.path(), &["checkout", "-q", "-b", "feature", "HEAD~40"]);
    run_git(
        dir.path(),
        &[
            "-c",
            "user.name=Alice",
            "-c",
            "user.email=alice@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "feature",
        ],
    );
    run_git(dir.path(), &["checkout", "-q", "master"]);
    run_git(
        dir.path(),
        &[
            "-c",
            "user.name=Bob",
            "-c",
            "user.email=bob@example.com",
            "merge",
            "--no-ff",
            "-qm",
            "merge feature",
            "feature",
        ],
    );
    run_git(dir.path(), &["checkout", "-q", "--", "."]);
    std::fs::write(dir.path().join("file.txt"), "stash changes").unwrap();
    run_git(
        dir.path(),
        &[
            "-c",
            "user.name=Bob",
            "-c",
            "user.email=bob@example.com",
            "stash",
            "push",
            "-qm",
            "stashed",
        ],
    );
    let repo = GixBackend.open(dir.path()).unwrap();
    for mode in [
        HistoryMode::FullReachable,
        HistoryMode::FirstParent,
        HistoryMode::NoMerges,
        HistoryMode::MergesOnly,
        HistoryMode::AllBranches,
    ] {
        for author in [None, Some("aLiCe"), Some("no such author")] {
            assert_index_matches(repo.as_ref(), mode, author);
        }
    }
    let cancellation = CancellationToken::new();
    let index = repo
        .build_history_index(HistoryMode::AllBranches, None, &cancellation, &mut |_| {})
        .unwrap()
        .unwrap();
    let before = repo
        .read_history_range(&index, 700..710, &cancellation)
        .unwrap();
    run_git(
        dir.path(),
        &[
            "-c",
            "user.name=Bob",
            "-c",
            "user.email=bob@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "new head",
        ],
    );
    assert_eq!(
        before,
        repo.read_history_range(&index, 700..710, &cancellation)
            .unwrap()
    );
    cancellation.cancel();
    assert!(matches!(
        repo.read_history_range(&index, 0..1, &cancellation)
            .unwrap_err()
            .kind(),
        ErrorKind::Cancelled
    ));
}

#[test]
fn indexed_history_matches_empty_and_shallow_repositories() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    let repo = GixBackend.open(dir.path()).unwrap();
    assert_index_matches(repo.as_ref(), HistoryMode::FullReachable, None);
    fast_import_linear_history(dir.path(), 40);
    let clone = tempfile::tempdir().unwrap();
    run_git(
        clone.path(),
        &[
            "clone",
            "-q",
            "--depth=10",
            &git_force_file_transport_url(dir.path()),
            ".",
        ],
    );
    let repo = GixBackend.open(clone.path()).unwrap();
    for mode in [
        HistoryMode::FullReachable,
        HistoryMode::FirstParent,
        HistoryMode::AllBranches,
    ] {
        assert_index_matches(repo.as_ref(), mode, None);
    }
}

#[test]
#[ignore = "set GITCOMET_HISTORY_BENCH_REPO to benchmark a real repository"]
fn indexed_history_large_repository_benchmark() {
    use std::time::Instant;
    let path = std::env::var("GITCOMET_HISTORY_BENCH_REPO").expect("GITCOMET_HISTORY_BENCH_REPO");
    let repo = GixBackend.open(Path::new(&path)).unwrap();
    let cancellation = CancellationToken::new();
    let started = Instant::now();
    let index = repo
        .build_history_index(HistoryMode::AllBranches, None, &cancellation, &mut |_| {})
        .unwrap()
        .unwrap();
    eprintln!(
        "index commits={} seconds={:.3} estimated_mib={:.1}",
        index.len(),
        started.elapsed().as_secs_f64(),
        index.estimated_bytes() as f64 / 1048576.0
    );
    let _capture = gitcomet_core::git_ops_trace::capture();
    let mut samples = Vec::new();
    for round in 0..4 {
        for fraction in [0, 9, 1, 7, 2, 8, 4, 6, 3, 5] {
            let start = index.len().saturating_sub(256) * fraction / 10;
            let started = Instant::now();
            let range = repo
                .read_history_range(&index, start..(start + 256).min(index.len()), &cancellation)
                .unwrap();
            assert_eq!(
                range.commits.first().map(|commit| &commit.id),
                index.commit_id(start).as_ref()
            );
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        if round == 0 {
            eprintln!("first ten block reads ms={samples:?}");
        }
    }
    for (name, values) in [("first_touch", &samples[..10]), ("warm", &samples[10..])] {
        let mut values = values.to_vec();
        values.sort_by(f64::total_cmp);
        eprintln!(
            "{name} range p50_ms={:.3} p95_ms={:.3} p99_ms={:.3}",
            values[values.len() / 2],
            values[values.len() * 95 / 100],
            values[values.len() * 99 / 100]
        );
    }
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "256-commit range reads p50_ms={:.2} p95_ms={:.2}",
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100]
    );
    assert_eq!(
        gitcomet_core::git_ops_trace::snapshot().log_walk.calls,
        0,
        "direct range reads must not walk history"
    );
}

#[test]
fn indexed_history_reads_headers_only_when_needed_and_decodes_ranges_once() {
    use gitcomet_core::history_perf::{Work, capture, count};
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    fast_import_linear_history_with_authors(dir.path(), 1100, |ix| {
        if ix % 3 == 0 {
            "Alice <alice@example.com>"
        } else {
            "Bob <bob@example.com>"
        }
    });
    for graph in [false, true] {
        if graph {
            run_git(dir.path(), &["commit-graph", "write", "--reachable"]);
        }
        let repo = GixBackend.open(dir.path()).unwrap();
        for mode in [
            HistoryMode::FullReachable,
            HistoryMode::FirstParent,
            HistoryMode::NoMerges,
            HistoryMode::MergesOnly,
            HistoryMode::AllBranches,
        ] {
            for author in [None, Some("Alice")] {
                let _capture = capture();
                let cancel = CancellationToken::new();
                let index = repo
                    .build_history_index(mode, author, &cancel, &mut |_| {})
                    .unwrap()
                    .unwrap();
                if author.is_none() {
                    assert_eq!(
                        count(Work::IndexObjectRead),
                        0,
                        "graph={graph} mode={mode:?}"
                    );
                }
                let end = index.len().min(256);
                let range = repo.read_history_range(&index, 0..end, &cancel).unwrap();
                assert_eq!(count(Work::RangeObjectRead), range.commits.len() as u64);
                let (page, _) = first(repo.as_ref(), mode, author, end.max(1));
                assert_eq!(range.commits, page.commits[..end]);
            }
        }
    }
}

#[test]
fn indexed_history_stash_topology_rejects_misleading_merge_messages() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    run_git(dir.path(), &["config", "user.name", "Alice"]);
    run_git(dir.path(), &["config", "user.email", "alice@example.com"]);
    fast_import_linear_history(dir.path(), 10);
    run_git(dir.path(), &["checkout", "-q", "-b", "side", "HEAD~5"]);
    run_git(dir.path(), &["commit", "--allow-empty", "-qm", "side"]);
    run_git(dir.path(), &["checkout", "-q", "master"]);
    run_git(
        dir.path(),
        &[
            "merge",
            "--no-ff",
            "-qm",
            "WIP on master: misleading merge",
            "side",
        ],
    );
    let merge = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    std::fs::write(dir.path().join("file.txt"), "changed tracked content").unwrap();
    std::fs::write(dir.path().join("untracked.txt"), "untracked content").unwrap();
    run_git(dir.path(), &["stash", "push", "-u", "-qm", "real stash"]);
    let stash = git_stdout(dir.path(), &["rev-parse", "refs/stash"]);
    for graph in [false, true] {
        if graph {
            run_git(dir.path(), &["commit-graph", "write", "--reachable"]);
        }
        let repo = GixBackend.open(dir.path()).unwrap();
        let index = repo
            .build_history_index(
                HistoryMode::AllBranches,
                None,
                &CancellationToken::new(),
                &mut |_| {},
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            index.is_probable_stash(index.position(merge.trim()).unwrap()),
            !graph
        );
        assert!(index.is_probable_stash(index.position(stash.trim()).unwrap()));
    }
}
