//! The commit message form: submit gating, submission, and the commit box.

use super::*;
use crate::view::pending_commit::PendingCommit;

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

    fn submit_commit(&mut self, push_after_commit: bool, cx: &mut gpui::Context<Self>) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };
        let message = self
            .commit_message_input
            .read_with(cx, |input, _| input.text().to_string());
        let amend = self.commit_amend_enabled;
        if !self.can_submit_commit(&message) {
            return false;
        }

        self.pending_commits.insert(
            repo_id,
            PendingCommit::new(
                &self.state,
                repo_id,
                amend,
                message.trim().to_string().into(),
            ),
        );
        if amend {
            self.store.dispatch(Msg::CommitAmend {
                repo_id,
                message: message.trim().to_string(),
                push_after_commit,
            });
        } else {
            self.store.dispatch(Msg::Commit {
                repo_id,
                message: message.trim().to_string(),
                push_after_commit,
            });
        }
        cx.notify();
        true
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

        let _ = self.submit_commit(false, cx);
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
        let amend = self.commit_amend_enabled;
        let commit_label = if amend { "Amend" } else { "Commit" };
        let push_label = if amend {
            "Amend & Push"
        } else {
            "Commit & Push"
        };
        let commit_tooltip = if amend {
            "Amend the previous commit"
        } else {
            "Commit staged changes"
        };
        let push_tooltip = if amend {
            "Amend the previous commit; published amends require explicit force push with lease"
        } else {
            "Commit staged changes and push"
        };
        let previous_messages_invoker: SharedString = "previous_commit_messages".into();
        let previous_messages_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == previous_messages_invoker.as_ref());
        let menu_selected_bg = components::control_open_background(theme);
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
        let start_slot =
            |id: &'static str, icon_selector: &'static str| -> components::ButtonSlot {
                if commit_in_flight {
                    spinner((id, repo_key))
                } else {
                    icon("icons/check.svg")
                        .debug_selector(move || icon_selector.to_string())
                        .into()
                }
            };
        let commit = components::Button::new("commit", commit_label)
            .bg(theme.colors.surface.raised)
            .start_slot(start_slot("commit_spinner", "commit_button_icon"))
            .style(components::ButtonStyle::Subtle)
            .disabled(!can_submit_commit)
            .on_click(theme, cx, |this, _e, _w, cx| {
                let _ = this.submit_commit(false, cx);
            })
            .debug_selector(|| "commit_button".to_string())
            .gitcomet_tooltip(theme, commit_tooltip.into());
        let commit_and_push = components::Button::new("commit_and_push", push_label)
            .bg(theme.colors.surface.raised)
            .start_slot(start_slot(
                "commit_push_spinner",
                "commit_and_push_button_icon",
            ))
            .style(components::ButtonStyle::Subtle)
            .disabled(!can_submit_commit)
            .on_click(theme, cx, |this, _e, _w, cx| {
                let _ = this.submit_commit(true, cx);
            })
            .debug_selector(|| "commit_and_push_button".to_string())
            .gitcomet_tooltip(theme, push_tooltip.into());
        // The border and raised fill the old split Commit button used, drawn on
        // the button itself so its hover and click area are the whole box.
        let framed = |button: gpui::Stateful<gpui::Div>| {
            button
                .flex_1()
                .min_w(px(0.0))
                .h(components::control_height(
                    crate::ui_scale::UiScale::from_percent(ui_scale_percent)
                        .with_appearance(theme.metrics),
                ))
                .justify_center()
                .rounded(px(theme.radii.control))
                .border_1()
                .border_color(crate::theme::with_alpha(
                    theme.colors.foreground.secondary,
                    if theme.is_dark { 0.34 } else { 0.26 },
                ))
        };
        let amend_toggle = {
            let box_size = scaled_px(14.0);
            let on = amend;
            div()
                .id("commit_amend_toggle")
                .debug_selector(|| "commit_amend_toggle".to_string())
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .child(
                    div()
                        .size(box_size)
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(theme.radii.control.min(4.0)))
                        .border_1()
                        .border_color(if on {
                            theme.colors.accent.foreground
                        } else {
                            theme.colors.stroke.default
                        })
                        .when(on, |d| d.bg(theme.colors.accent.foreground))
                        .when(on, |d| {
                            d.child(svg_icon(
                                "icons/check.svg",
                                theme.colors.surface.canvas,
                                scaled_px(10.0),
                            ))
                        }),
                )
                .child(
                    div()
                        .text_size(theme.ui_text(13.0))
                        .text_color(theme.colors.foreground.primary)
                        .child("Amend previous commit"),
                )
                .on_activate(
                    false,
                    components::ControlActivation::Action,
                    cx.listener(|this, e: &ClickEvent, _w, cx| {
                        if e.standard_click() {
                            this.toggle_commit_amend(cx);
                        }
                    }),
                )
        };
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
        let message_card = div()
            .flex()
            .flex_col()
            .w_full()
            .border_1()
            .border_color(theme.colors.stroke.default)
            .rounded(px(theme.radii.panel))
            .overflow_hidden()
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .border_b_1()
                    .border_color(theme.colors.stroke.subtle)
                    .text_size(theme.ui_text(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.colors.foreground.primary)
                    .child("Commit Message"),
            )
            .child(div().px_3().py_2().child(commit_message));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(message_card)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(amend_toggle)
                    .child(previous_messages_menu),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(framed(commit))
                    .child(framed(commit_and_push)),
            )
    }
}
