use super::*;
use gitcomet_core::domain::{FileSource, RepositoryRequirement, WORKING_TREE_REQUIRED_MESSAGE};

impl PopoverKind {
    pub(in crate::view) fn working_tree_repository(
        &self,
        active: Option<RepoId>,
    ) -> Option<RepoId> {
        match self {
            Self::CheckoutRemoteBranchPrompt { repo_id, .. }
            | Self::CommitPrompt { repo_id }
            | Self::StashDropConfirm { repo_id, .. }
            | Self::StashPickerPrompt { repo_id, .. }
            | Self::CherryPickCommitConfirm { repo_id, .. }
            | Self::InteractiveCherryPickConfirm { repo_id, .. }
            | Self::RevertCommitConfirm { repo_id, .. }
            | Self::ApplyFileChangeConfirm { repo_id, .. }
            | Self::MergeCommitConfirm { repo_id, .. }
            | Self::MergeAbortConfirm { repo_id }
            | Self::DiscardChangesConfirm { repo_id, .. }
            | Self::AddToGitignorePrompt { repo_id, .. }
            | Self::StageConflictMarkersConfirm { repo_id, .. }
            | Self::PullReconcilePrompt { repo_id, .. }
            | Self::DiscardFolderChangesConfirm { repo_id, .. }
            | Self::RebaseOntoConfirm { repo_id, .. } => Some(*repo_id),
            Self::BranchPicker {
                purpose: BranchPickerPurpose::Checkout | BranchPickerPurpose::RebaseOnto,
            }
            | Self::StashPrompt
            | Self::RebaseReword { .. } => active,
            Self::ResetPrompt { repo_id, mode, .. } => match mode {
                ResetMode::Soft => None,
                ResetMode::Mixed | ResetMode::Hard => Some(*repo_id),
            },
            Self::BranchExistsPrompt {
                repo_id, operation, ..
            } => match operation {
                BranchExistsPromptOperation::CreateBranch
                | BranchExistsPromptOperation::CheckoutRemoteBranch { .. } => Some(*repo_id),
                BranchExistsPromptOperation::RenameBranch { .. } => None,
            },
            Self::Repo { repo_id, kind } => match kind {
                RepoPopoverKind::Submodule(
                    SubmodulePopoverKind::AddPrompt
                    | SubmodulePopoverKind::ChangePointerPrompt { .. }
                    | SubmodulePopoverKind::TrustConfirm
                    | SubmodulePopoverKind::OpenPicker
                    | SubmodulePopoverKind::RemovePicker
                    | SubmodulePopoverKind::RemoveConfirm { .. },
                ) => Some(*repo_id),
                RepoPopoverKind::Annex(AnnexPopoverKind::Prompt(AnnexPrompt::Webapp)) => {
                    Some(*repo_id)
                }
                RepoPopoverKind::Annex(AnnexPopoverKind::Prompt(AnnexPrompt::ForceDrop {
                    paths,
                })) => (!paths.is_empty()).then_some(*repo_id),
                RepoPopoverKind::RepositoryModeMenu
                | RepoPopoverKind::DisableBareModeConfirm
                | RepoPopoverKind::Remote(_)
                | RepoPopoverKind::Worktree(_)
                | RepoPopoverKind::Submodule(
                    SubmodulePopoverKind::SectionMenu | SubmodulePopoverKind::Menu { .. },
                )
                | RepoPopoverKind::Annex(
                    AnnexPopoverKind::SectionMenu
                    | AnnexPopoverKind::RepositoryMenu { .. }
                    | AnnexPopoverKind::Prompt(
                        AnnexPrompt::AddSpecialRemote
                        | AnnexPrompt::EnableSpecialRemote { .. }
                        | AnnexPrompt::Describe { .. }
                        | AnnexPrompt::Numcopies { .. }
                        | AnnexPrompt::Trust { .. }
                        | AnnexPrompt::Unused,
                    ),
                ) => None,
            },
            Self::Hosted { .. }
            | Self::HookActivity { .. }
            | Self::ErrorDetails { .. }
            | Self::RepoPicker { .. }
            | Self::BranchPicker {
                purpose: BranchPickerPurpose::Delete,
            }
            | Self::UpstreamPicker { .. }
            | Self::CreateBranchFromRefPrompt { .. }
            | Self::RenameBranchPrompt { .. }
            | Self::FileHistoryFilePicker { .. }
            | Self::StashMenu { .. }
            | Self::CloneRepo
            | Self::SquashPrompt { .. }
            | Self::CreateTagPrompt { .. }
            | Self::ReflogEntryMenu { .. }
            | Self::FileHistoryCommitMenu { .. }
            | Self::PushSetUpstreamPrompt { .. }
            | Self::ForcePushConfirm { .. }
            | Self::ForceDeleteBranchConfirm { .. }
            | Self::ForceRemoveWorktreeConfirm { .. }
            | Self::PullPicker
            | Self::PushPicker
            | Self::CommitOptionsMenu { .. }
            | Self::CommitFileSortMenu { .. }
            | Self::FileListLayoutMenu { .. }
            | Self::FileListFolderMenu { .. }
            | Self::PreviousCommitMessagesMenu { .. }
            | Self::RepoTabMenu { .. }
            | Self::AppMenu
            | Self::AddRepoMenu
            | Self::TerminalShutdownConfirm(_)
            | Self::UnsavedFileEditsConfirm(_)
            | Self::FilesystemConflict(_)
            | Self::DeletePermanentlyConfirm(_)
            | Self::FilesystemUnsavedEditsConfirm(_)
            | Self::CloseGuardConfirm(_)
            | Self::TerminalMenu { .. }
            | Self::DiffActionMenu
            | Self::MergetoolSettingsMenu
            | Self::DiffHunkMenu { .. }
            | Self::WebLinkMenu { .. }
            | Self::LocalFileLinkMenu { .. }
            | Self::CommitShaLinkMenu { .. }
            | Self::DiffEditorMenu { .. }
            | Self::ConflictResolverInputRowMenu { .. }
            | Self::ConflictResolverChunkMenu { .. }
            | Self::ConflictResolverOutputMenu { .. }
            | Self::CommitMenu { .. }
            | Self::StatusFileMenu { .. }
            | Self::StatusConflictMenu { .. }
            | Self::BranchMenu { .. }
            | Self::BranchSectionMenu { .. }
            | Self::BranchGroupMenu { .. }
            | Self::SidebarPinnedOverflow { .. }
            | Self::SidebarAncestorMenu { .. }
            | Self::DeleteBranchesConfirm { .. }
            | Self::CommitFileMenu { .. }
            | Self::WorktreeFileMenu { .. }
            | Self::CommitRangeFileMenu { .. }
            | Self::FileBrowserFileMenu { .. }
            | Self::FileBrowserFolderMenu { .. }
            | Self::ExplorerSettingsMenu { .. }
            | Self::BrowseHistoryMenu { .. }
            | Self::SubmoduleInnerDiffMenu { .. }
            | Self::TagMenu { .. }
            | Self::HistoryBranchFilter { .. }
            | Self::HistoryAuthorFilter { .. }
            | Self::DiffContentModeSettings
            | Self::TextFormatMenu { .. }
            | Self::ChangeTrackingSettings
            | Self::UiScalePicker
            | Self::InteractiveRebaseActionMenu { .. }
            | Self::InteractiveRebaseAutosquashMenu => None,
        }
    }
}

impl PopoverHost {
    pub(super) fn explain_working_tree_required(&self, cx: &mut gpui::Context<Self>) {
        let root = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root.request(
                RootRequest::Toast(
                    components::ToastKind::Warning,
                    WORKING_TREE_REQUIRED_MESSAGE.to_owned(),
                ),
                None,
                cx,
            );
        });
    }

    pub(super) fn close_working_tree_prompt_in_bare_mode(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self
            .popover
            .as_ref()
            .and_then(|kind| kind.working_tree_repository(self.active_repo_id()))
        else {
            return;
        };
        if self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(RepoState::is_bare)
        {
            self.close_popover(cx);
            self.explain_working_tree_required(cx);
        }
    }

    pub(in crate::view) fn bare_action_reason(
        &self,
        action: &ContextMenuAction,
    ) -> Option<&'static str> {
        let repo_id = match action {
            ContextMenuAction::Explorer { repo_id, .. }
            | ContextMenuAction::SelectConflictDiff { repo_id, .. }
            | ContextMenuAction::OpenFile { repo_id, .. }
            | ContextMenuAction::OpenFileLocation { repo_id, .. }
            | ContextMenuAction::EditFile { repo_id, .. }
            | ContextMenuAction::DiscardFileEdits { repo_id, .. }
            | ContextMenuAction::CompareWithWorkingTree { repo_id, .. }
            | ContextMenuAction::CheckoutCommit { repo_id, .. }
            | ContextMenuAction::CherryPickCommit { repo_id, .. }
            | ContextMenuAction::RevertCommit { repo_id, .. }
            | ContextMenuAction::StageStatusFolder { repo_id, .. }
            | ContextMenuAction::ApplyFileChange { repo_id, .. }
            | ContextMenuAction::CheckoutBranch { repo_id, .. }
            | ContextMenuAction::StageSelectionOrPath { repo_id, .. }
            | ContextMenuAction::UnstageSelectionOrPath { repo_id, .. }
            | ContextMenuAction::DiscardWorktreeChangesSelectionOrPath { repo_id, .. }
            | ContextMenuAction::AddExplorerToGitignore { repo_id, .. }
            | ContextMenuAction::AddToGitignoreSelectionOrPath { repo_id, .. }
            | ContextMenuAction::CheckoutConflictSideSelectionOrPath { repo_id, .. }
            | ContextMenuAction::LaunchMergetool { repo_id, .. }
            | ContextMenuAction::UpdateSubmodules { repo_id }
            | ContextMenuAction::LoadSubmodule { repo_id, .. }
            | ContextMenuAction::PullBranch { repo_id, .. }
            | ContextMenuAction::MergeRef { repo_id, .. }
            | ContextMenuAction::SquashRef { repo_id, .. }
            | ContextMenuAction::ApplyStash { repo_id, .. }
            | ContextMenuAction::PopStash { repo_id, .. }
            | ContextMenuAction::DropStashConfirm { repo_id, .. }
            | ContextMenuAction::LoadInteractiveRebaseSetup { repo_id, .. }
            | ContextMenuAction::OpenInteractiveCherryPickSetup { repo_id, .. }
            | ContextMenuAction::ApplyIndexPatch { repo_id, .. }
            | ContextMenuAction::ApplyWorktreePatch { repo_id, .. }
            | ContextMenuAction::StageHunk { repo_id, .. }
            | ContextMenuAction::UnstageHunk { repo_id, .. } => Some(*repo_id),
            ContextMenuAction::SelectDiff { repo_id, target } => match target {
                DiffTarget::WorkingTree { worktree: None, .. }
                | DiffTarget::CommitRange {
                    to_commit_id: None, ..
                } => Some(*repo_id),
                DiffTarget::WorkingTree {
                    worktree: Some(_), ..
                }
                | DiffTarget::Commit { .. }
                | DiffTarget::CommitRange {
                    to_commit_id: Some(_),
                    ..
                } => None,
            },
            ContextMenuAction::OpenFileContent {
                repo_id, source, ..
            } => match source {
                FileSource::WorkingDirectory => Some(*repo_id),
                FileSource::Commit(_) | FileSource::Branch(_) => None,
            },
            ContextMenuAction::RunLargeFileCommand { repo_id, command } => {
                (command.repository_requirement() == RepositoryRequirement::WorkingTree)
                    .then_some(*repo_id)
            }
            ContextMenuAction::OpenPopover { kind } => {
                kind.working_tree_repository(self.active_repo_id())
            }
            ContextMenuAction::RunExtensionCommand { id, repo_id } => self
                .extension_repo_tab_menu
                .iter()
                .find(|entry| entry.id == *id)
                .filter(|entry| entry.repository_requirement == RepositoryRequirement::WorkingTree)
                .map(|_| *repo_id),
            ContextMenuAction::AppMenu(AppMenuAction::ApplyPatch { repo_id }) => *repo_id,
            ContextMenuAction::AppMenu(AppMenuAction::ExtensionCommand { id }) => self
                .extension_app_menu
                .iter()
                .find(|entry| entry.id == *id)
                .filter(|entry| entry.repository_requirement == RepositoryRequirement::WorkingTree)
                .and(self.active_repo_id()),
            ContextMenuAction::AddGitattributesRule { .. }
            | ContextMenuAction::SetCommitAmendEnabled { .. }
            | ContextMenuAction::SetCommitPushAfterEnabled { .. }
            | ContextMenuAction::UseCommitMessage { .. }
            | ContextMenuAction::SetInteractiveRebaseAction { .. }
            | ContextMenuAction::SetInteractiveRebaseAutosquashMode { .. }
            | ContextMenuAction::ConflictResolverPick { .. }
            | ContextMenuAction::ConflictResolverUnresolve { .. }
            | ContextMenuAction::ConflictResolverSplitSelection
            | ContextMenuAction::ConflictResolverAlignManually
            | ContextMenuAction::ConflictResolverClearManualAlignments
            | ContextMenuAction::ConflictResolverJoinRegions { .. }
            | ContextMenuAction::ConflictResolverOutputCut { .. }
            | ContextMenuAction::ConflictResolverOutputPaste => self.active_repo_id(),
            ContextMenuAction::AppMenu(
                AppMenuAction::CommandPalette
                | AppMenuAction::LocateFileInExplorer
                | AppMenuAction::OpenRemoteInBrowser
                | AppMenuAction::Settings
                | AppMenuAction::OpenWorkspace
                | AppMenuAction::ZoomIn
                | AppMenuAction::ZoomOut
                | AppMenuAction::ResetZoom
                | AppMenuAction::OpenInCodeEditor { .. }
                | AppMenuAction::CheckForUpdates
                | AppMenuAction::ShowReflog { .. }
                | AppMenuAction::Quit
                | AppMenuAction::CloseWindow,
            ) => None,
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            ContextMenuAction::AppMenu(AppMenuAction::InstallDesktopIntegration) => None,
            ContextMenuAction::PreviewDisableBareMode { .. }
            | ContextMenuAction::Hosted(_)
            | ContextMenuAction::SetImageSampling { .. }
            | ContextMenuAction::SetImageSmoothFilter { .. }
            | ContextMenuAction::ImageZoom { .. }
            | ContextMenuAction::SetTextEncoding { .. }
            | ContextMenuAction::ConvertLineEndings { .. }
            | ContextMenuAction::SaveWithEncoding { .. }
            | ContextMenuAction::SetTabSize { .. }
            | ContextMenuAction::ToggleHistoryRefGroup { .. }
            | ContextMenuAction::AddRepoMenu(_)
            | ContextMenuAction::OpenWorktreeDiff { .. }
            | ContextMenuAction::OpenWorktreeFile { .. }
            | ContextMenuAction::OpenWorktreeFileLocation { .. }
            | ContextMenuAction::OpenRepositoryLocation { .. }
            | ContextMenuAction::OpenInCodeEditor { .. }
            | ContextMenuAction::ToggleFileBrowserDir { .. }
            | ContextMenuAction::ToggleSidebarCollapseKey { .. }
            | ContextMenuAction::SetSidebarCollapseKey { .. }
            | ContextMenuAction::SetBranchGroupCollapsedRecursive { .. }
            | ContextMenuAction::UnpinAllBranches { .. }
            | ContextMenuAction::ConfirmDeleteBranchGroup { .. }
            | ContextMenuAction::SetFileBrowserDirExpandedRecursive { .. }
            | ContextMenuAction::RevealHistoryCommit { .. }
            | ContextMenuAction::ShowFileChangesAtCommit { .. }
            | ContextMenuAction::OpenFileAtCommit { .. }
            | ContextMenuAction::OpenFileAtCommitParent { .. }
            | ContextMenuAction::BrowseRepositoryAtCommit { .. }
            | ContextMenuAction::ResetBrowseToLive { .. }
            | ContextMenuAction::OpenRepo { .. }
            | ContextMenuAction::OpenWorktreeAddPrompt { .. }
            | ContextMenuAction::ActivateRepo { .. }
            | ContextMenuAction::CloseRepo { .. }
            | ContextMenuAction::CloseRepos { .. }
            | ContextMenuAction::MoveRepoToWorkspace { .. }
            | ContextMenuAction::ActivateWorkspace { .. }
            | ContextMenuAction::DeleteWorkspace { .. }
            | ContextMenuAction::OpenWorkspaceSettings { .. }
            | ContextMenuAction::PinRepository { .. }
            | ContextMenuAction::UnpinRepository { .. }
            | ContextMenuAction::ForgetRecentRepository { .. }
            | ContextMenuAction::OpenSubmoduleDiffInTab { .. }
            | ContextMenuAction::ExportPatch { .. }
            | ContextMenuAction::MarkForComparison { .. }
            | ContextMenuAction::CompareWithMarked { .. }
            | ContextMenuAction::CompareWithMergeBase { .. }
            | ContextMenuAction::ClearComparisonMark { .. }
            | ContextMenuAction::SetFileListFolderCollapsed { .. }
            | ContextMenuAction::SquashSelectedCommits { .. }
            | ContextMenuAction::DeleteBranch { .. }
            | ContextMenuAction::ToggleBranchPin { .. }
            | ContextMenuAction::ToggleBranchGroupPin { .. }
            | ContextMenuAction::SetHistoryScope { .. }
            | ContextMenuAction::SetBranchGroupFocus { .. }
            | ContextMenuAction::ToggleHistoryFocus { .. }
            | ContextMenuAction::SetHistoryFocus { .. }
            | ContextMenuAction::SetRemoteHistoryVisibility { .. }
            | ContextMenuAction::SetCommitFileSort { .. }
            | ContextMenuAction::SetFileListLayout { .. }
            | ContextMenuAction::SetDiffContentMode { .. }
            | ContextMenuAction::SetDiffWhitespaceMode { .. }
            | ContextMenuAction::SetDiffEngineOverride { .. }
            | ContextMenuAction::SetDiffRevealWhitespaceChars { .. }
            | ContextMenuAction::SetDiffWordWrap { .. }
            | ContextMenuAction::SetDiffShowLineNumbers { .. }
            | ContextMenuAction::SetExplorerVisibility { .. }
            | ContextMenuAction::SetChangeTrackingView { .. }
            | ContextMenuAction::SetUiScale { .. }
            | ContextMenuAction::LoadLfsLocks { .. }
            | ContextMenuAction::FetchAll { .. }
            | ContextMenuAction::PruneMergedBranches { .. }
            | ContextMenuAction::PruneLocalTags { .. }
            | ContextMenuAction::LoadWorktrees { .. }
            | ContextMenuAction::PushWithTags { .. }
            | ContextMenuAction::Push { .. }
            | ContextMenuAction::SetUpstreamBranch { .. }
            | ContextMenuAction::UnsetUpstreamBranch { .. }
            | ContextMenuAction::OpenFileHistory { .. }
            | ContextMenuAction::SetMergetoolAutoAdvance { .. }
            | ContextMenuAction::ToggleMergetoolCollapseUnchanged
            | ContextMenuAction::SetMergetoolOutputScrollSync { .. }
            | ContextMenuAction::SetMergetoolShowLineNumbers { .. }
            | ContextMenuAction::SetMergetoolThreeWayView { .. }
            | ContextMenuAction::CopyText { .. }
            | ContextMenuAction::CopyLinkAddress { .. }
            | ContextMenuAction::LoadRemoteMarkdownImage { .. }
            | ContextMenuAction::OpenWebUrl { .. }
            | ContextMenuAction::CopyDiffSelection { .. }
            | ContextMenuAction::CopyDiffText { .. }
            | ContextMenuAction::TerminalCommand { .. }
            | ContextMenuAction::TerminalOpenExternal { .. }
            | ContextMenuAction::DeleteTag { .. }
            | ContextMenuAction::PushTag { .. }
            | ContextMenuAction::DeleteRemoteTag { .. }
            // `pull_unavailable_reason` covers bare repositories and annex takeover.
            | ContextMenuAction::Pull { .. } => None,
        }?;
        self.state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(RepoState::is_bare)
            .then_some(WORKING_TREE_REQUIRED_MESSAGE)
    }

    pub(super) fn gate_bare_menu(&self, mut model: ContextMenuModel) -> ContextMenuModel {
        for (ix, item) in model.items.iter_mut().enumerate() {
            if let ContextMenuItem::Entry { label, action, .. } = item
                && let ContextMenuAction::RunLargeFileCommand { repo_id, command } = action.as_ref()
                && self
                    .state
                    .repos
                    .iter()
                    .find(|repo| repo.id == *repo_id)
                    .is_some_and(RepoState::is_bare)
                && matches!(command,
                    gitcomet_core::large_files::LargeFileCommand::AnnexGet { paths, .. }
                    | gitcomet_core::large_files::LargeFileCommand::AnnexDrop { paths, .. }
                    | gitcomet_core::large_files::LargeFileCommand::AnnexCopy { paths, .. }
                    | gitcomet_core::large_files::LargeFileCommand::AnnexMove { paths, .. } if paths.is_empty())
            {
                *label = format!("{label} (all versions)").into();
            }
            if let ContextMenuItem::Entry {
                disabled, action, ..
            } = item
                && let Some(reason) = self.bare_action_reason(action)
            {
                *disabled = true;
                model.entry_tooltips.insert(ix, reason.into());
            }
        }
        model
    }
}
