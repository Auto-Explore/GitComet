use super::*;

pub(super) fn model(host: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let (hidden, ignored) = host
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map_or((true, false), |repo| {
            (repo.file_browser.show_hidden, repo.file_browser.show_ignored)
        });
    model_for_visibility(repo_id, hidden, ignored)
}

fn model_for_visibility(repo_id: RepoId, hidden: bool, ignored: bool) -> ContextMenuModel {
    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Files".into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Show hidden files".into(),
            icon: hidden.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetExplorerVisibility {
                repo_id,
                hidden: !hidden,
                ignored,
            }),
        },
        ContextMenuItem::Entry {
            label: "Show ignored files".into(),
            icon: ignored.then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetExplorerVisibility {
                repo_id,
                hidden,
                ignored: !ignored,
            }),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(model: &ContextMenuModel, label: &str) -> (bool, (bool, bool)) {
        model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry {
                    label: entry_label,
                    icon,
                    action,
                    ..
                } if entry_label.as_ref() == label => match action.as_ref() {
                    ContextMenuAction::SetExplorerVisibility {
                        repo_id: RepoId(7),
                        hidden,
                        ignored,
                    } => Some((
                        icon.as_ref()
                            .is_some_and(|icon| icon.as_ref() == "icons/check.svg"),
                        (*hidden, *ignored),
                    )),
                    _ => None,
                },
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {label:?} entry"))
    }

    #[test]
    fn model_marks_enabled_flags_and_carries_next_values() {
        let model = model_for_visibility(RepoId(7), true, false);
        assert_eq!(entry(&model, "Show hidden files"), (true, (false, false)));
        assert_eq!(entry(&model, "Show ignored files"), (false, (true, true)));

        let model = model_for_visibility(RepoId(7), false, true);
        assert_eq!(entry(&model, "Show hidden files"), (false, (true, true)));
        assert_eq!(entry(&model, "Show ignored files"), (true, (false, false)));
    }
}
