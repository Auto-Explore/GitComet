use super::*;

pub(super) fn model(repo_id: RepoId, commit_id: &CommitId) -> ContextMenuModel {
    ContextMenuModel::new(vec![
        ContextMenuItem::Header("Commit".into()),
        ContextMenuItem::Label(commit_id.as_ref().to_owned().into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Reveal commit".into(),
            icon: Some("icons/link.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::RevealHistoryCommit {
                repo_id,
                commit_id: commit_id.clone(),
            }),
        },
        ContextMenuItem::Entry {
            label: "Start file browsing".into(),
            icon: Some("icons/history.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::BrowseRepositoryAtCommit {
                repo_id,
                commit_id: commit_id.clone(),
            }),
        },
        ContextMenuItem::Entry {
            label: "Copy to clipboard".into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CopyCommitSha {
                commit_id: commit_id.clone(),
            }),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_offers_revealing_and_browsing() {
        let commit_id = CommitId("deadbeef".into());
        let model = model(RepoId(3), &commit_id);

        assert_eq!(
            model.entry_labels(),
            vec!["Reveal commit", "Start file browsing", "Copy to clipboard"]
        );
        // The id is shown so a reference in prose can be checked before acting.
        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Label(label) if label.as_ref() == "deadbeef"
        )));
    }

    #[test]
    fn entries_carry_the_repo_and_commit_they_act_on() {
        let commit_id = CommitId("deadbeef".into());
        let model = model(RepoId(3), &commit_id);
        let actions = model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { action, .. } => Some(action.as_ref()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(matches!(
            actions[0],
            ContextMenuAction::RevealHistoryCommit { repo_id, commit_id }
                if *repo_id == RepoId(3) && commit_id.as_ref() == "deadbeef"
        ));
        assert!(matches!(
            actions[1],
            ContextMenuAction::BrowseRepositoryAtCommit { repo_id, commit_id }
                if *repo_id == RepoId(3) && commit_id.as_ref() == "deadbeef"
        ));
        assert!(matches!(
            actions[2],
            ContextMenuAction::CopyCommitSha { commit_id } if commit_id.as_ref() == "deadbeef"
        ));
    }
}
