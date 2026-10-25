use super::*;

#[derive(Debug)]
pub(super) struct AppUiModel {
    pub(super) state: Arc<AppState>,
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
        Self {
            state,
            preferences,
            documents: documents::buffers::DocumentOwner::new(store, cx),
        }
    }

    pub(super) fn set_state(&mut self, state: Arc<AppState>, cx: &mut gpui::Context<Self>) {
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
