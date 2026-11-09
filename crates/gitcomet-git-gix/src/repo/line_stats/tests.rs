use super::*;
use crate::test_support::{git_success, init_test_repo, open_repo, write_file};

/// The counts alone: these tests are about counting, not the edit.
fn counts(stats: Option<&LineStats>) -> Option<(Option<u32>, Option<u32>)> {
    stats.map(|stats| (stats.additions, stats.deletions))
}

#[test]
fn review_unlocked_annex_stats_skip_payload_reads_on_every_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(
        dir,
        "asset.bin",
        "/annex/objects/WORM-s3000000-m1--payload\n",
    );
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "annex pointer"]);
    std::fs::write(dir.join("asset.bin"), vec![b'x'; 3_000_000]).unwrap();
    let repo = open_repo(dir);
    let entries = [FileStatus {
        path: "asset.bin".into(),
        kind: FileStatusKind::Modified,
        conflict: None,
    }];
    take_worktree_reads_for_tests();
    for _ in 0..2 {
        assert_eq!(
            repo.line_stats_for_entries_impl(&entries, &CancellationToken::new())
                .unwrap()
                .unstaged[std::path::Path::new("asset.bin")],
            LineStats::UNKNOWN
        );
    }
    assert_eq!(
        take_worktree_reads_for_tests(),
        0,
        "unknown annex counts must not read/hash the unlocked payload"
    );
}

#[test]
fn unstaged_lfs_payloads_never_start_the_clean_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    let pointer = |oid: char, size: usize| {
        format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {size}\n",
            oid.to_string().repeat(64)
        )
    };
    for path in ["staged.bin", "unstaged.bin"] {
        write_file(dir, path, &pointer('a', 5));
    }
    // Stored as content: committed before a driver was configured.
    write_file(dir, "content.bin", "content\n");
    write_file(dir, ".gitattributes", "*.bin filter=lfs\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "pointers"]);
    // A staged change is pointer against pointer, both already stored.
    write_file(dir, "staged.bin", &pointer('b', 7));
    git_success(dir, &["add", "staged.bin"]);
    write_file(dir, "intent.bin", "new payload\n");
    git_success(dir, &["add", "-N", "intent.bin"]);
    let ran = dir.join("clean-filter-ran");
    git_success(
        dir,
        &[
            "config",
            "filter.lfs.clean",
            &format!("touch '{}'; cat", ran.display()),
        ],
    );
    std::fs::write(dir.join("unstaged.bin"), b"smudged payload\n").unwrap();
    std::fs::write(dir.join("untracked.bin"), b"new payload\n").unwrap();
    write_file(dir, "content.bin", "content\nedited\n");
    // Listed by hand: the status walk itself re-reads the racily clean
    // `staged.bin` through the filter, which is not the counts' doing.
    let entries = [
        ("unstaged.bin", FileStatusKind::Modified),
        ("untracked.bin", FileStatusKind::Untracked),
        ("intent.bin", FileStatusKind::Added),
        ("content.bin", FileStatusKind::Modified),
    ]
    .map(|(path, kind)| FileStatus {
        path: path.into(),
        kind,
        conflict: None,
    });
    let repo = open_repo(dir);
    take_worktree_reads_for_tests();
    for _ in 0..2 {
        let stats = repo
            .line_stats_for_entries_impl(&entries, &CancellationToken::new())
            .unwrap();
        for path in ["unstaged.bin", "untracked.bin", "intent.bin", "content.bin"] {
            assert_eq!(
                stats.unstaged[std::path::Path::new(path)],
                LineStats::UNKNOWN,
                "{path}"
            );
        }
        assert_eq!(
            counts(stats.staged.get(std::path::Path::new("staged.bin"))),
            Some((Some(2), Some(2)))
        );
    }
    assert_eq!(take_worktree_reads_for_tests(), 0);
    assert!(!ran.exists(), "the LFS clean filter must not run");
}

/// A `working-tree-encoding` file counts the lines git stores, decoded
/// with git's (iconv) labels as the diff panel decodes them; gix knows
/// only WHATWG labels.
#[test]
fn working_tree_encoded_files_count_their_decoded_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    let utf16 = |text: &str| -> Vec<u8> {
        [0xFF, 0xFE]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect()
    };
    write_file(
        dir,
        ".gitattributes",
        "*.txt working-tree-encoding=UTF-16LE-BOM\n",
    );
    std::fs::write(dir.join("tracked.txt"), utf16("a\n")).unwrap();
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    std::fs::write(dir.join("tracked.txt"), utf16("a\nb\n")).unwrap();
    std::fs::write(dir.join("new.txt"), utf16("a\nb\nc\n")).unwrap();

    let stats = open_repo(dir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .unwrap();

    let unstaged = |path: &str| counts(stats.unstaged.get(std::path::Path::new(path)));
    assert_eq!(unstaged("tracked.txt"), Some((Some(1), Some(0))));
    assert_eq!(unstaged("new.txt"), Some((Some(3), Some(0))));
}

/// A cleaning driver can turn binary-looking bytes into text, so its
/// paths are counted from what it stores, not sniffed raw.
#[cfg(unix)]
#[test]
fn a_cleaning_driver_decides_binary_after_it_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, ".gitattributes", "*.dat filter=strip\n");
    write_file(dir, "data.dat", "one\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    git_success(dir, &["config", "filter.strip.clean", "tr -d '\\000'"]);
    std::fs::write(dir.join("data.dat"), b"one\n\0two\n").unwrap();

    let stats = open_repo(dir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .unwrap();

    assert_eq!(
        counts(stats.unstaged.get(std::path::Path::new("data.dat"))),
        Some((Some(1), Some(0)))
    );
}

/// `git lfs track` leaves `.gitattributes` uncommitted at first; the
/// worktree copy already decides which reads would start the filter.
#[test]
fn an_uncommitted_lfs_track_keeps_the_clean_filter_from_running() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "asset.bin", "base\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    write_file(dir, ".gitattributes", "*.bin filter=lfs\n");
    let ran = dir.join("clean-filter-ran");
    git_success(
        dir,
        &[
            "config",
            "filter.lfs.clean",
            &format!("touch '{}'; cat", ran.display()),
        ],
    );
    write_file(dir, "asset.bin", "base\nedited\n");
    write_file(dir, "new.bin", "payload\n");

    let stats = open_repo(dir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .unwrap();

    for path in ["asset.bin", "new.bin"] {
        assert_eq!(
            stats.unstaged[std::path::Path::new(path)],
            LineStats::UNKNOWN,
            "{path}"
        );
    }
    assert_eq!(
        counts(stats.unstaged.get(std::path::Path::new(".gitattributes"))),
        Some((Some(1), Some(0)))
    );
    assert!(!ran.exists(), "the LFS clean filter must not run");
}

#[test]
fn lfs_pointer_counts_stay_known_where_no_clean_filter_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    let pointer = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 5\n",
        "a".repeat(64)
    );
    for path in ["deleted.bin", "plain.txt", "tracked.dat"] {
        write_file(dir, path, &pointer);
    }
    write_file(
        dir,
        ".gitattributes",
        "*.bin filter=lfs\n*.dat filter=lfs\n",
    );
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "pointers"]);
    std::fs::remove_file(dir.join("deleted.bin")).unwrap();
    write_file(dir, "plain.txt", "edited\n");
    write_file(dir, "tracked.dat", "smudged payload\n");
    write_file(dir, "new.txt", "new\n");
    write_file(dir, "new.bin", "new payload\n");
    let unstaged = |path: &str| {
        open_repo(dir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap()
            .unstaged[std::path::Path::new(path)]
    };
    // No LFS driver is configured (git-lfs not installed).
    assert_eq!(
        counts(Some(&unstaged("deleted.bin"))),
        Some((Some(0), Some(3)))
    );
    assert_eq!(
        counts(Some(&unstaged("plain.txt"))),
        Some((Some(1), Some(3)))
    );
    assert_eq!(counts(Some(&unstaged("new.txt"))), Some((Some(1), Some(0))));

    let ran = dir.join("clean-filter-ran");
    git_success(
        dir,
        &[
            "config",
            "filter.lfs.clean",
            &format!("touch '{}'; cat", ran.display()),
        ],
    );
    // A deletion and a path outside `filter=lfs` still read no filter.
    assert_eq!(
        counts(Some(&unstaged("deleted.bin"))),
        Some((Some(0), Some(3)))
    );
    assert_eq!(
        counts(Some(&unstaged("plain.txt"))),
        Some((Some(1), Some(3)))
    );
    assert_eq!(counts(Some(&unstaged("new.txt"))), Some((Some(1), Some(0))));
    assert_eq!(unstaged("tracked.dat"), LineStats::UNKNOWN);
    assert_eq!(unstaged("new.bin"), LineStats::UNKNOWN);
    assert!(!ran.exists(), "the LFS clean filter must not run");
}

/// Times six runs of a probe's refresh and prints min, median and max.
fn print_timing(label: &str, mut refresh: impl FnMut()) {
    let mut ms: Vec<f64> = (0..6)
        .map(|_| {
            let started = std::time::Instant::now();
            refresh();
            started.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    ms.sort_by(f64::total_cmp);
    eprintln!(
        "{label}: min {:.1} ms, median {:.1} ms, max {:.1} ms",
        ms[0],
        ms[ms.len() / 2],
        ms[ms.len() - 1]
    );
}

/// Refresh cost of counting untracked files: 4 KiB each, in 200 folders,
/// text and binary. Files inside the racy window are read on every
/// refresh; older ones only on the first.
#[test]
#[ignore = "timing probe"]
fn perf_untracked_line_stats_refresh() {
    let body = |seed: usize| -> String {
        (0..64)
            .map(|line| format!("untracked {seed:06} line {line:02} representative text ...\n"))
            .collect()
    };
    for untracked in [2_000usize, 20_000] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "seed.txt", "seed\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-q", "-m", "seed"]);
        for ix in 0..untracked {
            write_file(
                dir,
                &format!("text/d{:03}/f{ix:06}.txt", ix % 200),
                &body(ix),
            );
            let binary = dir.join(format!("binary/d{:03}/f{ix:06}.bin", ix % 200));
            std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
            let mut bytes = body(ix).into_bytes();
            bytes[10] = 0;
            std::fs::write(binary, bytes).unwrap();
        }
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
        assert_eq!(entries.len(), 2 * untracked);
        print_timing(&format!("untracked={untracked} x2: status walk"), || {
            std::hint::black_box(repo.worktree_status_cancellable_impl(&token).unwrap());
        });
        for kind in ["text", "binary"] {
            let files: Vec<FileStatus> = entries
                .iter()
                .filter(|entry| entry.path.starts_with(kind))
                .cloned()
                .collect();
            let _aged = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
            let refresh = || {
                std::hint::black_box(repo.line_stats_for_entries_impl(&files, &token).unwrap());
            };
            print_timing(
                &format!("untracked={untracked} {kind}: line stats, first count"),
                || {
                    *repo.line_stats_memo() = LineStatsMemo::default();
                    refresh();
                },
            );
            refresh();
            print_timing(
                &format!("untracked={untracked} {kind}: line stats, unchanged"),
                refresh,
            );
        }
    }
}

/// Refresh cost of untracked binaries: 200 of 2 MiB, read only to be
/// reported unknown.
#[test]
#[ignore = "timing probe"]
fn perf_untracked_binary_line_stats_refresh() {
    // End-of-line conversion may apply, as in most repositories; and a
    // filter naming no configured driver, like `git lfs track`'s line
    // where git-lfs is not installed.
    for attributes in [
        "* text=auto\n",
        "* text=auto\n*.bin filter=nodriver -text\n",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "seed.txt", "seed\n");
        write_file(dir, ".gitattributes", attributes);
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-q", "-m", "seed"]);
        for ix in 0..200u32 {
            let payload: Vec<u8> = (0..2 * 1024 * 1024u32)
                .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8 ^ ix as u8)
                .collect();
            std::fs::write(dir.join(format!("blob{ix:03}.bin")), payload).unwrap();
        }
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
        assert_eq!(entries.len(), 200);
        print_timing(&format!("untracked binaries under {attributes:?}"), || {
            let stats = repo.line_stats_for_entries_impl(&entries, &token).unwrap();
            assert!(
                stats
                    .unstaged
                    .values()
                    .all(|stats| *stats == LineStats::UNKNOWN)
            );
        });
    }
}

/// Refresh cost of 200 untracked `working-tree-encoding` files in a
/// repository of 20,000 tracked ones.
#[test]
#[ignore = "timing probe"]
fn perf_untracked_encoded_line_stats_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(
        dir,
        ".gitattributes",
        "*.u16 working-tree-encoding=UTF-16LE-BOM\n",
    );
    for ix in 0..20_000u32 {
        write_file(
            dir,
            &format!("tracked/d{:03}/f{ix:05}.txt", ix % 200),
            "x\n",
        );
    }
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-q", "-m", "seed"]);
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("a\nb\n".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    for ix in 0..200u32 {
        std::fs::write(dir.join(format!("new{ix:03}.u16")), &utf16).unwrap();
    }
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
    assert_eq!(entries.len(), 200);
    print_timing("untracked encoded", || {
        let stats = repo.line_stats_for_entries_impl(&entries, &token).unwrap();
        assert!(
            stats
                .unstaged
                .values()
                .all(|stats| stats.additions == Some(2))
        );
    });
}

/// Refresh cost with real `git-lfs`: 20 modified 1 MiB payloads.
#[test]
#[ignore = "timing probe; needs git-lfs"]
fn perf_lfs_unstaged_line_stats_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    git_success(dir, &["lfs", "install", "--local"]);
    git_success(dir, &["lfs", "track", "*.bin"]);
    let payload = |seed: u8| -> Vec<u8> {
        (0..1024 * 1024u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8 ^ seed)
            .collect()
    };
    for ix in 0..20u8 {
        std::fs::write(dir.join(format!("asset{ix:02}.bin")), payload(ix)).unwrap();
    }
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-q", "-m", "payloads"]);
    for ix in 0..20u8 {
        std::fs::write(dir.join(format!("asset{ix:02}.bin")), payload(ix ^ 0x5a)).unwrap();
    }
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    print_timing("lfs unstaged line stats refresh", || {
        let stats = repo.uncommitted_line_stats_impl(&token).unwrap();
        assert_eq!(stats.unstaged.len(), 20);
    });
}

#[test]
fn review_line_stats_memo_separates_identical_pairs_in_each_lane() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    for path in ["staged.txt", "unstaged.txt"] {
        write_file(dir, path, "before\n");
    }
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    for path in ["staged.txt", "unstaged.txt"] {
        write_file(dir, path, "after\n");
    }
    git_success(dir, &["add", "staged.txt"]);
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    take_line_stats_diffs_for_tests();
    let first = repo.uncommitted_line_stats_impl(&token).unwrap();
    assert_eq!(
        take_line_stats_diffs_for_tests(),
        2,
        "each lane must compute its own entry before reusing it"
    );
    assert_eq!(repo.uncommitted_line_stats_impl(&token).unwrap(), first);
    assert_eq!(take_line_stats_diffs_for_tests(), 0);
}

#[test]
fn line_stats_do_not_run_annex_filters_on_ordinary_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "notes.txt", "before\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    // Annex installs a catch-all filter, including files stored in Git.
    write_file(dir, ".gitattributes", "* filter=annex\n");
    git_success(
        dir,
        &["config", "filter.annex.clean", "printf 'filtered\\n'; cat"],
    );
    git_success(dir, &["config", "filter.annex.required", "true"]);
    let repo = open_repo(dir);
    write_file(dir, "notes.txt", "after\nextra\n");
    let entries = [FileStatus {
        path: "notes.txt".into(),
        kind: FileStatusKind::Modified,
        conflict: None,
    }];
    let stats = repo
        .line_stats_for_entries_impl(&entries, &CancellationToken::new())
        .unwrap();
    assert_eq!(
        counts(stats.unstaged.get(std::path::Path::new("notes.txt"))),
        Some((Some(2), Some(1)))
    );
}

#[test]
fn annex_payload_counts_never_reuse_staged_pointer_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    git_success(dir, &["config", "annex.uuid", "test-annex"]);
    for path in ["staged.bin", "unstaged.bin"] {
        write_file(dir, path, "/annex/objects/WORM-s8-m1--payload\n");
    }
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "pointers"]);
    for path in ["staged.bin", "unstaged.bin"] {
        write_file(dir, path, "payload\n");
    }
    git_success(dir, &["add", "staged.bin"]);
    let stats = open_repo(dir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .unwrap();
    assert_eq!(
        stats.staged[std::path::Path::new("staged.bin")].additions,
        Some(1)
    );
    assert_eq!(
        stats.unstaged[std::path::Path::new("unstaged.bin")],
        LineStats::UNKNOWN
    );
}

#[test]
fn supplied_status_counts_match_standalone_across_file_kinds() {
    use gitcomet_core::services::GitRepository;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    let token = CancellationToken::new();
    let check = || {
        let repo = open_repo(dir);
        let status = repo.status(&token).unwrap();
        assert_eq!(
            repo.uncommitted_line_stats_for_status(&status, &token)
                .unwrap(),
            repo.uncommitted_line_stats(&token).unwrap()
        );
    };
    check(); // Empty, unborn repository.
    write_file(dir, ".gitattributes", "*.txt text eol=crlf\n");
    for name in ["edit.txt", "delete.txt", "rename.txt"] {
        write_file(dir, name, &lines("base", 30));
    }
    std::fs::write(dir.join("binary.bin"), b"\0binary").unwrap();
    git_success(dir, &["add", "."]);
    check(); // Staged additions on an unborn HEAD.
    git_success(dir, &["commit", "-m", "seed"]);
    git_success(dir, &["mv", "rename.txt", "renamed.txt"]);
    write_file(dir, "renamed.txt", &lines("base", 31));
    write_file(dir, "edit.txt", "base\r\nstaged\r\n");
    git_success(dir, &["add", "."]);
    write_file(dir, "edit.txt", "base\r\nstaged\r\nunstaged\r\n");
    std::fs::remove_file(dir.join("delete.txt")).unwrap();
    std::fs::write(dir.join("binary.bin"), b"\0changed binary").unwrap();
    write_file(dir, "untracked.txt", "new\n");
    check(); // Mixed staged/unstaged, rename, deletion, binary, CRLF, untracked.
}

#[test]
fn empty_supplied_status_does_not_launch_clean_filter_or_rescan_worktree() {
    use gitcomet_core::services::GitRepository;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "asset.bin", "before\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    write_file(dir, ".gitattributes", "*.bin filter=broken\n");
    git_success(
        dir,
        &[
            "config",
            "filter.broken.clean",
            "gitcomet-intentionally-missing-filter",
        ],
    );
    git_success(dir, &["config", "filter.broken.required", "true"]);
    write_file(dir, "asset.bin", "after!\n");
    std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join("asset.bin"))
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(120)),
        )
        .unwrap();
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let empty = gitcomet_core::domain::RepoStatus::default();
    let counts = repo
        .uncommitted_line_stats_for_status(&empty, &token)
        .unwrap();
    assert!(counts.staged.is_empty() && counts.unstaged.is_empty());
    // Control: a traversal actually needs the broken clean filter.
    assert!(repo.worktree_status(&token).is_err());
}

#[cfg(unix)]
#[test]
fn unstaged_symlinks_have_counts() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    symlink("missing-before", dir.join("link")).unwrap();
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    std::fs::remove_file(dir.join("link")).unwrap();
    symlink("missing-after", dir.join("link")).unwrap();
    let stats = open_repo(dir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .unwrap();
    assert_eq!(
        counts(stats.unstaged.get(std::path::Path::new("link"))),
        Some((Some(1), Some(1)))
    );
}

/// APFS and HFS+ refuse non-UTF-8 file names with `EILSEQ`, so this
/// fixture cannot be created on macOS.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn unstaged_non_utf8_paths_have_counts() {
    use std::os::unix::ffi::OsStringExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    let path = PathBuf::from(std::ffi::OsString::from_vec(b"file-\xff.txt".to_vec()));
    std::fs::write(dir.join(&path), "before\n").unwrap();
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    std::fs::write(dir.join(&path), "after\n").unwrap();
    let stats = open_repo(dir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .unwrap();
    assert_eq!(counts(stats.unstaged.get(&path)), Some((Some(1), Some(1))));
}

#[test]
fn shared_pipeline_applies_each_files_attributes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, ".gitattributes", "*.txt text eol=crlf\nraw/* -text\n");
    write_file(dir, "a.txt", "base\n");
    write_file(dir, "nested/b.txt", "base\n");
    write_file(dir, "raw/c.txt", "base\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    for path in ["a.txt", "nested/b.txt", "raw/c.txt"] {
        write_file(dir, path, "base\r\nadded\r\n");
    }
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
    let stats = repo.line_stats_for_entries_impl(&entries, &token).unwrap();
    for path in ["a.txt", "nested/b.txt"] {
        assert_eq!(
            counts(stats.unstaged.get(std::path::Path::new(path))),
            Some((Some(1), Some(0)))
        );
    }
    assert_eq!(
        counts(stats.unstaged.get(std::path::Path::new("raw/c.txt"))),
        Some((Some(2), Some(1)))
    );
    // The supplied snapshot bounds the scan; it must not rediscover other changes.
    let status = gitcomet_core::domain::RepoStatus {
        unstaged: std::sync::Arc::new(entries[..1].to_vec()),
        staged: Default::default(),
    };
    let subset = gitcomet_core::services::GitRepository::uncommitted_line_stats_for_status(
        &repo, &status, &token,
    )
    .unwrap();
    assert_eq!(subset.unstaged.len(), 1);
}

fn lines(prefix: &str, count: usize) -> String {
    (0..count)
        .map(|index| format!("{prefix} line {index}\n"))
        .collect()
}

#[test]
fn counts_both_lanes_independently() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    write_file(workdir, "staged.txt", &lines("base", 5));
    write_file(workdir, "unstaged.txt", &lines("base", 5));
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-m", "seed"]);

    write_file(workdir, "staged.txt", &lines("base", 7));
    git_success(workdir, &["add", "staged.txt"]);
    write_file(workdir, "unstaged.txt", &lines("base", 4));

    let stats = open_repo(workdir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .expect("line stats");

    assert_eq!(
        counts(stats.staged.get(std::path::Path::new("staged.txt"))),
        Some((Some(2), Some(0)))
    );
    assert_eq!(
        counts(stats.unstaged.get(std::path::Path::new("unstaged.txt"))),
        Some((Some(0), Some(1)))
    );
    assert!(
        !stats
            .staged
            .contains_key(std::path::Path::new("unstaged.txt")),
        "an unstaged edit must not leak into the staged lane"
    );
}

/// Joining `git diff --numstat` by path would report the whole file as
/// added; gix hands us both ids, so the counts are the real edit.
#[test]
fn a_staged_rename_reports_the_edit_not_the_whole_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    write_file(workdir, "before.txt", &lines("base", 200));
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-m", "seed"]);

    git_success(workdir, &["mv", "before.txt", "after.txt"]);
    write_file(workdir, "after.txt", &lines("base", 201));
    git_success(workdir, &["add", "after.txt"]);

    let stats = open_repo(workdir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .expect("line stats");
    let renamed = stats
        .staged
        .get(std::path::Path::new("after.txt"))
        .copied()
        .expect("renamed file has counts");

    assert_eq!(renamed.additions, Some(1), "only the appended line is new");
    assert_eq!(renamed.deletions, Some(0));
}

/// The index holds nothing for an untracked file, so all of it is added,
/// as the diff panel shows it.
#[test]
fn untracked_files_count_every_line_as_added() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    write_file(workdir, "seed.txt", "seed\n");
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-m", "seed"]);

    write_file(workdir, "brand-new.txt", &lines("new", 3));
    write_file(workdir, "nested/unterminated.txt", "one\ntwo");
    write_file(workdir, "empty.txt", "");

    let stats = open_repo(workdir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .expect("line stats");
    let unstaged = |path: &str| counts(stats.unstaged.get(std::path::Path::new(path)));
    assert_eq!(unstaged("brand-new.txt"), Some((Some(3), Some(0))));
    assert_eq!(
        unstaged("nested/unterminated.txt"),
        Some((Some(2), Some(0)))
    );
    assert_eq!(unstaged("empty.txt"), Some((Some(0), Some(0))));
    assert!(stats.staged.is_empty(), "nothing was staged");
}

/// `git add -N` moves a new file from untracked to added; what it adds
/// stays the same.
#[test]
fn an_untracked_file_counts_like_the_same_file_added_with_intent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    write_file(workdir, "seed.txt", "seed\n");
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-m", "seed"]);
    write_file(workdir, "new.rs", "use a;\nfn a() {}\n");
    let new_file = |workdir: &std::path::Path| {
        open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats")
            .unstaged
            .get(std::path::Path::new("new.rs"))
            .copied()
    };

    let untracked = new_file(workdir);
    git_success(workdir, &["add", "-N", "new.rs"]);
    let intent_to_add = new_file(workdir);

    assert_eq!(counts(untracked.as_ref()), Some((Some(2), Some(0))));
    assert!(untracked.is_some_and(|stats| stats.edit.is_some()));
    assert_eq!(untracked, intent_to_add);
}

/// An untracked file is read once; later refreshes reuse its counts
/// while it keeps its stamp.
#[test]
fn unchanged_untracked_files_are_not_read_again() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed.txt", "seed\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    write_file(dir, "a.txt", &lines("a", 3));
    write_file(dir, "nested/b.txt", &lines("b", 2));
    let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    take_worktree_reads_for_tests();

    let first = repo.uncommitted_line_stats_impl(&token).unwrap();
    assert_eq!(take_worktree_reads_for_tests(), 2);
    let second = repo.uncommitted_line_stats_impl(&token).unwrap();

    assert_eq!(second, first);
    assert_eq!(
        counts(second.unstaged.get(std::path::Path::new("a.txt"))),
        Some((Some(3), Some(0)))
    );
    // Off Unix a stamp has no inode or ctime, so it is never trusted.
    assert_eq!(
        take_worktree_reads_for_tests(),
        if cfg!(unix) { 0 } else { 2 }
    );
}

/// A binary untracked file is sniffed once; later refreshes reuse its
/// unknown while it keeps its stamp.
#[test]
fn unchanged_untracked_binaries_are_not_read_again() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed.txt", "seed\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    std::fs::write(dir.join("a.bin"), b"\0binary").unwrap();
    std::fs::write(dir.join("b.bin"), b"also\0binary").unwrap();
    let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    take_worktree_reads_for_tests();

    repo.uncommitted_line_stats_impl(&token).unwrap();
    assert_eq!(take_worktree_reads_for_tests(), 2);
    let second = repo.uncommitted_line_stats_impl(&token).unwrap();

    for path in ["a.bin", "b.bin"] {
        assert_eq!(
            second.unstaged[std::path::Path::new(path)],
            LineStats::UNKNOWN,
            "{path}"
        );
    }
    // Off Unix a stamp has no inode or ctime, so it is never trusted.
    assert_eq!(
        take_worktree_reads_for_tests(),
        if cfg!(unix) { 0 } else { 2 }
    );
}

/// An edit that keeps the size and modification time still changes the
/// ctime, so the file is counted again.
#[test]
fn an_edited_untracked_file_is_counted_again() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed.txt", "seed\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    let path = dir.join("notes.txt");
    std::fs::write(&path, "ab\ncd\n").unwrap();
    let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let repo = open_repo(dir);
    let notes = || {
        let stats = repo
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap();
        counts(stats.unstaged.get(std::path::Path::new("notes.txt")))
    };
    assert_eq!(notes(), Some((Some(2), Some(0))));

    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, "abcde\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();

    assert_eq!(notes(), Some((Some(1), Some(0))));
}

/// A file inside the racy window can change again without its stamp
/// moving, so its counts are not kept.
#[test]
fn a_racy_untracked_file_is_read_on_every_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed.txt", "seed\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    write_file(dir, "new.txt", &lines("new", 2));
    // Frozen right after the write, however slowly the scans run.
    #[cfg(unix)]
    let _clock =
        crate::repo::RacyClockSkew::set_for_path(dir.join("new.txt"), std::time::Duration::ZERO);
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    take_worktree_reads_for_tests();

    let first = repo.uncommitted_line_stats_impl(&token).unwrap();
    let second = repo.uncommitted_line_stats_impl(&token).unwrap();

    assert_eq!(second, first);
    assert_eq!(take_worktree_reads_for_tests(), 2);
}

/// The memo keys on how a file converts: without its
/// `working-tree-encoding`, the same UTF-16 bytes are binary.
#[test]
fn an_attribute_change_counts_untracked_files_again() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(
        dir,
        ".gitattributes",
        "*.txt working-tree-encoding=UTF-16LE-BOM\n",
    );
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("a\nb\nc\n".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    std::fs::write(dir.join("notes.txt"), utf16).unwrap();
    let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let repo = open_repo(dir);
    let notes = || {
        repo.uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap()
            .unstaged[std::path::Path::new("notes.txt")]
    };
    assert_eq!(counts(Some(&notes())), Some((Some(3), Some(0))));

    write_file(dir, ".gitattributes", "*.md text\n");

    assert_eq!(notes(), LineStats::UNKNOWN);
}

/// The memo keys on the conversion config: with `core.safecrlf`, a CRLF
/// file that `core.autocrlf=input` would rewrite cannot be stored.
#[test]
fn a_conversion_config_change_counts_untracked_files_again() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed.txt", "seed\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    git_success(dir, &["config", "core.autocrlf", "input"]);
    std::fs::write(dir.join("crlf.txt"), "a\r\nb\r\n").unwrap();
    let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let repo = open_repo(dir);
    let crlf = || {
        repo.uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap()
            .unstaged[std::path::Path::new("crlf.txt")]
    };
    assert_eq!(counts(Some(&crlf())), Some((Some(2), Some(0))));

    git_success(dir, &["config", "core.safecrlf", "true"]);

    assert_eq!(crlf(), LineStats::UNKNOWN);
}

/// A clean program's output can change while the file does not, so the
/// files it converts are counted on every refresh.
#[cfg(unix)]
#[test]
fn files_a_clean_program_converts_are_counted_on_every_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, ".gitattributes", "*.dat filter=pick\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    git_success(dir, &["config", "filter.pick.clean", "cat"]);
    write_file(dir, "data.dat", "one\ntwo\n");
    let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let repo = open_repo(dir);
    let data = || {
        let stats = repo
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap();
        counts(stats.unstaged.get(std::path::Path::new("data.dat")))
    };
    assert_eq!(data(), Some((Some(2), Some(0))));

    git_success(dir, &["config", "filter.pick.clean", "sed -n 1p"]);

    assert_eq!(data(), Some((Some(1), Some(0))));
}

/// A count kept before git-lfs was configured is not served after it:
/// the LFS gate decides before the memo is consulted.
#[test]
fn an_lfs_driver_configured_after_a_count_leaves_the_file_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, ".gitattributes", "*.bin filter=lfs\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    write_file(dir, "new.bin", "payload\n");
    let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let repo = open_repo(dir);
    let new_bin = || {
        repo.uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap()
            .unstaged[std::path::Path::new("new.bin")]
    };
    // No driver is configured yet, so the file is read as it is.
    assert_eq!(counts(Some(&new_bin())), Some((Some(1), Some(0))));

    let ran = dir.join("clean-filter-ran");
    git_success(
        dir,
        &[
            "config",
            "filter.lfs.clean",
            &format!("touch '{}'; cat", ran.display()),
        ],
    );

    assert_eq!(new_bin(), LineStats::UNKNOWN);
    assert!(!ran.exists(), "the LFS clean filter must not run");
}

/// The memo holds only files the last scan listed, so paths that stop
/// being untracked do not pile up.
#[cfg(unix)]
#[test]
fn untracked_counts_are_kept_only_for_listed_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "seed.txt", "seed\n");
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    write_file(dir, "kept.txt", "kept\n");
    write_file(dir, "removed.txt", "removed\n");
    let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
    let repo = open_repo(dir);
    let token = CancellationToken::new();
    let memo_paths = || {
        let mut paths: Vec<PathBuf> = repo.line_stats_memo().untracked.keys().cloned().collect();
        paths.sort();
        paths
    };
    repo.uncommitted_line_stats_impl(&token).unwrap();
    assert_eq!(
        memo_paths(),
        [PathBuf::from("kept.txt"), PathBuf::from("removed.txt")]
    );

    std::fs::remove_file(dir.join("removed.txt")).unwrap();
    repo.uncommitted_line_stats_impl(&token).unwrap();

    assert_eq!(memo_paths(), [PathBuf::from("kept.txt")]);
}

#[test]
fn binary_and_oversized_files_report_unknown_counts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    std::fs::write(workdir.join("blob.bin"), b"\0\0\0binary\0\0").expect("write binary");
    git_success(workdir, &["add", "blob.bin"]);
    git_success(workdir, &["commit", "-m", "binary"]);
    std::fs::write(workdir.join("blob.bin"), b"\0\0\0changed\0\0").expect("edit binary");
    std::fs::write(workdir.join("new.bin"), b"\0\0\0new\0\0").expect("write binary");
    let oversized = "line\n".repeat(COMMIT_STATS_MAX_BLOB_BYTES / 5 + 1);
    write_file(workdir, "oversized.txt", &oversized);

    let stats = open_repo(workdir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .expect("line stats");

    for path in ["blob.bin", "new.bin", "oversized.txt"] {
        assert_eq!(
            stats.unstaged.get(std::path::Path::new(path)),
            Some(&LineStats::UNKNOWN),
            "{path} is reported as unknown, not zero"
        );
    }
}

/// A repo that closed mid-scan must not keep this running.
#[test]
fn a_cancelled_token_stops_the_scan() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    write_file(workdir, "a.txt", &lines("base", 3));
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-m", "seed"]);
    write_file(workdir, "a.txt", &lines("edit", 3));

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let err = open_repo(workdir)
        .uncommitted_line_stats_impl(&cancelled)
        .expect_err("a cancelled scan must not return counts");
    assert!(
        matches!(err.kind(), gitcomet_core::error::ErrorKind::Cancelled),
        "expected Cancelled, got {err:?}"
    );

    // Control: the guard is not just refusing everything.
    assert!(
        open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .is_ok()
    );
}

#[test]
fn rescans_diff_only_pairs_whose_content_moved() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "big.txt", &lines("base", 200));
    write_file(dir, "staged.txt", &lines("base", 20));
    std::fs::write(dir.join("blob.bin"), b"\0before").unwrap();
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    let edited = |from: usize| lines("base", from) + &lines("edited", 200 - from);
    write_file(dir, "big.txt", &edited(150));
    write_file(dir, "staged.txt", &lines("base", 25));
    git_success(dir, &["add", "staged.txt"]);
    std::fs::write(dir.join("blob.bin"), b"\0after").unwrap();

    let token = CancellationToken::new();
    let scan = |repo: &super::super::GixRepo| {
        let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
        repo.line_stats_for_entries_impl(&entries, &token).unwrap()
    };
    let repo = open_repo(dir);
    take_line_stats_diffs_for_tests();
    let first = scan(&repo);
    // big + staged; a binary worktree file is never diffed.
    assert_eq!(take_line_stats_diffs_for_tests(), 2);

    // An editor save of unchanged content.
    write_file(dir, "big.txt", &edited(150));
    assert_eq!(scan(&repo), first);
    assert_eq!(take_line_stats_diffs_for_tests(), 0);

    write_file(dir, "big.txt", &edited(100));
    let moved = scan(&repo);
    assert_eq!(take_line_stats_diffs_for_tests(), 1);
    assert_eq!(moved, scan(&open_repo(dir)));
    assert_eq!(
        counts(moved.unstaged.get(std::path::Path::new("big.txt"))),
        Some((Some(100), Some(100)))
    );

    // Moving a pair to the staged lane computes that lane's own entry.
    git_success(dir, &["add", "big.txt"]);
    take_line_stats_diffs_for_tests();
    let staged = scan(&repo);
    assert_eq!(take_line_stats_diffs_for_tests(), 1);
    assert_eq!(staged, scan(&open_repo(dir)));
    assert!(
        !staged
            .unstaged
            .contains_key(std::path::Path::new("big.txt"))
    );
}

/// A save between two staged walks reuses the first (a cache hit) and
/// re-seeds the memo as that walk left it: counted pairs stay, unknown
/// ones (a binary) are never kept, so the next walk sniffs the binary
/// again and diffs only the newly staged file, save or no save.
#[test]
fn a_staged_walk_reuse_keeps_the_memo_as_the_walk_left_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    for name in ["a.txt", "b.txt", "c.txt"] {
        write_file(dir, name, &lines("base", 10));
    }
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    write_file(dir, "a.txt", &lines("base", 12));
    std::fs::write(dir.join("blob.bin"), b"\0staged").unwrap();
    git_success(dir, &["add", "a.txt", "blob.bin"]);

    let token = CancellationToken::new();
    let scan = |repo: &super::super::GixRepo| {
        let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
        repo.line_stats_for_entries_impl(&entries, &token).unwrap()
    };
    let repo = open_repo(dir);
    take_line_stats_diffs_for_tests();
    scan(&repo);
    assert_eq!(
        take_line_stats_diffs_for_tests(),
        2,
        "a.txt and the binary sniff"
    );

    write_file(dir, "b.txt", &lines("edited", 10));
    scan(&repo);
    assert_eq!(take_line_stats_diffs_for_tests(), 1, "the save: only b.txt");

    // Distinct content: the memo is keyed by blob ids, not paths, and
    // would serve c.txt from b.txt's pair otherwise.
    write_file(dir, "c.txt", &lines("changed", 10));
    git_success(dir, &["add", "c.txt"]);
    scan(&repo);
    assert_eq!(
        take_line_stats_diffs_for_tests(),
        2,
        "c.txt and the binary sniffed again; a.txt and b.txt from the memo"
    );
}

/// A worktree save changes neither HEAD nor the index, so the staged lane
/// reuses its last walk; staging and committing each walk again.
#[test]
fn staged_counts_walk_again_only_when_head_or_index_moves() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_test_repo(dir);
    write_file(dir, "a.txt", &lines("base", 10));
    write_file(dir, "b.txt", &lines("base", 10));
    git_success(dir, &["add", "."]);
    git_success(dir, &["commit", "-m", "seed"]);
    write_file(dir, "a.txt", &lines("base", 12));
    git_success(dir, &["add", "a.txt"]);

    let token = CancellationToken::new();
    let scan = |repo: &super::super::GixRepo| {
        let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
        repo.line_stats_for_entries_impl(&entries, &token).unwrap()
    };
    let repo = open_repo(dir);
    take_staged_walks_for_tests();
    let first = scan(&repo);
    assert_eq!(take_staged_walks_for_tests(), 1);

    write_file(dir, "b.txt", &lines("edited", 10));
    let saved = scan(&repo);
    assert_eq!(take_staged_walks_for_tests(), 0, "a save reuses the walk");
    assert_eq!(saved.staged, first.staged);
    assert_eq!(saved, scan(&open_repo(dir)));

    git_success(dir, &["add", "b.txt"]);
    take_staged_walks_for_tests();
    let staged = scan(&repo);
    assert_eq!(take_staged_walks_for_tests(), 1, "staging moves the index");
    assert_eq!(staged, scan(&open_repo(dir)));
    assert!(staged.staged.contains_key(std::path::Path::new("b.txt")));

    git_success(dir, &["commit", "-m", "both"]);
    take_staged_walks_for_tests();
    let committed = scan(&repo);
    assert_eq!(take_staged_walks_for_tests(), 1, "a commit moves HEAD");
    assert!(committed.staged.is_empty());

    // HEAD alone: a soft reset leaves the index file untouched.
    git_success(dir, &["reset", "--soft", "HEAD~1"]);
    let reset = scan(&repo);
    assert_eq!(take_staged_walks_for_tests(), 1, "a reset moves HEAD");
    assert_eq!(reset, scan(&open_repo(dir)));
    assert_eq!(reset.staged.len(), 2);
}

#[test]
fn staged_walk_preserves_cancellation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_repo(tmp.path());
    write_file(tmp.path(), "a.txt", &lines("staged", 3));
    git_success(tmp.path(), &["add", "."]);

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let repo = open_repo(tmp.path()).repo();
    let err = staged_line_stats(&repo, &mut MemoScan::default(), &cancellation)
        .expect_err("cancelled staged walk");
    assert!(matches!(err.kind(), ErrorKind::Cancelled), "{err:?}");
    assert_eq!(
        staged_line_stats(&repo, &mut MemoScan::default(), &CancellationToken::new())
            .expect("uncancelled staged walk")
            .len(),
        1
    );
}

#[test]
fn staged_counts_work_on_an_unborn_head() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    write_file(workdir, "first.txt", &lines("new", 4));
    git_success(workdir, &["add", "first.txt"]);

    let stats = open_repo(workdir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .expect("line stats");
    assert_eq!(
        counts(stats.staged.get(std::path::Path::new("first.txt"))),
        Some((Some(4), Some(0)))
    );
}

/// A file the status walk listed but that is gone when read has no counts
/// until the next walk: a gone untracked file is not an empty one, and the
/// next walk lists a gone tracked file as deleted.
#[test]
fn a_file_gone_before_its_read_has_unknown_counts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    write_file(workdir, "tracked.txt", &lines("base", 3));
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-m", "seed"]);
    std::fs::remove_file(workdir.join("tracked.txt")).expect("remove");
    let entries = [
        FileStatus {
            path: "tracked.txt".into(),
            kind: FileStatusKind::Modified,
            conflict: None,
        },
        FileStatus {
            path: "untracked.txt".into(),
            kind: FileStatusKind::Untracked,
            conflict: None,
        },
    ];

    let stats = open_repo(workdir)
        .line_stats_for_entries_impl(&entries, &CancellationToken::new())
        .expect("line stats");

    for path in ["tracked.txt", "untracked.txt"] {
        assert_eq!(
            stats.unstaged[std::path::Path::new(path)],
            LineStats::UNKNOWN,
            "{path}"
        );
    }
}

#[test]
fn a_deleted_worktree_file_counts_every_line_as_removed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);

    write_file(workdir, "gone.txt", &lines("base", 3));
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-m", "seed"]);
    std::fs::remove_file(workdir.join("gone.txt")).expect("remove");

    let stats = open_repo(workdir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .expect("line stats");
    assert_eq!(
        counts(stats.unstaged.get(std::path::Path::new("gone.txt"))),
        Some((Some(0), Some(3)))
    );
}
/// `a.rs` and `nested/b.rs` take the same edit (at different indents),
/// `c.rs` a different one.
fn seed_same_edit(workdir: &std::path::Path, from: &str, to: &str) {
    write_file(workdir, "a.rs", &format!("use {from};\nfn a() {{}}\n"));
    write_file(
        workdir,
        "nested/b.rs",
        &format!("    use {from};\nfn b() {{}}\n"),
    );
    write_file(workdir, "c.rs", &format!("use {from};\nfn c() {{}}\n"));
    if !to.is_empty() {
        write_file(workdir, "a.rs", &format!("use {to};\nfn a() {{}}\n"));
        write_file(
            workdir,
            "nested/b.rs",
            &format!("    use {to};\nfn b() {{}}\n"),
        );
        write_file(workdir, "c.rs", &format!("use {to}_other;\nfn c() {{}}\n"));
    }
}

fn assert_same_edit(edit: impl Fn(&str) -> Option<gitcomet_core::edit_signature::EditSignature>) {
    let a = edit("a.rs");
    assert!(a.is_some(), "a text edit has a signature");
    assert_eq!(a, edit("nested/b.rs"), "the same edit shares it");
    assert_ne!(a, edit("c.rs"), "a different edit does not");
}

fn head_id(workdir: &std::path::Path) -> gitcomet_core::domain::CommitId {
    let repo = gix::open(workdir).expect("open repo");
    gitcomet_core::domain::CommitId(repo.head_id().expect("head").to_string().into())
}

#[test]
fn the_same_edit_shares_a_signature_in_both_uncommitted_lanes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    seed_same_edit(workdir, "old", "");
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-m", "seed"]);
    seed_same_edit(workdir, "old", "staged");
    git_success(workdir, &["add", "."]);
    seed_same_edit(workdir, "staged", "unstaged");

    let stats = open_repo(workdir)
        .uncommitted_line_stats_impl(&CancellationToken::new())
        .expect("line stats");
    for lane in [&stats.staged, &stats.unstaged] {
        assert_same_edit(|path| lane[std::path::Path::new(path)].edit);
    }
    assert_ne!(
        stats.staged[std::path::Path::new("a.rs")].edit,
        stats.unstaged[std::path::Path::new("a.rs")].edit,
        "each lane signs its own edit"
    );
}

#[test]
fn the_same_edit_shares_a_signature_in_a_commit_and_against_the_worktree() {
    use gitcomet_core::services::GitRepository as _;
    let tmp = tempfile::tempdir().expect("tempdir");
    let workdir = tmp.path();
    init_test_repo(workdir);
    seed_same_edit(workdir, "old", "");
    git_success(workdir, &["add", "."]);
    git_success(workdir, &["commit", "-m", "seed"]);
    let seed = head_id(workdir);
    seed_same_edit(workdir, "old", "new");
    git_success(workdir, &["commit", "-am", "edit"]);
    let repo = open_repo(workdir);

    let details = repo.commit_details(&head_id(workdir)).expect("details");
    let committed = |path: &str| {
        details
            .files
            .iter()
            .find(|f| f.path == std::path::Path::new(path))?
            .line_stats
            .edit
    };
    assert_same_edit(committed);

    // The worktree side has no blob; its counts come from numstat, so the
    // edit is read separately.
    seed_same_edit(workdir, "new", "newer");
    let against_worktree = repo
        .compare_files(
            &seed,
            None,
            &gitcomet_core::services::ComparisonOptions::direct(),
            &CancellationToken::new(),
        )
        .expect("comparison");
    let edit = |path: &str| {
        against_worktree
            .files
            .iter()
            .find(|f| f.path == std::path::Path::new(path))?
            .line_stats
            .edit
    };
    assert_same_edit(edit);
    assert_ne!(
        edit("a.rs"),
        committed("a.rs"),
        "measured from the seed, the edit is old to newer"
    );
}
