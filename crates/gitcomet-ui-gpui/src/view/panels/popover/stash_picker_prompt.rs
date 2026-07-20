use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    purpose: StashPickerPurpose,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    let title = match purpose {
        StashPickerPurpose::Pop => "Pop Stash",
        StashPickerPurpose::Apply => "Apply Stash",
        StashPickerPurpose::Drop => "Drop Stash",
    };

    let mut menu = div()
        .flex()
        .flex_col()
        .w(scaled_px(420.0))
        .child(popover_title(title))
        .child(div().border_t_1().border_color(theme.colors.border));

    if let Some(search) = this.stash_picker_search_input.clone() {
        match this.active_repo().and_then(|r| {
            if let Loadable::Ready(s) = &r.stashes {
                Some(s.clone())
            } else {
                None
            }
        }) {
            Some(stashes) => {
                let query = search.read_with(cx, |i, _| i.text().trim().to_ascii_lowercase());
                let filtered: Vec<(usize, SharedString)> = stashes
                    .iter()
                    .filter_map(|s| {
                        let label = s.message.to_string();
                        if query.is_empty() || label.to_ascii_lowercase().contains(&query) {
                            Some((s.index, label.into()))
                        } else {
                            None
                        }
                    })
                    .collect();
                let items: Vec<SharedString> = filtered.iter().map(|(_, l)| l.clone()).collect();
                let git_indices: Vec<usize> = filtered.iter().map(|(idx, _)| *idx).collect();
                let messages: Vec<String> = filtered.iter().map(|(_, m)| m.to_string()).collect();

                menu = menu.child(
                    components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                        .items(items)
                        .tooltip_host(this.tooltip_host.clone())
                        .empty_text("No stashes")
                        .max_height(scaled_px(240.0))
                        .selected_index(this.stash_picker_prompt_selected_index)
                        .render(
                            theme,
                            ui_scale_percent,
                            cx,
                            move |this, ix, _e, window, cx| {
                                if let Some(&git_index) = git_indices.get(ix) {
                                    match purpose {
                                        StashPickerPurpose::Pop => {
                                            this.store.dispatch(Msg::PopStash {
                                                repo_id,
                                                index: git_index,
                                            });
                                            this.store.dispatch(Msg::LoadStashes { repo_id });
                                            this.close_popover(cx);
                                        }
                                        StashPickerPurpose::Apply => {
                                            this.store.dispatch(Msg::ApplyStash {
                                                repo_id,
                                                index: git_index,
                                            });
                                            this.store.dispatch(Msg::LoadStashes { repo_id });
                                            this.close_popover(cx);
                                        }
                                        StashPickerPurpose::Drop => {
                                            let message =
                                                messages.get(ix).cloned().unwrap_or_default();
                                            this.open_popover_centered(
                                                PopoverKind::StashDropConfirm {
                                                    repo_id,
                                                    index: git_index,
                                                    message,
                                                },
                                                window,
                                                cx,
                                            );
                                        }
                                    }
                                }
                            },
                        ),
                );
            }
            None => {
                let is_loading = this
                    .active_repo()
                    .map(|r| matches!(&r.stashes, Loadable::Loading))
                    .unwrap_or(false);
                let text = if is_loading {
                    "Loading…"
                } else {
                    "No stashes"
                };
                menu = menu.child(components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    text,
                    Some(this.tooltip_host.clone()),
                    cx,
                ));
            }
        }
    }

    components::context_menu(theme, menu).w(scaled_px(420.0))
}
