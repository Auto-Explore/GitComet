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
        match &repo.worktrees {
            Loadable::Loading => components::context_menu_label(theme, ui_scale_percent, "Loading"),
            Loadable::NotLoaded => {
                components::context_menu_label(theme, ui_scale_percent, "Not loaded")
            }
            Loadable::Error(e) => {
                components::context_menu_label(theme, ui_scale_percent, e.clone())
            }
            Loadable::Ready(worktrees) => {
                let workdir = repo.spec.workdir.clone();
                let items = worktrees
                    .iter()
                    .filter(|w| w.path != workdir)
                    .map(|w| {
                        let label = if let Some(branch) = &w.branch {
                            format!("{branch}  {}", w.path.display())
                        } else if w.detached {
                            format!("(detached)  {}", w.path.display())
                        } else {
                            w.path.display().to_string()
                        };
                        label.into()
                    })
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
                            .max_height(scaled_px(260.0))
                            .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                                let Some(path) = paths.get(ix).cloned() else {
                                    return;
                                };
                                this.store.dispatch(Msg::OpenRepo(path));
                                this.popover = None;
                                this.popover_anchor = None;
                                cx.notify();
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
