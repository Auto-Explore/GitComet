use super::*;
use gitcomet_extension_api::RefMenuTarget;

/// Extension entries for a branch or commit menu opened on `target`, in
/// registration order; the branches-only ones only when `target` is a
/// branch.
pub(super) fn entries(
    host: &PopoverHost,
    repo_id: RepoId,
    target: RefMenuTarget,
) -> impl Iterator<Item = ContextMenuItem> + '_ {
    let branch = target.is_branch();
    host.extension_ref_menu
        .iter()
        .filter(move |entry| branch || !entry.branches_only)
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
