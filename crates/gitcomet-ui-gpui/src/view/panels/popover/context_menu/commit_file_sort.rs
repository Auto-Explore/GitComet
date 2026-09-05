use super::*;

pub(super) fn model(host: &PopoverHost, cx: &App) -> ContextMenuModel {
    let current = host.details_pane.read(cx).commit_file_sort;
    model_for_sort(current)
}

fn model_for_sort(current: crate::view::rows::CommitFileSort) -> ContextMenuModel {
    let check = |selected: bool| selected.then_some("icons/check.svg".into());
    let mut items = vec![
        ContextMenuItem::Header("Sort committed files".into()),
        ContextMenuItem::Separator,
    ];
    for sort in crate::view::rows::CommitFileSort::ALL {
        items.push(ContextMenuItem::Entry {
            label: sort.label().into(),
            icon: check(sort == current),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetCommitFileSort { sort }),
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
        let model = super::model_for_sort(current);
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
}
