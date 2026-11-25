#[cfg(target_os = "linux")]
#[path = "native/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "native/macos.rs"]
mod platform;
#[cfg(windows)]
#[path = "native/windows.rs"]
mod platform;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    #[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
    pub(in crate::crashlog) struct ProcessIdentity {
        pub(in crate::crashlog) pid: u32,
    }

    pub(in crate::crashlog) fn current() -> Option<ProcessIdentity> {
        None
    }

    pub(in crate::crashlog) fn find(_: &ProcessIdentity, _: u64) -> Option<super::NativeFault> {
        None
    }
}

pub(super) use platform::{ProcessIdentity, current, find};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NativeFault {
    pub(super) description: String,
    pub(super) timestamp_unix_ms: u64,
    pub(super) time_is_event: bool,
}
