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
            shortcut: None,
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
            shortcut: None,
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
            shortcut: None,
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
        items.push(ContextMenuItem::Entry {
            label: "Pick A".into(),
            icon: Some(conflict_source_icon(
                selected_choices.contains(&conflict_resolver::ConflictChoice::Ours),
                "icons/computer.svg",
            )),
            shortcut: None,
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
            label: "Pick B".into(),
            icon: Some(conflict_source_icon(
                selected_choices.contains(&conflict_resolver::ConflictChoice::Theirs),
                "icons/cloud.svg",
            )),
            shortcut: None,
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

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_three_way_includes_a_b_and_c() {
        let model = super::model(2, true, true, &[], None);
        assert_eq!(model.items.len(), 4);
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
    fn model_two_way_includes_only_a_and_b() {
        let model = super::model(1, false, false, &[], Some(3));
        assert_eq!(model.items.len(), 3);
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
