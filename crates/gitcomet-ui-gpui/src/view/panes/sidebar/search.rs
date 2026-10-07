use super::*;
use crate::view::components::{QuickSearchBar, QuickSearchStatus};

pub(super) struct SectionLocatorCache {
    state: Arc<AppState>,
    section: CollapsedSidebarSection,
    target: Option<sticky::NavigationTarget>,
}

impl SidebarPaneView {
    pub(super) fn render_search_toggle(
        &self,
        files: bool,
        id: &'static str,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let scale = ui_scale::UiScale::current(cx);
        let open = if files {
            self.file_search_open
        } else {
            self.branch_search_open
        };
        components::Button::new(id, "")
            .borderless()
            .style(components::ButtonStyle::Subtle)
            .open(open)
            .selected_bg(with_alpha(
                theme.colors.accent.foreground,
                if theme.is_dark { 0.34 } else { 0.24 },
            ))
            .start_slot(icons::svg_icon(
                "icons/zoom.svg",
                if open {
                    theme.colors.accent.foreground
                } else {
                    theme.colors.foreground.secondary
                },
                scale.px(13.0),
            ))
            .on_click(theme, cx, move |this, _, window, cx| {
                let open = if files {
                    &mut this.file_search_open
                } else {
                    &mut this.branch_search_open
                };
                *open = !*open;
                if *open {
                    let input = if files {
                        &this.file_browser_search_input
                    } else {
                        &this.branch_filter_input
                    };
                    window.focus(&input.read(cx).focus_handle(), cx);
                } else {
                    this.clear_sidebar_search(files, cx);
                    this.focus_after_search(window, cx);
                }
                cx.notify();
            })
            .w(components::control_height(scale))
            .h(components::control_height(scale))
            .gitcomet_tooltip(theme, if open { "Hide search" } else { "Search" }.into())
            .debug_selector(move || id.to_owned())
            .into_any_element()
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

    fn focus_after_search(&self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if self.collapsed_popover_section.is_some() {
            // Keep Escape available for dismissing the flyout after its input
            // is hidden. The expanded panel otherwise keeps its passive focus
            // behavior when the user clicks outside an editor/input.
            window.focus(&self.sidebar_focus, cx);
        } else {
            window.blur(cx);
        }
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
        let files = self
            .collapsed_popover_section
            .map_or(self.state.sidebar_mode == SidebarMode::Files, |section| {
                section == CollapsedSidebarSection::Files
            });
        let open = if files {
            &mut self.file_search_open
        } else {
            &mut self.branch_search_open
        };
        if *open {
            *open = false;
            self.clear_sidebar_search(files, cx);
            self.focus_after_search(window, cx);
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

    pub(super) fn render_sidebar_search(
        &mut self,
        files: bool,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = ui_scale::UiScale::current(cx);
        let (empty, invalid, total, complete) = if files {
            let matchers = self.cached_file_matchers();
            let rows = self.file_browser_visible_rows(cx);
            let mut cache = self.file_match_count_cache.borrow_mut();
            let count = if let Some((_, count)) = cache
                .as_ref()
                .filter(|(source, _)| Rc::ptr_eq(source, &rows))
            {
                *count
            } else {
                let entries = self
                    .active_repo()
                    .and_then(|repo| repo.file_browser.entries.ready());
                let count =
                    rows.iter()
                        .filter(|row| match row {
                            FileBrowserVisibleRow::FileSetFile { path, .. } => {
                                file_search_matches(&matchers, &path.to_string_lossy())
                            }
                            FileBrowserVisibleRow::Entry { entry_index, .. } => entries
                                .is_some_and(|entries| {
                                    file_search_matches(
                                        &matchers,
                                        &entries[*entry_index].path.to_string_lossy(),
                                    )
                                }),
                            FileBrowserVisibleRow::FileSetHeader { .. }
                            | FileBrowserVisibleRow::NameEntry { .. } => false,
                        })
                        .count();
                *cache = Some((Rc::clone(&rows), count));
                count
            };
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
                current: None,
                total,
                complete,
            }
        } else if complete {
            QuickSearchStatus::NoMatches
        } else {
            QuickSearchStatus::Searching
        };
        let input = if files {
            self.file_browser_search_input.clone()
        } else {
            self.branch_filter_input.clone()
        };
        let options = if files {
            self.file_search_options
        } else {
            self.branch_search_options
        };
        let bar = QuickSearchBar::new(
            if files {
                "file_search"
            } else {
                "branch_filter"
            },
            status,
        )
        .sidebar()
        .input(input)
        .options(options, move |this: &mut Self, options, _, cx| {
            if files {
                this.file_search_options = options;
                this.file_browser_scroll
                    .scroll_to_item(0, gpui::ScrollStrategy::Top);
            } else {
                this.branch_search_options = options;
                this.sticky_context = None;
                this.pending_sidebar_navigation = None;
                this.sidebar_scroll_animation = None;
                this.branches_scroll
                    .scroll_to_item(0, gpui::ScrollStrategy::Top);
                this.sync_popover_branch_filter(cx);
            }
            cx.notify();
        })
        .on_clear(move |this, _, cx| this.clear_sidebar_search(files, cx))
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
                    if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                        if files {
                            this.file_search_open = false;
                        } else {
                            this.branch_search_open = false;
                        }
                        this.clear_sidebar_search(files, cx);
                        this.focus_after_search(window, cx);
                        cx.stop_propagation();
                    }
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

    #[gpui::test]
    fn sidebar_files_search_shares_options_and_clears_on_close(cx: &mut gpui::TestAppContext) {
        let _guard = crate::test_support::lock_visual_test();
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
            })
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_files_search").is_none());
        let toggle = cx.debug_bounds("sidebar_search_toggle").unwrap();
        cx.simulate_click(toggle.center(), gpui::Modifiers::default());
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
        let clear = cx.debug_bounds("file_search_clear").unwrap();
        cx.simulate_click(clear.center(), gpui::Modifiers::default());
        test_support::drain_store_worker(&view, cx);
        cx.update(|_, app| {
            view.update(app, |view, cx| test_support::sync_store_snapshot(view, cx))
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_files_search").is_some());
        cx.update(|window, app| {
            assert!(
                pane.read(app)
                    .active_repo()
                    .unwrap()
                    .file_browser
                    .search_query
                    .is_empty()
            );
            window.focus(
                &pane
                    .read(app)
                    .file_browser_search_input
                    .read(app)
                    .focus_handle(),
                app,
            );
        });
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
        let _guard = crate::test_support::lock_visual_test();
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
            })
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("sidebar_branches_search").is_none());
        let button = cx.debug_bounds("sidebar_search_toggle").unwrap();
        cx.simulate_click(button.center(), gpui::Modifiers::default());
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
        let _guard = crate::test_support::lock_visual_test();
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
                let bar = cx.debug_bounds("sidebar_branches_search").unwrap();
                let panel = cx.debug_bounds("sidebar_pane").unwrap();
                let mut previous_right = panel.left();
                for id in [
                    "sidebar_tab_branches",
                    "sidebar_tab_files",
                    "sidebar_search_toggle",
                    "sidebar_locate_active_branch",
                ] {
                    let bounds = cx.debug_bounds(id).unwrap();
                    assert!(
                        bounds.left() >= previous_right && bounds.right() <= panel.right(),
                        "{id} must fit the header at {density:?}/{percent}: {bounds:?}, {panel:?}"
                    );
                    previous_right = bounds.right();
                }
                let input = cx.debug_bounds("branch_filter_input_slot").unwrap();
                assert!(input.size.width > px(80.0));
                for id in [
                    "branch_filter_match_case",
                    "branch_filter_whole_word",
                    "branch_filter_regex",
                    "branch_filter_clear",
                    "branch_filter_match_label",
                ] {
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
        let _guard = crate::test_support::lock_visual_test();
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
