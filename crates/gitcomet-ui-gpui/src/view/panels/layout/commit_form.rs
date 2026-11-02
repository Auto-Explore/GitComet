//! The commit message form: submit gating, submission, and the commit box.

use super::*;
use crate::view::pending_commit::PendingCommit;
use gitcomet_core::services::GitHooks;

pub(super) fn merge_active(repo: Option<&RepoState>) -> bool {
    repo.is_some_and(|r| matches!(&r.merge_commit_message, Loadable::Ready(Some(_))))
}

pub(super) fn commit_allowed(is_merge_active: bool, staged_count: usize) -> bool {
    staged_count > 0 || is_merge_active
}

/// The commit options a submit carries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CommitOptions {
    pub(super) amend: bool,
    pub(super) push_after: bool,
    pub(super) skip_hooks: bool,
}

impl CommitOptions {
    pub(super) fn msg(self, repo_id: RepoId, message: String) -> Msg {
        let hooks = GitHooks::from_skip(self.skip_hooks);
        if self.amend {
            Msg::CommitAmend {
                repo_id,
                message,
                push_after_commit: self.push_after,
                hooks,
            }
        } else {
            Msg::Commit {
                repo_id,
                message,
                push_after_commit: self.push_after,
                hooks,
            }
        }
    }

    /// The commit button's label and tooltip. Skipped hooks show on the button
    /// itself, since the choice outlasts the commit it was made for.
    pub(super) fn button_text(self) -> (&'static str, &'static str) {
        match (self.amend, self.push_after, self.skip_hooks) {
            (false, false, false) => ("Commit", "Commit staged changes"),
            (false, true, false) => ("Commit changes and Push", "Commit staged changes and push"),
            (true, false, false) => ("Amend Previous Commit", "Amend the previous commit"),
            (true, true, false) => (
                "Amend and Push Safely",
                "Amend the previous commit; published amends require explicit force push with lease",
            ),
            (false, false, true) => (
                "Commit without Hooks",
                "Commit staged changes without running the pre-commit and commit-msg hooks",
            ),
            (false, true, true) => (
                "Commit without Hooks and Push",
                "Commit staged changes and push without running the pre-commit, commit-msg and pre-push hooks",
            ),
            (true, false, true) => (
                "Amend without Hooks",
                "Amend the previous commit without running the pre-commit and commit-msg hooks",
            ),
            (true, true, true) => (
                "Amend without Hooks and Push Safely",
                "Amend the previous commit without running the pre-commit, commit-msg and pre-push hooks; published amends require explicit force push with lease",
            ),
        }
    }
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

    pub(super) fn can_submit_commit(&self, message: &str) -> bool {
        let Some(repo) = self.active_repo() else {
            return false;
        };
        if repo.is_bare()
            || self.pending_commits.contains_key(&repo.id)
            || repo.commit_in_flight > 0
        {
            return false;
        }
        if message.trim().is_empty() {
            return false;
        }
        if self.commit_amend_enabled {
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
        let options = self.commit_options();
        if !self.can_submit_commit(&message) {
            return false;
        }

        self.pending_commits.insert(
            repo_id,
            PendingCommit::new(
                &self.state,
                repo_id,
                options.amend,
                message.trim().to_string().into(),
            ),
        );
        self.store
            .dispatch(options.msg(repo_id, message.trim().to_string()));
        cx.notify();
        true
    }

    pub(super) fn commit_options(&self) -> CommitOptions {
        CommitOptions {
            amend: self.commit_amend_enabled,
            push_after: self.commit_push_after_enabled,
            skip_hooks: self.commit_skip_hooks_enabled,
        }
    }

    pub(in crate::view) fn handle_commit_submit_shortcut(
        &mut self,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.handle_commit_reword_submit_shortcut(window, cx) {
            return true;
        }
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
        let can_submit_commit = self.can_submit_commit(&commit_message_text);
        let repo_key = self.active_repo_id().map(|id| id.0).unwrap_or(0);
        let icon_color = theme.colors.accent.foreground;
        let icon = |path: &'static str| svg_icon(path, icon_color, scaled_px(14.0));
        let spinner = |id: (&'static str, u64)| {
            components::ButtonSlot::spinner(id, icon_color, scaled_px(14.0))
        };
        let (commit_label, commit_tooltip) = self.commit_options().button_text();
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
                spinner(("commit_spinner", repo_key))
            } else {
                icon("icons/check.svg")
                    .debug_selector(|| "commit_button_icon".to_string())
                    .into()
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
