//! Reproduce a newly fetched older tip in a disposable shared clone.
//! Set `GITCOMET_HISTORY_TEST_REPO` and run this release test with `--ignored --nocapture`.
#![allow(
    clippy::disallowed_methods,
    reason = "manual Git fixture and wall clock measurements"
)]

use gitcomet_core::domain::HistoryMode;
use gitcomet_core::history_perf::{self, Work};
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_git_gix::GixBackend;
use std::ffi::OsStr;
use std::path::Path;
use std::time::Instant;

fn git(dir: &Path, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> String {
    let result = gitcomet_core::test_support::git_fixture::git_command()
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .env("GIT_AUTHOR_DATE", "1000000000 +0000")
        .env("GIT_COMMITTER_DATE", "1000000000 +0000")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}

#[test]
#[ignore = "measures an older tip against GITCOMET_HISTORY_TEST_REPO"]
fn fetch_older_tip() {
    let source = std::env::var_os("GITCOMET_HISTORY_TEST_REPO").expect("repository path");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clone");
    git(
        dir.path(),
        [
            OsStr::new("clone"),
            OsStr::new("--quiet"),
            OsStr::new("--shared"),
            OsStr::new("--no-checkout"),
            OsStr::new("--no-tags"),
            OsStr::new("--single-branch"),
            source.as_ref(),
            path.as_os_str(),
        ],
    );
    let repo = GixBackend.open(&path).unwrap();
    let index = || {
        repo.build_history_index(
            &HistoryMode::AllBranches.into(),
            None,
            &Default::default(),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .unwrap()
        .unwrap()
    };
    let _capture = history_perf::capture();
    let older = index();
    let late = git(
        &path,
        [
            "commit-tree",
            "HEAD~100^{tree}",
            "-p",
            "HEAD~100",
            "-m",
            "older work",
        ],
    );
    git(&path, ["update-ref", "refs/remotes/origin/older", &late]);
    let builds = history_perf::count(Work::HistoryIndexBuild);
    let index_extensions = history_perf::count(Work::HistoryIndexExtend);
    let index_reads = history_perf::count(Work::IndexObjectRead);
    let reads = history_perf::count(Work::LogWalkObjectRead);
    let ext = history_perf::count(Work::LogTopologyExtend);
    let start = Instant::now();
    let refreshed = index();
    let elapsed = start.elapsed();
    let row = refreshed
        .position_bytes(gix::ObjectId::from_hex(late.as_bytes()).unwrap().as_bytes())
        .unwrap();
    assert!(row > 0);
    assert_eq!(refreshed.len(), older.len() + 1);
    println!(
        "rows={} older_row={} elapsed_ms={} index_builds={} index_extensions={} topology_extensions={} topology_header_reads={} index_header_reads={}",
        refreshed.len(),
        row,
        elapsed.as_secs_f64() * 1000.0,
        history_perf::count(Work::HistoryIndexBuild) - builds,
        history_perf::count(Work::HistoryIndexExtend) - index_extensions,
        history_perf::count(Work::LogTopologyExtend) - ext,
        history_perf::count(Work::LogWalkObjectRead) - reads,
        history_perf::count(Work::IndexObjectRead) - index_reads,
    );
}
