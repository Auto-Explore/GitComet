use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_submit = this.can_submit_worktree_add(cx);
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    let ref_row = if let Some(search) = this.branch_picker_search_input.clone() {
        let is_focused = search
            .read_with(cx, |input, _| input.focus_handle())
            .is_focused(window);

        if is_focused {
            let branches: Vec<String> = this
                .active_repo()
                .map(|repo| {
                    let mut names: Vec<String> = vec!["HEAD".to_string()];
                    if let Loadable::Ready(branches) = &repo.branches {
                        names.extend(branches.iter().map(|b| b.name.clone()));
                    }
                    if let Loadable::Ready(tags) = &repo.tags {
                        names.extend(tags.iter().map(|t| t.name.clone()));
                    }
                    names
                })
                .unwrap_or_default();
            let items: Vec<SharedString> = branches.iter().map(|n| n.clone().into()).collect();

            div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                    .items(items)
                    .tooltip_host(this.tooltip_host.clone())
                    .empty_text("No matches")
                    .max_height(scaled_px(240.0))
                    .selected_index(this.branch_picker_selected_index)
                    .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                        let branches: Vec<String> = this
                            .active_repo()
                            .map(|repo| {
                                let mut names: Vec<String> = vec!["HEAD".to_string()];
                                if let Loadable::Ready(branches) = &repo.branches {
                                    names.extend(branches.iter().map(|b| b.name.clone()));
                                }
                                if let Loadable::Ready(tags) = &repo.tags {
                                    names.extend(tags.iter().map(|t| t.name.clone()));
                                }
                                names
                            })
                            .unwrap_or_default();
                        if let Some(name) = branches.get(ix).cloned() {
                            let repo_id = this.active_repo_id().unwrap_or(RepoId(0));
                            this.handle_inline_branch_picker_select(name, repo_id, cx);
                        }
                    }),
            )
        } else {
            div().px_2().pb_1().w_full().min_w(px(0.0)).child(search)
        }
    } else {
        div()
            .px_2()
            .pb_1()
            .w_full()
            .min_w(px(0.0))
            .child(this.worktree_ref_input.clone())
    };

    div()
        .flex()
        .flex_col()
        .w(scaled_px(640.0))
        .child(popover_title("Add worktree"))
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(input_label(theme, "Worktree folder"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(this.worktree_path_input.clone()),
                )
                .child(
                    components::Button::new("worktree_browse", "Browse")
                        .focus_handle(this.worktree_browse_focus_handle.clone())
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |_this, _e, window, cx| {
                            cx.stop_propagation();
                            let view = cx.weak_entity();
                            let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                                files: false,
                                directories: true,
                                multiple: false,
                                prompt: Some("Select worktree folder".into()),
                            });

                            window
                                .spawn(cx, async move |cx| {
                                    let result = rx.await;
                                    let paths = match result {
                                        Ok(Ok(Some(paths))) => paths,
                                        Ok(Ok(None)) => return,
                                        Ok(Err(_)) | Err(_) => return,
                                    };
                                    let Some(path) = paths.into_iter().next() else {
                                        return;
                                    };
                                    let _ = view.update(cx, |this, cx| {
                                        this.worktree_path_input.update(cx, |input, cx| {
                                            input.set_text(path.display().to_string(), cx);
                                        });
                                        cx.notify();
                                    });
                                })
                                .detach();
                        }),
                ),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Branch / commit (optional)"),
        )
        .child(ref_row)
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    cancel_button("worktree_add_cancel", "worktree_add_cancel_hint", theme)
                        .focus_handle(this.worktree_focus.cancel.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("worktree_add_go", "Add")
                        .focus_handle(this.worktree_focus.submit.clone())
                        .disabled(!can_submit)
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "worktree_add_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_worktree_add(cx);
                        }),
                ),
        )
}
