use super::*;
use crate::view::branch_sidebar;

/// Context menu for the "Pinned Local/Remote Branches" header row.
///
/// Pins are a view-only affordance, so the menu is deliberately small: the
/// collapse toggle the row's click already does, and the one bulk operation
/// that is tedious by hand — clearing the section out again.
pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    section: BranchSection,
) -> ContextMenuModel {
    let title = match section {
        BranchSection::Local => "Pinned Local Branches",
        BranchSection::Remote => "Pinned Remote Branches",
    };
    let collapse_key = branch_sidebar::pinned_section_storage_key(section);
    // A live filter force-expands the pinned section regardless of the stored
    // key, so reading the key alone would offer "Expand" on a section that is
    // visibly open — and activating it would collapse it.
    let collapsed = this.active_branch_filter().is_none()
        && this.sidebar_collapse_key_is_collapsed(repo_id, collapse_key);
    let pinned = this.pinned_branch_count(repo_id, section);

    let mut items = vec![ContextMenuItem::Header(title.into())];
    items.push(ContextMenuItem::Separator);

    items.push(ContextMenuItem::Entry {
        label: if collapsed { "Expand" } else { "Collapse" }.into(),
        icon: Some(
            if collapsed {
                "icons/chevron_right.svg"
            } else {
                "icons/chevron_down.svg"
            }
            .into(),
        ),
        shortcut: None,
        disabled: false,
        // Set, not toggle: under a live filter the label is computed from the
        // force-expanded row while the stored key still says collapsed, and a
        // flip would clear that key — leaving the section expanded once the
        // filter clears, the opposite of the "Collapse" the user clicked.
        action: Box::new(ContextMenuAction::SetSidebarCollapseKey {
            collapse_key: collapse_key.into(),
            collapsed: !collapsed,
        }),
    });

    items.push(ContextMenuItem::Entry {
        label: format!("Unpin all ({pinned})").into(),
        icon: Some("icons/pin.svg".into()),
        shortcut: None,
        disabled: pinned == 0,
        action: Box::new(ContextMenuAction::UnpinAllBranches { repo_id, section }),
    });

    ContextMenuModel::new(items)
}
