use super::*;
use gpui::Div;

const STATUS_SECTION_MIN_HEIGHT_PX: f32 = 80.0;

fn merge_active(repo: Option<&RepoState>) -> bool {
    repo.is_some_and(|r| matches!(&r.merge_commit_message, Loadable::Ready(Some(_))))
}

fn commit_allowed(is_merge_active: bool, staged_count: usize) -> bool {
    staged_count > 0 || is_merge_active
}

fn commit_details_selectable_row(
    theme: AppTheme,
    key: &'static str,
    input: Entity<components::TextInput>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(key),
        )
        .child(
            div()
                .w_full()
                .min_w(px(0.0))
                .text_sm()
                .font_family(crate::view::UI_MONOSPACE_FONT_FAMILY)
                .child(input),
        )
}

fn min_change_tracking_stack_height(split_change_tracking: bool, handle_h: Pixels) -> Pixels {
    let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
    if split_change_tracking {
        section_min_h * 2.0 + handle_h
    } else {
        section_min_h
    }
}

fn clamp_vertical_split_height(
    requested_top: Pixels,
    total_height: Pixels,
    min_top: Pixels,
    min_bottom: Pixels,
) -> Pixels {
    if total_height <= px(0.0) {
        return px(0.0);
    }

    let min_total = min_top + min_bottom;
    if total_height <= min_total {
        return (total_height - min_bottom).max(px(0.0));
    }

    requested_top.max(min_top).min(total_height - min_bottom)
}

fn resolved_vertical_split_height(
    requested_top: Option<Pixels>,
    total_height: Pixels,
    min_top: Pixels,
    min_bottom: Pixels,
) -> Pixels {
    if total_height <= px(0.0) {
        return px(0.0);
    }

    let default_top = (total_height * 0.5)
        .max(min_top)
        .min((total_height - min_bottom).max(px(0.0)));
    clamp_vertical_split_height(
        requested_top.unwrap_or(default_top),
        total_height,
        min_top,
        min_bottom,
    )
}

fn visible_bounds_probe() -> Div {
    // Use a fill probe to capture the clipped viewport bounds for a container.
    // Unioning child bounds can stay larger than the visible area after window resizes.
    div().absolute().top_0().left_0().size_full()
}

impl DetailsPaneView {
    fn measured_status_sections_total_height(&self, resize_handle_h: Pixels) -> Option<Pixels> {
        self.current_status_sections_bounds()
            .map(|bounds| (bounds.size.height - resize_handle_h).max(px(0.0)))
    }

    fn resolved_measured_change_tracking_section_height(
        &self,
        resize_handle_h: Pixels,
    ) -> Option<Pixels> {
        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let min_height = min_change_tracking_stack_height(
            self.change_tracking_view == ChangeTrackingView::SplitUntracked,
            resize_handle_h,
        );

        self.measured_status_sections_total_height(resize_handle_h)
            .map(|total_height| {
                resolved_vertical_split_height(
                    self.change_tracking_height,
                    total_height,
                    min_height,
                    section_min_h,
                )
            })
    }

    fn resolved_measured_change_tracking_stack_total_height(
        &self,
        resize_handle_h: Pixels,
    ) -> Option<Pixels> {
        self.resolved_measured_change_tracking_section_height(resize_handle_h)
            .map(|section_height| (section_height - resize_handle_h).max(px(0.0)))
            .or_else(|| {
                self.current_change_tracking_stack_bounds()
                    .map(|bounds| (bounds.size.height - resize_handle_h).max(px(0.0)))
            })
    }

    pub(in super::super) fn sanitized_restored_change_tracking_height(
        view: ChangeTrackingView,
        height: Option<u32>,
    ) -> Option<Pixels> {
        let min_height = min_change_tracking_stack_height(
            view == ChangeTrackingView::SplitUntracked,
            px(PANE_RESIZE_HANDLE_PX),
        );
        height.map(|value| px(value as f32).max(min_height))
    }

    pub(in super::super) fn sanitized_restored_untracked_height(
        height: Option<u32>,
    ) -> Option<Pixels> {
        height.map(|value| px(value as f32).max(px(STATUS_SECTION_MIN_HEIGHT_PX)))
    }

    fn status_resize_total_height(
        &self,
        handle: StatusSectionResizeHandle,
        resize_handle_h: Pixels,
    ) -> Option<Pixels> {
        match handle {
            StatusSectionResizeHandle::ChangeTrackingAndStaged => {
                self.measured_status_sections_total_height(resize_handle_h)
            }
            StatusSectionResizeHandle::UntrackedAndUnstaged => {
                self.resolved_measured_change_tracking_stack_total_height(resize_handle_h)
            }
        }
    }

    fn start_status_section_resize(
        &mut self,
        handle: StatusSectionResizeHandle,
        start_y: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let resize_handle_h = px(PANE_RESIZE_HANDLE_PX);
        let total_height = self.status_resize_total_height(handle, resize_handle_h);
        let start_height = match handle {
            StatusSectionResizeHandle::ChangeTrackingAndStaged => total_height
                .map(|total_height| {
                    resolved_vertical_split_height(
                        self.change_tracking_height,
                        total_height,
                        min_change_tracking_stack_height(
                            self.change_tracking_view == ChangeTrackingView::SplitUntracked,
                            resize_handle_h,
                        ),
                        section_min_h,
                    )
                })
                .or(self.change_tracking_height)
                .unwrap_or(section_min_h),
            StatusSectionResizeHandle::UntrackedAndUnstaged => total_height
                .map(|total_height| {
                    resolved_vertical_split_height(
                        self.untracked_height,
                        total_height,
                        section_min_h,
                        section_min_h,
                    )
                })
                .or(self.untracked_height)
                .unwrap_or(section_min_h),
        };

        self.status_section_resize = Some(StatusSectionResizeState {
            handle,
            start_y,
            start_height,
        });
        cx.notify();
    }

    pub(in super::super) fn update_status_section_resize(
        &mut self,
        current_y: Pixels,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(state) = self.status_section_resize else {
            return false;
        };

        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let resize_handle_h = px(PANE_RESIZE_HANDLE_PX);
        let total_height = self.status_resize_total_height(state.handle, resize_handle_h);

        let delta_y = current_y - state.start_y;
        let mut changed = false;
        match state.handle {
            StatusSectionResizeHandle::ChangeTrackingAndStaged => {
                let min_top = min_change_tracking_stack_height(
                    self.change_tracking_view == ChangeTrackingView::SplitUntracked,
                    resize_handle_h,
                );
                let next_height = if let Some(total_height) = total_height {
                    clamp_vertical_split_height(
                        state.start_height + delta_y,
                        total_height,
                        min_top,
                        section_min_h,
                    )
                } else {
                    (state.start_height + delta_y).max(min_top)
                };
                if self.change_tracking_height != Some(next_height) {
                    self.change_tracking_height = Some(next_height);
                    changed = true;
                }
            }
            StatusSectionResizeHandle::UntrackedAndUnstaged => {
                let next_height = if let Some(total_height) = total_height {
                    clamp_vertical_split_height(
                        state.start_height + delta_y,
                        total_height,
                        section_min_h,
                        section_min_h,
                    )
                } else {
                    (state.start_height + delta_y).max(section_min_h)
                };
                if self.untracked_height != Some(next_height) {
                    self.untracked_height = Some(next_height);
                    changed = true;
                }
            }
        }

        if changed {
            cx.notify();
        }
        changed
    }

    pub(in super::super) fn finish_status_section_resize(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.status_section_resize.take().is_some() {
            let pane = cx.entity();
            self.schedule_ui_settings_persist(cx);
            cx.notify();
            cx.defer(move |cx| {
                pane.update(cx, |_this, cx| {
                    cx.notify();
                });
            });
            true
        } else {
            false
        }
    }

    fn can_submit_commit(repo: Option<&RepoState>, message: &str) -> bool {
        let Some(repo) = repo else {
            return false;
        };
        if repo.commit_in_flight > 0 {
            return false;
        }
        let staged_count = match &repo.status {
            Loadable::Ready(status) => status.staged.len(),
            _ => 0,
        };
        let is_merge_active = merge_active(Some(repo));
        commit_allowed(is_merge_active, staged_count) && !message.trim().is_empty()
    }

    fn sync_commit_details_input_value(
        input: &Entity<components::TextInput>,
        value: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        if input.read(cx).text() != value {
            input.update(cx, |input, cx| {
                input.set_text(value.to_string(), cx);
            });
        }
    }

    pub(in super::super) fn commit_details_view(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let active_repo_id = self.active_repo_id();
        let selected_id = self
            .active_repo()
            .and_then(|repo| repo.history_state.selected_commit.clone());

        if let (Some(repo_id), Some(selected_id)) = (active_repo_id, selected_id) {
            let show_delayed_loading = self.commit_details_delay.as_ref().is_some_and(|s| {
                s.repo_id == repo_id && s.commit_id == selected_id && s.show_loading
            });

            let header_title: SharedString = "Commit details".into();

            let header = div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(components::CONTROL_HEIGHT_MD_PX))
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
                    components::Button::new("commit_details_close", "")
                        .start_slot(svg_icon(
                            "icons/generic_close.svg",
                            theme.colors.text_muted,
                            px(12.0),
                        ))
                        .style(components::ButtonStyle::Transparent)
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

            let body: AnyElement = match self.active_repo().map(|r| &r.history_state.commit_details)
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
                                .h_full()
                                .min_h(px(0.0))
                                .track_scroll(self.commit_files_scroll.clone());
                                let files_scrollbar_gutter = components::Scrollbar::visible_gutter(
                                    self.commit_files_scroll.clone(),
                                    components::ScrollbarAxis::Vertical,
                                );

                                div()
                                    .id(("commit_details_files_container", repo_id.0))
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .h_full()
                                    .min_h(px(0.0))
                                    .w_full()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .w_full()
                                            .flex_1()
                                            .h_full()
                                            .min_h(px(0.0))
                                            .pr(files_scrollbar_gutter)
                                            .child(list),
                                    )
                                    .child(
                                        components::Scrollbar::new(
                                            ("commit_details_files_scrollbar", repo_id.0),
                                            self.commit_files_scroll.clone(),
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
                            Self::sync_commit_details_input_value(
                                &self.commit_details_sha_input,
                                details.id.as_ref(),
                                cx,
                            );
                            Self::sync_commit_details_input_value(
                                &self.commit_details_date_input,
                                details.committed_at.as_str(),
                                cx,
                            );
                            Self::sync_commit_details_input_value(
                                &self.commit_details_parent_input,
                                parent.as_str(),
                                cx,
                            );

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
                                        .pr(components::Scrollbar::visible_gutter(
                                            self.commit_scroll.clone(),
                                            components::ScrollbarAxis::Vertical,
                                        ))
                                        .overflow_y_scroll()
                                        .track_scroll(&self.commit_scroll)
                                        .child(self.commit_details_message_input.clone()),
                                )
                                .child(
                                    components::Scrollbar::new(
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
                                        .child(commit_details_selectable_row(
                                            theme,
                                            "Commit SHA",
                                            self.commit_details_sha_input.clone(),
                                        ))
                                        .child(commit_details_selectable_row(
                                            theme,
                                            "Commit date",
                                            self.commit_details_date_input.clone(),
                                        ))
                                        .child(commit_details_selectable_row(
                                            theme,
                                            "Parent commit SHA",
                                            self.commit_details_parent_input.clone(),
                                        )),
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
                            .h_full()
                            .min_h(px(0.0))
                            .track_scroll(self.commit_files_scroll.clone());
                            let files_scrollbar_gutter = components::Scrollbar::visible_gutter(
                                self.commit_files_scroll.clone(),
                                components::ScrollbarAxis::Vertical,
                            );

                            div()
                                .id(("commit_details_files_container", repo_id.0))
                                .relative()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .h_full()
                                .min_h(px(0.0))
                                .w_full()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .w_full()
                                        .flex_1()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .pr(files_scrollbar_gutter)
                                        .child(list),
                                )
                                .child(
                                    components::Scrollbar::new(
                                        ("commit_details_files_scrollbar", repo_id.0),
                                        self.commit_files_scroll.clone(),
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
                        Self::sync_commit_details_input_value(
                            &self.commit_details_sha_input,
                            details.id.as_ref(),
                            cx,
                        );
                        Self::sync_commit_details_input_value(
                            &self.commit_details_date_input,
                            details.committed_at.as_str(),
                            cx,
                        );
                        Self::sync_commit_details_input_value(
                            &self.commit_details_parent_input,
                            parent.as_str(),
                            cx,
                        );

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
                                    .pr(components::Scrollbar::visible_gutter(
                                        self.commit_scroll.clone(),
                                        components::ScrollbarAxis::Vertical,
                                    ))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.commit_scroll)
                                    .child(self.commit_details_message_input.clone()),
                            )
                            .child(
                                components::Scrollbar::new(
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
                                    .child(commit_details_selectable_row(
                                        theme,
                                        "Commit SHA",
                                        self.commit_details_sha_input.clone(),
                                    ))
                                    .child(commit_details_selectable_row(
                                        theme,
                                        "Commit date",
                                        self.commit_details_date_input.clone(),
                                    ))
                                    .child(commit_details_selectable_row(
                                        theme,
                                        "Parent commit SHA",
                                        self.commit_details_parent_input.clone(),
                                    )),
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

        let local_actions_in_flight = self
            .active_repo()
            .map(|r| r.local_actions_in_flight > 0)
            .unwrap_or(false);
        let (staged_count, unstaged_count) = self
            .active_repo()
            .and_then(|r| match &r.status {
                Loadable::Ready(s) => Some((s.staged.len(), s.unstaged.len())),
                _ => None,
            })
            .unwrap_or((0, 0));
        let (untracked_count, split_unstaged_count, untracked_paths, split_unstaged_paths) = self
            .active_repo()
            .and_then(|r| match &r.status {
                Loadable::Ready(s) => {
                    let mut untracked = Vec::new();
                    let mut tracked = Vec::new();
                    for entry in &s.unstaged {
                        if entry.kind == FileStatusKind::Untracked {
                            untracked.push(entry.path.clone());
                        } else {
                            tracked.push(entry.path.clone());
                        }
                    }
                    Some((untracked.len(), tracked.len(), untracked, tracked))
                }
                _ => None,
            })
            .unwrap_or_else(|| (0, 0, Vec::new(), Vec::new()));

        let repo_id = self.active_repo_id();
        let selected_combined_unstaged = repo_id
            .and_then(|rid| {
                self.status_multi_selection
                    .get(&rid)
                    .map(|s| s.selected_count_for_area(DiffArea::Unstaged))
            })
            .unwrap_or(0);
        let selected_untracked = repo_id
            .and_then(|rid| {
                self.status_multi_selection
                    .get(&rid)
                    .map(|s| s.untracked.len())
            })
            .unwrap_or(0);
        let selected_split_unstaged = repo_id
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
        let split_change_tracking = self.change_tracking_view == ChangeTrackingView::SplitUntracked;
        let icon_muted = with_alpha(theme.colors.accent, if theme.is_dark { 0.72 } else { 0.82 });

        let stage_all = components::Button::new("stage_all", "Stage all changes")
            .style(components::ButtonStyle::Subtle)
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

        let stage_selected = components::Button::new(
            "stage_selected",
            format!("Stage ({selected_combined_unstaged})"),
        )
        .style(components::ButtonStyle::Outlined)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let paths = this
                .status_multi_selection
                .remove(&repo_id)
                .map(|s| s.take_selected_paths_for_area(DiffArea::Unstaged))
                .unwrap_or_default();
            if paths.is_empty() {
                return;
            }
            this.store.dispatch(Msg::ClearDiffSelection { repo_id });
            this.store.dispatch(Msg::StagePaths { repo_id, paths });
            cx.notify();
        });

        let discard_selected = components::Button::new(
            "discard_selected",
            format!("Discard ({selected_combined_unstaged})"),
        )
        .style(components::ButtonStyle::Outlined)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, e, window, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let count = this
                .status_multi_selection
                .get(&repo_id)
                .map(|s| s.selected_count_for_area(DiffArea::Unstaged))
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

        let untracked_paths_for_stage_all = untracked_paths.clone();
        let stage_all_untracked = components::Button::new("stage_all_untracked", "Stage all")
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
                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                this.store.dispatch(Msg::StagePaths {
                    repo_id,
                    paths: untracked_paths_for_stage_all.clone(),
                });
                cx.notify();
            });

        let stage_selected_untracked = components::Button::new(
            "stage_selected_untracked",
            format!("Stage ({selected_untracked})"),
        )
        .style(components::ButtonStyle::Outlined)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, _e, _w, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let paths = this
                .status_multi_selection
                .remove(&repo_id)
                .map(|s| s.untracked)
                .unwrap_or_default();
            if paths.is_empty() {
                return;
            }
            this.store.dispatch(Msg::ClearDiffSelection { repo_id });
            this.store.dispatch(Msg::StagePaths { repo_id, paths });
            cx.notify();
        });

        let discard_selected_untracked = components::Button::new(
            "discard_selected_untracked",
            format!("Discard ({selected_untracked})"),
        )
        .style(components::ButtonStyle::Outlined)
        .disabled(local_actions_in_flight)
        .on_click(theme, cx, |this, e, window, cx| {
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let count = this
                .status_multi_selection
                .get(&repo_id)
                .map(|s| s.untracked.len())
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

        let split_unstaged_paths_for_stage_all = split_unstaged_paths.clone();
        let stage_all_split_unstaged =
            components::Button::new("stage_all_split_unstaged", "Stage all")
                .style(components::ButtonStyle::Subtle)
                .disabled(local_actions_in_flight || split_unstaged_paths_for_stage_all.is_empty())
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    let Some(repo_id) = this.active_repo_id() else {
                        return;
                    };
                    if split_unstaged_paths_for_stage_all.is_empty() {
                        return;
                    }
                    this.status_multi_selection.remove(&repo_id);
                    this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                    this.store.dispatch(Msg::StagePaths {
                        repo_id,
                        paths: split_unstaged_paths_for_stage_all.clone(),
                    });
                    cx.notify();
                });

        let stage_selected_split_unstaged = components::Button::new(
            "stage_selected_split_unstaged",
            format!("Stage ({selected_split_unstaged})"),
        )
        .style(components::ButtonStyle::Outlined)
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

        let discard_selected_split_unstaged = components::Button::new(
            "discard_selected_split_unstaged",
            format!("Discard ({selected_split_unstaged})"),
        )
        .style(components::ButtonStyle::Outlined)
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

        let unstage_all = components::Button::new("unstage_all", "Unstage all changes")
            .style(components::ButtonStyle::Subtle)
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
            components::Button::new("unstage_selected", format!("Unstage ({selected_staged})"))
                .style(components::ButtonStyle::Outlined)
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
                .h(px(components::CONTROL_HEIGHT_MD_PX))
                .px_2()
                .bg(theme.colors.surface_bg_elevated)
                .border_b_1()
                .border_color(theme.colors.border)
                .child(title)
                .when(show_action, |d| d.child(action))
                .into_any_element()
        };

        let normal_header_title = |label: &'static str| {
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(label)
                .into_any_element()
        };

        let section_min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let resize_handle_h = px(PANE_RESIZE_HANDLE_PX);

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
            if selected_combined_unstaged > 0 {
                actions = actions.child(stage_selected).child(discard_selected);
            }
            actions.child(stage_all).into_any_element()
        };

        let untracked_actions = {
            let mut actions = div().flex().items_center().gap_2();
            if local_actions_in_flight {
                actions = actions.child(
                    spinner(
                        ("untracked_actions_spinner", repo_key),
                        with_alpha(theme.colors.accent, if theme.is_dark { 0.72 } else { 0.82 }),
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
            if local_actions_in_flight {
                actions = actions.child(
                    spinner(
                        ("split_unstaged_actions_spinner", repo_key),
                        with_alpha(theme.colors.accent, if theme.is_dark { 0.72 } else { 0.82 }),
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
            components::empty_state(theme, "Unstaged", "Clean.").into_any_element()
        } else {
            self.status_list(cx, StatusSection::CombinedUnstaged, unstaged_count)
        };

        let untracked_body = if untracked_count == 0 {
            components::empty_state(theme, "Untracked", "No untracked files.").into_any_element()
        } else {
            self.status_list(cx, StatusSection::Untracked, untracked_count)
        };

        let split_unstaged_body = if split_unstaged_count == 0 {
            components::empty_state(theme, "Unstaged", "Clean.").into_any_element()
        } else {
            self.status_list(cx, StatusSection::Unstaged, split_unstaged_count)
        };

        let staged_list = if staged_count == 0 {
            components::empty_state(theme, "Staged", "No staged changes.").into_any_element()
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
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_1()
                    .h(px(18.0))
                    .rounded(px(theme.radii.row))
                    .when(change_tracking_active, |d| d.bg(theme.colors.active))
                    .hover(move |s| {
                        if change_tracking_active {
                            s.bg(theme.colors.active)
                        } else {
                            s.bg(with_alpha(theme.colors.hover, 0.55))
                        }
                    })
                    .active(move |s| s.bg(theme.colors.active))
                    .cursor(CursorStyle::PointingHand)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(label),
                    )
                    .child(svg_icon("icons/chevron_down.svg", icon_muted, px(12.0)))
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        this.activate_context_menu_invoker(change_tracking_invoker.clone(), cx);
                        this.open_popover_at(
                            PopoverKind::ChangeTrackingSettings,
                            e.position(),
                            window,
                            cx,
                        );
                        cx.notify();
                    }))
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

        let build_status_resize_handle = |id: &'static str, handle: StatusSectionResizeHandle| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .w_full()
                .h(resize_handle_h)
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .cursor(CursorStyle::ResizeUpDown)
                .hover(move |s| s.bg(with_alpha(theme.colors.hover, 0.65)))
                .active(move |s| s.bg(theme.colors.active))
                .child(div().h(px(1.0)).w_full().bg(theme.colors.border))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
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
        let unstaged_section = div()
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

        let untracked_section = div()
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

        let split_unstaged_section = div()
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

        let staged_section = div()
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
                    .child(
                        div()
                            .border_t_1()
                            .border_color(theme.colors.border)
                            .bg(theme.colors.surface_bg)
                            .px_2()
                            .py_2()
                            .child(self.commit_box(cx)),
                    )
                    .into_any_element()
            } else {
                components::empty_state(theme, "Changes", "No repository selected.")
                    .into_any_element()
            })
            .into_any_element()
    }

    pub(in super::super) fn status_list(
        &mut self,
        cx: &mut gpui::Context<Self>,
        section: StatusSection,
        count: usize,
    ) -> AnyElement {
        let theme = self.theme;
        if count == 0 {
            return components::empty_state(theme, "Status", "Clean.").into_any_element();
        }
        match section {
            StatusSection::CombinedUnstaged => {
                let list =
                    uniform_list("unstaged", count, cx.processor(Self::render_unstaged_rows))
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(self.unstaged_scroll.clone());
                let list = div()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .pr(components::Scrollbar::visible_gutter(
                        self.unstaged_scroll.clone(),
                        components::ScrollbarAxis::Vertical,
                    ))
                    .child(list);
                div()
                    .id("unstaged_scroll_container")
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(list)
                    .child(
                        components::Scrollbar::new(
                            "unstaged_scrollbar",
                            self.unstaged_scroll.clone(),
                        )
                        .render(theme),
                    )
                    .into_any_element()
            }
            StatusSection::Untracked => {
                let list = uniform_list(
                    "untracked",
                    count,
                    cx.processor(Self::render_untracked_rows),
                )
                .h_full()
                .min_h(px(0.0))
                .track_scroll(self.untracked_scroll.clone());
                let list = div()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .pr(components::Scrollbar::visible_gutter(
                        self.untracked_scroll.clone(),
                        components::ScrollbarAxis::Vertical,
                    ))
                    .child(list);
                div()
                    .id("untracked_scroll_container")
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(list)
                    .child(
                        components::Scrollbar::new(
                            "untracked_scrollbar",
                            self.untracked_scroll.clone(),
                        )
                        .render(theme),
                    )
                    .into_any_element()
            }
            StatusSection::Unstaged => {
                let list = uniform_list(
                    "split_unstaged",
                    count,
                    cx.processor(Self::render_split_unstaged_rows),
                )
                .h_full()
                .min_h(px(0.0))
                .track_scroll(self.unstaged_scroll.clone());
                let list = div()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .pr(components::Scrollbar::visible_gutter(
                        self.unstaged_scroll.clone(),
                        components::ScrollbarAxis::Vertical,
                    ))
                    .child(list);
                div()
                    .id("split_unstaged_scroll_container")
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(list)
                    .child(
                        components::Scrollbar::new(
                            "split_unstaged_scrollbar",
                            self.unstaged_scroll.clone(),
                        )
                        .render(theme),
                    )
                    .into_any_element()
            }
            StatusSection::Staged => {
                let list = uniform_list("staged", count, cx.processor(Self::render_staged_rows))
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(self.staged_scroll.clone());
                let list = div()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .pr(components::Scrollbar::visible_gutter(
                        self.staged_scroll.clone(),
                        components::ScrollbarAxis::Vertical,
                    ))
                    .child(list);
                div()
                    .id("staged_scroll_container")
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(list)
                    .child(
                        components::Scrollbar::new("staged_scrollbar", self.staged_scroll.clone())
                            .render(theme),
                    )
                    .into_any_element()
            }
        }
    }

    pub(in super::super) fn commit_box(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let commit_in_flight = self
            .active_repo()
            .is_some_and(|repo| repo.commit_in_flight > 0);
        let commit_message_text = self.commit_message_input.read(cx).text().to_string();
        let can_submit_commit = Self::can_submit_commit(self.active_repo(), &commit_message_text);
        let repo_key = self.active_repo_id().map(|id| id.0).unwrap_or(0);
        let icon_color = theme.colors.accent;
        let icon = |path: &'static str| svg_icon(path, icon_color, px(14.0));
        let spinner = |id: (&'static str, u64)| svg_spinner(id, icon_color, px(14.0));
        let commit_message = div()
            .id(("commit_message_container", repo_key))
            .relative()
            .w_full()
            .min_w(px(0.0))
            .child(
                div()
                    .id(("commit_message_scroll_surface", repo_key))
                    .relative()
                    .w_full()
                    .min_w(px(0.0))
                    .max_h(px(COMMIT_MESSAGE_INPUT_MAX_HEIGHT_PX))
                    .pr(components::Scrollbar::visible_gutter(
                        self.commit_message_scroll.clone(),
                        components::ScrollbarAxis::Vertical,
                    ))
                    .overflow_y_scroll()
                    .track_scroll(&self.commit_message_scroll)
                    .child(self.commit_message_input.clone()),
            )
            .child(
                components::Scrollbar::new(
                    ("commit_message_scrollbar", repo_key),
                    self.commit_message_scroll.clone(),
                )
                .render(theme),
            );
        div().flex().flex_col().gap_2().child(commit_message).child(
            div().flex().items_center().justify_end().child(
                div().flex().items_center().gap_2().child(
                    components::Button::new("commit", "Commit")
                        .start_slot(if commit_in_flight {
                            spinner(("commit_spinner", repo_key)).into_any_element()
                        } else {
                            icon("icons/check.svg").into_any_element()
                        })
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_submit_commit)
                        .render(theme)
                        .debug_selector(|| "commit_button".to_string())
                        .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
                            let Some(repo_id) = this.active_repo_id() else {
                                return;
                            };
                            let message = this
                                .commit_message_input
                                .read_with(cx, |i, _| i.text().to_string());
                            if !Self::can_submit_commit(this.active_repo(), &message) {
                                return;
                            }
                            let message = message.trim().to_string();
                            this.store.dispatch(Msg::Commit { repo_id, message });
                            this.commit_message_programmatic_change = true;
                            this.commit_message_input
                                .update(cx, |i, cx| i.set_text(String::new(), cx));
                            this.commit_message_scroll
                                .set_offset(point(px(0.0), px(0.0)));
                            cx.notify();
                        }))
                        .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                            let text: SharedString = "Commit staged changes".into();
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
                ),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_state::model::{Loadable, RepoId, RepoState};
    use std::path::PathBuf;

    fn test_repo() -> RepoState {
        RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        )
    }

    #[test]
    fn commit_allowed_when_staged_changes_exist() {
        assert!(commit_allowed(false, 1));
    }

    #[test]
    fn commit_allowed_when_merge_is_active_without_staged_changes() {
        let mut repo = test_repo();
        repo.merge_commit_message = Loadable::Ready(Some("Merge branch 'feature'".to_string()));
        assert!(commit_allowed(merge_active(Some(&repo)), 0));
    }

    #[test]
    fn commit_not_allowed_without_staged_changes_or_merge() {
        assert!(!commit_allowed(false, 0));
    }

    #[test]
    fn split_height_clamps_to_minimum_section_heights() {
        let min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let total_h = px(400.0);

        let top_clamped = clamp_vertical_split_height(px(-300.0), total_h, min_h, min_h);
        let bottom_clamped = clamp_vertical_split_height(px(900.0), total_h, min_h, min_h);

        assert_eq!(top_clamped, min_h);
        assert_eq!(bottom_clamped, total_h - min_h);
    }

    #[test]
    fn resolved_split_height_defaults_to_half_when_unset() {
        let min_h = px(STATUS_SECTION_MIN_HEIGHT_PX);
        let total_h = px(400.0);

        assert_eq!(
            resolved_vertical_split_height(None, total_h, min_h, min_h),
            px(200.0)
        );
    }

    #[test]
    fn split_change_tracking_min_height_includes_inner_handle() {
        assert_eq!(
            min_change_tracking_stack_height(false, px(PANE_RESIZE_HANDLE_PX)),
            px(STATUS_SECTION_MIN_HEIGHT_PX)
        );
        assert_eq!(
            min_change_tracking_stack_height(true, px(PANE_RESIZE_HANDLE_PX)),
            px((STATUS_SECTION_MIN_HEIGHT_PX * 2.0) + PANE_RESIZE_HANDLE_PX)
        );
    }

    #[test]
    fn restored_status_section_heights_clamp_to_visible_minimums() {
        assert_eq!(
            DetailsPaneView::sanitized_restored_change_tracking_height(
                ChangeTrackingView::Combined,
                Some(1),
            ),
            Some(px(STATUS_SECTION_MIN_HEIGHT_PX))
        );
        assert_eq!(
            DetailsPaneView::sanitized_restored_change_tracking_height(
                ChangeTrackingView::SplitUntracked,
                Some(1),
            ),
            Some(px(
                (STATUS_SECTION_MIN_HEIGHT_PX * 2.0) + PANE_RESIZE_HANDLE_PX
            ))
        );
        assert_eq!(
            DetailsPaneView::sanitized_restored_untracked_height(Some(1)),
            Some(px(STATUS_SECTION_MIN_HEIGHT_PX))
        );
    }
}
