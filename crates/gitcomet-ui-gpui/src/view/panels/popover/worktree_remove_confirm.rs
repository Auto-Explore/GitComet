use super::*;
use crate::kit::interaction as controls;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: std::path::PathBuf,
    branch: Option<String>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let delete_branch = this.worktree_remove_delete_branch;

    let mut dialog = ConfirmDialog::new("Remove worktree", DIALOG_420_WIDTH)
        .text(theme, path.display().to_string());
    if let Some(branch) = branch.as_ref() {
        dialog = dialog.divider(theme).section(
            div().px_1().pb_1().child(
                checkbox_row(
                    "worktree_remove_delete_branch",
                    "worktree_remove_delete_branch",
                    format!("Also delete local branch '{branch}'"),
                    delete_branch,
                    &this.worktree_remove_delete_branch_focus_handle,
                    theme,
                    cx,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _w, cx| {
                        this.worktree_remove_delete_branch = !this.worktree_remove_delete_branch;
                        cx.notify();
                    }),
                ),
            ),
        );
        if delete_branch {
            dialog = dialog.note(
                theme,
                "The branch is deleted after the worktree. An unmerged branch asks first.",
            );
        }
    }

    dialog.render(
        theme,
        dialog_cancel_button(
            "worktree_remove_cancel",
            "worktree_remove_cancel_hint",
            theme,
            cx,
        ),
        components::Button::new("worktree_remove_go", "Remove")
            .style(components::ButtonStyle::Danger)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                this.store.dispatch(Msg::RemoveWorktree {
                    repo_id,
                    path: path.clone(),
                    delete_branch: branch
                        .clone()
                        .filter(|_| this.worktree_remove_delete_branch),
                });
                this.close_popover(cx);
            }),
        cx,
    )
}
