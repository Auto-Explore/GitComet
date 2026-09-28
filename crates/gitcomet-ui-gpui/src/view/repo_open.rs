use super::*;

fn initialize_repository_command(path: &std::path::Path) -> std::process::Command {
    let mut command = gitcomet_core::process::git_command();
    command.arg("-C").arg(path).args(["init", "--quiet"]);
    command
}

fn interpret_initialize_repository_output(
    success: bool,
    status: &str,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<(), String> {
    if success {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    if detail.is_empty() {
        Err(format!("Git init failed with {status}."))
    } else {
        Err(format!("Git init failed: {detail}"))
    }
}

fn initialize_repository(path: &std::path::Path) -> Result<(), String> {
    let output = initialize_repository_command(path)
        .output()
        .map_err(|err| format!("Could not start Git: {err}"))?;

    interpret_initialize_repository_output(
        output.status.success(),
        &output.status.to_string(),
        &output.stdout,
        &output.stderr,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RepoTabDirection {
    Previous,
    Next,
}

fn adjacent_repo_tab_id(
    repo_ids: &[RepoId],
    active_repo: Option<RepoId>,
    direction: RepoTabDirection,
) -> Option<RepoId> {
    if repo_ids.is_empty() {
        return None;
    }

    let Some(active_ix) = active_repo.and_then(|repo_id| {
        repo_ids
            .iter()
            .position(|candidate_repo_id| *candidate_repo_id == repo_id)
    }) else {
        return repo_ids.first().copied();
    };

    if repo_ids.len() == 1 {
        return None;
    }

    let next_ix = match direction {
        RepoTabDirection::Previous => {
            if active_ix == 0 {
                repo_ids.len() - 1
            } else {
                active_ix - 1
            }
        }
        RepoTabDirection::Next => (active_ix + 1) % repo_ids.len(),
    };
    repo_ids.get(next_ix).copied()
}

impl GitCometView {
    /// Keyboard/menu entry point for the repository switcher: it toggles, and
    /// anchors to the same titlebar chevron the mouse uses. Only the command
    /// palette opens the picker centred, via
    /// [`Self::open_repository_switcher_centered`].
    pub(crate) fn toggle_repository_switcher(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.toggle_repo_picker(RepoPickerScope::All, window, cx);
    }

    /// Command-palette entry point: the palette itself is centred, so the
    /// picker that replaces it is too.
    pub(crate) fn open_repository_switcher_centered(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover_centered(
            PopoverKind::RepoPicker {
                scope: RepoPickerScope::All,
            },
            window,
            cx,
        );
    }

    /// Open Workspace: the same picker, listing only workspaces.
    pub(crate) fn toggle_workspace_picker(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.toggle_repo_picker(RepoPickerScope::WorkspacesOnly, window, cx);
    }

    pub(crate) fn open_workspace_picker_centered(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover_centered(
            PopoverKind::RepoPicker {
                scope: RepoPickerScope::WorkspacesOnly,
            },
            window,
            cx,
        );
    }

    fn toggle_repo_picker(
        &mut self,
        scope: RepoPickerScope,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind = PopoverKind::RepoPicker { scope };
        if self.popover_host.read(cx).is_kind_open(&kind) {
            self.popover_host.update(cx, |host, cx| {
                host.close_popover_and_restore_focus(window, cx)
            });
            return;
        }

        // The chevron has no painted bounds in a window that has not drawn it
        // (a new window, or Home without a workspace), so fall back to the
        // centred placement there.
        let Some(anchor) = self.title_bar.read(cx).repo_picker_toggle_bounds() else {
            self.open_popover_centered(kind, window, cx);
            return;
        };
        self.open_popover_for_bounds(kind, anchor, window, cx);
    }

    pub(crate) fn show_open_repo_panel_fallback(
        &mut self,
        window: Option<&mut Window>,
        show_notice: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_repo_panel = true;
        self.open_repo_input
            .update(cx, |input, cx| input.set_text("", cx));
        if let Some(window) = window {
            let focus = self
                .open_repo_input
                .read_with(cx, |input, _| input.focus_handle());
            window.focus(&focus, cx);
        }
        if show_notice {
            self.push_toast(
                components::ToastKind::Warning,
                "Native folder picker unavailable. Enter a repository path manually.".to_string(),
                cx,
            );
        }
        cx.notify();
    }

    pub(crate) fn activate_repo_path(
        &mut self,
        path: &std::path::Path,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo_id) = self.repo_id_for_path(path) else {
            return false;
        };
        if self.state.active_repo == Some(repo_id) {
            return false;
        }

        self.store.dispatch(Msg::SetActiveRepo { repo_id });
        cx.notify();
        true
    }

    /// Select a repository known to belong to this window, even if its saved
    /// tabs have not reached the view snapshot yet. `OpenRepo` is idempotent at
    /// the store: queued behind `RestoreSession` it selects the restored tab,
    /// and after restoration it selects the already-open repository.
    pub(crate) fn activate_or_open_repo_path(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.repo_id_for_path(&path).is_some() {
            self.activate_repo_path(&path, cx);
        } else if self
            .pending_repo_open_reservations
            .get(&path)
            .is_some_and(|pending| !pending.persist_in_workspace)
        {
            // Select the queued drop without treating its unvalidated path as
            // a normal open before the view catches up.
            self.store.dispatch(Msg::OpenRepoFromExternalDrop(path));
            cx.notify();
        } else {
            self.open_repo_path_locally(path, cx);
        }
    }

    pub(crate) fn close_active_repo_tab(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };

        if self.request_terminal_shutdown_action(TerminalShutdownAction::CloseRepo { repo_id }, cx)
        {
            return true;
        }

        self.store.dispatch(Msg::CloseRepo { repo_id });
        cx.notify();
        true
    }

    pub(crate) fn request_move_repo_to_workspace(
        &mut self,
        repo_id: RepoId,
        path: std::path::PathBuf,
        target_workspace: Option<gitcomet_state::session::WorkspaceId>,
        cx: &mut gpui::Context<Self>,
    ) {
        if crate::app::repository_move_target_is_noop(
            cx,
            self.window_handle.window_id(),
            target_workspace,
        ) {
            return;
        }
        if !self.prepare_repo_move(repo_id, &path, target_workspace, cx) {
            return;
        }

        let action = TerminalShutdownAction::MoveRepo {
            repo_id,
            path: path.clone(),
            target_workspace,
        };
        if self.request_terminal_shutdown_action(action, cx) {
            return;
        }
        crate::app::move_repository_to_workspace_from_view(
            cx,
            self.window_handle.window_id(),
            repo_id,
            path,
            target_workspace,
        );
    }

    /// Also checked at the deferred transfer boundary: validation or editor
    /// saves may still be pending after a menu or terminal dialog was opened.
    pub(crate) fn prepare_repo_move(
        &mut self,
        repo_id: RepoId,
        path: &std::path::Path,
        target_workspace: Option<session::WorkspaceId>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if !self.store.snapshot().repos.iter().any(|repo| {
            repo.id == repo_id
                && repo.spec.workdir == path
                && !repo.is_provisional_external_drop_open()
        }) {
            return false;
        }
        !self.request_unsaved_file_edits_prompt(
            UnsavedFileEditsAction::MoveRepo {
                window_id: self.window_handle.window_id(),
                repo_id,
                path: path.to_path_buf(),
                target_workspace,
            },
            cx,
        )
    }

    pub(crate) fn detach_repo_for_move(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        self.store.dispatch(Msg::MoveRepoOut { repo_id });
        cx.notify();
    }

    pub(crate) fn activate_previous_repo_tab(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        self.activate_repo_tab_in_direction(RepoTabDirection::Previous, cx)
    }

    pub(crate) fn activate_next_repo_tab(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        self.activate_repo_tab_in_direction(RepoTabDirection::Next, cx)
    }

    fn repo_id_for_path(&self, path: &std::path::Path) -> Option<RepoId> {
        self.state
            .repos
            .iter()
            .find(|repo| repo.spec.workdir == path)
            .map(|repo| repo.id)
    }

    fn activate_repo_tab_in_direction(
        &mut self,
        direction: RepoTabDirection,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let repo_ids: Vec<RepoId> = self.state.repos.iter().map(|repo| repo.id).collect();
        let Some(next_repo_id) = adjacent_repo_tab_id(&repo_ids, self.state.active_repo, direction)
        else {
            return false;
        };

        if self.state.active_repo == Some(next_repo_id) {
            return false;
        }

        self.store.dispatch(Msg::SetActiveRepo {
            repo_id: next_repo_id,
        });
        cx.notify();
        true
    }

    pub(crate) fn open_repo_path(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        crate::app::open_repository_from_view(cx, self.window_handle.window_id(), path);
        self.open_repo_panel = false;
        cx.notify();
    }

    pub(crate) fn open_repo_path_locally(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        self.reserve_pending_repo_open(&path, true, cx);
        if self.store.snapshot().git_runtime.is_available() {
            self.store.dispatch(Msg::OpenRepo(path));
        } else {
            self.queue_repo_open_until_git_recovers(path);
        }
        self.open_repo_panel = false;
        cx.notify();
    }

    pub(crate) fn open_dropped_repo_locally(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        self.reserve_pending_repo_open(&path, false, cx);
        self.store.dispatch(Msg::OpenRepoFromExternalDrop(path));
        cx.notify();
    }

    /// Publish ownership before the store reduces the open. Normal opens also
    /// reserve durable membership so a move can safely detach its source.
    fn reserve_pending_repo_open(
        &mut self,
        path: &std::path::Path,
        persist_in_workspace: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let failure_revision = self.store.snapshot().repo_open_failure_revision;
        self.pending_repo_open_reservations
            .entry(path.to_path_buf())
            .or_insert(PendingRepoOpen {
                failure_revision,
                persist_in_workspace,
            });
        if persist_in_workspace {
            self.pending_repo_open_active = Some(path.to_path_buf());
        }
        self.sync_workspace_and_registry(cx);
    }

    /// The last window outlives its deleted workspace: it forgets it and
    /// returns to Home instead of closing.
    pub(crate) fn reset_to_home_after_workspace_delete(&mut self, cx: &mut gpui::Context<Self>) {
        crate::workspaces::discard_workspace_for_window(cx, self.window_handle.window_id());
        // The window no longer claims the deleted id, in its syncs or the
        // window registry. A stale snapshot that still lists repositories can
        // briefly create an anonymous workspace; it goes once they close.
        self.workspace_id = None;
        self.pending_repo_open_reservations.clear();
        self.pending_repo_open_active = None;
        let repo_ids: Vec<RepoId> = self
            .store
            .snapshot()
            .repos
            .iter()
            .map(|repo| repo.id)
            .collect();
        if !repo_ids.is_empty() {
            self.store.dispatch(Msg::CloseRepos {
                repo_ids,
                activate_after: None,
            });
        }
        self.workspace_changed(cx);
        cx.notify();
    }

    /// Take `workspace` as this (empty) window's workspace. The window keeps
    /// its own placement; layout, repositories, colour and theme come along.
    pub(crate) fn adopt_workspace(
        &mut self,
        workspace: session::Workspace,
        cx: &mut gpui::Context<Self>,
    ) {
        let layout = workspace.layout.clone();
        let scale = self.ui_scale();
        if let Some(width) = layout.sidebar_width {
            self.set_sidebar_width_from_pixels(scale.px(width as f32));
        }
        if let Some(width) = layout.details_width {
            self.set_details_width_from_pixels(scale.px(width as f32));
        }
        self.details_pane.update(cx, |pane, _cx| {
            if let Some(height) = layout.change_tracking_height {
                pane.set_change_tracking_height_from_pixels(Some(scale.px(height as f32)));
            }
            if let Some(height) = layout.untracked_height {
                pane.set_untracked_height_from_pixels(Some(scale.px(height as f32)));
            }
        });
        self.set_sidebar_collapsed(layout.sidebar_collapsed, cx);
        self.clamp_pane_widths_to_window();

        self.workspace_id = Some(workspace.id);
        self.persisted_workspace_repo_paths
            .clone_from(&workspace.repositories);
        self.persisted_workspace_active_repository
            .clone_from(&workspace.active_repository);
        if !workspace.repositories.is_empty() {
            // Pending bootstrap keeps the saved membership through snapshots
            // taken before the store has reduced the restore.
            self.startup_repo_bootstrap_pending = true;
            if self.state.git_runtime.is_available() {
                self.store.dispatch(Msg::RestoreSession {
                    open_repos: workspace.repositories,
                    active_repo: workspace.active_repository,
                });
            } else {
                self.deferred_repo_bootstrap = Some(DeferredRepoBootstrap::RestoreSession {
                    open_repos: workspace.repositories,
                    active_repo: workspace.active_repository,
                });
            }
        }
        self.sync_workspace_and_registry(cx);
        self.workspace_changed(cx);
    }

    fn queue_repo_open_until_git_recovers(&mut self, path: std::path::PathBuf) {
        let push_unique = |paths: &mut Vec<std::path::PathBuf>, path: std::path::PathBuf| {
            if !paths.contains(&path) {
                paths.push(path);
            }
        };
        match self.deferred_repo_bootstrap.as_mut() {
            Some(DeferredRepoBootstrap::RestoreSession {
                open_repos,
                active_repo,
            }) => {
                push_unique(open_repos, path.clone());
                *active_repo = Some(path);
            }
            Some(DeferredRepoBootstrap::OpenRepos(paths)) => push_unique(paths, path),
            None => {
                self.deferred_repo_bootstrap = Some(DeferredRepoBootstrap::OpenRepos(vec![path]));
            }
        }
        self.startup_repo_bootstrap_pending = true;
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn apply_patch_from_file(
        &mut self,
        patch: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo_id) = self.state.active_repo else {
            return;
        };
        self.store.dispatch(Msg::ApplyPatch { repo_id, patch });
        cx.notify();
    }

    pub(crate) fn prompt_open_repo(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let view = cx.weak_entity();

        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Git Repository".into()),
        });

        window
            .spawn(cx, async move |cx| {
                let result = rx.await;
                let paths = match result {
                    Ok(Ok(Some(paths))) => paths,
                    Ok(Ok(None)) => return,
                    Ok(Err(_)) | Err(_) => {
                        let _ = view.update(cx, |this, cx| {
                            this.show_open_repo_panel_fallback(None, false, cx);
                        });
                        return;
                    }
                };

                let Some(path) = paths.into_iter().next() else {
                    return;
                };

                // Let the backend decide whether the path is a repository.
                // Frontend checks are brittle across bare repos/worktrees/submodules.
                let _ = view.update(cx, |this, cx| this.open_repo_path(path, cx));
            })
            .detach();
    }

    pub(crate) fn prompt_init_repo(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let view = cx.weak_entity();
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Initialize Git Repository".into()),
        });

        window
            .spawn(cx, async move |cx| {
                let result = rx.await;
                let paths = match result {
                    Ok(Ok(Some(paths))) => paths,
                    Ok(Ok(None)) => return,
                    Ok(Err(err)) => {
                        let _ = view.update(cx, |this, cx| {
                            this.push_toast(
                                components::ToastKind::Error,
                                format!("Could not open the folder picker: {err}"),
                                cx,
                            );
                        });
                        return;
                    }
                    Err(err) => {
                        let _ = view.update(cx, |this, cx| {
                            this.push_toast(
                                components::ToastKind::Error,
                                format!("Could not open the folder picker: {err}"),
                                cx,
                            );
                        });
                        return;
                    }
                };

                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let init_path = path.clone();
                let result = smol::unblock(move || initialize_repository(&init_path)).await;

                let _ = view.update(cx, |this, cx| match result {
                    Ok(()) => {
                        this.push_toast(
                            components::ToastKind::Success,
                            format!("Initialized repository at {}", path.display()),
                            cx,
                        );
                        this.open_repo_path(path, cx);
                    }
                    Err(message) => {
                        this.push_toast(components::ToastKind::Error, message, cx);
                    }
                });
            })
            .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_repository_command_targets_selected_folder() {
        let path = std::path::Path::new("/tmp/gitcomet-init-wrapper-test");
        let command = initialize_repository_command(path);
        let args: Vec<_> = command.get_args().map(std::ffi::OsStr::to_owned).collect();

        assert_eq!(
            args,
            vec![
                // Every command from `git_command()` carries the ext guard.
                std::ffi::OsString::from("-c"),
                std::ffi::OsString::from("protocol.ext.allow=never"),
                std::ffi::OsString::from("-C"),
                path.as_os_str().to_owned(),
                std::ffi::OsString::from("init"),
                std::ffi::OsString::from("--quiet"),
            ]
        );
    }

    #[test]
    fn initialize_repository_output_accepts_success() {
        assert_eq!(
            interpret_initialize_repository_output(true, "exit status: 0", b"", b""),
            Ok(())
        );
    }

    #[test]
    fn initialize_repository_output_surfaces_git_error() {
        assert_eq!(
            interpret_initialize_repository_output(
                false,
                "exit status: 128",
                b"ignored stdout",
                b"fatal: cannot initialize repository\n",
            ),
            Err("Git init failed: fatal: cannot initialize repository".to_string())
        );
    }

    #[test]
    fn initialize_repository_output_reports_status_without_git_detail() {
        assert_eq!(
            interpret_initialize_repository_output(false, "exit status: 1", b"", b""),
            Err("Git init failed with exit status: 1.".to_string())
        );
    }

    #[test]
    fn adjacent_repo_tab_id_wraps_left_from_first_repo() {
        let repo_ids = [RepoId(1), RepoId(2), RepoId(3)];

        let target = adjacent_repo_tab_id(&repo_ids, Some(RepoId(1)), RepoTabDirection::Previous);

        assert_eq!(target, Some(RepoId(3)));
    }

    #[test]
    fn adjacent_repo_tab_id_wraps_right_from_last_repo() {
        let repo_ids = [RepoId(1), RepoId(2), RepoId(3)];

        let target = adjacent_repo_tab_id(&repo_ids, Some(RepoId(3)), RepoTabDirection::Next);

        assert_eq!(target, Some(RepoId(1)));
    }

    #[test]
    fn adjacent_repo_tab_id_defaults_to_first_when_no_repo_is_active() {
        let repo_ids = [RepoId(4), RepoId(5)];

        let target = adjacent_repo_tab_id(&repo_ids, None, RepoTabDirection::Next);

        assert_eq!(target, Some(RepoId(4)));
    }

    #[test]
    fn adjacent_repo_tab_id_noops_for_single_active_repo() {
        let repo_ids = [RepoId(9)];

        let target = adjacent_repo_tab_id(&repo_ids, Some(RepoId(9)), RepoTabDirection::Next);

        assert_eq!(target, None);
    }
}
