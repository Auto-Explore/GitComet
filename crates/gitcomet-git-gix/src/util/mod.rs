use gitcomet_core::auth::AuthOutcome;
use gitcomet_core::auth::askpass::{
    GIT_COMMAND_TIMEOUT_ENV, PromptAuth, append_host_prompt_to_stderr,
    append_passphrase_prompt_to_stderr, configure_git_auth_prompt, create_askpass_script,
    resolve_auth_outcome, take_pending_git_auth,
};
use gitcomet_core::domain::{Commit, CommitId, CommitParentIds, LogPage};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::git_operation::{
    self, GitOperationContext, GitOperationEvent, GitOutputChunk, GitOutputStream, HookExecutionId,
};
use gitcomet_core::process::{configure_background_command, git_command};
use gitcomet_core::services::{CancellationToken, CommandOutput, Result};
use std::io::{self, BufRead as _, Read};
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

pub(crate) use gitcomet_core::auth::askpass::git_command_timeout;

/// The process exit and its authentication exchange are independent: a pull
/// can unlock its key and fetch successfully before a local merge fails.
#[derive(Debug)]
pub(crate) struct GitOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) auth_outcome: AuthOutcome,
}

thread_local! {
    static SHARED_GIT_AUTH: std::cell::RefCell<Option<Option<PromptAuth>>> = const {
        std::cell::RefCell::new(None)
    };
}

fn command_git_auth() -> Option<PromptAuth> {
    SHARED_GIT_AUTH
        .with(|slot| slot.borrow().clone())
        .unwrap_or_else(take_pending_git_auth)
}

/// Consume staged credentials once for a multi-command operation. Keep them
/// on this worker only, and restore the previous scope even on panic.
pub(crate) fn with_shared_git_auth<T>(run: impl FnOnce() -> T) -> T {
    struct RestoreAuth(Option<Option<PromptAuth>>);
    impl Drop for RestoreAuth {
        fn drop(&mut self) {
            SHARED_GIT_AUTH.with(|slot| slot.replace(self.0.take()));
        }
    }
    let auth = command_git_auth();
    let _restore = RestoreAuth(SHARED_GIT_AUTH.with(|slot| slot.replace(Some(auth))));
    run()
}

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
