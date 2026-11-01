use super::*;

/// The layout icon's right-click menu: every layout, the list's own checked.
pub(super) fn model(
    host: &PopoverHost,
    repo_id: RepoId,
    list: crate::view::rows::FileListId,
    cx: &App,
) -> ContextMenuModel {
    let current = host
        .details_pane
        .read(cx)
        .file_list_layout_for(repo_id, list);
    model_for_layout(repo_id, list, current)
}

fn model_for_layout(
    repo_id: RepoId,
    list: crate::view::rows::FileListId,
    current: crate::view::FileListLayout,
) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header("Layout".into()),
        ContextMenuItem::Separator,
    ];
    for layout in crate::view::FileListLayout::ALL {
        items.push(ContextMenuItem::Entry {
            label: layout.label().into(),
            icon: (layout == current).then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetFileListLayout {
                repo_id,
                list,
                layout,
            }),
        });
    }
    ContextMenuModel::new(items)
}
