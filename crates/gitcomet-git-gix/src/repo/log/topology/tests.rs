use super::*;
use gitcomet_core::services::CancellationToken;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc;
use std::time::Duration;

type Build = Box<dyn FnOnce(&dyn Fn() -> bool, Option<&Previous>) -> gix::Result<Topology> + Send>;

/// Adds commits `first..=last` to a repository through `git fast-import`,
/// each on some of the dozen before it: merges of up to five parents,
/// with times that repeat and run backwards.
fn import_commits(root: &std::path::Path, seed: u64, first: u64, last: u64) {
    let mut stream = String::new();
    for commit in first..=last {
        let mut state = seed ^ commit.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut below = |limit: u64| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % limit
        };
        let time = 1_700_000_000 + (commit / 4) * 10 - below(3) * 25;
        stream.push_str(&format!(
            "commit refs/heads/tip{}\nmark :{commit}\n\
                 committer T <t@example.invalid> {time} +0000\ndata 2\nc\n",
            commit % 7
        ));
        let reach = (commit - 1).min(12);
        let mut parents = Vec::new();
        let wanted = match below(20) {
            _ if commit == 1 => 0,
            0..=2 => 3 + below(3),
            3..=6 => 2,
            _ => 1,
        };
        while (parents.len() as u64) < wanted.min(reach) {
            let parent = commit - 1 - below(reach);
            if !parents.contains(&parent) {
                parents.push(parent);
            }
        }
        for (at, parent) in parents.iter().enumerate() {
            let edge = if at == 0 { "from" } else { "merge" };
            stream.push_str(&format!("{edge} :{parent}\n"));
        }
        stream.push('\n');
    }
    let marks = root.join(".git/marks");
    let import = format!("--import-marks-if-exists={}", marks.display());
    let export = format!("--export-marks={}", marks.display());
    crate::repo::log::tests::git_with_stdin(
        root,
        &["fast-import", "--quiet", "--force", &import, &export],
        stream.as_bytes(),
    );
}

/// The commits of a topology in its order.
fn walked(topology: Topology) -> Vec<(gix::ObjectId, Vec<gix::ObjectId>, Option<i64>)> {
    let walk = TopologyWalk {
        topology: Arc::new(topology),
        next: 0,
    };
    walk.map(|info| (info.id, info.parent_ids.to_vec(), info.commit_time))
        .collect()
}

/// The graph's commits are read in chunks on several threads and followed
/// from compact records; the order must be the one the objects give.
#[test]
fn a_commit_graph_scan_orders_a_history_as_the_object_scan_does() {
    use crate::test_support::{git_stdout, git_success, init_test_repo};
    for seed in [1, 2, 3] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init_test_repo(root);
        // A graph in two files, and commits newer than both.
        import_commits(root, seed, 1, 260);
        git_success(
            root,
            &["commit-graph", "write", "--reachable", "--split=no-merge"],
        );
        import_commits(root, seed, 261, 460);
        git_success(
            root,
            &["commit-graph", "write", "--reachable", "--split=no-merge"],
        );
        import_commits(root, seed, 461, 500);
        let graphs = std::fs::read_dir(root.join(".git/objects/info/commit-graphs"))
            .unwrap()
            .filter(|entry| {
                let name = entry.as_ref().unwrap().file_name();
                name.to_string_lossy().ends_with(".graph")
            })
            .count();
        assert_eq!(graphs, 2);

        // Every branch, one of them twice, and an ancestor of one.
        let mut tips: Vec<gix::ObjectId> = git_stdout(
            root,
            &["for-each-ref", "--format=%(objectname)", "refs/heads"],
        )
        .lines()
        .map(|id| gix::ObjectId::from_hex(id.as_bytes()).unwrap())
        .collect();
        tips.push(tips[2]);
        let ancestor = git_stdout(root, &["rev-parse", "refs/heads/tip3~3"]);
        tips.push(gix::ObjectId::from_hex(ancestor.trim().as_bytes()).unwrap());

        let store = gix::open(root).unwrap().into_sync();
        let graph = store
            .to_thread_local()
            .commit_graph_if_enabled()
            .unwrap()
            .unwrap();
        let never = || false;
        let capture = gitcomet_core::history_perf::capture();
        let from_graph = graph_scan::scan_in_chunks(&store, &graph, &tips, &never, 64, 0).unwrap();
        let read = gitcomet_core::history_perf::count(Work::LogWalkObjectRead);
        drop(capture);
        // Records followed one by one first, all of them, or none: the
        // rows come out numbered alike.
        for bulk in [1, 37, usize::MAX] {
            let scanned =
                graph_scan::scan_in_chunks(&store, &graph, &tips, &never, 64, bulk).unwrap();
            assert_eq!(scanned, from_graph, "seed {seed}, bulk after {bulk}");
        }
        let from_graph = walked(Topology::ordered(20, from_graph, &never).unwrap());
        let from_objects = scan_objects(&store, &tips, &never).unwrap();
        let from_objects = walked(Topology::ordered(20, from_objects, &never).unwrap());
        assert_eq!(from_graph, from_objects, "seed {seed}");

        let reachable = git_stdout(root, &["rev-list", "--count", "--branches"]);
        assert_eq!(
            from_graph.len().to_string(),
            reachable.trim(),
            "seed {seed}"
        );
        let many = from_graph
            .iter()
            .filter(|(_, parents, _)| parents.len() > 2)
            .count();
        assert!(
            many > 5,
            "seed {seed}: {many} commits of three parents or more"
        );
        // Only the commits newer than the graph are read as objects.
        assert!((1..=40).contains(&read), "seed {seed}: {read} objects read");
    }
}

/// `count` commits in a line on `tip0`, through `git fast-import`.
fn import_line(root: &std::path::Path, count: u64) {
    let mut stream = String::new();
    for commit in 1..=count {
        let time = 1_700_000_000 + commit;
        stream.push_str(&format!(
            "commit refs/heads/tip0\nmark :{commit}\n\
                 committer T <t@example.invalid> {time} +0000\ndata 2\nc\n"
        ));
        if commit > 1 {
            stream.push_str(&format!("from :{}\n", commit - 1));
        }
        stream.push('\n');
    }
    crate::repo::log::tests::git_with_stdin(root, &["fast-import", "--quiet"], stream.as_bytes());
}

/// A commit no branch shares history with.
fn orphan(root: &std::path::Path) -> gix::ObjectId {
    use crate::test_support::git_stdout;
    let tree = git_stdout(root, &["mktree"]);
    let id = git_stdout(root, &["commit-tree", "-m", "orphan", tree.trim()]);
    gix::ObjectId::from_hex(id.trim().as_bytes()).unwrap()
}

fn head(root: &std::path::Path) -> gix::ObjectId {
    let id = crate::test_support::git_stdout(root, &["rev-parse", "tip0"]);
    gix::ObjectId::from_hex(id.trim().as_bytes()).unwrap()
}

/// What scanning from `tips` read, as `work` counts it.
fn scanned_reads(
    tips: &[gix::ObjectId],
    work: Work,
    scan: impl FnOnce(&[gix::ObjectId]) -> Scanned,
) -> (usize, u64) {
    let _capture = gitcomet_core::history_perf::capture();
    let rows = scan(tips).degrees.len();
    (rows, gitcomet_core::history_perf::count(work))
}

/// A short branch of a long history reads its own few records, and a
/// long walk the whole graph at once.
#[test]
fn the_commit_graph_is_read_in_bulk_only_for_a_long_walk() {
    use crate::test_support::{git_success, init_test_repo};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    import_line(root, 6_000);
    let orphan = orphan(root);
    git_success(
        root,
        &["update-ref", "refs/heads/orphan", &orphan.to_string()],
    );
    git_success(root, &["commit-graph", "write", "--reachable"]);
    let store = gix::open(root).unwrap().into_sync();
    let graph = store
        .to_thread_local()
        .commit_graph_if_enabled()
        .unwrap()
        .unwrap();
    let never = || false;
    let scan = |tips: &[gix::ObjectId]| graph_scan::scan(&store, &graph, tips, &never).unwrap();
    assert_eq!(
        scanned_reads(&[orphan], Work::CommitGraphRead, scan),
        (1, 0)
    );
    assert_eq!(
        scanned_reads(&[head(root)], Work::CommitGraphRead, scan),
        (6_000, 1)
    );
}

/// The same for the packs' commits without a commit-graph.
#[test]
fn packed_commits_are_read_ahead_only_for_a_long_walk() {
    use crate::test_support::{git_success, init_test_repo};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    import_line(root, 9_000);
    // Commits first, as Git packs a clone.
    git_success(root, &["repack", "-adq"]);
    let orphan = orphan(root);
    let store = gix::open(root).unwrap().into_sync();
    let never = || false;
    let scan = |tips: &[gix::ObjectId]| scan_objects(&store, tips, &never).unwrap();
    assert_eq!(
        scanned_reads(&[orphan], Work::LogHeaderReadAhead, scan),
        (1, 0)
    );
    assert_eq!(
        scanned_reads(&[head(root)], Work::LogHeaderReadAhead, scan),
        (9_000, 9_000)
    );

    // Read ahead at once, later, or never: the rows come out alike.
    let tips = [head(root), orphan];
    let alike = scan_objects_reading_ahead_after(&store, &tips, &never, usize::MAX).unwrap();
    for read in [0, 1, 4_096] {
        let scanned = scan_objects_reading_ahead_after(&store, &tips, &never, read).unwrap();
        assert_eq!(scanned, alike, "read ahead after {read}");
    }
}

/// Asked to stop, the read-ahead opens no pack index: opening a large
/// one alone checks a gigabyte of offsets.
#[test]
fn a_cancelled_read_ahead_opens_no_pack_index() {
    use super::super::read_ahead::INDEXES_OPENED;
    use crate::test_support::{git_success, init_test_repo};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_test_repo(root);
    import_line(root, 10);
    git_success(root, &["repack", "-adq"]);
    let store = gix::open(root).unwrap().into_sync();
    let opened = |cancelled: bool| {
        INDEXES_OPENED.set(0);
        let _ = PackedCommits::read(&store, &|| cancelled);
        INDEXES_OPENED.get()
    };
    assert_eq!(opened(false), 1);
    assert_eq!(opened(true), 0);
}

fn tips() -> Vec<gix::ObjectId> {
    vec![gix::ObjectId::null(gix::hash::Kind::Sha1)]
}

/// Counts itself, then polls for cancellation as the row loop does until
/// `gate` opens.
fn build(
    builds: &Arc<AtomicUsize>,
    started: Option<mpsc::Sender<()>>,
    gate: Option<mpsc::Receiver<()>>,
) -> Build {
    let builds = builds.clone();
    Box::new(move |cancelled, _| {
        builds.fetch_add(1, SeqCst);
        if let Some(started) = started {
            started.send(()).unwrap();
        }
        while let Some(gate) = &gate {
            check_cancelled(cancelled())?;
            if gate.recv_timeout(Duration::from_millis(1)) != Err(mpsc::RecvTimeoutError::Timeout) {
                break;
            }
        }
        Ok(Topology {
            hash_len: 20,
            ids: [].into(),
            offsets: [0].into(),
            parents: [].into(),
            times: [].into(),
            order: [].into(),
            sorted: Default::default(),
        })
    })
}

fn wait_for_subscribers(cache: &TopologyCache, count: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !cache
        .state
        .lock()
        .unwrap()
        .build
        .as_ref()
        .is_some_and(|flight| flight.subscribers.load(SeqCst) == count)
    {
        assert!(std::time::Instant::now() < deadline, "no subscription");
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct Shared {
    builds: Arc<AtomicUsize>,
    cache: Arc<TopologyCache>,
    tokens: [CancellationToken; 2],
    release: mpsc::Sender<()>,
    builder: std::thread::JoinHandle<gix::Result<Arc<Topology>>>,
    waiter: mpsc::Receiver<gix::Result<Arc<Topology>>>,
}

/// `build`, ending as a commit gone missing would end it.
fn failing(build: Build) -> Build {
    Box::new(move |cancelled, previous| {
        build(cancelled, previous)?;
        let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "a commit is missing");
        Err(gix::Error::from_error(missing))
    })
}

/// A gated builder with one subscribed waiter for the same tips.
fn flight() -> Shared {
    flight_with(|build| build)
}

/// [`flight`] with the builder's build passed through `wrap`.
fn flight_with(wrap: fn(Build) -> Build) -> Shared {
    let builds = Arc::new(AtomicUsize::new(0));
    let cache = Arc::new(TopologyCache::default());
    let tokens = [CancellationToken::new(), CancellationToken::new()];
    let (started_tx, started) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (done_tx, waiter) = mpsc::channel();
    let request = |token: &CancellationToken, build: Build| {
        let (cache, token) = (cache.clone(), token.clone());
        move || cache.get_or_build(&tips(), &LogWalkCancellation::new(Some(&token)), 1, build)
    };
    let builder = std::thread::spawn(request(
        &tokens[0],
        wrap(build(&builds, Some(started_tx), Some(gate))),
    ));
    started.recv().unwrap();
    let wait = request(&tokens[1], build(&builds, None, None));
    std::thread::spawn(move || done_tx.send(wait()));
    wait_for_subscribers(&cache, 2);
    Shared {
        builds,
        cache,
        tokens,
        release,
        builder,
        waiter,
    }
}

fn is_cancelled(result: gix::Result<Arc<Topology>>) -> bool {
    result.is_err_and(|error| error.to_string().contains("cancelled"))
}

#[test]
fn a_waiter_receives_the_single_build() {
    let flight = flight();
    flight.release.send(()).unwrap();
    let built = flight.builder.join().unwrap().unwrap();
    let waited = flight.waiter.recv().unwrap().unwrap();
    assert!(Arc::ptr_eq(&built, &waited));
    assert_eq!(flight.builds.load(SeqCst), 1);
}

/// A shared index build cancelled meanwhile reaches a requester that is
/// still waiting: as a cancellation, not as a failure to show.
#[test]
fn cancelled_index_tables_report_a_cancellation() {
    let walk = TopologyWalk {
        topology: Arc::new(Topology {
            hash_len: 20,
            ids: [].into(),
            offsets: [0].into(),
            parents: [].into(),
            times: [].into(),
            order: [].into(),
            sorted: Default::default(),
        }),
        next: 0,
    };
    let token = CancellationToken::new();
    token.cancel();
    let error = walk.index_tables(&token).err().expect("cancelled");
    assert!(matches!(
        error.kind(),
        gitcomet_core::error::ErrorKind::Cancelled
    ));
}

/// A store invalidation clears the cache under the store's own locks, so
/// clearing must not wait for a build; the build still serves its
/// subscriber, and caches nothing.
#[test]
fn clearing_the_cache_leaves_a_build_to_its_subscribers() {
    let flight = flight();
    let (cleared_tx, cleared) = mpsc::channel();
    let cache = flight.cache.clone();
    std::thread::spawn(move || {
        cache.clear();
        cleared_tx.send(()).unwrap();
    });
    cleared
        .recv_timeout(Duration::from_secs(2))
        .expect("clearing must not wait for the build");
    flight.release.send(()).unwrap();
    let built = flight.builder.join().unwrap().unwrap();
    let waited = flight.waiter.recv().unwrap().unwrap();
    assert!(Arc::ptr_eq(&built, &waited));
    assert_eq!(flight.builds.load(SeqCst), 1);
    let later = build(&flight.builds, None, None);
    flight
        .cache
        .get_or_build(&tips(), &LogWalkCancellation::default(), 1, later)
        .unwrap();
    assert_eq!(flight.builds.load(SeqCst), 2, "nothing was cached");
}

/// Its subscriber gets a failed build's error, classified as gix
/// classified it, instead of building again.
#[test]
fn a_failed_build_fails_its_subscriber_as_it_failed() {
    let flight = flight_with(failing);
    flight.release.send(()).unwrap();
    let not_found = |result: gix::Result<Arc<Topology>>| result.is_err_and(|e| e.is_not_found());
    assert!(not_found(flight.builder.join().unwrap()));
    assert!(not_found(flight.waiter.recv().unwrap()));
    assert_eq!(flight.builds.load(SeqCst), 1);
}

#[test]
fn a_cancelled_waiter_leaves_while_the_build_completes() {
    let flight = flight();
    flight.tokens[1].cancel();
    let waited = flight
        .waiter
        .recv_timeout(Duration::from_secs(2))
        .expect("a cancelled waiter must not wait for the build");
    assert!(is_cancelled(waited));
    flight.release.send(()).unwrap();
    flight.builder.join().unwrap().unwrap();
    assert_eq!(flight.builds.load(SeqCst), 1);
}

#[test]
fn a_cancelled_builder_finishes_for_the_remaining_subscriber() {
    let flight = flight();
    flight.tokens[0].cancel();
    // The build is still polling for cancellation here and must not see it.
    std::thread::sleep(Duration::from_millis(20));
    let _ = flight.release.send(());
    assert!(is_cancelled(flight.builder.join().unwrap()));
    flight.waiter.recv().unwrap().unwrap();
    assert_eq!(
        flight.builds.load(SeqCst),
        1,
        "the subscriber must not restart the build"
    );
}

#[test]
fn a_build_stops_with_its_last_subscriber() {
    let flight = flight();
    flight.tokens[1].cancel();
    assert!(is_cancelled(flight.waiter.recv().unwrap()));
    flight.tokens[0].cancel();
    // The gate stays closed: only cancellation can end this build.
    assert!(is_cancelled(flight.builder.join().unwrap()));
    let later = build(&flight.builds, None, None);
    flight
        .cache
        .get_or_build(&tips(), &LogWalkCancellation::default(), 1, later)
        .unwrap();
    assert_eq!(
        flight.builds.load(SeqCst),
        2,
        "an abandoned build is not cached"
    );
}
