use super::*;
use crate::view::fingerprint as view_fingerprint;
use gitcomet_state::model::CloneProgressStage;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

pub(super) fn notify_fingerprint(state: &AppState, popover: &PopoverKind) -> u64 {
    let mut hasher = FxHasher::default();
    hash_popover_kind(popover, &mut hasher);

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
        PopoverKind::RepoPicker => {
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
        | PopoverKind::MarkdownLinkMenu { .. }
        | PopoverKind::DiffActionMenu
        | PopoverKind::MergetoolSettingsMenu
        | PopoverKind::ChangeTrackingSettings
        | PopoverKind::UiScalePicker
        | PopoverKind::AppMenu
        | PopoverKind::AddRepoMenu => {
            // Mostly local UI state; depend only on whether a repo is active/open.
            state.active_repo.hash(&mut hasher);
            if let Some(repo) = repo_for_popover(state, popover) {
                view_fingerprint::hash_loadable_kind(&repo.open, &mut hasher);
            }
        }
        PopoverKind::Repo {
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::TrustConfirm),
            ..
        } => {
            if let Some(repo) = repo_for_popover(state, popover) {
                hash_repo_for_popover(repo, popover, &mut hasher);
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
            if let Some(repo) = repo_for_popover(state, popover) {
                hash_repo_for_popover(repo, popover, &mut hasher);
            } else {
                state.active_repo.hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}

fn repo_for_popover<'a>(state: &'a AppState, popover: &PopoverKind) -> Option<&'a RepoState> {
    let repo_id = match popover {
        PopoverKind::RepoPicker
        | PopoverKind::CloneRepo
        | PopoverKind::DiffContentModeSettings
        | PopoverKind::MarkdownLinkMenu { .. }
        | PopoverKind::DiffActionMenu
        | PopoverKind::MergetoolSettingsMenu
        | PopoverKind::ChangeTrackingSettings
        | PopoverKind::UiScalePicker => None,

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
        | PopoverKind::UnsavedFileEditsConfirm(_)
        | PopoverKind::ConflictResolverInputRowMenu { .. }
        | PopoverKind::ConflictResolverChunkMenu { .. }
        | PopoverKind::ConflictResolverOutputMenu { .. } => state.active_repo,

        // Popovers that carry an explicit repo id.
        PopoverKind::CommitPrompt { repo_id }
        | PopoverKind::StashPickerPrompt { repo_id, .. }
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
        | PopoverKind::FileHistory { repo_id, .. }
        | PopoverKind::PushSetUpstreamPrompt { repo_id, .. }
        | PopoverKind::ForcePushConfirm { repo_id }
        | PopoverKind::CherryPickCommitConfirm { repo_id, .. }
        | PopoverKind::MergeAbortConfirm { repo_id }
        | PopoverKind::ForceDeleteBranchConfirm { repo_id, .. }
        | PopoverKind::ForceRemoveWorktreeConfirm { repo_id, .. }
        | PopoverKind::DiscardChangesConfirm { repo_id, .. }
        | PopoverKind::StageConflictMarkersConfirm { repo_id, .. }
        | PopoverKind::PullReconcilePrompt { repo_id }
        | PopoverKind::RebaseOntoConfirm { repo_id, .. }
        | PopoverKind::CommitOptionsMenu { repo_id }
        | PopoverKind::PreviousCommitMessagesMenu { repo_id }
        | PopoverKind::DiffHunkMenu { repo_id, .. }
        | PopoverKind::DiffEditorMenu { repo_id, .. }
        | PopoverKind::CommitMenu { repo_id, .. }
        | PopoverKind::StatusFileMenu { repo_id, .. }
        | PopoverKind::BranchMenu { repo_id, .. }
        | PopoverKind::BranchSectionMenu { repo_id, .. }
        | PopoverKind::CommitFileMenu { repo_id, .. }
        | PopoverKind::FileBrowserFileMenu { repo_id, .. }
        | PopoverKind::BrowseHistoryMenu { repo_id }
        | PopoverKind::SubmoduleInnerDiffMenu { repo_id, .. }
        | PopoverKind::TagMenu { repo_id, .. }
        | PopoverKind::TerminalMenu { repo_id, .. }
        | PopoverKind::TagRefMenu { repo_id, .. }
        | PopoverKind::HistoryBranchFilter { repo_id }
        | PopoverKind::HistoryAuthorFilter { repo_id } => Some(*repo_id),
    }?;

    state.repos.iter().find(|r| r.id == repo_id)
}

fn hash_repo_for_popover<H: Hasher>(repo: &RepoState, popover: &PopoverKind, hasher: &mut H) {
    view_fingerprint::hash_loadable_kind(&repo.open, hasher);

    match popover {
        PopoverKind::BranchPicker { .. }
        | PopoverKind::CreateBranchFromRefPrompt { .. }
        | PopoverKind::RenameBranchPrompt { .. }
        | PopoverKind::BranchMenu { .. }
        | PopoverKind::BranchSectionMenu { .. }
        | PopoverKind::ForceDeleteBranchConfirm { .. }
        | PopoverKind::PushSetUpstreamPrompt { .. } => {
            repo.head_branch_rev.hash(hasher);
            repo.branches_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
            repo.tags_rev.hash(hasher);
            // The checkout picker's rows carry each ref's author, date and
            // summary on their detail line, so metadata landing while the picker
            // is open has to repaint it — the rows change height, not just text.
            repo.ref_metadata_rev.hash(hasher);
        }

        PopoverKind::Repo {
            kind: RepoPopoverKind::Remote(_),
            ..
        } => {
            repo.remotes_rev.hash(hasher);
            repo.remote_branches_rev.hash(hasher);
        }

        PopoverKind::Repo {
            kind: RepoPopoverKind::Worktree(_),
            ..
        } => {
            repo.worktrees_rev.hash(hasher);
            // The badge picker's create row reads HEAD for its "Based off <ref>"
            // line (`workspace_picker::create_base_ref`), so a checkout landing
            // while the picker is open has to repaint it — otherwise the row keeps
            // promising a base the Add dialog will no longer use.
            repo.head_branch_rev.hash(hasher);
        }

        PopoverKind::Repo {
            kind: RepoPopoverKind::Submodule(_),
            ..
        } => {
            repo.submodules_rev.hash(hasher);
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

        PopoverKind::FileHistory { .. } => {
            repo.history_state.file_history_path.hash(hasher);
            view_fingerprint::hash_loadable_arc(&repo.history_state.file_history, hasher);
        }

        PopoverKind::DiffHunkMenu { .. }
        | PopoverKind::DiffEditorMenu { .. }
        | PopoverKind::DiscardChangesConfirm { .. } => {
            repo.diff_state.diff_rev.hash(hasher);
            if let Some(t) = repo.diff_state.diff_target.as_ref() {
                view_fingerprint::hash_diff_target(t, hasher)
            }
            view_fingerprint::hash_loadable_arc(&repo.diff_state.diff, hasher);
            repo.diff_state.diff_file_rev.hash(hasher);
            view_fingerprint::hash_loadable_kind(&repo.diff_state.diff_file, hasher);
            view_fingerprint::hash_loadable_kind(&repo.diff_state.diff_file_image, hasher);

            // Working tree diff popovers need status for file-kind/conflict decisions.
            if matches!(
                repo.diff_state.diff_target,
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
            // Author suggestions come from the loaded log pages.
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

        PopoverKind::TagMenu { .. } | PopoverKind::TagRefMenu { .. } => {
            repo.tags_rev.hash(hasher);
            repo.remotes_rev.hash(hasher);
            repo.remote_tags_rev.hash(hasher);
        }

        // Most prompt-style popovers don't require live state updates.
        PopoverKind::InteractiveRebaseActionMenu { .. }
        | PopoverKind::InteractiveRebaseAutosquashMenu
        | PopoverKind::RebaseReword { .. }
        | PopoverKind::RebaseOntoConfirm { .. }
        | PopoverKind::CherryPickCommitConfirm { .. }
        | PopoverKind::MergeAbortConfirm { .. }
        | PopoverKind::ResetPrompt { .. }
        | PopoverKind::CheckoutRemoteBranchPrompt { .. }
        | PopoverKind::CreateTagPrompt { .. }
        | PopoverKind::ForceRemoveWorktreeConfirm { .. }
        | PopoverKind::CommitMenu { .. }
        | PopoverKind::CommitFileMenu { .. }
        | PopoverKind::FileBrowserFileMenu { .. }
        | PopoverKind::BrowseHistoryMenu { .. }
        | PopoverKind::SubmoduleInnerDiffMenu { .. }
        | PopoverKind::StatusFileMenu { .. }
        | PopoverKind::StageConflictMarkersConfirm { .. }
        | PopoverKind::DiffContentModeSettings
        | PopoverKind::MarkdownLinkMenu { .. }
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
        | PopoverKind::UnsavedFileEditsConfirm(_)
        | PopoverKind::TerminalMenu { .. }
        | PopoverKind::RepoPicker
        | PopoverKind::CloneRepo
        | PopoverKind::CommitPrompt { .. } => {}
    }
}

fn hash_pending_force_push_lease(repo: &RepoState, hasher: &mut impl Hasher) {
    match &repo.pending_force_push_lease {
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

fn hash_popover_kind<H: Hasher>(kind: &PopoverKind, hasher: &mut H) {
    match kind {
        PopoverKind::RepoPicker => 0u8.hash(hasher),
        PopoverKind::AddRepoMenu => 66u8.hash(hasher),
        PopoverKind::BranchPicker { purpose } => {
            1u8.hash(hasher);
            (*purpose as u8).hash(hasher);
        }
        PopoverKind::CreateBranchFromRefPrompt {
            repo_id,
            target,
            source_selectable,
        } => {
            66u8.hash(hasher);
            repo_id.hash(hasher);
            target.hash(hasher);
            source_selectable.hash(hasher);
        }
        PopoverKind::RenameBranchPrompt {
            repo_id,
            name,
            is_current_branch,
        } => {
            80u8.hash(hasher);
            repo_id.hash(hasher);
            name.hash(hasher);
            is_current_branch.hash(hasher);
        }
        PopoverKind::CheckoutRemoteBranchPrompt {
            repo_id,
            remote,
            branch,
        } => {
            50u8.hash(hasher);
            repo_id.hash(hasher);
            remote.hash(hasher);
            branch.hash(hasher);
        }
        PopoverKind::StashPrompt => 3u8.hash(hasher),
        PopoverKind::StashDropConfirm {
            repo_id,
            index,
            message,
        } => {
            55u8.hash(hasher);
            repo_id.hash(hasher);
            index.hash(hasher);
            message.hash(hasher);
        }
        PopoverKind::StashMenu {
            repo_id,
            index,
            message,
        } => {
            56u8.hash(hasher);
            repo_id.hash(hasher);
            index.hash(hasher);
            message.hash(hasher);
        }
        PopoverKind::CloneRepo => 4u8.hash(hasher),
        PopoverKind::ChangeTrackingSettings => 66u8.hash(hasher),
        PopoverKind::DiffContentModeSettings => 67u8.hash(hasher),
        PopoverKind::UiScalePicker => 68u8.hash(hasher),
        PopoverKind::MarkdownLinkMenu { url } => {
            96u8.hash(hasher);
            url.hash(hasher);
        }
        PopoverKind::DiffActionMenu => 69u8.hash(hasher),
        PopoverKind::MergetoolSettingsMenu => 75u8.hash(hasher),
        PopoverKind::CommitOptionsMenu { repo_id } => {
            70u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::PreviousCommitMessagesMenu { repo_id } => {
            71u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::RepoTabMenu { repo_id } => {
            72u8.hash(hasher);
            repo_id.hash(hasher);
        }

        PopoverKind::ResetPrompt {
            repo_id,
            target,
            mode,
        } => {
            6u8.hash(hasher);
            repo_id.hash(hasher);
            target.hash(hasher);
            hash_reset_mode(*mode, hasher);
        }
        PopoverKind::SquashPrompt { repo_id } => {
            75u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::CreateTagPrompt { repo_id, target } => {
            8u8.hash(hasher);
            repo_id.hash(hasher);
            target.hash(hasher);
        }
        PopoverKind::Repo { repo_id, kind } => {
            hash_repo_popover_kind(*repo_id, kind, hasher);
        }

        PopoverKind::FileHistory { repo_id, path } => {
            28u8.hash(hasher);
            repo_id.hash(hasher);
            path.hash(hasher);
        }
        PopoverKind::PushSetUpstreamPrompt { repo_id, remote } => {
            30u8.hash(hasher);
            repo_id.hash(hasher);
            remote.hash(hasher);
        }
        PopoverKind::ForcePushConfirm { repo_id } => {
            31u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::CherryPickCommitConfirm { repo_id, commit_id } => {
            76u8.hash(hasher);
            repo_id.hash(hasher);
            commit_id.hash(hasher);
        }
        PopoverKind::ForceDeleteBranchConfirm { repo_id, name } => {
            32u8.hash(hasher);
            repo_id.hash(hasher);
            name.hash(hasher);
        }
        PopoverKind::ForceRemoveWorktreeConfirm {
            repo_id,
            path,
            branch,
        } => {
            61u8.hash(hasher);
            repo_id.hash(hasher);
            path.hash(hasher);
            branch.hash(hasher);
        }
        PopoverKind::DiscardChangesConfirm {
            repo_id,
            area,
            path,
        } => {
            34u8.hash(hasher);
            repo_id.hash(hasher);
            hash_diff_area(*area, hasher);
            path.hash(hasher);
        }
        PopoverKind::StageConflictMarkersConfirm {
            repo_id,
            paths,
            unresolved,
            clear_selection,
        } => {
            81u8.hash(hasher);
            repo_id.hash(hasher);
            paths.hash(hasher);
            unresolved.hash(hasher);
            clear_selection.hash(hasher);
        }
        PopoverKind::PullReconcilePrompt { repo_id } => {
            35u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::PullPicker => 36u8.hash(hasher),
        PopoverKind::PushPicker => 37u8.hash(hasher),
        PopoverKind::AppMenu => 38u8.hash(hasher),
        PopoverKind::TerminalShutdownConfirm(prompt) => {
            67u8.hash(hasher);
            prompt.action.hash(hasher);
            prompt.summary.terminal_count.hash(hasher);
            prompt.summary.running_command_count.hash(hasher);
            prompt.summary.repo_names.hash(hasher);
        }
        PopoverKind::UnsavedFileEditsConfirm(prompt) => {
            68u8.hash(hasher);
            prompt.action.hash(hasher);
            prompt.files.hash(hasher);
        }
        PopoverKind::DiffHunkMenu { repo_id, src_ix } => {
            40u8.hash(hasher);
            repo_id.hash(hasher);
            src_ix.hash(hasher);
        }
        PopoverKind::DiffEditorMenu {
            repo_id,
            area,
            path,
            hunks_count,
            lines_count,
            ..
        } => {
            41u8.hash(hasher);
            repo_id.hash(hasher);
            hash_diff_area(*area, hasher);
            path.hash(hasher);
            hunks_count.hash(hasher);
            lines_count.hash(hasher);
        }
        PopoverKind::ConflictResolverInputRowMenu {
            line_label,
            line_target,
            chunk_label,
            chunk_target,
        } => {
            53u8.hash(hasher);
            line_label.hash(hasher);
            line_target.hash(hasher);
            chunk_label.hash(hasher);
            chunk_target.hash(hasher);
        }
        PopoverKind::ConflictResolverChunkMenu {
            conflict_ix,
            has_base,
            is_three_way,
            selected_choices,
            output_line_ix,
            split_selection_rows,
            join_previous_region,
            join_next_region,
            alignment_marked_columns,
            has_manual_alignments,
            output_is_protected,
        } => {
            59u8.hash(hasher);
            conflict_ix.hash(hasher);
            has_base.hash(hasher);
            is_three_way.hash(hasher);
            selected_choices.hash(hasher);
            output_line_ix.hash(hasher);
            split_selection_rows.hash(hasher);
            join_previous_region.hash(hasher);
            join_next_region.hash(hasher);
            alignment_marked_columns.hash(hasher);
            has_manual_alignments.hash(hasher);
            output_is_protected.hash(hasher);
        }
        PopoverKind::ConflictResolverOutputMenu {
            cursor_line,
            selected_text,
            has_source_a,
            has_source_b,
            has_source_c,
            is_three_way,
        } => {
            54u8.hash(hasher);
            cursor_line.hash(hasher);
            selected_text.hash(hasher);
            has_source_a.hash(hasher);
            has_source_b.hash(hasher);
            has_source_c.hash(hasher);
            is_three_way.hash(hasher);
        }
        PopoverKind::CommitMenu { repo_id, commit_id } => {
            42u8.hash(hasher);
            repo_id.hash(hasher);
            commit_id.hash(hasher);
        }
        PopoverKind::StatusFileMenu {
            repo_id,
            area,
            path,
        } => {
            43u8.hash(hasher);
            repo_id.hash(hasher);
            hash_diff_area(*area, hasher);
            path.hash(hasher);
        }
        PopoverKind::BranchMenu {
            repo_id,
            section,
            name,
        } => {
            44u8.hash(hasher);
            repo_id.hash(hasher);
            hash_branch_section(*section, hasher);
            name.hash(hasher);
        }
        PopoverKind::BranchSectionMenu { repo_id, section } => {
            45u8.hash(hasher);
            repo_id.hash(hasher);
            hash_branch_section(*section, hasher);
        }
        PopoverKind::CommitFileMenu {
            repo_id,
            commit_id,
            path,
        } => {
            46u8.hash(hasher);
            repo_id.hash(hasher);
            commit_id.hash(hasher);
            path.hash(hasher);
        }
        PopoverKind::FileBrowserFileMenu { repo_id, path } => {
            62u8.hash(hasher);
            repo_id.hash(hasher);
            path.hash(hasher);
        }
        PopoverKind::BrowseHistoryMenu { repo_id } => {
            63u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::SubmoduleInnerDiffMenu {
            repo_id,
            submodule_repo_path,
            target,
        } => {
            60u8.hash(hasher);
            repo_id.hash(hasher);
            submodule_repo_path.hash(hasher);
            view_fingerprint::hash_diff_target(target, hasher);
        }
        PopoverKind::TagMenu { repo_id, commit_id } => {
            47u8.hash(hasher);
            repo_id.hash(hasher);
            commit_id.hash(hasher);
        }
        PopoverKind::TagRefMenu {
            repo_id,
            commit_id,
            name,
        } => {
            72u8.hash(hasher);
            repo_id.hash(hasher);
            commit_id.hash(hasher);
            name.hash(hasher);
        }
        PopoverKind::HistoryBranchFilter { repo_id } => {
            48u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::HistoryAuthorFilter { repo_id } => {
            97u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::TerminalMenu { repo_id, context } => {
            72u8.hash(hasher);
            repo_id.hash(hasher);
            context.hash(hasher);
        }
        PopoverKind::MergeAbortConfirm { repo_id } => {
            51u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::CommitPrompt { repo_id } => {
            73u8.hash(hasher);
            repo_id.hash(hasher);
        }
        PopoverKind::StashPickerPrompt { repo_id, purpose } => {
            74u8.hash(hasher);
            repo_id.hash(hasher);
            (*purpose as u8).hash(hasher);
        }
        PopoverKind::RebaseOntoConfirm { repo_id, onto } => {
            75u8.hash(hasher);
            repo_id.hash(hasher);
            onto.hash(hasher);
        }
        PopoverKind::RebaseReword {
            ix,
            original_action,
            original_message,
        } => {
            77u8.hash(hasher);
            ix.hash(hasher);
            (*original_action as u8).hash(hasher);
            original_message.hash(hasher);
        }
        PopoverKind::InteractiveRebaseActionMenu {
            ix,
            can_squash,
            can_drop,
        } => {
            78u8.hash(hasher);
            ix.hash(hasher);
            can_squash.hash(hasher);
            can_drop.hash(hasher);
            can_squash.hash(hasher);
        }
        PopoverKind::InteractiveRebaseAutosquashMenu => {
            79u8.hash(hasher);
        }
    }
}

fn hash_repo_popover_kind<H: Hasher>(repo_id: RepoId, kind: &RepoPopoverKind, hasher: &mut H) {
    match kind {
        RepoPopoverKind::Remote(remote_kind) => match remote_kind {
            RemotePopoverKind::AddPrompt => {
                9u8.hash(hasher);
                repo_id.hash(hasher);
            }
            RemotePopoverKind::EditUrlPrompt { name, kind } => {
                13u8.hash(hasher);
                repo_id.hash(hasher);
                name.hash(hasher);
                hash_remote_url_kind(*kind, hasher);
            }
            RemotePopoverKind::RemoveConfirm { name } => {
                14u8.hash(hasher);
                repo_id.hash(hasher);
                name.hash(hasher);
            }
            RemotePopoverKind::Menu { name } => {
                15u8.hash(hasher);
                repo_id.hash(hasher);
                name.hash(hasher);
            }
            RemotePopoverKind::DeleteBranchConfirm { remote, branch } => {
                33u8.hash(hasher);
                repo_id.hash(hasher);
                remote.hash(hasher);
                branch.hash(hasher);
            }
        },
        RepoPopoverKind::Worktree(worktree_kind) => match worktree_kind {
            WorktreePopoverKind::SectionMenu => {
                16u8.hash(hasher);
                repo_id.hash(hasher);
            }
            WorktreePopoverKind::Menu { path, branch } => {
                17u8.hash(hasher);
                repo_id.hash(hasher);
                path.hash(hasher);
                branch.hash(hasher);
            }
            WorktreePopoverKind::AddPrompt => {
                20u8.hash(hasher);
                repo_id.hash(hasher);
            }
            WorktreePopoverKind::OpenPicker => {
                21u8.hash(hasher);
                repo_id.hash(hasher);
            }
            WorktreePopoverKind::RemovePicker => {
                22u8.hash(hasher);
                repo_id.hash(hasher);
            }
            WorktreePopoverKind::BadgePicker => {
                36u8.hash(hasher);
                repo_id.hash(hasher);
            }
            WorktreePopoverKind::RemoveConfirm { path, branch } => {
                23u8.hash(hasher);
                repo_id.hash(hasher);
                path.hash(hasher);
                branch.hash(hasher);
            }
        },
        RepoPopoverKind::Submodule(submodule_kind) => match submodule_kind {
            SubmodulePopoverKind::SectionMenu => {
                18u8.hash(hasher);
                repo_id.hash(hasher);
            }
            SubmodulePopoverKind::Menu { path } => {
                19u8.hash(hasher);
                repo_id.hash(hasher);
                path.hash(hasher);
            }
            SubmodulePopoverKind::AddPrompt => {
                24u8.hash(hasher);
                repo_id.hash(hasher);
            }
            SubmodulePopoverKind::ChangePointerPrompt { path } => {
                29u8.hash(hasher);
                repo_id.hash(hasher);
                path.hash(hasher);
            }
            SubmodulePopoverKind::TrustConfirm => {
                28u8.hash(hasher);
                repo_id.hash(hasher);
            }
            SubmodulePopoverKind::OpenPicker => {
                25u8.hash(hasher);
                repo_id.hash(hasher);
            }
            SubmodulePopoverKind::RemovePicker => {
                26u8.hash(hasher);
                repo_id.hash(hasher);
            }
            SubmodulePopoverKind::RemoveConfirm { path } => {
                27u8.hash(hasher);
                repo_id.hash(hasher);
                path.hash(hasher);
            }
        },
    }
}

fn hash_diff_area<H: Hasher>(area: DiffArea, hasher: &mut H) {
    match area {
        DiffArea::Staged => 0u8.hash(hasher),
        DiffArea::Unstaged => 1u8.hash(hasher),
    }
}

fn hash_branch_section<H: Hasher>(section: BranchSection, hasher: &mut H) {
    match section {
        BranchSection::Local => 0u8.hash(hasher),
        BranchSection::Remote => 1u8.hash(hasher),
    }
}

fn hash_remote_url_kind<H: Hasher>(kind: RemoteUrlKind, hasher: &mut H) {
    match kind {
        RemoteUrlKind::Fetch => 0u8.hash(hasher),
        RemoteUrlKind::Push => 1u8.hash(hasher),
    }
}

fn hash_reset_mode<H: Hasher>(mode: ResetMode, hasher: &mut H) {
    match mode {
        ResetMode::Soft => 0u8.hash(hasher),
        ResetMode::Mixed => 1u8.hash(hasher),
        ResetMode::Hard => 2u8.hash(hasher),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Branch, CommitId, Upstream};
    use std::sync::Arc;

    fn hash_kind(kind: PopoverKind) -> u64 {
        let mut hasher = FxHasher::default();
        hash_popover_kind(&kind, &mut hasher);
        hasher.finish()
    }

    fn test_force_push_lease() -> gitcomet_core::services::ForcePushLease {
        gitcomet_core::services::ForcePushLease {
            remote: "origin".to_string(),
            branch: "main".to_string(),
            expected: CommitId("1111111111111111111111111111111111111111".into()),
            local_branch: "main".to_string(),
            local_head: CommitId("2222222222222222222222222222222222222222".into()),
        }
    }

    #[test]
    fn grouped_repo_popover_hash_changes_with_nested_payload() {
        let repo_id = RepoId(7);
        let hash_origin = hash_kind(PopoverKind::remote(
            repo_id,
            RemotePopoverKind::Menu {
                name: "origin".to_string(),
            },
        ));
        let hash_upstream = hash_kind(PopoverKind::remote(
            repo_id,
            RemotePopoverKind::Menu {
                name: "upstream".to_string(),
            },
        ));

        assert_ne!(hash_origin, hash_upstream);
    }

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
            ..Default::default()
        };

        let popover = PopoverKind::worktree(repo_id, WorktreePopoverKind::SectionMenu);
        let resolved = repo_for_popover(&state, &popover).expect("expected repo lookup to work");

        assert_eq!(resolved.id, repo_id);
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
            ..AppState::default()
        };
        state.repos.push(repo);

        let before = notify_fingerprint(&state, &PopoverKind::PullPicker);
        state.repos[0].branches_rev = state.repos[0].branches_rev.wrapping_add(1);
        let after = notify_fingerprint(&state, &PopoverKind::PullPicker);

        assert_ne!(before, after);
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
            ..AppState::default()
        };
        state.repos.push(repo);

        let popover = PopoverKind::CommitOptionsMenu { repo_id };
        let before = notify_fingerprint(&state, &popover);

        state.repos[0].head_branch_rev = state.repos[0].head_branch_rev.wrapping_add(1);
        let after_head_branch = notify_fingerprint(&state, &popover);
        assert_ne!(before, after_head_branch);

        state.repos[0].branches_rev = state.repos[0].branches_rev.wrapping_add(1);
        assert_ne!(after_head_branch, notify_fingerprint(&state, &popover));
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
            ..AppState::default()
        };
        state.repos.push(repo);

        let before = notify_fingerprint(&state, &PopoverKind::ForcePushConfirm { repo_id });
        state.repos[0].pending_force_push_lease = Some(test_force_push_lease());
        let after = notify_fingerprint(&state, &PopoverKind::ForcePushConfirm { repo_id });

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
            ..AppState::default()
        };
        state.repos.push(repo);

        let before = notify_fingerprint(&state, &PopoverKind::PushPicker);
        state.repos[0].pending_force_push_lease = Some(test_force_push_lease());
        let after = notify_fingerprint(&state, &PopoverKind::PushPicker);

        assert_ne!(before, after);
    }

    #[test]
    fn repo_picker_fingerprint_changes_with_active_repo() {
        let base = AppState::default();
        let with_active_repo = AppState {
            active_repo: Some(RepoId(99)),
            ..Default::default()
        };

        let before = notify_fingerprint(&base, &PopoverKind::RepoPicker);
        let after = notify_fingerprint(&with_active_repo, &PopoverKind::RepoPicker);

        assert_ne!(before, after);
    }
}
