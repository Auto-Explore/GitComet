//! A dialog whose body an extension supplies. The host owns the frame, the
//! Close button, Escape, click-away, and focus restoration.

use super::*;

/// The open extension dialog's title and content.
pub(in crate::view) struct ExtensionDialog {
    pub(super) id: u64,
    pub(super) title: SharedString,
    pub(super) content: gpui::AnyView,
}

pub(super) fn panel(
    this: &mut PopoverHost,
    dialog_id: u64,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let Some(dialog) = this
        .extension_dialog
        .as_ref()
        .filter(|dialog| dialog.id == dialog_id)
    else {
        return div();
    };
    let title = dialog.title.clone();
    let content = dialog.content.clone();
    ConfirmDialog::new(title, DIALOG_440_WIDTH)
        .section(
            div()
                .id("extension_dialog_content")
                .debug_selector(|| "extension_dialog_content".to_string())
                .px_2()
                .py_1()
                .child(content),
        )
        .render(
            theme,
            div(),
            components::Button::new("extension_dialog_close", "Close").on_click(
                theme,
                cx,
                |this, _e, window, cx| {
                    this.close_popover_and_restore_focus(window, cx);
                },
            ),
            cx,
        )
}

impl PopoverHost {
    pub(in crate::view) fn open_extension_dialog(
        &mut self,
        dialog_id: u64,
        title: SharedString,
        content: gpui::AnyView,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.extension_dialog = Some(ExtensionDialog {
            id: dialog_id,
            title,
            content,
        });
        self.open_popover_centered(PopoverKind::ExtensionDialog { id: dialog_id }, window, cx);
    }

    /// Closes dialog `dialog_id` if it is still the one showing.
    pub(in crate::view) fn close_extension_dialog(
        &mut self,
        dialog_id: u64,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(self.popover, Some(PopoverKind::ExtensionDialog { id }) if id == dialog_id) {
            self.close_popover_and_restore_focus(window, cx);
        }
    }
}
