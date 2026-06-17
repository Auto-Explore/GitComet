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
                    let is_delete = matches!(
                        this.popover,
                        Some(PopoverKind::BranchPicker {
                            purpose: BranchPickerPurpose::Delete
                        })
                    );
                    let head_branch = match &repo.head_branch {
                        Loadable::Ready(head) => Some(head.as_str()),
                        _ => None,
                    };
                    let branch_names = branches
                        .iter()
                        .filter_map(|b| {
                            if is_delete && head_branch == Some(b.name.as_str()) {
                                None
                            } else {
                                Some(b.name.clone())
                            }
                        })
                        .collect::<Vec<_>>();
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
                            .selected_index(this.branch_picker_selected_index)
                            .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                                if let Some(name) = branch_names.get(ix).cloned() {
                                    if is_delete {
                                        this.store.dispatch(Msg::DeleteBranch { repo_id, name });
                                    } else {
                                        this.store
                                            .dispatch(Msg::CheckoutBranch { repo_id, name });
                                    }
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
                                    let is_delete = matches!(
                                        this.popover,
                                        Some(PopoverKind::BranchPicker {
                                            purpose: BranchPickerPurpose::Delete
                                        })
                                    );
                                    if is_delete {
                                        this.store.dispatch(Msg::DeleteBranch {
                                            repo_id,
                                            name: name.clone(),
                                        });
                                    } else {
                                        this.store.dispatch(Msg::CheckoutBranch {
                                            repo_id,
                                            name: name.clone(),
                                        });
                                    }
                                    this.close_popover(cx);
                                },
                            )),
                        );
                    }
                }
            }
            Loadable::Loading => {
                menu = menu.child(branch_picker_status_panel(this, "Loading", cx));
            }
            Loadable::Error(e) => {
                menu = menu.child(branch_picker_status_panel(this, e.clone(), cx));
            }
            Loadable::NotLoaded => {
                menu = menu.child(branch_picker_status_panel(this, "Not loaded", cx));
            }
        }
    }

    components::context_menu(theme, menu)
        .w(scaled_px(420.0))
        .max_w(scaled_px(820.0))
}

fn branch_picker_status_panel(
    this: &mut PopoverHost,
    empty_text: impl Into<SharedString>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    if let Some(search) = this.branch_picker_search_input.clone() {
        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
            .items(Vec::<SharedString>::new())
            .tooltip_host(this.tooltip_host.clone())
            .empty_text(empty_text)
            .max_height(scaled_px(240.0))
            .selected_index(this.branch_picker_selected_index)
            .render(theme, ui_scale_percent, cx, |_, _, _, _, _| {})
    } else {
        components::context_menu_label(
            theme,
            ui_scale_percent,
            empty_text.into(),
            Some(this.tooltip_host.clone()),
            cx,
        )
    }
}
