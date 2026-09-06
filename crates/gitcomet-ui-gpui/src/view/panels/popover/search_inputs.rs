use super::picker_nav::{PickerNavKeys, PickerNavOutcome, PickerNavRows, handle_picker_nav};
use super::*;

impl PopoverHost {
    fn ensure_search_input_entity(
        slot: &mut Option<Entity<components::TextInput>>,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        slot.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: placeholder.into(),
                        ..Default::default()
                    },
                    window,
                    cx,
                )
            })
        })
        .clone()
    }

    /// Resets the search input (text, theme, pending key presses), rewinds the
    /// picker scroll position, and focuses the input.
    fn reset_picker_search_input(
        &self,
        input: &Entity<components::TextInput>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let theme = self.theme;
        input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
    }

    /// Scrolls a windowed picker's row list so the selected row is in view.
    ///
    /// These lists are windowed once they grow past a couple of viewports, so a
    /// row further down has no element for `ScrollHandle::scroll_to_item` to
    /// find; the row geometry says where it would be instead.
    ///
    /// `viewport_px` must be the same height the panel gave the picker as its
    /// `max_height`, since that is the viewport the window was built for.
    fn scroll_picker_prompt_to_row(
        &self,
        geometry: &components::PickerPromptGeometry,
        sel: usize,
        viewport_px: f32,
        cx: &mut gpui::Context<Self>,
    ) {
        let ui_scale = super::popover_ui_scale(cx);
        let viewport = ui_scale.px(viewport_px);
        let current = -self.picker_prompt_scroll.offset().y;
        let offset = geometry.reveal_offset(sel, viewport, current);
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), -offset));
    }

    /// Scrolls the history author dropdown so displayed row `sel` is in view.
    ///
    /// Its list is windowed like the badge pickers', so the row may not have
    /// been built; the geometry says where it would be.
    pub(super) fn scroll_history_author_filter_to_row(
        &mut self,
        sel: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::HistoryAuthorFilter { repo_id }) = self.popover else {
            return;
        };
        let query = self
            .history_author_filter_search_input
            .as_ref()
            .map(|input| input.read(cx).text().trim().to_string())
            .unwrap_or_default();
        let (items, layout) = author_filter::rendered_rows(self, repo_id, &query);
        self.scroll_picker_prompt_to_row(
            &components::PickerPromptGeometry::new(&items, &layout, super::popover_ui_scale(cx)),
            sel,
            components::PICKER_LIST_MAX_HEIGHT_PX,
            cx,
        );
    }

    /// Shared keyboard-navigation subscription for picker search inputs.
    ///
    /// `is_active` gates the subscription to the picker's popover kind.
    /// `items` returns the filtered list the picker currently displays for
    /// `query` (or `None` while the underlying data is still loading), so the
    /// selected index and the Enter target can't drift apart. `on_enter`
    /// receives the selected payload (if any) plus the raw query.
    #[allow(clippy::too_many_arguments)]
    fn picker_search_subscription<R: PickerNavRows>(
        input: &Entity<components::TextInput>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
        is_active: fn(&Self) -> bool,
        selected_index: fn(&mut Self) -> &mut Option<usize>,
        items: impl Fn(&mut Self, &str, &mut gpui::Context<Self>) -> Option<R> + 'static,
        on_escape: impl Fn(&mut Self, &mut gpui::Context<Self>) + 'static,
        scroll_to: impl Fn(&mut Self, usize, &mut gpui::Context<Self>) + 'static,
        on_enter: impl Fn(&mut Self, Option<R::Item>, String, &mut Window, &mut gpui::Context<Self>)
        + 'static,
    ) -> gpui::Subscription {
        cx.observe_in(input, window, move |this, input, window, cx| {
            let keys = input.update(cx, |i, _| PickerNavKeys::take(i));

            if !is_active(this) {
                return;
            }

            let query = input.read_with(cx, |i, _| i.text().trim().to_string());
            let Some(list) = items(this, &query, cx) else {
                // Data not ready yet — still let Esc dismiss the picker.
                if keys.escape {
                    on_escape(this, cx);
                }
                return;
            };

            let page_rows = if keys.page_up || keys.page_down {
                let ui_scale = super::popover_ui_scale(cx);
                let two_line = branch_picker::is_checkout_picker(this)
                    || workspace_picker_state(this).is_some()
                    || upstream_picker_state(this).is_some()
                    || matches!(this.popover, Some(PopoverKind::FileHistory { .. }));
                let row_height = components::picker_row_height(ui_scale, two_line);
                let viewport = this.picker_prompt_scroll.bounds().size.height;
                let viewport: f32 = viewport.into();
                let row_height: f32 = row_height.into();
                (viewport / row_height).floor().max(1.0) as usize
            } else {
                1
            };
            match handle_picker_nav(&keys, selected_index(this), list.len(), page_rows) {
                PickerNavOutcome::Escape => {
                    on_escape(this, cx);
                    return;
                }
                PickerNavOutcome::Navigated => {
                    if let Some(sel) = *selected_index(this) {
                        scroll_to(this, sel, cx);
                    }
                    cx.notify();
                    return;
                }
                PickerNavOutcome::Enter => {
                    // Clamp exactly the way `PickerPrompt::render` does. Typing
                    // can shrink the filtered list below a previously chosen
                    // index; without this the highlighted row (clamped) and the
                    // Enter target (unclamped) disagree and Enter silently
                    // does nothing.
                    let payload = (*selected_index(this))
                        .filter(|_| !list.is_empty())
                        .map(|sel| sel.min(list.len() - 1))
                        .and_then(|sel| list.get(sel));
                    on_enter(this, payload, query, window, cx);
                }
                PickerNavOutcome::Idle => {}
            }
            cx.notify();
        })
    }

    pub(super) fn ensure_repo_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let input = Self::ensure_search_input_entity(
            &mut self.repo_picker_search_input,
            "Filter repositories",
            window,
            cx,
        );
        if self._repo_picker_search_input_subscription.is_none() {
            self._repo_picker_search_input_subscription = Some(Self::picker_search_subscription(
                &input,
                window,
                cx,
                |this| matches!(this.popover, Some(PopoverKind::RepoPicker)),
                |this| &mut this.repo_picker_selected_index,
                // Navigation walks the same filtered order the picker renders,
                // so Enter can't land on a different repository than the
                // highlighted row — including across all three sections. While
                // the sort menu covers the list, it walks the sort options
                // instead.
                |this, query, cx| {
                    let query_changed = this.repo_picker_search_query != query;
                    if query_changed {
                        this.repo_picker_search_query.clear();
                        this.repo_picker_search_query.push_str(query);
                    }
                    // Editing the filter re-orders the rows a row menu is
                    // floating over, so the menu goes before the targets below
                    // are read — otherwise it keeps the arrow keys while the
                    // list moves under its highlight.
                    picker_row_menu::close_on_query_change(this, query, cx);
                    if query_changed {
                        // Typing into the still-focused input while Sort is open
                        // returns to repository results, just like typing while
                        // a row menu is open dismisses that menu.
                        this.repo_picker_sort_menu_open = false;
                    }

                    let targets = repo_picker::nav_targets(this, query, cx);
                    if query_changed {
                        // A real search always starts at its top-ranked visible
                        // result. Empty and no-match queries remain neutral, and
                        // resetting the scroll keeps row zero and its section
                        // header on screen after navigation deep into the list.
                        this.repo_picker_selected_index =
                            (!query.is_empty() && !targets.is_empty()).then_some(0);
                        this.picker_prompt_scroll
                            .set_offset(point(px(0.0), px(0.0)));
                    }
                    Some(targets)
                },
                repo_picker::dismiss,
                |this, sel, cx| {
                    // Both of these replace the repository rows as the arrow
                    // keys' target, so the selection is not a row index to
                    // scroll to.
                    if this.repo_picker_sort_menu_open || this.picker_row_menu.is_some() {
                        return;
                    }
                    let query = this
                        .repo_picker_search_input
                        .as_ref()
                        .map(|input| input.read(cx).text().trim().to_string())
                        .unwrap_or_default();
                    // The list is windowed, so a row past the viewport has no
                    // element to scroll to by child slot: its geometry says
                    // where it would be. Headers are part of that geometry, so
                    // scrolling to a section's first row shows its header too.
                    let rows = repo_picker::cached(this, &query);
                    this.scroll_picker_prompt_to_row(
                        &rows.geometry(super::popover_ui_scale(cx)),
                        sel,
                        repo_picker::REPO_PICKER_LIST_MAX_HEIGHT_PX,
                        cx,
                    );
                },
                |this, payload, _query, window, cx| {
                    if let Some(target) = payload {
                        repo_picker::activate_nav_target(this, target, window, cx);
                    }
                },
            ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }

    pub(super) fn ensure_branch_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let input = Self::ensure_search_input_entity(
            &mut self.branch_picker_search_input,
            "Filter branches",
            window,
            cx,
        );
        input.update(cx, |input, cx| {
            input.set_chromeless(false, cx);
            input.set_leading_icon(None, cx);
        });
        if self._branch_picker_search_input_subscription.is_none() {
            self._branch_picker_search_input_subscription = Some(Self::picker_search_subscription(
                &input,
                window,
                cx,
                |this| this.inline_branch_picker_active(),
                |this| &mut this.branch_picker_selected_index,
                |this, query, cx| {
                    // A menu floating over a row takes the arrow keys, and an
                    // edit to the filter dismisses it — the rows underneath are
                    // about to be re-filtered out from under its highlight.
                    picker_row_menu::close_on_query_change(this, query, cx);
                    if let Some(actions) = picker_row_menu::nav_actions(this, cx) {
                        return Some(
                            (0..actions.len())
                                .map(branch_picker::BranchPickerNavTarget::RowAction)
                                .collect::<Vec<_>>(),
                        );
                    }
                    // The checkout picker renders sectioned, multi-part rows, so
                    // its nav order must come from the picker's own layout over
                    // the very same items. `match_branches` sorts differently
                    // (no section term, name length rather than row length) and
                    // would make Enter check out a branch other than the
                    // highlighted one.
                    if branch_picker::is_checkout_picker(this) {
                        return Some(branch_picker::nav_targets(this, query));
                    }

                    Some(
                        branch_picker::ref_nav_targets(this, ref_rows_spec(this), query)
                            .into_iter()
                            .map(branch_picker::BranchPickerNavTarget::Ref)
                            .collect::<Vec<_>>(),
                    )
                },
                |this, cx| {
                    // Escape backs out of the menu before it closes the picker.
                    if this.picker_row_menu.is_some() {
                        picker_row_menu::close(this, cx);
                        return;
                    }
                    this.handle_inline_branch_picker_escape(cx)
                },
                |this, sel, cx| {
                    // The selection indexes the open menu's entries, not a row.
                    if this.picker_row_menu.is_some() {
                        return;
                    }
                    let query = this
                        .branch_picker_search_input
                        .as_ref()
                        .map(|input| input.read(cx).text().trim().to_string())
                        .unwrap_or_default();
                    // The checkout picker's sectioned rows and the plain ref
                    // lists are laid out differently, so each scrolls by its own
                    // geometry — and each was built for its own viewport.
                    if branch_picker::is_checkout_picker(this) {
                        let rows = branch_picker::cached(this, &query);
                        this.scroll_picker_prompt_to_row(
                            &rows.geometry(super::popover_ui_scale(cx)),
                            sel,
                            components::PICKER_LIST_MAX_HEIGHT_PX,
                            cx,
                        );
                        return;
                    }
                    let rows = branch_picker::ref_rows_cached(this, ref_rows_spec(this), &query);
                    this.scroll_picker_prompt_to_row(
                        &rows.geometry(super::popover_ui_scale(cx)),
                        sel,
                        branch_picker::REF_PICKER_LIST_MAX_HEIGHT_PX,
                        cx,
                    );
                },
                |this, payload, query, window, cx| {
                    // Enter runs the highlighted menu entry while a menu is up.
                    if let Some(branch_picker::BranchPickerNavTarget::RowAction(ix)) = payload {
                        picker_row_menu::activate_nth(this, ix, window, cx);
                        return;
                    }
                    let Some(repo_id) = this.active_repo().map(|repo| repo.id) else {
                        return;
                    };
                    if branch_picker::is_checkout_picker(this) {
                        // Same as the workspace picker: a typed query plus Enter
                        // must reach the top row (often "Create branch <name>")
                        // without arrowing to it first.
                        let target = payload.or_else(|| {
                            (!query.trim().is_empty())
                                .then(|| {
                                    branch_picker::nav_targets(this, query.trim())
                                        .into_iter()
                                        .next()
                                })
                                .flatten()
                        });
                        if let Some(target) = target {
                            branch_picker::activate(this, repo_id, target, window, cx);
                        }
                        return;
                    }
                    // The prompts that branch from a ref accept a typed name that
                    // matched nothing, so Enter can create one.
                    if ref_rows_spec(this).offers_source_refs() {
                        let name = match payload {
                            Some(branch_picker::BranchPickerNavTarget::Ref(name)) => name,
                            _ => query,
                        };
                        if !name.is_empty() {
                            if matches!(
                                this.popover,
                                Some(PopoverKind::Repo {
                                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                                    ..
                                })
                            ) {
                                this.suppress_worktree_submit_after_ref_enter = true;
                            }
                            this.handle_inline_branch_picker_select(name, repo_id, window, cx);
                        }
                    } else if let Some(branch_picker::BranchPickerNavTarget::Ref(name)) = payload {
                        this.handle_inline_branch_picker_select(name, repo_id, window, cx);
                    }
                },
            ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }

    pub(super) fn ensure_upstream_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let input = Self::ensure_search_input_entity(
            &mut self.remote_picker_search_input,
            "Filter remote branches",
            window,
            cx,
        );
        if self._upstream_picker_search_input_subscription.is_none() {
            self._upstream_picker_search_input_subscription =
                Some(Self::picker_search_subscription(
                    &input,
                    window,
                    cx,
                    |this| upstream_picker_state(this).is_some(),
                    |this| &mut this.upstream_picker_selected_index,
                    |this, query, _cx| {
                        let (repo_id, branch) = upstream_picker_state(this)?;
                        let targets = upstream_picker::nav_targets(this, repo_id, &branch, query);
                        let leading = upstream_picker::leading_action_count(this, repo_id, &branch);
                        upstream_picker::clear_missing_branch_selection(
                            &mut this.upstream_picker_selected_index,
                            targets.len() - leading,
                            leading,
                        );
                        Some(targets)
                    },
                    |this, cx| this.close_popover(cx),
                    |this, sel, cx| {
                        let Some((repo_id, branch)) = upstream_picker_state(this) else {
                            return;
                        };
                        let query = this
                            .remote_picker_search_input
                            .as_ref()
                            .map(|input| input.read(cx).text().trim().to_string())
                            .unwrap_or_default();
                        let rows = upstream_picker::cached(this, repo_id, &branch, &query);
                        // The fixed Create new / unlink action leads the branch
                        // list in the same navigation model and sits outside its
                        // scroll viewport. Translate global navigation indices to
                        // branch-row indices before revealing a row.
                        let Some(branch_sel) = sel.checked_sub(
                            upstream_picker::leading_action_count(this, repo_id, &branch),
                        ) else {
                            return;
                        };
                        if branch_sel >= rows.layout.item_indices.len() {
                            return;
                        }
                        this.scroll_picker_prompt_to_row(
                            &rows.geometry(super::popover_ui_scale(cx)),
                            branch_sel,
                            upstream_picker::UPSTREAM_PICKER_LIST_MAX_HEIGHT_PX,
                            cx,
                        );
                    },
                    |this, payload, _query, window, cx| {
                        let (Some(target), Some((repo_id, branch))) =
                            (payload, upstream_picker_state(this))
                        else {
                            return;
                        };
                        upstream_picker::activate(this, repo_id, &branch, target, window, cx);
                    },
                ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }

    pub(super) fn ensure_worktree_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let input = Self::ensure_search_input_entity(
            &mut self.worktree_picker_search_input,
            "Filter worktrees",
            window,
            cx,
        );
        if self._worktree_picker_search_input_subscription.is_none() {
            self._worktree_picker_search_input_subscription =
                Some(Self::picker_search_subscription(
                    &input,
                    window,
                    cx,
                    |this| worktree_picker_state(this).is_some(),
                    |this| &mut this.worktree_picker_selected_index,
                    |this, query, _cx| {
                        let (repo_id, is_remove) = worktree_picker_state(this)?;
                        Some(worktree_picker::nav_targets(
                            this, repo_id, is_remove, query,
                        ))
                    },
                    |this, cx| this.close_popover(cx),
                    |this, sel, cx| {
                        let Some((repo_id, is_remove)) = worktree_picker_state(this) else {
                            return;
                        };
                        let query = this
                            .worktree_picker_search_input
                            .as_ref()
                            .map(|input| input.read(cx).text().trim().to_string())
                            .unwrap_or_default();
                        let rows = worktree_picker::cached(this, repo_id, is_remove, &query);
                        this.scroll_picker_prompt_to_row(
                            &rows.geometry(super::popover_ui_scale(cx)),
                            sel,
                            worktree_picker::WORKTREE_PICKER_LIST_MAX_HEIGHT_PX,
                            cx,
                        );
                    },
                    |this, payload, _query, window, cx| {
                        let Some(path) = payload else {
                            return;
                        };
                        let Some((repo_id, is_remove)) = worktree_picker_state(this) else {
                            return;
                        };
                        worktree_picker::activate(this, repo_id, is_remove, path, None, window, cx);
                    },
                ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }

    pub(super) fn ensure_workspace_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let input = Self::ensure_search_input_entity(
            &mut self.workspace_picker_search_input,
            "Select or type to create a worktree",
            window,
            cx,
        );
        if self._workspace_picker_search_input_subscription.is_none() {
            self._workspace_picker_search_input_subscription =
                Some(Self::picker_search_subscription(
                    &input,
                    window,
                    cx,
                    |this| workspace_picker_state(this).is_some(),
                    |this| &mut this.workspace_picker_selected_index,
                    |this, query, cx| {
                        // A menu floating over a row takes the arrow keys, and an
                        // edit to the filter dismisses it.
                        picker_row_menu::close_on_query_change(this, query, cx);
                        if let Some(actions) = picker_row_menu::nav_actions(this, cx) {
                            return Some(
                                (0..actions.len())
                                    .map(workspace_picker::WorkspaceRow::RowAction)
                                    .collect::<Vec<_>>(),
                            );
                        }
                        let repo_id = workspace_picker_state(this)?;
                        // Layout-driven so Enter can never land on a different
                        // row than the highlighted one.
                        Some(workspace_picker::nav_targets(this, repo_id, query))
                    },
                    |this, cx| {
                        // Escape backs out of the menu before it closes the picker.
                        if this.picker_row_menu.is_some() {
                            picker_row_menu::close(this, cx);
                            return;
                        }
                        this.close_popover(cx)
                    },
                    |this, sel, cx| {
                        // The selection indexes the open menu's entries, not a row.
                        if this.picker_row_menu.is_some() {
                            return;
                        }
                        let Some(repo_id) = workspace_picker_state(this) else {
                            return;
                        };
                        let query = this
                            .workspace_picker_search_input
                            .as_ref()
                            .map(|input| input.read(cx).text().trim().to_string())
                            .unwrap_or_default();
                        let rows = workspace_picker::cached(this, repo_id, &query);
                        this.scroll_picker_prompt_to_row(
                            &rows.geometry(super::popover_ui_scale(cx)),
                            sel,
                            components::PICKER_LIST_MAX_HEIGHT_PX,
                            cx,
                        );
                    },
                    |this, payload, query, window, cx| {
                        // Enter runs the highlighted menu entry while a menu is up.
                        if let Some(workspace_picker::WorkspaceRow::RowAction(ix)) = payload {
                            picker_row_menu::activate_nth(this, ix, window, cx);
                            return;
                        }
                        let Some(repo_id) = workspace_picker_state(this) else {
                            return;
                        };
                        // "Select or type to create a worktree": after typing,
                        // Enter must act even though nothing was arrowed to.
                        // Only with a query, so a stray Enter on the freshly
                        // opened picker stays inert.
                        let row = payload.or_else(|| {
                            (!query.trim().is_empty())
                                .then(|| {
                                    workspace_picker::nav_targets(this, repo_id, query.trim())
                                        .into_iter()
                                        .next()
                                })
                                .flatten()
                        });
                        let Some(row) = row else {
                            return;
                        };
                        workspace_picker::activate(this, repo_id, row, &query, window, cx);
                    },
                ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }

    pub(super) fn ensure_submodule_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let input = Self::ensure_search_input_entity(
            &mut self.submodule_picker_search_input,
            "Filter submodules",
            window,
            cx,
        );
        if self._submodule_picker_search_input_subscription.is_none() {
            self._submodule_picker_search_input_subscription =
                Some(Self::picker_search_subscription(
                    &input,
                    window,
                    cx,
                    |this| submodule_picker_state(this).is_some(),
                    |this| &mut this.submodule_picker_selected_index,
                    |this, query, _cx| {
                        let (repo_id, _) = submodule_picker_state(this)?;
                        Some(submodule_picker::nav_targets(this, repo_id, query))
                    },
                    |this, cx| this.close_popover(cx),
                    |this, sel, cx| {
                        let Some((repo_id, _)) = submodule_picker_state(this) else {
                            return;
                        };
                        let query = this
                            .submodule_picker_search_input
                            .as_ref()
                            .map(|input| input.read(cx).text().trim().to_string())
                            .unwrap_or_default();
                        let rows = submodule_picker::cached(this, repo_id, &query);
                        this.scroll_picker_prompt_to_row(
                            &rows.geometry(super::popover_ui_scale(cx)),
                            sel,
                            submodule_picker::SUBMODULE_PICKER_LIST_MAX_HEIGHT_PX,
                            cx,
                        );
                    },
                    |this, payload, _query, window, cx| {
                        let Some(path) = payload else {
                            return;
                        };
                        let Some((repo_id, is_remove)) = submodule_picker_state(this) else {
                            return;
                        };
                        submodule_picker::activate(
                            this, repo_id, is_remove, path, None, window, cx,
                        );
                    },
                ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }

    pub(super) fn ensure_stash_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let input = Self::ensure_search_input_entity(
            &mut self.stash_picker_search_input,
            "Filter stashes",
            window,
            cx,
        );
        if self._stash_picker_search_input_subscription.is_none() {
            self._stash_picker_search_input_subscription = Some(Self::picker_search_subscription(
                &input,
                window,
                cx,
                |this| matches!(this.popover, Some(PopoverKind::StashPickerPrompt { .. })),
                |this| &mut this.stash_picker_prompt_selected_index,
                |this, query, _cx| Some(stash_picker_prompt::nav_targets(this, query)),
                |this, cx| this.close_popover(cx),
                |this, sel, cx| {
                    let query = this
                        .stash_picker_search_input
                        .as_ref()
                        .map(|input| input.read(cx).text().trim().to_string())
                        .unwrap_or_default();
                    let rows = stash_picker_prompt::cached(this, &query);
                    this.scroll_picker_prompt_to_row(
                        &rows.geometry(super::popover_ui_scale(cx)),
                        sel,
                        stash_picker_prompt::STASH_PICKER_LIST_MAX_HEIGHT_PX,
                        cx,
                    );
                },
                |this, payload, _query, window, cx| {
                    let Some(row) = payload else {
                        return;
                    };
                    let Some(PopoverKind::StashPickerPrompt { repo_id, purpose }) =
                        this.popover.clone()
                    else {
                        return;
                    };
                    stash_picker_prompt::activate(this, repo_id, purpose, row, window, cx);
                },
            ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }

    pub(super) fn ensure_file_history_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let input = Self::ensure_search_input_entity(
            &mut self.file_history_search_input,
            "Filter commits",
            window,
            cx,
        );
        if self._file_history_search_input_subscription.is_none() {
            self._file_history_search_input_subscription = Some(Self::picker_search_subscription(
                &input,
                window,
                cx,
                |this| matches!(this.popover, Some(PopoverKind::FileHistory { .. })),
                |this| &mut this.file_history_selected_index,
                |this, query, cx| {
                    picker_row_menu::close_on_query_change(this, query, cx);
                    if let Some(actions) = picker_row_menu::nav_actions(this, cx) {
                        return Some(super::picker_nav::IndexedNavRows {
                            len: actions.len(),
                            resolve: Box::new(move |ix| {
                                (ix < actions.len())
                                    .then_some(file_history::NavTarget::RowAction(ix))
                            }),
                        });
                    }
                    Some(file_history::nav_targets(this, query))
                },
                |this, cx| {
                    if this.picker_row_menu.is_some() {
                        picker_row_menu::close(this, cx);
                    } else {
                        this.close_popover(cx);
                    }
                },
                |this, sel, cx| {
                    if this.picker_row_menu.is_some() {
                        return;
                    }
                    let query = this
                        .file_history_search_input
                        .as_ref()
                        .map(|input| input.read(cx).text().trim().to_string())
                        .unwrap_or_default();
                    let rows = file_history::cached(this, &query);
                    this.scroll_picker_prompt_to_row(
                        &rows.geometry(super::popover_ui_scale(cx)),
                        sel,
                        file_history::FILE_HISTORY_LIST_MAX_HEIGHT_PX,
                        cx,
                    );
                },
                |this, payload, _query, window, cx| {
                    if let Some(file_history::NavTarget::RowAction(ix)) = payload {
                        picker_row_menu::activate_nth(this, ix, window, cx);
                        return;
                    }
                    let Some(file_history::NavTarget::Commit(commit_id)) = payload else {
                        return;
                    };
                    let Some(PopoverKind::FileHistory { repo_id, path }) = this.popover.clone()
                    else {
                        return;
                    };
                    file_history::open_at_commit(this, repo_id, commit_id, path, cx);
                },
            ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }

    pub(super) fn ensure_history_author_filter_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let input = Self::ensure_search_input_entity(
            &mut self.history_author_filter_search_input,
            "Filter authors",
            window,
            cx,
        );
        if self
            ._history_author_filter_search_input_subscription
            .is_none()
        {
            self._history_author_filter_search_input_subscription =
                Some(Self::picker_search_subscription(
                    &input,
                    window,
                    cx,
                    |this| matches!(this.popover, Some(PopoverKind::HistoryAuthorFilter { .. })),
                    |this| &mut this.history_author_filter_selected_index,
                    |this, query, _cx| {
                        let Some(PopoverKind::HistoryAuthorFilter { repo_id }) = &this.popover
                        else {
                            return None;
                        };
                        let repo_id = *repo_id;
                        Some(author_filter::nav_targets(this, repo_id, query))
                    },
                    |this, cx| this.close_popover(cx),
                    Self::scroll_history_author_filter_to_row,
                    |this, payload, query, _window, cx| {
                        let Some(PopoverKind::HistoryAuthorFilter { repo_id }) = this.popover
                        else {
                            return;
                        };
                        // Suggestions only cover the authors of the commits
                        // loaded so far, and the backend filter is a
                        // case-insensitive substring match, so a name that is
                        // not in the list is still worth applying as typed.
                        let target = match payload {
                            Some(target) => target,
                            None if !query.is_empty() => {
                                author_filter::AuthorTarget::Author(query.into())
                            }
                            None => return,
                        };
                        author_filter::apply(this, repo_id, target, cx);
                    },
                ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }
}

/// True when the branch picker should offer refs beyond branches (HEAD, tags)
/// and accept a free-form ref typed into the search box.
/// Which ref list the popover in hand is showing. The prompts that branch from a
/// ref offer HEAD and tags as well; delete and rebase-onto list branches alone
/// and cannot offer the one that is checked out.
fn ref_rows_spec(this: &PopoverHost) -> branch_picker::RefRowsSpec {
    match &this.popover {
        Some(PopoverKind::CreateBranchFromRefPrompt { .. })
        | Some(PopoverKind::Repo {
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
            ..
        }) => branch_picker::RefRowsSpec::source_ref(),
        Some(PopoverKind::BranchPicker {
            purpose: BranchPickerPurpose::Delete | BranchPickerPurpose::RebaseOnto,
        }) => branch_picker::RefRowsSpec::branches(true),
        _ => branch_picker::RefRowsSpec::branches(false),
    }
}

fn worktree_picker_state(this: &PopoverHost) -> Option<(RepoId, bool)> {
    match &this.popover {
        Some(PopoverKind::Repo {
            repo_id,
            kind:
                RepoPopoverKind::Worktree(
                    kind @ (WorktreePopoverKind::OpenPicker | WorktreePopoverKind::RemovePicker),
                ),
        }) => Some((*repo_id, matches!(kind, WorktreePopoverKind::RemovePicker))),
        _ => None,
    }
}

fn workspace_picker_state(this: &PopoverHost) -> Option<RepoId> {
    match &this.popover {
        Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::BadgePicker),
        }) => Some(*repo_id),
        _ => None,
    }
}

fn submodule_picker_state(this: &PopoverHost) -> Option<(RepoId, bool)> {
    match &this.popover {
        Some(PopoverKind::Repo {
            repo_id,
            kind:
                RepoPopoverKind::Submodule(
                    kind @ (SubmodulePopoverKind::OpenPicker | SubmodulePopoverKind::RemovePicker),
                ),
        }) => Some((*repo_id, matches!(kind, SubmodulePopoverKind::RemovePicker))),
        _ => None,
    }
}

fn upstream_picker_state(this: &PopoverHost) -> Option<(RepoId, String)> {
    match &this.popover {
        Some(PopoverKind::UpstreamPicker { repo_id, branch }) => Some((*repo_id, branch.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `match_branches` — deleted with this commit — used to filter the plain ref
    /// lists for the arrow keys while the rows themselves were filtered by the
    /// picker's own matcher. The two agreed only by luck of the row shape, and
    /// the ref lists now go through the picker's matcher alone. This keeps the
    /// old ordering rule as an oracle so that agreement stays pinned: the order
    /// the user sees must not move.
    ///
    /// They coincide because these rows are single-part and section-less, which
    /// collapses the layout's sort key `(group, match_start, display_len,
    /// display_text)` to the old `(match_start, name_len, name)`.
    fn match_branches_oracle(branches: &[String], query: &str) -> Vec<String> {
        if query.is_empty() {
            return branches.to_vec();
        }
        let query_lower = query.to_ascii_lowercase();
        let mut out: Vec<_> = branches
            .iter()
            .filter_map(|name| {
                let lower = name.to_ascii_lowercase();
                lower
                    .find(&query_lower)
                    .map(|start| (start, name.len(), name.clone()))
            })
            .collect();
        out.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        out.into_iter().map(|(.., name)| name).collect()
    }

    #[test]
    fn the_picker_matcher_orders_plain_ref_rows_the_way_match_branches_did() {
        let names: Vec<String> = [
            "main",
            "feature/alpha",
            "release/main-line",
            "MAIN-uppercase",
            "topic/mainly",
            "hotfix",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();

        for query in ["", "main", "MAIN", "ma", "e", "zzz"] {
            let items: Vec<components::PickerPromptItem> = names
                .iter()
                .map(|name| components::PickerPromptItem::plain(name.clone()))
                .collect();
            let layout = components::picker_prompt_layout(&items, query);
            let through_layout: Vec<String> = layout
                .item_indices
                .iter()
                .map(|ix| names[*ix].clone())
                .collect();

            assert_eq!(
                through_layout,
                match_branches_oracle(&names, query),
                "the two matchers must agree for {query:?}"
            );
        }
    }
}
