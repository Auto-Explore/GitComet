use super::repository_registry::subscribers;
use super::worker_channel::StoreWorkerCommand;
use crate::msg::{InternalMsg, Msg};
use gitcomet_core::domain::BareModeDisabled;
use gitcomet_core::error::{Error, ErrorKind};
use std::path::Path;
use std::sync::mpsc;

pub(super) fn quiesce(
    common_dir: &Path,
    cancellation: &gitcomet_core::services::CancellationToken,
) -> Result<(), Error> {
    let request_cancellation =
        gitcomet_core::services::CancellationToken::new().with_parent(cancellation.clone());
    let mut acknowledgements = Vec::new();
    for (_, sender) in subscribers() {
        let (reply, acknowledgement) = mpsc::channel();
        if sender
            .send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
                InternalMsg::QuiesceRepository {
                    common_dir: common_dir.to_path_buf(),
                    cancellation: request_cancellation.clone(),
                    reply,
                },
            ))))
            .is_ok()
        {
            acknowledgements.push(acknowledgement);
        }
    }
    let result = (|| {
        for acknowledgement in acknowledgements {
            loop {
                cancellation.check_cancelled()?;
                match acknowledgement.recv_timeout(std::time::Duration::from_millis(50)) {
                    Ok(()) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(error @ mpsc::RecvTimeoutError::Disconnected) => return Err(Error::new(ErrorKind::Backend("A repository window closed before conversion could pause its work. Retry disabling bare mode.".into())).with_source(error)),
                }
            }
        }
        Ok(())
    })();
    request_cancellation.cancel();
    result
}

pub(super) fn finished(
    common_dir: &Path,
    result: &Result<BareModeDisabled, Error>,
    source_store: u64,
) {
    for (id, sender) in subscribers() {
        if id == source_store {
            continue;
        }
        let _ = sender.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
            InternalMsg::BareModeDisabled {
                common_dir: common_dir.to_path_buf(),
                result: result.clone(),
            },
        ))));
    }
}
