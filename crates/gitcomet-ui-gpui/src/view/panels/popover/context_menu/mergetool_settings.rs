use super::*;
use gitcomet_core::merge::OverviewMode;

/// Cog-wheel settings menu for the merge conflict resolver (section 30). Holds the
/// resolver-specific view options that used to borrow the diff-actions menu.
pub(super) fn model(host: &PopoverHost, cx: &gpui::App) -> ContextMenuModel {
    let pane = host.main_pane.read(cx);
    let (auto_advance, _collapse_default, output_scroll_sync, show_line_numbers) =
        pane.mergetool_preferences();
    let collapse_context = pane.conflict_resolver_collapse_context();
    let three_way_view = pane.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay;
    let overview_mode = pane.conflict_resolver.overview_mode;
    // The pairwise comparisons need an ancestor to compare against, and the
    // row is moot when the overview column itself is hidden.
    let overview_available = pane.conflict_resolver.has_overview()
        && !pane.conflict_resolver.three_way_text.base.is_empty();
    model_for_mergetool_settings(
        three_way_view,
        overview_mode,
        overview_available,
        auto_advance,
        collapse_context,
        output_scroll_sync,
        show_line_numbers,
    )
}

fn model_for_mergetool_settings(
    three_way_view: bool,
    overview_mode: OverviewMode,
    overview_available: bool,
    auto_advance: bool,
    collapse_context: bool,
    output_scroll_sync: bool,
    show_line_numbers: bool,
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
    if overview_available {
        items.push(ContextMenuItem::Segmented {
            label: "Overview".into(),
            segments: OverviewMode::ALL
                .into_iter()
                .map(|mode| ContextMenuSegment {
                    id: overview_segment_id(mode).into(),
                    label: mode.label().into(),
                    tooltip: Some(overview_tooltip(mode).into()),
                    selected: overview_mode == mode,
                    action: ContextMenuAction::SetMergetoolOverviewMode { mode },
                })
                .collect(),
        });
    }
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
    ContextMenuModel::new(items)
}

fn overview_segment_id(mode: OverviewMode) -> &'static str {
    match mode {
        OverviewMode::Merge => "mergetool_overview_merge",
        OverviewMode::BaseVsLocal => "mergetool_overview_ab",
        OverviewMode::BaseVsRemote => "mergetool_overview_ac",
        OverviewMode::LocalVsRemote => "mergetool_overview_bc",
    }
}

fn overview_tooltip(mode: OverviewMode) -> &'static str {
    match mode {
        OverviewMode::Merge => "Overview: the merge — each side's changes and the conflicts",
        OverviewMode::BaseVsLocal => "Overview: every line where Local differs from Base",
        OverviewMode::BaseVsRemote => "Overview: every line where Remote differs from Base",
        OverviewMode::LocalVsRemote => "Overview: every line where Local differs from Remote",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_marks_enabled_options_and_toggles_them() {
        let model =
            model_for_mergetool_settings(true, OverviewMode::Merge, true, true, false, true, true);

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
        let model =
            model_for_mergetool_settings(false, OverviewMode::Merge, true, true, false, true, true);
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
    fn overview_row_lists_every_mode_and_marks_the_active_one() {
        let model = model_for_mergetool_settings(
            true,
            OverviewMode::BaseVsRemote,
            true,
            true,
            false,
            true,
            true,
        );
        let overview = segments(&model, "Overview");

        assert_eq!(
            overview
                .iter()
                .map(|segment| segment.label.as_ref())
                .collect::<Vec<_>>(),
            vec!["Merge", "A-B", "A-C", "B-C"],
        );
        assert!(overview.iter().filter(|segment| segment.selected).count() == 1);
        assert!(
            overview
                .iter()
                .find(|segment| segment.selected)
                .is_some_and(|segment| matches!(
                    segment.action,
                    ContextMenuAction::SetMergetoolOverviewMode {
                        mode: OverviewMode::BaseVsRemote
                    }
                ))
        );
    }

    #[test]
    fn overview_row_is_omitted_without_an_overview_column() {
        let model =
            model_for_mergetool_settings(true, OverviewMode::Merge, false, true, false, true, true);

        assert!(!model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Segmented { label, .. } if label.as_ref() == "Overview"
        )));
    }
}
