use super::*;

pub(super) fn model(repo_id: RepoId) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header("Subtrees".into())];
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Add subtree…".into(),
        icon: Some("icons/plus.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(repo_id, SubtreePopoverKind::AddPrompt),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Refresh subtrees".into(),
        icon: Some("icons/refresh.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::LoadSubtrees { repo_id }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Pull subtree…".into(),
        icon: Some("icons/arrow_down.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(repo_id, SubtreePopoverKind::PullPicker),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Push subtree…".into(),
        icon: Some("icons/arrow_up.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(repo_id, SubtreePopoverKind::PushPicker),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Merge subtree…".into(),
        icon: Some("icons/swap.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(
                repo_id,
                SubtreePopoverKind::MergePrompt { initial_path: None },
            ),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Split subtree…".into(),
        icon: Some("icons/git_branch.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(
                repo_id,
                SubtreePopoverKind::SplitPrompt { initial_path: None },
            ),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Open subtree source repo…".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(repo_id, SubtreePopoverKind::OpenPicker),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Remove subtree…".into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::subtree(repo_id, SubtreePopoverKind::RemovePicker),
        }),
    });

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_includes_refresh_and_omits_reveal_entry() {
        let model = model(RepoId(1));

        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, .. } if label.as_ref() == "Refresh subtrees"
            )
        }));
        assert!(model.items.iter().all(|item| {
            !matches!(
                item,
                ContextMenuItem::Entry { label, .. } if label.as_ref() == "Reveal subtree…"
            )
        }));
    }

    #[test]
    fn model_uses_registered_subtree_icons() {
        let model = model(RepoId(1));

        let icons = model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, icon, .. } => Some((
                    label.as_ref().to_string(),
                    icon.as_ref().map(|icon| icon.as_ref()),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(
            icons
                .iter()
                .any(|(label, icon)| label == "Pull subtree…"
                    && *icon == Some("icons/arrow_down.svg"))
        );
        assert!(
            icons.iter().any(
                |(label, icon)| label == "Push subtree…" && *icon == Some("icons/arrow_up.svg")
            )
        );
        assert!(
            icons
                .iter()
                .any(|(label, icon)| label == "Merge subtree…" && *icon == Some("icons/swap.svg"))
        );
        assert!(icons.iter().any(
            |(label, icon)| label == "Split subtree…" && *icon == Some("icons/git_branch.svg")
        ));
    }

    #[test]
    fn model_opens_editable_split_prompt() {
        let model = model(RepoId(1));

        let split_action = model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == "Split subtree…" =>
                {
                    Some((**action).clone())
                }
                _ => None,
            })
            .expect("expected split entry");

        assert!(matches!(
            split_action,
            ContextMenuAction::OpenPopover {
                kind: PopoverKind::Repo {
                    repo_id: RepoId(1),
                    kind: RepoPopoverKind::Subtree(SubtreePopoverKind::SplitPrompt {
                        initial_path: None,
                    }),
                },
            }
        ));
    }

    #[test]
    fn model_opens_blank_merge_prompt() {
        let model = model(RepoId(1));

        let merge_action = model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == "Merge subtree…" =>
                {
                    Some((**action).clone())
                }
                _ => None,
            })
            .expect("expected merge entry");

        assert!(matches!(
            merge_action,
            ContextMenuAction::OpenPopover {
                kind: PopoverKind::Repo {
                    repo_id: RepoId(1),
                    kind: RepoPopoverKind::Subtree(SubtreePopoverKind::MergePrompt {
                        initial_path: None,
                    }),
                },
            }
        ));
    }
}
