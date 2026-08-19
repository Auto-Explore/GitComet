use super::*;

/// Dropdown opened from the purple historical-browse badge: every commit the user
/// has browsed this session (current marked with `●`), plus "Go live".
pub(super) fn model(this: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let current = repo.and_then(|r| r.browsing_commit().cloned());
    let history: Vec<CommitId> = repo.map(|r| r.browse_history.clone()).unwrap_or_default();

    let mut items = vec![ContextMenuItem::Header("Browsing history".into())];
    let mut tooltips = HashMap::with_capacity_and_hasher(history.len(), Default::default());
    if history.is_empty() {
        items.push(ContextMenuItem::Label("No browsed commits yet.".into()));
    }
    for commit_id in &history {
        let sha = commit_id.as_ref().to_string();
        let short = sha.get(0..8).unwrap_or(&sha);
        let summary = repo.and_then(|r| match &r.log {
            Loadable::Ready(page) => page
                .commits
                .iter()
                .find(|c| c.id == *commit_id)
                .map(|c| c.summary.clone()),
            _ => None,
        });
        let label = match &summary {
            Some(summary) if !summary.is_empty() => format!("{short}  {summary}"),
            _ => short.to_string(),
        };
        // Full commit subject as a hover tooltip (the row label is clamped to one
        // line and gets truncated in the narrow menu).
        if let Some(summary) = summary.as_ref() {
            let full = summary.trim();
            if !full.is_empty() {
                tooltips.insert(items.len(), full.to_owned().into());
            }
        }
        let is_current = current.as_ref() == Some(commit_id);
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some("icons/history.svg".into()),
            shortcut: is_current.then(|| "●".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::BrowseRepositoryAtCommit {
                repo_id,
                commit_id: commit_id.clone(),
            }),
        });
    }

    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Go live".into(),
        icon: Some("icons/undo.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::ResetBrowseToLive { repo_id }),
    });

    ContextMenuModel::new(items).with_entry_tooltips(tooltips)
}
