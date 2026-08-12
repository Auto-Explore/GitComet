use super::*;

fn conflict_source_icon(selected: bool, icon: &'static str) -> SharedString {
    if selected {
        "icons/check.svg".into()
    } else {
        icon.into()
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn model(
    conflict_ix: usize,
    has_base: bool,
    is_three_way: bool,
    selected_choices: &[conflict_resolver::ConflictChoice],
    output_line_ix: Option<usize>,
    split_selection_rows: Option<usize>,
    join_previous_region: Option<ConflictResolverJoinTarget>,
    join_next_region: Option<ConflictResolverJoinTarget>,
    alignment_marked_columns: usize,
    has_manual_alignments: bool,
    output_is_protected: bool,
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
            disabled: !has_base || output_is_protected,
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
            disabled: output_is_protected,
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
            disabled: output_is_protected,
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
            label: "Pick A (Local)".into(),
            icon: Some(conflict_source_icon(
                selected_choices.contains(&conflict_resolver::ConflictChoice::Ours),
                "icons/computer.svg",
            )),
            shortcut: Some("A / Ctrl+1".into()),
            disabled: output_is_protected,
            action: Box::new(ContextMenuAction::ConflictResolverPick {
                target: ResolverPickTarget::Chunk {
                    conflict_ix,
                    choice: conflict_resolver::ConflictChoice::Ours,
                    output_line_ix,
                },
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Pick B (Remote)".into(),
            icon: Some(conflict_source_icon(
                selected_choices.contains(&conflict_resolver::ConflictChoice::Theirs),
                "icons/cloud.svg",
            )),
            shortcut: Some("B / Ctrl+2".into()),
            disabled: output_is_protected,
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
        label: if is_three_way {
            "Keep both (B + C)".into()
        } else {
            "Keep both (A + B)".into()
        },
        icon: Some(conflict_source_icon(
            selected_choices.contains(&conflict_resolver::ConflictChoice::Both),
            "icons/copy.svg",
        )),
        shortcut: Some(if is_three_way {
            "D".into()
        } else {
            "C / Ctrl+3".into()
        }),
        disabled: output_is_protected,
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
        disabled: output_is_protected,
        action: Box::new(ContextMenuAction::ConflictResolverUnresolve { conflict_ix }),
    });

    if split_selection_rows.is_some()
        || join_previous_region.is_some()
        || join_next_region.is_some()
    {
        items.push(ContextMenuItem::Separator);
    }
    if let Some(rows) = split_selection_rows {
        items.push(ContextMenuItem::Entry {
            label: "Split selection into own conflict".into(),
            icon: Some("icons/unlink.svg".into()),
            shortcut: Some(
                if rows == 1 {
                    "1 row".to_string()
                } else {
                    format!("{rows} rows")
                }
                .into(),
            ),
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverSplitSelection),
        });
    }
    if let Some(target) = join_previous_region {
        items.push(ContextMenuItem::Entry {
            label: "Join with previous".into(),
            icon: Some("icons/link.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverJoinRegions { target }),
        });
    }
    if let Some(target) = join_next_region {
        items.push(ContextMenuItem::Entry {
            label: "Join with next".into(),
            icon: Some("icons/link.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverJoinRegions { target }),
        });
    }

    // kdiff3 manual diff help. The align entry appears only once lines are
    // marked (Alt+click in the source columns); the clear entry only once
    // something is pinned.
    if alignment_marked_columns > 0 || has_manual_alignments {
        items.push(ContextMenuItem::Separator);
    }
    if alignment_marked_columns > 0 {
        items.push(ContextMenuItem::Entry {
            label: "Align marked lines".into(),
            icon: Some("icons/link.svg".into()),
            shortcut: Some("Ctrl+Y".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverAlignManually),
        });
    }
    if has_manual_alignments {
        items.push(ContextMenuItem::Entry {
            label: "Clear manual alignments".into(),
            icon: Some("icons/undo.svg".into()),
            shortcut: Some("Ctrl+Shift+Y".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::ConflictResolverClearManualAlignments),
        });
    }

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The unprotected menu, which is what every case below but
    /// `protected_output_greys_out_every_resolution_entry` is about.
    #[allow(clippy::too_many_arguments)]
    fn model(
        conflict_ix: usize,
        has_base: bool,
        is_three_way: bool,
        selected_choices: &[conflict_resolver::ConflictChoice],
        output_line_ix: Option<usize>,
        split_selection_rows: Option<usize>,
        join_previous_region: Option<ConflictResolverJoinTarget>,
        join_next_region: Option<ConflictResolverJoinTarget>,
        alignment_marked_columns: usize,
        has_manual_alignments: bool,
    ) -> ContextMenuModel {
        super::model(
            conflict_ix,
            has_base,
            is_three_way,
            selected_choices,
            output_line_ix,
            split_selection_rows,
            join_previous_region,
            join_next_region,
            alignment_marked_columns,
            has_manual_alignments,
            false,
        )
    }

    fn join_target(first_region_index: usize) -> ConflictResolverJoinTarget {
        ConflictResolverJoinTarget {
            repo_id: gitcomet_state::model::RepoId(1),
            path: std::path::PathBuf::from("file.txt").into(),
            conflict_rev: 7,
            first_region_index,
        }
    }

    #[test]
    fn model_three_way_includes_a_b_c_both_and_unresolve() {
        // Header + A/B/C picks + Keep both + separator + Unresolve.
        let model = model(2, true, true, &[], None, None, None, None, 0, false);
        assert_eq!(model.items.len(), 7);
    }

    #[test]
    fn model_entries_carry_shortcut_hints() {
        let model = model(2, true, true, &[], None, None, None, None, 0, false);
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

    /// A protected output rejects every resolution action on the way in, so the
    /// entries have to say so. Left enabled they looked live and did nothing.
    #[test]
    fn protected_output_greys_out_every_resolution_entry() {
        let protected = super::model(
            0,
            true,
            true,
            &[],
            None,
            Some(2),
            None,
            None,
            0,
            false,
            true,
        );
        let resolution_entries: Vec<(&str, bool)> = protected
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry {
                    label,
                    action,
                    disabled,
                    ..
                } => matches!(
                    action.as_ref(),
                    ContextMenuAction::ConflictResolverPick { .. }
                        | ContextMenuAction::ConflictResolverUnresolve { .. }
                )
                .then_some((label.as_ref(), *disabled)),
                _ => None,
            })
            .collect();

        assert_eq!(resolution_entries.len(), 5);
        assert!(
            resolution_entries.iter().all(|(_, disabled)| *disabled),
            "every pick and unresolve entry must grey out: {resolution_entries:?}"
        );

        // Structural entries are unaffected: splitting a chunk rewrites the
        // in-memory projection, not the protected output buffer.
        assert!(protected.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { action, disabled, .. }
                if matches!(**action, ContextMenuAction::ConflictResolverSplitSelection)
                    && !*disabled
        )));
    }

    #[test]
    fn model_three_way_disables_a_when_base_missing() {
        let model = model(0, false, true, &[], None, None, None, None, 0, false);
        match &model.items[1] {
            ContextMenuItem::Entry { disabled, .. } => assert!(*disabled),
            _ => panic!("expected entry"),
        }
    }

    #[test]
    fn model_two_way_includes_b_c_both_and_unresolve() {
        // Header + A/B picks + Keep both + separator + Unresolve.
        let model = model(1, false, false, &[], Some(3), None, None, None, 0, false);
        assert_eq!(model.items.len(), 6);
    }

    #[test]
    fn model_two_way_uses_a_b_c_labels_and_shortcuts() {
        let model = model(1, false, false, &[], Some(3), None, None, None, 0, false);
        let entries: Vec<(String, Option<String>)> = model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry {
                    label, shortcut, ..
                } => Some((label.to_string(), shortcut.as_ref().map(|s| s.to_string()))),
                _ => None,
            })
            .collect();

        assert_eq!(
            entries,
            vec![
                ("Pick A (Local)".to_string(), Some("A / Ctrl+1".to_string())),
                (
                    "Pick B (Remote)".to_string(),
                    Some("B / Ctrl+2".to_string())
                ),
                (
                    "Keep both (A + B)".to_string(),
                    Some("C / Ctrl+3".to_string()),
                ),
                ("Unresolve".to_string(), Some("U".to_string())),
            ]
        );
    }

    #[test]
    fn model_two_way_uses_svg_source_icons_when_unselected() {
        let model = model(1, false, false, &[], Some(3), None, None, None, 0, false);
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
        let model = model(
            1,
            false,
            false,
            &selected,
            Some(3),
            None,
            None,
            None,
            0,
            false,
        );
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
        let model = model(1, true, true, &selected, None, None, None, None, 0, false);
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
        let model = model(1, true, true, &[], None, None, None, None, 0, false);
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

    #[test]
    fn model_shows_split_only_for_a_valid_selection() {
        let without_selection = model(0, false, false, &[], None, None, None, None, 0, false);
        assert_eq!(without_selection.items.len(), 6);

        let with_selection = model(0, false, false, &[], None, Some(1), None, None, 0, false);
        assert_eq!(with_selection.items.len(), 8);
        match with_selection.items.last().expect("split entry") {
            ContextMenuItem::Entry {
                label,
                shortcut,
                action,
                ..
            } => {
                assert_eq!(label.as_ref(), "Split selection into own conflict");
                assert_eq!(shortcut.as_deref(), Some("1 row"));
                assert!(matches!(
                    action.as_ref(),
                    ContextMenuAction::ConflictResolverSplitSelection
                ));
            }
            _ => panic!("expected split entry"),
        }
    }

    #[test]
    fn model_join_entries_match_first_middle_and_last_regions() {
        let cases = [
            (None, Some(join_target(0)), vec![("Join with next", 0)]),
            (
                Some(join_target(0)),
                Some(join_target(1)),
                vec![("Join with previous", 0), ("Join with next", 1)],
            ),
            (Some(join_target(1)), None, vec![("Join with previous", 1)]),
        ];

        for (previous, next, expected) in cases {
            let model = model(1, false, false, &[], None, None, previous, next, 0, false);
            let actual: Vec<(&str, usize)> = model
                .items
                .iter()
                .filter_map(|item| match item {
                    ContextMenuItem::Entry { label, action, .. } => match action.as_ref() {
                        ContextMenuAction::ConflictResolverJoinRegions { target } => {
                            Some((label.as_ref(), target.first_region_index))
                        }
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn manual_alignment_entries_appear_only_once_there_is_something_to_act_on() {
        let plain = model(0, false, false, &[], None, None, None, None, 0, false);
        assert!(
            !plain
                .items
                .iter()
                .any(|item| matches!(item, ContextMenuItem::Entry { label, .. }
                    if label.contains("lign"))),
            "nothing marked and nothing pinned leaves the menu unchanged"
        );

        let marked = model(0, false, false, &[], None, None, None, None, 2, false);
        assert!(
            marked
                .items
                .iter()
                .any(|item| matches!(item, ContextMenuItem::Entry { action, .. }
                    if matches!(**action, ContextMenuAction::ConflictResolverAlignManually))),
        );
        assert!(
            !marked
                .items
                .iter()
                .any(|item| matches!(item, ContextMenuItem::Entry { action, .. }
                if matches!(
                    **action,
                    ContextMenuAction::ConflictResolverClearManualAlignments
                ))),
            "there is nothing pinned to clear yet"
        );

        let pinned = model(0, false, false, &[], None, None, None, None, 0, true);
        assert!(pinned.items.iter().any(
            |item| matches!(item, ContextMenuItem::Entry { action, .. }
            if matches!(
                **action,
                ContextMenuAction::ConflictResolverClearManualAlignments
            ))
        ),);
    }
}
