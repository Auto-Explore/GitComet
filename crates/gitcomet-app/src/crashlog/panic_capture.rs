use super::session::{self, CrashCause};
use gitcomet_core::panic::{Containment, note_contained, payload_message};
use gitcomet_core::startup_exit::{
    PanicCaptureId, SessionToken, capture_private_panic, current_session_token,
};
use std::any::Any;
use std::backtrace::Backtrace;
use std::fmt::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};

const MAX_OBSERVATION_BYTES: usize = 4 * 1024;

pub(super) fn observe(info: &PanicHookInfo<'_>) {
    let Containment::Report { escaped } = note_contained(info) else {
        return;
    };
    let token = current_session_token();
    if capture_private_panic(token, PanicCaptureId::new_v4()) || !include_issue_details() {
        return;
    }
    if !super::SESSION_ACTIVE.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let Some(dir) = super::crash_dir() else {
        return;
    };
    let time = super::unix_time_ms();
    let mut text = format!(
        "Rust panic observed at {time} (Unix ms)\n{}\n",
        payload_message(info.payload())
    );
    if let Some(location) = info.location() {
        let _ = writeln!(text, "{}#L{}", location.file(), location.line());
    }
    if let Some(earlier) = escaped {
        let _ = writeln!(
            text,
            "Earlier panic observed on the same thread\n{}",
            earlier.message
        );
    }
    let _ = write!(text, "{}", Backtrace::force_capture());
    let _ = gitcomet_core::fs_utils::write_private_file(
        &observation_path(&dir, token),
        limited(text).as_bytes(),
    );
}

pub(crate) fn confirm(context: &'static str, payload: &(dyn Any + Send)) {
    let Some(dir) = super::crash_dir() else {
        return;
    };
    let event_time = super::unix_time_ms();
    let token = current_session_token();
    let capture_id = PanicCaptureId::new_v4();
    let private_capture = capture_private_panic(token, capture_id);
    let include_details = !private_capture && include_issue_details();
    let cause = CrashCause::RustPanic {
        capture_id: private_capture.then_some(capture_id),
        context: context.into(),
        message: include_details.then(|| limited(payload_message(payload))),
        thread: std::thread::current()
            .name()
            .filter(|_| include_details)
            .map(|name| limited(name.to_owned())),
        last_observed_panic: include_details
            .then(|| {
                session::read_limited(&observation_path(&dir, token), MAX_OBSERVATION_BYTES as u64)
                    .and_then(|bytes| String::from_utf8(bytes).ok())
            })
            .flatten(),
    };
    if let Err(error) = session::confirm_panic(&dir, cause, event_time) {
        eprintln!("Could not save a confirmed crash report: {error}");
    }
}

pub(super) fn include_issue_details() -> bool {
    !gitcomet_core::startup_exit::has_private_panic_capture()
        && gitcomet_core::identity::current()
            .links()
            .new_issue
            .is_some()
}

fn limited(mut text: String) -> String {
    text.truncate(text.floor_char_boundary(MAX_OBSERVATION_BYTES.min(text.len())));
    text
}

pub(super) fn observation_path(dir: &Path, session: SessionToken) -> PathBuf {
    dir.join(format!("last-panic-{session}.log"))
}
