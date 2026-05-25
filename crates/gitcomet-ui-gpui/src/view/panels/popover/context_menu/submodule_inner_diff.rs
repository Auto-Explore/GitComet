use super::*;

pub(super) fn model(
    _repo_id: RepoId,
    submodule_repo_path: &std::path::Path,
    target: &DiffTarget,
) -> ContextMenuModel {
    let label = match target {
        DiffTarget::WorkingTree { path, .. } => path.display().to_string(),
        DiffTarget::Commit {
            path: Some(path), ..
        }
        | DiffTarget::CommitRange {
            path: Some(path), ..
        } => path.display().to_string(),
        DiffTarget::Commit { path: None, .. } | DiffTarget::CommitRange { path: None, .. } => {
            "Nested diff".to_string()
        }
    };
    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Submodule diff".into()),
        ContextMenuItem::Label(label.into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Open in submodule tab".into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenSubmoduleDiffInTab {
                path: submodule_repo_path.to_path_buf(),
                target: target.clone(),
            }),
        },
    ])
}
