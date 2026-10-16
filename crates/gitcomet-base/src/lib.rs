//! Product identity, per-user directories, platform detection, file utilities,
//! the background loading state and instrumentation shared by every GitComet
//! layer. It holds no Git domain types, so the UI kit builds on it rather than
//! on `gitcomet-core`.

pub mod environment;
pub mod fs_utils;
pub mod history_perf;
pub mod identity;
mod loadable;
pub mod op_trace;
pub mod platform;

pub use loadable::Loadable;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes one line to stderr, ignoring a closed stream.
pub fn write_stderr_line(args: std::fmt::Arguments<'_>) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{args}");
}

/// A process-unique, nonzero operation id. Filesystem operations and the file
/// clipboard's ownership tokens share this sequence, so they never collide.
pub fn next_operation_id() -> u64 {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let next = NEXT.get_or_init(|| {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        AtomicU64::new((time ^ u64::from(std::process::id()).rotate_left(32)).max(1))
    });
    next.fetch_add(1, Ordering::Relaxed)
}
