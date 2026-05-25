use super::*;
use gitcomet_core::domain::SubmoduleStatus;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SubmoduleMenuState {
    pub(super) open_path: Option<std::path::PathBuf>,
    pub(super) status: Option<SubmoduleStatus>,
    pub(super) can_open: bool,
    pub(super) can_change_pointer: bool,
    pub(super) show_load: bool,
}

fn fallback_submodule_initialized(path: Option<&std::path::Path>) -> bool {
    path.is_some_and(|path| path.join(".git").exists())
}

pub(super) fn menu_state(
    this: &PopoverHost,
    repo_id: RepoId,
    path: &std::path::Path,
) -> SubmoduleMenuState {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let open_path = repo.map(|repo| repo.spec.workdir.join(path));
    let status = repo.and_then(|repo| match &repo.submodules {
        Loadable::Ready(submodules) => submodules
            .iter()
            .find(|submodule| submodule.path == path)
            .map(|submodule| submodule.status),
        _ => None,
    });

    let fallback_initialized = fallback_submodule_initialized(open_path.as_deref());
    let initialized = match status {
        Some(SubmoduleStatus::NotInitialized) => false,
        Some(SubmoduleStatus::MergeConflict | SubmoduleStatus::MissingMapping) => false,
        Some(_) => true,
        None => fallback_initialized,
    };
    let show_load = matches!(status, Some(SubmoduleStatus::NotInitialized))
        || (status.is_none() && !fallback_initialized);
    let can_open = open_path.is_some() && initialized;
    let can_change_pointer = can_open
        && !matches!(
            status,
            Some(SubmoduleStatus::MergeConflict | SubmoduleStatus::MissingMapping)
        );

    SubmoduleMenuState {
        open_path,
        status,
        can_open,
        can_change_pointer,
        show_load,
    }
}

pub(super) fn status_label(status: Option<SubmoduleStatus>) -> Option<&'static str> {
    match status {
        Some(SubmoduleStatus::NotInitialized) => Some("Not loaded"),
        Some(SubmoduleStatus::HeadMismatch) => Some("Head mismatch"),
        Some(SubmoduleStatus::MergeConflict) => Some("Conflict"),
        Some(SubmoduleStatus::MissingMapping) => Some("Missing mapping"),
        Some(SubmoduleStatus::Unknown(_)) => Some("Unknown"),
        Some(SubmoduleStatus::UpToDate) | None => None,
    }
}

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    path: &std::path::Path,
) -> ContextMenuModel {
    let state = menu_state(this, repo_id, path);
    let mut items = vec![ContextMenuItem::Header("Submodule".into())];
    items.push(ContextMenuItem::Label(
        components::ContextMenuText::path_single_line(path.display().to_string()),
    ));
    if let Some(status_label) = status_label(state.status) {
        items.push(ContextMenuItem::Label(status_label.into()));
    }
    items.push(ContextMenuItem::Separator);

    items.push(ContextMenuItem::Entry {
        label: "Open submodule".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: !state.can_open,
        action: Box::new(ContextMenuAction::OpenRepo {
            path: state.open_path.clone().unwrap_or_default(),
        }),
    });

    if state.show_load {
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
        label: "Change pointer…".into(),
        icon: Some("icons/swap.svg".into()),
        shortcut: None,
        disabled: !state.can_change_pointer,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::submodule(
                repo_id,
                SubmodulePopoverKind::ChangePointerPrompt {
                    path: path.to_path_buf(),
                },
            ),
        }),
    });

    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Remove…".into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::submodule(
                repo_id,
                SubmodulePopoverKind::RemoveConfirm {
                    path: path.to_path_buf(),
                },
            ),
        }),
    });

    ContextMenuModel::new(items)
}
