use super::*;

mod branch;
mod branch_group;
mod branch_refs;
mod branch_section;
mod browse_history;
mod change_tracking_settings;
mod commit;
mod commit_file;
mod commit_file_sort;
mod commit_options;
mod commit_sha_link;
mod conflict_resolver_chunk;
mod conflict_resolver_input_row;
mod conflict_resolver_output;
mod diff_actions;
mod diff_content_mode_settings;
mod diff_editor;
mod diff_hunk;
mod file_browser_file;
mod file_browser_folder;
mod history_branch_filter;
mod mergetool_settings;
mod pinned_section;
mod previous_commit_messages;
mod pull;
mod push;
mod reflog_entry;
mod remote;
mod repo_picker_row;
mod repo_tab;
mod stash;
mod status_file;
mod submodule;
mod submodule_inner_diff;
mod submodule_section;
mod tag;
mod terminal;
mod ui_scale_picker;
mod web_link;
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

/// One line of the "Add to .gitignore" field as a submittable pattern, or
/// `None` when the line is blank.
///
/// Trailing spaces go through git's own rule rather than `str::trim`, which
/// would unescape a deliberate `foo\ ` back into a dangling backslash. Leading
/// whitespace is dropped: it is significant to git, but a leading space in a
/// hand-typed line is copy-paste noise far more often than intent, and the
/// resulting pattern would silently match nothing.
fn gitignore_pattern_line(line: &str) -> Option<&str> {
    let line = gitcomet_core::gitignore::trim_trailing_spaces(line.trim_start());
    (!line.is_empty()).then_some(line)
}

pub(super) fn path_text_for_copy(path: &std::path::Path) -> String {
    normalize_platform_path(path.to_path_buf())
        .display()
        .to_string()
}

/// The `Copy absolute path` / `Copy relative path` pair every file-ish menu
/// ends with, for a repo-relative `path`.
///
/// Built in one place so the labels, icons and mnemonic cannot drift apart
/// between menus: the mnemonic is matched on the key alone, so `c` has to mean
/// the same thing in whichever menu is open.
pub(super) fn push_copy_path_entries(
    items: &mut Vec<ContextMenuItem>,
    host: &PopoverHost,
    repo_id: RepoId,
    path: &std::path::Path,
    relative_shortcut: Option<SharedString>,
) {
    // Offered only when the workdir join actually resolves. Falling back to the
    // repo-relative text would put identical content behind two entries whose
    // labels promise different things.
    if let Ok(absolute) = host.resolve_workdir_path(repo_id, path) {
        items.push(ContextMenuItem::Entry {
            label: "Copy absolute path".into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CopyText {
                text: path_text_for_copy(&absolute),
            }),
        });
    }
    items.push(ContextMenuItem::Entry {
        label: "Copy relative path".into(),
        icon: Some("icons/copy.svg".into()),
        shortcut: relative_shortcut,
        disabled: false,
        action: Box::new(ContextMenuAction::CopyText {
            text: path_text_for_copy(path),
        }),
    });
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

fn context_menu_entry_tooltip(action: &ContextMenuAction) -> Option<SharedString> {
    match action {
        ContextMenuAction::UseCommitMessage { message } => {
            let text = message.trim();
            (!text.is_empty()).then(|| text.to_owned().into())
        }
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
        let shortcut_key = shortcut
            .as_ref()
            .rsplit('+')
            .next()
            .unwrap_or(shortcut.as_ref());
        shortcut_key.eq_ignore_ascii_case(key).then_some(ix)
    })
}

impl PopoverHost {
    pub(super) fn workdir_for_repo(&self, repo_id: RepoId) -> Option<std::path::PathBuf> {
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

    fn open_path_default(&mut self, path: std::path::PathBuf, cx: &mut gpui::Context<Self>) {
        crate::view::platform_open::spawn_launch(
            cx,
            move || crate::view::platform_open::open_path_blocking(&path),
            |this, result, cx| {
                if let Err(err) = result {
                    this.push_toast(
                        components::ToastKind::Error,
                        format!("Failed to open: {err}"),
                        cx,
                    );
                }
            },
        );
    }

    fn open_file_location(&mut self, path: std::path::PathBuf, cx: &mut gpui::Context<Self>) {
        crate::view::platform_open::spawn_launch(
            cx,
            move || crate::view::platform_open::open_file_location_blocking(&path),
            |this, result, cx| {
                if let Err(err) = result {
                    this.push_toast(
                        components::ToastKind::Error,
                        format!("Failed to open location: {err}"),
                        cx,
                    );
                }
            },
        );
    }

    fn reveal_path_in_file_manager(
        &mut self,
        path: std::path::PathBuf,
        fallback: Option<std::path::PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        let target = if path.exists() {
            path
        } else {
            path.parent()
                .map(ToOwned::to_owned)
                .or(fallback)
                .unwrap_or(path)
        };

        if !target.exists() {
            self.push_toast(
                components::ToastKind::Error,
                format!("Path not found: {}", target.display()),
                cx,
            );
        } else {
            self.open_file_location(target, cx);
        }
    }

    /// The paths a context-menu action on `clicked_path` covers, plus whether
    /// they came out of the row selection. Reads only — see
    /// [`Self::clear_status_multi_selection`] for the other half.
    /// The screen point a follow-up popover should open at, derived from the
    /// anchor of the menu that is currently open.
    ///
    /// Every context-menu action that opens a dialog needs this, and the six
    /// copies it replaced all carried the same duplicated fallback constant.
    fn popover_anchor_point(&self) -> gpui::Point<Pixels> {
        self.popover_anchor
            .as_ref()
            .map(|anchor| match anchor {
                PopoverAnchor::Point(point) => *point,
                PopoverAnchor::Bounds(bounds) => bounds.bottom_right(),
                PopoverAnchor::Centered => point(px(64.0), px(64.0)),
            })
            .unwrap_or_else(|| point(px(64.0), px(64.0)))
    }

    fn status_paths_for_action(
        &self,
        repo_id: RepoId,
        area: DiffArea,
        clicked_path: &std::path::PathBuf,
        cx: &gpui::App,
    ) -> (Vec<std::path::PathBuf>, bool) {
        self.details_pane
            .read(cx)
            .status_selected_paths_for_action(repo_id, area, clicked_path)
    }

    /// Drop the row selection because an action has gone ahead with it. Never
    /// call this before the action is settled: a confirmation the user cancels
    /// must leave the selection standing.
    pub(super) fn clear_status_multi_selection(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        self.details_pane.update(cx, |pane, cx| {
            pane.clear_status_multi_selection(repo_id);
            cx.notify();
        });
    }

    fn take_status_paths_for_action(
        &mut self,
        repo_id: RepoId,
        area: DiffArea,
        clicked_path: &std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) -> (Vec<std::path::PathBuf>, bool) {
        let (paths, used_selection) = self.status_paths_for_action(repo_id, area, clicked_path, cx);
        if used_selection {
            self.clear_status_multi_selection(repo_id, cx);
        }
        (paths, used_selection)
    }

    fn repo_is_open(&self, repo_id: RepoId) -> bool {
        self.state.repos.iter().any(|repo| repo.id == repo_id)
    }

    /// A toast rather than an error banner: nothing failed, the row just went
    /// stale, and the banner belongs to whichever repository is active now — not
    /// to the one that left.
    fn warn_repository_gone(&mut self, cx: &mut gpui::Context<Self>) {
        self.push_toast(
            components::ToastKind::Warning,
            "That repository is no longer open.".to_owned(),
            cx,
        );
    }

    /// The menu a repository row in the picker offers. Not reachable through
    /// [`Self::context_menu_model`] because it has no popover kind of its own:
    /// it is only ever floated over the picker by
    /// [`super::picker_row_menu`](crate::view::panels::popover), never opened as
    /// a popover in its own right.
    pub(super) fn repo_picker_row_menu_model(
        &self,
        entry: &repo_picker::RepoPickerEntry,
    ) -> ContextMenuModel {
        repo_picker_row::model(self, entry)
    }

    pub(in super::super) fn context_menu_model(
        &self,
        kind: &PopoverKind,
        cx: &gpui::Context<Self>,
    ) -> Option<ContextMenuModel> {
        match kind {
            PopoverKind::AppMenu => Some(app_menu::model(self)),
            PopoverKind::AddRepoMenu => Some(add_repo_menu::model()),
            PopoverKind::PullPicker => Some(pull::model(self)),
            PopoverKind::PushPicker => Some(push::model(self)),
            PopoverKind::CommitOptionsMenu { repo_id } => {
                Some(commit_options::model(self, *repo_id))
            }
            PopoverKind::PreviousCommitMessagesMenu { repo_id } => {
                Some(previous_commit_messages::model(self, *repo_id))
            }
            PopoverKind::RepoTabMenu { repo_id } => Some(repo_tab::model(self, *repo_id)),
            PopoverKind::CommitMenu { repo_id, commit_id } => {
                Some(commit::model(self, *repo_id, commit_id))
            }
            PopoverKind::ReflogEntryMenu {
                repo_id,
                target,
                selector,
            } => Some(reflog_entry::model(*repo_id, selector, target)),
            PopoverKind::TagMenu { repo_id, commit_id } => {
                Some(tag::model(self, *repo_id, commit_id))
            }
            PopoverKind::TagRefMenu {
                repo_id,
                commit_id,
                name,
            } => Some(tag::model_for_tag(self, *repo_id, commit_id, name)),
            PopoverKind::StatusFileMenu {
                repo_id,
                area,
                path,
            } => Some(status_file::model(self, *repo_id, *area, path, cx)),
            PopoverKind::BranchMenu { repo_id, target } => {
                Some(branch::model(self, *repo_id, target))
            }
            PopoverKind::BranchRefsMenu {
                repo_id,
                display_name,
                targets,
            } => Some(branch_refs::model(*repo_id, display_name, targets)),
            PopoverKind::BranchSectionMenu { repo_id, section } => {
                Some(branch_section::model(self, *repo_id, *section))
            }
            PopoverKind::Repo {
                repo_id,
                kind: RepoPopoverKind::Remote(RemotePopoverKind::Menu { name }),
            } => Some(remote::model(self, *repo_id, name)),
            PopoverKind::WebLinkMenu {
                url,
                load_remote_image_url,
            } => {
                let load_remote_image_url = load_remote_image_url.as_deref().filter(|_| {
                    self.main_pane.read(cx).remote_markdown_image_policy
                        == RemoteMarkdownImagePolicy::AskBeforeLoading
                });
                Some(web_link::model(url, load_remote_image_url))
            }
            PopoverKind::CommitShaLinkMenu {
                repo_id,
                commit_id,
                allow_navigate,
            } => Some(commit_sha_link::model(*repo_id, commit_id, *allow_navigate)),
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
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::Menu { path, branch }),
            } => Some(worktree::model(*repo_id, path, branch.as_deref())),
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
            PopoverKind::CommitFileSortMenu => Some(commit_file_sort::model(self, cx)),
            PopoverKind::FileBrowserFileMenu { repo_id, path } => {
                Some(file_browser_file::model(self, *repo_id, path, cx))
            }
            PopoverKind::FileBrowserFolderMenu { repo_id, path } => {
                Some(file_browser_folder::model(self, *repo_id, path))
            }
            PopoverKind::BranchGroupMenu {
                repo_id,
                section,
                remote,
                path,
            } => Some(branch_group::model(
                self,
                *repo_id,
                *section,
                remote.as_deref(),
                path,
            )),
            PopoverKind::PinnedSectionMenu { repo_id, section } => {
                Some(pinned_section::model(self, *repo_id, *section))
            }
            PopoverKind::BrowseHistoryMenu { repo_id } => {
                Some(browse_history::model(self, *repo_id))
            }
            PopoverKind::SubmoduleInnerDiffMenu {
                repo_id,
                submodule_repo_path,
                target,
            } => Some(submodule_inner_diff::model(
                *repo_id,
                submodule_repo_path,
                target,
            )),
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
                copy_target,
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
                *copy_target,
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
                split_selection_rows,
                join_previous_region,
                join_next_region,
                alignment_marked_columns,
                has_manual_alignments,
                output_is_protected,
            } => Some(conflict_resolver_chunk::model(
                *conflict_ix,
                *has_base,
                *is_three_way,
                selected_choices,
                *output_line_ix,
                *split_selection_rows,
                join_previous_region.clone(),
                join_next_region.clone(),
                *alignment_marked_columns,
                *has_manual_alignments,
                *output_is_protected,
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
                Some(history_branch_filter::model(self, *repo_id))
            }
            PopoverKind::DiffActionMenu => Some(diff_actions::model(self)),
            PopoverKind::MergetoolSettingsMenu => Some(mergetool_settings::model(self, cx)),
            PopoverKind::DiffContentModeSettings => Some(diff_content_mode_settings::model(self)),
            PopoverKind::ChangeTrackingSettings => Some(change_tracking_settings::model(self)),
            PopoverKind::UiScalePicker => Some(ui_scale_picker::model(cx)),
            PopoverKind::InteractiveRebaseActionMenu {
                ix,
                can_squash,
                can_drop,
            } => {
                let pick_locked = self
                    .main_pane
                    .read_with(cx, |pane, _| pane.active_entry_pick_locked(*ix));
                Some(interactive_rebase_action_menu_model(
                    *ix,
                    *can_squash,
                    *can_drop,
                    pick_locked,
                ))
            }
            PopoverKind::InteractiveRebaseAutosquashMenu => {
                Some(interactive_rebase_autosquash_menu_model())
            }
            PopoverKind::TerminalMenu { repo_id, context } => {
                Some(terminal::model(*repo_id, *context, cx))
            }
            _ => None,
        }
    }

    pub(in crate::view) fn context_menu_activate_action(
        &mut self,
        action: ContextMenuAction,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut close_after_action = true;
        let mut restore_diff_panel_focus_after_action = false;
        match action {
            ContextMenuAction::AppMenu(action) => {
                app_menu::activate(self, action, window, cx);
                return;
            }
            ContextMenuAction::AddRepoMenu(action) => {
                add_repo_menu::activate(self, action, window, cx);
                return;
            }
            ContextMenuAction::SelectDiff { repo_id, target } => {
                self.store.dispatch(Msg::SelectDiff { repo_id, target });
            }
            ContextMenuAction::OpenFileContent {
                repo_id,
                source,
                path,
            } => {
                self.store.dispatch(Msg::OpenFileContent {
                    repo_id,
                    source,
                    path,
                });
            }
            ContextMenuAction::EditFile { repo_id, path } => {
                self.store.dispatch(Msg::OpenFileEditor { repo_id, path });
            }
            ContextMenuAction::DiscardFileEdits { repo_id, path } => {
                self.main_pane.update(cx, |pane, cx| {
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
                let _ = self.root_view.update(cx, |root, cx| {
                    root.open_path_in_external_code_editor(full_path, cx);
                });
            }
            ContextMenuAction::OpenRepo { path } => {
                self.store.dispatch(Msg::OpenRepo(path));
            }
            ContextMenuAction::ActivateRepo { repo_id } => {
                if !self.repo_is_open(repo_id) {
                    self.warn_repository_gone(cx);
                    return;
                }
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
                self.store.dispatch(Msg::CloseRepo { repo_id });
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
                self.store.dispatch(Msg::CloseRepos {
                    repo_ids,
                    activate_after,
                });
            }
            ContextMenuAction::OpenSubmoduleDiffInTab { path, target } => {
                self.main_pane.update(cx, |pane, cx| {
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
            ContextMenuAction::RevertCommit { repo_id, commit_id } => {
                self.store
                    .dispatch(Msg::RevertCommit { repo_id, commit_id });
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
                let _ = self.root_view.update(cx, |root, _| {
                    root.pending_force_delete_branch_centered = false;
                });
                self.store.dispatch(Msg::DeleteBranch { repo_id, name });
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
            ContextMenuAction::SetCommitFileSort { sort } => {
                self.details_pane.update(cx, |pane, cx| {
                    pane.set_commit_file_sort(sort, cx);
                });
            }
            ContextMenuAction::SetDiffContentMode { mode } => {
                self.diff_content_mode = mode;
                let main_pane = self.main_pane.clone();
                cx.defer(move |cx| {
                    main_pane.update(cx, |pane, cx| {
                        pane.set_diff_content_mode_and_persist(mode, cx);
                    });
                });
            }
            ContextMenuAction::SetDiffWhitespaceMode { mode } => {
                close_after_action = false;
                restore_diff_panel_focus_after_action = true;
                self.diff_whitespace_mode = mode;
                let main_pane = self.main_pane.clone();
                cx.defer(move |cx| {
                    main_pane.update(cx, |pane, cx| {
                        pane.set_diff_whitespace_mode_and_persist(mode, cx);
                    });
                });
            }
            ContextMenuAction::SetDiffRevealWhitespaceChars { enabled } => {
                close_after_action = false;
                restore_diff_panel_focus_after_action = true;
                self.diff_reveal_whitespace_chars = enabled;
                let main_pane = self.main_pane.clone();
                cx.defer(move |cx| {
                    main_pane.update(cx, |pane, cx| {
                        pane.set_diff_reveal_whitespace_chars_and_persist(enabled, cx);
                    });
                });
            }
            ContextMenuAction::SetDiffWordWrap { enabled } => {
                close_after_action = false;
                restore_diff_panel_focus_after_action = true;
                self.diff_word_wrap = enabled;
                let main_pane = self.main_pane.clone();
                cx.defer(move |cx| {
                    main_pane.update(cx, |pane, cx| {
                        pane.set_diff_word_wrap_and_persist(enabled, cx);
                    });
                });
            }
            ContextMenuAction::SetDiffShowLineNumbers { enabled } => {
                close_after_action = false;
                restore_diff_panel_focus_after_action = true;
                self.diff_show_line_numbers = enabled;
                let main_pane = self.main_pane.clone();
                cx.defer(move |cx| {
                    main_pane.update(cx, |pane, cx| {
                        pane.set_diff_show_line_numbers_and_persist(enabled, cx);
                    });
                });
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
            ContextMenuAction::SetCommitAmendEnabled { enabled } => {
                close_after_action = false;
                self.commit_amend_enabled = enabled;
                let root_view = self.root_view.clone();
                cx.defer(move |cx| {
                    let _ = root_view.update(cx, |root, cx| {
                        root.set_commit_amend_enabled(enabled, cx);
                    });
                });
            }
            ContextMenuAction::SetCommitPushAfterEnabled { enabled } => {
                close_after_action = false;
                self.commit_push_after_enabled = enabled;
                let root_view = self.root_view.clone();
                cx.defer(move |cx| {
                    let _ = root_view.update(cx, |root, cx| {
                        root.set_commit_push_after_enabled(enabled, cx);
                    });
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
            ContextMenuAction::AddToGitignoreSelectionOrPath {
                repo_id,
                area,
                path,
            } => {
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
            ContextMenuAction::Push { repo_id } => {
                let request = self
                    .state
                    .repos
                    .iter()
                    .find(|repo| repo.id == repo_id)
                    .map(push_request)
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
            ContextMenuAction::SetUiScale { percent } => {
                cx.defer(move |cx| {
                    crate::app::set_app_ui_scale_percent(cx, percent);
                });
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
                            let _ = root_view.update(cx, |root, cx| {
                                root.open_popover_centered(
                                    PopoverKind::RebaseReword {
                                        ix,
                                        original_action,
                                        original_message: msg,
                                    },
                                    window,
                                    cx,
                                );
                            });
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
            ContextMenuAction::OpenPopover { kind } => {
                let anchor = self.popover_anchor_point();
                self.open_popover_at(kind, anchor, window, cx);
                return;
            }
            ContextMenuAction::ConflictResolverPick { target } => {
                self.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_apply_pick_target(target, cx);
                });
            }
            ContextMenuAction::ConflictResolverUnresolve { conflict_ix } => {
                self.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_select_conflict(conflict_ix, cx);
                    pane.conflict_resolver_unresolve_active_conflict(cx);
                });
            }
            ContextMenuAction::ConflictResolverSplitSelection => {
                self.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_split_selection(cx);
                });
            }
            ContextMenuAction::ConflictResolverAlignManually => {
                self.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_align_manually(cx);
                });
            }
            ContextMenuAction::ConflictResolverClearManualAlignments => {
                self.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_clear_manual_alignments(cx);
                });
            }
            ContextMenuAction::ConflictResolverJoinRegions { target } => {
                self.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_join_regions(target, cx);
                });
            }
            ContextMenuAction::SetMergetoolAutoAdvance { enabled } => {
                close_after_action = false;
                self.main_pane.update(cx, |pane, cx| {
                    pane.set_mergetool_auto_advance_and_persist(enabled, cx);
                });
                cx.notify();
            }
            ContextMenuAction::ToggleMergetoolCollapseUnchanged => {
                close_after_action = false;
                self.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_toggle_collapse_context(cx);
                });
                cx.notify();
            }
            ContextMenuAction::SetMergetoolOutputScrollSync { enabled } => {
                close_after_action = false;
                self.main_pane.update(cx, |pane, cx| {
                    pane.set_mergetool_output_scroll_sync_and_persist(enabled, cx);
                });
                cx.notify();
            }
            ContextMenuAction::SetMergetoolShowLineNumbers { enabled } => {
                close_after_action = false;
                self.main_pane.update(cx, |pane, cx| {
                    pane.set_mergetool_show_line_numbers_and_persist(enabled, cx);
                });
                cx.notify();
            }
            ContextMenuAction::SetMergetoolThreeWayView { enabled } => {
                close_after_action = false;
                self.main_pane.update(cx, |pane, cx| {
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
                self.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_output_delete_selection(cx);
                });
            }
            ContextMenuAction::ConflictResolverOutputPaste => {
                if let Some(text) = crate::clipboard::read_text(cx) {
                    self.main_pane.update(cx, |pane, cx| {
                        pane.conflict_resolver_output_paste_text(&text, cx);
                    });
                }
            }
            ContextMenuAction::CopyText { text } => {
                window.activate_window();
                crate::clipboard::write_text(cx, text, crate::clipboard::CopySource::ContextMenu);
            }
            ContextMenuAction::CopyLinkAddress { url } => {
                window.activate_window();
                crate::clipboard::write_text(cx, url, crate::clipboard::CopySource::ContextMenu);
                self.push_toast(
                    components::ToastKind::Success,
                    "Link copied to clipboard".to_string(),
                    cx,
                );
            }
            ContextMenuAction::LoadRemoteMarkdownImage { url } => {
                self.main_pane.update(cx, |pane, cx| {
                    pane.approve_remote_markdown_image(url, cx);
                });
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
                window.activate_window();
                crate::clipboard::write_text(
                    cx,
                    text,
                    crate::clipboard::CopySource::DiffContextMenu,
                );
            }
            ContextMenuAction::CopyDiffText { visible_ix, region } => {
                window.activate_window();
                self.main_pane.update(cx, |pane, cx| {
                    pane.copy_diff_text_for_context_menu_to_clipboard(visible_ix, region, cx);
                });
            }
            ContextMenuAction::TerminalCopy { repo_id } => {
                window.activate_window();
                let _ = self.root_view.update(cx, |root, cx| {
                    root.copy_terminal_selection_for_repo(repo_id, window, cx);
                });
            }
            ContextMenuAction::TerminalPaste { repo_id } => {
                let _ = self.root_view.update(cx, |root, cx| {
                    root.paste_terminal_clipboard_for_repo(repo_id, window, cx);
                });
            }
            ContextMenuAction::TerminalSelectAll { repo_id } => {
                let _ = self.root_view.update(cx, |root, cx| {
                    root.select_all_terminal_for_repo(repo_id, window, cx);
                });
            }
            ContextMenuAction::TerminalClear { repo_id } => {
                let _ = self.root_view.update(cx, |root, cx| {
                    root.clear_terminal_for_repo(repo_id, window, cx);
                });
            }
            ContextMenuAction::TerminalOpenExternal { repo_id } => {
                let _ = self.root_view.update(cx, |root, cx| {
                    root.open_external_terminal_from_menu(repo_id, window, cx);
                });
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
        // A menu floating over a picker is not the popover: it closed itself on
        // the way in, and whether the picker underneath survives is the picker's
        // call, not the action's.
        if close_after_action && !self.suppress_popover_close_after_action {
            self.close_popover_and_restore_focus(window, cx);
        } else {
            if restore_diff_panel_focus_after_action {
                let focus = self.main_pane.read(cx).diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
            }
            cx.notify();
        }
    }

    fn context_menu_activate_model_entry(
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

    /// What an "Add to .gitignore" action on `path` would cover, or `None` when
    /// the action does not apply.
    ///
    /// The single source of truth for eligibility: the menu uses it to decide
    /// whether to show the entry and the dialog uses it to seed itself. Two
    /// copies of this rule would let the menu offer an action the dialog then
    /// refuses (or the reverse), and nothing would catch the drift.
    ///
    /// The selection is read but *not* consumed — the dialog is cancellable, and
    /// losing a selection to a dialog the user backed out of is exactly the
    /// failure the read/take split documented above exists to prevent.
    pub(super) fn add_to_gitignore_target(
        &self,
        repo_id: RepoId,
        area: DiffArea,
        path: &std::path::PathBuf,
        cx: &gpui::App,
    ) -> Option<(
        Vec<std::path::PathBuf>,
        gitcomet_core::gitignore::GitignoreSuggestions,
    )> {
        use gitcomet_core::domain::FileStatusKind;

        // `.gitignore` has no effect on anything already in the index, so a
        // pattern for a tracked path is a line that changes nothing and leaves
        // the row exactly where it was.
        if area != DiffArea::Unstaged {
            return None;
        }
        let repo = self.state.repos.iter().find(|r| r.id == repo_id)?;
        let (paths, used_selection) = self.status_paths_for_action(repo_id, area, path, cx);

        // Every targeted path must be untracked, not just the clicked one: the
        // untracked and unstaged buckets are separate, and a tracked path that
        // snuck into the selection is a silent no-op the user would debug.
        // Indexed when there is a selection to check, because one
        // `status_entry_for_path` per path is a linear scan of the whole status
        // list each time and this runs on every right-click.
        let all_untracked = if used_selection {
            let untracked: FxHashSet<&std::path::Path> = repo
                .status_entries_for_area(area)
                .unwrap_or(&[])
                .iter()
                .filter(|entry| entry.kind == FileStatusKind::Untracked)
                .map(|entry| entry.path.as_path())
                .collect();
            paths.iter().all(|p| untracked.contains(p.as_path()))
        } else {
            paths.first().is_some_and(|p| {
                matches!(
                    repo.status_entry_for_path(area, p).map(|s| s.kind),
                    Some(FileStatusKind::Untracked)
                )
            })
        };
        if !all_untracked {
            return None;
        }

        // Rules out anything with no expressible pattern at all: a non-UTF-8
        // path, or a name containing a line break.
        let suggestions = gitcomet_core::gitignore::suggestions_for_paths(&paths)?;
        Some((paths, suggestions))
    }

    /// Seed the "Add to .gitignore" dialog when it opens.
    pub(super) fn prepare_add_to_gitignore(
        &mut self,
        repo_id: RepoId,
        area: DiffArea,
        path: &std::path::PathBuf,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let target = self.add_to_gitignore_target(repo_id, area, path, cx);
        let scope = gitcomet_core::gitignore::GitignoreScope::File;
        let text = target
            .as_ref()
            .map(|(_, suggestions)| suggestions.lines_for(scope).join("\n"))
            .unwrap_or_default();
        let (paths, suggestions) = match target {
            Some((paths, suggestions)) => (paths, Some(suggestions)),
            None => (vec![path.clone()], None),
        };

        self.gitignore_paths = paths;
        self.gitignore_suggestions = suggestions;
        self.gitignore_scope = scope;

        let theme = self.theme;
        self.gitignore_patterns_input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text(&text, cx);
            cx.notify();
        });
        // `set_text` resets only the horizontal offset, so a dialog reopened
        // after scrolling a long selection would show blank space where the
        // patterns are.
        self.gitignore_patterns_scroll
            .set_offset(gpui::point(px(0.0), px(0.0)));
        let focus = self
            .gitignore_patterns_input
            .read_with(cx, |i, _| i.focus_handle());
        window.focus(&focus, cx);
    }

    /// Re-seed the pattern field after the user picks a different scope.
    ///
    /// This overwrites whatever is in the field. That is the point: the user
    /// just asked for a different pattern, and merging the old text into the
    /// new scope would leave a field matching neither.
    pub(super) fn set_add_to_gitignore_scope(
        &mut self,
        scope: gitcomet_core::gitignore::GitignoreScope,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(text) = self
            .gitignore_suggestions
            .as_ref()
            .map(|s| s.lines_for(scope).join("\n"))
        else {
            return;
        };
        self.gitignore_scope = scope;
        self.gitignore_patterns_input.update(cx, |input, cx| {
            input.set_text(&text, cx);
            cx.notify();
        });
        // As in `prepare_add_to_gitignore`: the new text is usually shorter than
        // what it replaced, so a stale vertical offset would scroll it off.
        self.gitignore_patterns_scroll
            .set_offset(gpui::point(px(0.0), px(0.0)));
        cx.notify();
    }

    /// The non-blank lines currently in the pattern field.
    pub(super) fn add_to_gitignore_patterns(&self, cx: &gpui::App) -> Vec<String> {
        self.gitignore_patterns_input.read_with(cx, |input, _| {
            input
                .text()
                .lines()
                .filter_map(gitignore_pattern_line)
                .map(ToOwned::to_owned)
                .collect()
        })
    }

    /// Whether the pattern field holds anything submittable.
    ///
    /// Separate from [`Self::add_to_gitignore_patterns`] because this runs every
    /// frame the dialog is on screen, and building the whole `Vec<String>` just
    /// to ask whether it is empty allocates one `String` per selected file per
    /// frame.
    pub(super) fn can_submit_add_to_gitignore(&self, cx: &gpui::App) -> bool {
        self.gitignore_patterns_input.read_with(cx, |input, _| {
            input
                .text()
                .lines()
                .any(|line| gitignore_pattern_line(line).is_some())
        })
    }

    pub(super) fn submit_add_to_gitignore(
        &mut self,
        repo_id: RepoId,
        area: DiffArea,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        let patterns = self.add_to_gitignore_patterns(cx);
        if patterns.is_empty() {
            return;
        }
        // Now that the action is going ahead, the row selection has served its
        // purpose and is cleared. The returned paths are unused — the patterns
        // come from the field, which the user may have edited.
        let _ = self.take_status_paths_for_action(repo_id, area, &path, cx);
        self.store
            .dispatch(Msg::AppendGitignorePatterns { repo_id, patterns });
        self.close_popover(cx);
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
            .and_then(|repo| {
                repo.status_entry_for_path(DiffArea::Unstaged, path.as_path())
                    .or_else(|| repo.status_entry_for_path(DiffArea::Staged, path.as_path()))
                    .map(|status| status.kind)
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
        let ui_scale = super::popover_ui_scale(cx);
        let width = super::popover_width_spec(&kind).unwrap_or(super::DEFAULT_CONTEXT_MENU_WIDTH);
        let model = self
            .context_menu_model(&kind, cx)
            .unwrap_or_else(|| ContextMenuModel::new(vec![]));
        let model_for_keys = model.clone();
        let model_for_mouse = model.clone();
        let tooltip_host = self.tooltip_host.clone();
        let entry_tooltips = model.entry_tooltips.clone();
        let entry_debug_selectors = model.entry_debug_selectors.clone();
        let shortcut_keycaps = model.shortcut_keycaps;

        let focus = self.context_menu_focus_handle.clone();
        // No fallback highlight: the menu opens with nothing selected (like
        // native menus), and hovering a disabled entry parks the selection on
        // it, which renders as no highlight at all rather than jumping to the
        // first selectable row.
        let current_selected = self.context_menu_selected_ix;
        let selected_for_render = current_selected.filter(|&ix| model.is_selectable(ix));

        // Keep labels aligned across entries when only some of them (e.g. the
        // checked option) carry an icon; icon-less menus stay compact.
        let reserve_icon_column = model
            .items
            .iter()
            .any(|item| matches!(item, ContextMenuItem::Entry { icon: Some(_), .. }));

        div()
            .flex()
            .flex_col()
            .items_stretch()
            .text_color(theme.colors.foreground.primary)
            .min_w(width.min_px(ui_scale))
            .max_w(width.max_px(ui_scale))
            .track_focus(&focus)
            .key_context("ContextMenu")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    window.focus(&this.context_menu_focus_handle, cx);
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
                            cx.stop_propagation();
                            this.close_popover_and_restore_focus(window, cx);
                        }
                        "up" => {
                            cx.stop_propagation();
                            let next =
                                model_for_keys.next_selectable(this.context_menu_selected_ix, -1);
                            this.context_menu_selected_ix = next;
                            cx.notify();
                        }
                        "down" => {
                            cx.stop_propagation();
                            let next =
                                model_for_keys.next_selectable(this.context_menu_selected_ix, 1);
                            this.context_menu_selected_ix = next;
                            cx.notify();
                        }
                        "tab" => {
                            cx.stop_propagation();
                            let direction = if mods.shift { -1 } else { 1 };
                            this.context_menu_selected_ix = model_for_keys
                                .next_selectable(this.context_menu_selected_ix, direction);
                            cx.notify();
                        }
                        "home" => {
                            cx.stop_propagation();
                            this.context_menu_selected_ix = model_for_keys.first_selectable();
                            cx.notify();
                        }
                        "end" => {
                            cx.stop_propagation();
                            this.context_menu_selected_ix = model_for_keys.last_selectable();
                            cx.notify();
                        }
                        "enter" | "space" => {
                            let Some(ix) = context_menu_activate_entry_ix(
                                &model_for_keys,
                                this.context_menu_selected_ix,
                            ) else {
                                return;
                            };
                            cx.stop_propagation();
                            this.context_menu_activate_model_entry(&model_for_keys, ix, window, cx);
                        }
                        _ => {
                            if let Some(ix) = context_menu_shortcut_entry_ix(&model_for_keys, key) {
                                cx.stop_propagation();
                                this.context_menu_activate_model_entry(
                                    &model_for_keys,
                                    ix,
                                    window,
                                    cx,
                                );
                            }
                        }
                    }
                }),
            )
            .children(model.items.into_iter().enumerate().map(move |(ix, item)| {
                match item {
                    ContextMenuItem::Separator => {
                        components::context_menu_separator(theme, ui_scale)
                            .id(("context_menu_sep", ix))
                            .into_any_element()
                    }
                    ContextMenuItem::Header(title) => components::context_menu_header(
                        theme,
                        ui_scale,
                        title,
                        Some(tooltip_host.clone()),
                        cx,
                    )
                    .id(("context_menu_header", ix))
                    .into_any_element(),
                    ContextMenuItem::Description(text) => components::context_menu_description(
                        theme,
                        ui_scale,
                        text,
                        Some(tooltip_host.clone()),
                        cx,
                    )
                    .id(("context_menu_description", ix))
                    .into_any_element(),
                    ContextMenuItem::Label(text) => components::context_menu_label(
                        theme,
                        ui_scale,
                        text,
                        Some(tooltip_host.clone()),
                        cx,
                    )
                    .id(("context_menu_label", ix))
                    .into_any_element(),
                    ContextMenuItem::Segmented { label, segments } => {
                        // Same construction as the toolbar's Inline/Split style
                        // toggles: one bordered pill, dividers between segments,
                        // the active one filled.
                        let mut control = div()
                            .id(("context_menu_segmented", ix))
                            .flex()
                            .items_center()
                            .h(components::control_height(ui_scale))
                            .rounded(px(theme.radii.row))
                            .border_1()
                            .border_color(theme.colors.stroke.default)
                            .overflow_hidden()
                            .p(px(1.0));
                        for (seg_ix, segment) in segments.into_iter().enumerate() {
                            if seg_ix > 0 {
                                control = control.child(
                                    div().h_full().w(px(1.0)).bg(theme.colors.stroke.default),
                                );
                            }
                            let ContextMenuSegment {
                                id,
                                label,
                                tooltip,
                                selected,
                                action,
                            } = segment;
                            let debug_selector = id.clone();
                            let mut button = components::Button::new(id, label)
                                .borderless()
                                .style(components::ButtonStyle::Subtle)
                                .selected(selected)
                                .selected_bg(theme.colors.interaction.pressed_background)
                                .on_click(theme, cx, move |this, _e, window, cx| {
                                    this.context_menu_activate_action(action.clone(), window, cx);
                                })
                                .debug_selector(move || debug_selector.to_string());
                            if let Some(tooltip) = tooltip {
                                button = button.gitcomet_tooltip(theme, tooltip);
                            }
                            control = control.child(button);
                        }
                        components::context_menu_label(
                            theme,
                            ui_scale,
                            label,
                            Some(tooltip_host.clone()),
                            cx,
                        )
                        .id(("context_menu_segmented_row", ix))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(control)
                        .into_any_element()
                    }
                    ContextMenuItem::Entry {
                        label,
                        icon,
                        shortcut,
                        disabled,
                        action,
                    } => {
                        let selected = selected_for_render == Some(ix);
                        let debug_selector = entry_debug_selectors
                            .get(&ix)
                            .map(|selector| selector.to_string())
                            .unwrap_or_else(|| context_menu_entry_debug_selector(label.as_ref()));
                        let tooltip_text = entry_tooltips
                            .get(&ix)
                            .cloned()
                            .or_else(|| context_menu_entry_tooltip(action.as_ref()));
                        let tooltip_host_for_move = tooltip_host.clone();
                        let tooltip_text_for_move = tooltip_text.clone();
                        let tooltip_host_for_hover = tooltip_host.clone();
                        let activate_on_left_release = model_for_mouse.clone();
                        let activate_on_right_release = model_for_mouse.clone();
                        let icon_slot = match icon {
                            Some(icon) => components::ContextMenuIconSlot::Icon(icon),
                            None if reserve_icon_column => {
                                components::ContextMenuIconSlot::Reserved
                            }
                            None => components::ContextMenuIconSlot::None,
                        };
                        let row =
                            components::ContextMenuEntry::new(("context_menu_entry", ix), label)
                                .icon(icon_slot)
                                .shortcut(shortcut)
                                .shortcut_keycaps(shortcut_keycaps)
                                .selected(selected)
                                .disabled(disabled)
                                .tooltip_host(tooltip_host.clone())
                                .render(theme, ui_scale, cx)
                                .debug_selector(move || debug_selector.clone());

                        row.on_mouse_move(cx.listener(
                            move |this, event: &MouseMoveEvent, _w, cx| {
                                this.context_menu_selected_ix = Some(ix);
                                if let Some(tooltip_text) = tooltip_text_for_move.as_ref() {
                                    let _ = tooltip_host_for_move.update(cx, |host, cx| {
                                        host.on_mouse_moved(event.position, cx);
                                        host.set_tooltip_text_if_changed(
                                            Some(tooltip_text.clone()),
                                            cx,
                                        );
                                    });
                                }
                                cx.notify();
                            },
                        ))
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            if *hovering {
                                this.context_menu_selected_ix = Some(ix);
                                cx.notify();
                            } else if let Some(tooltip_text) = tooltip_text.as_ref() {
                                let _ = tooltip_host_for_hover.update(cx, |host, cx| {
                                    host.clear_tooltip_if_matches(tooltip_text, cx);
                                });
                            }
                        }))
                        .when(!disabled, |row| {
                            row.on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, _e: &MouseUpEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.context_menu_activate_model_entry(
                                        &activate_on_left_release,
                                        ix,
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Right,
                                cx.listener(move |this, _e: &MouseUpEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.context_menu_activate_model_entry(
                                        &activate_on_right_release,
                                        ix,
                                        window,
                                        cx,
                                    );
                                }),
                            )
                        })
                        .into_any_element()
                    }
                }
            }))
    }
}

fn interactive_rebase_action_menu_model(
    ix: usize,
    can_squash: bool,
    can_drop: bool,
    pick_locked: bool,
) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Entry {
            label: "pick".into(),
            icon: None,
            shortcut: None,
            // A squash run's target is auto-managed: it stays Reword while
            // commits squash into it, and a dropped entry in target position
            // would re-promote the instant it were picked back — so `pick`
            // locks for the position rather than turning into a surprise
            // reword. Demote a target by dropping it or removing the squash.
            disabled: pick_locked,
            action: Box::new(ContextMenuAction::SetInteractiveRebaseAction {
                ix,
                action: InteractiveRebaseAction::Pick,
            }),
        },
        ContextMenuItem::Entry {
            label: "reword".into(),
            icon: None,
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetInteractiveRebaseAction {
                ix,
                action: InteractiveRebaseAction::Reword,
            }),
        },
        ContextMenuItem::Entry {
            label: "drop".into(),
            icon: None,
            shortcut: None,
            disabled: !can_drop,
            action: Box::new(ContextMenuAction::SetInteractiveRebaseAction {
                ix,
                action: InteractiveRebaseAction::Drop,
            }),
        },
    ];
    if can_squash {
        items.push(ContextMenuItem::Entry {
            label: "squash".into(),
            icon: None,
            shortcut: None,
            disabled: !can_squash,
            action: Box::new(ContextMenuAction::SetInteractiveRebaseAction {
                ix,
                action: InteractiveRebaseAction::Squash,
            }),
        });
    }
    ContextMenuModel::new(items)
}

fn interactive_rebase_autosquash_menu_model() -> ContextMenuModel {
    // Auto Squash is a one-shot action: pick a strategy and it folds the
    // duplicate-message commits, no persisted on/off state to display.
    let entry = |mode: AutosquashMode| ContextMenuItem::Entry {
        label: mode.label().into(),
        icon: None,
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::SetInteractiveRebaseAutosquashMode { mode }),
    };
    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Auto Squash".into()),
        entry(AutosquashMode::ToTop),
        entry(AutosquashMode::Neighbor),
        entry(AutosquashMode::ToBottom),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::view::shortcut_labels::secondary_shortcut;

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
                shortcut: Some(secondary_shortcut("C").into()),
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
        assert_eq!(context_menu_shortcut_entry_ix(&model, "c"), Some(3));
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

    #[test]
    fn use_commit_message_action_exposes_full_message_tooltip() {
        let tooltip = context_menu_entry_tooltip(&ContextMenuAction::UseCommitMessage {
            message: "\n\nsubject\n\nbody".to_string(),
        });

        assert_eq!(
            tooltip.as_ref().map(|text| text.as_ref()),
            Some("subject\n\nbody")
        );
    }

    #[test]
    fn non_commit_message_actions_do_not_expose_entry_tooltips() {
        assert!(
            context_menu_entry_tooltip(&ContextMenuAction::FetchAll { repo_id: RepoId(1) })
                .is_none()
        );
    }
}
