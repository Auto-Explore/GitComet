use super::*;

pub(super) fn model(
    host: &PopoverHost,
    list: crate::view::rows::FileListId,
    cx: &App,
) -> ContextMenuModel {
    let current = host.details_pane.read(cx).file_list_sort_for(list);
    model_for_sort(list, current)
}

fn model_for_sort(
    list: crate::view::rows::FileListId,
    current: crate::view::rows::CommitFileSort,
) -> ContextMenuModel {
    let check = |selected: bool| selected.then_some("icons/check.svg".into());
    let header =
        match list {
            // A linked worktree's list is uncommitted changes too, so it cannot
            // borrow the committed-files wording.
            crate::view::rows::FileListId::Status(_)
            | crate::view::rows::FileListId::WorktreeFiles => "Sort files",
            crate::view::rows::FileListId::CommitFiles
            | crate::view::rows::FileListId::RangeFiles => "Sort committed files",
        };
    let mut items = vec![
        ContextMenuItem::Header(header.into()),
        ContextMenuItem::Separator,
    ];
    for sort in crate::view::rows::CommitFileSort::ALL {
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

    fn header(list: crate::view::rows::FileListId) -> String {
        super::model_for_sort(list, crate::view::rows::CommitFileSort::PathAscending)
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Header(label) => Some(label.as_ref().to_string()),
                _ => None,
            })
            .expect("the sort menu opens with a header")
    }

    /// A linked worktree's list holds *uncommitted* changes, so calling them
    /// committed states the opposite of what the rows are.
    #[test]
    fn no_menu_header_calls_uncommitted_changes_committed() {
        use crate::view::StatusSection;
        use crate::view::rows::FileListId;

        for list in [
            FileListId::Status(StatusSection::Staged),
            FileListId::Status(StatusSection::Unstaged),
            FileListId::Status(StatusSection::Untracked),
            FileListId::WorktreeFiles,
        ] {
            let header = header(list);
            assert!(
                !header.contains("committed"),
                "{list:?} lists uncommitted changes but its header reads {header:?}"
            );
        }

        for list in [FileListId::CommitFiles, FileListId::RangeFiles] {
            assert_eq!(header(list), "Sort committed files");
        }
    }

    fn sort_labels(list: crate::view::rows::FileListId) -> Vec<String> {
        super::model_for_sort(list, crate::view::rows::CommitFileSort::PathAscending)
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, .. } => Some(label.to_string()),
                _ => None,
            })
            .collect()
    }

    /// Untracked files have counts and edits too, so every section offers
    /// the edit-size and edit sorts.
    #[test]
    fn every_status_section_offers_every_sort() {
        use crate::view::StatusSection;
        use crate::view::rows::{CommitFileSort, FileListId};

        let every_sort: Vec<String> = CommitFileSort::ALL
            .iter()
            .map(|sort| sort.label().to_string())
            .collect();
        for section in [
            StatusSection::Untracked,
            StatusSection::Unstaged,
            StatusSection::CombinedUnstaged,
            StatusSection::Staged,
        ] {
            assert_eq!(
                sort_labels(FileListId::Status(section)),
                every_sort,
                "{section:?}"
            );
        }
    }
}
