//! The file browser tab: its search bar, visible rows (including unsaved edits) and row rendering.

use super::{
    AnyElement, AppTheme, Arc, CollapsedSidebarSection, ElementId,
    FILE_BROWSER_UNSAVED_SECTION_KEY, FileBrowserVisibleRow, FileEntry, FileEntryKind, FontWeight,
    FxHashSet, Loadable, MouseButton, Msg, PathBuf, PopoverKind, Range, Rc, RepoState,
    SharedString, SidebarPaneView, UnsavedFileRowCtx, Window, branch_sidebar, components, controls,
    div, explorer_operations, file_browser_row_label_color, file_browser_search_is_active,
    file_icons, file_search_highlight_ranges, file_search_matches, px,
    restrict_scroll_to_vertical_axis, selected_branch_row_bg, sidebar_list_row_height, ui_scale,
    uniform_list, unsaved_file_row, with_alpha,
};
use crate::kit::click::PointerClickExt;
use crate::kit::components::ControlInteractionExt;
use crate::kit::components::InteractiveRowExt;
use gpui::prelude::*;
use palette::IntoColor;

impl SidebarPaneView {
    pub(super) fn render_file_browser_search_bar(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        self.render_sidebar_search(true, theme, cx)
    }

    pub(super) fn render_file_browser_content(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = ui_scale::scaler(ui_scale_percent);
        let search_bar = self
            .file_search_open
            .then(|| self.render_file_browser_search_bar(theme, cx));

        let visible_rows = self.file_browser_visible_rows(cx);

        let body: AnyElement = if visible_rows.is_empty() {
            let repo = self.active_repo();
            let message = match repo {
                None => "No repository selected.",
                Some(r) => match &r.file_browser.entries {
                    Loadable::NotLoaded => "Loading files...",
                    Loadable::Loading => "Loading files...",
                    Loadable::Ready(entries) if entries.is_empty() => "Empty repository.",
                    Loadable::Ready(_) => "No files visible.",
                    Loadable::Error(_) => "Error loading files.",
                },
            };
            self.explorer_empty_state(theme, message)
        } else {
            let row_count = visible_rows.len();
            let list = uniform_list(
                "file_browser",
                row_count,
                cx.processor(Self::render_file_browser_rows),
            )
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.file_browser_scroll);
            let list = restrict_scroll_to_vertical_axis(list);
            // Same overlay-scrollbar treatment as the branches list above.
            let list = div()
                .flex_1()
                .min_h(px(0.0))
                .pt(scaled_px(2.0))
                .pl(scaled_px(components::ROW_HIGHLIGHT_INSET_PX))
                .pr(scaled_px(components::ROW_HIGHLIGHT_INSET_PX))
                .child(list);
            div()
                .id("file_browser_scroll_container")
                .debug_selector(|| "file_browser_scroll_container".to_string())
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .h_full()
                .child(list.into_any_element())
                .child(
                    components::Scrollbar::new(
                        "file_browser_scrollbar",
                        self.file_browser_scroll.clone(),
                    )
                    .auto_hide()
                    .render(theme),
                )
                .into_any_element()
        };

        let browsing_commit = self
            .active_repo()
            .is_some_and(|r| r.browsing_commit().is_some());
        let accepts_drops = self.active_repo().is_some_and(|repo| {
            repo.file_browser.source == gitcomet_core::domain::FileSource::WorkingDirectory
        });
        div()
            .id("explorer_focus_scope")
            .on_activate(
                false,
                controls::ControlActivation::Composite,
                cx.listener(|this, _, window, cx| this.explorer_background_click(window, cx)),
            )
            .track_focus(&self.explorer_focus)
            .capture_key_down(cx.listener(Self::explorer_key_down))
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    if let Some(repo_id) = this.active_repo_id() {
                        window.focus(&this.explorer_focus, cx);
                        this.open_popover_at(
                            PopoverKind::FileBrowserFolderMenu {
                                repo_id,
                                path: PathBuf::new(),
                            },
                            event.position,
                            window,
                            cx,
                        );
                    }
                    cx.stop_propagation();
                }),
            )
            // Drops are only meaningful against the working tree; while a
            // commit is being browsed there is nothing to write to.
            .when(accepts_drops, |scope| {
                scope
                    // One autoscroll driver for the whole tree rather than one
                    // per row.
                    .on_drag_move(cx.listener(
                        |this,
                         event: &gpui::DragMoveEvent<explorer_operations::ExplorerDrag>,
                         window,
                         cx| {
                            this.explorer_drag_move(event.event.position, window, cx)
                        },
                    ))
                    .on_drag_move(cx.listener(
                        |this, event: &gpui::DragMoveEvent<gpui::ExternalPaths>, window, cx| {
                            this.explorer_drag_move(event.event.position, window, cx)
                        },
                    ))
                    .on_mouse_exit(cx.listener(|this, _: &gpui::MouseExitEvent, _window, cx| {
                        this.clear_explorer_drag_state(cx)
                    }))
                    // Resolve drops from the same geometry as hover feedback;
                    // controls, pinned buffers and scrollbars are excluded.
                    .on_drop(
                        cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                            let Some(target) = this.explorer_pointer_target(window, cx) else {
                                return;
                            };
                            this.explorer_drop(
                                paths.paths().to_vec(),
                                Some(target),
                                true,
                                window,
                                cx,
                            )
                        }),
                    )
                    .on_drop(cx.listener(
                        |this, drag: &explorer_operations::ExplorerDrag, window, cx| {
                            let Some(target) = this.explorer_pointer_target(window, cx) else {
                                return;
                            };
                            this.explorer_drop(drag.paths.to_vec(), Some(target), false, window, cx)
                        },
                    ))
            })
            .relative()
            .flex()
            .flex_col()
            .h_full()
            .min_h(px(0.0))
            // A wash rather than a frame: browse mode should read as a change of
            // surface, not as a box drawn around the list.
            .when(browsing_commit, |d| {
                // Over `surface.chrome`: that is what the pane behind this paints, so
                // browse mode shifts the hue without also stepping the lightness.
                d.bg(crate::theme::historical_surface_bg(
                    theme,
                    theme.colors.surface.chrome,
                ))
            })
            .children(search_bar)
            .child(body)
            .into_any()
    }

    /// Shared with the cache: the tree rows are read several times per frame
    /// and were deep-copied on every read, hit or miss.
    pub(super) fn file_browser_visible_rows(&self, cx: &gpui::App) -> Rc<[FileBrowserVisibleRow]> {
        let Some(repo) = self.active_repo() else {
            return Rc::from(Vec::new());
        };
        if let Some(edit) = &self.explorer_name_edit
            && edit.repo_id == repo.id
        {
            let mut rows =
                self.compute_file_browser_visible_rows(repo, self.unsaved_file_edit_paths(cx));
            let relative = edit
                .path
                .strip_prefix(&repo.spec.workdir)
                .unwrap_or(&edit.path);
            let ix = if let Loadable::Ready(entries) = &repo.file_browser.entries {
                rows.iter().position(|row| {
                    row.entry_index()
                        .is_some_and(|ix| entries[ix].path.as_path() == relative)
                })
            } else {
                None
            };
            if edit.action == explorer_operations::ExplorerAction::Rename {
                if let Some(ix) = ix {
                    rows[ix] = FileBrowserVisibleRow::NameEntry {
                        depth: relative.components().count().saturating_sub(1),
                    };
                }
            } else {
                rows.insert(
                    ix.map_or_else(
                        || {
                            rows.iter()
                                .position(|row| row.entry_index().is_some())
                                .unwrap_or(rows.len())
                        },
                        |i| i + 1,
                    ),
                    FileBrowserVisibleRow::NameEntry {
                        depth: relative.components().count(),
                    },
                );
            }
            return rows.into();
        }

        // Key on the repo id too: file_browser_rev is a per-repo counter, so two
        // repos can share a value and collide otherwise (stale rows for the wrong
        // tree after switching repos).
        //
        // The unsaved revision has to be in here as well: those buffers live in
        // the main pane, so nothing the store owns — `file_browser_rev`
        // included — moves when one goes dirty, and the cache would keep serving
        // rows without the pinned section.
        let cache_key = (
            repo.id,
            repo.file_browser.file_browser_rev,
            self.file_search_options,
            self.unsaved_file_edits_rev(cx),
            self.unsaved_section_is_collapsed(),
        );
        let mut cache = self.file_browser_rows_cache.borrow_mut();
        if let Some((cached_key, cached_rows)) = cache.as_ref()
            && *cached_key == cache_key
        {
            return Rc::clone(cached_rows);
        }

        let rows: Rc<[FileBrowserVisibleRow]> = self
            .compute_file_browser_visible_rows(repo, self.unsaved_file_edit_paths(cx))
            .into();
        *cache = Some((cache_key, Rc::clone(&rows)));
        rows
    }

    pub(super) fn unsaved_file_edit_paths(&self, _: &gpui::App) -> Vec<PathBuf> {
        let Some(repo) = self.active_repo() else {
            return Vec::new();
        };
        let mut paths: Vec<_> = self
            .documents
            .dirty_paths(repo.id)
            .into_iter()
            .filter_map(|path| {
                path.strip_prefix(&repo.spec.workdir)
                    .ok()
                    .map(std::path::Path::to_path_buf)
            })
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    pub(super) fn unsaved_file_edits_rev(&self, cx: &gpui::App) -> u64 {
        self.documents.revision(cx)
    }

    /// Collapsed state rides in the same per-repo map the branch tree's sections
    /// use, so it persists across sessions the way those do.
    pub(super) fn unsaved_section_is_collapsed(&self) -> bool {
        self.active_repo().is_some_and(|repo| {
            self.sidebar_collapsed_items_by_repo
                .get(&repo.spec.workdir)
                .is_some_and(|items| {
                    branch_sidebar::is_collapsed(items, FILE_BROWSER_UNSAVED_SECTION_KEY)
                })
        })
    }

    pub(super) fn compute_file_browser_visible_rows(
        &self,
        repo: &RepoState,
        unsaved: Vec<PathBuf>,
    ) -> Vec<FileBrowserVisibleRow> {
        let Loadable::Ready(entries) = &repo.file_browser.entries else {
            // Still worth showing the pinned section: the buffers exist whether
            // or not the tree behind them has loaded.
            return self.unsaved_file_edit_rows(unsaved);
        };

        let matchers = self.cached_file_matchers();
        let has_search = !matchers.is_empty();

        let mut tree_rows: Vec<FileBrowserVisibleRow> = if has_search {
            let mut matching_entry_indices = FxHashSet::default();
            let mut ancestor_paths = FxHashSet::default();

            for (i, entry) in entries.iter().enumerate() {
                let path_str = entry.path.to_string_lossy();
                if file_search_matches(&matchers, path_str.as_ref()) {
                    matching_entry_indices.insert(i);
                    let mut parent = entry.path.parent();
                    while let Some(p) = parent {
                        if !p.as_os_str().is_empty() {
                            ancestor_paths.insert(Arc::new(p.to_path_buf()));
                        }
                        parent = p.parent();
                    }
                }
            }

            entries
                .iter()
                .enumerate()
                .filter(|(i, entry)| {
                    matching_entry_indices.contains(i) || ancestor_paths.contains(&entry.path)
                })
                .map(|(i, entry)| {
                    let is_expanded = match entry.kind {
                        FileEntryKind::Directory => true,
                        FileEntryKind::File => false,
                    };
                    FileBrowserVisibleRow::Entry {
                        entry_index: i,
                        depth: entry.depth,
                        is_directory: entry.kind == FileEntryKind::Directory,
                        is_expanded,
                    }
                })
                .collect()
        } else {
            let visible_mask = self.file_browser_visible_mask(entries);

            entries
                .iter()
                .enumerate()
                .filter(|(i, _)| visible_mask.contains(i))
                .map(|(i, entry)| {
                    let is_expanded = entry.kind == FileEntryKind::Directory
                        && repo.file_browser.expanded_dirs.contains(&entry.path);
                    FileBrowserVisibleRow::Entry {
                        entry_index: i,
                        depth: entry.depth,
                        is_directory: entry.kind == FileEntryKind::Directory,
                        is_expanded,
                    }
                })
                .collect()
        };

        let mut rows = self.unsaved_file_edit_rows(unsaved);
        tree_rows.retain(|row| {
            row.entry_index().is_none_or(|ix| {
                let path = &entries[ix].path;
                !path
                    .components()
                    .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
                    && (repo.file_browser.show_hidden
                        || repo
                            .file_browser
                            .revealed_paths
                            .iter()
                            .any(|revealed| revealed.starts_with(path.as_ref()))
                        || !gitcomet_state::explorer::is_hidden_path(path))
            })
        });
        rows.append(&mut tree_rows);
        rows
    }

    /// The pinned section's rows: a header, then one row per unsaved file.
    ///
    /// Empty when nothing is unsaved, so the section costs no vertical space in
    /// the common case, and header-only when it is collapsed.
    pub(super) fn unsaved_file_edit_rows(
        &self,
        mut unsaved: Vec<PathBuf>,
    ) -> Vec<FileBrowserVisibleRow> {
        let matchers = self.cached_file_matchers();
        unsaved.retain(|path| file_search_matches(&matchers, &path.to_string_lossy()));
        if unsaved.is_empty() {
            return Vec::new();
        }
        let Some(repo_id) = self.active_repo_id() else {
            return Vec::new();
        };
        let weak_store = Arc::downgrade(&self.store);
        let files = gitcomet_extension_api::SidebarFileSet::new(unsaved, move |path, _| {
            if let Some(store) = weak_store.upgrade() {
                store.dispatch(Msg::OpenFileEditor {
                    repo_id,
                    path: path.clone(),
                });
            }
        });
        let mut rows = vec![FileBrowserVisibleRow::FileSetHeader {
            count: files.paths.len(),
        }];
        if !self.unsaved_section_is_collapsed() {
            rows.extend(files.paths.iter().zip(files.rows()).map(|(path, row)| {
                FileBrowserVisibleRow::FileSetFile {
                    path: Arc::new(path.clone()),
                    open: row.action,
                }
            }));
        }
        rows
    }

    pub(super) fn file_browser_visible_mask(&self, entries: &[FileEntry]) -> FxHashSet<usize> {
        let Some(repo) = self.active_repo() else {
            return FxHashSet::default();
        };
        let expanded = &repo.file_browser.expanded_dirs;

        let mut visible = FxHashSet::default();
        let mut skip_until_sibling: Option<(usize, usize)> = None;

        for (i, entry) in entries.iter().enumerate() {
            if let Some((skip_depth, sibling_end)) = skip_until_sibling {
                if i < sibling_end && entry.depth > skip_depth {
                    continue;
                }
                skip_until_sibling = None;
            }

            visible.insert(i);

            if entry.kind == FileEntryKind::Directory && !expanded.contains(&entry.path) {
                let skip_depth = entry.depth;
                let sibling_end = entries[i + 1..]
                    .iter()
                    .position(|e| e.depth <= skip_depth)
                    .map(|pos| i + 1 + pos)
                    .unwrap_or(entries.len());
                skip_until_sibling = Some((skip_depth, sibling_end));
            }
        }

        visible
    }

    pub(in super::super::super) fn render_file_browser_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        const INDENT_STEP_PX: f32 = 8.0;
        const CHEVRON_SLOT_PX: f32 = 12.0;
        const ICON_SLOT_PX: f32 = 16.0;

        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = ui_scale::scaler(ui_scale_percent);

        #[cfg(test)]
        {
            this.rendered_rows += range.len();
        }
        let Some(repo_id) = this.active_repo_id() else {
            return Vec::new();
        };
        let theme = this.theme;
        let row_height = sidebar_list_row_height(theme, ui_scale_percent);
        let icon_muted = with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.6 } else { 0.5 },
        );
        // Zed renders file/folder icons in a neutral, muted tone rather than a
        // bright accent — match that so the tree reads the same way.
        let icon_color = theme.colors.foreground.secondary;
        let row_surface = if matches!(
            this.collapsed_popover_section,
            Some(CollapsedSidebarSection::Files)
        ) {
            theme.colors.surface.raised
        } else {
            theme.colors.surface.chrome
        };
        // Match the Branches tree: both are continuous lists, and their
        // hover/selection fills should have the same square row silhouette.
        let row_style = components::InteractiveRowStyle::new(theme, row_surface).flat();
        // GPUI hides ordinary hover styles during a drag, but their listeners
        // still invalidate the sidebar when crossing rows. Drop highlighting
        // below supplies the feedback and only changes when the target changes.
        let row_style = if cx.has_active_drag() {
            row_style.without_hover()
        } else {
            row_style
        };
        let store = Arc::clone(&this.store);
        let search_matchers = this.cached_file_matchers();

        let visible_rows = this.file_browser_visible_rows(cx);
        let drop_region = this.explorer_drop_range(&visible_rows);
        let cut_paths = this.explorer_cut_paths(cx);
        // Every selected row drags the whole selection, so build it once rather
        // than joining and cloning it per row per frame.
        let selection_sources: Rc<[PathBuf]> = Rc::from(this.explorer_sources(None));
        let repo = this.active_repo();
        let status_badges =
            repo.and_then(|repo| this.file_browser_status_cache.borrow_mut().get(repo));
        // The file the main pane is showing, so the tree can mark it. Read
        // whatever the target names — a diff of a file is still "this file is
        // open", not only the read-only content view.
        let open_path = repo.and_then(|repo| repo.open_file_path().map(|p| p.to_path_buf()));
        // The same wash a selected branch row wears, so both trees in the
        // sidebar mark "this is the one you are looking at" identically.
        let open_row_bg = selected_branch_row_bg(theme);
        let entries = repo
            .and_then(|r| match &r.file_browser.entries {
                Loadable::Ready(e) => Some(e.as_slice()),
                _ => None,
            })
            .unwrap_or(&[]);
        // A filtered tree renders every directory expanded and never reads
        // `expanded_dirs`, so the reducer refuses the toggle. Drop the row's
        // chevron and its click with it — a control that cannot move is worse
        // than none, and the folder menu greys its Expand/Collapse entries for
        // exactly the same reason. Asked of the query rather than the matchers
        // so this tracks what the reducer will actually honour.
        let expansion_frozen =
            repo.is_some_and(|repo| file_browser_search_is_active(&repo.file_browser.search_query));

        let svg_icon = |path: &'static str, color: gpui::Rgba, size_px: f32| {
            crate::view::icons::svg_icon(path, color, scaled_px(size_px))
        };

        let svg_chevron =
            |expanded: bool| svg_icon(file_icons::chevron_icon(expanded), icon_muted, 10.0);

        let chevron_slot = |is_directory: bool, is_expanded: bool| {
            div()
                .w(scaled_px(CHEVRON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .when(is_directory, |d| d.child(svg_chevron(is_expanded)))
        };

        let file_or_folder_icon_path = |entry: &FileEntry, expanded: bool| -> &'static str {
            if entry.kind == FileEntryKind::Directory {
                file_icons::folder_icon(expanded)
            } else {
                file_icons::file_icon_for_path(&entry.path)
            }
        };

        let icon_slot = |path: &'static str| {
            let tint = file_icons::file_icon_color(path, theme.is_dark).unwrap_or(icon_color);
            div()
                .w(scaled_px(ICON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(path, tint, 12.0))
        };

        let icon_slot_tinted = |path: &'static str, tint: gpui::Rgba| {
            div()
                .w(scaled_px(ICON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(path, tint, 12.0))
        };

        // Looked up per row, so a set rather than the ordered vec the pinned
        // section is built from.
        let unsaved_paths: FxHashSet<PathBuf> =
            this.unsaved_file_edit_paths(cx).into_iter().collect();

        let unsaved_collapsed = this.unsaved_section_is_collapsed();

        range
            .filter_map(|ix| {
                let row = visible_rows.get(ix)?;
                let (entry_index, depth, is_directory, is_expanded) = match row {
                    // The pinned section shares the list with the tree but not
                    // its shape, so both rows are built here and return early.
                    FileBrowserVisibleRow::NameEntry { depth } => {
                        let icon = this.explorer_name_edit.as_ref()?.icon;
                        return this.explorer_name_entry(cx).map(|entry| {
                            div()
                                .debug_selector(|| "explorer_inline_row".into())
                                .flex()
                                .items_center()
                                .w_full()
                                .min_w(px(0.0))
                                .flex_none()
                                .h(row_height)
                                .pl(scaled_px(6.0 + INDENT_STEP_PX * *depth as f32))
                                .pr_2()
                                .gap(scaled_px(4.0))
                                .child(chevron_slot(false, false))
                                .child(icon_slot(icon))
                                .child(entry)
                                .into_any_element()
                        });
                    }
                    FileBrowserVisibleRow::FileSetHeader { count } => {
                        return Some(
                            div()
                                .id(ElementId::Name(format!("file_browser_row_{ix}").into()))
                                .debug_selector(|| "file_browser_unsaved_header".to_string())
                                .flex()
                                .flex_row()
                                .items_center()
                                .h(row_height)
                                .w_full()
                                .pl(scaled_px(6.0))
                                .pr_2()
                                .gap(scaled_px(4.0))
                                .interactive_row(
                                    row_style,
                                    components::InteractiveRowState::default(),
                                )
                                .on_activate(
                                    false,
                                    controls::ControlActivation::Composite,
                                    cx.listener(move |this, _e: &gpui::ClickEvent, _window, cx| {
                                        this.toggle_active_repo_collapse_key(
                                            SharedString::from(FILE_BROWSER_UNSAVED_SECTION_KEY),
                                            cx,
                                        );
                                    }),
                                )
                                .child(chevron_slot(true, !unsaved_collapsed))
                                .child(icon_slot_tinted(
                                    "icons/pencil.svg",
                                    theme.colors.status.warning.foreground,
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_size(theme.ui_text(12.0))
                                        .text_color(theme.colors.foreground.secondary)
                                        .child(format!("Unsaved edits ({count})")),
                                )
                                .into_any_element(),
                        );
                    }
                    FileBrowserVisibleRow::FileSetFile { path, open } => {
                        return Some(unsaved_file_row(
                            UnsavedFileRowCtx {
                                theme,
                                row_style,
                                open_row_bg,
                                repo_id,
                                ix,
                                is_open: open_path.as_deref() == Some(path.as_path()),
                            },
                            Arc::clone(path),
                            scaled_px(6.0 + INDENT_STEP_PX),
                            row_height,
                            scaled_px(ICON_SLOT_PX),
                            open.clone(),
                            cx,
                        ));
                    }
                    FileBrowserVisibleRow::Entry {
                        entry_index,
                        depth,
                        is_directory,
                        is_expanded,
                    } => (*entry_index, *depth, *is_directory, *is_expanded),
                };
                let entry = entries.get(entry_index)?;
                let element = {
                    let left_pad = scaled_px(6.0 + INDENT_STEP_PX * depth as f32);
                    let store = Arc::clone(&store);
                    // Files and folders get separate invoker names so the two
                    // menus can never light up each other's row.
                    let menu_invoker = SharedString::from(if is_directory {
                        format!("file_browser_folder_{ix}")
                    } else {
                        format!("file_browser_file_{ix}")
                    });
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&menu_invoker);
                    let is_open_file = !is_directory
                        && open_path
                            .as_ref()
                            .is_some_and(|open| open.as_path() == entry.path.as_path());
                    let selected = repo.is_some_and(|r| {
                        r.file_browser.selection.paths.contains(entry.path.as_ref())
                    });
                    let focused = repo.is_some_and(|r| {
                        r.file_browser.selection.focused.as_ref() == Some(entry.path.as_ref())
                    });
                    let cut = cut_paths.as_ref().is_some_and(|paths| {
                        repo.is_some_and(|r| {
                            paths
                                .iter()
                                .any(|p| r.spec.workdir.join(entry.path.as_ref()).starts_with(p))
                        })
                    });
                    // A cut row dims even when selected: the selection is usually
                    // what was just cut, and that is the feedback the user needs.
                    let row_text_color = if cut {
                        theme.colors.foreground.disabled
                    } else if entry.ignored && !selected && !is_open_file {
                        theme.colors.foreground.secondary
                    } else {
                        file_browser_row_label_color(theme, selected || is_open_file)
                    };
                    let row_style = row_style.tinted(crate::view::rows::explorer_row_tint(
                        &theme,
                        gitcomet_state::explorer::is_hidden_path(entry.path.as_path()),
                        entry.ignored,
                    ));
                    // The pen marks the file wherever it sits in the tree, so a user
                    // who navigated to it rather than to the pinned section still
                    // sees that it is holding unsaved text.
                    let has_unsaved_edits =
                        !is_directory && unsaved_paths.contains(entry.path.as_ref());
                    let status = status_badges
                        .as_ref()
                        .and_then(|badges| badges.get(&entry.path, is_directory));
                    let row_state = components::InteractiveRowState::default()
                        .selected(
                            selected
                                || (is_open_file
                                    && repo.is_some_and(|r| {
                                        r.file_browser.selection.paths.is_empty()
                                    })),
                            row_style.selection_fill(open_row_bg),
                        )
                        .open(context_menu_active);

                    let mut row_div = div()
                        .id(ElementId::Name(format!("file_browser_row_{ix}").into()))
                        .debug_selector(move || format!("file_browser_row_{ix}"))
                        .flex()
                        .flex_row()
                        .items_center()
                        .h(row_height)
                        .w_full()
                        .pl(left_pad)
                        .pr_2()
                        .gap(scaled_px(4.0))
                        .interactive_row(row_style, row_state)
                        .when(focused, |row| {
                            row.row_accent(theme.colors.accent.foreground)
                        });

                    let paths = if selected {
                        Rc::clone(&selection_sources)
                    } else {
                        Rc::from(
                            repo.map(|r| vec![r.spec.workdir.join(entry.path.as_ref())])
                                .unwrap_or_default(),
                        )
                    };
                    let native_root = this.root_view.clone();
                    let drag_focus = this.explorer_focus.clone();
                    row_div = row_div
                        .when(
                            repo.is_some_and(|r| {
                                r.file_browser.source
                                    == gitcomet_core::domain::FileSource::WorkingDirectory
                            }),
                            |row| {
                                row.on_drag(
                                    explorer_operations::ExplorerDrag { paths },
                                    move |drag, offset, window, cx| {
                                        window.focus(&drag_focus, cx);
                                        cx.new(|_| {
                                            explorer_operations::ExplorerDragPreview::new(
                                                drag,
                                                is_directory,
                                                theme,
                                                offset,
                                            )
                                        })
                                    },
                                )
                                .drag_move_refresh(gpui::DragMoveRefresh::Preview)
                                .external_drag_payload_async(
                                    move |drag: &explorer_operations::ExplorerDrag, window, cx| {
                                        let intent = if (cfg!(target_os = "macos")
                                            && window.modifiers().alt)
                                            || (!cfg!(target_os = "macos")
                                                && window.modifiers().control)
                                        {
                                            gpui::FileTransferOperation::Copy
                                        } else {
                                            gpui::FileTransferOperation::Move
                                        };
                                        native_root
                                            .update(cx, |root, cx| {
                                                root.prepare_native_drag(
                                                    drag.paths.to_vec(),
                                                    intent,
                                                    cx,
                                                )
                                            })
                                            .unwrap_or_else(|_| gpui::Task::ready(None))
                                    },
                                )
                            },
                        )
                        .when(drop_region.contains(&ix), |row| {
                            let first = ix == drop_region.start;
                            let last = ix + 1 == drop_region.end;
                            let color = theme.colors.accent.foreground;
                            row.relative().bg(with_alpha(color, 0.16)).child(
                                gpui::canvas(
                                    |_, _, _| {},
                                    move |bounds, _, window, _| {
                                        window.paint_quad(
                                            gpui::PaintQuad::new(bounds)
                                                .border_color(with_alpha(color, 0.7))
                                                .border_widths(gpui::Edges {
                                                    top: px(if first { 1.0 } else { 0.0 }),
                                                    bottom: px(if last { 1.0 } else { 0.0 }),
                                                    left: px(1.0),
                                                    right: px(1.0),
                                                }),
                                        );
                                    },
                                )
                                .absolute()
                                .inset_0(),
                            )
                        });

                    if is_directory {
                        let path = (*entry.path).clone();
                        let menu_path = path.clone();
                        row_div = row_div
                            .on_activate(
                                false,
                                controls::ControlActivation::Composite,
                                cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                                    this.explorer_folder_click(
                                        path.clone(),
                                        e.modifiers(),
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .on_pointer_click(
                                MouseButton::Right,
                                cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.explorer_select(
                                        menu_path.clone(),
                                        e.modifiers,
                                        true,
                                        window,
                                        cx,
                                    );
                                    this.open_popover_at(
                                        (PopoverKind::FileBrowserFolderMenu {
                                            repo_id,
                                            path: menu_path.clone(),
                                        })
                                        .invoked_by(menu_invoker.clone()),
                                        e.position,
                                        window,
                                        cx,
                                    );
                                }),
                            );
                    } else {
                        let path = (*entry.path).clone();
                        let menu_path = path.clone();
                        let source = repo
                            .map(|r| r.file_browser.source.clone())
                            .unwrap_or(gitcomet_core::domain::FileSource::WorkingDirectory);
                        row_div = row_div
                            .on_activate(
                                false,
                                controls::ControlActivation::Composite,
                                cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                                    this.explorer_select(
                                        path.clone(),
                                        e.modifiers(),
                                        false,
                                        window,
                                        cx,
                                    );
                                    let modifiers = e.modifiers();
                                    if modifiers.control || modifiers.platform || modifiers.shift {
                                        return;
                                    }
                                    this.show_repository_canvas(cx);
                                    // A file the editor is holding unsaved text for
                                    // opens straight back into the editor. Opening
                                    // the read-only view would show the text on
                                    // disk, which is not what the user left here.
                                    if has_unsaved_edits {
                                        store.dispatch(Msg::OpenFileEditor {
                                            repo_id,
                                            path: path.clone(),
                                        });
                                    } else {
                                        store.dispatch(Msg::OpenFileContent {
                                            repo_id,
                                            source: source.clone(),
                                            path: path.clone(),
                                        });
                                    }
                                }),
                            )
                            .on_pointer_click(
                                MouseButton::Right,
                                cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.explorer_select(
                                        menu_path.clone(),
                                        e.modifiers,
                                        true,
                                        window,
                                        cx,
                                    );
                                    this.open_popover_at(
                                        (PopoverKind::FileBrowserFileMenu {
                                            repo_id,
                                            path: menu_path.clone(),
                                        })
                                        .invoked_by(menu_invoker.clone()),
                                        e.position,
                                        window,
                                        cx,
                                    );
                                }),
                            );
                    }

                    row_div
                        .child({
                            let path = (*entry.path).clone();
                            chevron_slot(is_directory && !expansion_frozen, is_expanded)
                                .id(("explorer_chevron", ix))
                                .debug_selector(move || format!("explorer_chevron_{ix}"))
                                .when(is_directory && !expansion_frozen, |d| {
                                    // The innermost click target owns the click,
                                    // so the row's own activation does not also run.
                                    d.on_activate(
                                        false,
                                        controls::ControlActivation::Composite,
                                        cx.listener(
                                            move |this, _: &gpui::ClickEvent, window, cx| {
                                                this.explorer_chevron_click(
                                                    path.clone(),
                                                    window,
                                                    cx,
                                                );
                                            },
                                        ),
                                    )
                                })
                        })
                        .child(if cut || entry.ignored {
                            icon_slot_tinted(
                                file_or_folder_icon_path(entry, is_expanded),
                                icon_muted,
                            )
                        } else {
                            icon_slot(file_or_folder_icon_path(entry, is_expanded))
                        })
                        .when(cut, |row| {
                            row.child(
                                div()
                                    .debug_selector(move || format!("explorer_cut_marker_{ix}"))
                                    .text_size(theme.ui_text(12.0))
                                    .child("✂"),
                            )
                        })
                        .child({
                            let highlight_ranges =
                                file_search_highlight_ranges(&search_matchers, entry.name.as_ref());
                            let mut label = components::TruncatedText::new(
                                entry.name.to_string(),
                                theme.ui_text(14.0),
                            )
                            .profile(components::TextTruncationProfile::End)
                            .text_color(row_text_color);
                            if !highlight_ranges.is_empty() {
                                let style = gpui::HighlightStyle {
                                    color: Some(theme.colors.accent.foreground.into_color()),
                                    font_weight: Some(FontWeight::BOLD),
                                    ..gpui::HighlightStyle::default()
                                };
                                label = label.highlights(
                                    highlight_ranges.into_iter().map(|range| (range, style)),
                                );
                            }
                            div()
                                .debug_selector(move || format!("explorer_label_{ix}"))
                                .flex_1()
                                .min_w(px(0.0))
                                .child(label.render(cx))
                        })
                        .when_some(status, |row, status| {
                            use gitcomet_core::domain::FileStatusKind;
                            let (label, color) = match status {
                                FileStatusKind::Untracked => {
                                    ("?", theme.colors.status.success.foreground)
                                }
                                FileStatusKind::Added => {
                                    ("A", theme.colors.status.success.foreground)
                                }
                                FileStatusKind::Deleted => {
                                    ("D", theme.colors.status.danger.foreground)
                                }
                                FileStatusKind::Conflicted => {
                                    ("!", theme.colors.status.danger.foreground)
                                }
                                FileStatusKind::Renamed => {
                                    ("R", theme.colors.status.warning.foreground)
                                }
                                FileStatusKind::Modified => {
                                    ("M", theme.colors.status.warning.foreground)
                                }
                            };
                            row.child(
                                div()
                                    .text_size(theme.ui_text(12.0))
                                    .text_color(color)
                                    .child(label),
                            )
                        })
                        .when(has_unsaved_edits, |row| {
                            row.child(div().flex_none().flex().items_center().child(svg_icon(
                                "icons/pencil.svg",
                                theme.colors.status.warning.foreground,
                                10.0,
                            )))
                        })
                        .into_any_element()
                };
                Some(element)
            })
            .collect()
    }
}
