use super::*;

impl SidebarPaneView {
    pub(in super::super) fn render_branch_sidebar_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repo_id) = this.active_repo_id() else {
            return Vec::new();
        };
        let Some(rows) = this.branch_sidebar_rows_cached() else {
            return Vec::new();
        };
        let repo_workdir = this.active_repo().map(|r| r.spec.workdir.clone());
        let theme = this.theme;
        let icon_primary = theme.colors.accent;
        let icon_muted = with_alpha(theme.colors.accent, if theme.is_dark { 0.72 } else { 0.82 });
        let show_worktrees_spinner = this
            .active_repo()
            .is_some_and(|r| r.worktrees_in_flight > 0 || matches!(r.worktrees, Loadable::Loading));

        let svg_icon = |path: &'static str, color: gpui::Rgba, size_px: f32| {
            super::super::icons::svg_icon(path, color, px(size_px))
        };
        let svg_spinner = |id: (&'static str, u64), color: gpui::Rgba, size_px: f32| {
            super::super::icons::svg_spinner(id, color, px(size_px))
        };

        fn indent_px(depth: usize) -> Pixels {
            px(6.0 + depth as f32 * 10.0)
        }

        range
            .filter_map(|ix| rows.get(ix).cloned().map(|r| (ix, r)))
            .map(|(ix, row)| match row {
                BranchSidebarRow::SectionHeader {
                    section,
                    top_border,
                } => {
                    let (icon_path, label) = match section {
                        BranchSection::Local => ("icons/computer.svg", "Local"),
                        BranchSection::Remote => ("icons/cloud.svg", "Remote"),
                    };
                    let tooltip: SharedString = match section {
                        BranchSection::Local => "Local branches".into(),
                        BranchSection::Remote => "Remote branches".into(),
                    };
                    let section_key = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    let context_menu_invoker: SharedString =
                        format!("branch_section_menu_{}_{}", repo_id.0, section_key).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();

                    div()
                        .id(("branch_section", ix))
                        .h(if section == BranchSection::Local {
                            px(26.0)
                        } else {
                            px(24.0)
                        })
                        .w_full()
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .bg(theme.colors.surface_bg_elevated)
                        .cursor(CursorStyle::PointingHand)
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .when(top_border, |d| {
                            d.border_t_1().border_color(theme.colors.border)
                        })
                        .child(svg_icon(icon_path, icon_primary, 14.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.text)
                                .child(label),
                        )
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |=
                                    this.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            this.activate_context_menu_invoker(
                                context_menu_invoker_for_click.clone(),
                                cx,
                            );
                            this.open_popover_at(
                                PopoverKind::BranchSectionMenu { repo_id, section },
                                e.position(),
                                window,
                                cx,
                            );
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::BranchSectionMenu { repo_id, section },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::SectionSpacer => div()
                    .id(("branch_section_spacer", ix))
                    .h(px(10.0))
                    .w_full()
                    .into_any_element(),
                BranchSidebarRow::StashHeader { top_border } => div()
                    .id(("stash_section", ix))
                    .h(px(24.0))
                    .w_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .bg(theme.colors.surface_bg_elevated)
                    .when(top_border, |d| {
                        d.border_t_1().border_color(theme.colors.border)
                    })
                    .child(svg_icon("icons/box.svg", icon_primary, 14.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.colors.text)
                            .child("Stash"),
                    )
                    .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                        let text: SharedString = "Stashes (Apply / Drop)".into();
                        let mut changed = false;
                        if *hovering {
                            changed |= this.set_tooltip_text_if_changed(Some(text), cx);
                        } else {
                            changed |= this.clear_tooltip_if_matches(&text, cx);
                        }
                        if changed {
                            cx.notify();
                        }
                    }))
                    .into_any_element(),
                BranchSidebarRow::StashPlaceholder { message } => div()
                    .id(("stash_placeholder", ix))
                    .h(px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::StashItem {
                    index,
                    message,
                    tooltip,
                    row_group,
                    apply_button_id,
                    pop_button_id,
                    drop_button_id,
                    created_at: _,
                } => {
                    let tooltip = tooltip.clone();
                    let row_group = row_group.clone();
                    let apply_button_id = apply_button_id.clone();
                    let pop_button_id = pop_button_id.clone();
                    let drop_button_id = drop_button_id.clone();

                    let apply_tooltip: SharedString = "Apply stash".into();
                    let apply_button = zed::Button::new(apply_button_id, "Apply")
                        .style(zed::ButtonStyle::Solid)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::ApplyStash { repo_id, index });
                            cx.notify();
                        })
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |= this
                                    .set_tooltip_text_if_changed(Some(apply_tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&apply_tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }));

                    let pop_tooltip: SharedString = "Pop stash".into();
                    let pop_button = zed::Button::new(pop_button_id, "Pop")
                        .style(zed::ButtonStyle::Solid)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::PopStash { repo_id, index });
                            cx.notify();
                        })
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |=
                                    this.set_tooltip_text_if_changed(Some(pop_tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&pop_tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }));

                    let drop_tooltip: SharedString = "Drop stash".into();
                    let drop_button = zed::Button::new(drop_button_id, "Drop")
                        .style(zed::ButtonStyle::DangerSolid)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::DropStash { repo_id, index });
                            cx.notify();
                        })
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |= this
                                    .set_tooltip_text_if_changed(Some(drop_tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&drop_tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }));

                    div()
                        .id(("stash_sidebar_row", index))
                        .relative()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .h(px(24.0))
                        .w_full()
                        .hover(move |s| s.bg(theme.colors.hover))
                        .active(move |s| s.bg(theme.colors.active))
                        .child(svg_icon("icons/box.svg", icon_primary, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .pr(px(160.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(message.clone()),
                        )
                        .child(
                            div()
                                .absolute()
                                .right(px(6.0))
                                .top(px(2.0))
                                .bottom(px(2.0))
                                .flex()
                                .items_center()
                                .gap_2()
                                .invisible()
                                .group_hover(row_group.clone(), |d| d.visible())
                                .child(apply_button)
                                .child(pop_button)
                                .child(drop_button),
                        )
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |=
                                    this.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .into_any_element()
                }
                BranchSidebarRow::Placeholder {
                    section: _,
                    message,
                } => div()
                    .id(("branch_placeholder", ix))
                    .h(px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::WorktreesHeader { top_border } => {
                    let context_menu_invoker: SharedString =
                        format!("worktrees_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();

                    div()
                        .id(("worktrees_section", ix))
                        .h(px(24.0))
                        .w_full()
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .bg(theme.colors.surface_bg_elevated)
                        .cursor(CursorStyle::PointingHand)
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .when(top_border, |d| {
                            d.border_t_1().border_color(theme.colors.border)
                        })
                        .child(svg_icon("icons/folder.svg", icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.text)
                                .child("Worktrees"),
                        )
                        .when(show_worktrees_spinner, |d| {
                            d.child(
                                div()
                                    .debug_selector(move || {
                                        format!("worktrees_spinner_{}", repo_id.0)
                                    })
                                    .child(svg_spinner(
                                        ("worktrees_spinner", repo_id.0),
                                        icon_muted,
                                        12.0,
                                    )),
                            )
                        })
                        .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                            let text: SharedString =
                                "Worktrees (Add / Refresh / Open / Remove)".into();
                            let mut changed = false;
                            if *hovering {
                                changed |= this.set_tooltip_text_if_changed(Some(text), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&text, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            this.activate_context_menu_invoker(
                                context_menu_invoker_for_click.clone(),
                                cx,
                            );
                            this.open_popover_at(
                                PopoverKind::WorktreeSectionMenu { repo_id },
                                e.position(),
                                window,
                                cx,
                            );
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::WorktreeSectionMenu { repo_id },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::WorktreePlaceholder { message } => div()
                    .id(("worktree_placeholder", ix))
                    .h(px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::WorktreeItem {
                    path,
                    label,
                    tooltip,
                    is_active,
                } => {
                    let tooltip = tooltip.clone();
                    let label = label.clone();
                    let path_for_open = path.clone();
                    let path_for_menu = path.clone();
                    let context_menu_invoker: SharedString =
                        format!("worktree_menu_{}_{}", repo_id.0, path.display()).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();

                    div()
                        .id(("worktree_item", ix))
                        .h(px(22.0))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl(indent_px(1))
                        .pr_2()
                        .rounded(px(theme.radii.row))
                        .when(is_active, |d| {
                            d.bg(with_alpha(
                                theme.colors.accent,
                                if theme.is_dark { 0.18 } else { 0.12 },
                            ))
                            .border_1()
                            .border_color(with_alpha(theme.colors.accent, 0.90))
                        })
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .child(svg_icon("icons/folder.svg", icon_primary, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(label),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() < 2 {
                                return;
                            }
                            this.store.dispatch(Msg::OpenRepo(path_for_open.clone()));
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::WorktreeMenu {
                                        repo_id,
                                        path: path_for_menu.clone(),
                                    },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |=
                                    this.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .into_any_element()
                }
                BranchSidebarRow::SubmodulesHeader { top_border } => {
                    let context_menu_invoker: SharedString =
                        format!("submodules_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();

                    div()
                        .id(("submodules_section", ix))
                        .h(px(24.0))
                        .w_full()
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .bg(theme.colors.surface_bg_elevated)
                        .cursor(CursorStyle::PointingHand)
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .when(top_border, |d| {
                            d.border_t_1().border_color(theme.colors.border)
                        })
                        .child(svg_icon("icons/box.svg", icon_primary, 14.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.text)
                                .child("Submodules"),
                        )
                        .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                            let text: SharedString =
                                "Submodules (Add / Update / Open / Remove)".into();
                            let mut changed = false;
                            if *hovering {
                                changed |= this.set_tooltip_text_if_changed(Some(text), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&text, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            this.activate_context_menu_invoker(
                                context_menu_invoker_for_click.clone(),
                                cx,
                            );
                            this.open_popover_at(
                                PopoverKind::SubmoduleSectionMenu { repo_id },
                                e.position(),
                                window,
                                cx,
                            );
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::SubmoduleSectionMenu { repo_id },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::SubmodulePlaceholder { message } => div()
                    .id(("submodule_placeholder", ix))
                    .h(px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::SubmoduleItem {
                    path,
                    label,
                    tooltip,
                } => {
                    let tooltip = tooltip.clone();
                    let label = label.clone();
                    let path_for_open = path.clone();
                    let path_for_menu = path.clone();
                    let repo_workdir_for_open = repo_workdir.clone();
                    let context_menu_invoker: SharedString =
                        format!("submodule_menu_{}_{}", repo_id.0, path.display()).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();

                    div()
                        .id(("submodule_item", ix))
                        .h(px(22.0))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl(indent_px(1))
                        .pr_2()
                        .rounded(px(theme.radii.row))
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .child(svg_icon("icons/box.svg", icon_primary, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(label),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() < 2 {
                                return;
                            }
                            let Some(base) = repo_workdir_for_open.clone() else {
                                return;
                            };
                            this.store
                                .dispatch(Msg::OpenRepo(base.join(&path_for_open)));
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::SubmoduleMenu {
                                        repo_id,
                                        path: path_for_menu.clone(),
                                    },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |=
                                    this.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .into_any_element()
                }
                BranchSidebarRow::RemoteHeader { name } => {
                    let remote_name: String = name.as_ref().to_owned();
                    let context_menu_invoker: SharedString =
                        format!("remote_menu_{}_{}", repo_id.0, remote_name).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let row_group: SharedString = format!("branch_remote_row_{ix}").into();
                    let menu_button_id: SharedString = format!("branch_remote_menu_{ix}").into();
                    let remote_name_for_button: String = name.as_ref().to_owned();
                    let context_menu_invoker_for_button = context_menu_invoker.clone();
                    let menu_button = zed::Button::new(menu_button_id, "⋯")
                        .style(zed::ButtonStyle::Transparent)
                        .on_click(theme, cx, move |this, e, window, cx| {
                            cx.stop_propagation();
                            this.activate_context_menu_invoker(
                                context_menu_invoker_for_button.clone(),
                                cx,
                            );
                            this.open_popover_at(
                                PopoverKind::RemoteMenu {
                                    repo_id,
                                    name: remote_name_for_button.clone(),
                                },
                                e.position(),
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    let remote_name_for_right_click: String = name.as_ref().to_owned();
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();

                    div()
                        .id(("branch_remote", ix))
                        .relative()
                        .group(row_group.clone())
                        .h(px(24.0))
                        .w_full()
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(theme.radii.row))
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.colors.text)
                        .child(svg_icon("icons/folder.svg", icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .pr(px(28.0))
                                .line_clamp(1)
                                .child(name),
                        )
                        .child(
                            div()
                                .absolute()
                                .right(px(6.0))
                                .top(px(2.0))
                                .bottom(px(2.0))
                                .flex()
                                .items_center()
                                .invisible()
                                .group_hover(row_group, |d| d.visible())
                                .child(menu_button),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::RemoteMenu {
                                        repo_id,
                                        name: remote_name_for_right_click.clone(),
                                    },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::GroupHeader { label, depth } => div()
                    .id(("branch_group", ix))
                    .h(px(22.0))
                    .w_full()
                    .pl(indent_px(depth))
                    .pr_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.colors.text_muted)
                    .child(svg_icon("icons/folder.svg", icon_primary, 14.0))
                    .child(label)
                    .into_any_element(),
                BranchSidebarRow::Branch {
                    label,
                    name,
                    section,
                    depth,
                    muted,
                    divergence: _,
                    divergence_ahead,
                    divergence_behind,
                    tooltip,
                    is_head,
                    is_upstream,
                } => {
                    let full_name_for_checkout: SharedString = name.clone();
                    let full_name_for_menu: SharedString = name.clone();
                    let section_key = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    let context_menu_invoker: SharedString = format!(
                        "branch_menu_{}_{}_{}",
                        repo_id.0,
                        section_key,
                        full_name_for_menu.as_ref()
                    )
                    .into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let branch_icon_color = if muted { icon_muted } else { icon_primary };
                    let mut row = div()
                        .id(("branch_item", ix))
                        .h(if section == BranchSection::Local {
                            px(24.0)
                        } else {
                            px(22.0)
                        })
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl(indent_px(depth))
                        .pr_2()
                        .rounded(px(theme.radii.row))
                        .when(is_head, |d| {
                            d.bg(with_alpha(
                                theme.colors.accent,
                                if theme.is_dark { 0.18 } else { 0.12 },
                            ))
                            .border_1()
                            .border_color(with_alpha(theme.colors.accent, 0.90))
                        })
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .when(muted, |d| d.text_color(theme.colors.text_muted))
                        .child(svg_icon("icons/git_branch.svg", branch_icon_color, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(label),
                        );

                    let mut right = div().flex().items_center().gap_2().ml_auto();
                    let mut has_right = false;

                    if is_upstream && section == BranchSection::Remote {
                        has_right = true;
                        right = right.child(
                            div()
                                .px(px(3.0))
                                .py(px(0.0))
                                .rounded(px(999.0))
                                .text_size(px(11.0))
                                .text_color(theme.colors.text_muted)
                                .bg(with_alpha(
                                    theme.colors.accent,
                                    if theme.is_dark { 0.16 } else { 0.10 },
                                ))
                                .border_1()
                                .border_color(with_alpha(
                                    theme.colors.accent,
                                    if theme.is_dark { 0.32 } else { 0.22 },
                                ))
                                .child("Upstream"),
                        );
                    }

                    if divergence_behind.is_some() || divergence_ahead.is_some() {
                        has_right = true;
                        if let Some(behind) = divergence_behind.as_ref() {
                            let color = theme.colors.warning;
                            right = right.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(color)
                                    .child(svg_icon("icons/arrow_down.svg", color, 11.0))
                                    .child(behind.clone()),
                            );
                        }
                        if let Some(ahead) = divergence_ahead.as_ref() {
                            let color = theme.colors.success;
                            right = right.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(color)
                                    .child(svg_icon("icons/arrow_up.svg", color, 11.0))
                                    .child(ahead.clone()),
                            );
                        }
                    }

                    if has_right {
                        row = row.child(right);
                    }

                    let branch_tooltip: SharedString = tooltip.clone();

                    row = row
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if !e.standard_click() || e.click_count() < 2 {
                                return;
                            }
                            match section {
                                BranchSection::Local => {
                                    this.store.dispatch(Msg::CheckoutBranch {
                                        repo_id,
                                        name: full_name_for_checkout.as_ref().to_owned(),
                                    });
                                    this.rebuild_diff_cache(cx);
                                    cx.notify();
                                }
                                BranchSection::Remote => {
                                    if let Some((remote, branch)) =
                                        full_name_for_checkout.as_ref().split_once('/')
                                    {
                                        this.open_popover_at(
                                            PopoverKind::CheckoutRemoteBranchPrompt {
                                                repo_id,
                                                remote: remote.to_string(),
                                                branch: branch.to_string(),
                                            },
                                            e.position(),
                                            window,
                                            cx,
                                        );
                                        cx.notify();
                                    }
                                }
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::BranchMenu {
                                        repo_id,
                                        section,
                                        name: full_name_for_menu.as_ref().to_owned(),
                                    },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |= this
                                    .set_tooltip_text_if_changed(Some(branch_tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&branch_tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }));

                    row.into_any_element()
                }
            })
            .collect()
    }
}

impl DetailsPaneView {
    pub(in super::super) fn render_commit_file_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let Loadable::Ready(details) = &repo.commit_details else {
            return Vec::new();
        };

        let theme = this.theme;
        let repo_id = repo.id;

        range
            .filter_map(|ix| details.files.get(ix).map(|f| (ix, f)))
            .map(|(ix, f)| {
                let commit_id = details.id.clone();
                let (icon, color) = match f.kind {
                    FileStatusKind::Added => (Some("+"), theme.colors.success),
                    FileStatusKind::Modified => (Some("✎"), theme.colors.warning),
                    FileStatusKind::Deleted => (Some("−"), theme.colors.danger),
                    FileStatusKind::Renamed => (Some("→"), theme.colors.accent),
                    FileStatusKind::Untracked => (Some("?"), theme.colors.warning),
                    FileStatusKind::Conflicted => (Some("!"), theme.colors.danger),
                };

                let path = f.path.clone();
                let context_menu_invoker: SharedString = format!(
                    "commit_file_menu_{}_{}_{}",
                    repo_id.0,
                    commit_id.0.as_str(),
                    path.display()
                )
                .into();
                let context_menu_active =
                    this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                let selected = repo.diff_target.as_ref().is_some_and(|t| match t {
                    DiffTarget::Commit {
                        commit_id: t_commit_id,
                        path: Some(t_path),
                    } => t_commit_id == &commit_id && t_path == &path,
                    _ => false,
                });
                let commit_id_for_click = commit_id.clone();
                let path_for_click = path.clone();
                let commit_id_for_menu = commit_id.clone();
                let path_for_menu = path.clone();
                let path_label = this.cached_path_display(&path);
                let tooltip = path_label.clone();

                let mut row = div()
                    .id(("commit_file", ix))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .w_full()
                    .rounded(px(theme.radii.row))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| {
                        if context_menu_active {
                            s.bg(theme.colors.active)
                        } else {
                            s.bg(theme.colors.hover)
                        }
                    })
                    .active(move |s| s.bg(theme.colors.active))
                    .child(
                        div()
                            .w(px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when_some(icon, |this, icon| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(color)
                                        .child(icon),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(path_label.clone()),
                    )
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        this.store.dispatch(Msg::SelectDiff {
                            repo_id,
                            target: DiffTarget::Commit {
                                commit_id: commit_id_for_click.clone(),
                                path: Some(path_for_click.clone()),
                            },
                        });
                        cx.notify();
                    }))
                    .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                        let mut changed = false;
                        if *hovering {
                            changed |= this.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
                        } else {
                            changed |= this.clear_tooltip_if_matches(&tooltip, cx);
                        }
                        if changed {
                            cx.notify();
                        }
                    }));
                row = row.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.store.dispatch(Msg::SelectDiff {
                            repo_id,
                            target: DiffTarget::Commit {
                                commit_id: commit_id_for_menu.clone(),
                                path: Some(path_for_menu.clone()),
                            },
                        });
                        this.activate_context_menu_invoker(
                            context_menu_invoker_for_right_click.clone(),
                            cx,
                        );
                        this.open_popover_at(
                            PopoverKind::CommitFileMenu {
                                repo_id,
                                commit_id: commit_id_for_menu.clone(),
                                path: path_for_menu.clone(),
                            },
                            e.position,
                            window,
                            cx,
                        );
                        cx.notify();
                    }),
                );

                if selected {
                    row = row.bg(with_alpha(
                        theme.colors.accent,
                        if theme.is_dark { 0.16 } else { 0.10 },
                    ));
                }
                if context_menu_active {
                    row = row.bg(theme.colors.active);
                }

                row.into_any_element()
            })
            .collect()
    }
}
