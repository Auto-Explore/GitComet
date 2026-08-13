//! The menu a repository row in the repository picker opens on right-click.
//!
//! Close kin to [`super::repo_tab`], which is the same repository's menu on its
//! tab. This one adds the two things only the picker can do — pinning a
//! repository to the top of the list, and forgetting a closed one — and offers
//! the rest for a repository that is not open at all.

use super::*;

pub(super) fn model(host: &PopoverHost, entry: &repo_picker::RepoPickerEntry) -> ContextMenuModel {
    let workdir = entry.workdir(host);
    let pinned = workdir
        .as_ref()
        .is_some_and(|path| host.cached_pinned_repos.iter().any(|pin| pin == path));

    let mut items = Vec::new();
    if let Some(workdir) = workdir.clone() {
        items.push(if pinned {
            ContextMenuItem::Entry {
                label: "Unpin repository".into(),
                icon: Some("icons/pin.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::UnpinRepository { path: workdir }),
            }
        } else {
            ContextMenuItem::Entry {
                label: "Pin repository".into(),
                icon: Some("icons/pin.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::PinRepository { path: workdir }),
            }
        });
        items.push(ContextMenuItem::Separator);
    }

    match entry {
        repo_picker::RepoPickerEntry::Open(repo_id) => items.push(ContextMenuItem::Entry {
            label: "Activate".into(),
            icon: Some("icons/check.svg".into()),
            shortcut: None,
            // The active repository cannot be activated again — a row menu
            // never offers a no-op.
            disabled: host.state.active_repo == Some(*repo_id),
            action: Box::new(ContextMenuAction::ActivateRepo { repo_id: *repo_id }),
        }),
        // Same wording and icon as the "+" menu's Open repository, which is the
        // other way into this action.
        repo_picker::RepoPickerEntry::Closed(path) => items.push(ContextMenuItem::Entry {
            label: "Open repository".into(),
            icon: Some("icons/disk.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenRepo { path: path.clone() }),
        }),
    }

    if let Some(workdir) = workdir.clone() {
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::Entry {
            label: "Open repository location".into(),
            icon: Some("icons/folder.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenRepositoryLocation {
                path: workdir.clone(),
            }),
        });
        if crate::external_editor::configured_setting().is_some() {
            items.push(ContextMenuItem::Entry {
                label: "Open in code editor".into(),
                icon: Some("icons/open_external.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::OpenInCodeEditor {
                    repo_id: None,
                    path: workdir.clone(),
                }),
            });
        }
        items.push(ContextMenuItem::Entry {
            label: "Copy path".into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CopyText {
                text: workdir.display().to_string(),
            }),
        });
    }

    // A pin is what keeps a closed repository listed, so forgetting it while it
    // is pinned would leave the row exactly where it was — the pinned closed row
    // is the one case with nothing to put here at all.
    let destructive = match entry {
        repo_picker::RepoPickerEntry::Open(repo_id) => Some(ContextMenuItem::Entry {
            label: "Close repository".into(),
            icon: Some("icons/repo_tab_close.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CloseRepo { repo_id: *repo_id }),
        }),
        repo_picker::RepoPickerEntry::Closed(_) if pinned => None,
        repo_picker::RepoPickerEntry::Closed(path) => Some(ContextMenuItem::Entry {
            label: "Remove from recently closed".into(),
            icon: Some("icons/repo_tab_close.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::ForgetRecentRepository { path: path.clone() }),
        }),
    };
    if let Some(destructive) = destructive {
        items.push(ContextMenuItem::Separator);
        items.push(destructive);
    }

    ContextMenuModel::new(items)
}
