use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let mut menu = div()
        .flex()
        .flex_col()
        .min_w(scaled_px(420.0))
        .max_w(scaled_px(820.0));

    if let Some(repo) = this.active_repo() {
        match &repo.branches {
            Loadable::Ready(branches) => {
                if let Some(search) = this.branch_picker_search_input.clone() {
                    let repo_id = repo.id;
                    let branch_names = branches.iter().map(|b| b.name.clone()).collect::<Vec<_>>();
                    let items = branch_names
                        .iter()
                        .map(|name| name.clone().into())
                        .collect::<Vec<SharedString>>();

                    menu = menu.child(
                        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                            .items(items)
                            .tooltip_host(this.tooltip_host.clone())
                            .empty_text("No branches")
                            .max_height(scaled_px(240.0))
                            .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                                if let Some(name) = branch_names.get(ix).cloned() {
                                    this.store.dispatch(Msg::CheckoutBranch { repo_id, name });
                                }
                                this.close_popover(cx);
                            }),
                    );
                } else {
                    for (ix, branch) in branches.iter().enumerate() {
                        let repo_id = repo.id;
                        let name = branch.name.clone();
                        let label: SharedString = name.clone().into();
                        menu = menu.child(
                            components::context_menu_entry(
                                ("branch_item", ix),
                                theme,
                                ui_scale_percent,
                                false,
                                false,
                                None,
                                label,
                                None,
                            )
                            .on_click(cx.listener(
                                move |this, _e: &ClickEvent, _w, cx| {
                                    this.store.dispatch(Msg::CheckoutBranch {
                                        repo_id,
                                        name: name.clone(),
                                    });
                                    this.close_popover(cx);
                                },
                            )),
                        );
                    }
                }
            }
            Loadable::Loading => {
                menu = menu.child(components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    "Loading",
                    Some(this.tooltip_host.clone()),
                    cx,
                ));
            }
            Loadable::Error(e) => {
                menu = menu.child(components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    e.clone(),
                    Some(this.tooltip_host.clone()),
                    cx,
                ));
            }
            Loadable::NotLoaded => {
                menu = menu.child(components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    "Not loaded",
                    Some(this.tooltip_host.clone()),
                    cx,
                ));
            }
        }
    }

    components::context_menu(theme, menu)
        .w(scaled_px(420.0))
        .max_w(scaled_px(820.0))
}
