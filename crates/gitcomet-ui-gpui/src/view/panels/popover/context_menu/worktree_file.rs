use super::*;

pub(super) fn model(
    host: &PopoverHost,
    repo_id: RepoId,
    worktree_path: &std::path::Path,
    target: &DiffTarget,
) -> ContextMenuModel {
    let Some(path) = target.file_path() else {
        return ContextMenuModel::new(Vec::new());
    };
    let absolute = resolve_checkout_file_path(worktree_path, path).ok();
    let exists = absolute.as_ref().is_some_and(|path| path.exists());
    let can_view = host
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .filter(|repo| repo.history_state.worktree_selection.as_deref() == Some(worktree_path))
        .and_then(|repo| repo.worktree_dirty.ready())
        .and_then(|summaries| {
            summaries
                .iter()
                .find(|summary| summary.path == worktree_path)
        })
        .is_some_and(|summary| match target {
            DiffTarget::WorkingTree {
                path,
                area: DiffArea::Staged,
                worktree: None,
                ..
            } => summary.staged.iter().any(|file| &file.path == path),
            DiffTarget::WorkingTree {
                path,
                area: DiffArea::Unstaged,
                worktree: None,
                ..
            } => summary.unstaged.iter().any(|file| &file.path == path),
            _ => false,
        });
    let entry =
        |label: &'static str, icon: &'static str, disabled, action| ContextMenuItem::Entry {
            label: label.into(),
            icon: Some(icon.into()),
            shortcut: None,
            disabled,
            action: Box::new(action),
        };
    let mut items = vec![
        ContextMenuItem::Header(
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned()
                .into(),
        ),
        ContextMenuItem::Label(components::ContextMenuText::path_single_line(
            path.display().to_string(),
        )),
        ContextMenuItem::Separator,
        entry(
            "View diff",
            "icons/file.svg",
            !can_view,
            ContextMenuAction::OpenWorktreeDiff {
                repo_id,
                worktree_path: worktree_path.to_path_buf(),
                target: target.clone(),
            },
        ),
        entry(
            "Open in worktree tab",
            "icons/open_external.svg",
            !can_view,
            ContextMenuAction::OpenSubmoduleDiffInTab {
                path: worktree_path.to_path_buf(),
                target: target.clone(),
            },
        ),
        entry(
            "Open file",
            "icons/file.svg",
            !exists,
            ContextMenuAction::OpenWorktreeFile {
                worktree_path: worktree_path.to_path_buf(),
                path: path.to_path_buf(),
            },
        ),
        entry(
            "Open file location",
            "icons/folder.svg",
            absolute.is_none(),
            ContextMenuAction::OpenWorktreeFileLocation {
                worktree_path: worktree_path.to_path_buf(),
                path: path.to_path_buf(),
            },
        ),
    ];
    if crate::external_editor::configured_setting().is_some() {
        items.push(entry(
            "Open in code editor",
            "icons/open_external.svg",
            !exists,
            ContextMenuAction::OpenInCodeEditor {
                repo_id: None,
                path: absolute.clone().unwrap_or_default(),
            },
        ));
    }
    items.push(ContextMenuItem::Separator);
    if let Some(absolute) = absolute {
        items.push(entry(
            "Copy absolute path",
            "icons/copy.svg",
            false,
            ContextMenuAction::CopyText {
                text: path_text_for_copy(&absolute),
            },
        ));
    }
    items.push(ContextMenuItem::Entry {
        label: "Copy relative path".into(),
        icon: Some("icons/copy.svg".into()),
        shortcut: Some("C".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::CopyText {
            text: path_text_for_copy(path),
        }),
    });
    ContextMenuModel::new(items)
}
