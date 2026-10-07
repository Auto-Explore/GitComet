//! Production operations for the opt-in scenario driver. No backend shortcuts.
use super::*;
use gitcomet_core::git_operation::GitOperationId;
use gitcomet_core::large_files::LargeFileCommand;
use gitcomet_core::services::PullMode;
use gitcomet_state::model::{CloneOpStatus, GitHookOperationStatus, RepoId};
use gitcomet_state::msg::FetchMsg;
use std::path::Path;
use std::time::SystemTime;

pub(super) struct Operation {
    op: u64,
    name: String,
    label: &'static str,
    repo: RepoId,
    before: Vec<GitOperationId>,
    id: Option<GitOperationId>,
    destination: Option<PathBuf>,
    started: Instant,
    started_wall: SystemTime,
    cancel_at: Option<Instant>,
    terminal_reported: bool,
}

impl Driver {
    pub(super) async fn minimize_activity(&mut self, cx: &mut AsyncApp) -> Result<(), String> {
        let deadline = Instant::now() + DEFAULT_WITNESS_TIMEOUT;
        loop {
            let (open, pending) = cx.update(|cx| {
                let view = self.view.read(cx);
                (
                    view.hook_activity_workflow_is_open(cx),
                    view.pending_hook_activity_open.is_some(),
                )
            });
            let terminal = matches!(self.operation_status(cx), Some(Ok(Some(_))) | Some(Err(_)));
            if open {
                cx.update(|cx| {
                    let host = self.view.read(cx).popover_host.clone();
                    host.update(cx, |host, cx| host.minimize_hook_activity(cx));
                });
                record("scenario_activity", json!({"state": "minimized"}));
            }
            if open || (terminal && !pending) {
                self.sleep(Duration::from_millis(50), cx).await;
                return if cx.update(|cx| self.view.read(cx).hook_activity_workflow_is_open(cx)) {
                    Err("activity dialog still occludes history".into())
                } else {
                    Ok(())
                };
            }
            if Instant::now() >= deadline {
                return Err("activity dialog did not become ready to minimize".into());
            }
            self.sleep(WITNESS_POLL, cx).await;
        }
    }

    pub(super) async fn start_operation(
        &mut self,
        name: &str,
        path: &Path,
        remote: &str,
        url: &str,
        dest: &Path,
        cx: &mut AsyncApp,
    ) -> Result<(), String> {
        if self.operation.is_some() {
            return Err("wait for the previous operation first".into());
        }
        let (store, repo_id, before) = cx
            .update(|cx| {
                let view = self.view.read(cx);
                let repo = view.active_repo()?;
                Some((
                    Arc::clone(&view.store),
                    repo.id,
                    repo.feedback
                        .hook_activity
                        .iter()
                        .map(|item| item.id)
                        .collect(),
                ))
            })
            .ok_or("no active repository")?;
        let paths = if path.as_os_str().is_empty() {
            vec![]
        } else {
            vec![path.to_path_buf()]
        };
        let mut destination = None;
        let large = match name {
            "lfs-fetch" => Some(LargeFileCommand::LfsFetchAll),
            "lfs-pull" => Some(LargeFileCommand::LfsPull {
                paths: paths.clone(),
            }),
            "lfs-push" => Some(LargeFileCommand::LfsPushAll {
                remote: if remote.is_empty() { "origin" } else { remote }.into(),
            }),
            "annex-get" => Some(LargeFileCommand::AnnexGet {
                paths: paths.clone(),
                from: (!remote.is_empty()).then(|| remote.into()),
            }),
            "annex-copy" => Some(LargeFileCommand::AnnexCopy {
                paths: paths.clone(),
                to: remote.into(),
            }),
            "annex-pull" => Some(LargeFileCommand::AnnexPull { content: true }),
            "annex-push" => Some(LargeFileCommand::AnnexPush { content: true }),
            "annex-sync" => Some(LargeFileCommand::AnnexSync { content: true }),
            _ => None,
        };
        let label = large.as_ref().map_or_else(
            || match name {
                "fetch" => "Fetch",
                "pull" | "pull-merge" | "pull-rebase" => "Pull",
                "push" => "Push",
                _ => "Clone",
            },
            LargeFileCommand::label,
        );
        let msg = if let Some(command) = large {
            Msg::RunLargeFileCommand { repo_id, command }
        } else {
            match name {
                "fetch" => Msg::Fetch(FetchMsg::All { repo_id }),
                "pull" | "pull-merge" => Msg::Pull {
                    repo_id,
                    mode: PullMode::Merge,
                },
                "pull-rebase" => Msg::Pull {
                    repo_id,
                    mode: PullMode::Rebase,
                },
                "push" => Msg::Push { repo_id },
                "clone" => {
                    if url.is_empty() || !dest.is_absolute() || dest.exists() {
                        return Err("clone needs a URL and a new absolute destination".into());
                    }
                    destination = Some(dest.to_path_buf());
                    Msg::CloneRepo {
                        url: url.into(),
                        dest: dest.to_path_buf(),
                    }
                }
                _ => return Err(format!("unknown scenario operation: {name}")),
            }
        };
        let op = op_trace::next_op();
        self.operation = Some(Operation {
            op,
            name: name.into(),
            label,
            repo: repo_id,
            before,
            id: None,
            destination,
            started: Instant::now(),
            started_wall: SystemTime::now(),
            cancel_at: None,
            terminal_reported: false,
        });
        record(
            "scenario_operation",
            json!({"op": op, "name": name, "state": "started"}),
        );
        {
            let _scope = op_trace::scope(op);
            store.dispatch(msg);
        }
        // Prove acceptance before claiming subsequent gestures overlapped it.
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if self.operation_status(cx).is_some() {
                record(
                    "scenario_operation",
                    json!({"op": op, "name": name, "state": "accepted"}),
                );
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("{name} was not accepted"));
            }
            self.sleep(WITNESS_POLL, cx).await;
        }
    }

    /// None means the store has not applied the start yet. Ok(None) is active.
    pub(super) fn operation_status(
        &mut self,
        cx: &mut AsyncApp,
    ) -> Option<Result<Option<bool>, String>> {
        let operation = self.operation.as_mut()?;
        let status = cx.update(|cx| {
            let state = &self.view.read(cx).state;
            if let Some(dest) = &operation.destination {
                let clone = state
                    .clone
                    .as_ref()
                    .filter(|item| item.dest.as_ref() == dest)?;
                return Some(match &clone.status {
                    CloneOpStatus::Running | CloneOpStatus::Cancelling => Ok(None),
                    CloneOpStatus::FinishedOk => Ok(Some(false)),
                    CloneOpStatus::Cancelled => Ok(Some(true)),
                    CloneOpStatus::FinishedErr(error) => Err(error.clone()),
                });
            }
            let repo = state.repos.iter().find(|repo| repo.id == operation.repo)?;
            let item = repo.feedback.hook_activity.iter().find(|item| {
                item.label == operation.label
                    && operation
                        .id
                        .map_or_else(|| !operation.before.contains(&item.id), |id| item.id == id)
            });
            // Silent commands can discard their transient hook activity on
            // completion. Their user-visible command log still witnesses the
            // result; never interpret mere disappearance as success.
            let Some(item) = item else {
                return repo.feedback.command_log.iter().rev().find_map(|entry| {
                    completion_from_log(
                        entry,
                        operation.label,
                        operation.started_wall,
                        operation.cancel_at.is_some(),
                    )
                });
            };
            operation.id = Some(item.id);
            Some(match item.status {
                GitHookOperationStatus::Running | GitHookOperationStatus::Cancelling => Ok(None),
                GitHookOperationStatus::Succeeded => Ok(Some(false)),
                GitHookOperationStatus::Cancelled => Ok(Some(true)),
                status => Err(format!("{status:?}: {}", item.latest_line)),
            })
        });
        if !operation.terminal_reported && matches!(status, Some(Ok(Some(_))) | Some(Err(_))) {
            operation.terminal_reported = true;
            record(
                "scenario_operation",
                json!({"op": operation.op, "name": operation.name,
                "state": "finished", "cancelled": matches!(status, Some(Ok(Some(true)))),
                "error": status.as_ref().and_then(|s| s.as_ref().err()),
                "milliseconds": operation.started.elapsed().as_secs_f64() * 1000.0,
                "cancel_ms": operation.cancel_at.map(|at| at.elapsed().as_secs_f64() * 1000.0)}),
            );
        }
        status
    }

    pub(super) fn cancel_operation(&mut self, cx: &mut AsyncApp) -> Result<(), String> {
        let operation = self.operation.as_mut().ok_or("no operation to cancel")?;
        let store = cx.update(|cx| Arc::clone(&self.view.read(cx).store));
        let msg = if let Some(dest) = &operation.destination {
            Msg::AbortCloneRepo { dest: dest.clone() }
        } else {
            Msg::CancelGitOperation {
                repo_id: operation.repo,
                operation_id: operation.id.ok_or("operation has no activity id")?,
            }
        };
        operation.cancel_at = Some(Instant::now());
        record(
            "scenario_operation",
            json!({"op": operation.op, "name": operation.name, "state": "cancel_requested"}),
        );
        let _scope = op_trace::scope(operation.op);
        store.dispatch(msg);
        Ok(())
    }

    pub(super) async fn wait_operation(
        &mut self,
        timeout_ms: u64,
        cancelled: bool,
        cx: &mut AsyncApp,
    ) -> Result<(), String> {
        if self.operation.is_none() {
            return Err("no operation to witness".into());
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            match self.operation_status(cx) {
                Some(Ok(Some(was_cancelled))) => {
                    self.operation.take();
                    return if was_cancelled == cancelled {
                        Ok(())
                    } else {
                        Err("unexpected cancellation outcome".into())
                    };
                }
                Some(Err(error)) => {
                    self.operation.take();
                    return Err(error);
                }
                _ => {}
            }
            if Instant::now() > deadline {
                return Err(format!("operation did not finish within {timeout_ms} ms"));
            }
            self.sleep(WITNESS_POLL, cx).await;
        }
    }
}

fn completion_from_log(
    entry: &gitcomet_state::model::CommandLogEntry,
    label: &str,
    started: SystemTime,
    cancellation_requested: bool,
) -> Option<Result<Option<bool>, String>> {
    if entry.time < started
        || !(entry.summary.starts_with(&format!("{label}:"))
            || entry.summary.starts_with(&format!("{label} failed:")))
    {
        return None;
    }
    Some(if entry.ok {
        Ok(Some(false))
    } else if cancellation_requested && entry.stderr.trim() == "Cancelled" {
        Ok(Some(true))
    } else {
        Err(entry.summary.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_command_completion_needs_a_matching_result() {
        let now = SystemTime::now();
        let mut entry = gitcomet_state::model::CommandLogEntry {
            time: now,
            ok: true,
            command: "git fetch --all".into(),
            summary: "Fetch: Synchronized".into(),
            stdout: "".into(),
            stderr: "".into(),
            announce_success: true,
            hook_operation_id: None,
        };
        assert_eq!(
            completion_from_log(&entry, "Fetch", now, false),
            Some(Ok(Some(false)))
        );
        assert!(completion_from_log(&entry, "Push", now, false).is_none());
        assert!(
            completion_from_log(&entry, "Fetch", now + Duration::from_secs(1), false).is_none()
        );
        entry.ok = false;
        entry.summary = "Fetch failed:\n\nCancelled".into();
        entry.stderr = "Cancelled".into();
        assert_eq!(
            completion_from_log(&entry, "Fetch", now, true),
            Some(Ok(Some(true)))
        );
        entry.stderr = "transport disconnected".into();
        assert!(matches!(
            completion_from_log(&entry, "Fetch", now, true),
            Some(Err(_))
        ));
    }
}
