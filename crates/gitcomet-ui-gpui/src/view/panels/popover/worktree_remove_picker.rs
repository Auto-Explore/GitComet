use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;

    if let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) {
        match &repo.worktrees {
            Loadable::Loading => components::context_menu_label(theme, "Loading"),
            Loadable::NotLoaded => components::context_menu_label(theme, "Not loaded"),
            Loadable::Error(e) => components::context_menu_label(theme, e.clone()),
            Loadable::Ready(worktrees) => {
                let workdir = repo.spec.workdir.clone();
                let items = worktrees
                    .iter()
                    .filter(|w| w.path != workdir)
                    .map(|w| w.path.display().to_string().into())
                    .collect::<Vec<SharedString>>();
                let paths = worktrees
                    .iter()
                    .filter(|w| w.path != workdir)
                    .map(|w| w.path.clone())
                    .collect::<Vec<_>>();

                if let Some(search) = this.worktree_picker_search_input.clone() {
                    components::context_menu(
                        theme,
                        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                            .items(items)
                            .empty_text("No worktrees")
                            .max_height(px(260.0))
                            .render(theme, cx, move |this, ix, e, window, cx| {
                                let Some(path) = paths.get(ix).cloned() else {
                                    return;
                                };
                                this.open_popover_at(
                                    PopoverKind::worktree(
                                        repo_id,
                                        WorktreePopoverKind::RemoveConfirm { path, branch: None },
                                    ),
                                    e.position(),
                                    window,
                                    cx,
                                );
                            }),
                    )
                    .w(px(520.0))
                    .max_w(px(820.0))
                } else {
                    components::context_menu_label(theme, "Search input not initialized")
                }
            }
        }
    } else {
        components::context_menu_label(theme, "No repository")
    }
}
