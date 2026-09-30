//! Lives in its own test binary because it enables the process-global
//! operation trace, which switches every store dispatch to the traced path.

use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::op_trace::{self, Record, Stage};
use gitcomet_core::process::{GitExecutablePreference, install_git_executable_preference};
use gitcomet_core::services::{GitBackend, GitRepository};
use gitcomet_state::msg::Msg;
use gitcomet_state::store::AppStore;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct FailingBackend;

impl GitBackend for FailingBackend {
    fn open(&self, _path: &Path) -> Result<Arc<dyn GitRepository>, Error> {
        Err(Error::new(ErrorKind::Unsupported("op trace test backend")))
    }
}

/// The live UI driver attributes store work to the input that caused it. An
/// opened repository runs through the store worker, a repo-load task and the
/// message that task sends back; all of it must carry the input's operation.
#[test]
fn a_traced_dispatch_is_followed_through_the_store_worker_and_its_tasks() {
    // The reducer drops Git-backed messages until a Git runtime is known.
    let runtime = install_git_executable_preference(GitExecutablePreference::SystemPath);
    assert!(runtime.is_available(), "git on PATH: {runtime:?}");
    op_trace::enable(Instant::now());
    let (store, _events) = AppStore::new(Arc::new(FailingBackend));
    let (_, before) = store.snapshot_with_publication();

    // An existing directory passes the reducer's checks and reaches the
    // repo-load pool, where the backend refuses it.
    let directory = std::env::temp_dir().join(format!("gitcomet-op-trace-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create test directory");
    let op = op_trace::next_op();
    {
        let _scope = op_trace::scope(op);
        store.dispatch(Msg::OpenRepo(directory.clone()));
    }

    let mut records: Vec<Record> = Vec::new();
    let mut threads = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let stages = |records: &[Record]| {
        records
            .iter()
            .filter(|record| record.op == op)
            .map(|record| (record.stage, record.label))
            .collect::<Vec<_>>()
    };
    loop {
        let drained = op_trace::drain();
        records.extend(drained.records);
        threads.extend(drained.threads);
        let seen = stages(&records);
        let received = seen
            .iter()
            .filter(|(stage, _)| *stage == Stage::Received)
            .count();
        if received >= 2 && seen.contains(&(Stage::TaskFinished, "OpenRepo")) {
            break;
        }
        assert!(Instant::now() < deadline, "incomplete trace: {seen:?}");
        std::thread::sleep(Duration::from_millis(5));
    }

    let seen = stages(&records);
    for expected in [
        (Stage::Dispatch, "OpenRepo"),
        (Stage::Received, "OpenRepo"),
        (Stage::Reduced, "reduce"),
        (Stage::EffectQueued, "OpenRepo"),
        // Tasks are named after the effect that spawned them.
        (Stage::TaskStarted, "OpenRepo"),
        (Stage::TaskFinished, "OpenRepo"),
    ] {
        assert!(seen.contains(&expected), "missing {expected:?} in {seen:?}");
    }
    // The failed open reports back as a message of the same operation,
    // labelled by its result rather than the load envelope it travels in.
    let follow_up = records
        .iter()
        .filter(|record| record.op == op && record.stage == Stage::Received)
        .nth(1)
        .expect("the load task's result message");
    assert_eq!(follow_up.label, "RepoOpenedErr");

    let reduced: Vec<u64> = records
        .iter()
        .filter(|record| record.op == op && record.stage == Stage::Reduced)
        .map(|record| record.b)
        .collect();
    let (_, after) = store.snapshot_with_publication();
    assert!(reduced.iter().all(|&seq| seq > before && seq <= after));

    let names: Vec<_> = threads.iter().map(|thread| thread.name.as_str()).collect();
    assert!(names.contains(&"gitcomet-store"), "{names:?}");
    // Either repo-load worker may take the task.
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("gitcomet-repo-load-")),
        "{names:?}"
    );
    let _ = std::fs::remove_dir(&directory);
}
