use super::*;
use std::path::Path;

pub(super) fn model(repo: Option<&RepoState>, repo_id: RepoId, path: &Path) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header("Subtree".into())];
    items.push(ContextMenuItem::Label(path.display().to_string().into()));
    items.push(ContextMenuItem::Separator);

    let open_path = repo.and_then(|repo| local_subtree_source_repo_path(repo, path));
    let open_disabled = open_path.is_none();
    let reveal_disabled = repo.is_none();
    items.push(ContextMenuItem::Entry {
        label: "Open source repo in new tab".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: open_disabled,
        action: Box::new(ContextMenuAction::OpenRepo {
            path: open_path.unwrap_or_default(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Reveal location".into(),
        icon: Some("icons/folder.svg".into()),
        shortcut: None,
        disabled: reveal_disabled,
        action: Box::new(ContextMenuAction::OpenFileLocation {
            repo_id,
            path: path.to_path_buf(),
        }),
    });

    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Pull…".into(),
        icon: Some("icons/pull.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(
                repo_id,
                SubtreePopoverKind::PullPrompt {
                    path: path.to_path_buf(),
                },
            ),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Push…".into(),
        icon: Some("icons/push.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(
                repo_id,
                SubtreePopoverKind::PushPrompt {
                    path: path.to_path_buf(),
                },
            ),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Split…".into(),
        icon: Some("icons/branch.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(
                repo_id,
                SubtreePopoverKind::SplitPrompt {
                    path: path.to_path_buf(),
                },
            ),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Remove…".into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(
                repo_id,
                SubtreePopoverKind::RemoveConfirm {
                    path: path.to_path_buf(),
                },
            ),
        }),
    });

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{RepoSpec, Subtree, SubtreeSourceConfig};
    use gitcomet_core::path_utils::canonicalize_or_original;

    fn repo_with_subtree_source(source_repository: Option<&str>) -> RepoState {
        let dir = tempfile::tempdir().expect("create tempdir");
        let workdir = dir.path().join("parent");
        std::fs::create_dir_all(&workdir).expect("create workdir");

        let source = source_repository.map(|source_repository| SubtreeSourceConfig {
            repository: source_repository.to_string(),
            reference: "main".to_string(),
            push_refspec: None,
            squash: true,
        });

        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: workdir.clone(),
            },
        );
        repo.subtrees = Loadable::Ready(std::sync::Arc::new(vec![Subtree {
            path: std::path::PathBuf::from("vendor/lib"),
            source,
        }]));
        repo
    }

    #[test]
    fn model_routes_open_through_open_repo_for_local_sources() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let workdir = dir.path().join("parent");
        let source = dir.path().join("source");
        std::fs::create_dir_all(&workdir).expect("create workdir");
        std::fs::create_dir_all(&source).expect("create source repo");

        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: workdir.clone(),
            },
        );
        repo.subtrees = Loadable::Ready(std::sync::Arc::new(vec![Subtree {
            path: std::path::PathBuf::from("vendor/lib"),
            source: Some(SubtreeSourceConfig {
                repository: "../source".to_string(),
                reference: "main".to_string(),
                push_refspec: None,
                squash: true,
            }),
        }]));

        let model = model(
            Some(&repo),
            RepoId(1),
            &std::path::PathBuf::from("vendor/lib"),
        );
        let open_action = model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == "Open source repo in new tab" =>
                {
                    Some((**action).clone())
                }
                _ => None,
            })
            .expect("expected open entry");

        assert!(matches!(
            open_action,
            ContextMenuAction::OpenRepo { path: open_path }
                if open_path == canonicalize_or_original(source)
        ));
    }

    #[test]
    fn model_disables_open_for_remote_sources() {
        let repo = repo_with_subtree_source(Some("https://example.com/repo.git"));
        let model = model(
            Some(&repo),
            RepoId(1),
            &std::path::PathBuf::from("vendor/lib"),
        );

        assert!(
            model
                .items
                .iter()
                .find(|item| matches!(
                    item,
                    ContextMenuItem::Entry { label, disabled, .. }
                        if label.as_ref() == "Open source repo in new tab" && *disabled
                ))
                .is_some()
        );
    }

    #[test]
    fn model_disables_open_without_stored_source() {
        let repo = repo_with_subtree_source(None);
        let model = model(
            Some(&repo),
            RepoId(1),
            &std::path::PathBuf::from("vendor/lib"),
        );

        assert!(
            model
                .items
                .iter()
                .find(|item| matches!(
                    item,
                    ContextMenuItem::Entry { label, disabled, .. }
                        if label.as_ref() == "Open source repo in new tab" && *disabled
                ))
                .is_some()
        );
    }
}
