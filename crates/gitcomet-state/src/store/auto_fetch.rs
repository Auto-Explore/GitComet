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
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

const RESUME_CHECK: Duration = Duration::from_secs(30);
/// Repositories restored at launch wait this long, so their fetches do not
/// compete with opening them.
const STARTUP_GRACE: Duration = Duration::from_secs(60);
/// Fetches to one host start at least this far apart, so many repositories on
/// one server never reach it back to back.
const HOST_SPACING: Duration = Duration::from_secs(2);
const SPACING_POLL: Duration = Duration::from_millis(50);

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
    /// The tab is the selected one in its window; its repository fetches first.
    pub(super) active: bool,
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
    active: bool,
}

impl Subscriber {
    fn publish(&self, group: &Group, result: Option<Arc<Result<CommandOutput>>>) {
        (self.sink)(Event::Updated {
            repo_id: self.repo_id,
            lifetime: self.lifetime,
            revision: group.revision,
            snapshot: group.snapshot.clone(),
            result,
        });
    }
}

/// Worktrees that share an object store and resolve the same fetch
/// configuration update the same refs, so one fetch serves all of them.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum GroupKey {
    /// A workdir no fetch has opened yet.
    Workdir(PathBuf),
    Store {
        common_dir: PathBuf,
        config: u64,
    },
}

struct Entry {
    subscribers: BTreeMap<RepoLifetime, Subscriber>,
    common_dir: Option<PathBuf>,
    group: GroupKey,
}

impl Entry {
    fn enabled_subscriber(&self) -> Option<&Subscriber> {
        self.subscribers
            .values()
            .find(|subscriber| subscriber.settings.auto_fetch.enabled)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FetchRemote {
    name: String,
    /// `None` for a repository on this machine.
    host: Option<String>,
}

struct Group {
    schedule: Schedule,
    snapshot: RepoAutoFetchState,
    revision: u64,
    request: Option<u64>,
    order: u64,
    /// The remotes this group's last fetch saw.
    remotes: Vec<FetchRemote>,
    /// Skipped until a manual fetch succeeds: retrying cannot fix them.
    paused: BTreeSet<String>,
}

impl Group {
    fn new(revision: u64, order: u64) -> Self {
        Self {
            schedule: Schedule::default(),
            snapshot: RepoAutoFetchState::default(),
            revision,
            request: None,
            order,
            remotes: Vec::new(),
            paused: BTreeSet::new(),
        }
    }

    /// A fetch succeeded: the hosts it contacted answer again.
    fn reached(&mut self, hosts: &mut BTreeMap<String, HostBlock>, skipped: &[String]) {
        self.schedule.failures = 0;
        self.snapshot.retry_at = None;
        for host in contacted(&self.remotes, skipped).filter_map(|remote| remote.host.as_ref()) {
            hosts.remove(host);
        }
    }

    /// Charges a failure to the remotes the output names, or to every remote
    /// the fetch contacted when it names none.
    fn failed(
        &mut self,
        hosts: &mut BTreeMap<String, HostBlock>,
        error: &Error,
        skipped: &[String],
        interval: AutoFetchInterval,
        now: SystemTime,
    ) {
        let Some(failure) = policy::classify(error) else {
            return;
        };
        let named = policy::failed_remotes(error);
        let mut failed: Vec<&FetchRemote> = contacted(&self.remotes, skipped).collect();
        if failed.iter().any(|remote| named.contains(&remote.name)) {
            failed.retain(|remote| named.contains(&remote.name));
        }
        if failure.blocks_host() {
            for host in failed.iter().filter_map(|remote| remote.host.as_ref()) {
                let block = hosts.entry(host.clone()).or_insert(HostBlock {
                    until: now,
                    strikes: 0,
                });
                block.strikes += 1;
                block.until = now + policy::host_block(failure, block.strikes);
            }
        }
        if failure.pauses() {
            let failed: Vec<String> = failed.iter().map(|remote| remote.name.clone()).collect();
            self.paused.extend(failed);
            self.snapshot.paused_remotes = self.paused.iter().cloned().collect();
        }
        self.schedule.failures += 1;
        self.snapshot.retry_at = self.schedule.due_at(interval);
    }

    /// Clears failure state after a fetch that reached every remote.
    fn recover(&mut self) {
        self.schedule.failures = 0;
        self.paused.clear();
        self.snapshot.retry_at = None;
        self.snapshot.paused_remotes.clear();
    }
}

/// A host that failed with a network or busy error. No automatic fetch
/// contacts it until `until`; the first one after that probes it.
struct HostBlock {
    until: SystemTime,
    strikes: u32,
}

struct Running {
    group: GroupKey,
    cancellation: CancellationToken,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<PathBuf, Entry>,
    groups: BTreeMap<GroupKey, Group>,
    hosts: BTreeMap<String, HostBlock>,
    /// When each host was last contacted, for spacing.
    contacts: BTreeMap<String, Instant>,
    startup_grace: Duration,
    /// Groups created before this wait until then.
    grace_until: Option<SystemTime>,
    running: Option<Running>,
    revision: u64,
    order: u64,
    stopped: bool,
}

impl State {
    /// Each group with an enabled subscriber, with its first such member.
    fn enabled_groups(&self) -> BTreeMap<&GroupKey, (&PathBuf, &Entry, &Subscriber)> {
        let mut enabled = BTreeMap::new();
        for (workdir, entry) in &self.entries {
            if let Some(subscriber) = entry.enabled_subscriber() {
                enabled
                    .entry(&entry.group)
                    .or_insert((workdir, entry, subscriber));
            }
        }
        enabled
    }

    fn publish(&self, key: &GroupKey, result: Option<Arc<Result<CommandOutput>>>) {
        let Some(group) = self.groups.get(key) else {
            return;
        };
        for entry in self.entries.values().filter(|entry| &entry.group == key) {
            for subscriber in entry.subscribers.values() {
                subscriber.publish(group, result.clone());
            }
        }
    }

    /// Remotes a fetch of `group` leaves out: paused ones and those whose host
    /// is blocked.
    fn skipped_remotes(
        &self,
        group: Option<&Group>,
        remotes: &[FetchRemote],
        now: SystemTime,
    ) -> Vec<String> {
        remotes
            .iter()
            .filter(|remote| {
                group.is_some_and(|group| group.paused.contains(&remote.name))
                    || remote.host.as_ref().is_some_and(|host| {
                        self.hosts.get(host).is_some_and(|block| block.until > now)
                    })
            })
            .map(|remote| remote.name.clone())
            .collect()
    }

    /// Every remote the group knows is skipped, so a fetch would contact none.
    fn group_waits(&self, group: &Group, now: SystemTime) -> bool {
        !group.remotes.is_empty()
            && self.skipped_remotes(Some(group), &group.remotes, now).len() == group.remotes.len()
    }

    fn cancel_inactive_fetch(&self) {
        if let Some(running) = &self.running
            && !self
                .entries
                .values()
                .any(|entry| entry.group == running.group && entry.enabled_subscriber().is_some())
        {
            running.cancellation.cancel();
        }
    }

    fn retain_groups(&mut self) {
        let Self {
            entries, groups, ..
        } = self;
        let used: FxHashSet<&GroupKey> = entries.values().map(|entry| &entry.group).collect();
        groups.retain(|key, _| used.contains(key));
    }

    /// Moves `workdir` into `key`. A new group takes over the schedule and
    /// status of the one the workdir leaves.
    fn regroup(&mut self, workdir: &Path, key: GroupKey) {
        let Some(entry) = self.entries.get_mut(workdir) else {
            return;
        };
        if entry.group == key {
            return;
        }
        let previous = std::mem::replace(&mut entry.group, key.clone());
        let orphaned = !self.entries.values().any(|entry| entry.group == previous);
        if !self.groups.contains_key(&key) {
            let group = if orphaned {
                self.groups.remove(&previous)
            } else {
                self.groups.get(&previous).map(|group| Group {
                    schedule: group.schedule,
                    snapshot: group.snapshot.clone(),
                    revision: group.revision,
                    request: None,
                    order: group.order,
                    remotes: Vec::new(),
                    paused: BTreeSet::new(),
                })
            };
            self.groups.insert(
                key,
                group.unwrap_or_else(|| Group::new(self.revision, self.order)),
            );
        } else if orphaned {
            self.groups.remove(&previous);
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
    /// A coordinator without launch grace, for tests that drive it directly.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn new() -> Arc<Self> {
        Self::with_startup_grace(Duration::ZERO)
    }

    fn with_startup_grace(startup_grace: Duration) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    startup_grace,
                    ..State::default()
                }),
                admission: Arc::new(Admission::default()),
            }),
            worker: Mutex::new(None),
            #[cfg(test)]
            syncs: Default::default(),
        })
    }

    pub(super) fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<AutoFetchCoordinator>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Self::with_startup_grace(STARTUP_GRACE)))
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
        let was_enabled: FxHashSet<GroupKey> =
            state.enabled_groups().into_keys().cloned().collect();
        let registered: FxHashSet<(&Path, RepoLifetime)> = registrations
            .iter()
            .map(|(key, registration)| (key.as_path(), registration.lifetime))
            .collect();
        for (key, entry) in &mut state.entries {
            entry.subscribers.retain(|lifetime, subscriber| {
                subscriber.store_id != store_id || registered.contains(&(key.as_path(), *lifetime))
            });
        }
        let now = SystemTime::now();
        if !registrations.is_empty() && state.grace_until.is_none() {
            state.grace_until = Some(now + state.startup_grace);
        }
        let not_before = state.grace_until.filter(|until| *until > now);
        let mut added = Vec::new();
        let mut created = FxHashSet::default();
        for (key, registration) in registrations {
            state.order += 1;
            let order = state.order;
            let State {
                entries, groups, ..
            } = &mut *state;
            let entry = entries.entry(key.clone()).or_insert_with(|| {
                let group = GroupKey::Workdir(key.clone());
                groups.entry(group.clone()).or_insert_with(|| {
                    created.insert(group.clone());
                    let mut group = Group::new(revision, order);
                    group.schedule.completed = last_fetch_time(&key);
                    group.schedule.not_before = not_before;
                    group
                });
                Entry {
                    subscribers: BTreeMap::new(),
                    common_dir: None,
                    group,
                }
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
                active: registration.active,
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
        state.retain_groups();
        let enabled: FxHashSet<GroupKey> = state.enabled_groups().into_keys().cloned().collect();
        let toggled: Vec<GroupKey> = state
            .groups
            .keys()
            .filter(|key| {
                !created.contains(*key) && was_enabled.contains(*key) != enabled.contains(*key)
            })
            .cloned()
            .collect();
        for key in toggled {
            if let Some(group) = state.groups.get_mut(&key) {
                group.schedule = Schedule::default();
                group.recover();
                group.request = None;
                if matches!(group.snapshot.status, AutoFetchStatus::Running) {
                    group.snapshot.status = AutoFetchStatus::Idle;
                }
                group.revision = revision;
            }
            state.publish(&key, None);
        }
        let State {
            entries, groups, ..
        } = &mut *state;
        for (key, lifetime) in added {
            if let Some(entry) = entries.get_mut(&key)
                && let Some(group) = groups.get(&entry.group)
                && let Some(subscriber) = entry.subscribers.get_mut(&lifetime)
            {
                subscriber.publish(group, None);
                // A tab whose repository fetched recently, or waits out the
                // launch, loads its last known count now instead.
                let settings = subscriber.settings.auto_fetch;
                if subscriber.needs_refresh
                    && settings.enabled
                    && !group.schedule.is_due(settings.interval, now)
                {
                    subscriber.needs_refresh = false;
                    (subscriber.sink)(Event::Refresh {
                        repo_id: subscriber.repo_id,
                        lifetime: subscriber.lifetime,
                    });
                }
            }
        }
        state.cancel_inactive_fetch();
        drop(state);
        if !enabled.is_empty() {
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
        state.retain_groups();
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
        let group_key = state.entries.get(&key).map(|entry| entry.group.clone());
        let State { groups, hosts, .. } = &mut *state;
        if let Some(group_key) = group_key
            && let Some(group) = groups.get_mut(&group_key)
        {
            if result.is_ok() {
                group.schedule.finish(now);
                group.snapshot.last_success_at = Some(now);
                // A manual fetch reached every remote: their hosts answer and
                // their credentials work again.
                group.recover();
                for host in group
                    .remotes
                    .iter()
                    .filter_map(|remote| remote.host.as_ref())
                {
                    hosts.remove(host);
                }
            }
            group.request = None;
            group.snapshot.status = status;
            group.revision = revision;
            state.publish(&group_key, None);
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
    group: GroupKey,
    workdir: PathBuf,
    request: u64,
    started: SystemTime,
    cancellation: CancellationToken,
    subscriber: Subscriber,
}

enum Completion {
    /// No fetch ran: a foreground command holds the store, every remote is
    /// skipped, or the workdir joined a group whose fetch is not due.
    Deferred {
        common_dir: PathBuf,
        key: GroupKey,
        remotes: Vec<FetchRemote>,
    },
    Finished {
        common_dir: Option<PathBuf>,
        key: Option<GroupKey>,
        remotes: Vec<FetchRemote>,
        skipped: Vec<String>,
        result: Result<CommandOutput>,
        refs_changed: bool,
    },
}

/// The store a group's fetch would lock, when known.
fn group_common_dir<'a>(key: &'a GroupKey, member: &'a Entry) -> Option<&'a PathBuf> {
    match key {
        GroupKey::Store { common_dir, .. } => Some(common_dir),
        GroupKey::Workdir(_) => member.common_dir.as_ref(),
    }
}

fn next_job(state: &mut State, admission: &Admission, now: SystemTime) -> Option<Job> {
    if state.running.is_some() {
        return None;
    }
    let active: FxHashSet<&GroupKey> = state
        .entries
        .values()
        .filter(|entry| {
            entry
                .subscribers
                .values()
                .any(|subscriber| subscriber.active)
        })
        .map(|entry| &entry.group)
        .collect();
    let last_contact = |group: &Group| {
        group
            .remotes
            .iter()
            .filter_map(|remote| state.contacts.get(remote.host.as_ref()?))
            .max()
            .copied()
    };
    // Selected tabs first, then hosts contacted least recently, then the
    // group that started longest ago.
    let (key, workdir) = state
        .enabled_groups()
        .into_iter()
        .filter_map(|(key, (workdir, entry, subscriber))| {
            let group = state.groups.get(key)?;
            (group
                .schedule
                .is_due(subscriber.settings.auto_fetch.interval, now)
                && !state.group_waits(group, now)
                && group_common_dir(key, entry).is_none_or(|common| !admission.blocked(common)))
            .then(|| {
                (
                    (!active.contains(key), last_contact(group), group.order),
                    key,
                    workdir,
                )
            })
        })
        .min_by_key(|(rank, ..)| *rank)
        .map(|(_, key, workdir)| (key.clone(), workdir.clone()))?;
    state.revision += 1;
    state.order += 1;
    let subscriber = state.entries.get(&workdir)?.enabled_subscriber()?.clone();
    let group = state.groups.get_mut(&key)?;
    let cancellation = CancellationToken::new();
    group.order = state.order;
    group.request = Some(state.revision);
    group.revision = state.revision;
    group.snapshot.status = AutoFetchStatus::Running;
    state.publish(&key, None);
    state.running = Some(Running {
        group: key.clone(),
        cancellation: cancellation.clone(),
    });
    Some(Job {
        group: key,
        workdir,
        request: state.revision,
        started: now,
        cancellation,
        subscriber,
    })
}

fn execute(job: &Job, inner: &Inner) -> Completion {
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
                key: None,
                remotes: Vec::new(),
                skipped: Vec::new(),
                result: Err(error),
                refs_changed: false,
            };
        }
    };
    let common_dir = repository_key(repo.as_ref());
    let key = repo.fetch_config_signature().map_or_else(
        |_| GroupKey::Workdir(job.workdir.clone()),
        |config| GroupKey::Store {
            common_dir: common_dir.clone(),
            config,
        },
    );
    let remotes: Vec<FetchRemote> = repo
        .list_remotes(&job.cancellation)
        .unwrap_or_default()
        .into_iter()
        .map(|remote| FetchRemote {
            host: remote.url.as_deref().and_then(policy::remote_host),
            name: remote.name,
        })
        .collect();
    let (joins_fetched_group, skipped, spacing) = {
        let state = inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let joined = state.groups.get(&key);
        // A sibling worktree fetched this store recently: join its schedule
        // instead of contacting the remote again.
        let joins_fetched_group = key != job.group
            && joined.is_some_and(|group| {
                !group
                    .schedule
                    .is_due(job.subscriber.settings.auto_fetch.interval, job.started)
            });
        let group = joined.or_else(|| state.groups.get(&job.group));
        let skipped = state.skipped_remotes(group, &remotes, job.started);
        let spacing = contacted(&remotes, &skipped)
            .filter_map(|remote| state.contacts.get(remote.host.as_ref()?))
            .map(|contacted| HOST_SPACING.saturating_sub(contacted.elapsed()))
            .max()
            .unwrap_or_default();
        (joins_fetched_group, skipped, spacing)
    };
    if joins_fetched_group || (!remotes.is_empty() && skipped.len() == remotes.len()) {
        return Completion::Deferred {
            common_dir,
            key,
            remotes,
        };
    }
    if let Err(error) = sleep_unless_cancelled(spacing, &job.cancellation) {
        return Completion::Finished {
            common_dir: Some(common_dir),
            key: Some(key),
            remotes,
            skipped,
            result: Err(error),
            refs_changed: false,
        };
    }
    let Some(_lease) = inner
        .admission
        .try_background(common_dir.clone(), job.cancellation.clone())
    else {
        return Completion::Deferred {
            common_dir,
            key,
            remotes,
        };
    };
    {
        let mut state = inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        for host in contacted(&remotes, &skipped).filter_map(|remote| remote.host.clone()) {
            state.contacts.insert(host, now);
        }
    }
    let refs_before = repo.remote_refs_fingerprint(&job.cancellation).ok();
    let result = job.cancellation.check_cancelled().and_then(|()| {
        repo.fetch_all_background_except(
            job.subscriber
                .settings
                .prune_deleted_remote_branches_on_fetch,
            &skipped,
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
        key: Some(key),
        remotes,
        skipped,
        result,
        refs_changed,
    }
}

/// The remotes a fetch that leaves out `skipped` contacts.
fn contacted<'a>(
    remotes: &'a [FetchRemote],
    skipped: &'a [String],
) -> impl Iterator<Item = &'a FetchRemote> {
    remotes
        .iter()
        .filter(|remote| !skipped.contains(&remote.name))
}

fn sleep_unless_cancelled(duration: Duration, cancellation: &CancellationToken) -> Result<()> {
    let deadline = Instant::now() + duration;
    loop {
        cancellation.check_cancelled()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        std::thread::sleep(remaining.min(SPACING_POLL));
    }
}

/// When git last fetched into `workdir`. Git rewrites FETCH_HEAD on every
/// fetch, including ones run outside GitComet.
fn last_fetch_time(workdir: &Path) -> Option<SystemTime> {
    let dot_git = workdir.join(".git");
    let git_dir = match std::fs::read_to_string(&dot_git) {
        // A linked worktree's `.git` file names its own Git directory.
        Ok(link) => {
            let target = Path::new(link.strip_prefix("gitdir:")?.trim());
            workdir.join(target)
        }
        Err(_) if dot_git.is_dir() => dot_git,
        Err(_) => workdir.to_path_buf(),
    };
    std::fs::metadata(git_dir.join("FETCH_HEAD"))
        .ok()?
        .modified()
        .ok()
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
    // A closed tab or a manual fetch since leaves this job nothing to record.
    let current = state
        .entries
        .get(&job.workdir)
        .is_some_and(|entry| entry.group == job.group)
        && state
            .groups
            .get(&job.group)
            .is_some_and(|group| group.request == Some(job.request));
    if current && let Some(group) = state.groups.get_mut(&job.group) {
        group.request = None;
        if matches!(group.snapshot.status, AutoFetchStatus::Running) {
            group.snapshot.status = AutoFetchStatus::Idle;
        }
    }
    let (common_dir, key, remotes, outcome) = match completion {
        Completion::Deferred {
            common_dir,
            key,
            remotes,
        } => (Some(common_dir), Some(key), remotes, None),
        Completion::Finished {
            common_dir,
            key,
            remotes,
            skipped,
            result,
            refs_changed,
        } => (
            common_dir,
            key,
            remotes,
            Some((result, skipped, refs_changed)),
        ),
    };
    if current {
        if let Some(entry) = state.entries.get_mut(&job.workdir) {
            entry.common_dir = common_dir.clone().or_else(|| entry.common_dir.clone());
        }
        if let Some(key) = key {
            state.regroup(&job.workdir, key);
        }
    }
    let Some(group_key) = state
        .entries
        .get(&job.workdir)
        .map(|entry| entry.group.clone())
    else {
        return;
    };
    if current
        && !remotes.is_empty()
        && let Some(group) = state.groups.get_mut(&group_key)
    {
        group.remotes = remotes;
    }
    let Some((result, skipped, refs_changed)) = outcome else {
        if current && let Some(group) = state.groups.get_mut(&group_key) {
            group.revision = revision;
            state.publish(&group_key, None);
        }
        return;
    };
    let State { groups, hosts, .. } = &mut *state;
    if current && let Some(group) = groups.get_mut(&group_key) {
        group.schedule.finish(now);
        group.snapshot.status = match &result {
            Ok(_) => {
                group.snapshot.last_success_at = Some(now);
                AutoFetchStatus::Succeeded { at: now }
            }
            Err(error) if matches!(error.kind(), ErrorKind::Cancelled) => AutoFetchStatus::Idle,
            Err(error) => AutoFetchStatus::Failed {
                at: now,
                error: error.clone(),
            },
        };
        match &result {
            Ok(_) => group.reached(hosts, &skipped),
            Err(error) => group.failed(
                hosts,
                error,
                &skipped,
                job.subscriber.settings.auto_fetch.interval,
                now,
            ),
        }
        group.revision = revision;
        state.publish(&group_key, Some(Arc::new(result)));
    }
    send_refreshes(
        state,
        &job.workdir,
        &group_key,
        common_dir.as_ref(),
        refs_changed,
    );
}

/// Refreshes every tab reading the fetched refs: the fetching workdir, its
/// group and anything else on the same store. Tabs that have loaded once skip
/// the refresh when the fetch moved nothing.
fn send_refreshes(
    state: &mut State,
    fetched: &Path,
    group: &GroupKey,
    common_dir: Option<&PathBuf>,
    refs_changed: bool,
) {
    for (workdir, entry) in &mut state.entries {
        if workdir != fetched
            && &entry.group != group
            && common_dir.is_none_or(|common| entry.common_dir.as_ref() != Some(common))
        {
            continue;
        }
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
            let completion =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(&job, &inner)))
                    .unwrap_or_else(|panic| Completion::Finished {
                        common_dir: None,
                        key: None,
                        remotes: Vec::new(),
                        skipped: Vec::new(),
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
    let enabled = state.enabled_groups();
    if enabled.is_empty() {
        return None;
    }
    Some(
        enabled
            .into_iter()
            .filter_map(|(key, (_, entry, subscriber))| {
                if group_common_dir(key, entry).is_some_and(|common| admission.blocked(common)) {
                    return None;
                }
                state
                    .groups
                    .get(key)?
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

#[derive(Clone, Copy, Default)]
struct Schedule {
    completed: Option<SystemTime>,
    /// Consecutive failed fetches; each one stretches the next wait.
    failures: u32,
    /// Launch grace: no fetch before this, whatever `completed` says.
    not_before: Option<SystemTime>,
}

impl Schedule {
    fn wait(&self, interval: AutoFetchInterval) -> Duration {
        policy::retry_delay(interval, self.failures)
    }

    fn is_due(&self, interval: AutoFetchInterval, now: SystemTime) -> bool {
        self.not_before.is_none_or(|not_before| now >= not_before)
            && self.completed.is_none_or(|completed| {
                now.duration_since(completed)
                    .map_or(true, |elapsed| elapsed >= self.wait(interval))
            })
    }

    fn due_at(&self, interval: AutoFetchInterval) -> Option<SystemTime> {
        let due = self.completed?.checked_add(self.wait(interval))?;
        Some(
            self.not_before
                .map_or(due, |not_before| due.max(not_before)),
        )
    }

    fn finish(&mut self, now: SystemTime) {
        self.completed = Some(now);
    }
}

mod policy;

#[cfg(test)]
mod tests;
