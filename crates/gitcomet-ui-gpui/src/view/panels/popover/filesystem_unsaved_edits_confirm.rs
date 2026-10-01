use super::*;

fn resolve(
    prompt_id: u64,
    choice: FilesystemUnsavedEditsChoice,
) -> impl Fn(&mut PopoverHost, &ClickEvent, &mut Window, &mut gpui::Context<PopoverHost>) + 'static
{
    move |this, _e, window, cx| {
        let _ = this.root_view.update(cx, |root, cx| {
            root.resolve_filesystem_unsaved_edits(prompt_id, choice, window, cx);
        });
        this.close_popover_and_restore_focus(window, cx);
    }
}

/// A file operation would move, delete or replace items that open buffers
/// still hold unsaved edits for.
pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: FilesystemUnsavedEditsPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let prompt_id = prompt.prompt_id;

    ConfirmDialog::new("These items contain unsaved edits", DIALOG_440_WIDTH)
        .text(
            theme,
            "Save the edits before continuing, or discard the unsaved buffers.",
        )
        .file_list(theme, &prompt.files)
        .render(
            theme,
            cancel_button(
                "filesystem_unsaved_edits_cancel",
                "filesystem_unsaved_edits_cancel_hint",
                theme,
            )
            .on_click(
                theme,
                cx,
                resolve(prompt_id, FilesystemUnsavedEditsChoice::Cancel),
            ),
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    components::Button::new("filesystem_unsaved_edits_discard", "Discard")
                        .style(components::ButtonStyle::Danger)
                        .on_click(
                            theme,
                            cx,
                            resolve(prompt_id, FilesystemUnsavedEditsChoice::Discard),
                        )
                        .debug_selector(|| "filesystem_unsaved_edits_discard".to_string()),
                )
                .child(
                    components::Button::new("filesystem_unsaved_edits_save", "Save")
                        .style(components::ButtonStyle::Filled)
                        .on_click(
                            theme,
                            cx,
                            resolve(prompt_id, FilesystemUnsavedEditsChoice::Save),
                        )
                        .debug_selector(|| "filesystem_unsaved_edits_save".to_string()),
                ),
            cx,
        )
}
