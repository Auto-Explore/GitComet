use super::*;

pub(in super::super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
    path: &std::path::Path,
) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let commit = repo
        .and_then(|repo| repo.history_state.file_history.ready())
        .and_then(|page| page.commits.iter().find(|commit| commit.id == *commit_id));
    let sha = commit_id.as_ref();
    let mut items = vec![ContextMenuItem::Header(
        format!("Commit {}", sha.get(..8).unwrap_or(sha)).into(),
    )];
    if let Some(commit) = commit {
        items.push(ContextMenuItem::Label(
            components::ContextMenuText::new(format!("{} — {}", commit.author, commit.summary))
                .max_lines(4),
        ));
    }
    items.push(ContextMenuItem::Separator);
    let mut entry = |label: &str, icon: &'static str, disabled: bool, action: ContextMenuAction| {
        items.push(ContextMenuItem::Entry {
            label: label.to_owned().into(),
            icon: Some(icon.into()),
            shortcut: None,
            disabled,
            action: Box::new(action),
        });
    };
    entry(
        "Open file at this commit",
        "icons/file.svg",
        false,
        ContextMenuAction::OpenFileAtCommit {
            repo_id,
            commit_id: commit_id.clone(),
            path: path.to_path_buf(),
        },
    );
    entry(
        "Open file at parent",
        "icons/file.svg",
        commit.is_none_or(|commit| commit.parent_ids.is_empty()),
        ContextMenuAction::OpenFileAtCommitParent {
            repo_id,
            commit_id: commit_id.clone(),
            path: path.to_path_buf(),
        },
    );
    entry(
        "Show changes to this file",
        "icons/open_external.svg",
        false,
        ContextMenuAction::ShowFileChangesAtCommit {
            repo_id,
            commit_id: commit_id.clone(),
            path: path.to_path_buf(),
        },
    );
    entry(
        "Reveal in history",
        "icons/history.svg",
        false,
        ContextMenuAction::RevealHistoryCommit {
            repo_id,
            commit_id: commit_id.clone(),
        },
    );
    entry(
        "Copy SHA",
        "icons/copy.svg",
        false,
        ContextMenuAction::CopyText {
            text: sha.to_owned(),
        },
    );
    items.push(ContextMenuItem::Separator);
    let actions = super::commit::action_items(this, repo_id, commit_id);
    let offset = items.len();
    items.extend(actions.items);
    ContextMenuModel::new(items).with_entry_tooltips(
        actions
            .entry_tooltips
            .into_iter()
            .map(|(ix, text)| (ix + offset, text))
            .collect(),
    )
}
