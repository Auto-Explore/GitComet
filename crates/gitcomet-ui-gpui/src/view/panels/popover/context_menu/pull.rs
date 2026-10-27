use super::*;

pub(super) fn model(this: &PopoverHost) -> ContextMenuModel {
    let active_repo_id = this.active_repo_id();
    let repo_disabled = active_repo_id.is_none();
    // Pull's own reason also covers bare repositories; the bare-action guard skips it.
    let pull_unavailable = this
        .active_repo()
        .and_then(|repo| pull_unavailable_reason(repo, &this.state.large_file_settings));
    let pull_disabled = this.active_repo().is_none() || pull_unavailable.is_some();
    let repo_id = active_repo_id.unwrap_or(RepoId(0));
    let upstream = super::active_branch_tracking_upstream(this);

    let mut model = ContextMenuModel::new(vec![
        ContextMenuItem::Header(super::action_menu_title("Pull", upstream).into()),
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
    ]);
    if let Some(reason) = pull_unavailable {
        for (ix, item) in model.items.iter().enumerate() {
            if let ContextMenuItem::Entry { action, .. } = item
                && matches!(action.as_ref(), ContextMenuAction::Pull { .. })
            {
                model.entry_tooltips.insert(ix, reason.into());
            }
        }
    }
    model.items.extend(super::large_file::pull_items(
        &this.state,
        this.active_repo(),
    ));
    model
        .items
        .extend(super::annex::pull_items(&this.state, this.active_repo()));
    model
}
