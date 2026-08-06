use super::*;

/// Cog-wheel settings menu for the merge conflict resolver (section 30). Holds the
/// resolver-specific view options that used to borrow the diff-actions menu.
pub(super) fn model(host: &PopoverHost, cx: &gpui::App) -> ContextMenuModel {
    let pane = host.main_pane.read(cx);
    let (auto_advance, _collapse_default, output_scroll_sync, show_line_numbers) =
        pane.mergetool_preferences();
    let collapse_context = pane.conflict_resolver_collapse_context();
    let three_way_view = pane.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay;
    model_for_mergetool_settings(
        three_way_view,
        auto_advance,
        collapse_context,
        output_scroll_sync,
        show_line_numbers,
        host.diff_whitespace_mode,
        host.diff_reveal_whitespace_chars,
    )
}

#[allow(clippy::too_many_arguments)]
fn model_for_mergetool_settings(
    three_way_view: bool,
    auto_advance: bool,
    collapse_context: bool,
    output_scroll_sync: bool,
    show_line_numbers: bool,
    whitespace_mode: DiffWhitespaceMode,
    reveal_whitespace_chars: bool,
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
        // The source columns honour the diff view's whitespace options, so the
        // cog offers the same two toggles rather than making the user leave the
        // merge tool to reach them.
        ContextMenuItem::Entry {
            label: "Show whitespace changes".into(),
            icon: (whitespace_mode == DiffWhitespaceMode::Show).then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetDiffWhitespaceMode {
                mode: whitespace_mode.toggled(),
            }),
        },
        ContextMenuItem::Entry {
            label: "Reveal whitespace characters".into(),
            icon: reveal_whitespace_chars.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetDiffRevealWhitespaceChars {
                enabled: !reveal_whitespace_chars,
            }),
        },
    ]);
    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(three_way_view: bool) -> ContextMenuModel {
        model_for_mergetool_settings(
            three_way_view,
            true,
            false,
            true,
            true,
            DiffWhitespaceMode::Show,
            false,
        )
    }

    #[test]
    fn model_marks_enabled_options_and_toggles_them() {
        let model = model(true);

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

    fn entry_labels(model: &ContextMenuModel) -> Vec<&str> {
        model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, .. } => Some(label.as_ref()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn whitespace_options_mirror_the_diff_view_and_drive_its_settings() {
        let model = model_for_mergetool_settings(
            true,
            true,
            false,
            true,
            true,
            DiffWhitespaceMode::Ignore,
            true,
        );

        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, icon, action, .. }
                    if label.as_ref() == "Show whitespace changes"
                        && icon.is_none()
                        && matches!(
                            action.as_ref(),
                            ContextMenuAction::SetDiffWhitespaceMode {
                                mode: DiffWhitespaceMode::Show
                            }
                        )
            )
        }));
        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, icon, action, .. }
                    if label.as_ref() == "Reveal whitespace characters"
                        && icon.is_some()
                        && matches!(
                            action.as_ref(),
                            ContextMenuAction::SetDiffRevealWhitespaceChars { enabled: false }
                        )
            )
        }));
    }

    #[test]
    fn the_whitespace_conflict_bulk_picks_are_gone() {
        // Whitespace-only conflicts are the user's call, one block at a time;
        // the cog is for view options, not for resolving.
        for three_way_view in [true, false] {
            let model = model(three_way_view);
            assert!(
                !entry_labels(&model)
                    .iter()
                    .any(|label| label.contains("for all")),
                "{:?}",
                entry_labels(&model)
            );
        }
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
        let model = model(false);
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
        let model = model(true);

        assert!(!model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Segmented { label, .. }
                if label.as_ref() == "Overview" || label.as_ref() == "Minimap"
        )));
    }
}
