//! Running a command palette or menu command on the window.

use super::commands::Command;
use super::{
    AnnexPopoverKind, AnnexPrompt, BranchPickerPurpose, CloseWindow, DecreaseUiScale, DiffArea,
    DiffViewMode, GitCometView, IncreaseUiScale, Loadable, Msg, NewWindow, OpenActiveViewSearch,
    OpenRepository, PopoverKind, PullMode, PullRequest, PushRequest, RemotePopoverKind,
    RepoPopoverKind, ResetUiScale, StashPickerPurpose, SubmodulePopoverKind, ToggleRevealCommit,
    Window, WorktreePopoverKind, command_palette, components, head_branch_has_live_upstream,
    head_is_detached, pull_request, push_request,
};

impl GitCometView {
    /// Runs the built-in or extension command with palette id `command_id`.
    pub(super) fn execute_command(
        &mut self,
        command_id: &str,
        window: Option<&mut Window>,
        cx: &mut gpui::Context<Self>,
    ) {
        match Command::from_id(command_id) {
            Some(command) => self.execute_builtin_command(command, window, cx),
            None => {
                if self.shell_action_allowed(super::shell_policy::ShellAction::Application)
                    && let Some(id) = super::extension_host::command_id_from_palette(command_id)
                {
                    self.run_extension_command(id, cx);
                }
            }
        }
    }

    fn execute_builtin_command(
        &mut self,
        command: Command,
        window: Option<&mut Window>,
        cx: &mut gpui::Context<Self>,
    ) {
        let action = match command {
            Command::OpenRepository
            | Command::SwitchRepository
            | Command::OpenWorkspace
            | Command::CloneRepository
            | Command::InitializeRepository => super::shell_policy::ShellAction::RepositoryEntry,
            _ if command_palette::COMMANDS
                .iter()
                .any(|entry| entry.id == command.id() && entry.requires_repo) =>
            {
                super::shell_policy::ShellAction::Repository
            }
            _ => super::shell_policy::ShellAction::Application,
        };
        if !self.shell_action_allowed(action) {
            return;
        }
        if let Some(entry) = command_palette::COMMANDS
            .iter()
            .find(|entry| entry.id == command.id())
            && let Some(reason) = command_palette::command_unavailable_reason(
                entry,
                &self.command_palette_context(cx),
            )
        {
            self.push_toast(components::ToastKind::Warning, reason.to_owned(), cx);
            return;
        }
        match command {
            Command::NewWindow => cx.defer(|cx| cx.dispatch_action(&NewWindow)),
            Command::OpenSettings => cx.defer(crate::view::open_settings_window),
            Command::Quit => cx.defer(crate::app::quit_app_or_warn),
            Command::MinimizeWindow => cx.defer(|cx| {
                if let Some(win) = cx.active_window() {
                    let _ = win.update(cx, |_root, win, _cx| win.minimize_window());
                }
            }),
            Command::ZoomWindow => cx.defer(|cx| {
                if let Some(win) = cx.active_window() {
                    let _ = win.update(cx, |_root, win, _cx| {
                        crate::kit::window_chrome::toggle_window_zoom(win)
                    });
                }
            }),
            Command::ToggleFullscreen => cx.defer(|cx| {
                if let Some(win) = cx.active_window() {
                    let _ = win.update(cx, |_root, win, _cx| win.toggle_fullscreen());
                }
            }),
            Command::IncreaseUiScale | Command::DecreaseUiScale | Command::ResetUiScale => {
                let action: Box<dyn gpui::Action> = match command {
                    Command::IncreaseUiScale => Box::new(IncreaseUiScale),
                    Command::DecreaseUiScale => Box::new(DecreaseUiScale),
                    _ => Box::new(ResetUiScale),
                };
                // Zoom is per window: dispatch in the window that ran the command.
                match window {
                    Some(window) => window.dispatch_action(action, cx),
                    None => cx.defer(move |cx| cx.dispatch_action(action.as_ref())),
                }
            }
            Command::CloseWindow => cx.defer(|cx| cx.dispatch_action(&CloseWindow)),
            Command::LocateFileInExplorer => self.locate_open_file_in_explorer(cx),
            Command::OpenRepository => cx.defer(|cx| cx.dispatch_action(&OpenRepository)),
            Command::SwitchRepository => {
                if let Some(window) = window {
                    self.open_repository_switcher_centered(window, cx);
                }
            }
            Command::OpenWorkspace => {
                if let Some(window) = window {
                    self.open_workspace_picker_centered(window, cx);
                }
            }
            Command::CloneRepository => {
                if let Some(window) = window {
                    self.open_popover_centered(PopoverKind::CloneRepo, window, cx);
                }
            }
            Command::CloseRepoTab => {
                self.close_active_repo_tab(cx);
            }
            Command::ReloadRepository => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::ReloadRepo { repo_id });
                }
            }
            Command::FetchRef => {
                if let Some(window) = window {
                    self.open_fetch_ref(window, cx);
                }
            }
            Command::FetchAll => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store
                        .dispatch(Msg::Fetch(gitcomet_state::msg::FetchMsg::All { repo_id }));
                }
            }
            Command::LfsDownloadAll
            | Command::LfsFetchAll
            | Command::LfsPrune
            | Command::LfsFsck
            | Command::LfsInstall => {
                use gitcomet_core::large_files::LargeFileCommand as C;
                let command = match command {
                    Command::LfsDownloadAll => C::LfsPull { paths: Vec::new() },
                    Command::LfsFetchAll => C::LfsFetchAll,
                    Command::LfsPrune => C::LfsPrune,
                    Command::LfsFsck => C::LfsFsck,
                    _ => C::LfsInstall,
                };
                if let Some(repo_id) = self.active_repo_id() {
                    self.store
                        .dispatch(Msg::RunLargeFileCommand { repo_id, command });
                }
            }
            Command::AnnexSync
            | Command::AnnexPull
            | Command::AnnexPush
            | Command::AnnexGetAll
            | Command::AnnexFsck
            | Command::AnnexAdjustUnlocked
            | Command::AnnexLeaveAdjusted
            | Command::AnnexInit
            | Command::AnnexRestage => {
                use gitcomet_core::large_files::LargeFileCommand as C;
                let content = self.state.large_file_settings.annex_sync_content;
                let base = self
                    .active_repo()
                    .and_then(|repo| repo.annex_adjusted_branch())
                    .map(|(base, _)| base.to_string());
                let command = match command {
                    Command::AnnexSync => Some(C::AnnexSync { content }),
                    Command::AnnexPull => Some(C::AnnexPull { content }),
                    Command::AnnexPush => Some(C::AnnexPush { content }),
                    Command::AnnexGetAll => Some(C::AnnexGet {
                        paths: vec![std::path::PathBuf::from(".")],
                        from: None,
                    }),
                    Command::AnnexFsck => Some(C::AnnexFsck),
                    Command::AnnexRestage => Some(C::AnnexRestage),
                    Command::AnnexAdjustUnlocked => Some(C::AnnexAdjust {
                        mode: gitcomet_core::large_files::AnnexAdjustMode::Unlock,
                    }),
                    Command::AnnexLeaveAdjusted => base.map(|base| C::AnnexLeaveAdjusted { base }),
                    _ => Some(C::AnnexInit),
                };
                if let (Some(repo_id), Some(command)) = (self.active_repo_id(), command) {
                    self.store
                        .dispatch(Msg::RunLargeFileCommand { repo_id, command });
                }
            }
            Command::AnnexUnused | Command::AnnexWebapp => {
                let prompt = if command == Command::AnnexUnused {
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
            Command::LfsRefreshLocks => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::LoadLfsLocks { repo_id });
                }
            }
            Command::PreviousRepoTab => {
                self.activate_previous_repo_tab(cx);
            }
            Command::NextRepoTab => {
                self.activate_next_repo_tab(cx);
            }
            Command::OpenActiveViewSearch => {
                cx.defer(|cx| cx.dispatch_action(&OpenActiveViewSearch))
            }
            // Routed through the action so this takes the same path — and the
            // same availability gate — as the hotkey.
            Command::RevealCommit => cx.defer(|cx| cx.dispatch_action(&ToggleRevealCommit)),
            // The app-level `CheckForUpdates` and `InitializeRepository` actions
            // only have handlers on macOS, so these call what the menus call.
            Command::CheckForUpdates => self.check_for_updates_manually(cx),
            Command::InitializeRepository => {
                if let Some(window) = window
                    && !self.blocks_repository_management_actions()
                {
                    self.prompt_init_repo(window, cx);
                }
            }
            Command::ToggleTerminal => {
                if let Some(window) = window {
                    self.toggle_terminal_for_active_repo(window, cx);
                }
            }
            Command::OpenExternalTerminal => {
                if let (Some(repo_id), Some(window)) = (self.active_repo_id(), window) {
                    self.open_external_terminal_from_menu(repo_id, window, cx);
                }
            }
            Command::OpenInCodeEditor => self.open_active_repo_in_external_code_editor(cx),
            Command::OpenRemoteInBrowser => {
                if let Some(window) = window {
                    self.open_remote_in_browser(window, cx);
                }
            }
            Command::PruneMergedBranches => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::PruneMergedBranches { repo_id });
                }
            }
            Command::PruneLocalTags => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::PruneLocalTags { repo_id });
                }
            }
            Command::PushWithAnnotatedTags | Command::PushWithAllTags => {
                let mode = if command == Command::PushWithAllTags {
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
            Command::AbortMerge | Command::AbortRebase => {
                if let (Some(repo_id), Some(window)) = (self.active_repo_id(), window) {
                    self.open_popover_centered(
                        PopoverKind::MergeAbortConfirm { repo_id },
                        window,
                        cx,
                    );
                }
            }
            Command::ContinueRebase => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::RebaseContinue { repo_id });
                }
            }
            Command::Hide => cx.defer(|cx| cx.hide()),
            Command::HideOthers => cx.defer(|cx| cx.hide_other_apps()),
            Command::InstallDesktopIntegration => {
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                self.install_linux_desktop_integration(cx);
            }
            Command::ToggleSidebar => {
                self.set_sidebar_collapsed(!self.sidebar_collapsed, cx);
            }
            Command::ToggleDetails => {
                self.set_details_collapsed(!self.details_collapsed, cx);
            }
            Command::ToggleDiffView => {
                let next = match self.diff_view_mode {
                    DiffViewMode::Split => DiffViewMode::Inline,
                    DiffViewMode::Inline => DiffViewMode::Split,
                };
                self.set_diff_view_mode(next, cx);
            }
            Command::ToggleDiffWordWrap => {
                self.set_diff_word_wrap(!self.diff_word_wrap, cx);
            }
            Command::ToggleLineNumbers => {
                self.set_diff_show_line_numbers(!self.diff_show_line_numbers, cx);
            }
            Command::ToggleWhitespaceChars => {
                self.set_diff_reveal_whitespace_chars(!self.diff_reveal_whitespace_chars, cx);
            }
            Command::CreateBranch => {
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
            Command::CheckoutBranch => {
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
            Command::DeleteBranch => {
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
            Command::RenameBranch => {
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
            Command::Pull => {
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
            Command::Push => {
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
            Command::ForcePush => {
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
            Command::Commit => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    self.open_popover_centered(PopoverKind::CommitPrompt { repo_id }, window, cx);
                }
            }
            Command::ApplyPatch => {
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
                        directory: None,
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
            Command::StageAll => {
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
            Command::UnstageAll => {
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
            Command::Stash => {
                if let Some(window) = window {
                    self.open_popover_centered(PopoverKind::StashPrompt, window, cx);
                }
            }
            Command::StashPop | Command::StashApply | Command::StashDrop => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    let purpose = match command {
                        Command::StashPop => StashPickerPurpose::Pop,
                        Command::StashApply => StashPickerPurpose::Apply,
                        _ => StashPickerPurpose::Drop,
                    };
                    self.open_popover_centered(
                        PopoverKind::StashPickerPrompt { repo_id, purpose },
                        window,
                        cx,
                    );
                }
            }
            Command::Rebase => {
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
            Command::CreateTag => {
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
            Command::ShowReflog => {
                self.open_reflog_panel_for_active_repo(cx);
            }
            Command::FileHistory => {
                if let Some(repo_id) = self.active_repo_id()
                    && let Some(window) = window
                {
                    self.open_popover_centered(
                        PopoverKind::FileHistoryFilePicker { repo_id },
                        window,
                        cx,
                    );
                }
            }
            Command::AddRemote => {
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
            Command::AddSubmodule => {
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
            Command::UpdateSubmodules => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.store.dispatch(Msg::UpdateSubmodules { repo_id });
                }
            }
            Command::AddWorktree => {
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
            Command::Blame => {
                self.set_annotate_enabled(!self.annotate_enabled, cx);
            }
            Command::Back => self.dispatch_global_nav(false, cx),
            Command::Forward => self.dispatch_global_nav(true, cx),
        }
    }
}
