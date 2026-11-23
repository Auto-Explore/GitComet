use super::*;
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Debug, Default)]
pub(super) struct ActiveRepoIndex(Rc<Cell<usize>>);

impl ActiveRepoIndex {
    fn update(&self, state: &AppState) {
        if let Some(index) = self.position(state) {
            self.0.set(index);
        }
    }

    pub(super) fn position(&self, state: &AppState) -> Option<usize> {
        let id = state.active_repo?;
        let index = self.0.get();
        if state.repos.get(index).is_some_and(|repo| repo.id == id) {
            Some(index)
        } else {
            state.repos.iter().position(|repo| repo.id == id)
        }
    }

    pub(super) fn get<'a>(&self, state: &'a AppState) -> Option<&'a RepoState> {
        state.repos.get(self.position(state)?)
    }
}

#[derive(Debug)]
pub(super) struct AppUiModel {
    pub(super) state: Arc<AppState>,
    pub(super) active_repo_index: ActiveRepoIndex,
    pub(super) publication: super::poller::PendingPublication,
    pub(super) preferences: UiPreferences,
    pub(super) documents: documents::buffers::DocumentOwner,
}

impl AppUiModel {
    #[cfg(test)]
    pub(super) fn new(
        state: Arc<AppState>,
        store: std::sync::Weak<AppStore>,
        cx: &mut App,
    ) -> Self {
        Self::new_with_preferences(state, UiPreferences::default(), store, cx)
    }

    pub(super) fn new_with_preferences(
        state: Arc<AppState>,
        preferences: UiPreferences,
        store: std::sync::Weak<AppStore>,
        cx: &mut App,
    ) -> Self {
        let active_repo_index = ActiveRepoIndex::default();
        active_repo_index.update(&state);
        Self {
            state,
            active_repo_index,
            publication: Default::default(),
            preferences,
            documents: documents::buffers::DocumentOwner::new(store, cx),
        }
    }

    pub(super) fn set_state(&mut self, state: Arc<AppState>, cx: &mut gpui::Context<Self>) {
        self.active_repo_index.update(&state);
        if !state.filesystem.completed.is_empty() {
            let owner = self.documents.clone();
            let state = Arc::clone(&state);
            cx.defer(move |cx| owner.finish_saves(&state, cx));
        }
        self.state = state;
        cx.notify();
    }

    pub(super) fn update_preferences(&mut self, update: impl FnOnce(&mut UiPreferences)) {
        update(&mut self.preferences);
    }
}
