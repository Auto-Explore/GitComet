//! The commit message form: submit gating, submission, and the commit box.

use super::*;
use crate::view::pending_commit::PendingCommit;

pub(super) fn merge_active(repo: Option<&RepoState>) -> bool {
    repo.is_some_and(|r| matches!(&r.merge_commit_message, Loadable::Ready(Some(_))))
}

pub(super) fn commit_allowed(is_merge_active: bool, staged_count: usize) -> bool {
    staged_count > 0 || is_merge_active
}

/// The parent tab a commit in the active submodule tab is also recorded in.
pub(super) struct ParentRecordTarget {
    pub(super) parent_repo_id: RepoId,
    pub(super) parent_name: SharedString,
    pub(super) parent_branch: Option<String>,
    pub(super) path: std::path::PathBuf,
    /// Staged files in the parent that stay staged and out of the commit.
    pub(super) parent_other_staged: usize,
}

/// The parent's commit message for a submodule commit.
pub(super) fn parent_record_message(path: &std::path::Path, message: &str) -> String {
    let summary = message.lines().next().unwrap_or_default().trim();
    if summary.is_empty() {
        format!("Update {}", path.display())
    } else {
        format!("Update {}: {summary}", path.display())
    }
}

impl DetailsPaneView {
    /// When the active tab is a submodule whose parent repository has a tab
    /// open, the parent the commit form can record a new commit in.
    pub(super) fn parent_record_target(&self) -> Option<ParentRecordTarget> {
        let repo = self.active_repo()?;
        let parent = crate::view::submodule_links::parent_tab(&self.state, repo)?;
        let parent_repo = self
            .state
            .repos
            .iter()
            .find(|candidate| candidate.id == parent.repo_id)?;
        let parent_branch = match &parent_repo.head_branch {
            Loadable::Ready(branch) if branch != "HEAD" => Some(branch.clone()),
            _ => None,
        };
        let parent_other_staged = parent_repo.staged_status_entries().map_or(0, |entries| {
            entries
                .iter()
                .filter(|entry| entry.path != parent.path)
                .count()
        });
        Some(ParentRecordTarget {
            parent_repo_id: parent.repo_id,
            parent_name: crate::view::path_display::repo_path_name(&parent_repo.spec.workdir),
            parent_branch,
            path: parent.path,
            parent_other_staged,
        })
    }

    /// "Also record in <parent>": a check box under the message, in the
    /// commit form's own type and colors.
    fn parent_record_row(
        &self,
        target: ParentRecordTarget,
        checked: bool,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let scaled_px = crate::ui_scale::scaler(crate::ui_scale::current(cx).percent);
        let border = if checked {
            theme.colors.accent.foreground
        } else {
            theme.colors.stroke.default
        };
        let mut detail = match &target.parent_branch {
            Some(branch) => format!("on {branch}"),
            None => "detached HEAD".to_string(),
        };
        if checked && target.parent_other_staged > 0 {
            let noun = if target.parent_other_staged == 1 {
                "file stays"
            } else {
                "files stay"
            };
            detail.push_str(&format!(
                " · {} other staged {noun} out",
                target.parent_other_staged
            ));
        }
        let tooltip: SharedString = format!(
            "After this commit, {} commits {} at the new commit, and nothing else",
            target.parent_name,
            target.path.display()
        )
        .into();
        div()
            .id("commit_record_in_parent")
            .debug_selector(|| "commit_record_in_parent".to_string())
            .flex()
            .items_center()
            .gap_2()
            .px_1()
            .min_w(px(0.0))
            .cursor_pointer()
            .child(
                div()
                    .flex_none()
                    .size(scaled_px(14.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(border)
                    .rounded(scaled_px(theme.radii.control * 0.5))
                    .when(checked, |this| {
                        this.bg(with_alpha(
                            theme.colors.accent.foreground,
                            if theme.is_dark { 0.18 } else { 0.12 },
                        ))
                        .child(svg_icon(
                            "icons/check.svg",
                            theme.colors.accent.foreground,
                            scaled_px(9.0),
                        ))
                    }),
            )
            .child(svg_icon(
                "icons/box.svg",
                theme.colors.foreground.secondary,
                scaled_px(12.0),
            ))
            .child(
                div()
                    .flex_none()
                    .text_size(theme.ui_text(13.0))
                    .text_color(theme.colors.foreground.primary)
                    .child(format!("Also record in {}", target.parent_name)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(detail),
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                    let Some(repo_id) = this.active_repo_id() else {
                        return;
                    };
                    if !this.record_in_parent_off.remove(&repo_id) {
                        this.record_in_parent_off.insert(repo_id);
                    }
                    cx.notify();
                }),
            )
            .gitcomet_tooltip(theme, tooltip)
    }

    fn records_in_parent(&self, repo_id: RepoId) -> bool {
        !self.commit_amend_enabled && !self.record_in_parent_off.contains(&repo_id)
    }

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
        let parent_record = self
            .parent_record_target()
            .filter(|_| self.records_in_parent(repo_id));
        if amend {
            self.store.dispatch(Msg::CommitAmend {
                repo_id,
                message: message.trim().to_string(),
                push_after_commit: self.commit_push_after_enabled,
            });
        } else if let Some(target) = parent_record {
            self.store.dispatch(Msg::CommitLinked {
                repo_id,
                parent_repo_id: target.parent_repo_id,
                parent_message: parent_record_message(&target.path, message.trim()),
                path: target.path,
                message: message.trim().to_string(),
                push: self.commit_push_after_enabled,
            });
        } else {
            self.store.dispatch(Msg::Commit {
                repo_id,
                message: message.trim().to_string(),
                push_after_commit: self.commit_push_after_enabled,
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
        let parent_record = self.parent_record_target();
        let records_in_parent = parent_record.is_some()
            && self
                .active_repo_id()
                .is_some_and(|repo_id| self.records_in_parent(repo_id));
        let commit_label = match (self.commit_amend_enabled, self.commit_push_after_enabled) {
            (false, false) if records_in_parent => "Commit and Record in Parent",
            (false, true) if records_in_parent => "Commit, Record and Push Both",
            (false, false) => "Commit",
            (false, true) => "Commit changes and Push",
            (true, false) => "Amend Previous Commit",
            (true, true) => "Amend and Push Safely",
        };
        let commit_tooltip = match (self.commit_amend_enabled, self.commit_push_after_enabled) {
            (false, false) if records_in_parent => {
                "Commit staged changes, then commit the new submodule commit in the parent"
            }
            (false, true) if records_in_parent => {
                "Commit, record the new commit in the parent, push this submodule, then push \
                 the parent once the submodule push succeeded"
            }
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
        let parent_record_row = parent_record
            .filter(|_| !self.commit_amend_enabled)
            .map(|target| self.parent_record_row(target, records_in_parent, cx));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(commit_message)
            .children(parent_record_row)
            .child(
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
