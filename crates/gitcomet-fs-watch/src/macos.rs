use dispatch2::{DispatchQueue, DispatchRetained};
use notify::{Event, EventHandler, EventKind, event::*};
use objc2_core_foundation as cf;
use objc2_core_services as fs;
use std::ffi::{CStr, OsStr, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

const MAX_REGISTRATIONS: usize = 4096;
static ACTIVE_STREAMS: AtomicUsize = AtomicUsize::new(0);
type Handler = Arc<Mutex<Box<dyn EventHandler>>>;
struct Reservation;
impl Reservation {
    fn acquire() -> notify::Result<Self> {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `limit` is a valid writable rlimit, and RLIMIT_NOFILE is supported on macOS.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // FSEvents can corrupt fd 0 above approximately RLIMIT_NOFILE / 10
        // watched roots. Keep the same process-wide /12 margin as notify.
        let budget = (limit.rlim_cur as usize / 12).min(MAX_REGISTRATIONS);
        ACTIVE_STREAMS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                count.checked_add(1).filter(|next| *next <= budget)
            })
            .map_err(|_| notify::Error::generic("FSEvents process path budget exhausted"))?;
        Ok(Self)
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        ACTIVE_STREAMS.fetch_sub(1, Ordering::Relaxed);
    }
}

struct Context {
    handler: Handler,
}

unsafe extern "C-unwind" fn callback(
    _stream: fs::ConstFSEventStreamRef,
    context: *mut c_void,
    count: usize,
    paths: NonNull<c_void>,
    flags: NonNull<fs::FSEventStreamEventFlags>,
    _ids: NonNull<fs::FSEventStreamEventId>,
) {
    // No Rust panic may unwind through CoreServices. All pointers are supplied
    // by FSEvents and valid for this invocation; no borrowed data is retained.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: the stream owns Context until callbacks have stopped and it is released.
        let context = unsafe { &*context.cast::<Context>() };
        let mut handler = context
            .handler
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for index in 0..count {
            // SAFETY: FileEvents without UseCFTypes supplies count NUL-terminated paths and flags.
            let path = unsafe {
                CStr::from_ptr(*paths.as_ptr().cast::<*const std::ffi::c_char>().add(index))
            };
            let path = PathBuf::from(OsStr::from_bytes(path.to_bytes()));
            let flags = unsafe { *flags.as_ptr().add(index) };
            if flags & fs::kFSEventStreamEventFlagHistoryDone != 0 {
                continue;
            }
            let mut event = Event::new(event_kind(flags)).add_path(path);
            if flags
                & (fs::kFSEventStreamEventFlagMustScanSubDirs
                    | fs::kFSEventStreamEventFlagUserDropped
                    | fs::kFSEventStreamEventFlagKernelDropped
                    | fs::kFSEventStreamEventFlagEventIdsWrapped
                    | fs::kFSEventStreamEventFlagRootChanged
                    | fs::kFSEventStreamEventFlagMount
                    | fs::kFSEventStreamEventFlagUnmount)
                != 0
            {
                event = event.set_flag(Flag::Rescan);
            }
            handler.handle_event(Ok(event));
        }
    }));
}

fn event_kind(flags: u32) -> EventKind {
    if flags & fs::kFSEventStreamEventFlagItemRenamed != 0 {
        EventKind::Modify(ModifyKind::Name(RenameMode::Any))
    } else if flags & fs::kFSEventStreamEventFlagItemRemoved != 0 {
        EventKind::Remove(if flags & fs::kFSEventStreamEventFlagItemIsDir != 0 {
            RemoveKind::Folder
        } else {
            RemoveKind::File
        })
    } else if flags & fs::kFSEventStreamEventFlagItemCreated != 0 {
        EventKind::Create(if flags & fs::kFSEventStreamEventFlagItemIsDir != 0 {
            CreateKind::Folder
        } else {
            CreateKind::File
        })
    } else if flags & fs::kFSEventStreamEventFlagItemModified != 0 {
        EventKind::Modify(ModifyKind::Data(DataChange::Content))
    } else {
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any))
    }
}

fn path_array(
    paths: &[PathBuf],
) -> notify::Result<cf::CFRetained<cf::CFMutableArray<cf::CFString>>> {
    let array: cf::CFRetained<cf::CFMutableArray<cf::CFString>> = cf::CFMutableArray::empty();
    for path in paths {
        let string = cf::CFURL::from_file_path(path)
            .and_then(|url| url.file_system_path(cf::CFURLPathStyle::CFURLPOSIXPathStyle))
            .ok_or_else(|| {
                notify::Error::generic("Cannot encode FSEvents path").add_path(path.clone())
            })?;
        array.append(&string);
    }
    Ok(array)
}

struct Stream {
    stream: fs::FSEventStreamRef,
    queue: DispatchRetained<DispatchQueue>,
    _context: Box<Context>,
    _reservation: Reservation,
}
impl Stream {
    fn new(root: PathBuf, exclusions: Vec<PathBuf>, handler: Handler) -> notify::Result<Self> {
        if exclusions.len() > 8 {
            return Err(notify::Error::generic(
                "FSEvents supports at most eight exclusions",
            ));
        }
        let reservation = Reservation::acquire()?;
        let paths = path_array(&[root])?;
        // FSEvents rejects an empty exclusion array instead of excluding nothing.
        let exclusions = (!exclusions.is_empty())
            .then(|| path_array(&exclusions))
            .transpose()?;
        let queue = DispatchQueue::new("gitcomet.fsevents", None);
        let mut owned_context = Box::new(Context { handler });
        let mut context = fs::FSEventStreamContext {
            version: 0,
            info: (&mut *owned_context as *mut Context).cast(),
            retain: None,
            release: None,
            copyDescription: None,
        };
        // SAFETY: arrays contain CFStrings; the owned Box keeps Context at a
        // stable address until the stream has stopped and callbacks have drained.
        let stream = unsafe {
            fs::FSEventStreamCreate(
                None,
                Some(callback),
                &mut context,
                paths.as_opaque(),
                fs::kFSEventStreamEventIdSinceNow,
                0.05,
                fs::kFSEventStreamCreateFlagFileEvents
                    | fs::kFSEventStreamCreateFlagNoDefer
                    | fs::kFSEventStreamCreateFlagWatchRoot,
            )
        };
        if stream.is_null() {
            return Err(notify::Error::generic("Cannot create FSEvents stream"));
        }
        let owned = Self {
            stream,
            queue,
            _context: owned_context,
            _reservation: reservation,
        };
        // SAFETY: stream and exclusion array are live, and setup precedes Start.
        unsafe {
            if let Some(exclusions) = &exclusions
                && !fs::FSEventStreamSetExclusionPaths(stream, exclusions.as_opaque())
            {
                return Err(notify::Error::generic(
                    "Cannot apply native FSEvents exclusions",
                ));
            }
            fs::FSEventStreamSetDispatchQueue(stream, Some(&owned.queue));
            if !fs::FSEventStreamStart(stream) {
                return Err(notify::Error::generic("Cannot start FSEvents stream"));
            }
        }
        Ok(owned)
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: only this owner manipulates the live stream. Stop/invalidate
        // prevents new callbacks; draining its serial queue finishes existing
        // callbacks before our Context field is dropped. Drop never runs on this queue.
        unsafe {
            fs::FSEventStreamStop(self.stream);
            fs::FSEventStreamInvalidate(self.stream);
        }
        self.queue.exec_sync(|| {});
        unsafe {
            fs::FSEventStreamRelease(self.stream);
        }
    }
}

/// One FSEvents stream per repository root; exclusions never create partitions.
pub struct FsEventsWatcher {
    _streams: Vec<Stream>,
}
impl FsEventsWatcher {
    /// Checkpoint callbacks already queued on each live stream. This does NOT
    /// flush the kernel or fseventsd, nor order future callbacks across streams.
    /// The caller must not wait on a stream's callback queue.
    #[cfg(any(test, feature = "test-support"))]
    pub fn checkpoint_callbacks(&self, callback: impl Fn(usize) + Send + Sync + 'static) -> usize {
        let callback = Arc::new(callback);
        for (id, stream) in self._streams.iter().enumerate() {
            let callback = callback.clone();
            stream.queue.exec_async(move || callback(id));
        }
        self._streams.len()
    }

    pub fn new(
        streams: Vec<(PathBuf, Vec<PathBuf>)>,
        handler: impl EventHandler,
    ) -> (Self, Vec<notify::Error>) {
        let handler: Handler = Arc::new(Mutex::new(Box::new(handler)));
        let mut live = Vec::new();
        let mut failures = Vec::new();
        for (root, exclusions) in streams {
            match Stream::new(root, exclusions, handler.clone()) {
                Ok(stream) => live.push(stream),
                Err(error) => failures.push(error),
            }
        }
        (Self { _streams: live }, failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sync_checkpoints_wait_for_each_live_callback_queue() {
        use std::sync::mpsc;
        use std::time::Duration;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (watcher, errors) = FsEventsWatcher::new(
            vec![(root.clone(), Vec::new()), (root, Vec::new())],
            |_event: notify::Result<Event>| {},
        );
        assert!(errors.is_empty());
        let mut releases = Vec::new();
        let (started_tx, started_rx) = mpsc::channel();
        for (id, stream) in watcher._streams.iter().enumerate() {
            let (release, wait) = mpsc::channel();
            releases.push(release);
            let started = started_tx.clone();
            stream.queue.exec_async(move || {
                started.send(id).unwrap();
                wait.recv_timeout(Duration::from_secs(10)).unwrap();
            });
        }
        for _ in 0..2 {
            started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        let (tx, rx) = mpsc::channel();
        assert_eq!(
            watcher.checkpoint_callbacks(move |id| {
                tx.send(id).unwrap();
            }),
            2
        );
        assert!(rx.try_recv().is_err());
        releases[0].send(()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), 0);
        assert!(
            rx.try_recv().is_err(),
            "one stream completed the other stream's checkpoint"
        );
        releases[1].send(()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), 1);
    }

    #[test]
    fn native_sync_checkpoints_exclude_failed_streams_and_survive_drop() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (watcher, errors) = FsEventsWatcher::new(
            vec![
                (
                    root.clone(),
                    (0..9).map(|id| root.join(id.to_string())).collect(),
                ),
                (root, Vec::new()),
            ],
            |_event: notify::Result<Event>| {},
        );
        assert_eq!(errors.len(), 1);
        let (tx, rx) = std::sync::mpsc::channel();
        assert_eq!(
            watcher.checkpoint_callbacks(move |id| {
                tx.send(id).unwrap();
            }),
            1
        );
        drop(watcher); // Must finish queued callbacks without holding their locks.
        assert_eq!(rx.try_recv().unwrap(), 0);
        assert!(rx.try_recv().is_err());
        let (watcher, errors) =
            FsEventsWatcher::new(Vec::new(), |_event: notify::Result<Event>| {});
        assert!(errors.is_empty());
        assert_eq!(
            watcher.checkpoint_callbacks(|_| panic!("no live streams")),
            0
        );
    }

    #[test]
    fn native_flags_preserve_structural_and_content_changes() {
        assert_eq!(
            event_kind(
                fs::kFSEventStreamEventFlagItemCreated | fs::kFSEventStreamEventFlagItemIsDir
            ),
            EventKind::Create(CreateKind::Folder)
        );
        assert_eq!(
            event_kind(
                fs::kFSEventStreamEventFlagItemRenamed | fs::kFSEventStreamEventFlagItemModified
            ),
            EventKind::Modify(ModifyKind::Name(RenameMode::Any))
        );
        assert_eq!(
            event_kind(fs::kFSEventStreamEventFlagItemModified),
            EventKind::Modify(ModifyKind::Data(DataChange::Content))
        );
    }

    #[test]
    fn native_exclusions_suppress_cache_but_keep_source_events() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let cache = root.join("cache");
        std::fs::create_dir(&cache).unwrap();
        let source = root.join("source.txt");
        let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
        let (_watcher, errors) = FsEventsWatcher::new(vec![(root, vec![cache.clone()])], tx);
        assert!(errors.is_empty());
        // A new stream can still report fixture creation. Establish a positive
        // readiness observation; it is not a fence for delayed native events.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            assert!(std::time::Instant::now() < deadline, "no readiness event");
            std::fs::write(&source, "readiness edit").unwrap();
            if let Ok(Ok(event)) = rx.recv_timeout(std::time::Duration::from_millis(100))
                && event.paths.contains(&source)
            {
                break;
            }
        }
        std::fs::write(cache.join("temporary"), "cache traffic").unwrap();
        std::fs::write(&source, "source edit").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut saw_source = false;
        while std::time::Instant::now() < deadline {
            if let Ok(Ok(event)) = rx.recv_timeout(std::time::Duration::from_millis(100)) {
                assert!(
                    !event.paths.iter().any(|path| path.starts_with(&cache)),
                    "{event:?}"
                );
                saw_source |= event.paths.contains(&source);
            }
        }
        assert!(saw_source);
    }

    #[test]
    fn stream_without_exclusions_delivers_events() {
        // A linked worktree or separate Git directory can leave its checkout
        // root with no cache roots and no ignored boundaries to exclude.
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("source.txt");
        let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
        let (_watcher, errors) = FsEventsWatcher::new(vec![(root, Vec::new())], tx);
        assert!(errors.is_empty(), "{errors:?}");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            assert!(std::time::Instant::now() < deadline, "no source event");
            std::fs::write(&source, "source edit").unwrap();
            if let Ok(Ok(event)) = rx.recv_timeout(std::time::Duration::from_millis(100))
                && event.paths.contains(&source)
            {
                break;
            }
        }
    }

    #[test]
    fn registration_failures_do_not_emit_events_that_retry_themselves() {
        let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
        let (_watcher, errors) = FsEventsWatcher::new(
            vec![(
                PathBuf::from("/tmp"),
                (0..9)
                    .map(|index| PathBuf::from(format!("/tmp/ignored-{index}")))
                    .collect(),
            )],
            tx,
        );
        assert_eq!(errors.len(), 1);
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(100))
                .is_err()
        );
    }
}
