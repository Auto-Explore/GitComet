use super::*;

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
    path: &std::path::Path,
) -> ContextMenuModel {
    let is_submodule = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| match &repo.history_state.commit_details {
            Loadable::Ready(details) if details.id == *commit_id => details
                .files
                .iter()
                .find(|file| file.path == path)
                .map(|file| file.is_submodule),
            _ => None,
        })
        .unwrap_or(false);
    let copy_path_text = this
        .resolve_workdir_path(repo_id, path)
        .map(|p| path_text_for_copy(&p))
        .unwrap_or_else(|_| path_text_for_copy(path));

    let mut items = vec![ContextMenuItem::Header(
        path.file_name()
            .and_then(|p| p.to_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{path:?}"))
            .into(),
    )];
    items.push(ContextMenuItem::Label(
        components::ContextMenuText::path_single_line(path.display().to_string()),
    ));
    if is_submodule {
        let submodule_state = super::submodule::menu_state(this, repo_id, path);
        if let Some(status_label) = super::submodule::status_label(submodule_state.status) {
            items.push(ContextMenuItem::Label(status_label.into()));
        }
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::Entry {
            label: "Open submodule summary".into(),
            icon: Some("icons/box.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SelectDiff {
                repo_id,
                target: DiffTarget::Commit {
                    commit_id: commit_id.clone(),
                    path: Some(path.to_path_buf()),
                },
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Open submodule".into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: None,
            disabled: !submodule_state.can_open,
            action: Box::new(ContextMenuAction::OpenRepo {
                path: submodule_state.open_path.clone().unwrap_or_default(),
            }),
        });
        if submodule_state.show_load {
            items.push(ContextMenuItem::Entry {
                label: "Load submodule".into(),
                icon: Some("icons/plus.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::LoadSubmodule {
                    repo_id,
                    path: path.to_path_buf(),
                }),
            });
        }
        items.push(ContextMenuItem::Entry {
            label: "Copy path".into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: Some("C".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::CopyText {
                text: copy_path_text,
            }),
        });
        return ContextMenuModel::new(items);
    }

    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Open diff".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::SelectDiff {
            repo_id,
            target: DiffTarget::Commit {
                commit_id: commit_id.clone(),
                path: Some(path.to_path_buf()),
            },
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Open file".into(),
        icon: Some("icons/file.svg".into()),
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
    items.push(ContextMenuItem::Entry {
        label: "File history".into(),
        icon: Some("icons/refresh.svg".into()),
        shortcut: Some("H".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::FileHistory {
                repo_id,
                path: path.to_path_buf(),
            },
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Copy path".into(),
        icon: Some("icons/copy.svg".into()),
        shortcut: Some("C".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::CopyText {
            text: copy_path_text,
        }),
    });

    ContextMenuModel::new(items)
}
