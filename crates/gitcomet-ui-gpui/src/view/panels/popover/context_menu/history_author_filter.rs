use super::*;

pub(super) fn model(host: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let Some(repo) = host.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return ContextMenuModel::new(Vec::new());
    };
    let current = repo.history_state.history_author_filter.clone();
    let authors = match &repo.history_state.log {
        Loadable::Ready(page) => collect_author_suggestions(&page.commits),
        _ => Vec::new(),
    };
    model_for(repo_id, current, authors)
}

/// Author names from the loaded log commits: trimmed, non-empty, deduplicated
/// case-insensitively and sorted.
fn collect_author_suggestions(commits: &[gitcomet_core::domain::Commit]) -> Vec<String> {
    let mut authors: Vec<String> = Vec::new();
    for commit in commits {
        let name = commit.author.trim().to_string();
        if !name.is_empty()
            && !authors
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&name))
        {
            authors.push(name);
        }
    }
    authors.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    authors
}

fn model_for(repo_id: RepoId, current: Option<String>, authors: Vec<String>) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header("Author filter".into()),
        ContextMenuItem::Description(
            "Show commits by a single author. Suggestions come from the loaded history."
                .into(),
        ),
        ContextMenuItem::Entry {
            label: "All authors".into(),
            icon: current.is_none().then_some("icons/check.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetHistoryAuthorFilter {
                repo_id,
                author: None,
            }),
        },
    ];
    if !authors.is_empty() {
        items.push(ContextMenuItem::Separator);
        items.extend(authors.into_iter().map(|name| {
            let active = current
                .as_deref()
                .is_some_and(|c| c.eq_ignore_ascii_case(&name));
            ContextMenuItem::Entry {
                label: name.clone().into(),
                icon: active.then_some("icons/check.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::SetHistoryAuthorFilter {
                    repo_id,
                    author: Some(name),
                }),
            }
        }));
    }
    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_labels_and_icons(
        model: &ContextMenuModel,
    ) -> Vec<(String, Option<String>)> {
        model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, icon, .. } => {
                    Some((label.to_string(), icon.as_ref().map(|i| i.to_string())))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn all_authors_entry_is_checked_when_no_filter() {
        let model = model_for(RepoId(11), None, vec!["Alice".into(), "Bob".into()]);

        let entries = entry_labels_and_icons(&model);
        assert_eq!(entries[0].0, "All authors");
        assert_eq!(entries[0].1, Some("icons/check.svg".into()));
        // Suggestions follow, none checked.
        assert_eq!(entries[1].0, "Alice");
        assert_eq!(entries[1].1, None);
        assert_eq!(entries[2].0, "Bob");
    }

    #[test]
    fn active_filter_is_checked_and_others_not() {
        let model = model_for(
            RepoId(11),
            Some("Alice".into()),
            vec!["Alice".into(), "Bob".into()],
        );

        let entries = entry_labels_and_icons(&model);
        assert_eq!(entries[0].1, None); // "All authors" unchecked
        assert_eq!(entries[1].0, "Alice");
        assert_eq!(entries[1].1, Some("icons/check.svg".into()));
        assert_eq!(entries[2].0, "Bob");
        assert_eq!(entries[2].1, None);
    }

    #[test]
    fn matching_author_filter_is_case_insensitive() {
        let model = model_for(
            RepoId(11),
            Some("alice".into()),
            vec!["Alice".into(), "Bob".into()],
        );

        let entries = entry_labels_and_icons(&model);
        assert_eq!(entries[1].1, Some("icons/check.svg".into()));
    }

    #[test]
    fn author_suggestions_are_deduplicated_case_insensitively() {
        use gitcomet_core::domain::{Commit, CommitId, CommitParentIds};

        let commit = |author: &str| Commit {
            id: CommitId("deadbeefdeadbeef".into()),
            parent_ids: CommitParentIds::new(),
            summary: "msg".into(),
            author: author.into(),
            time: std::time::SystemTime::UNIX_EPOCH,
        };
        let authors = collect_author_suggestions(&[
            commit("Alice"),
            commit("alice"),
            commit("  Bob  "),
            commit("Bob"),
            commit(""),
        ]);

        assert_eq!(authors, vec!["Alice", "Bob"]);
    }
}
