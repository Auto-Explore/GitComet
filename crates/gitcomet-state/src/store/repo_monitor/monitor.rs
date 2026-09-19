//! Monitor orchestration: input reloads and coverage decisions meet at one flush.
use super::native_watcher::WATCH_MODE;
use super::*;

pub(super) const MAX_WORKTREE_WATCH_DIRS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WatchSetupOutcome {
    Watching { failed_dirs: usize },
    WorktreeSubdirsSkipped { dir_count: usize },
    PolicyFailed,
}

pub(super) struct MonitorConfig {
    pub debounce: Duration,
    pub max_delay: Duration,
    pub idle_tick: Duration,
    pub recovery_interval: Duration,
    pub setup_passes: usize,
    pub dir_limit: usize,
    pub before_registration: Option<Box<dyn FnMut() + Send>>,
    #[cfg(test)]
    pub native_events: Option<Arc<AtomicU64>>,
    #[cfg(all(test, target_os = "linux"))]
    pub native_barrier: Option<Arc<NativeEventBarrier>>,
}
impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(250),
            max_delay: Duration::from_secs(2),
            idle_tick: Duration::from_secs(30),
            recovery_interval: DEGRADED_WATCH_RECHECK_INTERVAL,
            setup_passes: 3,
            dir_limit: MAX_WORKTREE_WATCH_DIRS,
            before_registration: None,
            #[cfg(test)]
            native_events: None,
            #[cfg(all(test, target_os = "linux"))]
            native_barrier: None,
        }
    }
}

#[derive(Default)]
pub(super) struct MonitorState {
    pub inputs: WatchInputs,
    pub rules: IgnoreRules,
    pub policy: PolicyCell,
    pub plan: WatchPlan,
}
impl MonitorState {
    pub fn snapshot(&self) -> Arc<PolicySnapshot> {
        self.policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    pub fn publish(&self, snapshot: PolicySnapshot) {
        *self
            .policy
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Arc::new(snapshot);
    }
    pub fn reload(&mut self, workdir: &Path, backend: &dyn GitBackend, index_only: bool) -> bool {
        repo_load_trace::trace!(
            "repo_monitor_reload scope={} workdir={}",
            if index_only { "index" } else { "all" },
            workdir.display()
        );
        // Bracket failed discovery too. Capturing after an error can record a
        // replacement that arrived during the failed read as already loaded,
        // delaying its recovery until the throttled retry. An unchanged broken
        // policy still waits for that retry instead of reloading on every tick.
        self.inputs.stamps.recapture();
        self.inputs.indexes.recapture();
        match WatchInputs::load(workdir, backend) {
            Ok(mut inputs) => {
                // Retain discovered nested .gitignore inputs between index-only
                // reloads. Link targets are resolved again from those spellings.
                if index_only {
                    inputs.add_inputs(
                        self.inputs
                            .inputs
                            .iter()
                            .filter(|path| {
                                path.starts_with(workdir)
                                    && path.file_name().is_some_and(|name| name == ".gitignore")
                            })
                            .cloned()
                            .collect(),
                    );
                }
                let loaded = self.rules.reload(workdir, backend, &inputs.info);
                self.inputs = inputs;
                loaded
            }
            Err(error) => {
                self.rules.failed = true;
                record_monitor_failure(MonitorFailureKind::Start, "discover watch inputs", error);
                false
            }
        }
    }

    pub fn setup(
        &mut self,
        workdir: &Path,
        backend: &dyn GitBackend,
        repo_id: RepoId,
        tx: &mpsc::Sender<MonitorMsg>,
        enabled: &Arc<AtomicBool>,
        config: &mut MonitorConfig,
        mut reload: bool,
    ) -> Option<(MonitorWatcher, WatchSetupOutcome)> {
        for attempt in 0..config.setup_passes.max(1) {
            if reload {
                self.reload(workdir, backend, false);
            }
            self.plan = WatchPlan::default();
            self.publish(PolicySnapshot::new(workdir, &self.inputs));
            if let Some(hook) = &mut config.before_registration {
                hook();
            }
            // Linux registers each directory in the BFS. Windows establishes
            // recursive root coverage first. macOS needs the final boundaries
            // before creating its streams; recursive coverage includes any
            // directories created during that scan as well.
            #[cfg(not(target_os = "macos"))]
            let (mut watcher, failures) = match MonitorWatcher::new(
                repo_id,
                tx,
                enabled,
                &self.policy,
                &[],
                #[cfg(test)]
                config.native_events.clone(),
                #[cfg(all(test, target_os = "linux"))]
                config.native_barrier.clone(),
            ) {
                Ok(result) => result,
                Err(error) => {
                    record_monitor_failure(MonitorFailureKind::Start, "create watcher", error);
                    return None;
                }
            };
            #[cfg(not(target_os = "macos"))]
            {
                self.plan.failures += failures;
            }
            let snapshot = self.snapshot();
            let mut roots: Vec<_> = snapshot.git_roots.iter().cloned().collect();
            roots.sort();
            for (roots, worktree) in [(roots, false), (vec![workdir.to_path_buf()], true)] {
                self.plan.walk(
                    roots,
                    worktree,
                    &snapshot,
                    &mut self.rules,
                    &mut self.inputs,
                    config.dir_limit,
                    |path| {
                        #[cfg(not(target_os = "macos"))]
                        {
                            watcher.add(path)
                        }
                        #[cfg(target_os = "macos")]
                        {
                            let _ = path;
                            Ok(())
                        }
                    },
                );
            }
            let mut snapshot = PolicySnapshot::new(workdir, &self.inputs);
            snapshot
                .excluded_roots
                .extend(self.plan.boundaries.iter().cloned());
            self.publish(snapshot);
            #[cfg(target_os = "macos")]
            let watcher = match MonitorWatcher::new(
                repo_id,
                tx,
                enabled,
                &self.policy,
                &self.plan.boundaries,
                #[cfg(test)]
                config.native_events.clone(),
            ) {
                Ok((watcher, failures)) => {
                    self.plan.failures += failures;
                    watcher
                }
                Err(error) => {
                    record_monitor_failure(MonitorFailureKind::Start, "create watcher", error);
                    return None;
                }
            };
            if !self.inputs.stamps.changed() && !self.inputs.indexes.changed() {
                return Some((
                    watcher,
                    self.plan.outcome(&self.rules, &self.inputs, WATCH_MODE),
                ));
            }
            if attempt + 1 == config.setup_passes.max(1) {
                // Keep useful partial coverage and retry through ordinary input
                // revalidation. No synthetic event can create a retry loop.
                self.plan.failures += 1;
                return Some((
                    watcher,
                    self.plan.outcome(&self.rules, &self.inputs, WATCH_MODE),
                ));
            }
            drop(watcher);
            reload = true;
        }
        unreachable!()
    }

    pub(super) fn apply_directories(
        &mut self,
        effect: &EventEffect,
        watcher: &mut MonitorWatcher,
        config: &MonitorConfig,
    ) {
        let snapshot = self.snapshot();
        let previous_boundaries = self.plan.boundaries.len();
        for path in &effect.dir_removed {
            if !self.plan.dirs.contains(path) && !snapshot.excluded_roots.contains(path) {
                continue;
            }
            watcher.remove_tree(path);
            self.plan.dirs.retain(|dir| !dir.starts_with(path));
            self.plan.worktree_dirs.retain(|dir| !dir.starts_with(path));
            self.plan.boundaries.retain(|dir| !dir.starts_with(path));
        }
        self.plan
            .boundaries
            .extend(effect.new_ignored_dirs.iter().cloned());
        for (path, worktree) in &effect.dir_added {
            self.plan.walk(
                [path.clone()],
                *worktree,
                &snapshot,
                &mut self.rules,
                &mut self.inputs,
                config.dir_limit,
                |dir| watcher.add(dir),
            );
        }
        if self.plan.boundaries.len() != previous_boundaries
            || !effect.dir_added.is_empty()
            || !effect.new_ignored_dirs.is_empty()
        {
            let mut next = PolicySnapshot::new(&snapshot.workdir, &self.inputs);
            // Publish once per batch, deduplicating boundaries discovered both
            // by an event and by traversal of its newly added parent.
            self.plan
                .boundaries
                .retain(|dir| next.excluded_roots.insert(dir.clone()));
            self.publish(next);
        }
    }
}

#[derive(Default, Debug, Eq, PartialEq)]
pub(super) struct EventEffect {
    pub change: Option<RepoExternalChange>,
    pub index_dirty: bool,
    pub policy_dirty: bool,
    pub dir_added: Vec<(PathBuf, bool)>,
    pub dir_removed: Vec<PathBuf>,
    pub new_ignored_dirs: Vec<PathBuf>,
}

pub(super) fn summarize(
    snapshot: &PolicySnapshot,
    rules: &mut IgnoreRules,
    event: &notify::Event,
) -> EventEffect {
    let mut effect = EventEffect::default();
    if should_ignore_event_kind(event) {
        return effect;
    }
    if event.need_rescan() || event.paths.is_empty() {
        effect.change = Some(RepoExternalChange::all());
        return effect;
    }
    let mut change = RepoExternalChange {
        worktree: false,
        index: false,
        git_state: false,
        tags: false,
        verification_context: false,
    };
    let structural = structural_event(event);
    for path in &event.paths {
        let class = snapshot.classify(path);
        // One stat per path, taken lazily: only structural events describe an
        // entry whose kind matters, and every reader below must see the same
        // entry, not whatever replaced it between two calls.
        let mut lstat: Option<Option<fs::Metadata>> = None;
        if class == PathClass::Cache || snapshot.is_git_directory_modify(path, event) {
            continue;
        }
        // A regular file holds no inputs. Any input that vanished when it
        // replaced a directory changed its stamp, checked at the same flush;
        // FSEvents can repeat that replacement's flags on later edits.
        if structural
            && snapshot.input_below(path)
            && path != &snapshot.workdir
            && !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
        {
            effect.policy_dirty = true;
        }
        // A directory-only ignore rule need not match its replacement file.
        // Rebuild only when the boundary becomes eligible; ignored directory
        // deletion/recreation and events deeper inside it remain quiet.
        if class == PathClass::Excluded
            && structural
            && snapshot.excluded_roots.contains(path)
            && let Ok(relative) = path.strip_prefix(&snapshot.workdir)
        {
            match fs::symlink_metadata(path) {
                Ok(metadata) if !rules.is_ignored_rel(relative, Some(metadata.is_dir())) => {
                    effect.policy_dirty = true;
                    change.worktree = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    effect.dir_removed.push(path.clone());
                }
                _ => {} // A delayed removal must not unexclude a recreated directory.
            }
        }
        if matches!(class, PathClass::Excluded | PathClass::Outside) {
            continue;
        }
        // A removed watch must be pruned even if its directory became ignored
        // after the last tracked exception was removed from the index.
        if matches!(
            event.kind,
            notify::EventKind::Remove(_)
                | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
        ) {
            effect.dir_removed.push(path.clone());
        }
        match class {
            PathClass::Control => {
                effect.policy_dirty = true;
                change.worktree = true;
            }
            PathClass::ControlEntry => {
                effect.policy_dirty |= structural;
                change.git_state = true;
            }
            PathClass::Index => {
                effect.index_dirty = true;
                change.index = true;
            }
            PathClass::Git { tags } => {
                change.git_state = true;
                change.tags |= tags;
            }
            PathClass::Worktree => {
                let relative = path.strip_prefix(&snapshot.workdir).unwrap();
                if path.file_name().is_some_and(|name| name == ".gitignore") {
                    if !rules.is_ignored_rel(relative.parent().unwrap_or(Path::new("")), Some(true))
                    {
                        effect.policy_dirty = true;
                        change.worktree = true;
                    }
                    continue;
                }
                // A removal describes the old entry, not a directory that may
                // already have replaced a previously visible file at this path.
                // So can a rename source; FSEvents never says which side it was.
                // The ignore matcher treats an unknown kind like a file, so a
                // plain modification needs no stat: its hint is `None` either way.
                let metadata = if structural {
                    lstat
                        .get_or_insert_with(|| fs::symlink_metadata(path).ok())
                        .as_ref()
                } else {
                    None
                };
                let directory = structural
                    && !matches!(
                        event.kind,
                        notify::EventKind::Remove(_)
                            | notify::EventKind::Modify(notify::event::ModifyKind::Name(
                                notify::event::RenameMode::From
                                    | notify::event::RenameMode::Any
                                    | notify::event::RenameMode::Other
                            ))
                    )
                    && path_dir_hint(event) != Some(false)
                    && metadata.is_some_and(|metadata| metadata.is_dir());
                // Backends may type a new entry by following its link (Windows
                // reports a directory symlink as a created folder), but Git
                // sees the link itself, which a directory-only rule never hides.
                let dir_hint = if directory {
                    Some(true)
                } else if metadata.is_some_and(|metadata| metadata.file_type().is_symlink()) {
                    Some(false)
                } else {
                    path_dir_hint(event)
                };
                if rules.is_ignored_rel(relative, dir_hint) {
                    if directory {
                        effect.new_ignored_dirs.push(path.clone());
                    }
                    continue;
                }
                change.worktree = true;
            }
            _ => {}
        }
        if structural
            && path_dir_hint(event) != Some(false)
            && lstat
                .get_or_insert_with(|| fs::symlink_metadata(path).ok())
                .as_ref()
                .is_some_and(|metadata| metadata.is_dir())
        {
            effect
                .dir_added
                .push((path.clone(), class == PathClass::Worktree));
        }
    }
    effect.change = (!change.is_empty()).then_some(change);
    effect
}

pub(super) fn repo_monitor_thread(
    repo_id: RepoId,
    workdir: PathBuf,
    msg_tx: StoreWorkerSender,
    monitor_rx: mpsc::Receiver<MonitorMsg>,
    monitor_tx: mpsc::Sender<MonitorMsg>,
    active_repo_id: Arc<AtomicU64>,
    monitor_enabled: Arc<AtomicBool>,
    backend: Arc<dyn GitBackend>,
    mut config: MonitorConfig,
) {
    let workdir = super::super::canonicalize_path(workdir);
    if !monitor_enabled.load(Ordering::Relaxed) {
        return;
    }
    let mut state = MonitorState::default();
    let initial = state.setup(
        &workdir,
        &*backend,
        repo_id,
        &monitor_tx,
        &monitor_enabled,
        &mut config,
        true,
    );
    let (mut watcher, mut outcome) = match initial {
        Some((watcher, outcome)) => (Some(watcher), outcome),
        None => (None, WatchSetupOutcome::Watching { failed_dirs: 1 }),
    };
    let mut degraded = false;
    note_watch_outcome(&msg_tx, repo_id, &mut degraded, outcome);
    let mut last_recovery = degraded.then(Instant::now);
    let mut debouncer = DebouncedChange::new(config.debounce, config.max_delay);
    #[cfg(all(test, target_os = "linux"))]
    let mut drains = Vec::new();
    let mut policy_dirty = false;
    let mut index_dirty = false;
    let mut rebuild = None;
    let mut idle_at = Instant::now() + config.idle_tick;
    let flush = |change| {
        let active = active_repo_id.load(Ordering::Relaxed);
        if active == repo_id.0 {
            trace_repo_monitor_flush("flush", repo_id, change, active);
            msg_tx.send_repo_monitor_or_log(
                Msg::RepoExternallyChanged { repo_id, change },
                "repo monitor flush",
            );
        }
    };
    loop {
        let now = Instant::now();
        let timeout = debouncer
            .next_timeout(now)
            .unwrap_or(config.idle_tick)
            .min(idle_at.saturating_duration_since(now));
        let mut due_change = None;
        let mut revalidate = false;
        match monitor_rx.recv_timeout(timeout) {
            Ok(MonitorMsg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            #[cfg(test)]
            Ok(MonitorMsg::Barrier(tx)) => {
                let _ = tx.send(());
            }
            #[cfg(all(test, target_os = "linux"))]
            Ok(MonitorMsg::Drain(tx)) => drains.push(tx),
            Ok(MonitorMsg::Revalidate) => revalidate = true,
            Ok(MonitorMsg::Event(result)) => {
                if !monitor_enabled.load(Ordering::Relaxed) {
                    break;
                }
                let snapshot = state.snapshot();
                match result {
                    Ok(event) => match triage(&snapshot, &event) {
                        Triage::Drop => {}
                        Triage::Rescan => {
                            // Lost lifecycle events can invalidate exclusions
                            // even when recursive native roots remain intact.
                            rebuild = Some("rescan");
                            due_change = debouncer.push(RepoExternalChange::all(), Instant::now());
                        }
                        Triage::Relevant => {
                            let effect = summarize(&snapshot, &mut state.rules, &event);
                            policy_dirty |= effect.policy_dirty;
                            index_dirty |= effect.index_dirty;
                            if let Some(watcher) = &mut watcher {
                                if watcher.root_lost(&event, &snapshot) {
                                    rebuild = Some("root-lost");
                                }
                                state.apply_directories(&effect, watcher, &config);
                                // Incremental registration can lose coverage too.
                                // Report it once and let the usual throttled retry
                                // recover, without turning directory events into
                                // immediate full rebuilds.
                                if !state.rules.failed {
                                    let next =
                                        state.plan.outcome(&state.rules, &state.inputs, WATCH_MODE);
                                    if next != outcome {
                                        outcome = next;
                                        let was_degraded = degraded;
                                        note_watch_outcome(
                                            &msg_tx,
                                            repo_id,
                                            &mut degraded,
                                            outcome,
                                        );
                                        if degraded && !was_degraded {
                                            last_recovery = Some(Instant::now());
                                        }
                                    }
                                }
                            }
                            repo_load_trace::trace!(
                                "monitor_event repo_id={:?} kind={:?} paths={} first={:?} change={:?}",
                                repo_id,
                                event.kind,
                                event.paths.len(),
                                event.paths.first(),
                                effect.change
                            );
                            if let Some(change) = effect.change {
                                due_change = debouncer.push(change, Instant::now());
                            }
                        }
                    },
                    Err(error) => {
                        record_monitor_failure(
                            MonitorFailureKind::Start,
                            "native watcher event",
                            error,
                        );
                        rebuild = Some("native-error");
                        due_change = debouncer.push(RepoExternalChange::all(), Instant::now());
                    }
                }
                if state.rules.failed && outcome != WatchSetupOutcome::PolicyFailed {
                    outcome = WatchSetupOutcome::PolicyFailed;
                    note_watch_outcome(&msg_tx, repo_id, &mut degraded, outcome);
                    rebuild = Some("ignore-lookup");
                    due_change = debouncer
                        .push(RepoExternalChange::all(), Instant::now())
                        .or(due_change);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !monitor_enabled.load(Ordering::Relaxed) {
            break;
        }
        let now = Instant::now();
        due_change = due_change.or_else(|| debouncer.take_if_due(now));
        let idle = now >= idle_at;
        if idle {
            idle_at = now + config.idle_tick;
        }
        if due_change.is_some() || revalidate || idle {
            if state.inputs.stamps.changed() {
                policy_dirty = true;
            }
            if state.inputs.indexes.changed() {
                index_dirty = true;
            }
            if degraded && recovery_recheck_due(last_recovery, now, config.recovery_interval) {
                rebuild = Some("recovery");
            }
            let mut loaded = false;
            if policy_dirty {
                rebuild = Some("policy");
            } else if index_dirty && rebuild.is_none() {
                let old_info = state.inputs.info.clone();
                let succeeded = state.reload(&workdir, &*backend, true);
                loaded = true;
                if !succeeded
                    || old_info != state.inputs.info
                    || state.rules.any_boundary_unignored(&state.plan.boundaries)
                    || state.rules.failed
                {
                    rebuild = Some("index-coverage");
                }
                due_change = Some(merge_change(
                    due_change.unwrap_or(RepoExternalChange {
                        worktree: false,
                        index: false,
                        git_state: false,
                        tags: false,
                        verification_context: false,
                    }),
                    RepoExternalChange {
                        worktree: false,
                        index: true,
                        git_state: false,
                        tags: false,
                        verification_context: false,
                    },
                ));
            }
            if let Some(reason) = rebuild.take() {
                repo_load_trace::trace!(
                    "repo_monitor_rebuild reason={} repo_id={:?}",
                    reason,
                    repo_id
                );
                drop(watcher.take());
                last_recovery = Some(now);
                match state.setup(
                    &workdir,
                    &*backend,
                    repo_id,
                    &monitor_tx,
                    &monitor_enabled,
                    &mut config,
                    !loaded,
                ) {
                    Some((new, result)) => {
                        watcher = Some(new);
                        outcome = result;
                    }
                    None => outcome = WatchSetupOutcome::Watching { failed_dirs: 1 },
                }
                note_watch_outcome(&msg_tx, repo_id, &mut degraded, outcome);
                debouncer.take();
                due_change = Some(RepoExternalChange {
                    verification_context: policy_dirty,
                    ..RepoExternalChange::all()
                });
            }
            policy_dirty = false;
            index_dirty = false;
            if let Some(change) = due_change {
                flush(change);
            }
        }
        #[cfg(all(test, target_os = "linux"))]
        if !debouncer.is_pending() {
            for tx in drains.drain(..) {
                let _ = tx.send(());
            }
        }
    }
    monitor_enabled.store(false, Ordering::Relaxed);
}
