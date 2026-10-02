//! Git operations in progress or awaiting follow-up: clone, commands, hooks.

use super::Loadable;
use gitcomet_core::git_operation::{GitOperationId, GitOutputStream, HookExecutionId};
use gitcomet_core::services::{ForcePushLease, InteractiveRebaseEntry};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloneOpState {
    pub url: Arc<str>,
    pub dest: Arc<PathBuf>,
    pub status: CloneOpStatus,
    pub progress: CloneProgressMeter,
    pub seq: u64,
    pub output_tail: VecDeque<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmoduleAddProgressState {
    pub url: String,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloneProgressStage {
    Loading,
    RemoteObjects,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CloneProgressMeter {
    pub stage: CloneProgressStage,
    pub percent: u8,
}

impl Default for CloneProgressMeter {
    fn default() -> Self {
        Self {
            stage: CloneProgressStage::Loading,
            percent: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CloneOpStatus {
    Running,
    Cancelling,
    FinishedOk,
    Cancelled,
    FinishedErr(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandLogEntry {
    pub time: SystemTime,
    pub ok: bool,
    pub command: String,
    pub summary: String,
    /// Shared and capped: the log is deep-copied on every store dispatch
    /// (copy-on-write state), so owned per-entry output turned every message
    /// into a memcpy of up to 200 command transcripts.
    pub stdout: Arc<str>,
    pub stderr: Arc<str>,
    /// Whether finishing this command is worth telling the user about. Routine,
    /// user-initiated edits announce themselves through the change they make —
    /// a toast per staged line is noise — but they still belong in the log.
    /// Failures are always surfaced, whatever this says.
    pub announce_success: bool,
    /// The hook-activity entry that owns user-facing reporting for this
    /// command. When present, the UI does not also show the generic command
    /// completion toast/banner.
    pub hook_operation_id: Option<GitOperationId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitHookOperationStatus {
    Running,
    Cancelling,
    Succeeded,
    SucceededWithHookFailure,
    Failed,
    Cancelled,
    TimedOut,
}

impl GitHookOperationStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }

    pub fn is_warning(self) -> bool {
        matches!(self, Self::SucceededWithHookFailure | Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitHookRunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHookRun {
    pub id: HookExecutionId,
    pub name: String,
    pub status: GitHookRunStatus,
    pub exit_code: Option<i32>,
    pub duration: Option<Duration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHookOutputChunk {
    pub stream: GitOutputStream,
    pub text: Arc<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHookOperation {
    pub id: GitOperationId,
    pub label: String,
    /// Single-line, user-facing context for the operation that invoked these
    /// hooks, such as a commit subject or branch/remote direction.
    pub context: Option<String>,
    pub time: SystemTime,
    pub duration: Option<Duration>,
    pub status: GitHookOperationStatus,
    pub hooks: Vec<GitHookRun>,
    pub output: Arc<VecDeque<GitHookOutputChunk>>,
    pub output_bytes: usize,
    pub output_truncated: bool,
    pub latest_line: String,
    /// An explicit command requested activity before producing any output.
    pub command_started: bool,
    /// Latest large-file transfer progress, when the operation moved content.
    pub transfer: Option<gitcomet_core::git_operation::TransferProgress>,
    /// Shown as a progress card while it runs (fetch, pull, maintenance).
    pub progress_lane: bool,
    /// The newest meter git printed, or one GitComet measured itself.
    pub progress: Option<gitcomet_core::git_progress::GitProgressMeter>,
}

impl GitHookOperation {
    pub fn has_hooks(&self) -> bool {
        !self.hooks.is_empty()
    }

    /// Explicit commands must expose cancellation even while silent.
    pub fn is_reportable(&self) -> bool {
        self.command_started || self.has_hooks() || self.transfer.is_some()
    }

    pub fn active_hook_name(&self) -> Option<&str> {
        self.hooks
            .iter()
            .rev()
            .find(|hook| hook.status == GitHookRunStatus::Running)
            .map(|hook| hook.name.as_str())
    }

    pub fn combined_output(&self) -> String {
        let mut output = String::new();
        if self.output_truncated {
            output.push_str("[Earlier hook output was truncated]\n");
        }
        for chunk in self.output.iter() {
            output.push_str(&chunk.text);
        }
        output
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitOperationOuterOutcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingCommitRetry {
    pub message: String,
    pub amend: bool,
    pub push_after_commit: bool,
}

#[derive(Clone, Debug)]
pub struct InteractiveRebaseSetup {
    pub base: String,
    pub entries: Loadable<Vec<InteractiveRebaseEntry>>,
}

#[derive(Clone, Debug)]
pub struct InteractiveCherryPickSetup {
    pub entries: Vec<InteractiveRebaseEntry>,
    pub source_colors: Vec<(String, u8)>,
    /// Full commit messages are loaded separately from the subject-only log
    /// entries. The editor must not expose rewording or start the operation
    /// until this is `Ready`, otherwise saving a reword can truncate a body.
    pub full_messages: Loadable<()>,
}

/// Deferred context for authentication retries, confirmed force pushes and
/// queued large-file commands.
#[derive(Clone, Debug, Default)]
pub struct RepoPendingState {
    pub commit_retry: Option<PendingCommitRetry>,
    pub force_push_lease: Option<ForcePushLease>,
    /// Annex operations waiting for a conflicting pull or push to finish.
    pub(crate) large_file_commands: std::collections::VecDeque<PendingLargeFileCommand>,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingLargeFileCommand {
    pub command: gitcomet_core::large_files::LargeFileCommand,
    pub auth: Option<gitcomet_core::auth::StagedGitAuth>,
}

#[derive(Clone, Debug)]
pub struct TagPushPreviewState {
    pub request: gitcomet_core::tag_push::TagPushRequest,
    pub generation: u64,
    pub cancellation: gitcomet_core::services::CancellationToken,
    pub result: Loadable<Arc<gitcomet_core::tag_push::TagPushPreview>>,
}
