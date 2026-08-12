use super::*;

/// Shown when closing the window or quitting would throw away buffers the file
/// editor is still holding.
///
/// Only reachable with auto-save off — with it on the buffers are already on
/// disk by the time anything can close.
pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: UnsavedFileEditsPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let (title, discard_label) = match prompt.action {
        UnsavedFileEditsAction::CloseWindow(_) => ("Close window?", "Discard and close"),
        UnsavedFileEditsAction::QuitApp => ("Quit GitComet?", "Discard and quit"),
    };
    let detail = if prompt.files.len() == 1 {
        "1 edited file has not been saved.".to_string()
    } else {
        format!("{} edited files have not been saved.", prompt.files.len())
    };

    let action = prompt.action;
    let dialog = ConfirmDialog::new(title, DIALOG_440_WIDTH)
        .text(
            theme,
            format!("{detail} Save them, or discard the changes."),
        )
        .section(
            div()
                .px_2()
                .pb_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        // Capped: a long list would push the buttons out of the
                        // dialog, and the count above already says how many.
                        .children(prompt.files.iter().take(8).map(|label| {
                            div()
                                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                                .ml_2()
                                .child(label.clone())
                        }))
                        .when(prompt.files.len() > 8, |d| {
                            d.child(
                                div()
                                    .ml_2()
                                    .child(format!("…and {} more", prompt.files.len() - 8)),
                            )
                        }),
                ),
        );

    dialog.render(
        theme,
        cancel_button(
            "unsaved_file_edits_cancel",
            "unsaved_file_edits_cancel_hint",
            theme,
        )
        .on_click(theme, cx, |this, _e, _window, cx| {
            let root_view = this.root_view.clone();
            let _ = root_view.update(cx, |root, cx| {
                root.clear_pending_unsaved_file_edits_prompt(cx);
            });
            this.close_popover(cx);
        }),
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                components::Button::new("unsaved_file_edits_discard", discard_label)
                    .style(components::ButtonStyle::Danger)
                    .on_click(theme, cx, move |this, _e, _window, cx| {
                        let root_view = this.root_view.clone();
                        let _ = root_view.update(cx, |root, cx| {
                            root.resolve_unsaved_file_edits(action, false, cx);
                        });
                        this.close_popover(cx);
                    }),
            )
            .child(
                components::Button::new("unsaved_file_edits_save", "Save all")
                    .style(components::ButtonStyle::Filled)
                    .on_click(theme, cx, move |this, _e, _window, cx| {
                        let root_view = this.root_view.clone();
                        let _ = root_view.update(cx, |root, cx| {
                            root.resolve_unsaved_file_edits(action, true, cx);
                        });
                        this.close_popover(cx);
                    }),
            ),
        cx,
    )
}
