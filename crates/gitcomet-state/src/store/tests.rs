use super::*;
use crate::model::{CloneOpStatus, CloneProgressStage, DiagnosticKind, Loadable, RepoState};
use crate::msg::{Effect, RepoActionKind, RepoCommandKind};
use gitcomet_core::domain::{
    Branch, Commit, CommitDetails, CommitId, DiffArea, DiffTarget, LogCursor, LogPage, LogScope,
    ReflogEntry, Remote, RemoteBranch, RepoSpec, RepoStatus, StashEntry,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::process::{
    GitExecutablePreference, current_git_executable_preference, install_git_executable_preference,
};
use gitcomet_core::services::{CancellationToken, CommandOutput, PullMode, Result};
use gitcomet_core::test_support::UnconfiguredRepository;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// The empty workspace snapshot the history-switch tests seed nav histories
/// with: no diff target, no preview/edit mode, no selection besides the
/// commit. Kept beside the other store fixtures so tests do not re-write the
/// same struct literal.
pub(in crate::store) fn snapshot_with_commit(
    selected_commit: Option<CommitId>,
) -> crate::model::MainViewSnapshot {
    crate::model::MainViewSnapshot {
        diff_target: None,
        content_preview: false,
        edit_mode: false,
        selected_commit,
        range_selection: None,
        worktree_selection: None,
    }
}

/// A repository that only knows its own workdir. The store keeps a backend per
/// repo id and reads `spec()` off it; these tests drive the reducer directly and
/// never call through to Git.
pub(in crate::store) type DummyRepo = UnconfiguredRepository;

struct FailingBackend;

impl GitBackend for FailingBackend {
    fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
        Err(Error::new(ErrorKind::Unsupported(
            "store test backend open failure",
        )))
    }
}

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_store_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let output = match Command::new("git")
            .args(["difftool", "--tool-help"])
            .output()
        {
            Ok(output) => output,
            Err(_) => return true,
        };
        if output.status.success() {
            return true;
        }
        let stdout =
            String::from_utf8(output.stdout).unwrap_or_else(|_| "<non-utf8 stdout>".to_string());
        let stderr =
            String::from_utf8(output.stderr).unwrap_or_else(|_| "<non-utf8 stderr>".to_string());
        let text = format!("{}{}", stdout, stderr);
        !is_git_shell_startup_failure(&text)
    })
}

fn require_git_shell_for_store_tests() -> bool {
    #[cfg(windows)]
    {
        if !git_shell_available_for_store_tests() {
            eprintln!(
                "skipping store integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

fn wait_for_state_changed(event_rx: &smol::channel::Receiver<StoreEvent>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match event_rx.try_recv() {
            Ok(StoreEvent::StateChanged) => return,
            Err(smol::channel::TryRecvError::Empty) => {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for StoreEvent::StateChanged"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(smol::channel::TryRecvError::Closed) => {
                panic!("store event channel closed unexpectedly")
            }
        }
    }
}

pub(crate) fn staged_auth_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn git_runtime_store_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

struct GitRuntimePreferenceResetGuard {
    original: GitExecutablePreference,
}

impl GitRuntimePreferenceResetGuard {
    fn install(preference: GitExecutablePreference) -> Self {
        let original = current_git_executable_preference();
        let _ = install_git_executable_preference(preference);
        Self { original }
    }
}

impl Drop for GitRuntimePreferenceResetGuard {
    fn drop(&mut self) {
        let _ = install_git_executable_preference(self.original.clone());
    }
}

#[cfg(unix)]
fn write_git_runtime_probe_script(script_path: &Path, probe_log: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    fs::write(
        script_path,
        format!(
            "#!/bin/sh\nprintf 'probe\\n' >> '{}'\nprintf 'git version 9.9.9-test\\n'\n",
            probe_log.display()
        ),
    )
    .expect("write git runtime probe script");
    let mut permissions = fs::metadata(script_path)
        .expect("git runtime probe script metadata")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(script_path, permissions)
        .expect("set git runtime probe script permissions");
}

#[cfg(windows)]
fn write_git_runtime_probe_script(script_path: &Path, probe_log: &Path) {
    fs::write(
        script_path,
        format!(
            "@echo off\r\necho probe>>\"{}\"\r\necho git version 9.9.9-test\r\n",
            probe_log.display()
        ),
    )
    .expect("write git runtime probe script");
}

fn git_runtime_probe_count(probe_log: &Path) -> usize {
    fs::read_to_string(probe_log)
        .unwrap_or_default()
        .lines()
        .count()
}

fn has_worktree_status_effect(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadWorktreeStatus { repo_id: candidate } if *candidate == repo_id
        )
    })
}

fn has_staged_status_effect(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadStagedStatus { repo_id: candidate } if *candidate == repo_id
        )
    })
}

fn has_combined_status_effect(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadStatus { repo_id: candidate } if *candidate == repo_id
        )
    })
}

fn has_status_refresh_effects(effects: &[Effect], repo_id: RepoId) -> bool {
    has_combined_status_effect(effects, repo_id)
        || (has_worktree_status_effect(effects, repo_id)
            && has_staged_status_effect(effects, repo_id))
}

#[test]
fn app_store_clone_dispatches_restore_and_close_paths() {
    let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
    let (store, event_rx) = AppStore::new(backend);
    let cloned = store.clone();

    cloned.dispatch(Msg::RestoreSession {
        open_repos: Vec::new(),
        active_repo: None,
    });
    wait_for_state_changed(&event_rx);

    store.dispatch(Msg::CloseRepo {
        repo_id: RepoId(999),
    });
    wait_for_state_changed(&event_rx);

    let snapshot = store.snapshot();
    assert!(snapshot.repos.is_empty());
    assert_eq!(snapshot.active_repo, None);
}

#[test]
fn app_store_dispatch_does_not_reprobe_git_runtime_for_git_messages() {
    let _lock = git_runtime_store_test_lock();
    let temp = tempfile::tempdir().expect("create tempdir for git runtime probe");
    let probe_log = temp.path().join("git-runtime-probes.log");
    #[cfg(unix)]
    let script_path = temp.path().join("git");
    #[cfg(windows)]
    let script_path = temp.path().join("git.cmd");
    write_git_runtime_probe_script(&script_path, &probe_log);

    let _restore =
        GitRuntimePreferenceResetGuard::install(GitExecutablePreference::Custom(script_path));
    let initial_probe_count = git_runtime_probe_count(&probe_log);

    let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
    let (store, _event_rx) = AppStore::new(backend);

    assert_eq!(
        git_runtime_probe_count(&probe_log),
        initial_probe_count,
        "creating the store should reuse the installed runtime state without probing again"
    );

    store.dispatch(Msg::ReloadRepo {
        repo_id: RepoId(999),
    });

    assert_eq!(
        git_runtime_probe_count(&probe_log),
        initial_probe_count,
        "dispatch should not re-run `git --version` for regular Git-backed messages"
    );
}

#[test]
fn app_store_open_repo_effect_propagates_open_error_into_state() {
    let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
    let (store, event_rx) = AppStore::new(backend);

    let base = std::env::temp_dir().join(format!(
        "gitcomet-store-open-repo-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).expect("temporary repo path should be creatable");
    let expected_workdir = canonicalize_or_original(base.clone());

    store.dispatch(Msg::OpenRepo(base));

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = store.snapshot();
        if let Some(repo) = snapshot.repos.first()
            && matches!(repo.open, Loadable::Error(_))
        {
            assert_eq!(repo.spec.workdir, expected_workdir);
            assert_eq!(snapshot.active_repo, Some(repo.id));
            let error = repo.feedback.last_error.as_deref().unwrap_or_default();
            assert!(
                error.contains("store test backend open failure"),
                "unexpected open error: {error}"
            );
            return;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for repo open error in store state"
        );
        let _ = event_rx.try_recv();
        std::thread::sleep(Duration::from_millis(10));
    }
}

mod actions_emit_effects;
mod auth_prompt;
mod conflict_session;
mod conflict_telemetry;
mod diff_selection;
mod effects;
mod external_and_history;
mod file_browser_follow;
mod reducer_diagnostics;
mod repo_management;
mod repo_monitor;
mod send_failures;
mod worktree_redirect;
