use gitcomet_state::model::{AppState, Loadable};
use serde_json::{Map, Value, json};
use std::env;
use std::io::Write as _;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const EVENT_PREFIX: &str = "GITCOMET_PERF_STARTUP_EVENT ";
const ENABLED_ENV: &str = "GITCOMET_PERF_STARTUP_PROBE";
const DISABLE_AUTO_RESTORE_ENV: &str = "GITCOMET_PERF_STARTUP_DISABLE_AUTO_RESTORE";
const AUTO_EXIT_ENV: &str = "GITCOMET_PERF_STARTUP_AUTO_EXIT";
const EXPECT_READY_REPOS_ENV: &str = "GITCOMET_PERF_STARTUP_EXPECT_READY_REPOS";

static FIRST_PAINT_EMITTED: AtomicBool = AtomicBool::new(false);
static FIRST_INTERACTIVE_EMITTED: AtomicBool = AtomicBool::new(false);
static LAST_REPO_PROGRESS_SNAPSHOT: AtomicU64 = AtomicU64::new(u64::MAX);
static CONFIG: OnceLock<StartupProbeConfig> = OnceLock::new();

#[derive(Clone, Copy, Debug)]
struct StartupProbeConfig {
    enabled: bool,
    disable_auto_restore: bool,
    auto_exit_after_interactive: bool,
    expected_ready_repos: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
enum StartupProbeEvent {
    FirstPaint,
    FirstInteractive,
    ReposLoaded,
}

impl StartupProbeEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::FirstPaint => "first_paint",
            Self::FirstInteractive => "first_interactive",
            Self::ReposLoaded => "repos_loaded",
        }
    }
}

pub(crate) fn is_enabled() -> bool {
    config().enabled
}

pub(crate) fn disable_auto_restore() -> bool {
    let config = config();
    config.enabled && config.disable_auto_restore
}

pub(crate) fn should_exit_after_first_interactive() -> bool {
    let config = config();
    config.enabled
        && config.auto_exit_after_interactive
        && config.expected_ready_repos.unwrap_or(0) == 0
}

pub(crate) fn observe_app_state(state: &AppState) -> bool {
    if !is_enabled() {
        return false;
    }

    let repos_loaded = count_ready_repos(state);
    let repos_total = state.repos.len();
    let snapshot = pack_repo_progress(repos_loaded, repos_total);
    if LAST_REPO_PROGRESS_SNAPSHOT.swap(snapshot, Ordering::SeqCst) != snapshot {
        emit_repo_progress(repos_loaded, repos_total);
    }

    let config = config();
    config.enabled
        && config.auto_exit_after_interactive
        && FIRST_INTERACTIVE_EMITTED.load(Ordering::SeqCst)
        && config
            .expected_ready_repos
            .is_some_and(|expected| repos_loaded >= expected)
}

pub(crate) fn mark_first_paint() -> bool {
    mark_once(StartupProbeEvent::FirstPaint, &FIRST_PAINT_EMITTED)
}

pub(crate) fn mark_first_interactive() -> bool {
    mark_once(
        StartupProbeEvent::FirstInteractive,
        &FIRST_INTERACTIVE_EMITTED,
    )
}

fn config() -> &'static StartupProbeConfig {
    CONFIG.get_or_init(|| StartupProbeConfig {
        enabled: env_flag(ENABLED_ENV),
        disable_auto_restore: env_flag(DISABLE_AUTO_RESTORE_ENV),
        auto_exit_after_interactive: env_flag(AUTO_EXIT_ENV),
        expected_ready_repos: env_usize(EXPECT_READY_REPOS_ENV),
    })
}

fn env_flag(key: &str) -> bool {
    env::var(key)
        .ok()
        .as_deref()
        .map(parse_bool_flag)
        .unwrap_or(false)
}

fn env_usize(key: &str) -> Option<usize> {
    env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

fn parse_bool_flag(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn mark_once(event: StartupProbeEvent, emitted: &AtomicBool) -> bool {
    if !is_enabled() || emitted.swap(true, Ordering::SeqCst) {
        return false;
    }

    emit_event(event);
    true
}

fn emit_event(event: StartupProbeEvent) {
    let mut payload = Map::new();
    payload.insert("name".to_string(), json!(event.as_str()));
    payload.insert("rss_kib".to_string(), json!(current_rss_kib()));
    crate::perf_alloc::current_alloc_metrics().append_to_payload(&mut payload);
    emit_payload(Value::Object(payload));
}

fn emit_repo_progress(repos_loaded: usize, repos_total: usize) {
    let mut payload = Map::new();
    payload.insert(
        "name".to_string(),
        json!(StartupProbeEvent::ReposLoaded.as_str()),
    );
    payload.insert("rss_kib".to_string(), json!(current_rss_kib()));
    payload.insert("repos_loaded".to_string(), json!(repos_loaded));
    payload.insert("repos_total".to_string(), json!(repos_total));
    crate::perf_alloc::current_alloc_metrics().append_to_payload(&mut payload);
    emit_payload(Value::Object(payload));
}

fn emit_payload(payload: serde_json::Value) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{EVENT_PREFIX}{payload}");
    let _ = stderr.flush();
}

fn count_ready_repos(state: &AppState) -> usize {
    state
        .repos
        .iter()
        .filter(|repo| matches!(&repo.open, Loadable::Ready(())))
        .count()
}

fn pack_repo_progress(repos_loaded: usize, repos_total: usize) -> u64 {
    ((repos_total as u64) << 32) | repos_loaded as u64
}

#[cfg(target_os = "linux")]
fn current_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?.split_whitespace().next()?;
        value.parse::<u64>().ok()
    })
}

#[cfg(not(target_os = "linux"))]
fn current_rss_kib() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::{count_ready_repos, pack_repo_progress, parse_bool_flag};
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_state::model::{AppState, Loadable, RepoId, RepoState};
    use std::path::PathBuf;

    #[test]
    fn parse_bool_flag_accepts_common_truthy_values() {
        for value in ["1", "true", "TRUE", "Yes", "on"] {
            assert!(parse_bool_flag(value), "expected {value:?} to be truthy");
        }
    }

    #[test]
    fn parse_bool_flag_rejects_other_values() {
        for value in ["0", "false", "no", "", "maybe"] {
            assert!(!parse_bool_flag(value), "expected {value:?} to be falsy");
        }
    }

    #[test]
    fn count_ready_repos_only_counts_opened_repositories() {
        let mut state = AppState::default();
        let mut ready = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/ready"),
            },
        );
        ready.open = Loadable::Ready(());
        state.repos.push(ready);
        state.repos.push(RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: PathBuf::from("/tmp/loading"),
            },
        ));

        assert_eq!(count_ready_repos(&state), 1);
    }

    #[test]
    fn pack_repo_progress_keeps_loaded_and_total_counts() {
        assert_eq!(pack_repo_progress(5, 12), (12u64 << 32) | 5u64);
    }
}
