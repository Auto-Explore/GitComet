use super::*;

impl DetailsPaneView {
    pub(in super::super) fn commit_details_view(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let active_repo_id = self.active_repo_id();
        let selected_id = self
            .active_repo()
            .and_then(|repo| repo.selected_commit.clone());

        if let (Some(repo_id), Some(selected_id)) = (active_repo_id, selected_id) {
            let show_delayed_loading = self.commit_details_delay.as_ref().is_some_and(|s| {
                s.repo_id == repo_id && s.commit_id == selected_id && s.show_loading
            });

            let header_title: SharedString = "Commit details".into();

            let header = div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(zed::CONTROL_HEIGHT_MD_PX))
                .px_2()
                .bg(theme.colors.surface_bg_elevated)
                .border_b_1()
                .border_color(theme.colors.border)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .line_clamp(1)
                        .child(header_title),
                )
                .child(
                    zed::Button::new("commit_details_close", "✕")
                        .style(zed::ButtonStyle::Transparent)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            if let Some(repo_id) = this.active_repo_id() {
                                this.store.dispatch(Msg::ClearCommitSelection { repo_id });
                                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                            }
                            cx.notify();
                        })
                        .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                            let text: SharedString = "Close commit details".into();
                            let mut changed = false;
                            if *hovering {
                                changed |= this.set_tooltip_text_if_changed(Some(text.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&text, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        })),
                );

            let body: AnyElement = match self.active_repo().map(|r| &r.commit_details) {
                None => zed::empty_state(theme, "Commit", "No repository.").into_any_element(),
                Some(Loadable::Loading) => {
                    if show_delayed_loading {
                        zed::empty_state(theme, "Commit", "Loading").into_any_element()
                    } else {
                        div().into_any_element()
                    }
                }
                Some(Loadable::Error(e)) => {
                    zed::empty_state(theme, "Commit", e.clone()).into_any_element()
                }
                Some(Loadable::NotLoaded) => {
                    if show_delayed_loading {
                        zed::empty_state(theme, "Commit", "Loading").into_any_element()
                    } else {
                        div().into_any_element()
                    }
                }
                Some(Loadable::Ready(details)) => {
                    if details.id != selected_id {
                        if show_delayed_loading {
                            zed::empty_state(theme, "Commit", "Loading").into_any_element()
                        } else {
                            let parent = details
                                .parent_ids
                                .first()
                                .map(|p: &CommitId| p.as_ref().to_string())
                                .unwrap_or_else(|| "—".to_string());

                            let files = if details.files.is_empty() {
                                div()
                                    .text_sm()
                                    .text_color(theme.colors.text_muted)
                                    .child("No files.")
                                    .into_any_element()
                            } else {
                                let total_files = details.files.len();
                                let list = uniform_list(
                                    ("commit_details_files_list", repo_id.0),
                                    total_files,
                                    cx.processor(Self::render_commit_file_rows),
                                )
                                .w_full()
                                .flex_1()
                                .min_h(px(0.0))
                                .track_scroll(self.commit_files_scroll.clone());
                                let scroll_handle =
                                    self.commit_files_scroll.0.borrow().base_handle.clone();

                                div()
                                    .id(("commit_details_files_container", repo_id.0))
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .h_full()
                                    .min_h(px(0.0))
                                    .w_full()
                                    .child(list)
                                    .child(
                                        zed::Scrollbar::new(
                                            ("commit_details_files_scrollbar", repo_id.0),
                                            scroll_handle,
                                        )
                                        .render(theme),
                                    )
                                    .into_any_element()
                            };

                            let needs_update = self.commit_details_message_input.read(cx).text()
                                != details.message.as_str();
                            if needs_update {
                                self.commit_details_message_input.update(cx, |input, cx| {
                                    input.set_text(details.message.clone(), cx);
                                });
                            }

                            let message = div()
                                .id(("commit_details_message_container", repo_id.0))
                                .relative()
                                .w_full()
                                .min_w(px(0.0))
                                .child(
                                    div()
                                        .id(("commit_details_message_scroll_surface", repo_id.0))
                                        .relative()
                                        .w_full()
                                        .min_w(px(0.0))
                                        .max_h(px(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX))
                                        .overflow_y_scroll()
                                        .track_scroll(&self.commit_scroll)
                                        .child(self.commit_details_message_input.clone()),
                                )
                                .child(
                                    zed::Scrollbar::new(
                                        ("commit_details_message_scrollbar", repo_id.0),
                                        self.commit_scroll.clone(),
                                    )
                                    .render(theme),
                                );

                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .flex_1()
                                .h_full()
                                .min_h(px(0.0))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .w_full()
                                        .min_w(px(0.0))
                                        .child(message)
                                        .child(zed::key_value_monospace_value(
                                            theme,
                                            "Commit SHA",
                                            details.id.as_ref().to_string(),
                                        ))
                                        .child(zed::key_value_monospace_value(
                                            theme,
                                            "Commit date",
                                            details.committed_at.clone(),
                                        ))
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .gap_1()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(theme.colors.text_muted)
                                                        .child("Parent commit SHA"),
                                                )
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_family("monospace")
                                                        .whitespace_nowrap()
                                                        .line_clamp(1)
                                                        .child(parent),
                                                ),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .flex_1()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(theme.colors.text_muted)
                                                .child("Committed files"),
                                        )
                                        .child(files),
                                )
                                .into_any_element()
                        }
                    } else {
                        let parent = details
                            .parent_ids
                            .first()
                            .map(|p: &CommitId| p.as_ref().to_string())
                            .unwrap_or_else(|| "—".to_string());

                        let files = if details.files.is_empty() {
                            div()
                                .text_sm()
                                .text_color(theme.colors.text_muted)
                                .child("No files.")
                                .into_any_element()
                        } else {
                            let total_files = details.files.len();
                            let list = uniform_list(
                                ("commit_details_files_list", repo_id.0),
                                total_files,
                                cx.processor(Self::render_commit_file_rows),
                            )
                            .w_full()
                            .flex_1()
                            .min_h(px(0.0))
                            .track_scroll(self.commit_files_scroll.clone());
                            let scroll_handle =
                                self.commit_files_scroll.0.borrow().base_handle.clone();

                            div()
                                .id(("commit_details_files_container", repo_id.0))
                                .relative()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .h_full()
                                .min_h(px(0.0))
                                .w_full()
                                .child(list)
                                .child(
                                    zed::Scrollbar::new(
                                        ("commit_details_files_scrollbar", repo_id.0),
                                        scroll_handle,
                                    )
                                    .render(theme),
                                )
                                .into_any_element()
                        };

                        let needs_update = self.commit_details_message_input.read(cx).text()
                            != details.message.as_str();
                        if needs_update {
                            self.commit_details_message_input.update(cx, |input, cx| {
                                input.set_text(details.message.clone(), cx);
                            });
                        }

                        let message = div()
                            .id(("commit_details_message_container", repo_id.0))
                            .relative()
                            .w_full()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .id(("commit_details_message_scroll_surface", repo_id.0))
                                    .relative()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .max_h(px(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.commit_scroll)
                                    .child(self.commit_details_message_input.clone()),
                            )
                            .child(
                                zed::Scrollbar::new(
                                    ("commit_details_message_scrollbar", repo_id.0),
                                    self.commit_scroll.clone(),
                                )
                                .render(theme),
                            );

                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .flex_1()
                            .h_full()
                            .min_h(px(0.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .child(message)
                                    .child(zed::key_value_monospace_value(
                                        theme,
                                        "Commit SHA",
                                        details.id.as_ref().to_string(),
                                    ))
                                    .child(zed::key_value_monospace_value(
                                        theme,
                                        "Commit date",
                                        details.committed_at.clone(),
                                    ))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(theme.colors.text_muted)
                                                    .child("Parent commit SHA"),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_family("monospace")
                                                    .whitespace_nowrap()
                                                    .line_clamp(1)
                                                    .child(parent),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .flex_1()
                                    .h_full()
                                    .min_h(px(0.0))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme.colors.text_muted)
                                            .child("Committed files"),
                                    )
                                    .child(files),
                            )
                            .into_any_element()
                    }
                }
            };

            return div()
                .id("commit_details_container")
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .h_full()
                .min_h(px(0.0))
                .child(header)
                .child(
                    div()
                        .id("commit_details_body_container")
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .h_full()
                        .min_h(px(0.0))
                        .p_2()
                        .child(body),
                )
                .into_any_element();
        }

        let repo = self.active_repo();
        let local_actions_in_flight = repo.map(|r| r.local_actions_in_flight > 0).unwrap_or(false);
        let (staged_count, unstaged_count) = repo
            .and_then(|r| match &r.status {
                Loadable::Ready(s) => Some((s.staged.len(), s.unstaged.len())),
                _ => None,
            })
            .unwrap_or((0, 0));

        let repo_id = self.active_repo_id();
        let selected_unstaged = repo_id
            .and_then(|rid| {
                self.status_multi_selection
                    .get(&rid)
                    .map(|s| s.unstaged.len())
            })
            .unwrap_or(0);
        let selected_staged = repo_id
            .and_then(|rid| {
                self.status_multi_selection
                    .get(&rid)
                    .map(|s| s.staged.len())
            })
            .unwrap_or(0);

        let spinner = |id: (&'static str, u64), color: gpui::Rgba| svg_spinner(id, color, px(14.0));
        let repo_key = repo_id.map(|id| id.0).unwrap_or(0);

        let stage_all = zed::Button::new("stage_all", "Stage all changes")
            .style(zed::ButtonStyle::Subtle)
            .disabled(local_actions_in_flight)
            .on_click(theme, cx, |this, _e, _w, cx| {
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };
                this.status_multi_selection.remove(&repo_id);
                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                this.store.dispatch(Msg::StagePaths {
                    repo_id,
                    paths: Vec::new(),
                });
                cx.notify();
            })
            .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                let text: SharedString = "Stage all changes".into();
                let mut changed = false;
                if *hovering {
                    changed |= this.set_tooltip_text_if_changed(Some(text.clone()), cx);
                } else {
                    changed |= this.clear_tooltip_if_matches(&text, cx);
                }
                if changed {
                    cx.notify();
                }
            }));

        let stage_selected =
            zed::Button::new("stage_selected", format!("Stage ({selected_unstaged})"))
                .style(zed::ButtonStyle::Outlined)
                .disabled(local_actions_in_flight)
                .on_click(theme, cx, |this, _e, _w, cx| {
                    let Some(repo_id) = this.active_repo_id() else {
                        return;
                    };
                    let paths = this
                        .status_multi_selection
                        .remove(&repo_id)
                        .map(|s| s.unstaged)
                        .unwrap_or_default();
                    if paths.is_empty() {
                        return;
                    }
                    this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                    this.store.dispatch(Msg::StagePaths { repo_id, paths });
                    cx.notify();
                });

        let discard_selected =
            zed::Button::new("discard_selected", format!("Discard ({selected_unstaged})"))
                .style(zed::ButtonStyle::Outlined)
                .disabled(local_actions_in_flight)
                .on_click(theme, cx, |this, e, window, cx| {
                    let Some(repo_id) = this.active_repo_id() else {
                        return;
                    };
                    let count = this
                        .status_multi_selection
                        .get(&repo_id)
                        .map(|s| s.unstaged.len())
                        .unwrap_or(0);
                    if count == 0 {
                        return;
                    }
                    this.open_popover_at(
                        PopoverKind::DiscardChangesConfirm {
                            repo_id,
                            area: DiffArea::Unstaged,
                            path: None,
                        },
                        e.position(),
                        window,
                        cx,
                    );
                    cx.notify();
                });

        let unstage_all = zed::Button::new("unstage_all", "Unstage all changes")
            .style(zed::ButtonStyle::Subtle)
            .disabled(local_actions_in_flight)
            .on_click(theme, cx, |this, _e, _w, cx| {
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };
                this.status_multi_selection.remove(&repo_id);
                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                this.store.dispatch(Msg::UnstagePaths {
                    repo_id,
                    paths: Vec::new(),
                });
                cx.notify();
            })
            .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                let text: SharedString = "Unstage all changes".into();
                let mut changed = false;
                if *hovering {
                    changed |= this.set_tooltip_text_if_changed(Some(text.clone()), cx);
                } else {
                    changed |= this.clear_tooltip_if_matches(&text, cx);
                }
                if changed {
                    cx.notify();
                }
            }));

        let unstage_selected =
            zed::Button::new("unstage_selected", format!("Unstage ({selected_staged})"))
                .style(zed::ButtonStyle::Outlined)
                .disabled(local_actions_in_flight)
                .on_click(theme, cx, |this, _e, _w, cx| {
                    let Some(repo_id) = this.active_repo_id() else {
                        return;
                    };
                    let paths = this
                        .status_multi_selection
                        .remove(&repo_id)
                        .map(|s| s.staged)
                        .unwrap_or_default();
                    if paths.is_empty() {
                        return;
                    }
                    this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                    this.store.dispatch(Msg::UnstagePaths { repo_id, paths });
                    cx.notify();
                });

        let section_header = |id: &'static str,
                              label: &'static str,
                              show_action: bool,
                              action: gpui::AnyElement|
         -> gpui::AnyElement {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_between()
                .h(px(zed::CONTROL_HEIGHT_MD_PX))
                .px_2()
                .bg(theme.colors.surface_bg_elevated)
                .border_b_1()
                .border_color(theme.colors.border)
                .child(div().text_sm().font_weight(FontWeight::BOLD).child(label))
                .when(show_action, |d| d.child(action))
                .into_any_element()
        };

        let unstaged_actions = {
            let mut actions = div().flex().items_center().gap_2();
            if local_actions_in_flight {
                actions = actions.child(
                    spinner(
                        ("unstaged_actions_spinner", repo_key),
                        with_alpha(theme.colors.accent, if theme.is_dark { 0.72 } else { 0.82 }),
                    )
                    .into_any_element(),
                );
            }
            if selected_unstaged > 0 {
                actions = actions.child(stage_selected).child(discard_selected);
            }
            actions.child(stage_all).into_any_element()
        };

        let staged_actions = {
            let mut actions = div().flex().items_center().gap_2();
            if local_actions_in_flight {
                actions = actions.child(
                    spinner(
                        ("staged_actions_spinner", repo_key),
                        with_alpha(theme.colors.accent, if theme.is_dark { 0.72 } else { 0.82 }),
                    )
                    .into_any_element(),
                );
            }
            if selected_staged > 0 {
                actions = actions.child(unstage_selected);
            }
            actions.child(unstage_all).into_any_element()
        };

        let unstaged_body = if unstaged_count == 0 {
            zed::empty_state(theme, "Unstaged", "Clean.").into_any_element()
        } else {
            self.status_list(cx, DiffArea::Unstaged, unstaged_count)
        };

        let staged_list = if staged_count == 0 {
            zed::empty_state(theme, "Staged", "No staged changes.").into_any_element()
        } else {
            self.status_list(cx, DiffArea::Staged, staged_count)
        };

        let unstaged_section = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .child(section_header(
                "unstaged_header",
                "Unstaged",
                unstaged_count > 0,
                unstaged_actions,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(unstaged_body),
            );

        let staged_section = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .child(section_header(
                "staged_header",
                "Staged",
                staged_count > 0,
                staged_actions,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(staged_list),
            );

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .h_full()
            .child(if repo_id.is_some() {
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(unstaged_section)
                    .child(div().border_t_1().border_color(theme.colors.border))
                    .child(staged_section)
                    .child(
                        div()
                            .border_t_1()
                            .border_color(theme.colors.border)
                            .bg(theme.colors.surface_bg)
                            .px_2()
                            .py_2()
                            .child(self.commit_box(staged_count > 0, cx)),
                    )
                    .into_any_element()
            } else {
                zed::empty_state(theme, "Changes", "No repository selected.").into_any_element()
            })
            .into_any_element()
    }

    pub(in super::super) fn status_list(
        &mut self,
        cx: &mut gpui::Context<Self>,
        area: DiffArea,
        count: usize,
    ) -> AnyElement {
        let theme = self.theme;
        if count == 0 {
            return zed::empty_state(theme, "Status", "Clean.").into_any_element();
        }
        match area {
            DiffArea::Unstaged => {
                let list =
                    uniform_list("unstaged", count, cx.processor(Self::render_unstaged_rows))
                        .flex_1()
                        .min_h(px(0.0))
                        .track_scroll(self.unstaged_scroll.clone());
                let scroll_handle = self.unstaged_scroll.0.borrow().base_handle.clone();
                div()
                    .id("unstaged_scroll_container")
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .child(list)
                    .child(zed::Scrollbar::new("unstaged_scrollbar", scroll_handle).render(theme))
                    .into_any_element()
            }
            DiffArea::Staged => {
                let list = uniform_list("staged", count, cx.processor(Self::render_staged_rows))
                    .flex_1()
                    .min_h(px(0.0))
                    .track_scroll(self.staged_scroll.clone());
                let scroll_handle = self.staged_scroll.0.borrow().base_handle.clone();
                div()
                    .id("staged_scroll_container")
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .child(list)
                    .child(zed::Scrollbar::new("staged_scrollbar", scroll_handle).render(theme))
                    .into_any_element()
            }
        }
    }

    pub(in super::super) fn commit_box(
        &mut self,
        can_commit: bool,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let commit_in_flight = self
            .active_repo()
            .is_some_and(|repo| repo.commit_in_flight > 0);
        let repo_key = self.active_repo_id().map(|id| id.0).unwrap_or(0);
        let icon_color = theme.colors.accent;
        let icon = |path: &'static str| svg_icon(path, icon_color, px(14.0));
        let spinner = |id: (&'static str, u64)| svg_spinner(id, icon_color, px(14.0));
        if let Some(message) =
            self.active_repo()
                .and_then(|repo| match &repo.merge_commit_message {
                    Loadable::Ready(Some(msg)) => Some(msg.clone()),
                    _ => None,
                })
        {
            let current = self.commit_message_input.read(cx).text();
            if current.trim().is_empty() && !self.commit_message_user_edited {
                self.commit_message_programmatic_change = true;
                self.commit_message_input
                    .update(cx, |i, cx| i.set_text(message, cx));
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.commit_message_input.clone())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("Commit staged changes"),
                    )
                    .child(
                        div().flex().items_center().gap_2().child(
                            zed::Button::new("commit", "Commit")
                                .start_slot(if commit_in_flight {
                                    spinner(("commit_spinner", repo_key)).into_any_element()
                                } else {
                                    icon("icons/check.svg").into_any_element()
                                })
                                .style(zed::ButtonStyle::Filled)
                                .disabled(!can_commit || commit_in_flight)
                                .on_click(theme, cx, |this, _e, _w, cx| {
                                    let Some(repo_id) = this.active_repo_id() else {
                                        return;
                                    };
                                    let message = this
                                        .commit_message_input
                                        .read_with(cx, |i, _| i.text().trim().to_string());
                                    if message.is_empty() {
                                        return;
                                    }
                                    this.store.dispatch(Msg::Commit { repo_id, message });
                                    this.commit_message_programmatic_change = true;
                                    this.commit_message_input
                                        .update(cx, |i, cx| i.set_text(String::new(), cx));
                                    cx.notify();
                                })
                                .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                                    let text: SharedString = "Commit staged changes".into();
                                    let mut changed = false;
                                    if *hovering {
                                        changed |= this
                                            .set_tooltip_text_if_changed(Some(text.clone()), cx);
                                    } else {
                                        changed |= this.clear_tooltip_if_matches(&text, cx);
                                    }
                                    if changed {
                                        cx.notify();
                                    }
                                })),
                        ),
                    ),
            )
    }
}
