use crate::model::{
    AppState, Loadable, RepoId, RepositoryPreferenceUpdate, RepositoryPreferencesSnapshot,
};
use crate::msg::Effect;

pub(super) fn update(
    state: &AppState,
    repo_id: RepoId,
    update: RepositoryPreferenceUpdate,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo.open, Loadable::Ready(())) {
        return Vec::new();
    }
    vec![Effect::UpdateRepositoryPreferences {
        repo_id,
        key: repo.repository_key(),
        update,
    }]
}

pub(super) fn set_explorer_visibility(
    state: &mut AppState,
    repo_id: RepoId,
    hidden: Option<bool>,
    ignored: Option<bool>,
) -> Vec<Effect> {
    if state
        .repos
        .iter()
        .any(|repo| repo.id == repo_id && repo.shared_preferences.is_some())
    {
        update(
            state,
            repo_id,
            RepositoryPreferenceUpdate::ExplorerVisibility { hidden, ignored },
        )
    } else {
        let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id) else {
            return Vec::new();
        };
        let hidden = hidden.unwrap_or(repo.file_browser.show_hidden);
        let ignored = ignored.unwrap_or(repo.file_browser.show_ignored);
        apply_explorer_visibility(state, repo_id, hidden, ignored)
    }
}

fn apply_explorer_visibility(
    state: &mut AppState,
    repo_id: RepoId,
    hidden: bool,
    ignored: bool,
) -> Vec<Effect> {
    if let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo.file_browser.show_hidden = hidden;
        repo.file_browser.show_ignored = ignored;
        if !ignored {
            repo.file_browser.pending_recursive_expansions.clear();
        }
        // Keyboard actions must not reach rows that just disappeared.
        if !hidden {
            let revealed = &repo.file_browser.revealed_paths;
            repo.file_browser.selection.retain(|path| {
                !crate::explorer::is_hidden_path(path)
                    || revealed.iter().any(|shown| shown.starts_with(path))
            });
        }
        repo.file_browser.stale = true;
        repo.file_browser.bump_rev();
        return vec![Effect::LoadFileBrowser {
            repo_id,
            source: repo.file_browser.source.clone(),
        }];
    }
    vec![]
}

pub(super) fn apply(state: &mut AppState, snapshot: RepositoryPreferencesSnapshot) -> Vec<Effect> {
    let mut effects = Vec::new();
    for ix in 0..state.repos.len() {
        if state.repos[ix].repository_key() != snapshot.key
            || state.repos[ix]
                .shared_preferences
                .as_ref()
                .is_some_and(|current| current.revision >= snapshot.revision)
        {
            continue;
        }
        let repo_id = state.repos[ix].id;
        let preferences = &snapshot.preferences;
        if state.repos[ix].file_browser.show_hidden != preferences.show_hidden_files
            || state.repos[ix].file_browser.show_ignored != preferences.show_ignored_files
        {
            effects.extend(apply_explorer_visibility(
                state,
                repo_id,
                preferences.show_hidden_files,
                preferences.show_ignored_files,
            ));
        }
        let repo = &mut state.repos[ix];
        if repo.history_state.history_scope != preferences.history_mode {
            repo.set_log_scope(preferences.history_mode);
            repo.retain_log_while_loading();
            repo.set_log_loading_more(false);
            repo.set_log_scan_progress(None);
            if state.active_repo == Some(repo_id) {
                repo.set_log(Loadable::Loading);
                let request = super::util::first_page_log_request(repo);
                effects.extend(super::util::request_log_effect(repo, request));
            } else {
                repo.loads_in_flight.invalidate_log();
                repo.set_log(Loadable::NotLoaded);
            }
        }
        repo.shared_preferences = Some(snapshot.clone());
        repo.branch_sidebar_rev += 1;
    }
    effects
}
