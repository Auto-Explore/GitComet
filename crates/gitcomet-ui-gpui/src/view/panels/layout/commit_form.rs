//! The commit message form: submit gating, submission, and the commit box.

use super::*;
use crate::view::pending_commit::PendingCommit;

pub(super) fn merge_active(repo: Option<&RepoState>) -> bool {
    repo.is_some_and(|r| matches!(&r.merge_commit_message, Loadable::Ready(Some(_))))
}

pub(super) fn commit_allowed(is_merge_active: bool, staged_count: usize) -> bool {
    staged_count > 0 || is_merge_active
}

/// Where a commit in a submodule tab is recorded: the parent worktree's open
/// tab and what the commit form tells about it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct ParentRecordTarget {
    pub(in crate::view) parent: RepoId,
    pub(in crate::view) parent_name: String,
    pub(in crate::view) child_name: String,
    /// The submodule's path in the parent.
    pub(in crate::view) path: std::path::PathBuf,
    /// The commit the parent records for the submodule now.
    pub(in crate::view) recorded: Option<String>,
    /// Files staged in the parent besides the submodule. They stay staged.
    pub(in crate::view) other_staged: usize,
}

pub(in crate::view) fn parent_record_target(
    state: &AppState,
    repo_id: RepoId,
) -> Option<ParentRecordTarget> {
    let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
    let link = repo.submodule_parent.as_ref()?;
    let parent = state
        .repos
        .iter()
        .find(|candidate| Some(candidate.id) == link.repo_id)?;
    let recorded = state
        .submodule_row_in_parent(repo_id)
        .map(|row| gitcomet_core::domain::short_commit_id(row.recorded_head.as_ref()).to_string());
    let other_staged = parent.staged_status_entries().map_or(0, |entries| {
        entries
            .iter()
            .filter(|entry| entry.path != link.path)
            .count()
    });
    Some(ParentRecordTarget {
        parent: parent.id,
        parent_name: link.parent_name(),
        child_name: link.submodule_name(),
        path: link.path.clone(),
        recorded,
        other_staged,
    })
}

/// First-level submodules with uncommitted changes, which a commit in `repo`
/// would leave behind.
pub(in crate::view) fn submodules_left_behind(repo: &RepoState) -> Vec<std::path::PathBuf> {
    match &repo.submodules {
        Loadable::Ready(rows) => rows
            .iter()
            .filter(|row| !row.nested && row.has_inner_changes)
            .map(|row| row.path.clone())
            .collect(),
        _ => Vec::new(),
    }
}

/// The note under the parent message about the parent's other staged files.
pub(in crate::view) fn other_staged_note(target: &ParentRecordTarget) -> Option<String> {
    match target.other_staged {
        0 => None,
        1 => Some(format!(
            "{} has 1 other staged file. It stays staged and is not part of this commit.",
            target.parent_name
        )),
        count => Some(format!(
            "{} has {count} other staged files. They stay staged and are not part of this commit.",
            target.parent_name
        )),
    }
}

impl DetailsPaneView {
    /// The parent a commit in the active tab is recorded in, unless the tab
    /// is not a submodule, the user turned recording off, or the commit
    /// amends.
    pub(in crate::view) fn active_parent_record_target(&self) -> Option<ParentRecordTarget> {
        let repo_id = self.active_repo_id()?;
        if self.commit_amend_enabled || self.record_in_parent_off.contains(&repo_id) {
            return None;
        }
        parent_record_target(&self.state, repo_id)
    }

    /// Keep the parent message on its default while the user has not edited
    /// it in this tab.
    pub(in crate::view) fn sync_parent_message(
        &mut self,
        commit_message: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.parent_message_user_edited {
            return;
        }
        let Some(target) = self
            .active_repo_id()
            .and_then(|repo_id| parent_record_target(&self.state, repo_id))
        else {
            return;
        };
        let message = gitcomet_state::model::default_parent_message(&target.path, commit_message);
        self.set_parent_message_programmatically(message, cx);
    }

    /// Put `message` in the parent message box without counting it as the
    /// user's edit.
    pub(in crate::view) fn set_parent_message_programmatically(
        &mut self,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.parent_message_last_text.as_ref() == message {
            return;
        }
        self.parent_message_last_text = message.clone().into();
        self.parent_message_input
            .update(cx, |input, cx| input.set_text(message, cx));
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

    pub(in crate::view) fn can_submit_commit(&self, message: &str) -> bool {
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
        self.submit_commit_with(false, None, cx)
    }

    pub(in crate::view) fn submit_commit_past_submodule_guard(
        &mut self,
        shelve: Option<gitcomet_state::model::SubmoduleShelve>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.submit_commit_with(true, shelve, cx)
    }

    fn submit_commit_with(
        &mut self,
        past_submodule_guard: bool,
        shelve: Option<gitcomet_state::model::SubmoduleShelve>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };
        let message = self
            .commit_message_input
            .read_with(cx, |input, _| input.text().to_string());
        let amend = self.commit_amend_enabled;
        // While the linked commit panel shows, the changed submodules are in
        // plain sight: those with staged files commit with the parent, and the
        // rest stay as they are without asking.
        let linked = self.active_linked_panel();
        let linked_children = match &linked {
            Some(panel) => {
                let children = self.linked_children(panel, cx);
                if !self.can_submit_linked_commit(&message, children.as_deref()) {
                    return false;
                }
                children.filter(|children| !children.is_empty())
            }
            None => {
                if !self.can_submit_commit(&message) {
                    return false;
                }
                None
            }
        };
        if !past_submodule_guard
            && !amend
            && linked.is_none()
            && let Some(repo) = self.active_repo()
        {
            let submodules = submodules_left_behind(repo);
            if !submodules.is_empty() {
                let prompt = SubmoduleLeaveBehindPrompt {
                    repo_id,
                    submodules,
                    subject: message
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                        .unwrap_or_default()
                        .to_string(),
                };
                let _ = self.root_view.update(cx, |root, cx| {
                    root.pending_submodule_leave_behind_prompt = Some(prompt);
                    cx.notify();
                });
                return false;
            }
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
        if let Some(children) = linked_children {
            self.pending_linked_messages
                .insert(repo_id, children.clone());
            self.store.dispatch(Msg::CommitLinked {
                origin: repo_id,
                children,
                parent_message: message.trim().to_string(),
                push: self.commit_push_after_enabled,
                // The panel lists every changed submodule: none is left
                // behind to shelve.
                shelve: None,
            });
        } else if let Some(target) = self.active_parent_record_target() {
            let parent_message = self
                .parent_message_input
                .read_with(cx, |input, _| input.text().trim().to_string());
            self.store.dispatch(Msg::CommitLinked {
                origin: repo_id,
                children: vec![gitcomet_state::model::LinkedChild {
                    path: target.path,
                    message: message.trim().to_string(),
                }],
                parent_message,
                push: self.commit_push_after_enabled,
                shelve,
            });
            // The next commit's parent message starts from its default again.
            self.parent_message_user_edited = false;
        } else if amend {
            self.store.dispatch(Msg::CommitAmend {
                repo_id,
                message: message.trim().to_string(),
                push_after_commit: self.commit_push_after_enabled,
            });
        } else if let Some(shelve) = shelve {
            self.store.dispatch(Msg::CommitShelvingSubmodules {
                repo_id,
                message: message.trim().to_string(),
                push_after_commit: self.commit_push_after_enabled,
                shelve,
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
        let linked = self.active_linked_panel();
        let can_submit_commit = match &linked {
            Some(panel) => {
                let children = self.linked_children(panel, cx);
                self.can_submit_linked_commit(&commit_message_text, children.as_deref())
            }
            None => self.can_submit_commit(&commit_message_text),
        };
        let repo_key = self.active_repo_id().map(|id| id.0).unwrap_or(0);
        let icon_color = theme.colors.accent.foreground;
        let icon = |path: &'static str| svg_icon(path, icon_color, scaled_px(14.0));
        let spinner = |id: (&'static str, u64)| {
            components::ButtonSlot::spinner(id, icon_color, scaled_px(14.0))
        };
        let record_target = self.active_parent_record_target();
        let linked_label = linked
            .as_ref()
            .map(|panel| linked_commit_label(panel, self.commit_push_after_enabled));
        let (commit_label, commit_tooltip): (SharedString, SharedString) = match (
            linked_label,
            &record_target,
        ) {
            (Some((label, tooltip)), _) => (label.into(), tooltip.into()),
            (None, Some(target)) if self.commit_push_after_enabled => (
                format!("Commit, record in {} and Push", target.parent_name).into(),
                format!(
                    "Commit staged changes, record the commit in {}, then push {} first and {} after it",
                    target.parent_name, target.child_name, target.parent_name
                )
                .into(),
            ),
            (None, Some(target)) => (
                format!("Commit and record in {}", target.parent_name).into(),
                format!(
                    "Commit staged changes and record the commit in {}",
                    target.parent_name
                )
                .into(),
            ),
            (None, None) => {
                let label = match (self.commit_amend_enabled, self.commit_push_after_enabled) {
                    (false, false) => "Commit",
                    (false, true) => "Commit changes and Push",
                    (true, false) => "Amend Previous Commit",
                    (true, true) => "Amend and Push Safely",
                };
                let tooltip = match (self.commit_amend_enabled, self.commit_push_after_enabled) {
                    (false, false) => "Commit staged changes",
                    (false, true) => "Commit staged changes and push",
                    (true, false) => "Amend the previous commit",
                    (true, true) => {
                        "Amend the previous commit; published amends require explicit force push with lease"
                    }
                };
                (label.into(), tooltip.into())
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
                    .gitcomet_tooltip(theme, commit_tooltip)
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
        let record_section = self.parent_record_section(&scaled_px, cx);
        let linked_toggle = self.linked_commit_toggle(cx);
        let push_note = linked
            .as_ref()
            .filter(|_| self.commit_push_after_enabled)
            .and_then(linked_push_note);
        let body = match &linked {
            Some(panel) => self
                .linked_commit_panel(panel, commit_message, cx)
                .into_any_element(),
            None => commit_message,
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(linked_toggle, |form, toggle| form.child(toggle))
            .child(body)
            .when_some(record_section, |form, section| form.child(section))
            .when_some(push_note, |form, note| {
                form.child(
                    div()
                        .debug_selector(|| "linked_commit_push_note".to_string())
                        .px_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(note),
                )
            })
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

    /// "Also record in <parent>" on a submodule tab's commit form: a check
    /// box, and while it is on, the pointer the commit moves, the parent's
    /// message and a note about the parent's other staged files.
    fn parent_record_section(
        &mut self,
        scaled_px: &impl Fn(f32) -> Pixels,
        cx: &mut gpui::Context<Self>,
    ) -> Option<gpui::Div> {
        let theme = self.theme;
        let repo_id = self.active_repo_id()?;
        if self.commit_amend_enabled {
            return None;
        }
        let target = parent_record_target(&self.state, repo_id)?;
        let checked = !self.record_in_parent_off.contains(&repo_id);
        let border = if checked {
            theme.colors.accent.foreground
        } else {
            theme.colors.stroke.default
        };
        let check_box = div()
            .flex_none()
            .size(scaled_px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(border)
            .rounded(scaled_px(theme.radii.control * 0.5))
            .when(checked, |check_box| {
                check_box
                    .bg(with_alpha(
                        theme.colors.accent.foreground,
                        if theme.is_dark { 0.18 } else { 0.12 },
                    ))
                    .child(svg_icon(
                        "icons/check.svg",
                        theme.colors.accent.foreground,
                        scaled_px(10.0),
                    ))
            });
        let toggle = div()
            .id("record_in_parent_toggle")
            .debug_selector(|| "record_in_parent_toggle".to_string())
            .flex()
            .items_center()
            .gap_2()
            .px_1()
            .rounded(px(theme.radii.row))
            .control_interaction(InteractionStyle::new(theme), InteractionState::default())
            .child(check_box)
            .child(
                div()
                    .text_size(theme.ui_text(13.0))
                    .text_color(theme.colors.foreground.primary)
                    .child(format!("Also record in {}", target.parent_name)),
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                    if !this.record_in_parent_off.remove(&repo_id) {
                        this.record_in_parent_off.insert(repo_id);
                    }
                    cx.notify();
                }),
            )
            .gitcomet_tooltip(
                theme,
                format!(
                    "Commit the new {} commit in {} too, without its other staged files",
                    target.child_name, target.parent_name
                )
                .into(),
            );
        let mut section = div()
            .debug_selector(|| "record_in_parent_section".to_string())
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .py_2()
            .rounded(px(theme.radii.control))
            .border_1()
            .border_color(theme.colors.stroke.default)
            .child(toggle);
        if !checked {
            return Some(section);
        }
        let pointer = format!(
            "{}  {} → new commit",
            target.path.display(),
            target.recorded.as_deref().unwrap_or("not recorded")
        );
        section = section
            .child(
                div()
                    .debug_selector(|| "record_in_parent_pointer".to_string())
                    .pl(scaled_px(24.0))
                    .text_size(theme.ui_text(12.0))
                    .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.foreground.secondary)
                    .truncate()
                    .child(pointer),
            )
            .child(
                div()
                    .pl(scaled_px(24.0))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child("Parent message"),
                    )
                    .child(self.parent_message_input.clone()),
            );
        if let Some(note) = other_staged_note(&target) {
            section = section.child(
                div()
                    .debug_selector(|| "record_in_parent_note".to_string())
                    .pl(scaled_px(24.0))
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(note),
            );
        }
        Some(section)
    }
}
