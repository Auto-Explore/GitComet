use super::commit_mainline;
use super::merge_commit_confirm::{merge_commit_destination_label, merge_commit_repo_is_ready};
use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    commit_id: CommitId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let mainline = commit_mainline::mainline(this, repo_id, &commit_id);
    let mainline_choices = mainline.choices;
    let is_merge = mainline_choices.len() > 1;
    let selected_mainline = is_merge.then_some(this.commit_mainline).flatten();
    let sha = commit_id.as_ref();
    let short = sha.get(0..7).unwrap_or(sha).to_string();
    let summary = commit_mainline::commit_summary(this, repo_id, &commit_id);
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let destination = merge_commit_destination_label(repo);
    // `revert_with_output` refuses a dirty index, so do not let the user
    // confirm into that error.
    let staged_changes = repo.is_some_and(|repo| {
        repo.staged_status_entries()
            .is_some_and(|entries| !entries.is_empty())
    });
    let actions_disabled = mainline.pending
        || staged_changes
        || !merge_commit_repo_is_ready(repo)
        || commit_mainline::mainline_actions_disabled(mainline_choices.len(), selected_mainline);

    let dispatch = move |this: &mut PopoverHost,
                         commit_now: bool,
                         window: &mut Window,
                         cx: &mut gpui::Context<PopoverHost>| {
        // Another operation may have started while the dialog was open.
        let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
        if !merge_commit_repo_is_ready(repo) {
            cx.notify();
            return;
        }
        this.store.dispatch(Msg::RevertCommit {
            repo_id,
            commit_id: commit_id.clone(),
            commit: commit_now,
            mainline: selected_mainline,
            summary: summary.clone(),
        });
        this.close_popover_and_restore_focus(window, cx);
    };

    let mut dialog = ConfirmDialog::new("Commit revert?", DIALOG_380_WIDTH)
        .text(theme, format!("Revert {short} on {destination}?"))
        .note(
            theme,
            "Commit the revert immediately? No stages the reverted changes without committing.",
        );
    if mainline.pending {
        dialog = dialog.note(theme, "Checking the commit's parents…");
    }
    if staged_changes {
        dialog = dialog.note(
            theme,
            "Commit or unstage your staged changes first: a revert needs a clean index.",
        );
    }
    if is_merge {
        dialog = dialog
            .note(
                theme,
                "Later merges of the same branch will not bring the reverted changes back.",
            )
            .section(commit_mainline::mainline_section(
                theme,
                mainline_choices,
                selected_mainline,
                "revert_mainline",
                "Choose the parent to keep; changes the merge brought in from the other side \
                 are undone.",
                cx,
            ));
    }

    dialog.render(
        theme,
        dialog_cancel_button(
            "revert_commit_cancel",
            "revert_commit_cancel_hint",
            theme,
            cx,
        ),
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                components::Button::new("revert_commit_no", "No")
                    .style(components::ButtonStyle::Outlined)
                    .disabled(actions_disabled)
                    .on_click(theme, cx, {
                        let dispatch = dispatch.clone();
                        move |this, _e, window, cx| dispatch(this, false, window, cx)
                    }),
            )
            .child(
                components::Button::new("revert_commit_yes", "Yes")
                    .style(components::ButtonStyle::Filled)
                    .disabled(actions_disabled)
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        dispatch(this, true, window, cx)
                    }),
            ),
        cx,
    )
}
