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
        "Delete worktree and branch anyway?".into()
    } else {
        "Delete worktree anyway?".into()
    };
    let description: SharedString = match branch.as_ref() {
        Some(branch) => format!(
            "This worktree has modified or untracked files. GitComet will force-remove it, then delete the local branch '{branch}'."
        )
        .into(),
        None => "This worktree has modified or untracked files.".into(),
    };

    ConfirmDialog::new(header, DIALOG_460_WIDTH)
        .text(theme, description)
        .mono_value(theme, path.display().to_string())
        .command(theme, format!("git worktree remove --force {}", path.display()))
        .render(
            theme,
            dialog_cancel_button(
                "force_remove_worktree_cancel",
                "force_remove_worktree_cancel_hint",
                theme,
                cx,
            ),
            components::Button::new("force_remove_worktree_go", "Delete anyway")
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
                    this.store.dispatch(Msg::ForceRemoveWorktree {
                        repo_id,
                        path: path.clone(),
                    });
                    this.close_popover(cx);
                }),
            cx,
        )
}
