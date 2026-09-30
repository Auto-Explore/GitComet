use super::merge_commit_confirm::merge_commit_repo_is_ready;
use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    target: DiffTarget,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let source = gitcomet_core::services::apply_file_change_source(&target)
        .map(|(path, revision)| (path.display().to_string(), revision));
    let actions_disabled = source.is_none() || !merge_commit_repo_is_ready(repo);
    let (path, revision) = source.unwrap_or_default();

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
        this.store.dispatch(Msg::ApplyFileChange {
            repo_id,
            target: target.clone(),
            commit,
        });
        this.close_popover_and_restore_focus(window, cx);
    };

    ConfirmDialog::new("Commit applied change?", DIALOG_380_WIDTH)
        .text(
            theme,
            format!("Apply the change to this file from {revision} to the current branch?"),
        )
        .mono_value(theme, path)
        .note(theme, "Commit the applied change immediately?")
        .render(
            theme,
            dialog_cancel_button(
                "apply_file_change_cancel",
                "apply_file_change_cancel_hint",
                theme,
                cx,
            ),
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    components::Button::new("apply_file_change_no", "No")
                        .style(components::ButtonStyle::Outlined)
                        .disabled(actions_disabled)
                        .on_click(theme, cx, {
                            let dispatch = dispatch.clone();
                            move |this, _e, window, cx| dispatch(this, false, window, cx)
                        }),
                )
                .child(
                    components::Button::new("apply_file_change_yes", "Yes")
                        .style(components::ButtonStyle::Filled)
                        .disabled(actions_disabled)
                        .on_click(theme, cx, move |this, _e, window, cx| {
                            dispatch(this, true, window, cx)
                        }),
                ),
            cx,
        )
}
