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
        .log_history_mode_page(HistoryMode::FullReachable, 20, None)
        .unwrap();
    assert_eq!(count(Work::LogWalkObjectRead), 4_096);
    assert_eq!(count(Work::LogTopologyBuild), 1);

    // The application requests its index after the bootstrap page. The index,
    // resumed pages and filtered views must all reuse the same topology.
    let index = repo
        .build_history_index(HistoryMode::FullReachable, None, &cancel, &mut |_| {})
        .unwrap()
        .unwrap();
    assert_eq!(index.len(), 4_096);
    let second = repo
        .log_history_mode_page(HistoryMode::FullReachable, 20, first.next_cursor.as_ref())
        .unwrap();
    let rows = repo.read_history_range(&index, 0..40, &cancel).unwrap();
    assert_eq!(&rows.commits[..20], first.commits.as_slice());
    assert_eq!(&rows.commits[20..], second.commits.as_slice());
    let filtered = repo
        .build_history_index(HistoryMode::NoMerges, Some("You"), &cancel, &mut |_| {})
        .unwrap()
        .unwrap();
    assert_eq!(filtered.len(), index.len());
    assert_eq!(count(Work::LogWalkObjectRead), 4_096);
    assert_eq!(count(Work::LogTopologyBuild), 1);

    // Same repository handle, new ref snapshot. Old indexed ranges stay valid.
    run_git(dir.path(), &["reset", "-q", "--hard", "HEAD~100"]);
    let changed = repo
        .build_history_index(HistoryMode::FullReachable, None, &cancel, &mut |_| {})
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

#[test]
fn indexed_history_compact_order_matches_gix_with_skew_ties_and_redundant_tips() {
    use std::io::Write as _;
    for format in ["sha1", "sha256"] {
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
        let mut command = Command::new("git");
        test_git_env::apply(&mut command);
        let mut child = command
            .arg("-C")
            .arg(dir.path())
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
            .build_history_index(HistoryMode::AllBranches, None, &cancel, &mut |_| {})
            .unwrap()
            .unwrap();
        assert_eq!(count(Work::LogWalkObjectRead), 256);
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
        .log_history_mode_page(HistoryMode::FullReachable, 20, None)
        .unwrap();
    assert_eq!(first.commits.len(), 20);
    assert_eq!(count(Work::LogWalkObjectRead), 0);
    assert_eq!(count(Work::LogTopologyBuild), 0);
}
