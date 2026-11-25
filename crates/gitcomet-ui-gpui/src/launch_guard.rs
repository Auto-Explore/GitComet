use gitcomet_core::{panic::payload_message, startup_exit::StartupExitReason};
use std::any::Any;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiLaunchError {
    context: &'static str,
    message: String,
    kind: LaunchFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchFailure {
    Panic,
    ReturnedError,
    EventLoop(StartupExitReason),
}

impl UiLaunchError {
    pub fn event_loop_reason(&self) -> Option<StartupExitReason> {
        match self.kind {
            LaunchFailure::EventLoop(reason) => Some(reason),
            _ => None,
        }
    }

    pub(crate) fn from_event_loop(error: gpui::EventLoopError) -> Self {
        let reason = match error {
            gpui::EventLoopError::X11 => StartupExitReason::X11EventLoopError,
            gpui::EventLoopError::Wayland => StartupExitReason::WaylandEventLoopError,
        };
        Self {
            context: "GPUI event loop",
            message: reason.description().into(),
            kind: LaunchFailure::EventLoop(reason),
        }
    }

    pub fn context(&self) -> &'static str {
        self.context
    }

    pub fn panic_message(&self) -> &str {
        &self.message
    }

    pub(crate) fn from_panic(context: &'static str, payload: Box<dyn Any + Send>) -> Self {
        Self {
            context,
            message: payload_message(payload.as_ref()),
            kind: LaunchFailure::Panic,
        }
    }

    pub(crate) fn from_launch_failure(context: &'static str, message: impl Into<String>) -> Self {
        Self {
            context,
            message: message.into(),
            kind: LaunchFailure::ReturnedError,
        }
    }
}

impl fmt::Display for UiLaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == LaunchFailure::Panic {
            write!(f, "{} panicked: {}", self.context, self.message)
        } else {
            write!(f, "{} failed: {}", self.context, self.message)
        }
    }
}

impl std::error::Error for UiLaunchError {}

pub(crate) fn run_with_panic_guard<F>(context: &'static str, launch: F) -> Result<(), UiLaunchError>
where
    F: FnOnce() -> Result<(), UiLaunchError>,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(launch))
        .map_err(|payload| UiLaunchError::from_panic(context, payload))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_guard_returns_ok_for_successful_launch() {
        assert!(run_with_panic_guard("main window", || Ok(())).is_ok());
    }

    #[test]
    fn panic_guard_captures_string_payload_and_context() {
        let err = run_with_panic_guard("main window", || panic!("wayland init failed"))
            .expect_err("launch should fail");
        assert_eq!(err.context(), "main window");
        assert_eq!(err.panic_message(), "wayland init failed");
        assert_eq!(err.to_string(), "main window panicked: wayland init failed");
    }

    #[test]
    fn panic_guard_handles_non_string_payloads() {
        let err = run_with_panic_guard("main window", || std::panic::panic_any(42_u8))
            .expect_err("launch should fail");
        assert_eq!(err.panic_message(), "unknown panic payload");
    }

    #[test]
    fn launch_failure_error_uses_failed_display_variant() {
        let err = UiLaunchError::from_launch_failure("main window", "window closed too quickly");
        assert_eq!(
            err.to_string(),
            "main window failed: window closed too quickly"
        );
    }
}
