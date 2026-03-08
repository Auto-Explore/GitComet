use super::*;
use crate::model::{CloneOpStatus, DiagnosticKind, Loadable, RepoState};
use crate::msg::{Effect, RepoCommandKind};
use gitcomet_core::domain::{
    Branch, Commit, CommitDetails, CommitId, DiffArea, DiffTarget, LogCursor, LogPage, LogScope,
    ReflogEntry, Remote, RemoteBranch, RepoSpec, RepoStatus, StashEntry,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, PullMode, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
#[cfg(windows)]
use std::sync::OnceLock;
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
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
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

mod actions_emit_effects;
mod conflict_session;
mod conflict_telemetry;
mod diff_selection;
mod effects;
mod external_and_history;
mod repo_management;
mod repo_monitor;
mod send_failures;
