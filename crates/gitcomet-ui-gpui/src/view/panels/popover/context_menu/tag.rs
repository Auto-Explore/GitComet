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

    tag_names_model(
        repo_id,
        format!("Tags on {short}").into(),
        tag_names,
        remote_names,
        remote_tags,
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
    tag_names_model(
        repo_id,
        format!("Tag {name} on {short}").into(),
        vec![name.clone()],
        remote_names,
        remote_tags,
    )
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

fn tag_names_model(
    repo_id: RepoId,
    title: SharedString,
    tag_names: Vec<String>,
    remote_names: Vec<String>,
    remote_tags: HashSet<(&str, &str)>,
) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header(title.into())];
    if tag_names.is_empty() {
        items.push(ContextMenuItem::Label("No tags".into()));
        return ContextMenuModel::new(items);
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
