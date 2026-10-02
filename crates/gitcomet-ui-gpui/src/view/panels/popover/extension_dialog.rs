//! A dialog whose body an extension supplies. The host owns the frame, the
//! Close button, Escape, click-away, and focus restoration.

use super::*;

/// The open extension dialog's title and content.
pub(in crate::view) struct ExtensionDialog {
    pub(super) id: u64,
    pub(super) title: SharedString,
    pub(super) content: Option<gpui::AnyView>,
    pub(super) menu: Option<ContextMenuModel>,
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
    let Some(content) = dialog.content.clone() else {
        return div();
    };
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
    pub(in crate::view) fn close_for_gate(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self
            .popover
            .as_ref()
            .is_some_and(|kind| !kind.survives_gate())
        {
            self.close_popover_and_restore_focus(window, cx);
        }
    }

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
            content: Some(content),
            menu: None,
        });
        self.open_popover_centered(
            PopoverKind::Hosted {
                id: dialog_id,
                menu: false,
            },
            window,
            cx,
        );
    }

    pub(in crate::view) fn open_hosted_menu(
        &mut self,
        id: u64,
        items: Vec<gitcomet_extension_api::HostedMenuItem>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        use gitcomet_extension_api::HostedMenuItem;
        let items = items
            .into_iter()
            .filter_map(|item| match item {
                HostedMenuItem::Action {
                    action,
                    icon,
                    disabled,
                    ..
                } => Some(ContextMenuItem::Entry {
                    label: action.label().clone(),
                    icon,
                    shortcut: None,
                    disabled,
                    action: Box::new(ContextMenuAction::Hosted(action)),
                }),
                HostedMenuItem::Header(label) => Some(ContextMenuItem::Header(label.into())),
                HostedMenuItem::Separator => Some(ContextMenuItem::Separator),
                _ => None,
            })
            .collect();
        self.extension_dialog = Some(ExtensionDialog {
            id,
            title: "".into(),
            content: None,
            menu: Some(ContextMenuModel::new(items)),
        });
        self.open_popover_at(PopoverKind::Hosted { id, menu: true }, anchor, window, cx);
    }

    pub(in crate::view) fn reanchor_hosted(
        &mut self,
        id: u64,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(kind @ PopoverKind::Hosted { id: current, .. }) = self.popover.clone()
            && id == current
        {
            self.open_popover_at(kind, anchor, window, cx);
        }
    }

    /// Closes dialog `dialog_id` if it is still the one showing.
    pub(in crate::view) fn close_extension_dialog(
        &mut self,
        dialog_id: u64,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(self.popover, Some(PopoverKind::Hosted { id, .. }) if id == dialog_id) {
            self.close_popover_and_restore_focus(window, cx);
        }
    }
}
