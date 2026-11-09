use super::*;
use crate::view::components::{QuickSearchBar, QuickSearchStatus};

/// The file query's line height; it grows to three lines, then scrolls.
pub(super) const FILE_SEARCH_LINE_HEIGHT_PX: f32 = 18.0;
const FILE_SEARCH_MAX_LINES: f32 = 3.0;

/// Whether a branch-list row is something a search finds, rather than a
/// heading grouping what it found.
fn is_searchable_branch_row(row: &BranchSidebarRow) -> bool {
    matches!(
        row,
        BranchSidebarRow::Branch { .. }
            | BranchSidebarRow::WorktreeItem { .. }
            | BranchSidebarRow::SubmoduleItem { .. }
            | BranchSidebarRow::AnnexRepositoryItem { .. }
            | BranchSidebarRow::StashItem { .. }
    )
}

pub(super) struct SectionLocatorCache {
    state: Arc<AppState>,
    section: CollapsedSidebarSection,
    target: Option<sticky::NavigationTarget>,
}

impl SidebarPaneView {
    /// Any press in the sidebar moves focus into it, so Cmd/Ctrl-F opens its
    /// search. Capture runs before children, which may stop the press (a
    /// scrollbar does); a child that focuses itself still wins.
    pub(super) fn focus_sidebar_on_press(
        &mut self,
        _: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.sidebar_focus.contains_focused(window, cx) {
            window.focus(&self.sidebar_focus, cx);
        }
    }

    /// Whether keyboard focus is anywhere in the sidebar.
    pub(in crate::view) fn contains_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.sidebar_focus.contains_focused(window, cx)
    }

    /// Whether this pane's search shows the file list (the Files mode or the
    /// collapsed Files flyout) rather than the branch list.
    fn search_targets_files(&self) -> bool {
        self.collapsed_popover_section
            .map_or(self.state.sidebar_mode == SidebarMode::Files, |section| {
                section == CollapsedSidebarSection::Files
            })
    }

    /// Cmd/Ctrl-F with focus in the sidebar: open the search of the list it
    /// shows, or refocus it with its query selected when already open.
    /// Returns false where there is no list to search (the History tab, an
    /// extension tab).
    pub(in crate::view) fn open_search(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let files = self.search_targets_files();
        if self.collapsed_popover_section.is_none()
            && !matches!(
                self.state.sidebar_mode,
                SidebarMode::Files | SidebarMode::Branches
            )
        {
            return false;
        }
        if files {
            self.file_search_open = true;
        } else {
            self.branch_search_open = true;
        }
        let input = if files {
            self.file_browser_search_input.clone()
        } else {
            self.branch_filter_input.clone()
        };
        input.update(cx, |input, cx| input.select_all_text(window, cx));
        window.focus(&input.read(cx).focus_handle(), cx);
        cx.notify();
        true
    }

    /// F2/F3 with focus in the sidebar: step its open search. Returns false
    /// when it is closed, so the keys keep their main-pane meaning.
    pub(in crate::view) fn step_search(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let files = self.search_targets_files();
        let open = if files {
            self.file_search_open
        } else {
            self.branch_search_open
        };
        if open {
            self.sidebar_search_step(files, forward, window, cx);
        }
        open
    }

    /// Close the search: clear its query and hand focus back to the sidebar,
    /// so another Cmd/Ctrl-F reopens it.
    fn close_sidebar_search(
        &mut self,
        files: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if files {
            self.file_search_open = false;
            self.file_search_bar.dismiss_menu();
        } else {
            self.branch_search_open = false;
            self.branch_search_bar.dismiss_menu();
        }
        self.clear_sidebar_search(files, cx);
        self.focus_after_search(window, cx);
        cx.notify();
    }

    /// Escape in the bar: close its overflow menu first, then the bar.
    fn sidebar_search_escape(
        &mut self,
        files: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let bar = if files {
            &self.file_search_bar
        } else {
            &self.branch_search_bar
        };
        if bar.dismiss_menu() {
            cx.notify();
        } else {
            self.close_sidebar_search(files, window, cx);
        }
    }

    fn clear_sidebar_search(&mut self, files: bool, cx: &mut gpui::Context<Self>) {
        if files {
            self.file_browser_search_input
                .update(cx, |input, cx| input.set_text("", cx));
            if let Some(repo_id) = self.active_repo_id() {
                self.store.dispatch(Msg::SetFileBrowserSearch {
                    repo_id,
                    query: String::new(),
                });
            }
            self.file_browser_scroll
                .scroll_to_item(0, gpui::ScrollStrategy::Top);
            cx.notify();
        } else {
            self.clear_branch_filter(cx);
        }
    }

    /// Keep focus in the sidebar: Escape can then dismiss a flyout, and
    /// Cmd/Ctrl-F routes back to this pane's search.
    fn focus_after_search(&self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        window.focus(&self.sidebar_focus, cx);
    }

    pub(super) fn on_sidebar_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if event.keystroke.key != "escape" || event.keystroke.modifiers.modified() {
            return;
        }
        let files = self.search_targets_files();
        let open = if files {
            self.file_search_open
        } else {
            self.branch_search_open
        };
        if open {
            self.sidebar_search_escape(files, window, cx);
        } else if self.collapsed_popover_section.is_some() {
            let root = self.root_view.clone();
            cx.defer(move |cx| {
                let _ = root.update(cx, |root, cx| root.close_sidebar_collapsed_popover(cx));
            });
        } else {
            return;
        }
        cx.stop_propagation();
    }

    pub(super) fn cached_file_matchers(&self) -> Rc<Vec<TextSearchMatcher>> {
        let query = self
            .active_repo()
            .map_or("", |repo| repo.file_browser.search_query.as_str());
        let mut cache = self.file_matchers_cache.borrow_mut();
        if let Some((cached, options, matchers)) = cache.as_ref()
            && cached == query
            && *options == self.file_search_options
        {
            return Rc::clone(matchers);
        }
        let matchers = Rc::new(file_search_matchers(query, self.file_search_options));
        *cache = Some((
            query.to_owned(),
            self.file_search_options,
            Rc::clone(&matchers),
        ));
        matchers
    }

    /// The path a file-browser row shows, for matching and selecting it.
    fn file_browser_row_path(&self, row: &FileBrowserVisibleRow) -> Option<std::path::PathBuf> {
        match row {
            FileBrowserVisibleRow::FileSetFile { path, .. } => Some(path.to_path_buf()),
            FileBrowserVisibleRow::Entry { entry_index, .. } => self
                .active_repo()
                .and_then(|repo| repo.file_browser.entries.ready())
                .and_then(|entries| entries.get(*entry_index))
                .map(|entry| entry.path.to_path_buf()),
            FileBrowserVisibleRow::FileSetHeader { .. }
            | FileBrowserVisibleRow::NameEntry { .. } => None,
        }
    }

    /// Indices of the visible file rows the search matches, cached per row
    /// list (the list is rebuilt whenever the query or options change).
    fn file_search_match_rows(&self, cx: &gpui::App) -> Rc<[usize]> {
        let rows = self.file_browser_visible_rows(cx);
        if let Some((_, matches)) = self
            .file_match_count_cache
            .borrow()
            .as_ref()
            .filter(|(source, _)| Rc::ptr_eq(source, &rows))
        {
            return Rc::clone(matches);
        }
        let matchers = self.cached_file_matchers();
        let entries = self
            .active_repo()
            .and_then(|repo| repo.file_browser.entries.ready());
        let matches: Rc<[usize]> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match row {
                FileBrowserVisibleRow::FileSetFile { path, .. } => {
                    file_search_matches(&matchers, &path.to_string_lossy())
                }
                FileBrowserVisibleRow::Entry { entry_index, .. } => {
                    entries.is_some_and(|entries| {
                        file_search_matches(
                            &matchers,
                            &entries[*entry_index].path.to_string_lossy(),
                        )
                    })
                }
                FileBrowserVisibleRow::FileSetHeader { .. }
                | FileBrowserVisibleRow::NameEntry { .. } => false,
            })
            .map(|(ix, _)| ix)
            .collect();
        *self.file_match_count_cache.borrow_mut() = Some((rows, Rc::clone(&matches)));
        matches
    }

    /// The branch-list rows a search can land on: refs, worktrees,
    /// submodules, annex repositories and stashes, not group headers.
    fn branch_search_match_rows(&mut self) -> Vec<usize> {
        let Some(presentation) = self.branch_sidebar_presentation_cached() else {
            return Vec::new();
        };
        if presentation.search.matcher.is_empty() {
            return Vec::new();
        }
        presentation
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| is_searchable_branch_row(row))
            .map(|(ix, _)| ix)
            .collect()
    }

    /// The zero-based position of the current match among all matches.
    fn sidebar_search_current(&mut self, files: bool, cx: &gpui::App) -> Option<usize> {
        if files {
            let focused = self.explorer_focused_path()?.to_path_buf();
            let rows = self.file_browser_visible_rows(cx);
            self.file_search_match_rows(cx).iter().position(|&ix| {
                self.file_browser_row_path(&rows[ix]).as_deref() == Some(focused.as_path())
            })
        } else {
            let cursor = self.branch_search_cursor?;
            self.branch_search_match_rows()
                .iter()
                .position(|&ix| ix == cursor)
        }
    }

    /// ↑/↓ (and Enter, F2/F3) in the bar: move to the previous or next
    /// matching row, wrapping around, as if it had been clicked.
    fn sidebar_search_step(
        &mut self,
        files: bool,
        forward: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let current = self.sidebar_search_current(files, cx);
        let matches: Vec<usize> = if files {
            self.file_search_match_rows(cx).to_vec()
        } else {
            self.branch_search_match_rows()
        };
        if matches.is_empty() {
            return;
        }
        let next = match (current, forward) {
            (None, true) => 0,
            (None, false) => matches.len() - 1,
            (Some(ix), true) => (ix + 1) % matches.len(),
            (Some(ix), false) => (ix + matches.len() - 1) % matches.len(),
        };
        let row_ix = matches[next];
        let input = if files {
            let rows = self.file_browser_visible_rows(cx);
            if let Some(path) = self.file_browser_row_path(&rows[row_ix]) {
                self.explorer_select(path, gpui::Modifiers::default(), false, window, cx);
            }
            self.file_browser_scroll
                .scroll_to_item(row_ix, gpui::ScrollStrategy::Center);
            self.file_browser_search_input.clone()
        } else {
            self.branch_search_cursor = Some(row_ix);
            self.select_branch_search_row(row_ix, cx);
            self.sidebar_scroll_animation = None;
            self.branches_scroll
                .scroll_to_item(row_ix, gpui::ScrollStrategy::Center);
            self.branch_filter_input.clone()
        };
        // Keep typing in the bar while stepping.
        window.focus(&input.read(cx).focus_handle(), cx);
        cx.notify();
    }

    /// Select a matched branch row the way a click does; other rows (a
    /// worktree, a stash) are only scrolled to.
    fn select_branch_search_row(&mut self, row_ix: usize, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let Some(presentation) = self.branch_sidebar_presentation_cached() else {
            return;
        };
        let BranchSidebarRow::Branch {
            target, is_head, ..
        } = &presentation.rows[row_ix]
        else {
            return;
        };
        let (target, is_head) = (target.clone(), *is_head);
        let pin_key =
            (row_ix < presentation.pins.len()).then(|| presentation.row_keys[row_ix].clone());
        let Some(reveal) = self.active_repo().and_then(|repo| {
            crate::view::rows::sidebar::branch_click_history_reveal_target(repo, &target, is_head)
        }) else {
            return;
        };
        self.select_branch_and_reveal_tip(
            repo_id,
            target,
            reveal.commit_id,
            reveal.fallback_scope,
            pin_key,
            cx,
        );
    }

    pub(super) fn render_sidebar_search(
        &mut self,
        files: bool,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = ui_scale::UiScale::current(cx);
        let (empty, invalid, total, complete) = if files {
            let matchers = self.cached_file_matchers();
            let count = self.file_search_match_rows(cx).len();
            (
                matchers.is_empty(),
                matchers.iter().any(|m| m.regex_error().is_some()),
                count,
                self.active_repo().is_some_and(|repo| {
                    matches!(
                        repo.file_browser.entries,
                        Loadable::Ready(_) | Loadable::Error(_)
                    )
                }),
            )
        } else {
            let presentation = self.branch_sidebar_presentation_cached();
            let complete =
                self.active_repo()
                    .is_some_and(|repo| match self.collapsed_popover_section {
                        Some(CollapsedSidebarSection::Local) => {
                            matches!(repo.branches, Loadable::Ready(_) | Loadable::Error(_))
                        }
                        Some(CollapsedSidebarSection::Remote) => matches!(
                            repo.remote_branches,
                            Loadable::Ready(_) | Loadable::Error(_)
                        ),
                        Some(CollapsedSidebarSection::Worktrees) => {
                            matches!(repo.worktrees, Loadable::Ready(_) | Loadable::Error(_))
                        }
                        Some(CollapsedSidebarSection::Submodules) => {
                            matches!(repo.submodules, Loadable::Ready(_) | Loadable::Error(_))
                        }
                        Some(CollapsedSidebarSection::Stashes) => {
                            matches!(repo.stashes, Loadable::Ready(_) | Loadable::Error(_))
                        }
                        Some(CollapsedSidebarSection::Annex) => matches!(
                            repo.large_file_support,
                            Loadable::Ready(_) | Loadable::Error(_)
                        ),
                        _ => {
                            (matches!(repo.branches, Loadable::Ready(_) | Loadable::Error(_))
                                && matches!(
                                    repo.remote_branches,
                                    Loadable::Ready(_) | Loadable::Error(_)
                                )
                                && matches!(
                                    repo.worktrees,
                                    Loadable::Ready(_) | Loadable::Error(_)
                                )
                                && matches!(
                                    repo.submodules,
                                    Loadable::Ready(_) | Loadable::Error(_)
                                )
                                && matches!(repo.stashes, Loadable::Ready(_) | Loadable::Error(_)))
                        }
                    });
            presentation.map_or((true, false, 0, complete), |p| {
                (
                    p.search.matcher.is_empty(),
                    p.search.matcher.regex_error().is_some(),
                    p.match_count,
                    complete,
                )
            })
        };
        let status = if invalid {
            QuickSearchStatus::InvalidRegex
        } else if empty {
            QuickSearchStatus::Empty
        } else if total > 0 {
            QuickSearchStatus::Position {
                current: self.sidebar_search_current(files, cx),
                total,
                complete,
            }
        } else if complete {
            QuickSearchStatus::NoMatches
        } else {
            QuickSearchStatus::Searching
        };
        let (input, options, bar) = if files {
            (
                self.file_browser_search_input.clone(),
                self.file_search_options,
                Rc::clone(&self.file_search_bar),
            )
        } else {
            (
                self.branch_filter_input.clone(),
                self.branch_search_options,
                Rc::clone(&self.branch_search_bar),
            )
        };
        // Pad the first line onto the centre of the bar's control row.
        let file_line = scale.px(FILE_SEARCH_LINE_HEIGHT_PX);
        let file_pad =
            ((components::quick_search_row_height(scale) - file_line) / 2.0).max(px(0.0));
        // An explicit height: the multi-line input fills its parent, so an
        // auto height would take the bar's row instead of the text's lines.
        let file_lines = input.read(cx).text().matches('\n').count() + 1;
        let file_input_height =
            file_line * (file_lines as f32).min(FILE_SEARCH_MAX_LINES) + file_pad * 2.0;
        let input: AnyElement = if files {
            // The multi-line file query grows to three lines, then scrolls.
            div()
                .id("file_search_input_scroll")
                .relative()
                .w_full()
                .min_w(px(0.0))
                .h(file_input_height)
                .py(file_pad)
                .overflow_y_scroll()
                .track_scroll(&self.file_search_scroll)
                .child(input)
                .into_any_element()
        } else {
            input.into_any_element()
        };
        let step = move |forward: bool| {
            move |this: &mut Self, window: &mut Window, cx: &mut gpui::Context<Self>| {
                this.sidebar_search_step(files, forward, window, cx);
            }
        };
        let bar = QuickSearchBar::new(
            if files {
                "file_search"
            } else {
                "branch_filter"
            },
            &bar,
            status,
        )
        .input(input)
        .options(
            components::search_toggles(options),
            move |this: &mut Self, toggles, _, cx| {
                let options = components::search_options(toggles);
                if files {
                    this.file_search_options = options;
                    this.file_browser_scroll
                        .scroll_to_item(0, gpui::ScrollStrategy::Top);
                } else {
                    this.branch_search_options = options;
                    this.branch_search_cursor = None;
                    this.sticky_context = None;
                    this.pending_sidebar_navigation = None;
                    this.sidebar_scroll_animation = None;
                    this.branches_scroll
                        .scroll_to_item(0, gpui::ScrollStrategy::Top);
                    this.sync_popover_branch_filter(cx);
                }
                cx.notify();
            },
        )
        .navigation(total > 0, step(false), step(true))
        .on_close(move |this, window, cx| this.close_sidebar_search(files, window, cx))
        .render(theme, scale, cx);
        div()
            .id(if files {
                "sidebar_files_search"
            } else {
                "sidebar_branches_search"
            })
            .debug_selector(move || {
                if files {
                    "sidebar_files_search"
                } else {
                    "sidebar_branches_search"
                }
                .to_owned()
            })
            .flex_none()
            .w_full()
            .min_w(px(0.0))
            .px(scale.px(8.0))
            .py(scale.px(6.0))
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    let key = &event.keystroke;
                    if key.modifiers.modified() && !(key.modifiers.shift && key.key == "enter") {
                        return;
                    }
                    match key.key.as_str() {
                        "escape" => this.sidebar_search_escape(files, window, cx),
                        // A file query takes newlines; branch queries step back.
                        "enter" if key.modifiers.shift && !files => {
                            this.sidebar_search_step(files, false, window, cx)
                        }
                        "enter" if !key.modifiers.shift => {
                            this.sidebar_search_step(files, true, window, cx)
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                }),
            )
            .child(bar)
    }

    pub(super) fn section_locator_target(
        &self,
        section: CollapsedSidebarSection,
    ) -> Option<sticky::NavigationTarget> {
        let mut cache = self.section_locator_cache.borrow_mut();
        if let Some(cached) = cache.as_ref()
            && cached.section == section
            && Arc::ptr_eq(&cached.state, &self.state)
        {
            return cached.target.clone();
        }
        // Resolve against a new snapshot once; scrolling the same list must
        // not walk a large branch/worktree collection just to enable a button.
        let target = self.resolve_section_locator_target(section);
        *cache = Some(SectionLocatorCache {
            state: Arc::clone(&self.state),
            section,
            target: target.clone(),
        });
        target
    }

    fn resolve_section_locator_target(
        &self,
        section: CollapsedSidebarSection,
    ) -> Option<sticky::NavigationTarget> {
        let repo = self.active_repo()?;
        match section {
            CollapsedSidebarSection::Local => active_local_branch_target(repo)
                .map(|(name, _)| sticky::NavigationTarget::Branch(BranchMenuTarget::local(name))),
            CollapsedSidebarSection::Remote => {
                let (name, _) = active_local_branch_target(repo)?;
                let Loadable::Ready(branches) = &repo.branches else {
                    return None;
                };
                let upstream = branches
                    .iter()
                    .find(|branch| branch.name == name)?
                    .upstream
                    .as_ref()?;
                let Loadable::Ready(remotes) = &repo.remote_branches else {
                    return None;
                };
                remotes
                    .iter()
                    .any(|branch| {
                        branch.remote == upstream.remote && branch.name == upstream.branch
                    })
                    .then(|| {
                        sticky::NavigationTarget::Branch(BranchMenuTarget::remote(
                            &upstream.remote,
                            &upstream.branch,
                        ))
                    })
            }
            CollapsedSidebarSection::Worktrees => {
                let Loadable::Ready(worktrees) = &repo.worktrees else {
                    return None;
                };
                worktrees
                    .iter()
                    .find(|tree| tree.path == repo.spec.workdir)
                    .map(|tree| {
                        sticky::NavigationTarget::RowKey(
                            format!("tree:worktree:{}", tree.path.display()).into(),
                        )
                    })
            }
            CollapsedSidebarSection::Submodules => {
                let path = repo
                    .diff_state
                    .inline_submodule_diff
                    .as_ref()
                    .map(|diff| &diff.parent_submodule_path)
                    .or_else(|| match &repo.diff_state.submodule_summary {
                        Loadable::Ready(summary) => Some(&summary.path),
                        _ => None,
                    })?;
                let Loadable::Ready(submodules) = &repo.submodules else {
                    return None;
                };
                submodules
                    .iter()
                    .any(|module| &module.path == path)
                    .then(|| {
                        sticky::NavigationTarget::RowKey(
                            format!("tree:submodule:{}", path.display()).into(),
                        )
                    })
            }
            CollapsedSidebarSection::Stashes => {
                let selected = repo.history_state.selected_commit.as_ref()?;
                let Loadable::Ready(stashes) = &repo.stashes else {
                    return None;
                };
                stashes.iter().any(|stash| &stash.id == selected).then(|| {
                    sticky::NavigationTarget::RowKey(format!("tree:stash:{}", selected.0).into())
                })
            }
            CollapsedSidebarSection::Annex | CollapsedSidebarSection::Files => None,
        }
    }

    pub(super) fn render_section_locator(
        &self,
        section: CollapsedSidebarSection,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let scale = ui_scale::UiScale::current(cx);
        let available = if section == CollapsedSidebarSection::Files {
            self.active_repo()
                .and_then(|repo| repo.open_file_path())
                .is_some()
        } else {
            self.section_locator_target(section).is_some()
        };
        components::Button::new("collapsed_popover_locator", "")
            .borderless()
            .style(components::ButtonStyle::Subtle)
            .disabled(!available)
            .start_slot(icons::svg_icon(
                "icons/locate.svg",
                theme.colors.foreground.secondary,
                scale.px(13.0),
            ))
            .on_click(theme, cx, move |this, _, _, cx| {
                this.locate_sidebar_section(section, cx)
            })
            .w(components::control_height(scale))
            .h(components::control_height(scale))
            .gitcomet_tooltip(
                theme,
                if available {
                    "Show current item"
                } else {
                    "No current item to show"
                }
                .into(),
            )
            .debug_selector(|| "collapsed_popover_locator".to_owned())
            .into_any_element()
    }

    fn locate_sidebar_section(
        &mut self,
        section: CollapsedSidebarSection,
        cx: &mut gpui::Context<Self>,
    ) {
        if section == CollapsedSidebarSection::Local {
            self.locate_active_local_branch(cx);
            return;
        }
        if section == CollapsedSidebarSection::Files {
            self.locate_open_file(cx);
            return;
        }
        let Some(target) = self.section_locator_target(section) else {
            return;
        };
        self.clear_branch_filter(cx);
        if let sticky::NavigationTarget::Branch(BranchMenuTarget::Remote { remote, branch }) =
            &target
            && let Some((repo_id, path)) = self
                .active_repo()
                .map(|repo| (repo.id, repo.spec.workdir.clone()))
        {
            let before = self
                .sidebar_collapsed_items_by_repo
                .get(&path)
                .cloned()
                .unwrap_or_default();
            let collapsed = self
                .sidebar_collapsed_items_by_repo
                .entry(path)
                .or_default();
            branch_sidebar::set_collapse_state(
                collapsed,
                &branch_sidebar::remote_header_storage_key(remote),
                false,
            );
            for end in branch
                .match_indices('/')
                .map(|(ix, _)| ix)
                .chain(std::iter::once(branch.len()))
            {
                branch_sidebar::set_collapse_state(
                    collapsed,
                    &branch_sidebar::remote_group_storage_key(remote, &branch[..end]),
                    false,
                );
            }
            self.sidebar_presentation_cache = SidebarPresentationCache::default();
            self.publish_sidebar_collapse_changes(repo_id, &before);
            self.sync_popover_collapsed_items(cx);
        }
        self.pending_sidebar_navigation = Some(target);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::test_support::{self, TestBackend};

    /// The production keymap, so Cmd/Ctrl-F and Escape route as in the app.
    fn bind_keys(app: &mut gpui::App) {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        crate::app::bind_text_input_keys_for_test(app);
    }

    #[gpui::test]
    fn sidebar_files_search_shares_options_and_clears_on_close(cx: &mut gpui::TestAppContext) {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) = cx.add_window_view(|window, cx| {
            window.activate();
            GitCometView::new(store, events, None, window, cx)
        });
        let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
        let mut state = super::super::long_list_tests::file_fixture(100);
        Arc::make_mut(&mut state).sidebar_mode = SidebarMode::Files;
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.store.replace_snapshot_for_test(state.clone());
                test_support::push_test_state(view, state, cx);
                view.set_sidebar_collapsed(false, cx);
                // Wide enough for the bar to show its toggles inline.
                test_support::set_sidebar_width_for_test(view, px(520.0), cx);
            })
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_files_search").is_none());
        cx.update(|window, app| {
            bind_keys(app);
            let focus = pane.read(app).sidebar_focus.clone();
            window.focus(&focus, app)
        });
        cx.simulate_keystrokes("secondary-f");
        test_support::redraw(cx);
        test_support::redraw(cx);
        cx.simulate_keystrokes("f i l e _ 0 0 0 0 0 3");
        for id in [
            "file_search_match_case",
            "file_search_whole_word",
            "file_search_regex",
        ] {
            let bounds = cx.debug_bounds(id).unwrap();
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            test_support::redraw(cx);
        }
        test_support::drain_store_worker(&view, cx);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                test_support::sync_store_snapshot(view, cx);
                view.set_sidebar_collapsed(true, cx);
                view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Files, cx);
            })
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_files_search").is_some());
        cx.update(|_, app| {
            let pane = pane.read(app);
            assert_eq!(
                pane.active_repo().unwrap().file_browser.search_query,
                "file_000003"
            );
            assert!(
                pane.file_search_options.match_case
                    && pane.file_search_options.whole_word
                    && pane.file_search_options.regex
            );
            assert_eq!(pane.file_browser_visible_rows(app).len(), 1);
            assert!(!pane.branch_search_open);
            assert!(pane.branch_filter_query.is_empty());
        });
        let close = cx.debug_bounds("file_search_close").unwrap();
        cx.simulate_click(close.center(), gpui::Modifiers::default());
        test_support::drain_store_worker(&view, cx);
        cx.update(|_, app| {
            view.update(app, |view, cx| test_support::sync_store_snapshot(view, cx))
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_files_search").is_none());
        cx.update(|window, app| {
            let pane = pane.read(app);
            assert!(
                pane.active_repo()
                    .unwrap()
                    .file_browser
                    .search_query
                    .is_empty()
            );
            // Closing hands focus back to the sidebar, so Cmd/Ctrl-F reopens
            // this search rather than the main pane's.
            assert!(pane.contains_focus(window, app));
        });
        cx.simulate_keystrokes("secondary-f");
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_files_search").is_some());
        cx.simulate_keystrokes("f i l e");
        test_support::drain_store_worker(&view, cx);
        cx.simulate_keystrokes("escape");
        test_support::drain_store_worker(&view, cx);
        cx.update(|_, app| {
            view.update(app, |view, cx| test_support::sync_store_snapshot(view, cx))
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_files_search").is_none());
        cx.update(|_, app| {
            let pane = pane.read(app);
            assert!(
                pane.active_repo()
                    .unwrap()
                    .file_browser
                    .search_query
                    .is_empty()
            );
            assert!(pane.file_search_options.regex);
            assert!(view.read(app).sidebar_collapsed_popover.is_some());
        });
    }

    #[gpui::test]
    fn sidebar_search_shares_query_options_visibility_and_focus_across_modes(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) = cx.add_window_view(|window, cx| {
            window.activate();
            GitCometView::new(store, events, None, window, cx)
        });
        let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
        let state = super::super::long_list_tests::branch_fixture(100);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.store.replace_snapshot_for_test(state.clone());
                test_support::push_test_state(view, state, cx);
                view.set_sidebar_collapsed(false, cx);
                // Wide enough for the bar to show its toggles inline.
                test_support::set_sidebar_width_for_test(view, px(520.0), cx);
            })
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_branches_search").is_none());
        cx.update(|window, app| {
            bind_keys(app);
            let focus = pane.read(app).sidebar_focus.clone();
            window.focus(&focus, app)
        });
        cx.simulate_keystrokes("secondary-f");
        test_support::redraw(cx);
        test_support::redraw(cx);
        cx.simulate_keystrokes("t o p i c - 0 0 0 0 0 3");
        test_support::redraw(cx);
        for id in [
            "branch_filter_match_case",
            "branch_filter_whole_word",
            "branch_filter_regex",
        ] {
            let bounds = cx.debug_bounds(id).unwrap();
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            test_support::redraw(cx);
        }
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.set_sidebar_collapsed(true, cx);
                view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Local, cx);
            })
        });
        test_support::redraw(cx);
        for section in [
            CollapsedSidebarSection::Local,
            CollapsedSidebarSection::Remote,
            CollapsedSidebarSection::Worktrees,
        ] {
            cx.update(|_, app| {
                view.update(app, |view, cx| {
                    view.open_sidebar_collapsed_popover(section, cx)
                })
            });
            test_support::redraw(cx);
            assert!(cx.debug_bounds("sidebar_branches_search").is_some());
            cx.update(|_, app| {
                pane.update(app, |pane, _| {
                    assert_eq!(pane.branch_filter_query, "topic-000003");
                    assert!(
                        pane.branch_search_options.match_case
                            && pane.branch_search_options.whole_word
                            && pane.branch_search_options.regex
                    );
                    let p = pane.branch_sidebar_presentation_cached().unwrap();
                    let count = p
                        .rows
                        .iter()
                        .filter(|row| matches!(row, BranchSidebarRow::Branch { .. }))
                        .count();
                    assert_eq!(
                        count,
                        usize::from(section != CollapsedSidebarSection::Worktrees)
                    );
                })
            });
        }
        cx.update(|window, app| {
            window.focus(
                &pane.read(app).branch_filter_input.read(app).focus_handle(),
                app,
            )
        });
        cx.simulate_keystrokes("escape");
        test_support::redraw(cx);
        cx.update(|_, app| {
            let pane = pane.read(app);
            assert!(!pane.branch_search_open);
            assert!(pane.branch_filter_query.is_empty());
            assert!(pane.branch_search_options.regex);
            assert!(view.read(app).sidebar_collapsed_popover.is_some());
        });
        cx.simulate_keystrokes("escape");
        test_support::redraw(cx);
        cx.update(|_, app| assert!(view.read(app).sidebar_collapsed_popover.is_none()));
        cx.update(|_, app| view.update(app, |view, cx| view.set_sidebar_collapsed(false, cx)));
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_branches_search").is_none());
    }

    #[gpui::test]
    fn sidebar_search_controls_fit_the_minimum_width_at_each_density_and_scale(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
        let state = super::super::long_list_tests::branch_fixture(10);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.store.replace_snapshot_for_test(state.clone());
                test_support::push_test_state(view, state, cx);
                view.set_sidebar_collapsed(false, cx);
                view.sidebar_width_design = 200.0;
                view.sidebar_width = px(200.0);
                view.sidebar_render_width = px(200.0);
                view.sidebar_pane.update(cx, |pane, cx| {
                    pane.branch_search_open = true;
                    pane.file_search_open = true;
                    cx.notify();
                });
            })
        });
        for density in [
            crate::appearance::UiDensity::Compact,
            crate::appearance::UiDensity::Comfortable,
            crate::appearance::UiDensity::Spacious,
        ] {
            for percent in [100, 150] {
                cx.update(|_, app| {
                    app.set_global(crate::appearance::Appearance {
                        density,
                        ..Default::default()
                    });
                    ui_scale::set_default(app, percent);
                    view.update(app, |view, cx| {
                        view.notify_font_preferences_changed(cx);
                        test_support::set_sidebar_width_for_test(
                            view,
                            px(200.0 * percent as f32 / 100.0),
                            cx,
                        );
                    });
                });
                test_support::redraw(cx);
                let panel = cx.debug_bounds("sidebar_pane").unwrap();
                let mut previous_right = panel.left();
                for id in [
                    "sidebar_tab_branches",
                    "sidebar_tab_files",
                    "sidebar_locate_active_branch",
                ] {
                    let bounds = cx.debug_bounds(id).unwrap();
                    assert!(
                        bounds.left() >= previous_right && bounds.right() <= panel.right(),
                        "{id} must fit the header at {density:?}/{percent}: {bounds:?}, {panel:?}"
                    );
                    previous_right = bounds.right();
                }
                // The bar settles on its width tier a frame after it is measured.
                test_support::redraw(cx);
                let bar = cx.debug_bounds("sidebar_branches_search").unwrap();
                let input = cx.debug_bounds("branch_filter_input_slot").unwrap();
                assert!(input.size.width > px(80.0));
                // At this width the toggles live in the "⋮" menu.
                assert!(cx.debug_bounds("branch_filter_match_case").is_none());
                for id in ["branch_filter_more", "branch_filter_close"] {
                    let bounds = cx.debug_bounds(id).unwrap();
                    assert!(
                        bounds.left() >= bar.left() && bounds.right() <= bar.right(),
                        "{id} at {density:?}/{percent}: {bounds:?}, {bar:?}"
                    );
                }
                let matcher = cx.update(|_, app| {
                    pane.update(app, |pane, _| {
                        pane.branch_sidebar_presentation_cached().unwrap().search
                    })
                });
                cx.update(|_, app| {
                    pane.update(app, |pane, cx| {
                        pane.branches_scroll
                            .scroll_to_item(5, gpui::ScrollStrategy::Top);
                        cx.notify();
                    })
                });
                test_support::redraw(cx);
                cx.update(|_, app| {
                    pane.update(app, |pane, _| {
                        assert!(Rc::ptr_eq(
                            &matcher,
                            &pane.branch_sidebar_presentation_cached().unwrap().search
                        ))
                    })
                });
            }
        }
    }

    #[gpui::test]
    fn section_locators_use_current_context_and_preserve_minimized_mode(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
        let mut state = super::super::long_list_tests::branch_fixture(100);
        let repo = &mut Arc::make_mut(&mut state).repos[0];
        repo.head_branch = Loadable::Ready("shared/topic-000050".into());
        if let Loadable::Ready(branches) = &mut repo.branches {
            Arc::make_mut(branches)[50].upstream = Some(gitcomet_core::domain::Upstream {
                remote: "origin".into(),
                branch: "shared/topic-000050".into(),
            });
        }
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.store.replace_snapshot_for_test(state.clone());
                test_support::push_test_state(view, state, cx);
                view.set_sidebar_collapsed(true, cx);
                view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Remote, cx);
                view.sidebar_pane.update(cx, |pane, cx| {
                    pane.branch_filter_query = "hidden".into();
                    pane.set_collapsed_keys_for_test(&[
                        "group:remote-header:origin",
                        "group:remote:origin:shared",
                    ]);
                    cx.notify();
                });
            })
        });
        test_support::redraw(cx);
        let locator = cx.debug_bounds("collapsed_popover_locator").unwrap();
        cx.simulate_click(locator.center(), gpui::Modifiers::default());
        test_support::redraw(cx);
        cx.update(|_, app| {
            pane.update(app, |pane, _| {
                assert_eq!(
                    pane.collapsed_popover_section,
                    Some(CollapsedSidebarSection::Remote)
                );
                assert!(pane.branch_filter_query.is_empty());
                assert!(
                    !pane
                        .collapsed_items_for_test()
                        .contains("group:remote-header:origin")
                );
                assert!(pane.branches_scroll.0.borrow().base_handle.offset().y < px(0.0));
                assert!(
                    pane.section_locator_target(CollapsedSidebarSection::Stashes)
                        .is_none()
                );
                assert!(
                    pane.section_locator_target(CollapsedSidebarSection::Submodules)
                        .is_none()
                );
            })
        });
    }
}
