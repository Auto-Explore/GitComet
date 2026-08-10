use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    branch_ids: Vec<u64>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let names: Vec<SharedString> = branch_ids
        .iter()
        .filter_map(|id| {
            repo.and_then(|r| {
                r.virtual_branches
                    .iter()
                    .find(|branch| branch.id == *id)
                    .map(|branch| branch.name.clone().into())
            })
        })
        .collect();

    let list = if names.is_empty() {
        SharedString::from("(no branches)")
    } else {
        names.join("\n").into()
    };

    ConfirmDialog::new("Remove stale virtual branches?", DIALOG_420_WIDTH)
        .mono_value(theme, list)
        .text(
            theme,
            "These branches have no worktree changes for their assigned paths and hold no parked patches.",
        )
        .note(
            theme,
            "Only their assignment is removed — committed or discarded work is untouched. Branches with parked hunks are kept.",
        )
        .render(
            theme,
            cancel_button("prune_virtual_branches_cancel", "prune_virtual_branches_cancel_hint", theme)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.close_popover(cx);
                }),
            components::Button::new("prune_virtual_branches_go", "Remove")
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::PruneVirtualBranches {
                        repo_id,
                        branch_ids: branch_ids.clone(),
                    });
                    this.close_popover(cx);
                }),
            cx,
        )
}
