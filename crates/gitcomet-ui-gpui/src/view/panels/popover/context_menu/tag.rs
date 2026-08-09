use super::*;
use rustc_hash::FxHashSet as HashSet;

pub(super) fn model(this: &PopoverHost, repo_id: RepoId, commit_id: &CommitId) -> ContextMenuModel {
    let sha = commit_id.as_ref().to_string();
    let short: SharedString = sha.get(0..8).unwrap_or(&sha).to_string().into();

    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let tags = match repo.map(|r| &r.tags) {
        Some(Loadable::Ready(tags)) => Some(tags.as_slice()),
        Some(Loadable::Error(err)) => {
            return ContextMenuModel::new(vec![
                ContextMenuItem::Header(format!("Tags on {short}").into()),
                ContextMenuItem::Separator,
                ContextMenuItem::Label(err.clone().into()),
            ]);
        }
        Some(Loadable::Loading) | Some(Loadable::NotLoaded) => {
            return ContextMenuModel::new(vec![
                ContextMenuItem::Header(format!("Tags on {short}").into()),
                ContextMenuItem::Separator,
                ContextMenuItem::Label("Loading tags…".into()),
            ]);
        }
        None => None,
    }
    .unwrap_or(&[]);
    let (remote_names, remote_tags) = remote_tag_context(repo);

    let mut tag_names = tags
        .iter()
        .filter(|t| t.target == *commit_id)
        .map(|t| t.name.clone())
        .collect::<Vec<_>>();
    tag_names.sort_unstable();

    let compare_label = tag_names
        .first()
        .cloned()
        .unwrap_or_else(|| short.to_string());
    let comparison_mark = comparison_mark_pair(repo);

    tag_names_model(
        repo_id,
        format!("Tags on {short}").into(),
        tag_names,
        remote_names,
        remote_tags,
        commit_id,
        compare_label,
        comparison_mark,
    )
}

pub(super) fn model_for_tag(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
    name: &String,
) -> ContextMenuModel {
    let sha = commit_id.as_ref().to_string();
    let short = sha.get(0..8).unwrap_or(&sha);
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let (remote_names, remote_tags) = remote_tag_context(repo);
    let comparison_mark = comparison_mark_pair(repo);
    tag_names_model(
        repo_id,
        format!("Tag {name} on {short}").into(),
        vec![name.clone()],
        remote_names,
        remote_tags,
        commit_id,
        name.clone(),
        comparison_mark,
    )
}

fn comparison_mark_pair(repo: Option<&RepoState>) -> Option<(CommitId, String)> {
    repo.and_then(|r| {
        r.comparison_mark
            .as_ref()
            .map(|mark| (mark.commit_id.clone(), mark.label.clone()))
    })
}

fn remote_tag_context(repo: Option<&RepoState>) -> (Vec<String>, HashSet<(&str, &str)>) {
    let mut remote_names = repo
        .and_then(|r| match &r.remotes {
            Loadable::Ready(remotes) => Some(
                remotes
                    .iter()
                    .map(|remote| remote.name.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    remote_names.sort_unstable();
    remote_names.dedup();
    let remote_tags: HashSet<(&str, &str)> = repo
        .and_then(|r| match &r.remote_tags {
            Loadable::Ready(tags) => Some(
                tags.iter()
                    .map(|tag| (tag.remote.as_str(), tag.name.as_str()))
                    .collect::<HashSet<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default();

    (remote_names, remote_tags)
}

#[allow(clippy::too_many_arguments)]
fn tag_names_model(
    repo_id: RepoId,
    title: SharedString,
    tag_names: Vec<String>,
    remote_names: Vec<String>,
    remote_tags: HashSet<(&str, &str)>,
    commit_id: &CommitId,
    compare_label: String,
    comparison_mark: Option<(CommitId, String)>,
) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header(title.into())];
    if tag_names.is_empty() {
        items.push(ContextMenuItem::Label("No tags".into()));
        return ContextMenuModel::new(items);
    }

    items.push(ContextMenuItem::Separator);
    // Comparison: mark this tag's commit, or compare it against a mark.
    items.push(ContextMenuItem::Entry {
        label: format!("Mark {compare_label} for comparison").into(),
        icon: Some("icons/tag.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::MarkForComparison {
            repo_id,
            commit_id: commit_id.clone(),
            label: compare_label.clone(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Compare with working tree".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::CompareWithWorkingTree {
            repo_id,
            commit_id: commit_id.clone(),
            label: compare_label.clone(),
        }),
    });
    if let Some(mark_label) = comparison_mark
        .filter(|(mark_commit, _)| mark_commit != commit_id)
        .map(|(_, label)| label)
    {
        items.push(ContextMenuItem::Entry {
            label: format!("Compare with {mark_label}").into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CompareWithMarked {
                repo_id,
                commit_id: commit_id.clone(),
                label: compare_label.clone(),
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Clear comparison mark".into(),
            icon: Some("icons/generic_close.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::ClearComparisonMark { repo_id }),
        });
    }
    items.push(ContextMenuItem::Separator);
    for (tag_ix, name) in tag_names.into_iter().enumerate() {
        if tag_ix > 0 {
            items.push(ContextMenuItem::Separator);
        }
        items.push(ContextMenuItem::Entry {
            label: format!("Delete tag {name}").into(),
            icon: Some("icons/trash.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::DeleteTag {
                repo_id,
                name: name.clone(),
            }),
        });

        for remote in &remote_names {
            items.push(ContextMenuItem::Entry {
                label: format!("Push tag {name} to {remote}").into(),
                icon: Some("icons/arrow_up.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::PushTag {
                    repo_id,
                    remote: remote.clone(),
                    name: name.clone(),
                }),
            });
            if remote_tags.contains(&(remote.as_str(), name.as_str())) {
                items.push(ContextMenuItem::Entry {
                    label: format!("Delete tag {name} from {remote}").into(),
                    icon: Some("icons/trash.svg".into()),
                    shortcut: None,
                    disabled: false,
                    action: Box::new(ContextMenuAction::DeleteRemoteTag {
                        repo_id,
                        remote: remote.clone(),
                        name: name.clone(),
                    }),
                });
            }
        }
    }

    ContextMenuModel::new(items)
}
