use super::*;
use gitcomet_extension_api::RefMenuTarget;

/// Extension entries for a branch or commit menu opened on `target`, in
/// registration order.
pub(super) fn entries(
    host: &PopoverHost,
    repo_id: RepoId,
    target: RefMenuTarget,
) -> impl Iterator<Item = ContextMenuItem> + '_ {
    host.extension_ref_menu
        .iter()
        .map(move |entry| ContextMenuItem::Entry {
            label: entry.label.clone(),
            icon: entry.icon.clone(),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::RunRefMenuItem {
                id: entry.id.clone(),
                repo_id,
                target: target.clone(),
            }),
        })
}
