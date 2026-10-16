use super::*;
use gitcomet_core::filesystem::{Operation, Request};

fn run_and_wait(store: &AppStore, operation: Operation) {
    let request = Request::new(operation);
    let id = request.id;
    store.dispatch(Msg::FilesystemRequest(request));
    super::worktree_redirect::wait_until("the filesystem operation", || {
        store
            .snapshot()
            .filesystem
            .completed
            .iter()
            .any(|result| result.id == id)
    });
}

/// UI tests run concurrently, each with its own store; one test's undo must
/// never reach into another's history.
#[test]
fn test_stores_undo_only_their_own_operations() {
    let directory = tempfile::tempdir().unwrap();
    let mine = directory.path().join("mine.txt");
    let theirs = directory.path().join("theirs.txt");
    let (store, _events) = AppStore::new_test(Arc::new(FailingBackend));
    let (other, _other_events) = AppStore::new_test(Arc::new(FailingBackend));

    run_and_wait(&store, Operation::CreateFile { path: mine.clone() });
    run_and_wait(
        &other,
        Operation::CreateFile {
            path: theirs.clone(),
        },
    );
    run_and_wait(&store, Operation::Undo);

    assert!(!mine.exists(), "undo reverts the store's own file");
    assert!(theirs.exists(), "another store's file is untouched");
}

/// Production windows each have a store, and undo spans them all.
#[test]
fn window_stores_share_one_undo_history() {
    let window = || {
        AppStore::with_initial_state(
            Arc::new(FailingBackend),
            AppState::test_default(),
            crate::store::repository_preferences::PreferenceHub::new(None),
            FilesystemScope::Process,
        )
    };
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    let (store, _events) = window();
    let (other, _other_events) = window();
    assert!(std::ptr::eq(store.filesystem(), other.filesystem()));

    run_and_wait(
        &store,
        Operation::CreateFile {
            path: first.clone(),
        },
    );
    run_and_wait(
        &other,
        Operation::CreateFile {
            path: second.clone(),
        },
    );
    run_and_wait(&store, Operation::Undo);

    assert!(first.exists(), "the first window's file stays");
    assert!(
        !second.exists(),
        "undo reverts the newest operation in any window"
    );
}
