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
                    let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());

                    if !matches!(this.popover, Some(PopoverKind::RepoPicker)) {
                        return;
                    }

                    if escape_pressed {
                        this.close_popover(cx);
                        return;
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
                    let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());
                    let arrow_up_pressed =
                        input.update(cx, |input, _| input.take_arrow_up_pressed());
                    let arrow_down_pressed =
                        input.update(cx, |input, _| input.take_arrow_down_pressed());
                    let tab_pressed = input.update(cx, |input, _| input.take_tab_pressed());
                    let shift_tab_pressed =
                        input.update(cx, |input, _| input.take_shift_tab_pressed());
                    let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());

                    if !matches!(this.popover, Some(PopoverKind::RecentRepositoryPicker)) {
                        return;
                    }

                    if escape_pressed {
                        this.close_popover(cx);
                        return;
                    }

                    let recent_repos = session::load().recent_repos;
                    let query = input.read_with(cx, |input, _| input.text().trim().to_string());
                    let match_count = count_recent_repo_matches(&recent_repos, &query);

                    if arrow_up_pressed || shift_tab_pressed {
                        this.recent_repo_picker_selected_index =
                            Some(match this.recent_repo_picker_selected_index {
                                Some(ix) if ix > 0 => ix - 1,
                                _ if match_count > 0 => match_count - 1,
                                _ => return,
                            });
                        scroll_recent_repo_picker_to_selected(
                            this.recent_repo_picker_selected_index.unwrap(),
                            &this.picker_prompt_scroll,
                            cx,
                        );
                        cx.notify();
                        return;
                    }

                    if arrow_down_pressed || tab_pressed {
                        this.recent_repo_picker_selected_index =
                            Some(match this.recent_repo_picker_selected_index {
                                Some(ix) if ix + 1 < match_count => ix + 1,
                                _ if match_count > 0 => 0,
                                _ => return,
                            });
                        scroll_recent_repo_picker_to_selected(
                            this.recent_repo_picker_selected_index.unwrap(),
                            &this.picker_prompt_scroll,
                            cx,
                        );
                        cx.notify();
                        return;
                    }

                    if enter_pressed {
                        if let Some(sel) = this.recent_repo_picker_selected_index {
                            let query_lower = query.to_ascii_lowercase();
                            let matched: Vec<_> = recent_repos
                                .iter()
                                .filter(|path| {
                                    recent_repo_display_text(path)
                                        .to_ascii_lowercase()
                                        .contains(&query_lower)
                                })
                                .collect();
                            if let Some(path) = matched.get(sel) {
                                let path = (*path).clone();
                                recent_repo_picker::select_recent_repository(this, path, cx);
                                return;
                            }
                        }
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
                    let escape_pressed = input.update(cx, |input, _| input.take_escape_pressed());
                    let arrow_up_pressed =
                        input.update(cx, |input, _| input.take_arrow_up_pressed());
                    let arrow_down_pressed =
                        input.update(cx, |input, _| input.take_arrow_down_pressed());
                    let tab_pressed = input.update(cx, |input, _| input.take_tab_pressed());
                    let shift_tab_pressed =
                        input.update(cx, |input, _| input.take_shift_tab_pressed());
                    let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());

                    if !this.inline_branch_picker_active() {
                        return;
                    }

                    if escape_pressed {
                        this.handle_inline_branch_picker_escape(cx);
                        return;
                    }

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
                    let query = input.read_with(cx, |input, _| input.text().trim().to_string());
                    let matches = match_branches(&branches, &query);
                    let match_count = matches.len();

                    if arrow_up_pressed || shift_tab_pressed {
                        this.branch_picker_selected_index =
                            Some(match this.branch_picker_selected_index {
                                Some(ix) if ix > 0 => ix - 1,
                                _ if match_count > 0 => match_count - 1,
                                _ => return,
                            });
                        this.picker_prompt_scroll
                            .scroll_to_item(this.branch_picker_selected_index.unwrap());
                        cx.notify();
                        return;
                    }

                    if arrow_down_pressed || tab_pressed {
                        this.branch_picker_selected_index =
                            Some(match this.branch_picker_selected_index {
                                Some(ix) if ix + 1 < match_count => ix + 1,
                                _ if match_count > 0 => 0,
                                _ => return,
                            });
                        this.picker_prompt_scroll
                            .scroll_to_item(this.branch_picker_selected_index.unwrap());
                        cx.notify();
                        return;
                    }

                    if enter_pressed {
                        if is_create_from_ref {
                            let name = if let Some(sel) = this.branch_picker_selected_index
                                && let Some(name) = matches.get(sel)
                            {
                                name.clone()
                            } else {
                                input.read_with(cx, |input, _| input.text().trim().to_string())
                            };
                            if !name.is_empty() {
                                let repo_id = repo.id;
                                this.handle_inline_branch_picker_select(name, repo_id, cx);
                                return;
                            }
                        } else if let Some(sel) = this.branch_picker_selected_index {
                            if let Some(name) = matches.get(sel) {
                                let name = name.clone();
                                let repo_id = repo.id;
                                this.handle_inline_branch_picker_select(name, repo_id, cx);
                                return;
                            }
                        }
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
        input.update(cx, |input, cx| {
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
        input.update(cx, |input, cx| {
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
        input.update(cx, |input, cx| {
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

fn count_recent_repo_matches(paths: &[std::path::PathBuf], query: &str) -> usize {
    if query.is_empty() {
        return paths.len();
    }
    let query_lower = query.to_ascii_lowercase();
    paths
        .iter()
        .filter(|path| {
            recent_repo_display_text(path)
                .to_ascii_lowercase()
                .contains(&query_lower)
        })
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
