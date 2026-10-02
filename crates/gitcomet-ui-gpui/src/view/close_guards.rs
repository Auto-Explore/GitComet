//! The close guards, in the order a close meets them: unsaved editor
//! buffers, running terminal commands, running Git operations, then
//! extension guards. Each stage either lets the close through or takes it
//! over with a prompt; resolving a prompt resumes at the next stage, so no
//! guard asks twice and none is skipped.

use super::*;

/// How long a save-and-close waits for the dispatched writes to land. A timeout
/// is not a save: it restores those recovery copies and asks the user again.
const UNSAVED_FILE_EDITS_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const UNSAVED_FILE_EDITS_FLUSH_POLL: std::time::Duration = std::time::Duration::from_millis(25);

/// Re-run whatever the unsaved-edits prompt interrupted.
fn retry_close_action(action: UnsavedFileEditsAction, cx: &mut gpui::App) {
    match action {
        UnsavedFileEditsAction::CloseWindow(window_id) => {
            crate::app::close_window_by_id_or_warn(cx, window_id)
        }
        UnsavedFileEditsAction::DeleteWorkspace { workspace_id, .. } => {
            crate::app::delete_workspace(cx, workspace_id)
        }
        UnsavedFileEditsAction::QuitApp => crate::app::quit_app_or_warn(cx),
        UnsavedFileEditsAction::MoveRepo {
            window_id,
            repo_id,
            path,
            target_workspace,
        } => crate::app::request_move_repository_to_workspace_by_id(
            cx,
            window_id,
            repo_id,
            path,
            target_workspace,
        ),
    }
}

impl GitCometView {
    pub(crate) fn request_close_window_or_warn(
        &mut self,
        window_id: gpui::WindowId,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.flush_workspace_environment(cx);
        if self
            .request_unsaved_file_edits_prompt(UnsavedFileEditsAction::CloseWindow(window_id), cx)
        {
            return true;
        }
        self.request_terminal_shutdown_action(TerminalShutdownAction::CloseWindow, cx)
            || self.request_late_close_guards(&TerminalShutdownAction::CloseWindow, cx)
    }

    /// The close guards, for deleting this window's workspace. Nothing is
    /// flushed: the layout is about to be forgotten.
    pub(crate) fn request_delete_workspace_or_warn(
        &mut self,
        window_id: gpui::WindowId,
        workspace_id: gitcomet_state::session::WorkspaceId,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.request_unsaved_file_edits_prompt(
            UnsavedFileEditsAction::DeleteWorkspace {
                window_id,
                workspace_id,
            },
            cx,
        ) {
            return true;
        }
        let action = TerminalShutdownAction::DeleteWorkspace { workspace_id };
        self.request_terminal_shutdown_action(action.clone(), cx)
            || self.request_late_close_guards(&action, cx)
    }

    /// [`Self::request_unsaved_file_edits_prompt`] for a quit, callable from
    /// the app-level shutdown path (which cannot name the action enum).
    pub(crate) fn request_quit_unsaved_file_edits_prompt(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.flush_workspace_environment(cx);
        self.request_unsaved_file_edits_prompt(UnsavedFileEditsAction::QuitApp, cx)
    }

    /// Queue the unsaved-edits dialog if the editor is holding writes that
    /// closing would throw away. Returns whether it took over the action.
    ///
    /// Resolving it re-runs the original request rather than closing directly,
    /// so a window with both unsaved edits and a running command still gets the
    /// terminal warning afterwards.
    pub(in crate::view) fn request_unsaved_file_edits_prompt(
        &mut self,
        action: UnsavedFileEditsAction,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.pending_unsaved_file_edits_flush.is_some() {
            // Closing the window or quitting supersedes a pending move. Keep
            // the requested action until the existing receipts have drained.
            let pending = self.pending_file_edits_action.as_ref();
            if !matches!(pending, Some(UnsavedFileEditsAction::QuitApp))
                && (action.moving_repo().is_none()
                    || pending.is_none_or(|pending| pending.moving_repo().is_some()))
            {
                self.pending_file_edits_action = Some(action);
            }
            return true;
        }
        // Explorer operations, native transfers and Documents saves share the
        // filesystem queue with editor saves; let them land first.
        if self.filesystem_writes_pending(cx) {
            self.retry_once_file_edit_writes_drain(action, cx);
            return true;
        }
        // `pending_*_prompt` is `take()`n by `Render` when it opens the popover,
        // so it is `None` for as long as the dialog is actually on screen. Ask
        // the popover host whether the dialog is up rather than mirroring that
        // into a bool: a mirror only stays true, and every way the popover can
        // go away without being closed — `open_popover` replacing it, say —
        // would leave it stuck and the window permanently unclosable.
        if self.pending_unsaved_file_edits_prompt.is_some()
            || self.unsaved_file_edits_dialog_open(cx)
        {
            return true;
        }
        // With auto-save on, a buffer inside its 800 ms quiet period is not an
        // unsaved edit — it is a write that has not fired yet, so the user is
        // asked nothing. But flushing only *dispatches* the write, and returning
        // `false` here let the caller quit out from under it: the store never
        // reduced the message and the edits were lost. Take over the close and
        // let it through once the write has actually drained. If encoding
        // fails, no write was dispatched and the dirty buffer needs the
        // Save/Discard prompt below.
        let moving_repo = action.moving_repo();
        let writes_pending = self.main_pane.update(cx, |pane, cx| {
            if moving_repo.is_none_or(|repo_id| {
                pane.file_editor_key
                    .as_ref()
                    .and_then(|key| pane.document_repo_path(key))
                    .is_some_and(|(editing_repo, _)| editing_repo == repo_id)
            }) {
                // Moving is an automatic flush, not permission to overwrite a
                // disk conflict. Dirty stashed buffers can also be held behind
                // a conflict, so leave those for the explicit Save/Discard prompt.
                pane.flush_file_editor_buffer(cx);
            }
            // Failed saves and dirty stashes are unsaved edits. Only an actual
            // queued save should delay the dialog and retry the action.
            pane.file_editor_saves_block_action(&action)
        });
        if writes_pending {
            self.retry_once_file_edit_writes_drain(action, cx);
            return true;
        }
        let files = self.unsaved_file_edit_labels_for(moving_repo, cx);
        if files.is_empty() {
            return false;
        }
        self.pending_unsaved_file_edits_prompt = Some(UnsavedFileEditsPrompt {
            action,
            files,
            waiting_for_writes: false,
        });
        cx.notify();
        true
    }

    /// Whether the unsaved-edits dialog is the popover currently on screen.
    fn unsaved_file_edits_dialog_open(&self, cx: &gpui::App) -> bool {
        self.popover_host
            .read(cx)
            .showing_unsaved_file_edits_prompt()
    }

    pub(in crate::view) fn clear_pending_unsaved_file_edits_prompt(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_unsaved_file_edits_prompt = None;
        cx.notify();
    }

    /// Save or discard the unsaved buffers, then retry what the user asked for.
    ///
    /// Discarding lets close and quit proceed immediately; moves still wait for
    /// dispatched writes. Saving must also wait for the store's command executor
    /// so the app cannot exit with files still unwritten.
    /// Each save has a completion receipt, including ones already dispatched
    /// before the prompt. The wait is bounded: a
    /// wedged command brings this dialog back instead of trapping the user.
    pub(in crate::view) fn resolve_unsaved_file_edits(
        &mut self,
        action: UnsavedFileEditsAction,
        save: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_unsaved_file_edits_prompt = None;
        let moving_repo = action.moving_repo();
        let saved = self.main_pane.update(cx, |pane, cx| {
            if save && let Some(repo_id) = moving_repo {
                pane.save_file_edits_for_repo(repo_id, cx)
            } else if save {
                pane.save_all_file_edits(cx)
            } else if let Some(repo_id) = moving_repo {
                pane.discard_file_edits_for_repo(repo_id, cx);
                true
            } else {
                pane.discard_all_file_edits(cx);
                true
            }
        });
        // Documents belong to the window, not to a repository being moved.
        if moving_repo.is_none() {
            self.documents.update(cx, |documents, cx| {
                if save {
                    documents.save_all(cx);
                } else {
                    documents.discard_all(cx);
                }
            });
        }

        if !saved {
            return;
        }

        if !save {
            // Ordering note: the caller's `close_popover` defers a clear of
            // `pending_unsaved_file_edits_prompt`, and it runs *after* this
            // retry. If the retry finds edits still outstanding and queues a
            // fresh prompt, that clear would silently swallow it and the close
            // would do nothing — so the retry is deferred behind the clear.
            cx.defer(move |cx| cx.defer(move |cx| retry_close_action(action, cx)));
            return;
        }
        self.retry_once_file_edit_writes_drain(action, cx);
    }

    fn unsaved_file_edit_labels_for(
        &self,
        moving_repo: Option<RepoId>,
        cx: &gpui::App,
    ) -> Vec<SharedString> {
        let pane = self.main_pane.read(cx);
        moving_repo.map_or_else(
            || {
                let mut files = pane.unsaved_file_edit_labels();
                files.extend(self.documents.read(cx).unsaved_labels(cx));
                files
            },
            |repo_id| pane.unsaved_file_edit_labels_for_repo(repo_id),
        )
    }

    /// Filesystem work outside the editor that closing must not cut short.
    fn filesystem_writes_pending(&self, cx: &gpui::App) -> bool {
        super::native_transfers::active(cx)
            || self.file_operations.has_pending()
            || !self.state.filesystem.pending.is_empty()
            || !self.documents.read(cx).saves_drained(cx)
    }

    /// Wait for receipts from the exact editor writes, rather than assuming an
    /// idle store has already processed their queued messages.
    fn retry_once_file_edit_writes_drain(
        &mut self,
        action: UnsavedFileEditsAction,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_file_edits_action = Some(action);
        self.pending_unsaved_file_edits_flush = Some(cx.spawn(async move |view, cx| {
            let mut deadline = cx.background_executor().now() + UNSAVED_FILE_EDITS_FLUSH_TIMEOUT;
            loop {
                cx.background_executor()
                    .timer(UNSAVED_FILE_EDITS_FLUSH_POLL)
                    .await;
                let timed_out = cx.background_executor().now() >= deadline;
                let Ok(pending) = view.update(cx, |this, cx| {
                    let action = this
                        .pending_file_edits_action
                        .as_ref()
                        .expect("pending save action");
                    this.filesystem_writes_pending(cx)
                        || this
                            .main_pane
                            .read(cx)
                            .file_editor_saves_block_action(action)
                }) else {
                    return;
                };
                if !pending {
                    let _ = view.update(cx, |this, cx| {
                        this.pending_unsaved_file_edits_flush = None;
                        let action = this
                            .pending_file_edits_action
                            .take()
                            .expect("pending save action");
                        // Failed saves stay dirty. Re-entering the guard
                        // prompts for those instead of detaching.
                        cx.defer(move |cx| retry_close_action(action, cx));
                    });
                    return;
                }
                if !timed_out {
                    continue;
                }
                // A timeout is not a save: ask again about what is still
                // unwritten. With nothing to list, keep waiting.
                let prompted = view.update(cx, |this, cx| {
                    let moving_repo = this
                        .pending_file_edits_action
                        .as_ref()
                        .expect("pending save action")
                        .moving_repo();
                    let mut files = this.unsaved_file_edit_labels_for(moving_repo, cx);
                    let waiting_for_writes = files.is_empty();
                    if waiting_for_writes && let Some(repo_id) = moving_repo {
                        files = this
                            .main_pane
                            .read(cx)
                            .pending_file_edit_labels_for_repo(repo_id);
                    }
                    if files.is_empty() {
                        return false;
                    }
                    this.pending_unsaved_file_edits_flush = None;
                    let action = this
                        .pending_file_edits_action
                        .take()
                        .expect("pending save action");
                    this.pending_unsaved_file_edits_prompt = Some(UnsavedFileEditsPrompt {
                        action,
                        files,
                        waiting_for_writes,
                    });
                    cx.notify();
                    true
                });
                if !matches!(prompted, Ok(false)) {
                    return;
                }
                deadline = cx.background_executor().now() + UNSAVED_FILE_EDITS_FLUSH_TIMEOUT;
            }
        }));
    }
}

/// A Git operation still running in `repo` that a close would interrupt:
/// fetches, pulls (worktree pulls included), pushes, commits, and submodule
/// clones. A window's repository clone is checked by the caller. Editor
/// writes are the unsaved-edits guard's, so they are not counted twice.
fn running_git_operation(repo: &RepoState) -> Option<&'static str> {
    if repo.push_in_flight > 0 {
        Some("a push")
    } else if repo.pull_in_flight > 0 {
        Some("a fetch or pull")
    } else if repo.commit_in_flight > 0 {
        Some("a commit")
    } else if repo.submodule_add_in_flight.is_some() {
        Some("a submodule clone")
    } else {
        None
    }
}

impl GitCometView {
    /// Closes repository tabs through every guard: running terminal
    /// commands, running Git operations, then extension guards. Every way to
    /// close a tab (its button, Cmd+W, the tab menu) comes through here.
    pub(crate) fn request_close_repos(
        &mut self,
        repo_ids: Vec<RepoId>,
        activate_after: Option<RepoId>,
        cx: &mut gpui::Context<Self>,
    ) {
        let action = match repo_ids.as_slice() {
            [repo_id] if activate_after.is_none() => {
                TerminalShutdownAction::CloseRepo { repo_id: *repo_id }
            }
            _ => TerminalShutdownAction::CloseRepos {
                repo_ids,
                activate_after,
            },
        };
        if self.request_terminal_shutdown_action(action.clone(), cx)
            || self.request_late_close_guards(&action, cx)
        {
            return;
        }
        match action {
            TerminalShutdownAction::CloseRepo { repo_id } => {
                self.store.dispatch(Msg::CloseRepo { repo_id });
            }
            TerminalShutdownAction::CloseRepos {
                repo_ids,
                activate_after,
            } => {
                self.store.dispatch(Msg::CloseRepos {
                    repo_ids,
                    activate_after,
                });
            }
            _ => {}
        }
        cx.notify();
    }

    /// The guards after the terminal's. Queues their confirmation and returns
    /// `true` when any has something to say.
    pub(in crate::view) fn request_late_close_guards(
        &mut self,
        action: &TerminalShutdownAction,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        // Only deduplicate the same action. A different request must run its
        // own guards and, when needed, replace the confirmation.
        if self
            .pending_close_guard_prompt
            .as_ref()
            .map(|prompt| &prompt.action)
            .or_else(|| self.popover_host.read(cx).close_guard_action())
            == Some(action)
        {
            return true;
        }
        let mut reasons = self.late_close_guard_reasons(action, cx);
        if matches!(action, TerminalShutdownAction::QuitApp) {
            reasons.extend(super::settings_window::close_guards::quit_reasons(cx));
            // A quit closes every window; the others' reasons count too.
            let this = cx.entity_id();
            for view in self
                .pending_quit_other_views
                .iter()
                .filter_map(|v| v.upgrade())
            {
                if view.entity_id() == this {
                    continue;
                }
                for reason in view.read(cx).quit_close_guard_reasons(cx) {
                    if !reasons.contains(&reason) {
                        reasons.push(reason);
                    }
                }
            }
        }
        if reasons.is_empty() {
            return false;
        }
        self.pending_close_guard_prompt = Some(CloseGuardPrompt {
            action: action.clone(),
            reasons,
        });
        cx.notify();
        true
    }

    /// Why closing for `action` needs confirming, from running Git
    /// operations and then extension guards. Empty when it does not.
    pub(in crate::view) fn late_close_guard_reasons(
        &self,
        action: &TerminalShutdownAction,
        cx: &App,
    ) -> Vec<SharedString> {
        use gitcomet_extension_api::CloseScope;
        let (scope, repos): (CloseScope, Vec<&RepoState>) = match action {
            TerminalShutdownAction::CloseRepo { repo_id } => (
                CloseScope::Repository,
                self.state
                    .repos
                    .iter()
                    .filter(|repo| repo.id == *repo_id)
                    .collect(),
            ),
            TerminalShutdownAction::CloseRepos { repo_ids, .. } => (
                CloseScope::Repository,
                self.state
                    .repos
                    .iter()
                    .filter(|repo| repo_ids.contains(&repo.id))
                    .collect(),
            ),
            TerminalShutdownAction::CloseWindow
            | TerminalShutdownAction::DeleteWorkspace { .. } => {
                (CloseScope::Window, self.state.repos.iter().collect())
            }
            TerminalShutdownAction::QuitApp => {
                (CloseScope::Application, self.state.repos.iter().collect())
            }
            TerminalShutdownAction::MoveRepo { .. }
            | TerminalShutdownAction::CloseTerminalForRepo { .. }
            | TerminalShutdownAction::CloseTerminalTab { .. } => return Vec::new(),
        };
        let mut reasons: Vec<SharedString> = repos
            .iter()
            .filter_map(|repo| {
                running_git_operation(repo).map(|operation| {
                    let name = repo
                        .spec
                        .workdir
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| repo.spec.workdir.display().to_string());
                    SharedString::from(format!("{name} is still running {operation}."))
                })
            })
            .collect();
        if matches!(scope, CloseScope::Application | CloseScope::Window)
            && self.state.clone.as_ref().is_some_and(|operation| {
                matches!(
                    operation.status,
                    gitcomet_state::model::CloneOpStatus::Running
                )
            })
        {
            reasons.push("A repository clone is still running.".into());
        }
        for reason in self.extension_close_reasons(scope, &repos, cx) {
            if !reasons.contains(&reason) {
                reasons.push(reason);
            }
        }
        reasons
    }

    fn extension_close_reasons(
        &self,
        scope: gitcomet_extension_api::CloseScope,
        repos: &[&RepoState],
        cx: &App,
    ) -> Vec<SharedString> {
        use gitcomet_extension_api::{CloseDecision, CloseRequest, CloseScope};
        let Some(window) = self.extension_window.as_ref() else {
            return Vec::new();
        };
        let Some(registry) = super::extension_host::registry(cx) else {
            return Vec::new();
        };
        if registry.close_guards().is_empty() {
            return Vec::new();
        }
        let host = window.host();
        let window_id = self.window_handle.window_id();
        let requests: Vec<CloseRequest> = match scope {
            CloseScope::Repository => repos
                .iter()
                .map(|repo| CloseRequest {
                    scope,
                    window: host.clone(),
                    repository: Some(super::extension_host::repository_handle(window_id, repo)),
                })
                .collect(),
            // Window and Application: one request for the whole window.
            _ => vec![CloseRequest {
                scope,
                window: host.clone(),
                repository: None,
            }],
        };
        let mut reasons: Vec<SharedString> = Vec::new();
        for request in &requests {
            for (_, guard) in registry.close_guards() {
                super::perf::extension_dispatch();
                if let CloseDecision::Confirm { reason } = guard(request, cx)
                    && !reasons.contains(&reason)
                {
                    reasons.push(reason);
                }
            }
        }
        reasons
    }

    /// The late guards' reasons for quitting, for the app-level quit that
    /// gathers them from every window.
    pub(crate) fn quit_close_guard_reasons(&self, cx: &App) -> Vec<SharedString> {
        self.late_close_guard_reasons(&TerminalShutdownAction::QuitApp, cx)
    }

    /// Asks before quitting for reasons gathered from every window; `views`
    /// are all windows, whose terminals a confirmed quit shuts down.
    pub(crate) fn request_quit_close_guards(
        &mut self,
        reasons: Vec<SharedString>,
        views: Vec<gpui::WeakEntity<Self>>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_quit_other_views = views;
        self.pending_close_guard_prompt = Some(CloseGuardPrompt {
            action: TerminalShutdownAction::QuitApp,
            reasons,
        });
        cx.notify();
    }
}
