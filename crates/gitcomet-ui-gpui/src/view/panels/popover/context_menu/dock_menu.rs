use super::*;
use crate::view::dock::{DockMenu, DockMenuEntry};

/// A panel's or a section's layout menu: the root built the rows from the
/// layout, so this only draws them.
pub(super) fn model(menu: &DockMenu) -> ContextMenuModel {
    let items = menu
        .entries
        .iter()
        .map(|entry| match entry {
            DockMenuEntry::Header(title) => ContextMenuItem::Header(title.clone().into()),
            DockMenuEntry::Separator => ContextMenuItem::Separator,
            DockMenuEntry::Item {
                label,
                command,
                checked,
                disabled,
            } => ContextMenuItem::Entry {
                label: label.clone(),
                icon: checked.then_some("icons/check.svg".into()),
                shortcut: None,
                disabled: *disabled,
                action: Box::new(ContextMenuAction::Dock(command.clone())),
            },
        })
        .collect();
    ContextMenuModel::new(items)
}
