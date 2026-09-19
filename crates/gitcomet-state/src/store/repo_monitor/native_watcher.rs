use super::*;

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
    #[cfg(test)]
    pub test_state: Arc<NativeTestState>,
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
    #[cfg(test)] observations: Option<Arc<NativeObservations>>,
    #[cfg(test)] test_state: Arc<NativeTestState>,
) -> impl FnMut(notify::Result<notify::Event>) + Send + 'static {
    move |result| {
        #[cfg(not(test))]
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
        #[cfg(test)]
        let mut result = result;
        #[cfg(test)]
        let (cookie_only, cookie_replies) = test_state.filter_cookies(&mut result);
        #[cfg(not(test))]
        let cookie_only = false;
        let mut forwarded = false;
        if !cookie_only {
            #[cfg(test)]
            if let Some(count) = &native_events {
                count.fetch_add(1, Ordering::Relaxed);
            }
            let dropped = if let Ok(event) = &result {
                let snapshot = policy
                    .read()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                triage(&snapshot, event) == Triage::Drop
            } else {
                false
            };
            #[cfg(test)]
            let paths = result
                .as_ref()
                .map(|event| event.paths.clone())
                .unwrap_or_default();
            if !dropped && enabled.load(Ordering::Relaxed) {
                forwarded = send_watcher_event_or_log(repo_id, &tx, result, &enabled);
            }
            #[cfg(test)]
            if let Some(observations) = &observations {
                observations.record(test_state.generation, paths, forwarded);
            }
        }
        let _ = forwarded;
        #[cfg(test)]
        for (id, reply) in cookie_replies {
            let _ = reply.send(Ok(id));
        }
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
    pub fn new(
        repo_id: RepoId,
        tx: &mpsc::Sender<MonitorMsg>,
        enabled: &Arc<AtomicBool>,
        policy: &PolicyCell,
        boundaries: &[PathBuf],
        #[cfg(test)] native_events: Option<Arc<AtomicU64>>,
        #[cfg(test)] observations: Option<Arc<NativeObservations>>,
    ) -> notify::Result<(Self, usize)> {
        let snapshot = policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if !snapshot.workdir.is_dir() {
            return Err(notify::Error::path_not_found().add_path(snapshot.workdir.clone()));
        }
        #[cfg(test)]
        let test_state = Arc::new(NativeTestState::new()?);
        let handler = callback(
            repo_id,
            tx.clone(),
            enabled.clone(),
            policy.clone(),
            #[cfg(test)]
            native_events,
            #[cfg(test)]
            observations,
            #[cfg(test)]
            test_state.clone(),
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
                #[cfg(test)]
                test_state,
                watcher,
                watched: FxHashSet::default(),
            };
            #[cfg(all(test, target_os = "linux"))]
            {
                result
                    .watcher
                    .watch(&result.test_state.cookie_root, RecursiveMode::NonRecursive)?;
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
                    #[cfg(test)]
                    test_state,
                    _watcher: watcher,
                    watched: roots.into_iter().collect(),
                },
                failures.len(),
            ))
        }
    }

    #[cfg(test)]
    pub fn checkpoint(&self) -> Result<NativeCheckpoint, SyncError> {
        #[cfg(target_os = "linux")]
        {
            // This auxiliary watch shares the repository's single inotify queue.
            self.test_state
                .cookies(std::slice::from_ref(&self.test_state.cookie_root))
        }
        #[cfg(windows)]
        {
            let mut roots: Vec<_> = self.watched.iter().cloned().collect();
            roots.sort();
            if roots
                .iter()
                .any(|root| !gitcomet_fs_watch::is_local_ntfs(root).unwrap_or(false))
            {
                return Err(SyncError::Unavailable(
                    "requires local NTFS without reparse points",
                ));
            }
            self.test_state.cookies(&roots)
        }
        #[cfg(target_os = "macos")]
        {
            let (tx, replies) = mpsc::channel();
            let registrations = self._watcher.checkpoint_callbacks(move |id| {
                let _ = tx.send(Ok(id));
            });
            if registrations == 0 {
                return Err(SyncError::Unavailable("no live FSEvents streams"));
            }
            Ok(NativeCheckpoint {
                generation: self.test_state.generation,
                registrations,
                replies,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        Err(SyncError::Unavailable("backend has no native checkpoint"))
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

#[cfg(test)]
impl Drop for MonitorWatcher {
    fn drop(&mut self) {
        self.test_state.cancel(SyncError::GenerationChanged);
    }
}
