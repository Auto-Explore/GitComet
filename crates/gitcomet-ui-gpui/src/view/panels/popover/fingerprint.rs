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
            } else {
                state.active_repo.hash(&mut hasher);
            }
        }
    }

    hasher.finish()
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
        // The pinned list's "Unpin all (N)" reads the live branch lists, so a
        // refresh landing while it is up has to repaint it.
        | PopoverKind::SidebarPinnedOverflow { .. }
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
        }

        // File counts come from the lists' files, and Apply follows the busy
        // state. Collapse state is view-local and only changes by closing it.
        PopoverKind::DiscardFolderChangesConfirm { .. } => {
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
        PopoverKind::StashDropConfirm { .. } | PopoverKind::StashPickerPrompt { .. } => {
            repo.stashes_rev.hash(hasher);
        }
        PopoverKind::FileHistoryFilePicker { .. } => {
            repo.file_browser.file_browser_rev.hash(hasher);
            repo.status_rev.hash(hasher);
            diff.diff_target_rev.hash(hasher);
        }

        PopoverKind::DiscardChangesConfirm { .. } => {
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

        PopoverKind::HistoryAuthorFilter { .. } => {
            repo.history_state.history_author_filter.hash(hasher);
            repo.history_state.history_scope.hash(hasher);
            repo.history_state.authors.rev.hash(hasher);
            repo.load_epoch.hash(hasher);
            repo.history_state.log_rev.hash(hasher);
        }

        PopoverKind::PullReconcilePrompt { .. } | PopoverKind::ForcePushConfirm { .. } => {
            repo.head_branch_rev.hash(hasher);
            repo.branches_rev.hash(hasher);
            repo.remotes_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
            hash_pending_force_push_lease(repo, hasher);
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
        | PopoverKind::CommitPrompt { .. }
        // A menu repaints when its model changes, whatever state that read.
        | PopoverKind::Repo {
            kind: RepoPopoverKind::RepositoryModeMenu,
            ..
        }
        | PopoverKind::BranchMenu { .. }
        | PopoverKind::BranchSectionMenu { .. }
        | PopoverKind::BranchGroupMenu { .. }
        | PopoverKind::SidebarAncestorMenu { .. }
        | PopoverKind::FileBrowserFolderMenu { .. }
        | PopoverKind::FileListFolderMenu { .. }
        | PopoverKind::StashMenu { .. }
        | PopoverKind::DiffHunkMenu { .. }
        | PopoverKind::DiffEditorMenu { .. }
        | PopoverKind::HistoryBranchFilter { .. }
        | PopoverKind::PullPicker
        | PopoverKind::PushPicker
        | PopoverKind::PreviousCommitMessagesMenu { .. }
        | PopoverKind::RepoTabMenu { .. }
        | PopoverKind::CommitMenu { .. }
        | PopoverKind::FileHistoryCommitMenu { .. }
        | PopoverKind::TagMenu { .. }
        | PopoverKind::CommitFileMenu { .. }
        | PopoverKind::WorktreeFileMenu { .. }
        | PopoverKind::CommitRangeFileMenu { .. }
        | PopoverKind::ExplorerSettingsMenu { .. } => {}
        PopoverKind::Repo {
            kind: RepoPopoverKind::DisableBareModeConfirm,
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
    use gitcomet_core::domain::CommitId;

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

    /// The Add dialog refuses a branch name that already exists.
    #[test]
    fn the_worktree_add_prompt_repaints_when_branches_change() {
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
        let popover = PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt);
        let before = notify_fingerprint(&state, &popover, None);
        state.repos[0].branches_rev = state.repos[0].branches_rev.wrapping_add(1);
        assert_ne!(before, notify_fingerprint(&state, &popover, None));
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
