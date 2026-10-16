//! Containing a panic in code GitComet does not own, whose panic the caller
//! maps to a fallback, so the crash hook does not report a crash that never
//! happened. A panic in GitComet's own code stays a crash report.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, PanicHookInfo};

const UNKNOWN_PAYLOAD: &str = "unknown panic payload";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainedPanic {
    pub message: String,
    pub location: Option<String>,
}

// A panic the hook let through to `contain`, until `contain` takes it.
struct Noted {
    location: Option<String>,
}

thread_local! {
    static CONTAINING: Cell<usize> = const { Cell::new(0) };
    static NOTED: RefCell<Option<Noted>> = const { RefCell::new(None) };
}

/// Runs `f`, returning its panic as `ContainedPanic`. A panic hook that calls
/// [`note_contained`] supplies the location; without one it is None.
pub fn contain<R>(f: impl FnOnce() -> R) -> Result<R, ContainedPanic> {
    CONTAINING.with(|depth| depth.set(depth.get() + 1));
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    CONTAINING.with(|depth| depth.set(depth.get() - 1));
    let noted = NOTED.with(|noted| noted.borrow_mut().take());
    result.map_err(|payload| ContainedPanic {
        message: payload_message(payload.as_ref()),
        location: noted.and_then(|noted| noted.location),
    })
}

/// For the process panic hook: true when this thread is inside [`contain`],
/// after recording where it panicked. A second panic before `contain` took the
/// first means the first never reached `contain` (it cannot unwind, or the
/// contained code caught it), so the hook reports it and no location is kept.
pub fn note_contained(info: &PanicHookInfo<'_>) -> bool {
    if CONTAINING.with(Cell::get) == 0 {
        return false;
    }
    NOTED.with(|noted| {
        let mut noted = noted.borrow_mut();
        if let Some(pending) = noted.as_mut() {
            pending.location = None;
            return false;
        }
        *noted = Some(Noted {
            location: info
                .location()
                .map(|location| format!("{}:{}", location.file(), location.line())),
        });
        true
    })
}

/// Renders a panic payload (from a hook, `catch_unwind` or
/// `JoinHandle::join`) for diagnostics.
pub fn payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        UNKNOWN_PAYLOAD.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_message_renders_str_string_and_other_payloads() {
        assert_eq!(payload_message(&"static"), "static");
        assert_eq!(payload_message(&"owned".to_string()), "owned");
        assert_eq!(payload_message(&123usize), "unknown panic payload");
    }

    #[test]
    fn contain_passes_the_value_through() {
        assert_eq!(contain(|| 7), Ok(7));
    }

    #[test]
    fn without_a_noting_hook_only_the_message_is_known() {
        assert_eq!(
            contain(|| panic!("failed {}", 3)),
            Err(ContainedPanic {
                message: "failed 3".to_owned(),
                location: None,
            })
        );
    }
}
