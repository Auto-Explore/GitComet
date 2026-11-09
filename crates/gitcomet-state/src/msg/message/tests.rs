use super::{InternalMsg, Msg, RepoActionKind};
use crate::model::RepoId;
use gitcomet_core::error::{Error, ErrorKind};
use std::path::PathBuf;

#[test]
fn wraps_internal_messages() {
    let msg: Msg = InternalMsg::RepoActionFinished {
        repo_id: RepoId(7),
        action: RepoActionKind::CheckoutBranch,
        result: Ok(()),
    }
    .into();

    assert!(matches!(
        msg,
        Msg::Internal(InternalMsg::RepoActionFinished {
            repo_id: RepoId(7),
            action: RepoActionKind::CheckoutBranch,
            result: Ok(())
        })
    ));
}

#[test]
fn clone_repo_finished_debug_keeps_result_compact() {
    let msg: Msg = InternalMsg::CloneRepoFinished {
        url: "https://example.invalid/repo.git".to_string(),
        dest: PathBuf::from("/tmp/repo"),
        result: Err(Error::new(ErrorKind::Backend("clone failed".to_string()))),
    }
    .into();
    let debug = format!("{msg:?}");

    assert!(debug.contains("CloneRepoFinished"));
    assert!(debug.contains("ok: false"));
    assert!(!debug.contains("clone failed"));
}
