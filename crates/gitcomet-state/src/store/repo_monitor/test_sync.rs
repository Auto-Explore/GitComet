//! Test-only synchronization. Delivered work, native checkpoints, and quiet
//! observation are deliberately different contracts; none flushes OS caches.
use super::*;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

pub(super) const SYNC_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const QUIET_WINDOW: Duration = Duration::from_secs(3);

pub(super) struct WaitTiming {
    phase: &'static str,
    _fixture_timer: gitcomet_core::test_support::git_fixture::FixtureTimer,
    started: Option<Instant>,
}
impl WaitTiming {
    pub fn new(phase: &'static str) -> Self {
        Self {
            phase,
            _fixture_timer: gitcomet_core::test_support::git_fixture::FixtureTimer::new(
                "watcher-wait",
                phase,
            ),
            started: std::env::var_os("GITCOMET_TEST_SYNC_TRACE").map(|_| Instant::now()),
        }
    }
}
impl Drop for WaitTiming {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            eprintln!(
                "monitor_test_sync phase={} elapsed_ms={:.3}",
                self.phase,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SyncError {
    GenerationChanged,
    Unavailable(&'static str),
    Io(String),
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DrainAck {
    pub generation: Option<u64>,
}

pub(super) struct DrainRequest {
    pub generation: Option<u64>,
    pub reply: mpsc::Sender<Result<DrainAck, SyncError>>,
}

impl DrainRequest {
    pub fn finish(self, generation: Option<u64>, native_healthy: bool) {
        let result = if self.generation.is_some() && self.generation != generation {
            Err(SyncError::GenerationChanged)
        } else if self.generation.is_some() && !native_healthy {
            Err(SyncError::Unavailable("native coverage became degraded"))
        } else {
            Ok(DrainAck { generation })
        };
        let _ = self.reply.send(result);
    }
}

pub(super) struct NativeCheckpoint {
    pub generation: u64,
    pub registrations: usize,
    pub replies: mpsc::Receiver<Result<usize, SyncError>>,
}

impl NativeCheckpoint {
    pub fn wait(self, deadline: Instant) -> Result<u64, SyncError> {
        let _timing = WaitTiming::new("native");
        assert!(self.registrations > 0, "empty native checkpoint");
        let mut pending: FxHashSet<_> = (0..self.registrations).collect();
        while !pending.is_empty() {
            match self
                .replies
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(Ok(id)) => {
                    assert!(id < self.registrations, "invalid registration {id}");
                    pending.remove(&id); // Duplicate callbacks cannot satisfy another root.
                }
                Ok(Err(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(SyncError::Stopped),
                Err(error) => panic!(
                    "native checkpoint generation={} pending={pending:?}: {error}",
                    self.generation
                ),
            }
        }
        Ok(self.generation)
    }
}

type CookieReply = (usize, mpsc::Sender<Result<usize, SyncError>>);

#[derive(Default)]
struct Cookies {
    owned: FxHashSet<PathBuf>,
    pending: FxHashMap<PathBuf, CookieReply>,
}

/// One instance per native watcher, including unsuccessful setup attempts.
/// Callback clones retain cookie tombstones until the backend really stops.
pub(super) struct NativeTestState {
    pub generation: u64,
    #[cfg(target_os = "linux")]
    _directory: tempfile::TempDir,
    #[cfg(target_os = "linux")]
    pub cookie_root: PathBuf,
    cookies: Mutex<Cookies>,
}

impl NativeTestState {
    pub fn new() -> notify::Result<Self> {
        static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
        #[cfg(target_os = "linux")]
        let directory = tempfile::tempdir()?;
        Ok(Self {
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            #[cfg(target_os = "linux")]
            cookie_root: normalized(&directory.path().canonicalize()?),
            #[cfg(target_os = "linux")]
            _directory: directory,
            cookies: Mutex::new(Cookies::default()),
        })
    }

    pub fn cookies(&self, roots: &[PathBuf]) -> Result<NativeCheckpoint, SyncError> {
        if roots.is_empty() {
            return Err(SyncError::Unavailable("no live roots"));
        }
        static NEXT_COOKIE: AtomicU64 = AtomicU64::new(1);
        let (tx, replies) = mpsc::channel();
        let mut registered = Vec::new();
        for (id, root) in roots.iter().enumerate() {
            // An 8.3-compatible name avoids short-name aliases on Windows.
            let sequence = NEXT_COOKIE.fetch_add(1, Ordering::Relaxed);
            assert!(sequence <= u32::MAX as u64, "cookie namespace exhausted");
            let path = normalized(&root.join(format!("{sequence:08x}.gct")));
            {
                let mut cookies = self.cookies.lock().unwrap();
                cookies.owned.insert(path.clone());
                cookies.pending.insert(path.clone(), (id, tx.clone()));
            }
            registered.push(path.clone());
            // Register before touching disk, but never hold our mutex across IO.
            let created = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path);
            let result = created.and_then(|file| {
                drop(file);
                fs::remove_file(&path)
            });
            if let Err(error) = result {
                let mut cookies = self.cookies.lock().unwrap();
                for path in registered {
                    cookies.pending.remove(&path);
                }
                // A collision is an error, never permission to remove someone
                // else's file. Retire this round instead of hiding the failure.
                cookies.owned.remove(&path);
                return Err(SyncError::Io(format!("cookie {}: {error}", path.display())));
            }
        }
        Ok(NativeCheckpoint {
            generation: self.generation,
            registrations: roots.len(),
            replies,
        })
    }

    /// Remove only owned cookie paths. Return acknowledgements to publish AFTER
    /// any ordinary paths in the same event have been forwarded to the monitor.
    pub fn filter_cookies(
        &self,
        result: &mut notify::Result<notify::Event>,
    ) -> (bool, Vec<CookieReply>) {
        let event = match result {
            Ok(event) if !event.need_rescan() => event,
            _ => {
                self.cancel(SyncError::GenerationChanged);
                return (false, Vec::new()); // Never hide overflow/error flags.
            }
        };
        if event.paths.is_empty() {
            return (false, Vec::new());
        }
        let mut cookies = self.cookies.lock().unwrap();
        let mut replies = Vec::new();
        let acknowledge = !cfg!(windows) || matches!(event.kind, notify::EventKind::Remove(_));
        event.paths.retain(|path| {
            let owned = cookies.owned.contains(path);
            #[cfg(target_os = "linux")]
            let owned = owned || path.starts_with(&self.cookie_root);
            if !owned {
                return true;
            }
            if acknowledge && let Some(reply) = cookies.pending.remove(path) {
                replies.push(reply);
            }
            false
        });
        (event.paths.is_empty(), replies)
    }

    pub fn cancel(&self, error: SyncError) {
        let pending = std::mem::take(&mut self.cookies.lock().unwrap().pending);
        for (_, (_, reply)) in pending {
            let _ = reply.send(Err(error.clone()));
        }
    }
}

#[derive(Clone, Debug)]
struct Observation {
    sequence: u64,
    generation: u64,
    paths: Vec<PathBuf>,
    forwarded: bool,
}

#[derive(Debug, Default)]
struct Observations {
    sequence: u64,
    last_relevant: Option<Instant>,
    recent: VecDeque<Observation>,
}

#[derive(Debug, Default)]
pub(super) struct NativeObservations {
    state: Mutex<Observations>,
    changed: Condvar,
}

impl NativeObservations {
    pub fn record(&self, generation: u64, paths: Vec<PathBuf>, forwarded: bool) {
        let mut state = self.state.lock().unwrap();
        state.sequence += 1;
        if forwarded {
            state.last_relevant = Some(Instant::now());
        }
        let sequence = state.sequence;
        state.recent.push_back(Observation {
            sequence,
            generation,
            paths,
            forwarded,
        });
        if state.recent.len() > 128 {
            state.recent.pop_front();
        }
        self.changed.notify_all();
    }

    pub fn sequence(&self) -> u64 {
        self.state.lock().unwrap().sequence
    }

    pub fn last_relevant(&self) -> Option<Instant> {
        self.state.lock().unwrap().last_relevant
    }

    #[cfg(windows)]
    pub fn has_path_since(&self, after: u64, path: &Path) -> bool {
        self.state.lock().unwrap().recent.iter().any(|event| {
            event.sequence > after && event.forwarded && event.paths.iter().any(|p| p == path)
        })
    }

    /// Only for operation-unique paths: a sequence alone cannot distinguish a
    /// late notification for an earlier write to the same path.
    pub fn wait_for_path(&self, after: u64, path: &Path, deadline: Instant) -> u64 {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(event) = state.recent.iter().find(|event| {
                event.sequence > after && event.forwarded && event.paths.iter().any(|p| p == path)
            }) {
                return event.generation;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "no native change for {}: {state:?}",
                path.display()
            );
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sync_coverage_loss_invalidates_a_delivered_native_fence() {
        let (reply, rx) = mpsc::channel();
        DrainRequest {
            generation: Some(1),
            reply,
        }
        .finish(Some(1), false);
        assert!(matches!(
            rx.try_recv().unwrap(),
            Err(SyncError::Unavailable(_))
        ));
        let (reply, rx) = mpsc::channel();
        DrainRequest {
            generation: None,
            reply,
        }
        .finish(Some(1), false);
        assert_eq!(
            rx.try_recv().unwrap(),
            Ok(DrainAck {
                generation: Some(1)
            })
        );
    }

    #[test]
    fn native_sync_duplicate_acknowledgements_do_not_cover_missing_roots() {
        let (tx, replies) = mpsc::channel();
        tx.send(Ok(0)).unwrap();
        tx.send(Ok(0)).unwrap();
        tx.send(Err(SyncError::GenerationChanged)).unwrap();
        let checkpoint = NativeCheckpoint {
            generation: 7,
            registrations: 2,
            replies,
        };
        assert_eq!(
            checkpoint.wait(Instant::now() + SYNC_TIMEOUT),
            Err(SyncError::GenerationChanged)
        );
    }

    #[test]
    fn native_sync_cookies_are_owned_exactly_and_errors_invalidate_rounds() {
        let state = NativeTestState::new().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = normalized(&temp.path().canonicalize().unwrap());
        let checkpoint = state.cookies(std::slice::from_ref(&root)).unwrap();
        let cookie = state
            .cookies
            .lock()
            .unwrap()
            .pending
            .keys()
            .next()
            .unwrap()
            .clone();
        assert!(
            !cookie.exists(),
            "cookie cleanup must finish before acknowledgement"
        );
        let real = root.join("user.gct");
        let mut mixed = Ok(notify::Event::new(notify::EventKind::Remove(
            notify::event::RemoveKind::File,
        ))
        .add_path(cookie.clone())
        .add_path(real.clone()));
        let (only, replies) = state.filter_cookies(&mut mixed);
        assert!(!only);
        assert_eq!(mixed.unwrap().paths, vec![real]);
        for (id, reply) in replies {
            reply.send(Ok(id)).unwrap();
        }
        assert_eq!(
            checkpoint.wait(Instant::now() + SYNC_TIMEOUT),
            Ok(state.generation)
        );
        let mut late = Ok(notify::Event::new(notify::EventKind::Any).add_path(cookie));
        let (only, replies) = state.filter_cookies(&mut late);
        assert!(only && replies.is_empty(), "late cookie was not suppressed");
        let checkpoint = state.cookies(&[root]).unwrap();
        let cookie = state
            .cookies
            .lock()
            .unwrap()
            .pending
            .keys()
            .next()
            .unwrap()
            .clone();
        let mut overflow = Ok(notify::Event::new(notify::EventKind::Any)
            .add_path(cookie)
            .set_flag(notify::event::Flag::Rescan));
        assert!(!state.filter_cookies(&mut overflow).0);
        assert!(overflow.unwrap().need_rescan());
        assert_eq!(
            checkpoint.wait(Instant::now() + SYNC_TIMEOUT),
            Err(SyncError::GenerationChanged)
        );
    }

    #[test]
    fn native_sync_retirement_cancels_all_pending_rounds() {
        let state = NativeTestState::new().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = normalized(&temp.path().canonicalize().unwrap());
        let first = state.cookies(std::slice::from_ref(&root)).unwrap();
        let second = state.cookies(&[root]).unwrap();
        state.cancel(SyncError::Stopped);
        for checkpoint in [first, second] {
            assert_eq!(
                checkpoint.wait(Instant::now() + SYNC_TIMEOUT),
                Err(SyncError::Stopped)
            );
        }
        assert!(matches!(state.cookies(&[]), Err(SyncError::Unavailable(_))));
    }
}
