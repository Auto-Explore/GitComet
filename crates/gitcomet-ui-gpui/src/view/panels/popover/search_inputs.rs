use super::picker_nav::{PickerNavKeys, PickerNavOutcome, handle_picker_nav};
use super::*;

impl PopoverHost {
    pub(super) fn ensure_repo_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.repo_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter repositories".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        if self._repo_picker_search_input_subscription.is_none() {
            self._repo_picker_search_input_subscription =
                Some(cx.observe(input, |this, input, cx| {
                    let keys = input.update(cx, |i, _| PickerNavKeys::take(i));

                    if !matches!(this.popover, Some(PopoverKind::RepoPicker)) {
                        return;
                    }

                    let repos = this.state.repos.clone();
                    let query = input.read_with(cx, |i, _| i.text().trim().to_string());
                    let count = count_path_matches_by(&repos, &query, |r: &RepoState| {
                        r.spec.workdir.display().to_string()
                    });

                    match handle_picker_nav(&keys, &mut this.repo_picker_selected_index, count) {
                        PickerNavOutcome::Escape => {
                            this.close_popover(cx);
                            return;
                        }
                        PickerNavOutcome::Navigated => {
                            this.picker_prompt_scroll
                                .scroll_to_item(this.repo_picker_selected_index.unwrap());
                            cx.notify();
                            return;
                        }
                        PickerNavOutcome::Enter => {
                            if let Some(sel) = this.repo_picker_selected_index {
                                let q = query.to_ascii_lowercase();
                                let matched: Vec<_> = repos
                                    .iter()
                                    .filter(|r| {
                                        r.spec
                                            .workdir
                                            .display()
                                            .to_string()
                                            .to_ascii_lowercase()
                                            .contains(&q)
                                    })
                                    .collect();
                                if let Some(repo) = matched.get(sel) {
                                    this.store.dispatch(Msg::SetActiveRepo { repo_id: repo.id });
                                    this.close_popover(cx);
                                    return;
                                }
                            }
                        }
                        PickerNavOutcome::Idle => {}
                    }
                    cx.notify();
                }));
        }
        input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_recent_repo_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.recent_repo_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter recent repositories".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        if self._recent_repo_picker_search_input_subscription.is_none() {
            self._recent_repo_picker_search_input_subscription =
                Some(cx.observe(input, |this, input, cx| {
                    let keys = input.update(cx, |i, _| PickerNavKeys::take(i));

                    if !matches!(this.popover, Some(PopoverKind::RecentRepositoryPicker)) {
                        return;
                    }

                    let recent_repos = session::load().recent_repos;
                    let query = input.read_with(cx, |i, _| i.text().trim().to_string());
                    let count =
                        count_path_matches_by(&recent_repos, &query, |p: &std::path::PathBuf| {
                            recent_repo_display_text(p)
                        });

                    match handle_picker_nav(
                        &keys,
                        &mut this.recent_repo_picker_selected_index,
                        count,
                    ) {
                        PickerNavOutcome::Escape => {
                            this.close_popover(cx);
                            return;
                        }
                        PickerNavOutcome::Navigated => {
                            scroll_recent_repo_picker_to_selected(
                                this.recent_repo_picker_selected_index.unwrap(),
                                &this.picker_prompt_scroll,
                                cx,
                            );
                            cx.notify();
                            return;
                        }
                        PickerNavOutcome::Enter => {
                            if let Some(sel) = this.recent_repo_picker_selected_index {
                                let q = query.to_ascii_lowercase();
                                let matched: Vec<_> = recent_repos
                                    .iter()
                                    .filter(|p| {
                                        recent_repo_display_text(p)
                                            .to_ascii_lowercase()
                                            .contains(&q)
                                    })
                                    .collect();
                                if let Some(path) = matched.get(sel) {
                                    let path = (*path).clone();
                                    recent_repo_picker::select_recent_repository(this, path, cx);
                                    return;
                                }
                            }
                        }
                        PickerNavOutcome::Idle => {}
                    }
                    cx.notify();
                }));
        }
        input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_branch_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.branch_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter branches".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        if self._branch_picker_search_input_subscription.is_none() {
            self._branch_picker_search_input_subscription =
                Some(cx.observe(input, |this, input, cx| {
                    let keys = input.update(cx, |i, _| PickerNavKeys::take(i));

                    if !this.inline_branch_picker_active() {
                        return;
                    }

                    if keys.escape {
                        this.handle_inline_branch_picker_escape(cx);
                        return;
                    }

                    // Extract all data from repo in a limited scope so it drops
                    // before the mutable borrow of `this.branch_picker_selected_index`.
                    let (repo_id, is_create_from_ref, matches) = {
                        let Some(repo) = this.active_repo() else {
                            return;
                        };

                        let is_delete = matches!(
                            this.popover,
                            Some(PopoverKind::BranchPicker {
                                purpose: BranchPickerPurpose::Delete
                            })
                        );
                        let is_create_from_ref = matches!(
                            this.popover,
                            Some(PopoverKind::CreateBranchFromRefPrompt { .. })
                        );
                        let branches: Vec<String> = match &repo.branches {
                            Loadable::Ready(branches) => {
                                let head_branch = match &repo.head_branch {
                                    Loadable::Ready(head) => Some(head.as_str()),
                                    _ => None,
                                };
                                let mut names: Vec<_> = branches
                                    .iter()
                                    .filter_map(|b| {
                                        if is_delete && head_branch == Some(b.name.as_str()) {
                                            None
                                        } else {
                                            Some(b.name.clone())
                                        }
                                    })
                                    .collect();
                                if is_create_from_ref {
                                    names.insert(0, "HEAD".to_string());
                                    if let Loadable::Ready(tags) = &repo.tags {
                                        names.extend(tags.iter().map(|t| t.name.clone()));
                                    }
                                }
                                names
                            }
                            _ => return,
                        };
                        let query = input.read_with(cx, |i, _| i.text().trim().to_string());
                        let matches = match_branches(&branches, &query);
                        (repo.id, is_create_from_ref, matches)
                    };

                    let query = input.read_with(cx, |i, _| i.text().trim().to_string());
                    let count = matches.len();

                    match handle_picker_nav(&keys, &mut this.branch_picker_selected_index, count) {
                        PickerNavOutcome::Escape => {
                            this.handle_inline_branch_picker_escape(cx);
                            return;
                        }
                        PickerNavOutcome::Navigated => {
                            this.picker_prompt_scroll
                                .scroll_to_item(this.branch_picker_selected_index.unwrap());
                            cx.notify();
                            return;
                        }
                        PickerNavOutcome::Enter => {
                            if is_create_from_ref {
                                let name = if let Some(sel) = this.branch_picker_selected_index
                                    && let Some(name) = matches.get(sel)
                                {
                                    name.clone()
                                } else {
                                    query
                                };
                                if !name.is_empty() {
                                    this.handle_inline_branch_picker_select(name, repo_id, cx);
                                    return;
                                }
                            } else if let Some(sel) = this.branch_picker_selected_index
                                && let Some(name) = matches.get(sel)
                            {
                                let name = name.clone();
                                this.handle_inline_branch_picker_select(name, repo_id, cx);
                                return;
                            }
                        }
                        PickerNavOutcome::Idle => {}
                    }
                    cx.notify();
                }));
        }
        input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_worktree_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.worktree_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter worktrees".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        if self._worktree_picker_search_input_subscription.is_none() {
            self._worktree_picker_search_input_subscription =
                Some(cx.observe_in(input, window, |this, input, window, cx| {
                    let keys = input.update(cx, |i, _| PickerNavKeys::take(i));

                    let (repo_id, is_remove) = match &this.popover {
                        Some(PopoverKind::Repo {
                            repo_id,
                            kind:
                                RepoPopoverKind::Worktree(
                                    WorktreePopoverKind::OpenPicker
                                    | WorktreePopoverKind::RemovePicker,
                                ),
                        }) => (
                            *repo_id,
                            matches!(
                                this.popover,
                                Some(PopoverKind::Repo {
                                    kind: RepoPopoverKind::Worktree(
                                        WorktreePopoverKind::RemovePicker
                                    ),
                                    ..
                                })
                            ),
                        ),
                        _ => return,
                    };

                    let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) else {
                        return;
                    };
                    let Loadable::Ready(worktrees) = &repo.worktrees else {
                        return;
                    };
                    let workdir = repo.spec.workdir.clone();
                    let paths: Vec<std::path::PathBuf> = worktrees
                        .iter()
                        .filter(|w| w.path != workdir)
                        .map(|w| w.path.clone())
                        .collect();
                    let match_texts: Vec<String> = worktrees
                        .iter()
                        .filter(|w| w.path != workdir)
                        .map(|w| {
                            if let Some(branch) = &w.branch {
                                format!("{}{}", branch, w.path.display())
                            } else {
                                w.path.display().to_string()
                            }
                        })
                        .collect();

                    let query = input.read_with(cx, |i, _| i.text().trim().to_string());
                    let count = count_path_matches_by(&match_texts, &query, |s: &String| s.clone());

                    match handle_picker_nav(&keys, &mut this.worktree_picker_selected_index, count)
                    {
                        PickerNavOutcome::Escape => {
                            this.close_popover(cx);
                            return;
                        }
                        PickerNavOutcome::Navigated => {
                            this.picker_prompt_scroll
                                .scroll_to_item(this.worktree_picker_selected_index.unwrap());
                            cx.notify();
                            return;
                        }
                        PickerNavOutcome::Enter => {
                            if let Some(sel) = this.worktree_picker_selected_index {
                                let q = query.to_ascii_lowercase();
                                let matched: Vec<usize> = match_texts
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, t)| t.to_ascii_lowercase().contains(&q))
                                    .map(|(i, _)| i)
                                    .collect();
                                if let Some(&item_ix) = matched.get(sel)
                                    && let Some(path) = paths.get(item_ix).cloned()
                                {
                                    if is_remove {
                                        this.open_popover_centered(
                                            PopoverKind::worktree(
                                                repo_id,
                                                WorktreePopoverKind::RemoveConfirm {
                                                    path,
                                                    branch: None,
                                                },
                                            ),
                                            window,
                                            cx,
                                        );
                                    } else {
                                        this.store.dispatch(Msg::OpenRepo(path));
                                        this.close_popover(cx);
                                    }
                                    return;
                                }
                            }
                        }
                        PickerNavOutcome::Idle => {}
                    }
                    cx.notify();
                }));
        }
        input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_submodule_picker_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.submodule_picker_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter submodules".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        if self._submodule_picker_search_input_subscription.is_none() {
            self._submodule_picker_search_input_subscription =
                Some(cx.observe_in(input, window, |this, input, window, cx| {
                    let keys = input.update(cx, |i, _| PickerNavKeys::take(i));

                    let (repo_id, is_remove) = match &this.popover {
                        Some(PopoverKind::Repo {
                            repo_id,
                            kind:
                                RepoPopoverKind::Submodule(
                                    SubmodulePopoverKind::OpenPicker
                                    | SubmodulePopoverKind::RemovePicker,
                                ),
                        }) => (
                            *repo_id,
                            matches!(
                                this.popover,
                                Some(PopoverKind::Repo {
                                    kind: RepoPopoverKind::Submodule(
                                        SubmodulePopoverKind::RemovePicker
                                    ),
                                    ..
                                })
                            ),
                        ),
                        _ => return,
                    };

                    let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) else {
                        return;
                    };
                    let Loadable::Ready(submodules) = &repo.submodules else {
                        return;
                    };
                    let base = repo.spec.workdir.clone();
                    let rel_paths: Vec<std::path::PathBuf> =
                        submodules.iter().map(|s| s.path.clone()).collect();
                    let match_texts: Vec<String> =
                        rel_paths.iter().map(|p| p.display().to_string()).collect();

                    let query = input.read_with(cx, |i, _| i.text().trim().to_string());
                    let count = count_path_matches_by(&match_texts, &query, |s: &String| s.clone());

                    match handle_picker_nav(&keys, &mut this.submodule_picker_selected_index, count)
                    {
                        PickerNavOutcome::Escape => {
                            this.close_popover(cx);
                            return;
                        }
                        PickerNavOutcome::Navigated => {
                            this.picker_prompt_scroll
                                .scroll_to_item(this.submodule_picker_selected_index.unwrap());
                            cx.notify();
                            return;
                        }
                        PickerNavOutcome::Enter => {
                            if let Some(sel) = this.submodule_picker_selected_index {
                                let q = query.to_ascii_lowercase();
                                let matched: Vec<usize> = match_texts
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, t)| t.to_ascii_lowercase().contains(&q))
                                    .map(|(i, _)| i)
                                    .collect();
                                if let Some(&item_ix) = matched.get(sel)
                                    && let Some(rel_path) = rel_paths.get(item_ix).cloned()
                                {
                                    if is_remove {
                                        this.open_popover_centered(
                                            PopoverKind::submodule(
                                                repo_id,
                                                SubmodulePopoverKind::RemoveConfirm {
                                                    path: rel_path,
                                                },
                                            ),
                                            window,
                                            cx,
                                        );
                                    } else {
                                        this.store.dispatch(Msg::OpenRepo(base.join(&rel_path)));
                                        this.close_popover(cx);
                                    }
                                    return;
                                }
                            }
                        }
                        PickerNavOutcome::Idle => {}
                    }
                    cx.notify();
                }));
        }
        input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
    }

    pub(super) fn ensure_file_history_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let theme = self.theme;
        let input = self.file_history_search_input.get_or_insert_with(|| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Filter commits".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        });
        if self._file_history_search_input_subscription.is_none() {
            self._file_history_search_input_subscription =
                Some(cx.observe(input, |this, input, cx| {
                    let keys = input.update(cx, |i, _| PickerNavKeys::take(i));

                    let (repo_id, path) = match &this.popover {
                        Some(PopoverKind::FileHistory { repo_id, path }) => {
                            (*repo_id, path.clone())
                        }
                        _ => return,
                    };

                    let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) else {
                        return;
                    };
                    let Loadable::Ready(page) = &repo.history_state.file_history else {
                        return;
                    };
                    let commits = page.commits.clone();

                    let query = input.read_with(cx, |i, _| i.text().trim().to_string());
                    let count =
                        count_path_matches_by(&commits, &query, |c| file_history_match_text(c));

                    match handle_picker_nav(&keys, &mut this.file_history_selected_index, count) {
                        PickerNavOutcome::Escape => {
                            this.close_popover(cx);
                            return;
                        }
                        PickerNavOutcome::Navigated => {
                            this.picker_prompt_scroll
                                .scroll_to_item(this.file_history_selected_index.unwrap());
                            cx.notify();
                            return;
                        }
                        PickerNavOutcome::Enter => {
                            if let Some(sel) = this.file_history_selected_index {
                                let q = query.to_ascii_lowercase();
                                let matched: Vec<_> = commits
                                    .iter()
                                    .filter(|c| {
                                        file_history_match_text(c).to_ascii_lowercase().contains(&q)
                                    })
                                    .collect();
                                if let Some(commit) = matched.get(sel) {
                                    let commit_id = commit.id.clone();
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
                                    return;
                                }
                            }
                        }
                        PickerNavOutcome::Idle => {}
                    }
                    cx.notify();
                }));
        }
        input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.picker_prompt_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus_handle = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus_handle, cx);
        input.clone()
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

/// Counts how many items match `query` using a case-insensitive substring check on the
/// text produced by `text_fn`. When `query` is empty every item matches.
fn count_path_matches_by<T>(items: &[T], query: &str, text_fn: impl Fn(&T) -> String) -> usize {
    if query.is_empty() {
        return items.len();
    }
    let q = query.to_ascii_lowercase();
    items
        .iter()
        .filter(|item| text_fn(item).to_ascii_lowercase().contains(&q))
        .count()
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
