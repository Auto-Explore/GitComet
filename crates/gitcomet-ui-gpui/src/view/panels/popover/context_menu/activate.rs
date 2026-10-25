//! Running the action a context menu entry chose.

use super::{
    ConflictResolverViewMode, ContextMenuAction, ContextMenuItem, ContextMenuModel, DiffArea,
    InteractiveRebaseAction, Msg, PopoverHost, PopoverKind, PushRequest, Window, add_repo_menu,
    app_menu, branch_group, components, context_menu_entry_action_at, push_request, repo_picker,
    resolve_checkout_file_path, session,
};
use crate::view::WorktreePopoverKind;
use crate::view::gitcomet_view_requests::{RootRequest, RootRequests as _};

impl PopoverHost {
    pub(in crate::view) fn context_menu_activate_action(
        &mut self,
        action: ContextMenuAction,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut close_after_action = true;
        let mut restore_diff_panel_focus_after_action = false;
        let mut restore_invoker_focus_after_action = true;
        match action {
            ContextMenuAction::Explorer {
                repo_id,
                path,
                action,
            } => {
                if self.state.active_repo == Some(repo_id) {
                    let editing = self.sidebar_pane.update(cx, |pane, cx| {
                        pane.explorer_action(action, Some(path), window, cx);
                        pane.explorer_inline_edit_owns_focus(window, cx)
                    });
                    restore_invoker_focus_after_action = !editing;
                }
            }
            ContextMenuAction::Hosted(action) => action.invoke(cx),
            ContextMenuAction::ToggleHistoryRefGroup { target } => {
                self.expanded_history_ref = if self.expanded_history_ref.as_ref() == Some(&target) {
                    None
                } else {
                    Some(target.clone())
                };
                if let Some(kind) = self.popover.as_ref()
                    && let Some(model) = self.context_menu_model(kind, cx)
                {
                    self.context_menu_selected_ix = model.items.iter().position(|item| matches!(item,
                        ContextMenuItem::Entry { action, .. } if matches!(action.as_ref(), ContextMenuAction::ToggleHistoryRefGroup { target: candidate } if candidate == &target)));
                }
                cx.notify();
                return;
            }
            ContextMenuAction::AppMenu(action) => {
                app_menu::activate(self, action, window, cx);
                return;
            }
            ContextMenuAction::RunExtensionCommand { id, repo_id } => {
                self.close_popover_and_restore_focus(window, cx);
                let _ = self.root_view.request(
                    RootRequest::RunExtensionCommand {
                        id: id.to_string(),
                        repo: Some(repo_id),
                    },
                    None,
                    cx,
                );
                return;
            }
            ContextMenuAction::AddRepoMenu(action) => {
                add_repo_menu::activate(self, action, window, cx);
                return;
            }
            ContextMenuAction::ShowFileChangesAtCommit {
                repo_id,
                commit_id,
                path,
            } => {
                self.store.dispatch(Msg::ShowFileChangesAtCommit {
                    repo_id,
                    commit_id,
                    path,
                });
            }
            ContextMenuAction::OpenFileAtCommit {
                repo_id,
                commit_id,
                path,
            } => {
                self.store.dispatch(Msg::OpenFileAtCommit {
                    repo_id,
                    commit_id,
                    path,
                });
            }
            ContextMenuAction::OpenFileAtCommitParent {
                repo_id,
                commit_id,
                path,
            } => {
                self.store.dispatch(Msg::OpenFileAtCommitParent {
                    repo_id,
                    commit_id,
                    path,
                });
            }
            ContextMenuAction::SelectDiff { repo_id, target } => {
                self.store.dispatch(Msg::SelectDiff { repo_id, target });
            }
            ContextMenuAction::OpenFileContent {
                repo_id,
                source,
                path,
            } => {
                let _ = self
                    .root_view
                    .request(RootRequest::ShowRepositoryCanvas, None, cx);
                self.store.dispatch(Msg::OpenFileContent {
                    repo_id,
                    source,
                    path,
                });
            }
            ContextMenuAction::EditFile { repo_id, path } => {
                let _ = self
                    .root_view
                    .request(RootRequest::ShowRepositoryCanvas, None, cx);
                self.store.dispatch(Msg::OpenFileEditor { repo_id, path });
            }
            ContextMenuAction::DiscardFileEdits { repo_id, path } => {
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.discard_file_edits_for(repo_id, &path, cx);
                });
            }
            ContextMenuAction::BrowseRepositoryAtCommit { repo_id, commit_id } => {
                self.store
                    .dispatch(Msg::BrowseRepositoryAtCommit { repo_id, commit_id });
            }
            ContextMenuAction::RevealHistoryCommit { repo_id, commit_id } => {
                self.main_pane.update(cx, |main, cx| {
                    main.reveal_history_commit(
                        repo_id,
                        commit_id,
                        Some(gitcomet_core::domain::LogScope::AllBranches),
                        cx,
                    );
                });
            }
            ContextMenuAction::ResetBrowseToLive { repo_id } => {
                self.store.dispatch(Msg::ResetBrowseToLive { repo_id });
            }
            ContextMenuAction::ToggleFileBrowserDir { repo_id, path } => {
                self.store
                    .dispatch(Msg::ToggleFileBrowserDir { repo_id, path });
            }
            ContextMenuAction::SetFileListFolderCollapsed {
                repo_id,
                list,
                key,
                chain,
                collapsed,
                recursive,
            } => {
                self.details_pane.update(cx, |pane, cx| {
                    pane.set_file_list_folder_collapsed(
                        repo_id, list, key, &chain, collapsed, recursive, cx,
                    );
                });
            }
            // The folder row's hover Stage/Unstage, from its menu.
            ContextMenuAction::StageStatusFolder {
                repo_id,
                section,
                key,
            } => {
                let paths = self
                    .details_pane
                    .read(cx)
                    .status_folder_subtree_paths(repo_id, section, &key);
                let area = section.diff_area();
                if area == DiffArea::Unstaged
                    && let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
                        &self.state,
                        repo_id,
                        paths.clone(),
                        // No selection was consumed, so cancelling must leave it.
                        false,
                    )
                {
                    let anchor = self.popover_anchor_point();
                    self.open_popover_at(confirm, anchor, window, cx);
                    return;
                }
                if !paths.is_empty() {
                    crate::view::status_actions::stage_or_unstage_paths(
                        &self.store,
                        repo_id,
                        area,
                        paths,
                    );
                }
            }
            // The branch tree's collapse state is view-owned rather than a
            // store message, so these four go through the sidebar pane.
            ContextMenuAction::ToggleSidebarCollapseKey { collapse_key } => {
                self.sidebar_pane.update(cx, |pane, cx| {
                    pane.toggle_active_repo_collapse_key(collapse_key, cx);
                });
            }
            ContextMenuAction::SetSidebarCollapseKey {
                collapse_key,
                collapsed,
            } => {
                self.sidebar_pane.update(cx, |pane, cx| {
                    pane.set_active_repo_collapse_key(collapse_key, collapsed, cx);
                });
            }
            ContextMenuAction::SetBranchGroupCollapsedRecursive {
                section,
                remote,
                path,
                collapsed,
            } => {
                self.sidebar_pane.update(cx, |pane, cx| {
                    pane.set_branch_group_collapsed_recursive(section, remote, path, collapsed, cx);
                });
            }
            ContextMenuAction::UnpinAllBranches { repo_id, section } => {
                self.sidebar_pane.update(cx, |pane, cx| {
                    pane.unpin_all_branches(repo_id, section, cx);
                });
            }
            ContextMenuAction::ConfirmDeleteBranchGroup {
                repo_id,
                section,
                remote,
                path,
                group_label,
            } => {
                let names = branch_group::deletable_branches(
                    self,
                    repo_id,
                    section,
                    remote.as_deref(),
                    &path,
                );
                // The entry is disabled at zero, so this only fires if the group
                // emptied between the last repaint and the click.
                if names.is_empty() {
                    self.close_popover(cx);
                    return;
                }
                let anchor = self.popover_anchor_point();
                self.open_popover_at(
                    PopoverKind::DeleteBranchesConfirm {
                        repo_id,
                        section,
                        remote,
                        group_label,
                        names,
                    },
                    anchor,
                    window,
                    cx,
                );
                return;
            }
            ContextMenuAction::SetFileBrowserDirExpandedRecursive {
                repo_id,
                path,
                expanded,
            } => {
                self.store
                    .dispatch(Msg::SetFileBrowserDirExpandedRecursive {
                        repo_id,
                        path,
                        expanded,
                    });
            }
            ContextMenuAction::SelectConflictDiff { repo_id, path } => {
                self.store
                    .dispatch(Msg::SelectConflictDiff { repo_id, path });
            }
            ContextMenuAction::OpenFile { repo_id, path } => {
                let full_path = match self.resolve_workdir_path(repo_id, &path) {
                    Ok(path) => path,
                    Err(err) => {
                        self.push_toast(components::ToastKind::Error, err, cx);
                        self.close_popover(cx);
                        return;
                    }
                };

                if !full_path.exists() {
                    self.push_toast(
                        components::ToastKind::Error,
                        format!("Path not found: {}", full_path.display()),
                        cx,
                    );
                } else {
                    self.open_path_default(full_path, cx);
                }
            }
            ContextMenuAction::OpenWorktreeDiff {
                repo_id,
                worktree_path,
                target,
            } => {
                self.details_pane.update(cx, |pane, cx| {
                    pane.open_worktree_file_diff(repo_id, &worktree_path, &target, window, cx);
                });
            }
            ContextMenuAction::OpenWorktreeFile {
                worktree_path,
                path,
            } => match resolve_checkout_file_path(&worktree_path, &path) {
                Ok(path) if path.exists() => self.open_path_default(path, cx),
                Ok(path) => self.push_toast(
                    components::ToastKind::Error,
                    format!("Path not found: {}", path.display()),
                    cx,
                ),
                Err(error) => self.push_toast(components::ToastKind::Error, error, cx),
            },
            ContextMenuAction::OpenWorktreeFileLocation {
                worktree_path,
                path,
            } => match resolve_checkout_file_path(&worktree_path, &path) {
                Ok(path) => self.reveal_path_in_file_manager(path, Some(worktree_path), cx),
                Err(error) => self.push_toast(components::ToastKind::Error, error, cx),
            },
            ContextMenuAction::OpenFileLocation { repo_id, path } => {
                let full_path = match self.resolve_workdir_path(repo_id, &path) {
                    Ok(path) => path,
                    Err(err) => {
                        self.push_toast(components::ToastKind::Error, err, cx);
                        self.close_popover(cx);
                        return;
                    }
                };

                let fallback = self.workdir_for_repo(repo_id);
                self.reveal_path_in_file_manager(full_path, fallback, cx);
            }
            ContextMenuAction::OpenRepositoryLocation { path } => {
                self.reveal_path_in_file_manager(path, None, cx);
            }
            ContextMenuAction::OpenInCodeEditor { repo_id, path } => {
                let full_path = match repo_id {
                    Some(repo_id) => match self.resolve_workdir_path(repo_id, &path) {
                        Ok(path) => path,
                        Err(err) => {
                            self.push_toast(components::ToastKind::Error, err, cx);
                            self.close_popover(cx);
                            return;
                        }
                    },
                    None => path,
                };

                // The root view owns this sequence (existence check, resolve,
                // background launch, toast); routing through it keeps one copy.
                let _ =
                    self.root_view
                        .request(RootRequest::OpenInExternalEditor(full_path), None, cx);
            }
            ContextMenuAction::OpenRepo { path } => {
                let _ = self
                    .root_view
                    .request(RootRequest::OpenRepoPath(path), None, cx);
            }
            ContextMenuAction::ActivateRepo { repo_id } => {
                if !self.repo_is_open(repo_id) {
                    self.warn_repository_gone(cx);
                    return;
                }
                let _ = self
                    .root_view
                    .request(RootRequest::ShowRepositoryCanvas, None, cx);
                self.store.dispatch(Msg::SetActiveRepo { repo_id });
            }
            ContextMenuAction::CloseRepo { repo_id } if !self.repo_is_open(repo_id) => {
                // The row this came from went stale — a concurrent close from a
                // repo tab, say. Dispatching would be a no-op the user cannot
                // see, and the menu is already down by now.
                self.warn_repository_gone(cx);
                return;
            }
            ContextMenuAction::CloseRepo { repo_id } => {
                // The reducer records the close as a recent; this keeps the
                // repository picker's own snapshot of that list in step, cap
                // included, for the frames before it next reads the session.
                if let Some(workdir) = self.workdir_for_repo(repo_id) {
                    session::promote_recent_repo(&mut self.cached_recent_repos, &workdir);
                }
                // Deferred: the guards may open a prompt in this host.
                let root_view = self.root_view.clone();
                cx.defer(move |cx| {
                    let _ = root_view.request(
                        RootRequest::CloseRepos {
                            repo_ids: vec![repo_id],
                            activate_after: None,
                        },
                        None,
                        cx,
                    );
                });
            }
            ContextMenuAction::MoveRepoToWorkspace {
                repo_id,
                path,
                target_workspace,
            } => {
                // This action runs inside a `PopoverHost` update. The move
                // workflow checks whether that same host is already showing an
                // unsaved-edits dialog, so entering it synchronously would read
                // an entity while GPUI still holds its update guard. Let this
                // menu close first, then start the guarded move.
                let root_view = self.root_view.clone();
                cx.defer(move |cx| {
                    let _ = root_view.request(
                        RootRequest::MoveRepoToWorkspace {
                            repo_id,
                            path,
                            target: target_workspace,
                        },
                        None,
                        cx,
                    );
                });
            }
            ContextMenuAction::ActivateWorkspace { workspace_id } => {
                repo_picker::activate_workspace(self, workspace_id, cx);
            }
            ContextMenuAction::DeleteWorkspace { workspace_id } => {
                // Deferred: it may close or reset the window this host is in.
                cx.defer(move |cx| crate::app::delete_workspace(cx, workspace_id));
            }
            ContextMenuAction::OpenWorkspaceSettings { workspace_id } => {
                cx.defer(move |cx| {
                    crate::view::open_settings_window_to_workspace(cx, workspace_id);
                });
            }
            ContextMenuAction::PinRepository { path } => {
                let _ = session::persist_pinned_repo(&path);
                if !self.cached_pinned_repos.contains(&path) {
                    self.cached_pinned_repos.push(path);
                }
                // Pinning is bookkeeping, not navigation: the menu that offered
                // it goes, but the list it was over stays.
                close_after_action = false;
            }
            ContextMenuAction::UnpinRepository { path } => {
                let _ = session::remove_pinned_repo(&path);
                self.cached_pinned_repos.retain(|pin| pin != &path);
                close_after_action = false;
            }
            ContextMenuAction::ForgetRecentRepository { path } => {
                // Open and pinned repositories have no entry for this, and the
                // guard keeps it that way: a pin is what keeps a closed
                // repository listed, so forgetting one would strand it.
                if !self.cached_pinned_repos.contains(&path) {
                    let _ = session::remove_recent_repo(&path);
                    self.cached_recent_repos.retain(|recent| recent != &path);
                }
                close_after_action = false;
            }
            ContextMenuAction::CloseRepos {
                repo_ids,
                activate_after,
            } => {
                let root_view = self.root_view.clone();
                cx.defer(move |cx| {
                    let _ = root_view.request(
                        RootRequest::CloseRepos {
                            repo_ids,
                            activate_after,
                        },
                        None,
                        cx,
                    );
                });
            }
            ContextMenuAction::OpenSubmoduleDiffInTab { path, target } => {
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.open_submodule_inner_diff(path, target, cx);
                });
            }
            ContextMenuAction::ExportPatch { repo_id, commit_id } => {
                cx.stop_propagation();
                let view = cx.weak_entity();
                let sha = commit_id.as_ref();
                let short = sha.get(0..8).unwrap_or(sha).to_string();
                let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some("Export patch to folder".into()),
                    directory: None,
                });
                window
                    .spawn(cx, async move |cx| {
                        let result = rx.await;
                        let paths = match result {
                            Ok(Ok(Some(paths))) => paths,
                            Ok(Ok(None)) => return,
                            Ok(Err(_)) | Err(_) => return,
                        };
                        let Some(folder) = paths.into_iter().next() else {
                            return;
                        };
                        let dest = folder.join(format!("commit-{short}.patch"));
                        let _ = view.update(cx, |this, cx| {
                            this.store.dispatch(Msg::ExportPatch {
                                repo_id,
                                commit_id: commit_id.clone(),
                                dest,
                            });
                            cx.notify();
                        });
                    })
                    .detach();
                self.close_popover(cx);
                return;
            }
            ContextMenuAction::CheckoutCommit { repo_id, commit_id } => {
                self.store
                    .dispatch(Msg::CheckoutCommit { repo_id, commit_id });
            }
            ContextMenuAction::MarkForComparison {
                repo_id,
                commit_id,
                label,
            } => {
                self.store.dispatch(Msg::MarkForComparison {
                    repo_id,
                    commit_id,
                    label,
                });
            }
            ContextMenuAction::CompareWithMarked {
                repo_id,
                commit_id,
                label,
            } => {
                self.store.dispatch(Msg::CompareWithMarked {
                    repo_id,
                    commit_id,
                    label,
                });
            }
            ContextMenuAction::CompareWithMergeBase {
                repo_id,
                commit_id,
                label,
            } => {
                if let Some(mark) = self
                    .state
                    .repos
                    .iter()
                    .find(|repo| repo.id == repo_id)
                    .and_then(|repo| repo.navigation.comparison_mark.as_ref())
                {
                    self.store.dispatch(Msg::CompareWithOptions {
                        repo_id,
                        from: mark.commit_id.clone(),
                        to: Some(commit_id),
                        options: gitcomet_core::services::ComparisonOptions::merge_base(),
                        from_label: mark.label.clone(),
                        to_label: label,
                    });
                }
            }
            ContextMenuAction::CompareWithWorkingTree {
                repo_id,
                commit_id,
                label,
            } => {
                self.store.dispatch(Msg::CompareWithWorkingTree {
                    repo_id,
                    from: commit_id,
                    from_label: label,
                });
            }
            ContextMenuAction::ClearComparisonMark { repo_id } => {
                self.store.dispatch(Msg::ClearComparisonMark { repo_id });
            }
            ContextMenuAction::CherryPickCommit { repo_id, commit_id } => {
                let anchor = self.popover_anchor_point();
                self.open_popover_at(
                    PopoverKind::CherryPickCommitConfirm { repo_id, commit_id },
                    anchor,
                    window,
                    cx,
                );
                return;
            }
            ContextMenuAction::ApplyFileChange { repo_id, target } => {
                let anchor = self.popover_anchor_point();
                self.open_popover_at(
                    PopoverKind::ApplyFileChangeConfirm { repo_id, target },
                    anchor,
                    window,
                    cx,
                );
                return;
            }
            ContextMenuAction::RevertCommit { repo_id, commit_id } => {
                let anchor = self.popover_anchor_point();
                self.open_popover_at(
                    PopoverKind::RevertCommitConfirm { repo_id, commit_id },
                    anchor,
                    window,
                    cx,
                );
                return;
            }
            ContextMenuAction::SquashSelectedCommits { repo_id } => {
                // PrepareSquash and the eventual SquashCommits are both
                // discarded silently when the git runtime is unavailable, which
                // would leave the prompt stuck on "Building combined message…".
                // Don't open it in that state.
                if !self.state.git_runtime.is_available() {
                    self.close_popover(cx);
                    return;
                }
                // Kick off the combined-message preview, then swap the menu
                // for the confirmation prompt.
                self.store.dispatch(Msg::PrepareSquash { repo_id });
                let anchor = self.popover_anchor_point();
                self.open_popover_at(PopoverKind::SquashPrompt { repo_id }, anchor, window, cx);
                return;
            }
            ContextMenuAction::CheckoutBranch { repo_id, name } => {
                self.store.dispatch(Msg::CheckoutBranch { repo_id, name });
            }
            ContextMenuAction::DeleteBranch { repo_id, name } => {
                self.root_view
                    .request(RootRequest::SetForceDeleteBranchCentered(false), None, cx);
                self.store.dispatch(Msg::DeleteBranch { repo_id, name });
            }
            ContextMenuAction::ToggleBranchGroupPin { repo_id, group_key } => {
                self.sidebar_pane.update(cx, |pane, cx| {
                    pane.toggle_sidebar_pin(repo_id, group_key, cx);
                });
            }
            ContextMenuAction::ToggleBranchPin {
                repo_id,
                section,
                name,
            } => {
                self.sidebar_pane.update(cx, |pane, cx| {
                    pane.toggle_pinned_branch(repo_id, section, &name, cx);
                });
            }
            ContextMenuAction::SetHistoryScope { repo_id, scope } => {
                self.store.dispatch(Msg::SetHistoryScope { repo_id, scope });
            }
            ContextMenuAction::SetCommitFileSort { list, sort } => {
                self.details_pane.update(cx, |pane, cx| {
                    pane.set_file_list_sort(list, sort, cx);
                });
            }
            ContextMenuAction::SetFileListLayout {
                repo_id,
                list,
                layout,
            } => {
                self.details_pane.update(cx, |pane, cx| {
                    pane.set_list_file_layout(repo_id, list, layout, cx);
                });
            }
            ContextMenuAction::SetImageSampling { mode } => {
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.set_image_preferences(Some(mode), None, cx)
                });
            }
            ContextMenuAction::SetImageSmoothFilter { filter } => {
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.set_image_preferences(None, Some(filter), cx)
                });
            }
            ContextMenuAction::ImageZoom { zoom } => {
                crate::view::image_preview::apply_menu_zoom(zoom, window, cx);
            }
            ContextMenuAction::SetTextEncoding { encoding } => {
                let diff_renderer = self.diff_renderer.clone();
                cx.defer(move |cx| {
                    diff_renderer.update(cx, |pane, cx| {
                        pane.set_text_encoding_override(encoding, cx);
                    });
                });
            }
            ContextMenuAction::SaveWithEncoding { format } => {
                let diff_renderer = self.diff_renderer.clone();
                cx.defer(move |cx| {
                    diff_renderer.update(cx, |pane, cx| {
                        pane.set_save_text_format(format, cx);
                    });
                });
            }
            ContextMenuAction::ConvertLineEndings { ending } => {
                let diff_renderer = self.diff_renderer.clone();
                cx.defer(move |cx| {
                    diff_renderer.update(cx, |pane, cx| {
                        pane.convert_file_editor_line_endings(ending, cx);
                    });
                });
            }
            ContextMenuAction::SetTabSize { size } => {
                self.diff_renderer
                    .update(cx, |pane, _| pane.set_tab_size_override(size));
            }
            ContextMenuAction::AddGitattributesRule { rule } => {
                self.diff_renderer
                    .update(cx, |pane, _| pane.add_text_encoding_rule(rule));
            }
            ContextMenuAction::SetDiffContentMode { mode } => {
                self.diff_content_mode = mode;
                let diff_renderer = self.diff_renderer.clone();
                cx.defer(move |cx| {
                    diff_renderer.update(cx, |pane, cx| {
                        pane.set_diff_content_mode_and_persist(mode, cx);
                    });
                });
            }
            ContextMenuAction::SetDiffWhitespaceMode { mode } => {
                close_after_action = false;
                restore_diff_panel_focus_after_action = true;
                self.diff_whitespace_mode = mode;
                let diff_renderer = self.diff_renderer.clone();
                cx.defer(move |cx| {
                    diff_renderer.update(cx, |pane, cx| {
                        pane.set_diff_whitespace_mode_and_persist(mode, cx);
                    });
                });
            }
            ContextMenuAction::SetDiffEngineOverride { engine } => {
                restore_diff_panel_focus_after_action = true;
                let renderer = self.diff_renderer.clone();
                cx.defer(move |cx| {
                    renderer.update(cx, |pane, cx| pane.set_diff_engine_override(engine, cx));
                });
            }
            ContextMenuAction::SetDiffRevealWhitespaceChars { enabled } => {
                close_after_action = false;
                restore_diff_panel_focus_after_action = true;
                self.diff_reveal_whitespace_chars = enabled;
                let diff_renderer = self.diff_renderer.clone();
                cx.defer(move |cx| {
                    diff_renderer.update(cx, |pane, cx| {
                        pane.set_diff_reveal_whitespace_chars_and_persist(enabled, cx);
                    });
                });
            }
            ContextMenuAction::SetDiffWordWrap { enabled } => {
                close_after_action = false;
                restore_diff_panel_focus_after_action = true;
                self.diff_word_wrap = enabled;
                let diff_renderer = self.diff_renderer.clone();
                cx.defer(move |cx| {
                    diff_renderer.update(cx, |pane, cx| {
                        pane.set_diff_word_wrap_and_persist(enabled, cx);
                    });
                });
            }
            ContextMenuAction::SetDiffShowLineNumbers { enabled } => {
                close_after_action = false;
                restore_diff_panel_focus_after_action = true;
                self.diff_show_line_numbers = enabled;
                let diff_renderer = self.diff_renderer.clone();
                cx.defer(move |cx| {
                    diff_renderer.update(cx, |pane, cx| {
                        pane.set_diff_show_line_numbers_and_persist(enabled, cx);
                    });
                });
            }
            ContextMenuAction::SetExplorerVisibility {
                repo_id,
                hidden,
                ignored,
            } => {
                close_after_action = false;
                self.store.dispatch(Msg::SetExplorerVisibility {
                    repo_id,
                    hidden,
                    ignored,
                });
            }
            ContextMenuAction::SetChangeTrackingView { view } => {
                self.change_tracking_view = view;
                let root_view = self.root_view.clone();
                cx.defer(move |cx| {
                    let _ = root_view.request(RootRequest::SetChangeTrackingView(view), None, cx);
                });
            }
            ContextMenuAction::SetUiScale { percent } => {
                let window_id = window.window_handle().window_id();
                cx.defer(move |cx| {
                    crate::app::set_window_ui_scale_percent(cx, window_id, percent);
                });
            }
            ContextMenuAction::SetCommitAmendEnabled { enabled } => {
                close_after_action = false;
                self.commit_amend_enabled = enabled;
                let root_view = self.root_view.clone();
                cx.defer(move |cx| {
                    let _ = root_view.request(RootRequest::SetCommitAmend(enabled), None, cx);
                });
            }
            ContextMenuAction::SetCommitPushAfterEnabled { enabled } => {
                close_after_action = false;
                self.commit_push_after_enabled = enabled;
                let root_view = self.root_view.clone();
                cx.defer(move |cx| {
                    let _ = root_view.request(RootRequest::SetCommitPushAfter(enabled), None, cx);
                });
            }
            ContextMenuAction::UseCommitMessage { message } => {
                self.details_pane.update(cx, |pane, cx| {
                    pane.set_commit_message_from_history(message, window, cx);
                });
            }
            ContextMenuAction::StageSelectionOrPath {
                repo_id,
                area,
                path,
            } => {
                // Staging is what marks a conflict resolved, so confirm first if
                // any of these files still has conflict markers in the worktree.
                // Resolved without consuming the selection, which the dialog
                // takes over responsibility for.
                let (paths, used_selection) =
                    self.status_paths_for_action(repo_id, area, &path, cx);
                if let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
                    &self.state,
                    repo_id,
                    paths.clone(),
                    used_selection,
                ) {
                    let anchor = crate::view::conflict_markers::centered_dialog_anchor(window);
                    self.open_popover_at(confirm, anchor, window, cx);
                    return;
                }
                if used_selection {
                    self.clear_status_multi_selection(repo_id, cx);
                }
                crate::view::status_actions::stage_or_unstage_paths(
                    &self.store,
                    repo_id,
                    DiffArea::Unstaged,
                    paths,
                );
            }
            ContextMenuAction::UnstageSelectionOrPath {
                repo_id,
                area,
                path,
            } => {
                let (paths, _) = self.take_status_paths_for_action(repo_id, area, &path, cx);
                crate::view::status_actions::stage_or_unstage_paths(
                    &self.store,
                    repo_id,
                    DiffArea::Staged,
                    paths,
                );
            }
            ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
                repo_id,
                area,
                path,
            } => {
                let anchor = self.popover_anchor_point();
                self.open_popover_at(
                    PopoverKind::DiscardChangesConfirm {
                        repo_id,
                        area,
                        path: Some(path),
                    },
                    anchor,
                    window,
                    cx,
                );
                return;
            }
            ContextMenuAction::AddExplorerToGitignore { repo_id, path } => {
                self.gitignore_explorer = true;
                self.open_popover_at(
                    PopoverKind::AddToGitignorePrompt {
                        repo_id,
                        area: DiffArea::Unstaged,
                        path,
                    },
                    self.popover_anchor_point(),
                    window,
                    cx,
                );
                return;
            }
            ContextMenuAction::AddToGitignoreSelectionOrPath {
                repo_id,
                area,
                path,
            } => {
                self.gitignore_explorer = false;
                let anchor = self.popover_anchor_point();
                // Deliberately does not consume the row selection: the dialog
                // can still be cancelled, and `submit_add_to_gitignore` is what
                // clears it once the action is committed.
                self.open_popover_at(
                    PopoverKind::AddToGitignorePrompt {
                        repo_id,
                        area,
                        path,
                    },
                    anchor,
                    window,
                    cx,
                );
                return;
            }
            ContextMenuAction::CheckoutConflictSideSelectionOrPath {
                repo_id,
                area,
                path,
                side,
            } => {
                let (paths, _) = self.take_status_paths_for_action(repo_id, area, &path, cx);
                self.details_pane.update(cx, |pane, cx| {
                    pane.status_multi_selection.remove(&repo_id);
                    cx.notify();
                });
                for path in paths {
                    self.store.dispatch(Msg::CheckoutConflictSide {
                        repo_id,
                        path,
                        side,
                    });
                }
            }
            ContextMenuAction::LaunchMergetool { repo_id, path } => {
                self.store.dispatch(Msg::LaunchMergetool { repo_id, path });
            }
            ContextMenuAction::RunLargeFileCommand { repo_id, command } => {
                self.store
                    .dispatch(Msg::RunLargeFileCommand { repo_id, command });
            }
            ContextMenuAction::LoadLfsLocks { repo_id } => {
                self.store.dispatch(Msg::LoadLfsLocks { repo_id });
            }
            ContextMenuAction::FetchAll { repo_id } => {
                self.store
                    .dispatch(Msg::Fetch(gitcomet_state::msg::FetchMsg::All { repo_id }));
            }
            ContextMenuAction::PruneMergedBranches { repo_id } => {
                self.store.dispatch(Msg::PruneMergedBranches { repo_id });
            }
            ContextMenuAction::PruneLocalTags { repo_id } => {
                self.store.dispatch(Msg::PruneLocalTags { repo_id });
            }
            ContextMenuAction::UpdateSubmodules { repo_id } => {
                self.store.dispatch(Msg::UpdateSubmodules { repo_id });
            }
            ContextMenuAction::LoadSubmodule { repo_id, path } => {
                self.store.dispatch(Msg::LoadSubmodule { repo_id, path });
            }
            ContextMenuAction::LoadWorktrees { repo_id } => {
                self.store.dispatch(Msg::LoadWorktrees { repo_id });
            }
            ContextMenuAction::Pull { repo_id, mode } => {
                self.store.dispatch(Msg::Pull { repo_id, mode });
            }
            ContextMenuAction::PullBranch {
                repo_id,
                remote,
                branch,
            } => {
                self.store.dispatch(Msg::PullBranch {
                    repo_id,
                    remote,
                    branch,
                });
            }
            ContextMenuAction::MergeRef { repo_id, reference } => {
                self.store.dispatch(Msg::MergeRef { repo_id, reference });
            }
            ContextMenuAction::SquashRef { repo_id, reference } => {
                self.store.dispatch(Msg::SquashRef { repo_id, reference });
            }
            ContextMenuAction::ApplyStash { repo_id, index } => {
                self.store.dispatch(Msg::ApplyStash { repo_id, index });
            }
            ContextMenuAction::PopStash { repo_id, index } => {
                self.store.dispatch(Msg::PopStash { repo_id, index });
            }
            ContextMenuAction::DropStashConfirm {
                repo_id,
                index,
                message,
            } => {
                let anchor = self.popover_anchor_point();
                self.open_popover_at(
                    PopoverKind::StashDropConfirm {
                        repo_id,
                        index,
                        message,
                    },
                    anchor,
                    window,
                    cx,
                );
                return;
            }
            ContextMenuAction::PushWithTags { repo_id, mode } => {
                let anchor = self.popover_anchor_point();
                if self.push_with_tags(repo_id, mode, Some(anchor), window, cx) {
                    return;
                }
            }
            ContextMenuAction::Push { repo_id } => {
                let request = self
                    .state
                    .repos
                    .iter()
                    .find(|repo| repo.id == repo_id)
                    .map(|repo| push_request(repo, &self.state.large_file_settings))
                    .unwrap_or(PushRequest::NotReady);
                match request {
                    PushRequest::Push => self.store.dispatch(Msg::Push { repo_id }),
                    PushRequest::SetUpstream { remote } => {
                        let anchor = self.popover_anchor_point();
                        self.open_popover_at(
                            PopoverKind::PushSetUpstreamPrompt {
                                repo_id,
                                remote,
                                configure_only_for: None,
                            },
                            anchor,
                            window,
                            cx,
                        );
                        return;
                    }
                    PushRequest::NoRemotes => self.push_toast(
                        components::ToastKind::Error,
                        "Cannot push: no remotes configured".to_string(),
                        cx,
                    ),
                    PushRequest::NotReady => {}
                }
            }
            ContextMenuAction::SetUpstreamBranch {
                repo_id,
                branch,
                upstream,
            } => {
                self.store.dispatch(Msg::SetUpstreamBranch {
                    repo_id,
                    branch,
                    upstream,
                });
            }
            ContextMenuAction::UnsetUpstreamBranch { repo_id, branch } => {
                self.store
                    .dispatch(Msg::UnsetUpstreamBranch { repo_id, branch });
            }
            ContextMenuAction::LoadInteractiveRebaseSetup { repo_id, base } => {
                self.store
                    .dispatch(Msg::LoadInteractiveRebaseSetup { repo_id, base });
            }
            ContextMenuAction::OpenInteractiveCherryPickSetup {
                repo_id,
                entries,
                source_colors,
            } => {
                self.store.dispatch(Msg::OpenInteractiveCherryPickSetup {
                    repo_id,
                    entries,
                    source_colors,
                });
            }
            ContextMenuAction::SetInteractiveRebaseAction { ix, action } => {
                let root_view = self.root_view.clone();
                let was_reword = action == InteractiveRebaseAction::Reword;
                let reword_state = if was_reword {
                    self.main_pane.read_with(cx, |pane, _| {
                        pane.active_irebase().and_then(|st| {
                            let action = st.entries.get(ix)?.action;
                            let msg = gitcomet_core::squash::reword_seed_message(&st.entries, ix);
                            Some((action, msg))
                        })
                    })
                } else {
                    None
                };
                self.main_pane.update(cx, |pane, cx| {
                    pane.set_rebase_action(ix, action, cx);
                });
                if let Some((original_action, msg)) = reword_state {
                    let wh = window.window_handle();
                    cx.defer(move |cx| {
                        let _ = wh.update(cx, |_, window, cx| {
                            let _ = root_view.request(
                                RootRequest::OpenPopoverCentered(PopoverKind::RebaseReword {
                                    ix,
                                    original_action,
                                    original_message: msg,
                                }),
                                Some(window),
                                cx,
                            );
                        });
                    });
                }
            }
            ContextMenuAction::SetInteractiveRebaseAutosquashMode { mode } => {
                let applied = self
                    .main_pane
                    .update(cx, |pane, cx| pane.apply_autosquash_mode(mode, cx));
                if !applied {
                    self.push_toast(
                        components::ToastKind::Warning,
                        "No automatic squashable commits found. Auto Squash searches for \
                         commits with identical messages and amend-commits them."
                            .to_string(),
                        cx,
                    );
                }
            }
            ContextMenuAction::OpenFileHistory { repo_id, path } => {
                // The tab's filter takes the focus; the invoker must not take it back.
                restore_invoker_focus_after_action = false;
                let _ = self.root_view.request(
                    RootRequest::OpenFileHistory { repo_id, path },
                    Some(window),
                    cx,
                );
            }
            ContextMenuAction::OpenPopover { kind } => {
                let anchor = self.popover_anchor_point();
                self.open_popover_at(kind, anchor, window, cx);
                return;
            }
            ContextMenuAction::OpenWorktreeAddPrompt { repo_id, prefill } => {
                self.pending_worktree_add_prefill = Some(prefill);
                self.open_popover_centered(
                    PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt),
                    window,
                    cx,
                );
                // Returning keeps `close_after_action` off the dialog just opened.
                return;
            }
            ContextMenuAction::ConflictResolverPick { target } => {
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.conflict_resolver_apply_pick_target(target, cx);
                });
            }
            ContextMenuAction::ConflictResolverUnresolve { conflict_ix } => {
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.conflict_resolver_select_conflict(conflict_ix, cx);
                    pane.conflict_resolver_unresolve_active_conflict(cx);
                });
            }
            ContextMenuAction::ConflictResolverSplitSelection => {
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.conflict_resolver_split_selection(cx);
                });
            }
            ContextMenuAction::ConflictResolverAlignManually => {
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.conflict_resolver_align_manually(cx);
                });
            }
            ContextMenuAction::ConflictResolverClearManualAlignments => {
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.conflict_resolver_clear_manual_alignments(cx);
                });
            }
            ContextMenuAction::ConflictResolverJoinRegions { target } => {
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.conflict_resolver_join_regions(target, cx);
                });
            }
            ContextMenuAction::SetMergetoolAutoAdvance { enabled } => {
                close_after_action = false;
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.set_mergetool_auto_advance_and_persist(enabled, cx);
                });
                cx.notify();
            }
            ContextMenuAction::ToggleMergetoolCollapseUnchanged => {
                close_after_action = false;
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.conflict_resolver_toggle_collapse_context(cx);
                });
                cx.notify();
            }
            ContextMenuAction::SetMergetoolOutputScrollSync { enabled } => {
                close_after_action = false;
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.set_mergetool_output_scroll_sync_and_persist(enabled, cx);
                });
                cx.notify();
            }
            ContextMenuAction::SetMergetoolShowLineNumbers { enabled } => {
                close_after_action = false;
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.set_mergetool_show_line_numbers_and_persist(enabled, cx);
                });
                cx.notify();
            }
            ContextMenuAction::SetMergetoolThreeWayView { enabled } => {
                close_after_action = false;
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.conflict_resolver_set_view_mode(
                        if enabled {
                            ConflictResolverViewMode::ThreeWay
                        } else {
                            ConflictResolverViewMode::TwoWayDiff
                        },
                        cx,
                    );
                });
                cx.notify();
            }
            ContextMenuAction::ConflictResolverOutputCut { text } => {
                crate::clipboard::write_text(cx, text, crate::clipboard::CopySource::ContextMenu);
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.conflict_resolver_output_delete_selection(cx);
                });
            }
            ContextMenuAction::ConflictResolverOutputPaste => {
                if let Some(text) = crate::clipboard::read_text(cx) {
                    self.diff_renderer.update(cx, |pane, cx| {
                        pane.conflict_resolver_output_paste_text(&text, cx);
                    });
                }
            }
            ContextMenuAction::CopyText { text } => {
                window.activate();
                crate::clipboard::write_text(cx, text, crate::clipboard::CopySource::ContextMenu);
            }
            ContextMenuAction::CopyLinkAddress { url } => {
                window.activate();
                crate::clipboard::write_text(cx, url, crate::clipboard::CopySource::ContextMenu);
                self.push_toast(
                    components::ToastKind::Success,
                    "Link copied to clipboard".to_string(),
                    cx,
                );
            }
            ContextMenuAction::LoadRemoteMarkdownImage { url } => {
                if let Some(view) = self.markdown_image_approval.clone() {
                    view.approve_image(url, cx);
                } else {
                    self.diff_renderer.update(cx, |pane, cx| {
                        pane.approve_remote_markdown_image(url, cx);
                    });
                }
            }
            ContextMenuAction::OpenWebUrl { url } => {
                crate::view::platform_open::spawn_launch(
                    cx,
                    move || crate::view::platform_open::open_url_blocking(&url),
                    |this, result, cx| {
                        if let Err(err) = result {
                            this.push_toast(
                                components::ToastKind::Error,
                                format!("Failed to open link: {err}"),
                                cx,
                            );
                        }
                    },
                );
            }
            ContextMenuAction::CopyDiffSelection { text } => {
                window.activate();
                crate::clipboard::write_text(
                    cx,
                    text,
                    crate::clipboard::CopySource::DiffContextMenu,
                );
            }
            ContextMenuAction::CopyDiffText { visible_ix, region } => {
                window.activate();
                self.diff_renderer.update(cx, |pane, cx| {
                    pane.copy_diff_text_for_context_menu_to_clipboard(visible_ix, region, cx);
                });
            }
            ContextMenuAction::TerminalCommand {
                repo_id,
                session_seq,
                command,
            } => {
                window.activate();
                let dispatched = self
                    .root_view
                    .update(cx, |root, cx| {
                        root.dispatch_terminal_command(repo_id, session_seq, command, window, cx)
                    })
                    .unwrap_or(false);
                if !dispatched {
                    self.close_popover(cx);
                    return;
                }
            }
            ContextMenuAction::TerminalOpenExternal {
                repo_id,
                session_seq,
            } => {
                let dispatched = self
                    .root_view
                    .update(cx, |root, cx| {
                        if root
                            .terminal_viewport_for_session(repo_id, session_seq)
                            .is_none()
                        {
                            return false;
                        }
                        root.open_external_terminal_from_menu(repo_id, window, cx);
                        true
                    })
                    .unwrap_or(false);
                if !dispatched {
                    self.close_popover(cx);
                    return;
                }
            }
            ContextMenuAction::ApplyIndexPatch {
                repo_id,
                patch,
                reverse,
            } => {
                if patch.is_blank() {
                    self.push_toast(
                        components::ToastKind::Error,
                        "Patch is empty".to_string(),
                        cx,
                    );
                } else if reverse {
                    self.store.dispatch(Msg::UnstageHunk { repo_id, patch });
                } else {
                    self.store.dispatch(Msg::StageHunk { repo_id, patch });
                }
            }
            ContextMenuAction::ApplyWorktreePatch {
                repo_id,
                patch,
                reverse,
            } => {
                if patch.is_blank() {
                    self.push_toast(
                        components::ToastKind::Error,
                        "Patch is empty".to_string(),
                        cx,
                    );
                } else {
                    self.store.dispatch(Msg::ApplyWorktreePatch {
                        repo_id,
                        patch,
                        reverse,
                    });
                }
            }
            ContextMenuAction::StageHunk { repo_id, src_ix } => {
                if let Some(patch) = self.build_unified_patch_for_hunk_src_ix(repo_id, src_ix, cx) {
                    self.store.dispatch(Msg::StageHunk { repo_id, patch });
                } else {
                    self.push_toast(
                        components::ToastKind::Error,
                        "Couldn't build patch for this hunk".to_string(),
                        cx,
                    );
                }
            }
            ContextMenuAction::UnstageHunk { repo_id, src_ix } => {
                if let Some(patch) = self.build_unified_patch_for_hunk_src_ix(repo_id, src_ix, cx) {
                    self.store.dispatch(Msg::UnstageHunk { repo_id, patch });
                } else {
                    self.push_toast(
                        components::ToastKind::Error,
                        "Couldn't build patch for this hunk".to_string(),
                        cx,
                    );
                }
            }
            ContextMenuAction::DeleteTag { repo_id, name } => {
                self.store.dispatch(Msg::DeleteTag { repo_id, name });
            }
            ContextMenuAction::PushTag {
                repo_id,
                remote,
                name,
            } => {
                self.store.dispatch(Msg::PushTag {
                    repo_id,
                    remote,
                    name,
                });
            }
            ContextMenuAction::DeleteRemoteTag {
                repo_id,
                remote,
                name,
            } => {
                self.store.dispatch(Msg::DeleteRemoteTag {
                    repo_id,
                    remote,
                    name,
                });
            }
        }
        // A menu floating over a picker is not the popover: it closed itself on
        // the way in, and whether the picker underneath survives is the picker's
        // call, not the action's.
        if close_after_action && !self.suppress_popover_close_after_action {
            if restore_invoker_focus_after_action {
                self.close_popover_and_restore_focus(window, cx);
            } else {
                // The action put focus in an inline editor; restoring the invoker would take it.
                self.close_popover(cx);
            }
        } else {
            if restore_diff_panel_focus_after_action {
                let focus = self.diff_renderer.read(cx).diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn context_menu_activate_model_entry(
        &mut self,
        model: &ContextMenuModel,
        ix: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(action) = context_menu_entry_action_at(model, ix) {
            self.context_menu_activate_action(action, window, cx);
        }
    }
}
