use super::*;
use gitcomet_core::history_perf::{Work, capture, count};
use gitcomet_core::services::CancellationToken;

#[test]
fn indexed_history_scans_headers_once_and_reuses_bootstrap_topology() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    fast_import_linear_history(dir.path(), 4_096);
    let repo = GixBackend.open(dir.path()).unwrap();
    let cancel = CancellationToken::new();
    let _capture = capture();
    let first = repo
        .log_history_mode_page(
            HistoryMode::FullReachable,
            20,
            None,
            &gitcomet_core::services::CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(count(Work::LogWalkObjectRead), 4_096);
    assert_eq!(count(Work::LogTopologyBuild), 1);

    // The application requests its index after the bootstrap page. The index,
    // resumed pages and filtered views must all reuse the same topology.
    let index = repo
        .build_history_index(
            &HistoryMode::FullReachable.into(),
            None,
            &Default::default(),
            &cancel,
            &mut |_| {},
        )
        .unwrap()
        .unwrap();
    assert_eq!(index.len(), 4_096);
    let second = repo
        .log_history_mode_page(
            HistoryMode::FullReachable,
            20,
            first.next_cursor.as_ref(),
            &gitcomet_core::services::CancellationToken::new(),
        )
        .unwrap();
    let rows = repo.read_history_range(&index, 0..40, &cancel).unwrap();
    assert_eq!(&rows.commits[..20], first.commits.as_slice());
    assert_eq!(&rows.commits[20..], second.commits.as_slice());
    let filtered = repo
        .build_history_index(
            &HistoryMode::NoMerges.into(),
            Some("You"),
            &Default::default(),
            &cancel,
            &mut |_| {},
        )
        .unwrap()
        .unwrap();
    assert_eq!(filtered.len(), index.len());
    assert_eq!(count(Work::LogWalkObjectRead), 4_096);
    assert_eq!(count(Work::LogTopologyBuild), 1);

    // Same repository handle, new ref snapshot. Old indexed ranges stay valid.
    run_git(dir.path(), &["reset", "-q", "--hard", "HEAD~100"]);
    let changed = repo
        .build_history_index(
            &HistoryMode::FullReachable.into(),
            None,
            &Default::default(),
            &cancel,
            &mut |_| {},
        )
        .unwrap()
        .unwrap();
    assert_eq!(changed.len(), 3_996);
    assert_eq!(changed.commit_id(0), index.commit_id(100));
    assert_eq!(count(Work::LogWalkObjectRead), 4_096 + 3_996);
    assert_eq!(count(Work::LogTopologyBuild), 2);
    assert_eq!(
        repo.read_history_range(&index, 0..40, &cancel).unwrap(),
        rows
    );
}

/// A history with a commit gone falls back to the commit-date walk after one
/// topology build: the paged walk would only build the same one again.
#[test]
fn a_missing_commit_builds_the_topology_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-q", "-b", "main"]);
    run_git(path, &["config", "user.email", "you@example.com"]);
    run_git(path, &["config", "user.name", "You"]);
    for message in ["one", "two", "three"] {
        run_git(path, &["commit", "-q", "--allow-empty", "-m", message]);
    }
    let root = git_stdout(path, &["rev-list", "--max-parents=0", "HEAD"]);
    let root = root.trim();
    std::fs::remove_file(path.join(".git/objects").join(&root[..2]).join(&root[2..])).unwrap();
    let repo = GixBackend.open(path).unwrap();
    let _capture = capture();
    let _ = repo.build_history_index(
        &HistoryMode::FullReachable.into(),
        None,
        &Default::default(),
        &CancellationToken::new(),
        &mut |_| {},
    );
    assert_eq!(count(Work::LogTopologyBuild), 1);
}

/// One damaged record in the commit-graph: the history is read from the
/// objects instead, as gix's own walk reads it. Tips far apart, so that the
/// graph is scanned whole rather than walked lazily by gix.
#[test]
fn a_damaged_commit_graph_record_still_gives_the_history() {
    const COMMITS: usize = 4_200;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-q", "-b", "master"]);
    fast_import_linear_history(path, COMMITS);
    let tree = git_stdout(path, &["mktree"]);
    let orphan = git_stdout(
        path,
        &["-c", "user.name=You", "-c", "user.email=you@example.com"]
            .into_iter()
            .chain(["commit-tree", "-m", "orphan", tree.trim()])
            .collect::<Vec<_>>(),
    );
    run_git(path, &["update-ref", "refs/heads/orphan", orphan.trim()]);
    run_git(path, &["commit-graph", "write", "--reachable"]);
    let index = |repo: &dyn gitcomet_core::services::GitRepository| {
        repo.build_history_index(
            &HistoryMode::AllBranches.into(),
            None,
            &Default::default(),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .unwrap()
        .unwrap()
        .len()
    };
    assert_eq!(index(&*GixBackend.open(path).unwrap()), COMMITS + 1);

    // The first root commit's record: a second parent and no first.
    let graph = path.join(".git/objects/info/commit-graph");
    let mut bytes = std::fs::read(&graph).unwrap();
    let chunk = |id: &[u8; 4]| {
        let chunks = bytes[6] as usize;
        (0..chunks)
            .map(|at| 8 + at * 12)
            .find(|&entry| &bytes[entry..entry + 4] == id)
            .map(|entry| u64::from_be_bytes(bytes[entry + 4..entry + 12].try_into().unwrap()))
            .unwrap() as usize
    };
    let (ids, data) = (chunk(b"OIDL"), chunk(b"CDAT"));
    let root = git_stdout(path, &["rev-list", "--max-parents=0", "master"]);
    let root = gix::ObjectId::from_hex(root.trim().as_bytes()).unwrap();
    let position = (0..=COMMITS)
        .find(|at| &bytes[ids + at * 20..ids + at * 20 + 20] == root.as_bytes())
        .unwrap();
    let second_parent = data + position * 36 + 24;
    bytes[second_parent..second_parent + 4].copy_from_slice(&0u32.to_be_bytes());
    let mut permissions = std::fs::metadata(&graph).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&graph, permissions).unwrap();
    std::fs::write(&graph, bytes).unwrap();
    assert_eq!(index(&*GixBackend.open(path).unwrap()), COMMITS + 1);
}

#[test]
fn indexed_history_compact_order_matches_gix_with_skew_ties_and_redundant_tips() {
    use std::io::Write as _;
    // Header by header, from a commit-graph of every commit, and from one
    // that holds only the commits one branch reaches.
    #[derive(Clone, Copy, Debug)]
    enum Graph {
        None,
        Whole,
        Partial,
    }
    for (format, graph) in [
        ("sha1", Graph::None),
        ("sha256", Graph::None),
        ("sha1", Graph::Whole),
        ("sha256", Graph::Whole),
        ("sha1", Graph::Partial),
    ] {
        let dir = tempfile::tempdir().unwrap();
        run_git(
            dir.path(),
            &["init", "-q", &format!("--object-format={format}")],
        );
        let mut stream = String::new();
        for row in 0..256 {
            let mark = row + 1;
            let time = 1_600_000_000 + (row * 17 % 11); // ties and dates before parents
            stream.push_str(&format!(
                "commit refs/heads/tip-{row}\nmark :{mark}\ncommitter You <you@example.com> {time} +0000\ndata 4\nnode\n"
            ));
            if row % 37 != 0 {
                stream.push_str(&format!("from :{row}\n"));
                if row > 2 && row % 3 == 0 {
                    stream.push_str(&format!("merge :{}\n", 1 + row / 2));
                }
                if row > 4 && row % 5 == 0 {
                    stream.push_str("merge :1\n");
                }
            }
            stream.push('\n');
        }
        let mut child = git_cmd(dir.path())
            .args(["fast-import", "--quiet"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stream.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        run_git(dir.path(), &["symbolic-ref", "HEAD", "refs/heads/tip-255"]);
        let in_graph = match graph {
            Graph::None => 0,
            Graph::Whole => {
                run_git(dir.path(), &["commit-graph", "write", "--reachable"]);
                256
            }
            Graph::Partial => {
                let tip = git_stdout(dir.path(), &["rev-parse", "refs/heads/tip-100"]);
                let mut child = git_cmd(dir.path())
                    .args(["commit-graph", "write", "--stdin-commits"])
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(tip.as_bytes())
                    .unwrap();
                assert!(child.wait().unwrap().success());
                git_stdout(dir.path(), &["rev-list", "--count", "refs/heads/tip-100"])
                    .trim()
                    .parse::<u64>()
                    .unwrap()
            }
        };

        let repo = gix::open(dir.path()).unwrap();
        let mut tips: Vec<_> = git_stdout(dir.path(), &["for-each-ref", "--format=%(objectname)"])
            .lines()
            .map(|id| gix::ObjectId::from_hex(id.as_bytes()).unwrap())
            .collect();
        tips.sort_unstable();
        let expected: Vec<_> = gix::traverse::commit::topo::Builder::new(&repo.objects)
            .with_tips(tips)
            .sorting(gix::traverse::commit::topo::Sorting::DateOrder)
            .build()
            .unwrap()
            .map(|info| info.unwrap())
            .collect();
        let repo = GixBackend.open(dir.path()).unwrap();
        let _capture = capture();
        let cancel = CancellationToken::new();
        let index = repo
            .build_history_index(
                &HistoryMode::AllBranches.into(),
                None,
                &Default::default(),
                &cancel,
                &mut |_| {},
            )
            .unwrap()
            .unwrap();
        // The index reads every commit, so it orders them in one pass: from
        // the commit-graph's tables where it has them, else from headers.
        assert_eq!(count(Work::LogTopologyBuild), 1, "{format} {graph:?}");
        assert_eq!(
            count(Work::LogWalkObjectRead),
            256 - in_graph,
            "{format} {graph:?}"
        );
        assert_eq!(index.len(), expected.len());
        for (row, info) in expected.iter().enumerate() {
            assert_eq!(
                index.id_bytes(row),
                Some(info.id.as_bytes()),
                "{format} {graph:?} row={row}"
            );
            assert_eq!(index.parents(row).len(), info.parent_ids.len());
            for (parent, id) in info.parent_ids.iter().enumerate() {
                assert_eq!(index.parent_id_bytes(row, parent), Some(id.as_bytes()));
            }
        }
    }
}

/// A pack written by Git starts with its commits, so their headers can be
/// read on several threads before the scan asks for them one by one.
#[test]
fn packed_commit_headers_are_read_ahead_and_give_the_same_order() {
    use std::io::Write as _;
    const COMMITS: usize = 9_000;
    for format in ["sha1", "sha256"] {
        let dir = tempfile::tempdir().unwrap();
        run_git(
            dir.path(),
            &["init", "-q", &format!("--object-format={format}")],
        );
        let mut stream = String::new();
        for row in 0..COMMITS {
            let mark = row + 1;
            let time = 1_600_000_000 + (row * 17 % 11); // ties and dates before parents
            let contents = format!("{row}\n");
            stream.push_str(&format!(
                "commit refs/heads/b{}\nmark :{mark}\ncommitter You <you@example.com> {time} +0000\ndata 4\nnode\n",
                row % 13,
            ));
            if row % 371 != 0 {
                stream.push_str(&format!("from :{row}\n"));
                if row > 2 && row % 3 == 0 {
                    stream.push_str(&format!("merge :{}\n", 1 + row / 2));
                }
                if row > 4 && row % 5 == 0 {
                    stream.push_str("merge :1\n");
                }
            }
            stream.push_str(&format!(
                "M 100644 inline file-{}.txt\ndata {}\n{contents}\n",
                row % 7,
                contents.len(),
            ));
        }
        let mut child = git_cmd(dir.path())
            .args(["fast-import", "--quiet"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stream.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        run_git(dir.path(), &["symbolic-ref", "HEAD", "refs/heads/b0"]);
        let commit_on = |branch: &str, parents: &[&str], time: i64| {
            let mut args: Vec<String> = [
                "-c",
                "user.name=You",
                "-c",
                "user.email=you@example.com",
                "commit-tree",
                "HEAD^{tree}",
            ]
            .map(str::to_owned)
            .into();
            for parent in parents {
                args.extend(["-p".to_owned(), (*parent).to_owned()]);
            }
            args.extend(["-m".to_owned(), "later".to_owned()]);
            let output = git_cmd(dir.path())
                .args(&args)
                .env("GIT_COMMITTER_DATE", format!("{time} +0000"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let id = String::from_utf8(output.stdout).unwrap().trim().to_owned();
            run_git(
                dir.path(),
                &["update-ref", &format!("refs/heads/{branch}"), &id],
            );
            id
        };
        let packs = || -> Vec<std::path::PathBuf> {
            std::fs::read_dir(dir.path().join(".git/objects/pack"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "pack")
                })
                .collect()
        };
        // One commit is kept in a pack of its own, out of the big pack that
        // holds its child: a header naming it cannot be read ahead.
        let imported = packs();
        let kept = commit_on("b6", &["refs/heads/b6"], 1_600_000_004);
        run_git(dir.path(), &["repack", "-dq"]);
        for pack in packs() {
            if !imported.contains(&pack) {
                std::fs::write(pack.with_extension("keep"), "").unwrap();
            }
        }
        commit_on("b6", &[&kept, "refs/heads/b7"], 1_600_000_002);
        // Commits first, then trees and blobs: the layout of a clone. Commits
        // no branch reaches are left out of the pack.
        run_git(dir.path(), &["repack", "-adq"]);
        let packed = git_stdout(dir.path(), &["rev-list", "--count", "--all"])
            .trim()
            .parse::<u64>()
            .unwrap()
            - 1;
        assert!(packed > 8_192, "{packed}");

        // Commits outside that pack are the scan's own to read: two in a
        // pack too small to be worth threads, three loose.
        let first = commit_on("b1", &["refs/heads/b1"], 1_600_000_003);
        commit_on("b2", &["refs/heads/b2", &first], 1_600_000_003);
        run_git(dir.path(), &["repack", "-dq"]);
        let third = commit_on("b3", &["refs/heads/b3", "refs/heads/b4"], 1_600_000_001);
        let fourth = commit_on("b3", &[&third], 1_600_000_020);
        commit_on("b5", &["refs/heads/b5", &fourth, &first], 1_600_000_000);

        let repo = gix::open(dir.path()).unwrap();
        let mut tips: Vec<_> = git_stdout(dir.path(), &["for-each-ref", "--format=%(objectname)"])
            .lines()
            .map(|id| gix::ObjectId::from_hex(id.as_bytes()).unwrap())
            .collect();
        tips.sort_unstable();
        let expected: Vec<_> = gix::traverse::commit::topo::Builder::new(&repo.objects)
            .with_tips(tips)
            .sorting(gix::traverse::commit::topo::Sorting::DateOrder)
            .build()
            .unwrap()
            .map(|info| info.unwrap())
            .collect();
        let repo = GixBackend.open(dir.path()).unwrap();
        let _capture = capture();
        let cancel = CancellationToken::new();
        let index = repo
            .build_history_index(
                &HistoryMode::AllBranches.into(),
                None,
                &Default::default(),
                &cancel,
                &mut |_| {},
            )
            .unwrap()
            .unwrap();
        // The scan reads its first 4,096 headers one by one, among them the
        // kept commit, its child and the five later ones, and the rest ahead.
        assert_eq!(count(Work::LogHeaderReadAhead), packed - 1, "{format}");
        assert_eq!(count(Work::LogWalkObjectRead), 4_096, "{format}");
        assert_eq!(index.len(), expected.len());
        for (row, info) in expected.iter().enumerate() {
            assert_eq!(
                index.id_bytes(row),
                Some(info.id.as_bytes()),
                "{format} row={row}"
            );
            assert_eq!(index.parents(row).len(), info.parent_ids.len());
            for (parent, id) in info.parent_ids.iter().enumerate() {
                assert_eq!(index.parent_id_bytes(row, parent), Some(id.as_bytes()));
            }
        }
    }
}

#[test]
fn indexed_history_uses_existing_commit_graph_without_eager_object_scan() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    fast_import_linear_history(dir.path(), 1_024);
    run_git(dir.path(), &["commit-graph", "write", "--reachable"]);
    let repo = GixBackend.open(dir.path()).unwrap();
    let _capture = capture();
    let first = repo
        .log_history_mode_page(
            HistoryMode::FullReachable,
            20,
            None,
            &gitcomet_core::services::CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(first.commits.len(), 20);
    assert_eq!(count(Work::LogWalkObjectRead), 0);
    assert_eq!(count(Work::LogTopologyBuild), 0);
}

/// With a commit-graph a page is walked lazily, which is at once from one tip
/// or from tips near each other. A tip far back makes that walk count its
/// way through everything above it first: such a page takes the topology an
/// index build orders in a fraction of that time, and the build shares it.
#[test]
fn a_page_from_tips_far_apart_orders_the_whole_graph_once_for_the_index_too() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    fast_import_linear_history(dir.path(), 6_000);
    run_git(dir.path(), &["branch", "near", "HEAD~8"]);
    run_git(dir.path(), &["commit-graph", "write", "--reachable"]);
    let cancel = CancellationToken::new();
    let page = |repo: &dyn gitcomet_core::services::GitRepository| {
        repo.log_history_mode_page(HistoryMode::AllBranches, 20, None, &cancel)
            .unwrap()
    };

    let repo = GixBackend.open(dir.path()).unwrap();
    let _capture = capture();
    let near = page(&*repo);
    assert_eq!(near.commits.len(), 20);
    assert_eq!(count(Work::LogTopologyBuild), 0);

    run_git(dir.path(), &["branch", "far", "HEAD~5000"]);
    let repo = GixBackend.open(dir.path()).unwrap();
    let far = page(&*repo);
    assert_eq!(far.commits, near.commits);
    assert_eq!(count(Work::LogTopologyBuild), 1);
    let index = repo
        .build_history_index(
            &HistoryMode::AllBranches.into(),
            None,
            &Default::default(),
            &cancel,
            &mut |_| {},
        )
        .unwrap()
        .unwrap();
    assert_eq!(index.len(), 6_000);
    let rows = repo.read_history_range(&index, 0..20, &cancel).unwrap();
    assert_eq!(rows.commits, far.commits);
    assert_eq!(count(Work::LogTopologyBuild), 1);
    assert_eq!(count(Work::LogWalkObjectRead), 0);
}

#[test]
fn indexed_history_honors_replacement_refs_like_git_and_bypasses_the_commit_graph() {
    fn rows(commits: &[gitcomet_core::domain::Commit]) -> Vec<String> {
        commits
            .iter()
            .map(|commit| {
                let parents: Vec<_> = commit.parent_ids.iter().map(|id| id.as_ref()).collect();
                format!("{} {} {}", commit.id, parents.join(" "), commit.summary)
            })
            .collect()
    }
    fn assert_matches_git(repo: &Path) {
        let expected: Vec<String> = git_stdout(repo, &["log", "--format=%H %P %s"])
            .lines()
            .map(str::to_owned)
            .collect();
        let opened = GixBackend.open(repo).unwrap();
        let cancel = CancellationToken::new();
        let page = opened
            .log_history_mode_page(HistoryMode::FullReachable, 50, None, &cancel)
            .unwrap();
        assert_eq!(
            rows(&page.commits),
            expected,
            "page rows must match git log"
        );
        let index = opened
            .build_history_index(
                &HistoryMode::FullReachable.into(),
                None,
                &Default::default(),
                &cancel,
                &mut |_| {},
            )
            .unwrap()
            .unwrap();
        let indexed = opened
            .read_history_range(&index, 0..index.len(), &cancel)
            .unwrap();
        assert_eq!(
            rows(&indexed.commits),
            expected,
            "index rows must match git log"
        );
    }

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    run_git(repo, &["init", "-q", "-b", "master"]);
    fast_import_linear_history(repo, 8);
    // The graph records the original parents, so it must not be consulted
    // while a replacement is installed.
    run_git(repo, &["commit-graph", "write", "--reachable"]);
    run_git(repo, &["replace", "HEAD~2", "HEAD~5"]);
    assert_eq!(
        git_stdout(repo, &["log", "--format=%s"])
            .lines()
            .collect::<Vec<_>>(),
        ["c7", "c6", "c2", "c1", "c0"]
    );
    assert_matches_git(repo);

    run_git(repo, &["config", "core.useReplaceRefs", "false"]);
    assert_eq!(git_stdout(repo, &["log", "--format=%s"]).lines().count(), 8);
    assert_matches_git(repo);
}
