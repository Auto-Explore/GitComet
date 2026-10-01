use gitcomet_core::auth::askpass::{
    GIT_COMMAND_TIMEOUT_ENV, append_host_prompt_to_stderr, append_passphrase_prompt_to_stderr,
    configure_git_auth_prompt, create_askpass_script, remember_successful_prompt_auth,
    take_pending_git_auth,
};
use gitcomet_core::domain::{Commit, CommitId, CommitParentIds, LogPage};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::git_operation::{
    self, GitOperationContext, GitOperationEvent, GitOutputChunk, GitOutputStream, HookExecutionId,
};
use gitcomet_core::process::{configure_background_command, git_command};
use gitcomet_core::services::{CancellationToken, CommandOutput, Result};
use std::io::{self, BufRead as _, Read as _};
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

pub(crate) use gitcomet_core::auth::askpass::git_command_timeout;

// Used by test-only helpers below.
#[cfg(test)]
use gitcomet_core::domain::RemoteBranch;
#[cfg(test)]
use std::ffi::OsString;

pub(super) const GIT_COMMAND_WAIT_POLL_MAX: Duration = Duration::from_millis(5);
pub(super) const GIT_ACTIVITY_OUTPUT_FLUSH: Duration = Duration::from_millis(100);
pub(super) const GIT_ACTIVITY_OUTPUT_BATCH_BYTES: usize = 16 * 1024;
pub(super) const GIT_TRACE2_POLL: Duration = Duration::from_millis(20);
#[cfg(unix)]
pub(super) const GIT_PROCESS_TERMINATE_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TestGitCommandEnvironment {
    pub(crate) global_config: PathBuf,
    pub(crate) home_dir: PathBuf,
    pub(crate) xdg_config_home: PathBuf,
    pub(crate) gnupg_home: PathBuf,
}

pub(super) static TEST_GIT_COMMAND_ENVIRONMENT: OnceLock<TestGitCommandEnvironment> =
    OnceLock::new();

pub(super) fn io_err(e: std::io::Error) -> Error {
    Error::new(ErrorKind::Io(e.kind()))
}

mod args;
mod log_parse;
mod process;
mod stream;
mod trace2;

pub(crate) use args::*;
pub(crate) use log_parse::*;
pub(crate) use process::*;
use stream::*;
use trace2::*;

#[cfg(test)]
mod tests;
