//! The details pane body: a worktree, comparison, multi-commit or single
//! commit view when one is selected, otherwise the status sections and commit box.

use super::*;

impl DetailsPaneView {
    pub(in crate::view) fn commit_details_view(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let active_repo_id = self.active_repo_id();
        let selected_id = self
            .active_repo()
            .and_then(|repo| repo.history_state.selected_commit.clone());

        // A selected worktree row owns the pane outright: its files belong to a
        // different checkout, so none of the commit-detail views below apply.
        //
        // Only while its scan entry is actually there, though. The reducer drops
        // the selection when the worktree goes clean, but a scan that is still in
        // flight (or that failed) leaves the selection pointing at nothing for a
        // frame or two, and this view has nothing to render without it.
        let has_worktree_selection = self.selected_worktree_summary().is_some();
        if let (Some(repo_id), true) = (active_repo_id, has_worktree_selection) {
            return self.worktree_uncommitted_view(repo_id, cx);
        }

        // An active two-point comparison takes precedence over both the single
        // and multi commit-detail views: show the range's changed files.
        let has_range_comparison = self
            .active_repo()
            .is_some_and(|repo| repo.history_state.range_selection.is_some());
        if let (Some(repo_id), true) = (active_repo_id, has_range_comparison) {
            return self.range_comparison_view(repo_id, cx);
        }

        let multi_count = self
            .active_repo()
            .filter(|repo| repo.history_state.multi_selection.is_multi())
            .map(|repo| repo.history_state.multi_selection.commits.len());
        if let (Some(repo_id), Some(count)) = (active_repo_id, multi_count) {
            return self.multi_commit_details_view(repo_id, count, cx);
        }

        if let (Some(repo_id), Some(selected_id)) = (active_repo_id, selected_id) {
            let show_delayed_loading = self.commit_details_delay.as_ref().is_some_and(|s| {
                s.repo_id == repo_id && s.commit_id == selected_id && s.show_loading
            });

            let header_title: SharedString = "Commit details".into();

            let header = div()
                .flex()
                .items_center()
                .justify_between()
                .h(components::content_header_height(ui_scale))
                .px_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(theme.ui_text(14.0))
                        .font_weight(FontWeight::BOLD)
                        .line_clamp(1)
                        .child(header_title),
                )
                .child(
                    components::Button::new("commit_details_close", "")
                        .start_slot(
                            svg_icon(
                                "icons/generic_close.svg",
                                theme.colors.foreground.secondary,
                                ui_scale.px(12.0),
                            )
                            .debug_selector(|| "commit_details_close_icon".to_string()),
                        )
                        .style(components::ButtonStyle::Transparent)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            // The commit details and diff views are independent
                            // panels; closing details must not close the diff.
                            if let Some(repo_id) = this.active_repo_id() {
                                this.store.dispatch(Msg::ClearCommitSelection { repo_id });
                            }
                            cx.notify();
                        })
                        .gitcomet_tooltip(theme, "Close commit details".into()),
                );

            let active_commit_details = self.active_repo().map(|repo| {
                (
                    repo.history_state.commit_details.clone(),
                    repo.history_state.commit_details_rev,
                )
            });
            let commit_signatures = self
                .active_repo()
                .map(|repo| repo.history_state.commit_signatures.clone())
                .unwrap_or_default();
            let commit_details_rev = active_commit_details
                .as_ref()
                .map(|(_, revision)| *revision)
                .unwrap_or_default();
            let body: AnyElement = match active_commit_details.as_ref().map(|(details, _)| details)
            {
                None => {
                    components::empty_state(theme, "Commit", "No repository.").into_any_element()
                }
                Some(Loadable::Loading) => {
                    if show_delayed_loading {
                        components::empty_state(theme, "Commit", "Loading").into_any_element()
                    } else {
                        div().into_any_element()
                    }
                }
                Some(Loadable::Error(e)) => {
                    components::empty_state(theme, "Commit", e.clone()).into_any_element()
                }
                Some(Loadable::NotLoaded) => {
                    if show_delayed_loading {
                        components::empty_state(theme, "Commit", "Loading").into_any_element()
                    } else {
                        div().into_any_element()
                    }
                }
                Some(Loadable::Ready(details)) => {
                    if details.id != selected_id {
                        if show_delayed_loading {
                            components::empty_state(theme, "Commit", "Loading").into_any_element()
                        } else {
                            let parent = details
                                .parent_ids
                                .first()
                                .map(|p: &CommitId| p.as_ref().to_string())
                                .unwrap_or_else(|| "—".to_string());

                            self.sync_retained_commit_details_message_input(
                                details.message.as_str(),
                                cx,
                            );
                            Self::sync_commit_details_input_value(
                                &self.commit_details_sha_input,
                                details.id.as_ref(),
                                cx,
                            );
                            Self::sync_commit_details_input_value(
                                &self.commit_details_date_input,
                                self.commit_details_date_display(details).as_str(),
                                cx,
                            );
                            self.sync_commit_details_parent_input(
                                parent.as_str(),
                                RepoId(0),
                                false,
                                theme,
                                cx,
                            );

                            let message = self.commit_details_message_view(theme, repo_id);

                            div()
                                .flex()
                                .flex_col()
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
                                        .pb_2()
                                        .child(message),
                                )
                                .children(
                                    commit_details_author_row(
                                        theme,
                                        ui_scale,
                                        details,
                                        commit_signatures.get(&details.id),
                                    )
                                    .map(|row| {
                                        row.border_t_1()
                                            .border_color(theme.colors.stroke.default)
                                            .pt_2()
                                            .pb_2()
                                    }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .w_full()
                                        .min_w(px(0.0))
                                        .border_t_1()
                                        .border_color(theme.colors.stroke.default)
                                        .pt_2()
                                        .pb_2()
                                        .child(commit_details_selectable_row(
                                            theme,
                                            "Commit SHA",
                                            commit_details_monospace_value(
                                                self.commit_details_sha_input.clone(),
                                            ),
                                        ))
                                        .child(commit_details_selectable_row(
                                            theme,
                                            "Commit date",
                                            commit_details_monospace_value(
                                                self.commit_details_date_input.clone(),
                                            ),
                                        ))
                                        .child(commit_details_selectable_row(
                                            theme,
                                            "Parent commit SHA",
                                            commit_details_monospace_value(
                                                self.commit_details_parent_input.clone(),
                                            ),
                                        )),
                                )
                                .child(self.commit_files_section(
                                    repo_id,
                                    commit_details_rev,
                                    details,
                                    cx,
                                ))
                                .into_any_element()
                        }
                    } else {
                        let parent = details
                            .parent_ids
                            .first()
                            .map(|p: &CommitId| p.as_ref().to_string())
                            .unwrap_or_else(|| "—".to_string());

                        self.sync_commit_details_message_input(
                            details.message.as_str(),
                            theme,
                            repo_id,
                            cx,
                        );
                        Self::sync_commit_details_input_value(
                            &self.commit_details_sha_input,
                            details.id.as_ref(),
                            cx,
                        );
                        Self::sync_commit_details_input_value(
                            &self.commit_details_date_input,
                            self.commit_details_date_display(details).as_str(),
                            cx,
                        );
                        self.sync_commit_details_sha_menu(
                            details.id.as_ref(),
                            repo_id,
                            true,
                            theme,
                            cx,
                        );
                        self.sync_commit_details_parent_input(
                            parent.as_str(),
                            repo_id,
                            parent != "—",
                            theme,
                            cx,
                        );

                        let message = self.commit_details_message_view(theme, repo_id);

                        div()
                            .flex()
                            .flex_col()
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
                                    .pb_2()
                                    .child(message),
                            )
                            .children(
                                commit_details_author_row(
                                    theme,
                                    ui_scale,
                                    details,
                                    commit_signatures.get(&details.id),
                                )
                                .map(|row| {
                                    row.border_t_1()
                                        .border_color(theme.colors.stroke.default)
                                        .pt_2()
                                        .pb_2()
                                }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .border_t_1()
                                    .border_color(theme.colors.stroke.default)
                                    .pt_2()
                                    .pb_2()
                                    .child(commit_details_selectable_row(
                                        theme,
                                        "Commit SHA",
                                        commit_details_monospace_element(
                                            self.commit_details_sha_link_menu
                                                .clone()
                                                .into_any_element(),
                                        ),
                                    ))
                                    .child(commit_details_selectable_row(
                                        theme,
                                        "Commit date",
                                        commit_details_monospace_value(
                                            self.commit_details_date_input.clone(),
                                        ),
                                    ))
                                    .child(commit_details_selectable_row(
                                        theme,
                                        "Parent commit SHA",
                                        commit_details_monospace_element(
                                            self.commit_details_parent_link_menu
                                                .clone()
                                                .into_any_element(),
                                        ),
                                    )),
                            )
                            .child(self.commit_files_section(
                                repo_id,
                                commit_details_rev,
                                details,
                                cx,
                            ))
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

        let local_actions_in_flight = self
            .active_repo()
            .map(|r| r.local_actions_in_flight > 0)
            .unwrap_or(false);
        let (staged_count, unstaged_count, untracked_count, split_unstaged_count) = self
            .active_repo()
            .map(|repo| {
                (
                    self.status_section_entries(repo, StatusSection::Staged)
                        .map_or(0, |entries| entries.len()),
                    self.status_section_entries(repo, StatusSection::CombinedUnstaged)
                        .map_or(0, |entries| entries.len()),
                    self.status_section_entries(repo, StatusSection::Untracked)
                        .map_or(0, |entries| entries.len()),
                    self.status_section_entries(repo, StatusSection::Unstaged)
                        .map_or(0, |entries| entries.len()),
                )
            })
            .unwrap_or((0, 0, 0, 0));
        let (untracked_paths, split_unstaged_paths) = self
            .active_repo()
            .map(|repo| {
                (
                    self.status_section_entries(repo, StatusSection::Untracked)
                        .map_or_else(Vec::new, |entries| entries.path_vec()),
                    self.status_section_entries(repo, StatusSection::Unstaged)
                        .map_or_else(Vec::new, |entries| entries.path_vec()),
                )
            })
            .unwrap_or_else(|| (Vec::new(), Vec::new()));
        let (unstaged_loading, untracked_loading, split_unstaged_loading, staged_loading) = self
            .active_repo()
            .map(|repo| {
                (
                    status_section_is_loading(repo, StatusSection::CombinedUnstaged),
                    status_section_is_loading(repo, StatusSection::Untracked),
                    status_section_is_loading(repo, StatusSection::Unstaged),
                    status_section_is_loading(repo, StatusSection::Staged),
                )
            })
            .unwrap_or((false, false, false, false));

        let repo_id = self.active_repo_id();
        let selected_combined_unstaged = repo_id
            .map(|rid| {
                self.status_section_action_selection(rid, StatusSection::CombinedUnstaged)
                    .count()
            })
            .unwrap_or(0);
        let selected_untracked = repo_id
            .map(|rid| {
                self.status_section_action_selection(rid, StatusSection::Untracked)
                    .count()
            })
            .unwrap_or(0);
        let selected_split_unstaged = repo_id
            .map(|rid| {
                self.status_section_action_selection(rid, StatusSection::Unstaged)
                    .count()
            })
            .unwrap_or(0);
        let selected_staged = repo_id
            .map(|rid| {
                self.status_section_action_selection(rid, StatusSection::Staged)
                    .count()
            })
            .unwrap_or(0);

        let spinner =
            |id: (&'static str, u64), color: gpui::Rgba| svg_spinner(id, color, ui_scale.px(14.0));
        let repo_key = repo_id.map(|id| id.0).unwrap_or(0);
        let split_change_tracking = self.change_tracking_view == ChangeTrackingView::SplitUntracked;
        let icon_muted = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.72 } else { 0.82 },
        );
        let ui_scale_percent = crate::ui_scale::current(cx).percent;

        // Measured last frame by the probe on the sections container below; the
        // prepaint callback refreshes the window when it changes. Unmeasured on
        // the very first frame, which reads as "plenty of room" and settles on
        // the next one.
        let header_width = self
            .current_status_sections_bounds()
            .map(|bounds| bounds.size.width)
            .unwrap_or(Pixels::MAX);
        let labels_for =
            |title_chars: usize, title_is_dropdown: bool, action_label_chars: &[usize]| {
                status_action_labels_for_width(
                    header_width,
                    title_chars,
                    title_is_dropdown,
                    action_label_chars,
                    local_actions_in_flight,
                    ui_scale_percent,
                    theme.metrics,
                )
            };
        let count_chars =
            |word: &str, count: usize| word.chars().count() + 3 + count.to_string().len();
        let unstaged_labels = if selected_combined_unstaged > 0 {
            labels_for(
                "Unstaged".len(),
                true,
                &[
                    count_chars("Stage", selected_combined_unstaged),
                    count_chars("Discard", selected_combined_unstaged),
                    "Stage all changes".len(),
                ],
            )
        } else {
            labels_for("Unstaged".len(), true, &["Stage all changes".len()])
        };
        let untracked_labels = if selected_untracked > 0 {
            labels_for(
                "Untracked".len(),
                true,
                &[
                    count_chars("Stage", selected_untracked),
                    count_chars("Discard", selected_untracked),
                    "Stage all".len(),
                ],
            )
        } else {
            labels_for("Untracked".len(), true, &["Stage all".len()])
        };
        let split_unstaged_labels = if selected_split_unstaged > 0 {
            labels_for(
                "Unstaged".len(),
                true,
                &[
                    count_chars("Stage", selected_split_unstaged),
                    count_chars("Discard", selected_split_unstaged),
                    "Stage all".len(),
                ],
            )
        } else {
            labels_for("Unstaged".len(), true, &["Stage all".len()])
        };
        let staged_labels = if selected_staged > 0 {
            labels_for(
                "Staged".len(),
                false,
                &[
                    count_chars("Unstage", selected_staged),
                    "Unstage all changes".len(),
                ],
            )
        } else {
            labels_for("Staged".len(), false, &["Unstage all changes".len()])
        };

        let stage_all = components::Button::new(
            "stage_all",
            status_action_all_label(unstaged_labels, "Stage all changes"),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            // Empty paths: this button stages every change there is.
            this.stage_all_with_conflict_confirmation(repo_id, Vec::new(), _w, cx);
        })
        .debug_selector(|| "stage_all_button".to_string())
        .gitcomet_tooltip(theme, "Stage all changes".into());

        let stage_selected = components::Button::new(
            "stage_selected",
            status_action_count_label(unstaged_labels, "Stage", selected_combined_unstaged),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            // Read without consuming: the confirmation below can still be
            // cancelled, and that must leave the selection as the user built it.
            let selection =
                this.status_section_action_selection(repo_id, StatusSection::CombinedUnstaged);
            let paths = selection.paths;
            if paths.is_empty() {
                return;
            }
            if let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
                &this.state,
                repo_id,
                paths.clone(),
                selection.from_explicit_selection,
            ) {
                let anchor = crate::view::conflict_markers::centered_dialog_anchor(_w);
                this.open_popover_at(confirm, anchor, _w, cx);
                cx.notify();
                return;
            }
            if selection.from_explicit_selection {
                this.clear_status_multi_selection(repo_id);
            }
            crate::view::status_actions::stage_or_unstage_paths(
                &this.store,
                repo_id,
                DiffArea::Unstaged,
                paths,
            );
            cx.notify();
        })
        .debug_selector(|| "stage_selected_button".to_string())
        .gitcomet_tooltip(
            theme,
            format!(
                "Stage {selected_combined_unstaged} selected {}",
                status_action_file_count(selected_combined_unstaged)
            )
            .into(),
        );

        let discard_selected = components::Button::new(
            "discard_selected",
            status_action_count_label(unstaged_labels, "Discard", selected_combined_unstaged),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, e, window, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let selection =
                this.status_section_action_selection(repo_id, StatusSection::CombinedUnstaged);
            if selection.paths.is_empty() {
                return;
            }
            this.open_popover_at(
                PopoverKind::DiscardChangesConfirm {
                    repo_id,
                    area: DiffArea::Unstaged,
                    path: selection.popover_path(),
                },
                e.position(),
                window,
                cx,
            );
            cx.notify();
        })
        .gitcomet_tooltip(
            theme,
            format!(
                "Discard changes in {selected_combined_unstaged} selected {}",
                status_action_file_count(selected_combined_unstaged)
            )
            .into(),
        );

        let untracked_paths_for_stage_all =
            gitcomet_state::msg::RepoPathList::from(untracked_paths.clone());
        let stage_all_untracked = components::Button::new(
            "stage_all_untracked",
            status_action_all_label(untracked_labels, "Stage all"),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight || untracked_paths_for_stage_all.is_empty())
        .on_click(theme, cx, move |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            if untracked_paths_for_stage_all.is_empty() {
                return;
            }
            this.status_multi_selection.remove(&repo_id);
            crate::view::status_actions::stage_or_unstage_paths(
                &this.store,
                repo_id,
                DiffArea::Unstaged,
                untracked_paths_for_stage_all.clone(),
            );
            cx.notify();
        })
        .debug_selector(|| "stage_all_untracked_button".to_string())
        .gitcomet_tooltip(theme, "Stage all untracked files".into());

        let stage_selected_untracked = components::Button::new(
            "stage_selected_untracked",
            status_action_count_label(untracked_labels, "Stage", selected_untracked),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            // Read without consuming: the confirmation below can still be
            // cancelled, and that must leave the selection as the user built it.
            let selection = this.status_section_action_selection(repo_id, StatusSection::Untracked);
            let paths = selection.paths;
            if paths.is_empty() {
                return;
            }
            if let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
                &this.state,
                repo_id,
                paths.clone(),
                selection.from_explicit_selection,
            ) {
                let anchor = crate::view::conflict_markers::centered_dialog_anchor(_w);
                this.open_popover_at(confirm, anchor, _w, cx);
                cx.notify();
                return;
            }
            if selection.from_explicit_selection {
                this.clear_status_multi_selection(repo_id);
            }
            crate::view::status_actions::stage_or_unstage_paths(
                &this.store,
                repo_id,
                DiffArea::Unstaged,
                paths,
            );
            cx.notify();
        })
        .gitcomet_tooltip(
            theme,
            format!(
                "Stage {selected_untracked} selected {}",
                status_action_file_count(selected_untracked)
            )
            .into(),
        );

        let discard_selected_untracked = components::Button::new(
            "discard_selected_untracked",
            status_action_count_label(untracked_labels, "Discard", selected_untracked),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, e, window, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let selection = this.status_section_action_selection(repo_id, StatusSection::Untracked);
            if selection.paths.is_empty() {
                return;
            }
            this.open_popover_at(
                PopoverKind::DiscardChangesConfirm {
                    repo_id,
                    area: DiffArea::Unstaged,
                    path: selection.popover_path(),
                },
                e.position(),
                window,
                cx,
            );
            cx.notify();
        })
        .gitcomet_tooltip(
            theme,
            format!(
                "Discard changes in {selected_untracked} selected {}",
                status_action_file_count(selected_untracked)
            )
            .into(),
        );

        let split_unstaged_paths_for_stage_all = split_unstaged_paths.clone();
        let stage_all_split_unstaged = components::Button::new(
            "stage_all_split_unstaged",
            status_action_all_label(split_unstaged_labels, "Stage all"),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight || split_unstaged_paths_for_stage_all.is_empty())
        .on_click(theme, cx, move |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            if split_unstaged_paths_for_stage_all.is_empty() {
                return;
            }
            // Named paths: this button stages the tracked-changes section only —
            // conflicted files among them, so it needs the same confirmation the
            // combined view's button gets.
            this.stage_all_with_conflict_confirmation(
                repo_id,
                split_unstaged_paths_for_stage_all.clone(),
                _w,
                cx,
            );
        })
        .debug_selector(|| "stage_all_split_unstaged_button".to_string())
        .gitcomet_tooltip(theme, "Stage all unstaged changes".into());

        let stage_selected_split_unstaged = components::Button::new(
            "stage_selected_split_unstaged",
            status_action_count_label(split_unstaged_labels, "Stage", selected_split_unstaged),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            // Read without consuming: the confirmation below can still be
            // cancelled, and that must leave the selection as the user built it.
            let selection = this.status_section_action_selection(repo_id, StatusSection::Unstaged);
            let paths = selection.paths;
            if paths.is_empty() {
                return;
            }
            if let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
                &this.state,
                repo_id,
                paths.clone(),
                selection.from_explicit_selection,
            ) {
                let anchor = crate::view::conflict_markers::centered_dialog_anchor(_w);
                this.open_popover_at(confirm, anchor, _w, cx);
                cx.notify();
                return;
            }
            if selection.from_explicit_selection {
                this.clear_status_multi_selection(repo_id);
            }
            crate::view::status_actions::stage_or_unstage_paths(
                &this.store,
                repo_id,
                DiffArea::Unstaged,
                paths,
            );
            cx.notify();
        })
        .gitcomet_tooltip(
            theme,
            format!(
                "Stage {selected_split_unstaged} selected {}",
                status_action_file_count(selected_split_unstaged)
            )
            .into(),
        );

        let discard_selected_split_unstaged = components::Button::new(
            "discard_selected_split_unstaged",
            status_action_count_label(split_unstaged_labels, "Discard", selected_split_unstaged),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, e, window, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let selection = this.status_section_action_selection(repo_id, StatusSection::Unstaged);
            if selection.paths.is_empty() {
                return;
            }
            this.open_popover_at(
                PopoverKind::DiscardChangesConfirm {
                    repo_id,
                    area: DiffArea::Unstaged,
                    path: selection.popover_path(),
                },
                e.position(),
                window,
                cx,
            );
            cx.notify();
        })
        .gitcomet_tooltip(
            theme,
            format!(
                "Discard changes in {selected_split_unstaged} selected {}",
                status_action_file_count(selected_split_unstaged)
            )
            .into(),
        );

        let unstage_all = components::Button::new(
            "unstage_all",
            status_action_all_label(staged_labels, "Unstage all changes"),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            this.status_multi_selection.remove(&repo_id);
            crate::view::status_actions::stage_or_unstage_paths(
                &this.store,
                repo_id,
                DiffArea::Staged,
                gitcomet_state::msg::RepoPathList::default(),
            );
            cx.notify();
        })
        .debug_selector(|| "unstage_all_button".to_string())
        .gitcomet_tooltip(theme, "Unstage all changes".into());

        let unstage_selected = components::Button::new(
            "unstage_selected",
            status_action_count_label(staged_labels, "Unstage", selected_staged),
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let paths = this
                .take_status_section_action_selection(repo_id, StatusSection::Staged)
                .paths;
            if paths.is_empty() {
                return;
            }
            crate::view::status_actions::stage_or_unstage_paths(
                &this.store,
                repo_id,
                DiffArea::Staged,
                paths,
            );
            cx.notify();
        })
        .debug_selector(|| "unstage_selected_button".to_string())
        .gitcomet_tooltip(
            theme,
            format!(
                "Unstage {selected_staged} selected {}",
                status_action_file_count(selected_staged)
            )
            .into(),
        );

        let section_controls = |pane: &mut Self,
                                section: StatusSection,
                                id_prefix: &'static str,
                                cx: &mut gpui::Context<Self>|
         -> Option<gpui::AnyElement> {
            let repo_id = repo_id?;
            Some(pane.file_list_controls(
                crate::view::rows::FileListId::Status(section),
                repo_id,
                id_prefix,
                false,
                cx,
            ))
        };
        let unstaged_controls =
            section_controls(self, StatusSection::CombinedUnstaged, "status_unstaged", cx);
        let untracked_controls =
            section_controls(self, StatusSection::Untracked, "status_untracked", cx);
        let split_unstaged_controls =
            section_controls(self, StatusSection::Unstaged, "status_split_unstaged", cx);
        let staged_controls = section_controls(self, StatusSection::Staged, "status_staged", cx);

        let section_header = |id: &'static str,
                              title: gpui::AnyElement,
                              show_action: bool,
                              action: gpui::AnyElement|
         -> gpui::AnyElement {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .h(components::content_header_height(
                    ui_scale::UiScale::from_percent(ui_scale_percent)
                        .with_appearance(theme.metrics),
                ))
                .px_2()
                .overflow_hidden()
                // The labels shrink before this matters, but a UI zoom or a font
                // wider than the budget assumes can still overrun the header —
                // and then the title, not the actions, is what gives way.
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(title),
                )
                .when(show_action, |d| d.child(div().flex_none().child(action)))
                .into_any_element()
        };

        let normal_header_title = |label: &'static str| {
            div()
                .text_size(theme.ui_text(14.0))
                .font_weight(FontWeight::BOLD)
                .line_clamp(1)
                .whitespace_nowrap()
                .child(label)
                .into_any_element()
        };

        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let resize_handle_h = px(PANE_RESIZE_HANDLE_PX);

        let unstaged_actions = {
            let mut actions = div().flex().items_center().gap_2();
            if let Some(controls) = unstaged_controls {
                actions = actions.child(controls);
            }
            if local_actions_in_flight {
                actions = actions.child(
                    spinner(
                        ("unstaged_actions_spinner", repo_key),
                        with_alpha(
                            theme.colors.accent.foreground,
                            if theme.is_dark { 0.72 } else { 0.82 },
                        ),
                    )
                    .into_any_element(),
                );
            }
            if selected_combined_unstaged > 0 {
                actions = actions.child(stage_selected).child(discard_selected);
            }
            actions.child(stage_all).into_any_element()
        };

        let untracked_actions = {
            let mut actions = div().flex().items_center().gap_2();
            if let Some(controls) = untracked_controls {
                actions = actions.child(controls);
            }
            if local_actions_in_flight {
                actions = actions.child(
                    spinner(
                        ("untracked_actions_spinner", repo_key),
                        with_alpha(
                            theme.colors.accent.foreground,
                            if theme.is_dark { 0.72 } else { 0.82 },
                        ),
                    )
                    .into_any_element(),
                );
            }
            if selected_untracked > 0 {
                actions = actions
                    .child(stage_selected_untracked)
                    .child(discard_selected_untracked);
            }
            actions.child(stage_all_untracked).into_any_element()
        };

        let split_unstaged_actions = {
            let mut actions = div().flex().items_center().gap_2();
            if let Some(controls) = split_unstaged_controls {
                actions = actions.child(controls);
            }
            if local_actions_in_flight {
                actions = actions.child(
                    spinner(
                        ("split_unstaged_actions_spinner", repo_key),
                        with_alpha(
                            theme.colors.accent.foreground,
                            if theme.is_dark { 0.72 } else { 0.82 },
                        ),
                    )
                    .into_any_element(),
                );
            }
            if selected_split_unstaged > 0 {
                actions = actions
                    .child(stage_selected_split_unstaged)
                    .child(discard_selected_split_unstaged);
            }
            actions.child(stage_all_split_unstaged).into_any_element()
        };

        let staged_actions = {
            let mut actions = div().flex().items_center().gap_2();
            if let Some(controls) = staged_controls {
                actions = actions.child(controls);
            }
            if local_actions_in_flight {
                actions = actions.child(
                    spinner(
                        ("staged_actions_spinner", repo_key),
                        with_alpha(
                            theme.colors.accent.foreground,
                            if theme.is_dark { 0.72 } else { 0.82 },
                        ),
                    )
                    .into_any_element(),
                );
            }
            if selected_staged > 0 {
                actions = actions.child(unstage_selected);
            }
            actions.child(unstage_all).into_any_element()
        };

        let unstaged_body = if unstaged_loading {
            components::empty_state_message(theme, "Loading…").into_any_element()
        } else if unstaged_count == 0 {
            components::empty_state_message(theme, "No unstaged changes.").into_any_element()
        } else {
            self.status_list(cx, StatusSection::CombinedUnstaged, unstaged_count)
        };

        let untracked_body = if untracked_loading {
            components::empty_state_message(theme, "Loading…").into_any_element()
        } else if untracked_count == 0 {
            components::empty_state_message(theme, "No untracked files.").into_any_element()
        } else {
            self.status_list(cx, StatusSection::Untracked, untracked_count)
        };

        let split_unstaged_body = if split_unstaged_loading {
            components::empty_state_message(theme, "Loading…").into_any_element()
        } else if split_unstaged_count == 0 {
            components::empty_state_message(theme, "No unstaged changes.").into_any_element()
        } else {
            self.status_list(cx, StatusSection::Unstaged, split_unstaged_count)
        };

        let staged_list = if staged_loading {
            components::empty_state_message(theme, "Loading…").into_any_element()
        } else if staged_count == 0 {
            components::empty_state_message(theme, "Nothing staged yet.").into_any_element()
        } else {
            self.status_list(cx, StatusSection::Staged, staged_count)
        };

        let build_change_tracking_header_title =
            |id: &'static str, invoker_key: &'static str, label: &'static str| {
                let change_tracking_invoker: SharedString = invoker_key.into();
                let change_tracking_active =
                    self.active_context_menu_invoker.as_ref() == Some(&change_tracking_invoker);
                let change_tracking_invoker = change_tracking_invoker.clone();
                div()
                    .id(id)
                    .debug_selector(move || id.to_string())
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_1()
                    .h(ui_scale.row_height(
                        CHANGE_TRACKING_HEADER_CHIP_HEIGHT_PX,
                        CHANGE_TRACKING_HEADER_CHIP_COMFORTABLE_HEIGHT_PX,
                    ))
                    .rounded(px(theme.radii.row))
                    .tab_index(0)
                    .control_interaction(
                        InteractionStyle::header(theme),
                        InteractionState::default().open(change_tracking_active),
                    )
                    .child(
                        div()
                            .text_size(theme.ui_text(14.0))
                            .font_weight(FontWeight::BOLD)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(label),
                    )
                    .child(
                        svg_icon("icons/chevron_down.svg", icon_muted, ui_scale.px(12.0))
                            .debug_selector(move || format!("{id}_chevron")),
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, e: &ClickEvent, window, cx| {
                            this.open_popover_at(
                                PopoverKind::ChangeTrackingSettings
                                    .invoked_by(change_tracking_invoker.clone()),
                                e.position(),
                                window,
                                cx,
                            );
                            cx.notify();
                        }),
                    )
                    .into_any_element()
            };

        let build_unstaged_header_title = || {
            build_change_tracking_header_title(
                "change_tracking_unstaged_header",
                "change_tracking_unstaged_header",
                "Unstaged",
            )
        };

        let build_untracked_header_title = || {
            build_change_tracking_header_title(
                "change_tracking_untracked_header",
                "change_tracking_untracked_header",
                "Untracked",
            )
        };

        let active_status_resize = self.status_section_resize;
        let build_status_resize_handle = |id: &'static str, handle: StatusSectionResizeHandle| {
            let dragging = active_status_resize.is_some_and(|state| state.handle == handle);
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .group(id)
                .w_full()
                .h(resize_handle_h)
                .flex_none()
                .cursor(CursorStyle::ResizeUpDown)
                .child(components::resize_grip(
                    theme,
                    ui_scale,
                    id,
                    components::ResizeGripAxis::Horizontal,
                    dragging,
                    Some(theme.colors.stroke.default),
                ))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        crate::press_gesture::claim_press(cx);
                        crate::text_selection_owner::preserve(cx);
                        this.start_status_section_resize(handle, e.position.y, cx);
                        window.refresh();
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _e, window, cx| {
                        if this
                            .status_section_resize
                            .is_some_and(|state| state.handle == handle)
                        {
                            this.finish_status_section_resize(cx);
                            window.refresh();
                        }
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(move |this, _e, window, cx| {
                        if this
                            .status_section_resize
                            .is_some_and(|state| state.handle == handle)
                        {
                            this.finish_status_section_resize(cx);
                            window.refresh();
                        }
                    }),
                )
        };

        let with_split_sizing = |mut section: gpui::Div,
                                 exact_height: Option<Pixels>,
                                 fallback_grow: f32,
                                 min_h: Pixels| {
            section = section.min_h(min_h);
            if let Some(exact_height) = exact_height {
                let exact_height = exact_height.max(min_h);
                section = section.h(exact_height).max_h(exact_height);
                section.style().flex_grow = Some(0.0);
                section.style().flex_shrink = Some(0.0);
                section.style().flex_basis = Some(exact_height.into());
            } else {
                section.style().flex_grow = Some(fallback_grow.max(1.0));
                section.style().flex_shrink = Some(1.0);
                section.style().flex_basis = Some(relative(0.0).into());
            }
            section
        };
        let px_to_grow = |value: Pixels| -> f32 {
            let px_value: f32 = value.into();
            px_value.max(1.0)
        };

        let change_tracking_total_height =
            self.measured_status_sections_total_height(resize_handle_h);
        let change_tracking_heights = change_tracking_total_height.map(|total_height| {
            let top_height = resolved_vertical_split_height(
                self.change_tracking_height,
                total_height,
                min_change_tracking_stack_height(split_change_tracking, resize_handle_h),
                section_min_h,
            );
            (top_height, (total_height - top_height).max(section_min_h))
        });

        let untracked_total_height =
            self.resolved_measured_change_tracking_stack_total_height(resize_handle_h);
        let untracked_heights = untracked_total_height.map(|total_height| {
            let top_height = resolved_vertical_split_height(
                self.untracked_height,
                total_height,
                section_min_h,
                section_min_h,
            );
            (top_height, (total_height - top_height).max(section_min_h))
        });
        let unstaged_section = self
            .status_section_container(StatusSection::CombinedUnstaged, cx)
            .flex()
            .flex_col()
            .min_h(section_min_h)
            .overflow_hidden()
            .child(section_header(
                "unstaged_header",
                build_unstaged_header_title(),
                unstaged_count > 0,
                unstaged_actions,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(unstaged_body),
            );

        let untracked_section = self
            .status_section_container(StatusSection::Untracked, cx)
            .flex()
            .flex_col()
            .min_h(section_min_h)
            .overflow_hidden()
            .child(section_header(
                "untracked_header",
                build_untracked_header_title(),
                untracked_count > 0,
                untracked_actions,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(untracked_body),
            );

        let split_unstaged_section = self
            .status_section_container(StatusSection::Unstaged, cx)
            .flex()
            .flex_col()
            .min_h(section_min_h)
            .overflow_hidden()
            .child(section_header(
                "split_unstaged_header",
                build_unstaged_header_title(),
                split_unstaged_count > 0,
                split_unstaged_actions,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(split_unstaged_body),
            );

        let staged_section = self
            .status_section_container(StatusSection::Staged, cx)
            .flex()
            .flex_col()
            .min_h(section_min_h)
            .overflow_hidden()
            .child(section_header(
                "staged_header",
                normal_header_title("Staged"),
                staged_count > 0,
                staged_actions,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(staged_list),
            );

        let change_tracking_section = if split_change_tracking {
            let change_tracking_stack_bounds_for_prepaint =
                std::rc::Rc::clone(&self.change_tracking_stack_bounds_ref);
            let stack_container = div()
                .relative()
                .flex()
                .flex_col()
                .w_full()
                .min_w_full()
                .max_w_full()
                .h_full()
                .min_h(min_change_tracking_stack_height(
                    split_change_tracking,
                    resize_handle_h,
                ))
                .overflow_hidden()
                .on_children_prepainted(move |children_bounds, window, _app| {
                    let next_bounds = children_bounds.first().copied();
                    let mut measured = change_tracking_stack_bounds_for_prepaint.borrow_mut();
                    if *measured != next_bounds {
                        *measured = next_bounds;
                        window.refresh();
                    }
                });
            let untracked_top_height = untracked_heights.map(|(top_height, _)| top_height);
            let split_unstaged_height = untracked_heights.map(|(_, bottom_height)| bottom_height);
            let (untracked_grow, split_unstaged_grow) = untracked_heights
                .map(|(top_height, bottom_height)| {
                    (px_to_grow(top_height), px_to_grow(bottom_height))
                })
                .unwrap_or((1.0, 1.0));
            stack_container
                .child(visible_bounds_probe())
                .child(
                    with_split_sizing(
                        untracked_section,
                        untracked_top_height,
                        untracked_grow,
                        section_min_h,
                    )
                    .debug_selector(|| "status_untracked_wrapper".to_string()),
                )
                .child(build_status_resize_handle(
                    "status_resize_untracked_unstaged",
                    StatusSectionResizeHandle::UntrackedAndUnstaged,
                ))
                .child(
                    with_split_sizing(
                        split_unstaged_section,
                        split_unstaged_height,
                        split_unstaged_grow,
                        section_min_h,
                    )
                    .debug_selector(|| "status_split_unstaged_wrapper".to_string()),
                )
        } else {
            unstaged_section
        };
        let (change_tracking_grow, staged_grow) = change_tracking_heights
            .map(|(top_height, bottom_height)| (px_to_grow(top_height), px_to_grow(bottom_height)))
            .unwrap_or((1.0, 1.0));
        let change_tracking_section = with_split_sizing(
            change_tracking_section,
            change_tracking_heights.map(|(top_height, _)| top_height),
            change_tracking_grow,
            min_change_tracking_stack_height(split_change_tracking, resize_handle_h),
        );
        let staged_section = with_split_sizing(
            staged_section,
            change_tracking_heights.map(|(_, bottom_height)| bottom_height),
            staged_grow,
            section_min_h,
        );
        let change_tracking_section =
            change_tracking_section.debug_selector(|| "status_change_tracking_wrapper".to_string());
        let staged_section = staged_section.debug_selector(|| "status_staged_wrapper".to_string());
        let status_sections_bounds_for_prepaint =
            std::rc::Rc::clone(&self.status_sections_bounds_ref);
        let status_sections_container = div()
            .relative()
            .w_full()
            .min_w_full()
            .max_w_full()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .on_children_prepainted(move |children_bounds, window, _app| {
                let next_bounds = children_bounds.first().copied();
                let mut measured = status_sections_bounds_for_prepaint.borrow_mut();
                if *measured != next_bounds {
                    *measured = next_bounds;
                    window.refresh();
                }
            });
        let status_sections = status_sections_container
            .child(visible_bounds_probe())
            .flex()
            .flex_col()
            .child(change_tracking_section)
            .child(build_status_resize_handle(
                "status_resize_change_tracking_staged",
                StatusSectionResizeHandle::ChangeTrackingAndStaged,
            ))
            .child(staged_section);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .h_full()
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.finish_status_section_resize(cx);
                }),
            )
            .child(if repo_id.is_some() {
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(status_sections)
                    .child(div().px_2().py_2().child(self.commit_box(cx)))
                    .into_any_element()
            } else {
                components::empty_state(theme, "Changes", "No repository selected.")
                    .into_any_element()
            })
            .into_any_element()
    }
}
