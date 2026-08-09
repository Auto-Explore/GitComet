use super::*;

pub(super) fn model(
    repo_id: RepoId,
    context: TerminalMenuContext,
    cx: &gpui::Context<PopoverHost>,
) -> ContextMenuModel {
    let clipboard_has_text = crate::clipboard::read_text(cx).is_some_and(|text| !text.is_empty());

    ContextMenuModel::new(vec![
        ContextMenuItem::Entry {
            label: "Copy".into(),
            icon: None,
            shortcut: Some(terminal_copy_shortcut().into()),
            disabled: !context.has_selection,
            action: Box::new(ContextMenuAction::TerminalCopy { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "Paste".into(),
            icon: None,
            shortcut: Some(terminal_paste_shortcut().into()),
            disabled: !context.connected || !clipboard_has_text,
            action: Box::new(ContextMenuAction::TerminalPaste { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "Select All".into(),
            icon: None,
            shortcut: Some(terminal_select_all_shortcut().into()),
            disabled: !context.has_session,
            action: Box::new(ContextMenuAction::TerminalSelectAll { repo_id }),
        },
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Clear".into(),
            icon: None,
            shortcut: None,
            disabled: !context.connected,
            action: Box::new(ContextMenuAction::TerminalClear { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "Open in External Terminal".into(),
            icon: None,
            shortcut: None,
            disabled: !context.has_session,
            action: Box::new(ContextMenuAction::TerminalOpenExternal { repo_id }),
        },
    ])
}

fn terminal_copy_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+C"
    } else {
        "Ctrl+Shift+C"
    }
}

fn terminal_paste_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+V"
    } else {
        "Ctrl+Shift+V"
    }
}

fn terminal_select_all_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+A"
    } else {
        "Ctrl+Shift+A"
    }
}
