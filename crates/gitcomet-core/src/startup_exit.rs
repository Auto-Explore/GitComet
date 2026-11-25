//! A recoverable session report can be acknowledged after a native support receipt.
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
pub use uuid::Uuid as SessionToken;

/// Local correlation between the process marker and the private panic collector.
pub fn current_session_token() -> SessionToken {
    static TOKEN: OnceLock<SessionToken> = OnceLock::new();
    *TOKEN.get_or_init(SessionToken::new_v4)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupExitReason {
    UnknownExit,
    X11EventLoopError,
    WaylandEventLoopError,
}

impl StartupExitReason {
    pub fn description(self) -> &'static str {
        match self {
            Self::UnknownExit => "The previous session ended unexpectedly. The cause is unknown.",
            Self::X11EventLoopError => "The X11 event loop returned an error.",
            Self::WaylandEventLoopError => "The Wayland event loop returned an error.",
        }
    }
}

/// An opaque acknowledgement for the exact recovery snapshot shown in a window.
#[derive(Clone)]
pub struct StartupExitReport(Arc<Snapshot>);

struct Snapshot {
    path: PathBuf,
    digest: [u8; 32],
    reason: StartupExitReason,
    session_token: Option<SessionToken>,
    acknowledged: AtomicBool,
}

impl std::fmt::Debug for StartupExitReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartupExitReport")
            .field("reason", &self.reason())
            .finish_non_exhaustive()
    }
}

impl PartialEq for StartupExitReport {
    fn eq(&self, other: &Self) -> bool {
        self.0.path == other.0.path
            && self.0.digest == other.0.digest
            && self.reason() == other.reason()
            && self.session_token() == other.session_token()
    }
}

impl Eq for StartupExitReport {}

/// Coordinates recovery, dismissal and submission across running processes.
pub fn lock_snapshot(path: &Path) -> io::Result<File> {
    crate::fs_utils::lock_private_file(&path.with_extension("lock"))
}

impl StartupExitReport {
    #[doc(hidden)]
    pub fn new(path: PathBuf, contents: &[u8], reason: StartupExitReason) -> Self {
        Self::new_with_session(path, contents, reason, None)
    }

    #[doc(hidden)]
    pub fn new_with_session(
        path: PathBuf,
        contents: &[u8],
        reason: StartupExitReason,
        session_token: Option<SessionToken>,
    ) -> Self {
        Self(Arc::new(Snapshot {
            path,
            digest: Sha256::digest(contents).into(),
            reason,
            session_token,
            acknowledged: AtomicBool::new(false),
        }))
    }

    pub fn reason(&self) -> StartupExitReason {
        self.0.reason
    }

    /// Local diagnostic correlation. Exclude this value from submissions and exports.
    pub fn session_token(&self) -> Option<SessionToken> {
        self.0.session_token
    }

    pub fn acknowledged(&self) -> bool {
        self.0.acknowledged.load(Ordering::Acquire)
    }

    /// Performs blocking disk I/O. A newer incident remains available for review.
    pub fn acknowledge(&self) -> io::Result<()> {
        let snapshot = &self.0;
        let _lock = lock_snapshot(&snapshot.path)?;
        if self.acknowledged() {
            return Ok(());
        }
        match std::fs::read(&snapshot.path) {
            Ok(bytes) => {
                if <[u8; 32]>::from(Sha256::digest(&bytes)) == snapshot.digest {
                    std::fs::remove_file(&snapshot.path)?;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        snapshot.acknowledged.store(true, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_clears_only_the_reviewed_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.log");
        let reviewed =
            StartupExitReport::new(path.clone(), b"first\n", StartupExitReason::UnknownExit);
        std::fs::write(&path, b"first\n\nsecond\n").unwrap();
        reviewed.acknowledge().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first\n\nsecond\n");
        reviewed.acknowledge().unwrap();
        let second = StartupExitReport::new(
            path.clone(),
            b"first\n\nsecond\n",
            StartupExitReason::UnknownExit,
        );
        second.acknowledge().unwrap();
        assert!(!path.exists());
        assert!(second.acknowledged());
    }
}
