//! Header rows of the branch sidebar: the section headers (branches,
//! stashes, worktrees, submodules) and the remote and group folders.

use super::*;

impl SidebarPaneView {
    /// Local / Remote Branches header.
    pub(super) fn sidebar_section_header_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        section: BranchSection,
        collapsed: bool,
        collapse_key: SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot {
            ix,
            row_style,
            row_surface,
            ..
        } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let is_collapsed_popover = ctx.is_collapsed_popover;
        let (icon_path, label): (&'static str, SharedString) = match section {
            BranchSection::Local => ("icons/computer.svg", "Local Branches".into()),
            BranchSection::Remote => ("icons/cloud.svg", "Remote Branches".into()),
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
        let row_state = components::InteractiveRowState::default().open(context_menu_active);

        div()
            .id(collapse_key.clone())
            .relative()
            .h(ctx.row_height())
            .w_full()
            .pl(ctx.indent_px(0))
            .pr(ctx.trailing_pad())
            .flex()
            .items_center()
            .gap(ctx.px(BRANCH_TREE_GAP_PX))
            .interactive_row(row_style, row_state)
            .child(
                ctx.tree_toggle_slot(is_collapsed_popover.then_some(collapsed))
                    .debug_selector(move || format!("sidebar_header_toggle_{ix}")),
            )
            .child(
                ctx.tree_icon_slot(icon_path, ctx.icon_primary, 14.0)
                    .debug_selector(move || format!("sidebar_header_icon_{ix}")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.colors.foreground.primary)
                    .child(label),
            )
            .gitcomet_tooltip(theme, tooltip.clone())
            .on_activate(
                false,
                ctx.header_activation,
                cx.listener(move |this, e: &ClickEvent, _w, cx| {
                    if !e.standard_click() || e.click_count() != 1 {
                        return;
                    }
                    if is_collapsed_popover {
                        this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                    } else {
                        this.navigate_sidebar_row(ix, cx);
                    }
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        (PopoverKind::BranchSectionMenu { repo_id, section })
                            .invoked_by(context_menu_invoker_for_right_click.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .map(|row| ctx.paint_header(row.into_any_element(), row_surface))
            .into_any_element()
    }

    /// Stashes section header, with a spinner while the list loads.
    pub(super) fn sidebar_stash_header_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        collapsed: bool,
        collapse_key: SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot {
            ix,
            row_style,
            row_surface,
            ..
        } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let is_collapsed_popover = ctx.is_collapsed_popover;
        let show_stash_spinner = this.active_repo().is_some_and(|r| {
            matches!(r.stashes, Loadable::Loading)
                || (!collapsed && matches!(r.stashes, Loadable::NotLoaded))
        });
        let context_menu_invoker: SharedString = format!("stash_section_menu_{}", repo_id.0).into();
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let context_menu_invoker_for_right_click = context_menu_invoker.clone();
        let row_state = components::InteractiveRowState::default().open(context_menu_active);

        div()
            .id(collapse_key.clone())
            .debug_selector(move || format!("stash_section_{ix}"))
            .relative()
            .h(ctx.row_height())
            .w_full()
            .pl(ctx.indent_px(0))
            .pr(ctx.trailing_pad())
            .flex()
            .items_center()
            .gap(ctx.px(BRANCH_TREE_GAP_PX))
            .interactive_row(row_style, row_state)
            .child(ctx.tree_toggle_slot(is_collapsed_popover.then_some(collapsed)))
            .child(ctx.tree_icon_slot(STASH_ICON_PATH, ctx.icon_primary, 14.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.colors.foreground.primary)
                    .child("Stashes"),
            )
            .when(show_stash_spinner, |d| {
                d.child(
                    div()
                        .debug_selector(move || format!("stash_spinner_{}", repo_id.0))
                        .child(ctx.svg_spinner(("stash_spinner", repo_id.0), ctx.icon_muted, 12.0)),
                )
            })
            .gitcomet_tooltip(theme, "Stashes (Right-click for actions)".into())
            .on_activate(
                false,
                ctx.header_activation,
                cx.listener(move |this, e: &ClickEvent, _w, cx| {
                    if !e.standard_click() || e.click_count() != 1 {
                        return;
                    }
                    if is_collapsed_popover {
                        this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                    } else {
                        this.navigate_sidebar_row(ix, cx);
                    }
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        PopoverKind::StashPrompt
                            .invoked_by(context_menu_invoker_for_right_click.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .map(|row| ctx.paint_header(row.into_any_element(), row_surface))
            .into_any_element()
    }

    /// Worktrees section header, with a spinner while the list loads.
    pub(super) fn sidebar_worktrees_header_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        collapsed: bool,
        collapse_key: SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot {
            ix,
            row_style,
            row_surface,
            ..
        } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let is_collapsed_popover = ctx.is_collapsed_popover;
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
        let row_state = components::InteractiveRowState::default().open(context_menu_active);

        div()
            .id(collapse_key.clone())
            .debug_selector(move || format!("worktrees_section_{ix}"))
            .relative()
            .h(ctx.row_height())
            .w_full()
            .pl(ctx.indent_px(0))
            .pr(ctx.trailing_pad())
            .flex()
            .items_center()
            .gap(ctx.px(BRANCH_TREE_GAP_PX))
            .interactive_row(row_style, row_state)
            .child(ctx.tree_toggle_slot(is_collapsed_popover.then_some(collapsed)))
            .child(ctx.tree_icon_slot(WORKTREE_ICON_PATH, ctx.icon_primary, 14.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.colors.foreground.primary)
                    .child("Worktrees"),
            )
            .when(show_worktrees_spinner, |d| {
                d.child(
                    div()
                        .debug_selector(move || format!("worktrees_spinner_{}", repo_id.0))
                        .child(ctx.svg_spinner(
                            ("worktrees_spinner", repo_id.0),
                            ctx.icon_muted,
                            12.0,
                        )),
                )
            })
            .gitcomet_tooltip(theme, "Worktrees (Add / Refresh / Open / Remove)".into())
            .on_activate(
                false,
                ctx.header_activation,
                cx.listener(move |this, e: &ClickEvent, _w, cx| {
                    if !e.standard_click() || e.click_count() != 1 {
                        return;
                    }
                    if is_collapsed_popover {
                        this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                    } else {
                        this.navigate_sidebar_row(ix, cx);
                    }
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        (PopoverKind::worktree(repo_id, WorktreePopoverKind::SectionMenu))
                            .invoked_by(context_menu_invoker_for_right_click.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .map(|row| ctx.paint_header(row.into_any_element(), row_surface))
            .into_any_element()
    }

    /// Submodules section header, with a spinner while the list loads.
    pub(super) fn sidebar_submodules_header_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        collapsed: bool,
        collapse_key: SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot {
            ix,
            row_style,
            row_surface,
            ..
        } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let is_collapsed_popover = ctx.is_collapsed_popover;
        let show_submodules_spinner = this
            .active_repo()
            .is_some_and(|r| matches!(r.submodules, Loadable::Loading));
        let context_menu_invoker: SharedString =
            format!("submodules_section_menu_{}", repo_id.0).into();
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let context_menu_invoker_for_right_click = context_menu_invoker.clone();
        let row_state = components::InteractiveRowState::default().open(context_menu_active);

        div()
            .id(collapse_key.clone())
            .debug_selector(move || format!("submodules_section_{ix}"))
            .relative()
            .h(ctx.row_height())
            .w_full()
            .pl(ctx.indent_px(0))
            .pr(ctx.trailing_pad())
            .flex()
            .items_center()
            .gap(ctx.px(BRANCH_TREE_GAP_PX))
            .interactive_row(row_style, row_state)
            .child(ctx.tree_toggle_slot(is_collapsed_popover.then_some(collapsed)))
            .child(ctx.tree_icon_slot("icons/box.svg", ctx.icon_primary, 14.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.colors.foreground.primary)
                    .child("Submodules"),
            )
            .when(show_submodules_spinner, |d| {
                d.child(
                    div()
                        .debug_selector(move || format!("submodules_spinner_{}", repo_id.0))
                        .child(ctx.svg_spinner(
                            ("submodules_spinner", repo_id.0),
                            ctx.icon_muted,
                            12.0,
                        )),
                )
            })
            .gitcomet_tooltip(theme, "Submodules (Add / Update / Open / Remove)".into())
            .on_activate(
                false,
                ctx.header_activation,
                cx.listener(move |this, e: &ClickEvent, _w, cx| {
                    if !e.standard_click() || e.click_count() != 1 {
                        return;
                    }
                    if is_collapsed_popover {
                        this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                    } else {
                        this.navigate_sidebar_row(ix, cx);
                    }
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        (PopoverKind::submodule(repo_id, SubmodulePopoverKind::SectionMenu))
                            .invoked_by(context_menu_invoker_for_right_click.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .map(|row| ctx.paint_header(row.into_any_element(), row_surface))
            .into_any_element()
    }

    /// One remote's folder row in the Remote Branches section.
    pub(super) fn sidebar_remote_header_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        name: SharedString,
        collapsed: bool,
        collapse_key: SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot {
            ix,
            stuck,
            row_style,
            row_surface,
        } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let is_collapsed_popover = ctx.is_collapsed_popover;
        let remote_color = ctx.branch_tree_color(BranchSection::Remote);
        let remote_name: String = name.as_ref().to_owned();
        let context_menu_invoker: SharedString =
            format!("remote_menu_{}_{}", repo_id.0, remote_name).into();
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let remote_name_for_right_click: String = name.as_ref().to_owned();
        let context_menu_invoker_for_right_click = context_menu_invoker.clone();
        let row_group: SharedString =
            format!("remote_header_row_{}_{}", repo_id.0, remote_name).into();
        let row_state = components::InteractiveRowState::default().open(context_menu_active);

        div()
            .id(collapse_key.clone())
            .relative()
            .h(ctx.row_height())
            .w_full()
            .pl(ctx.indent_px(0))
            .pr(ctx.trailing_pad())
            .group(row_group.clone())
            .flex()
            .items_center()
            .gap(ctx.px(BRANCH_TREE_GAP_PX))
            .interactive_row(row_style, row_state)
            .text_size(theme.ui_text(14.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(remote_color)
            .child(if is_collapsed_popover {
                ctx.tree_toggle_slot(Some(collapsed)).into_any_element()
            } else {
                let key = collapse_key.clone();
                ctx.tree_toggle_slot(Some(collapsed))
                    .id(("sidebar_group_toggle", ix))
                    .debug_selector(move || format!("sidebar_group_toggle_{ix}"))
                    .h_full()
                    .on_activate(
                        false,
                        controls::ControlActivation::Nested,
                        cx.listener(move |this, _, _, cx| {
                            if stuck {
                                this.navigate_sidebar_row(ix, cx);
                            } else {
                                this.toggle_active_repo_collapse_key(key.clone(), cx);
                            }
                        }),
                    )
                    .into_any_element()
            })
            .child(ctx.tree_icon_slot(
                crate::view::file_icons::folder_icon(!collapsed),
                remote_color,
                14.0,
            ))
            .child(
                components::FadingText::new(
                    div().child(name),
                    row_style.resolved_background(row_state),
                )
                .hover_bg(
                    row_group.clone(),
                    row_style.resolved_hover_background(row_state),
                )
                .render(ctx.ui_scale_percent)
                .flex_1(),
            )
            .on_activate(
                false,
                ctx.header_activation,
                cx.listener(move |this, e: &ClickEvent, _w, cx| {
                    if !e.standard_click() || e.click_count() != 1 {
                        return;
                    }
                    if stuck {
                        this.navigate_sidebar_row(ix, cx);
                    } else {
                        this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                    }
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        (PopoverKind::remote(
                            repo_id,
                            RemotePopoverKind::Menu {
                                name: remote_name_for_right_click.clone(),
                            },
                        ))
                        .invoked_by(context_menu_invoker_for_right_click.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .map(|row| ctx.paint_header(row.into_any_element(), row_surface))
            .into_any_element()
    }

    /// A branch-name folder (`feat/`), in the tree or among the pins.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sidebar_group_header_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        label: SharedString,
        path: SharedString,
        remote: Option<SharedString>,
        section: BranchSection,
        depth: u16,
        collapsed: bool,
        collapse_key: SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot {
            ix,
            stuck,
            row_style,
            row_surface,
        } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let is_collapsed_popover = ctx.is_collapsed_popover;
        let from_pins = ix < ctx.pin_count;
        let pinned_root = from_pins && depth == 0;
        let prefix = if from_pins { "pinned_" } else { "" };
        let row_group: SharedString =
            format!("{prefix}branch_group_row_{}_{}", repo_id.0, ix).into();
        let section_key = match section {
            BranchSection::Local => "local",
            BranchSection::Remote => "remote",
        };
        let context_menu_invoker: SharedString = format!(
            "{prefix}branch_group_menu_{}_{}_{}_{}",
            repo_id.0,
            section_key,
            remote.as_deref().unwrap_or_default(),
            path
        )
        .into();
        let context_menu_invoker: SharedString = if from_pins {
            format!("{context_menu_invoker}_{ix}").into()
        } else {
            context_menu_invoker
        };
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let context_menu_invoker_for_right_click = context_menu_invoker.clone();
        let menu_kind = PopoverKind::BranchGroupMenu {
            repo_id,
            section,
            remote: remote.as_ref().map(|remote| remote.to_string()),
            path: path.to_string(),
        };
        let menu_kind_for_right_click = menu_kind.clone();
        let row_state = components::InteractiveRowState::default().open(context_menu_active);

        div()
            .id(("branch_group", ix))
            .debug_selector(move || format!("{prefix}branch_group_{ix}"))
            .h(ctx.row_height())
            .w_full()
            .pl(ctx.indent_px(usize::from(depth)))
            .pr(ctx.trailing_pad())
            .group(row_group.clone())
            .flex()
            .items_center()
            .gap(ctx.px(BRANCH_TREE_GAP_PX))
            .interactive_row(row_style, row_state)
            .text_size(theme.ui_text(12.0))
            .font_weight(FontWeight::NORMAL)
            .text_color(if from_pins {
                theme.colors.foreground.primary
            } else {
                theme.colors.foreground.secondary
            })
            .child(if is_collapsed_popover {
                ctx.tree_toggle_slot(Some(collapsed)).into_any_element()
            } else {
                let key = collapse_key.clone();
                ctx.tree_toggle_slot(Some(collapsed))
                    .id(("sidebar_group_toggle", ix))
                    .debug_selector(move || format!("{prefix}sidebar_group_toggle_{ix}"))
                    .h_full()
                    .on_activate(
                        false,
                        controls::ControlActivation::Nested,
                        cx.listener(move |this, _, _, cx| {
                            if stuck {
                                this.navigate_sidebar_row(ix, cx);
                            } else {
                                this.toggle_active_repo_collapse_key(key.clone(), cx);
                            }
                        }),
                    )
                    .into_any_element()
            })
            .child(
                ctx.tree_icon_slot(
                    if pinned_root {
                        "icons/pin.svg"
                    } else {
                        crate::view::file_icons::folder_icon(!collapsed)
                    },
                    ctx.icon_primary,
                    14.0,
                )
                .when(pinned_root, |icon| {
                    icon.debug_selector(move || format!("sidebar_group_pin_marker_{ix}"))
                }),
            )
            .child(
                components::FadingText::new(
                    filtered_label_element(
                        label,
                        Some(&remote.as_ref().map_or_else(
                            || format!("{path}/"),
                            |remote| format!("{remote}/{path}/"),
                        )),
                        &ctx.filter_query,
                        if from_pins {
                            theme.colors.foreground.primary
                        } else {
                            theme.colors.foreground.secondary
                        },
                        theme.colors.accent.foreground,
                        gpui::rems(0.75).into(),
                        FontWeight::NORMAL,
                        cx,
                    ),
                    row_style.resolved_background(row_state),
                )
                .hover_bg(
                    row_group.clone(),
                    row_style.resolved_hover_background(row_state),
                )
                .render(ctx.ui_scale_percent)
                .flex_1(),
            )
            .on_activate(
                false,
                ctx.header_activation,
                cx.listener(move |this, e: &ClickEvent, _w, cx| {
                    if !e.standard_click() || e.click_count() != 1 {
                        return;
                    }
                    if stuck {
                        this.navigate_sidebar_row(ix, cx);
                    } else {
                        this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                    }
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        menu_kind_for_right_click
                            .clone()
                            .invoked_by(context_menu_invoker_for_right_click.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .map(|row| ctx.paint_header(row.into_any_element(), row_surface))
            .into_any_element()
    }
}
