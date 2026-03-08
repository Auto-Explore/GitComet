use crate::msg::Msg;
use gitcomet_core::services::GitRepository;
use rustc_hash::FxHashMap as HashMap;
use std::sync::{Arc, mpsc};

use super::super::send_diagnostics::{SendFailureKind, send_or_log as send_on_channel_or_log};
use super::super::{RepoId, executor::TaskExecutor};

pub(super) type RepoMap = HashMap<RepoId, Arc<dyn GitRepository>>;

pub(super) fn spawn_with_repo(
    executor: &TaskExecutor,
    repos: &RepoMap,
    repo_id: RepoId,
    msg_tx: mpsc::Sender<Msg>,
    task: impl FnOnce(Arc<dyn GitRepository>, mpsc::Sender<Msg>) + Send + 'static,
) {
    if let Some(repo) = repos.get(&repo_id).cloned() {
        executor.spawn(move || task(repo, msg_tx));
    }
}

pub(super) fn send_or_log(msg_tx: &mpsc::Sender<Msg>, msg: Msg) {
    send_on_channel_or_log(
        msg_tx,
        msg,
        SendFailureKind::EffectMessage,
        "store effect pipeline",
    )
}
