use super::*;
use crate::view::panes::ExplorerAction;
use crate::view::shortcut_labels::{Shortcut, secondary_shortcut};

/// Appends one menu group, separated from the entries before it. A group that
/// adds nothing leaves no separator behind.
pub(super) fn push_group(
    items: &mut Vec<ContextMenuItem>,
    fill: impl FnOnce(&mut Vec<ContextMenuItem>),
) {
    let start = items.len();
    fill(items);
    if start > 0 && items.len() > start {
        items.insert(start, ContextMenuItem::Separator);
    }
}

/// The explorer's file operations for one row, split into groups so each menu
/// can place them. Entries stay struct literals because the icon guard test
/// only checks SVG paths written out at the entry.
pub(super) struct ExplorerOps<'a> {
    host: &'a PopoverHost,
    repo_id: RepoId,
    path: &'a std::path::Path,
    root: bool,
}

impl<'a> ExplorerOps<'a> {
    /// `None` outside the working tree: a commit or branch listing has nothing
    /// to write to.
    pub(super) fn new(
        host: &'a PopoverHost,
        repo_id: RepoId,
        path: &'a std::path::Path,
        source: &gitcomet_core::domain::FileSource,
    ) -> Option<Self> {
        (*source == gitcomet_core::domain::FileSource::WorkingDirectory).then_some(Self {
            host,
            repo_id,
            path,
            root: path.as_os_str().is_empty(),
        })
    }

    fn action(&self, action: ExplorerAction) -> Box<ContextMenuAction> {
        Box::new(ContextMenuAction::Explorer {
            repo_id: self.repo_id,
            path: self.path.to_path_buf(),
            action,
        })
    }

    pub(super) fn push_create(&self, items: &mut Vec<ContextMenuItem>) {
        items.push(ContextMenuItem::Entry {
            label: "New file".into(),
            icon: Some("icons/file_plus.svg".into()),
            shortcut: None,
            disabled: false,
            action: self.action(ExplorerAction::NewFile),
        });
        items.push(ContextMenuItem::Entry {
            label: "New folder".into(),
            icon: Some("icons/folder_plus.svg".into()),
            shortcut: None,
            disabled: false,
            action: self.action(ExplorerAction::NewFolder),
        });
    }

    /// The root has nothing to cut, copy or duplicate, only a place to paste.
    pub(super) fn push_clipboard(&self, items: &mut Vec<ContextMenuItem>) {
        if !self.root {
            items.push(ContextMenuItem::Entry {
                label: "Cut".into(),
                icon: Some("icons/scissors.svg".into()),
                shortcut: Some(secondary_shortcut("X").into()),
                disabled: false,
                action: self.action(ExplorerAction::Cut),
            });
            items.push(ContextMenuItem::Entry {
                label: "Copy".into(),
                icon: Some("icons/copy.svg".into()),
                shortcut: Some(secondary_shortcut("C").into()),
                disabled: false,
                action: self.action(ExplorerAction::Copy),
            });
        }
        items.push(ContextMenuItem::Entry {
            label: "Paste".into(),
            icon: Some("icons/clipboard_paste.svg".into()),
            shortcut: Some(secondary_shortcut("V").into()),
            disabled: false,
            action: self.action(ExplorerAction::Paste),
        });
        if !self.root {
            items.push(ContextMenuItem::Entry {
                label: "Duplicate".into(),
                icon: Some("icons/copy_plus.svg".into()),
                shortcut: Some(secondary_shortcut("D").into()),
                disabled: false,
                action: self.action(ExplorerAction::Duplicate),
            });
        }
    }

    pub(super) fn push_rename_ignore(&self, items: &mut Vec<ContextMenuItem>) {
        if self.root {
            return;
        }
        let multi_selection = self
            .host
            .state
            .repos
            .iter()
            .find(|r| r.id == self.repo_id)
            .is_some_and(|r| r.file_browser.selection.paths.len() > 1);
        items.push(ContextMenuItem::Entry {
            label: "Rename…".into(),
            icon: Some("icons/pencil.svg".into()),
            shortcut: Some("F2".into()),
            disabled: multi_selection,
            action: self.action(ExplorerAction::Rename),
        });
        items.push(ContextMenuItem::Entry {
            label: "Add to .gitignore".into(),
            icon: Some("icons/eye_off.svg".into()),
            shortcut: None,
            disabled: self
                .host
                .explorer_gitignore_target(self.repo_id, self.path)
                .is_none(),
            action: Box::new(ContextMenuAction::AddExplorerToGitignore {
                repo_id: self.repo_id,
                path: self.path.to_path_buf(),
            }),
        });
    }

    pub(super) fn push_destroy(&self, items: &mut Vec<ContextMenuItem>) {
        if self.root {
            return;
        }
        items.push(ContextMenuItem::Entry {
            label: "Trash".into(),
            icon: Some("icons/trash.svg".into()),
            shortcut: Some("Delete".into()),
            disabled: false,
            action: self.action(ExplorerAction::Trash),
        });
        items.push(ContextMenuItem::Entry {
            label: "Delete permanently…".into(),
            icon: Some("icons/trash.svg".into()),
            shortcut: Some("Shift+Delete".into()),
            disabled: false,
            action: self.action(ExplorerAction::Delete),
        });
    }

    pub(super) fn push_undo_redo(&self, items: &mut Vec<ContextMenuItem>) {
        let filesystem = &self.host.state.filesystem;
        items.push(ContextMenuItem::Entry {
            label: "Undo".into(),
            icon: Some("icons/undo.svg".into()),
            shortcut: Some(secondary_shortcut("Z").into()),
            disabled: !filesystem.undo_available,
            action: self.action(ExplorerAction::Undo),
        });
        // Ctrl+Y rather than Ctrl+Shift+Z off macOS, so the menu's single-key
        // mnemonic (the last `+` part) does not collide with Undo's `z`.
        let redo = Shortcut::Platform {
            macos: "Cmd+Shift+Z",
            other: "Ctrl+Y",
        };
        items.push(ContextMenuItem::Entry {
            label: "Redo".into(),
            icon: Some("icons/redo.svg".into()),
            shortcut: redo.label().map(Into::into),
            disabled: !filesystem.redo_available,
            action: self.action(ExplorerAction::Redo),
        });
    }
}
