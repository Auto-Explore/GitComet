use super::*;

fn resolve(
    prompt_id: u64,
    confirmed: bool,
) -> impl Fn(&mut PopoverHost, &ClickEvent, &mut Window, &mut gpui::Context<PopoverHost>) + 'static
{
    move |this, _e, window, cx| {
        let _ = this.root_view.update(cx, |root, cx| {
            root.resolve_delete_permanently(prompt_id, confirmed, window, cx);
        });
        this.close_popover_and_restore_focus(window, cx);
    }
}

/// Explorer "Delete permanently": the one file operation that skips the trash
/// and the undo journal.
pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: DeletePermanentlyPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let prompt_id = prompt.prompt_id;
    let detail = match prompt.names.len() {
        1 => "1 item will be permanently removed. This cannot be undone.".to_string(),
        count => format!("{count} items will be permanently removed. This cannot be undone."),
    };

    ConfirmDialog::new("Delete permanently?", DIALOG_440_WIDTH)
        .text(theme, detail)
        .file_list(theme, &prompt.names)
        .render(
            theme,
            cancel_button(
                "delete_permanently_cancel",
                "delete_permanently_cancel_hint",
                theme,
            )
            .on_click(theme, cx, resolve(prompt_id, false)),
            components::Button::new("delete_permanently_confirm", "Delete permanently")
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, resolve(prompt_id, true))
                .debug_selector(|| "delete_permanently_confirm".to_string()),
            cx,
        )
}
