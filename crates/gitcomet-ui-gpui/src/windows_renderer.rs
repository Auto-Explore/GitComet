//! Saved Windows renderer policy and the result of this process's selection.
use gitcomet_state::session;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum RendererPreference {
    #[default]
    Auto,
    Dx11,
}

impl RendererPreference {
    pub(crate) fn parse(value: Option<&str>) -> Self {
        if value == Some("dx11") {
            Self::Dx11
        } else {
            Self::Auto
        }
    }
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dx11 => "dx11",
        }
    }
    #[cfg(target_os = "windows")]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Dx11 => "DirectX 11 — compatibility",
        }
    }
}

pub(crate) fn preference_settings(value: RendererPreference) -> session::UiSettings {
    session::UiSettings {
        windows_renderer: Some(value.key().into()),
        ..Default::default()
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use crate::preference_writer::PreferenceWriter;
    use gpui_platform::{
        WindowsRendererBackend as Backend, WindowsRendererEvent as Event, WindowsRendererOptions,
        WindowsRendererPreference,
    };
    use std::{cell::RefCell, rc::Rc};

    pub(crate) struct RendererState {
        pub writer: PreferenceWriter,
        pub saved: RendererPreference,
        pub applied: RendererPreference,
        pub active: Option<Backend>,
        pub overridden: bool,
        pub saving: bool,
        pub fallback: bool,
        pub error: Option<String>,
        pub restart_allowed: bool,
    }

    #[derive(Clone)]
    pub(crate) struct RendererSession(pub Rc<RefCell<RendererState>>);
    impl gpui::Global for RendererSession {}

    impl RendererState {
        pub fn new(saved: RendererPreference, restart_allowed: bool) -> Self {
            Self {
                writer: PreferenceWriter::default(),
                saved,
                applied: saved,
                active: None,
                overridden: false,
                saving: false,
                fallback: false,
                error: None,
                restart_allowed,
            }
        }
        pub fn restart_required(&self) -> bool {
            self.saved != self.applied && !self.overridden
        }
        pub fn status_message(&self) -> String {
            let active = match self.active {
                Some(Backend::Dx12) => "DirectX 12",
                Some(Backend::Dx11) => "DirectX 11",
                None => "Starting",
            };
            let mut status = format!("Active: {active}.");
            if self.overridden {
                status.push_str(" Controlled by GPUI_WINDOWS_RENDERER.");
            }
            if self.restart_required() {
                status.push_str(" Restart required.");
            }
            if !self.restart_allowed {
                status.push_str(" Changes apply on the next launch.");
            }
            if self.fallback {
                status.push_str(" DirectX 12 unavailable.");
                if self.saved == RendererPreference::Dx11 && self.error.is_none() {
                    status.push_str(" DirectX 11 saved for future launches.");
                }
            }
            if let Some(error) = &self.error {
                status.push(' ');
                status.push_str(error);
            }
            status
        }
        pub fn complete_manual_save(
            &mut self,
            sequence: u64,
            value: RendererPreference,
            result: std::io::Result<bool>,
        ) -> bool {
            self.saving = false;
            if !self.writer.is_current(sequence) {
                return false;
            }
            match result {
                Ok(true) => {
                    self.saved = value;
                    self.error = None;
                }
                Ok(false) => return false,
                Err(error) => {
                    self.error = Some(format!("Could not save graphics renderer: {error}"));
                    return false;
                }
            }
            self.restart_required() && self.restart_allowed
        }
        fn record_fallback(&mut self, runtime: bool, result: std::io::Result<bool>) {
            self.fallback = true;
            match result {
                Ok(true) => {
                    self.saved = RendererPreference::Dx11;
                    if !runtime {
                        self.applied = RendererPreference::Dx11;
                    }
                    self.error = None;
                }
                Ok(false) => unreachable!("foreground fallback is the latest renderer write"),
                Err(error) => {
                    let message = format!(
                        "Could not save DirectX 11 preference: {error}. The next launch may retry DirectX 12."
                    );
                    eprintln!("{message}");
                    self.error = Some(message);
                }
            }
        }
    }

    pub(crate) fn application(restart_allowed: bool) -> (gpui::Application, RendererSession) {
        let saved = RendererPreference::parse(session::load().windows_renderer.as_deref());
        let state = Rc::new(RefCell::new(RendererState::new(saved, restart_allowed)));
        let observer = state.clone();
        let options = WindowsRendererOptions {
            preference: match saved {
                RendererPreference::Auto => WindowsRendererPreference::Auto,
                RendererPreference::Dx11 => WindowsRendererPreference::Dx11,
            },
            on_event: Some(Rc::new(move |event| match event {
                Event::Failed { runtime, .. } => {
                    // Rare failure path: finish the narrow transaction before GPUI's fallback/fatal handling.
                    let writer = observer.borrow().writer.clone();
                    let sequence = writer.next();
                    let result = writer.persist(sequence, || {
                        session::persist_ui_settings(preference_settings(RendererPreference::Dx11))
                    });
                    observer.borrow_mut().record_fallback(runtime, result);
                }
                Event::Selected {
                    backend,
                    overridden,
                } => {
                    let mut state = observer.borrow_mut();
                    state.active = Some(backend);
                    state.overridden = overridden;
                }
            })),
        };
        (
            gpui_platform::application_with_windows_renderer(options),
            RendererSession(state),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn fallback_updates_saved_selection_but_runtime_keeps_applied_selection() {
            for runtime in [false, true] {
                let mut state = RendererState::new(RendererPreference::Auto, true);
                state.record_fallback(runtime, Ok(true));
                assert_eq!(state.saved, RendererPreference::Dx11);
                assert_eq!(state.restart_required(), runtime);
            }
        }
        #[test]
        fn save_failure_keeps_previous_selection_and_disables_restart_prompt() {
            let mut state = RendererState::new(RendererPreference::Auto, true);
            let sequence = state.writer.next();
            state.saving = true;
            assert!(!state.complete_manual_save(
                sequence,
                RendererPreference::Dx11,
                Err(std::io::Error::other("read only"))
            ));
            assert!(!state.saving);
            assert_eq!(state.saved, RendererPreference::Auto);
            assert!(state.error.is_some());
            state.record_fallback(false, Err(std::io::Error::other("read only")));
            assert_eq!(state.saved, RendererPreference::Auto);
            assert_eq!(state.applied, RendererPreference::Auto);
            assert!(!state.restart_required());
            assert!(!state.status_message().contains("Restart required."));
        }
        #[test]
        fn failed_manual_save_retains_an_existing_restart_requirement() {
            let mut state = RendererState::new(RendererPreference::Auto, true);
            let saved = state.writer.next();
            assert!(state.complete_manual_save(saved, RendererPreference::Dx11, Ok(true)));
            let failed = state.writer.next();
            assert!(!state.complete_manual_save(
                failed,
                RendererPreference::Auto,
                Err(std::io::Error::other("read only"))
            ));
            assert_eq!(state.saved, RendererPreference::Dx11);
            assert!(state.restart_required());
            assert!(state.status_message().contains("Restart required."));
        }
        #[test]
        fn delayed_completion_cannot_restore_automatic_after_fallback() {
            let mut state = RendererState::new(RendererPreference::Dx11, true);
            let manual = state.writer.next();
            state.writer.persist(manual, || Ok(())).unwrap();
            let fallback = state.writer.next();
            state.writer.persist(fallback, || Ok(())).unwrap();
            state.record_fallback(false, Ok(true));
            assert!(!state.complete_manual_save(manual, RendererPreference::Auto, Ok(true)));
            assert_eq!(state.saved, RendererPreference::Dx11);
        }
        #[test]
        fn retry_automatic_requires_restart_except_for_standalone_or_override() {
            for (allowed, overridden) in [(true, false), (false, false), (true, true)] {
                let mut state = RendererState::new(RendererPreference::Dx11, allowed);
                state.overridden = overridden;
                let sequence = state.writer.next();
                assert_eq!(
                    state.complete_manual_save(sequence, RendererPreference::Auto, Ok(true)),
                    allowed && !overridden
                );
                assert_eq!(state.saved, RendererPreference::Auto);
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) use windows::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_or_missing_preferences_use_automatic() {
        for value in [None, Some("auto"), Some("dx12"), Some("unknown")] {
            assert_eq!(RendererPreference::parse(value), RendererPreference::Auto);
        }
        assert_eq!(
            RendererPreference::parse(Some("dx11")),
            RendererPreference::Dx11
        );
    }
    #[test]
    fn renderer_write_only_changes_renderer() {
        let settings = preference_settings(RendererPreference::Dx11);
        assert_eq!(settings.windows_renderer.as_deref(), Some("dx11"));
        assert_eq!(settings.theme_mode, None);
        assert_eq!(settings.window_width, None);
    }
}
