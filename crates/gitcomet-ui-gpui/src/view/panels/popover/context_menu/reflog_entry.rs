use super::*;

/// The reflog panel's right-click menu: the same three reset actions the
/// history log's commit context menu offers (see `commit.rs`), targeting the
/// commit the clicked reflog entry points at rather than a history row.
pub(super) fn model(
    repo_id: RepoId,
    selector: &SharedString,
    target: &CommitId,
) -> ContextMenuModel {
    let sha = target.as_ref().to_string();
    let short: SharedString = sha.get(0..8).unwrap_or(&sha).to_string().into();

    let mut items = vec![
        ContextMenuItem::Header(components::ContextMenuText::new(selector.clone())),
        ContextMenuItem::Label(components::ContextMenuText::new(format!("commit {short}"))),
        ContextMenuItem::Separator,
    ];

    for (label, icon, mode) in [
        (
            "Reset (--soft) to here",
            "icons/refresh.svg",
            ResetMode::Soft,
        ),
        (
            "Reset (--mixed) to here",
            "icons/refresh.svg",
            ResetMode::Mixed,
        ),
        (
            "Reset (--hard) to here",
            "icons/refresh.svg",
            ResetMode::Hard,
        ),
    ] {
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some(icon.into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::ResetPrompt {
                    repo_id,
                    target: sha.clone(),
                    mode,
                },
            }),
        });
    }

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_labels(model: &ContextMenuModel) -> Vec<String> {
        model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, .. } => Some(label.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn model_offers_the_three_reset_modes() {
        let commit_id = CommitId("deadbeefcafe".into());
        let selector: SharedString = "HEAD@{2}".into();
        let model = model(RepoId(1), &selector, &commit_id);

        assert_eq!(
            entry_labels(&model),
            vec![
                "Reset (--soft) to here",
                "Reset (--mixed) to here",
                "Reset (--hard) to here",
            ]
        );
    }

    #[test]
    fn each_entry_targets_the_reflog_entrys_commit() {
        let commit_id = CommitId("deadbeefcafe".into());
        let selector: SharedString = "HEAD@{2}".into();
        let model = model(RepoId(1), &selector, &commit_id);

        let modes: Vec<ResetMode> = model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { action, .. } => match action.as_ref() {
                    ContextMenuAction::OpenPopover {
                        kind:
                            PopoverKind::ResetPrompt {
                                repo_id,
                                target,
                                mode,
                            },
                    } => {
                        assert_eq!(*repo_id, RepoId(1));
                        assert_eq!(target, "deadbeefcafe");
                        Some(*mode)
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            modes,
            vec![ResetMode::Soft, ResetMode::Mixed, ResetMode::Hard]
        );
    }
}
