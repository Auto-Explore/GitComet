use super::auto_fetch_admission::Admission;
use crate::model::{
    AutoFetchInterval, AutoFetchStatus, RemoteSettings, RepoAutoFetchState, RepoId, RepoLifetime,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{
    CancellationToken, CommandOutput, GitBackend, GitRepository, Result,
};
use rustc_hash::FxHashSet;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

const RESUME_CHECK: Duration = Duration::from_secs(30);

pub(super) type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

pub(super) enum Event {
    Updated {
        repo_id: RepoId,
        lifetime: RepoLifetime,
        revision: u64,
        snapshot: RepoAutoFetchState,
        result: Option<Arc<Result<CommandOutput>>>,
    },
    Refresh {
        repo_id: RepoId,
        lifetime: RepoLifetime,
    },
}

pub(super) struct Registration {
    pub(super) repo_id: RepoId,
    pub(super) lifetime: RepoLifetime,
    pub(super) workdir: PathBuf,
    pub(super) common_dir: Option<PathBuf>,
    pub(super) repository: Option<Arc<dyn GitRepository>>,
}

#[derive(Clone)]
struct Subscriber {
    store_id: u64,
    repo_id: RepoId,
    lifetime: RepoLifetime,
    original_path: PathBuf,
    settings: RemoteSettings,
    backend: Arc<dyn GitBackend>,
    repository: Option<Arc<dyn GitRepository>>,
    sink: EventSink,
    /// A new tab loads its first incoming count through a refresh, so it gets
    /// one even after a fetch that moved nothing.
    needs_refresh: bool,
}

impl Subscriber {
    fn publish(&self, entry: &Entry, result: Option<Arc<Result<CommandOutput>>>) {
        (self.sink)(Event::Updated {
            repo_id: self.repo_id,
            lifetime: self.lifetime,
            revision: entry.revision,
            snapshot: entry.snapshot.clone(),
            result,
        });
    }
}

struct Entry {
    subscribers: BTreeMap<RepoLifetime, Subscriber>,
    common_dir: Option<PathBuf>,
    schedule: Schedule,
    snapshot: RepoAutoFetchState,
    revision: u64,
    request: Option<u64>,
    order: u64,
}

impl Entry {
    fn enabled_subscriber(&self) -> Option<&Subscriber> {
        self.subscribers
            .values()
            .find(|subscriber| subscriber.settings.auto_fetch.enabled)
    }

    fn publish(&self, result: Option<Arc<Result<CommandOutput>>>) {
        for subscriber in self.subscribers.values() {
            subscriber.publish(self, result.clone());
        }
    }
}

struct Running {
    workdir: PathBuf,
    cancellation: CancellationToken,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<PathBuf, Entry>,
    running: Option<Running>,
    revision: u64,
    order: u64,
    stopped: bool,
}

impl State {
    fn cancel_inactive_fetch(&self) {
        if let Some(running) = &self.running
            && self
                .entries
                .get(&running.workdir)
                .is_none_or(|entry| entry.enabled_subscriber().is_none())
        {
            running.cancellation.cancel();
        }
    }
}

struct Inner {
    state: Mutex<State>,
    admission: Arc<Admission>,
}

pub(super) struct AutoFetchCoordinator {
    inner: Arc<Inner>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    #[cfg(test)]
    syncs: std::sync::atomic::AtomicUsize,
}

impl AutoFetchCoordinator {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                admission: Arc::new(Admission::default()),
            }),
            worker: Mutex::new(None),
            #[cfg(test)]
            syncs: Default::default(),
        })
    }

    pub(super) fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<AutoFetchCoordinator>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(Self::new))
    }

    pub(super) fn admission(&self) -> &Arc<Admission> {
        &self.inner.admission
    }

    pub(super) fn sync(
        &self,
        store_id: u64,
        registrations: Vec<Registration>,
        settings: RemoteSettings,
        backend: &Arc<dyn GitBackend>,
        sink: EventSink,
    ) {
        #[cfg(test)]
        self.syncs
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let known: HashMap<_, _> = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .iter()
            .flat_map(|(key, entry)| {
                entry
                    .subscribers
                    .values()
                    .filter(move |subscriber| subscriber.store_id == store_id)
                    .map(move |subscriber| {
                        (
                            subscriber.lifetime,
                            (subscriber.original_path.clone(), key.clone()),
                        )
                    })
            })
            .collect();
        let registrations: Vec<_> = registrations
            .into_iter()
            .map(|registration| {
                let key = known
                    .get(&registration.lifetime)
                    .filter(|(path, _)| path == &registration.workdir)
                    .map(|(_, key)| key.clone())
                    .unwrap_or_else(|| canonicalize_or_original(registration.workdir.clone()));
                (key, registration)
            })
            .collect();
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return;
        }
        state.revision += 1;
        let revision = state.revision;
        let was_enabled: HashMap<_, _> = state
            .entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.enabled_subscriber().is_some()))
            .collect();
        let registered: FxHashSet<(&Path, RepoLifetime)> = registrations
            .iter()
            .map(|(key, registration)| (key.as_path(), registration.lifetime))
            .collect();
        for (key, entry) in &mut state.entries {
            entry.subscribers.retain(|lifetime, subscriber| {
                subscriber.store_id != store_id || registered.contains(&(key.as_path(), *lifetime))
            });
        }
        let mut added = Vec::new();
        for (key, registration) in registrations {
            state.order += 1;
            let order = state.order;
            let entry = state.entries.entry(key.clone()).or_insert_with(|| Entry {
                subscribers: BTreeMap::new(),
                common_dir: None,
                schedule: Schedule::default(),
                snapshot: RepoAutoFetchState::default(),
                revision,
                request: None,
                order,
            });
            if registration.common_dir.is_some() {
                entry.common_dir = registration.common_dir;
            }
            let needs_refresh = entry
                .subscribers
                .get(&registration.lifetime)
                .is_none_or(|subscriber| subscriber.needs_refresh);
            let subscriber = Subscriber {
                store_id,
                repo_id: registration.repo_id,
                lifetime: registration.lifetime,
                original_path: registration.workdir,
                settings,
                backend: Arc::clone(backend),
                repository: registration.repository,
                sink: Arc::clone(&sink),
                needs_refresh,
            };
            if entry
                .subscribers
                .insert(subscriber.lifetime, subscriber)
                .is_none()
            {
                added.push((key, registration.lifetime));
            }
        }
        state
            .entries
            .retain(|_, entry| !entry.subscribers.is_empty());
        for (key, entry) in &mut state.entries {
            let enabled = entry.enabled_subscriber().is_some();
            if was_enabled.get(key).copied().unwrap_or(false) != enabled {
                entry.schedule = Schedule::default();
                entry.request = None;
                if matches!(entry.snapshot.status, AutoFetchStatus::Running) {
                    entry.snapshot.status = AutoFetchStatus::Idle;
                }
                entry.revision = revision;
                entry.publish(None);
            }
        }
        for (key, lifetime) in added {
            if let Some(entry) = state.entries.get(&key)
                && let Some(subscriber) = entry.subscribers.get(&lifetime)
            {
                subscriber.publish(entry, None);
            }
        }
        state.cancel_inactive_fetch();
        let enabled = state
            .entries
            .values()
            .any(|entry| entry.enabled_subscriber().is_some());
        drop(state);
        if enabled {
            let mut worker = self
                .worker
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let stopped = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .stopped;
            if worker.is_none() && !stopped {
                let inner = Arc::clone(&self.inner);
                *worker = Some(
                    std::thread::Builder::new()
                        .name(format!(
                            "{}-auto-fetch",
                            gitcomet_core::identity::current().executable_name()
                        ))
                        .spawn(move || run(inner))
                        .expect("spawn automatic fetch worker"),
                );
            }
        }
        self.inner.admission.changed.notify_all();
    }

    pub(super) fn unsubscribe(&self, store_id: u64) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for entry in state.entries.values_mut() {
            entry
                .subscribers
                .retain(|_, subscriber| subscriber.store_id != store_id);
        }
        state
            .entries
            .retain(|_, entry| !entry.subscribers.is_empty());
        state.cancel_inactive_fetch();
        self.inner.admission.changed.notify_all();
    }

    pub(super) fn manual_fetch_finished(
        &self,
        workdir: &Path,
        now: SystemTime,
        result: &Result<CommandOutput>,
    ) {
        let status = match result {
            Ok(_) => AutoFetchStatus::Succeeded { at: now },
            Err(error) if matches!(error.kind(), ErrorKind::Cancelled) => return,
            Err(error) => AutoFetchStatus::Failed {
                at: now,
                error: error.clone(),
            },
        };
        let key = canonicalize_or_original(workdir.to_path_buf());
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.revision += 1;
        let revision = state.revision;
        if let Some(entry) = state.entries.get_mut(&key) {
            if result.is_ok() {
                entry.schedule.finish(now);
                entry.snapshot.last_success_at = Some(now);
            }
            entry.request = None;
            entry.snapshot.status = status;
            entry.revision = revision;
            entry.publish(None);
        }
        self.inner.admission.changed.notify_all();
    }

    pub(super) fn refresh_other_subscribers(
        &self,
        store_id: u64,
        repo_id: RepoId,
        workdir: &Path,
        common_dir: &Path,
    ) {
        let workdir = canonicalize_or_original(workdir.to_path_buf());
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for (key, entry) in &state.entries {
            if key != &workdir && entry.common_dir.as_deref() != Some(common_dir) {
                continue;
            }
            for subscriber in entry.subscribers.values() {
                if subscriber.store_id == store_id && subscriber.repo_id == repo_id {
                    continue;
                }
                (subscriber.sink)(Event::Refresh {
                    repo_id: subscriber.repo_id,
                    lifetime: subscriber.lifetime,
                });
            }
        }
    }

    pub(super) fn shutdown(&self) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.stopped = true;
        if let Some(running) = &state.running {
            running.cancellation.cancel();
        }
        drop(state);
        self.inner.admission.changed.notify_all();
        let mut worker = self
            .worker
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(worker) = worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for AutoFetchCoordinator {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Job {
    workdir: PathBuf,
    request: u64,
    cancellation: CancellationToken,
    subscriber: Subscriber,
}

enum Completion {
    Deferred {
        common_dir: PathBuf,
    },
    Finished {
        common_dir: Option<PathBuf>,
        result: Result<CommandOutput>,
        refs_changed: bool,
    },
}

fn next_job(state: &mut State, admission: &Admission, now: SystemTime) -> Option<Job> {
    if state.running.is_some() {
        return None;
    }
    let workdir = state
        .entries
        .iter()
        .filter(|(_, entry)| {
            entry.enabled_subscriber().is_some_and(|subscriber| {
                entry
                    .schedule
                    .is_due(subscriber.settings.auto_fetch.interval, now)
                    && entry
                        .common_dir
                        .as_ref()
                        .is_none_or(|common| !admission.blocked(common))
            })
        })
        .min_by_key(|(_, entry)| entry.order)
        .map(|(workdir, _)| workdir.clone())?;
    state.revision += 1;
    state.order += 1;
    let entry = state.entries.get_mut(&workdir)?;
    let subscriber = entry.enabled_subscriber()?.clone();
    let cancellation = CancellationToken::new();
    entry.order = state.order;
    entry.request = Some(state.revision);
    entry.revision = state.revision;
    entry.snapshot.status = AutoFetchStatus::Running;
    entry.publish(None);
    state.running = Some(Running {
        workdir: workdir.clone(),
        cancellation: cancellation.clone(),
    });
    Some(Job {
        workdir,
        request: state.revision,
        cancellation,
        subscriber,
    })
}

fn execute(job: &Job, admission: &Arc<Admission>) -> Completion {
    let opened = job.cancellation.check_cancelled().and_then(|()| {
        job.subscriber.repository.clone().map_or_else(
            || {
                job.subscriber
                    .backend
                    .open_cancellable(&job.workdir, &job.cancellation)
            },
            Ok,
        )
    });
    let repo = match opened {
        Ok(repo) => repo,
        Err(error) => {
            return Completion::Finished {
                common_dir: None,
                result: Err(error),
                refs_changed: false,
            };
        }
    };
    let common_dir = repository_key(repo.as_ref());
    let Some(_lease) = admission.try_background(common_dir.clone(), job.cancellation.clone())
    else {
        return Completion::Deferred { common_dir };
    };
    let refs_before = repo.remote_refs_fingerprint(&job.cancellation).ok();
    let result = job.cancellation.check_cancelled().and_then(|()| {
        repo.fetch_all_background(
            job.subscriber
                .settings
                .prune_deleted_remote_branches_on_fetch,
            &job.cancellation,
        )
    });
    // An unreadable fingerprint counts as a change.
    let refs_changed = refs_before.is_none()
        || repo.remote_refs_fingerprint(&job.cancellation).ok() != refs_before;
    if refs_changed {
        // Refreshes must read the packs this fetch wrote, and old packs must
        // not stay mapped. Releasing one handle refreshes every handle that
        // shares its object store.
        repo.release_object_store();
        super::effects::release_all_worktree_scan_handles();
    }
    Completion::Finished {
        common_dir: Some(common_dir),
        result,
        refs_changed,
    }
}

pub(super) fn repository_key(repo: &dyn GitRepository) -> PathBuf {
    canonicalize_or_original(
        repo.common_dir()
            .unwrap_or_else(|| repo.spec().workdir.clone()),
    )
}

fn finish(state: &mut State, job: &Job, completion: Completion, now: SystemTime) {
    state.running = None;
    state.revision += 1;
    let revision = state.revision;
    let (common_dir, refs_changed) = match completion {
        Completion::Deferred { common_dir } => {
            if let Some(entry) = state.entries.get_mut(&job.workdir)
                && entry.request == Some(job.request)
            {
                entry.common_dir = Some(common_dir);
                entry.snapshot.status = AutoFetchStatus::Idle;
                entry.request = None;
                entry.revision = revision;
                entry.publish(None);
            }
            return;
        }
        Completion::Finished {
            common_dir,
            result,
            refs_changed,
        } => {
            if let Some(entry) = state.entries.get_mut(&job.workdir)
                && entry.request == Some(job.request)
            {
                entry.common_dir = common_dir.clone().or_else(|| entry.common_dir.clone());
                entry.schedule.finish(now);
                entry.snapshot.status = match &result {
                    Ok(_) => {
                        entry.snapshot.last_success_at = Some(now);
                        AutoFetchStatus::Succeeded { at: now }
                    }
                    Err(error) if matches!(error.kind(), ErrorKind::Cancelled) => {
                        AutoFetchStatus::Idle
                    }
                    Err(error) => AutoFetchStatus::Failed {
                        at: now,
                        error: error.clone(),
                    },
                };
                entry.request = None;
                entry.revision = revision;
                entry.publish(Some(Arc::new(result)));
            }
            (common_dir, refs_changed)
        }
    };
    for (workdir, entry) in &mut state.entries {
        if workdir == &job.workdir
            || common_dir
                .as_ref()
                .is_some_and(|common| entry.common_dir.as_ref() == Some(common))
        {
            for subscriber in entry.subscribers.values_mut() {
                if !refs_changed && !subscriber.needs_refresh {
                    continue;
                }
                subscriber.needs_refresh = false;
                (subscriber.sink)(Event::Refresh {
                    repo_id: subscriber.repo_id,
                    lifetime: subscriber.lifetime,
                });
            }
        }
    }
}

fn run(inner: Arc<Inner>) {
    let mut state = inner
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    loop {
        if state.stopped {
            return;
        }
        let now = SystemTime::now();
        if let Some(job) = next_job(&mut state, &inner.admission, now) {
            drop(state);
            let completion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                execute(&job, &inner.admission)
            }))
            .unwrap_or_else(|panic| Completion::Finished {
                common_dir: None,
                result: Err(Error::new(ErrorKind::Backend(format!(
                    "automatic fetch failed: {}",
                    gitcomet_core::panic::payload_message(panic.as_ref())
                )))),
                refs_changed: true,
            });
            state = inner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            finish(&mut state, &job, completion, SystemTime::now());
            continue;
        }
        state = match wait_duration(&state, &inner.admission, now) {
            Some(wait) => {
                inner
                    .admission
                    .changed
                    .wait_timeout(state, wait)
                    .unwrap_or_else(|error| error.into_inner())
                    .0
            }
            None => inner
                .admission
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner()),
        };
    }
}

fn wait_duration(state: &State, admission: &Admission, now: SystemTime) -> Option<Duration> {
    if !state
        .entries
        .values()
        .any(|entry| entry.enabled_subscriber().is_some())
    {
        return None;
    }
    Some(
        state
            .entries
            .values()
            .filter_map(|entry| {
                let subscriber = entry.enabled_subscriber()?;
                if entry
                    .common_dir
                    .as_ref()
                    .is_some_and(|common| admission.blocked(common))
                {
                    return None;
                }
                entry
                    .schedule
                    .completed
                    .and_then(|completed| {
                        completed.checked_add(subscriber.settings.auto_fetch.interval.duration())
                    })
                    .and_then(|deadline| deadline.duration_since(now).ok())
            })
            .min()
            .unwrap_or(RESUME_CHECK)
            .min(RESUME_CHECK),
    )
}

#[derive(Default)]
struct Schedule {
    completed: Option<SystemTime>,
}

impl Schedule {
    fn is_due(&self, interval: AutoFetchInterval, now: SystemTime) -> bool {
        self.completed.is_none_or(|completed| {
            now.duration_since(completed)
                .map_or(true, |elapsed| elapsed >= interval.duration())
        })
    }

    fn finish(&mut self, now: SystemTime) {
        self.completed = Some(now);
    }
}

#[cfg(test)]
mod tests;
