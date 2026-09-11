use super::*;

/// The complete row context, independent of which painted chip was clicked.
pub(super) fn model(this: &PopoverHost, repo_id: RepoId, commit_id: &CommitId) -> ContextMenuModel {
    let commit = commit::model(this, repo_id, commit_id);
    let sha = commit_id.as_ref();
    let mut model = ContextMenuModel::new(vec![ContextMenuItem::Header(
        format!("Commit {}", sha.get(..8).unwrap_or(sha)).into(),
    )]);
    if let Some(ContextMenuItem::Label(summary)) = commit.items.get(1) {
        model.items.push(ContextMenuItem::Label(summary.clone()));
    }
    model.items.push(ContextMenuItem::Separator);

    let Some(repo) = this.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return model;
    };
    let mut refs = Vec::new();
    if let Some(branches) = repo.branches.ready() {
        refs.extend(
            branches
                .iter()
                .filter(|branch| branch.target == *commit_id)
                .map(|branch| HistoryMenuRef::Branch(BranchMenuTarget::local(&branch.name))),
        );
    }
    if let Some(branches) = repo.remote_branches.ready() {
        refs.extend(
            branches
                .iter()
                .filter(|branch| branch.target == *commit_id)
                .map(|branch| {
                    HistoryMenuRef::Branch(BranchMenuTarget::remote(&branch.remote, &branch.name))
                }),
        );
    }
    if let Some(tags) = repo.tags.ready() {
        refs.extend(
            tags.iter()
                .filter(|tag| tag.target == *commit_id)
                .map(|tag| HistoryMenuRef::Tag(tag.name.clone())),
        );
    }
    refs.sort();
    refs.dedup();
    for target in refs {
        let expanded = this.expanded_history_ref.as_ref() == Some(&target);
        let label = match &target {
            HistoryMenuRef::Branch(BranchMenuTarget::Local { name }) => {
                format!("Local branch {name}")
            }
            HistoryMenuRef::Branch(BranchMenuTarget::Remote { remote, branch }) => {
                format!("Remote branch {remote}/{branch}")
            }
            HistoryMenuRef::Tag(name) => format!("Tag {name}"),
        };
        let index = model.items.len();
        model
            .entry_debug_selectors
            .insert(index, format!("history_ref_group_{target:?}").into());
        model.entry_tooltips.insert(index, label.clone().into());
        model.items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some(
                if expanded {
                    "icons/chevron_down.svg"
                } else {
                    "icons/chevron_right.svg"
                }
                .into(),
            ),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::ToggleHistoryRefGroup {
                target: target.clone(),
            }),
        });
        if expanded {
            let actions = match &target {
                HistoryMenuRef::Branch(target) => branch::model(this, repo_id, target),
                HistoryMenuRef::Tag(name) => tag::model_for_tag(this, repo_id, commit_id, name),
            };
            append_actions(&mut model, actions, true);
            model.items.push(ContextMenuItem::Separator);
        }
    }
    if !matches!(&repo.tags, Loadable::Ready(_)) {
        model.items.push(ContextMenuItem::Label(match &repo.tags {
            Loadable::Error(error) => format!("Tags unavailable: {error}").into(),
            _ => "Loading tags…".into(),
        }));
    }
    if model.items.len() > 3 {
        model.items.push(ContextMenuItem::Separator);
    }
    append_actions(&mut model, commit, false);
    model
}

/// Keep tooltip/debug metadata attached to its action while composing menus.
fn append_actions(model: &mut ContextMenuModel, source: ContextMenuModel, clear_shortcuts: bool) {
    let first = source
        .items
        .iter()
        .position(|item| matches!(item, ContextMenuItem::Entry { .. }))
        .unwrap_or(source.items.len());
    for (old_index, mut item) in source.items.into_iter().enumerate().skip(first) {
        if clear_shortcuts && let ContextMenuItem::Entry { shortcut, .. } = &mut item {
            *shortcut = None;
        }
        let index = model.items.len();
        if let Some(tooltip) = source.entry_tooltips.get(&old_index) {
            model.entry_tooltips.insert(index, tooltip.clone());
        }
        if let Some(selector) = source.entry_debug_selectors.get(&old_index) {
            model.entry_debug_selectors.insert(index, selector.clone());
        }
        model.items.push(item);
    }
}
