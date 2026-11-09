use super::*;

fn can_amend(repo: Option<&RepoState>) -> bool {
    let Some(repo) = repo else {
        return false;
    };

    repo.commit_in_flight == 0 && amend_allowed(repo)
}

pub(super) fn model(this: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    model_for_state(
        repo,
        this.commit_amend_enabled,
        this.commit_push_after_enabled,
        this.commit_skip_hooks_enabled,
    )
}

const SKIP_HOOKS_LABEL: &str = "Skip Git hooks";
const SKIP_HOOKS_TOOLTIP: &str = "Commit and push with --no-verify: the pre-commit, commit-msg and \
     pre-push hooks do not run. Applies until turned off or the window closes.";

fn model_for_state(
    repo: Option<&RepoState>,
    commit_amend_enabled: bool,
    commit_push_after_enabled: bool,
    commit_skip_hooks_enabled: bool,
) -> ContextMenuModel {
    let check = |enabled: bool| enabled.then_some("icons/check.svg".into());

    let mut model = ContextMenuModel::new(vec![
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
        ContextMenuItem::Entry {
            label: SKIP_HOOKS_LABEL.into(),
            icon: check(commit_skip_hooks_enabled),
            shortcut: Some("H".into()),
            disabled: repo.is_none(),
            action: Box::new(ContextMenuAction::SetCommitSkipHooksEnabled {
                enabled: !commit_skip_hooks_enabled,
            }),
        },
    ]);
    model
        .entry_tooltips
        .insert(model.items.len() - 1, SKIP_HOOKS_TOOLTIP.into());
    model
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

    fn skip_hooks_entry(model: &ContextMenuModel) -> (usize, bool, bool, bool) {
        model
            .items
            .iter()
            .enumerate()
            .find_map(|(ix, item)| match item {
                ContextMenuItem::Entry {
                    label,
                    icon,
                    disabled,
                    action,
                    ..
                } if label.as_ref() == SKIP_HOOKS_LABEL => {
                    let ContextMenuAction::SetCommitSkipHooksEnabled { enabled } = action.as_ref()
                    else {
                        panic!("skip hooks entry should toggle the option");
                    };
                    Some((ix, icon.is_some(), *disabled, *enabled))
                }
                _ => None,
            })
            .expect("skip hooks option should exist")
    }

    #[test]
    fn skip_hooks_option_toggles_and_shows_its_state() {
        let repo = repo_state();

        let model = model_for_state(Some(&repo), false, false, false);
        let (ix, checked, disabled, enables) = skip_hooks_entry(&model);
        assert!(!checked && !disabled && enables);
        let tooltip = model.entry_tooltips.get(&ix).expect("explains the option");
        assert!(tooltip.contains("--no-verify"));

        let model = model_for_state(Some(&repo), false, true, true);
        let (_, checked, disabled, enables) = skip_hooks_entry(&model);
        assert!(checked && !disabled && !enables);

        let (_, _, disabled, _) = skip_hooks_entry(&model_for_state(None, false, false, false));
        assert!(disabled, "no repository, nothing to commit");
    }

    #[test]
    fn model_does_not_include_previous_commit_messages() {
        let model = model_for_state(None, false, false, false);

        assert!(!model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry {
                action,
                ..
            } if matches!(action.as_ref(), ContextMenuAction::UseCommitMessage { .. })
        )));
        assert_eq!(model.items.len(), 5);
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

        let model = model_for_state(Some(&repo), false, false, false);

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

        let model = model_for_state(Some(&repo), false, false, false);

        assert!(amend_option_disabled(&model));
    }
}
