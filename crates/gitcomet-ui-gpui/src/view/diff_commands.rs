use super::panes::diff::services::{
    DiffCommand, DiffCommands, DiffNavigation, DiffPopoverAnchor, DiffPreference,
};
use super::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub(super) fn for_window(root: WeakEntity<GitCometView>) -> DiffCommands {
    let navigation = Rc::new(WindowNavigation(root.clone()));
    DiffCommands {
        navigation: Some(navigation),
        dispatch: Some(Rc::new(move |command, cx| {
            if let DiffCommand::OpenHostedMenu {
                id,
                items,
                anchor,
                window,
            } = command
            {
                let _ = window.update(cx, |_, window, cx| {
                    let _ = root.update(cx, |root, cx| {
                        root.popover_host.update(cx, |host, cx| {
                            host.open_hosted_menu(id, items, anchor, None, window, cx)
                        });
                    });
                });
                return;
            }
            if let DiffCommand::OpenPopover {
                request,
                anchor,
                pane,
                window,
            } = command
            {
                let request = *request;
                let _ = window.update(cx, |_, window, cx| {
                    let _ = root.update(cx, |root, cx| {
                        if let Some(pane) = pane {
                            root.popover_host.update(cx, |host, cx| {
                                host.bind_diff_pane(pane, cx);
                                match anchor {
                                    DiffPopoverAnchor::Point(point) => {
                                        host.open_popover_at(request, point, window, cx)
                                    }
                                    DiffPopoverAnchor::Bounds(bounds) => {
                                        host.open_popover_for_bounds(request, bounds, window, cx)
                                    }
                                }
                            });
                            cx.notify();
                        } else {
                            match anchor {
                                DiffPopoverAnchor::Point(point) => {
                                    root.open_popover_at(request, point, window, cx)
                                }
                                DiffPopoverAnchor::Bounds(bounds) => {
                                    root.open_popover_for_bounds(request, bounds, window, cx)
                                }
                            }
                        }
                    });
                });
                return;
            }
            if let DiffCommand::OpenFileHistory { repo, path } = command {
                // The sidebar lives in the main window, not in a pop-out window.
                let Ok(root_window) = root.read_with(cx, |root, _| root.window_handle) else {
                    return;
                };
                let _ = root_window.update(cx, |_, window, cx| {
                    let _ = root.update(cx, |root, cx| {
                        root.open_file_history(repo, path, window, cx)
                    });
                });
                return;
            }
            let _ = root.update(cx, |root, cx| match command {
                DiffCommand::PersistSettings => root.schedule_ui_settings_persist(cx),
                DiffCommand::Preference(preference) => match preference {
                    DiffPreference::Content(value) => {
                        root.sync_diff_content_mode_from_pane(value, cx)
                    }
                    DiffPreference::Whitespace(value) => {
                        root.sync_diff_whitespace_mode_from_pane(value, cx)
                    }
                    DiffPreference::RevealWhitespace(value) => {
                        root.sync_diff_reveal_whitespace_chars_from_pane(value, cx)
                    }
                    DiffPreference::Wrap(value) => root.sync_diff_word_wrap_from_pane(value, cx),
                    DiffPreference::LineNumbers(value) => {
                        root.sync_diff_show_line_numbers_from_pane(value, cx)
                    }
                    DiffPreference::Layout(value) => root.set_diff_view_mode(value, cx),
                    DiffPreference::Annotate(value) => root.set_annotate_enabled(value, cx),
                },
                DiffCommand::Error(report) => root.report_error(report, cx),
                DiffCommand::Toast(kind, message) => root.push_toast(kind, message.to_string(), cx),
                DiffCommand::OpenExternal(path) => root.open_path_in_external_code_editor(path, cx),
                DiffCommand::OpenSubmodule { path, target } => {
                    let bootstrap = SubmoduleDiffBootstrap::new(path, target);
                    if let gitcomet_extension_api::GateDecision::Deny { reason } =
                        super::extension_host::entry_decision(
                            &bootstrap.repo_path,
                            gitcomet_extension_api::EntryOrigin::Chooser,
                            cx,
                        )
                    {
                        root.show_repository_entry_denial(reason, cx);
                        return;
                    }
                    root.submodule_diff_bootstrap = Some(bootstrap);
                    root.drive_submodule_diff_bootstrap();
                    cx.notify();
                }
                DiffCommand::ClearStatusSelection { repo, multi_only } => {
                    root.details_pane.update(cx, |pane, cx| {
                        if multi_only {
                            pane.status_multi_selection.remove(&repo);
                        } else {
                            pane.clear_status_multi_selection(repo);
                        }
                        cx.notify();
                    })
                }
                DiffCommand::RevealStatus { section, path } => {
                    root.details_pane.update(cx, |pane, cx| {
                        let Some(position) = pane.status_path_display_position(section, &path)
                        else {
                            return;
                        };
                        let index = pane
                            .reveal_status_row(section, position, cx)
                            .unwrap_or(position);
                        let scroll = match section {
                            StatusSection::CombinedUnstaged | StatusSection::Unstaged => {
                                &pane.unstaged_scroll
                            }
                            StatusSection::Untracked => &pane.untracked_scroll,
                            StatusSection::Staged => &pane.staged_scroll,
                        };
                        scroll.scroll_to_item_strict(index, gpui::ScrollStrategy::Center);
                        cx.notify();
                    })
                }
                DiffCommand::RevealCommitFile { position, range } => {
                    root.details_pane.update(cx, |pane, cx| {
                        if range {
                            let row = pane.reveal_range_file_row(position, cx).unwrap_or(position);
                            pane.range_files_scroll
                                .scroll_to_item_strict(row, gpui::ScrollStrategy::Center);
                        } else {
                            let row = pane
                                .reveal_commit_file_row(position, cx)
                                .unwrap_or(position);
                            pane.commit_files_scroll
                                .scroll_to_item_strict(row, gpui::ScrollStrategy::Center);
                        }
                        cx.notify();
                    })
                }
                DiffCommand::OpenPopover { .. }
                | DiffCommand::OpenHostedMenu { .. }
                | DiffCommand::OpenFileHistory { .. } => unreachable!(),
            });
        })),
    }
}

struct WindowNavigation(WeakEntity<GitCometView>);

impl DiffNavigation for WindowNavigation {
    fn layout(&self, cx: &App) -> Option<(Pixels, Pixels, bool, bool)> {
        self.0
            .read_with(cx, |root, _| {
                (
                    root.sidebar_render_width,
                    root.details_render_width,
                    root.sidebar_collapsed,
                    root.details_collapsed,
                )
            })
            .ok()
    }
    fn selected_paths(
        &self,
        repo: RepoId,
        area: DiffArea,
        path: &Path,
        cx: &App,
    ) -> Option<Vec<PathBuf>> {
        self.0
            .read_with(cx, |root, cx| {
                let (paths, selected) = root
                    .details_pane
                    .read(cx)
                    .status_selected_paths_for_action(repo, area, &path.to_path_buf());
                selected.then_some(paths)
            })
            .ok()
            .flatten()
    }
    fn selected_single(&self, repo: RepoId, area: DiffArea, path: &Path, cx: &App) -> bool {
        self.0
            .read_with(cx, |root, cx| {
                root.details_pane
                    .read(cx)
                    .status_selected_paths_for_area(repo, area)
                    == [path]
            })
            .unwrap_or(false)
    }
    fn change_tracking(&self, cx: &App) -> ChangeTrackingView {
        self.0
            .read_with(cx, |root, _| root.change_tracking_view)
            .unwrap_or(ChangeTrackingView::Combined)
    }
    fn status_order(&self, repo: RepoId, section: StatusSection, cx: &App) -> Option<Arc<[usize]>> {
        self.0
            .read_with(cx, |root, cx| {
                root.details_pane
                    .read(cx)
                    .active_status_section_order(repo, section)
            })
            .ok()
            .flatten()
    }
    fn status_position(
        &self,
        repo: &RepoState,
        path: &Path,
        area: DiffArea,
        view: ChangeTrackingView,
        cx: &App,
    ) -> Option<Option<(StatusSection, usize, usize)>> {
        self.0
            .read_with(cx, |root, cx| {
                root.details_pane
                    .read(cx)
                    .status_footer_position(repo, path, area, view)
            })
            .ok()
            .flatten()
    }
    fn file_order(&self, repo: RepoId, range: bool, cx: &App) -> Option<Arc<[usize]>> {
        self.0
            .read_with(cx, |root, cx| {
                let details = root.details_pane.read(cx);
                if range {
                    details.active_range_file_source_indices(repo)
                } else {
                    details.active_commit_file_source_indices(repo)
                }
            })
            .ok()
            .flatten()
    }
    fn worktree_order(&self, repo: RepoId, path: &Path, cx: &App) -> Option<Arc<[usize]>> {
        self.0
            .read_with(cx, |root, cx| {
                root.details_pane
                    .read(cx)
                    .active_worktree_file_source_indices(repo, path)
            })
            .ok()
            .flatten()
    }
    fn page_rows(
        &self,
        repo: RepoId,
        list: crate::view::rows::FileListId,
        cx: &App,
    ) -> Option<usize> {
        self.0
            .read_with(cx, |root, cx| {
                root.details_pane.read(cx).file_list_page_rows(repo, list)
            })
            .ok()
            .flatten()
    }
}
