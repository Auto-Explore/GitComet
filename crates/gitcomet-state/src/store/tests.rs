use super::*;
use crate::model::{CloneOpStatus, DiagnosticKind, Loadable, RepoState};
use crate::msg::{Effect, RepoCommandKind};
use gitcomet_core::domain::{
    Branch, Commit, CommitDetails, CommitId, DiffArea, DiffTarget, LogCursor, LogPage, LogScope,
    ReflogEntry, Remote, RemoteBranch, RepoSpec, RepoStatus, StashEntry,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{CommandOutput, PullMode, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

struct DummyRepo {
    spec: RepoSpec,
}

impl DummyRepo {
    fn new(path: &str) -> Self {
        Self {
            spec: RepoSpec {
                workdir: PathBuf::from(path),
            },
        }
    }
}

impl GitRepository for DummyRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
        unimplemented!()
    }
    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unimplemented!()
    }
    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unimplemented!()
    }
    fn current_branch(&self) -> Result<String> {
        unimplemented!()
    }
    fn list_branches(&self) -> Result<Vec<Branch>> {
        unimplemented!()
    }
    fn list_remotes(&self) -> Result<Vec<Remote>> {
        unimplemented!()
    }
    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        unimplemented!()
    }
    fn status(&self) -> Result<RepoStatus> {
        unimplemented!()
    }
    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unimplemented!()
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        unimplemented!()
    }
    fn delete_branch(&self, _name: &str) -> Result<()> {
        unimplemented!()
    }
    fn checkout_branch(&self, _name: &str) -> Result<()> {
        unimplemented!()
    }
    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        unimplemented!()
    }
    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        unimplemented!()
    }
    fn revert(&self, _id: &CommitId) -> Result<()> {
        unimplemented!()
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unimplemented!()
    }
    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        unimplemented!()
    }
    fn stash_apply(&self, _index: usize) -> Result<()> {
        unimplemented!()
    }
    fn stash_drop(&self, _index: usize) -> Result<()> {
        unimplemented!()
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        unimplemented!()
    }
    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        unimplemented!()
    }
    fn commit(&self, _message: &str) -> Result<()> {
        unimplemented!()
    }
    fn fetch_all(&self) -> Result<()> {
        unimplemented!()
    }
    fn pull(&self, _mode: PullMode) -> Result<()> {
        unimplemented!()
    }
    fn push(&self) -> Result<()> {
        unimplemented!()
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unimplemented!()
    }
}

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
            let error = repo.last_error.as_deref().unwrap_or_default();
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
mod reducer_diagnostics;
mod repo_management;
mod repo_monitor;
mod send_failures;
