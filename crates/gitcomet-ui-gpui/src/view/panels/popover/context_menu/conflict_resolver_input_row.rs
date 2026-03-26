use super::*;

pub(super) fn model(
    line_label: &SharedString,
    line_target: &ResolverPickTarget,
    chunk_label: &SharedString,
    chunk_target: &ResolverPickTarget,
) -> ContextMenuModel {
    ContextMenuModel::new(vec![
        ContextMenuItem::Entry {
            label: line_label.clone(),
            icon: Some("icons/plus.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverPick {
                target: line_target.clone(),
            }),
        },
        ContextMenuItem::Entry {
            label: chunk_label.clone(),
            icon: Some("icons/box.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverPick {
                target: chunk_target.clone(),
            }),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_builds_line_and_chunk_entries() {
        let line_label: SharedString = "Select line (12)".into();
        let chunk_label: SharedString = "Select chunk (Ln 10 - 14)".into();
        let line_target = ResolverPickTarget::TwoWaySplitLine {
            row_ix: 7,
            side: ConflictPickSide::Ours,
        };
        let chunk_target = ResolverPickTarget::Chunk {
            conflict_ix: 3,
            choice: conflict_resolver::ConflictChoice::Ours,
            output_line_ix: None,
        };

        let model = super::model(&line_label, &line_target, &chunk_label, &chunk_target);

        assert_eq!(model.items.len(), 2);

        match &model.items[0] {
            ContextMenuItem::Entry {
                label,
                disabled,
                action,
                ..
            } => {
                assert_eq!(label, &line_label);
                assert!(!*disabled);
                assert!(matches!(
                    action.as_ref(),
                    ContextMenuAction::ConflictResolverPick {
                        target: ResolverPickTarget::TwoWaySplitLine {
                            row_ix: 7,
                            side: ConflictPickSide::Ours,
                        }
                    }
                ));
            }
            _ => panic!("expected entry item for line action"),
        }

        match &model.items[1] {
            ContextMenuItem::Entry {
                label,
                disabled,
                action,
                ..
            } => {
                assert_eq!(label, &chunk_label);
                assert!(!*disabled);
                assert!(matches!(
                    action.as_ref(),
                    ContextMenuAction::ConflictResolverPick {
                        target: ResolverPickTarget::Chunk {
                            conflict_ix: 3,
                            choice: conflict_resolver::ConflictChoice::Ours,
                            output_line_ix: None,
                        }
                    }
                ));
            }
            _ => panic!("expected entry item for chunk action"),
        }
    }
}
