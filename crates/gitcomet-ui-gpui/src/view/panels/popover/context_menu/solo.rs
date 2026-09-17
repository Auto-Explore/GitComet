use super::*;
use gitcomet_core::domain::{HistorySolo, HistorySoloSet};

/// The refs the repository's history is currently soloed on.
pub(super) fn active_solo(host: &PopoverHost, repo_id: RepoId) -> HistorySoloSet {
    host.state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| repo.history_state.history_solo.clone())
        .unwrap_or_default()
}

/// The "Solo" entry for one ref.
///
/// Solo is a per-ref toggle, so the entry never renames itself: it is always
/// "Solo", and a tick says whether this ref is one of the soloed ones. Picking
/// it again on a soloed ref takes it back out of the set.
pub(super) fn entry(host: &PopoverHost, repo_id: RepoId, target: HistorySolo) -> ContextMenuItem {
    let active = active_solo(host, repo_id).contains(&target);
    ContextMenuItem::Entry {
        label: "Solo".into(),
        icon: active.then(|| "icons/check.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::ToggleHistorySolo { repo_id, target }),
    }
}
