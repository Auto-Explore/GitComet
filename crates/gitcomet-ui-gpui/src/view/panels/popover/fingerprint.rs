use super::*;
use crate::view::fingerprint as view_fingerprint;
use gitcomet_state::model::CloneProgressStage;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

/// What the open popover was last drawn from: itself, and the state it reads.
#[derive(Default)]
pub(super) struct NotifyFingerprint {
    popover: Option<PopoverKind>,
    state: u64,
}

impl NotifyFingerprint {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Records what is shown now. True when the popover or the state it reads
    /// differs from the last record, so the popover needs a repaint.
    pub(super) fn record(
        &mut self,
        state: &AppState,
        popover: &PopoverKind,
        binding: Option<crate::view::pane_store::DiffBinding>,
    ) -> bool {
        let state = notify_fingerprint(state, popover, binding);
        let same_popover = self.popover.as_ref() == Some(popover);
        if same_popover && state == self.state {
            return false;
        }
        if !same_popover {
            self.popover = Some(popover.clone());
        }
        self.state = state;
        true
    }
}

/// A hash of the state `popover` reads. The popover itself is compared by
/// [`NotifyFingerprint`].
pub(super) fn notify_fingerprint(
    state: &AppState,
    popover: &PopoverKind,
    binding: Option<crate::view::pane_store::DiffBinding>,
) -> u64 {
    let mut hasher = FxHasher::default();
    let active_repository = binding
        .map(|binding| binding.repo_id)
        .or(state.active_repo)
        .and_then(|id| state.repos.iter().find(|repo| repo.id == id));
    active_repository
        .and_then(|repo| repo.shared_preferences.as_ref())
        .map(|snapshot| snapshot.revision)
        .hash(&mut hasher);
    if let Some(repo) = repo_for_popover(state, popover, binding).or(active_repository) {
        repo.repository_mode.hash(&mut hasher);
        std::mem::discriminant(&repo.bare_mode_conversion).hash(&mut hasher);
    }
    if matches!(popover, PopoverKind::CommitMenu { .. }) {
        state.git_log_settings.show_history_tags.hash(&mut hasher);
    }
    if matches!(popover, PopoverKind::PullPicker) {
        state.remote_settings.auto_fetch.enabled.hash(&mut hasher);
        if let Some(repo) = repo_for_popover(state, popover, binding) {
            repo.auto_fetch.last_success_at.hash(&mut hasher);
            repo.auto_fetch.retry_at.hash(&mut hasher);
            repo.auto_fetch.paused_remotes.hash(&mut hasher);
            std::mem::discriminant(&repo.auto_fetch.status).hash(&mut hasher);
            match &repo.auto_fetch.status {
                gitcomet_state::model::AutoFetchStatus::Failed { at, error } => {
                    at.hash(&mut hasher);
                    super::context_menu::pull::auto_fetch_failure_description(error)
                        .hash(&mut hasher);
                    super::context_menu::pull::auto_fetch_auth_recovery_hint(error)
                        .hash(&mut hasher);
                }
                gitcomet_state::model::AutoFetchStatus::Succeeded { at } => at.hash(&mut hasher),
                gitcomet_state::model::AutoFetchStatus::Idle
                | gitcomet_state::model::AutoFetchStatus::Running => {}
            }
        }
    }

    match popover {
        PopoverKind::CloneRepo => match &state.clone {
            None => 0u8.hash(&mut hasher),
            Some(clone) => {
                1u8.hash(&mut hasher);
                clone.seq.hash(&mut hasher);
                clone.url.hash(&mut hasher);
                clone.dest.hash(&mut hasher);
                match &clone.status {
                    CloneOpStatus::Running => 0u8.hash(&mut hasher),
                    CloneOpStatus::Cancelling => 1u8.hash(&mut hasher),
                    CloneOpStatus::FinishedOk => 2u8.hash(&mut hasher),
                    CloneOpStatus::Cancelled => 3u8.hash(&mut hasher),
                    CloneOpStatus::WaitingForAuthentication => 5u8.hash(&mut hasher),
                    CloneOpStatus::FinishedErr(err) => {
                        4u8.hash(&mut hasher);
                        err.hash(&mut hasher);
                    }
                }
                clone.progress.percent.hash(&mut hasher);
                match clone.progress.stage {
                    CloneProgressStage::Loading => 0u8.hash(&mut hasher),
                    CloneProgressStage::RemoteObjects => 1u8.hash(&mut hasher),
                }
            }
        },
        PopoverKind::RepoPicker { .. } => {
            state.active_repo.hash(&mut hasher);
            state.repos.len().hash(&mut hasher);
            // Repo picker list is usually small; hashing all ids+workdirs is fine and avoids stale lists.
            for repo in &state.repos {
                repo.id.hash(&mut hasher);
                repo.spec.workdir.hash(&mut hasher);
                view_fingerprint::hash_loadable_kind(&repo.open, &mut hasher);
            }
        }
        PopoverKind::RepoTabMenu { .. } => {
            state.active_repo.hash(&mut hasher);
            state.repos.len().hash(&mut hasher);
            for repo in &state.repos {
                repo.id.hash(&mut hasher);
            }
        }
        PopoverKind::DiffContentModeSettings
        | PopoverKind::TextFormatMenu { .. }
        | PopoverKind::CommitFileSortMenu { .. }
        | PopoverKind::FileListLayoutMenu { .. }
        | PopoverKind::WebLinkMenu { .. }
        | PopoverKind::LocalFileLinkMenu { .. }
        | PopoverKind::CommitShaLinkMenu { .. }
        | PopoverKind::DiffActionMenu
        | PopoverKind::MergetoolSettingsMenu
        | PopoverKind::ChangeTrackingSettings
        | PopoverKind::UiScalePicker
        | PopoverKind::AppMenu
        | PopoverKind::AddRepoMenu => {
            // Mostly local UI state; depend only on whether a repo is active/open.
            state.active_repo.hash(&mut hasher);
            if let Some(repo) = repo_for_popover(state, popover, binding) {
                view_fingerprint::hash_loadable_kind(&repo.open, &mut hasher);
                // The app menu enables "Open remote in web browser" from them.
                if matches!(popover, PopoverKind::AppMenu) {
                    repo.remotes_rev.hash(&mut hasher);
                }
            }
        }
        PopoverKind::Repo {
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::TrustConfirm),
            ..
        } => {
            if let Some(repo) = repo_for_popover(state, popover, binding) {
                let diff = binding
                    .and_then(|binding| binding.diff(repo))
                    .unwrap_or(&repo.diff_state);
                hash_repo_for_popover(repo, diff, popover, &mut hasher);
            } else {
                state.active_repo.hash(&mut hasher);
            }
            if let Some(prompt) = state.submodule_trust_prompt.as_ref() {
                prompt.repo_id.hash(&mut hasher);
                match &prompt.operation {
                    SubmoduleTrustPromptOperation::Add {
                        url,
                        path,
                        branch,
                        name,
                        force,
                    } => {
                        0u8.hash(&mut hasher);
                        url.hash(&mut hasher);
                        path.hash(&mut hasher);
                        branch.hash(&mut hasher);
                        name.hash(&mut hasher);
                        force.hash(&mut hasher);
                    }
                    SubmoduleTrustPromptOperation::Update => {
                        1u8.hash(&mut hasher);
                    }
                    SubmoduleTrustPromptOperation::Load { path } => {
                        2u8.hash(&mut hasher);
                        path.hash(&mut hasher);
                    }
                }
                for source in &prompt.sources {
                    source.submodule_path.hash(&mut hasher);
                    source.display_source.hash(&mut hasher);
                    source.local_source_path.hash(&mut hasher);
                }
            }
            // The pending spinner and the resolved dialog share this popover
            // kind, so fold the pending check in to force a re-render when the
            // check starts or resolves.
            if let Some(check) = state.submodule_trust_check_pending.as_ref() {
                check.repo_id.hash(&mut hasher);
                std::mem::discriminant(&check.operation).hash(&mut hasher);
            }
        }
        _ => {
            if let Some(repo) = repo_for_popover(state, popover, binding) {
                let diff = binding
                    .and_then(|binding| binding.diff(repo))
                    .unwrap_or(&repo.diff_state);
                hash_repo_for_popover(repo, diff, popover, &mut hasher);
                if matches!(popover, PopoverKind::PullPicker | PopoverKind::PushPicker) {
                    hash_large_file_picker_entries(state, repo, &mut hasher);
                }
            } else {
                state.active_repo.hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}

/// Pull and Push list LFS and git-annex entries built from the support
/// summary, the installed tools, the settings and the running pull or push.
fn hash_large_file_picker_entries(state: &AppState, repo: &RepoState, hasher: &mut FxHasher) {
    let settings = &state.large_file_settings;
    repo.annex_takes_over_pull_push(settings).hash(hasher);
    settings.annex_sync_content.hash(hasher);
    repo.large_file_support_rev.hash(hasher);
    state.large_file_tools.git_lfs.is_not_found().hash(hasher);
    state.large_file_tools.git_annex.is_not_found().hash(hasher);
    (repo.push_in_flight > 0).hash(hasher);
    (repo.worktree_pull_in_flight > 0).hash(hasher);
}

fn repo_for_popover<'a>(
    state: &'a AppState,
    popover: &PopoverKind,
    binding: Option<crate::view::pane_store::DiffBinding>,
) -> Option<&'a RepoState> {
    let repo_id = match popover {
        PopoverKind::RepoPicker { .. }
        | PopoverKind::CloneRepo
        | PopoverKind::DiffContentModeSettings
        | PopoverKind::TextFormatMenu { .. }
        | PopoverKind::CommitFileSortMenu { .. }
        | PopoverKind::WebLinkMenu { .. }
        | PopoverKind::DiffActionMenu
        | PopoverKind::MergetoolSettingsMenu
        | PopoverKind::ChangeTrackingSettings
        | PopoverKind::UiScalePicker
        | PopoverKind::Hosted { .. }
        | PopoverKind::ErrorDetails { .. }
        | PopoverKind::KeyboardShortcuts
        | PopoverKind::FilesystemConflict(_)
        | PopoverKind::DeletePermanentlyConfirm(_)
        | PopoverKind::FilesystemUnsavedEditsConfirm(_) => None,

        // Popovers that implicitly use the currently active repo.
        PopoverKind::BranchPicker { .. }
        | PopoverKind::StashPrompt
        | PopoverKind::PullPicker
        | PopoverKind::PushPicker
        | PopoverKind::AppMenu
        | PopoverKind::AddRepoMenu
        | PopoverKind::RebaseReword { .. }
        | PopoverKind::InteractiveRebaseActionMenu { .. }
        | PopoverKind::InteractiveRebaseAutosquashMenu
        | PopoverKind::TerminalShutdownConfirm(_)
        | PopoverKind::CloseGuardConfirm(_)
        | PopoverKind::UnsavedFileEditsConfirm(_)
        | PopoverKind::ConflictResolverInputRowMenu { .. }
        | PopoverKind::ConflictResolverChunkMenu { .. }
        | PopoverKind::ConflictResolverOutputMenu { .. } => {
            binding.map(|binding| binding.repo_id).or(state.active_repo)
        }

        // Popovers that carry an explicit repo id.
        PopoverKind::CommitPrompt { repo_id }
        | PopoverKind::StashPickerPrompt { repo_id, .. }
        | PopoverKind::FileHistoryFilePicker { repo_id }
        | PopoverKind::UpstreamPicker { repo_id, .. }
        | PopoverKind::CreateBranchFromRefPrompt { repo_id, .. }
        | PopoverKind::RenameBranchPrompt { repo_id, .. }
        | PopoverKind::ResetPrompt { repo_id, .. }
        | PopoverKind::SquashPrompt { repo_id }
        | PopoverKind::CheckoutRemoteBranchPrompt { repo_id, .. }
        | PopoverKind::StashDropConfirm { repo_id, .. }
        | PopoverKind::StashMenu { repo_id, .. }
        | PopoverKind::RepoTabMenu { repo_id }
        | PopoverKind::CreateTagPrompt { repo_id, .. }
        | PopoverKind::Repo { repo_id, .. }
        | PopoverKind::PushSetUpstreamPrompt { repo_id, .. }
        | PopoverKind::ForcePushConfirm { repo_id }
        | PopoverKind::CherryPickCommitConfirm { repo_id, .. }
        | PopoverKind::InteractiveCherryPickConfirm { repo_id, .. }
        | PopoverKind::ApplyFileChangeConfirm { repo_id, .. }
        | PopoverKind::RevertCommitConfirm { repo_id, .. }
        | PopoverKind::MergeCommitConfirm { repo_id, .. }
        | PopoverKind::MergeAbortConfirm { repo_id }
        | PopoverKind::BranchExistsPrompt { repo_id, .. }
        | PopoverKind::ForceDeleteBranchConfirm(gitcomet_state::model::ForceDeleteBranchPrompt {
            repo_id,
            ..
        })
        | PopoverKind::ForceRemoveWorktreeConfirm(
            gitcomet_state::model::ForceRemoveWorktreePrompt { repo_id, .. },
        )
        | PopoverKind::DiscardChangesConfirm { repo_id, .. }
        | PopoverKind::DiscardFolderChangesConfirm { repo_id, .. }
        | PopoverKind::FileListFolderMenu { repo_id, .. }
        | PopoverKind::FileListLayoutMenu { repo_id, .. }
        | PopoverKind::AddToGitignorePrompt { repo_id, .. }
        | PopoverKind::StageConflictMarkersConfirm { repo_id, .. }
        | PopoverKind::PullReconcilePrompt { repo_id }
        | PopoverKind::RebaseOntoConfirm { repo_id, .. }
        | PopoverKind::CommitOptionsMenu { repo_id }
        | PopoverKind::PreviousCommitMessagesMenu { repo_id }
        | PopoverKind::DiffHunkMenu { repo_id, .. }
        | PopoverKind::CommitMenu { repo_id, .. }
        | PopoverKind::StatusFileMenu { repo_id, .. }
        | PopoverKind::StatusConflictMenu { repo_id, .. }
        | PopoverKind::BranchMenu { repo_id, .. }
        | PopoverKind::BranchSectionMenu { repo_id, .. }
        | PopoverKind::BranchGroupMenu { repo_id, .. }
        | PopoverKind::SidebarPinnedOverflow { repo_id, .. }
        | PopoverKind::SidebarAncestorMenu { repo_id, .. }
        | PopoverKind::DeleteBranchesConfirm { repo_id, .. }
        | PopoverKind::CommitFileMenu { repo_id, .. }
        | PopoverKind::WorktreeFileMenu { repo_id, .. }
        | PopoverKind::CommitRangeFileMenu { repo_id, .. }
        | PopoverKind::FileBrowserFileMenu { repo_id, .. }
        | PopoverKind::FileBrowserFolderMenu { repo_id, .. }
        | PopoverKind::ExplorerSettingsMenu { repo_id }
        | PopoverKind::BrowseHistoryMenu { repo_id }
        | PopoverKind::SubmoduleInnerDiffMenu { repo_id, .. }
        | PopoverKind::TagMenu { repo_id, .. }
        | PopoverKind::TerminalMenu { repo_id, .. }
        | PopoverKind::HistoryBranchFilter { repo_id }
        | PopoverKind::HistoryAuthorFilter { repo_id }
        | PopoverKind::CommitShaLinkMenu { repo_id, .. }
        | PopoverKind::LocalFileLinkMenu { repo_id, .. }
        | PopoverKind::ReflogEntryMenu { repo_id, .. }
        | PopoverKind::FileHistoryCommitMenu { repo_id, .. }
        | PopoverKind::HookActivity { repo_id, .. } => Some(*repo_id),
        PopoverKind::DiffEditorMenu { repo_id, .. } => *repo_id,
    }?;

    if let Some(binding) = binding {
        return binding.repository(state).filter(|repo| repo.id == repo_id);
    }
    state.repos.iter().find(|r| r.id == repo_id)
}

fn hash_repo_for_popover<H: Hasher>(
    repo: &RepoState,
    diff: &gitcomet_state::model::DiffState,
    popover: &PopoverKind,
    hasher: &mut H,
) {
    view_fingerprint::hash_loadable_kind(&repo.open, hasher);

    match popover {
        PopoverKind::BranchPicker { .. }
        | PopoverKind::UpstreamPicker { .. }
        | PopoverKind::CreateBranchFromRefPrompt { .. }
        | PopoverKind::RenameBranchPrompt { .. }
        | PopoverKind::BranchMenu { .. }
        | PopoverKind::BranchSectionMenu { .. }
        // The group menu's branch count and the pinned menu's "Unpin all (N)"
        // both read the live branch lists, so a refresh landing while the menu
        // is up has to repaint it rather than leave a stale count.
        | PopoverKind::BranchGroupMenu { .. }
        | PopoverKind::SidebarPinnedOverflow { .. }
        | PopoverKind::SidebarAncestorMenu { .. }
        | PopoverKind::ForceDeleteBranchConfirm(_)
        | PopoverKind::PushSetUpstreamPrompt { .. } => {
            repo.head_branch_rev.hash(hasher);
            repo.branches_rev.hash(hasher);
            repo.remotes_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
            repo.tags_rev.hash(hasher);
            repo.annex_refs_hidden.hash(hasher);
            // The checkout picker's rows carry each ref's author, date and
            // summary on their detail line, so metadata landing while the picker
            // is open has to repaint it — the rows change height, not just text.
            repo.ref_metadata_rev.hash(hasher);
            // A branch's menu offers its worktree once the worktree list knows it.
            repo.worktrees_rev.hash(hasher);
        }

        // Its toggle label reads `expanded_dirs` and its disabled state reads
        // `search_query`. "Locate file in explorer" is a global action that
        // rewrites both without any click on the tree, so the menu has to
        // repaint when it lands rather than keep a label that is now a lie.
        PopoverKind::FileBrowserFolderMenu { .. } => {
            repo.file_browser.file_browser_rev.hash(hasher);
        }

        // Its check marks read `show_hidden` / `show_ignored`; the menu stays
        // open while they toggle.
        PopoverKind::ExplorerSettingsMenu { .. } => {
            repo.file_browser.file_browser_rev.hash(hasher);
        }

        // File counts come from the lists' files, and Apply follows the busy
        // state. Collapse state is view-local and only changes by closing it.
        PopoverKind::FileListFolderMenu { .. } | PopoverKind::DiscardFolderChangesConfirm { .. } => {
            repo.history_rewrite_busy().hash(hasher);
            repo.history_state.commit_details_rev.hash(hasher);
            repo.history_state.range_files_rev.hash(hasher);
            repo.worktree_status_cache_rev().hash(hasher);
            repo.staged_status_cache_rev().hash(hasher);
        }

        PopoverKind::Repo {
            kind: RepoPopoverKind::Remote(_),
            ..
        } => {
            repo.remotes_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
        }

        // Its worktree note reads the worktree list and HEAD.
        PopoverKind::BranchExistsPrompt { .. } => {
            repo.worktrees_rev.hash(hasher);
            repo.head_branch_rev.hash(hasher);
        }

        PopoverKind::Repo {
            kind: RepoPopoverKind::Worktree(_),
            ..
        } => {
            repo.worktrees_rev.hash(hasher);
            // The badge picker's create row reads HEAD for its "Based off <ref>"
            // line (`worktree_badge_picker::create_base_ref`), so a checkout landing
            // while the picker is open has to repaint it — otherwise the row keeps
            // promising a base the Add dialog will no longer use.
            repo.head_branch_rev.hash(hasher);
            // The Add dialog refuses a new branch name that already exists.
            repo.branches_rev.hash(hasher);
            // A history row's worktree menu leaves out dirty worktrees.
            repo.worktree_dirty_rev.hash(hasher);
        }

        PopoverKind::Repo {
            kind: RepoPopoverKind::Submodule(_),
            ..
        } => {
            repo.submodules_rev.hash(hasher);
        }

        // Annex menus list repositories and remotes from the support summary.
        PopoverKind::Repo {
            kind: RepoPopoverKind::Annex(_),
            ..
        } => {
            repo.large_file_support_rev.hash(hasher);
            repo.annex_unused_rev.hash(hasher);
        }

        PopoverKind::StashPrompt => {
            repo.stashes_rev.hash(hasher);
            repo.status_cache_rev().hash(hasher);
        }
        PopoverKind::StashDropConfirm { .. }
        | PopoverKind::StashMenu { .. }
        | PopoverKind::StashPickerPrompt { .. } => {
            repo.stashes_rev.hash(hasher);
        }
        PopoverKind::FileHistoryFilePicker { .. } => {
            repo.file_browser.file_browser_rev.hash(hasher);
            repo.status_rev.hash(hasher);
            diff.diff_target_rev.hash(hasher);
        }

        PopoverKind::DiffHunkMenu { .. }
        | PopoverKind::DiffEditorMenu { .. }
        | PopoverKind::DiscardChangesConfirm { .. } => {
            diff.diff_rev.hash(hasher);
            if let Some(t) = diff.diff_target.as_ref() {
                view_fingerprint::hash_diff_target(t, hasher)
            }
            view_fingerprint::hash_loadable_arc(&diff.diff, hasher);
            diff.diff_file_rev.hash(hasher);
            view_fingerprint::hash_loadable_kind(&diff.diff_file, hasher);
            view_fingerprint::hash_loadable_kind(&diff.diff_file_image, hasher);

            // Working tree diff popovers need status for file-kind/conflict decisions.
            if matches!(
                diff.diff_target,
                Some(DiffTarget::WorkingTree { .. })
            ) {
                repo.status_cache_rev().hash(hasher);
            }
        }

        PopoverKind::HistoryBranchFilter { .. } => {
            repo.history_state.history_scope.hash(hasher);
            repo.branches_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
            repo.tags_rev.hash(hasher);
        }

        PopoverKind::HistoryAuthorFilter { .. } => {
            repo.history_state.history_author_filter.hash(hasher);
            repo.history_state.history_scope.hash(hasher);
            repo.history_state.authors.rev.hash(hasher);
            repo.load_epoch.hash(hasher);
            repo.history_state.log_rev.hash(hasher);
            repo.log_rev.hash(hasher);
        }

        PopoverKind::PullPicker
        | PopoverKind::PushPicker
        | PopoverKind::PullReconcilePrompt { .. }
        | PopoverKind::ForcePushConfirm { .. } => {
            repo.head_branch_rev.hash(hasher);
            repo.branches_rev.hash(hasher);
            repo.remotes_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
            hash_pending_force_push_lease(repo, hasher);
        }

        PopoverKind::PreviousCommitMessagesMenu { .. } => {
            repo.recent_commit_messages_rev.hash(hasher);
        }

        PopoverKind::RepoTabMenu { .. } => {
            repo.id.hash(hasher);
        }

        PopoverKind::CommitOptionsMenu { .. } => {
            repo.log_rev.hash(hasher);
            repo.ops_rev.hash(hasher);
            repo.merge_message_rev.hash(hasher);
            repo.head_branch_rev.hash(hasher);
            repo.branches_rev.hash(hasher);
        }

        // The squash prompt tracks the message preview plus everything that
        // can invalidate the selection's eligibility while it is open.
        PopoverKind::SquashPrompt { .. } => {
            repo.history_state.squash_preview_rev.hash(hasher);
            repo.history_state.selected_commit_rev.hash(hasher);
            repo.history_state.log_rev.hash(hasher);
            repo.head_branch_rev.hash(hasher);
            repo.branches_rev.hash(hasher);
        }

        PopoverKind::CommitMenu { .. } => {
            repo.head_branch_rev.hash(hasher);
            repo.branches_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
            repo.tags_rev.hash(hasher);
            repo.remotes_rev.hash(hasher);
            repo.remote_tags_rev.hash(hasher);
            repo.status_rev.hash(hasher);
            repo.history_state.selected_commit_rev.hash(hasher);
            repo.history_state.log_rev.hash(hasher);
            repo.ops_rev.hash(hasher);
            repo.worktrees_rev.hash(hasher);
        }
        PopoverKind::FileHistoryCommitMenu { .. } => {
            repo.head_branch_rev.hash(hasher);
            repo.branches_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
            repo.tags_rev.hash(hasher);
            repo.history_state.log_rev.hash(hasher);
            repo.ops_rev.hash(hasher);
            repo.history_rewrite_busy().hash(hasher);
            repo.navigation.comparison_mark.hash(hasher);
        }
        PopoverKind::TagMenu { .. } => {
            repo.tags_rev.hash(hasher);
            repo.remotes_rev.hash(hasher);
            repo.remote_tags_rev.hash(hasher);
        }

        PopoverKind::HookActivity { .. } => {
            repo.feedback.hook_activity_rev.hash(hasher);
        }

        // The parent list arrives from a lookup issued on open, and the
        // buttons follow the repo's busy state.
        PopoverKind::CherryPickCommitConfirm { .. } | PopoverKind::RevertCommitConfirm { .. } => {
            let lookup = &repo.history_state.mainline_lookup;
            lookup.request.hash(hasher);
            lookup.reference.hash(hasher);
            std::mem::discriminant(&lookup.result).hash(hasher);
            repo.history_state.commit_details_rev.hash(hasher);
            // Summaries and ref labels come from the rows; the buttons follow
            // the repo's busy state and its staged files.
            repo.history_state.log_rev.hash(hasher);
            view_fingerprint::hash_loadable_kind(&repo.history_state.file_history, hasher);
            repo.branches_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
            repo.staged_status_rev.hash(hasher);
            repo.history_rewrite_busy().hash(hasher);
        }

        // The buttons follow the repo's busy state.
        PopoverKind::ApplyFileChangeConfirm { .. }
        | PopoverKind::InteractiveCherryPickConfirm { .. } => {
            repo.history_rewrite_busy().hash(hasher);
        }

        // "Apply change" follows the repo's busy state.
        PopoverKind::CommitFileMenu { .. } => {
            repo.history_rewrite_busy().hash(hasher);
        }
        // Foreign-file actions follow the selected checkout and scan membership.
        PopoverKind::WorktreeFileMenu { .. } => {
            repo.worktree_dirty_rev.hash(hasher);
            repo.history_state.worktree_selection_rev.hash(hasher);
        }
        // The submodule flag of a row comes from the loaded file list.
        PopoverKind::CommitRangeFileMenu { .. } => {
            repo.history_state.range_files_rev.hash(hasher);
            repo.history_rewrite_busy().hash(hasher);
        }

        // Most prompt-style popovers don't require live state updates.
        PopoverKind::InteractiveRebaseActionMenu { .. }
        | PopoverKind::InteractiveRebaseAutosquashMenu
        | PopoverKind::RebaseReword { .. }
        | PopoverKind::RebaseOntoConfirm { .. }
        | PopoverKind::MergeCommitConfirm { .. }
        | PopoverKind::MergeAbortConfirm { .. }
        | PopoverKind::ResetPrompt { .. }
        | PopoverKind::CheckoutRemoteBranchPrompt { .. }
        | PopoverKind::CreateTagPrompt { .. }
        | PopoverKind::ForceRemoveWorktreeConfirm(_)
        // Its member list is resolved when it opens and carried on the kind, so
        // it must not change under the user mid-confirmation.
        | PopoverKind::DeleteBranchesConfirm { .. }
        | PopoverKind::FileBrowserFileMenu { .. }
        | PopoverKind::BrowseHistoryMenu { .. }
        | PopoverKind::SubmoduleInnerDiffMenu { .. }
        | PopoverKind::StatusFileMenu { .. }
        | PopoverKind::StatusConflictMenu { .. }
        | PopoverKind::StageConflictMarkersConfirm { .. }
        // Its contents are computed once when it opens and then owned by the
        // text input. Re-hashing status would rebuild the dialog under the
        // user's cursor when a refresh lands mid-edit.
        | PopoverKind::AddToGitignorePrompt { .. }
        | PopoverKind::DiffContentModeSettings
        | PopoverKind::TextFormatMenu { .. }
        | PopoverKind::CommitFileSortMenu { .. }
        | PopoverKind::FileListLayoutMenu { .. }
        | PopoverKind::WebLinkMenu { .. }
        | PopoverKind::LocalFileLinkMenu { .. }
        | PopoverKind::CommitShaLinkMenu { .. }
        | PopoverKind::DiffActionMenu
        | PopoverKind::MergetoolSettingsMenu
        | PopoverKind::ChangeTrackingSettings
        | PopoverKind::UiScalePicker
        | PopoverKind::ConflictResolverInputRowMenu { .. }
        | PopoverKind::ConflictResolverChunkMenu { .. }
        | PopoverKind::ConflictResolverOutputMenu { .. }
        | PopoverKind::AppMenu
        | PopoverKind::AddRepoMenu
        | PopoverKind::TerminalShutdownConfirm(_)
        | PopoverKind::CloseGuardConfirm(_)
        | PopoverKind::UnsavedFileEditsConfirm(_)
        | PopoverKind::FilesystemConflict(_)
        | PopoverKind::DeletePermanentlyConfirm(_)
        | PopoverKind::FilesystemUnsavedEditsConfirm(_)
        | PopoverKind::TerminalMenu { .. }
        | PopoverKind::RepoPicker { .. }
        | PopoverKind::CloneRepo
        | PopoverKind::ReflogEntryMenu { .. }
        | PopoverKind::ErrorDetails { .. }
        | PopoverKind::KeyboardShortcuts
        | PopoverKind::Hosted { .. }
        | PopoverKind::CommitPrompt { .. } => {}
        PopoverKind::Repo {
            kind: RepoPopoverKind::RepositoryModeMenu | RepoPopoverKind::DisableBareModeConfirm,
            ..
        } => {
            match &repo.bare_mode_conversion {
                gitcomet_state::model::BareModeConversionState::AwaitingConfirmation(preview) => Arc::as_ptr(preview).hash(hasher),
                gitcomet_state::model::BareModeConversionState::Failed(error) => Arc::as_ptr(error).hash(hasher),
                gitcomet_state::model::BareModeConversionState::Idle
                | gitcomet_state::model::BareModeConversionState::Previewing
                | gitcomet_state::model::BareModeConversionState::Converting
                | gitcomet_state::model::BareModeConversionState::Reopening => {}
            }
        }
    }
}

fn hash_pending_force_push_lease(repo: &RepoState, hasher: &mut impl Hasher) {
    match &repo.pending.force_push_lease {
        Some(lease) => {
            1u8.hash(hasher);
            lease.remote.hash(hasher);
            lease.branch.hash(hasher);
            lease.expected.hash(hasher);
            lease.local_branch.hash(hasher);
            lease.local_head.hash(hasher);
        }
        None => 0u8.hash(hasher),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Branch, CommitId, Upstream};
    use std::sync::Arc;

    fn test_force_push_lease() -> gitcomet_core::services::ForcePushLease {
        gitcomet_core::services::ForcePushLease {
            remote: "origin".to_string(),
            branch: "main".to_string(),
            expected: CommitId("1111111111111111111111111111111111111111".into()),
            local_branch: "main".to_string(),
            local_head: CommitId("2222222222222222222222222222222222222222".into()),
        }
    }

    /// Kinds without fields hash nothing but their identity, so two sharing a
    /// tag would look identical to the host's notify gate.
    #[test]
    fn grouped_repo_popover_resolves_explicit_repo_id() {
        let repo_id = RepoId(42);
        let repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_repo_popover_repo_for_popover"),
            },
        );
        let state = AppState {
            repos: vec![repo],
            active_repo: None,
            ..AppState::test_default()
        };

        let popover = PopoverKind::worktree(repo_id, WorktreePopoverKind::SectionMenu);
        let resolved =
            repo_for_popover(&state, &popover, None).expect("expected repo lookup to work");

        assert_eq!(resolved.id, repo_id);
    }

    /// The branch menus add "Open worktree" once the worktree list names the
    /// branch; the Add dialog refuses an existing name.
    #[test]
    fn worktree_aware_popovers_repaint_when_their_lists_change() {
        let repo_id = RepoId(9);
        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::test_default()
        };
        state.repos.push(RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_worktree_menu_fingerprint"),
            },
        ));
        let changes = |popover: PopoverKind, bump: fn(&mut RepoState)| {
            let mut state = state.clone();
            let before = notify_fingerprint(&state, &popover, None);
            bump(&mut state.repos[0]);
            before != notify_fingerprint(&state, &popover, None)
        };
        let bump_worktrees: fn(&mut RepoState) =
            |repo| repo.worktrees_rev = repo.worktrees_rev.wrapping_add(1);
        let bump_branches: fn(&mut RepoState) =
            |repo| repo.branches_rev = repo.branches_rev.wrapping_add(1);

        assert!(changes(
            PopoverKind::BranchMenu {
                repo_id,
                target: crate::view::BranchMenuTarget::Local {
                    name: "main".to_string(),
                },
            },
            bump_worktrees,
        ));
        assert!(changes(
            PopoverKind::CommitMenu {
                repo_id,
                commit_id: CommitId("deadbeef".into()),
                refs: CommitMenuRefs::All,
            },
            bump_worktrees,
        ));
        assert!(changes(
            PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt),
            bump_branches,
        ));
    }

    #[test]
    fn pull_picker_fingerprint_changes_when_branches_rev_changes() {
        let repo_id = RepoId(9);
        let mut repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_pull_picker_fingerprint"),
            },
        );
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: "main".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: Some(Upstream {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            }),
            divergence: None,
        }]));

        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::test_default()
        };
        state.repos.push(repo);

        let before = notify_fingerprint(&state, &PopoverKind::PullPicker, None);
        state.repos[0].branches_rev = state.repos[0].branches_rev.wrapping_add(1);
        let after = notify_fingerprint(&state, &PopoverKind::PullPicker, None);

        assert_ne!(before, after);
    }

    #[test]
    fn pull_picker_fingerprint_changes_between_failed_snapshots_without_a_running_snapshot() {
        use gitcomet_state::model::AutoFetchStatus;
        let at = std::time::UNIX_EPOCH;
        let failed = |at, message: &str| AutoFetchStatus::Failed {
            at,
            error: gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Backend(
                message.into(),
            )),
        };
        let repo_id = RepoId(9);
        let mut repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_pull_picker_failed_fetch_fingerprint"),
            },
        );
        repo.auto_fetch.last_success_at = Some(at);
        repo.auto_fetch.status = failed(at, "network disconnected");
        let mut state = AppState {
            active_repo: Some(repo_id),
            repos: vec![repo],
            ..AppState::test_default()
        };
        state.remote_settings.auto_fetch.enabled = true;
        let first = notify_fingerprint(&state, &PopoverKind::PullPicker, None);
        state.repos[0].auto_fetch.status = failed(at, "authentication required");
        let second = notify_fingerprint(&state, &PopoverKind::PullPicker, None);
        assert_ne!(first, second);
        state.repos[0].auto_fetch.status = failed(
            at + std::time::Duration::from_secs(60),
            "authentication required",
        );
        assert_ne!(
            second,
            notify_fingerprint(&state, &PopoverKind::PullPicker, None)
        );
        let failure = gitcomet_core::error::GitFailure::new(
            "git fetch --all",
            gitcomet_core::error::GitFailureId::CommandFailed,
            Some(128),
            Vec::new(),
            Vec::new(),
            Some("authentication required".into()),
        );
        state.repos[0].auto_fetch.status = AutoFetchStatus::Failed {
            at,
            error: gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Git(
                failure.clone(),
            )),
        };
        let without_auth_hint = notify_fingerprint(&state, &PopoverKind::PullPicker, None);
        state.repos[0].auto_fetch.status = AutoFetchStatus::Failed {
            at,
            error: gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Git(
                failure.with_auth_outcome(gitcomet_core::auth::AuthOutcome::NeedsInput(
                    gitcomet_core::auth::AuthChallenge::from_prompt(
                        gitcomet_core::auth::GitAuthKind::UsernamePassword,
                        "Password for test remote:",
                    ),
                )),
            )),
        };
        let with_auth_hint = notify_fingerprint(&state, &PopoverKind::PullPicker, None);
        assert_ne!(without_auth_hint, with_auth_hint);
        state.repos[0].auto_fetch.retry_at = Some(at + std::time::Duration::from_secs(3600));
        let retrying = notify_fingerprint(&state, &PopoverKind::PullPicker, None);
        assert_ne!(with_auth_hint, retrying);
        state.repos[0].auto_fetch.paused_remotes = vec!["origin".into()];
        assert_ne!(
            retrying,
            notify_fingerprint(&state, &PopoverKind::PullPicker, None)
        );
    }

    #[test]
    fn explorer_settings_menu_fingerprint_changes_when_file_browser_rev_changes() {
        let repo_id = RepoId(9);
        let repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_explorer_settings_fingerprint"),
            },
        );
        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::test_default()
        };
        state.repos.push(repo);

        let kind = PopoverKind::ExplorerSettingsMenu { repo_id };
        let before = notify_fingerprint(&state, &kind, None);
        state.repos[0].file_browser.show_ignored = true;
        state.repos[0].file_browser.bump_rev();
        let after = notify_fingerprint(&state, &kind, None);

        assert_ne!(before, after, "the check marks read the visibility flags");
    }

    #[test]
    fn pull_and_push_picker_fingerprints_follow_annex_takeover() {
        for popover in [PopoverKind::PullPicker, PopoverKind::PushPicker] {
            let repo_id = RepoId(9);
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: "/tmp/repo".into(),
                },
            );
            repo.head_branch = Loadable::Ready("adjusted/main(unlocked)".into());
            let mut state = AppState {
                active_repo: Some(repo_id),
                repos: vec![repo],
                ..AppState::test_default()
            };
            let before = notify_fingerprint(&state, &popover, None);
            let mut support = gitcomet_core::large_files::LargeFileSupport::default();
            support.annex.uuid = Some("u".into());
            state.repos[0].large_file_support = Loadable::Ready(Arc::new(support));
            let detected = notify_fingerprint(&state, &popover, None);
            assert_eq!(
                before, detected,
                "unknown support already protects the adjusted branch"
            );
            state.repos[0].large_file_support = Loadable::Ready(Arc::default());
            assert_ne!(detected, notify_fingerprint(&state, &popover, None));
            state.repos[0].large_file_support = Loadable::Loading;
            state.large_file_settings.annex_pull_push = false;
            assert_ne!(detected, notify_fingerprint(&state, &popover, None));
        }
    }

    /// The LFS and git-annex entries under Pull and Push come from the support
    /// summary, the installed tools and the running pull or push. An open
    /// menu must repaint when any of them changes.
    #[test]
    fn pull_and_push_picker_fingerprints_follow_large_file_entries() {
        use gitcomet_core::large_file_tools::ToolAvailability;
        for popover in [PopoverKind::PullPicker, PopoverKind::PushPicker] {
            let repo_id = RepoId(9);
            let repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: "/tmp/repo".into(),
                },
            );
            let mut state = AppState {
                active_repo: Some(repo_id),
                repos: vec![repo],
                ..AppState::test_default()
            };
            let mut previous = notify_fingerprint(&state, &popover, None);
            let mut assert_changed = |state: &AppState, what: &str| {
                let next = notify_fingerprint(state, &popover, None);
                assert_ne!(previous, next, "{popover:?}: {what}");
                previous = next;
            };

            let mut support = gitcomet_core::large_files::LargeFileSupport::default();
            support.annex.uuid = Some("u".into());
            state.repos[0].large_file_support = Loadable::Ready(Arc::new(support));
            state.repos[0].large_file_support_rev += 1;
            assert_changed(&state, "support loaded");
            state.large_file_tools.git_annex = ToolAvailability::NotFound {
                detail: String::new(),
            };
            assert_changed(&state, "git-annex missing");
            state.large_file_tools.git_lfs = ToolAvailability::NotFound {
                detail: String::new(),
            };
            assert_changed(&state, "git-lfs missing");
            state.repos[0].push_in_flight = 1;
            assert_changed(&state, "push started");
            state.repos[0].worktree_pull_in_flight = 1;
            assert_changed(&state, "pull started");
        }
    }

    #[test]
    fn app_menu_fingerprint_changes_when_the_active_repos_remotes_change() {
        let repo_id = RepoId(9);
        let repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_app_menu_remotes_fingerprint"),
            },
        );
        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::test_default()
        };
        state.repos.push(repo);

        let before = notify_fingerprint(&state, &PopoverKind::AppMenu, None);
        state.repos[0].remotes_rev = state.repos[0].remotes_rev.wrapping_add(1);
        let after = notify_fingerprint(&state, &PopoverKind::AppMenu, None);

        assert_ne!(before, after, "the Open remote row reads the remotes");
    }

    #[test]
    fn remote_picker_fingerprint_changes_when_remotes_change() {
        let repo_id = RepoId(9);
        let repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_remote_picker_fingerprint"),
            },
        );
        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::test_default()
        };
        state.repos.push(repo);
        let picker = PopoverKind::remote(repo_id, RemotePopoverKind::OpenInBrowserMenu);

        let before = notify_fingerprint(&state, &picker, None);
        state.repos[0].remotes_rev = state.repos[0].remotes_rev.wrapping_add(1);
        let after = notify_fingerprint(&state, &picker, None);

        assert_ne!(before, after, "the picker's rows are the remotes");
    }

    /// A different popover over the same state is a repaint: the popover is
    /// compared itself, so no field of it can be left out of a hash.
    #[test]
    fn a_different_popover_over_the_same_state_is_recorded_as_changed() {
        let state = AppState::test_default();
        let menu = |name: &str| {
            PopoverKind::remote(
                RepoId(9),
                RemotePopoverKind::Menu {
                    name: name.to_string(),
                },
            )
        };
        let mut shown = NotifyFingerprint::default();

        assert!(shown.record(&state, &menu("origin"), None));
        assert!(!shown.record(&state, &menu("origin"), None));
        assert!(shown.record(&state, &menu("upstream"), None));
        assert!(shown.record(
            &state,
            &PopoverKind::remote(RepoId(9), RemotePopoverKind::OpenInBrowserMenu),
            None
        ));
        assert!(shown.record(
            &state,
            &PopoverKind::remote(RepoId(2), RemotePopoverKind::OpenInBrowserMenu),
            None
        ));
        shown.clear();
        assert!(shown.record(
            &state,
            &PopoverKind::remote(RepoId(2), RemotePopoverKind::OpenInBrowserMenu),
            None
        ));
    }

    #[test]
    fn commit_options_fingerprint_changes_when_amend_availability_revisions_change() {
        let repo_id = RepoId(9);
        let repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_commit_options_fingerprint"),
            },
        );
        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::test_default()
        };
        state.repos.push(repo);

        let popover = PopoverKind::CommitOptionsMenu { repo_id };
        let before = notify_fingerprint(&state, &popover, None);

        state.repos[0].head_branch_rev = state.repos[0].head_branch_rev.wrapping_add(1);
        let after_head_branch = notify_fingerprint(&state, &popover, None);
        assert_ne!(before, after_head_branch);

        state.repos[0].branches_rev = state.repos[0].branches_rev.wrapping_add(1);
        assert_ne!(
            after_head_branch,
            notify_fingerprint(&state, &popover, None)
        );
    }

    #[test]
    fn commit_menu_fingerprint_changes_when_history_tag_visibility_changes() {
        let repo_id = RepoId(9);
        let repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_commit_menu_fingerprint"),
            },
        );
        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::test_default()
        };
        state.repos.push(repo);

        let popover = PopoverKind::CommitMenu {
            repo_id,
            commit_id: CommitId("deadbeef".into()),
            refs: CommitMenuRefs::All,
        };
        let before = notify_fingerprint(&state, &popover, None);
        state.git_log_settings.show_history_tags = !state.git_log_settings.show_history_tags;

        assert_ne!(before, notify_fingerprint(&state, &popover, None));
    }

    #[test]
    fn force_push_confirm_fingerprint_changes_when_pending_lease_changes() {
        let repo_id = RepoId(9);
        let repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_force_push_fingerprint"),
            },
        );
        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::test_default()
        };
        state.repos.push(repo);

        let before = notify_fingerprint(&state, &PopoverKind::ForcePushConfirm { repo_id }, None);
        state.repos[0].pending.force_push_lease = Some(test_force_push_lease());
        let after = notify_fingerprint(&state, &PopoverKind::ForcePushConfirm { repo_id }, None);

        assert_ne!(before, after);
    }

    #[test]
    fn push_picker_fingerprint_changes_when_pending_lease_changes() {
        let repo_id = RepoId(9);
        let repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet_push_picker_lease_fingerprint"),
            },
        );
        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::test_default()
        };
        state.repos.push(repo);

        let before = notify_fingerprint(&state, &PopoverKind::PushPicker, None);
        state.repos[0].pending.force_push_lease = Some(test_force_push_lease());
        let after = notify_fingerprint(&state, &PopoverKind::PushPicker, None);

        assert_ne!(before, after);
    }

    #[test]
    fn repo_picker_fingerprint_changes_with_active_repo() {
        let base = AppState::test_default();
        let with_active_repo = AppState {
            active_repo: Some(RepoId(99)),
            ..AppState::test_default()
        };

        let before = notify_fingerprint(
            &base,
            &PopoverKind::RepoPicker {
                scope: RepoPickerScope::All,
            },
            None,
        );
        let after = notify_fingerprint(
            &with_active_repo,
            &PopoverKind::RepoPicker {
                scope: RepoPickerScope::All,
            },
            None,
        );

        assert_ne!(before, after);
    }
}
