use gitcomet_state::model::{AppState, Loadable};
use serde_json::{Map, Value, json};
use std::env;
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const EVENT_PREFIX: &str = "GITCOMET_PERF_STARTUP_EVENT ";
const ENABLED_ENV: &str = "GITCOMET_PERF_STARTUP_PROBE";
const DISABLE_AUTO_RESTORE_ENV: &str = "GITCOMET_PERF_STARTUP_DISABLE_AUTO_RESTORE";
const AUTO_EXIT_ENV: &str = "GITCOMET_PERF_STARTUP_AUTO_EXIT";
const EXPECT_READY_REPOS_ENV: &str = "GITCOMET_PERF_STARTUP_EXPECT_READY_REPOS";
const WAIT_FOR_ACTIVE_REPO_ENV: &str = "GITCOMET_PERF_STARTUP_WAIT_FOR_ACTIVE_REPO";
const WAIT_FOR_HOME_ENV: &str = "GITCOMET_PERF_STARTUP_WAIT_FOR_HOME_READY";

static FIRST_PAINT_EMITTED: AtomicBool = AtomicBool::new(false);
static FIRST_INTERACTIVE_EMITTED: AtomicBool = AtomicBool::new(false);
static ACTIVE_REPO_READY_EMITTED: AtomicBool = AtomicBool::new(false);
static HOME_READY_EMITTED: AtomicBool = AtomicBool::new(false);
static LAST_REPO_PROGRESS_SNAPSHOT: AtomicU64 = AtomicU64::new(u64::MAX);
static CONFIG: OnceLock<StartupProbeConfig> = OnceLock::new();

#[derive(Clone, Copy, Debug)]
struct StartupProbeConfig {
    enabled: bool,
    #[cfg_attr(test, allow(dead_code))]
    disable_auto_restore: bool,
    auto_exit_after_interactive: bool,
    expected_ready_repos: Option<usize>,
    wait_for_active_repo: bool,
    wait_for_home: bool,
}

#[derive(Clone, Copy, Debug)]
enum StartupProbeEvent {
    FirstPaint,
    FirstInteractive,
    ReposLoaded,
    ActiveRepoReady,
    HomeReady,
}

impl StartupProbeEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::FirstPaint => "first_paint",
            Self::FirstInteractive => "first_interactive",
            Self::ReposLoaded => "repos_loaded",
            Self::ActiveRepoReady => "active_repo_ready",
            Self::HomeReady => "home_ready",
        }
    }
}

pub(crate) fn is_enabled() -> bool {
    config().enabled
}

#[cfg_attr(test, allow(dead_code))]
pub(crate) fn disable_auto_restore() -> bool {
    let config = config();
    config.enabled && config.disable_auto_restore
}

pub(crate) fn should_exit_after_first_interactive() -> bool {
    let config = config();
    let snapshot = LAST_REPO_PROGRESS_SNAPSHOT.load(Ordering::SeqCst);
    let loaded = (snapshot != u64::MAX).then_some((snapshot & u64::from(u32::MAX)) as usize);
    should_exit(
        config,
        FIRST_INTERACTIVE_EMITTED.load(Ordering::SeqCst),
        loaded,
        ACTIVE_REPO_READY_EMITTED.load(Ordering::SeqCst),
        home_ready_recorded(),
    )
}

fn should_exit(
    config: &StartupProbeConfig,
    interactive: bool,
    repos_loaded: Option<usize>,
    active_ready: bool,
    home_ready: bool,
) -> bool {
    let expected = config.expected_ready_repos.unwrap_or(0);
    config.enabled
        && config.auto_exit_after_interactive
        && interactive
        && (expected == 0 || repos_loaded.is_some_and(|loaded| loaded >= expected))
        && (!config.wait_for_active_repo || active_ready)
        && (!config.wait_for_home || home_ready)
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
    if active_repo_ready(state) {
        mark_once(
            StartupProbeEvent::ActiveRepoReady,
            &ACTIVE_REPO_READY_EMITTED,
        );
    }

    should_exit_after_first_interactive()
}

pub(crate) fn home_ready_recorded() -> bool {
    HOME_READY_EMITTED.load(Ordering::SeqCst)
}

/// Called on the frame after Home was prepainted with Git available. The first
/// interactive frame alone may have displayed the Git-checking loading screen.
pub(crate) fn mark_home_ready() -> bool {
    mark_once(StartupProbeEvent::HomeReady, &HOME_READY_EMITTED)
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
        enabled: env_flag(ENABLED_ENV) || gitcomet_core::startup_trace::is_enabled(),
        disable_auto_restore: env_flag(DISABLE_AUTO_RESTORE_ENV),
        auto_exit_after_interactive: env_flag(AUTO_EXIT_ENV),
        expected_ready_repos: env_usize(EXPECT_READY_REPOS_ENV),
        wait_for_active_repo: env_flag(WAIT_FOR_ACTIVE_REPO_ENV),
        wait_for_home: env_flag(WAIT_FOR_HOME_ENV),
    })
}

pub(crate) use gitcomet_ui_kit::ui_probe::env_flag;

fn env_usize(key: &str) -> Option<usize> {
    env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

fn mark_once(event: StartupProbeEvent, emitted: &AtomicBool) -> bool {
    if !is_enabled() || emitted.swap(true, Ordering::SeqCst) {
        return false;
    }

    emit_event(event);
    true
}

fn emit_event(event: StartupProbeEvent) {
    crate::ui_probe::scenario_record("startup_milestone", json!({"name": event.as_str()}));
    gitcomet_core::startup_trace::mark(event.as_str());
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
    let _ = write_payload_record(&mut stderr, &payload);
}

fn write_payload_record(
    writer: &mut impl Write,
    payload: &serde_json::Value,
) -> std::io::Result<()> {
    // Formatting Value directly into stderr sends each JSON token separately.
    // In particular, redirected Windows pipes can stall between those writes.
    let line = format!("{EVENT_PREFIX}{payload}\n");
    writer.write_all(line.as_bytes())?;
    writer.flush()
}

fn count_ready_repos(state: &AppState) -> usize {
    state
        .repos
        .iter()
        .filter(|repo| matches!(&repo.open, Loadable::Ready(())))
        .count()
}

/// Opening a handle is much earlier than showing its initial contents. Only
/// the selected repository loads those contents; restored inactive tabs are lazy.
fn active_repo_ready(state: &AppState) -> bool {
    state.repos.iter().any(|repo| {
        Some(repo.id) == state.active_repo
            && matches!(repo.open, Loadable::Ready(()))
            && matches!(repo.status, Loadable::Ready(_))
            && matches!(repo.log, Loadable::Ready(_))
            && !repo.loads_in_flight.any_foreground_in_flight()
            && repo.local_actions_in_flight == 0
            && repo.pull_in_flight == 0
            && repo.push_in_flight == 0
    })
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
    use super::{
        EVENT_PREFIX, StartupProbeConfig, active_repo_ready, count_ready_repos, pack_repo_progress,
        should_exit, write_payload_record,
    };
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_state::model::{AppState, Loadable, RepoId, RepoState};
    use gitcomet_ui_kit::ui_probe::parse_bool_flag;
    use std::path::PathBuf;

    #[test]
    fn startup_payload_is_one_complete_flushed_write_with_escaped_json_preserved() {
        #[derive(Default)]
        struct Writes {
            records: Vec<Vec<u8>>,
            flushes: usize,
        }
        impl std::io::Write for Writes {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.records.push(bytes.to_vec());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.flushes += 1;
                Ok(())
            }
        }

        let payload = serde_json::json!({
            "name": "first_interactive",
            "rss_kib": null,
            "alloc_bytes": 0,
            "note": "quote\" slash\\ newline\n雪",
        });
        let mut writer = Writes::default();
        write_payload_record(&mut writer, &payload).unwrap();
        assert_eq!(
            writer.records.len(),
            1,
            "JSON tokens must not cause separate writes"
        );
        assert_eq!(writer.flushes, 1, "publish the complete record immediately");
        let line = std::str::from_utf8(&writer.records[0]).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.lines().count(), 1);
        let parsed: serde_json::Value =
            serde_json::from_str(line.strip_prefix(EVENT_PREFIX).unwrap()).unwrap();
        assert_eq!(parsed, payload);

        let home = serde_json::json!({"name": "home_ready", "rss_kib": null});
        write_payload_record(&mut writer, &home).unwrap();
        assert_eq!(writer.records.len(), 2);
        assert_eq!(writer.flushes, 2, "do not buffer milestones until shutdown");
        let parsed: serde_json::Value =
            serde_json::from_slice(&writer.records[1][EVENT_PREFIX.len()..]).unwrap();
        assert_eq!(parsed, home);
    }

    #[test]
    fn parse_bool_flag_accepts_common_truthy_values() {
        for value in ["1", "true", "TRUE", "Yes", "on"] {
            assert!(parse_bool_flag(value), "expected {value:?} to be truthy");
        }
    }

    #[test]
    fn parse_bool_flag_rejects_other_values() {
        for value in ["0", "false", "no", "off", "", "maybe"] {
            assert!(!parse_bool_flag(value), "expected {value:?} to be falsy");
        }
    }

    #[test]
    fn count_ready_repos_only_counts_opened_repositories() {
        let mut state = AppState::test_default();
        let mut ready = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/ready"),
            },
        );
        ready.open = Loadable::Ready(());
        ready.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
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

    #[test]
    fn repository_probe_waits_for_both_interactivity_and_contents_in_either_order() {
        let config = StartupProbeConfig {
            enabled: true,
            disable_auto_restore: false,
            auto_exit_after_interactive: true,
            expected_ready_repos: Some(5),
            wait_for_active_repo: true,
            wait_for_home: false,
        };
        assert!(!should_exit(&config, true, Some(5), false, false));
        assert!(!should_exit(&config, false, Some(5), true, false));
        assert!(!should_exit(&config, true, Some(4), true, false));
        assert!(!should_exit(&config, true, None, true, false));
        assert!(should_exit(&config, true, Some(5), true, false));
        let empty = StartupProbeConfig {
            expected_ready_repos: Some(0),
            wait_for_active_repo: false,
            ..config
        };
        assert!(should_exit(&empty, true, None, false, false));
        let picker = StartupProbeConfig {
            wait_for_home: true,
            ..empty
        };
        assert!(!should_exit(&picker, true, None, false, false));
        assert!(!should_exit(&picker, false, None, false, true));
        assert!(should_exit(&picker, true, None, false, true));
    }

    #[test]
    fn active_repo_ready_waits_for_contents_and_foreground_work() {
        let mut state = AppState::test_default();
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/ready"),
            },
        );
        state.active_repo = Some(repo.id);
        repo.open = Loadable::Ready(());
        repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
        state.repos.push(repo);
        assert!(
            !active_repo_ready(&state),
            "an open handle has no contents yet"
        );
        state.repos[0].status = Loadable::Ready(Default::default());
        assert!(!active_repo_ready(&state), "history is still missing");
        state.repos[0].log = Loadable::Ready(
            gitcomet_core::domain::LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }
            .into(),
        );
        assert!(active_repo_ready(&state));
        state.repos[0].local_actions_in_flight = 1;
        assert!(!active_repo_ready(&state));
        state.repos[0].local_actions_in_flight = 0;
        state.repos[0].status = Loadable::Loading;
        assert!(!active_repo_ready(&state), "a new status load is not ready");
    }

    #[test]
    fn inactive_repositories_do_not_stand_in_for_the_selected_repository() {
        let mut state = AppState::test_default();
        state.repos.push(RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/inactive"),
            },
        ));
        let repo = &mut state.repos[0];
        repo.open = Loadable::Ready(());
        repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
        repo.status = Loadable::Ready(Default::default());
        repo.log = Loadable::Ready(
            gitcomet_core::domain::LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }
            .into(),
        );
        assert!(!active_repo_ready(&state));
        state.active_repo = Some(RepoId(2));
        assert!(!active_repo_ready(&state));
        state.active_repo = Some(RepoId(1));
        state.repos.push(RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: PathBuf::from("/tmp/lazy"),
            },
        ));
        assert!(
            active_repo_ready(&state),
            "inactive tabs load contents on selection"
        );
    }
}
