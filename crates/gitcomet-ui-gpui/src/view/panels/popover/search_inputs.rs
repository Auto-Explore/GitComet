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
                    let payload = (*selected_index(this)).and_then(|sel| list.get(sel).cloned());
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
                |this, query, _cx| {
                    Some(filter_by_query(
                        this.state
                            .repos
                            .iter()
                            .map(|r| (r.id, r.spec.workdir.display().to_string())),
                        query,
                    ))
                },
                |this, cx| this.close_popover(cx),
                Self::scroll_picker_prompt_to_item,
                |this, payload, _query, _window, cx| {
                    if let Some(repo_id) = payload {
                        this.store.dispatch(Msg::SetActiveRepo { repo_id });
                        this.close_popover(cx);
                    }
                },
            ));
        }
        self.reset_picker_search_input(&input, window, cx);
        input
    }

    pub(super) fn ensure_recent_repo_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let input = Self::ensure_search_input_entity(
            &mut self.recent_repo_picker_search_input,
            "Filter recent repositories",
            window,
            cx,
        );
        if self._recent_repo_picker_search_input_subscription.is_none() {
            self._recent_repo_picker_search_input_subscription =
                Some(Self::picker_search_subscription(
                    &input,
                    window,
                    cx,
                    |this| matches!(this.popover, Some(PopoverKind::RecentRepositoryPicker)),
                    |this| &mut this.recent_repo_picker_selected_index,
                    |this, query, _cx| {
                        Some(filter_by_query(
                            this.recent_repo_picker_cached_repos
                                .iter()
                                .map(|p| (p.clone(), recent_repo_display_text(p))),
                            query,
                        ))
                    },
                    |this, cx| this.close_popover(cx),
                    |this, sel, cx| {
                        scroll_recent_repo_picker_to_selected(sel, &this.picker_prompt_scroll, cx);
                    },
                    |this, payload, _query, _window, cx| {
                        if let Some(path) = payload {
                            recent_repo_picker::select_recent_repository(this, path, cx);
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
        if self._branch_picker_search_input_subscription.is_none() {
            self._branch_picker_search_input_subscription = Some(Self::picker_search_subscription(
                &input,
                window,
                cx,
                |this| this.inline_branch_picker_active(),
                |this| &mut this.branch_picker_selected_index,
                |this, query, _cx| {
                    let is_delete = matches!(
                        this.popover,
                        Some(PopoverKind::BranchPicker {
                            purpose: BranchPickerPurpose::Delete
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
                            if is_delete && head_branch == Some(b.name.as_str()) {
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
                    Some(match_branches(&names, query))
                },
                |this, cx| this.handle_inline_branch_picker_escape(cx),
                Self::scroll_picker_prompt_to_item,
                |this, payload, query, _window, cx| {
                    let Some(repo_id) = this.active_repo().map(|repo| repo.id) else {
                        return;
                    };
                    if branch_picker_offers_refs(this) {
                        let name = payload.unwrap_or(query);
                        if !name.is_empty() {
                            this.handle_inline_branch_picker_select(name, repo_id, cx);
                        }
                    } else if let Some(name) = payload {
                        this.handle_inline_branch_picker_select(name, repo_id, cx);
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

fn recent_repo_display_text(path: &std::path::Path) -> String {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return path.display().to_string();
    };
    let Some(parent) = path.parent() else {
        return name.to_owned();
    };
    format!("{} - {}", name, parent.display())
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

fn scroll_recent_repo_picker_to_selected(
    sel: usize,
    scroll_handle: &ScrollHandle,
    cx: &mut impl BorrowAppContext,
) {
    let ui_scale = ui_scale::UiScale::current(cx);
    let item_h = ui_scale.px(32.0);
    let item_y = item_h * sel as f32;
    let viewport_h = ui_scale.px(320.0) - item_h;
    let target = (item_y - viewport_h * 0.5).max(px(0.0));
    scroll_handle.set_offset(point(px(0.0), target));
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
