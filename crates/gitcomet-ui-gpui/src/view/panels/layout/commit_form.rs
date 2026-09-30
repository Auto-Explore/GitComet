//! The commit message form: submit gating, submission, and the commit box.

use super::*;

pub(super) fn merge_active(repo: Option<&RepoState>) -> bool {
    repo.is_some_and(|r| matches!(&r.merge_commit_message, Loadable::Ready(Some(_))))
}

pub(super) fn commit_allowed(is_merge_active: bool, staged_count: usize) -> bool {
    staged_count > 0 || is_merge_active
}

impl DetailsPaneView {
    fn repo_has_head_commit(repo: &RepoState) -> bool {
        if repo.detached_head_commit.is_some() {
            return true;
        }

        match &repo.head_branch {
            Loadable::Ready(head) if head == "HEAD" => true,
            Loadable::Ready(head) => match &repo.branches {
                Loadable::Ready(branches) => branches.iter().any(|branch| branch.name == *head),
                _ => true,
            },
            _ => true,
        }
    }

    pub(super) fn can_submit_commit(repo: Option<&RepoState>, message: &str, amend: bool) -> bool {
        let Some(repo) = repo else {
            return false;
        };
        if repo.commit_in_flight > 0 {
            return false;
        }
        if message.trim().is_empty() {
            return false;
        }
        if amend {
            return !merge_active(Some(repo))
                && !matches!(repo.rebase_in_progress, Loadable::Ready(true))
                && Self::repo_has_head_commit(repo);
        }
        let staged_count = repo
            .staged_status_entries()
            .map_or(0, |entries| entries.len());
        let is_merge_active = merge_active(Some(repo));
        commit_allowed(is_merge_active, staged_count)
    }

    fn submit_commit(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };
        let message = self
            .commit_message_input
            .read_with(cx, |input, _| input.text().to_string());
        let amend = self.commit_amend_enabled;
        if !Self::can_submit_commit(self.active_repo(), &message, amend) {
            return false;
        }

        if amend {
            self.mark_pending_commit_amend(repo_id);
            self.store.dispatch(Msg::CommitAmend {
                repo_id,
                message: message.trim().to_string(),
                push_after_commit: self.commit_push_after_enabled,
            });
        } else {
            self.store.dispatch(Msg::Commit {
                repo_id,
                message: message.trim().to_string(),
                push_after_commit: self.commit_push_after_enabled,
            });
        }
        self.commit_message_programmatic_change = true;
        self.commit_message_input
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self.commit_message_scroll
            .set_offset(point(px(0.0), px(0.0)));
        cx.notify();
        true
    }

    pub(in crate::view) fn handle_commit_submit_shortcut(
        &mut self,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if !self
            .commit_message_input
            .read(cx)
            .focus_handle()
            .is_focused(window)
        {
            return false;
        }

        let _ = self.submit_commit(cx);
        true
    }

    pub(in crate::view) fn commit_box(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let commit_in_flight = self
            .active_repo()
            .is_some_and(|repo| repo.commit_in_flight > 0);
        let commit_message_text = self.commit_message_input.read(cx).text().to_string();
        let can_submit_commit = Self::can_submit_commit(
            self.active_repo(),
            &commit_message_text,
            self.commit_amend_enabled,
        );
        let repo_key = self.active_repo_id().map(|id| id.0).unwrap_or(0);
        let icon_color = theme.colors.accent.foreground;
        let icon = |path: &'static str| svg_icon(path, icon_color, scaled_px(14.0));
        let spinner = |id: (&'static str, u64)| svg_spinner(id, icon_color, scaled_px(14.0));
        let commit_label = match (self.commit_amend_enabled, self.commit_push_after_enabled) {
            (false, false) => "Commit",
            (false, true) => "Commit changes and Push",
            (true, false) => "Amend Previous Commit",
            (true, true) => "Amend and Push Safely",
        };
        let commit_tooltip = match (self.commit_amend_enabled, self.commit_push_after_enabled) {
            (false, false) => "Commit staged changes",
            (false, true) => "Commit staged changes and push",
            (true, false) => "Amend the previous commit",
            (true, true) => {
                "Amend the previous commit; published amends require explicit force push with lease"
            }
        };
        let commit_options_invoker: SharedString = "commit_options".into();
        let commit_options_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == commit_options_invoker.as_ref());
        let previous_messages_invoker: SharedString = "previous_commit_messages".into();
        let previous_messages_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == previous_messages_invoker.as_ref());
        let menu_selected_bg = components::control_open_background(theme);
        let menu_icon_color = if commit_options_active {
            theme.colors.accent.foreground
        } else {
            theme.colors.foreground.secondary
        };
        let previous_messages_icon_color = if previous_messages_active {
            theme.colors.accent.foreground
        } else {
            theme.colors.foreground.secondary
        };
        let commit_message = components::ScrollContainer::vertical(
            ("commit_message_scroll_surface", repo_key),
            ("commit_message_scrollbar", repo_key),
            self.commit_message_scroll.clone(),
            scaled_px(COMMIT_MESSAGE_INPUT_MAX_HEIGHT_PX),
        )
        .container_id(("commit_message_container", repo_key))
        .render(theme, self.commit_message_input.clone());
        let commit_main = components::Button::new("commit", commit_label)
            .start_slot(if commit_in_flight {
                spinner(("commit_spinner", repo_key)).into_any_element()
            } else {
                icon("icons/check.svg")
                    .debug_selector(|| "commit_button_icon".to_string())
                    .into_any_element()
            })
            .style(components::ButtonStyle::Subtle)
            .disabled(!can_submit_commit);
        let commit_menu = components::Button::new("commit_options", "")
            .start_slot(
                svg_icon("icons/chevron_down.svg", menu_icon_color, scaled_px(14.0))
                    .debug_selector(|| "commit_options_icon".to_string()),
            )
            .style(components::ButtonStyle::Subtle)
            .open(commit_options_active)
            .selected_bg(menu_selected_bg)
            .disabled(self.active_repo_id().is_none());
        let commit = components::SplitButton::from_buttons(
            commit_main,
            commit_menu,
            cx,
            |button, cx| {
                button
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        let _ = this.submit_commit(cx);
                    })
                    .debug_selector(|| "commit_button".to_string())
                    .gitcomet_tooltip(theme, commit_tooltip.into())
            },
            |button, cx| {
                button
                    .on_click_with_bounds(theme, cx, move |this, _e, bounds, window, cx| {
                        let Some(repo_id) = this.active_repo_id() else {
                            return;
                        };
                        this.open_popover_for_bounds(
                            (PopoverKind::CommitOptionsMenu { repo_id })
                                .invoked_by(commit_options_invoker.clone()),
                            bounds,
                            window,
                            cx,
                        );
                    })
                    .gitcomet_tooltip(theme, "Commit options".into())
            },
        )
        .style(components::SplitButtonStyle::Filled)
        .render(theme, ui_scale_percent)
        .debug_selector(|| "commit_split_button".to_string());
        let previous_messages_menu = components::Button::new("previous_commit_messages", "")
            .start_slot(
                svg_icon(
                    "icons/history.svg",
                    previous_messages_icon_color,
                    scaled_px(14.0),
                )
                .debug_selector(|| "previous_commit_messages_icon".to_string()),
            )
            .style(components::ButtonStyle::Subtle)
            .open(previous_messages_active)
            .selected_bg(menu_selected_bg)
            .disabled(self.active_repo_id().is_none())
            .on_click(theme, cx, move |this, e, window, cx| {
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };

                this.open_popover_at(
                    (PopoverKind::PreviousCommitMessagesMenu { repo_id })
                        .invoked_by(previous_messages_invoker.clone()),
                    e.position(),
                    window,
                    cx,
                );
            })
            .debug_selector(|| "previous_commit_messages_button".to_string())
            .gitcomet_tooltip(theme, "Previous commit messages".into());
        div().flex().flex_col().gap_2().child(commit_message).child(
            div().flex().items_center().justify_end().child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(previous_messages_menu)
                    .child(commit),
            ),
        )
    }
}
