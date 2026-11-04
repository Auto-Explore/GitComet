//! The Tags section: hidden in the factory layout, shown where the user
//! places it. Clicking a tag reveals its commit in the history.

use super::*;

const TAG_ICON_PATH: &str = "icons/tag.svg";

impl SidebarPaneView {
    pub(super) fn sidebar_tags_header_row(
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
        let loading = this.active_repo().is_some_and(|repo| {
            matches!(repo.tags, Loadable::Loading)
                || (!collapsed && matches!(repo.tags, Loadable::NotLoaded))
        });
        let row_state = components::InteractiveRowState::default();

        div()
            .id(collapse_key.clone())
            .debug_selector(move || format!("tags_section_{ix}"))
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
            .child(ctx.tree_icon_slot(TAG_ICON_PATH, ctx.icon_primary, 14.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.colors.foreground.primary)
                    .child("Tags"),
            )
            .when(loading, |d| {
                d.child(ctx.svg_spinner(("tags_spinner", repo_id.0), ctx.icon_muted, 12.0))
            })
            .gitcomet_tooltip(theme, "Tags".into())
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
            .map(|row| ctx.paint_header(row.into_any_element(), row_surface))
            .into_any_element()
    }

    /// One local tag; a click shows its commit, a right-click its menu.
    pub(super) fn sidebar_tag_item_row(
        this: &Self,
        ctx: &SidebarRowContext,
        slot: SidebarRowSlot,
        name: SharedString,
        target: CommitId,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let SidebarRowSlot { ix, row_style, .. } = slot;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let context_menu_invoker: SharedString =
            format!("tag_sidebar_menu_{}_{}", repo_id.0, name).into();
        let context_menu_active =
            this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
        let row_group: SharedString = format!("tag_row_{}_{}", repo_id.0, ix).into();
        let row_state = components::InteractiveRowState::default().open(context_menu_active);
        let target_for_click = target.clone();
        let tooltip: SharedString = format!("{name} ({})", short_commit(&target.0)).into();

        div()
            .id(("tag_sidebar_row", ix))
            .debug_selector(move || format!("tag_sidebar_row_{ix}"))
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
            .child(ctx.tree_icon_slot(TAG_ICON_PATH, ctx.icon_primary, 14.0))
            .child(
                components::FadingText::new(
                    div()
                        .text_size(theme.ui_text(14.0))
                        .child(filtered_label_element(
                            name.clone(),
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
                .hover_bg(row_group, row_style.resolved_hover_background(row_state))
                .render(ctx.ui_scale_percent)
                .flex_1(),
            )
            .on_activate(
                false,
                controls::ControlActivation::Composite,
                cx.listener(move |this, e: &ClickEvent, _w, cx| {
                    if !e.standard_click() {
                        return;
                    }
                    this.reveal_commit_in_history(repo_id, target_for_click.clone(), cx);
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_popover_at(
                        (PopoverKind::TagMenu {
                            repo_id,
                            commit_id: target.clone(),
                        })
                        .invoked_by(context_menu_invoker.clone()),
                        e.position,
                        window,
                        cx,
                    );
                }),
            )
            .gitcomet_tooltip(theme, tooltip)
            .into_any_element()
    }
}

fn short_commit(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
