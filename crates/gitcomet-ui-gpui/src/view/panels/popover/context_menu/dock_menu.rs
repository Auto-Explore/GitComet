use super::*;
use crate::view::dock::{DockMenu, DockMenuEntry, DockMenuTarget, SectionId};

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

/// Adds "Layout…" to a section header's own menu. The header's right-click
/// opens that menu, so without it the section's layout menu (move up or
/// down, move to a panel, hide) would be out of reach.
pub(super) fn with_layout_entry(
    mut model: ContextMenuModel,
    section: &'static str,
) -> ContextMenuModel {
    model.items.push(ContextMenuItem::Separator);
    model.items.push(ContextMenuItem::Entry {
        label: "Layout…".into(),
        icon: Some("icons/list_groups.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenDockMenu(DockMenuTarget::Section(
            SectionId::from(section),
        ))),
    });
    model
}
