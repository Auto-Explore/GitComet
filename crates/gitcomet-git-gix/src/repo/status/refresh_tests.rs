use crate::test_support::{git_success, init_test_repo, open_repo, write_file};
use gitcomet_core::domain::{FileStatus, FileStatusKind};
use gitcomet_core::services::CancellationToken;
use std::path::Path;

const CHILD: &str = "modules/child [literal] é";

fn empty_gitlink_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    write_file(root, "file.txt", "base\n");
    git_success(root, &["add", "."]);
    git_success(root, &["commit", "-qm", "seed"]);
    let oid = crate::refs::head_oid(&gix::open(root).unwrap())
        .unwrap()
        .unwrap()
        .to_string();
    write_file(
        root,
        ".gitmodules",
        &format!("[submodule \"child\"]\n path = {CHILD}\n url = ../child\n"),
    );
    git_success(root, &["add", ".gitmodules"]);
    git_success(
        root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &oid,
            CHILD,
        ],
    );
    git_success(root, &["commit", "-qm", "gitlink"]);
    std::fs::create_dir_all(root.join(CHILD)).unwrap();
    dir
}

fn selected_gitlinks(root: &Path, staged: &[FileStatus]) -> Option<Vec<std::path::PathBuf>> {
    super::gitlink_status_paths(&gix::open(root).unwrap(), staged)
}

// Keep this independent Git oracle outside the per-file production-command audit.
#[cfg(test)]
fn gitlink_status_from_git(root: &Path) -> (Vec<FileStatus>, Vec<FileStatus>) {
    let output = crate::util::git_workdir_cmd_for(root)
        .args([
            "--literal-pathspecs",
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "-z",
            "--ignore-submodules=none",
            "--",
            CHILD,
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let (mut staged, mut unstaged) = (Vec::new(), Vec::new());
    for record in output.stdout.split(|byte| *byte == 0) {
        if record.starts_with(b"1 ") {
            super::apply_porcelain_v2_gitlink_status_record(record, &mut staged, &mut unstaged)
                .unwrap();
        }
    }
    (staged, unstaged)
}

fn assert_gitlink_lanes_match_git(
    root: &Path,
    repo: &super::super::GixRepo,
    needs_commands: (bool, bool),
) -> (Vec<FileStatus>, Vec<FileStatus>) {
    let expected = gitlink_status_from_git(root);
    let token = CancellationToken::new();
    let (combined, commands) = crate::command_trace::capture(|| repo.status_impl().unwrap());
    assert_eq!(*combined.staged, expected.0);
    assert_eq!(*combined.unstaged, expected.1);
    assert_eq!(
        commands.len(),
        usize::from(needs_commands.0),
        "{commands:?}"
    );
    let (worktree, commands) =
        crate::command_trace::capture(|| repo.worktree_status_cancellable_impl(&token).unwrap());
    assert_eq!(worktree, expected.1);
    assert_eq!(
        commands.len(),
        usize::from(needs_commands.1),
        "{commands:?}"
    );
    assert_eq!(
        repo.staged_status_cancellable_impl(&token).unwrap(),
        expected.0
    );
    expected
}

fn assert_ignored_native_submodule_provider_is_equivalent(root: &Path) {
    use gix::status::plumbing::index_as_worktree::traits::SubmoduleStatus as _;
    let repo = gix::open(root).unwrap();
    let index = repo.index_or_empty().unwrap();
    let mut builtin = gix::status::index_worktree::BuiltinSubmoduleStatus::new(
        repo.clone().into_sync(),
        gix::status::Submodule::Given {
            ignore: gix::submodule::config::Ignore::All,
            check_dirty: false,
        },
    )
    .unwrap();
    for entry in index
        .entries()
        .iter()
        .filter(|entry| entry.mode.is_submodule())
    {
        assert!(builtin.status(entry, entry.path(&index)).unwrap().is_none());
        assert!(
            super::NoopSubmoduleStatus
                .status(entry, entry.path(&index))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn empty_gitlinks_skip_git_but_child_initialization_and_dirt_restore_it() {
    let dir = empty_gitlink_fixture();
    let root = dir.path();
    let child = root.join(CHILD);
    let index_before = std::fs::read(root.join(".git/index")).unwrap();
    // Check cold staged requests too, before the combined-status cache exists.
    let repo = open_repo(root);
    let (staged, commands) = crate::command_trace::capture(|| {
        repo.staged_status_cancellable_impl(&CancellationToken::new())
            .unwrap()
    });
    assert!(staged.is_empty());
    assert!(commands.is_empty());
    for _ in 0..2 {
        let (staged, unstaged) = assert_gitlink_lanes_match_git(root, &repo, (false, false));
        assert!(staged.is_empty() && unstaged.is_empty());
    }
    assert_ignored_native_submodule_provider_is_equivalent(root);

    init_test_repo(&child);
    write_file(&child, "child.txt", "base\n");
    git_success(&child, &["add", "."]);
    git_success(&child, &["commit", "-qm", "child"]);
    assert_eq!(
        std::fs::read(root.join(".git/index")).unwrap(),
        index_before
    );
    let (staged, unstaged) = assert_gitlink_lanes_match_git(root, &repo, (true, true));
    assert!(staged.is_empty());
    assert_eq!(unstaged[0].kind, FileStatusKind::Modified);
    assert_ignored_native_submodule_provider_is_equivalent(root);

    git_success(root, &["add", "--", CHILD]);
    let (staged, unstaged) = assert_gitlink_lanes_match_git(root, &repo, (true, true));
    assert_eq!(staged[0].kind, FileStatusKind::Modified);
    assert!(unstaged.is_empty());
    git_success(root, &["commit", "-qm", "child pointer"]);
    let (staged, unstaged) = assert_gitlink_lanes_match_git(root, &repo, (true, true));
    assert!(staged.is_empty() && unstaged.is_empty());
    for name in ["child.txt", "untracked.txt"] {
        write_file(&child, name, "changed\n");
        let (staged, unstaged) = assert_gitlink_lanes_match_git(root, &repo, (true, true));
        assert!(staged.is_empty());
        assert_eq!(unstaged[0].kind, FileStatusKind::Modified);
        assert_ignored_native_submodule_provider_is_equivalent(root);
        if name == "child.txt" {
            git_success(&child, &["checkout", "--", name]);
        }
    }

    std::fs::remove_dir_all(&child).unwrap();
    assert_eq!(selected_gitlinks(root, &[]), Some(vec![CHILD.into()]));
    assert_gitlink_lanes_match_git(root, &repo, (true, true));
    std::fs::create_dir(&child).unwrap();
    let (staged, unstaged) = assert_gitlink_lanes_match_git(root, &repo, (false, false));
    assert!(staged.is_empty() && unstaged.is_empty());
}

#[test]
fn gitlink_selection_keeps_uncertain_layouts_and_staged_changes() {
    let dir = empty_gitlink_fixture();
    let root = dir.path();
    let child = root.join(CHILD);
    let selected = Some(vec![CHILD.into()]);
    assert_eq!(selected_gitlinks(root, &[]), Some(Vec::new()));
    for name in [".git", "ordinary.txt"] {
        write_file(&child, name, "not an initialized repository\n");
        assert_eq!(selected_gitlinks(root, &[]), selected);
        std::fs::remove_file(child.join(name)).unwrap();
    }
    write_file(root, "modules/.git", "gitdir: missing\n");
    assert_eq!(selected_gitlinks(root, &[]), selected);
    std::fs::remove_file(root.join("modules/.git")).unwrap();
    std::fs::remove_dir(&child).unwrap();
    assert_eq!(selected_gitlinks(root, &[]), selected);
    write_file(root, CHILD, "replacement\n");
    assert_eq!(selected_gitlinks(root, &[]), selected);
    std::fs::remove_file(&child).unwrap();
    std::fs::create_dir(&child).unwrap();
    for path in [CHILD, "modules"] {
        let changed = FileStatus {
            path: path.into(),
            kind: FileStatusKind::Modified,
            conflict: None,
        };
        assert_eq!(selected_gitlinks(root, &[changed]), selected);
    }

    let repo = open_repo(root);
    git_success(root, &["rm", "--cached", "--", CHILD]);
    let (staged, _) = assert_gitlink_lanes_match_git(root, &repo, (true, false));
    assert_eq!(staged[0].kind, FileStatusKind::Deleted);
    std::fs::remove_dir(&child).unwrap();
    write_file(root, CHILD, "replacement\n");
    git_success(root, &["add", "--", CHILD]);
    let (staged, _) = assert_gitlink_lanes_match_git(root, &repo, (true, false));
    assert_eq!(staged[0].kind, FileStatusKind::Modified);
}

#[test]
fn gitlink_selection_keeps_sparse_intent_to_add_and_conflicted_entries() {
    use gix::index::entry::{Flags, Stage};
    let dir = empty_gitlink_fixture();
    let root = dir.path();
    let repo = gix::open(root).unwrap();
    let original = repo.index_or_empty().unwrap();
    let position = original
        .entries()
        .iter()
        .position(|entry| entry.mode.is_submodule())
        .unwrap();
    for flags in [
        Flags::SKIP_WORKTREE | Flags::EXTENDED,
        Flags::INTENT_TO_ADD | Flags::EXTENDED,
        Flags::from_stage(Stage::Ours),
    ] {
        let mut index = (*original).clone();
        index.entries_mut()[position].flags = flags;
        index.write(Default::default()).unwrap();
        assert_eq!(selected_gitlinks(root, &[]), Some(vec![CHILD.into()]));
    }
}

#[cfg(unix)]
#[test]
fn gitlink_selection_keeps_symlinked_checkout_and_ancestors() {
    use std::os::unix::fs::symlink;
    let dir = empty_gitlink_fixture();
    let root = dir.path();
    let child = root.join(CHILD);
    let target = root.join("elsewhere");
    std::fs::create_dir(&target).unwrap();
    std::fs::remove_dir(&child).unwrap();
    symlink(&target, &child).unwrap();
    assert_eq!(selected_gitlinks(root, &[]), Some(vec![CHILD.into()]));
    std::fs::remove_file(&child).unwrap();
    std::fs::remove_dir(root.join("modules")).unwrap();
    std::fs::create_dir(target.join(child.file_name().unwrap())).unwrap();
    symlink(&target, root.join("modules")).unwrap();
    assert_eq!(selected_gitlinks(root, &[]), Some(vec![CHILD.into()]));
}

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
