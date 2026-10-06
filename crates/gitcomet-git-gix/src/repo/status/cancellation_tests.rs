use super::cancellation::test_progress::{ScanHook, with_hook};
use super::tests::{git_success, init_test_repo, open_repo, write_file};
use gitcomet_core::error::ErrorKind;
use gitcomet_core::services::CancellationToken;
use std::sync::{Arc, Mutex, atomic::Ordering, mpsc};
use std::time::{Duration, Instant};

fn cancels_clean_walk(cached: bool, through_parent: bool) {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    init_test_repo(workdir);
    git_success(workdir, &["config", "core.trustctime", "false"]);
    let count = super::STAT_REFRESH_MIN_ENTRIES + 16;
    let timestamp = filetime::FileTime::now().unix_seconds() - 30;
    let touch = |seconds| {
        for ix in 0..count {
            filetime::set_file_mtime(
                workdir.join(format!("{ix:03}.txt")),
                filetime::FileTime::from_unix_time(seconds, 0),
            )
            .unwrap();
        }
    };
    for ix in 0..count {
        write_file(workdir, &format!("{ix:03}.txt"), "unchanged\n");
    }
    touch(timestamp);
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-q", "-m", "seed"]);
    let repo = open_repo(workdir);
    touch(timestamp + 1);
    if cached {
        assert!(repo.status_impl().unwrap().unstaged.is_empty());
        assert!(repo.worktree_status_impl().unwrap().is_empty());
        assert!(repo.stat_refreshed_index.lock().unwrap().is_some());
        touch(timestamp + 2);
    }
    let index_before = std::fs::read(workdir.join(".git/index")).unwrap();
    let stats = || {
        repo.stat_refreshed_index
            .lock()
            .unwrap()
            .as_ref()
            .map(|cache| {
                cache
                    .index
                    .entries()
                    .iter()
                    .map(|entry| entry.stat)
                    .collect::<Vec<_>>()
            })
    };
    let cached_stats = stats();
    let (started_tx, started_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let hook = Arc::new(ScanHook {
        visited: Arc::default(),
        started: started_tx,
        resume: Mutex::new(resume_rx),
    });
    let parent = CancellationToken::new();
    let cancellation = CancellationToken::new().with_parent(parent.clone());
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            with_hook(Arc::clone(&hook), || {
                repo.status_cancellable_impl(&cancellation)
            })
        });
        let interrupt = started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("gix entered the clean-file walk");
        if through_parent {
            parent.cancel();
        } else {
            cancellation.cancel();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while !interrupt.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        // Release before asserting so a failed test never strands the walker.
        resume_tx.send(()).unwrap();
        assert!(
            interrupt.load(Ordering::Acquire),
            "cancellation must reach gix's interrupt flag"
        );
        let error = worker
            .join()
            .unwrap()
            .expect_err("interrupted status cannot publish a clean result");
        assert!(matches!(error.kind(), ErrorKind::Cancelled), "{error:?}");
    });
    assert!(
        hook.visited.load(Ordering::Relaxed) < count,
        "gix must stop walking even when no changed items are emitted"
    );
    assert_eq!(
        stats(),
        cached_stats,
        "interrupted walks retain the last complete cache without partial stats"
    );
    assert_eq!(
        repo.tree_index_cache.lock().unwrap().is_some(),
        cached,
        "cold cancellation must not cache a completed staged result"
    );
    assert_eq!(
        std::fs::read(workdir.join(".git/index")).unwrap(),
        index_before,
        "status never writes the index"
    );
    write_file(workdir, "000.txt", "real edit after cancellation\n");
    let status = repo.status_impl().expect("a fresh scan still works");
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, std::path::PathBuf::from("000.txt"));
}

#[test]
fn cold_clean_status_walk_cancels_without_publishing_or_walking_every_file() {
    cancels_clean_walk(false, false);
}
#[test]
fn cached_clean_status_walk_cancels_without_losing_completed_stats() {
    cancels_clean_walk(true, false);
}
#[test]
fn parent_cancellation_interrupts_cold_and_cached_clean_status_walks() {
    cancels_clean_walk(false, true);
    cancels_clean_walk(true, true);
}
