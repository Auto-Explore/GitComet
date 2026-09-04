use super::*;

pub(super) fn model(this: &PopoverHost) -> ContextMenuModel {
    let active_repo_id = this.active_repo_id();
    let repo_disabled = active_repo_id.is_none();
    let pull_disabled = this
        .active_repo()
        .is_none_or(|repo| !matches!(pull_request(repo), PullRequest::Pull));
    let repo_id = active_repo_id.unwrap_or(RepoId(0));
    let tracking_branch_name = super::active_branch_tracking_upstream_name(this);

    ContextMenuModel::new(vec![
        ContextMenuItem::Header(
            super::action_menu_title("Pull", tracking_branch_name.as_deref()).into(),
        ),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Pull (default)".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: None,
            disabled: pull_disabled,
            action: Box::new(ContextMenuAction::Pull {
                repo_id,
                mode: PullMode::Default,
            }),
        },
        ContextMenuItem::Entry {
            label: "Pull (fast-forward if possible)".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("F".into()),
            disabled: pull_disabled,
            action: Box::new(ContextMenuAction::Pull {
                repo_id,
                mode: PullMode::FastForwardIfPossible,
            }),
        },
        ContextMenuItem::Entry {
            label: "Pull (fast-forward only)".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("O".into()),
            disabled: pull_disabled,
            action: Box::new(ContextMenuAction::Pull {
                repo_id,
                mode: PullMode::FastForwardOnly,
            }),
        },
        ContextMenuItem::Entry {
            label: "Pull (rebase)".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("R".into()),
            disabled: pull_disabled,
            action: Box::new(ContextMenuAction::Pull {
                repo_id,
                mode: PullMode::Rebase,
            }),
        },
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Fetch all".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("A".into()),
            disabled: repo_disabled,
            action: Box::new(ContextMenuAction::FetchAll { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "Prune merged branches".into(),
            icon: Some("icons/broom.svg".into()),
            shortcut: None,
            disabled: repo_disabled,
            action: Box::new(ContextMenuAction::PruneMergedBranches { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "Prune local tags".into(),
            icon: Some("icons/tag.svg".into()),
            shortcut: None,
            disabled: repo_disabled,
            action: Box::new(ContextMenuAction::PruneLocalTags { repo_id }),
        },
    ])
}
