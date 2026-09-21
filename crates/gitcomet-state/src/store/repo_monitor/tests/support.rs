//! Small fixtures around the production policy, planner and setup entry points.
use super::*;

#[derive(Default)]
pub(super) struct FaultyBackend {
    pub load_failure: Arc<AtomicBool>,
    pub lookup_failure: Arc<AtomicBool>,
    pub reloads: Arc<AtomicU64>,
}
struct FaultyMatcher {
    inner: Box<dyn WorktreeIgnoreMatcher>,
    unavailable: Arc<AtomicBool>,
}
impl WorktreeIgnoreMatcher for FaultyMatcher {
    fn is_ignored(
        &mut self,
        path: &Path,
        kind: WorktreePathKind,
    ) -> gitcomet_core::services::Result<bool> {
        if path == Path::new("source/file.txt") && self.unavailable.load(Ordering::Relaxed) {
            return Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("injected per-path ignore failure".into()),
            ));
        }
        self.inner.is_ignored(path, kind)
    }
}
impl GitBackend for FaultyBackend {
    fn open(
        &self,
        root: &Path,
    ) -> gitcomet_core::services::Result<Arc<dyn gitcomet_core::services::GitRepository>> {
        gitcomet_git_gix::GixBackend.open(root)
    }
    fn repository_watch_info(
        &self,
        root: &Path,
    ) -> gitcomet_core::services::Result<Option<gitcomet_core::services::RepositoryWatchInfo>> {
        gitcomet_git_gix::GixBackend.repository_watch_info(root)
    }
    fn worktree_ignore_matcher(
        &self,
        root: &Path,
    ) -> gitcomet_core::services::Result<Option<Box<dyn WorktreeIgnoreMatcher>>> {
        self.reloads.fetch_add(1, Ordering::Relaxed);
        if self.load_failure.load(Ordering::Relaxed) {
            return Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend(
                    "injected temporary policy failure".into(),
                ),
            ));
        }
        Ok(gitcomet_git_gix::GixBackend
            .worktree_ignore_matcher(root)?
            .map(|inner| {
                Box::new(FaultyMatcher {
                    inner,
                    unavailable: self.lookup_failure.clone(),
                }) as Box<dyn WorktreeIgnoreMatcher>
            }))
    }
}

pub(super) struct TestRules {
    pub state: MonitorState,
    backend: Arc<dyn GitBackend>,
    pub config: MonitorConfig,
}
impl Default for TestRules {
    fn default() -> Self {
        Self {
            state: MonitorState::default(),
            backend: Arc::new(gitcomet_git_gix::GixBackend),
            config: MonitorConfig::default(),
        }
    }
}
impl std::ops::Deref for TestRules {
    type Target = IgnoreRules;
    fn deref(&self) -> &IgnoreRules {
        &self.state.rules
    }
}
impl std::ops::DerefMut for TestRules {
    fn deref_mut(&mut self) -> &mut IgnoreRules {
        &mut self.state.rules
    }
}
impl TestRules {
    pub fn load(root: &Path, backend: Arc<dyn GitBackend>) -> Self {
        let mut result = Self {
            backend,
            ..Default::default()
        };
        result.reload(root);
        result
    }
    pub fn reload(&mut self, root: &Path) {
        if self.state.reload(root, &*self.backend, false) {
            self.state
                .publish(PolicySnapshot::new(root, &self.state.inputs));
        }
    }

    pub fn start_watcher(
        &mut self,
        root: &Path,
    ) -> (
        MonitorWatcher,
        WatchSetupOutcome,
        mpsc::Receiver<MonitorMsg>,
    ) {
        let (tx, rx) = mpsc::channel();
        let (watcher, outcome) = self
            .state
            .setup(
                root,
                &*self.backend,
                RepoId(1),
                &tx,
                &Arc::new(AtomicBool::new(true)),
                &mut self.config,
                false,
            )
            .expect("watcher setup must succeed");
        (watcher, outcome, rx)
    }
}

pub(super) struct TestPlan {
    pub policy: PolicySnapshot,
    pub dirs: FxHashSet<PathBuf>,
    pub worktree_dirs: FxHashSet<PathBuf>,
    pub skipped: Option<usize>,
}
impl TestPlan {
    pub fn build(root: &Path, git: Option<&Path>, rules: &mut TestRules) -> Self {
        Self::build_with_limit(root, git, rules, MAX_WORKTREE_WATCH_DIRS)
    }
    pub fn build_with_limit(
        root: &Path,
        _git: Option<&Path>,
        rules: &mut TestRules,
        limit: usize,
    ) -> Self {
        let mut plan = WatchPlan::default();
        let policy = PolicySnapshot::new(root, &rules.state.inputs);
        let roots = policy.git_roots.iter().cloned().collect::<Vec<_>>();
        for (roots, worktree) in [(roots, false), (vec![root.to_path_buf()], true)] {
            plan.walk(
                roots,
                worktree,
                &policy,
                &mut rules.state.rules,
                &mut rules.state.inputs,
                limit,
                |_| Ok(()),
            );
        }
        let mut policy = PolicySnapshot::new(root, &rules.state.inputs);
        policy.excluded_roots.extend(plan.boundaries);
        Self {
            policy,
            dirs: plan.dirs,
            worktree_dirs: plan.worktree_dirs,
            skipped: plan.skipped,
        }
    }
}

// These assertions exercise the new classifier; no production predicates or
// alternate policy implementation are retained for the old tests.
impl PolicySnapshot {
    pub fn is_cache(&self, path: &Path) -> bool {
        self.classify(path) == PathClass::Cache
    }
    pub fn relevant(&self, path: &Path) -> bool {
        !matches!(
            self.classify(path),
            PathClass::Cache | PathClass::Excluded | PathClass::Outside
        )
    }
}

pub(super) fn summarize_event(
    root: &Path,
    git: Option<&Path>,
    rules: &mut TestRules,
    event: &notify::Event,
) -> EventEffect {
    let snapshot = if rules.state.snapshot().workdir.as_os_str().is_empty() {
        let mut inputs = WatchInputs::default();
        if let Some(git) = git {
            inputs.info.git_dirs.push(git.to_path_buf());
            inputs.index_file = Some(normalized(git).join("index"));
            inputs.add_inputs(vec![
                git.join("config"),
                git.join("info/exclude"),
                root.join(".gitignore"),
            ]);
        }
        let mut policy = PolicySnapshot::new(root, &inputs);
        if let Some(git) = git {
            policy.cache_roots.extend([
                git.join("objects"),
                git.join("lfs"),
                git.join("index.lock"),
            ]);
        }
        Arc::new(policy)
    } else {
        rules.state.snapshot()
    };
    let mut event = event.clone();
    event
        .paths
        .iter_mut()
        .for_each(|path| *path = normalized(path));
    summarize(&snapshot, &mut rules.state.rules, &event)
}
