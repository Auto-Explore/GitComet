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
    pub location: Option<PanicLocation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PanicLocation {
    pub file: String,
    pub line: u32,
}

impl std::fmt::Display for PanicLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

// A panic the hook let through to `contain`, until `contain` takes it.
enum Noted {
    Pending(ContainedPanic),
    // A later panic arrived first, and the hook reported this one.
    Reported,
}

/// What the crash hook does with a panic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Containment {
    /// [`contain`] will return it; no crash report.
    Contained,
    /// Report it. `escaped` is this thread's earlier panic that never reached
    /// `contain` (it could not unwind, or the contained code caught it): the
    /// cause of this one when it could not unwind.
    Report { escaped: Option<ContainedPanic> },
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
    result.map_err(|payload| match noted {
        Some(Noted::Pending(panic)) => panic,
        Some(Noted::Reported) | None => ContainedPanic {
            message: payload_message(payload.as_ref()),
            location: None,
        },
    })
}

/// For the process panic hook: `Contained` when this thread is inside
/// [`contain`], after recording the panic. A second panic before `contain`
/// took the first means the first never reached it, so the hook reports the
/// second with the first as `escaped`, and `contain` keeps no location.
pub fn note_contained(info: &PanicHookInfo<'_>) -> Containment {
    if CONTAINING.with(Cell::get) == 0 {
        return Containment::Report { escaped: None };
    }
    NOTED.with(|noted| {
        let mut noted = noted.borrow_mut();
        match noted.replace(Noted::Reported) {
            Some(Noted::Pending(escaped)) => Containment::Report {
                escaped: Some(escaped),
            },
            Some(Noted::Reported) => Containment::Report { escaped: None },
            None => {
                *noted = Some(Noted::Pending(ContainedPanic {
                    message: payload_message(info.payload()),
                    location: info.location().map(|location| PanicLocation {
                        file: location.file().to_owned(),
                        line: location.line(),
                    }),
                }));
                Containment::Contained
            }
        }
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

#[cfg(feature = "test-support")]
pub mod test_support {
    use std::cell::Cell;
    use std::sync::Once;

    thread_local! {
        static UNCONTAINED: Cell<usize> = const { Cell::new(0) };
    }

    /// Runs `f` under a hook that answers [`super::note_contained`] like the
    /// crash hook, returning how many of this thread's panics it would report.
    pub fn uncontained_panics<R>(f: impl FnOnce() -> R) -> (R, usize) {
        static HOOK: Once = Once::new();
        HOOK.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if super::note_contained(info) == super::Containment::Contained {
                    return;
                }
                UNCONTAINED.with(|count| count.set(count.get() + 1));
                previous(info);
            }));
        });
        let before = UNCONTAINED.with(Cell::get);
        let result = f();
        (result, UNCONTAINED.with(Cell::get) - before)
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
