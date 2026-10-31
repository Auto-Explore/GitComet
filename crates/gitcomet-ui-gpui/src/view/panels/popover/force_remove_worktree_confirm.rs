use super::*;
use gitcomet_core::error::WorktreeRemovalBlocker;
use gitcomet_state::model::ForceRemoveWorktreePrompt;

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: ForceRemoveWorktreePrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ForceRemoveWorktreePrompt {
        repo_id,
        path,
        delete_branch,
        blocker,
        ..
    } = prompt;
    let header: SharedString = if delete_branch.is_some() {
        "Delete worktree and branch anyway?".into()
    } else {
        "Delete worktree anyway?".into()
    };
    let reason = match blocker {
        WorktreeRemovalBlocker::Changes => "This worktree has modified or untracked files.",
        WorktreeRemovalBlocker::Submodules => {
            "This worktree has initialized submodules, so Git removes it only when forced. Uncommitted changes and any submodule commits that exist only in this worktree are lost."
        }
    };
    let description: SharedString = match delete_branch.as_ref() {
        Some(branch) => format!(
            "{reason} {} will force-remove it, then delete the local branch '{branch}'.",
            crate::view::product_name()
        )
        .into(),
        None => reason.into(),
    };

    ConfirmDialog::new(header, DIALOG_460_WIDTH)
        .text(theme, description)
        .mono_value(theme, path.display().to_string())
        .command(
            theme,
            format!("git worktree remove --force {}", path.display()),
        )
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
                    this.store.dispatch(Msg::ForceRemoveWorktree {
                        repo_id,
                        path: path.clone(),
                        delete_branch: delete_branch.clone(),
                    });
                    this.close_resolved_popover(cx);
                }),
            cx,
        )
}
