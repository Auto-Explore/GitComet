use super::*;

fn conflict_source_icon(selected: bool, icon: &'static str) -> SharedString {
    if selected {
        "icons/check.svg".into()
    } else {
        icon.into()
    }
}

pub(super) fn model(
    conflict_ix: usize,
    has_base: bool,
    is_three_way: bool,
    selected_choices: &[conflict_resolver::ConflictChoice],
    output_line_ix: Option<usize>,
) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header(
        format!("Resolve chunk {}", conflict_ix.saturating_add(1)).into(),
    )];

    if is_three_way {
        items.push(ContextMenuItem::Entry {
            label: "Pick A (Base)".into(),
            icon: Some(conflict_source_icon(
                selected_choices.contains(&conflict_resolver::ConflictChoice::Base),
                "icons/box.svg",
            )),
            shortcut: Some("A / Ctrl+1".into()),
            disabled: !has_base,
            action: Box::new(ContextMenuAction::ConflictResolverPick {
                target: ResolverPickTarget::Chunk {
                    conflict_ix,
                    choice: conflict_resolver::ConflictChoice::Base,
                    output_line_ix,
                },
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Pick B (Local)".into(),
            icon: Some(conflict_source_icon(
                selected_choices.contains(&conflict_resolver::ConflictChoice::Ours),
                "icons/computer.svg",
            )),
            shortcut: Some("B / Ctrl+2".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverPick {
                target: ResolverPickTarget::Chunk {
                    conflict_ix,
                    choice: conflict_resolver::ConflictChoice::Ours,
                    output_line_ix,
                },
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Pick C (Remote)".into(),
            icon: Some(conflict_source_icon(
                selected_choices.contains(&conflict_resolver::ConflictChoice::Theirs),
                "icons/cloud.svg",
            )),
            shortcut: Some("C / Ctrl+3".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverPick {
                target: ResolverPickTarget::Chunk {
                    conflict_ix,
                    choice: conflict_resolver::ConflictChoice::Theirs,
                    output_line_ix,
                },
            }),
        });
    } else {
        // Two-way labels follow the shared keyboard model: B = local
        // (ours), C = remote (theirs) — same as the toolbar pick cluster.
        items.push(ContextMenuItem::Entry {
            label: "Pick B (Local)".into(),
            icon: Some(conflict_source_icon(
                selected_choices.contains(&conflict_resolver::ConflictChoice::Ours),
                "icons/computer.svg",
            )),
            shortcut: Some("B / Ctrl+2".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverPick {
                target: ResolverPickTarget::Chunk {
                    conflict_ix,
                    choice: conflict_resolver::ConflictChoice::Ours,
                    output_line_ix,
                },
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Pick C (Remote)".into(),
            icon: Some(conflict_source_icon(
                selected_choices.contains(&conflict_resolver::ConflictChoice::Theirs),
                "icons/cloud.svg",
            )),
            shortcut: Some("C / Ctrl+3".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverPick {
                target: ResolverPickTarget::Chunk {
                    conflict_ix,
                    choice: conflict_resolver::ConflictChoice::Theirs,
                    output_line_ix,
                },
            }),
        });
    }

    items.push(ContextMenuItem::Entry {
        label: "Keep both (B + C)".into(),
        icon: Some(conflict_source_icon(
            selected_choices.contains(&conflict_resolver::ConflictChoice::Both),
            "icons/copy.svg",
        )),
        shortcut: Some("D".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::ConflictResolverPick {
            target: ResolverPickTarget::Chunk {
                conflict_ix,
                choice: conflict_resolver::ConflictChoice::Both,
                output_line_ix,
            },
        }),
    });

    // Not gated on `selected_choices`: auto-solved regions can be resolved
    // without a plain choice, and un-resolving an unresolved chunk is a no-op.
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Unresolve".into(),
        icon: Some("icons/undo.svg".into()),
        shortcut: Some("U".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::ConflictResolverUnresolve { conflict_ix }),
    });

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_three_way_includes_a_b_c_both_and_unresolve() {
        // Header + A/B/C picks + Keep both + separator + Unresolve.
        let model = super::model(2, true, true, &[], None);
        assert_eq!(model.items.len(), 7);
    }

    #[test]
    fn model_entries_carry_shortcut_hints() {
        let model = super::model(2, true, true, &[], None);
        let shortcuts: Vec<Option<String>> = model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { shortcut, .. } => {
                    Some(shortcut.as_ref().map(|s| s.to_string()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            shortcuts,
            vec![
                Some("A / Ctrl+1".to_string()),
                Some("B / Ctrl+2".to_string()),
                Some("C / Ctrl+3".to_string()),
                Some("D".to_string()),
                Some("U".to_string()),
            ]
        );
    }

    #[test]
    fn model_three_way_disables_a_when_base_missing() {
        let model = super::model(0, false, true, &[], None);
        match &model.items[1] {
            ContextMenuItem::Entry { disabled, .. } => assert!(*disabled),
            _ => panic!("expected entry"),
        }
    }

    #[test]
    fn model_two_way_includes_b_c_both_and_unresolve() {
        // Header + B/C picks + Keep both + separator + Unresolve.
        let model = super::model(1, false, false, &[], Some(3));
        assert_eq!(model.items.len(), 6);
    }

    #[test]
    fn model_two_way_uses_svg_source_icons_when_unselected() {
        let model = super::model(1, false, false, &[], Some(3));
        match &model.items[1] {
            ContextMenuItem::Entry { icon, .. } => {
                assert_eq!(
                    icon.as_ref().map(|s| s.as_ref()),
                    Some("icons/computer.svg")
                );
            }
            _ => panic!("expected entry"),
        }
        match &model.items[2] {
            ContextMenuItem::Entry { icon, .. } => {
                assert_eq!(icon.as_ref().map(|s| s.as_ref()), Some("icons/cloud.svg"));
            }
            _ => panic!("expected entry"),
        }
    }

    #[test]
    fn model_two_way_marks_selected_entry() {
        let selected = vec![conflict_resolver::ConflictChoice::Theirs];
        let model = super::model(1, false, false, &selected, Some(3));
        match &model.items[2] {
            ContextMenuItem::Entry { icon, .. } => {
                assert_eq!(icon.as_ref().map(|s| s.as_ref()), Some("icons/check.svg"));
            }
            _ => panic!("expected entry"),
        }
    }

    #[test]
    fn model_three_way_marks_multiple_selected_entries() {
        let selected = vec![
            conflict_resolver::ConflictChoice::Base,
            conflict_resolver::ConflictChoice::Ours,
        ];
        let model = super::model(1, true, true, &selected, None);
        match &model.items[1] {
            ContextMenuItem::Entry { icon, .. } => {
                assert_eq!(icon.as_ref().map(|s| s.as_ref()), Some("icons/check.svg"));
            }
            _ => panic!("expected entry"),
        }
        match &model.items[2] {
            ContextMenuItem::Entry { icon, .. } => {
                assert_eq!(icon.as_ref().map(|s| s.as_ref()), Some("icons/check.svg"));
            }
            _ => panic!("expected entry"),
        }
    }

    #[test]
    fn model_three_way_uses_svg_source_icons_when_unselected() {
        let model = super::model(1, true, true, &[], None);
        match &model.items[1] {
            ContextMenuItem::Entry { icon, .. } => {
                assert_eq!(icon.as_ref().map(|s| s.as_ref()), Some("icons/box.svg"));
            }
            _ => panic!("expected entry"),
        }
        match &model.items[2] {
            ContextMenuItem::Entry { icon, .. } => {
                assert_eq!(
                    icon.as_ref().map(|s| s.as_ref()),
                    Some("icons/computer.svg")
                );
            }
            _ => panic!("expected entry"),
        }
        match &model.items[3] {
            ContextMenuItem::Entry { icon, .. } => {
                assert_eq!(icon.as_ref().map(|s| s.as_ref()), Some("icons/cloud.svg"));
            }
            _ => panic!("expected entry"),
        }
    }
}
