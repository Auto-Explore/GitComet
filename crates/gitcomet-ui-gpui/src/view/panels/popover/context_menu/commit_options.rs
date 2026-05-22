use super::*;

fn merge_active(repo: &RepoState) -> bool {
    matches!(&repo.merge_commit_message, Loadable::Ready(Some(_)))
}

fn repo_has_head_commit(repo: &RepoState) -> bool {
    if repo.detached_head_commit.is_some() {
        return true;
    }

    match &repo.head_branch {
        Loadable::Ready(head) if head == "HEAD" => true,
        Loadable::Ready(head) => match &repo.branches {
            Loadable::Ready(branches) => branches.iter().any(|branch| branch.name == *head),
            _ => true,
        },
        _ => true,
    }
}

fn can_amend(repo: Option<&RepoState>) -> bool {
    let Some(repo) = repo else {
        return false;
    };

    repo.commit_in_flight == 0
        && !merge_active(repo)
        && !matches!(repo.rebase_in_progress, Loadable::Ready(true))
        && repo_has_head_commit(repo)
}

pub(super) fn model(this: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    model_for_state(
        repo,
        this.commit_amend_enabled,
        this.commit_push_after_enabled,
    )
}

fn model_for_state(
    repo: Option<&RepoState>,
    commit_amend_enabled: bool,
    commit_push_after_enabled: bool,
) -> ContextMenuModel {
    let check = |enabled: bool| enabled.then_some("icons/check.svg".into());

    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Commit options".into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Amend previous commit".into(),
            icon: check(commit_amend_enabled),
            shortcut: Some("A".into()),
            disabled: !commit_amend_enabled && !can_amend(repo),
            action: Box::new(ContextMenuAction::SetCommitAmendEnabled {
                enabled: !commit_amend_enabled,
            }),
        },
        ContextMenuItem::Entry {
            label: "Push after commit".into(),
            icon: check(commit_push_after_enabled),
            shortcut: Some("P".into()),
            disabled: repo.is_none(),
            action: Box::new(ContextMenuAction::SetCommitPushAfterEnabled {
                enabled: !commit_push_after_enabled,
            }),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Branch, CommitId, LogPage, RepoSpec};
    use gitcomet_state::model::{Loadable, RepoId, RepoState};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn repo_state() -> RepoState {
        RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        )
    }

    fn branch(name: &str, target: &str) -> Branch {
        Branch {
            name: name.to_string(),
            target: CommitId(target.into()),
            upstream: None,
            divergence: None,
        }
    }

    fn amend_option_disabled(model: &ContextMenuModel) -> bool {
        model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry {
                    label, disabled, ..
                } if label.as_ref() == "Amend previous commit" => Some(*disabled),
                _ => None,
            })
            .expect("amend option should exist")
    }

    #[test]
    fn model_does_not_include_previous_commit_messages() {
        let model = model_for_state(None, false, false);

        assert!(!model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry {
                action,
                ..
            } if matches!(action.as_ref(), ContextMenuAction::UseCommitMessage { .. })
        )));
        assert_eq!(model.items.len(), 4);
    }

    #[test]
    fn amend_option_enabled_when_filtered_log_is_empty_but_head_branch_exists() {
        let mut repo = repo_state();
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "abc123")]));
        repo.log = Loadable::Ready(Arc::new(LogPage {
            commits: Vec::new(),
            next_cursor: None,
        }));

        let model = model_for_state(Some(&repo), false, false);

        assert!(!amend_option_disabled(&model));
    }

    #[test]
    fn amend_option_disabled_on_unborn_head_branch() {
        let mut repo = repo_state();
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.branches = Loadable::Ready(Arc::new(Vec::new()));
        repo.log = Loadable::Ready(Arc::new(LogPage {
            commits: Vec::new(),
            next_cursor: None,
        }));

        let model = model_for_state(Some(&repo), false, false);

        assert!(amend_option_disabled(&model));
    }
}
