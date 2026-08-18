use super::*;

/// Dropdown opened from the action bar's "Automations" button.
///
/// Kept as a list rather than a single button so new automations have a
/// home: adding one is a new [`ContextMenuItem::Entry`] here, not a new
/// top-level action-bar button.
pub(super) fn model(repo_id: RepoId) -> ContextMenuModel {
    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Automations".into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Branch extractor".into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopoverCentered {
                kind: PopoverKind::CherryPickRangePrompt {
                    repo_id,
                    prefill_source: None,
                    prefill_range: None,
                    prefill_base: None,
                },
            }),
        },
    ])
    .with_entry_tooltips(std::collections::HashMap::from([(
        2usize,
        "Copy a range of commits from one branch onto a new branch".into(),
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_lists_branch_extractor_as_first_entry() {
        let repo_id = RepoId(1);
        let items = model(repo_id).items;

        assert!(
            matches!(&items[0], ContextMenuItem::Header(label) if label.as_ref() == "Automations")
        );
        assert!(matches!(&items[1], ContextMenuItem::Separator));
        match &items[2] {
            ContextMenuItem::Entry { label, action, .. } => {
                assert_eq!(label.as_ref(), "Branch extractor");
                match action.as_ref() {
                    ContextMenuAction::OpenPopoverCentered {
                        kind: PopoverKind::CherryPickRangePrompt { repo_id: rid, .. },
                    } => assert_eq!(*rid, repo_id),
                    _ => panic!("expected OpenPopoverCentered(CherryPickRangePrompt)"),
                }
            }
            _ => panic!("expected an entry"),
        }
    }
}
