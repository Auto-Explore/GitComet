use super::*;

impl GitCometView {
    fn hook_activity_workflow_repo(kind: &PopoverKind) -> Option<RepoId> {
        match kind {
            PopoverKind::HookActivity { repo_id, .. } => Some(*repo_id),
            _ => None,
        }
    }

    pub(in crate::view) fn set_hook_activity_dialog_repo(
        &mut self,
        repo_id: Option<RepoId>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.toast_host.update(cx, |host, cx| {
            host.set_hook_activity_dialog_repo(repo_id, cx)
        });
    }

    pub(in crate::view) fn minimize_hook_activity_chains(
        &mut self,
        chains: impl IntoIterator<Item = (RepoId, GitOperationId)>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.minimized_hook_activity_chains.extend(chains);
        self.pending_hook_activity_open = None;
        self.set_hook_activity_dialog_repo(None, cx);
        cx.notify();
    }

    pub(in crate::view) fn minimize_hook_activity_repo(
        &mut self,
        repo_id: RepoId,
        chains: impl IntoIterator<Item = (RepoId, GitOperationId)>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.minimized_hook_activity_repos.insert(repo_id);
        self.minimize_hook_activity_chains(chains, cx);
    }

    pub(in crate::view) fn resume_hook_activity_auto_open(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        self.minimized_hook_activity_repos.remove(&repo_id);
        self.minimized_hook_activity_chains
            .retain(|(minimized_repo_id, _)| *minimized_repo_id != repo_id);
        cx.notify();
    }

    pub(in crate::view) fn hook_activity_workflow_is_open(&self, cx: &App) -> bool {
        self.popover_host.read(cx).is_hook_activity_workflow_open()
    }

    pub(in crate::view) fn hook_activity_workflow_repo_id(&self, cx: &App) -> Option<RepoId> {
        self.popover_host.read(cx).hook_activity_workflow_repo_id()
    }

    pub(in crate::view) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.set_hook_activity_dialog_repo(Self::hook_activity_workflow_repo(&kind), cx);
        self.history_refs_hover_host
            .update(cx, |host, cx| host.close(cx));
        self.popover_host.update(cx, |host, cx| {
            host.open_popover_at(kind, anchor, window, cx)
        });
    }

    /// Close the submodule trust popover only while it is showing its pending
    /// spinner for `repo_id` (no trust prompt yet). Used when a background trust
    /// check resolves to a silent proceed or an error, so the spinner does not
    /// linger. A no-op if the user already dismissed it or another popover is up.
    pub(in crate::view) fn close_submodule_trust_spinner(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind = PopoverKind::submodule(repo_id, SubmodulePopoverKind::TrustConfirm);
        self.popover_host.update(cx, |host, cx| {
            if host.is_kind_open(&kind) {
                host.close_popover(cx);
            }
        });
    }

    pub(in crate::view) fn open_popover_centered(
        &mut self,
        kind: PopoverKind,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let PopoverKind::HookActivity { repo_id, .. } = &kind {
            self.pending_hook_activity_open = None;
            self.minimized_hook_activity_chains
                .retain(|(suppressed_repo_id, _)| suppressed_repo_id != repo_id);
        }
        self.set_hook_activity_dialog_repo(Self::hook_activity_workflow_repo(&kind), cx);
        self.history_refs_hover_host
            .update(cx, |host, cx| host.close(cx));
        self.popover_host
            .update(cx, |host, cx| host.open_popover_centered(kind, window, cx));
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn open_clone_repository_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover_centered(PopoverKind::CloneRepo, window, cx);
    }

    pub(in crate::view) fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.set_hook_activity_dialog_repo(Self::hook_activity_workflow_repo(&kind), cx);
        self.history_refs_hover_host
            .update(cx, |host, cx| host.close(cx));
        self.popover_host.update(cx, |host, cx| {
            host.open_popover_for_bounds(kind, anchor_bounds, window, cx)
        });
    }

    pub(super) fn open_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.command_palette_open = true;
        let restore_focus = window
            .focused(cx)
            .or_else(|| self.pre_palette_focus.clone());
        let fallback_focus = self.main_pane.read(cx).diff_panel_focus_handle.clone();
        let has_active_repo = self.active_repo_id().is_some();
        self.command_palette.update(cx, |palette, cx| {
            palette.open(restore_focus, fallback_focus, has_active_repo, window, cx);
        });
    }

    pub(super) fn close_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.command_palette_open = false;
        self.command_palette
            .update(cx, |palette, cx| palette.close(window, cx));
    }

    pub(super) fn command_palette_did_close(
        &mut self,
        command: Option<&str>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.command_palette_open = false;
        if let Some(command) = command {
            self.execute_command(command, Some(window), cx);
        }
    }

    pub(crate) fn toggle_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.command_palette_open {
            self.close_command_palette(window, cx);
        } else {
            self.open_command_palette(window, cx);
        }
    }

    pub(super) fn execute_command(
        &mut self,
        command_id: &str,
        window: Option<&mut Window>,
        cx: &mut gpui::Context<Self>,
    ) {
        match command_id {
            "new-window" => cx.defer(|cx| cx.dispatch_action(&NewWindow)),
            "open-settings" => cx.defer(crate::view::open_settings_window),
            "quit" => cx.defer(crate::app::quit_app_or_warn),
            "minimize-window" => cx.defer(|cx| {
                if let Some(win) = cx.active_window() {
                    let _ = win.update(cx, |_root, win, _cx| win.minimize_window());
                }
            }),
            "zoom-window" => cx.defer(|cx| {
                if let Some(win) = cx.active_window() {
                    let _ = win.update(cx, |_root, win, _cx| {
                        super::super::app::toggle_window_zoom(win)
                    });
                }
            }),
            "toggle-fullscreen" => cx.defer(|cx| {
                if let Some(win) = cx.active_window() {
                    let _ = win.update(cx, |_root, win, _cx| win.toggle_fullscreen());
                }
            }),
            "increase-ui-scale" => cx.defer(|cx| cx.dispatch_action(&IncreaseUiScale)),
            "decrease-ui-scale" => cx.defer(|cx| cx.dispatch_action(&DecreaseUiScale)),
            "reset-ui-scale" => cx.defer(|cx| cx.dispatch_action(&ResetUiScale)),
            "close-window" => cx.defer(|cx| cx.dispatch_action(&CloseWindow)),
            "locate-file-in-explorer" => self.locate_open_file_in_explorer(cx),
            "open-repository" => cx.defer(|cx| cx.dispatch_action(&OpenRepository)),
            "switch-repository" => {
                if let Some(window) = window {
                    self.open_repository_switcher_centered(window, cx);
                }
            }
            "clone-repository" => {
                if let Some(window) = window {
                    self.open_popover_centered(PopoverKind::CloneRepo, window, cx);
                }
            }
            "close-repo-tab" => {
                self.close_active_repo_tab(cx);
            }
            "reload-repository" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::ReloadRepo { repo_id });
                }
            }
            "fetch-all" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::FetchAll { repo_id });
                }
            }
            "previous-repo-tab" => {
                self.activate_previous_repo_tab(cx);
            }
            "next-repo-tab" => {
                self.activate_next_repo_tab(cx);
            }
            "open-active-view-search" => cx.defer(|cx| cx.dispatch_action(&OpenActiveViewSearch)),
            "toggle-sidebar" => {
                self.set_sidebar_collapsed(!self.sidebar_collapsed, cx);
            }
            "toggle-details" => {
                self.set_details_collapsed(!self.details_collapsed, cx);
            }
            "toggle-diff-view" => {
                let next = match self.diff_view_mode {
                    DiffViewMode::Split => DiffViewMode::Inline,
                    DiffViewMode::Inline => DiffViewMode::Split,
                };
                self.set_diff_view_mode(next, cx);
            }
            "toggle-diff-word-wrap" => {
                self.set_diff_word_wrap(!self.diff_word_wrap, cx);
            }
            "toggle-line-numbers" => {
                self.set_diff_show_line_numbers(!self.diff_show_line_numbers, cx);
            }
            "toggle-whitespace-chars" => {
                self.set_diff_reveal_whitespace_chars(!self.diff_reveal_whitespace_chars, cx);
            }
            "create-branch" => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    let target = self
                        .state
                        .repos
                        .iter()
                        .find(|r| r.id == repo_id)
                        .and_then(|repo| {
                            if let Loadable::Ready(head) = &repo.head_branch {
                                Some(head.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "HEAD".to_string());
                    self.open_popover_centered(
                        PopoverKind::CreateBranchFromRefPrompt {
                            repo_id,
                            target,
                            source_selectable: true,
                            name_prefix: String::new(),
                        },
                        window,
                        cx,
                    );
                }
            }
            "checkout-branch" => {
                if let Some(window) = window {
                    self.open_popover_centered(
                        PopoverKind::BranchPicker {
                            purpose: BranchPickerPurpose::Checkout,
                        },
                        window,
                        cx,
                    );
                }
            }
            "delete-branch" => {
                if let Some(window) = window {
                    self.open_popover_centered(
                        PopoverKind::BranchPicker {
                            purpose: BranchPickerPurpose::Delete,
                        },
                        window,
                        cx,
                    );
                }
            }
            "rename-branch" => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                    && let Some(name) = self
                        .state
                        .repos
                        .iter()
                        .find(|repo| repo.id == repo_id)
                        .and_then(|repo| match &repo.head_branch {
                            Loadable::Ready(name) if name != "HEAD" && !name.is_empty() => {
                                Some(name.clone())
                            }
                            _ => None,
                        })
                {
                    self.open_popover_centered(
                        PopoverKind::RenameBranchPrompt {
                            repo_id,
                            name,
                            is_current_branch: true,
                        },
                        window,
                        cx,
                    );
                }
            }
            "checkout-remote-branch" => {
                // TODO: Open remote branch picker
            }
            "pull" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::Pull {
                        repo_id,
                        mode: PullMode::Default,
                    });
                }
            }
            "push" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::Push { repo_id });
                }
            }
            "force-push" => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    self.open_popover_centered(
                        PopoverKind::ForcePushConfirm { repo_id },
                        window,
                        cx,
                    );
                }
            }
            "delete-remote-branch" => {
                // TODO: Implement delete remote branch
            }
            "commit" => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    self.open_popover_centered(PopoverKind::CommitPrompt { repo_id }, window, cx);
                }
            }
            "apply-patch" => {
                let Some(repo_id) = self.active_repo_id() else {
                    return;
                };
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                        files: true,
                        directories: false,
                        multiple: false,
                        prompt: Some("Select patch file".into()),
                    });
                    cx.spawn(async move |cx| {
                        let result = rx.await;
                        let paths = match result {
                            Ok(Ok(Some(paths))) => paths,
                            _ => return,
                        };
                        let Some(patch) = paths.into_iter().next() else {
                            return;
                        };
                        let _ = view.update(cx, |this, _cx| {
                            this.store.dispatch(Msg::ApplyPatch { repo_id, patch });
                        });
                    })
                    .detach();
                });
            }
            "stage-all" => {
                let Some(repo_id) = self.active_repo_id() else {
                    return;
                };
                let paths: Vec<_> = self
                    .state
                    .repos
                    .iter()
                    .find(|r| r.id == repo_id)
                    .and_then(|repo| repo.worktree_status_entries())
                    .map(|entries| entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>())
                    .unwrap_or_default();
                if paths.is_empty() {
                    return;
                }
                // Staging is what marks a conflict resolved, so confirm first if
                // any of it still has conflict markers in the worktree. With no
                // window there is nothing to confirm in, and staging unasked is
                // the one outcome this must not have.
                // No row selection is involved here, so there is none to consume.
                if let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
                    &self.state,
                    repo_id,
                    paths.clone(),
                    false,
                ) {
                    if let Some(window) = window {
                        self.open_popover_centered(confirm, window, cx);
                    }
                    return;
                }
                self.store.dispatch(Msg::StagePaths {
                    repo_id,
                    paths: paths.into(),
                });
            }
            "unstage-all" => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id)
                {
                    let paths: Vec<_> = repo
                        .staged_status_entries()
                        .map(|entries| entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>())
                        .unwrap_or_default();
                    if !paths.is_empty() {
                        self.store.dispatch(Msg::UnstagePaths {
                            repo_id,
                            paths: paths.into(),
                        });
                    }
                }
            }
            "discard-all" => {
                // TODO: Implement discard all changes command
            }
            "stash" => {
                if let Some(window) = window {
                    self.open_popover_centered(PopoverKind::StashPrompt, window, cx);
                }
            }
            "stash-pop" | "stash-apply" | "stash-drop" => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    let purpose = match command_id {
                        "stash-pop" => StashPickerPurpose::Pop,
                        "stash-apply" => StashPickerPurpose::Apply,
                        _ => StashPickerPurpose::Drop,
                    };
                    self.open_popover_centered(
                        PopoverKind::StashPickerPrompt { repo_id, purpose },
                        window,
                        cx,
                    );
                }
            }
            "merge" => {
                // TODO: Implement merge branch/ref
            }
            "rebase" => {
                if let Some(window) = window {
                    self.open_popover_centered(
                        PopoverKind::BranchPicker {
                            purpose: BranchPickerPurpose::RebaseOnto,
                        },
                        window,
                        cx,
                    );
                }
            }
            "create-tag" => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    self.open_popover_centered(
                        PopoverKind::CreateTagPrompt {
                            repo_id,
                            target: "HEAD".into(),
                        },
                        window,
                        cx,
                    );
                }
            }
            "delete-tag" => {
                // TODO: Implement delete tag
            }
            "show-reflog" => {
                self.open_reflog_panel_for_active_repo(cx);
            }
            "add-remote" => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    self.open_popover_centered(
                        PopoverKind::Repo {
                            repo_id,
                            kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
                        },
                        window,
                        cx,
                    );
                }
            }
            "remove-remote" => {
                // TODO: Implement remove remote
            }
            "edit-remote-url" => {
                // TODO: Implement edit remote URL
            }
            "add-submodule" => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    self.open_popover_centered(
                        PopoverKind::Repo {
                            repo_id,
                            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
                        },
                        window,
                        cx,
                    );
                }
            }
            "update-submodules" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::UpdateSubmodules { repo_id });
                }
            }
            "remove-submodule" => {
                // TODO: Implement remove submodule
            }
            "add-worktree" => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    self.open_popover_centered(
                        PopoverKind::Repo {
                            repo_id,
                            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                        },
                        window,
                        cx,
                    );
                }
            }
            "remove-worktree" => {
                // TODO: Implement remove worktree
            }
            "blame" => {
                self.set_annotate_enabled(!self.annotate_enabled, cx);
            }
            "back" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::GlobalNavBack { repo_id });
                }
            }
            "forward" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::GlobalNavForward { repo_id });
                }
            }
            _ => {}
        }
    }

    /// Whether a popover, dialog, prompt, or context menu is currently open
    /// (all are tracked as a `PopoverKind` by the popover host).
    pub(in crate::view) fn is_overlay_open(&self, cx: &App) -> bool {
        // The collapsed-sidebar section popover covers the history view too, so it
        // must suppress ref hovers the same way the popover host does.
        self.popover_host.read(cx).is_open() || self.sidebar_collapsed_popover.is_some()
    }

    pub(in crate::view) fn show_history_refs_hover(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        source_bounds: Bounds<Pixels>,
        items: Arc<[HistoryRefListItem]>,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        // Don't surface the refs hover while an overlay (popover, dialog, or
        // context menu) is open on top of the history view — the history canvas
        // handles mouse-move at the window level, so it still fires under the
        // overlay. If the open overlay is the hover's own item menu, leave the
        // existing hover in place.
        if self.is_overlay_open(cx) && !self.history_refs_hover_host.read(cx).is_item_menu_open() {
            self.close_history_refs_hover(cx);
            return;
        }
        self.history_refs_hover_host.update(cx, |host, cx| {
            host.show(
                repo_id,
                commit_id,
                source_bounds,
                items,
                pointer,
                window,
                cx,
            )
        });
    }

    pub(in crate::view) fn show_commit_message_hover(
        &mut self,
        next: CommitMessageHoverState,
        pointer: Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Same reasoning as the refs hover: the history canvas listens for
        // mouse-move at the window level, so it still fires under an open
        // overlay and the card would surface on top of it.
        if self.is_overlay_open(cx) {
            self.dismiss_commit_message_hover(cx);
            return;
        }
        self.commit_message_hover_host
            .update(cx, |host, cx| host.show(next, pointer, cx));
    }

    pub(in crate::view) fn dismiss_commit_message_hover(&mut self, cx: &mut gpui::Context<Self>) {
        self.commit_message_hover_host
            .update(cx, |host, cx| host.dismiss(cx));
    }

    pub(in crate::view) fn close_history_refs_hover(&mut self, cx: &mut gpui::Context<Self>) {
        self.history_refs_hover_host
            .update(cx, |host, cx| host.close(cx));
    }

    pub(in crate::view) fn dismiss_history_refs_menus(&mut self, cx: &mut gpui::Context<Self>) {
        self.close_history_refs_hover(cx);

        let history_refs_menu_open =
            self.active_context_menu_invoker
                .as_ref()
                .is_some_and(|invoker| {
                    invoker
                        .as_ref()
                        .starts_with(HISTORY_REFS_HOVER_MENU_INVOKER_PREFIX)
                });
        if history_refs_menu_open {
            self.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
        }
    }

    pub(in crate::view) fn set_history_refs_hover_item_menu_open(
        &mut self,
        open: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_refs_hover_host
            .update(cx, |host, cx| host.set_item_menu_open(open, cx));
    }

    pub(in crate::view) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next.clone();

        let sidebar_pane = self.sidebar_pane.clone();
        let main_pane = self.main_pane.clone();
        let details_pane = self.details_pane.clone();
        let repo_tabs_bar = self.repo_tabs_bar.clone();
        let action_bar = self.action_bar.clone();
        let bottom_status_bar = self.bottom_status_bar.clone();

        cx.defer(move |cx| {
            sidebar_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            main_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            details_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            repo_tabs_bar.update(cx, |bar, cx| {
                bar.set_active_context_menu_invoker(next.clone(), cx);
            });
            action_bar.update(cx, |bar, cx| {
                bar.set_active_context_menu_invoker(next.clone(), cx);
            });
            bottom_status_bar.update(cx, |bar, cx| {
                bar.set_active_context_menu_invoker(next.clone(), cx);
            });
        });
    }

    pub(in crate::view) fn register_pending_worktree_branch_removal(
        &mut self,
        repo_id: RepoId,
        path: std::path::PathBuf,
        branch: String,
    ) {
        self.pending_worktree_branch_removals
            .insert((repo_id, path), branch);
    }

    pub(super) fn take_pending_worktree_branch_removal(
        &mut self,
        repo_id: RepoId,
        path: &std::path::Path,
    ) -> Option<String> {
        self.pending_worktree_branch_removals
            .remove(&(repo_id, path.to_path_buf()))
    }

    #[cfg(test)]
    pub fn new(
        store: AppStore,
        events: smol::channel::Receiver<StoreEvent>,
        initial_path: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let config = match initial_path {
            Some(path) => GitCometViewConfig::normal_with_initial_repository(path, None),
            None => GitCometViewConfig::normal(None),
        };
        Self::new_with_config(store, events, config, window, cx)
    }

    pub fn new_with_config(
        store: AppStore,
        events: smol::channel::Receiver<StoreEvent>,
        config: GitCometViewConfig,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let GitCometViewConfig {
            mut initial_path,
            initial_repository_launch_mode,
            view_mode,
            focused_mergetool,
            focused_mergetool_exit_code,
            startup_crash_report,
        } = config;
        if initial_path.is_none() {
            initial_path = focused_mergetool.as_ref().map(|cfg| cfg.repo_path.clone());
        }
        let focused_mergetool_labels = focused_mergetool.as_ref().map(|cfg| cfg.labels.clone());
        let focused_mergetool_bootstrap = if view_mode == GitCometViewMode::FocusedMergetool {
            focused_mergetool
                .clone()
                .map(FocusedMergetoolBootstrap::from_view_config)
        } else {
            None
        };
        let store = Arc::new(store);

        let mut ui_session = session::load();
        let mut ui_preferences = UiPreferences::from_session(&ui_session);
        let ui_scale = ui_scale::current_or_initialize_from_session(&ui_session, cx);
        // The application-wide scale may already have been initialized by
        // another window. Keep the shared runtime preferences aligned with the
        // value every view will actually render with.
        ui_preferences.appearance.ui_scale_percent = ui_scale.percent;
        let _font_preferences =
            crate::font_preferences::current_or_initialize_from_session(window, &ui_session, cx);
        if should_seed_initial_repository_from_session(
            view_mode,
            initial_path.as_deref(),
            initial_repository_launch_mode,
            !ui_session.open_repos.is_empty(),
        ) && let Some(path) = initial_path.as_ref()
        {
            if !ui_session.open_repos.iter().any(|p| p == path) {
                ui_session.open_repos.push(path.clone());
            }
            ui_session.active_repo = Some(path.clone());
        }

        let restored_sidebar_width = ui_preferences.window.sidebar_width;
        let restored_details_width = ui_preferences.window.details_width;
        let restored_sidebar_collapsed = ui_preferences.window.sidebar_collapsed;
        let _ = crate::theme::ensure_user_themes_dir_exists();
        let theme_mode = ui_preferences.appearance.theme_mode.clone();
        let initial_theme = theme_mode.resolve_theme(window.appearance());
        let date_time_format = ui_preferences.appearance.date_time_format;
        let timezone = ui_preferences.appearance.timezone;
        let show_timezone = ui_preferences.appearance.show_timezone;
        let change_tracking_view = ui_preferences.change_tracking.view;
        let terminal_preferences = ui_preferences.terminal.clone();
        let diff_scroll_sync = ui_preferences.diff.scroll_sync;
        let diff_content_mode = ui_preferences.diff.content_mode;
        let diff_whitespace_mode = ui_preferences.diff.whitespace_mode;
        let diff_view_mode = ui_preferences.diff.view_mode;
        let annotate_enabled = ui_preferences.diff.annotate_enabled;
        let diff_reveal_whitespace_chars = ui_preferences.diff.reveal_whitespace_chars;
        let diff_word_wrap = ui_preferences.diff.word_wrap;
        let diff_show_line_numbers = ui_preferences.diff.show_line_numbers;
        let auto_save_file_edits = ui_preferences.file_editing.auto_save;
        let commit_push_after_enabled = ui_preferences.repository.commit_push_after_enabled;
        let history_show_tags = ui_preferences.history.show_tags;
        let history_tag_fetch_mode = ui_preferences.history.tag_fetch_mode;
        let default_tag_type = ui_preferences.repository.default_tag_type;
        store.dispatch(Msg::SetGitLogSettings {
            show_history_tags: history_show_tags,
            tag_fetch_mode: history_tag_fetch_mode,
        });
        store.dispatch(Msg::SetDefaultTagType(default_tag_type));
        let saved_open_repos = ui_session.open_repos.clone();
        let saved_active_repo = ui_session.active_repo.clone();
        let mut startup_repo_bootstrap_pending = false;
        let mut deferred_repo_bootstrap = None;

        // Only auto-restore/open on startup if the store hasn't already been preloaded.
        // This avoids re-opening repos (and changing RepoIds) when the UI is attached to an
        // already-initialized store (notably in `gpui::test` setup).
        let initial_store_state = store.snapshot();
        let store_preloaded = !initial_store_state.repos.is_empty();
        let git_runtime_available = initial_store_state.git_runtime.is_available();
        let should_auto_restore = !crate::startup_probe::disable_auto_restore()
            && view_mode != GitCometViewMode::FocusedMergetool
            && crate::ui_runtime::current().auto_restores_session()
            && !store_preloaded;

        if should_auto_restore {
            if !saved_open_repos.is_empty() {
                if git_runtime_available {
                    store.dispatch(Msg::RestoreSession {
                        open_repos: saved_open_repos,
                        active_repo: saved_active_repo,
                    });
                    startup_repo_bootstrap_pending = true;
                } else {
                    deferred_repo_bootstrap = Some(DeferredRepoBootstrap::RestoreSession {
                        open_repos: saved_open_repos,
                        active_repo: saved_active_repo,
                    });
                }
            }
        } else if store_preloaded {
            if let Some(path) = initial_path.as_ref() {
                if git_runtime_available {
                    store.dispatch(Msg::OpenRepo(path.clone()));
                } else {
                    deferred_repo_bootstrap = Some(DeferredRepoBootstrap::OpenRepo(path.clone()));
                }
            }
        } else if let Some(path) = initial_path.as_ref() {
            if git_runtime_available {
                store.dispatch(Msg::OpenRepo(path.clone()));
                startup_repo_bootstrap_pending = true;
            } else {
                deferred_repo_bootstrap = Some(DeferredRepoBootstrap::OpenRepo(path.clone()));
            }
        }

        let initial_state = store.snapshot();
        if !initial_state.repos.is_empty() {
            startup_repo_bootstrap_pending = false;
        }
        let ui_model = cx.new(|_cx| {
            AppUiModel::new_with_preferences(Arc::clone(&initial_state), ui_preferences.clone())
        });

        let ui_model_subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let should_quit = crate::startup_probe::observe_app_state(next.as_ref());
            let should_notify = this.apply_state_snapshot(next, cx);
            if should_notify {
                cx.notify();
            }
            if should_quit {
                crate::app::mark_clean_shutdown_from_view(cx);
                cx.quit();
            }
        });

        let weak_view = cx.weak_entity();
        let poller = Poller::start(Arc::clone(&store), events, ui_model.downgrade(), window, cx);

        let title_bar = cx.new(|cx| {
            TitleBarView::new(
                initial_theme,
                weak_view.clone(),
                titlebar_workspace_actions_enabled(view_mode, !initial_state.repos.is_empty()),
                cx,
            )
        });
        let tooltip_host = cx.new(|_cx| TooltipHost::new(initial_theme));
        let toast_host = cx.new(|_cx| ToastHost::new(initial_theme, weak_view.clone()));
        let history_refs_hover_host =
            cx.new(|_cx| HistoryRefsHoverHost::new(initial_theme, weak_view.clone()));
        let commit_message_hover_host = cx.new(|_cx| {
            CommitMessageHoverHost::new(initial_theme, Arc::clone(&store), ui_model.clone())
        });
        let repo_tabs_bar = cx.new(|cx| {
            RepoTabsBarView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                weak_view.clone(),
                cx,
            )
        });
        let action_bar = cx.new(|cx| {
            ActionBarView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                weak_view.clone(),
                cx,
            )
        });
        let bottom_status_bar = cx.new(|cx| {
            BottomStatusBarView::new(initial_theme, ui_model.clone(), weak_view.clone(), cx)
        });

        let sidebar_pane = cx.new(|cx| {
            SidebarPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                ui_session.repo_sidebar_collapsed_items.clone(),
                ui_session.repo_sidebar_pinned_branches.clone(),
                weak_view.clone(),
                tooltip_host.downgrade(),
                cx,
            )
        });
        let main_pane = cx.new(|cx| {
            MainPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                MainPaneInit {
                    theme: initial_theme,
                    view_mode,
                    focused_mergetool_labels,
                    focused_mergetool_exit_code: focused_mergetool_exit_code.clone(),
                    root_view: weak_view.clone(),
                    tooltip_host: tooltip_host.downgrade(),
                },
                window,
                cx,
            )
        });
        window
            .observe_release(&main_pane, cx, |pane, window, cx| {
                if let Some(cancel) = pane.conflict_image_preview_cancel.take() {
                    cancel.store(true, std::sync::atomic::Ordering::Release);
                }
                pane.conflict_image_preview_task = None;
                pane.file_image_preview_animation_task = None;

                let mut images = Vec::new();
                if let Some(image) = pane.file_image_diff_cache_old.take() {
                    images.push(image);
                }
                if let Some(image) = pane.file_image_diff_cache_new.take()
                    && images
                        .iter()
                        .all(|known: &Arc<gpui::RenderImage>| known.id != image.id)
                {
                    images.push(image);
                }
                for side in ThreeWayColumn::ALL {
                    if let Loadable::Ready(Some(ConflictPreviewImage::Rendered(image))) =
                        pane.conflict_resolver.image_preview.image(side)
                        && images
                            .iter()
                            .all(|known: &Arc<gpui::RenderImage>| known.id != image.id)
                    {
                        images.push(Arc::clone(image));
                    }
                }
                for image in images {
                    cx.drop_image(image, Some(&mut *window));
                }
            })
            .detach();
        let details_pane = cx.new(|cx| {
            DetailsPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                DetailsPaneInit {
                    theme: initial_theme,
                    root_view: weak_view.clone(),
                    main_pane: main_pane.downgrade(),
                    tooltip_host: tooltip_host.downgrade(),
                },
                window,
                cx,
            )
        });

        let reflog_pane = cx.new(|cx| {
            ReflogPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                ReflogPaneInit {
                    theme: initial_theme,
                    root_view: weak_view.clone(),
                },
                cx,
            )
        });

        let popover_host = cx.new(|cx| {
            PopoverHost::new(
                Arc::clone(&store),
                ui_model.clone(),
                PopoverHostInit {
                    theme: initial_theme,
                    root_view: weak_view.clone(),
                    root_view_mode: view_mode,
                    tooltip_host: tooltip_host.downgrade(),
                    main_pane: main_pane.clone(),
                    details_pane: details_pane.clone(),
                    reflog_pane: reflog_pane.clone(),
                    sidebar_pane: sidebar_pane.clone(),
                    pinned_branches_by_repo: ui_session.repo_sidebar_pinned_branches.clone(),
                    collapsed_items_by_repo: ui_session.repo_sidebar_collapsed_items.clone(),
                },
                window,
                cx,
            )
        });

        let command_palette = cx.new(|cx| {
            command_palette::CommandPaletteView::new(
                initial_theme,
                initial_state.active_repo.is_some(),
                weak_view.clone(),
                window,
                cx,
            )
        });

        let activation_subscription = cx.observe_window_activation(window, |this, window, cx| {
            let now = Instant::now();
            if !window.is_window_active() {
                // Leaving the app is one of the two moments auto-save has to
                // mean more than "after a pause": the pending timer would
                // otherwise fire against a window the user has already left,
                // and an external edit to the same file could land first.
                this.main_pane
                    .update(cx, |pane, cx| pane.flush_file_editor_buffer(cx));
                // Capture the focused element before the platform blur() fires and clears it.
                // This is the restore target when opening the palette via a global hotkey while
                // this window is in the background.
                this.pre_palette_focus = window.focused(cx);
                // A deactivation right after we asked for a move/resize grab is
                // the compositor taking focus for the drag, not the user leaving
                // the app. Remember it so the matching re-activation does not
                // refresh the repo.
                this.window_grab_activation_suppressed_at =
                    crate::app::take_window_grab_started_within(now, WINDOW_GRAB_DEACTIVATE_GRACE)
                        .then_some(now);
                return;
            }
            let self_initiated_grab =
                consume_window_grab_activation(&mut this.window_grab_activation_suppressed_at, now);
            let runtime = refresh_git_runtime();
            if runtime != this.state.git_runtime {
                this.store
                    .dispatch(Msg::SetGitRuntimeState(runtime.clone()));
            }
            // Suppressed activations skip `repo_activation_msg` entirely, so its
            // throttle map is not stamped and a genuine alt-tab immediately after
            // a drag still refreshes.
            if !runtime.is_available() || self_initiated_grab {
                return;
            }
            if let Some(msg) =
                repo_activation_msg(&this.state, &mut this.last_repo_activation_dispatch_at, now)
            {
                // Other worktrees have no watcher of their own — the repo
                // monitor only flushes for the active repo — so coming back to
                // the window is the moment their uncommitted-change counts get
                // reconciled. Rides the same throttle as the activation refresh.
                if let Some(repo_id) = this.state.active_repo {
                    this.store.dispatch(Msg::LoadWorktreeDirty { repo_id });
                }
                this.store.dispatch(msg);
            }
        });

        let appearance_subscription = {
            let view = cx.weak_entity();
            let mut first = true;
            window.observe_window_appearance(move |window, app| {
                if first {
                    first = false;
                    return;
                }
                let _ = view.update(app, |this, cx| {
                    if !this.theme_mode.is_automatic() {
                        return;
                    }
                    let theme = this.theme_mode.resolve_theme(window.appearance());
                    this.set_theme(theme, cx);
                    cx.notify();
                });
            })
        };

        let open_repo_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/repo".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let error_banner_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    multiline: true,
                    read_only: true,
                    chromeless: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let auth_prompt_username_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Username".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let auth_prompt_secret_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Password / passphrase / confirmation".into(),
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_masked(true, cx);
            input
        });

        let open_repo_input_subscription = cx.observe(&open_repo_input, |this, input, cx| {
            let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
            let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

            if !this.open_repo_panel {
                return;
            }

            if escape_pressed {
                this.open_repo_panel = false;
                cx.notify();
                return;
            }
            if enter_pressed {
                this.submit_open_repo_panel(cx);
                return;
            }
            cx.notify();
        });

        let auth_prompt_username_input_subscription =
            cx.observe(&auth_prompt_username_input, |this, input, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

                if escape_pressed {
                    this.store.dispatch(Msg::CancelAuthPrompt);
                    cx.notify();
                    return;
                }
                if enter_pressed {
                    this.try_auth_prompt_submit(cx);
                    return;
                }
                cx.notify();
            });

        let auth_prompt_secret_input_subscription =
            cx.observe(&auth_prompt_secret_input, |this, input, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

                if escape_pressed {
                    this.store.dispatch(Msg::CancelAuthPrompt);
                    cx.notify();
                    return;
                }
                if enter_pressed {
                    this.try_auth_prompt_submit(cx);
                    return;
                }
                cx.notify();
            });

        let scale = ui_scale::UiScale::from_percent(ui_scale.percent);
        let initial_sidebar_width_design =
            ui_scale::design_units_from_stored(restored_sidebar_width)
                .unwrap_or(280.0)
                .max(SIDEBAR_MIN_PX);
        let initial_details_width_design =
            ui_scale::design_units_from_stored(restored_details_width)
                .unwrap_or(420.0)
                .max(DETAILS_MIN_PX);
        let initial_sidebar_width = scale.px(initial_sidebar_width_design);
        let initial_details_width = scale.px(initial_details_width_design);
        // Reopen collapsed if the user quit while collapsed: the render width must
        // also start at the collapsed strip so it doesn't flash open on launch.
        let initial_sidebar_render_width = if restored_sidebar_collapsed {
            scale.px(PANE_COLLAPSED_PX)
        } else {
            initial_sidebar_width
        };

        let terminal_keystroke_interceptor = Self::install_terminal_keystroke_interceptor(cx);

        let mut view = Self {
            state: Arc::clone(&initial_state),
            window_handle: window.window_handle(),
            ui_model,
            store,
            _poller: poller,
            _ui_model_subscription: ui_model_subscription,
            _activation_subscription: activation_subscription,
            _appearance_subscription: appearance_subscription,
            _terminal_keystroke_interceptor: terminal_keystroke_interceptor,
            _auth_prompt_username_input_subscription: auth_prompt_username_input_subscription,
            _open_repo_input_subscription: open_repo_input_subscription,
            _auth_prompt_secret_input_subscription: auth_prompt_secret_input_subscription,
            view_mode,
            theme_mode,
            theme: initial_theme,
            title_bar,
            sidebar_pane,
            main_pane,
            details_pane,
            repo_tabs_bar,
            action_bar,
            bottom_status_bar,
            tooltip_host,
            toast_host,
            history_refs_hover_host,
            commit_message_hover_host,
            popover_host,
            command_palette,
            command_palette_open: false,
            pre_palette_focus: None,
            focused_mergetool_bootstrap,
            submodule_diff_bootstrap: None,
            deferred_repo_bootstrap,
            startup_repo_bootstrap_pending,
            splash_backdrop_image: splash::load_splash_backdrop_image(),
            last_window_size: size(px(0.0), px(0.0)),
            ui_window_size_last_seen: size(px(0.0), px(0.0)),
            ui_settings_persist_seq: 0,
            last_repo_activation_dispatch_at: FxHashMap::default(),
            window_grab_activation_suppressed_at: None,
            date_time_format,
            timezone,
            show_timezone,
            change_tracking_view,
            terminal_preferences,
            terminal_sessions: FxHashMap::default(),
            terminal_panel_height: px(TERMINAL_PANEL_DEFAULT_HEIGHT_PX),
            terminal_panel_resize: None,
            next_terminal_session_seq: 1,
            terminal_cursor_blink_visible: true,
            terminal_cursor_blink_hold_until: Instant::now(),
            terminal_cursor_blink_active: false,
            terminal_cursor_blink_task_scheduled: false,
            terminal_cursor_blink_seq: 0,
            reflog_pane,
            active_bottom_panel: FxHashMap::default(),
            commit_push_after_enabled,
            diff_scroll_sync,
            diff_content_mode,
            diff_whitespace_mode,
            diff_view_mode,
            annotate_enabled,
            diff_reveal_whitespace_chars,
            diff_word_wrap,
            diff_show_line_numbers,
            auto_save_file_edits,
            ui_scale_percent: ui_scale.percent,
            open_repo_panel: false,
            open_repo_input,
            external_drag_paths: None,
            external_drag_payload: None,
            external_drag_classification_seq: 0,
            external_drag_drop_pending: false,
            hover_resize_edge: None,
            sidebar_collapsed: restored_sidebar_collapsed,
            sidebar_collapsed_popover: None,
            sidebar_collapsed_popover_closing: None,
            sidebar_collapsed_popover_anim_seq: 0,
            sidebar_collapsed_before_merge_view: None,
            details_collapsed: false,
            sidebar_width_design: initial_sidebar_width_design,
            details_width_design: initial_details_width_design,
            sidebar_width: initial_sidebar_width,
            details_width: initial_details_width,
            sidebar_render_width: initial_sidebar_render_width,
            details_render_width: initial_details_width,
            sidebar_width_anim_seq: 0,
            details_width_anim_seq: 0,
            sidebar_width_animating: false,
            details_width_animating: false,
            pane_resize: None,
            last_mouse_pos: point(px(0.0), px(0.0)),
            pending_terminal_shutdown_prompt: None,
            pending_unsaved_file_edits_prompt: None,
            pending_unsaved_file_edits_flush: None,
            pending_quit_other_views: Vec::new(),
            pending_pull_reconcile_prompt: None,
            pending_branch_exists_prompt: initial_state.branch_exists_prompt.clone(),
            pending_force_delete_branch_prompt: None,
            pending_force_delete_branch_centered: false,
            pending_force_remove_worktree_prompt: None,
            pending_submodule_trust_prompt: None,
            pending_submodule_trust_check: None,
            pending_hook_activity_open: None,
            minimized_hook_activity_chains: FxHashSet::default(),
            minimized_hook_activity_repos: FxHashSet::default(),
            pending_worktree_branch_removals: FxHashMap::default(),
            startup_crash_report,
            #[cfg(target_os = "macos")]
            recent_repos_menu_fingerprint: ui_session.recent_repos.clone(),
            error_banner_input,
            auth_prompt_username_input,
            auth_prompt_secret_input,
            auth_prompt_key: None,
            active_context_menu_invoker: None,
        };

        view.set_theme(initial_theme, cx);
        view.sync_action_bar_terminal_target(cx);

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        view.maybe_auto_install_linux_desktop_integration(cx);

        view.drive_focused_mergetool_bootstrap();
        view.drive_submodule_diff_bootstrap();
        view.maybe_show_user_survey_on_startup(cx);
        view.maybe_check_for_updates_on_startup(cx);

        crate::app::sync_gitcomet_window_state(
            cx,
            view.window_handle,
            cx.weak_entity(),
            view.main_pane.downgrade(),
            view.view_mode,
            view.state
                .repos
                .iter()
                .map(|repo| repo.spec.workdir.clone())
                .collect(),
        );

        view
    }

    pub(super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        for session in self.terminal_sessions.values() {
            for instance in &session.instances {
                instance.viewport.update(cx, |viewport, cx| {
                    viewport.set_theme(theme, cx);
                });
            }
        }
        self.title_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.sidebar_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.main_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.details_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.reflog_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.repo_tabs_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.action_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.bottom_status_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.tooltip_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.toast_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.history_refs_hover_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.commit_message_hover_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.popover_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.command_palette
            .update(cx, |palette, cx| palette.set_theme(theme, cx));
        self.open_repo_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.error_banner_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.auth_prompt_username_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.auth_prompt_secret_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        cx.notify();
    }

    pub(super) fn notify_font_preferences_changed(&mut self, cx: &mut gpui::Context<Self>) {
        for session in self.terminal_sessions.values() {
            for instance in &session.instances {
                instance.viewport.update(cx, |viewport, cx| {
                    viewport.invalidate_layout(cx);
                });
            }
        }
        self.title_bar.update(cx, |_bar, cx| cx.notify());
        self.sidebar_pane.update(cx, |_pane, cx| cx.notify());
        self.main_pane
            .update(cx, |pane, cx| pane.invalidate_font_metrics(cx));
        self.details_pane.update(cx, |_pane, cx| cx.notify());
        self.reflog_pane.update(cx, |_pane, cx| cx.notify());
        self.repo_tabs_bar.update(cx, |_bar, cx| cx.notify());
        self.action_bar.update(cx, |_bar, cx| cx.notify());
        self.bottom_status_bar.update(cx, |_bar, cx| cx.notify());
        self.tooltip_host.update(cx, |_host, cx| cx.notify());
        self.toast_host.update(cx, |_host, cx| cx.notify());
        self.popover_host.update(cx, |_host, cx| cx.notify());
        self.open_repo_input.update(cx, |_input, cx| cx.notify());
        self.error_banner_input.update(cx, |_input, cx| cx.notify());
        self.auth_prompt_username_input
            .update(cx, |_input, cx| cx.notify());
        self.auth_prompt_secret_input
            .update(cx, |_input, cx| cx.notify());
        cx.notify();
    }

    /// Repaint the panes that show which files have unsaved editor buffers.
    ///
    /// The main pane owns those buffers and the sidebar draws them, and the two
    /// are separate entities with no store snapshot between them to carry the
    /// change — so the pane that changed it says so, here.
    pub(in crate::view) fn notify_unsaved_file_edits_changed(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.sidebar_pane.update(cx, |_pane, cx| cx.notify());
        cx.notify();
    }

    pub(super) fn refresh_main_pane_after_panel_animation(&mut self, cx: &mut gpui::Context<Self>) {
        let main_pane = self.main_pane.clone();
        cx.defer(move |cx| {
            main_pane.update(cx, |pane, cx| {
                pane.sync_root_layout_snapshot(cx);
                cx.notify();
            });
        });
    }

    /// Evaluate a CSS-style `cubic-bezier(x1, y1, x2, y2)` timing function at
    /// progress `t` in `[0, 1]`. Endpoints P0=(0,0) and P3=(1,1) are implicit.
    ///
    /// The curve is parametric in `s`, so for a given time `t` we first solve
    /// `bezier_x(s) = t` (a few Newton-Raphson steps — the x-curve is monotonic
    /// for the control points we use) and then read off `bezier_y(s)`.
    pub(super) fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }

        // B(s) = 3(1-s)^2 s c1 + 3(1-s) s^2 c2 + s^3, with c0 = 0, c3 = 1.
        let bezier = |c1: f32, c2: f32, s: f32| {
            let inv = 1.0 - s;
            3.0 * inv * inv * s * c1 + 3.0 * inv * s * s * c2 + s * s * s
        };
        // B'(s) = 3(1-s)^2 c1 + 6(1-s) s (c2 - c1) + 3 s^2 (1 - c2).
        let bezier_prime = |c1: f32, c2: f32, s: f32| {
            let inv = 1.0 - s;
            3.0 * inv * inv * c1 + 6.0 * inv * s * (c2 - c1) + 3.0 * s * s * (1.0 - c2)
        };

        let mut s = t;
        for _ in 0..8 {
            let x = bezier(x1, x2, s) - t;
            if x.abs() < 1e-4 {
                break;
            }
            let dx = bezier_prime(x1, x2, s);
            if dx.abs() < 1e-6 {
                break;
            }
            s = (s - x / dx).clamp(0.0, 1.0);
        }

        bezier(y1, y2, s)
    }

    /// Easing for pane collapse/expand: a "fast-out, slow-in" cubic bezier
    /// (the Material standard curve) that reads smoothly in both directions.
    pub(super) fn pane_collapse_ease(t: f32) -> f32 {
        Self::cubic_bezier(0.4, 0.0, 0.2, 1.0, t)
    }

    pub(super) fn animate_sidebar_render_width_to(
        &mut self,
        target: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let start = self.sidebar_render_width;
        let start_f: f32 = start.into();
        let target_f: f32 = target.into();
        self.sidebar_width_anim_seq = self.sidebar_width_anim_seq.wrapping_add(1);
        let seq = self.sidebar_width_anim_seq;
        if (start_f - target_f).abs() <= 0.5 {
            self.sidebar_render_width = target;
            self.sidebar_width_animating = false;
            return;
        }

        if !crate::ui_runtime::current().uses_pane_animations() {
            self.sidebar_render_width = target;
            self.sidebar_width_animating = false;
            self.refresh_main_pane_after_panel_animation(cx);
            cx.notify();
            return;
        }

        self.sidebar_width_animating = true;
        let started = Instant::now();
        let duration = Duration::from_millis(PANE_COLLAPSE_ANIM_MS);

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(16)).await;

                let mut t =
                    started.elapsed().as_secs_f32() / duration.as_secs_f32().max(f32::EPSILON);
                if !t.is_finite() {
                    t = 1.0;
                }
                let t = t.clamp(0.0, 1.0);
                let eased = Self::pane_collapse_ease(t);
                let mut done = t >= 1.0;

                let _ = view.update(cx, |this, cx| {
                    if this.sidebar_width_anim_seq != seq {
                        done = true;
                        return;
                    }

                    let mut changed = false;
                    let next_width = px(start_f + (target_f - start_f) * eased);
                    if this.sidebar_render_width != next_width {
                        this.sidebar_render_width = next_width;
                        changed = true;
                    }
                    if t >= 1.0 {
                        if this.sidebar_render_width != px(target_f) {
                            this.sidebar_render_width = px(target_f);
                        }
                        this.sidebar_width_animating = false;
                        this.refresh_main_pane_after_panel_animation(cx);
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                });

                if done {
                    break;
                }
            },
        )
        .detach();
    }

    pub(super) fn animate_details_render_width_to(
        &mut self,
        target: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let start = self.details_render_width;
        let start_f: f32 = start.into();
        let target_f: f32 = target.into();
        self.details_width_anim_seq = self.details_width_anim_seq.wrapping_add(1);
        let seq = self.details_width_anim_seq;
        if (start_f - target_f).abs() <= 0.5 {
            self.details_render_width = target;
            self.details_width_animating = false;
            return;
        }

        if !crate::ui_runtime::current().uses_pane_animations() {
            self.details_render_width = target;
            self.details_width_animating = false;
            self.refresh_main_pane_after_panel_animation(cx);
            cx.notify();
            return;
        }

        self.details_width_animating = true;
        let started = Instant::now();
        let duration = Duration::from_millis(PANE_COLLAPSE_ANIM_MS);

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(16)).await;

                let mut t =
                    started.elapsed().as_secs_f32() / duration.as_secs_f32().max(f32::EPSILON);
                if !t.is_finite() {
                    t = 1.0;
                }
                let t = t.clamp(0.0, 1.0);
                let eased = Self::pane_collapse_ease(t);
                let mut done = t >= 1.0;

                let _ = view.update(cx, |this, cx| {
                    if this.details_width_anim_seq != seq {
                        done = true;
                        return;
                    }

                    let mut changed = false;
                    let next_width = px(start_f + (target_f - start_f) * eased);
                    if this.details_render_width != next_width {
                        this.details_render_width = next_width;
                        changed = true;
                    }
                    if t >= 1.0 {
                        if this.details_render_width != px(target_f) {
                            this.details_render_width = px(target_f);
                        }
                        this.details_width_animating = false;
                        this.refresh_main_pane_after_panel_animation(cx);
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                });

                if done {
                    break;
                }
            },
        )
        .detach();
    }

    pub(super) fn set_sidebar_collapsed(&mut self, collapsed: bool, cx: &mut gpui::Context<Self>) {
        if self.sidebar_collapsed == collapsed {
            return;
        }

        self.sidebar_collapsed = collapsed;
        // The collapsed-rail popover only exists while collapsed; drop it (and any
        // in-flight fade) instantly when the full sidebar comes back so it can't
        // linger over the expanded pane.
        if !collapsed {
            self.sidebar_collapsed_popover = None;
            self.sidebar_collapsed_popover_closing = None;
            self.sidebar_collapsed_popover_anim_seq =
                self.sidebar_collapsed_popover_anim_seq.wrapping_add(1);
        }
        if matches!(
            self.pane_resize,
            Some(PaneResizeState {
                handle: PaneResizeHandle::Sidebar,
                ..
            })
        ) {
            self.pane_resize = None;
        }
        if !collapsed {
            // Mark the sidebar as animating before clamping: the width reconcile
            // in `clamp_pane_widths_to_window` snaps `sidebar_render_width` to the
            // target whenever it isn't animating, which would collapse the open
            // animation to a single frame (start == target). With the flag set it
            // preserves the current (collapsed) render width so the animation below
            // can grow it out.
            self.sidebar_width_animating = true;
            self.clamp_pane_widths_to_window();
        }

        let target = if collapsed {
            self.pane_collapsed_width()
        } else {
            self.sidebar_width
        };
        self.animate_sidebar_render_width_to(target, cx);
        // Persist so the sidebar reopens in the same state next launch.
        self.schedule_ui_settings_persist(cx);
        cx.notify();
    }

    /// Toggle the collapsed-sidebar popover for `section`. Clicking the icon of
    /// the open section closes it; clicking a different one switches to it and
    /// triggers any lazy data load that section needs.
    pub(in crate::view) fn toggle_sidebar_collapsed_popover(
        &mut self,
        section: CollapsedSidebarSection,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.sidebar_collapsed_popover == Some(section) {
            self.close_sidebar_collapsed_popover(cx);
        } else {
            self.open_sidebar_collapsed_popover(section, cx);
        }
    }

    pub(super) fn open_sidebar_collapsed_popover(
        &mut self,
        section: CollapsedSidebarSection,
        cx: &mut gpui::Context<Self>,
    ) {
        self.sidebar_collapsed_popover = Some(section);
        self.sidebar_collapsed_popover_closing = None;
        self.sidebar_collapsed_popover_anim_seq =
            self.sidebar_collapsed_popover_anim_seq.wrapping_add(1);
        self.sidebar_pane.update(cx, |pane, cx| {
            pane.ensure_collapsed_section_data(section, cx);
        });
        cx.notify();
    }

    /// Begin dismissing the popover: hand the section to `..._closing` so it stays
    /// mounted for the fade-out, then clear it after the fade with a seq-guarded
    /// timer so a fresh open during the fade isn't clobbered.
    pub(in crate::view) fn close_sidebar_collapsed_popover(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(section) = self.sidebar_collapsed_popover.take() else {
            return;
        };
        self.sidebar_collapsed_popover_closing = Some(section);
        self.sidebar_collapsed_popover_anim_seq =
            self.sidebar_collapsed_popover_anim_seq.wrapping_add(1);
        let seq = self.sidebar_collapsed_popover_anim_seq;
        cx.notify();

        // Time the fade-out on the app's executor rather than a bare
        // `smol::Timer`, which would arm the global reactor and fire on its own
        // thread — deterministic under test, identical in the running app.
        let fade_out = cx
            .background_executor()
            .timer(Duration::from_millis(COLLAPSED_POPOVER_FADE_MS));
        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                fade_out.await;
                let _ = view.update(cx, |this, cx| {
                    if this.sidebar_collapsed_popover_anim_seq == seq
                        && this.sidebar_collapsed_popover_closing.is_some()
                    {
                        this.sidebar_collapsed_popover_closing = None;
                        cx.notify();
                    }
                });
            },
        )
        .detach();
    }

    pub(super) fn set_details_collapsed(&mut self, collapsed: bool, cx: &mut gpui::Context<Self>) {
        if self.details_collapsed == collapsed {
            return;
        }

        self.details_collapsed = collapsed;
        if matches!(
            self.pane_resize,
            Some(PaneResizeState {
                handle: PaneResizeHandle::Details,
                ..
            })
        ) {
            self.pane_resize = None;
        }
        if !collapsed {
            // Same reasoning as the sidebar: flag the animation before clamping so
            // the width reconcile preserves the collapsed render width instead of
            // snapping to the target and cancelling the open animation.
            self.details_width_animating = true;
            self.clamp_pane_widths_to_window();
        }

        let target = if collapsed {
            self.pane_collapsed_width()
        } else {
            self.details_width
        };
        self.animate_details_render_width_to(target, cx);
        cx.notify();
    }

    pub(super) fn pane_resize_handle(
        &self,
        theme: AppTheme,
        id: &'static str,
        handle: PaneResizeHandle,
        cx: &gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let collapsed = match handle {
            PaneResizeHandle::Sidebar => self.sidebar_collapsed,
            PaneResizeHandle::Details => self.details_collapsed,
        };
        if collapsed {
            return div().id(id).w(px(0.0)).h_full();
        }

        // Only the details divider shows an idle hairline: it separates two
        // regions inside the content card. The sidebar handle sits on the
        // bare canvas and stays invisible until hovered or dragged.
        let idle_line = matches!(handle, PaneResizeHandle::Details);
        let dragging = self.pane_resize.is_some_and(|state| state.handle == handle);
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .group(id)
            .w(self.pane_resize_handle_width())
            .h_full()
            .cursor(CursorStyle::ResizeLeftRight)
            .child(components::resize_grip(
                theme,
                self.ui_scale_percent,
                id,
                components::ResizeGripAxis::Vertical,
                dragging,
                idle_line.then_some(theme.colors.stroke.subtle),
            ))
            .on_drag(handle, |_handle, _offset, _window, cx| {
                cx.new(|_cx| PaneResizeDragGhost)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                    crate::press_gesture::claim_press(cx);
                    match handle {
                        PaneResizeHandle::Sidebar => {
                            this.sidebar_width_anim_seq =
                                this.sidebar_width_anim_seq.wrapping_add(1);
                            this.sidebar_width_animating = false;
                            this.sidebar_render_width = this.sidebar_width;
                        }
                        PaneResizeHandle::Details => {
                            this.details_width_anim_seq =
                                this.details_width_anim_seq.wrapping_add(1);
                            this.details_width_animating = false;
                            this.details_render_width = this.details_width;
                        }
                    }
                    this.pane_resize = Some(PaneResizeState::new(
                        handle,
                        e.position.x,
                        this.sidebar_width,
                        this.details_width,
                        this.last_window_size.width,
                        this.sidebar_collapsed,
                        this.details_collapsed,
                    ));
                    cx.notify();
                }),
            )
            .on_drag_move(cx.listener(
                move |this, e: &gpui::DragMoveEvent<PaneResizeHandle>, _w, cx| {
                    let Some(state) = this.pane_resize else {
                        return;
                    };
                    if state.handle != *e.drag(cx) {
                        return;
                    }

                    let total_w = this.last_window_size.width;
                    let next_width = next_pane_resize_drag_width(
                        &state,
                        e.event.position.x,
                        total_w,
                        this.sidebar_collapsed,
                        this.details_collapsed,
                    );
                    let mut changed = false;
                    match state.handle {
                        PaneResizeHandle::Sidebar => {
                            if this.sidebar_width != next_width {
                                this.set_sidebar_width_from_pixels(next_width);
                                changed = true;
                            }
                            if this.sidebar_render_width != next_width {
                                this.sidebar_render_width = next_width;
                                changed = true;
                            }
                        }
                        PaneResizeHandle::Details => {
                            if this.details_width != next_width {
                                this.set_details_width_from_pixels(next_width);
                                changed = true;
                            }
                            if this.details_render_width != next_width {
                                this.details_render_width = next_width;
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        cx.notify();
                    }
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    if this.pane_resize.take().is_some() {
                        this.schedule_ui_settings_persist(cx);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    if this.pane_resize.take().is_some() {
                        this.schedule_ui_settings_persist(cx);
                        cx.notify();
                    }
                }),
            )
    }

    pub(super) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(super) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|repo| repo.id == repo_id)
    }

    pub(super) fn drive_focused_mergetool_bootstrap(&mut self) {
        if !self.state.git_runtime.is_available() {
            return;
        }

        let Some(bootstrap) = self.focused_mergetool_bootstrap.as_ref() else {
            return;
        };
        let Some(action) = focused_mergetool_bootstrap_action(&self.state, bootstrap) else {
            return;
        };

        match action {
            FocusedMergetoolBootstrapAction::OpenRepo(path) => {
                self.store.dispatch(Msg::OpenRepo(path))
            }
            FocusedMergetoolBootstrapAction::SetActiveRepo(repo_id) => {
                self.store.dispatch(Msg::SetActiveRepo { repo_id });
            }
            FocusedMergetoolBootstrapAction::SelectConflictDiff { repo_id, path } => {
                self.store
                    .dispatch(Msg::SelectConflictDiff { repo_id, path });
            }
            FocusedMergetoolBootstrapAction::LoadConflictFile { repo_id, path } => {
                self.store.dispatch(Msg::LoadConflictFile {
                    repo_id,
                    path,
                    mode: gitcomet_state::model::ConflictFileLoadMode::CurrentOnly,
                });
            }
            FocusedMergetoolBootstrapAction::Complete => {
                self.focused_mergetool_bootstrap = None;
            }
        }
    }

    pub(super) fn drive_submodule_diff_bootstrap(&mut self) {
        if !self.state.git_runtime.is_available() {
            return;
        }

        let Some(bootstrap) = self.submodule_diff_bootstrap.as_ref() else {
            return;
        };
        let Some(action) = submodule_diff_bootstrap_action(&self.state, bootstrap) else {
            return;
        };

        match action {
            SubmoduleDiffBootstrapAction::OpenRepo(path) => {
                self.store.dispatch(Msg::OpenRepo(path))
            }
            SubmoduleDiffBootstrapAction::SetActiveRepo(repo_id) => {
                self.store.dispatch(Msg::SetActiveRepo { repo_id });
            }
            SubmoduleDiffBootstrapAction::SelectDiff { repo_id, target } => {
                self.store.dispatch(Msg::SelectDiff { repo_id, target });
            }
            SubmoduleDiffBootstrapAction::Complete => {
                self.submodule_diff_bootstrap = None;
            }
        }
    }

    #[cfg(test)]
    pub(super) fn remote_rows(repo: &RepoState) -> Vec<RemoteRow> {
        let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();

        if let Loadable::Ready(remote_branches) = &repo.remote_branches {
            for branch in remote_branches.iter() {
                grouped
                    .entry(branch.remote.clone())
                    .or_default()
                    .push(branch.name.clone());
            }
        }

        if grouped.is_empty()
            && let Loadable::Ready(remotes) = &repo.remotes
        {
            for remote in remotes.iter() {
                grouped.entry(remote.name.clone()).or_default();
            }
        }

        let mut rows = Vec::new();
        for (remote, mut branches) in grouped {
            branches.sort_unstable();
            branches.dedup();
            rows.push(RemoteRow::Header(remote.clone()));
            for name in branches {
                rows.push(RemoteRow::Branch {
                    remote: remote.clone(),
                    name,
                });
            }
        }

        rows
    }

    pub(super) fn show_error_banner(&mut self, repo_id: Option<RepoId>, message: String) {
        if message.trim().is_empty() {
            return;
        }

        if self
            .state
            .banner_error
            .as_ref()
            .is_some_and(|banner| banner.repo_id == repo_id && banner.message == message)
        {
            return;
        }

        self.store
            .dispatch(Msg::ShowBannerError { repo_id, message });
    }

    pub(super) fn split_error_banner_message(
        err_text: &str,
    ) -> (Option<SharedString>, SharedString) {
        let lines: Vec<&str> = err_text.lines().collect();
        let Some(cmd_start) = lines.iter().position(|line| line.starts_with("    git ")) else {
            return (None, err_text.to_string().into());
        };

        let mut cmd_end = cmd_start;
        while cmd_end < lines.len() && lines[cmd_end].starts_with("    ") {
            cmd_end += 1;
        }

        let command = lines[cmd_start..cmd_end]
            .iter()
            .map(|line| line.strip_prefix("    ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");

        let mut body_lines: Vec<String> = Vec::with_capacity(lines.len());
        for line in &lines[..cmd_start] {
            body_lines.push((*line).to_string());
        }
        for line in &lines[cmd_end..] {
            body_lines.push(line.strip_prefix("    ").unwrap_or(line).to_string());
        }

        let mut collapsed: Vec<String> = Vec::with_capacity(body_lines.len());
        let mut prev_blank = false;
        for line in body_lines {
            let blank = line.trim().is_empty();
            if blank && prev_blank {
                continue;
            }
            collapsed.push(line);
            prev_blank = blank;
        }

        (Some(command.into()), collapsed.join("\n").into())
    }

    pub(super) fn should_show_error_banner_overflow_hint(err_text: &str) -> bool {
        err_text.lines().count() > ERROR_BANNER_OVERFLOW_HINT_MIN_LINES
            || err_text.len() > ERROR_BANNER_OVERFLOW_HINT_MIN_CHARS
    }

    pub(super) fn should_render_generic_error_banner(auth_prompt_active: bool) -> bool {
        !auth_prompt_active
    }

    pub(super) fn auth_prompt_banner_colors(theme: AppTheme) -> (gpui::Rgba, gpui::Rgba) {
        (
            with_alpha(theme.colors.accent.foreground, 0.15),
            with_alpha(theme.colors.accent.foreground, 0.3),
        )
    }

    pub(super) fn try_auth_prompt_submit(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(prompt) = self.state.auth_prompt.as_ref() else {
            return;
        };
        let requires_username = prompt.kind == AuthPromptKind::UsernamePassword;
        let secret_required_message = match prompt.kind {
            AuthPromptKind::UsernamePassword => "Password is required.",
            AuthPromptKind::Passphrase => "Passphrase is required.",
            AuthPromptKind::HostVerification => "Confirmation is required (`yes` or fingerprint).",
        };

        let username = self
            .auth_prompt_username_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        let secret = self.auth_prompt_secret_input.read(cx).text().to_string();

        if requires_username && username.is_empty() {
            self.push_toast(
                components::ToastKind::Error,
                "Username is required.".to_string(),
                cx,
            );
            return;
        }
        if secret.trim().is_empty() {
            self.push_toast(
                components::ToastKind::Error,
                secret_required_message.to_string(),
                cx,
            );
            return;
        }

        self.store.dispatch(Msg::SubmitAuthPrompt {
            username: requires_username.then_some(username),
            secret,
        });
        cx.notify();
    }

    pub(super) fn push_toast(
        &mut self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(kind, components::ToastKind::Error) {
            self.show_error_banner(self.active_repo_id(), message);
            return;
        }
        self.toast_host
            .update(cx, |host, cx| host.push_toast(kind, message, cx));
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn push_toast_with_link(
        &mut self,
        kind: components::ToastKind,
        message: String,
        link_url: String,
        link_label: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(kind, components::ToastKind::Error) {
            self.show_error_banner(self.active_repo_id(), message);
            return;
        }
        self.toast_host.update(cx, |host, cx| {
            host.push_toast_with_link(kind, message, link_url, link_label, cx)
        });
    }

    pub(super) fn active_repo_workdir(&self) -> Option<std::path::PathBuf> {
        let repo_id = self.active_repo_id()?;
        self.state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| repo.spec.workdir.clone())
    }

    pub(crate) fn open_active_repo_in_external_code_editor(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(workdir) = self.active_repo_workdir() else {
            self.push_toast(
                components::ToastKind::Error,
                "No active repository to open in code editor.".to_string(),
                cx,
            );
            return;
        };
        self.open_path_in_external_code_editor(workdir, cx);
    }

    pub(in crate::view) fn open_path_in_external_code_editor(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        if !path.exists() {
            self.push_toast(
                components::ToastKind::Error,
                format!("Path not found: {}", path.display()),
                cx,
            );
            return;
        }

        let command = match crate::external_editor::launch_command_for_configured_editor(&path) {
            Ok(command) => command,
            Err(err) => {
                self.push_toast(
                    components::ToastKind::Error,
                    format!("Failed to open in code editor: {err}"),
                    cx,
                );
                return;
            }
        };

        platform_open::spawn_launch(
            cx,
            move || crate::external_editor::spawn_launch_command(command),
            |this, result, cx| {
                if let Err(err) = result {
                    this.push_toast(
                        components::ToastKind::Error,
                        format!("Failed to open in code editor: {err}"),
                        cx,
                    );
                }
            },
        );
    }

    pub(in crate::view) fn startup_crash_report_issue_url(&self) -> Option<String> {
        self.startup_crash_report
            .as_ref()
            .map(|report| report.issue_url.clone())
    }

    /// The "Report Issue" button's whole body, so the button stays a one-liner
    /// and tests can drive the real sequence through
    /// [`Self::report_startup_crash_report_with`].
    pub(super) fn report_startup_crash_report(&mut self, cx: &mut gpui::Context<Self>) {
        self.report_startup_crash_report_with(cx, |url| platform_open::open_url_blocking(&url));
    }

    pub(super) fn report_startup_crash_report_with(
        &mut self,
        cx: &mut gpui::Context<Self>,
        open_url: impl FnOnce(String) -> Result<(), std::io::Error> + Send + 'static,
    ) {
        let Some(url) = self.startup_crash_report_issue_url() else {
            return;
        };
        platform_open::spawn_launch(
            cx,
            move || open_url(url),
            |this, result, cx| {
                match result {
                    Ok(()) => this.push_toast(
                        components::ToastKind::Success,
                        "Opened crash report page in your browser.".to_string(),
                        cx,
                    ),
                    Err(err) => this.push_toast(
                        components::ToastKind::Error,
                        format!("Failed to open browser: {err}"),
                        cx,
                    ),
                }
                // `push_toast` sends an Error straight to `show_error_banner`,
                // which never touches `cx`, so without this the error path would
                // depend entirely on a store round-trip to repaint.
                cx.notify();
            },
        );
    }

    pub(super) fn ignore_startup_crash_report(&mut self) -> Result<(), std::io::Error> {
        let Some(report) = self.startup_crash_report.as_ref() else {
            return Ok(());
        };
        match std::fs::remove_file(&report.crash_log_path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        self.startup_crash_report = None;
        Ok(())
    }

    pub(super) fn defer_text_input_main_pane_action<F>(
        &self,
        cx: &mut gpui::Context<Self>,
        action: F,
    ) where
        F: FnOnce(&mut MainPaneView, &mut Window, &mut gpui::Context<MainPaneView>) -> bool
            + 'static,
    {
        let main_pane = self.main_pane.clone();
        let window_handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                main_pane.update(cx, |pane, cx| {
                    if action(pane, window, cx) {
                        cx.notify();
                        window.refresh();
                    }
                });
            });
        });
    }

    pub(super) fn defer_text_input_adjacent_diff_file_navigation(
        &self,
        direction: i8,
        cx: &mut gpui::Context<Self>,
    ) {
        self.defer_text_input_main_pane_action(cx, move |pane, window, cx| {
            let Some(repo_id) = pane.active_repo_id() else {
                return false;
            };
            pane.try_select_adjacent_diff_file_preserving_focus(repo_id, direction, window, cx)
        });
    }

    pub(super) fn defer_adjacent_diff_file_navigation(
        &self,
        direction: i8,
        cx: &mut gpui::Context<Self>,
    ) {
        self.defer_text_input_main_pane_action(cx, move |pane, window, cx| {
            let Some(repo_id) = pane.active_repo_id() else {
                return false;
            };
            pane.try_select_adjacent_diff_file(repo_id, direction, window, cx)
        });
    }

    /// Mouse back/forward side buttons: step the active repo's global navigation
    /// history (diffs, file content, commit selections). Active anywhere in the
    /// window.
    pub(super) fn dispatch_global_nav(&self, forward: bool, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.main_pane.read(cx).active_repo_id() else {
            return;
        };
        let msg = if forward {
            Msg::GlobalNavForward { repo_id }
        } else {
            Msg::GlobalNavBack { repo_id }
        };
        self.store.dispatch(msg);
        cx.notify();
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn is_popover_open(&self, app: &App) -> bool {
        self.popover_host.read(app).is_open()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn tooltip_host_for_test(&self) -> Entity<TooltipHost> {
        self.tooltip_host.clone()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn tooltip_text_for_test(&self, app: &App) -> Option<SharedString> {
        self.tooltip_host
            .read(app)
            .tooltip_text_for_test()
            .or_else(tooltip::tooltip_text_for_test)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn open_repo_panel_visible_for_test(&self) -> bool {
        self.open_repo_panel
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn show_timezone_for_test(&self) -> bool {
        self.show_timezone
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(in crate::view) fn change_tracking_view_for_test(&self) -> ChangeTrackingView {
        self.change_tracking_view
    }

    #[cfg(test)]
    pub(in crate::view) fn terminal_preferences_for_test(&self) -> &TerminalPreferences {
        &self.terminal_preferences
    }

    pub(super) fn resume_after_git_runtime_recovery(&mut self) {
        if let Some(bootstrap) = self.deferred_repo_bootstrap.take() {
            match bootstrap {
                DeferredRepoBootstrap::RestoreSession {
                    open_repos,
                    active_repo,
                } => {
                    self.startup_repo_bootstrap_pending = true;
                    self.store.dispatch(Msg::RestoreSession {
                        open_repos,
                        active_repo,
                    });
                }
                DeferredRepoBootstrap::OpenRepo(path) => {
                    self.startup_repo_bootstrap_pending = true;
                    self.store.dispatch(Msg::OpenRepo(path));
                }
            }
            return;
        }

        if !self.state.repos.is_empty() {
            let repo_ids: Vec<_> = self.state.repos.iter().map(|repo| repo.id).collect();
            for repo_id in repo_ids {
                self.store.dispatch(Msg::ReloadRepo { repo_id });
            }
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(in crate::view) fn diff_scroll_sync_for_test(&self) -> DiffScrollSync {
        self.diff_scroll_sync
    }

    pub(super) fn set_external_drag_payload(
        &mut self,
        payload: Option<external_drag::ClassifiedExternalPaths>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.external_drag_payload == payload {
            return;
        }

        let folder_active = payload
            .as_ref()
            .is_some_and(|classified| classified.directory().is_some());
        self.external_drag_payload = payload;
        self.repo_tabs_bar.update(cx, |bar, cx| {
            bar.set_external_folder_drag_active(folder_active, cx);
        });
        cx.notify();
    }

    pub(super) fn begin_external_drag_classification(
        &mut self,
        paths: gpui::ExternalPaths,
        repository_bar_already_cleared: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.external_drag_paths.as_ref() == Some(&paths) {
            return;
        }

        self.external_drag_classification_seq =
            self.external_drag_classification_seq.wrapping_add(1);
        let classification_seq = self.external_drag_classification_seq;
        self.external_drag_paths = Some(paths.clone());
        self.external_drag_drop_pending = false;
        if repository_bar_already_cleared {
            self.external_drag_payload = None;
            cx.notify();
        } else {
            self.external_drag_payload = None;
            let show_drop_zone = matches!(paths.paths(), [_]);
            self.repo_tabs_bar.update(cx, |bar, cx| {
                bar.set_external_folder_drag_active(show_drop_zone, cx);
            });
            cx.notify();
        }

        let expected_paths = paths.clone();
        let classification_task =
            cx.background_spawn(
                async move { external_drag::classify_external_paths_blocking(&paths) },
            );
        cx.spawn(async move |view, cx| {
            let classification = classification_task.await;
            let _ = view.update(cx, |this, cx| {
                if this.external_drag_classification_seq != classification_seq
                    || this.external_drag_paths.as_ref() != Some(&expected_paths)
                {
                    return;
                }

                if this.external_drag_drop_pending {
                    this.external_drag_payload = Some(classification);
                    this.finish_external_drag_drop(false, cx);
                } else {
                    this.set_external_drag_payload(Some(classification), cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn clear_external_drag_state(
        &mut self,
        repository_bar_already_cleared: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let changed = self.external_drag_paths.take().is_some()
            || self.external_drag_payload.take().is_some()
            || self.external_drag_drop_pending;
        self.external_drag_drop_pending = false;
        self.external_drag_classification_seq =
            self.external_drag_classification_seq.wrapping_add(1);
        if !repository_bar_already_cleared {
            self.repo_tabs_bar.update(cx, |bar, cx| {
                bar.set_external_folder_drag_active(false, cx);
            });
        }
        if changed {
            cx.notify();
        }
    }

    pub(super) fn finish_external_drag_drop(
        &mut self,
        repository_bar_already_cleared: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let directory = self
            .external_drag_payload
            .as_ref()
            .and_then(external_drag::ClassifiedExternalPaths::directory)
            .cloned();
        self.clear_external_drag_state(repository_bar_already_cleared, cx);
        if let Some(path) = directory {
            self.store.dispatch(Msg::OpenRepoFromExternalDrop(path));
        }
    }

    /// Called by the repository bar after it has cleared its own highlight, so
    /// this must not update that entity re-entrantly from its drop handler.
    pub(in crate::view) fn submit_external_drag_payload_after_repo_drop(
        &mut self,
        paths: gpui::ExternalPaths,
        cx: &mut gpui::Context<Self>,
    ) {
        if !matches!(paths.paths(), [_]) {
            self.clear_external_drag_state(true, cx);
            return;
        }

        if self.external_drag_paths.as_ref() != Some(&paths) {
            self.begin_external_drag_classification(paths, true, cx);
        }
        self.external_drag_drop_pending = true;
        if self.external_drag_payload.is_some() {
            self.finish_external_drag_drop(true, cx);
        } else {
            cx.notify();
        }
    }
}
