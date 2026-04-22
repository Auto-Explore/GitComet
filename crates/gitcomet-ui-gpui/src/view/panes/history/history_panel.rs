use super::super::super::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::HistoryView;

impl Render for HistoryView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.last_window_size = window.viewport_size();
        self.history_view_inner(cx)
    }
}

impl HistoryView {
    fn history_view_inner(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        self.ensure_history_cache(cx);
        self.drive_pending_history_reveal(cx);
        let (show_working_tree_summary_row, _) = self.ensure_history_worktree_summary_cache();
        let repo = self.active_repo();
        let commits_count = self
            .history_cache
            .as_ref()
            .map(|c| c.visible_indices.len())
            .unwrap_or(0);
        let count = commits_count + usize::from(show_working_tree_summary_row);

        let bg = theme.colors.window_bg;

        let body: AnyElement = if count == 0 {
            match repo.map(|r| &r.log) {
                None => {
                    components::empty_state(theme, "History", "No repository.").into_any_element()
                }
                Some(Loadable::Loading) => {
                    components::empty_state(theme, "History", "Loading").into_any_element()
                }
                Some(Loadable::Error(e)) => {
                    components::empty_state(theme, "History", e.clone()).into_any_element()
                }
                Some(Loadable::NotLoaded) | Some(Loadable::Ready(_)) => {
                    components::empty_state(theme, "History", "No commits.").into_any_element()
                }
            }
        } else {
            let list = uniform_list(
                "history_main",
                count,
                cx.processor(Self::render_history_table_rows),
            )
            .h_full()
            .track_scroll(&self.history_scroll);
            let should_load_more = {
                let state = self.history_scroll.0.borrow();
                let scroll_handle = state.base_handle.clone();
                let max_offset = scroll_handle.max_offset().y.max(px(0.0));
                let should_load_by_scroll = if max_offset > px(0.0) {
                    scroll_is_near_bottom(&scroll_handle, px(240.0))
                } else {
                    true
                };

                state.last_item_size.is_some()
                    && repo.is_some_and(|repo| {
                        !repo.log_loading_more
                            && matches!(
                                &repo.log,
                                Loadable::Ready(page) if page.next_cursor.is_some()
                            )
                    })
                    && should_load_by_scroll
            };
            if should_load_more && let Some(repo_id) = self.active_repo_id() {
                self.store.dispatch(Msg::LoadMoreHistory { repo_id });
            }
            let scrollbar_gutter = components::Scrollbar::visible_gutter(
                self.history_scroll.clone(),
                components::ScrollbarAxis::Vertical,
            );
            div()
                .id("history_main_scroll_container")
                .relative()
                .h_full()
                .child(
                    div()
                        .h_full()
                        .min_h(px(0.0))
                        .pr(scrollbar_gutter)
                        .child(list),
                )
                .child(
                    components::Scrollbar::new(
                        "history_main_scrollbar",
                        self.history_scroll.clone(),
                    )
                    .render(theme),
                )
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .h_full()
            .min_h(px(0.0))
            .bg(bg)
            .track_focus(&self.history_panel_focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    window.focus(&this.history_panel_focus_handle, cx);
                }),
            )
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _window, cx| {
                let key = e.keystroke.key.as_str();
                let mods = e.keystroke.modifiers;

                let handled = !mods.control
                    && !mods.alt
                    && !mods.platform
                    && !mods.function
                    && !mods.shift
                    && match key {
                        "up" => this.history_select_adjacent_commit(-1, cx),
                        "down" => this.history_select_adjacent_commit(1, cx),
                        _ => false,
                    };

                if handled {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                self.history_column_headers(cx)
                    .bg(bg)
                    .border_b_1()
                    .border_color(theme.colors.border),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(div().flex_1().min_h(px(0.0)).child(body)),
            )
    }

    fn history_select_adjacent_commit(
        &mut self,
        direction: i8,
        _cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };

        let (show_working_tree_summary_row, _) = self.ensure_history_worktree_summary_cache();
        let offset = usize::from(show_working_tree_summary_row);

        let (selected_commit, page, log_rev, stashes_rev, history_scope) = match self.active_repo()
        {
            Some(repo) => {
                let page = match &repo.log {
                    Loadable::Ready(page) => Arc::clone(page),
                    _ => return false,
                };
                (
                    repo.history_state.selected_commit.clone(),
                    page,
                    repo.log_rev,
                    repo.stashes_rev,
                    repo.history_state.history_scope,
                )
            }
            None => return false,
        };

        let cache = self
            .history_cache
            .as_ref()
            .filter(|c| c.request.repo_id == repo_id);
        let Some(cache) = cache else {
            return false;
        };

        let total_commits = cache.visible_indices.len();
        if total_commits == 0 {
            return false;
        }

        let list_len = total_commits + offset;

        let current_list_ix = super::resolve_history_selected_list_index(
            &mut self.history_selected_list_index_cache,
            repo_id,
            log_rev,
            stashes_rev,
            history_scope,
            show_working_tree_summary_row,
            selected_commit.as_ref(),
            &cache.visible_indices,
            &page.commits,
        );

        let next_list_ix = match (current_list_ix, direction.is_negative()) {
            (Some(current_list_ix), true) => current_list_ix.saturating_sub(1),
            (Some(current_list_ix), false) => {
                let next = current_list_ix + 1;
                if next < list_len {
                    next
                } else {
                    current_list_ix
                }
            }
            (None, true) => list_len.saturating_sub(1),
            (None, false) => offset,
        };

        if current_list_ix.is_some_and(|ix| ix == next_list_ix) {
            return true;
        }

        if show_working_tree_summary_row && next_list_ix == 0 {
            self.store.dispatch(Msg::ClearCommitSelection { repo_id });
            self.store.dispatch(Msg::ClearDiffSelection { repo_id });
            super::set_history_selected_list_index_cache(
                &mut self.history_selected_list_index_cache,
                repo_id,
                log_rev,
                stashes_rev,
                history_scope,
                show_working_tree_summary_row,
                None,
                0,
            );
            self.history_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Center);
            return true;
        }

        let visible_ix = next_list_ix.saturating_sub(offset);
        let Some(commit_ix) = cache.visible_indices.get(visible_ix) else {
            return false;
        };
        let Some(commit) = page.commits.get(commit_ix) else {
            return false;
        };

        self.store.dispatch(Msg::SelectCommit {
            repo_id,
            commit_id: commit.id.clone(),
        });
        super::set_history_selected_list_index_cache(
            &mut self.history_selected_list_index_cache,
            repo_id,
            log_rev,
            stashes_rev,
            history_scope,
            show_working_tree_summary_row,
            Some(commit.id.clone()),
            next_list_ix,
        );
        self.history_scroll
            .scroll_to_item_strict(next_list_ix, gpui::ScrollStrategy::Center);
        true
    }

    fn history_column_headers(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let icon_muted = with_alpha(theme.colors.accent, if theme.is_dark { 0.72 } else { 0.82 });
        let (show_graph, show_author, show_date, show_sha) = self.history_visible_columns();
        let col_author = self.history_col_author;
        let col_date = self.history_col_date;
        let col_sha = self.history_col_sha;
        let handle_w = px(HISTORY_COL_HANDLE_PX);
        let handle_half = px(HISTORY_COL_HANDLE_PX / 2.0);
        let cell_pad = handle_half;
        let scope_label: SharedString = self
            .active_repo()
            .map(|r| match r.history_state.history_scope {
                gitcomet_core::domain::LogScope::CurrentBranch => "Current branch".to_string(),
                gitcomet_core::domain::LogScope::AllBranches => "All branches".to_string(),
            })
            .unwrap_or_else(|| "Current branch".to_string())
            .into();
        let scope_repo_id = self.active_repo_id();
        let scope_invoker: SharedString = "history_scope_header".into();
        let scope_anchor_bounds: Rc<RefCell<Option<Bounds<Pixels>>>> = Rc::new(RefCell::new(None));
        let scope_anchor_bounds_for_prepaint = Rc::clone(&scope_anchor_bounds);
        let scope_anchor_bounds_for_click = Rc::clone(&scope_anchor_bounds);
        let scope_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == scope_invoker.as_ref());

        let resize_handle = |id: &'static str, handle: HistoryColResizeHandle| {
            div()
                .id(id)
                .absolute()
                .w(handle_w)
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .justify_center()
                .cursor(CursorStyle::ResizeLeftRight)
                .hover(move |s| s.bg(theme.colors.hover))
                .active(move |s| s.bg(theme.colors.active))
                .child(div().w(px(1.0)).h(px(14.0)).bg(theme.colors.border))
                .on_drag(handle, |_handle, _offset, _window, cx| {
                    cx.new(|_cx| HistoryColResizeDragGhost)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                        cx.stop_propagation();
                        if handle == HistoryColResizeHandle::Graph {
                            this.history_col_graph_auto = false;
                        }
                        let available_width = this.history_content_width;
                        let drag_layout = super::HistoryColumnDragLayout {
                            show_graph: this.history_show_graph,
                            show_author: this.history_show_author,
                            show_date: this.history_show_date,
                            show_sha: this.history_show_sha,
                            branch_w: this.history_col_branch,
                            graph_w: this.history_col_graph,
                            author_w: this.history_col_author,
                            date_w: this.history_col_date,
                            sha_w: this.history_col_sha,
                        };
                        this.history_col_resize = Some(super::history_column_resize_state(
                            handle,
                            e.position.x,
                            available_width,
                            drag_layout,
                        ));
                        cx.notify();
                    }),
                )
                .on_drag_move(cx.listener(
                    move |this, e: &gpui::DragMoveEvent<HistoryColResizeHandle>, _w, cx| {
                        let Some(mut state) = this.history_col_resize else {
                            return;
                        };
                        if state.handle != *e.drag(cx) {
                            return;
                        }

                        let available_width = this.history_content_width;
                        let next = super::history_column_drag_clamped_width_for_state(
                            &mut state,
                            e.event.position.x,
                            available_width,
                        );
                        let width = this.history_column_width_mut(state.handle);
                        let changed = *width != next;
                        if changed {
                            *width = next;
                        }
                        this.history_col_resize = Some(state);
                        if changed {
                            cx.notify();
                        }
                    },
                ))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _e, _w, cx| {
                        this.history_col_resize = None;
                        cx.notify();
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _e, _w, cx| {
                        this.history_col_resize = None;
                        cx.notify();
                    }),
                )
        };

        let mut header = div()
            .relative()
            .flex()
            .h(px(24.0))
            .w_full()
            .items_center()
            .px_2()
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .text_color(theme.colors.text_muted)
            .child(
                div()
                    .w(self.history_col_branch)
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_w(px(0.0))
                    .px(cell_pad)
                    .overflow_hidden()
                    .child(
                        div()
                            .on_children_prepainted(move |children_bounds, _w, _cx| {
                                if let Some(bounds) = children_bounds.first() {
                                    *scope_anchor_bounds_for_prepaint.borrow_mut() = Some(*bounds);
                                }
                            })
                            .child(
                                div()
                                    .id("history_scope_header")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_1()
                                    .h(px(18.0))
                                    .line_height(px(18.0))
                                    .rounded(px(theme.radii.row))
                                    .when(scope_active, |d| d.bg(theme.colors.active))
                                    .hover(move |s| {
                                        if scope_active {
                                            s.bg(theme.colors.active)
                                        } else {
                                            s.bg(with_alpha(theme.colors.hover, 0.55))
                                        }
                                    })
                                    .active(move |s| s.bg(theme.colors.active))
                                    .cursor(CursorStyle::PointingHand)
                                    .child(
                                        div()
                                            .min_w(px(0.0))
                                            .line_clamp(1)
                                            .whitespace_nowrap()
                                            .child(scope_label.clone()),
                                    )
                                    .child(svg_icon("icons/chevron_down.svg", icon_muted, px(12.0)))
                                    .when_some(scope_repo_id, |this, repo_id| {
                                        let scope_invoker = scope_invoker.clone();
                                        let scope_anchor_bounds_for_click =
                                            Rc::clone(&scope_anchor_bounds_for_click);
                                        this.on_click(cx.listener(
                                            move |this, e: &ClickEvent, window, cx| {
                                                this.activate_context_menu_invoker(
                                                    scope_invoker.clone(),
                                                    cx,
                                                );
                                                if let Some(bounds) =
                                                    *scope_anchor_bounds_for_click.borrow()
                                                {
                                                    this.open_popover_for_bounds(
                                                        PopoverKind::HistoryBranchFilter {
                                                            repo_id,
                                                        },
                                                        bounds,
                                                        window,
                                                        cx,
                                                    );
                                                } else {
                                                    this.open_popover_at(
                                                        PopoverKind::HistoryBranchFilter {
                                                            repo_id,
                                                        },
                                                        e.position(),
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            },
                                        ))
                                    })
                                    .when(scope_repo_id.is_none(), |this| {
                                        this.opacity(0.6).cursor(CursorStyle::Arrow)
                                    })
                                    .gitcomet_tooltip(
                                        theme,
                                        "History scope (Current branch / All branches)".into(),
                                    ),
                            ),
                    ),
            )
            .when(show_graph, |header| {
                header.child(
                    div()
                        .w(self.history_col_graph)
                        .flex()
                        .justify_center()
                        .px(cell_pad)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child("GRAPH"),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .px(cell_pad)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child("COMMIT MESSAGE"),
                    ),
            )
            .when(show_author, |header| {
                header.child(
                    div()
                        .w(col_author)
                        .flex()
                        .items_center()
                        .justify_end()
                        .px(cell_pad)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child("AUTHOR"),
                )
            });

        if show_date {
            header = header.child(
                div()
                    .w(col_date)
                    .flex()
                    .items_center()
                    .justify_end()
                    .px(cell_pad)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .font_family(UI_MONOSPACE_FONT_FAMILY)
                    .child("Commit date"),
            );
        }

        if show_sha {
            header = header.child(
                div()
                    .w(col_sha)
                    .flex()
                    .items_center()
                    .justify_end()
                    .px(cell_pad)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .font_family(UI_MONOSPACE_FONT_FAMILY)
                    .child("SHA"),
            );
        }

        let mut header_with_handles = header.child(
            resize_handle("history_col_resize_branch", HistoryColResizeHandle::Branch)
                .left((self.history_col_branch - handle_half).max(px(0.0))),
        );

        if show_graph {
            header_with_handles = header_with_handles.child(
                resize_handle("history_col_resize_graph", HistoryColResizeHandle::Graph).left(
                    (self.history_col_branch + self.history_col_graph - handle_half).max(px(0.0)),
                ),
            );
        }

        if show_author {
            let right_fixed = col_author
                + if show_date { col_date } else { px(0.0) }
                + if show_sha { col_sha } else { px(0.0) };
            header_with_handles = header_with_handles.child(
                resize_handle("history_col_resize_author", HistoryColResizeHandle::Author)
                    .right((right_fixed - handle_half).max(px(0.0))),
            );
        }

        if show_date {
            let right_fixed = col_date + if show_sha { col_sha } else { px(0.0) };
            header_with_handles = header_with_handles.child(
                resize_handle("history_col_resize_date", HistoryColResizeHandle::Date)
                    .right((right_fixed - handle_half).max(px(0.0))),
            );
        }

        if show_sha {
            header_with_handles = header_with_handles.child(
                resize_handle("history_col_resize_sha", HistoryColResizeHandle::Sha)
                    .right((col_sha - handle_half).max(px(0.0))),
            );
        }

        header_with_handles
    }
}
