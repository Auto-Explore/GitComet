use super::*;

pub(super) fn model(
    host: &PopoverHost,
    list: crate::view::rows::FileListId,
    cx: &App,
) -> ContextMenuModel {
    let current = host.details_pane.read(cx).file_list_sort_for(list);
    model_for_sort(list, current)
}

/// Status entries carry no `+/-` counts, so the edit-size modes have nothing to
/// order by and are left out of their menus.
fn sorts_for(list: crate::view::rows::FileListId) -> &'static [crate::view::rows::CommitFileSort] {
    use crate::view::rows::CommitFileSort;
    const PATH_ONLY: [CommitFileSort; 2] = [
        CommitFileSort::PathAscending,
        CommitFileSort::PathDescending,
    ];
    match list {
        crate::view::rows::FileListId::Status(_) => &PATH_ONLY,
        _ => &CommitFileSort::ALL,
    }
}

fn model_for_sort(
    list: crate::view::rows::FileListId,
    current: crate::view::rows::CommitFileSort,
) -> ContextMenuModel {
    let check = |selected: bool| selected.then_some("icons/check.svg".into());
    let header = match list {
        crate::view::rows::FileListId::Status(_) => "Sort files",
        _ => "Sort committed files",
    };
    let mut items = vec![
        ContextMenuItem::Header(header.into()),
        ContextMenuItem::Separator,
    ];
    for sort in sorts_for(list).iter().copied() {
        items.push(ContextMenuItem::Entry {
            label: sort.label().into(),
            icon: check(sort == current),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetCommitFileSort { list, sort }),
        });
    }
    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_lists_every_sort_and_checks_the_current_one() {
        let current = crate::view::rows::CommitFileSort::EditSizeDescending;
        let model = super::model_for_sort(crate::view::rows::FileListId::CommitFiles, current);
        let entries = model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, icon, .. } => Some((label.as_ref(), icon.as_ref())),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), crate::view::rows::CommitFileSort::ALL.len());
        assert!(entries.iter().any(|(label, icon)| {
            *label == current.label() && icon.is_some_and(|icon| icon.as_ref() == "icons/check.svg")
        }));
    }

    /// Status lists have no edit counts, so offering "Edit size" there would be
    /// a mode that silently does nothing.
    #[test]
    fn status_lists_are_offered_path_sorts_only() {
        let model = super::model_for_sort(
            crate::view::rows::FileListId::Status(crate::view::StatusSection::Staged),
            crate::view::rows::CommitFileSort::PathAscending,
        );
        let labels = model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, .. } => Some(label.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![
                crate::view::rows::CommitFileSort::PathAscending
                    .label()
                    .to_string(),
                crate::view::rows::CommitFileSort::PathDescending
                    .label()
                    .to_string(),
            ]
        );
    }
}
