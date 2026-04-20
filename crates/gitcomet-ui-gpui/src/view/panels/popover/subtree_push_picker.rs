use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    if let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) {
        match &repo.subtrees {
            Loadable::Loading => components::context_menu_label(theme, ui_scale_percent, "Loading"),
            Loadable::NotLoaded => {
                components::context_menu_label(theme, ui_scale_percent, "Not loaded")
            }
            Loadable::Error(e) => {
                components::context_menu_label(theme, ui_scale_percent, e.clone())
            }
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
                            .max_height(scaled_px(260.0))
                            .render(theme, ui_scale_percent, cx, move |this, ix, e, window, cx| {
                                let Some(path) = paths.get(ix).cloned() else {
                                    return;
                                };
                                this.open_popover_at(
                                    PopoverKind::subtree(
                                        repo_id,
                                        SubtreePopoverKind::PushPrompt { path },
                                    ),
                                    e.position(),
                                    window,
                                    cx,
                                );
                            }),
                    )
                    .w(scaled_px(520.0))
                    .max_w(scaled_px(820.0))
                } else {
                    components::context_menu_label(
                        theme,
                        ui_scale_percent,
                        "Search input not initialized",
                    )
                }
            }
        }
    } else {
        components::context_menu_label(theme, ui_scale_percent, "No repository")
    }
}
