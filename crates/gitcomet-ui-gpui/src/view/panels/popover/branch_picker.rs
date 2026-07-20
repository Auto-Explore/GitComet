use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let width = super::PICKER_WIDTH;
    let is_delete = matches!(
        this.popover,
        Some(PopoverKind::BranchPicker {
            purpose: BranchPickerPurpose::Delete
        })
    );
    let title = if is_delete {
        "Delete Branch"
    } else {
        "Checkout Branch"
    };

    let mut menu = div()
        .flex()
        .flex_col()
        .min_w(width.min_px(ui_scale))
        .max_w(width.max_px(ui_scale))
        .child(popover_title(title))
        .child(div().border_t_1().border_color(theme.colors.border));

    if let Some(repo) = this.active_repo() {
        match &repo.branches {
            Loadable::Ready(branches) => {
                if let Some(search) = this.branch_picker_search_input.clone() {
                    let repo_id = repo.id;
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
                    let checked_out_index = head_branch
                        .and_then(|head| branch_names.iter().position(|name| name == head));

                    menu = menu.child(
                        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                            .items(items)
                            .tooltip_host(this.tooltip_host.clone())
                            .empty_text("No branches")
                            .max_height(scaled_px(240.0))
                            .selected_index(this.branch_picker_selected_index)
                            .marked_index(checked_out_index)
                            .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                                if let Some(name) = branch_names.get(ix).cloned() {
                                    this.handle_inline_branch_picker_select(name, repo_id, cx);
                                }
                            }),
                    );
                } else {
                    for (ix, branch) in branches.iter().enumerate() {
                        let repo_id = repo.id;
                        let name = branch.name.clone();
                        let label: SharedString = name.clone().into();
                        menu = menu.child(
                            components::ContextMenuEntry::new(
                                ("branch_item", ix),
                                components::ContextMenuText::new(label)
                                    .max_lines(1)
                                    .tooltip_mode(
                                        components::TruncatedTextTooltipMode::FullTextIfTruncated,
                                    ),
                            )
                            .tooltip_host(this.tooltip_host.clone())
                            .render(theme, ui_scale_percent, cx)
                            .on_click(cx.listener(
                                move |this, _e: &ClickEvent, _w, cx| {
                                    this.handle_inline_branch_picker_select(
                                        name.clone(),
                                        repo_id,
                                        cx,
                                    );
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

    // Fixed width: PickerPrompt rows size with `w_full`, which does not
    // stretch under fit-content parents.
    components::context_menu(theme, menu).w(width.preferred_px(ui_scale))
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
