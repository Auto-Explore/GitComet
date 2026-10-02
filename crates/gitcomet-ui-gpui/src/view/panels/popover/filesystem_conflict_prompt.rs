use super::*;
use crate::kit::interaction as controls;
use gitcomet_core::filesystem::ConflictChoice;

/// Answers the open dialog in the root view, then closes it.
fn resolve(
    prompt_id: u64,
    choice: ConflictChoice,
) -> impl Fn(&mut PopoverHost, &ClickEvent, &mut Window, &mut gpui::Context<PopoverHost>) + 'static
{
    move |this, _e, window, cx| {
        let apply_to_all =
            choice != ConflictChoice::Cancel && this.filesystem_conflict_apply_to_all;
        let _ = this.root_view.update(cx, |root, cx| {
            root.resolve_filesystem_conflict(prompt_id, choice, apply_to_all, window, cx);
        });
        this.close_popover_and_restore_focus(window, cx);
    }
}

/// Asked when a copy, move, paste or rename finds an item with the same name.
pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: FilesystemConflictPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let dialog_width = DIALOG_540_WIDTH.preferred_px(popover_ui_scale(cx));
    let prompt_id = prompt.prompt_id;

    let destination = components::TruncatedText::new(
        SharedString::from(prompt.destination.display().to_string()),
        theme.ui_text(14.0),
    )
    .id("filesystem_conflict_destination_text")
    // The file name is the part to keep when the path is long.
    .profile(components::TextTruncationProfile::Path)
    .text_color(theme.colors.foreground.secondary)
    .full_text_tooltip(this.tooltip_host.clone());

    let mut note = String::from(
        "Keep both adds “copy” to the new item’s name. Replace moves the existing item into the undo journal.",
    );
    if prompt.can_merge {
        note.push_str(
            " Merge copies the folder’s contents into the existing folder; collisions inside it are asked about separately.",
        );
    }

    let mut dialog = ConfirmDialog::new("An item with this name already exists", DIALOG_540_WIDTH)
        .section(
            div()
                .debug_selector(|| "filesystem_conflict_destination".to_string())
                .px_2()
                .py_1()
                .min_w(px(0.0))
                .max_w(dialog_width)
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .child(destination.render(cx)),
        )
        .note(theme, note);
    if prompt.remaining > 0 || prompt.in_directory_merge {
        let label = if prompt.remaining > 0 {
            format!("Apply to all remaining ({})", prompt.remaining)
        } else {
            "Apply to all remaining".to_string()
        };
        dialog = dialog.section(
            div().px_1().pb_1().child(
                checkbox_row(
                    "filesystem_conflict_apply_all",
                    "filesystem_conflict_apply_all",
                    label,
                    this.filesystem_conflict_apply_to_all,
                    &this.filesystem_conflict_apply_to_all_focus_handle,
                    theme,
                    cx,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _w, cx| {
                        this.filesystem_conflict_apply_to_all =
                            !this.filesystem_conflict_apply_to_all;
                        cx.notify();
                    }),
                ),
            ),
        );
    }

    // Safe primary action last, the destructive one beside it (house order).
    dialog.render(
        theme,
        cancel_button(
            "filesystem_conflict_cancel",
            "filesystem_conflict_cancel_hint",
            theme,
        )
        .on_click(theme, cx, resolve(prompt_id, ConflictChoice::Cancel)),
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                components::Button::new("filesystem_conflict_skip", "Skip")
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, resolve(prompt_id, ConflictChoice::Skip))
                    .debug_selector(|| "filesystem_conflict_skip".to_string()),
            )
            .when(prompt.can_merge, |buttons| {
                buttons.child(
                    components::Button::new("filesystem_conflict_merge", "Merge")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, resolve(prompt_id, ConflictChoice::Merge))
                        .debug_selector(|| "filesystem_conflict_merge".to_string()),
                )
            })
            .child(
                components::Button::new("filesystem_conflict_replace", "Replace")
                    .style(components::ButtonStyle::Danger)
                    .on_click(theme, cx, resolve(prompt_id, ConflictChoice::Replace))
                    .debug_selector(|| "filesystem_conflict_replace".to_string()),
            )
            .child(
                components::Button::new("filesystem_conflict_keep_both", "Keep both")
                    .style(components::ButtonStyle::Filled)
                    .on_click(theme, cx, resolve(prompt_id, ConflictChoice::KeepBoth))
                    .debug_selector(|| "filesystem_conflict_keep_both".to_string()),
            ),
        cx,
    )
}
