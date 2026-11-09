//! The commit message form: submit gating, submission, and the commit box.

use super::*;
use crate::view::pending_commit::PendingCommit;
use gitcomet_core::services::GitHooks;

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
        Msg::Commit {
            repo_id,
            request: gitcomet_state::model::CommitRequest {
                message,
                amend: self.amend,
                push_after_commit: self.push_after,
                hooks,
            },
        }
    }

    /// The commit button's label and tooltip. Skipped hooks show on the button
    /// itself, since the choice outlasts the commit it was made for.
    pub(super) fn button_text(self) -> (&'static str, &'static str) {
        match (self.amend, self.push_after, self.skip_hooks) {
            (false, false, false) => ("Commit", "Commit staged changes"),
            (false, true, false) => ("Commit & Push", "Commit staged changes and push"),
            (true, false, false) => ("Amend", "Amend the previous commit"),
            (true, true, false) => (
                "Amend & Push",
                "Amend the previous commit; published amends require explicit force push with lease",
            ),
            (false, false, true) => (
                "Commit without Hooks",
                "Commit staged changes without running the pre-commit and commit-msg hooks",
            ),
            (false, true, true) => (
                "Commit & Push without Hooks",
                "Commit staged changes and push without running the pre-commit, commit-msg and pre-push hooks",
            ),
            (true, false, true) => (
                "Amend without Hooks",
                "Amend the previous commit without running the pre-commit and commit-msg hooks",
            ),
            (true, true, true) => (
                "Amend & Push without Hooks",
                "Amend the previous commit without running the pre-commit, commit-msg and pre-push hooks; published amends require explicit force push with lease",
            ),
        }
    }
}

impl DetailsPaneView {
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
            return amend_allowed(repo);
        }
        let staged_count = repo
            .staged_status_entries()
            .map_or(0, |entries| entries.len());
        let is_merge_active = merge_in_progress(repo);
        commit_allowed(is_merge_active, staged_count)
    }

    fn submit_commit(&mut self, push_after_commit: bool, cx: &mut gpui::Context<Self>) -> bool {
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
        self.store.dispatch(
            CommitOptions {
                push_after: push_after_commit,
                ..options
            }
            .msg(repo_id, message.trim().to_string()),
        );
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
        push_after_commit: bool,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if !push_after_commit && self.handle_commit_reword_submit_shortcut(window, cx) {
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

        let _ = self.submit_commit(push_after_commit, cx);
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
        let options = self.commit_options();
        let amend = options.amend;
        let (commit_label, commit_tooltip) = CommitOptions {
            push_after: false,
            ..options
        }
        .button_text();
        let (push_label, push_tooltip) = CommitOptions {
            push_after: true,
            ..options
        }
        .button_text();
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
        let commit_message = div()
            .id(("commit_message_container", repo_key))
            .w_full()
            .min_w(px(0.0))
            .child(self.commit_message_input.clone());
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
            .start_slot(start_slot("commit_spinner", "commit_button_icon"))
            .style(components::ButtonStyle::Subtle)
            .disabled(!can_submit_commit)
            .on_click(theme, cx, |this, _e, _w, cx| {
                let _ = this.submit_commit(false, cx);
            })
            .debug_selector(|| "commit_button".to_string())
            .gitcomet_tooltip_keyed(
                theme,
                commit_tooltip.into(),
                vec![components::secondary_shortcut("Enter").into()],
            );
        let commit_and_push = components::Button::new("commit_and_push", push_label)
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
            .gitcomet_tooltip_keyed(
                theme,
                push_tooltip.into(),
                vec![components::secondary_shortcut("Shift+Enter").into()],
            );
        // The frame and raised fill the old split Commit button used.
        let framed = |frame_selector: &'static str, button: gpui::Stateful<gpui::Div>| {
            components::filled_action_frame(
                theme,
                crate::ui_scale::UiScale::from_percent(ui_scale_percent)
                    .with_appearance(theme.metrics),
            )
            .flex_1()
            .debug_selector(move || frame_selector.to_string())
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .w_full()
                    .rounded(px(theme.radii.control))
                    .bg(theme.colors.surface.raised)
                    .overflow_hidden()
                    // The whole frame is the click target, not just the label.
                    .child(button.flex_1()),
            )
        };
        let checkbox = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .child(
                    div()
                        .size(scaled_px(14.0))
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
                        .child(label),
                )
        };
        let amend_toggle = checkbox("commit_amend_toggle", "Amend previous commit", amend)
            .on_activate(
                false,
                components::ControlActivation::Action,
                cx.listener(|this, e: &ClickEvent, _w, cx| {
                    if e.standard_click() {
                        this.toggle_commit_amend(cx);
                    }
                }),
            );
        let skip_hooks_toggle =
            checkbox("commit_skip_hooks_toggle", "Skip hooks", options.skip_hooks)
                .on_activate(
                    false,
                    components::ControlActivation::Action,
                    cx.listener(|this, e: &ClickEvent, _w, cx| {
                        if e.standard_click() {
                            this.toggle_commit_skip_hooks(cx);
                        }
                    }),
                )
                .gitcomet_tooltip(
                    theme,
                    "Skip the pre-commit, commit-msg and pre-push hooks until this window closes"
                        .into(),
                );
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
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_x_3()
                            .gap_y_1()
                            .child(amend_toggle)
                            .child(skip_hooks_toggle),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(self.extension_commit_items.iter().cloned())
                            .child(previous_messages_menu),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(framed("commit_button_frame", commit))
                    .child(framed("commit_and_push_button_frame", commit_and_push)),
            )
    }
}
