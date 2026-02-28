use super::*;

pub(super) fn model(host: &PopoverHost, cx: &gpui::Context<PopoverHost>) -> ContextMenuModel {
    let (show_author, show_date, show_sha) = host
        .main_pane
        .read(cx)
        .history_visible_column_preferences(cx);

    let check = |enabled: bool| enabled.then_some("✓".into());

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
        ContextMenuItem::Separator,
        ContextMenuItem::Label("Columns may auto-hide in narrow windows".into()),
    ])
}
