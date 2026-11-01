use super::*;
use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet};

/// The refs the repository's history is currently focused on.
pub(super) fn active_focus(host: &PopoverHost, repo_id: RepoId) -> HistoryFocusSet {
    host.state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| repo.history_state.history_focus.clone())
        .unwrap_or_default()
}

/// The "Focus" / "Stop focus" entry for one ref.
///
/// The entry says what picking it will do: "Focus" adds the ref to the set,
/// "Stop focus" takes it back out. A branch under a focused remote reads as
/// focused, because its commits are already in view.
pub(super) fn entry(host: &PopoverHost, repo_id: RepoId, target: HistoryFocus) -> ContextMenuItem {
    let in_history_focus = active_focus(host, repo_id).covers(&target);
    ContextMenuItem::Entry {
        label: if in_history_focus {
            "Stop focus"
        } else {
            "Focus"
        }
        .into(),
        // The same eyes the sidebar draws: open to focus, crossed out to stop.
        icon: Some(
            if in_history_focus {
                "icons/eye_off.svg"
            } else {
                "icons/eye.svg"
            }
            .into(),
        ),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::ToggleHistoryFocus { repo_id, target }),
    }
}
