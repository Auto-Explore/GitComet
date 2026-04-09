#[cfg(target_os = "linux")]
use crate::cli::{AppMode, exit_code};

const WL_COMPOSITOR_MIN_VERSION: u32 = 3;
const WL_SHM_MIN_VERSION: u32 = 1;
const WL_SEAT_MIN_VERSION: u32 = 5;
const WL_OUTPUT_MIN_VERSION: u32 = 2;
const XDG_WM_BASE_MIN_VERSION: u32 = 2;

#[cfg(target_os = "linux")]
const LINUX_X11_RELAUNCH_ENV: &str = "GITCOMET_SKIP_LINUX_X11_RELAUNCH";
#[cfg(target_os = "linux")]
const LINUX_X11_FALLBACK_DISABLE_ENV: &str = "GITCOMET_DISABLE_LINUX_X11_FALLBACK";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LinuxGuiLaunchEnvironment {
    has_wayland_display: bool,
    has_x11_display: bool,
    skip_x11_relaunch: bool,
    disable_x11_fallback: bool,
}

impl LinuxGuiLaunchEnvironment {
    #[cfg(target_os = "linux")]
    fn detect() -> Self {
        Self {
            has_wayland_display: env_var_is_non_empty("WAYLAND_DISPLAY"),
            has_x11_display: env_var_is_non_empty("DISPLAY"),
            skip_x11_relaunch: std::env::var_os(LINUX_X11_RELAUNCH_ENV).is_some(),
            disable_x11_fallback: std::env::var_os(LINUX_X11_FALLBACK_DISABLE_ENV).is_some(),
        }
    }

    #[cfg(test)]
    fn from_sources(
        has_wayland_display: bool,
        has_x11_display: bool,
        skip_x11_relaunch: bool,
        disable_x11_fallback: bool,
    ) -> Self {
        Self {
            has_wayland_display,
            has_x11_display,
            skip_x11_relaunch,
            disable_x11_fallback,
        }
    }

    fn should_preflight_wayland(self, mode_uses_gpui: bool) -> bool {
        mode_uses_gpui
            && self.has_wayland_display
            && !self.skip_x11_relaunch
            && !self.disable_x11_fallback
    }

    fn fallback_action_after_failed_wayland_preflight(self) -> LinuxWaylandFallbackAction {
        if self.has_x11_display {
            LinuxWaylandFallbackAction::RelaunchUnderX11
        } else {
            LinuxWaylandFallbackAction::ExitWithError
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LinuxWaylandFallbackAction {
    RelaunchUnderX11,
    ExitWithError,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WaylandAdvertisedGlobal {
    interface: String,
    version: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WaylandPreflightError {
    #[cfg(target_os = "linux")]
    Connection(String),
    #[cfg(target_os = "linux")]
    Registry(String),
    MissingGlobal {
        interface: &'static str,
        required_version: u32,
    },
    UnsupportedVersion {
        interface: &'static str,
        required_version: u32,
        advertised_version: u32,
    },
}

impl std::fmt::Display for WaylandPreflightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(target_os = "linux")]
            Self::Connection(err) => {
                write!(
                    f,
                    "failed to connect to the advertised Wayland compositor: {err}"
                )
            }
            #[cfg(target_os = "linux")]
            Self::Registry(err) => {
                write!(f, "failed to read the Wayland global registry: {err}")
            }
            Self::MissingGlobal {
                interface,
                required_version,
            } => write!(
                f,
                "required Wayland global `{interface}` was not advertised (need version {required_version}+)"
            ),
            Self::UnsupportedVersion {
                interface,
                required_version,
                advertised_version,
            } => write!(
                f,
                "Wayland global `{interface}` was advertised at version {advertised_version}, but GitComet requires version {required_version}+"
            ),
        }
    }
}

impl std::error::Error for WaylandPreflightError {}

#[cfg(target_os = "linux")]
pub(crate) fn maybe_relaunch_with_linux_x11_fallback(mode: &AppMode) -> Option<i32> {
    let env = LinuxGuiLaunchEnvironment::detect();
    if !env.should_preflight_wayland(app_mode_uses_gpui(mode)) {
        return None;
    }

    let err = match linux_wayland_preflight() {
        Ok(()) => return None,
        Err(err) => err,
    };

    eprintln!("Wayland session detected, but it is not usable for GitComet: {err}");

    match env.fallback_action_after_failed_wayland_preflight() {
        LinuxWaylandFallbackAction::ExitWithError => Some(exit_code::ERROR),
        LinuxWaylandFallbackAction::RelaunchUnderX11 => relaunch_under_x11_fallback(),
    }
}

#[cfg(target_os = "linux")]
fn app_mode_uses_gpui(mode: &AppMode) -> bool {
    match mode {
        AppMode::Browser { .. } => true,
        AppMode::Difftool(config) => config.gui,
        AppMode::Mergetool(config) => config.gui,
        AppMode::Setup { .. } | AppMode::Uninstall { .. } | AppMode::ExtractMergeFixtures(_) => {
            false
        }
    }
}

#[cfg(target_os = "linux")]
fn relaunch_under_x11_fallback() -> Option<i32> {
    let Ok(current_exe) = std::env::current_exe() else {
        eprintln!("Failed to locate the current executable for X11 fallback relaunch.");
        return Some(exit_code::ERROR);
    };

    eprintln!("Relaunching GitComet under X11 fallback.");

    let mut relaunch = std::process::Command::new(current_exe);
    relaunch.args(std::env::args_os().skip(1));
    relaunch.env(LINUX_X11_RELAUNCH_ENV, "1");
    relaunch.env("XDG_SESSION_TYPE", "x11");
    relaunch.env_remove("WAYLAND_DISPLAY");
    relaunch.env_remove("WAYLAND_SOCKET");

    match relaunch.spawn() {
        Ok(_) => Some(exit_code::SUCCESS),
        Err(err) => {
            eprintln!("Failed to relaunch under X11 fallback: {err}");
            Some(exit_code::ERROR)
        }
    }
}

#[cfg(target_os = "linux")]
fn env_var_is_non_empty(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn validate_wayland_advertised_globals(
    globals: &[WaylandAdvertisedGlobal],
) -> Result<(), WaylandPreflightError> {
    require_wayland_global(globals, "wl_compositor", WL_COMPOSITOR_MIN_VERSION)?;
    require_wayland_global(globals, "wl_shm", WL_SHM_MIN_VERSION)?;
    require_wayland_global(globals, "wl_seat", WL_SEAT_MIN_VERSION)?;
    require_wayland_global(globals, "xdg_wm_base", XDG_WM_BASE_MIN_VERSION)?;
    require_all_wayland_globals_at_least(globals, "wl_output", WL_OUTPUT_MIN_VERSION)?;
    Ok(())
}

fn require_wayland_global(
    globals: &[WaylandAdvertisedGlobal],
    interface: &'static str,
    required_version: u32,
) -> Result<u32, WaylandPreflightError> {
    let advertised_version = globals
        .iter()
        .filter(|global| global.interface == interface)
        .map(|global| global.version)
        .max()
        .ok_or(WaylandPreflightError::MissingGlobal {
            interface,
            required_version,
        })?;

    if advertised_version < required_version {
        return Err(WaylandPreflightError::UnsupportedVersion {
            interface,
            required_version,
            advertised_version,
        });
    }

    Ok(advertised_version)
}

fn require_all_wayland_globals_at_least(
    globals: &[WaylandAdvertisedGlobal],
    interface: &'static str,
    required_version: u32,
) -> Result<(), WaylandPreflightError> {
    for global in globals
        .iter()
        .filter(|global| global.interface == interface)
    {
        if global.version < required_version {
            return Err(WaylandPreflightError::UnsupportedVersion {
                interface,
                required_version,
                advertised_version: global.version,
            });
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct WaylandPreflightState;

#[cfg(target_os = "linux")]
impl
    wayland_client::Dispatch<
        wayland_client::protocol::wl_registry::WlRegistry,
        wayland_client::globals::GlobalListContents,
    > for WaylandPreflightState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_registry::WlRegistry,
        _event: wayland_client::protocol::wl_registry::Event,
        _data: &wayland_client::globals::GlobalListContents,
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

#[cfg(target_os = "linux")]
fn linux_wayland_preflight() -> Result<(), WaylandPreflightError> {
    let connection = wayland_client::Connection::connect_to_env()
        .map_err(|err| WaylandPreflightError::Connection(err.to_string()))?;
    let (globals, _queue) =
        wayland_client::globals::registry_queue_init::<WaylandPreflightState>(&connection)
            .map_err(|err| WaylandPreflightError::Registry(err.to_string()))?;

    let advertised_globals = globals.contents().with_list(|list| {
        list.iter()
            .map(|global| WaylandAdvertisedGlobal {
                interface: global.interface.clone(),
                version: global.version,
            })
            .collect::<Vec<_>>()
    });

    validate_wayland_advertised_globals(&advertised_globals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advertised_global(interface: &str, version: u32) -> WaylandAdvertisedGlobal {
        WaylandAdvertisedGlobal {
            interface: interface.to_string(),
            version,
        }
    }

    #[test]
    fn preflights_wayland_for_gpui_sessions_when_no_fallback_guard_is_set() {
        let env = LinuxGuiLaunchEnvironment::from_sources(true, true, false, false);
        assert!(env.should_preflight_wayland(true));
    }

    #[test]
    fn skips_wayland_preflight_when_fallback_is_already_in_progress() {
        let env = LinuxGuiLaunchEnvironment::from_sources(true, true, true, false);
        assert!(!env.should_preflight_wayland(true));
    }

    #[test]
    fn skips_wayland_preflight_when_x11_fallback_is_disabled() {
        let env = LinuxGuiLaunchEnvironment::from_sources(true, true, false, true);
        assert!(!env.should_preflight_wayland(true));
    }

    #[test]
    fn skips_wayland_preflight_for_headless_modes() {
        let env = LinuxGuiLaunchEnvironment::from_sources(true, true, false, false);
        assert!(!env.should_preflight_wayland(false));
    }

    #[test]
    fn skips_wayland_preflight_without_wayland_display() {
        let env = LinuxGuiLaunchEnvironment::from_sources(false, true, false, false);
        assert!(!env.should_preflight_wayland(true));
    }

    #[test]
    fn uses_x11_fallback_when_wayland_preflight_fails_and_x11_is_available() {
        let env = LinuxGuiLaunchEnvironment::from_sources(true, true, false, false);
        assert_eq!(
            env.fallback_action_after_failed_wayland_preflight(),
            LinuxWaylandFallbackAction::RelaunchUnderX11
        );
    }

    #[test]
    fn exits_when_wayland_preflight_fails_without_x11_fallback() {
        let env = LinuxGuiLaunchEnvironment::from_sources(true, false, false, false);
        assert_eq!(
            env.fallback_action_after_failed_wayland_preflight(),
            LinuxWaylandFallbackAction::ExitWithError
        );
    }

    #[test]
    fn validate_wayland_advertised_globals_accepts_required_globals() {
        let globals = vec![
            advertised_global("wl_compositor", WL_COMPOSITOR_MIN_VERSION),
            advertised_global("wl_shm", WL_SHM_MIN_VERSION),
            advertised_global("wl_seat", WL_SEAT_MIN_VERSION),
            advertised_global("xdg_wm_base", XDG_WM_BASE_MIN_VERSION),
            advertised_global("wl_output", WL_OUTPUT_MIN_VERSION),
        ];

        assert_eq!(validate_wayland_advertised_globals(&globals), Ok(()));
    }

    #[test]
    fn validate_wayland_advertised_globals_fails_when_xdg_wm_base_is_missing() {
        let globals = vec![
            advertised_global("wl_compositor", WL_COMPOSITOR_MIN_VERSION),
            advertised_global("wl_shm", WL_SHM_MIN_VERSION),
            advertised_global("wl_seat", WL_SEAT_MIN_VERSION),
        ];

        assert_eq!(
            validate_wayland_advertised_globals(&globals),
            Err(WaylandPreflightError::MissingGlobal {
                interface: "xdg_wm_base",
                required_version: XDG_WM_BASE_MIN_VERSION,
            })
        );
    }

    #[test]
    fn validate_wayland_advertised_globals_fails_when_xdg_wm_base_version_is_too_low() {
        let globals = vec![
            advertised_global("wl_compositor", WL_COMPOSITOR_MIN_VERSION),
            advertised_global("wl_shm", WL_SHM_MIN_VERSION),
            advertised_global("wl_seat", WL_SEAT_MIN_VERSION),
            advertised_global("xdg_wm_base", XDG_WM_BASE_MIN_VERSION - 1),
        ];

        assert_eq!(
            validate_wayland_advertised_globals(&globals),
            Err(WaylandPreflightError::UnsupportedVersion {
                interface: "xdg_wm_base",
                required_version: XDG_WM_BASE_MIN_VERSION,
                advertised_version: XDG_WM_BASE_MIN_VERSION - 1,
            })
        );
    }

    #[test]
    fn validate_wayland_advertised_globals_fails_when_wl_seat_version_is_too_low() {
        let globals = vec![
            advertised_global("wl_compositor", WL_COMPOSITOR_MIN_VERSION),
            advertised_global("wl_shm", WL_SHM_MIN_VERSION),
            advertised_global("wl_seat", WL_SEAT_MIN_VERSION - 1),
            advertised_global("xdg_wm_base", XDG_WM_BASE_MIN_VERSION),
        ];

        assert_eq!(
            validate_wayland_advertised_globals(&globals),
            Err(WaylandPreflightError::UnsupportedVersion {
                interface: "wl_seat",
                required_version: WL_SEAT_MIN_VERSION,
                advertised_version: WL_SEAT_MIN_VERSION - 1,
            })
        );
    }

    #[test]
    fn validate_wayland_advertised_globals_ignores_absent_wl_output() {
        let globals = vec![
            advertised_global("wl_compositor", WL_COMPOSITOR_MIN_VERSION),
            advertised_global("wl_shm", WL_SHM_MIN_VERSION),
            advertised_global("wl_seat", WL_SEAT_MIN_VERSION),
            advertised_global("xdg_wm_base", XDG_WM_BASE_MIN_VERSION),
        ];

        assert_eq!(validate_wayland_advertised_globals(&globals), Ok(()));
    }

    #[test]
    fn validate_wayland_advertised_globals_rejects_old_wl_output_versions() {
        let globals = vec![
            advertised_global("wl_compositor", WL_COMPOSITOR_MIN_VERSION),
            advertised_global("wl_shm", WL_SHM_MIN_VERSION),
            advertised_global("wl_seat", WL_SEAT_MIN_VERSION),
            advertised_global("xdg_wm_base", XDG_WM_BASE_MIN_VERSION),
            advertised_global("wl_output", WL_OUTPUT_MIN_VERSION - 1),
        ];

        assert_eq!(
            validate_wayland_advertised_globals(&globals),
            Err(WaylandPreflightError::UnsupportedVersion {
                interface: "wl_output",
                required_version: WL_OUTPUT_MIN_VERSION,
                advertised_version: WL_OUTPUT_MIN_VERSION - 1,
            })
        );
    }

    #[test]
    fn require_wayland_global_uses_highest_advertised_version() {
        let globals = vec![
            advertised_global("xdg_wm_base", 1),
            advertised_global("xdg_wm_base", 4),
            advertised_global("xdg_wm_base", 2),
        ];

        assert_eq!(
            require_wayland_global(&globals, "xdg_wm_base", XDG_WM_BASE_MIN_VERSION),
            Ok(4)
        );
    }
}
