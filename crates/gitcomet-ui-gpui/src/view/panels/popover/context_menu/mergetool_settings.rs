use super::*;

/// Cog-wheel settings menu for the merge conflict resolver (§30). Holds the
/// resolver-specific view options that used to borrow the diff-actions menu.
pub(super) fn model(host: &PopoverHost, cx: &gpui::App) -> ContextMenuModel {
    let pane = host.main_pane.read(cx);
    let (auto_advance, _collapse_default, _vertical_split, output_scroll_sync) =
        pane.mergetool_preferences();
    let collapse_context = pane.conflict_resolver_collapse_context();
    model_for_mergetool_settings(auto_advance, collapse_context, output_scroll_sync)
}

// NOTE: a "Stack columns vertically" entry (backed by the persisted
// `mergetool_vertical_split` setting and `SetMergetoolVerticalSplit`) is
// deliberately not offered yet — the stacked column rendering is deferred.
fn model_for_mergetool_settings(
    auto_advance: bool,
    collapse_context: bool,
    output_scroll_sync: bool,
) -> ContextMenuModel {
    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Merge tool settings".into()),
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
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_marks_enabled_options_and_toggles_them() {
        let model = model_for_mergetool_settings(true, false, true);

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
    }
}
