//! Editing the shown commit's message in the commit details.

use super::*;
use gitcomet_core::domain::{CommitDetails, FileStatusKind};
use gitcomet_core::squash::FirstParentPosition;

/// Whether and how the shown commit's message can be edited, worked out
/// once per render.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct CommitRewordAvailability {
    /// Why the message cannot be edited right now, worded for a tooltip;
    /// `None` when it can.
    pub(in crate::view) blocker: Option<&'static str>,
    /// The upstream (`origin/main`) that already has the commit, so updating
    /// it rewrites published history.
    pub(in crate::view) published_on: Option<String>,
}

/// HEAD is amended in place, merge or not. An older commit is reworded by a
/// rebase that replays its descendants, so it must sit on HEAD's linear
/// first-parent history with a clean worktree.
pub(in crate::view) fn commit_reword_availability(
    repo: &RepoState,
    details: &CommitDetails,
) -> CommitRewordAvailability {
    // The reducer refuses a bare repository's reword before recording the
    // request, which would leave the submitted editor waiting.
    if repo.is_bare() {
        return CommitRewordAvailability {
            blocker: Some(gitcomet_core::domain::WORKING_TREE_REQUIRED_MESSAGE),
            published_on: None,
        };
    }
    if repo.history_rewrite_busy() {
        return CommitRewordAvailability {
            blocker: Some("Finish the running Git operation to edit this commit's message"),
            published_on: None,
        };
    }
    let position = repo.first_parent_position_from_head(&details.id);
    let blocker = match position {
        FirstParentPosition::Linear { distance: 0 } => None,
        _ if details.parent_ids.len() > 1 => {
            Some("A merge commit's message can only be edited while it is HEAD")
        }
        FirstParentPosition::Linear { .. } if details.parent_ids.is_empty() => {
            Some("The root commit's message can only be edited while it is HEAD")
        }
        FirstParentPosition::Linear { .. } => uncommitted_changes_blocker(repo),
        FirstParentPosition::BelowMerge => {
            Some("A merge after this commit keeps its message from being edited")
        }
        FirstParentPosition::NotFound => {
            Some("Only commits in the current branch's loaded history can be edited")
        }
    };
    let published_on = match (blocker, position) {
        (None, FirstParentPosition::Linear { distance }) => published_upstream(repo, distance),
        _ => None,
    };
    CommitRewordAvailability {
        blocker,
        published_on,
    }
}

#[cfg(test)]
pub(in crate::view) fn commit_reword_blocker(
    repo: &RepoState,
    details: &CommitDetails,
) -> Option<&'static str> {
    commit_reword_availability(repo, details).blocker
}

/// The current branch's upstream, as `remote/branch`, when it already has the
/// commit `distance` first parents below HEAD. The `ahead` commits are those
/// the upstream lacks; on a linear chain from HEAD they are its top commits,
/// whether or not the branch is also behind, so the commit is published
/// exactly when it sits at least `ahead` steps down. `None` without an
/// upstream or a known divergence.
fn published_upstream(repo: &RepoState, distance: usize) -> Option<String> {
    let Loadable::Ready(head) = &repo.head_branch else {
        return None;
    };
    let Loadable::Ready(branches) = &repo.branches else {
        return None;
    };
    let branch = branches.iter().find(|branch| branch.name == *head)?;
    let upstream = branch.upstream.as_ref()?;
    let ahead = match (&branch.divergence, &repo.upstream_divergence) {
        (Some(divergence), _) | (None, Loadable::Ready(Some(divergence))) => divergence.ahead,
        _ => return None,
    };
    (distance >= ahead).then(|| format!("{}/{}", upstream.remote, upstream.branch))
}

/// Changes `git rebase` refuses to run over, or a status not known yet.
/// Untracked files don't count.
fn uncommitted_changes_blocker(repo: &RepoState) -> Option<&'static str> {
    let (Some(unstaged), Some(staged)) =
        (repo.worktree_status_entries(), repo.staged_status_entries())
    else {
        let failed = matches!(repo.worktree_status, Loadable::Error(_))
            || matches!(repo.staged_status, Loadable::Error(_))
            || matches!(repo.status, Loadable::Error(_));
        return Some(if failed {
            "The working tree status couldn't be loaded"
        } else {
            "Loading the working tree status…"
        });
    };
    let dirty = !staged.is_empty()
        || unstaged
            .iter()
            .any(|entry| entry.kind != FileStatusKind::Untracked);
    dirty.then_some("Commit or stash your changes to edit an older commit's message")
}

/// Whether `draft` would change the commit: not blank, and different from
/// `original` beyond the surrounding whitespace git strips anyway.
pub(in crate::view) fn commit_reword_submittable(original: &str, draft: &str) -> bool {
    let draft = draft.trim();
    !draft.is_empty() && draft != original.trim()
}

impl DetailsPaneView {
    /// The shown commit's details, when they are loaded.
    fn loaded_commit_details(
        &self,
        repo_id: RepoId,
        commit_id: &CommitId,
    ) -> Option<&CommitDetails> {
        let repo = self.state.repos.iter().find(|repo| repo.id == repo_id)?;
        match &repo.history_state.commit_details {
            Loadable::Ready(details) if details.id == *commit_id => Some(details),
            _ => None,
        }
    }

    fn commit_reword_availability_for(
        &self,
        repo_id: RepoId,
        commit_id: &CommitId,
    ) -> Option<CommitRewordAvailability> {
        let repo = self.state.repos.iter().find(|repo| repo.id == repo_id)?;
        let details = self.loaded_commit_details(repo_id, commit_id)?;
        Some(commit_reword_availability(repo, details))
    }

    /// Discards the draft unless it belongs to `shown`, the commit the pane
    /// is showing: selecting another commit or repository ends the edit.
    pub(super) fn retain_commit_reword_for(
        &mut self,
        shown: Option<(RepoId, &CommitId)>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self
            .commit_reword
            .as_ref()
            .is_some_and(|draft| shown != Some((draft.repo_id, &draft.commit_id)))
        {
            self.dismiss_commit_reword(cx);
        }
        self.advance_commit_reword_submission(cx);
    }

    /// Only this request's terminal status releases the submitted draft.
    /// A missing record can simply mean dispatch has not reached the store.
    pub(in crate::view) fn advance_commit_reword_submission(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(draft) = self.commit_reword.as_mut() else {
            return;
        };
        let Some(submission) = draft.submitting.or_else(|| {
            draft
                .retained_request
                .map(|request_id| CommitRewordSubmission { request_id })
        }) else {
            return;
        };
        let Some(repo) = self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == draft.repo_id)
        else {
            return;
        };
        let Some(request) = repo.reword.requests.get(&submission.request_id) else {
            return;
        };
        if request.status.is_pending() {
            draft.submitting = Some(submission);
            self.commit_reword_input.update(cx, |input, cx| {
                input.set_read_only(true, cx);
            });
            return;
        }
        if request.status == gitcomet_state::model::RewordStatus::Completed {
            self.dismiss_commit_reword(cx);
        } else {
            let resumable = repo.reword.can_resume(submission.request_id);
            if !resumable {
                self.store.dispatch(Msg::DismissRewordCommit {
                    repo_id: draft.repo_id,
                    request_id: submission.request_id,
                });
            }
            draft.retained_request = resumable.then_some(submission.request_id);
            draft.submitting = None;
            self.commit_reword_input.update(cx, |input, cx| {
                input.set_read_only(resumable, cx);
            });
        }
    }

    fn dismiss_commit_reword(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(draft) = self.commit_reword.take()
            && let Some(request_id) = draft
                .submitting
                .map(|submission| submission.request_id)
                .or(draft.retained_request)
        {
            self.store.dispatch(Msg::DismissRewordCommit {
                repo_id: draft.repo_id,
                request_id,
            });
        }
        self.commit_reword_input.update(cx, |input, cx| {
            input.set_read_only(false, cx);
        });
    }

    pub(super) fn commit_reword_is_editing(&self, repo_id: RepoId, commit_id: &CommitId) -> bool {
        self.commit_reword
            .as_ref()
            .is_some_and(|draft| draft.repo_id == repo_id && draft.commit_id == *commit_id)
    }

    pub(in crate::view) fn begin_commit_reword(
        &mut self,
        repo_id: RepoId,
        commit_id: &CommitId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(availability) = self.commit_reword_availability_for(repo_id, commit_id) else {
            return;
        };
        if availability.blocker.is_some() {
            return;
        }
        let Some(details) = self.loaded_commit_details(repo_id, commit_id) else {
            return;
        };
        let message = details.message.clone();
        self.commit_reword = Some(CommitRewordDraft {
            repo_id,
            commit_id: commit_id.clone(),
            original: message.clone(),
            submittable: false,
            confirming: None,
            submitting: None,
            retained_request: None,
        });
        self.commit_reword_input.update(cx, |input, cx| {
            input.set_read_only(false, cx);
            input.set_text(message, cx);
        });
        self.commit_reword_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus = self.commit_reword_input.read(cx).focus_handle();
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Drops the draft and hands focus back to the history list, where the
    /// commit was picked.
    pub(in crate::view) fn cancel_commit_reword(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_reword.is_none() {
            return;
        }
        self.dismiss_commit_reword(cx);
        self.focus_history_panel(window, cx);
        cx.notify();
    }

    /// Dispatches the reword. Success closes the draft; a failure leaves the
    /// edit in place. A
    /// published commit takes a second submit, after the first turned the
    /// button into "Confirm update"; editing the message withdraws it.
    pub(in crate::view) fn submit_commit_reword(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let Some(draft) = self.commit_reword.as_ref() else {
            return false;
        };
        if draft.submitting.is_some()
            || draft.retained_request.is_some_and(|request_id| {
                self.state
                    .repos
                    .iter()
                    .find(|repo| repo.id == draft.repo_id)
                    .is_some_and(|repo| repo.reword.can_resume(request_id))
            })
        {
            return false;
        }
        let message = self.commit_reword_input.read(cx).text().to_string();
        if !commit_reword_submittable(&draft.original, &message) {
            return false;
        }
        let (repo_id, commit_id) = (draft.repo_id, draft.commit_id.clone());
        let confirmed = draft.confirming.as_deref() == Some(message.as_str());
        let Some(availability) = self.commit_reword_availability_for(repo_id, &commit_id) else {
            return false;
        };
        if availability.blocker.is_some() {
            return false;
        }
        let Some(draft) = self.commit_reword.as_mut() else {
            return false;
        };
        if availability.published_on.is_some() && !confirmed {
            draft.confirming = Some(message);
            cx.notify();
            return true;
        }
        draft.confirming = None;
        if let Some(request_id) = draft.retained_request.take() {
            self.store.dispatch(Msg::DismissRewordCommit {
                repo_id,
                request_id,
            });
        }
        let request_id = gitcomet_state::model::RewordRequestId::new();
        draft.submitting = Some(CommitRewordSubmission { request_id });
        self.commit_reword_input.update(cx, |input, cx| {
            input.set_read_only(true, cx);
        });
        self.store.dispatch(Msg::RewordCommit {
            repo_id,
            request_id,
            commit_id,
            message: message.trim().to_string(),
        });
        cx.notify();
        true
    }

    pub(in crate::view) fn handle_commit_reword_submit_shortcut(
        &mut self,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.commit_reword.is_none()
            || !self
                .commit_reword_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
        {
            return false;
        }
        let _ = self.submit_commit_reword(cx);
        true
    }

    /// The message block: the read-only message beside its edit control, or
    /// the editor while the message is being edited. `availability` is
    /// `None` without a repository.
    pub(super) fn commit_details_message_section(
        &self,
        theme: AppTheme,
        repo_id: RepoId,
        details: &CommitDetails,
        availability: Option<&CommitRewordAvailability>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let blocker =
            availability.map_or(Some("No repository"), |availability| availability.blocker);
        if self.commit_reword_is_editing(repo_id, &details.id) {
            let published_on =
                availability.and_then(|availability| availability.published_on.as_deref());
            return self.commit_reword_editor(theme, repo_id, blocker, published_on, cx);
        }
        let ui_scale = self.ui_scale();
        let commit_id = details.id.clone();
        let edit = components::Button::new("commit_details_edit_message", "")
            .start_slot(
                svg_icon(
                    "icons/pencil.svg",
                    theme.colors.foreground.secondary,
                    ui_scale.px(12.0),
                )
                .debug_selector(|| "commit_details_edit_message_icon".to_string()),
            )
            .style(components::ButtonStyle::Transparent)
            .disabled(blocker.is_some())
            .on_click(theme, cx, move |this, _e, window, cx| {
                this.begin_commit_reword(repo_id, &commit_id, window, cx);
            })
            .debug_selector(|| "commit_details_edit_message".to_string())
            .gitcomet_tooltip(theme, blocker.unwrap_or("Edit commit message").into());

        div()
            .flex()
            .items_start()
            .gap_1()
            .w_full()
            .min_w(px(0.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(self.commit_details_message_view(theme, repo_id)),
            )
            .child(div().flex_none().child(edit))
            .into_any_element()
    }

    /// The editable message with "Cancel" and "Update commit" under it, and a
    /// warning when the commit is already published. Secondary-Enter submits
    /// and Escape cancels while the input has focus.
    fn commit_reword_editor(
        &self,
        theme: AppTheme,
        repo_id: RepoId,
        blocker: Option<&'static str>,
        published_on: Option<&str>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let ui_scale = self.ui_scale();
        let draft = self.commit_reword.as_ref();
        let text = self.commit_reword_input.read(cx).text();
        let submitting = draft.is_some_and(|draft| draft.submitting.is_some());
        let paused = !submitting && draft.is_some_and(|draft| draft.retained_request.is_some());
        let submittable =
            draft.is_some_and(|draft| commit_reword_submittable(&draft.original, text));
        let confirming = published_on.is_some()
            && draft.is_some_and(|draft| draft.confirming.as_deref() == Some(text));
        let icon_color = theme.colors.accent.foreground;
        let input = components::ScrollContainer::vertical(
            ("commit_reword_scroll_surface", repo_id.0),
            ("commit_reword_scrollbar", repo_id.0),
            self.commit_reword_scroll.clone(),
            ui_scale.px(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX),
        )
        .container_id(("commit_reword_container", repo_id.0))
        .render(theme, self.commit_reword_input.clone());
        let warning = published_on.map(|upstream| {
            div()
                .debug_selector(|| "commit_reword_published_warning".to_string())
                .flex()
                .items_start()
                .gap_1()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.status.warning.foreground)
                .child(
                    svg_icon(
                        "icons/warning.svg",
                        theme.colors.status.warning.foreground,
                        ui_scale.px(12.0),
                    )
                    .flex_none()
                    .mt(ui_scale.px(2.0)),
                )
                .child(div().flex_1().min_w(px(0.0)).child(format!(
                    "This commit is already on {upstream}. Updating it rewrites published \
                     history and will need a force push."
                )))
        });
        let cancel = components::Button::new(
            "commit_reword_cancel",
            if submitting || paused {
                "Close editor"
            } else {
                "Cancel"
            },
        )
        .style(components::ButtonStyle::Outlined)
        .on_click(theme, cx, |this, _e, window, cx| {
            this.cancel_commit_reword(window, cx);
        })
        .debug_selector(|| "commit_reword_cancel".to_string())
        .gitcomet_tooltip_keyed(
            theme,
            if paused {
                "Close the editor; the Git operation stays paused"
            } else if submitting {
                "Close the editor; the Git operation will continue"
            } else {
                "Discard the edit"
            }
            .into(),
            vec!["Esc".into()],
        );
        let (label, style, tooltip) = if confirming {
            (
                "Confirm update",
                components::ButtonStyle::Danger,
                "Rewrite the published commit; pushing it afterwards needs a force push",
            )
        } else {
            (
                "Update commit",
                components::ButtonStyle::Filled,
                "Rewrite the commit with this message",
            )
        };
        let update = components::Button::new("commit_reword_update", label)
            .start_slot(if submitting {
                components::ButtonSlot::spinner(
                    ("commit_reword_spinner", repo_id.0),
                    icon_color,
                    ui_scale.px(14.0),
                )
            } else {
                svg_icon("icons/check.svg", icon_color, ui_scale.px(14.0)).into()
            })
            .style(style)
            .disabled(submitting || paused || blocker.is_some() || !submittable)
            .on_click(theme, cx, |this, _e, _w, cx| {
                let _ = this.submit_commit_reword(cx);
            })
            .debug_selector(|| "commit_reword_update".to_string())
            .gitcomet_tooltip_keyed(
                theme,
                blocker.unwrap_or(tooltip).into(),
                vec![crate::kit::components::secondary_shortcut("Enter").into()],
            );

        div()
            .debug_selector(|| "commit_reword_editor".to_string())
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .min_w(px(0.0))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                    this.cancel_commit_reword(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(input)
            .when(paused, |editor| {
                editor.child(
                    div()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child("The update is paused. Continue or abort it before editing."),
                )
            })
            .children(warning)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(cancel)
                    .child(update),
            )
            .into_any_element()
    }
}
