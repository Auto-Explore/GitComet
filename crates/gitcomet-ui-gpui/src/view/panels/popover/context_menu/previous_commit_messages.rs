use super::*;
use gitcomet_core::domain::RecentCommitMessage;
use std::sync::Arc;

fn first_message_line(message: &str, fallback: &str) -> SharedString {
    message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
        .into()
}

pub(super) fn model(this: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    model_for_recent_commit_messages(repo.map(|repo| &repo.recent_commit_messages))
}

fn model_for_recent_commit_messages(
    messages: Option<&Loadable<Arc<Vec<RecentCommitMessage>>>>,
) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header("Previous commit messages".into())];

    match messages {
        Some(Loadable::Ready(messages)) if !messages.is_empty() => {
            items.push(ContextMenuItem::Separator);
            for message in messages.iter() {
                items.push(ContextMenuItem::Entry {
                    label: first_message_line(&message.message, &message.summary),
                    icon: Some("icons/file.svg".into()),
                    shortcut: None,
                    disabled: false,
                    action: Box::new(ContextMenuAction::UseCommitMessage {
                        message: message.message.clone(),
                    }),
                });
            }
        }
        Some(Loadable::Loading) => {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::Label("Loading...".into()));
        }
        Some(Loadable::Error(error)) => {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::Label(error.clone().into()));
        }
        Some(Loadable::Ready(_)) | Some(Loadable::NotLoaded) | None => {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::Label("No previous commit messages".into()));
        }
    }

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{CommitId, RecentCommitMessage};
    use std::sync::Arc;

    #[test]
    fn model_uses_first_non_empty_line_and_full_message_action() {
        let messages = Arc::new(vec![RecentCommitMessage {
            id: CommitId(Arc::from("abc123")),
            summary: Arc::from("fallback summary"),
            message: "\n\nsubject\n\nbody".to_string(),
        }]);
        let loadable = Loadable::Ready(messages);
        let model = model_for_recent_commit_messages(Some(&loadable));

        let entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } => Some((label, action)),
            _ => None,
        });

        assert!(matches!(entry, Some((label, action))
            if label.as_ref() == "subject"
                && matches!(action.as_ref(), ContextMenuAction::UseCommitMessage { message } if message == "\n\nsubject\n\nbody")
        ));
    }

    #[test]
    fn model_handles_empty_state_without_actions() {
        let messages = Arc::new(Vec::new());
        let loadable = Loadable::Ready(messages);
        let model = model_for_recent_commit_messages(Some(&loadable));

        assert!(
            !model
                .items
                .iter()
                .any(|item| matches!(item, ContextMenuItem::Entry { .. }))
        );
    }
}
