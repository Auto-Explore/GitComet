use super::*;

/// The model the whole view tree observes.
///
/// `state` is the live store snapshot: writing it notifies every observer.
/// `preferences` is a plain field the pane constructors read once, when the
/// window is built — no observer reads it, so keeping it current is bookkeeping
/// for the next construction rather than a change anyone renders from.
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

    /// Deliberately does not notify. The observers all read `state`, and
    /// `GitCometView::apply_state_snapshot` is a diff against the snapshot it
    /// already holds: waking them for a preference write re-runs that whole pass
    /// (plus, on macOS, a `session::load()` disk read) and always concludes that
    /// nothing changed. Every preference setter already propagates its own value
    /// to the panes that render it.
    pub(super) fn update_preferences(&mut self, update: impl FnOnce(&mut UiPreferences)) {
        update(&mut self.preferences);
    }
}
