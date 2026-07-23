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
    )
}

fn model_for_mergetool_settings(
    three_way_view: bool,
    auto_advance: bool,
    collapse_context: bool,
    output_scroll_sync: bool,
    show_line_numbers: bool,
) -> ContextMenuModel {
    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Merge tool settings".into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "3-way merge".into(),
            icon: three_way_view.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetMergetoolThreeWayView { enabled: true }),
        },
        ContextMenuItem::Entry {
            label: "2-way diff".into(),
            icon: (!three_way_view).then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetMergetoolThreeWayView { enabled: false }),
        },
        ContextMenuItem::Separator,
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
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_marks_enabled_options_and_toggles_them() {
        let model = model_for_mergetool_settings(true, true, false, true, true);

        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, icon, action, .. }
                    if label.as_ref() == "3-way merge"
                        && icon.is_some()
                        && matches!(
                            action.as_ref(),
                            ContextMenuAction::SetMergetoolThreeWayView { enabled: true }
                        )
            )
        }));
        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, icon, action, .. }
                    if label.as_ref() == "2-way diff"
                        && icon.is_none()
                        && matches!(
                            action.as_ref(),
                            ContextMenuAction::SetMergetoolThreeWayView { enabled: false }
                        )
            )
        }));

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
}
