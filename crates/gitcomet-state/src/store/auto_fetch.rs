use super::auto_fetch_admission::Admission;
use crate::model::{AutoFetchStatus, RemoteSettings, RepoAutoFetchState, RepoId, RepoLifetime};
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

/// Repositories restored at launch wait this long, so their fetches do not
/// compete with opening them.
const STARTUP_GRACE: Duration = Duration::from_secs(60);
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
    /// The tab has no current incoming count and is not loading one.
    pub(super) needs_refresh: bool,
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
    /// The tab's incoming count is unknown or greyed, so the next fetch
    /// reloads it even when it moves nothing. Each sync takes the store's
    /// word; a sent refresh or a failed fetch updates it in between.
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

    /// Reloads the tab's incoming count from the refs on disk.
    fn refresh(&mut self) {
        self.needs_refresh = false;
        (self.sink)(Event::Refresh {
            repo_id: self.repo_id,
            lifetime: self.lifetime,
        });
    }
}

struct Entry {
    subscribers: BTreeMap<RepoLifetime, Subscriber>,
    common_dir: Option<PathBuf>,
    group: GroupKey,
}

impl Entry {
    /// The selected tab in some window shows this workdir.
    fn is_active(&self) -> bool {
        self.subscribers
            .values()
            .any(|subscriber| subscriber.active)
    }

    fn enabled_subscriber(&self) -> Option<&Subscriber> {
        self.subscribers
            .values()
            .find(|subscriber| subscriber.settings.auto_fetch.enabled)
    }
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
    /// Stores whose window asks the user for credentials: a fetch started
    /// meanwhile could only fail or ask again.
    signing_in: BTreeSet<u64>,
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
                        self.hosts
                            .get(host)
                            .is_some_and(|block| block.until.is_none_or(|until| until > now))
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

    /// A failure greys the counts of `key`'s tabs until a fetch reloads them.
    fn mark_stale(&mut self, key: &GroupKey) {
        for entry in self
            .entries
            .values_mut()
            .filter(|entry| &entry.group == key)
        {
            for subscriber in entry.subscribers.values_mut() {
                subscriber.needs_refresh = true;
            }
        }
    }

    fn group_is_active(&self, key: &GroupKey) -> bool {
        self.entries
            .values()
            .any(|entry| &entry.group == key && entry.is_active())
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
            let subscriber = Subscriber {
                store_id,
                repo_id: registration.repo_id,
                lifetime: registration.lifetime,
                original_path: registration.workdir,
                settings,
                backend: Arc::clone(backend),
                repository: registration.repository,
                sink: Arc::clone(&sink),
                needs_refresh: registration.needs_refresh,
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
        // Selecting a tab ends its repository's quiet backoff.
        let State {
            entries, groups, ..
        } = &mut *state;
        for entry in entries.values() {
            if entry.is_active()
                && let Some(group) = groups.get_mut(&entry.group)
            {
                group.schedule.quiet = 0;
            }
        }
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
            let State { groups, hosts, .. } = &mut *state;
            if let Some(group) = groups.get_mut(&key) {
                group.schedule = Schedule::default();
                group.recover(hosts);
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
                    subscriber.refresh();
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

    /// Holds every automatic fetch while a store's sign-in dialog is open.
    pub(super) fn set_sign_in_open(&self, store_id: u64, open: bool) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let changed = if open {
            state.signing_in.insert(store_id)
        } else {
            state.signing_in.remove(&store_id)
        };
        drop(state);
        if changed {
            self.inner.admission.changed.notify_all();
        }
    }

    pub(super) fn unsubscribe(&self, store_id: u64) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.signing_in.remove(&store_id);
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
                group.recover(hosts);
            }
            group.request = None;
            group.snapshot.status = status;
            group.revision = revision;
            state.publish(&group_key, None);
            if result.is_err() {
                state.mark_stale(&group_key);
            }
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

/// When git last fetched into the store `workdir` uses. Git rewrites
/// FETCH_HEAD on every fetch, including ones run outside GitComet, but only in
/// the worktree that ran it, so a store fetched through a sibling counts too.
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
    let common_dir = std::fs::read_to_string(git_dir.join("commondir"))
        .map_or_else(|_| git_dir.clone(), |common| git_dir.join(common.trim()));
    let linked = std::fs::read_dir(common_dir.join("worktrees"))
        .into_iter()
        .flatten()
        .filter_map(|entry| Some(entry.ok()?.path()));
    std::iter::once(common_dir)
        .chain(linked)
        .filter_map(|dir| {
            std::fs::metadata(dir.join("FETCH_HEAD"))
                .ok()?
                .modified()
                .ok()
        })
        .max()
}

pub(super) fn repository_key(repo: &dyn GitRepository) -> PathBuf {
    canonicalize_or_original(
        repo.common_dir()
            .unwrap_or_else(|| repo.spec().workdir.clone()),
    )
}

mod policy;
mod schedule;
mod worker;

use schedule::*;
use worker::*;

#[cfg(test)]
mod tests;
