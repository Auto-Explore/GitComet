use super::*;

pub(super) fn model(repo_id: RepoId, index: usize, message: &str) -> ContextMenuModel {
    let reference = format!("stash@{{{index}}}");
    let summary = if message.is_empty() {
        reference.clone()
    } else {
        format!("{reference} {message}")
    };

    let items = vec![
        ContextMenuItem::Header("Stash".into()),
        ContextMenuItem::Label(summary.into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Create branch from stash".into(),
            icon: Some("icons/git_branch.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::CreateBranchFromStashPrompt {
                    repo_id,
                    index,
                    message: message.to_owned(),
                },
            }),
        },
        ContextMenuItem::Entry {
            label: "Apply stash".into(),
            icon: Some("icons/refresh.svg".into()),
            shortcut: Some("A".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::ApplyStash { repo_id, index }),
        },
        ContextMenuItem::Entry {
            label: "Pop stash".into(),
            icon: Some("icons/arrow_up.svg".into()),
            shortcut: Some("P".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::PopStash { repo_id, index }),
        },
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Drop stash…".into(),
            icon: Some("icons/trash.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::DropStashConfirm {
                repo_id,
                index,
                message: message.to_owned(),
            }),
        },
    ];

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_includes_apply_pop_and_drop_entries() {
        let repo_id = RepoId(11);
        let model = model(repo_id, 3, "WIP");

        let apply_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Apply stash" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            apply_action,
            Some(ContextMenuAction::ApplyStash {
                repo_id: rid,
                index: 3
            }) if rid == repo_id
        ));

        let create_branch_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Create branch from stash" =>
            {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            create_branch_action,
            Some(ContextMenuAction::OpenPopover {
                kind: PopoverKind::CreateBranchFromStashPrompt {
                    repo_id: rid,
                    index: 3,
                    message,
                },
            }) if rid == repo_id && message == "WIP"
        ));

        let pop_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Pop stash" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            pop_action,
            Some(ContextMenuAction::PopStash {
                repo_id: rid,
                index: 3
            }) if rid == repo_id
        ));

        let drop_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Drop stash…" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            drop_action,
            Some(ContextMenuAction::DropStashConfirm {
                repo_id: rid,
                index: 3,
                message,
            }) if rid == repo_id && message == "WIP"
        ));
    }
}
