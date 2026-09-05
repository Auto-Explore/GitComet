use super::*;

fn hook_completion_notice(operation: &GitHookOperation) -> (components::ToastKind, String) {
    let failed_hook = operation
        .hooks
        .iter()
        .rev()
        .find(|hook| hook.status == GitHookRunStatus::Failed)
        .map(|hook| hook.name.as_str());
    match operation.status {
        GitHookOperationStatus::Succeeded => (
            components::ToastKind::Success,
            format!("{}: Git hooks passed", operation.label),
        ),
        GitHookOperationStatus::SucceededWithHookFailure => {
            let hook = failed_hook.unwrap_or("post-operation");
            let message = if operation.label == "Commit" && hook == "post-commit" {
                "Commit created, but post-commit hook failed".to_string()
            } else {
                format!("{} completed, but {hook} hook failed", operation.label)
            };
            (components::ToastKind::Warning, message)
        }
        GitHookOperationStatus::Failed => {
            let hook = failed_hook.unwrap_or("Git");
            let message = if operation.label == "Commit" && hook == "pre-commit" {
                "Commit blocked by pre-commit hook".to_string()
            } else {
                format!("{} failed in {hook} hook", operation.label)
            };
            (components::ToastKind::Error, message)
        }
        GitHookOperationStatus::Cancelled => (
            components::ToastKind::Warning,
            format!(
                "{} stopped; repository state was refreshed",
                operation.label
            ),
        ),
        GitHookOperationStatus::TimedOut => (
            components::ToastKind::Error,
            format!("{} timed out while running Git hooks", operation.label),
        ),
        GitHookOperationStatus::Running | GitHookOperationStatus::Cancelling => {
            (components::ToastKind::Success, String::new())
        }
    }
}

fn outer_failure_after_hooks(operation: &GitHookOperation) -> bool {
    operation.status == GitHookOperationStatus::Failed
        && !operation
            .hooks
            .iter()
            .any(|hook| hook.status == GitHookRunStatus::Failed)
}

impl GitCometView {
    pub(super) fn apply_state_snapshot(
        &mut self,
        next: Arc<AppState>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let git_runtime_changed = self.state.git_runtime != next.git_runtime;
        let prev_git_runtime_available = self.state.git_runtime.is_available();
        let prev_had_repos = !self.state.repos.is_empty();
        let prev_banner_error = self.state.banner_error.clone();
        let prev_auth_prompt = self.state.auth_prompt.clone();
        let prev_branch_exists_prompt = self.state.branch_exists_prompt.clone();
        let prev_submodule_trust_prompt = self.state.submodule_trust_prompt.clone();
        let prev_submodule_trust_check = self.state.submodule_trust_check_pending;
        let next_banner_error = next.banner_error.clone();
        let merge_view_active = active_merge_view_target(next.as_ref()).is_some();
        let mut follow_up_msgs = Vec::new();
        let hook_activity_workflow_repo = self
            .hook_activity_workflow_repo_id(cx)
            .or_else(|| self.pending_hook_activity_open.map(|(repo_id, _)| repo_id));

        let old_notification_len = self.state.notifications.len();
        let new_notifications = next
            .notifications
            .iter()
            .skip(old_notification_len.min(next.notifications.len()))
            .cloned()
            .collect::<Vec<_>>();
        for notification in new_notifications {
            let kind = match notification.kind {
                AppNotificationKind::Warning => components::ToastKind::Warning,
                AppNotificationKind::Info | AppNotificationKind::Success => {
                    components::ToastKind::Success
                }
                AppNotificationKind::Error => {
                    self.show_error_banner(None, notification.message);
                    continue;
                }
            };
            self.push_toast(kind, notification.message, cx);
        }

        for next_repo in &next.repos {
            let (old_diag_len, old_cmd_len) = self
                .state
                .repos
                .iter()
                .find(|r| r.id == next_repo.id)
                .map(|r| (r.feedback.diagnostics.len(), r.feedback.command_log.len()))
                .unwrap_or((0, 0));

            let new_diag_messages = next_repo
                .feedback
                .diagnostics
                .iter()
                .skip(old_diag_len.min(next_repo.feedback.diagnostics.len()))
                .filter(|d| d.kind == DiagnosticKind::Error)
                .map(|d| d.message.clone())
                .collect::<Vec<_>>();
            for msg in new_diag_messages {
                if self.pending_force_delete_branch_prompt.is_none()
                    && let Some(name) = parse_force_delete_branch_name(&msg)
                {
                    self.pending_force_delete_branch_prompt = Some((next_repo.id, name));
                }
                self.show_error_banner(Some(next_repo.id), msg);
            }

            let new_command_entries = next_repo
                .feedback
                .command_log
                .iter()
                .skip(old_cmd_len.min(next_repo.feedback.command_log.len()))
                .collect::<Vec<_>>();
            for entry in &new_command_entries {
                if entry.command.starts_with("telemetry.") {
                    continue;
                }

                let force_remove_worktree_path = if entry.ok {
                    None
                } else {
                    parse_force_remove_worktree_path(&entry.command, &entry.stderr)
                };
                if let Some(path) = force_remove_worktree_path.clone() {
                    if self.pending_force_remove_worktree_prompt.is_none() {
                        let branch = self.take_pending_worktree_branch_removal(next_repo.id, &path);
                        self.pending_force_remove_worktree_prompt =
                            Some((next_repo.id, path, branch));
                    }
                    continue;
                }

                let worktree_remove_path = parse_worktree_remove_path_from_command(&entry.command);
                if let Some(path) = worktree_remove_path.as_ref() {
                    if entry.ok {
                        if let Some(branch) =
                            self.take_pending_worktree_branch_removal(next_repo.id, path)
                        {
                            follow_up_msgs.push(Msg::DeleteBranch {
                                repo_id: next_repo.id,
                                name: branch,
                            });
                        }
                    } else {
                        self.take_pending_worktree_branch_removal(next_repo.id, path);
                    }
                }

                if let Some(operation_id) = entry.hook_operation_id {
                    let outer_failure_after_hooks = !entry.ok
                        && next_repo
                            .feedback
                            .hook_activity
                            .iter()
                            .find(|operation| operation.id == operation_id)
                            .is_some_and(outer_failure_after_hooks);
                    if outer_failure_after_hooks {
                        self.show_error_banner(Some(next_repo.id), entry.summary.clone());
                    }
                    continue;
                }

                if entry.ok {
                    if entry.announce_success {
                        self.push_toast(components::ToastKind::Success, entry.summary.clone(), cx);
                    }
                } else {
                    self.show_error_banner(Some(next_repo.id), entry.summary.clone());
                }
            }

            let previous_repo = self.state.repos.iter().find(|repo| repo.id == next_repo.id);
            for operation in next_repo
                .feedback
                .hook_activity
                .iter()
                .filter(|operation| operation.has_hooks() && !operation.status.is_active())
            {
                let was_completed = previous_repo
                    .and_then(|repo| {
                        repo.feedback
                            .hook_activity
                            .iter()
                            .find(|previous| previous.id == operation.id)
                    })
                    .is_some_and(|previous| !previous.status.is_active());
                if was_completed {
                    continue;
                }
                if outer_failure_after_hooks(operation) {
                    // Git can fail after every hook passed (for example while
                    // signing the commit). The ordinary command log owns that
                    // banner because it retains the real Git error detail.
                    continue;
                }
                if hook_activity_workflow_repo == Some(next_repo.id) {
                    // The final state and output are already visible in the
                    // Activity workflow; a second hook notification would
                    // duplicate the same result behind the dialog.
                    continue;
                }
                let (kind, message) = hook_completion_notice(operation);
                self.toast_host.update(cx, |host, cx| {
                    host.push_hook_activity_toast(kind, message, next_repo.id, operation.id, cx);
                });
            }

            if self.pending_pull_reconcile_prompt.is_none()
                && next.active_repo == Some(next_repo.id)
                && new_command_entries.iter().any(|entry| {
                    if entry.ok {
                        return false;
                    }
                    if !entry.command.trim_start().starts_with("git pull") {
                        return false;
                    }

                    let stderr = entry.stderr.as_str();
                    stderr.contains("Need to specify how to reconcile divergent branches")
                        || stderr.contains(
                            "divergent branches and need to specify how to reconcile them",
                        )
                        || stderr.contains("Not possible to fast-forward")
                })
            {
                self.pending_pull_reconcile_prompt = Some(next_repo.id);
            }
        }

        let next_submodule_add_progress = next
            .repos
            .iter()
            .filter_map(|repo| repo.submodule_add_in_flight.clone())
            .collect::<Vec<_>>();
        let next_hook_progress = next
            .repos
            .iter()
            .flat_map(|repo| {
                repo.feedback
                    .hook_activity
                    .iter()
                    .filter(|operation| operation.has_hooks() && operation.status.is_active())
                    .cloned()
                    .map(move |operation| (repo.id, operation))
            })
            .collect::<Vec<_>>();

        let active_hook_chains = next_hook_progress
            .iter()
            .map(|(repo_id, operation)| (*repo_id, operation.id))
            .collect::<FxHashSet<_>>();
        self.minimized_hook_activity_chains
            .retain(|chain| active_hook_chains.contains(chain));
        self.minimized_hook_activity_repos
            .retain(|repo_id| next.repos.iter().any(|repo| repo.id == *repo_id));

        let newly_started_hook_chains = next_hook_progress
            .iter()
            .filter(|(repo_id, operation)| {
                !self
                    .state
                    .repos
                    .iter()
                    .find(|repo| repo.id == *repo_id)
                    .and_then(|repo| {
                        repo.feedback
                            .hook_activity
                            .iter()
                            .find(|previous| previous.id == operation.id)
                    })
                    .is_some_and(|previous| previous.has_hooks() && previous.status.is_active())
            })
            .map(|(repo_id, operation)| (*repo_id, operation.id, operation.time))
            .collect::<Vec<_>>();

        if !newly_started_hook_chains.is_empty() && !self.hook_activity_workflow_is_open(cx) {
            let another_overlay_is_open = self.is_overlay_open(cx) || self.command_palette_open;
            if another_overlay_is_open {
                self.minimized_hook_activity_chains.extend(
                    newly_started_hook_chains
                        .iter()
                        .map(|(repo_id, operation_id, _)| (*repo_id, *operation_id)),
                );
            } else if let Some((repo_id, operation_id, _)) = newly_started_hook_chains
                .into_iter()
                .filter(|(repo_id, operation_id, _)| {
                    !self.minimized_hook_activity_repos.contains(repo_id)
                        && !self
                            .minimized_hook_activity_chains
                            .contains(&(*repo_id, *operation_id))
                })
                .max_by_key(|(_, _, time)| *time)
            {
                self.pending_hook_activity_open = Some((repo_id, operation_id));
            }
        }

        let hook_activity_workflow_repo = self
            .hook_activity_workflow_repo_id(cx)
            .or_else(|| self.pending_hook_activity_open.map(|(repo_id, _)| repo_id));
        self.toast_host.update(cx, |host, cx| {
            host.sync_clone_progress(next.clone.as_ref(), cx);
            host.sync_submodule_add_progress(&next_submodule_add_progress, cx);
            host.sync_hook_progress(next_hook_progress, cx);
            host.set_hook_activity_dialog_repo(hook_activity_workflow_repo, cx);
        });

        #[cfg(target_os = "macos")]
        if self.view_mode == GitCometViewMode::Normal {
            for path in newly_opened_repo_paths(&self.state, next.as_ref()) {
                cx.add_recent_document(&path);
            }
            let recent_repos = session::load().recent_repos;
            if self.recent_repos_menu_fingerprint != recent_repos {
                self.recent_repos_menu_fingerprint = recent_repos;
                crate::app::refresh_macos_app_menus(cx);
            }
        }

        self.state = next;
        self.command_palette.update(cx, |palette, cx| {
            palette.set_has_active_repo(self.state.active_repo.is_some(), cx);
        });
        match (self.sidebar_collapsed_before_merge_view, merge_view_active) {
            (None, true) => {
                self.sidebar_collapsed_before_merge_view = Some(self.sidebar_collapsed);
                self.set_sidebar_collapsed(true, cx);
            }
            (Some(collapsed_before_merge_view), false) => {
                self.sidebar_collapsed_before_merge_view = None;
                self.set_sidebar_collapsed(collapsed_before_merge_view, cx);
            }
            _ => {}
        }
        self.sync_terminal_sessions_with_state(cx);
        self.sync_reflog_panels_with_state();
        if !prev_git_runtime_available && self.state.git_runtime.is_available() {
            self.resume_after_git_runtime_recovery();
        }
        for msg in follow_up_msgs {
            self.store.dispatch(msg);
        }
        if !self.state.repos.is_empty() {
            self.startup_repo_bootstrap_pending = false;
        }
        if prev_auth_prompt != self.state.auth_prompt {
            self.auth_prompt_key = None;
        }
        if prev_branch_exists_prompt != self.state.branch_exists_prompt {
            self.pending_branch_exists_prompt = self.state.branch_exists_prompt.clone();
        }
        if prev_submodule_trust_prompt != self.state.submodule_trust_prompt {
            self.pending_submodule_trust_prompt = self.state.submodule_trust_prompt.clone();
        }
        if prev_submodule_trust_check != self.state.submodule_trust_check_pending {
            // A newly-started check opens the spinner popover on the next render.
            if self.state.submodule_trust_check_pending.is_some() {
                self.pending_submodule_trust_check = self.state.submodule_trust_check_pending;
            } else if let Some(prev) = prev_submodule_trust_check.as_ref()
                && self.state.submodule_trust_prompt.is_none()
            {
                // The check resolved without a prompt (silent proceed or error),
                // so dismiss the spinner popover if it is still the one showing.
                self.close_submodule_trust_spinner(prev.repo_id, cx);
            }
        }
        if prev_had_repos && self.state.repos.is_empty() {
            self.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
            self.open_repo_panel = false;
        }
        self.sync_title_bar_workspace_actions(cx);
        self.drive_focused_mergetool_bootstrap();
        self.drive_submodule_diff_bootstrap();

        crate::app::sync_gitcomet_window_state(
            cx,
            self.window_handle,
            cx.weak_entity(),
            self.main_pane.downgrade(),
            self.view_mode,
            self.state
                .repos
                .iter()
                .map(|repo| repo.spec.workdir.clone())
                .collect(),
        );

        git_runtime_changed
            || prev_banner_error != next_banner_error
            || prev_auth_prompt != self.state.auth_prompt
            || prev_branch_exists_prompt != self.state.branch_exists_prompt
    }
}

#[cfg(target_os = "macos")]
fn newly_opened_repo_paths(prev: &AppState, next: &AppState) -> Vec<std::path::PathBuf> {
    next.repos
        .iter()
        .filter_map(|next_repo| {
            if !matches!(next_repo.open, Loadable::Ready(())) {
                return None;
            }
            let was_ready = prev
                .repos
                .iter()
                .find(|repo| repo.id == next_repo.id)
                .is_some_and(|repo| matches!(repo.open, Loadable::Ready(())));
            (!was_ready).then(|| next_repo.spec.workdir.clone())
        })
        .collect()
}

fn parse_force_delete_branch_name(message: &str) -> Option<String> {
    if !message.contains("git branch -d failed:") {
        return None;
    }
    let needle = "run 'git branch -D ";
    let start = message.find(needle)? + needle.len();
    let rest = &message[start..];
    let end = rest.find('\'')?;
    let name = rest[..end].trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn parse_force_remove_worktree_path(command: &str, stderr: &str) -> Option<std::path::PathBuf> {
    if !is_force_remove_worktree_required_error(command, stderr) {
        return None;
    }
    parse_worktree_path_from_fatal(stderr)
        .or_else(|| parse_worktree_remove_path_from_command(command))
}

fn is_force_remove_worktree_required_error(command: &str, stderr: &str) -> bool {
    let command = command.trim();
    let is_worktree_remove = command.starts_with("git worktree remove ")
        && !command.starts_with("git worktree remove --force ");
    is_worktree_remove
        && stderr.contains("contains modified or untracked files")
        && stderr.contains("use --force to delete it")
}

fn parse_worktree_path_from_fatal(stderr: &str) -> Option<std::path::PathBuf> {
    let needle = "fatal: '";
    let start = stderr.find(needle)? + needle.len();
    let rest = &stderr[start..];
    let end = rest.find("' contains modified or untracked files")?;
    let path = rest[..end].trim();
    (!path.is_empty()).then(|| std::path::PathBuf::from(path))
}

fn parse_worktree_remove_path_from_command(command: &str) -> Option<std::path::PathBuf> {
    let command = command.trim();
    let rest = command.strip_prefix("git worktree remove ")?;
    let rest = rest.strip_prefix("--force ").unwrap_or(rest);
    let path = rest.trim();
    if path.is_empty() || path.starts_with('-') {
        return None;
    }
    Some(std::path::PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::git_operation::{GitOperationId, HookExecutionId};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    fn completed_hook_operation(
        status: GitHookOperationStatus,
        hook_name: &str,
    ) -> GitHookOperation {
        GitHookOperation {
            id: GitOperationId(1),
            label: "Commit".to_string(),
            context: Some("Exercise hook reporting".to_string()),
            time: SystemTime::UNIX_EPOCH,
            duration: Some(Duration::from_millis(50)),
            status,
            hooks: vec![gitcomet_state::model::GitHookRun {
                id: HookExecutionId {
                    sid: Arc::from("test-session"),
                    child_id: 1,
                },
                name: hook_name.to_string(),
                status: GitHookRunStatus::Failed,
                exit_code: Some(7),
                duration: Some(Duration::from_millis(20)),
            }],
            output: Default::default(),
            output_bytes: 0,
            output_truncated: false,
            latest_line: String::new(),
        }
    }

    #[cfg(target_os = "macos")]
    fn repo_with_open_state(repo_id: RepoId, path: &str, ready: bool) -> RepoState {
        let mut repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from(path),
            },
        );
        if ready {
            repo.open = Loadable::Ready(());
        }
        repo
    }

    #[test]
    fn parse_force_remove_worktree_path_prefers_fatal_path() {
        let command = "git worktree remove /tmp/from-command";
        let stderr = "git worktree remove /tmp/from-command failed: fatal: '/tmp/from-stderr' contains modified or untracked files, use --force to delete it.";
        assert_eq!(
            parse_force_remove_worktree_path(command, stderr),
            Some(PathBuf::from("/tmp/from-stderr"))
        );
    }

    #[test]
    fn parse_force_remove_worktree_path_falls_back_to_command_path() {
        let command = "git worktree remove /tmp/worktree";
        let stderr = "contains modified or untracked files, use --force to delete it";
        assert_eq!(
            parse_force_remove_worktree_path(command, stderr),
            Some(PathBuf::from("/tmp/worktree"))
        );
    }

    #[test]
    fn parse_force_remove_worktree_path_ignores_non_matching_errors() {
        let command = "git worktree remove /tmp/worktree";
        let stderr = "fatal: '/tmp/worktree' is not a working tree";
        assert_eq!(parse_force_remove_worktree_path(command, stderr), None);
    }

    #[test]
    fn parse_force_remove_worktree_path_ignores_already_forced_command() {
        let command = "git worktree remove --force /tmp/worktree";
        let stderr = "contains modified or untracked files, use --force to delete it";
        assert_eq!(parse_force_remove_worktree_path(command, stderr), None);
    }

    #[test]
    fn parse_worktree_remove_path_from_command_supports_forced_and_plain_remove() {
        assert_eq!(
            parse_worktree_remove_path_from_command("git worktree remove /tmp/worktree"),
            Some(PathBuf::from("/tmp/worktree"))
        );
        assert_eq!(
            parse_worktree_remove_path_from_command("git worktree remove --force /tmp/worktree"),
            Some(PathBuf::from("/tmp/worktree"))
        );
    }

    #[test]
    fn hook_completion_notice_distinguishes_blocking_and_ignored_failures() {
        let (kind, message) = hook_completion_notice(&completed_hook_operation(
            GitHookOperationStatus::Failed,
            "pre-commit",
        ));
        assert!(matches!(kind, components::ToastKind::Error));
        assert_eq!(message, "Commit blocked by pre-commit hook");

        let (kind, message) = hook_completion_notice(&completed_hook_operation(
            GitHookOperationStatus::SucceededWithHookFailure,
            "post-commit",
        ));
        assert!(matches!(kind, components::ToastKind::Warning));
        assert_eq!(message, "Commit created, but post-commit hook failed");
    }

    #[test]
    fn outer_failure_after_passed_hooks_keeps_the_original_git_error_path() {
        let mut operation = completed_hook_operation(GitHookOperationStatus::Failed, "pre-commit");
        operation.hooks[0].status = GitHookRunStatus::Succeeded;
        operation.hooks[0].exit_code = Some(0);

        assert!(outer_failure_after_hooks(&operation));

        operation.hooks[0].status = GitHookRunStatus::Failed;
        operation.hooks[0].exit_code = Some(1);
        assert!(!outer_failure_after_hooks(&operation));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn newly_opened_repo_paths_returns_only_repos_that_become_ready() {
        let prev = AppState {
            repos: vec![
                repo_with_open_state(RepoId(1), "/tmp/repo-a", false),
                repo_with_open_state(RepoId(2), "/tmp/repo-b", true),
            ],
            ..Default::default()
        };
        let next = AppState {
            repos: vec![
                repo_with_open_state(RepoId(1), "/tmp/repo-a", true),
                repo_with_open_state(RepoId(2), "/tmp/repo-b", true),
                repo_with_open_state(RepoId(3), "/tmp/repo-c", false),
            ],
            ..Default::default()
        };

        assert_eq!(
            newly_opened_repo_paths(&prev, &next),
            vec![PathBuf::from("/tmp/repo-a")]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn newly_opened_repo_paths_includes_brand_new_ready_repos_and_ignores_loading_ones() {
        let prev = AppState::default();
        let next = AppState {
            repos: vec![
                repo_with_open_state(RepoId(10), "/tmp/repo-new", true),
                repo_with_open_state(RepoId(11), "/tmp/repo-loading", false),
            ],
            ..Default::default()
        };

        assert_eq!(
            newly_opened_repo_paths(&prev, &next),
            vec![PathBuf::from("/tmp/repo-new")]
        );
    }
}
