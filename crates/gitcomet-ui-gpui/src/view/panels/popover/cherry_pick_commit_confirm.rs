use super::commit_mainline;
use super::merge_commit_confirm::merge_commit_repo_is_ready;
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
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let actions_disabled = mainline.pending
        || !merge_commit_repo_is_ready(repo)
        || commit_mainline::mainline_actions_disabled(mainline_choices.len(), selected_mainline);
    let sha = commit_id.as_ref();
    let short = sha.get(0..7).unwrap_or(sha).to_string();
    let summary = commit_mainline::commit_summary(this, repo_id, &commit_id);

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
        this.store.dispatch(Msg::CherryPickCommit {
            repo_id,
            commit_id: commit_id.clone(),
            commit: commit_now,
            mainline: selected_mainline,
            summary: summary.clone(),
        });
        this.close_popover_and_restore_focus(window, cx);
    };

    let mut dialog = ConfirmDialog::new("Commit cherry-picked commit?", DIALOG_380_WIDTH)
        .text(theme, format!("Apply {short} to the current branch?"))
        .note(theme, "Commit the cherry-picked change immediately?");
    if mainline.pending {
        dialog = dialog.note(theme, "Checking the commit's parents…");
    }
    if is_merge {
        dialog = dialog.section(commit_mainline::mainline_section(
            theme,
            mainline_choices,
            selected_mainline,
            "cherry_pick_mainline",
            "Choose the parent Git should treat as the merge's mainline.",
            cx,
        ));
    }

    dialog.render(
        theme,
        dialog_cancel_button(
            "cherry_pick_commit_cancel",
            "cherry_pick_commit_cancel_hint",
            theme,
            cx,
        ),
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                components::Button::new("cherry_pick_commit_no", "No")
                    .style(components::ButtonStyle::Outlined)
                    .disabled(actions_disabled)
                    .on_click(theme, cx, {
                        let dispatch = dispatch.clone();
                        move |this, _e, window, cx| dispatch(this, false, window, cx)
                    }),
            )
            .child(
                components::Button::new("cherry_pick_commit_yes", "Yes")
                    .style(components::ButtonStyle::Filled)
                    .disabled(actions_disabled)
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        dispatch(this, true, window, cx)
                    }),
            ),
        cx,
    )
}
