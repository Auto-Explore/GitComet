use super::merge_commit_confirm::merge_commit_repo_is_ready;
use super::*;
use gitcomet_core::domain::ApplyChangeTarget;

/// Paths the dialog lists before summarizing the rest as "and N more".
const LISTED_PATHS: usize = 5;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    target: ApplyChangeTarget,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let actions_disabled = target.paths.is_empty() || !merge_commit_repo_is_ready(repo);
    let revision = gitcomet_core::services::apply_change_revision(&target.source);
    let single = target.paths.len() == 1;
    let listed: Vec<String> = target
        .paths
        .iter()
        .take(if target.paths.len() > LISTED_PATHS + 1 {
            LISTED_PATHS
        } else {
            target.paths.len()
        })
        .map(|path| path.display().to_string())
        .collect();
    let unlisted = target.paths.len() - listed.len();

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
            commit_retry: None,
        });
        // The selection was for this action; it has gone ahead.
        if target.paths.len() > 1 {
            this.details_pane.update(cx, |pane, cx| {
                pane.clear_commit_list_selections(repo_id);
                cx.notify();
            });
        }
        this.close_popover_and_restore_focus(window, cx);
    };

    let mut dialog = if single {
        ConfirmDialog::new("Commit applied change?", DIALOG_380_WIDTH).text(
            theme,
            format!("Apply the change to this file from {revision} to the current branch?"),
        )
    } else {
        ConfirmDialog::new("Commit applied changes?", DIALOG_380_WIDTH).text(
            theme,
            format!(
                "Apply the changes to these {} files from {revision} to the current branch?",
                listed.len() + unlisted
            ),
        )
    };
    for path in listed {
        dialog = dialog.mono_value(theme, path);
    }
    if unlisted > 0 {
        dialog = dialog.note(theme, format!("and {unlisted} more"));
    }
    dialog
        .note(
            theme,
            if single {
                "Commit the applied change immediately?"
            } else {
                "Commit the applied changes immediately?"
            },
        )
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
