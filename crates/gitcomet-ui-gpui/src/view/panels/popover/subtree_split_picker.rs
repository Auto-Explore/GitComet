use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;

    if let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) {
        match &repo.subtrees {
            Loadable::Loading => components::context_menu_label(theme, "Loading"),
            Loadable::NotLoaded => components::context_menu_label(theme, "Not loaded"),
            Loadable::Error(e) => components::context_menu_label(theme, e.clone()),
            Loadable::Ready(subtrees) => {
                let items = subtrees
                    .iter()
                    .map(|subtree| subtree.path.display().to_string().into())
                    .collect::<Vec<SharedString>>();
                let paths = subtrees
                    .iter()
                    .map(|subtree| subtree.path.clone())
                    .collect::<Vec<_>>();

                if let Some(search) = this.subtree_picker_search_input.clone() {
                    components::context_menu(
                        theme,
                        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                            .items(items)
                            .empty_text("No subtrees")
                            .max_height(px(260.0))
                            .render(theme, cx, move |this, ix, e, window, cx| {
                                let Some(path) = paths.get(ix).cloned() else {
                                    return;
                                };
                                this.open_popover_at(
                                    PopoverKind::subtree(
                                        repo_id,
                                        SubtreePopoverKind::SplitPrompt { path },
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
