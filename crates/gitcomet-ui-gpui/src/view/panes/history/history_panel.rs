use super::super::super::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::HistoryView;
use crate::view::caches::HistoryListRow;

impl Render for HistoryView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.last_window_size = window.viewport_size();
        self.history_view_inner(cx)
    }
}

impl HistoryView {
    pub(super) fn dismiss_history_refs_hover(&self, cx: &mut gpui::Context<Self>) {
        let root_view = self.root_view.clone();
        // History reveal completion can run while GitCometView is already inside a root update.
        // Defer hover dismissal so GPUI does not attempt to lease the root view twice.
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.dismiss_history_refs_menus(cx);
            });
        });
    }

    fn history_view_inner(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let scrollbar_gutter = super::history_scrollbar_gutter();
        self.apply_pending_history_cache();
        self.ensure_history_cache(cx);
        self.ensure_relative_time_tick(cx);
        self.drive_pending_history_reveal(cx);
        let plan = self.ensure_history_list_plan();
        self.sync_history_viewport(&plan);
        let repo = self.active_repo();
        let commits_count = self
            .history_cache
            .as_ref()
            .map(|cache| cache.base.visible_indices.len())
            .unwrap_or(0);
        let count = plan.list_len(commits_count);

        let bg = theme.colors.surface.canvas;

        let body: AnyElement = if count == 0 {
            match repo.map(|r| &r.log) {
                None => {
                    components::empty_state(theme, "History", "No repository.").into_any_element()
                }
                Some(Loadable::Loading) => div().into_any_element(),
                Some(Loadable::Error(e)) => {
                    components::empty_state(theme, "History", e.clone()).into_any_element()
                }
                Some(Loadable::NotLoaded) | Some(Loadable::Ready(_)) => {
                    components::empty_state(theme, "History", "No commits.").into_any_element()
                }
            }
        } else {
            let root_view_for_scroll = self.root_view.clone();
            let list = uniform_list(
                "history_main",
                count,
                cx.processor(Self::render_history_table_rows),
            )
            .h_full()
            .track_scroll(&self.history_scroll)
            .on_scroll_wheel(move |_event, _window, cx| {
                let _ = root_view_for_scroll.update(cx, |root, cx| {
                    root.close_history_refs_hover(cx);
                    // Rows move out from under the pointer while scrolling, so
                    // an open card would end up describing a different commit.
                    root.dismiss_commit_message_hover(cx);
                });
            });
            let list = restrict_scroll_to_vertical_axis(list);
            let should_load_more = {
                let state = self.history_scroll.0.borrow();
                let scroll_handle = state.base_handle.clone();
                // The scroll handle still describes the previous layout here.
                // Use the rows this frame will lay out, and do not paginate a
                // newer source while its graph is still being built.
                let viewport = state
                    .last_item_size
                    .map(|size| size.item.height)
                    .unwrap_or(px(0.0));
                let content = crate::view::rows::history_row_height(self.ui_scale()) * count as f32;
                let max_offset = (content - viewport).max(px(0.0));
                let should_load_by_scroll = -scroll_handle.offset().y + px(240.0) >= max_offset;

                state.last_item_size.is_some()
                    && repo.is_some_and(|repo| {
                        !repo.log_loading_more
                            && matches!(
                                &repo.log,
                                Loadable::Ready(page) if page.next_cursor.is_some()
                                    && self.history_cache.as_ref().is_some_and(|cache| Arc::ptr_eq(&cache.page, page))
                            )
                    })
                    && should_load_by_scroll
            };
            if should_load_more && let Some(repo_id) = self.active_repo_id() {
                self.store.dispatch(Msg::LoadMoreHistory { repo_id });
            }
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
                    .always_visible()
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
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
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
                        "enter" => this.history_open_selected_worktree(cx),
                        _ => false,
                    };

                if handled {
                    cx.stop_propagation();
                    cx.notify();
                    window.refresh();
                }
            }))
            .child(
                div()
                    .w_full()
                    .bg(bg)
                    .border_b_1()
                    .border_color(theme.colors.stroke.subtle)
                    .child(
                        div()
                            .pr(scrollbar_gutter)
                            .child(self.history_column_headers(cx)),
                    ),
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

    /// Enter on a worktree row opens that worktree, matching what clicking its
    /// badge does. Returns `false` for every other selection so the key keeps
    /// falling through to whatever else handles it.
    pub(in crate::view) fn history_open_selected_worktree(
        &mut self,
        _cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(path) = self
            .active_repo()
            .and_then(|repo| repo.history_state.worktree_selection.clone())
        else {
            return false;
        };
        self.store.dispatch(Msg::OpenRepo(path));
        true
    }

    pub(in crate::view) fn history_select_adjacent_commit(
        &mut self,
        direction: i8,
        _cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };

        let plan = self.ensure_history_list_plan();
        let show_working_tree_summary_row = plan.show_working_tree_summary_row();
        let offset = usize::from(show_working_tree_summary_row);

        let (primary_selection, page, log_rev, stashes_rev, history_scope) =
            match self.active_repo() {
                Some(repo) => {
                    let Some(cache) = self
                        .history_cache
                        .as_ref()
                        .filter(|cache| cache.base.request.repo_id == repo.id)
                    else {
                        return false;
                    };
                    let page = Arc::clone(&cache.page);
                    (
                        super::history_primary_selection(repo, show_working_tree_summary_row),
                        page,
                        cache.base.request.log_source as u64,
                        cache.base.request.stashes_rev,
                        cache.base.request.history_scope,
                    )
                }
                None => return false,
            };

        let cache = self
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request.repo_id == repo_id);
        let Some(cache) = cache else {
            return false;
        };

        let total_commits = cache.base.visible_indices.len();
        if total_commits == 0 {
            return false;
        }

        let list_len = plan.list_len(total_commits);

        let selected_commit = match &primary_selection {
            Some(super::HistoryPrimarySelection::Commit(commit_id)) => Some(commit_id),
            Some(
                super::HistoryPrimarySelection::WorkingTree
                | super::HistoryPrimarySelection::Worktree(_),
            )
            | None => None,
        };
        let selected_worktree = match &primary_selection {
            Some(super::HistoryPrimarySelection::Worktree(path)) => Some(path.clone()),
            Some(
                super::HistoryPrimarySelection::WorkingTree
                | super::HistoryPrimarySelection::Commit(_),
            )
            | None => None,
        };

        let current_list_ix = super::resolve_history_selected_list_index(
            &mut self.history_selected_list_index_cache,
            repo_id,
            log_rev,
            stashes_rev,
            history_scope,
            &plan,
            super::HistorySelectionRef {
                commit: selected_commit,
                worktree_selected: selected_worktree.is_some(),
            },
            &cache.base.visible_indices,
            &page.commits,
        );

        let current_list_ix = selected_worktree
            .as_deref()
            .and_then(|path| super::worktree_row_list_ix(&plan, self.active_repo(), path))
            .or(current_list_ix);

        // A selected worktree with nothing to anchor to -- it went clean, its
        // HEAD left the loaded page, or the scan has not answered yet -- leaves
        // no row to step from. The `None` arms below mean "nothing is selected"
        // and wrap to the far end of the list, which from a live selection reads
        // as the log teleporting rather than moving by one.
        if current_list_ix.is_none() && selected_worktree.is_some() {
            return false;
        }

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

        if let Some(HistoryListRow::WorktreeUncommitted { worktree_ix, .. }) =
            plan.row_at(next_list_ix)
        {
            let path = self
                .active_repo()
                .and_then(|repo| match &repo.worktree_dirty {
                    Loadable::Ready(dirty) => dirty.get(worktree_ix).map(|s| s.path.clone()),
                    _ => None,
                });
            let Some(path) = path else {
                return false;
            };
            self.store
                .dispatch(Msg::SelectWorktreeUncommitted { repo_id, path });
            self.dismiss_history_refs_hover(_cx);
            self.history_scroll
                .scroll_to_item_strict(next_list_ix, gpui::ScrollStrategy::Center);
            return true;
        }
        if show_working_tree_summary_row && next_list_ix == 0 {
            self.select_working_tree_summary_row(repo_id, _cx);
            super::set_history_selected_list_index_cache(
                &mut self.history_selected_list_index_cache,
                repo_id,
                log_rev,
                stashes_rev,
                history_scope,
                &plan,
                None,
                0,
            );
            return true;
        }

        let Some(HistoryListRow::Commit { visible_ix }) = plan.row_at(next_list_ix) else {
            return false;
        };
        let Some(commit_ix) = cache.base.visible_indices.get(visible_ix) else {
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
            &plan,
            Some(commit.id.clone()),
            next_list_ix,
        );
        self.dismiss_history_refs_hover(_cx);
        self.history_scroll
            .scroll_to_item_strict(next_list_ix, gpui::ScrollStrategy::Center);
        true
    }

    fn history_column_headers(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let scaled_px = |value| ui_scale::design_px_from_percent(value, self.ui_scale_percent);
        let icon_muted = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.72 } else { 0.82 },
        );
        let (show_graph, show_author, show_date, show_sha) = self.history_visible_columns();
        let col_author = self.history_col_author;
        let col_date = self.history_col_date;
        let col_sha = self.history_col_sha;
        let handle_w = scaled_px(HISTORY_COL_HANDLE_PX);
        let handle_half = scaled_px(HISTORY_COL_HANDLE_PX / 2.0);
        let cell_pad = handle_half;
        let scope_label: SharedString = self
            .active_repo()
            .map(|r| {
                crate::view::history_mode::history_mode_label(r.history_state.history_scope)
                    .to_string()
            })
            .unwrap_or_else(|| {
                crate::view::history_mode::history_mode_label(
                    gitcomet_core::domain::HistoryMode::default(),
                )
                .to_string()
            })
            .into();
        let scope_repo_id = self.active_repo_id();
        let scope_invoker: SharedString = "history_mode_header".into();
        let scope_anchor_bounds: Rc<RefCell<Option<Bounds<Pixels>>>> = Rc::new(RefCell::new(None));
        let scope_anchor_bounds_for_prepaint = Rc::clone(&scope_anchor_bounds);
        let scope_anchor_bounds_for_click = Rc::clone(&scope_anchor_bounds);
        let scope_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == scope_invoker.as_ref());
        let author_label: SharedString = self
            .active_repo()
            .and_then(|r| r.history_state.history_author_filter.clone())
            .unwrap_or_else(|| "Author".to_string())
            .into();
        let author_invoker: SharedString = "history_author_filter_header".into();
        let author_anchor_bounds: Rc<RefCell<Option<Bounds<Pixels>>>> = Rc::new(RefCell::new(None));
        let author_anchor_bounds_for_prepaint = Rc::clone(&author_anchor_bounds);
        let author_anchor_bounds_for_click = Rc::clone(&author_anchor_bounds);
        let author_filter_active = self
            .active_repo()
            .is_some_and(|r| r.history_state.history_author_filter.is_some());
        let author_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == author_invoker.as_ref());
        // The names on offer come from the commits loaded so far, not from the
        // whole repository, so say where they are from — otherwise an author who
        // has not been paged in yet looks like an author who does not exist. Kept
        // to one line: the tooltip bubble shapes its text as a single run.
        let author_tooltip: SharedString = self
            .active_repo()
            .and_then(|r| r.history_state.history_author_filter.clone())
            .map(|name| format!("Author filter: {name} — suggestions from loaded history"))
            .unwrap_or_else(|| {
                "Filter history by author — suggestions from loaded history".to_string()
            })
            .into();

        let ui_scale_percent = self.ui_scale_percent;
        let active_col_resize = self.history_col_resize;
        let resize_handle = |id: &'static str, handle: HistoryColResizeHandle| {
            let dragging = active_col_resize.is_some_and(|state| state.handle == handle);
            div()
                .id(id)
                .group(id)
                .absolute()
                .w(handle_w)
                .top_0()
                .bottom_0()
                .cursor(CursorStyle::ResizeLeftRight)
                .child(components::resize_grip(
                    theme,
                    ui_scale_percent,
                    id,
                    components::ResizeGripAxis::Vertical,
                    dragging,
                    Some(theme.colors.stroke.subtle),
                ))
                .on_drag(handle, |_handle, _offset, _window, cx| {
                    cx.new(|_cx| HistoryColResizeDragGhost)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                        cx.stop_propagation();
                        crate::press_gesture::claim_press(cx);
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
                            this.ui_scale_percent,
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
                            this.ui_scale_percent,
                        );
                        let width = this.history_column_width_mut(state.handle);
                        let changed = *width != next;
                        if changed {
                            *width = next;
                            this.sync_history_column_design_widths_from_pixels();
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
            .h(scaled_px(24.0))
            .w_full()
            .items_center()
            .px_2()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.colors.foreground.secondary)
            .child(
                div()
                    .w(self.history_col_branch)
                    .flex_none()
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
                                    .id("history_mode_header")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_1()
                                    .h(scaled_px(18.0))
                                    .line_height(scaled_px(18.0))
                                    .rounded(px(theme.radii.row))
                                    .when(scope_active, |d| {
                                        d.bg(theme.colors.interaction.pressed_background)
                                    })
                                    .hover(move |s| {
                                        if scope_active {
                                            s.bg(theme.colors.interaction.pressed_background)
                                        } else {
                                            s.bg(with_alpha(
                                                theme.colors.interaction.hover_background,
                                                0.55,
                                            ))
                                        }
                                    })
                                    .active(move |s| {
                                        s.bg(theme.colors.interaction.pressed_background)
                                    })
                                    .cursor(CursorStyle::PointingHand)
                                    .child(
                                        div()
                                            .min_w(px(0.0))
                                            .line_clamp(1)
                                            .whitespace_nowrap()
                                            .child(scope_label.clone()),
                                    )
                                    .child(svg_icon(
                                        "icons/chevron_down.svg",
                                        icon_muted,
                                        scaled_px(12.0),
                                    ))
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
                                        crate::view::history_mode::HISTORY_MODE_TOOLTIP_TEXT.into(),
                                    ),
                            ),
                    ),
            )
            .when(show_graph, |header| {
                // The graph column explains itself; a header label only adds noise.
                header.child(
                    div()
                        .w(self.history_col_graph)
                        .flex_none()
                        .px(cell_pad)
                        .overflow_hidden(),
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
                            .child("MESSAGE"),
                    ),
            )
            .when(show_author, |header| {
                header.child(
                    div()
                        .w(col_author)
                        .flex_none()
                        .flex()
                        .items_center()
                        // Clear the column resize handle straddling the left
                        // boundary so the label never sits under it.
                        .pl(handle_half + cell_pad)
                        .pr(cell_pad)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(
                            div()
                                .on_children_prepainted(move |children_bounds, _w, _cx| {
                                    if let Some(bounds) = children_bounds.first() {
                                        *author_anchor_bounds_for_prepaint.borrow_mut() =
                                            Some(*bounds);
                                    }
                                })
                                .child(
                                    div()
                                        .id("history_author_filter_header")
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_1()
                                        .h(scaled_px(18.0))
                                        .line_height(scaled_px(18.0))
                                        .rounded(px(theme.radii.row))
                                        .when(author_active, |d| {
                                            d.bg(theme.colors.interaction.pressed_background)
                                        })
                                        .hover(move |s| {
                                            if author_active {
                                                s.bg(theme.colors.interaction.pressed_background)
                                            } else {
                                                s.bg(with_alpha(
                                                    theme.colors.interaction.hover_background,
                                                    0.55,
                                                ))
                                            }
                                        })
                                        .active(move |s| {
                                            s.bg(theme.colors.interaction.pressed_background)
                                        })
                                        .cursor(CursorStyle::PointingHand)
                                        .child(
                                            div()
                                                .min_w(px(0.0))
                                                .line_clamp(1)
                                                .whitespace_nowrap()
                                                .when(author_filter_active, |d| {
                                                    d.text_color(theme.colors.accent.foreground)
                                                })
                                                .when(!author_filter_active, |d| {
                                                    d.text_color(theme.colors.foreground.secondary)
                                                })
                                                .child(author_label.clone()),
                                        )
                                        .child(svg_icon(
                                            "icons/chevron_down.svg",
                                            icon_muted,
                                            scaled_px(12.0),
                                        ))
                                        .when_some(scope_repo_id, |this, repo_id| {
                                            let author_invoker = author_invoker.clone();
                                            let author_anchor_bounds_for_click =
                                                Rc::clone(&author_anchor_bounds_for_click);
                                            this.on_click(cx.listener(
                                                move |this, e: &ClickEvent, window, cx| {
                                                    this.activate_context_menu_invoker(
                                                        author_invoker.clone(),
                                                        cx,
                                                    );
                                                    if let Some(bounds) =
                                                        *author_anchor_bounds_for_click.borrow()
                                                    {
                                                        this.open_popover_for_bounds(
                                                            PopoverKind::HistoryAuthorFilter {
                                                                repo_id,
                                                            },
                                                            bounds,
                                                            window,
                                                            cx,
                                                        );
                                                    } else {
                                                        this.open_popover_at(
                                                            PopoverKind::HistoryAuthorFilter {
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
                                        .gitcomet_tooltip(theme, author_tooltip),
                                ),
                        ),
                )
            });

        if show_date {
            header = header.child(
                div()
                    .w(col_date)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .px(cell_pad)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child("DATE"),
            );
        }

        if show_sha {
            header = header.child(
                div()
                    .w(col_sha)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .px(cell_pad)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child("SHA"),
            );
        }

        // Absolute insets resolve against the header's padding box, while the
        // cells start one `.px_2()` (0.5 rem = 8 design px) further in — the
        // same inset the row canvas applies. Without this correction every
        // handle renders 8px off its column boundary (the author handle's
        // hairline used to touch the AUTHOR label).
        let cell_edge_pad = scaled_px(8.0);

        let mut header_with_handles = header.child(
            resize_handle("history_col_resize_branch", HistoryColResizeHandle::Branch)
                .left((cell_edge_pad + self.history_col_branch - handle_half).max(px(0.0))),
        );

        if show_graph {
            header_with_handles = header_with_handles.child(
                resize_handle("history_col_resize_graph", HistoryColResizeHandle::Graph).left(
                    (cell_edge_pad + self.history_col_branch + self.history_col_graph
                        - handle_half)
                        .max(px(0.0)),
                ),
            );
        }

        if show_author {
            let right_fixed = col_author
                + if show_date { col_date } else { px(0.0) }
                + if show_sha { col_sha } else { px(0.0) };
            header_with_handles = header_with_handles.child(
                resize_handle("history_col_resize_author", HistoryColResizeHandle::Author)
                    .right((cell_edge_pad + right_fixed - handle_half).max(px(0.0))),
            );
        }

        if show_date {
            let right_fixed = col_date + if show_sha { col_sha } else { px(0.0) };
            header_with_handles = header_with_handles.child(
                resize_handle("history_col_resize_date", HistoryColResizeHandle::Date)
                    .right((cell_edge_pad + right_fixed - handle_half).max(px(0.0))),
            );
        }

        if show_sha {
            header_with_handles = header_with_handles.child(
                resize_handle("history_col_resize_sha", HistoryColResizeHandle::Sha)
                    .right((cell_edge_pad + col_sha - handle_half).max(px(0.0))),
            );
        }

        header_with_handles
    }
}
