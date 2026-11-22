use crate::history_filter::HistoryFilterMsg;
use crate::model::{AppState, Loadable};
use crate::msg::{Effect, PersistenceEffect};
use gitcomet_core::domain::LogScope;

pub(super) fn reduce(state: &mut AppState, message: HistoryFilterMsg) -> Vec<Effect> {
    match message {
        HistoryFilterMsg::SetScope { repo_id, scope } => set_history_scope(state, repo_id, scope),
        HistoryFilterMsg::SetAuthorFilter { repo_id, author } => {
            set_history_author_filter(state, repo_id, author)
        }
        HistoryFilterMsg::SetFocus { repo_id, focus } => set_history_focus(state, repo_id, focus),
        HistoryFilterMsg::ToggleFocus { repo_id, target } => {
            toggle_history_focus(state, repo_id, target)
        }
        HistoryFilterMsg::FocusRefs {
            repo_id,
            targets,
            focused,
        } => focus_history_refs(state, repo_id, &targets, focused),
    }
}

fn restart_history_load(
    state: &mut AppState,
    repo_ix: usize,
    persist: impl FnOnce(std::path::PathBuf) -> Effect,
) -> Vec<Effect> {
    let repo_state = &mut state.repos[repo_ix];
    repo_state.retain_log_while_loading();
    repo_state.set_log(Loadable::Loading);
    repo_state.set_log_loading_more(false);
    // Any count on screen belongs to the walk being replaced.
    repo_state.set_log_scan_progress(None);

    let mut effects = vec![persist(repo_state.spec.workdir.clone())];
    let request = super::util::first_page_log_request(repo_state);
    effects.extend(super::util::request_log_effect(repo_state, request));
    effects
}

pub(super) fn set_history_scope(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    scope: LogScope,
) -> Vec<Effect> {
    let Some(repo_ix) = state.repos.iter().position(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let shared_key = state.repos[repo_ix]
        .shared_preferences
        .as_ref()
        .map(|snapshot| snapshot.key.clone());
    if state.repos[repo_ix].history_state.history_scope == scope {
        // Another tab's choice may still be queued. Every explicit shared
        // selection must reach the queue, even when this tab needs no reload.
        return shared_key
            .map(|key| {
                Effect::Persistence(PersistenceEffect::UpdateRepositoryPreferences {
                    repo_id,
                    key,
                    update: crate::model::RepositoryPreferenceUpdate::HistoryMode(scope),
                })
            })
            .into_iter()
            .collect();
    }
    state.repos[repo_ix].set_log_scope(scope);

    restart_history_load(state, repo_ix, |workdir| match shared_key {
        Some(key) => Effect::Persistence(PersistenceEffect::UpdateRepositoryPreferences {
            repo_id,
            key,
            update: crate::model::RepositoryPreferenceUpdate::HistoryMode(scope),
        }),
        None => Effect::Persistence(PersistenceEffect::PersistRepoHistoryMode {
            repo_id: Some(repo_id),
            workdir,
            mode: scope,
            action: "updating history mode",
        }),
    })
}

pub(super) fn set_history_author_filter(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    author: Option<String>,
) -> Vec<Effect> {
    let Some(repo_ix) = state.repos.iter().position(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if state.repos[repo_ix].history_state.history_author_filter == author {
        return Vec::new();
    }
    state.repos[repo_ix].set_history_author_filter(author.clone());

    restart_history_load(state, repo_ix, |workdir| {
        Effect::Persistence(PersistenceEffect::PersistRepoHistoryAuthorFilter {
            repo_id: Some(repo_id),
            workdir,
            author,
            action: "updating history author filter",
        })
    })
}

pub(super) fn set_history_focus(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    focus: gitcomet_core::domain::HistoryFocusSet,
) -> Vec<Effect> {
    let Some(repo_ix) = state.repos.iter().position(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if state.repos[repo_ix].history_state.history_focus == focus {
        return Vec::new();
    }
    state.repos[repo_ix].set_history_focus(focus.clone());

    restart_history_load(state, repo_ix, |workdir| {
        Effect::Persistence(PersistenceEffect::PersistRepoHistoryFocus {
            repo_id: Some(repo_id),
            workdir,
            focus,
            action: "updating history focus",
        })
    })
}

/// Focuses `target`, or unfocuses it when the focus already covers it. Every
/// per-ref menu entry and sidebar eye goes through this, so the same click that
/// focused a ref is the one that unfocuses it.
///
/// Covers, not contains: a branch under a focused remote reads as focused, so
/// clicking it takes it out rather than adding an entry the remote already
/// implies.
pub(super) fn toggle_history_focus(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    target: gitcomet_core::domain::HistoryFocus,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let focused = !repo.history_state.history_focus.covers(&target);
    focus_history_refs(state, repo_id, &[target], focused)
}

/// Focuses or unfocuses every one of `targets` at once — a branch group's
/// members, or a single ref from [`toggle_history_focus`].
pub(super) fn focus_history_refs(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    targets: &[gitcomet_core::domain::HistoryFocus],
    focused: bool,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    // Unfocusing one branch of a focused remote replaces the remote with its
    // other branches, which only the loaded remote-branch list can name.
    let focus = repo
        .history_state
        .history_focus
        .with_refs_focused(targets, focused, |remote| match &repo.remote_branches {
            Loadable::Ready(branches) => branches
                .iter()
                .filter(|branch| branch.remote == remote)
                .map(|branch| branch.name.clone())
                .collect(),
            _ => Vec::new(),
        });
    set_history_focus(state, repo_id, focus)
}

/// Which ref list just loaded, and so which focus targets it can speak for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FocusRefList {
    LocalBranches,
    RemoteBranches,
    Remotes,
}

/// Clears focus targets whose refs have gone away.
///
/// A focused branch can be deleted or renamed locally, and a remote branch can
/// vanish under a pruning fetch. A focus pointing at nothing seeds no tips, so
/// the history would simply go empty; dropping such a target as soon as the ref
/// list that would contain it says it is gone restores the normal walk instead.
///
/// Absence is only evidence of deletion under two conditions, because clearing a
/// focus the user set is not something to guess at:
///
/// - the list that just loaded is the one that governs the target, so a remote
///   list arriving says nothing about a local branch; and
/// - that list loaded successfully. A list still loading or one that failed says
///   nothing, but a successful empty list does: removing the last remote or
///   pruning the last remote-tracking branch leaves exactly that. Reloads go
///   through `Loading`, and a remote-branch list caught mid-fetch is held back
///   before it gets here, so an empty `Ready` is never a transient.
pub(super) fn clear_missing_history_focus(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    loaded: FocusRefList,
) -> Vec<Effect> {
    use gitcomet_core::domain::HistoryFocus;

    let Some(repo) = state.repos.iter().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let focus = repo.history_state.history_focus.clone();
    if focus.is_empty() {
        return Vec::new();
    }
    /// `Some(list)` only when it loaded successfully; see above.
    fn conclusive<T>(list: &Loadable<std::sync::Arc<Vec<T>>>) -> Option<&[T]> {
        list.ready().map(|list| list.as_slice())
    }
    let kept = focus.retaining(|target| match (target, loaded) {
        (HistoryFocus::LocalBranch { name }, FocusRefList::LocalBranches) => {
            match conclusive(&repo.branches) {
                Some(branches) => branches.iter().any(|branch| branch.name == name.as_ref()),
                None => true,
            }
        }
        (HistoryFocus::RemoteBranch { remote, branch }, FocusRefList::RemoteBranches) => {
            match conclusive(&repo.remote_branches) {
                Some(branches) => branches.iter().any(|candidate| {
                    candidate.remote == remote.as_ref() && candidate.name == branch.as_ref()
                }),
                None => true,
            }
        }
        (HistoryFocus::Remote { name }, FocusRefList::Remotes) => match conclusive(&repo.remotes) {
            Some(remotes) => remotes.iter().any(|remote| remote.name == name.as_ref()),
            None => true,
        },
        // This list does not govern this target.
        _ => true,
    });
    if kept == focus {
        return Vec::new();
    }
    // Only the vanished refs drop out; the rest of the focus stands.
    set_history_focus(state, repo_id, kept)
}
