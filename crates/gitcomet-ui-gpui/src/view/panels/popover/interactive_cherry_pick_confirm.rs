use super::merge_commit_confirm::merge_commit_repo_is_ready;
use super::*;
use gitcomet_core::services::InteractiveRebaseEntry;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    entries: Vec<InteractiveRebaseEntry>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let actions_disabled = !merge_commit_repo_is_ready(repo);
    let picked = entries
        .iter()
        .filter(|entry| entry.action != InteractiveRebaseAction::Drop)
        .count();
    // Reword and squash steps rewrite commit messages, so they need commits.
    let needs_commits = entries.iter().any(|entry| {
        !matches!(
            entry.action,
            InteractiveRebaseAction::Pick | InteractiveRebaseAction::Drop
        )
    });

    let dispatch = move |this: &mut PopoverHost,
                         commit: bool,
                         window: &mut Window,
                         cx: &mut gpui::Context<PopoverHost>| {
        // Another operation may have started while the dialog was open.
        let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
        if !merge_commit_repo_is_ready(repo) {
            cx.notify();
            return;
        }
        this.store.dispatch(Msg::InteractiveCherryPick {
            repo_id,
            entries: entries.clone(),
            commit,
        });
        this.store
            .dispatch(Msg::CancelInteractiveCherryPickSetup { repo_id });
        this.close_popover_and_restore_focus(window, cx);
    };

    let commits = if picked == 1 { "commit" } else { "commits" };
    let mut dialog = ConfirmDialog::new("Commit cherry-picked commits?", DIALOG_380_WIDTH)
        .text(
            theme,
            format!("Apply {picked} {commits} to the current branch?"),
        )
        .note(theme, "Commit each cherry-picked commit immediately?");
    if needs_commits {
        dialog = dialog.note(theme, "Reword and squash steps need commits.");
    }

    dialog.render(
        theme,
        dialog_cancel_button(
            "interactive_cherry_pick_cancel",
            "interactive_cherry_pick_cancel_hint",
            theme,
            cx,
        ),
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                components::Button::new("interactive_cherry_pick_no", "No")
                    .style(components::ButtonStyle::Outlined)
                    .disabled(actions_disabled || needs_commits)
                    .on_click(theme, cx, {
                        let dispatch = dispatch.clone();
                        move |this, _e, window, cx| dispatch(this, false, window, cx)
                    }),
            )
            .child(
                components::Button::new("interactive_cherry_pick_yes", "Yes")
                    .style(components::ButtonStyle::Filled)
                    .disabled(actions_disabled)
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        dispatch(this, true, window, cx)
                    }),
            ),
        cx,
    )
}
