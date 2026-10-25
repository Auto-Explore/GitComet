//! The git-annex section: its header, placeholder and repository rows.

use super::*;

impl SidebarPaneView {
    pub(super) fn sidebar_annex_header_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        collapsed: bool,
        collapse_key: SharedString,
        summary: Option<SharedString>,
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
        let context_menu_invoker: SharedString = format!("annex_section_menu_{}", repo_id.0).into();
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let row_state = components::InteractiveRowState::default().open(context_menu_active);

        div()
            .id(collapse_key.clone())
            .debug_selector(move || format!("annex_section_{ix}"))
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
            .child(ctx.tree_icon_slot("icons/disk.svg", ctx.icon_primary, 14.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.colors.foreground.primary)
                    .child("git-annex"),
            )
            .when_some(summary, |row, summary| {
                row.child(
                    div()
                        .flex_none()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(summary),
                )
            })
            .gitcomet_tooltip(
                theme,
                "git-annex repositories and special remotes (right-click for sync, remotes and more)".into(),
            )
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
                        PopoverKind::annex(repo_id, AnnexPopoverKind::SectionMenu)
                            .invoked_by(context_menu_invoker.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .map(|row| ctx.paint_header(row.into_any_element(), row_surface))
            .into_any_element()
    }

    pub(super) fn sidebar_annex_placeholder_row(
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        message: SharedString,
        can_init: bool,
        can_restage: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        div()
            .id(("annex_placeholder", slot.ix))
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
            .when(can_init, |row| {
                // Every clone of an annex repo needs this once.
                row.child(
                    components::Button::new(
                        format!("annex_placeholder_init_{}", repo_id.0),
                        "Initialize",
                    )
                    .borderless()
                    .on_click(theme, cx, move |this, _e, _window, _cx| {
                        this.store.dispatch(Msg::RunLargeFileCommand {
                            repo_id,
                            command: gitcomet_core::large_files::LargeFileCommand::AnnexInit,
                        });
                    }),
                )
            })
            .when(can_restage, |row| {
                // After an interrupted download Git still sees the old
                // pointer files; this refreshes its index.
                row.child(
                    components::Button::new(
                        format!("annex_placeholder_restage_{}", repo_id.0),
                        "Refresh",
                    )
                    .borderless()
                    .on_click(theme, cx, move |this, _e, _window, _cx| {
                        this.store.dispatch(Msg::RunLargeFileCommand {
                            repo_id,
                            command: gitcomet_core::large_files::LargeFileCommand::AnnexRestage,
                        });
                    }),
                )
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn sidebar_annex_repository_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        uuid: SharedString,
        name: SharedString,
        detail: SharedString,
        here: bool,
        untrusted: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot { ix, row_style, .. } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let context_menu_invoker: SharedString =
            format!("annex_repository_menu_{}_{}", repo_id.0, uuid).into();
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let row_state = components::InteractiveRowState::default().open(context_menu_active);
        let icon = if here {
            "icons/computer.svg"
        } else {
            "icons/cloud.svg"
        };
        let icon_color = if untrusted {
            theme.colors.status.warning.foreground
        } else {
            ctx.icon_primary
        };
        let uuid_for_menu = uuid.to_string();
        div()
            .id(("annex_repository", ix))
            .debug_selector(move || format!("annex_repository_{ix}"))
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
            .child(ctx.tree_icon_slot(icon, icon_color, 14.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .child(filtered_label_element(
                        name,
                        None,
                        &ctx.filter_query,
                        theme.colors.foreground.primary,
                        theme.colors.accent.foreground,
                        theme.ui_text(14.0).into(),
                        FontWeight::NORMAL,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(detail),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        PopoverKind::annex(
                            repo_id,
                            AnnexPopoverKind::RepositoryMenu {
                                uuid: uuid_for_menu.clone(),
                            },
                        )
                        .invoked_by(context_menu_invoker.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .into_any_element()
    }
}
