use super::*;

impl SidebarPaneView {
    pub(in super::super) fn render_branch_sidebar_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        const BRANCH_TREE_BASE_PAD_PX: f32 = 8.0;
        const BRANCH_TREE_DEPTH_STEP_PX: f32 = 10.0;
        const BRANCH_TREE_TOGGLE_SLOT_PX: f32 = 12.0;
        const BRANCH_TREE_ICON_SLOT_PX: f32 = 16.0;
        const BRANCH_TREE_GAP_PX: f32 = 6.0;
        const CONTEXT_MENU_INDICATOR_SIZE_PX: f32 = 18.0;
        const CONTEXT_MENU_INDICATOR_RIGHT_PX: f32 = 6.0;

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

        let svg_icon = |path: &'static str, color: gpui::Rgba, size_px: f32| {
            super::super::icons::svg_icon(path, color, px(size_px))
        };
        let svg_spinner = |id: (&'static str, u64), color: gpui::Rgba, size_px: f32| {
            super::super::icons::svg_spinner(id, color, px(size_px))
        };
        let svg_collapse = |collapsed: bool| {
            svg_icon(
                if collapsed {
                    "icons/arrow_right.svg"
                } else {
                    "icons/chevron_down.svg"
                },
                icon_muted,
                10.0,
            )
        };
        let tree_toggle_slot = |collapsed: Option<bool>| {
            div()
                .w(px(BRANCH_TREE_TOGGLE_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .when_some(collapsed, |this, collapsed| {
                    this.child(svg_collapse(collapsed))
                })
        };
        let tree_icon_slot = |path: &'static str, color: gpui::Rgba, size_px: f32| {
            div()
                .w(px(BRANCH_TREE_ICON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(path, color, size_px))
        };
        let branch_tree_color = |section: BranchSection| match section {
            BranchSection::Local => theme.colors.text,
            BranchSection::Remote => theme.colors.text_muted,
        };
        let mix_color = |a: gpui::Rgba, b: gpui::Rgba, t: f32| {
            let t = t.clamp(0.0, 1.0);
            gpui::Rgba {
                r: a.r + (b.r - a.r) * t,
                g: a.g + (b.g - a.g) * t,
                b: a.b + (b.b - a.b) * t,
                a: a.a + (b.a - a.a) * t,
            }
        };
        let context_menu_indicator_icon =
            with_alpha(theme.colors.text, if theme.is_dark { 0.82 } else { 0.70 });
        let context_menu_indicator_bg = mix_color(
            theme.colors.window_bg,
            theme.colors.surface_bg,
            if theme.is_dark { 0.64 } else { 0.52 },
        );
        let context_menu_indicator_hover_bg = mix_color(
            context_menu_indicator_bg,
            theme.colors.active_section,
            if theme.is_dark { 0.60 } else { 0.42 },
        );
        let context_menu_indicator_active_bg = mix_color(
            theme.colors.surface_bg,
            theme.colors.accent,
            if theme.is_dark { 0.34 } else { 0.20 },
        );
        let context_menu_indicator_border = mix_color(
            theme.colors.window_bg,
            theme.colors.border,
            if theme.is_dark { 0.92 } else { 0.86 },
        );
        let context_menu_indicator =
            |id: SharedString, row_group: SharedString, visible: bool, menu_active: bool| {
                div()
                    .id(id)
                    .absolute()
                    .right(px(CONTEXT_MENU_INDICATOR_RIGHT_PX))
                    .top_0()
                    .bottom_0()
                    .w(px(CONTEXT_MENU_INDICATOR_SIZE_PX))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(if menu_active {
                        context_menu_indicator_active_bg
                    } else {
                        context_menu_indicator_bg
                    })
                    .border_1()
                    .border_color(context_menu_indicator_border)
                    .cursor(CursorStyle::PointingHand)
                    .invisible()
                    .when(visible, |d| d.visible())
                    .group_hover(row_group, |d| d.visible())
                    .hover(move |s| {
                        if menu_active {
                            s.bg(context_menu_indicator_active_bg)
                        } else {
                            s.bg(context_menu_indicator_hover_bg)
                        }
                    })
                    .active(move |s| s.bg(context_menu_indicator_active_bg))
                    .child(svg_icon(
                        "icons/menu.svg",
                        context_menu_indicator_icon,
                        12.0,
                    ))
            };

        fn indent_px(depth: usize) -> Pixels {
            px(BRANCH_TREE_BASE_PAD_PX + depth as f32 * BRANCH_TREE_DEPTH_STEP_PX)
        }

        fn left_divider(color: gpui::Rgba, radius: Pixels) -> gpui::Div {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(px(2.0))
                .rounded_l(radius)
                .bg(color)
        }

        fn top_divider(color: gpui::Rgba) -> gpui::Div {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(1.0))
                .bg(color)
        }

        range
            .filter_map(|ix| rows.get(ix).cloned().map(|r| (ix, r)))
            .map(|(ix, row)| match row {
                BranchSidebarRow::SectionHeader {
                    section,
                    top_border,
                    collapsed,
                    collapse_key,
                } => {
                    let (icon_path, label): (&'static str, SharedString) = match section {
                        BranchSection::Local => ("icons/computer.svg", "Local Branches".into()),
                        BranchSection::Remote => ("icons/cloud.svg", "Remote branches".into()),
                    };
                    let tooltip = label.clone();
                    let section_key = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    let context_menu_invoker: SharedString =
                        format!("branch_section_menu_{}_{}", repo_id.0, section_key).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let row_group: SharedString =
                        format!("branch_section_row_{}_{}", repo_id.0, section_key).into();

                    div()
                        .id(("branch_section", ix))
                        .relative()
                        .h(px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr_2()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
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
                        .when(top_border, |d| d.child(top_divider(theme.colors.border)))
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot(icon_path, icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.text)
                                .child(label),
                        )
                        .child(
                            context_menu_indicator(
                                format!(
                                    "branch_section_menu_indicator_{}_{}",
                                    repo_id.0, section_key
                                )
                                .into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::BranchSectionMenu { repo_id, section },
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
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
                BranchSidebarRow::StashHeader {
                    top_border,
                    collapsed,
                    collapse_key,
                } => {
                    let show_stash_spinner = this.active_repo().is_some_and(|r| {
                        matches!(r.stashes, Loadable::Loading)
                            || (!collapsed && matches!(r.stashes, Loadable::NotLoaded))
                    });
                    let context_menu_invoker: SharedString =
                        format!("stash_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let row_group: SharedString = format!("stash_section_row_{}", repo_id.0).into();

                    div()
                        .id(("stash_section", ix))
                        .relative()
                        .h(px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr_2()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
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
                        .when(top_border, |d| d.child(top_divider(theme.colors.border)))
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/box.svg", icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.text)
                                .child("Stash"),
                        )
                        .when(show_stash_spinner, |d| {
                            d.child(
                                div()
                                    .debug_selector(move || format!("stash_spinner_{}", repo_id.0))
                                    .child(svg_spinner(
                                        ("stash_spinner", repo_id.0),
                                        icon_muted,
                                        12.0,
                                    )),
                            )
                        })
                        .child(
                            context_menu_indicator(
                                format!("stash_section_menu_indicator_{}", repo_id.0).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::StashPrompt,
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                            let text: SharedString =
                                "Stashes (Right-click or use the menu button for actions)".into();
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
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
                                    PopoverKind::StashPrompt,
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
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
                    created_at: _,
                } => {
                    let tooltip = tooltip.clone();
                    let stash_message_for_menu = message.as_ref().to_owned();
                    let context_menu_invoker: SharedString =
                        format!("stash_menu_{}_{}", repo_id.0, index).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let stash_message_for_right_click = stash_message_for_menu.clone();
                    let stash_message_for_indicator = stash_message_for_menu.clone();
                    let row_group: SharedString =
                        format!("stash_row_{}_{}", repo_id.0, index).into();

                    div()
                        .id(("stash_sidebar_row", index))
                        .relative()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr_2()
                        .h(px(24.0))
                        .w_full()
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
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot("icons/box.svg", icon_primary, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(message.clone()),
                        )
                        .child(
                            context_menu_indicator(
                                format!("stash_menu_indicator_{}_{}", repo_id.0, index).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::StashMenu {
                                            repo_id,
                                            index,
                                            message: stash_message_for_indicator.clone(),
                                        },
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() < 2 {
                                return;
                            }
                            this.store.dispatch(Msg::ApplyStash { repo_id, index });
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
                                    PopoverKind::StashMenu {
                                        repo_id,
                                        index,
                                        message: stash_message_for_right_click.clone(),
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
                BranchSidebarRow::WorktreesHeader {
                    top_border,
                    collapsed,
                    collapse_key,
                } => {
                    let show_worktrees_spinner = this.active_repo().is_some_and(|r| {
                        r.worktrees_in_flight > 0
                            || matches!(r.worktrees, Loadable::Loading)
                            || (!collapsed && matches!(r.worktrees, Loadable::NotLoaded))
                    });
                    let context_menu_invoker: SharedString =
                        format!("worktrees_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let row_group: SharedString =
                        format!("worktrees_section_row_{}", repo_id.0).into();

                    div()
                        .id(("worktrees_section", ix))
                        .relative()
                        .h(px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr_2()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
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
                        .when(top_border, |d| d.child(top_divider(theme.colors.border)))
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/folder.svg", icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
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
                        .child(
                            context_menu_indicator(
                                format!("worktrees_section_menu_indicator_{}", repo_id.0).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::worktree(
                                            repo_id,
                                            WorktreePopoverKind::SectionMenu,
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
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
                                    PopoverKind::worktree(
                                        repo_id,
                                        WorktreePopoverKind::SectionMenu,
                                    ),
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
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let path_for_indicator = path.clone();
                    let row_group: SharedString =
                        format!("worktree_row_{}_{}", repo_id.0, ix).into();

                    div()
                        .id(("worktree_item", ix))
                        .relative()
                        .h(px(22.0))
                        .w_full()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr_2()
                        .rounded(px(theme.radii.row))
                        .when(is_active, |d| {
                            d.bg(with_alpha(
                                theme.colors.accent,
                                if theme.is_dark { 0.18 } else { 0.12 },
                            ))
                            .child(left_divider(
                                with_alpha(theme.colors.accent, 0.90),
                                px(theme.radii.row),
                            ))
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
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot("icons/folder.svg", icon_primary, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(label),
                        )
                        .child(
                            context_menu_indicator(
                                format!("worktree_menu_indicator_{}_{}", repo_id.0, ix).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::worktree(
                                            repo_id,
                                            WorktreePopoverKind::Menu {
                                                path: path_for_indicator.clone(),
                                            },
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
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
                                    PopoverKind::worktree(
                                        repo_id,
                                        WorktreePopoverKind::Menu {
                                            path: path_for_menu.clone(),
                                        },
                                    ),
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
                BranchSidebarRow::SubmodulesHeader {
                    top_border,
                    collapsed,
                    collapse_key,
                } => {
                    let show_submodules_spinner = this.active_repo().is_some_and(|r| {
                        matches!(r.submodules, Loadable::Loading)
                            || (!collapsed && matches!(r.submodules, Loadable::NotLoaded))
                    });
                    let context_menu_invoker: SharedString =
                        format!("submodules_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let row_group: SharedString =
                        format!("submodules_section_row_{}", repo_id.0).into();

                    div()
                        .id(("submodules_section", ix))
                        .relative()
                        .h(px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr_2()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
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
                        .when(top_border, |d| d.child(top_divider(theme.colors.border)))
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/box.svg", icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.text)
                                .child("Submodules"),
                        )
                        .when(show_submodules_spinner, |d| {
                            d.child(
                                div()
                                    .debug_selector(move || {
                                        format!("submodules_spinner_{}", repo_id.0)
                                    })
                                    .child(svg_spinner(
                                        ("submodules_spinner", repo_id.0),
                                        icon_muted,
                                        12.0,
                                    )),
                            )
                        })
                        .child(
                            context_menu_indicator(
                                format!("submodules_section_menu_indicator_{}", repo_id.0).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::submodule(
                                            repo_id,
                                            SubmodulePopoverKind::SectionMenu,
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
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
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
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
                                    PopoverKind::submodule(
                                        repo_id,
                                        SubmodulePopoverKind::SectionMenu,
                                    ),
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
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let path_for_indicator = path.clone();
                    let row_group: SharedString =
                        format!("submodule_row_{}_{}", repo_id.0, ix).into();

                    div()
                        .id(("submodule_item", ix))
                        .relative()
                        .h(px(22.0))
                        .w_full()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
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
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot("icons/box.svg", icon_primary, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(label),
                        )
                        .child(
                            context_menu_indicator(
                                format!("submodule_menu_indicator_{}_{}", repo_id.0, ix).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::submodule(
                                            repo_id,
                                            SubmodulePopoverKind::Menu {
                                                path: path_for_indicator.clone(),
                                            },
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
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
                                    PopoverKind::submodule(
                                        repo_id,
                                        SubmodulePopoverKind::Menu {
                                            path: path_for_menu.clone(),
                                        },
                                    ),
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
                BranchSidebarRow::RemoteHeader {
                    name,
                    collapsed,
                    collapse_key,
                } => {
                    let remote_color = branch_tree_color(BranchSection::Remote);
                    let remote_name: String = name.as_ref().to_owned();
                    let context_menu_invoker: SharedString =
                        format!("remote_menu_{}_{}", repo_id.0, remote_name).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let remote_name_for_right_click: String = name.as_ref().to_owned();
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let remote_name_for_indicator: String = name.as_ref().to_owned();
                    let row_group: SharedString =
                        format!("remote_header_row_{}_{}", repo_id.0, remote_name).into();

                    div()
                        .id(("branch_remote", ix))
                        .relative()
                        .h(px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr_2()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .rounded(px(theme.radii.row))
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
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(remote_color)
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/folder.svg", remote_color, 14.0))
                        .child(div().flex_1().min_w(px(0.0)).line_clamp(1).child(name))
                        .child(
                            context_menu_indicator(
                                format!("remote_menu_indicator_{}_{}", repo_id.0, ix).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::remote(
                                            repo_id,
                                            RemotePopoverKind::Menu {
                                                name: remote_name_for_indicator.clone(),
                                            },
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
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
                                    PopoverKind::remote(
                                        repo_id,
                                        RemotePopoverKind::Menu {
                                            name: remote_name_for_right_click.clone(),
                                        },
                                    ),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::GroupHeader {
                    label,
                    section,
                    depth,
                    collapsed,
                    collapse_key,
                } => {
                    let group_text_color = branch_tree_color(section);
                    let group_icon_color = match section {
                        BranchSection::Local => icon_primary,
                        BranchSection::Remote => theme.colors.text_muted,
                    };
                    div()
                        .id(("branch_group", ix))
                        .h(px(22.0))
                        .w_full()
                        .pl(indent_px(depth))
                        .pr_2()
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .rounded(px(theme.radii.row))
                        .cursor(CursorStyle::PointingHand)
                        .hover(move |s| s.bg(theme.colors.hover))
                        .active(move |s| s.bg(theme.colors.active))
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(group_text_color)
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/folder.svg", group_icon_color, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(label),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .into_any_element()
                }
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
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let full_name_for_indicator: SharedString = name.clone();
                    let row_group: SharedString = format!("branch_row_{}_{}", repo_id.0, ix).into();
                    let branch_has_right_metadata = (is_upstream
                        && section == BranchSection::Remote)
                        || divergence_behind.is_some()
                        || divergence_ahead.is_some();
                    let branch_text_color = if muted {
                        theme.colors.text_muted
                    } else {
                        branch_tree_color(section)
                    };
                    let branch_icon_color = match section {
                        BranchSection::Local => {
                            if muted {
                                icon_muted
                            } else {
                                icon_primary
                            }
                        }
                        BranchSection::Remote => theme.colors.text_muted,
                    };
                    let mut row = div()
                        .id(("branch_item", ix))
                        .relative()
                        .h(if section == BranchSection::Local {
                            px(24.0)
                        } else {
                            px(22.0)
                        })
                        .w_full()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(depth))
                        .pr_2()
                        .rounded(px(theme.radii.row))
                        .when(is_head, |d| {
                            d.bg(with_alpha(
                                theme.colors.accent,
                                if theme.is_dark { 0.18 } else { 0.12 },
                            ))
                            .child(left_divider(
                                with_alpha(theme.colors.accent, 0.90),
                                px(theme.radii.row),
                            ))
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
                        .text_color(branch_text_color)
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot(
                            "icons/git_branch.svg",
                            branch_icon_color,
                            12.0,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(label),
                        );

                    let mut right = div().flex().items_center().gap_2().ml_auto().when(
                        branch_has_right_metadata,
                        |d| {
                            d.pr(px(CONTEXT_MENU_INDICATOR_SIZE_PX
                                + CONTEXT_MENU_INDICATOR_RIGHT_PX
                                + 4.0))
                        },
                    );

                    if is_upstream && section == BranchSection::Remote {
                        right = right.child(
                            div()
                                .px(px(3.0))
                                .py(px(0.0))
                                .rounded(px(2.0))
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

                    row = row.child(right);
                    row = row.child(
                        context_menu_indicator(
                            format!("branch_menu_indicator_{}_{}", repo_id.0, ix).into(),
                            row_group.clone(),
                            context_menu_active,
                            context_menu_active,
                        )
                        .on_click(cx.listener(
                            move |this, e: &ClickEvent, window, cx| {
                                if !e.standard_click() || e.click_count() != 1 {
                                    return;
                                }
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_indicator.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::BranchMenu {
                                        repo_id,
                                        section,
                                        name: full_name_for_indicator.as_ref().to_owned(),
                                    },
                                    e.position(),
                                    window,
                                    cx,
                                );
                            },
                        )),
                    );

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
        let Loadable::Ready(details) = &repo.history_state.commit_details else {
            return Vec::new();
        };

        let theme = this.theme;
        let repo_id = repo.id;

        range
            .filter_map(|ix| details.files.get(ix).map(|f| (ix, f)))
            .map(|(ix, f)| {
                let commit_id = details.id.clone();
                let (icon, color) = match f.kind {
                    FileStatusKind::Added => (Some("icons/plus.svg"), theme.colors.success),
                    FileStatusKind::Modified => (Some("icons/pencil.svg"), theme.colors.warning),
                    FileStatusKind::Deleted => (Some("icons/minus.svg"), theme.colors.danger),
                    FileStatusKind::Renamed => (Some("icons/swap.svg"), theme.colors.accent),
                    FileStatusKind::Untracked => (Some("icons/question.svg"), theme.colors.warning),
                    FileStatusKind::Conflicted => (Some("icons/warning.svg"), theme.colors.danger),
                };

                let path = f.path.clone();
                let context_menu_invoker: SharedString = format!(
                    "commit_file_menu_{}_{}_{}",
                    repo_id.0,
                    commit_id.as_ref(),
                    path.display()
                )
                .into();
                let context_menu_active =
                    this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                let selected = repo
                    .diff_state
                    .diff_target
                    .as_ref()
                    .is_some_and(|t| match t {
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
                                this.child(svg_icon(icon, color, px(14.0)))
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
