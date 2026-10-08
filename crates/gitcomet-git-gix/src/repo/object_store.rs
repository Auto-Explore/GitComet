//! Reporting and recovery for object-store failures gix does not recover from.
//!
//! gix rescans the pack directory only when a pack vanished (`NotFound`). Any
//! other failure to open or map one — a pack another process holds, Windows
//! running out of commit for the copy-on-write views — stays a hard error for
//! as long as the store lives, so the history readers reopen it and retry.
use gitcomet_core::error::{Error, ErrorKind};
use std::sync::atomic::{AtomicU64, Ordering};

/// Object-store I/O failures seen so far. Process-wide because the readers
/// are free functions and parallel decode threads; a concurrent failure in
/// another repository only costs one spurious reopen.
static OBJECT_STORE_IO_FAILURES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn io_failures() -> u64 {
    OBJECT_STORE_IO_FAILURES.load(Ordering::Acquire)
}

/// A backend error for a failed gix call, with the I/O causes gix keeps out of
/// its top-level message appended (the pack path and the OS error code).
pub(crate) fn gix_error(context: &str, error: &(dyn std::error::Error + 'static)) -> Error {
    let (message, store_failure) = describe(context, error);
    if store_failure {
        OBJECT_STORE_IO_FAILURES.fetch_add(1, Ordering::AcqRel);
    }
    Error::new(ErrorKind::Backend(message))
}

/// The message, and whether an I/O cause is one gix will not recover from.
fn describe(context: &str, error: &(dyn std::error::Error + 'static)) -> (String, bool) {
    let mut message = format!("{context}: {error}");
    let mut store_failure = false;
    let mut inspect = |cause: &(dyn std::error::Error + 'static)| {
        let Some(io) = cause.downcast_ref::<std::io::Error>() else {
            return;
        };
        let text = io.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        // NotFound is what gix recovers from itself; Interrupted is our own
        // walk cancellation.
        store_failure |= !matches!(
            io.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::Interrupted
        );
    };
    // gix 0.89's semantic classifications omit many I/O kinds, including the
    // pack-mapping failures that require reopening the store. Inspect every
    // concrete cause, including payloads std::io::Error::source() skips.
    let mut cause = Some(error);
    while let Some(error) = cause {
        if let Some(error) = error.downcast_ref::<gix::Error>() {
            error.iter_errors().for_each(&mut inspect);
            break;
        }
        inspect(error);
        cause = match error.downcast_ref::<std::io::Error>() {
            Some(io) => io.get_ref().map(|source| source as _),
            None => error.source(),
        };
    }
    (message, store_failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_detail_includes_wrapped_os_error_once() {
        // The shape gix-odb produces: an outer io error that keeps the kind
        // but not the OS code, wrapping the real one.
        let inner = std::io::Error::from_raw_os_error(13);
        let outer = std::io::Error::new(inner.kind(), inner);

        let (message, store_failure) = describe("gix peel", &outer);

        assert!(message.starts_with("gix peel: "), "{message}");
        assert_eq!(message.matches("os error 13").count(), 1, "{message}");
        assert!(store_failure);
    }

    #[test]
    fn missing_or_cancelled_is_not_a_store_failure() {
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::Interrupted,
        ] {
            let (_, store_failure) = describe("gix find", &std::io::Error::from(kind));
            assert!(!store_failure, "{kind:?}");
        }
    }

    #[test]
    fn unclassified_io_causes_still_trigger_store_recovery() {
        for kind in [std::io::ErrorKind::Other, std::io::ErrorKind::InvalidData] {
            let inner = gix::Error::from_error(std::io::Error::new(kind, "pack mapping failed"));
            let outer = gix::Error::from_error(std::io::Error::new(kind, inner));

            let (message, store_failure) = describe("gix peel", &outer);

            assert!(store_failure, "{kind:?}: {message}");
            assert_eq!(
                message.matches("pack mapping failed").count(),
                1,
                "{message}"
            );
        }
    }
}
