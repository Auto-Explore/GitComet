use crate::preference_writer::PreferenceWriter;
use gitcomet_state::session;
use gpui_platform::{
    LinuxRendererBackend, LinuxRendererEvent, LinuxRendererOptions, LinuxRendererPreference,
};
use std::{cell::RefCell, rc::Rc};

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
        }
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
    let saved = RendererPreference::parse(session::load().linux_renderer.as_deref());
    let state = Rc::new(RefCell::new(RendererState::new(saved, restart_allowed)));
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
