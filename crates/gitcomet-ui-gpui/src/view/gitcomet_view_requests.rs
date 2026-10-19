//! What popovers ask their window's root view to do. A popover sends a
//! `RootRequest` value through its weak root handle instead of calling root
//! methods, so the root's API toward popovers is this one enum.

use super::{
    ChangeTrackingView, ErrorReport, FilesystemUnsavedEditsChoice, GitCometView, PopoverKind,
    RepoId, SharedString, TerminalShutdownAction, TerminalShutdownPrompt, ThemeMode,
    UnsavedFileEditsAction, WeakEntity, Window, components,
};
use crate::view::date_time::{DateTimeFormat, Timezone};
use gitcomet_core::filesystem::ConflictChoice;
use gitcomet_core::git_operation::GitOperationId;
use std::path::PathBuf;

/// A request from a popover to the root view of its window.
pub(in crate::view) enum RootRequest {
    PromptOpenRepo,
    PromptInitRepo,
    LocateOpenFileInExplorer,
    OpenInExternalEditor(PathBuf),
    OpenReflogPanel(RepoId),
    CheckForUpdates,
    RunExtensionCommand {
        id: String,
        repo: Option<RepoId>,
    },
    PerformCloseAction(TerminalShutdownAction),
    ShowRepositoryCanvas,
    OpenRepoPath(PathBuf),
    CloseRepos {
        repo_ids: Vec<RepoId>,
        activate_after: Option<RepoId>,
    },
    MoveRepoToWorkspace {
        repo_id: RepoId,
        path: PathBuf,
        target: Option<gitcomet_state::session::WorkspaceId>,
    },
    SetChangeTrackingView(ChangeTrackingView),
    SetCommitAmend(bool),
    SetCommitPushAfter(bool),
    OpenPopoverCentered(PopoverKind),
    OpenFileHistory {
        repo_id: RepoId,
        path: PathBuf,
    },
    ResolveDeletePermanently {
        prompt_id: u64,
        confirmed: bool,
    },
    ReviewLocalChanges(RepoId),
    ReportError(ErrorReport),
    ResolveFilesystemConflict {
        prompt_id: u64,
        choice: ConflictChoice,
        apply_to_all: bool,
    },
    ResolveFilesystemUnsavedEdits {
        prompt_id: u64,
        choice: FilesystemUnsavedEditsChoice,
    },
    RegisterPendingWorktreeBranchRemoval {
        repo_id: RepoId,
        path: PathBuf,
        branch: String,
    },
    SetActiveContextMenuInvoker(Option<SharedString>),
    MinimizeHookActivity {
        repo_id: RepoId,
        active_chains: Vec<(RepoId, GitOperationId)>,
    },
    ResumeHookActivityAutoOpen(RepoId),
    ClearPendingUnsavedFileEditsPrompt,
    SyncTitlebarMenus {
        app_menu_open: bool,
        repo_picker_open: bool,
    },
    /// Whether the next force-delete prompt for a branch opens centered.
    SetForceDeleteBranchCentered(bool),
    /// The popover changed display preferences; record and persist them.
    PersistDisplayPreferences {
        theme_mode: ThemeMode,
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        show_timezone: bool,
    },
    /// The popover closed; these say what the closing dismissed.
    PopoverClosed {
        dismissing_unsaved_prompt: bool,
        dismissing_hook_activity: bool,
        dismissed_filesystem_prompt: Option<u64>,
    },
    FilesystemDialogDisplaced(u64),
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    InstallLinuxDesktopIntegration,
    Toast(components::ToastKind, String),
    ClearPendingTerminalShutdownPrompt,
    ConfirmTerminalShutdown(TerminalShutdownPrompt),
    ResolveUnsavedFileEdits {
        action: UnsavedFileEditsAction,
        save: bool,
    },
}

/// Sends requests to a root view through a weak handle.
pub(in crate::view) trait RootRequests {
    /// Runs `request` on the root now; false when the root is gone.
    /// Requests that act on the window need `window`.
    fn request(
        &self,
        request: RootRequest,
        window: Option<&mut Window>,
        cx: &mut gpui::App,
    ) -> bool;
}

impl RootRequests for WeakEntity<GitCometView> {
    fn request(
        &self,
        request: RootRequest,
        window: Option<&mut Window>,
        cx: &mut gpui::App,
    ) -> bool {
        self.update(cx, |root, cx| root.handle_root_request(request, window, cx))
            .is_ok()
    }
}

impl GitCometView {
    fn handle_root_request(
        &mut self,
        request: RootRequest,
        window: Option<&mut Window>,
        cx: &mut gpui::Context<Self>,
    ) {
        match request {
            RootRequest::LocateOpenFileInExplorer => self.locate_open_file_in_explorer(cx),
            RootRequest::OpenInExternalEditor(path) => {
                self.open_path_in_external_code_editor(path, cx)
            }
            RootRequest::OpenReflogPanel(repo_id) => self.open_reflog_panel(repo_id, cx),
            RootRequest::CheckForUpdates => self.check_for_updates_manually(cx),
            RootRequest::RunExtensionCommand { id, repo: None } => {
                self.run_extension_command(&id, cx);
            }
            RootRequest::RunExtensionCommand { id, repo } => {
                self.run_extension_command_for(&id, repo, cx);
            }
            RootRequest::ShowRepositoryCanvas => self.show_repository_canvas(cx),
            RootRequest::OpenRepoPath(path) => self.open_repo_path(path, cx),
            RootRequest::CloseRepos {
                repo_ids,
                activate_after,
            } => self.request_close_repos(repo_ids, activate_after, cx),
            RootRequest::MoveRepoToWorkspace {
                repo_id,
                path,
                target,
            } => self.request_move_repo_to_workspace(repo_id, path, target, cx),
            RootRequest::SetChangeTrackingView(view) => self.set_change_tracking_view(view, cx),
            RootRequest::SetCommitAmend(enabled) => self.set_commit_amend_enabled(enabled, cx),
            RootRequest::SetCommitPushAfter(enabled) => {
                self.set_commit_push_after_enabled(enabled, cx)
            }
            RootRequest::ReportError(report) => self.report_error(report, cx),
            RootRequest::RegisterPendingWorktreeBranchRemoval {
                repo_id,
                path,
                branch,
            } => self.register_pending_worktree_branch_removal(repo_id, path, branch),
            RootRequest::SetActiveContextMenuInvoker(next) => {
                self.set_active_context_menu_invoker(next, cx)
            }
            RootRequest::MinimizeHookActivity {
                repo_id,
                active_chains,
            } => self.minimize_hook_activity_repo(repo_id, active_chains, cx),
            RootRequest::ResumeHookActivityAutoOpen(repo_id) => {
                self.resume_hook_activity_auto_open(repo_id, cx)
            }
            RootRequest::ClearPendingUnsavedFileEditsPrompt => {
                self.clear_pending_unsaved_file_edits_prompt(cx)
            }
            RootRequest::SyncTitlebarMenus {
                app_menu_open,
                repo_picker_open,
            } => self.title_bar.update(cx, |title_bar, cx| {
                title_bar.set_app_menu_open(app_menu_open, cx);
                title_bar.set_repo_picker_open(repo_picker_open, cx);
            }),
            RootRequest::SetForceDeleteBranchCentered(centered) => {
                self.pending_force_delete_branch_centered = centered;
            }
            RootRequest::PersistDisplayPreferences {
                theme_mode,
                date_time_format,
                timezone,
                show_timezone,
            } => {
                self.theme_mode = theme_mode;
                self.date_time_format = date_time_format;
                self.timezone = timezone;
                self.show_timezone = show_timezone;
                self.schedule_ui_settings_persist(cx);
            }
            RootRequest::PopoverClosed {
                dismissing_unsaved_prompt,
                dismissing_hook_activity,
                dismissed_filesystem_prompt,
            } => {
                if dismissing_unsaved_prompt {
                    self.clear_pending_unsaved_file_edits_prompt(cx);
                }
                if dismissing_hook_activity {
                    self.set_hook_activity_dialog_repo(None, cx);
                }
                // Esc and the scrim cancel; buttons resolved before closing.
                if let Some(prompt_id) = dismissed_filesystem_prompt {
                    self.filesystem_dialog_dismissed(prompt_id, cx);
                }
                self.set_history_refs_hover_item_menu_open(false, cx);
                // A file-operation dialog may be waiting for this one to close.
                self.notify_if_filesystem_dialog_queued(cx);
            }
            RootRequest::FilesystemDialogDisplaced(prompt_id) => {
                self.filesystem_dialog_displaced(prompt_id, cx)
            }
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            RootRequest::InstallLinuxDesktopIntegration => {
                self.install_linux_desktop_integration(cx)
            }
            RootRequest::Toast(kind, message) => self.push_toast(kind, message, cx),
            RootRequest::ClearPendingTerminalShutdownPrompt => {
                self.clear_pending_terminal_shutdown_prompt(cx)
            }
            RootRequest::ResolveUnsavedFileEdits { action, save } => {
                self.resolve_unsaved_file_edits(action, save, cx)
            }
            windowed => {
                let Some(window) = window else {
                    debug_assert!(false, "this root request needs its window");
                    return;
                };
                self.handle_windowed_root_request(windowed, window, cx);
            }
        }
    }

    fn handle_windowed_root_request(
        &mut self,
        request: RootRequest,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match request {
            RootRequest::PromptOpenRepo => self.prompt_open_repo(window, cx),
            RootRequest::PromptInitRepo => self.prompt_init_repo(window, cx),
            RootRequest::PerformCloseAction(action) => {
                self.perform_close_action(action, window, cx)
            }
            RootRequest::OpenPopoverCentered(kind) => self.open_popover_centered(kind, window, cx),
            RootRequest::OpenFileHistory { repo_id, path } => {
                self.open_file_history(repo_id, path, window, cx)
            }
            RootRequest::ResolveDeletePermanently {
                prompt_id,
                confirmed,
            } => self.resolve_delete_permanently(prompt_id, confirmed, window, cx),
            RootRequest::ReviewLocalChanges(repo_id) => {
                self.review_local_changes(repo_id, window, cx)
            }
            RootRequest::ResolveFilesystemConflict {
                prompt_id,
                choice,
                apply_to_all,
            } => self.resolve_filesystem_conflict(prompt_id, choice, apply_to_all, window, cx),
            RootRequest::ResolveFilesystemUnsavedEdits { prompt_id, choice } => {
                self.resolve_filesystem_unsaved_edits(prompt_id, choice, window, cx)
            }
            RootRequest::ConfirmTerminalShutdown(prompt) => {
                self.confirm_terminal_shutdown(prompt, window, cx)
            }
            other => self.handle_root_request(other, None, cx),
        }
    }
}
