//! File operations through `reduce`.

use super::*;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::filesystem::{Operation, Request};
use std::sync::atomic::AtomicU64;

#[test]
fn a_pending_operation_is_tracked_without_copying_its_request() {
    let mut repos = RepositoryRuntimes::default();
    let ids = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let contents: Arc<[u8]> = Arc::from(&b"saved"[..]);
    let request = Request::new(Operation::Save {
        path: PathBuf::from("/tmp/saved.txt"),
        worktree: None,
        contents: Arc::clone(&contents),
        expected: None,
        overwrite: true,
    });
    let id = request.id;

    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::FilesystemRequest(request),
    );

    assert!(matches!(&effects[..], [Effect::Filesystem(sent)] if sent.id == id));
    assert_eq!(state.filesystem.pending.len(), 1);
    // Every store dispatch deep-copies AppState, so a request kept in it
    // (with a transfer's whole source list) is copied once per message.
    assert_eq!(
        Arc::strong_count(&contents),
        2,
        "only this test and the effect hold the request"
    );
}
