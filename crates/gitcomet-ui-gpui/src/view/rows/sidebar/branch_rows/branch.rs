//! Local and remote branch rows, with their divergence, upstream and
//! worktree badges.

use super::*;

impl SidebarPaneView {
    /// One local or remote branch, in the tree or among the pins.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sidebar_branch_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        name: SharedString,
        target: BranchMenuTarget,
        section: BranchSection,
        depth: u16,
        muted: bool,
        divergence_ahead: Option<NonZeroU32>,
        divergence_behind: Option<NonZeroU32>,
        is_head: bool,
        is_upstream: bool,
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
        let full_name_for_checkout: SharedString = name.clone();
        let surface = if ix < ctx.pin_count {
            SidebarRowSurface::Pins
        } else {
            ctx.surface
        };
        let pin_key = (ix < ctx.pin_count).then(|| ctx.row_keys[ix].clone());
        let selected_branch = this.selected_branch_for_row(pin_key.as_ref()).cloned();
        let full_name_for_menu: SharedString = name.clone();
        let full_name_for_tooltip: SharedString = name.clone();
        let target_for_reveal = target.clone();
        let target_for_checkout = target.clone();
        let target_for_menu = target.clone();
        let section_key = match section {
            BranchSection::Local => "local",
            BranchSection::Remote => "remote",
        };
        let menu_prefix = if surface == SidebarRowSurface::Pins {
            "pinned_"
        } else {
            ""
        };
        let context_menu_invoker: SharedString = format!(
            "{menu_prefix}branch_menu_{}_{}_{}",
            repo_id.0,
            section_key,
            full_name_for_menu.as_ref()
        )
        .into();
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let context_menu_invoker_for_right_click = context_menu_invoker.clone();
        let full_label = (surface == SidebarRowSurface::Pins && depth == 0)
            || matches!(surface, SidebarRowSurface::Sticky { compact: true });
        let label: SharedString = if full_label {
            name.clone()
        } else {
            crate::view::branch_sidebar::branch_sidebar_branch_label(name.as_ref())
                .to_owned()
                .into()
        };
        let badge_worktree_path = (section == BranchSection::Local)
            .then(|| ctx.worktree_badges.listed_path(name.as_ref()).cloned())
            .flatten();
        let active_worktree_path = (section == BranchSection::Local)
            .then(|| ctx.worktree_badges.active_path(name.as_ref()).cloned())
            .flatten();
        let worktree_badge_path = branch_worktree_badge_path(
            badge_worktree_path.as_deref(),
            active_worktree_path.as_deref(),
        );
        let branch_selected = branch_row_is_selected(
            selected_branch.as_ref(),
            repo_id,
            &target,
            ctx.selected_commit.as_ref(),
            ctx.selected_branch_commit_id.as_ref(),
        );
        let has_worktree = worktree_badge_path.is_some();
        let has_active_worktree = active_worktree_path.is_some();
        let show_worktree_badge = has_worktree;
        let worktree_row_menu_invoker: Option<SharedString> =
            worktree_badge_path.as_ref().map(|path| {
                format!(
                    "{menu_prefix}worktree_menu_{}_{}",
                    repo_id.0,
                    path.display()
                )
                .into()
            });
        let worktree_menu_active = worktree_row_menu_invoker
            .as_ref()
            .is_some_and(|invoker| this.active_context_menu_invoker.as_ref() == Some(invoker));
        let row_group: SharedString = if surface == SidebarRowSurface::Pins {
            format!("pinned_branch_row_{}_{}", repo_id.0, ix).into()
        } else {
            format!("branch_row_{}_{}", repo_id.0, ix).into()
        };
        let row_debug_selector = row_group.as_ref().to_owned();
        let branch_text_color = if surface == SidebarRowSurface::Pins {
            theme.colors.foreground.primary
        } else if muted {
            theme.colors.foreground.secondary
        } else {
            ctx.branch_tree_color(section)
        };
        let branch_selected_bg = selected_branch_row_bg(theme);
        let branch_selected_label_color = if branch_selected {
            selected_branch_label_color(theme)
        } else {
            branch_text_color
        };
        let branch_icon_color = if is_head {
            ctx.icon_current
        } else if muted {
            ctx.icon_muted
        } else {
            ctx.icon_primary
        };
        let badge_gap_px = ctx.px(BRANCH_BADGE_GAP_PX);
        let head_highlight = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.18 } else { 0.12 },
        );
        let row_state = components::InteractiveRowState::default()
            .selected(
                is_head || branch_selected,
                if branch_selected {
                    branch_selected_bg
                } else {
                    head_highlight
                },
            )
            .open(context_menu_active);

        let mut row = div()
            .id(("branch_item", ix))
            .debug_selector(move || row_debug_selector.clone())
            .relative()
            .h(ctx.row_height())
            .w_full()
            .group(row_group.clone())
            .flex()
            .items_center()
            .gap(ctx.px(BRANCH_TREE_GAP_PX))
            .pl(ctx.indent_px(if full_label { 0 } else { usize::from(depth) }))
            .pr(ctx.trailing_pad())
            .bg(row_surface)
            .interactive_row(row_style, row_state)
            .when(branch_selected, |row| {
                row.row_accent(theme.colors.accent.foreground)
            })
            .text_color(branch_text_color)
            .child(ctx.tree_toggle_slot(None).when(
                surface == SidebarRowSurface::Pins && depth == 0,
                |slot| {
                    slot.debug_selector(move || format!("sidebar_pin_marker_{ix}"))
                        .child(ctx.svg_icon("icons/pin.svg", ctx.icon_primary, 12.0))
                },
            ))
            .child(
                ctx.tree_icon_slot("icons/git_branch.svg", branch_icon_color, 14.0)
                    .debug_selector(move || format!("sidebar_branch_icon_{surface:?}_{ix}")),
            )
            .child(
                // Long branch names run into the trailing badges;
                // fade them into the row instead of slicing a glyph.
                components::FadingText::new(
                    div()
                        .text_size(theme.ui_text(14.0))
                        .text_color(branch_selected_label_color)
                        .child(filtered_label_element(
                            label,
                            Some(&name),
                            &ctx.filter_query,
                            branch_selected_label_color,
                            theme.colors.accent.foreground,
                            gpui::rems(0.875).into(),
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
            );

        let show_branch_badges = divergence_behind.is_some()
            || divergence_ahead.is_some()
            || (is_upstream && section == BranchSection::Remote)
            || show_worktree_badge;
        let mut end_accessories = div()
            .ml_auto()
            .flex_none()
            .flex()
            .items_center()
            .gap(badge_gap_px);

        if divergence_behind.is_some() || divergence_ahead.is_some() {
            if let Some(behind) = divergence_behind {
                let color = theme.colors.status.warning.foreground;
                end_accessories = end_accessories.child(ctx.divergence_badge(
                    "icons/arrow_down.svg",
                    color,
                    behind,
                    Some(format!("branch_pull_badge_{ix}")),
                ));
            }
            if let Some(ahead) = divergence_ahead {
                let color = theme.colors.status.success.foreground;
                end_accessories = end_accessories.child(ctx.divergence_badge(
                    "icons/arrow_up.svg",
                    color,
                    ahead,
                    Some(format!("branch_push_badge_{ix}")),
                ));
            }
        }

        if is_upstream && section == BranchSection::Remote {
            end_accessories = end_accessories
                .child(ctx.upstream_badge(Some(format!("branch_upstream_badge_{ix}"))));
        }

        if show_worktree_badge {
            let Some(worktree_badge_path) = worktree_badge_path.clone() else {
                unreachable!("workspace badge requires a worktree path");
            };
            let worktree_badge = Self::sidebar_branch_worktree_badge(
                ctx,
                ix,
                &name,
                worktree_badge_path,
                worktree_row_menu_invoker,
                has_active_worktree,
                worktree_menu_active,
                cx,
            );
            end_accessories = end_accessories.child(worktree_badge);
        }

        if show_branch_badges {
            row = row.child(end_accessories);
        }

        row = row
            .on_activate(
                false,
                controls::ControlActivation::Composite,
                cx.listener(move |this, e: &ClickEvent, window, cx| {
                    if !e.standard_click() {
                        return;
                    }
                    if e.click_count() == 1 {
                        let Some(target) = this.active_repo().and_then(|repo| {
                            branch_click_history_reveal_target(repo, &target_for_reveal, is_head)
                        }) else {
                            return;
                        };
                        this.select_branch_and_reveal_tip(
                            repo_id,
                            target_for_reveal.clone(),
                            target.commit_id,
                            target.fallback_scope,
                            pin_key.clone(),
                            cx,
                        );
                        this.retain_sidebar_click_target(ix, surface, e);
                        return;
                    }
                    if e.click_count() < 2 {
                        return;
                    }
                    match section {
                        BranchSection::Local => {
                            match local_branch_double_click_action(
                                full_name_for_checkout.as_ref(),
                                badge_worktree_path.as_deref(),
                            ) {
                                LocalBranchDoubleClickAction::CheckoutBranch { name } => {
                                    this.store.dispatch(Msg::CheckoutBranch { repo_id, name });
                                    this.rebuild_diff_cache(cx);
                                    cx.notify();
                                }
                                LocalBranchDoubleClickAction::OpenWorktree { path } => {
                                    crate::app::open_repository_from_view(
                                        cx,
                                        window.window_handle().window_id(),
                                        path,
                                    );
                                    cx.notify();
                                }
                            }
                        }
                        BranchSection::Remote => {
                            if let Some((remote, branch)) = target_for_checkout.remote_parts() {
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
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        (PopoverKind::BranchMenu {
                            repo_id,
                            target: target_for_menu.clone(),
                        })
                        .invoked_by(context_menu_invoker_for_right_click.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .gitcomet_tooltip(
                theme,
                crate::view::branch_sidebar::branch_sidebar_branch_tooltip(
                    full_name_for_tooltip.as_ref(),
                    is_upstream,
                ),
            );

        row.into_any_element()
    }

    /// The pill naming the worktree a local branch is checked out in.
    fn sidebar_branch_worktree_badge(
        ctx: &SidebarRowContext,
        ix: usize,
        name: &SharedString,
        worktree_badge_path: std::path::PathBuf,
        worktree_row_menu_invoker: Option<SharedString>,
        has_active_worktree: bool,
        worktree_menu_active: bool,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let worktree_menu_invoker_for_click = worktree_row_menu_invoker.clone();
        let worktree_menu_invoker_for_right_click = worktree_row_menu_invoker.clone();
        let worktree_path_for_menu = worktree_badge_path.clone();
        let worktree_path_for_open = worktree_badge_path.clone();
        let worktree_path_for_right_click = worktree_badge_path.clone();
        let worktree_badge_label = crate::view::path_display::repo_path_name(&worktree_badge_path);
        let worktree_badge_tooltip: SharedString = worktree_badge_path.display().to_string().into();
        let branch_name_for_click = name.to_string();
        let branch_name_for_right_click = branch_name_for_click.clone();
        let badge_colors = worktree_badge_colors(
            ctx.worktree_badge_palette,
            has_active_worktree,
            worktree_menu_active,
        );
        div()
            .id(("branch_worktree_badge", ix))
            .debug_selector(move || format!("branch_worktree_badge_{ix}"))
            .flex()
            .items_center()
            .gap(ctx.px(3.0))
            .px(ctx.px(6.0))
            .h(worktree_badge_height(
                ui_scale::UiScale::from_percent(ctx.ui_scale_percent)
                    .with_appearance(theme.metrics),
            ))
            // Squared off on the control radius the buttons and
            // tabs use, matching the upstream status chip rather
            // than the fully-round decorative pills.
            .rounded(px(theme.radii.control))
            .border_1()
            .border_color(badge_colors.border)
            .bg(badge_colors.bg)
            .text_size(theme.ui_text(11.0))
            .text_color(badge_colors.text)
            .cursor(CursorStyle::PointingHand)
            // A worktree folder can outrun the pane. Cap and
            // truncate the pill rather than let it push the
            // row's other badges off the trailing edge. An
            // absolute cap, not a percentage: the pill's
            // containing block is the accessory run, which is
            // itself sized by this pill.
            .max_w(ctx.px(BRANCH_WORKTREE_BADGE_MAX_W_PX))
            .overflow_hidden()
            .child(ctx.svg_icon(WORKTREE_ICON_PATH, badge_colors.icon, 9.0))
            .child(
                div().min_w(px(0.0)).overflow_hidden().child(
                    components::TruncatedText::new(worktree_badge_label, theme.ui_text(11.0))
                        .id(("branch_worktree_badge_text", ix))
                        // Explicit color: TruncatedText resolves an
                        // unset one from the ambient text style in a
                        // deferred measure closure that never sees the
                        // pill's `.text_color`.
                        .text_color(badge_colors.text)
                        .render(cx),
                ),
            )
            .control_interaction(
                worktree_badge_interaction(theme),
                controls::InteractionState::default()
                    .selected(has_active_worktree, ctx.worktree_badge_palette.active_bg)
                    .open(worktree_menu_active),
            )
            .on_activate(
                false,
                controls::ControlActivation::Nested,
                cx.listener(move |this, e: &ClickEvent, window, cx| {
                    if !e.standard_click() {
                        return;
                    }
                    cx.stop_propagation();
                    if e.click_count() >= 2 {
                        crate::app::open_repository_from_view(
                            cx,
                            window.window_handle().window_id(),
                            worktree_path_for_open.clone(),
                        );
                        cx.notify();
                        return;
                    }
                    let Some(invoker) = worktree_menu_invoker_for_click.clone() else {
                        return;
                    };

                    this.open_popover_at(
                        (PopoverKind::worktree(
                            repo_id,
                            WorktreePopoverKind::Menu {
                                path: worktree_path_for_menu.clone(),
                                branch: Some(branch_name_for_click.clone()),
                            },
                        ))
                        .invoked_by(invoker),
                        e.position(),
                        window,
                        cx,
                    );
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let Some(invoker) = worktree_menu_invoker_for_right_click.clone() else {
                        return;
                    };

                    this.open_popover_at(
                        (PopoverKind::worktree(
                            repo_id,
                            WorktreePopoverKind::Menu {
                                path: worktree_path_for_right_click.clone(),
                                branch: Some(branch_name_for_right_click.clone()),
                            },
                        ))
                        .invoked_by(invoker),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .gitcomet_tooltip(theme, worktree_badge_tooltip.clone())
    }
}
