use super::*;
use crate::kit::interaction as controls;

fn annotated_toggle(
    theme: AppTheme,
    enabled: bool,
    focus_handle: &FocusHandle,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
    super::checkbox_row(
        "create_tag_annotated_toggle",
        "create_tag_annotated_toggle",
        "Annotated tag",
        enabled,
        focus_handle,
        theme,
        cx,
    )
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    target: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_create = this.can_submit_create_tag(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);
    let message_scroll = this.create_tag_message_scroll.clone();
    let annotated = this.create_tag_annotated;

    div()
        .flex()
        .flex_col()
        .w(scaled_px(420.0))
        .child(popover_title(theme, "Create tag"))
        .child(super::popover_rule(theme))
        .child(super::popover_detail(theme, format!("Target: {target}")))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.create_tag_input.clone()),
        )
        .child(super::popover_rule(theme))
        .child(
            annotated_toggle(
                theme,
                annotated,
                &this.create_tag_annotated_focus_handle,
                cx,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _w, cx| {
                    this.create_tag_annotated = !this.create_tag_annotated;
                    cx.notify();
                }),
            ),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child("Annotated tags can be GPG signed and include a message"),
        )
        .when(annotated, |panel| {
            panel
                .child(super::popover_rule(theme))
                .child(
                    div()
                        .px_2()
                        .pt_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child("Annotation message"),
                )
                .child(
                    div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                        components::ScrollContainer::vertical(
                            "create_tag_message_scroll_surface",
                            "create_tag_message_scrollbar",
                            message_scroll,
                            scaled_px(140.0),
                        )
                        .render(theme, this.create_tag_message_input.clone()),
                    ),
                )
        })
        .child(super::popover_rule(theme))
        .child(
            super::prompt_footer_row()
                .child(
                    cancel_button("create_tag_cancel", "create_tag_cancel_hint", theme)
                        .focus_handle(this.create_tag_focus.cancel.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("create_tag_go", "Create")
                        .focus_handle(this.create_tag_focus.submit.clone())
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "create_tag_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_create)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_create_tag(cx);
                        }),
                ),
        )
}
