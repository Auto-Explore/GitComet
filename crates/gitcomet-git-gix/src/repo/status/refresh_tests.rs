use super::tests::{git_success, init_test_repo, open_repo, write_file};
use gitcomet_core::domain::{FileStatus, FileStatusKind};
use gitcomet_core::services::CancellationToken;
use std::path::Path;

fn shell_path(path: &Path) -> String {
    format!(
        "'{}'",
        path.to_str()
            .unwrap()
            .replace('\\', "/")
            .replace('\'', "'\\''")
    )
}

#[test]
fn small_clean_filtered_files_keep_refreshed_stats_without_restarting_filters() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    init_test_repo(workdir);
    write_file(workdir, ".gitattributes", "*.txt filter=count\n");
    let counter = workdir.join(".git/filter-count");
    git_success(
        workdir,
        &[
            "config",
            "filter.count.clean",
            &format!("printf x >> {}; cat", shell_path(&counter)),
        ],
    );
    let timestamp = filetime::FileTime::now().unix_seconds() - 30;
    for number in 0..10 {
        let name = format!("{number}.txt");
        write_file(workdir, &name, &"unchanged text\n".repeat(80));
        filetime::set_file_mtime(
            workdir.join(name),
            filetime::FileTime::from_unix_time(timestamp, 0),
        )
        .unwrap();
    }
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-qm", "seed"]);
    let index_path = workdir.join(".git/index");
    let index_before = std::fs::read(&index_path).unwrap();
    let index_mtime = std::fs::metadata(&index_path).unwrap().modified().unwrap();
    std::fs::write(&counter, []).unwrap();
    for number in 0..10 {
        filetime::set_file_mtime(
            workdir.join(format!("{number}.txt")),
            filetime::FileTime::from_unix_time(timestamp + 1, 0),
        )
        .unwrap();
    }

    let repo = open_repo(workdir);
    let cancellation = CancellationToken::new();
    assert!(repo.status_impl().unwrap().unstaged.is_empty());
    let count = std::fs::read(&counter).unwrap();
    assert_eq!(count.len(), 10, "cold scan verifies each touched file");
    for _ in 0..2 {
        assert!(repo.status_impl().unwrap().unstaged.is_empty());
        assert!(
            repo.worktree_status_cancellable_impl(&cancellation)
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(std::fs::read(&counter).unwrap(), count);
    assert_eq!(std::fs::read(&index_path).unwrap(), index_before);
    assert_eq!(
        std::fs::metadata(&index_path).unwrap().modified().unwrap(),
        index_mtime,
        "retained status stats must not write the index"
    );

    write_file(workdir, "0.txt", &"different text\n".repeat(80));
    let status = repo.status_impl().unwrap();
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, Path::new("0.txt"));
    assert!(std::fs::read(&counter).unwrap().len() > count.len());
    git_success(workdir, &["add", "0.txt"]);
    let status = repo.status_impl().unwrap();
    assert!(status.unstaged.is_empty());
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, Path::new("0.txt"));
}

#[test]
fn one_small_plain_touch_does_not_clone_a_large_index() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    init_test_repo(workdir);
    git_success(workdir, &["config", "core.trustctime", "false"]);
    let timestamp = filetime::FileTime::now().unix_seconds() - 30;
    for number in 0..=super::STAT_REFRESH_SMALL_INDEX_ENTRIES {
        let name = format!("{number:03}.txt");
        write_file(workdir, &name, "unchanged\n");
        filetime::set_file_mtime(
            workdir.join(name),
            filetime::FileTime::from_unix_time(timestamp, 0),
        )
        .unwrap();
    }
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-qm", "seed"]);
    filetime::set_file_mtime(
        workdir.join("000.txt"),
        filetime::FileTime::from_unix_time(timestamp + 1, 0),
    )
    .unwrap();
    let repo = open_repo(workdir);
    assert!(repo.status_impl().unwrap().unstaged.is_empty());
    assert!(repo.stat_refreshed_index.lock().unwrap().is_none());
}

#[test]
fn filesystem_cache_configuration_preserves_status_and_is_fresh_per_scan() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    init_test_repo(workdir);
    write_file(workdir, "file.txt", "base\n");
    write_file(workdir, "nested/deleted.txt", "base\n");
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-qm", "seed"]);
    let repo = open_repo(workdir);
    for enabled in [false, true] {
        git_success(
            workdir,
            &[
                "config",
                "core.fscache",
                if enabled { "true" } else { "false" },
            ],
        );
        assert_eq!(
            super::filesystem_cache_enabled(&repo.status_repo()),
            enabled
        );
        assert!(repo.status_impl().unwrap().unstaged.is_empty());
    }
    write_file(workdir, "file.txt", "edit\n");
    std::fs::remove_file(workdir.join("nested/deleted.txt")).unwrap();
    write_file(workdir, "new/file.txt", "new\n");
    let expected = [
        ("file.txt", FileStatusKind::Modified),
        ("nested/deleted.txt", FileStatusKind::Deleted),
        ("new/file.txt", FileStatusKind::Untracked),
    ]
    .map(|(path, kind)| FileStatus {
        path: path.into(),
        kind,
        conflict: None,
    });
    for enabled in [false, true] {
        git_success(
            workdir,
            &[
                "config",
                "core.fscache",
                if enabled { "true" } else { "false" },
            ],
        );
        assert_eq!(repo.status_impl().unwrap().unstaged.as_ref(), &expected);
    }
    write_file(workdir, "file.txt", "base\n");
    write_file(workdir, "nested/deleted.txt", "base\n");
    std::fs::remove_file(workdir.join("new/file.txt")).unwrap();
    assert!(repo.status_impl().unwrap().unstaged.is_empty());
    // The application uses gix's lenient opens: invalid values fall back just
    // like absent ones, to the upstream Windows default.
    git_success(workdir, &["config", "core.fscache", "invalid"]);
    assert_eq!(
        super::filesystem_cache_enabled(&repo.status_repo()),
        cfg!(windows)
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "existing Windows taskkill runner loses MSYS fork/exec parentage"
)]
fn cancellation_interrupts_the_gitlink_subprocess_in_every_status_lane() {
    cancels_gitlink_subprocess(false, false);
}

#[test]
fn pre_cancelled_gitlink_status_never_starts_the_subprocess() {
    cancels_gitlink_subprocess(false, true);
}

#[cfg(windows)]
#[test]
#[ignore = "existing Windows taskkill runner cannot reach orphaned MSYS exec descendants"]
fn cancellation_interrupts_a_gitlink_hook_with_an_orphaned_exec_descendant() {
    cancels_gitlink_subprocess(true, false);
}

fn cancels_gitlink_subprocess(orphan_with_exec: bool, pre_cancelled: bool) {
    use gitcomet_core::error::ErrorKind;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    init_test_repo(workdir);
    write_file(workdir, "file.txt", "base\n");
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-qm", "seed"]);
    let head = crate::refs::head_oid(&gix::open(workdir).unwrap())
        .unwrap()
        .unwrap();
    git_success(
        workdir,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &head.to_string(),
            "module",
        ],
    );
    let marker = workdir.join(".git/fsmonitor-started");
    let hook = workdir.join(".git/slow-fsmonitor.sh");
    let sleep = if orphan_with_exec {
        "exec sleep 15"
    } else {
        // Avoid intentionally replacing the shell. Windows MSYS can still
        // lose native parentage during fork; the ignored tests retain both
        // forms of that existing process-runner limitation.
        "sleep 15\nprintf finished"
    };
    std::fs::write(
        &hook,
        format!("printf started > {}\n{sleep}\n", shell_path(&marker)),
    )
    .unwrap();
    // Git runs command-valued hooks via an outer `sh -c` too. Prevent its
    // final-command exec optimization in the ordinary hook case.
    let outer_shell_tail = if orphan_with_exec { "" } else { "; :" };
    git_success(
        workdir,
        &[
            "config",
            "core.fsmonitor",
            &format!("sh {}{outer_shell_tail}", shell_path(&hook)),
        ],
    );

    for lane in ["combined", "worktree", "staged"] {
        let repo = open_repo(workdir);
        let cancellation = CancellationToken::new();
        if pre_cancelled {
            cancellation.cancel();
        }
        std::thread::scope(|scope| {
            let (tx, rx) = mpsc::channel();
            let token = cancellation.clone();
            let worker = scope.spawn(move || {
                let result = match lane {
                    "combined" => repo.status_cancellable_impl(&token).map(|_| ()),
                    "worktree" => repo.worktree_status_cancellable_impl(&token).map(|_| ()),
                    _ => repo.staged_status_cancellable_impl(&token).map(|_| ()),
                };
                tx.send(result).unwrap();
            });
            if !pre_cancelled {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !marker.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            let started = marker.exists();
            cancellation.cancel();
            // Bounded hook sleep makes even a regression terminate; receive
            // before joining proves cancellation, not natural completion.
            let result = rx.recv_timeout(Duration::from_secs(5));
            worker.join().unwrap();
            assert_eq!(started, !pre_cancelled, "{lane} subprocess start");
            let error = result
                .expect("cancelled Git must stop promptly")
                .unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::Cancelled),
                "{lane}: {error:?}"
            );
        });
        if pre_cancelled {
            assert!(!marker.exists(), "{lane} must not spawn after cancellation");
        } else {
            std::fs::remove_file(&marker).unwrap();
        }
    }
}
