use gpui::Decorations;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct LinuxGuiEnvironment {
    pub(crate) is_wsl: bool,
    pub(crate) has_x11: bool,
    pub(crate) has_wayland: bool,
    pub(crate) has_xdg_runtime_dir: bool,
}

impl LinuxGuiEnvironment {
    #[cfg(target_os = "linux")]
    pub(crate) fn detect() -> Self {
        let osrelease = read_linux_osrelease();
        Self::from_sources(
            detect_is_wsl(
                env_has_non_empty_os(std::env::var_os("WSL_DISTRO_NAME").as_deref()),
                env_has_non_empty_os(std::env::var_os("WSL_INTEROP").as_deref()),
                osrelease.as_deref(),
            ),
            env_has_non_empty_os(std::env::var_os("DISPLAY").as_deref()),
            env_has_non_empty_os(std::env::var_os("WAYLAND_DISPLAY").as_deref()),
            env_has_non_empty_os(std::env::var_os("XDG_RUNTIME_DIR").as_deref()),
        )
    }

    #[cfg(any(target_os = "linux", test))]
    fn from_sources(
        is_wsl: bool,
        has_x11: bool,
        has_wayland: bool,
        has_xdg_runtime_dir: bool,
    ) -> Self {
        Self {
            is_wsl,
            has_x11,
            has_wayland,
            has_xdg_runtime_dir,
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn session_is_gui_capable(&self) -> bool {
        self.has_x11 || (self.has_wayland && self.has_xdg_runtime_dir)
    }

    pub(crate) fn should_suppress_custom_window_frame(decorations: Decorations) -> bool {
        !cfg!(target_os = "macos") && matches!(decorations, Decorations::Server)
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn launch_failure_message(&self) -> String {
        if self.is_wsl {
            if self.has_wayland && !self.has_xdg_runtime_dir && !self.has_x11 {
                return "WAYLAND_DISPLAY is set in this WSL environment, but XDG_RUNTIME_DIR is missing. Start GitComet from a WSLg-enabled terminal or ensure the WSLg session variables are exported.".to_string();
            }
            return "No GUI session detected in this WSL environment. GitComet requires WSLg with WAYLAND_DISPLAY or DISPLAY set to open windows.".to_string();
        }

        if self.has_wayland && !self.has_xdg_runtime_dir && !self.has_x11 {
            return "WAYLAND_DISPLAY is set, but XDG_RUNTIME_DIR is missing. Launch GitComet from an active desktop session.".to_string();
        }

        "No GUI session detected. GitComet requires an X11 or Wayland session to open windows. Launch it from an active desktop session.".to_string()
    }
}

#[cfg(any(target_os = "linux", test))]
fn env_has_non_empty_os(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

#[cfg(any(target_os = "linux", test))]
fn detect_is_wsl(
    has_wsl_distro_name: bool,
    has_wsl_interop: bool,
    osrelease: Option<&str>,
) -> bool {
    has_wsl_distro_name || has_wsl_interop || osrelease_mentions_microsoft(osrelease)
}

#[cfg(any(target_os = "linux", test))]
fn osrelease_mentions_microsoft(osrelease: Option<&str>) -> bool {
    osrelease
        .map(|value| value.to_ascii_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn read_linux_osrelease() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Tiling;

    #[test]
    fn detect_is_wsl_prefers_explicit_wsl_environment_variables() {
        assert!(detect_is_wsl(true, false, None));
        assert!(detect_is_wsl(false, true, None));
    }

    #[test]
    fn detect_is_wsl_falls_back_to_linux_osrelease() {
        assert!(detect_is_wsl(
            false,
            false,
            Some("6.6.87.2-microsoft-standard-WSL2")
        ));
        assert!(!detect_is_wsl(false, false, Some("6.8.0-generic")));
    }

    #[test]
    fn session_is_gui_capable_accepts_x11_without_xdg_runtime_dir() {
        let env = LinuxGuiEnvironment::from_sources(false, true, false, false);
        assert!(env.session_is_gui_capable());
    }

    #[test]
    fn session_is_gui_capable_requires_xdg_runtime_dir_for_wayland_only_sessions() {
        let env = LinuxGuiEnvironment::from_sources(false, false, true, false);
        assert!(!env.session_is_gui_capable());
    }

    #[test]
    fn suppresses_custom_window_frame_for_server_decorations() {
        assert!(LinuxGuiEnvironment::should_suppress_custom_window_frame(
            Decorations::Server
        ));
        assert!(!LinuxGuiEnvironment::should_suppress_custom_window_frame(
            Decorations::Client {
                tiling: Tiling::default(),
            }
        ));
    }

    #[test]
    fn launch_failure_message_mentions_wslg_for_wsl_sessions_without_display() {
        let env = LinuxGuiEnvironment::from_sources(true, false, false, false);
        let message = env.launch_failure_message();
        assert!(message.contains("WSLg"));
        assert!(message.contains("WAYLAND_DISPLAY"));
    }

    #[test]
    fn launch_failure_message_mentions_xdg_runtime_for_wayland_only_sessions() {
        let env = LinuxGuiEnvironment::from_sources(false, false, true, false);
        let message = env.launch_failure_message();
        assert!(message.contains("XDG_RUNTIME_DIR"));
    }
}
