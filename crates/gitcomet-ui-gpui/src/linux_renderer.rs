use crate::preference_writer::PreferenceWriter;
use gitcomet_state::session;
use gpui_platform::{
    LinuxRendererBackend, LinuxRendererEvent, LinuxRendererOptions, LinuxRendererPreference,
};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum RendererPreference {
    #[default]
    Auto,
    Vulkan,
    Gl,
}

impl RendererPreference {
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("vulkan") => Self::Vulkan,
            Some("gl") => Self::Gl,
            _ => Self::Auto,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Vulkan => "vulkan",
            Self::Gl => "gl",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Vulkan => "Vulkan",
            Self::Gl => "OpenGL",
        }
    }
}

pub(crate) fn preference_settings(value: RendererPreference) -> session::UiSettings {
    session::UiSettings {
        linux_renderer: Some(value.key().into()),
        ..Default::default()
    }
}

pub(crate) struct RendererState {
    pub writer: PreferenceWriter,
    pub saved: RendererPreference,
    pub applied: RendererPreference,
    pub active: Option<LinuxRendererBackend>,
    pub overridden: bool,
    pub saving: bool,
    pub error: Option<String>,
    pub restart_allowed: bool,
    startup_session_path: Option<PathBuf>,
    startup_backend_override: Option<String>,
}

#[derive(Clone)]
pub(crate) struct RendererSession(pub Rc<RefCell<RendererState>>);
impl gpui::Global for RendererSession {}

impl RendererState {
    pub(crate) fn new(saved: RendererPreference, restart_allowed: bool) -> Self {
        Self {
            writer: PreferenceWriter::default(),
            saved,
            applied: saved,
            active: None,
            overridden: false,
            saving: false,
            error: None,
            restart_allowed,
            startup_session_path: None,
            startup_backend_override: None,
        }
    }

    fn from_startup(
        session_path: Option<PathBuf>,
        backend_override: Option<String>,
        restart_allowed: bool,
    ) -> Self {
        let session_path = session_path.map(|path| std::path::absolute(&path).unwrap_or(path));
        let session = session_path
            .as_deref()
            .map(session::load_from_path)
            .unwrap_or_default();
        let saved = RendererPreference::parse(session.linux_renderer.as_deref());
        let mut state = Self::new(saved, restart_allowed);
        state.startup_session_path = session_path;
        state.startup_backend_override = backend_override;
        state
    }

    pub(crate) fn startup_failure_help(&self) -> String {
        let mut help = format!(
            "Linux graphics renderer preference: {} ({}).",
            self.applied.label(),
            self.applied.key()
        );
        if let Some(value) = &self.startup_backend_override {
            help.push_str(&format!(
                " WGPU_BACKEND={value:?} overrides the saved preference for this launch. \
                 Unset WGPU_BACKEND to use the saved preference."
            ));
        }
        let Some(path) = &self.startup_session_path else {
            help.push_str(
                " No session settings file was loaded; persistence is disabled or unavailable.",
            );
            return help;
        };
        if path.is_absolute() {
            help.push_str(&format!(" Settings file: {path:?}."));
        } else {
            help.push_str(&format!(
                " Settings file (relative path; startup working directory could not be resolved): {path:?}."
            ));
        }
        help.push_str(
            " Close the app, then set \"linux_renderer\": \"auto\" or remove only that key in this file, \
             preserving the other session data. Then launch the app again."
        );
        help
    }

    pub(crate) fn restart_required(&self) -> bool {
        self.saved != self.applied && !self.overridden
    }

    pub(crate) fn status_message(&self) -> String {
        let active = match self.active {
            Some(LinuxRendererBackend::Vulkan) => "Vulkan",
            Some(LinuxRendererBackend::Gl) => "OpenGL",
            None => "Starting",
        };
        let mut status = format!("Active: {active}.");
        if self.overridden {
            status.push_str(" Controlled by WGPU_BACKEND.");
        }
        if self.restart_required() {
            status.push_str(" Restart required.");
        }
        if !self.restart_allowed {
            status.push_str(" Changes apply on the next launch.");
        }
        if let Some(error) = &self.error {
            status.push(' ');
            status.push_str(error);
        }
        status
    }

    pub(crate) fn complete_manual_save(
        &mut self,
        sequence: u64,
        value: RendererPreference,
        result: std::io::Result<bool>,
    ) -> bool {
        if !self.writer.is_current(sequence) {
            return false;
        }
        self.saving = false;
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
}

pub(crate) fn application(restart_allowed: bool) -> (gpui::Application, RendererSession) {
    let state = RendererState::from_startup(
        session::default_session_file_path(),
        std::env::var("WGPU_BACKEND").ok(),
        restart_allowed,
    );
    let saved = state.applied;
    let state = Rc::new(RefCell::new(state));
    let observer = state.clone();
    let options = LinuxRendererOptions {
        preference: match saved {
            RendererPreference::Auto => LinuxRendererPreference::Auto,
            RendererPreference::Vulkan => LinuxRendererPreference::Vulkan,
            RendererPreference::Gl => LinuxRendererPreference::Gl,
        },
        on_event: Some(Rc::new(move |event| match event {
            LinuxRendererEvent::Selected {
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
        gpui_platform::application_with_linux_renderer(options),
        RendererSession(state),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_failure_help_identifies_the_saved_renderer_and_its_exact_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom-session.json");
        for (key, label) in [
            ("auto", "Automatic"),
            ("vulkan", "Vulkan"),
            ("gl", "OpenGL"),
        ] {
            let contents =
                format!(r#"{{"version":6,"linux_renderer":"{key}","theme_mode":"light"}}"#);
            std::fs::write(&path, &contents).unwrap();
            let mut state = RendererState::from_startup(Some(path.clone()), None, true);
            state.saved = RendererPreference::Vulkan;
            assert_eq!(state.active, None);
            assert!(!state.overridden);
            let help = state.startup_failure_help();
            assert!(help.contains(&format!(
                "Linux graphics renderer preference: {label} ({key})."
            )));
            assert!(help.contains(&format!("Settings file: {path:?}")));
            assert!(help.contains(r#"set "linux_renderer": "auto" or remove only that key"#));
            assert!(help.contains("preserving the other session data"));
            assert!(!help.contains("WGPU_BACKEND"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
        }
    }

    #[test]
    fn startup_override_is_reported_even_before_a_backend_was_selected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, r#"{"version":6,"linux_renderer":"gl"}"#).unwrap();
        let state = RendererState::from_startup(Some(path), Some("vulkan,gl".into()), false);
        assert!(!state.overridden);
        assert_eq!(state.active, None);
        let help = state.startup_failure_help();
        assert!(help.contains("Linux graphics renderer preference: OpenGL (gl)."));
        assert!(help.contains(r#"WGPU_BACKEND="vulkan,gl" overrides the saved preference"#));
        assert!(help.contains("Unset WGPU_BACKEND"));
    }

    #[test]
    fn startup_relative_settings_path_is_anchored_without_requiring_the_file_to_exist() {
        let cwd = std::env::current_dir().unwrap();
        let dir = tempfile::tempdir_in(&cwd).unwrap();
        let relative = dir
            .path()
            .strip_prefix(&cwd)
            .unwrap()
            .join("missing-session.json");
        let expected = cwd.join(&relative);
        let state = RendererState::from_startup(Some(relative), None, true);
        assert!(
            state
                .startup_failure_help()
                .contains(&format!("Settings file: {expected:?}"))
        );
    }

    #[test]
    fn startup_without_session_persistence_does_not_invent_a_settings_file() {
        let state = RendererState::from_startup(None, None, true);
        let help = state.startup_failure_help();
        assert!(help.contains("No session settings file was loaded"));
        assert!(!help.contains("Settings file:"));
        assert!(!help.contains("remove only that key"));
    }

    #[test]
    fn persisted_preferences_select_each_backend_and_unknown_values_use_automatic() {
        for (value, expected) in [
            (None, RendererPreference::Auto),
            (Some("auto"), RendererPreference::Auto),
            (Some("vulkan"), RendererPreference::Vulkan),
            (Some("gl"), RendererPreference::Gl),
            (Some("dx11"), RendererPreference::Auto),
            (Some("unknown"), RendererPreference::Auto),
        ] {
            assert_eq!(RendererPreference::parse(value), expected);
        }
        for (value, key) in [
            (RendererPreference::Auto, "auto"),
            (RendererPreference::Vulkan, "vulkan"),
            (RendererPreference::Gl, "gl"),
        ] {
            assert_eq!(
                preference_settings(value),
                session::UiSettings {
                    linux_renderer: Some(key.into()),
                    ..Default::default()
                }
            );
        }
    }

    #[test]
    fn save_completes_before_restart_is_offered_and_keeps_the_active_backend() {
        let mut state = RendererState::new(RendererPreference::Auto, true);
        state.active = Some(LinuxRendererBackend::Vulkan);
        let sequence = state.writer.next();
        state.saving = true;
        assert!(!state.restart_required());
        assert!(state.complete_manual_save(sequence, RendererPreference::Gl, Ok(true)));
        assert!(!state.saving);
        assert_eq!(state.saved, RendererPreference::Gl);
        assert_eq!(state.applied, RendererPreference::Auto);
        assert_eq!(state.active, Some(LinuxRendererBackend::Vulkan));
        assert_eq!(state.status_message(), "Active: Vulkan. Restart required.");
    }

    #[test]
    fn failed_save_keeps_the_saved_preference_and_does_not_offer_restart() {
        let mut state = RendererState::new(RendererPreference::Auto, true);
        let sequence = state.writer.next();
        state.saving = true;
        assert!(!state.complete_manual_save(
            sequence,
            RendererPreference::Gl,
            Err(std::io::Error::other("read only")),
        ));
        assert!(!state.saving);
        assert_eq!(state.saved, RendererPreference::Auto);
        assert!(!state.restart_required());
        assert!(
            state
                .status_message()
                .contains("Could not save graphics renderer: read only")
        );
    }

    #[test]
    fn stale_completion_cannot_replace_a_newer_preference_or_finish_its_save() {
        let mut state = RendererState::new(RendererPreference::Auto, true);
        let stale = state.writer.next();
        let current = state.writer.next();
        state.saving = true;
        assert!(!state.complete_manual_save(stale, RendererPreference::Gl, Ok(true)));
        assert!(state.saving);
        assert_eq!(state.saved, RendererPreference::Auto);
        assert!(state.complete_manual_save(current, RendererPreference::Vulkan, Ok(true)));
        assert_eq!(state.saved, RendererPreference::Vulkan);
    }

    #[test]
    fn focused_launch_changes_are_saved_for_the_next_launch_without_a_restart_prompt() {
        let mut state = RendererState::new(RendererPreference::Gl, false);
        let sequence = state.writer.next();
        assert!(!state.complete_manual_save(sequence, RendererPreference::Auto, Ok(true)));
        assert_eq!(state.saved, RendererPreference::Auto);
        assert!(state.restart_required());
        assert!(
            state
                .status_message()
                .contains("Changes apply on the next launch.")
        );
    }

    #[test]
    fn environment_override_reports_the_active_backend_without_requiring_restart() {
        let mut state = RendererState::new(RendererPreference::Gl, true);
        state.active = Some(LinuxRendererBackend::Vulkan);
        state.overridden = true;
        let sequence = state.writer.next();
        assert!(!state.complete_manual_save(sequence, RendererPreference::Auto, Ok(true)));
        assert_eq!(state.saved, RendererPreference::Auto);
        assert!(!state.restart_required());
        assert_eq!(
            state.status_message(),
            "Active: Vulkan. Controlled by WGPU_BACKEND."
        );
    }
}
