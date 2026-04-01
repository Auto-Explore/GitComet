use super::*;

mod branch;
mod branch_section;
mod change_tracking_settings;
mod commit;
mod commit_file;
mod conflict_resolver_chunk;
mod conflict_resolver_input_row;
mod conflict_resolver_output;
mod diff_editor;
mod diff_hunk;
mod history_branch_filter;
mod history_column_settings;
mod pull;
mod push;
mod remote;
mod stash;
mod status_file;
mod submodule;
mod submodule_section;
mod tag;
mod worktree;
mod worktree_section;

fn normalize_platform_path(path: std::path::PathBuf) -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        let mut normalized = std::path::PathBuf::new();
        for component in path.components() {
            normalized.push(component.as_os_str());
        }
        normalized
    }

    #[cfg(not(target_os = "windows"))]
    {
        path
    }
}

pub(super) fn path_text_for_copy(path: &std::path::Path) -> String {
    normalize_platform_path(path.to_path_buf())
        .display()
        .to_string()
}

fn active_branch_tracking_upstream_name(host: &PopoverHost) -> Option<String> {
    let repo_id = host.active_repo_id()?;
    let repo = host.state.repos.iter().find(|repo| repo.id == repo_id)?;
    let Loadable::Ready(head) = &repo.head_branch else {
        return None;
    };
    let Loadable::Ready(branches) = &repo.branches else {
        return None;
    };

    branches
        .iter()
        .find(|branch| branch.name == *head)
        .and_then(|branch| branch.upstream.as_ref())
        .map(|upstream| format!("{}/{}", upstream.remote, upstream.branch))
}

fn action_menu_title(base: &'static str, tracking_branch_name: Option<&str>) -> SharedString {
    match tracking_branch_name {
        Some(name) => format!("{base} {name}").into(),
        None => base.into(),
    }
}

fn context_menu_entry_debug_selector(label: &str) -> String {
    let mut slug = String::with_capacity(label.len());
    let mut previous_was_separator = true;

    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            slug.push('_');
            previous_was_separator = true;
        }
    }

    while slug.ends_with('_') {
        slug.pop();
    }

    if slug.is_empty() {
        "context_menu_entry".to_string()
    } else {
        format!("context_menu_{slug}")
    }
}

fn context_menu_entry_action_at(model: &ContextMenuModel, ix: usize) -> Option<ContextMenuAction> {
    match model.items.get(ix) {
        Some(ContextMenuItem::Entry { action, .. }) => Some((**action).clone()),
        _ => None,
    }
}

pub(in super::super) fn context_menu_activate_entry_ix(
    model: &ContextMenuModel,
    selected_ix: Option<usize>,
) -> Option<usize> {
    selected_ix
        .filter(|&ix| model.is_selectable(ix))
        .or_else(|| model.first_selectable())
}

pub(in super::super) fn context_menu_shortcut_entry_ix(
    model: &ContextMenuModel,
    key: &str,
) -> Option<usize> {
    if key.chars().count() != 1 {
        return None;
    }

    model.items.iter().enumerate().find_map(|(ix, item)| {
        let ContextMenuItem::Entry {
            shortcut, disabled, ..
        } = item
        else {
            return None;
        };
        if *disabled {
            return None;
        }
        let shortcut = shortcut.as_ref()?;
        shortcut.as_ref().eq_ignore_ascii_case(key).then_some(ix)
    })
}

impl PopoverHost {
    fn workdir_for_repo(&self, repo_id: RepoId) -> Option<std::path::PathBuf> {
        self.state
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .map(|r| r.spec.workdir.clone())
    }

    fn resolve_workdir_path(
        &self,
        repo_id: RepoId,
        path: &std::path::Path,
    ) -> Result<std::path::PathBuf, String> {
        if path.is_absolute()
            || path.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir
                        | std::path::Component::Prefix(_)
                        | std::path::Component::RootDir
                )
            })
        {
            return Err("Refusing to open path outside repository".to_string());
        }

        let workdir = self
            .workdir_for_repo(repo_id)
            .ok_or_else(|| "Repository is not available".to_string())?;
        Ok(normalize_platform_path(workdir.join(path)))
    }

    fn open_path_default(&mut self, path: &std::path::Path) -> Result<(), std::io::Error> {
        super::super::super::platform_open::open_path(path)
    }

    fn open_file_location(&mut self, path: &std::path::Path) -> Result<(), std::io::Error> {
        super::super::super::platform_open::open_file_location(path)
    }

    fn take_status_paths_for_action(
        &mut self,
        repo_id: RepoId,
        area: DiffArea,
        clicked_path: &std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) -> (Vec<std::path::PathBuf>, bool) {
        let selection = self.details_pane.update(cx, |pane, cx| {
            let selection = pane
                .status_multi_selection
                .get(&repo_id)
                .map(|sel| sel.selected_paths_for_area(area))
                .unwrap_or(&[]);

            let use_selection = selection.len() > 1 && selection.iter().any(|p| p == clicked_path);
            if !use_selection {
                return None;
            }

            let sel = pane.status_multi_selection.remove(&repo_id)?;
            cx.notify();
            Some(sel.take_selected_paths_for_area(area))
        });

        match selection {
            Some(paths) if !paths.is_empty() => (paths, true),
            _ => (vec![clicked_path.clone()], false),
        }
    }

    pub(in super::super) fn context_menu_model(
        &self,
        kind: &PopoverKind,
        cx: &gpui::Context<Self>,
    ) -> Option<ContextMenuModel> {
        match kind {
            PopoverKind::PullPicker => Some(pull::model(self)),
            PopoverKind::PushPicker => Some(push::model(self)),
            PopoverKind::CommitMenu { repo_id, commit_id } => {
                Some(commit::model(self, *repo_id, commit_id))
            }
            PopoverKind::TagMenu { repo_id, commit_id } => {
                Some(tag::model(self, *repo_id, commit_id))
            }
            PopoverKind::StatusFileMenu {
                repo_id,
                area,
                path,
            } => Some(status_file::model(self, *repo_id, *area, path, cx)),
            PopoverKind::BranchMenu {
                repo_id,
                section,
                name,
            } => Some(branch::model(self, *repo_id, *section, name)),
            PopoverKind::BranchSectionMenu { repo_id, section } => {
                Some(branch_section::model(self, *repo_id, *section))
            }
            PopoverKind::Repo {
                repo_id,
                kind: RepoPopoverKind::Remote(RemotePopoverKind::Menu { name }),
            } => Some(remote::model(self, *repo_id, name)),
            PopoverKind::StashMenu {
                repo_id,
                index,
                message,
            } => Some(stash::model(*repo_id, *index, message)),
            PopoverKind::Repo {
                repo_id,
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::SectionMenu),
            } => Some(worktree_section::model(*repo_id)),
            PopoverKind::Repo {
                repo_id,
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::Menu { path }),
            } => Some(worktree::model(*repo_id, path)),
            PopoverKind::Repo {
                repo_id,
                kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::SectionMenu),
            } => Some(submodule_section::model(*repo_id)),
            PopoverKind::Repo {
                repo_id,
                kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::Menu { path }),
            } => Some(submodule::model(self, *repo_id, path)),
            PopoverKind::CommitFileMenu {
                repo_id,
                commit_id,
                path,
            } => Some(commit_file::model(self, *repo_id, commit_id, path)),
            PopoverKind::DiffHunkMenu { repo_id, src_ix } => {
                Some(diff_hunk::model(self, *repo_id, *src_ix))
            }
            PopoverKind::DiffEditorMenu {
                repo_id,
                area,
                path,
                hunk_patch,
                hunks_count,
                lines_patch,
                discard_lines_patch,
                lines_count,
                copy_text,
            } => Some(diff_editor::model(
                *repo_id,
                *area,
                path,
                hunk_patch,
                *hunks_count,
                lines_patch,
                discard_lines_patch,
                *lines_count,
                copy_text,
            )),
            PopoverKind::ConflictResolverInputRowMenu {
                line_label,
                line_target,
                chunk_label,
                chunk_target,
            } => Some(conflict_resolver_input_row::model(
                line_label,
                line_target,
                chunk_label,
                chunk_target,
            )),
            PopoverKind::ConflictResolverChunkMenu {
                conflict_ix,
                has_base,
                is_three_way,
                selected_choices,
                output_line_ix,
            } => Some(conflict_resolver_chunk::model(
                *conflict_ix,
                *has_base,
                *is_three_way,
                selected_choices,
                *output_line_ix,
            )),
            PopoverKind::ConflictResolverOutputMenu {
                cursor_line,
                selected_text,
                has_source_a,
                has_source_b,
                has_source_c,
                is_three_way,
            } => Some(conflict_resolver_output::model(
                *cursor_line,
                selected_text,
                *has_source_a,
                *has_source_b,
                *has_source_c,
                *is_three_way,
            )),
            PopoverKind::HistoryBranchFilter { repo_id } => {
                Some(history_branch_filter::model(*repo_id))
            }
            PopoverKind::HistoryColumnSettings => Some(history_column_settings::model(self, cx)),
            PopoverKind::ChangeTrackingSettings => Some(change_tracking_settings::model(self)),
            _ => None,
        }
    }

    pub(super) fn context_menu_activate_action(
        &mut self,
        action: ContextMenuAction,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut close_after_action = true;
        match action {
            ContextMenuAction::SelectDiff { repo_id, target } => {
                self.store.dispatch(Msg::SelectDiff { repo_id, target });
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
                } else if let Err(err) = self.open_path_default(&full_path) {
                    self.push_toast(
                        components::ToastKind::Error,
                        format!("Failed to open: {err}"),
                        cx,
                    );
                }
            }
            ContextMenuAction::OpenFileLocation { repo_id, path } => {
                let full_path = match self.resolve_workdir_path(repo_id, &path) {
                    Ok(path) => path,
                    Err(err) => {
                        self.push_toast(components::ToastKind::Error, err, cx);
                        self.close_popover(cx);
                        return;
                    }
                };

                let target = if full_path.exists() {
                    full_path
                } else {
                    full_path
                        .parent()
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| {
                            self.workdir_for_repo(repo_id)
                                .unwrap_or_else(|| full_path.clone())
                        })
                };

                if !target.exists() {
                    self.push_toast(
                        components::ToastKind::Error,
                        format!("Path not found: {}", target.display()),
                        cx,
                    );
                } else if let Err(err) = self.open_file_location(&target) {
                    self.push_toast(
                        components::ToastKind::Error,
                        format!("Failed to open location: {err}"),
                        cx,
                    );
                }
            }
            ContextMenuAction::OpenRepo { path } => {
                self.store.dispatch(Msg::OpenRepo(path));
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
            ContextMenuAction::CherryPickCommit { repo_id, commit_id } => {
                self.store
                    .dispatch(Msg::CherryPickCommit { repo_id, commit_id });
            }
            ContextMenuAction::RevertCommit { repo_id, commit_id } => {
                self.store
                    .dispatch(Msg::RevertCommit { repo_id, commit_id });
            }
            ContextMenuAction::CheckoutBranch { repo_id, name } => {
                self.store.dispatch(Msg::CheckoutBranch { repo_id, name });
            }
            ContextMenuAction::DeleteBranch { repo_id, name } => {
                self.store.dispatch(Msg::DeleteBranch { repo_id, name });
            }
            ContextMenuAction::SetHistoryScope { repo_id, scope } => {
                self.store.dispatch(Msg::SetHistoryScope { repo_id, scope });
            }
            ContextMenuAction::SetHistoryColumns {
                show_author,
                show_date,
                show_sha,
            } => {
                self.main_pane.update(cx, |pane, cx| {
                    pane.history_view.update(cx, |view, cx| {
                        view.history_show_author = show_author;
                        view.history_show_date = show_date;
                        view.history_show_sha = show_sha;
                        cx.notify();
                    });
                });
                self.schedule_ui_settings_persist(cx);
                close_after_action = false;
            }
            ContextMenuAction::ResetHistoryColumnWidths => {
                self.main_pane.update(cx, |pane, cx| {
                    pane.history_view.update(cx, |view, cx| {
                        view.reset_history_column_widths();
                        cx.notify();
                    });
                });
                close_after_action = false;
            }
            ContextMenuAction::SetChangeTrackingView { view } => {
                self.change_tracking_view = view;
                let root_view = self.root_view.clone();
                cx.defer(move |cx| {
                    let _ = root_view.update(cx, |root, cx| {
                        root.set_change_tracking_view(view, cx);
                    });
                });
            }
            ContextMenuAction::StageSelectionOrPath {
                repo_id,
                area,
                path,
            } => {
                let (paths, used_selection) =
                    self.take_status_paths_for_action(repo_id, area, &path, cx);
                if used_selection {
                    self.store.dispatch(Msg::ClearDiffSelection { repo_id });
                    self.store.dispatch(Msg::StagePaths {
                        repo_id,
                        paths: paths.into(),
                    });
                } else {
                    self.store.dispatch(Msg::SelectDiff {
                        repo_id,
                        target: DiffTarget::WorkingTree {
                            path: path.clone(),
                            area,
                        },
                    });
                    self.store.dispatch(Msg::StagePath { repo_id, path });
                }
            }
            ContextMenuAction::UnstageSelectionOrPath {
                repo_id,
                area,
                path,
            } => {
                let (paths, used_selection) =
                    self.take_status_paths_for_action(repo_id, area, &path, cx);
                if used_selection {
                    self.store.dispatch(Msg::ClearDiffSelection { repo_id });
                    self.store.dispatch(Msg::UnstagePaths {
                        repo_id,
                        paths: paths.into(),
                    });
                } else {
                    self.store.dispatch(Msg::SelectDiff {
                        repo_id,
                        target: DiffTarget::WorkingTree {
                            path: path.clone(),
                            area,
                        },
                    });
                    self.store.dispatch(Msg::UnstagePath { repo_id, path });
                }
            }
            ContextMenuAction::DiscardWorktreeChangesSelectionOrPath {
                repo_id,
                area,
                path,
            } => {
                let anchor = self
                    .popover_anchor
                    .as_ref()
                    .map(|anchor| match anchor {
                        PopoverAnchor::Point(point) => *point,
                        PopoverAnchor::Bounds(bounds) => bounds.bottom_right(),
                    })
                    .unwrap_or_else(|| point(px(64.0), px(64.0)));
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
                self.store.dispatch(Msg::ClearDiffSelection { repo_id });
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
            ContextMenuAction::FetchAll { repo_id } => {
                self.store.dispatch(Msg::FetchAll { repo_id });
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
                let anchor = self
                    .popover_anchor
                    .as_ref()
                    .map(|anchor| match anchor {
                        PopoverAnchor::Point(point) => *point,
                        PopoverAnchor::Bounds(bounds) => bounds.bottom_right(),
                    })
                    .unwrap_or_else(|| point(px(64.0), px(64.0)));
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
            ContextMenuAction::Push { repo_id } => {
                self.store.dispatch(Msg::Push { repo_id });
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
            ContextMenuAction::OpenPopover { kind } => {
                let anchor = self
                    .popover_anchor
                    .as_ref()
                    .map(|anchor| match anchor {
                        PopoverAnchor::Point(point) => *point,
                        PopoverAnchor::Bounds(bounds) => bounds.bottom_right(),
                    })
                    .unwrap_or_else(|| point(px(64.0), px(64.0)));
                self.open_popover_at(kind, anchor, window, cx);
                return;
            }
            ContextMenuAction::ConflictResolverPick { target } => {
                self.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_apply_pick_target(target, cx);
                });
            }
            ContextMenuAction::ConflictResolverOutputCut { text } => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                self.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_output_delete_selection(cx);
                });
            }
            ContextMenuAction::ConflictResolverOutputPaste => {
                if let Some(text) = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text().map(|s| s.to_string()))
                {
                    self.main_pane.update(cx, |pane, cx| {
                        pane.conflict_resolver_output_paste_text(&text, cx);
                    });
                }
            }
            ContextMenuAction::CopyText { text } => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
            ContextMenuAction::ApplyIndexPatch {
                repo_id,
                patch,
                reverse,
            } => {
                if patch.trim().is_empty() {
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
                if patch.trim().is_empty() {
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
                if let Some(patch) = self.build_unified_patch_for_hunk_src_ix(repo_id, src_ix) {
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
                if let Some(patch) = self.build_unified_patch_for_hunk_src_ix(repo_id, src_ix) {
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
        if close_after_action {
            self.close_popover_and_restore_focus(window, cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn discard_worktree_changes_confirmed(
        &mut self,
        repo_id: RepoId,
        area: DiffArea,
        path: Option<std::path::PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        let (paths, _used_selection) = match path.as_ref() {
            Some(clicked_path) => {
                let selection = self.details_pane.update(cx, |pane, cx| {
                    let selection = pane
                        .status_multi_selection
                        .get(&repo_id)
                        .map(|sel| sel.selected_paths_for_area(area))
                        .unwrap_or(&[]);

                    let use_selection =
                        selection.len() > 1 && selection.iter().any(|p| p == clicked_path);
                    if !use_selection {
                        return None;
                    }

                    let sel = pane.status_multi_selection.remove(&repo_id)?;
                    cx.notify();
                    Some(sel.take_selected_paths_for_area(area))
                });

                match selection {
                    Some(paths) if !paths.is_empty() => (paths, true),
                    _ => (vec![clicked_path.clone()], false),
                }
            }
            None => {
                let paths = self
                    .details_pane
                    .update(cx, |pane, cx| {
                        let sel = pane.status_multi_selection.remove(&repo_id)?;
                        cx.notify();
                        Some(sel.take_selected_paths_for_area(area))
                    })
                    .unwrap_or_default();
                if paths.is_empty() {
                    return;
                }
                (paths, true)
            }
        };

        if paths.len() > 1 {
            self.store.dispatch(Msg::ClearDiffSelection { repo_id });
            self.store
                .dispatch(Msg::DiscardWorktreeChangesPaths { repo_id, paths });
            return;
        }

        let Some(path) = paths.into_iter().next() else {
            return;
        };

        let is_added_file = self
            .state
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .and_then(|r| match &r.status {
                Loadable::Ready(status) => status
                    .unstaged
                    .iter()
                    .chain(status.staged.iter())
                    .find(|s| s.path == path)
                    .map(|s| s.kind),
                _ => None,
            })
            .is_some_and(|kind| matches!(kind, FileStatusKind::Untracked | FileStatusKind::Added));

        if is_added_file {
            let path_is_selected = self
                .active_repo()
                .filter(|r| r.id == repo_id)
                .and_then(|r| r.diff_state.diff_target.as_ref())
                .is_some_and(|target| {
                    matches!(target, DiffTarget::WorkingTree { path: selected, .. } if *selected == path)
                });
            if path_is_selected {
                self.store.dispatch(Msg::ClearDiffSelection { repo_id });
            }
        } else {
            self.store.dispatch(Msg::SelectDiff {
                repo_id,
                target: DiffTarget::WorkingTree {
                    path: path.clone(),
                    area: DiffArea::Unstaged,
                },
            });
        }
        self.store
            .dispatch(Msg::DiscardWorktreeChangesPath { repo_id, path });
    }

    pub(super) fn build_unified_patch_for_hunk_src_ix(
        &self,
        repo_id: RepoId,
        hunk_src_ix: usize,
    ) -> Option<String> {
        let repo = self.state.repos.iter().find(|r| r.id == repo_id)?;
        let Loadable::Ready(diff) = &repo.diff_state.diff else {
            return None;
        };
        crate::view::diff_utils::build_unified_patch_for_hunk(diff.lines.as_slice(), hunk_src_ix)
    }

    pub(super) fn context_menu_view(
        &mut self,
        kind: PopoverKind,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let model = self
            .context_menu_model(&kind, cx)
            .unwrap_or_else(|| ContextMenuModel::new(vec![]));
        let model_for_keys = model.clone();

        let focus = self.context_menu_focus_handle.clone();
        let current_selected = self.context_menu_selected_ix;
        let selected_for_render = current_selected
            .filter(|&ix| model.is_selectable(ix))
            .or_else(|| model.first_selectable());

        components::context_menu(
            theme,
            div()
                .w_full()
                .min_w_full()
                .flex()
                .flex_col()
                .track_focus(&focus)
                .key_context("ContextMenu")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _e: &MouseDownEvent, window, _cx| {
                        window.focus(&this.context_menu_focus_handle);
                    }),
                )
                .on_key_down(
                    cx.listener(move |this, e: &gpui::KeyDownEvent, window, cx| {
                        let key = e.keystroke.key.as_str();
                        let mods = e.keystroke.modifiers;
                        if mods.control || mods.platform || mods.alt || mods.function {
                            return;
                        }

                        match key {
                            "escape" => {
                                this.close_popover_and_restore_focus(window, cx);
                            }
                            "up" => {
                                let next = model_for_keys
                                    .next_selectable(this.context_menu_selected_ix, -1);
                                this.context_menu_selected_ix = next;
                                cx.notify();
                            }
                            "down" => {
                                let next = model_for_keys
                                    .next_selectable(this.context_menu_selected_ix, 1);
                                this.context_menu_selected_ix = next;
                                cx.notify();
                            }
                            "home" => {
                                this.context_menu_selected_ix = model_for_keys.first_selectable();
                                cx.notify();
                            }
                            "end" => {
                                this.context_menu_selected_ix = model_for_keys.last_selectable();
                                cx.notify();
                            }
                            "enter" => {
                                let Some(ix) = context_menu_activate_entry_ix(
                                    &model_for_keys,
                                    this.context_menu_selected_ix,
                                ) else {
                                    return;
                                };
                                if let Some(action) =
                                    context_menu_entry_action_at(&model_for_keys, ix)
                                {
                                    this.context_menu_activate_action(action, window, cx);
                                }
                            }
                            _ => {
                                if let Some(ix) =
                                    context_menu_shortcut_entry_ix(&model_for_keys, key)
                                    && let Some(action) =
                                        context_menu_entry_action_at(&model_for_keys, ix)
                                {
                                    this.context_menu_activate_action(action, window, cx);
                                }
                            }
                        }
                    }),
                )
                .children(model.items.into_iter().enumerate().map(|(ix, item)| {
                    match item {
                        ContextMenuItem::Separator => components::context_menu_separator(theme)
                            .id(("context_menu_sep", ix))
                            .into_any_element(),
                        ContextMenuItem::Header(title) => {
                            components::context_menu_header(theme, title)
                                .id(("context_menu_header", ix))
                                .into_any_element()
                        }
                        ContextMenuItem::Label(text) => components::context_menu_label(theme, text)
                            .id(("context_menu_label", ix))
                            .into_any_element(),
                        ContextMenuItem::Entry {
                            label,
                            icon,
                            shortcut,
                            disabled,
                            action,
                        } => {
                            let selected = selected_for_render == Some(ix);
                            let debug_selector = context_menu_entry_debug_selector(label.as_ref());
                            let activate_on_click = action.as_ref().clone();
                            let activate_on_right_release = activate_on_click.clone();
                            let row = components::context_menu_entry(
                                ("context_menu_entry", ix),
                                theme,
                                selected,
                                disabled,
                                icon,
                                label,
                                shortcut,
                            )
                            .debug_selector(move || debug_selector.clone());

                            row.on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                                if *hovering {
                                    this.context_menu_selected_ix = Some(ix);
                                    cx.notify();
                                }
                            }))
                            .when(!disabled, |row| {
                                row.on_mouse_up(
                                    MouseButton::Right,
                                    cx.listener(move |this, _e: &MouseUpEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.context_menu_activate_action(
                                            activate_on_right_release.clone(),
                                            window,
                                            cx,
                                        );
                                    }),
                                )
                                .on_click(cx.listener(
                                    move |this, e: &ClickEvent, window, cx| {
                                        if e.is_right_click() {
                                            cx.stop_propagation();
                                            return;
                                        }
                                        cx.stop_propagation();
                                        this.context_menu_activate_action(
                                            activate_on_click.clone(),
                                            window,
                                            cx,
                                        );
                                    },
                                ))
                            })
                            .into_any_element()
                        }
                    }
                }))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_menu_shortcut_entry_ix_matches_first_enabled_single_character_entry() {
        let model = ContextMenuModel::new(vec![
            ContextMenuItem::Header("Test".into()),
            ContextMenuItem::Entry {
                label: "Disabled A".into(),
                icon: None,
                shortcut: Some("A".into()),
                disabled: true,
                action: Box::new(ContextMenuAction::FetchAll { repo_id: RepoId(1) }),
            },
            ContextMenuItem::Entry {
                label: "Enter".into(),
                icon: None,
                shortcut: Some("Enter".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::FetchAll { repo_id: RepoId(2) }),
            },
            ContextMenuItem::Entry {
                label: "Ctrl Copy".into(),
                icon: None,
                shortcut: Some("Ctrl+C".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::FetchAll { repo_id: RepoId(3) }),
            },
            ContextMenuItem::Entry {
                label: "Enabled A".into(),
                icon: None,
                shortcut: Some("A".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::FetchAll { repo_id: RepoId(4) }),
            },
        ]);

        assert_eq!(context_menu_shortcut_entry_ix(&model, "a"), Some(4));
        assert_eq!(context_menu_shortcut_entry_ix(&model, "A"), Some(4));
        assert_eq!(context_menu_shortcut_entry_ix(&model, "c"), None);
        assert_eq!(context_menu_shortcut_entry_ix(&model, "e"), None);
        assert_eq!(context_menu_shortcut_entry_ix(&model, "enter"), None);
    }

    #[test]
    fn context_menu_activate_entry_ix_prefers_selected_entry_and_falls_back_to_first_selectable() {
        let model = ContextMenuModel::new(vec![
            ContextMenuItem::Header("Test".into()),
            ContextMenuItem::Entry {
                label: "Disabled".into(),
                icon: None,
                shortcut: Some("D".into()),
                disabled: true,
                action: Box::new(ContextMenuAction::FetchAll { repo_id: RepoId(1) }),
            },
            ContextMenuItem::Entry {
                label: "First".into(),
                icon: None,
                shortcut: Some("Enter".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::FetchAll { repo_id: RepoId(2) }),
            },
            ContextMenuItem::Entry {
                label: "Second".into(),
                icon: None,
                shortcut: Some("S".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::FetchAll { repo_id: RepoId(3) }),
            },
        ]);

        assert_eq!(context_menu_activate_entry_ix(&model, None), Some(2));
        assert_eq!(context_menu_activate_entry_ix(&model, Some(3)), Some(3));
        assert_eq!(context_menu_activate_entry_ix(&model, Some(1)), Some(2));
        assert_eq!(context_menu_activate_entry_ix(&model, Some(99)), Some(2));
    }
}
