use gitcomet_ui_kit::components::{ContextMenuIconSlot, ContextMenuTone};
use gpui::SharedString;

pub(super) fn entry_presentation(
    label: &str,
    icon: Option<SharedString>,
    reserve_icon_column: bool,
) -> (ContextMenuIconSlot, ContextMenuTone) {
    let path = icon
        .as_ref()
        .and_then(|icon| context_menu_icon_path(icon, label))
        .map(SharedString::from);
    let tone = context_menu_tone(label, path.as_deref());
    let slot = match path {
        Some(path) => ContextMenuIconSlot::Icon(path),
        None if reserve_icon_column || icon.is_some() => ContextMenuIconSlot::Reserved,
        None => ContextMenuIconSlot::None,
    };
    (slot, tone)
}

pub(super) fn context_menu_tone(label: &str, icon_path: Option<&str>) -> ContextMenuTone {
    if label == "Close" && icon_path == Some("icons/repo_tab_close.svg") {
        return ContextMenuTone::Accent;
    }
    if label == "Trash" {
        return ContextMenuTone::Accent;
    }

    if label.starts_with("Force push")
        || matches!(
            icon_path,
            Some("icons/trash.svg") | Some("icons/repo_tab_close.svg")
        )
        || label.contains("Delete")
        || label.contains("Drop")
        || label.contains("Remove")
    {
        return ContextMenuTone::Danger;
    }
    if matches!(icon_path, Some("icons/warning.svg"))
        || label.contains("Force")
        || label.contains("Discard")
    {
        return ContextMenuTone::Warning;
    }
    if matches!(icon_path, Some("icons/arrow_up.svg")) || label.starts_with("Push") {
        return ContextMenuTone::Success;
    }
    if matches!(icon_path, Some("icons/arrow_down.svg")) || label.starts_with("Pull") {
        return ContextMenuTone::Warning;
    }
    if matches!(icon_path, Some("icons/plus.svg")) || label.starts_with("Stage") {
        return ContextMenuTone::Success;
    }
    if matches!(icon_path, Some("icons/minus.svg")) || label.starts_with("Unstage") {
        return ContextMenuTone::Warning;
    }

    ContextMenuTone::Accent
}

fn context_menu_icon_path<'a>(icon: &'a str, label: &str) -> Option<&'a str> {
    let trimmed = icon.trim();
    if trimmed.starts_with("icons/") || trimmed.starts_with("extensions/") {
        return Some(trimmed);
    }
    match trimmed {
        "link" => return Some("icons/link.svg"),
        "unlink" => return Some("icons/unlink.svg"),
        _ => {}
    }

    if label.starts_with("Pull") {
        return Some("icons/arrow_down.svg");
    }
    if label.starts_with("Push") {
        return Some("icons/arrow_up.svg");
    }
    if label.contains("Delete") || label.contains("Drop") || label.contains("Remove") {
        return Some("icons/trash.svg");
    }
    if label.contains("Tag") {
        return Some("icons/tag.svg");
    }
    if label.contains("Open") && label.contains("location") {
        return Some("icons/folder.svg");
    }
    if label.contains("Open") {
        return Some("icons/open_external.svg");
    }
    if label.starts_with("Stage") {
        return Some("icons/plus.svg");
    }
    if label.starts_with("Unstage") {
        return Some("icons/minus.svg");
    }
    if label.contains("Squash") {
        return Some("icons/arrow_right.svg");
    }
    if label.contains("Edit") {
        return Some("icons/pencil.svg");
    }
    if label.contains("Reset") {
        return Some("icons/refresh.svg");
    }
    if label.contains("Revert") {
        return Some("icons/undo.svg");
    }
    if label.contains("Copy") {
        return Some("icons/copy.svg");
    }
    None
}

#[cfg(test)]
mod tests;
