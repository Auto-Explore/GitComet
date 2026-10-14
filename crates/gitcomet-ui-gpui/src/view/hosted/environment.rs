use crate::view::AppUiModel;
use crate::view::panes::diff::services::DiffServices;
use gpui::{AnyWindowHandle, App, WeakEntity};

#[derive(Clone)]
pub(in crate::view) enum DiffPreferences {
    Window(WeakEntity<AppUiModel>),
    Fixed(crate::view::preferences::UiPreferences),
}

impl DiffPreferences {
    pub fn read(&self, cx: &App) -> crate::view::preferences::UiPreferences {
        match self {
            Self::Window(model) => model
                .read_with(cx, |model, _| model.preferences.clone())
                .unwrap_or_default(),
            Self::Fixed(preferences) => preferences.clone(),
        }
    }
}

#[derive(Clone)]
pub(in crate::view) struct DiffPaneEnvironment {
    pub services: DiffServices,
    pub preferences: DiffPreferences,
    pub window: AnyWindowHandle,
}
