//! Leaf rows of the stash, worktree and submodule sections.

use super::*;

impl SidebarPaneView {
    /// One stash entry; double-click applies it.
    pub(super) fn sidebar_stash_item_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        index: usize,
        message: SharedString,
        tooltip: SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot { row_style, .. } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let tooltip = tooltip.clone();
        let stash_message_for_menu = message.as_ref().to_owned();
        let context_menu_invoker: SharedString =
            format!("stash_menu_{}_{}", repo_id.0, index).into();
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let context_menu_invoker_for_right_click = context_menu_invoker.clone();
        let stash_message_for_right_click = stash_message_for_menu.clone();
        let row_group: SharedString = format!("stash_row_{}_{}", repo_id.0, index).into();
        let row_state = components::InteractiveRowState::default().open(context_menu_active);

        div()
            .id(("stash_sidebar_row", index))
            .debug_selector(move || format!("stash_sidebar_row_{index}"))
            .relative()
            .group(row_group.clone())
            .flex()
            .items_center()
            .gap(ctx.px(BRANCH_TREE_GAP_PX))
            .pl(ctx.indent_px(0))
            .pr(ctx.trailing_pad())
            .h(ctx.row_height())
            .w_full()
            .interactive_row(row_style, row_state)
            .child(ctx.tree_toggle_slot(None))
            .child(ctx.tree_icon_slot(STASH_ICON_PATH, ctx.icon_primary, 14.0))
            .child(
                components::FadingText::new(
                    div()
                        .text_size(theme.ui_text(14.0))
                        .child(filtered_label_element(
                            message.clone(),
                            None,
                            &ctx.filter_query,
                            theme.colors.foreground.primary,
                            theme.colors.accent.foreground,
                            theme.ui_text(14.0).into(),
                            FontWeight::NORMAL,
                            cx,
                        )),
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
                controls::ControlActivation::Composite,
                cx.listener(move |this, e: &ClickEvent, _w, cx| {
                    if !e.standard_click() || e.click_count() < 2 {
                        return;
                    }
                    this.store.dispatch(Msg::ApplyStash { repo_id, index });
                    cx.notify();
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        (PopoverKind::StashMenu {
                            repo_id,
                            index,
                            message: stash_message_for_right_click.clone(),
                        })
                        .invoked_by(context_menu_invoker_for_right_click.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .gitcomet_tooltip(theme, tooltip.clone())
            .into_any_element()
    }

    /// One linked worktree with its branch pill; double-click opens it.
    pub(super) fn sidebar_worktree_item_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        path: std::path::PathBuf,
        branch: Option<SharedString>,
        detached: bool,
        is_active: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot { ix, row_style, .. } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let branch = branch.clone();
        let path_for_open = path.clone();
        let path_for_menu = path.clone();
        let branch_for_menu = branch.as_ref().map(|name| name.to_string());
        let path_label = this.cached_path_display(&path);
        let context_menu_invoker: SharedString =
            format!("worktree_menu_{}_{}", repo_id.0, path.display()).into();
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let open_worktree_repo = this.open_repo_for_workdir(&path);
        let worktree_tab_open = open_worktree_repo.is_some();
        let branch_badge_label =
            worktree_branch_badge_label(branch.as_ref(), detached, open_worktree_repo);
        let branch_badge_colors = worktree_badge_colors(
            ctx.worktree_badge_palette,
            worktree_tab_open,
            context_menu_active,
        );
        let context_menu_invoker_for_right_click = context_menu_invoker.clone();
        let row_group: SharedString = format!("worktree_row_{}_{}", repo_id.0, ix).into();
        let row_debug_selector = row_group.as_ref().to_owned();
        let active_background = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.18 } else { 0.12 },
        );
        let row_state = components::InteractiveRowState::default()
            .selected(is_active, active_background)
            .open(context_menu_active);

        div()
            .id(("worktree_item", ix))
            .debug_selector(move || row_debug_selector.clone())
            .relative()
            .h(ctx.row_height())
            .w_full()
            .flex()
            .items_center()
            .gap(ctx.px(BRANCH_TREE_GAP_PX))
            .pl(ctx.indent_px(0))
            .pr(ctx.trailing_pad())
            .interactive_row(row_style, row_state)
            .child(ctx.tree_toggle_slot(None))
            .child(ctx.tree_icon_slot(WORKTREE_ICON_PATH, ctx.icon_primary, 14.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .gap(ctx.px(5.0))
                    .child(
                        div()
                            .debug_selector(move || format!("worktree_path_label_{ix}"))
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(
                                components::TruncatedText::path(
                                    path_label.clone(),
                                    theme.ui_text(14.0),
                                )
                                .id(("worktree_path_text", ix))
                                .highlights(search_label_highlights(
                                    &ctx.filter_query,
                                    &path_label,
                                    theme.colors.accent.foreground,
                                ))
                                // Set the color explicitly: TruncatedText
                                // resolves an unset color from the ambient text
                                // style inside a deferred measure closure, which
                                // doesn't see ancestor `text_color` — so in the
                                // collapsed popover it would render near-black.
                                .text_color(theme.colors.foreground.primary)
                                .full_text_tooltip(this.tooltip_host.clone())
                                .render(cx),
                            ),
                    )
                    .when_some(branch_badge_label.clone(), |row, badge_label| {
                        row.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(ctx.px(3.0))
                                .px(ctx.px(6.0))
                                .h(worktree_badge_height(
                                    ui_scale::UiScale::from_percent(ctx.ui_scale_percent)
                                        .with_appearance(theme.metrics),
                                ))
                                // Same control radius as the branch
                                // rows' worktree badge; the two are
                                // the same chip in two lists.
                                .rounded(px(theme.radii.control))
                                .border_1()
                                .border_color(branch_badge_colors.border)
                                .bg(branch_badge_colors.bg)
                                .text_size(theme.ui_text(11.0))
                                .text_color(branch_badge_colors.text)
                                .id(("worktree_branch_badge", ix))
                                .debug_selector(move || format!("worktree_branch_badge_{ix}"))
                                .max_w_1_2()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .child(ctx.svg_icon(
                                    "icons/git_branch.svg",
                                    branch_badge_colors.icon,
                                    9.0,
                                ))
                                .child(
                                    div()
                                        .debug_selector(move || {
                                            format!("worktree_branch_badge_label_{ix}")
                                        })
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .child(
                                            components::TruncatedText::new(
                                                badge_label.clone(),
                                                theme.ui_text(11.0),
                                            )
                                            .id(("worktree_branch_badge_text", ix))
                                            .highlights(search_label_highlights(
                                                &ctx.filter_query,
                                                &badge_label,
                                                theme.colors.accent.foreground,
                                            ))
                                            // Explicit color: TruncatedText resolves an
                                            // unset color from the ambient text style in
                                            // a deferred measure closure that misses the
                                            // pill's `.text_color`, rendering near-black
                                            // in the collapsed popover.
                                            .text_color(branch_badge_colors.text)
                                            .full_text_tooltip(this.tooltip_host.clone())
                                            .render(cx),
                                        ),
                                ),
                        )
                    }),
            )
            .on_activate(
                false,
                controls::ControlActivation::Composite,
                cx.listener(move |this, e: &ClickEvent, window, cx| {
                    if !e.standard_click() {
                        return;
                    }
                    if e.click_count() >= 2 {
                        crate::app::open_repository_from_view(
                            cx,
                            window.window_handle().window_id(),
                            path_for_open.clone(),
                        );
                        cx.notify();
                        return;
                    }
                    // Single click mirrors a branch row: scroll the log
                    // to this worktree and select its row, without
                    // leaving the tab.
                    this.reveal_worktree_in_history(repo_id, path_for_open.clone(), is_active, cx);
                    cx.notify();
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        (PopoverKind::worktree(
                            repo_id,
                            WorktreePopoverKind::Menu {
                                path: path_for_menu.clone(),
                                branch: branch_for_menu.clone(),
                            },
                        ))
                        .invoked_by(context_menu_invoker_for_right_click.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .into_any_element()
    }

    /// The submodules placeholder, with a Load button when loading is deferred.
    pub(super) fn sidebar_submodule_placeholder_row(
        ctx: &SidebarRowContext,
        ix: usize,
        message: SharedString,
        can_load: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        div()
            .id(("submodule_placeholder", ix))
            .h(ctx.row_height())
            .w_full()
            .pl_2()
            .pr_1()
            .flex()
            .items_center()
            .gap(ctx.px(6.0))
            .text_size(theme.ui_text(14.0))
            .text_color(theme.colors.foreground.secondary)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .child(message),
            )
            .when(can_load, |row| {
                row.child(
                    components::Button::new(
                        format!("submodule_placeholder_load_{}", repo_id.0),
                        "Load",
                    )
                    .borderless()
                    .on_click(theme, cx, move |this, _e, _window, _cx| {
                        this.store.dispatch(Msg::LoadSubmodules { repo_id });
                    }),
                )
            })
            .into_any_element()
    }

    /// One submodule with its status pill; double-click opens it.
    pub(super) fn sidebar_submodule_item_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        path: std::path::PathBuf,
        status: SubmoduleStatus,
        recorded_head: CommitId,
        checked_out_head: Option<CommitId>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot { ix, row_style, .. } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let path_for_open = path.clone();
        let path_for_menu = path.clone();
        let repo_workdir_for_open = ctx.repo_workdir.clone();
        let path_label = this.cached_path_display(&path);
        let (icon_color, badge_label, can_open, tooltip) = {
            let badge_label = match status {
                SubmoduleStatus::NotInitialized => Some("Not loaded"),
                SubmoduleStatus::HeadMismatch => Some("Head mismatch"),
                SubmoduleStatus::MergeConflict => Some("Conflict"),
                SubmoduleStatus::MissingMapping => Some("Missing mapping"),
                SubmoduleStatus::Unknown(_) => Some("Unknown"),
                SubmoduleStatus::UpToDate => None,
            };
            let icon_color = match status {
                SubmoduleStatus::NotInitialized => with_alpha(
                    theme.colors.foreground.secondary,
                    if theme.is_dark { 0.78 } else { 0.92 },
                ),
                SubmoduleStatus::HeadMismatch => theme.colors.status.warning.foreground,
                SubmoduleStatus::MergeConflict | SubmoduleStatus::MissingMapping => {
                    theme.colors.status.danger.foreground
                }
                SubmoduleStatus::UpToDate | SubmoduleStatus::Unknown(_) => ctx.icon_primary,
            };
            let can_open = !matches!(
                status,
                SubmoduleStatus::NotInitialized
                    | SubmoduleStatus::MergeConflict
                    | SubmoduleStatus::MissingMapping
            );
            let checked_out = checked_out_head
                .as_ref()
                .map(|head| head.as_ref())
                .unwrap_or("not loaded");
            let tooltip: SharedString = format!(
                "{}\nRecorded: {}\nChecked out: {}",
                path.display(),
                recorded_head.as_ref(),
                checked_out,
            )
            .into();
            (icon_color, badge_label, can_open, tooltip)
        };
        let context_menu_invoker: SharedString =
            format!("submodule_menu_{}_{}", repo_id.0, path.display()).into();
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let context_menu_invoker_for_right_click = context_menu_invoker.clone();
        let row_state = components::InteractiveRowState::default().open(context_menu_active);

        div()
            .id(("submodule_item", ix))
            .relative()
            .h(ctx.row_height())
            .w_full()
            .flex()
            .items_center()
            .gap(ctx.px(BRANCH_TREE_GAP_PX))
            .pl(ctx.indent_px(0))
            .pr(ctx.trailing_pad())
            .interactive_row(row_style, row_state)
            .child(ctx.tree_toggle_slot(None))
            .child(ctx.tree_icon_slot("icons/box.svg", icon_color, 14.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .debug_selector(move || format!("submodule_label_{ix}"))
                    .child(filtered_label_element(
                        path_label,
                        None,
                        &ctx.filter_query,
                        theme.colors.foreground.primary,
                        theme.colors.accent.foreground,
                        theme.ui_text(14.0).into(),
                        FontWeight::NORMAL,
                        cx,
                    )),
            )
            .when_some(badge_label, |row, badge_label| {
                row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(ctx.px(3.0))
                        .px(ctx.px(4.0))
                        .py(ctx.px(0.0))
                        .rounded(px(theme.radii.pill))
                        .border_1()
                        .border_color(if context_menu_active {
                            theme.colors.stroke.default
                        } else {
                            with_alpha(
                                theme.colors.foreground.secondary,
                                if theme.is_dark { 0.32 } else { 0.24 },
                            )
                        })
                        .bg(with_alpha(
                            theme.colors.surface.panel,
                            if theme.is_dark { 0.9 } else { 0.7 },
                        ))
                        .text_size(theme.ui_text(11.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(badge_label),
                )
            })
            .on_activate(
                false,
                controls::ControlActivation::Composite,
                cx.listener(move |_this, e: &ClickEvent, window, cx| {
                    if !e.standard_click() || e.click_count() < 2 {
                        return;
                    }
                    if !can_open {
                        return;
                    }
                    let Some(base) = repo_workdir_for_open.clone() else {
                        return;
                    };
                    crate::app::open_repository_from_view(
                        cx,
                        window.window_handle().window_id(),
                        base.join(&path_for_open),
                    );
                    cx.notify();
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        (PopoverKind::submodule(
                            repo_id,
                            SubmodulePopoverKind::Menu {
                                path: path_for_menu.clone(),
                            },
                        ))
                        .invoked_by(context_menu_invoker_for_right_click.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .gitcomet_tooltip(theme, tooltip.clone())
            .into_any_element()
    }
}
