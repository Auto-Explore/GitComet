use super::*;
use crate::msg::InternalMsg;

/// The reducer for messages the store's own workers send back.
pub(super) fn reduce(
    inputs: &ReducerInputs,
    repos: &mut RepositoryRuntimes,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    msg: InternalMsg,
) -> Vec<Effect> {
    match msg {
        InternalMsg::BareConversionPreviewLoaded { repo_id, result } => {
            bare_conversion::preview_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::QuiesceRepository {
            common_dir,
            cancellation,
            reply,
        } => {
            if cancellation.is_cancelled() {
                return vec![Effect::Control(
                    crate::msg::ControlEffect::RepositoryQuiesced { reply },
                )];
            }
            bare_conversion::quiesce(repos, state, &common_dir, reply)
        }
        InternalMsg::BareModeDisabled { common_dir, result } => {
            bare_conversion::finished(inputs, repos, state, &common_dir, result)
        }
        InternalMsg::RepositoryModeLoaded { repo_id, result } => {
            bare_conversion::mode_loaded(inputs, repos, state, repo_id, result)
        }
        InternalMsg::AutoFetchUpdated {
            repo_id,
            lifetime,
            revision,
            snapshot,
            result,
        } => auto_fetch::updated(inputs, state, repo_id, lifetime, revision, snapshot, result),
        InternalMsg::AutoFetchRefresh { repo_id, lifetime } => {
            auto_fetch::refresh(state, repo_id, lifetime)
        }
        InternalMsg::AuthPromptValidated {
            generation,
            result,
            auth,
        } => auth::auth_prompt_validated(inputs, repos, id_alloc, state, generation, result, auth),
        InternalMsg::GitOperationStarted {
            repo_id,
            operation_id,
            label,
            context,
            time,
            progress_lane,
        } => git_operations::git_operation_started(
            state,
            repo_id,
            operation_id,
            label,
            context,
            time,
            progress_lane,
        ),
        InternalMsg::GitOperationEvent {
            repo_id,
            operation_id,
            event,
        } => git_operations::git_operation_event(state, repo_id, operation_id, event),
        InternalMsg::GitOperationFinished {
            repo_id,
            operation_id,
            outer_outcome,
            duration,
            message,
        } => git_operations::git_operation_finished(
            inputs,
            repos,
            id_alloc,
            state,
            repo_id,
            operation_id,
            outer_outcome,
            duration,
            message,
        ),
        InternalMsg::SessionPersistFailed {
            repo_id,
            action,
            error,
        } => {
            util::handle_session_persist_result(
                inputs,
                state,
                repo_id,
                action,
                Err(std::io::Error::other(error)),
            );
            Vec::new()
        }
        InternalMsg::CloneRepoProgress { dest, line } => {
            repo_management::clone_repo_progress(state, dest, line)
        }
        InternalMsg::CloneRepoFinished { url, dest, result } => {
            auth::clone_repo_finished(state, url, dest, result)
        }
        InternalMsg::AnnexWhereisLoaded {
            repo_id,
            key,
            result,
        } => {
            if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id)
                && repo.annex_whereis.contains_key(&key)
            {
                let loaded = match result {
                    Ok(whereis) => Loadable::Ready(Arc::new(whereis)),
                    Err(error) => Loadable::Error(error.to_string()),
                };
                repo.set_annex_whereis(key, loaded);
            }
            Vec::new()
        }
        InternalMsg::AnnexUnusedLoaded { repo_id, result } => {
            effects::annex_unused_loaded(state, repo_id, result)
        }
        InternalMsg::LfsLocksLoaded { repo_id, result } => {
            effects::lfs_locks_loaded(state, repo_id, result)
        }
        InternalMsg::RepoMaintenanceChecked { repo_id, needed } => {
            maintenance::checked(state, repo_id, needed)
        }
        InternalMsg::TagPushPreviewLoaded {
            repo_id,
            mode,
            generation,
            result,
        } => loads::tag_push_preview_loaded(state, repo_id, mode, generation, result),
        InternalMsg::RepoOpenedOk {
            repository_mode,
            head_commit,
            repo_id,
            spec,
            repo,
            preferences,
        } => repo_management::repo_opened_ok(
            inputs,
            repos,
            state,
            repo_id,
            spec,
            repo,
            preferences,
            repository_mode,
            head_commit,
        ),
        InternalMsg::RepoLoadFinished {
            repo_id,
            load_epoch,
            message,
        } => {
            loads::repo_load_finished(inputs, repos, id_alloc, state, repo_id, load_epoch, message)
        }
        InternalMsg::RepoOpenedErr {
            repo_id,
            spec,
            error,
        } => repo_management::repo_opened_err(inputs, repos, state, repo_id, spec, error),
        InternalMsg::BranchesLoaded { repo_id, result } => {
            effects::branches_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::RemotesLoaded { repo_id, result } => {
            effects::remotes_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::RemoteBranchesLoaded { repo_id, result } => {
            effects::remote_branches_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::WorktreeStatusLoaded { repo_id, result } => {
            effects::worktree_status_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::StagedStatusLoaded { repo_id, result } => {
            effects::staged_status_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::UncommittedLineStatsLoaded {
            repo_id,
            generation,
            result,
            large_files,
        } => {
            effects::uncommitted_line_stats_loaded(state, repo_id, generation, result, large_files)
        }
        InternalMsg::LargeFileSupportLoaded { repo_id, result } => {
            effects::large_file_support_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::StatusLoaded { repo_id, result } => {
            effects::status_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::HeadBranchLoaded { repo_id, result } => {
            effects::head_branch_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::UpstreamDivergenceLoaded { repo_id, result } => {
            effects::upstream_divergence_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::HeadTrackingLoaded { repo_id, result } => {
            effects::head_tracking_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::LogLoaded {
            repo_id,
            seq,
            scope,
            cursor,
            result,
        } => external_and_history::log_loaded(inputs, state, repo_id, seq, scope, cursor, result),
        InternalMsg::LogChunkLoaded {
            repo_id,
            seq,
            commits,
            scanned,
        } => external_and_history::log_chunk_loaded(state, repo_id, seq, commits, scanned),
        InternalMsg::TagsLoaded { repo_id, result } => {
            effects::tags_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::RemoteTagsLoaded { repo_id, result } => {
            effects::remote_tags_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::StashesLoaded { repo_id, result } => {
            effects::stashes_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::ReflogLoaded { repo_id, result } => {
            effects::reflog_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::RebaseStateLoaded { repo_id, result } => {
            external_and_history::rebase_state_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::InteractiveRebaseSetupLoaded {
            repo_id,
            base,
            result,
        } => external_and_history::interactive_rebase_setup_loaded(
            inputs, state, repo_id, base, result,
        ),
        InternalMsg::InteractiveCherryPickMessagesLoaded {
            repo_id,
            requested_ids,
            result,
        } => external_and_history::interactive_cherry_pick_messages_loaded(
            state,
            repo_id,
            requested_ids,
            result,
        ),
        InternalMsg::CommitMessageSuggestionConsumed { repo_id, message } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
                && repo_state.suggested_commit_message.as_deref() == Some(message.as_str())
            {
                repo_state.set_suggested_commit_message(None);
            }
            Vec::new()
        }
        InternalMsg::CommitMessageSuggested { repo_id, message } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
                repo_state.set_suggested_commit_message(Some(message));
            }
            Vec::new()
        }
        InternalMsg::MergeCommitMessageLoaded { repo_id, result } => {
            external_and_history::merge_commit_message_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::HoverCommitMessageLoaded {
            repo_id,
            commit_id,
            result,
        } => effects::hover_commit_message_loaded(state, repo_id, commit_id, result),
        InternalMsg::FileHistoryLoaded {
            request,
            repo_id,
            path,
            cursor,
            result,
        } => {
            file_history::file_history_loaded(inputs, state, repo_id, request, path, cursor, result)
        }
        InternalMsg::BlameLoaded {
            repo_id,
            path,
            source,
            result,
        } => effects::blame_loaded(inputs, state, repo_id, path, source, result),
        InternalMsg::ConflictFileLoaded {
            request,
            repo_id,
            path,
            result,
            conflict_session,
        } => conflict_load::conflict_file_loaded(
            inputs,
            state,
            repo_id,
            request,
            path,
            *result,
            conflict_session.map(|session| *session),
        ),
        InternalMsg::WorktreesLoaded {
            repo_id,
            read,
            result,
        } => effects::worktrees_loaded(inputs, state, repo_id, read, result),
        InternalMsg::WorktreeDirtyLoaded {
            repo_id,
            scope,
            result,
        } => effects::worktree_dirty_loaded(state, repo_id, scope, result),
        InternalMsg::RefMetadataLoaded { repo_id, result } => {
            effects::ref_metadata_loaded(state, repo_id, result)
        }
        InternalMsg::SubmodulesLoaded { repo_id, result } => {
            effects::submodules_loaded(inputs, state, repo_id, result)
        }
        InternalMsg::FileBrowserLoaded {
            cancellation,
            repo_id,
            source,
            result,
        } => {
            if cancellation.is_some_and(|token| token.is_cancelled()) {
                Vec::new()
            } else {
                effects::file_browser_loaded(inputs, state, repo_id, source, result)
            }
        }
        InternalMsg::SubmoduleAddTrustChecked {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            result,
        } => submodule_trust::add_trust_checked(
            inputs, state, repo_id, url, path, branch, name, force, result,
        ),
        InternalMsg::SubmoduleUpdateTrustChecked { repo_id, result } => {
            submodule_trust::update_trust_checked(inputs, state, repo_id, result)
        }
        InternalMsg::SubmoduleLoadTrustChecked {
            repo_id,
            path,
            result,
        } => submodule_trust::load_trust_checked(inputs, state, repo_id, path, result),
        InternalMsg::CommitDetailsLoaded {
            repo_id,
            commit_id,
            result,
        } => effects::commit_details_loaded(inputs, state, repo_id, commit_id, result),
        InternalMsg::CommitSignaturesVerified {
            repo_id,
            epoch,
            batch,
            result,
        } => effects::commit_signatures_verified(state, repo_id, epoch, batch, result),
        InternalMsg::CommitRevealResolved {
            repo_id,
            reference,
            result,
        } => effects::commit_reveal_resolved(inputs, state, repo_id, reference, result),
        InternalMsg::CommitLookupResolved {
            repo_id,
            reference,
            request,
            purpose,
            result,
        } => effects::commit_lookup_resolved(state, repo_id, reference, request, purpose, result),
        InternalMsg::RangeFilesLoaded {
            repo_id,
            from,
            to,
            request,
            result,
        } => effects::range_files_loaded(inputs, state, repo_id, from, to, request, result),
        InternalMsg::SquashMessagePreviewLoaded {
            repo_id,
            oldest,
            head,
            result,
        } => effects::squash_message_preview_loaded(inputs, state, repo_id, oldest, head, result),
        InternalMsg::SquashRebaseSetupLoaded {
            repo_id,
            base,
            actual_head,
            selected_ids,
            reword_id,
            message,
            count,
            result,
        } => effects::squash_rebase_setup_loaded(
            inputs,
            state,
            repo_id,
            effects::SquashRebaseSetup {
                base,
                actual_head,
                selected_ids,
                reword_id,
                message,
                count,
            },
            result,
        ),
        InternalMsg::RecentCommitMessagesLoaded {
            repo_id,
            request_rev,
            result,
        } => effects::recent_commit_messages_loaded(inputs, state, repo_id, request_rev, result),
        InternalMsg::DiffLoaded {
            repo_id,
            target,
            result,
        } => diff_selection::diff_loaded(inputs, state, repo_id, target, result),
        InternalMsg::DiffFileLoaded {
            repo_id,
            target,
            result,
        } => diff_selection::diff_file_loaded(inputs, state, repo_id, target, result),
        InternalMsg::TextAttributesLoaded {
            repo_id,
            target,
            result,
        } => diff_selection::text_attributes_loaded(inputs, state, repo_id, target, result),
        InternalMsg::DiffPreviewTextFileLoaded {
            repo_id,
            target,
            side,
            result,
        } => diff_selection::diff_preview_text_file_loaded(
            inputs, state, repo_id, target, side, result,
        ),
        InternalMsg::SubmoduleSummaryLoaded {
            repo_id,
            target,
            result,
        } => diff_selection::submodule_summary_loaded(inputs, state, repo_id, target, result),
        InternalMsg::InlineSubmoduleDiffLoaded {
            repo_id,
            inline_rev,
            target,
            result,
        } => diff_selection::inline_submodule_load_finished(
            inputs,
            state,
            repo_id,
            inline_rev,
            target,
            result.map(Arc::new),
            |inline| (&mut inline.diff_rev, &mut inline.diff),
        ),
        InternalMsg::InlineSubmoduleDiffFileLoaded {
            repo_id,
            inline_rev,
            target,
            result,
        } => diff_selection::inline_submodule_load_finished(
            inputs,
            state,
            repo_id,
            inline_rev,
            target,
            result.map(|file| file.map(Arc::new)),
            |inline| (&mut inline.diff_file_rev, &mut inline.diff_file),
        ),
        InternalMsg::InlineSubmoduleDiffFileImageLoaded {
            repo_id,
            inline_rev,
            target,
            result,
        } => diff_selection::inline_submodule_load_finished(
            inputs,
            state,
            repo_id,
            inline_rev,
            target,
            result.map(|image| image.map(Arc::new)),
            |inline| (&mut inline.diff_file_rev, &mut inline.diff_file_image),
        ),
        InternalMsg::DiffFileImageLoaded {
            repo_id,
            target,
            result,
        } => diff_selection::diff_file_image_loaded(inputs, state, repo_id, target, result),
        InternalMsg::RepoActionFinished {
            repo_id,
            action,
            result,
        } => external_and_history::repo_action_finished(inputs, state, repo_id, action, result),
        InternalMsg::RepoPathsActionFinished {
            repo_id,
            action,
            paths,
            result,
        } => external_and_history::repo_paths_action_finished(
            inputs, state, repo_id, action, paths, result,
        ),
        InternalMsg::BranchAlreadyExists { action, prompt } => {
            external_and_history::branch_already_exists(inputs, state, action, prompt)
        }
        InternalMsg::BranchNotFullyMerged { repo_id, name } => {
            external_and_history::branch_not_fully_merged(inputs, state, repo_id, name)
        }
        InternalMsg::RepoActionFinishedInWorktree {
            repo_id,
            action,
            worktree_path,
            result,
        } => external_and_history::repo_action_finished_in_worktree(
            inputs,
            id_alloc,
            state,
            repo_id,
            action,
            worktree_path,
            result,
        ),
        InternalMsg::CommitFinished {
            repo_id,
            amend,
            result,
        } => auth::commit_finished(inputs, repos, state, repo_id, result, amend),
        InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context,
            auth,
            result,
        } => auth::safe_push_after_commit_finished(
            inputs, repos, state, repo_id, context, auth, result,
        ),
        InternalMsg::RepoCommandFinished {
            repo_id,
            command,
            result,
        } => auth::repo_command_finished(inputs, repos, id_alloc, state, repo_id, command, result),
    }
}
