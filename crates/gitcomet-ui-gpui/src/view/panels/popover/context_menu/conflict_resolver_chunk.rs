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
mod tests;
