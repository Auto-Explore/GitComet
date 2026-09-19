use super::*;

/// A cookie on the same inotify queue as the repository's watches fences prior
/// native callbacks. It lives outside the fixture and never contributes to the
/// repository's callback counts or ignore policy. Other OS backends need their
/// own ordering guarantees before they can use this test-only synchronization.
#[cfg(all(test, target_os = "linux"))]
pub(super) struct NativeEventBarrier {
    _directory: tempfile::TempDir,
    path: PathBuf,
    sequence: AtomicU64,
    pending: std::sync::Mutex<Option<(PathBuf, mpsc::Sender<()>)>>,
}

#[cfg(all(test, target_os = "linux"))]
impl NativeEventBarrier {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = normalized(&directory.path().canonicalize().unwrap());
        Self {
            _directory: directory,
            path,
            sequence: AtomicU64::new(0),
            pending: std::sync::Mutex::new(None),
        }
    }

    pub fn wait(&self) {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let marker = self.path.join(sequence.to_string());
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            assert!(pending.is_none(), "native barrier already pending");
            *pending = Some((marker.clone(), tx));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            // A watch rebuild can cross the first write. Repeat the same
            // cookie until a live registration and the monitor acknowledge it.
            fs::write(&marker, "barrier").unwrap();
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(()) => return,
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                other => panic!("native event barrier did not finish: {other:?}"),
            }
        }
    }

    fn observe(&self, event: &notify::Event, tx: &mpsc::Sender<MonitorMsg>) -> bool {
        if event.paths.is_empty() || !event.paths.iter().all(|path| path.starts_with(&self.path)) {
            return false;
        }
        let mut pending = self.pending.lock().unwrap();
        if pending
            .as_ref()
            .is_some_and(|(marker, _)| event.paths.contains(marker))
        {
            let (_, ready) = pending.take().unwrap();
            // Wait for the real debounce deadline and any watch rebuild too.
            let _ = tx.send(MonitorMsg::Drain(ready));
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WatchMode {
    Shallow,
    Recursive,
}
pub(super) const WATCH_MODE: WatchMode = if cfg!(target_os = "linux") {
    WatchMode::Shallow
} else {
    WatchMode::Recursive
};

pub(super) struct MonitorWatcher {
    #[cfg(not(target_os = "macos"))]
    watcher: RecommendedWatcher,
    #[cfg(target_os = "macos")]
    _watcher: gitcomet_fs_watch::FsEventsWatcher,
    pub watched: FxHashSet<PathBuf>,
}

fn callback(
    repo_id: RepoId,
    tx: mpsc::Sender<MonitorMsg>,
    enabled: Arc<AtomicBool>,
    policy: PolicyCell,
    #[cfg(test)] native_events: Option<Arc<AtomicU64>>,
    #[cfg(all(test, target_os = "linux"))] native_barrier: Option<Arc<NativeEventBarrier>>,
) -> impl FnMut(notify::Result<notify::Event>) + Send + 'static {
    move |result| {
        #[cfg(all(test, target_os = "linux"))]
        if let (Some(barrier), Ok(event)) = (&native_barrier, &result)
            && barrier.observe(event, &tx)
        {
            return;
        }
        #[cfg(test)]
        if let Some(count) = &native_events {
            count.fetch_add(1, Ordering::Relaxed);
        }
        if !enabled.load(Ordering::Relaxed) {
            return;
        }
        let result = result.map(|mut event| {
            event
                .paths
                .iter_mut()
                .for_each(|path| *path = normalized(path));
            event
        });
        if let Ok(event) = &result {
            let snapshot = policy
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            if triage(&snapshot, event) == Triage::Drop {
                return;
            }
        }
        send_watcher_event_or_log(repo_id, &tx, result, &enabled);
    }
}

pub(super) fn minimal_roots(policy: &PolicySnapshot) -> Vec<PathBuf> {
    let mut candidates = vec![policy.workdir.clone()];
    candidates.extend(policy.git_roots.iter().cloned());
    candidates.sort_by_key(|path| (path.components().count(), path.clone()));
    let mut roots: Vec<PathBuf> = Vec::new();
    for path in candidates {
        if !roots.iter().any(|root| path.starts_with(root)) {
            roots.push(path);
        }
    }
    roots
}

impl MonitorWatcher {
    #[cfg(all(test, target_os = "macos"))]
    pub fn flush_pending_events(&mut self) -> bool {
        self._watcher.flush_pending_events()
    }

    pub fn new(
        repo_id: RepoId,
        tx: &mpsc::Sender<MonitorMsg>,
        enabled: &Arc<AtomicBool>,
        policy: &PolicyCell,
        boundaries: &[PathBuf],
        #[cfg(test)] native_events: Option<Arc<AtomicU64>>,
        #[cfg(all(test, target_os = "linux"))] native_barrier: Option<Arc<NativeEventBarrier>>,
    ) -> notify::Result<(Self, usize)> {
        let snapshot = policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if !snapshot.workdir.is_dir() {
            return Err(notify::Error::path_not_found().add_path(snapshot.workdir.clone()));
        }
        let handler = callback(
            repo_id,
            tx.clone(),
            enabled.clone(),
            policy.clone(),
            #[cfg(test)]
            native_events,
            #[cfg(all(test, target_os = "linux"))]
            native_barrier.clone(),
        );
        let roots = minimal_roots(&snapshot);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = boundaries;
            let watcher = RecommendedWatcher::new(
                handler,
                NotifyConfig::default()
                    .with_event_kinds(WATCHED_EVENT_KINDS)
                    .with_follow_symlinks(false),
            )?;
            let mut result = Self {
                watcher,
                watched: FxHashSet::default(),
            };
            #[cfg(all(test, target_os = "linux"))]
            if let Some(barrier) = native_barrier {
                result
                    .watcher
                    .watch(&barrier.path, RecursiveMode::NonRecursive)?;
            }
            let mut failures = 0;
            if WATCH_MODE == WatchMode::Recursive {
                for root in roots {
                    if result
                        .watcher
                        .watch(&root, RecursiveMode::Recursive)
                        .is_err()
                    {
                        failures += 1;
                    } else {
                        result.watched.insert(root);
                    }
                }
            }
            Ok((result, failures))
        }
        #[cfg(target_os = "macos")]
        {
            let streams = roots
                .iter()
                .map(|root| {
                    (
                        root.clone(),
                        plan::native_exclusions(root, &snapshot, boundaries),
                    )
                })
                .collect();
            let (watcher, failures) = gitcomet_fs_watch::FsEventsWatcher::new(streams, handler);
            Ok((
                Self {
                    _watcher: watcher,
                    watched: roots.into_iter().collect(),
                },
                failures.len(),
            ))
        }
    }

    pub fn add(&mut self, path: &Path) -> notify::Result<()> {
        #[cfg(not(target_os = "macos"))]
        if WATCH_MODE == WatchMode::Shallow && !self.watched.contains(path) {
            self.watcher.watch(path, RecursiveMode::NonRecursive)?;
            self.watched.insert(path.to_path_buf());
        }
        #[cfg(target_os = "macos")]
        let _ = path;
        Ok(())
    }

    pub fn remove_tree(&mut self, path: &Path) {
        if WATCH_MODE != WatchMode::Shallow {
            return;
        }
        self.watched.retain(|watched| {
            let removed = watched.starts_with(path);
            if removed {
                #[cfg(not(target_os = "macos"))]
                let _ = self.watcher.unwatch(watched);
            }
            !removed
        });
    }

    pub fn root_lost(&self, event: &notify::Event, policy: &PolicySnapshot) -> bool {
        matches!(
            event.kind,
            notify::EventKind::Remove(_)
                | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
        ) && event.paths.iter().any(|path| {
            if WATCH_MODE == WatchMode::Recursive {
                self.watched.contains(path)
            } else {
                path == &policy.workdir || policy.git_roots.contains(path)
            }
        })
    }
}
