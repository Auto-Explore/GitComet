use super::util;
use crate::model::AppState;
use crate::msg::{Msg, PersistenceEffect};
use crate::session;
use crate::store::{executor::TaskExecutor, worker_channel::StoreWorkerSender};
use std::sync::{Arc, RwLock};

pub(super) fn schedule(
    session_persist_executor: &TaskExecutor,
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    msg_tx: StoreWorkerSender,
    work: PersistenceEffect,
) {
    let failure = work.failure();
    match work {
        PersistenceEffect::UpdateRepositoryPreferences { key, update, .. } => {
            session_persist_executor.spawn(move || {
                if let Err(error) = msg_tx.preferences.update(key, &update) {
                    util::send_or_log(&msg_tx, failure.message(error));
                }
            });
        }
        PersistenceEffect::PersistSession { .. } => {
            let Some(session_file_path) = session::default_session_file_path_for_effect() else {
                return;
            };
            let state_snapshot = {
                let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
                Arc::clone(&state)
            };
            session_persist_executor.spawn(move || {
                if let Err(error) =
                    session::persist_from_state_to_path(&state_snapshot, &session_file_path)
                {
                    util::send_or_log(&msg_tx, failure.message(error));
                }
            });
        }
        PersistenceEffect::PersistRecentRepo { workdir, .. } => {
            let Some(session_file_path) = session::default_session_file_path_for_effect() else {
                return;
            };
            session_persist_executor.spawn(move || {
                if let Err(error) =
                    session::persist_recent_repo_to_path(&workdir, &session_file_path)
                {
                    util::send_or_log(&msg_tx, failure.message(error));
                }
            });
        }
        PersistenceEffect::PersistRepoHistoryMode {
            repo_id,
            workdir,
            mode,
            ..
        } => {
            // Opening defaults cannot precede shared-identity migration. Real
            // opened repositories use the shared preference writer instead.
            if repo_id.is_some_and(|id| {
                let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
                state
                    .repos
                    .iter()
                    .find(|repo| repo.id == id)
                    .is_some_and(|repo| {
                        repo.shared_preferences.is_some()
                            || !matches!(repo.open, crate::model::Loadable::Ready(()))
                    })
            }) {
                return;
            }
            let Some(session_file_path) = session::default_session_file_path_for_effect() else {
                return;
            };
            session_persist_executor.spawn(move || {
                if let Err(error) =
                    session::persist_repo_history_mode_to_path(&workdir, mode, &session_file_path)
                {
                    util::send_or_log(&msg_tx, failure.message(error));
                }
            });
        }
        PersistenceEffect::PersistRepoHistoryModesBatch { updates, .. } => {
            let updates = {
                let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
                updates
                    .into_iter()
                    .filter(|(path, _)| {
                        !state.repos.iter().any(|repo| {
                            repo.spec.workdir == *path
                                && (repo.shared_preferences.is_some()
                                    || !matches!(repo.open, crate::model::Loadable::Ready(())))
                        })
                    })
                    .collect::<Vec<_>>()
            };
            if updates.is_empty() {
                return;
            }
            let Some(session_file_path) = session::default_session_file_path_for_effect() else {
                return;
            };
            session_persist_executor.spawn(move || {
                if let Err(error) =
                    session::persist_repo_history_modes_batch_to_path(&updates, &session_file_path)
                {
                    util::send_or_log(&msg_tx, failure.message(error));
                }
            });
        }
        PersistenceEffect::PersistRepoHistoryAuthorFilter {
            workdir, author, ..
        } => {
            let Some(session_file_path) = session::default_session_file_path_for_effect() else {
                return;
            };
            session_persist_executor.spawn(move || {
                if let Err(error) = session::persist_repo_history_author_filter_to_path(
                    &workdir,
                    author.as_deref(),
                    &session_file_path,
                ) {
                    util::send_or_log(&msg_tx, failure.message(error));
                }
            });
        }
        PersistenceEffect::PersistRepoMaintenanceSnooze { common_dir } => {
            session_persist_executor.spawn(move || {
                if let Err(error) = session::persist_repo_maintenance_snooze(&common_dir) {
                    util::send_or_log(&msg_tx, failure.message(error));
                }
            });
        }
    }
}

struct PersistenceFailure {
    repo_id: Option<crate::model::RepoId>,
    action: &'static str,
}

impl PersistenceFailure {
    fn message(self, error: impl std::fmt::Display) -> Msg {
        Msg::Internal(crate::msg::InternalMsg::SessionPersistFailed {
            repo_id: self.repo_id,
            action: self.action,
            error: error.to_string(),
        })
    }
}

impl PersistenceEffect {
    fn failure(&self) -> PersistenceFailure {
        match self {
            Self::UpdateRepositoryPreferences { repo_id, .. } => PersistenceFailure {
                repo_id: Some(*repo_id),
                action: "updating repository preferences",
            },
            Self::PersistSession { repo_id, action }
            | Self::PersistRecentRepo {
                repo_id, action, ..
            }
            | Self::PersistRepoHistoryMode {
                repo_id, action, ..
            }
            | Self::PersistRepoHistoryModesBatch {
                repo_id, action, ..
            }
            | Self::PersistRepoHistoryAuthorFilter {
                repo_id, action, ..
            } => PersistenceFailure {
                repo_id: *repo_id,
                action,
            },
            Self::PersistRepoMaintenanceSnooze { .. } => PersistenceFailure {
                repo_id: None,
                action: "remember the maintenance reminder",
            },
        }
    }
}
