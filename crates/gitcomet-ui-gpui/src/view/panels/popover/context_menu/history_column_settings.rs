use super::*;

pub(super) fn model(host: &PopoverHost, cx: &gpui::Context<PopoverHost>) -> ContextMenuModel {
    let (show_author, show_date, show_sha) = host
        .main_pane
        .read(cx)
        .history_visible_column_preferences(cx);

    model_for_preferences(show_author, show_date, show_sha)
}

fn model_for_preferences(show_author: bool, show_date: bool, show_sha: bool) -> ContextMenuModel {
    let check = |enabled: bool| enabled.then_some("icons/check.svg".into());

    ContextMenuModel::new(vec![
        ContextMenuItem::Header("History columns".into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Author".into(),
            icon: check(show_author),
            shortcut: Some("A".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::SetHistoryColumns {
                show_author: !show_author,
                show_date,
                show_sha,
            }),
        },
        ContextMenuItem::Entry {
            label: "Commit date".into(),
            icon: check(show_date),
            shortcut: Some("D".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::SetHistoryColumns {
                show_author,
                show_date: !show_date,
                show_sha,
            }),
        },
        ContextMenuItem::Entry {
            label: "SHA".into(),
            icon: check(show_sha),
            shortcut: Some("S".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::SetHistoryColumns {
                show_author,
                show_date,
                show_sha: !show_sha,
            }),
        },
        ContextMenuItem::Entry {
            label: "Reset column widths".into(),
            icon: Some("icons/refresh.svg".into()),
            shortcut: Some("R".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::ResetHistoryColumnWidths),
        },
        ContextMenuItem::Separator,
        ContextMenuItem::Label("Columns may auto-hide in narrow windows".into()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_includes_reset_column_widths_entry() {
        let model = super::model_for_preferences(true, true, true);

        let has_reset_entry = model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == "Reset column widths"
                        && matches!(&**action, ContextMenuAction::ResetHistoryColumnWidths)
            )
        });
        assert!(has_reset_entry);
    }
}
