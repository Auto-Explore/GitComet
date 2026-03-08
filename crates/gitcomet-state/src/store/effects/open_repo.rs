use crate::msg::Msg;
use gitcomet_core::domain::RepoSpec;
use gitcomet_core::services::GitBackend;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use super::super::{RepoId, executor::TaskExecutor};
use super::util::send_or_log;

pub(super) fn schedule_open_repo(
    executor: &TaskExecutor,
    backend: Arc<dyn GitBackend>,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
) {
    executor.spawn(move || {
        let spec = RepoSpec { workdir: path };
        match backend.open(&spec.workdir) {
            Ok(repo) => {
                send_or_log(
                    &msg_tx,
                    Msg::RepoOpenedOk {
                        repo_id,
                        spec,
                        repo,
                    },
                );
            }
            Err(error) => {
                send_or_log(
                    &msg_tx,
                    Msg::RepoOpenedErr {
                        repo_id,
                        spec,
                        error,
                    },
                );
            }
        }
    });
}
