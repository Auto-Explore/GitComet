use super::picker_nav::{PickerNavKeys, PickerNavOutcome, handle_picker_nav};
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

    fn scroll_picker_prompt_to_item(&mut self, sel: usize, _cx: &mut gpui::Context<Self>) {
        self.picker_prompt_scroll.scroll_to_item(sel);
    }

    /// Scrolls the badge pickers' row list so the selected row is in view.
    ///
    /// These two lists are windowed once they grow past a couple of viewports, so
    /// a row further down has no element for `ScrollHandle::scroll_to_item` to
    /// find; the row geometry says where it would be instead.
    fn scroll_picker_prompt_to_row(
        &self,
        items: &[components::PickerPromptItem],
        layout: &components::PickerPromptLayout,
        sel: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let ui_scale = super::popover_ui_scale(cx);
        let geometry = components::PickerPromptGeometry::new(items, layout, ui_scale);
        let viewport = ui_scale.px(components::PICKER_LIST_MAX_HEIGHT_PX);
        let current = -self.picker_prompt_scroll.offset().y;
        let offset = geometry.reveal_offset(sel, viewport, current);
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), -offset));
    }

    /// Shared keyboard-navigation subscription for picker search inputs.
    ///
    /// `is_active` gates the subscription to the picker's popover kind.
    /// `items` returns the filtered list the picker currently displays for
    /// `query` (or `None` while the underlying data is still loading), so the
    /// selected index and the Enter target can't drift apart. `on_enter`
    /// receives the selected payload (if any) plus the raw query.
    #[allow(clippy::too_many_arguments)]
    fn picker_search_subscription<T: Clone + 'static>(
        input: &Entity<components::TextInput>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
        is_active: fn(&Self) -> bool,
        selected_index: fn(&mut Self) -> &mut Option<usize>,
        items: impl Fn(&mut Self, &str, &mut gpui::Context<Self>) -> Option<Vec<T>> + 'static,
        on_escape: impl Fn(&mut Self, &mut gpui::Context<Self>) + 'static,
        scroll_to: impl Fn(&mut Self, usize, &mut gpui::Context<Self>) + 'static,
        on_enter: impl Fn(&mut Self, Option<T>, String, &mut Window, &mut gpui::Context<Self>) + 'static,
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

            match handle_picker_nav(&keys, selected_index(this), list.len()) {
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
                        .and_then(|sel| list.get(sel).cloned());
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
                // highlighted row — including across the two sections. While
                // the sort menu covers the list, it walks the sort options
                // instead.
                |this, query, _cx| Some(repo_picker::nav_targets(this, query)),
                repo_picker::dismiss,
                |this, sel, cx| {
                    // Both of these replace the repository rows as the arrow
                    // keys' target, so the selection is not a row index to
                    // scroll to.
                    if this.repo_picker_sort_menu_open || this.repo_picker_row_menu.is_some() {
                        return;
                    }
                    let query = this
                        .repo_picker_search_input
                        .as_ref()
                        .map(|input| input.read(cx).text().trim().to_string())
                        .unwrap_or_default();
                    // Section headers occupy scroll children too, so scroll to
                    // the row's child slot rather than its selection index.
                    let child_ix = repo_picker::filtered_layout(this, &query)
                        .1
                        .child_indices
                        .get(sel)
                        .copied()
                        .unwrap_or(sel);
                    this.picker_prompt_scroll.scroll_to_item(child_ix);
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
                |this, query, _cx| {
                    // The checkout picker renders sectioned, multi-part rows, so
                    // its nav order must come from the picker's own layout over
                    // the very same items. `match_branches` sorts differently
                    // (no section term, name length rather than row length) and
                    // would make Enter check out a branch other than the
                    // highlighted one.
                    if branch_picker::is_checkout_picker(this) {
                        return Some(branch_picker::nav_targets(this, query));
                    }

                    // The current branch is not offered as a rebase target (it
                    // cannot be rebased onto itself) and cannot be deleted.
                    let hide_current_branch = matches!(
                        this.popover,
                        Some(PopoverKind::BranchPicker {
                            purpose: BranchPickerPurpose::Delete | BranchPickerPurpose::RebaseOnto
                        })
                    );
                    let with_refs = branch_picker_offers_refs(this);
                    let repo = this.active_repo()?;
                    let Loadable::Ready(branches) = &repo.branches else {
                        return None;
                    };
                    let head_branch = match &repo.head_branch {
                        Loadable::Ready(head) => Some(head.as_str()),
                        _ => None,
                    };
                    let mut names: Vec<String> = branches
                        .iter()
                        .filter_map(|b| {
                            if hide_current_branch && head_branch == Some(b.name.as_str()) {
                                None
                            } else {
                                Some(b.name.clone())
                            }
                        })
                        .collect();
                    if with_refs {
                        names.insert(0, "HEAD".to_string());
                        if let Loadable::Ready(tags) = &repo.tags {
                            names.extend(tags.iter().map(|t| t.name.clone()));
                        }
                    }
                    Some(
                        match_branches(&names, query)
                            .into_iter()
                            .map(branch_picker::BranchPickerNavTarget::Ref)
                            .collect(),
                    )
                },
                |this, cx| this.handle_inline_branch_picker_escape(cx),
                |this, sel, cx| {
                    if !branch_picker::is_checkout_picker(this) {
                        this.picker_prompt_scroll.scroll_to_item(sel);
                        return;
                    }
                    let query = this
                        .branch_picker_search_input
                        .as_ref()
                        .map(|input| input.read(cx).text().trim().to_string())
                        .unwrap_or_default();
                    let rows = branch_picker::cached(this, &query);
                    this.scroll_picker_prompt_to_row(&rows.items, &rows.layout, sel, cx);
                },
                |this, payload, query, window, cx| {
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
                    if branch_picker_offers_refs(this) {
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
                        let (repo_id, _) = worktree_picker_state(this)?;
                        let repo = this.state.repos.iter().find(|r| r.id == repo_id)?;
                        let Loadable::Ready(worktrees) = &repo.worktrees else {
                            return None;
                        };
                        let workdir = &repo.spec.workdir;
                        Some(filter_by_query(
                            worktrees.iter().filter(|w| &w.path != workdir).map(|w| {
                                let text = if let Some(branch) = &w.branch {
                                    format!("{}{}", branch, w.path.display())
                                } else {
                                    w.path.display().to_string()
                                };
                                (w.path.clone(), text)
                            }),
                            query,
                        ))
                    },
                    |this, cx| this.close_popover(cx),
                    Self::scroll_picker_prompt_to_item,
                    |this, payload, _query, window, cx| {
                        let Some(path) = payload else {
                            return;
                        };
                        let Some((repo_id, is_remove)) = worktree_picker_state(this) else {
                            return;
                        };
                        if is_remove {
                            this.open_popover_centered(
                                PopoverKind::worktree(
                                    repo_id,
                                    WorktreePopoverKind::RemoveConfirm { path, branch: None },
                                ),
                                window,
                                cx,
                            );
                        } else {
                            this.store.dispatch(Msg::OpenRepo(path));
                            this.close_popover(cx);
                        }
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
                    |this, query, _cx| {
                        let repo_id = workspace_picker_state(this)?;
                        // Layout-driven so Enter can never land on a different
                        // row than the highlighted one.
                        Some(workspace_picker::nav_targets(this, repo_id, query))
                    },
                    |this, cx| this.close_popover(cx),
                    |this, sel, cx| {
                        let Some(repo_id) = workspace_picker_state(this) else {
                            return;
                        };
                        let query = this
                            .workspace_picker_search_input
                            .as_ref()
                            .map(|input| input.read(cx).text().trim().to_string())
                            .unwrap_or_default();
                        let rows = workspace_picker::cached(this, repo_id, &query);
                        this.scroll_picker_prompt_to_row(&rows.items, &rows.layout, sel, cx);
                    },
                    |this, payload, query, window, cx| {
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
                        let repo = this.state.repos.iter().find(|r| r.id == repo_id)?;
                        let Loadable::Ready(submodules) = &repo.submodules else {
                            return None;
                        };
                        Some(filter_by_query(
                            submodules
                                .iter()
                                .map(|s| (s.path.clone(), s.path.display().to_string())),
                            query,
                        ))
                    },
                    |this, cx| this.close_popover(cx),
                    Self::scroll_picker_prompt_to_item,
                    |this, payload, _query, window, cx| {
                        let Some(rel_path) = payload else {
                            return;
                        };
                        let Some((repo_id, is_remove)) = submodule_picker_state(this) else {
                            return;
                        };
                        if is_remove {
                            this.open_popover_centered(
                                PopoverKind::submodule(
                                    repo_id,
                                    SubmodulePopoverKind::RemoveConfirm { path: rel_path },
                                ),
                                window,
                                cx,
                            );
                        } else {
                            let Some(base) = this
                                .state
                                .repos
                                .iter()
                                .find(|r| r.id == repo_id)
                                .map(|r| r.spec.workdir.clone())
                            else {
                                return;
                            };
                            this.store.dispatch(Msg::OpenRepo(base.join(&rel_path)));
                            this.close_popover(cx);
                        }
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
                |this, query, _cx| {
                    let Loadable::Ready(stashes) = &this.active_repo()?.stashes else {
                        return None;
                    };
                    Some(filter_by_query(
                        stashes.iter().map(|s| (s.index, s.message.to_string())),
                        query,
                    ))
                },
                |this, cx| this.close_popover(cx),
                Self::scroll_picker_prompt_to_item,
                |this, payload, _query, _window, cx| {
                    let Some(git_index) = payload else {
                        return;
                    };
                    let Some(PopoverKind::StashPickerPrompt { repo_id, purpose }) =
                        this.popover.clone()
                    else {
                        return;
                    };
                    match purpose {
                        StashPickerPurpose::Pop => {
                            this.store.dispatch(Msg::PopStash {
                                repo_id,
                                index: git_index,
                            });
                        }
                        StashPickerPurpose::Apply => {
                            this.store.dispatch(Msg::ApplyStash {
                                repo_id,
                                index: git_index,
                            });
                        }
                        StashPickerPurpose::Drop => {
                            this.store.dispatch(Msg::DropStash {
                                repo_id,
                                index: git_index,
                            });
                        }
                    }
                    this.store.dispatch(Msg::LoadStashes { repo_id });
                    this.close_popover(cx);
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
                |this, query, _cx| {
                    let Some(PopoverKind::FileHistory { repo_id, .. }) = &this.popover else {
                        return None;
                    };
                    let repo = this.state.repos.iter().find(|r| r.id == *repo_id)?;
                    let Loadable::Ready(page) = &repo.history_state.file_history else {
                        return None;
                    };
                    Some(filter_by_query(
                        page.commits
                            .iter()
                            .map(|c| (c.id.clone(), file_history_match_text(c))),
                        query,
                    ))
                },
                |this, cx| this.close_popover(cx),
                Self::scroll_picker_prompt_to_item,
                |this, payload, _query, _window, cx| {
                    let Some(commit_id) = payload else {
                        return;
                    };
                    let Some(PopoverKind::FileHistory { repo_id, path }) = this.popover.clone()
                    else {
                        return;
                    };
                    this.store.dispatch(Msg::SelectCommit {
                        repo_id,
                        commit_id: commit_id.clone(),
                    });
                    this.store.dispatch(Msg::SelectDiff {
                        repo_id,
                        target: DiffTarget::Commit {
                            commit_id,
                            path: Some(path),
                        },
                    });
                    this.close_popover(cx);
                },
            ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }
}

/// True when the branch picker should offer refs beyond branches (HEAD, tags)
/// and accept a free-form ref typed into the search box.
fn branch_picker_offers_refs(this: &PopoverHost) -> bool {
    matches!(
        this.popover,
        Some(PopoverKind::CreateBranchFromRefPrompt { .. })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                ..
            })
    )
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

fn file_history_match_text(commit: &gitcomet_core::domain::Commit) -> String {
    let sha = commit.id.as_ref();
    let short = sha.get(0..8).unwrap_or(sha);
    format!("{}{}", short, commit.summary)
}

/// Case-insensitive substring filter over `(payload, match_text)` pairs,
/// preserving input order. An empty query matches everything.
fn filter_by_query<T>(items: impl IntoIterator<Item = (T, String)>, query: &str) -> Vec<T> {
    let q = query.to_ascii_lowercase();
    items
        .into_iter()
        .filter(|(_, text)| q.is_empty() || text.to_ascii_lowercase().contains(&q))
        .map(|(item, _)| item)
        .collect()
}

fn match_branches(branches: &[String], query: &str) -> Vec<String> {
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
