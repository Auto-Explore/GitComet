//! The automatic-fetch worker: picks the next due group, fetches it and
//! records the outcome.

use super::*;

pub(super) const RESUME_CHECK: Duration = Duration::from_secs(30);
/// Fetches to one host start at least this far apart, so many repositories on
/// one server never reach it back to back.
pub(super) const HOST_SPACING: Duration = Duration::from_secs(2);
pub(super) const SPACING_POLL: Duration = Duration::from_millis(50);

pub(super) struct Job {
    pub(super) group: GroupKey,
    pub(super) workdir: PathBuf,
    pub(super) request: u64,
    pub(super) started: SystemTime,
    pub(super) cancellation: CancellationToken,
    pub(super) subscriber: Subscriber,
    /// The group's other workdirs, each with one of its subscribers.
    pub(super) members: Vec<(PathBuf, Subscriber)>,
}

pub(super) enum Completion {
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
        /// Members whose fetch configuration now puts them in another group.
        regrouped: Vec<(PathBuf, GroupKey)>,
    },
}

/// The store a group's fetch would lock, when known.
pub(super) fn group_common_dir<'a>(key: &'a GroupKey, member: &'a Entry) -> Option<&'a PathBuf> {
    match key {
        GroupKey::Store { common_dir, .. } => Some(common_dir),
        GroupKey::Workdir(_) => member.common_dir.as_ref(),
    }
}

pub(super) fn next_job(state: &mut State, admission: &Admission, now: SystemTime) -> Option<Job> {
    if state.running.is_some() || !state.signing_in.is_empty() {
        return None;
    }
    let active: FxHashSet<&GroupKey> = state
        .entries
        .values()
        .filter(|entry| entry.is_active())
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
    let members = state
        .entries
        .iter()
        .filter(|(member, entry)| **member != workdir && entry.group == key)
        .filter_map(|(member, entry)| {
            Some((member.clone(), entry.subscribers.values().next()?.clone()))
        })
        .collect();
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
        members,
    })
}

fn open_repository(
    subscriber: &Subscriber,
    workdir: &Path,
    cancellation: &CancellationToken,
) -> Result<Arc<dyn GitRepository>> {
    cancellation.check_cancelled()?;
    subscriber.repository.clone().map_or_else(
        || subscriber.backend.open_cancellable(workdir, cancellation),
        Ok,
    )
}

/// The group whose fetch serves `repo`, the worktree at `workdir`.
fn fetch_group(repo: &dyn GitRepository, common_dir: &Path, workdir: &Path) -> GroupKey {
    repo.fetch_config_signature().map_or_else(
        |_| GroupKey::Workdir(workdir.to_path_buf()),
        |config| GroupKey::Store {
            common_dir: common_dir.to_path_buf(),
            config,
        },
    )
}

pub(super) fn execute(job: &Job, inner: &Inner) -> Completion {
    let repo = match open_repository(&job.subscriber, &job.workdir, &job.cancellation) {
        Ok(repo) => repo,
        Err(error) => {
            return Completion::Finished {
                common_dir: None,
                key: None,
                remotes: Vec::new(),
                skipped: Vec::new(),
                result: Err(error),
                refs_changed: false,
                regrouped: Vec::new(),
            };
        }
    };
    let common_dir = repository_key(repo.as_ref());
    let key = fetch_group(repo.as_ref(), &common_dir, &job.workdir);
    // Only remotes the fetch contacts can fail it or be charged for it.
    let fetched = repo.fetch_all_remote_names().ok();
    let remotes: Vec<FetchRemote> = repo
        .list_remotes(&job.cancellation)
        .unwrap_or_default()
        .into_iter()
        .filter(|remote| {
            fetched
                .as_ref()
                .is_none_or(|names| names.contains(&remote.name))
        })
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
            regrouped: Vec::new(),
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
        crate::store::effects::release_all_worktree_scan_handles();
    }
    // A worktree's own configuration can change after it joined the group.
    let regrouped = job
        .members
        .iter()
        .filter_map(|(workdir, member)| {
            let repo = open_repository(member, workdir, &job.cancellation).ok()?;
            let key = fetch_group(repo.as_ref(), &repository_key(repo.as_ref()), workdir);
            (key != job.group).then(|| (workdir.clone(), key))
        })
        .collect();
    Completion::Finished {
        common_dir: Some(common_dir),
        key: Some(key),
        remotes,
        skipped,
        result,
        refs_changed,
        regrouped,
    }
}

pub(super) fn sleep_unless_cancelled(
    duration: Duration,
    cancellation: &CancellationToken,
) -> Result<()> {
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

pub(super) fn finish(state: &mut State, job: &Job, completion: Completion, now: SystemTime) {
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
    let (common_dir, key, remotes, outcome, regrouped) = match completion {
        Completion::Deferred {
            common_dir,
            key,
            remotes,
        } => (Some(common_dir), Some(key), remotes, None, Vec::new()),
        Completion::Finished {
            common_dir,
            key,
            remotes,
            skipped,
            result,
            refs_changed,
            regrouped,
        } => (
            common_dir,
            key,
            remotes,
            Some((result, skipped, refs_changed)),
            regrouped,
        ),
    };
    let mut moved = Vec::new();
    if current {
        if let Some(entry) = state.entries.get_mut(&job.workdir) {
            entry.common_dir = common_dir.clone().or_else(|| entry.common_dir.clone());
        }
        if let Some(key) = key {
            state.regroup(&job.workdir, key);
        }
        // Before the schedule records this fetch, so a member that leaves
        // for a new group is due at once.
        for (workdir, key) in regrouped {
            if state
                .entries
                .get(&workdir)
                .is_some_and(|entry| entry.group == job.group)
            {
                state.regroup(&workdir, key.clone());
                moved.push(key);
            }
        }
    }
    let Some(group_key) = state
        .entries
        .get(&job.workdir)
        .map(|entry| entry.group.clone())
    else {
        return;
    };
    // Members that joined `group_key` get its update below.
    for key in moved.iter().filter(|key| **key != group_key) {
        if let Some(group) = state.groups.get_mut(key) {
            group.revision = revision;
            state.publish(key, None);
        }
    }
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
            // A workdir that joins its siblings' group reads the refs their
            // fetches wrote.
            let joined = group_key != job.group
                && state
                    .entries
                    .iter()
                    .any(|(workdir, entry)| *workdir != job.workdir && entry.group == group_key);
            if joined && let Some(entry) = state.entries.get_mut(&job.workdir) {
                entry.subscribers.values_mut().for_each(Subscriber::refresh);
            }
        }
        return;
    };
    let active = state.group_is_active(&group_key);
    let failed = result
        .as_ref()
        .is_err_and(|error| !matches!(error.kind(), ErrorKind::Cancelled));
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
            Ok(_) => {
                group.reached(hosts, &skipped);
                group.schedule.quiet = if refs_changed || active {
                    0
                } else {
                    group.schedule.quiet.saturating_add(1)
                };
            }
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
    if current && failed {
        state.mark_stale(&group_key);
    }
}

/// Refreshes every tab reading the fetched refs: the fetching workdir, its
/// group and anything else on the same store. Tabs with a current count skip
/// the refresh when the fetch moved nothing.
pub(super) fn send_refreshes(
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
            if refs_changed || subscriber.needs_refresh {
                subscriber.refresh();
            }
        }
    }
}

pub(super) fn run(inner: Arc<Inner>) {
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
                        regrouped: Vec::new(),
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

pub(super) fn wait_duration(
    state: &State,
    admission: &Admission,
    now: SystemTime,
) -> Option<Duration> {
    let enabled = state.enabled_groups();
    if enabled.is_empty() || !state.signing_in.is_empty() {
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
