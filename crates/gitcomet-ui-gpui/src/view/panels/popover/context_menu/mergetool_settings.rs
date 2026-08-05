use super::*;

/// Cog-wheel settings menu for the merge conflict resolver (section 30). Holds the
/// resolver-specific view options that used to borrow the diff-actions menu.
pub(super) fn model(host: &PopoverHost, cx: &gpui::App) -> ContextMenuModel {
    let pane = host.main_pane.read(cx);
    let (auto_advance, _collapse_default, output_scroll_sync, show_line_numbers) =
        pane.mergetool_preferences();
    let collapse_context = pane.conflict_resolver_collapse_context();
    let three_way_view = pane.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay;
    let unsolved_whitespace = pane.conflict_resolver_unsolved_whitespace_conflicts();
    model_for_mergetool_settings(
        three_way_view,
        auto_advance,
        collapse_context,
        output_scroll_sync,
        show_line_numbers,
        unsolved_whitespace,
    )
}

fn model_for_mergetool_settings(
    three_way_view: bool,
    auto_advance: bool,
    collapse_context: bool,
    output_scroll_sync: bool,
    show_line_numbers: bool,
    unsolved_whitespace: usize,
) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header("Merge tool settings".into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Segmented {
            label: "View".into(),
            segments: vec![
                ContextMenuSegment {
                    id: "mergetool_view_three_way".into(),
                    label: "3-way".into(),
                    tooltip: Some(
                        "Three-way merge view: Base, Local and Remote side by side".into(),
                    ),
                    selected: three_way_view,
                    action: ContextMenuAction::SetMergetoolThreeWayView { enabled: true },
                },
                ContextMenuSegment {
                    id: "mergetool_view_two_way".into(),
                    label: "2-way".into(),
                    tooltip: Some("Two-way diff view: Local against Remote".into()),
                    selected: !three_way_view,
                    action: ContextMenuAction::SetMergetoolThreeWayView { enabled: false },
                },
            ],
        },
    ];
    items.push(ContextMenuItem::Separator);
    items.extend([
        ContextMenuItem::Entry {
            label: "Auto-advance to next unresolved after pick".into(),
            icon: auto_advance.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetMergetoolAutoAdvance {
                enabled: !auto_advance,
            }),
        },
        ContextMenuItem::Entry {
            label: "Collapse unchanged context".into(),
            icon: collapse_context.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::ToggleMergetoolCollapseUnchanged),
        },
        ContextMenuItem::Entry {
            label: "Sync resolved output scroll with source".into(),
            icon: output_scroll_sync.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetMergetoolOutputScrollSync {
                enabled: !output_scroll_sync,
            }),
        },
        ContextMenuItem::Entry {
            label: "Show line numbers".into(),
            icon: show_line_numbers.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetMergetoolShowLineNumbers {
                enabled: !show_line_numbers,
            }),
        },
    ]);
    items.extend(whitespace_conflict_items(
        three_way_view,
        unsolved_whitespace,
    ));
    ContextMenuModel::new(items)
}

/// kdiff3's Choose A/B/C for All Unsolved Whitespace Conflicts.
///
/// Whitespace-only conflicts are never auto-picked (see the auto-solve policy
/// in UI_DESIGN.md section 30), so this is the bulk escape hatch for a file
/// full of reindented lines. Hidden when the file has none left.
fn whitespace_conflict_items(
    three_way_view: bool,
    unsolved_whitespace: usize,
) -> Vec<ContextMenuItem> {
    if unsolved_whitespace == 0 {
        return Vec::new();
    }

    let noun = if unsolved_whitespace == 1 {
        "whitespace conflict"
    } else {
        "whitespace conflicts"
    };
    let mut items = vec![
        ContextMenuItem::Separator,
        ContextMenuItem::Header(format!("{unsolved_whitespace} unsolved {noun}").into()),
    ];

    let choices: &[(conflict_resolver::ConflictChoice, &str)] = if three_way_view {
        &[
            (conflict_resolver::ConflictChoice::Base, "A (Base)"),
            (conflict_resolver::ConflictChoice::Ours, "B (Local)"),
            (conflict_resolver::ConflictChoice::Theirs, "C (Remote)"),
        ]
    } else {
        &[
            (conflict_resolver::ConflictChoice::Ours, "A (Local)"),
            (conflict_resolver::ConflictChoice::Theirs, "B (Remote)"),
        ]
    };

    items.extend(
        choices
            .iter()
            .map(|(choice, label)| ContextMenuItem::Entry {
                label: format!("Choose {label} for all").into(),
                icon: None,
                shortcut: None,
                disabled: false,
                action: Box::new(
                    ContextMenuAction::ConflictResolverChooseForWhitespaceConflicts {
                        choice: *choice,
                    },
                ),
            }),
    );
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_marks_enabled_options_and_toggles_them() {
        let model = model_for_mergetool_settings(true, true, false, true, true, 0);

        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, icon, action, .. }
                    if label.as_ref() == "Auto-advance to next unresolved after pick"
                        && icon.is_some()
                        && matches!(
                            action.as_ref(),
                            ContextMenuAction::SetMergetoolAutoAdvance { enabled: false }
                        )
            )
        }));
        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, icon, action, .. }
                    if label.as_ref() == "Collapse unchanged context"
                        && icon.is_none()
                        && matches!(
                            action.as_ref(),
                            ContextMenuAction::ToggleMergetoolCollapseUnchanged
                        )
            )
        }));
        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, icon, action, .. }
                    if label.as_ref() == "Show line numbers"
                        && icon.is_some()
                        && matches!(
                            action.as_ref(),
                            ContextMenuAction::SetMergetoolShowLineNumbers { enabled: false }
                        )
            )
        }));
    }

    fn whitespace_entry_labels(model: &ContextMenuModel) -> Vec<&str> {
        model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if matches!(
                        action.as_ref(),
                        ContextMenuAction::ConflictResolverChooseForWhitespaceConflicts { .. }
                    ) =>
                {
                    Some(label.as_ref())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn whitespace_bulk_picks_are_offered_per_side_with_a_count_header() {
        let model = model_for_mergetool_settings(true, true, false, true, true, 3);

        assert_eq!(
            whitespace_entry_labels(&model),
            vec![
                "Choose A (Base) for all",
                "Choose B (Local) for all",
                "Choose C (Remote) for all",
            ],
        );
        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Header(label) if label.as_ref() == "3 unsolved whitespace conflicts"
        )));
    }

    #[test]
    fn whitespace_bulk_picks_drop_base_in_the_two_way_view() {
        let model = model_for_mergetool_settings(false, true, false, true, true, 1);

        assert_eq!(
            whitespace_entry_labels(&model),
            vec!["Choose A (Local) for all", "Choose B (Remote) for all"],
        );
        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Header(label) if label.as_ref() == "1 unsolved whitespace conflict"
        )));
    }

    #[test]
    fn whitespace_bulk_picks_are_omitted_when_none_remain() {
        let model = model_for_mergetool_settings(true, true, false, true, true, 0);

        assert!(whitespace_entry_labels(&model).is_empty());
    }

    fn segments<'a>(model: &'a ContextMenuModel, row: &str) -> &'a [ContextMenuSegment] {
        model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Segmented { label, segments } if label.as_ref() == row => {
                    Some(segments.as_slice())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no `{row}` segmented row in the model"))
    }

    #[test]
    fn view_row_selects_the_active_mode_and_offers_the_other() {
        let model = model_for_mergetool_settings(false, true, false, true, true, 0);
        let view = segments(&model, "View");

        assert_eq!(view.len(), 2);
        assert!(!view[0].selected);
        assert!(matches!(
            view[0].action,
            ContextMenuAction::SetMergetoolThreeWayView { enabled: true }
        ));
        assert!(view[1].selected);
        assert!(matches!(
            view[1].action,
            ContextMenuAction::SetMergetoolThreeWayView { enabled: false }
        ));
    }

    #[test]
    fn the_minimap_has_no_settings_row() {
        // The minimap always shows the merge itself; kdiff3's pairwise
        // overview modes are not offered.
        let model = model_for_mergetool_settings(true, true, false, true, true, 0);

        assert!(!model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Segmented { label, .. }
                if label.as_ref() == "Overview" || label.as_ref() == "Minimap"
        )));
    }
}
