use super::*;

fn text_field(
    theme: AppTheme,
    label: &'static str,
    input: Entity<components::TextInput>,
) -> gpui::Div {
    div()
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(label),
        )
        .child(div().px_2().pb_1().w_full().min_w(px(0.0)).child(input))
}

fn checkbox_toggle(
    theme: AppTheme,
    id: &'static str,
    label: &'static str,
    enabled: bool,
    accent: gpui::Rgba,
) -> gpui::Stateful<gpui::Div> {
    let border = if enabled { accent } else { theme.colors.border };
    let background = if enabled {
        with_alpha(accent, if theme.is_dark { 0.18 } else { 0.12 })
    } else {
        gpui::rgba(0x00000000)
    };

    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .w_full()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(theme.radii.row))
        .hover(move |this| this.bg(theme.colors.hover))
        .active(move |this| this.bg(theme.colors.active))
        .cursor(CursorStyle::PointingHand)
        .child(
            div()
                .size(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(border)
                .rounded(px(4.0))
                .bg(background)
                .when(enabled, |this| {
                    this.child(crate::view::icons::svg_icon(
                        "icons/check.svg",
                        accent,
                        px(10.0),
                    ))
                }),
        )
        .child(div().text_sm().child(label))
}

fn publish_remote_toggle(theme: AppTheme, enabled: bool) -> gpui::Stateful<gpui::Div> {
    checkbox_toggle(
        theme,
        "subtree_split_publish_remote_toggle",
        "Publish to remote",
        enabled,
        theme.colors.success,
    )
}

fn advanced_options_toggle(theme: AppTheme, expanded: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id("subtree_split_advanced_toggle")
        .debug_selector(|| "subtree_split_advanced_toggle".to_string())
        .w_full()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .hover(move |this| this.bg(theme.colors.hover))
        .active(move |this| this.bg(theme.colors.active))
        .child(div().text_sm().child("Advanced options"))
        .child(
            div()
                .font_family(crate::view::UI_MONOSPACE_FONT_FAMILY)
                .text_sm()
                .text_color(theme.colors.text_muted)
                .debug_selector(move || {
                    if expanded {
                        "subtree_split_advanced_indicator_expanded".to_string()
                    } else {
                        "subtree_split_advanced_indicator_collapsed".to_string()
                    }
                })
                .child(if expanded { "^" } else { "v" }),
        )
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    initial_path: Option<std::path::PathBuf>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;

    let advanced_enabled = this.subtree_split_advanced_enabled;
    let rejoin_enabled = this.subtree_split_rejoin_enabled;
    let ignore_joins_enabled = this.subtree_split_ignore_joins_enabled;
    let remote_enabled = this.subtree_split_remote_enabled;

    div()
        .flex()
        .flex_col()
        .w(px(760.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Split subtree"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Path (repo-relative)"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.subtree_path_input.clone()),
        )
        .child(
            div()
                .px_2()
                .pb_2()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(if initial_path.is_some() {
                    "Edit the repo-relative prefix passed to git subtree split --prefix. Leave destination blank to only materialize standalone history in the current repository."
                } else {
                    "Choose any repo-relative directory to pass to git subtree split --prefix. Leave destination blank to only materialize standalone history in the current repository."
                }),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Destination repo folder (optional)"),
        )
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
                        .child(this.subtree_split_destination_repo_input.clone()),
                )
                .child(
                    components::Button::new("subtree_split_destination_browse", "Browse")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |_this, _e, window, cx| {
                            cx.stop_propagation();
                            let view = cx.weak_entity();
                            let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                                files: false,
                                directories: true,
                                multiple: false,
                                prompt: Some("Select extracted repository folder".into()),
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
                                        this.subtree_split_destination_repo_input
                                            .update(cx, |input, cx| {
                                                input.set_text(path.display().to_string(), cx);
                                            });
                                        cx.notify();
                                    });
                                })
                                .detach();
                        }),
                ),
        )
        .child(text_field(
            theme,
            "Destination branch (optional)",
            this.subtree_split_destination_branch_input.clone(),
        ))
        .child(
            publish_remote_toggle(theme, remote_enabled).on_click(cx.listener(
                |this, _e: &ClickEvent, _w, cx| {
                    this.subtree_split_remote_enabled = !this.subtree_split_remote_enabled;
                    cx.notify();
                },
            )),
        )
        .when(remote_enabled, |this_div| {
            this_div.child(text_field(
                theme,
                "Publish remote URL (optional)",
                this.subtree_split_remote_repository_input.clone(),
            ))
        })
        .child(
            advanced_options_toggle(theme, advanced_enabled).on_click(cx.listener(
                |this, _e: &ClickEvent, _w, cx| {
                    this.subtree_split_advanced_enabled = !this.subtree_split_advanced_enabled;
                    cx.notify();
                },
            )),
        )
        .when(advanced_enabled, |this_div| {
            this_div
                .child(text_field(
                    theme,
                    "Source split branch (optional)",
                    this.subtree_split_branch_input.clone(),
                ))
                .child(text_field(
                    theme,
                    "Through ref/commit (optional)",
                    this.subtree_split_through_revision_input.clone(),
                ))
                .child(text_field(
                    theme,
                    "Annotate prefix (optional)",
                    this.subtree_split_annotate_input.clone(),
                ))
                .child(text_field(
                    theme,
                    "Onto revision (optional)",
                    this.subtree_split_onto_input.clone(),
                ))
                .child(
                    div()
                        .px_2()
                        .pt_1()
                        .pb_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            components::Button::new(
                                "subtree_split_toggle_rejoin",
                                if rejoin_enabled {
                                    "Rejoin enabled"
                                } else {
                                    "Rejoin disabled"
                                },
                            )
                            .style(if rejoin_enabled {
                                components::ButtonStyle::Filled
                            } else {
                                components::ButtonStyle::Outlined
                            })
                            .on_click(theme, cx, |this, _e, _w, cx| {
                                this.subtree_split_rejoin_enabled =
                                    !this.subtree_split_rejoin_enabled;
                                cx.notify();
                            }),
                        )
                        .child(
                            components::Button::new(
                                "subtree_split_toggle_ignore_joins",
                                if ignore_joins_enabled {
                                    "Ignore joins enabled"
                                } else {
                                    "Ignore joins disabled"
                                },
                            )
                            .style(if ignore_joins_enabled {
                                components::ButtonStyle::Filled
                            } else {
                                components::ButtonStyle::Outlined
                            })
                            .on_click(theme, cx, |this, _e, _w, cx| {
                                this.subtree_split_ignore_joins_enabled =
                                    !this.subtree_split_ignore_joins_enabled;
                                cx.notify();
                            }),
                        ),
                )
        })
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    components::Button::new("subtree_split_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.close_popover(cx);
                        }),
                )
                .child(
                    components::Button::new("subtree_split_go", "Split")
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            let path_text = this
                                .subtree_path_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let branch = this
                                .subtree_split_branch_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let through_revision = this
                                .subtree_split_through_revision_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let annotate = this
                                .subtree_split_annotate_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let onto = this
                                .subtree_split_onto_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let destination_repo = this
                                .subtree_split_destination_repo_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let destination_branch = this
                                .subtree_split_destination_branch_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let remote_repository = this
                                .subtree_split_remote_repository_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let remote_repository = if this.subtree_split_remote_enabled {
                                remote_repository
                            } else {
                                String::new()
                            };

                            if path_text.is_empty() {
                                this.push_toast(
                                    components::ToastKind::Error,
                                    "Subtree path is required".to_string(),
                                    cx,
                                );
                                return;
                            }

                            if !remote_repository.is_empty() && destination_repo.is_empty() {
                                this.push_toast(
                                    components::ToastKind::Error,
                                    "Choose a destination repo before publishing to a remote"
                                        .to_string(),
                                    cx,
                                );
                                return;
                            }

                            this.store.dispatch(Msg::ExtractSubtree {
                                repo_id,
                                path: std::path::PathBuf::from(path_text),
                                options: gitcomet_core::domain::SubtreeExtractOptions {
                                    split: gitcomet_core::domain::SubtreeSplitOptions {
                                        branch: (!branch.is_empty()).then_some(branch),
                                        through_revision: (!through_revision.is_empty())
                                            .then_some(through_revision),
                                        annotate: (!annotate.is_empty()).then_some(annotate),
                                        onto: (!onto.is_empty()).then_some(onto),
                                        rejoin: this.subtree_split_rejoin_enabled,
                                        ignore_joins: this.subtree_split_ignore_joins_enabled,
                                    },
                                    destination_repository: (!destination_repo.is_empty())
                                        .then(|| std::path::PathBuf::from(destination_repo)),
                                    destination_branch: (!destination_branch.is_empty())
                                        .then_some(destination_branch),
                                    remote_repository: (!remote_repository.is_empty())
                                        .then_some(remote_repository),
                                },
                            });
                            this.close_popover(cx);
                        }),
                ),
        )
}
