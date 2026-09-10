use super::*;
use crate::view::panes::ExplorerAction;

pub(super) fn append(
    this: &PopoverHost,
    items: &mut Vec<ContextMenuItem>,
    repo_id: RepoId,
    path: &std::path::Path,
    source: &gitcomet_core::domain::FileSource,
) {
    if *source != gitcomet_core::domain::FileSource::WorkingDirectory {
        return;
    }
    let root = path.as_os_str().is_empty();

    for (label, action) in [
        ("New File", ExplorerAction::NewFile),
        ("New Folder", ExplorerAction::NewFolder),
        ("Cut", ExplorerAction::Cut),
        ("Copy", ExplorerAction::Copy),
        ("Paste", ExplorerAction::Paste),
        ("Duplicate", ExplorerAction::Duplicate),
        ("Rename…", ExplorerAction::Rename),
        ("Trash", ExplorerAction::Trash),
        ("Delete permanently…", ExplorerAction::Delete),
        ("Undo", ExplorerAction::Undo),
        ("Redo", ExplorerAction::Redo),
    ] {
        if root
            && matches!(
                action,
                ExplorerAction::Cut
                    | ExplorerAction::Copy
                    | ExplorerAction::Duplicate
                    | ExplorerAction::Rename
                    | ExplorerAction::Trash
                    | ExplorerAction::Delete
            )
        {
            continue;
        }
        if matches!(
            action,
            ExplorerAction::Cut | ExplorerAction::Trash | ExplorerAction::Undo
        ) {
            items.push(ContextMenuItem::Separator);
        }
        if action == ExplorerAction::Trash && !root {
            items.push(ContextMenuItem::Entry {
                label: "Add to .gitignore".into(),
                icon: None,
                shortcut: None,
                disabled: this.explorer_gitignore_target(repo_id, path).is_none(),
                action: Box::new(ContextMenuAction::AddExplorerToGitignore {
                    repo_id,
                    path: path.to_path_buf(),
                }),
            });
        }
        let disabled = match action {
            ExplorerAction::Undo => !this.state.filesystem.undo_available,
            ExplorerAction::Redo => !this.state.filesystem.redo_available,
            ExplorerAction::Rename => this
                .state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .is_some_and(|r| r.file_browser.selection.paths.len() > 1),
            _ => false,
        };
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: None,
            shortcut: None,
            disabled,
            action: Box::new(ContextMenuAction::Explorer {
                repo_id,
                path: path.to_path_buf(),
                action,
            }),
        });
    }
}
