use super::*;
use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet};
use gitcomet_state::history_filter::HistoryFilterMsg;

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
    labeled_entry(
        &active_focus(host, repo_id),
        repo_id,
        target,
        "Focus".into(),
        "Stop focus".into(),
    )
}

/// [`entry`] with its own labels, for menus that list several refs at once.
pub(super) fn labeled_entry(
    focus: &HistoryFocusSet,
    repo_id: RepoId,
    target: HistoryFocus,
    focus_label: SharedString,
    stop_label: SharedString,
) -> ContextMenuItem {
    let in_history_focus = focus.covers(&target);
    ContextMenuItem::Entry {
        label: if in_history_focus {
            stop_label
        } else {
            focus_label
        },
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
        action: Box::new(ContextMenuAction::HistoryFilter(
            HistoryFilterMsg::ToggleFocus { repo_id, target },
        )),
    }
}
