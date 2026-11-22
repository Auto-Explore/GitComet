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
    super::gitlink_selection(&gix::open(root).unwrap(), staged).map(super::GitlinkSelection::asked)
}

// Keep this independent Git oracle outside the per-file production-command audit.
#[cfg(test)]
fn gitlink_status_from_git(root: &Path, paths: &[&str]) -> (Vec<FileStatus>, Vec<FileStatus>) {
    let output = crate::util::git_workdir_cmd_for(root)
        .args([
            "--literal-pathspecs",
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "-z",
            "--ignore-submodules=none",
            "--",
        ])
        .args(paths)
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
    let expected = gitlink_status_from_git(root, &[CHILD]);
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
        crate::test_support::set_file_mtime(
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
        crate::test_support::set_file_mtime(
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
        crate::test_support::set_file_mtime(
            workdir.join(name),
            filetime::FileTime::from_unix_time(timestamp, 0),
        )
        .unwrap();
    }
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-qm", "seed"]);
    crate::test_support::set_file_mtime(
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

#[test]
fn gitlink_paths_are_dealt_into_queries_worth_a_process_each() {
    let paths = |count: usize| -> Vec<std::path::PathBuf> {
        (0..count).map(|path| format!("{path:03}").into()).collect()
    };
    // Too few to share out.
    assert_eq!(super::gitlink_queries(paths(15), 8), [paths(15)]);
    assert_eq!(super::gitlink_queries(paths(40), 1), [paths(40)]);
    // Dealt in turn, so that neighbours (one vendor directory, often the
    // large checkouts) land in different queries.
    let queries = super::gitlink_queries(paths(20), 8);
    assert_eq!(queries.len(), 2);
    assert_eq!(
        queries[0],
        paths(20).into_iter().step_by(2).collect::<Vec<_>>()
    );
    assert_eq!(super::gitlink_queries(paths(152), 8).len(), 8);
    assert_eq!(super::gitlink_queries(paths(152), 32).len(), 8);
    let mut all: Vec<_> = super::gitlink_queries(paths(152), 8).concat();
    all.sort();
    assert_eq!(all, paths(152));
}

/// What the index's cached root tree says, and whether the comparison it
/// spares agrees: a root equal to HEAD's tree must mean nothing is staged.
fn cached_tree_says_nothing_is_staged(root: &Path) -> bool {
    let repo = crate::test_support::open_gix(root);
    let tree = crate::refs::head_tree_id_or_empty(&repo).unwrap();
    let index = repo.index_or_empty().unwrap();
    let mut compared = Vec::new();
    super::compare_tree_with_index(&repo, &tree, &index, "test", |change| {
        super::collect_tree_index_change(change, &mut compared).unwrap();
        Ok(std::ops::ControlFlow::Continue(()))
    })
    .unwrap();
    let says = super::index_is_tree(&index, &tree);
    assert!(!says || compared.is_empty(), "{compared:?}");
    let mut staged = super::collect_staged_status_from_tree_index(&repo, &tree).unwrap();
    staged.sort_by(|a, b| a.path.cmp(&b.path));
    compared.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(staged, compared);
    says
}

#[test]
fn the_cached_root_tree_spares_the_staged_comparison_only_when_nothing_is_staged() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    // Unborn, with an empty index and then a filled one.
    cached_tree_says_nothing_is_staged(root);
    write_file(root, "a/b/one.txt", "one\n");
    write_file(root, "a/two.txt", "two\n");
    write_file(root, "three.txt", "three\n");
    git_success(root, &["add", "."]);
    assert!(!cached_tree_says_nothing_is_staged(root));
    git_success(root, &["commit", "-qm", "seed"]);
    assert!(cached_tree_says_nothing_is_staged(root));

    // A change in the working tree alone leaves the index as HEAD has it.
    write_file(root, "a/b/one.txt", "changed\n");
    assert!(cached_tree_says_nothing_is_staged(root));
    git_success(root, &["add", "a/b/one.txt"]);
    assert!(!cached_tree_says_nothing_is_staged(root));
    git_success(root, &["commit", "-qm", "change"]);
    assert!(cached_tree_says_nothing_is_staged(root));

    write_file(root, "a/new.txt", "new\n");
    git_success(root, &["add", "-N", "a/new.txt"]);
    assert!(!cached_tree_says_nothing_is_staged(root));
    git_success(root, &["reset", "-q"]);
    cached_tree_says_nothing_is_staged(root);
    git_success(root, &["rm", "-q", "--cached", "three.txt"]);
    assert!(!cached_tree_says_nothing_is_staged(root));
    git_success(root, &["reset", "-q", "--hard"]);
    assert!(cached_tree_says_nothing_is_staged(root));

    // Stages of a conflict are not what HEAD has either.
    git_success(root, &["checkout", "-q", "-b", "side"]);
    write_file(root, "a/two.txt", "theirs\n");
    git_success(root, &["commit", "-qam", "theirs"]);
    git_success(root, &["checkout", "-q", "-"]);
    write_file(root, "a/two.txt", "ours\n");
    git_success(root, &["commit", "-qam", "ours"]);
    crate::test_support::git_expect_failure(root, &["merge", "-q", "side"]);
    assert!(!cached_tree_says_nothing_is_staged(root));
    git_success(root, &["merge", "--abort"]);
    cached_tree_says_nothing_is_staged(root);

    // An older HEAD is not what the index was written as.
    git_success(root, &["reset", "-q", "--soft", "HEAD~1"]);
    assert!(!cached_tree_says_nothing_is_staged(root));
}

/// Each side waits for a word from the other, which only arrives when both
/// run at once.
#[test]
fn the_gitlink_query_and_the_walk_run_at_once() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let wait = Duration::from_secs(30);
    let (to_walk, from_ask) = channel();
    let (to_ask, from_walk) = channel();
    let (walked, asked) = super::beside(
        move || {
            to_walk.send("asking").unwrap();
            from_walk.recv_timeout(wait)
        },
        move || {
            to_ask.send("walking").unwrap();
            from_ask.recv_timeout(wait)
        },
    );
    assert_eq!((walked, asked), (Ok("asking"), Ok("walking")));
}

#[test]
fn a_submodules_own_status_says_whether_its_gitlink_is_modified() {
    let recorded = gix::ObjectId::from_hex(b"1111111111111111111111111111111111111111").unwrap();
    let modified = |status: &str| super::checkout_is_modified(status.as_bytes(), &recorded);
    let same = "# branch.oid 1111111111111111111111111111111111111111\n# branch.head main\n";
    assert!(!modified(same));
    assert!(!modified(""));
    // Nothing is checked out yet, which Git takes to match any commit.
    assert!(!modified("# branch.oid (initial)\n# branch.head main\n"));
    assert!(modified(
        "# branch.oid 2222222222222222222222222222222222222222\n# branch.head (detached)\n"
    ));
    for record in [
        "? new.txt",
        "1 .M N... 100644 100644 100644 1111 1111 changed.txt",
        "2 R. N... 100644 100644 100644 1111 1111 R100 to.txt\tfrom.txt",
        "u UU N... 100644 100644 100644 100644 1111 1111 1111 both.txt",
    ] {
        assert!(modified(&format!("{same}{record}\n")), "{record}");
    }
    // Ignored files are not shown, and would not make it dirty.
    assert!(!modified(&format!("{same}! ignored.txt\n")));
}

/// A child repository in `path` with one commit, its Git directory inside
/// the checkout or, as `git submodule` leaves it, among the parent's modules.
fn commit_child(root: &Path, path: &str, absorbed: bool) -> String {
    let child = root.join(path);
    std::fs::create_dir_all(&child).unwrap();
    if absorbed {
        let modules = root.join(".git/modules").join(path);
        std::fs::create_dir_all(modules.parent().unwrap()).unwrap();
        let separate = format!("--separate-git-dir={}", modules.display());
        git_success(&child, &["init", "-q", &separate]);
        for (key, value) in [("user.name", "t"), ("user.email", "t@example.invalid")] {
            git_success(&child, &["config", key, value]);
        }
        git_success(&child, &["config", "commit.gpgsign", "false"]);
    } else {
        init_test_repo(&child);
    }
    write_file(&child, "tracked.txt", "base\n");
    git_success(&child, &["add", "."]);
    git_success(&child, &["commit", "-qm", "child"]);
    crate::refs::head_oid(&gix::open(&child).unwrap())
        .unwrap()
        .unwrap()
        .to_string()
}

fn add_gitlink(root: &Path, path: &str, oid: &str) {
    git_success(
        root,
        &["update-index", "--add", "--cacheinfo", "160000", oid, path],
    );
}

#[test]
fn checked_out_submodules_are_asked_directly_and_the_rest_through_the_repository() {
    use gix::index::entry::Flags;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    write_file(root, "file.txt", "base\n");
    git_success(root, &["add", "."]);
    git_success(root, &["commit", "-qm", "seed"]);
    let plain = commit_child(root, "plain", false);
    let absorbed = commit_child(root, "absorbed", true);
    add_gitlink(root, "plain", &plain);
    add_gitlink(root, "absorbed", &absorbed);
    // Not checked out, and checked out as something Git may not take for a
    // repository: an empty `.git`, and files with no `.git` at all.
    for path in ["empty", "hollow", "files"] {
        add_gitlink(root, path, &plain);
        std::fs::create_dir(root.join(path)).unwrap();
    }
    std::fs::create_dir(root.join("hollow/.git")).unwrap();
    write_file(root, "files/note.txt", "not a repository\n");
    git_success(root, &["commit", "-qm", "gitlinks"]);

    let select = |staged: &[FileStatus]| {
        let selection = super::gitlink_selection(&gix::open(root).unwrap(), staged).unwrap();
        let checkouts: Vec<_> = selection
            .checkouts
            .iter()
            .map(|(path, oid)| (path.to_str().unwrap().to_owned(), oid.to_string()))
            .collect();
        (checkouts, selection.paths)
    };
    let both = vec![
        ("absorbed".to_owned(), absorbed.clone()),
        ("plain".to_owned(), plain.clone()),
    ];
    let uncertain: Vec<std::path::PathBuf> = vec!["files".into(), "hollow".into()];
    assert_eq!(select(&[]), (both.clone(), uncertain.clone()));

    // A staged change at the gitlink is Git's to report on both sides.
    let staged = FileStatus {
        path: "plain".into(),
        kind: FileStatusKind::Modified,
        conflict: None,
    };
    let (checkouts, paths) = select(&[staged]);
    assert_eq!(checkouts, both[..1]);
    assert_eq!(
        paths,
        ["files", "hollow", "plain"].map(std::path::PathBuf::from)
    );

    // So is a gitlink Git is told to assume unchanged.
    let repo = gix::open(root).unwrap();
    let mut index = (*repo.index_or_empty().unwrap()).clone();
    let position = index
        .entries()
        .iter()
        .position(|entry| entry.path(&index) == "absorbed")
        .unwrap();
    index.entries_mut()[position].flags |= Flags::ASSUME_VALID;
    index.write(Default::default()).unwrap();
    let (checkouts, paths) = select(&[]);
    assert_eq!(checkouts, both[1..]);
    assert_eq!(
        paths,
        ["absorbed", "files", "hollow"].map(std::path::PathBuf::from)
    );
}

/// Asked in a checkout whose `.git` it does not take for a repository, Git
/// looks for one above and would answer for the parent.
#[test]
fn a_checkout_is_never_answered_for_by_the_repository_above_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    write_file(root, "file.txt", "base\n");
    write_file(
        root,
        "untracked/inside.txt",
        "the parent would report this\n",
    );
    git_success(root, &["add", "file.txt"]);
    git_success(root, &["commit", "-qm", "seed"]);
    let token = CancellationToken::new();
    let ask = |path: &str| {
        super::gitlink_status_output(root, super::GitlinkAsk::Checkout(Path::new(path)), &token)
            .unwrap()
    };
    assert_eq!(ask("untracked"), None);
    let oid = commit_child(root, "child", false);
    let status = String::from_utf8(ask("child").unwrap()).unwrap();
    assert!(
        status.starts_with(&format!("# branch.oid {oid}\n")),
        "{status}"
    );
    assert!(!status.contains("inside.txt"), "{status}");
}

/// Git looks into each checked-out submodule in turn. Asked directly, or
/// through the repository in several queries at once, each must come out as
/// in Git's one query.
#[test]
fn checked_out_gitlinks_read_as_git_reads_them_in_one_query() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    write_file(root, "file.txt", "base\n");
    git_success(root, &["add", "."]);
    git_success(root, &["commit", "-qm", "seed"]);
    let child_path = |child: usize| std::path::PathBuf::from(format!("modules/c{child:02}"));
    let children: Vec<String> = (0..20)
        .map(|child| child_path(child).to_str().unwrap().to_owned())
        .collect();
    for (number, child) in children.iter().enumerate() {
        let oid = commit_child(root, child, number % 2 == 1);
        add_gitlink(root, child, &oid);
    }
    // A submodule of a submodule, recorded before the parent's commit.
    let nesting = root.join(&children[12]);
    let nested = commit_child(&nesting, "nested", false);
    add_gitlink(&nesting, "nested", &nested);
    git_success(&nesting, &["commit", "-qm", "nested"]);
    let oid = crate::refs::head_oid(&gix::open(&nesting).unwrap())
        .unwrap()
        .unwrap()
        .to_string();
    add_gitlink(root, &children[12], &oid);
    git_success(root, &["commit", "-qm", "gitlinks"]);

    // One of each kind of change, in checkouts that different queries get.
    let child = |number: usize| root.join(&children[number]);
    write_file(&child(3), "untracked.txt", "new\n");
    write_file(&child(10), "tracked.txt", "changed\n");
    write_file(&child(17), "tracked.txt", "committed\n");
    git_success(&child(17), &["commit", "-qam", "ahead"]);
    write_file(&child(6), "tracked.txt", "staged\n");
    git_success(&child(6), &["add", "."]);
    write_file(&nesting.join("nested"), "untracked.txt", "new\n");
    // And of what leaves a gitlink as it is: nothing checked out yet, an
    // ignored file, and untracked files the submodule is set not to show.
    std::fs::remove_dir_all(child(4).join(".git")).unwrap();
    std::fs::remove_file(child(4).join("tracked.txt")).unwrap();
    init_test_repo(&child(4));
    write_file(&child(14), ".git/info/exclude", "ignored.txt\n");
    write_file(&child(14), "ignored.txt", "ignored\n");
    git_success(&child(8), &["config", "status.showUntrackedFiles", "no"]);
    write_file(&child(8), "untracked.txt", "new\n");

    let paths: Vec<&str> = children.iter().map(String::as_str).collect();
    let (staged, expected) = gitlink_status_from_git(root, &paths);
    let modified: Vec<_> = expected.iter().map(|entry| entry.path.clone()).collect();
    assert_eq!(modified, [3, 6, 10, 12, 17].map(child_path), "{expected:?}");

    let repo = open_repo(root);
    assert_eq!(
        selected_gitlinks(root, &[]).map(|paths| paths.len()),
        Some(20)
    );
    // Asked before anything is cached: what is staged needs no checkout.
    let token = CancellationToken::new();
    let (staged_alone, commands) =
        crate::command_trace::capture(|| repo.staged_status_cancellable_impl(&token).unwrap());
    assert_eq!(staged_alone, staged);
    assert_eq!(commands.len(), 0, "{commands:?}");
    let (status, commands) = crate::command_trace::capture(|| repo.status_impl().unwrap());
    assert_eq!(*status.unstaged, expected);
    assert_eq!(*status.staged, staged);
    assert_eq!(commands.len(), 20, "one question to each checkout");
    assert_eq!(
        repo.worktree_status_cancellable_impl(&token).unwrap(),
        expected
    );

    let queries = super::gitlink_queries(paths.iter().map(Into::into).collect(), 8);
    assert_eq!(queries.len(), 2);
    let records = super::run_repository_query(root, super::RepositoryQuery::Paths(queries), &token);
    let outputs = super::GitlinkOutputs {
        records,
        modified: Ok(Vec::new()),
    };
    let (mut staged, mut unstaged) = (Vec::new(), Vec::new());
    super::apply_gitlink_outputs(outputs, &mut staged, &mut unstaged).unwrap();
    unstaged.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(unstaged, expected);
    assert!(staged.is_empty());

    // The repository's own untracked files are not Git's to look for here,
    // while untracked files inside a submodule still make it modified.
    write_file(root, "untracked.txt", "new\n");
    let everything = super::RepositoryQuery::Everything;
    let records = super::run_repository_query(root, everything, &token);
    let record = records[0].as_ref().unwrap().as_ref().unwrap();
    let records: Vec<_> = record.split(|byte| *byte == 0).collect();
    assert!(!records.iter().any(|record| record.starts_with(b"? ")));
    let gitlinks = |number: usize| {
        let path = children[number].as_bytes();
        records
            .iter()
            .filter(|record| record.ends_with(path))
            .count()
    };
    assert_eq!([3, 12].map(gitlinks), [1, 1]);
}
