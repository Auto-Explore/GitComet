use super::*;

pub(super) fn model(
    repo_id: RepoId,
    commit_id: &CommitId,
    allow_navigate: bool,
) -> ContextMenuModel {
    model_for_commit_sha_link(repo_id, commit_id, allow_navigate)
}

fn model_for_commit_sha_link(
    repo_id: RepoId,
    commit_id: &CommitId,
    allow_navigate: bool,
) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header("Commit".into()),
        ContextMenuItem::Label(commit_id.as_ref().to_owned().into()),
        ContextMenuItem::Separator,
    ];

    // Navigating to the commit you are already looking at is a no-op, so the
    // commit's own SHA field offers only the browse entry.
    if allow_navigate {
        items.push(ContextMenuItem::Entry {
            label: "Navigate".into(),
            icon: Some("icons/link.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::RevealHistoryCommit {
                repo_id,
                commit_id: commit_id.clone(),
            }),
        });
    }

    items.push(ContextMenuItem::Entry {
        label: "Start file browsing".into(),
        icon: Some("icons/history.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::BrowseRepositoryAtCommit {
            repo_id,
            commit_id: commit_id.clone(),
        }),
    });

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_labels(model: &ContextMenuModel) -> Vec<String> {
        model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, .. } => Some(label.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn model_offers_navigating_and_browsing() {
        let commit_id = CommitId("deadbeef".into());
        let model = model_for_commit_sha_link(RepoId(3), &commit_id, true);

        assert_eq!(
            entry_labels(&model),
            vec!["Navigate", "Start file browsing"]
        );
        // The id is shown so a reference in prose can be checked before acting.
        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Label(label) if label.as_ref() == "deadbeef"
        )));
    }

    #[test]
    fn a_commits_own_sha_cannot_navigate_to_itself() {
        let commit_id = CommitId("deadbeef".into());
        let model = model_for_commit_sha_link(RepoId(3), &commit_id, false);

        assert_eq!(entry_labels(&model), vec!["Start file browsing"]);
    }

    #[test]
    fn entries_carry_the_repo_and_commit_they_act_on() {
        let commit_id = CommitId("deadbeef".into());
        let model = model_for_commit_sha_link(RepoId(3), &commit_id, true);
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
    }
}
