use super::*;

pub enum FsEventScenario {
    /// Single file save → git status → status diff.
    SingleFileSave { tracked_files: usize },
    /// Simulate `git checkout` changing many files at once → status.
    GitCheckoutBatch {
        tracked_files: usize,
        checkout_files: usize,
    },
    /// Rapidly dirty N files → single coalesced status call (debounce model).
    RapidSavesDebounceCoalesce {
        tracked_files: usize,
        save_count: usize,
    },
    /// Dirty N files then revert → status should find 0 dirty (false positive).
    FalsePositiveUnderChurn {
        tracked_files: usize,
        churn_files: usize,
    },
}

#[cfg(any(test, feature = "benchmarks"))]
pub struct FsEventFixture {
    _repo_root: TempDir,
    repo: Arc<dyn GitRepository>,
    repo_path: std::path::PathBuf,
    scenario: FsEventScenario,
    tracked_files: usize,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FsEventMetrics {
    pub tracked_files: u64,
    pub mutation_files: u64,
    pub dirty_files_detected: u64,
    pub status_entries_total: u64,
    pub false_positives: u64,
    pub coalesced_saves: u64,
    pub status_calls: u64,
    pub status_ms: f64,
}

#[cfg(any(test, feature = "benchmarks"))]
impl FsEventFixture {
    pub fn single_file_save(tracked_files: usize) -> Self {
        let tracked_files = tracked_files.max(10);
        let repo_root = build_git_ops_status_repo(tracked_files, 0);
        let repo_path = repo_root.path().to_path_buf();
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open fs_event single_file_save benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            repo_path,
            scenario: FsEventScenario::SingleFileSave { tracked_files },
            tracked_files,
        }
    }

    pub fn git_checkout_batch(tracked_files: usize, checkout_files: usize) -> Self {
        let tracked_files = tracked_files.max(10);
        let checkout_files = checkout_files.min(tracked_files).max(1);
        let repo_root = build_git_ops_status_repo(tracked_files, 0);
        let repo_path = repo_root.path().to_path_buf();
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open fs_event git_checkout_batch benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            repo_path,
            scenario: FsEventScenario::GitCheckoutBatch {
                tracked_files,
                checkout_files,
            },
            tracked_files,
        }
    }

    pub fn rapid_saves_debounce(tracked_files: usize, save_count: usize) -> Self {
        let tracked_files = tracked_files.max(10);
        let save_count = save_count.min(tracked_files).max(1);
        let repo_root = build_git_ops_status_repo(tracked_files, 0);
        let repo_path = repo_root.path().to_path_buf();
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open fs_event rapid_saves_debounce benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            repo_path,
            scenario: FsEventScenario::RapidSavesDebounceCoalesce {
                tracked_files,
                save_count,
            },
            tracked_files,
        }
    }

    pub fn false_positive_under_churn(tracked_files: usize, churn_files: usize) -> Self {
        let tracked_files = tracked_files.max(10);
        let churn_files = churn_files.min(tracked_files).max(1);
        let repo_root = build_git_ops_status_repo(tracked_files, 0);
        let repo_path = repo_root.path().to_path_buf();
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open fs_event false_positive_under_churn benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            repo_path,
            scenario: FsEventScenario::FalsePositiveUnderChurn {
                tracked_files,
                churn_files,
            },
            tracked_files,
        }
    }

    pub fn run(&self) -> u64 {
        self.execute().0
    }

    pub fn run_with_metrics(&self) -> (u64, FsEventMetrics) {
        self.execute()
    }

    fn execute(&self) -> (u64, FsEventMetrics) {
        let mut metrics = FsEventMetrics::default();
        metrics.tracked_files = u64::try_from(self.tracked_files).unwrap_or(u64::MAX);

        match &self.scenario {
            FsEventScenario::SingleFileSave { .. } => {
                // 1. Mutate one file (simulates save).
                let target = git_ops_status_relative_path(0);
                let full_path = self.repo_path.join(&target);
                let original = fs::read(&full_path).expect("read original file");
                fs::write(&full_path, b"fs-event-mutation\n").expect("write fs_event dirty file");
                metrics.mutation_files = 1;

                // 2. Run git status.
                let start = std::time::Instant::now();
                let status = self
                    .repo
                    .status()
                    .expect("fs_event single_file_save status");
                metrics.status_ms = start.elapsed().as_secs_f64() * 1_000.0;
                metrics.status_calls = 1;

                let dirty = status.staged.len().saturating_add(status.unstaged.len());
                metrics.dirty_files_detected = u64::try_from(dirty).unwrap_or(u64::MAX);
                metrics.status_entries_total = metrics.dirty_files_detected;

                let hash = hash_repo_status(&status);

                // 3. Restore.
                fs::write(&full_path, &original).expect("restore original file");

                (hash, metrics)
            }
            FsEventScenario::GitCheckoutBatch { checkout_files, .. } => {
                let checkout_files = *checkout_files;

                // 1. Mutate checkout_files files.
                let mut originals = Vec::with_capacity(checkout_files);
                for index in 0..checkout_files {
                    let target = git_ops_status_relative_path(index);
                    let full_path = self.repo_path.join(&target);
                    originals.push((
                        full_path.clone(),
                        fs::read(&full_path).expect("read original"),
                    ));
                    fs::write(&full_path, format!("checkout-mutation-{index:05}\n"))
                        .expect("write fs_event checkout file");
                }
                metrics.mutation_files = u64::try_from(checkout_files).unwrap_or(u64::MAX);

                // 2. Run git status.
                let start = std::time::Instant::now();
                let status = self
                    .repo
                    .status()
                    .expect("fs_event git_checkout_batch status");
                metrics.status_ms = start.elapsed().as_secs_f64() * 1_000.0;
                metrics.status_calls = 1;

                let dirty = status.staged.len().saturating_add(status.unstaged.len());
                metrics.dirty_files_detected = u64::try_from(dirty).unwrap_or(u64::MAX);
                metrics.status_entries_total = metrics.dirty_files_detected;

                let hash = hash_repo_status(&status);

                // 3. Restore all files.
                for (path, original) in &originals {
                    fs::write(path, original).expect("restore checkout file");
                }

                (hash, metrics)
            }
            FsEventScenario::RapidSavesDebounceCoalesce { save_count, .. } => {
                let save_count = *save_count;

                // 1. Rapidly dirty save_count files (simulating rapid saves before debounce fires).
                let mut originals = Vec::with_capacity(save_count);
                for index in 0..save_count {
                    let target = git_ops_status_relative_path(index);
                    let full_path = self.repo_path.join(&target);
                    originals.push((
                        full_path.clone(),
                        fs::read(&full_path).expect("read original"),
                    ));
                    fs::write(&full_path, format!("rapid-save-{index:05}\n"))
                        .expect("write fs_event rapid save file");
                }
                metrics.mutation_files = u64::try_from(save_count).unwrap_or(u64::MAX);
                metrics.coalesced_saves = metrics.mutation_files;

                // 2. Single coalesced status call (debounce model).
                let start = std::time::Instant::now();
                let status = self
                    .repo
                    .status()
                    .expect("fs_event rapid_saves_debounce status");
                metrics.status_ms = start.elapsed().as_secs_f64() * 1_000.0;
                metrics.status_calls = 1;

                let dirty = status.staged.len().saturating_add(status.unstaged.len());
                metrics.dirty_files_detected = u64::try_from(dirty).unwrap_or(u64::MAX);
                metrics.status_entries_total = metrics.dirty_files_detected;

                let hash = hash_repo_status(&status);

                // 3. Restore.
                for (path, original) in &originals {
                    fs::write(path, original).expect("restore rapid save file");
                }

                (hash, metrics)
            }
            FsEventScenario::FalsePositiveUnderChurn { churn_files, .. } => {
                let churn_files = *churn_files;

                // 1. Dirty churn_files files.
                let mut originals = Vec::with_capacity(churn_files);
                for index in 0..churn_files {
                    let target = git_ops_status_relative_path(index);
                    let full_path = self.repo_path.join(&target);
                    let original = fs::read(&full_path).expect("read original");
                    originals.push((full_path.clone(), original));
                    fs::write(&full_path, format!("churn-{index:05}\n"))
                        .expect("write fs_event churn file");
                }

                // 2. Revert all files to original content (simulating churn that settles).
                for (path, original) in &originals {
                    fs::write(path, original).expect("revert churn file");
                }
                metrics.mutation_files = u64::try_from(churn_files).unwrap_or(u64::MAX);

                // 3. Status should find 0 dirty files — the FS events were false positives.
                let start = std::time::Instant::now();
                let status = self
                    .repo
                    .status()
                    .expect("fs_event false_positive_under_churn status");
                metrics.status_ms = start.elapsed().as_secs_f64() * 1_000.0;
                metrics.status_calls = 1;

                let dirty = status.staged.len().saturating_add(status.unstaged.len());
                metrics.dirty_files_detected = u64::try_from(dirty).unwrap_or(u64::MAX);
                metrics.status_entries_total = metrics.dirty_files_detected;
                // Every churn file triggered an FS event but resulted in 0 dirty files.
                metrics.false_positives = if dirty == 0 {
                    metrics.mutation_files
                } else {
                    0
                };

                let hash = hash_repo_status(&status);
                (hash, metrics)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// IdleResourceFixture — long-running idle CPU/RSS sampling harness
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdleResourceScenario {
    CpuUsageSingleRepo60s,
    CpuUsageTenRepos60s,
    MemoryGrowthSingleRepo10Min,
    MemoryGrowthTenRepos10Min,
    BackgroundRefreshCostPerCycle,
    WakeFromSleepResume,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdleResourceConfig {
    pub repo_count: usize,
    pub tracked_files_per_repo: usize,
    pub sample_window: Duration,
    pub sample_interval: Duration,
    pub refresh_cycles: usize,
    pub wake_gap: Duration,
}

#[cfg(any(test, feature = "benchmarks"))]
impl IdleResourceConfig {
    pub fn cpu_usage_single_repo() -> Self {
        Self {
            repo_count: 1,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::from_secs(60),
            sample_interval: Duration::from_secs(1),
            refresh_cycles: 0,
            wake_gap: Duration::ZERO,
        }
    }

    pub fn cpu_usage_ten_repos() -> Self {
        Self {
            repo_count: 10,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::from_secs(60),
            sample_interval: Duration::from_secs(1),
            refresh_cycles: 0,
            wake_gap: Duration::ZERO,
        }
    }

    pub fn memory_growth_single_repo() -> Self {
        Self {
            repo_count: 1,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::from_secs(600),
            sample_interval: Duration::from_secs(1),
            refresh_cycles: 0,
            wake_gap: Duration::ZERO,
        }
    }

    pub fn memory_growth_ten_repos() -> Self {
        Self {
            repo_count: 10,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::from_secs(600),
            sample_interval: Duration::from_secs(1),
            refresh_cycles: 0,
            wake_gap: Duration::ZERO,
        }
    }

    pub fn background_refresh_cost_per_cycle() -> Self {
        Self {
            repo_count: 10,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::ZERO,
            sample_interval: Duration::from_millis(250),
            refresh_cycles: 10,
            wake_gap: Duration::ZERO,
        }
    }

    pub fn wake_from_sleep_resume() -> Self {
        Self {
            repo_count: 10,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::ZERO,
            sample_interval: Duration::ZERO,
            refresh_cycles: 1,
            wake_gap: Duration::from_secs(1),
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
pub struct IdleResourceFixture {
    _repo_roots: Vec<TempDir>,
    repos: Vec<Arc<dyn GitRepository>>,
    scenario: IdleResourceScenario,
    config: IdleResourceConfig,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IdleResourceMetrics {
    pub open_repos: u64,
    pub tracked_files_per_repo: u64,
    pub sample_duration_ms: f64,
    pub sample_count: u64,
    pub avg_cpu_pct: f64,
    pub peak_cpu_pct: f64,
    pub rss_start_kib: u64,
    pub rss_end_kib: u64,
    pub rss_delta_kib: i64,
    pub refresh_cycles: u64,
    pub repos_refreshed: u64,
    pub status_calls: u64,
    pub status_ms: f64,
    pub avg_refresh_cycle_ms: f64,
    pub max_refresh_cycle_ms: f64,
    pub wake_resume_ms: f64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
struct IdleSampleSummary {
    sample_duration_ms: f64,
    sample_count: u64,
    avg_cpu_pct: f64,
    peak_cpu_pct: f64,
    rss_start_kib: u64,
    rss_end_kib: u64,
    rss_delta_kib: i64,
}

#[cfg(any(test, feature = "benchmarks"))]
struct IdleSampler {
    started_at: Instant,
    last_at: Instant,
    start_cpu_runtime_ns: Option<u64>,
    last_cpu_runtime_ns: Option<u64>,
    peak_cpu_pct: f64,
    sample_count: u64,
    rss_start_kib: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
impl IdleSampler {
    fn start() -> Self {
        let now = Instant::now();
        let cpu_runtime_ns = current_cpu_runtime_ns();
        Self {
            started_at: now,
            last_at: now,
            start_cpu_runtime_ns: cpu_runtime_ns,
            last_cpu_runtime_ns: cpu_runtime_ns,
            peak_cpu_pct: 0.0,
            sample_count: 0,
            rss_start_kib: current_rss_kib().unwrap_or(0),
        }
    }

    fn sample(&mut self) {
        let now = Instant::now();
        if let (Some(previous_cpu_ns), Some(current_cpu_ns)) =
            (self.last_cpu_runtime_ns, current_cpu_runtime_ns())
        {
            let elapsed_wall_ns = now.duration_since(self.last_at).as_nanos() as f64;
            if elapsed_wall_ns > 0.0 {
                let cpu_pct =
                    current_cpu_ns.saturating_sub(previous_cpu_ns) as f64 / elapsed_wall_ns * 100.0;
                self.peak_cpu_pct = self.peak_cpu_pct.max(cpu_pct);
            }
            self.last_cpu_runtime_ns = Some(current_cpu_ns);
        }
        self.last_at = now;
        self.sample_count = self.sample_count.saturating_add(1);
    }

    fn finish(self) -> IdleSampleSummary {
        let finished_at = Instant::now();
        let elapsed_ns = finished_at.duration_since(self.started_at).as_nanos() as f64;
        let avg_cpu_pct = match (self.start_cpu_runtime_ns, current_cpu_runtime_ns()) {
            (Some(start_cpu_ns), Some(end_cpu_ns)) if elapsed_ns > 0.0 => {
                end_cpu_ns.saturating_sub(start_cpu_ns) as f64 / elapsed_ns * 100.0
            }
            _ => 0.0,
        };
        let rss_end_kib = current_rss_kib().unwrap_or(self.rss_start_kib);

        IdleSampleSummary {
            sample_duration_ms: elapsed_ns / 1_000_000.0,
            sample_count: self.sample_count,
            avg_cpu_pct,
            peak_cpu_pct: self.peak_cpu_pct,
            rss_start_kib: self.rss_start_kib,
            rss_end_kib,
            rss_delta_kib: i64::try_from(rss_end_kib).unwrap_or(i64::MAX)
                - i64::try_from(self.rss_start_kib).unwrap_or(i64::MAX),
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
impl IdleResourceFixture {
    pub fn cpu_usage_single_repo_60s() -> Self {
        Self::build(
            IdleResourceScenario::CpuUsageSingleRepo60s,
            IdleResourceConfig::cpu_usage_single_repo(),
        )
    }

    pub fn cpu_usage_ten_repos_60s() -> Self {
        Self::build(
            IdleResourceScenario::CpuUsageTenRepos60s,
            IdleResourceConfig::cpu_usage_ten_repos(),
        )
    }

    pub fn memory_growth_single_repo_10min() -> Self {
        Self::build(
            IdleResourceScenario::MemoryGrowthSingleRepo10Min,
            IdleResourceConfig::memory_growth_single_repo(),
        )
    }

    pub fn memory_growth_ten_repos_10min() -> Self {
        Self::build(
            IdleResourceScenario::MemoryGrowthTenRepos10Min,
            IdleResourceConfig::memory_growth_ten_repos(),
        )
    }

    pub fn background_refresh_cost_per_cycle() -> Self {
        Self::build(
            IdleResourceScenario::BackgroundRefreshCostPerCycle,
            IdleResourceConfig::background_refresh_cost_per_cycle(),
        )
    }

    pub fn wake_from_sleep_resume() -> Self {
        Self::build(
            IdleResourceScenario::WakeFromSleepResume,
            IdleResourceConfig::wake_from_sleep_resume(),
        )
    }

    pub fn with_config(scenario: IdleResourceScenario, config: IdleResourceConfig) -> Self {
        Self::build(scenario, config)
    }

    fn build(scenario: IdleResourceScenario, mut config: IdleResourceConfig) -> Self {
        config.repo_count = config.repo_count.max(1);
        config.tracked_files_per_repo = config.tracked_files_per_repo.max(1);
        config.sample_interval = if config.sample_interval.is_zero() {
            Duration::from_millis(1)
        } else {
            config.sample_interval
        };
        if matches!(
            scenario,
            IdleResourceScenario::BackgroundRefreshCostPerCycle
                | IdleResourceScenario::WakeFromSleepResume
        ) {
            config.refresh_cycles = config.refresh_cycles.max(1);
        }

        let mut repo_roots = Vec::with_capacity(config.repo_count);
        let mut repos = Vec::with_capacity(config.repo_count);
        let backend = GixBackend;
        for _ in 0..config.repo_count {
            let repo_root = build_git_ops_status_repo(config.tracked_files_per_repo, 0);
            let repo = backend
                .open(repo_root.path())
                .expect("open idle_resource benchmark repo");
            repo_roots.push(repo_root);
            repos.push(repo);
        }

        Self {
            _repo_roots: repo_roots,
            repos,
            scenario,
            config,
        }
    }

    pub fn run(&self) -> u64 {
        self.execute().0
    }

    pub fn run_with_metrics(&self) -> (u64, IdleResourceMetrics) {
        self.execute()
    }

    fn execute(&self) -> (u64, IdleResourceMetrics) {
        let mut metrics = IdleResourceMetrics {
            open_repos: u64::try_from(self.repos.len()).unwrap_or(u64::MAX),
            tracked_files_per_repo: u64::try_from(self.config.tracked_files_per_repo)
                .unwrap_or(u64::MAX),
            ..IdleResourceMetrics::default()
        };
        let mut work_hash = 0u64;

        let sample_summary = match self.scenario {
            IdleResourceScenario::CpuUsageSingleRepo60s
            | IdleResourceScenario::CpuUsageTenRepos60s
            | IdleResourceScenario::MemoryGrowthSingleRepo10Min
            | IdleResourceScenario::MemoryGrowthTenRepos10Min => {
                self.measure_passive_window(self.config.sample_window, self.config.sample_interval)
            }
            IdleResourceScenario::BackgroundRefreshCostPerCycle => {
                let mut sampler = IdleSampler::start();
                let mut total_cycle_ms = 0.0f64;
                let mut max_cycle_ms = 0.0f64;
                for cycle_index in 0..self.config.refresh_cycles {
                    let cycle_started = Instant::now();
                    let (cycle_hash, status_calls, status_ms) = self.refresh_all_repos();
                    work_hash ^= cycle_hash;
                    metrics.status_calls = metrics.status_calls.saturating_add(status_calls);
                    metrics.status_ms += status_ms;
                    let cycle_ms = cycle_started.elapsed().as_secs_f64() * 1_000.0;
                    total_cycle_ms += cycle_ms;
                    max_cycle_ms = max_cycle_ms.max(cycle_ms);
                    metrics.refresh_cycles = metrics.refresh_cycles.saturating_add(1);
                    metrics.repos_refreshed = metrics
                        .repos_refreshed
                        .saturating_add(u64::try_from(self.repos.len()).unwrap_or(u64::MAX));
                    sampler.sample();
                    if cycle_index + 1 < self.config.refresh_cycles
                        && !self.config.sample_interval.is_zero()
                    {
                        std::thread::sleep(self.config.sample_interval);
                    }
                }
                metrics.avg_refresh_cycle_ms = total_cycle_ms / self.config.refresh_cycles as f64;
                metrics.max_refresh_cycle_ms = max_cycle_ms;
                sampler.finish()
            }
            IdleResourceScenario::WakeFromSleepResume => {
                if !self.config.wake_gap.is_zero() {
                    std::thread::sleep(self.config.wake_gap);
                }
                let mut sampler = IdleSampler::start();
                let cycle_started = Instant::now();
                let (cycle_hash, status_calls, status_ms) = self.refresh_all_repos();
                work_hash = cycle_hash;
                metrics.status_calls = status_calls;
                metrics.status_ms = status_ms;
                metrics.wake_resume_ms = cycle_started.elapsed().as_secs_f64() * 1_000.0;
                metrics.refresh_cycles = 1;
                metrics.repos_refreshed = u64::try_from(self.repos.len()).unwrap_or(u64::MAX);
                metrics.avg_refresh_cycle_ms = metrics.wake_resume_ms;
                metrics.max_refresh_cycle_ms = metrics.wake_resume_ms;
                sampler.sample();
                sampler.finish()
            }
        };

        metrics.sample_duration_ms = sample_summary.sample_duration_ms;
        metrics.sample_count = sample_summary.sample_count;
        metrics.avg_cpu_pct = sample_summary.avg_cpu_pct;
        metrics.peak_cpu_pct = sample_summary.peak_cpu_pct;
        metrics.rss_start_kib = sample_summary.rss_start_kib;
        metrics.rss_end_kib = sample_summary.rss_end_kib;
        metrics.rss_delta_kib = sample_summary.rss_delta_kib;

        let mut h = FxHasher::default();
        std::mem::discriminant(&self.scenario).hash(&mut h);
        metrics.open_repos.hash(&mut h);
        metrics.tracked_files_per_repo.hash(&mut h);
        metrics.sample_count.hash(&mut h);
        metrics.refresh_cycles.hash(&mut h);
        metrics.repos_refreshed.hash(&mut h);
        metrics.status_calls.hash(&mut h);
        work_hash.hash(&mut h);
        (h.finish(), metrics)
    }

    fn measure_passive_window(&self, window: Duration, interval: Duration) -> IdleSampleSummary {
        let mut sampler = IdleSampler::start();
        let steps = idle_sample_steps(window, interval);
        let mut remaining = window;

        for step in 0..steps {
            let sleep_for = if step + 1 == steps {
                remaining
            } else {
                remaining.min(interval)
            };
            if !sleep_for.is_zero() {
                std::thread::sleep(sleep_for);
                remaining = remaining.saturating_sub(sleep_for);
            }
            sampler.sample();
        }

        sampler.finish()
    }

    fn refresh_all_repos(&self) -> (u64, u64, f64) {
        let mut h = FxHasher::default();
        let mut status_calls = 0u64;
        let mut status_ms = 0.0f64;
        for repo in &self.repos {
            let started_at = Instant::now();
            let status = repo.status().expect("idle_resource repo refresh status");
            status_ms += started_at.elapsed().as_secs_f64() * 1_000.0;
            status_calls = status_calls.saturating_add(1);
            hash_repo_status(&status).hash(&mut h);
        }
        (h.finish(), status_calls, status_ms)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn idle_sample_steps(window: Duration, interval: Duration) -> usize {
    if window.is_zero() {
        return 1;
    }

    let interval_nanos = interval.as_nanos().max(1);
    let window_nanos = window.as_nanos();
    let steps = window_nanos.saturating_add(interval_nanos.saturating_sub(1)) / interval_nanos;
    usize::try_from(steps.max(1)).unwrap_or(usize::MAX)
}

#[cfg(target_os = "linux")]
#[cfg(any(test, feature = "benchmarks"))]
fn current_cpu_runtime_ns() -> Option<u64> {
    let schedstat = fs::read_to_string("/proc/self/schedstat").ok()?;
    let runtime_ns = schedstat.split_whitespace().next()?;
    runtime_ns.parse::<u64>().ok()
}

#[cfg(not(target_os = "linux"))]
#[cfg(any(test, feature = "benchmarks"))]
fn current_cpu_runtime_ns() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
#[cfg(any(test, feature = "benchmarks"))]
fn current_rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?.split_whitespace().next()?;
        value.parse::<u64>().ok()
    })
}

#[cfg(not(target_os = "linux"))]
#[cfg(any(test, feature = "benchmarks"))]
fn current_rss_kib() -> Option<u64> {
    None
}

// ---------------------------------------------------------------------------
// Clipboard — copy from diff, paste into commit message, selection range
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardScenario {
    /// Extract 10k diff lines into a single clipboard string.
    CopyFromDiff,
    /// Insert a large block of text into an empty commit-message TextModel.
    PasteIntoCommitMessage,
    /// Compute the extracted text across a 5k-line selection range in a diff.
    SelectRangeInDiff,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClipboardMetrics {
    pub total_lines: u64,
    pub total_bytes: u64,
    pub line_iterations: u64,
    pub allocations_approx: u64,
}

pub struct ClipboardFixture {
    /// Pre-built diff lines for copy/select scenarios, or None for paste.
    diff_lines: Option<Vec<DiffLine>>,
    /// Pre-generated large text for the paste scenario.
    paste_text: Option<String>,
    scenario: ClipboardScenario,
    /// Number of lines to select (for SelectRangeInDiff).
    select_range_lines: usize,
}

impl ClipboardFixture {
    /// Copy 10k lines from an inline diff view — measures the string extraction
    /// cost that `selected_diff_text_string()` would pay before writing to the
    /// system clipboard.
    pub fn copy_from_diff(line_count: usize) -> Self {
        let diff_lines = build_synthetic_diff_lines(line_count.max(1));
        Self {
            diff_lines: Some(diff_lines),
            paste_text: None,
            scenario: ClipboardScenario::CopyFromDiff,
            select_range_lines: 0,
        }
    }

    /// Paste a large block of text into an empty commit-message `TextModel`.
    /// Measures the cost of `TextModel::replace_range` with a large insertion.
    pub fn paste_into_commit_message(line_count: usize, line_bytes: usize) -> Self {
        let lines = build_synthetic_source_lines(line_count.max(1), line_bytes.max(32));
        let text = lines.join("\n");
        Self {
            diff_lines: None,
            paste_text: Some(text),
            scenario: ClipboardScenario::PasteIntoCommitMessage,
            select_range_lines: 0,
        }
    }

    /// Select a range of 5k lines in a diff — measures the iteration and text
    /// extraction cost of building the selection string.
    pub fn select_range_in_diff(total_lines: usize, select_lines: usize) -> Self {
        let total = total_lines.max(1);
        let select = select_lines.min(total).max(1);
        let diff_lines = build_synthetic_diff_lines(total);
        Self {
            diff_lines: Some(diff_lines),
            paste_text: None,
            scenario: ClipboardScenario::SelectRangeInDiff,
            select_range_lines: select,
        }
    }

    pub fn run(&self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&self) -> (u64, ClipboardMetrics) {
        match self.scenario {
            ClipboardScenario::CopyFromDiff => self.run_copy(),
            ClipboardScenario::PasteIntoCommitMessage => self.run_paste(),
            ClipboardScenario::SelectRangeInDiff => self.run_select(),
        }
    }

    /// Simulates `selected_diff_text_string()` — iterates all diff lines,
    /// extracts the text content, and concatenates into a clipboard string.
    fn run_copy(&self) -> (u64, ClipboardMetrics) {
        let lines = self.diff_lines.as_ref().expect("copy needs diff_lines");
        let mut h = FxHasher::default();
        let mut out = String::new();
        let mut line_iterations = 0u64;
        let mut allocations_approx = 0u64;

        for line in lines.iter() {
            line_iterations += 1;
            // Skip header/hunk lines (like the real copy path does — header
            // lines appear in the gutter but their text is not part of the
            // user-visible selection).
            match line.kind {
                DiffLineKind::Header | DiffLineKind::Hunk => continue,
                _ => {}
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&line.text);
            allocations_approx += 1;
        }

        out.len().hash(&mut h);
        out.as_bytes().first().copied().unwrap_or(0).hash(&mut h);
        out.as_bytes().last().copied().unwrap_or(0).hash(&mut h);

        let metrics = ClipboardMetrics {
            total_lines: lines.len() as u64,
            total_bytes: out.len() as u64,
            line_iterations,
            allocations_approx,
        };

        (h.finish(), metrics)
    }

    /// Simulates pasting a large text block into the commit message editor.
    fn run_paste(&self) -> (u64, ClipboardMetrics) {
        let text = self.paste_text.as_ref().expect("paste needs paste_text");
        let mut h = FxHasher::default();

        // Create a fresh TextModel and insert the paste text at position 0.
        let mut model = TextModel::new();
        let inserted = model.replace_range(0..0, text);

        model.len().hash(&mut h);
        inserted.start.hash(&mut h);
        inserted.end.hash(&mut h);
        model.revision().hash(&mut h);

        let line_count = text.lines().count() as u64;
        let metrics = ClipboardMetrics {
            total_lines: line_count,
            total_bytes: text.len() as u64,
            line_iterations: 1, // single bulk insertion
            allocations_approx: 1,
        };

        (h.finish(), metrics)
    }

    /// Simulates computing a selection range across `select_range_lines` diff
    /// lines and extracting the text — the same work that happens when the user
    /// shift-clicks to extend a selection.
    fn run_select(&self) -> (u64, ClipboardMetrics) {
        let lines = self.diff_lines.as_ref().expect("select needs diff_lines");
        let end = self.select_range_lines.min(lines.len());
        let mut h = FxHasher::default();
        let mut out = String::new();
        let mut line_iterations = 0u64;
        let mut allocations_approx = 0u64;

        for line in lines[..end].iter() {
            line_iterations += 1;
            match line.kind {
                DiffLineKind::Header | DiffLineKind::Hunk => continue,
                _ => {}
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&line.text);
            allocations_approx += 1;
        }

        out.len().hash(&mut h);
        out.as_bytes().first().copied().unwrap_or(0).hash(&mut h);
        out.as_bytes().last().copied().unwrap_or(0).hash(&mut h);

        let metrics = ClipboardMetrics {
            total_lines: lines.len() as u64,
            total_bytes: out.len() as u64,
            line_iterations,
            allocations_approx,
        };

        (h.finish(), metrics)
    }
}

// ---------------------------------------------------------------------------
// Network-adjacent operations — mocked transport progress and cancellation
// ---------------------------------------------------------------------------

/// Synthetic network benchmark scenarios.
///
/// GitComet currently only exposes structured long-running progress state for
/// clone operations, so these fixtures reuse the real clone-progress reducer
/// path (`Msg::CloneRepo` + `InternalMsg::CloneRepoProgress`) while modeling a
/// fetch-style remote operation on top of it.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkScenario {
    UiResponsivenessDuringFetch,
    ProgressBarUpdateRenderCost,
    CancelOperationLatency,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NetworkMetrics {
    pub total_frames: u64,
    pub scroll_frames: u64,
    pub progress_updates: u64,
    pub render_passes: u64,
    pub output_tail_lines: u64,
    pub tail_trim_events: u64,
    pub rendered_bytes: u64,
    pub total_rows: u64,
    pub window_rows: u64,
    pub bar_width: u64,
    pub cancel_frames_until_stopped: u64,
    pub drained_updates_after_cancel: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Debug)]
struct MockNetworkProgressSnapshot {
    seq: u64,
    objects_done: u64,
    objects_total: u64,
    bytes_done: u64,
    bytes_total: u64,
    progress_line: String,
}

#[cfg(any(test, feature = "benchmarks"))]
enum MockNetworkEvent {
    Progress(MockNetworkProgressSnapshot),
    Finished,
    Cancelled,
}

#[cfg(any(test, feature = "benchmarks"))]
struct MockNetworkTransport<'a> {
    snapshots: &'a [MockNetworkProgressSnapshot],
    cursor: usize,
    cancel_drain_events_remaining: Option<usize>,
    terminal_emitted: bool,
}

#[cfg(any(test, feature = "benchmarks"))]
impl<'a> MockNetworkTransport<'a> {
    fn new(snapshots: &'a [MockNetworkProgressSnapshot]) -> Self {
        Self {
            snapshots,
            cursor: 0,
            cancel_drain_events_remaining: None,
            terminal_emitted: false,
        }
    }

    fn request_cancel(&mut self, drain_events: usize) {
        self.cancel_drain_events_remaining = Some(drain_events);
    }

    fn next_event(&mut self) -> Option<MockNetworkEvent> {
        if self.terminal_emitted {
            return None;
        }

        if let Some(remaining) = self.cancel_drain_events_remaining.as_mut() {
            if *remaining == 0 {
                self.terminal_emitted = true;
                return Some(MockNetworkEvent::Cancelled);
            }
            *remaining = remaining.saturating_sub(1);
        }

        if let Some(snapshot) = self.snapshots.get(self.cursor).cloned() {
            self.cursor = self.cursor.saturating_add(1);
            return Some(MockNetworkEvent::Progress(snapshot));
        }

        self.terminal_emitted = true;
        Some(match self.cancel_drain_events_remaining {
            Some(_) => MockNetworkEvent::Cancelled,
            None => MockNetworkEvent::Finished,
        })
    }
}

pub struct NetworkFixture {
    baseline: AppState,
    transport_url: String,
    transport_dest: std::path::PathBuf,
    snapshots: Vec<MockNetworkProgressSnapshot>,
    history_fixture: Option<HistoryListScrollFixture>,
    scenario: NetworkScenario,
    window_rows: usize,
    scroll_step_rows: usize,
    bar_width: usize,
    frame_budget_ns: u64,
    cancel_after_updates: usize,
    cancel_drain_events: usize,
}

impl NetworkFixture {
    pub fn ui_responsiveness_during_fetch(
        history_commits: usize,
        local_branches: usize,
        remote_branches: usize,
        window_rows: usize,
        scroll_step_rows: usize,
        frames: usize,
        line_bytes: usize,
        bar_width: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let transport_url = "https://example.invalid/gitcomet/network.git".to_string();
        let transport_dest = std::path::PathBuf::from("/tmp/gitcomet-network-ui-responsiveness");
        let baseline = build_network_baseline_state(&transport_url, &transport_dest);
        let total_frames = frames.max(1);

        Self {
            baseline,
            transport_url,
            transport_dest,
            snapshots: build_mock_network_progress_snapshots(total_frames, line_bytes.max(48)),
            history_fixture: Some(HistoryListScrollFixture::new(
                history_commits,
                local_branches,
                remote_branches,
            )),
            scenario: NetworkScenario::UiResponsivenessDuringFetch,
            window_rows: window_rows.max(1),
            scroll_step_rows: scroll_step_rows.max(1),
            bar_width: bar_width.max(8),
            frame_budget_ns: frame_budget_ns.max(1),
            cancel_after_updates: 0,
            cancel_drain_events: 0,
        }
    }

    pub fn progress_bar_update_render_cost(
        updates: usize,
        line_bytes: usize,
        bar_width: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let transport_url = "https://example.invalid/gitcomet/network.git".to_string();
        let transport_dest = std::path::PathBuf::from("/tmp/gitcomet-network-progress-bar");
        let baseline = build_network_baseline_state(&transport_url, &transport_dest);
        let total_frames = updates.max(1);

        Self {
            baseline,
            transport_url,
            transport_dest,
            snapshots: build_mock_network_progress_snapshots(total_frames, line_bytes.max(48)),
            history_fixture: None,
            scenario: NetworkScenario::ProgressBarUpdateRenderCost,
            window_rows: 0,
            scroll_step_rows: 0,
            bar_width: bar_width.max(8),
            frame_budget_ns: frame_budget_ns.max(1),
            cancel_after_updates: 0,
            cancel_drain_events: 0,
        }
    }

    pub fn cancel_operation_latency(
        cancel_after_updates: usize,
        cancel_drain_events: usize,
        total_updates: usize,
        line_bytes: usize,
        bar_width: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let transport_url = "https://example.invalid/gitcomet/network.git".to_string();
        let transport_dest = std::path::PathBuf::from("/tmp/gitcomet-network-cancel");
        let baseline = build_network_baseline_state(&transport_url, &transport_dest);
        let cancel_after_updates = cancel_after_updates.max(1);
        let total_frames = total_updates.max(
            cancel_after_updates
                .saturating_add(cancel_drain_events)
                .saturating_add(1),
        );

        Self {
            baseline,
            transport_url,
            transport_dest,
            snapshots: build_mock_network_progress_snapshots(total_frames, line_bytes.max(48)),
            history_fixture: None,
            scenario: NetworkScenario::CancelOperationLatency,
            window_rows: 0,
            scroll_step_rows: 0,
            bar_width: bar_width.max(8),
            frame_budget_ns: frame_budget_ns.max(1),
            cancel_after_updates,
            cancel_drain_events,
        }
    }

    fn fresh_state(&self) -> AppState {
        self.baseline.clone()
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, crate::view::perf::FrameTimingStats, NetworkMetrics) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, NetworkMetrics) {
        let mut state = self.fresh_state();
        let mut transport = MockNetworkTransport::new(&self.snapshots);
        let mut hash = 0u64;
        let mut metrics = NetworkMetrics {
            bar_width: bench_counter_u64(self.bar_width),
            ..NetworkMetrics::default()
        };

        match self.scenario {
            NetworkScenario::UiResponsivenessDuringFetch => {
                let history_fixture = self
                    .history_fixture
                    .as_ref()
                    .expect("ui responsiveness needs history fixture");
                let total_rows = history_fixture.total_rows();
                let window_rows = self.window_rows.min(total_rows.max(1));
                let max_start = total_rows.saturating_sub(window_rows);
                let mut start = 0usize;

                metrics.total_rows = bench_counter_u64(total_rows);
                metrics.window_rows = bench_counter_u64(window_rows);

                while let Some(event) = transport.next_event() {
                    let MockNetworkEvent::Progress(snapshot) = event else {
                        break;
                    };

                    let frame_started = Instant::now();
                    apply_mock_network_progress(&mut state, &self.transport_dest, &snapshot);
                    let clone_state = state.clone.as_ref().expect("clone progress state");
                    let (render_hash, rendered_bytes) = render_mock_network_progress(
                        &self.transport_url,
                        &self.transport_dest,
                        &clone_state.output_tail,
                        &snapshot,
                        self.bar_width,
                        None,
                    );
                    hash ^= render_hash;
                    hash ^= history_fixture.run_scroll_step(start, window_rows);

                    if max_start > 0 {
                        start = start.saturating_add(self.scroll_step_rows);
                        if start > max_start {
                            start %= max_start + 1;
                        }
                    }

                    metrics.total_frames = metrics.total_frames.saturating_add(1);
                    metrics.scroll_frames = metrics.scroll_frames.saturating_add(1);
                    metrics.progress_updates = metrics.progress_updates.saturating_add(1);
                    metrics.render_passes = metrics.render_passes.saturating_add(1);
                    metrics.rendered_bytes =
                        metrics.rendered_bytes.saturating_add(rendered_bytes as u64);

                    if let Some(capture) = capture.as_deref_mut() {
                        capture.record_frame(frame_started.elapsed());
                    }
                }
            }
            NetworkScenario::ProgressBarUpdateRenderCost => {
                while let Some(event) = transport.next_event() {
                    let MockNetworkEvent::Progress(snapshot) = event else {
                        break;
                    };

                    let frame_started = Instant::now();
                    apply_mock_network_progress(&mut state, &self.transport_dest, &snapshot);
                    let clone_state = state.clone.as_ref().expect("clone progress state");
                    let (render_hash, rendered_bytes) = render_mock_network_progress(
                        &self.transport_url,
                        &self.transport_dest,
                        &clone_state.output_tail,
                        &snapshot,
                        self.bar_width,
                        None,
                    );
                    hash ^= render_hash;

                    metrics.total_frames = metrics.total_frames.saturating_add(1);
                    metrics.progress_updates = metrics.progress_updates.saturating_add(1);
                    metrics.render_passes = metrics.render_passes.saturating_add(1);
                    metrics.rendered_bytes =
                        metrics.rendered_bytes.saturating_add(rendered_bytes as u64);

                    if let Some(capture) = capture.as_deref_mut() {
                        capture.record_frame(frame_started.elapsed());
                    }
                }
            }
            NetworkScenario::CancelOperationLatency => {
                let mut cancel_requested = false;
                let mut last_snapshot = self
                    .snapshots
                    .first()
                    .cloned()
                    .expect("network snapshots should not be empty");

                while let Some(event) = transport.next_event() {
                    let frame_started = Instant::now();
                    match event {
                        MockNetworkEvent::Progress(snapshot) => {
                            apply_mock_network_progress(
                                &mut state,
                                &self.transport_dest,
                                &snapshot,
                            );
                            let clone_state = state.clone.as_ref().expect("clone progress state");
                            let (render_hash, rendered_bytes) = render_mock_network_progress(
                                &self.transport_url,
                                &self.transport_dest,
                                &clone_state.output_tail,
                                &snapshot,
                                self.bar_width,
                                None,
                            );
                            hash ^= render_hash;

                            metrics.total_frames = metrics.total_frames.saturating_add(1);
                            metrics.progress_updates = metrics.progress_updates.saturating_add(1);
                            metrics.render_passes = metrics.render_passes.saturating_add(1);
                            metrics.rendered_bytes =
                                metrics.rendered_bytes.saturating_add(rendered_bytes as u64);

                            if cancel_requested {
                                metrics.cancel_frames_until_stopped =
                                    metrics.cancel_frames_until_stopped.saturating_add(1);
                                metrics.drained_updates_after_cancel =
                                    metrics.drained_updates_after_cancel.saturating_add(1);
                            }

                            last_snapshot = snapshot;
                            if !cancel_requested
                                && metrics.progress_updates
                                    >= bench_counter_u64(self.cancel_after_updates)
                            {
                                transport.request_cancel(self.cancel_drain_events);
                                cancel_requested = true;
                            }
                        }
                        MockNetworkEvent::Cancelled => {
                            let clone_state = state.clone.as_ref().expect("clone progress state");
                            let (render_hash, rendered_bytes) = render_mock_network_progress(
                                &self.transport_url,
                                &self.transport_dest,
                                &clone_state.output_tail,
                                &last_snapshot,
                                self.bar_width,
                                Some("Fetch cancelled"),
                            );
                            hash ^= render_hash;
                            metrics.total_frames = metrics.total_frames.saturating_add(1);
                            metrics.render_passes = metrics.render_passes.saturating_add(1);
                            metrics.rendered_bytes =
                                metrics.rendered_bytes.saturating_add(rendered_bytes as u64);
                            if cancel_requested {
                                metrics.cancel_frames_until_stopped =
                                    metrics.cancel_frames_until_stopped.saturating_add(1);
                            }

                            if let Some(capture) = capture.as_deref_mut() {
                                capture.record_frame(frame_started.elapsed());
                            }
                            break;
                        }
                        MockNetworkEvent::Finished => break,
                    }

                    if let Some(capture) = capture.as_deref_mut() {
                        capture.record_frame(frame_started.elapsed());
                    }
                }
            }
        }

        if let Some(clone_state) = state.clone.as_ref() {
            metrics.output_tail_lines = bench_counter_u64(clone_state.output_tail.len());
            metrics.tail_trim_events = metrics
                .progress_updates
                .saturating_sub(metrics.output_tail_lines);
        }

        hash ^= metrics.total_frames;
        hash ^= metrics.progress_updates;
        hash ^= metrics.render_passes;
        hash ^= metrics.rendered_bytes;
        hash ^= metrics.cancel_frames_until_stopped;

        (hash, metrics)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn build_network_baseline_state(url: &str, dest: &Path) -> AppState {
    let mut state = AppState::default();
    let _ = dispatch_sync(
        &mut state,
        Msg::CloneRepo {
            url: url.to_string(),
            dest: dest.to_path_buf(),
        },
    );
    state
}

#[cfg(any(test, feature = "benchmarks"))]
fn build_mock_network_progress_snapshots(
    updates: usize,
    line_bytes: usize,
) -> Vec<MockNetworkProgressSnapshot> {
    let updates = updates.max(1);
    let line_bytes = line_bytes.max(48);
    let objects_total = u64::try_from(updates.saturating_mul(24)).unwrap_or(u64::MAX);
    let bytes_total =
        u64::try_from(updates.saturating_mul(line_bytes).saturating_mul(64)).unwrap_or(u64::MAX);
    let mut snapshots = Vec::with_capacity(updates);

    for ix in 0..updates {
        let progress_ix = ix.saturating_add(1);
        let objects_done = u64::try_from(progress_ix.saturating_mul(24)).unwrap_or(u64::MAX);
        let bytes_done = u64::try_from(progress_ix.saturating_mul(line_bytes).saturating_mul(64))
            .unwrap_or(u64::MAX);
        let percent = ((progress_ix.saturating_mul(100)) / updates).min(100);
        let phase = match ix % 3 {
            0 => "remote: Counting objects",
            1 => "Receiving objects",
            _ => "Resolving deltas",
        };
        let mut progress_line = format!(
            "{phase}: {percent:>3}% ({objects_done}/{objects_total}) bytes={bytes_done}/{bytes_total}"
        );
        if progress_line.len() < line_bytes {
            progress_line.push(' ');
            progress_line.push_str("//");
            while progress_line.len() < line_bytes {
                let _ = write!(progress_line, " net_token_{}", ix % 97);
            }
        }

        snapshots.push(MockNetworkProgressSnapshot {
            seq: u64::try_from(progress_ix).unwrap_or(u64::MAX),
            objects_done,
            objects_total,
            bytes_done,
            bytes_total,
            progress_line,
        });
    }

    snapshots
}

#[cfg(any(test, feature = "benchmarks"))]
fn apply_mock_network_progress(
    state: &mut AppState,
    dest: &Path,
    snapshot: &MockNetworkProgressSnapshot,
) {
    let _ = dispatch_sync(
        state,
        Msg::Internal(InternalMsg::CloneRepoProgress {
            dest: dest.to_path_buf(),
            line: snapshot.progress_line.clone(),
        }),
    );
}

#[cfg(any(test, feature = "benchmarks"))]
fn render_mock_network_progress(
    url: &str,
    dest: &Path,
    output_tail: &[String],
    snapshot: &MockNetworkProgressSnapshot,
    bar_width: usize,
    title: Option<&str>,
) -> (u64, usize) {
    let bar_width = bar_width.max(8);
    let fill = usize::try_from(
        ((snapshot.bytes_done.saturating_mul(bar_width as u64)) / snapshot.bytes_total.max(1))
            .min(bar_width as u64),
    )
    .unwrap_or(bar_width);
    let empty = bar_width.saturating_sub(fill);
    let percent =
        ((snapshot.bytes_done.saturating_mul(100)) / snapshot.bytes_total.max(1)).min(100);

    let mut message = String::new();
    message.push_str(title.unwrap_or("Fetching repository..."));
    message.push('\n');
    message.push_str(url);
    message.push('\n');
    message.push_str("-> ");
    let _ = write!(message, "{}", dest.display());
    let _ = write!(
        message,
        "\n[{}{}] {percent:>3}% {}/{} objects | {} / {} KiB",
        "#".repeat(fill),
        "-".repeat(empty),
        snapshot.objects_done,
        snapshot.objects_total,
        snapshot.bytes_done / 1024,
        snapshot.bytes_total / 1024
    );

    if !output_tail.is_empty() {
        message.push_str("\n\n");
        let visible_start = output_tail.len().saturating_sub(12);
        for (ix, line) in output_tail.iter().skip(visible_start).enumerate() {
            if ix > 0 {
                message.push('\n');
            }
            message.push_str(line);
        }
    }

    let mut h = FxHasher::default();
    message.hash(&mut h);
    snapshot.seq.hash(&mut h);
    (h.finish(), message.len())
}

// ---------------------------------------------------------------------------
// display — render cost at different scale factors, multi-window, DPI switch
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayScenario {
    RenderCostByScale,
    TwoWindowsSameRepo,
    WindowMoveBetweenDpis,
}

/// Sidecar metrics for display benchmark scenarios.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DisplayMetrics {
    pub scale_factors_tested: u64,
    pub total_layout_passes: u64,
    pub total_rows_rendered: u64,
    pub history_rows_per_pass: u64,
    pub diff_rows_per_pass: u64,
    pub layout_width_min_px: f64,
    pub layout_width_max_px: f64,
    pub windows_rendered: u64,
    pub re_layout_passes: u64,
}

pub struct DisplayFixture {
    scenario: DisplayScenario,
    history: HistoryListScrollFixture,
    diff: FileDiffOpenFixture,
    history_window_rows: usize,
    diff_window_rows: usize,
    /// Scale factors to test (e.g. [1, 2, 3] for 1x/2x/3x).
    scale_factors: Vec<u32>,
    /// Base window width at 1x in logical pixels.
    base_window_width: f32,
    /// Sidebar and details widths (for layout computation).
    sidebar_w: f32,
    details_w: f32,
}

impl DisplayFixture {
    /// `render_cost_1x_vs_2x_vs_3x_scale`: measure rendering cost at three
    /// DPI scale factors. At higher scales, the effective physical viewport is
    /// wider, so more columns of text and wider layout computations are needed.
    /// We model this by running layout at `base_width * scale` and rendering
    /// history + diff rows for a visible window at each scale.
    pub fn render_cost_by_scale(
        history_commits: usize,
        local_branches: usize,
        remote_branches: usize,
        diff_lines: usize,
        history_window_rows: usize,
        diff_window_rows: usize,
        base_window_width: f32,
        sidebar_w: f32,
        details_w: f32,
    ) -> Self {
        Self {
            scenario: DisplayScenario::RenderCostByScale,
            history: HistoryListScrollFixture::new(
                history_commits.max(1),
                local_branches,
                remote_branches,
            ),
            diff: FileDiffOpenFixture::new(diff_lines.max(10)),
            history_window_rows: history_window_rows.max(1),
            diff_window_rows: diff_window_rows.max(1),
            scale_factors: vec![1, 2, 3],
            base_window_width: base_window_width.max(400.0),
            sidebar_w,
            details_w,
        }
    }

    /// `two_windows_same_repo`: render two viewports from the same repo state
    /// (one history, one diff) concurrently, testing cache sharing cost.
    pub fn two_windows_same_repo(
        history_commits: usize,
        local_branches: usize,
        remote_branches: usize,
        diff_lines: usize,
        history_window_rows: usize,
        diff_window_rows: usize,
        base_window_width: f32,
        sidebar_w: f32,
        details_w: f32,
    ) -> Self {
        Self {
            scenario: DisplayScenario::TwoWindowsSameRepo,
            history: HistoryListScrollFixture::new(
                history_commits.max(1),
                local_branches,
                remote_branches,
            ),
            diff: FileDiffOpenFixture::new(diff_lines.max(10)),
            history_window_rows: history_window_rows.max(1),
            diff_window_rows: diff_window_rows.max(1),
            scale_factors: vec![1],
            base_window_width: base_window_width.max(400.0),
            sidebar_w,
            details_w,
        }
    }

    /// `window_move_between_dpis`: render at 1x, then re-render at 2x to
    /// simulate moving a window from a standard monitor to a HiDPI monitor.
    pub fn window_move_between_dpis(
        history_commits: usize,
        local_branches: usize,
        remote_branches: usize,
        diff_lines: usize,
        history_window_rows: usize,
        diff_window_rows: usize,
        base_window_width: f32,
        sidebar_w: f32,
        details_w: f32,
    ) -> Self {
        Self {
            scenario: DisplayScenario::WindowMoveBetweenDpis,
            history: HistoryListScrollFixture::new(
                history_commits.max(1),
                local_branches,
                remote_branches,
            ),
            diff: FileDiffOpenFixture::new(diff_lines.max(10)),
            history_window_rows: history_window_rows.max(1),
            diff_window_rows: diff_window_rows.max(1),
            scale_factors: vec![1, 2],
            base_window_width: base_window_width.max(400.0),
            sidebar_w,
            details_w,
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal().0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, DisplayMetrics) {
        self.run_internal()
    }

    fn run_internal(&self) -> (u64, DisplayMetrics) {
        use crate::view::panes::main::pane_content_width_for_layout;

        let mut hash = 0u64;
        let total_history = self.history.total_rows();
        let history_window = self.history_window_rows.min(total_history.max(1));
        let mut metrics = DisplayMetrics {
            history_rows_per_pass: bench_counter_u64(history_window),
            diff_rows_per_pass: bench_counter_u64(self.diff_window_rows),
            ..DisplayMetrics::default()
        };
        let mut min_width: f32 = f32::MAX;
        let mut max_width: f32 = f32::MIN;

        match self.scenario {
            DisplayScenario::RenderCostByScale => {
                // Render history + diff at each scale factor.
                for &scale in &self.scale_factors {
                    let effective_width = self.base_window_width * scale as f32;
                    let main_w = pane_content_width_for_layout(
                        px(effective_width),
                        px(self.sidebar_w * scale as f32),
                        px(self.details_w * scale as f32),
                        false,
                        false,
                    );
                    let main_f: f32 = main_w.into();
                    hash ^= main_f.to_bits() as u64;
                    if main_f < min_width {
                        min_width = main_f;
                    }
                    if main_f > max_width {
                        max_width = main_f;
                    }
                    metrics.total_layout_passes += 1;

                    // Render history window from the middle.
                    let history_start = total_history.saturating_sub(history_window) / 2;
                    hash ^= self.history.run_scroll_step(history_start, history_window);
                    metrics.total_rows_rendered += history_window as u64;

                    // Render diff window.
                    hash ^= self.diff.run_split_first_window(self.diff_window_rows);
                    metrics.total_rows_rendered += self.diff_window_rows as u64;

                    metrics.windows_rendered += 1;
                }
                metrics.scale_factors_tested = self.scale_factors.len() as u64;
            }
            DisplayScenario::TwoWindowsSameRepo => {
                // Two concurrent viewports at the same scale.
                let effective_width = self.base_window_width;
                let main_w = pane_content_width_for_layout(
                    px(effective_width),
                    px(self.sidebar_w),
                    px(self.details_w),
                    false,
                    false,
                );
                let main_f: f32 = main_w.into();
                hash ^= main_f.to_bits() as u64;
                min_width = main_f;
                max_width = main_f;
                metrics.total_layout_passes += 1;

                // Window 1: history from top.
                let history_start_1 = 0;
                hash ^= self
                    .history
                    .run_scroll_step(history_start_1, history_window);
                metrics.total_rows_rendered += history_window as u64;
                metrics.windows_rendered += 1;

                // Window 1: diff.
                hash ^= self.diff.run_split_first_window(self.diff_window_rows);
                metrics.total_rows_rendered += self.diff_window_rows as u64;

                // Window 2: history from bottom.
                let history_start_2 = total_history.saturating_sub(history_window);
                hash ^= self
                    .history
                    .run_scroll_step(history_start_2, history_window);
                metrics.total_rows_rendered += history_window as u64;
                metrics.windows_rendered += 1;

                // Window 2: diff (inline view instead of split).
                hash ^= self.diff.run_inline_first_window(self.diff_window_rows);
                metrics.total_rows_rendered += self.diff_window_rows as u64;

                metrics.scale_factors_tested = 1;
            }
            DisplayScenario::WindowMoveBetweenDpis => {
                // Initial render at 1x.
                let scale_1x = self.scale_factors.first().copied().unwrap_or(1);
                let width_1x = self.base_window_width * scale_1x as f32;
                let main_1x = pane_content_width_for_layout(
                    px(width_1x),
                    px(self.sidebar_w * scale_1x as f32),
                    px(self.details_w * scale_1x as f32),
                    false,
                    false,
                );
                let main_1x_f: f32 = main_1x.into();
                hash ^= main_1x_f.to_bits() as u64;
                min_width = main_1x_f;
                max_width = main_1x_f;
                metrics.total_layout_passes += 1;

                let history_start = total_history.saturating_sub(history_window) / 2;
                hash ^= self.history.run_scroll_step(history_start, history_window);
                metrics.total_rows_rendered += history_window as u64;
                hash ^= self.diff.run_split_first_window(self.diff_window_rows);
                metrics.total_rows_rendered += self.diff_window_rows as u64;
                metrics.windows_rendered += 1;

                // Re-render at higher DPI (simulates monitor move).
                let scale_hi = self.scale_factors.last().copied().unwrap_or(2);
                let width_hi = self.base_window_width * scale_hi as f32;
                let main_hi = pane_content_width_for_layout(
                    px(width_hi),
                    px(self.sidebar_w * scale_hi as f32),
                    px(self.details_w * scale_hi as f32),
                    false,
                    false,
                );
                let main_hi_f: f32 = main_hi.into();
                hash ^= main_hi_f.to_bits() as u64;
                if main_hi_f < min_width {
                    min_width = main_hi_f;
                }
                if main_hi_f > max_width {
                    max_width = main_hi_f;
                }
                metrics.re_layout_passes += 1;
                metrics.total_layout_passes += 1;

                // Full re-render at new scale — both history and diff.
                hash ^= self.history.run_scroll_step(history_start, history_window);
                metrics.total_rows_rendered += history_window as u64;
                hash ^= self.diff.run_split_first_window(self.diff_window_rows);
                metrics.total_rows_rendered += self.diff_window_rows as u64;
                metrics.windows_rendered += 1;

                metrics.scale_factors_tested = self.scale_factors.len() as u64;
            }
        }

        metrics.layout_width_min_px = min_width as f64;
        metrics.layout_width_max_px = max_width as f64;

        (hash, metrics)
    }
}
