use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: std::path::PathBuf,
    branch: Option<String>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let remove_branch = branch.clone();
    let header: SharedString = if branch.is_some() {
        "Remove worktree and branch".into()
    } else {
        "Remove worktree".into()
    };

    let mut dialog = ConfirmDialog::new(header, DIALOG_420_WIDTH)
        .text(theme, path.display().to_string());
    if let Some(branch) = branch.as_ref() {
        dialog = dialog.divider(theme).text(
            theme,
            format!(
                "This will remove the worktree folder and delete the local branch '{branch}'."
            ),
        );
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
                if let Some(branch) = remove_branch.clone() {
                    let root_view = this.root_view.clone();
                    let _ = root_view.update(cx, |root, _cx| {
                        root.register_pending_worktree_branch_removal(
                            repo_id,
                            path.clone(),
                            branch,
                        );
                    });
                }
                this.store.dispatch(Msg::RemoveWorktree {
                    repo_id,
                    path: path.clone(),
                });
                this.close_popover(cx);
            }),
        cx,
    )
}
