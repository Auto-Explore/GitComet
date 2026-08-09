use super::*;

use crate::view::shortcut_labels::secondary_shortcut;

pub(super) fn model(host: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let workdir = host
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| repo.spec.workdir.clone());
    model_for_state(host.state.as_ref(), repo_id, workdir)
}

fn model_for_state(
    state: &AppState,
    repo_id: RepoId,
    workdir: Option<std::path::PathBuf>,
) -> ContextMenuModel {
    let Some(repo_ix) = state.repos.iter().position(|repo| repo.id == repo_id) else {
        return ContextMenuModel::new(Vec::new());
    };

    let close_to_right: Vec<RepoId> = state
        .repos
        .iter()
        .skip(repo_ix + 1)
        .map(|repo| repo.id)
        .collect();
    let close_others: Vec<RepoId> = state
        .repos
        .iter()
        .filter_map(|repo| (repo.id != repo_id).then_some(repo.id))
        .collect();
    let activate_after_close_to_right = state
        .active_repo
        .filter(|active_repo| close_to_right.contains(active_repo))
        .map(|_| repo_id);

    let mut items = vec![ContextMenuItem::Entry {
        label: "Activate".into(),
        icon: Some("icons/check.svg".into()),
        shortcut: None,
        disabled: state.active_repo == Some(repo_id),
        action: Box::new(ContextMenuAction::ActivateRepo { repo_id }),
    }];

    if let Some(ref workdir) = workdir {
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
    }

    if crate::external_editor::configured_setting().is_some()
        && let Some(ref workdir) = workdir
    {
        items.push(ContextMenuItem::Entry {
            label: "Open in code editor".into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: Some(secondary_shortcut("Shift+E").into()),
            disabled: false,
            action: Box::new(ContextMenuAction::OpenInCodeEditor {
                repo_id: None,
                path: workdir.clone(),
            }),
        });
    }

    items.push(ContextMenuItem::Separator);
    items.extend_from_slice(&[
        ContextMenuItem::Entry {
            label: "Close".into(),
            icon: Some("icons/repo_tab_close.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CloseRepo { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "Close repositories to the right".into(),
            icon: Some("icons/arrow_right.svg".into()),
            shortcut: None,
            disabled: close_to_right.is_empty(),
            action: Box::new(ContextMenuAction::CloseRepos {
                repo_ids: close_to_right,
                activate_after: activate_after_close_to_right,
            }),
        },
        ContextMenuItem::Entry {
            label: "Close other repositories".into(),
            icon: Some("icons/swap.svg".into()),
            shortcut: None,
            disabled: close_others.is_empty(),
            action: Box::new(ContextMenuAction::CloseRepos {
                repo_ids: close_others,
                activate_after: Some(repo_id),
            }),
        },
    ]);

    ContextMenuModel::new(items).with_shortcut_keycaps()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_state::model::RepoState;
    use std::path::PathBuf;

    fn state_with_repo_tabs(active_repo: RepoId, repo_count: u64) -> AppState {
        let mut state = AppState {
            active_repo: Some(active_repo),
            ..AppState::default()
        };
        for ix in 1..=repo_count {
            state.repos.push(RepoState::new_opening(
                RepoId(ix),
                RepoSpec {
                    workdir: PathBuf::from(format!("/tmp/repo-tab-menu-{ix}")),
                },
            ));
        }
        state
    }

    fn entry_with_label<'a>(model: &'a ContextMenuModel, expected: &str) -> &'a ContextMenuItem {
        model
            .items
            .iter()
            .find(|item| {
                matches!(
                    item,
                    ContextMenuItem::Entry { label, .. } if label.as_ref() == expected
                )
            })
            .unwrap_or_else(|| panic!("expected {expected} menu item"))
    }

    fn entry_action<'a>(
        model: &'a ContextMenuModel,
        expected: &str,
    ) -> (bool, &'a ContextMenuAction) {
        let ContextMenuItem::Entry {
            disabled, action, ..
        } = entry_with_label(model, expected)
        else {
            panic!("expected {expected} menu item to be an entry");
        };
        (*disabled, action.as_ref())
    }

    #[test]
    fn activate_entry_activates_inactive_repo_tab() {
        let state = state_with_repo_tabs(RepoId(1), 3);
        let model = model_for_state(&state, RepoId(2), None);

        let (disabled, action) = entry_action(&model, "Activate");

        assert!(!disabled);
        assert!(matches!(
            action,
            ContextMenuAction::ActivateRepo { repo_id } if *repo_id == RepoId(2)
        ));
    }

    #[test]
    fn activate_entry_is_disabled_for_active_repo_tab() {
        let state = state_with_repo_tabs(RepoId(2), 3);
        let model = model_for_state(&state, RepoId(2), None);

        let (disabled, action) = entry_action(&model, "Activate");

        assert!(disabled);
        assert!(matches!(
            action,
            ContextMenuAction::ActivateRepo { repo_id } if *repo_id == RepoId(2)
        ));
    }

    #[test]
    fn close_repo_entry_uses_repo_tab_close_icon() {
        let state = state_with_repo_tabs(RepoId(1), 3);
        let model = model_for_state(&state, RepoId(2), None);

        let ContextMenuItem::Entry {
            icon,
            disabled,
            action,
            ..
        } = entry_with_label(&model, "Close")
        else {
            panic!("expected Close menu item to be an entry");
        };

        assert_eq!(
            icon.as_ref().map(|icon| icon.as_ref()),
            Some("icons/repo_tab_close.svg")
        );
        assert!(!disabled);
        assert!(matches!(
            action.as_ref(),
            ContextMenuAction::CloseRepo { repo_id } if *repo_id == RepoId(2)
        ));
    }

    #[test]
    fn open_repository_location_entry_targets_the_repository_workdir() {
        let state = state_with_repo_tabs(RepoId(1), 3);
        let workdir = PathBuf::from("/tmp/repo-tab-menu-2");
        let model = model_for_state(&state, RepoId(2), Some(workdir.clone()));

        let (disabled, action) = entry_action(&model, "Open repository location");

        assert!(!disabled);
        assert!(matches!(
            action,
            ContextMenuAction::OpenRepositoryLocation { path } if path == &workdir
        ));
    }

    #[test]
    fn repo_tab_menu_uses_shared_shortcut_keycaps() {
        let state = state_with_repo_tabs(RepoId(1), 3);
        let model = model_for_state(
            &state,
            RepoId(2),
            Some(PathBuf::from("/tmp/repo-tab-menu-2")),
        );

        assert!(model.shortcut_keycaps);
    }

    #[test]
    fn close_right_entry_targets_only_repos_to_the_right() {
        let state = state_with_repo_tabs(RepoId(3), 3);
        let model = model_for_state(&state, RepoId(2), None);

        let (disabled, action) = entry_action(&model, "Close repositories to the right");

        assert!(!disabled);
        let ContextMenuAction::CloseRepos {
            repo_ids,
            activate_after,
        } = action
        else {
            panic!("expected Close repositories to the right to close multiple repos");
        };
        assert_eq!(repo_ids, &vec![RepoId(3)]);
        assert_eq!(*activate_after, Some(RepoId(2)));
    }

    #[test]
    fn close_right_entry_is_disabled_for_last_repo_tab() {
        let state = state_with_repo_tabs(RepoId(2), 3);
        let model = model_for_state(&state, RepoId(3), None);

        let (disabled, action) = entry_action(&model, "Close repositories to the right");

        assert!(disabled);
        let ContextMenuAction::CloseRepos {
            repo_ids,
            activate_after,
        } = action
        else {
            panic!("expected Close repositories to the right to close multiple repos");
        };
        assert!(repo_ids.is_empty());
        assert_eq!(*activate_after, None);
    }

    #[test]
    fn close_other_repositories_entry_targets_every_repo_except_selected() {
        let state = state_with_repo_tabs(RepoId(1), 3);
        let model = model_for_state(&state, RepoId(2), None);

        let (disabled, action) = entry_action(&model, "Close other repositories");

        assert!(!disabled);
        let ContextMenuAction::CloseRepos {
            repo_ids,
            activate_after,
        } = action
        else {
            panic!("expected Close other repositories to close multiple repos");
        };
        assert_eq!(repo_ids, &vec![RepoId(1), RepoId(3)]);
        assert_eq!(*activate_after, Some(RepoId(2)));
    }

    #[test]
    fn close_other_repositories_entry_is_disabled_for_single_repo_tab() {
        let state = state_with_repo_tabs(RepoId(1), 1);
        let model = model_for_state(&state, RepoId(1), None);

        let (disabled, action) = entry_action(&model, "Close other repositories");

        assert!(disabled);
        let ContextMenuAction::CloseRepos {
            repo_ids,
            activate_after,
        } = action
        else {
            panic!("expected Close other repositories to close multiple repos");
        };
        assert!(repo_ids.is_empty());
        assert_eq!(*activate_after, Some(RepoId(1)));
    }

    #[test]
    fn missing_repo_tab_returns_empty_menu_model() {
        let state = state_with_repo_tabs(RepoId(1), 3);

        assert!(model_for_state(&state, RepoId(99), None).items.is_empty());
    }
}
