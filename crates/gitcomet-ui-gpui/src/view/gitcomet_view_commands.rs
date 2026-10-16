//! Running a command palette or menu command on the window.

use super::{
    AnnexPopoverKind, AnnexPrompt, BranchPickerPurpose, CloseWindow, DecreaseUiScale, DiffArea,
    DiffViewMode, GitCometView, IncreaseUiScale, Loadable, Msg, NewWindow, OpenActiveViewSearch,
    OpenRepository, PopoverKind, PullMode, PullRequest, PushRequest, RemotePopoverKind,
    RepoPopoverKind, ResetUiScale, StashPickerPurpose, SubmodulePopoverKind, ToggleRevealCommit,
    Window, WorktreePopoverKind, command_palette, components, head_branch_has_live_upstream,
    head_is_detached, pull_request, push_request,
};

impl GitCometView {
    pub(super) fn execute_command(
        &mut self,
        command_id: &str,
        window: Option<&mut Window>,
        cx: &mut gpui::Context<Self>,
    ) {
        let action = match command_id {
            "open-repository"
            | "switch-repository"
            | "open-workspace"
            | "clone-repository"
            | "initialize-repository" => super::shell_policy::ShellAction::RepositoryEntry,
            _ if command_palette::COMMANDS
                .iter()
                .any(|command| command.id == command_id && command.requires_repo) =>
            {
                super::shell_policy::ShellAction::Repository
            }
            _ => super::shell_policy::ShellAction::Application,
        };
        if !self.shell_action_allowed(action) {
            return;
        }
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
            "increase-ui-scale" | "decrease-ui-scale" | "reset-ui-scale" => {
                let action: Box<dyn gpui::Action> = match command_id {
                    "increase-ui-scale" => Box::new(IncreaseUiScale),
                    "decrease-ui-scale" => Box::new(DecreaseUiScale),
                    _ => Box::new(ResetUiScale),
                };
                // Zoom is per window: dispatch in the window that ran the command.
                match window {
                    Some(window) => window.dispatch_action(action, cx),
                    None => cx.defer(move |cx| cx.dispatch_action(action.as_ref())),
                }
            }
            "close-window" => cx.defer(|cx| cx.dispatch_action(&CloseWindow)),
            "locate-file-in-explorer" => self.locate_open_file_in_explorer(cx),
            "open-repository" => cx.defer(|cx| cx.dispatch_action(&OpenRepository)),
            "switch-repository" => {
                if let Some(window) = window {
                    self.open_repository_switcher_centered(window, cx);
                }
            }
            "open-workspace" => {
                if let Some(window) = window {
                    self.open_workspace_picker_centered(window, cx);
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
            "fetch-ref" => {
                if let Some(window) = window {
                    self.open_fetch_ref(window, cx);
                }
            }
            "fetch-all" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store
                        .dispatch(Msg::Fetch(gitcomet_state::msg::FetchMsg::All { repo_id }));
                }
            }
            "lfs-download-all" | "lfs-fetch-all" | "lfs-prune" | "lfs-fsck" | "lfs-install" => {
                use gitcomet_core::large_files::LargeFileCommand as C;
                let command = match command_id {
                    "lfs-download-all" => C::LfsPull { paths: Vec::new() },
                    "lfs-fetch-all" => C::LfsFetchAll,
                    "lfs-prune" => C::LfsPrune,
                    "lfs-fsck" => C::LfsFsck,
                    _ => C::LfsInstall,
                };
                if let Some(repo_id) = self.active_repo_id() {
                    self.store
                        .dispatch(Msg::RunLargeFileCommand { repo_id, command });
                }
            }
            "annex-sync"
            | "annex-pull"
            | "annex-push"
            | "annex-get-all"
            | "annex-fsck"
            | "annex-adjust-unlocked"
            | "annex-leave-adjusted"
            | "annex-init"
            | "annex-restage" => {
                use gitcomet_core::large_files::LargeFileCommand as C;
                let content = self.state.large_file_settings.annex_sync_content;
                let base = self
                    .active_repo()
                    .and_then(|repo| repo.annex_adjusted_branch())
                    .map(|(base, _)| base.to_string());
                let command = match command_id {
                    "annex-sync" => Some(C::AnnexSync { content }),
                    "annex-pull" => Some(C::AnnexPull { content }),
                    "annex-push" => Some(C::AnnexPush { content }),
                    "annex-get-all" => Some(C::AnnexGet {
                        paths: vec![std::path::PathBuf::from(".")],
                        from: None,
                    }),
                    "annex-fsck" => Some(C::AnnexFsck),
                    "annex-restage" => Some(C::AnnexRestage),
                    "annex-adjust-unlocked" => Some(C::AnnexAdjust {
                        mode: gitcomet_core::large_files::AnnexAdjustMode::Unlock,
                    }),
                    "annex-leave-adjusted" => base.map(|base| C::AnnexLeaveAdjusted { base }),
                    _ => Some(C::AnnexInit),
                };
                if let (Some(repo_id), Some(command)) = (self.active_repo_id(), command) {
                    self.store
                        .dispatch(Msg::RunLargeFileCommand { repo_id, command });
                }
            }
            "annex-unused" | "annex-webapp" => {
                let prompt = if command_id == "annex-unused" {
                    AnnexPrompt::Unused
                } else {
                    AnnexPrompt::Webapp
                };
                if let (Some(window), Some(repo_id)) = (window, self.active_repo_id()) {
                    self.open_popover_centered(
                        PopoverKind::annex(repo_id, AnnexPopoverKind::Prompt(prompt)),
                        window,
                        cx,
                    );
                }
            }
            "lfs-refresh-locks" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::LoadLfsLocks { repo_id });
                }
            }
            "previous-repo-tab" => {
                self.activate_previous_repo_tab(cx);
            }
            "next-repo-tab" => {
                self.activate_next_repo_tab(cx);
            }
            "open-active-view-search" => cx.defer(|cx| cx.dispatch_action(&OpenActiveViewSearch)),
            // Routed through the action so this takes the same path — and the
            // same availability gate — as the hotkey.
            "reveal-commit" => cx.defer(|cx| cx.dispatch_action(&ToggleRevealCommit)),
            // The app-level `CheckForUpdates` and `InitializeRepository` actions
            // only have handlers on macOS, so these call what the menus call.
            "check-for-updates" => self.check_for_updates_manually(cx),
            "initialize-repository" => {
                if let Some(window) = window
                    && !self.blocks_repository_management_actions()
                {
                    self.prompt_init_repo(window, cx);
                }
            }
            "toggle-terminal" => {
                if let Some(window) = window {
                    self.toggle_terminal_for_active_repo(window, cx);
                }
            }
            "open-external-terminal" => {
                if let (Some(repo_id), Some(window)) = (self.active_repo_id(), window) {
                    self.open_external_terminal_from_menu(repo_id, window, cx);
                }
            }
            "open-in-code-editor" => self.open_active_repo_in_external_code_editor(cx),
            "open-remote-in-browser" => {
                if let Some(window) = window {
                    self.open_remote_in_browser(window, cx);
                }
            }
            "prune-merged-branches" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::PruneMergedBranches { repo_id });
                }
            }
            "prune-local-tags" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::PruneLocalTags { repo_id });
                }
            }
            "push-with-annotated-tags" | "push-with-all-tags" => {
                let mode = if command_id == "push-with-all-tags" {
                    gitcomet_core::tag_push::TagPushMode::All
                } else {
                    gitcomet_core::tag_push::TagPushMode::FollowAnnotated
                };
                if let (Some(repo_id), Some(window)) = (self.active_repo_id(), window) {
                    self.popover_host.update(cx, |host, cx| {
                        host.push_with_tags(repo_id, mode, None, window, cx);
                    });
                }
            }
            // Both abort through the action bar's confirmation.
            "abort-merge" | "abort-rebase" => {
                if let (Some(repo_id), Some(window)) = (self.active_repo_id(), window) {
                    self.open_popover_centered(
                        PopoverKind::MergeAbortConfirm { repo_id },
                        window,
                        cx,
                    );
                }
            }
            "continue-rebase" => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::RebaseContinue { repo_id });
                }
            }
            "hide" => cx.defer(|cx| cx.hide()),
            "hide-others" => cx.defer(|cx| cx.hide_other_apps()),
            "install-desktop-integration" => {
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                self.install_linux_desktop_integration(cx);
            }
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
                let Some(repo) = self.active_repo() else {
                    return;
                };
                let repo_id = repo.id;
                match pull_request(repo, &self.state.large_file_settings) {
                    PullRequest::Pull => self.store.dispatch(Msg::Pull {
                        repo_id,
                        mode: PullMode::Default,
                    }),
                    PullRequest::NoRemotes => self.push_toast(
                        components::ToastKind::Error,
                        "Cannot pull: no remotes configured".to_string(),
                        cx,
                    ),
                    PullRequest::NotReady => {}
                }
            }
            "push" => {
                let Some(repo) = self.active_repo() else {
                    return;
                };
                let repo_id = repo.id;
                match push_request(repo, &self.state.large_file_settings) {
                    PushRequest::Push => self.store.dispatch(Msg::Push { repo_id }),
                    PushRequest::SetUpstream { remote } => {
                        if let Some(window) = window {
                            self.open_popover_centered(
                                PopoverKind::PushSetUpstreamPrompt {
                                    repo_id,
                                    remote,
                                    configure_only_for: None,
                                },
                                window,
                                cx,
                            );
                        }
                    }
                    PushRequest::NoRemotes => self.push_toast(
                        components::ToastKind::Error,
                        "Cannot push: no remotes configured".to_string(),
                        cx,
                    ),
                    PushRequest::NotReady => {}
                }
            }
            "force-push" => {
                let Some(repo) = self.active_repo() else {
                    return;
                };
                let repo_id = repo.id;
                if !head_branch_has_live_upstream(repo) {
                    let reason = if head_is_detached(repo) {
                        "HEAD is detached"
                    } else {
                        "current branch has no remote upstream"
                    };
                    self.push_toast(
                        components::ToastKind::Error,
                        format!("Cannot force push: {reason}"),
                        cx,
                    );
                    return;
                }
                if let Some(window) = window {
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
                crate::view::status_actions::stage_or_unstage_paths(
                    &self.store,
                    repo_id,
                    DiffArea::Unstaged,
                    paths,
                );
            }
            "unstage-all" => {
                // An empty list unstages from the index, like the Unstage all
                // button; the status list omits a staged rename's source path.
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id)
                    && repo
                        .staged_status_entries()
                        .is_some_and(|entries| !entries.is_empty())
                {
                    crate::view::status_actions::stage_or_unstage_paths(
                        &self.store,
                        repo_id,
                        DiffArea::Staged,
                        Vec::new(),
                    );
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
            "back" => self.dispatch_global_nav(false, cx),
            "forward" => self.dispatch_global_nav(true, cx),
            other => {
                if let Some(id) = super::extension_host::command_id_from_palette(other) {
                    self.run_extension_command(id, cx);
                }
            }
        }
    }
}
