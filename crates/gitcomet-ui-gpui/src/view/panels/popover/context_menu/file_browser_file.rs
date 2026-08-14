use super::*;
use gitcomet_core::domain::{DiffArea, FileSource};

/// Context menu for a file row in the sidebar file browser. The browsed source
/// (working directory / commit) is read from state so the menu offers the right
/// actions for each.
pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    path: &std::path::Path,
    cx: &gpui::Context<PopoverHost>,
) -> ContextMenuModel {
    let source = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| repo.file_browser.source.clone())
        .unwrap_or_default();

    let mut items = vec![ContextMenuItem::Header(
        path.file_name()
            .and_then(|p| p.to_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{path:?}"))
            .into(),
    )];
    items.push(ContextMenuItem::Label(
        components::ContextMenuText::path_single_line(path.display().to_string()),
    ));
    items.push(ContextMenuItem::Separator);

    items.push(ContextMenuItem::Entry {
        label: "Open".into(),
        icon: Some("icons/file.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenFileContent {
            repo_id,
            source: source.clone(),
            path: path.to_path_buf(),
        }),
    });

    if let Some(target) = diff_target_for_source(&source, path) {
        items.push(ContextMenuItem::Entry {
            label: "Open diff".into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SelectDiff { repo_id, target }),
        });
    }

    items.push(ContextMenuItem::Entry {
        label: "Edit file".into(),
        icon: Some("icons/pencil.svg".into()),
        shortcut: None,
        // The editor always opens the workspace copy, so browsing a commit is
        // no obstacle — but a branch listing has no path on disk at all, and a
        // picture has no text to edit.
        disabled: matches!(source, FileSource::Branch(_))
            || crate::view::should_bypass_text_file_preview_for_path(path),
        action: Box::new(ContextMenuAction::EditFile {
            repo_id,
            path: path.to_path_buf(),
        }),
    });

    // Only offered for a file the editor is actually holding unsaved text for —
    // an always-present "Discard changes" would read as "revert the file", which
    // is what the status list's discard does and is a different, far more
    // destructive thing.
    if this
        .main_pane
        .read(cx)
        .file_edits_are_unsaved_for(repo_id, path)
    {
        items.push(ContextMenuItem::Entry {
            label: "Discard unsaved edits".into(),
            icon: Some("icons/undo.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::DiscardFileEdits {
                repo_id,
                path: path.to_path_buf(),
            }),
        });
    }

    // The working-tree file is on disk, so these OS actions only apply there.
    if matches!(source, FileSource::WorkingDirectory) {
        items.push(ContextMenuItem::Entry {
            label: "Open file".into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenFile {
                repo_id,
                path: path.to_path_buf(),
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Open file location".into(),
            icon: Some("icons/folder.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenFileLocation {
                repo_id,
                path: path.to_path_buf(),
            }),
        });
    }

    items.push(ContextMenuItem::Entry {
        label: "File history".into(),
        icon: Some("icons/refresh.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory {
                repo_id,
                path: path.to_path_buf(),
            },
        }),
    });

    let file_permalink = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| {
            let remotes = match &repo.remotes {
                Loadable::Ready(remotes) => remotes,
                _ => return None,
            };
            // The browsed source decides the permalink reference: the commit
            // or branch being browsed, or the current branch for the working
            // directory.
            let (reference, is_branch) = match &source {
                FileSource::Commit(commit_id) => (commit_id.as_ref().to_string(), false),
                FileSource::Branch(name) => (name.clone(), true),
                FileSource::WorkingDirectory => match &repo.head_branch {
                    Loadable::Ready(head) if !head.is_empty() && head != "HEAD" => {
                        (head.clone(), true)
                    }
                    _ => return None,
                },
            };
            // A branch permalink is a `blob/<branch>` link that only resolves
            // while the branch exists on the permalink's remote. For a
            // local-only branch (never pushed) it would point at a nonexistent
            // source, so don't offer the action in that case.
            if is_branch
                && let Loadable::Ready(remote_branches) = &repo.remote_branches
                && !crate::view::permalink::branch_exists_on_permalink_remote(
                    remotes,
                    remote_branches,
                    &reference,
                )
            {
                return None;
            }
            crate::view::permalink::file_permalink(remotes, &reference, &path.display().to_string())
        });
    if let Some(permalink) = file_permalink {
        items.push(ContextMenuItem::Entry {
            label: "Copy file permalink".into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CopyText { text: permalink }),
        });
    }

    items.push(ContextMenuItem::Separator);
    push_copy_path_entries(&mut items, this, repo_id, path, None);

    ContextMenuModel::new(items)
}

fn diff_target_for_source(source: &FileSource, path: &std::path::Path) -> Option<DiffTarget> {
    match source {
        FileSource::WorkingDirectory => Some(DiffTarget::WorkingTree {
            path: path.to_path_buf(),
            area: DiffArea::Unstaged,
        }),
        FileSource::Commit(commit_id) => Some(DiffTarget::Commit {
            commit_id: commit_id.clone(),
            path: Some(path.to_path_buf()),
        }),
        FileSource::Branch(_) => None,
    }
}
