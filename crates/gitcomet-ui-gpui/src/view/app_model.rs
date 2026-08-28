use super::*;

#[derive(Debug)]
pub(super) struct AppUiModel {
    pub(super) state: Arc<AppState>,
    pub(super) preferences: UiPreferences,
    pub(super) seq: u64,
}

impl AppUiModel {
    #[cfg(test)]
    pub(super) fn new(state: Arc<AppState>) -> Self {
        Self::new_with_preferences(state, UiPreferences::default())
    }

    pub(super) fn new_with_preferences(state: Arc<AppState>, preferences: UiPreferences) -> Self {
        Self {
            state,
            preferences,
            seq: 0,
        }
    }

    pub(super) fn set_state(&mut self, state: Arc<AppState>, cx: &mut gpui::Context<Self>) {
        self.state = state;
        self.seq = self.seq.wrapping_add(1);
        cx.notify();
    }

    pub(super) fn update_preferences(
        &mut self,
        update: impl FnOnce(&mut UiPreferences),
        cx: &mut gpui::Context<Self>,
    ) {
        update(&mut self.preferences);
        self.seq = self.seq.wrapping_add(1);
        cx.notify();
    }
}
