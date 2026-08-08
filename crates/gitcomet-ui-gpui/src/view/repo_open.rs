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
        if self
            .popover_host
            .read(cx)
            .is_kind_open(&PopoverKind::RepoPicker)
        {
            self.popover_host.update(cx, |host, cx| {
                host.close_popover_and_restore_focus(window, cx)
            });
            return;
        }

        // The chevron has no painted bounds yet in a window that has not drawn
        // its titlebar (the "open a new window, then show the switcher" path),
        // so fall back to the centred placement there.
        let Some(anchor) = self.title_bar.read(cx).repo_picker_toggle_bounds() else {
            self.open_repository_switcher_centered(window, cx);
            return;
        };
        self.open_popover_for_bounds(PopoverKind::RepoPicker, anchor, window, cx);
    }

    /// Command-palette entry point: the palette itself is centred, so the
    /// picker that replaces it is too.
    pub(crate) fn open_repository_switcher_centered(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover_centered(PopoverKind::RepoPicker, window, cx);
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
        self.store.dispatch(Msg::OpenRepo(path));
        self.open_repo_panel = false;
        cx.notify();
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
