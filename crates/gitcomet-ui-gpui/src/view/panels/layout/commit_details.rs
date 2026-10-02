//! The details pane body: a worktree, comparison, multi-commit or single
//! commit view when one is selected, otherwise the status view.

use super::*;

impl DetailsPaneView {
    pub(in crate::view) fn commit_details_view(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
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
            return self.single_commit_details_view(repo_id, selected_id, cx);
        }

        self.status_sections_view(cx)
    }

    /// One selected commit: its header, metadata, message, and files.
    fn single_commit_details_view(
        &mut self,
        repo_id: RepoId,
        selected_id: CommitId,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let show_delayed_loading = self
            .commit_details_delay
            .as_ref()
            .is_some_and(|s| s.repo_id == repo_id && s.commit_id == selected_id && s.show_loading);

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
                            this.store.dispatch(Msg::ClearCommitSelection {
                                request_id: None,
                                repo_id,
                            });
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
        let body: AnyElement = match active_commit_details.as_ref().map(|(details, _)| details) {
            None => components::empty_state(theme, "Commit", "No repository.").into_any_element(),
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
                let current = details.id == selected_id;
                if !current && show_delayed_loading {
                    components::empty_state(theme, "Commit", "Loading").into_any_element()
                } else {
                    let parent = details
                        .parent_ids
                        .first()
                        .map(|p: &CommitId| p.as_ref().to_string())
                        .unwrap_or_else(|| "—".to_string());
                    let find = self.commit_details_find_highlights(details);

                    if current {
                        self.sync_commit_details_message_input(
                            details.message.as_str(),
                            details.id.as_ref().len(),
                            theme,
                            repo_id,
                            &find.summary,
                            cx,
                        );
                        self.sync_commit_details_sha_menu(
                            details.id.as_ref(),
                            repo_id,
                            true,
                            find.sha,
                            theme,
                            cx,
                        );
                    } else {
                        self.sync_retained_commit_details_message_input(
                            details.message.as_str(),
                            &find.summary,
                            cx,
                        );
                        self.sync_commit_details_sha_input(
                            details.id.as_ref(),
                            find.sha,
                            theme,
                            cx,
                        );
                    }
                    Self::sync_commit_details_input_value(
                        &self.commit_details_date_input,
                        self.commit_details_date_display(details).as_str(),
                        cx,
                    );
                    self.sync_commit_details_parent_input(
                        parent.as_str(),
                        if current { repo_id } else { RepoId(0) },
                        current && parent != "—",
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
                                &find.author,
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
                                    commit_details_sha_value(
                                        theme,
                                        if current {
                                            self.commit_details_sha_link_menu
                                                .clone()
                                                .into_any_element()
                                        } else {
                                            self.commit_details_sha_input.clone().into_any_element()
                                        },
                                        details.id.as_ref(),
                                        &find,
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
                                    if current {
                                        commit_details_monospace_element(
                                            self.commit_details_parent_link_menu
                                                .clone()
                                                .into_any_element(),
                                        )
                                    } else {
                                        commit_details_monospace_value(
                                            self.commit_details_parent_input.clone(),
                                        )
                                    },
                                )),
                        )
                        .child(self.commit_files_section(repo_id, commit_details_rev, details, cx))
                        .into_any_element()
                }
            }
        };

        div()
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
            .into_any_element()
    }
}
