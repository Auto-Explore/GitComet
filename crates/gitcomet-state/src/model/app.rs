//! App-wide state: settings, prompts, and notifications.

use super::{CloneOpState, RepoId, RepoState};
use crate::msg::RepoCommandKind;
use gitcomet_core::domain::SignatureFormats;
use gitcomet_core::process::GitRuntimeState;
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::services::{SafePushAfterCommitContext, SubmoduleTrustTarget};
use gitcomet_core::signing_tools::SigningToolsState;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SidebarMode {
    #[default]
    Branches,
    Files,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GitLogTagFetchMode {
    #[default]
    OnRepositoryActivation,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GitLogSettings {
    pub show_history_tags: bool,
    pub tag_fetch_mode: GitLogTagFetchMode,
    /// Escape hatch: a misconfigured `gpg.program` or a wedged `gpg-agent`
    /// would otherwise slow every history page with no way to turn it off.
    pub verify_commit_signatures: bool,
}

impl Default for GitLogSettings {
    fn default() -> Self {
        Self {
            show_history_tags: true,
            tag_fetch_mode: GitLogTagFetchMode::OnRepositoryActivation,
            verify_commit_signatures: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum DefaultTagType {
    #[default]
    Lightweight,
    Annotated,
}

impl GitLogSettings {
    pub fn auto_fetch_tags_on_repo_activation(self) -> bool {
        matches!(
            self.tag_fetch_mode,
            GitLogTagFetchMode::OnRepositoryActivation
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteSettings {
    pub prune_deleted_remote_branches_on_fetch: bool,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            prune_deleted_remote_branches_on_fetch: true,
        }
    }
}

/// How GitComet treats Git LFS and git-annex repositories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LargeFileSettings {
    /// Hide `git-annex` and `synced/*` branches from branch lists.
    pub hide_annex_refs: bool,
    /// On an adjusted branch, Pull and Push run `git annex pull` / `push`:
    /// a plain merge into an adjusted branch is git-annex's documented footgun.
    pub annex_pull_push: bool,
    /// Annex pull, push and sync also move annexed content.
    pub annex_sync_content: bool,
}

impl Default for LargeFileSettings {
    fn default() -> Self {
        Self {
            hide_annex_refs: true,
            annex_pull_push: true,
            annex_sync_content: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaintenanceSettings {
    /// Check daily whether a repository needs maintenance, and offer to run it.
    pub recommend: bool,
}

impl Default for MaintenanceSettings {
    fn default() -> Self {
        Self { recommend: true }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileBrowserSettings {
    /// Active file browsing follows the selected history row.
    pub follow_selected_commit: bool,
}

impl Default for FileBrowserSettings {
    fn default() -> Self {
        Self {
            follow_selected_commit: true,
        }
    }
}

// ── App state ───────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct AppState {
    pub filesystem: FilesystemState,
    /// Recent failed opens remain observable after their provisional tab is removed.
    pub repository_open_failures:
        std::collections::BTreeMap<PathBuf, (gitcomet_core::filesystem::OperationId, String)>,
    pub repos: Vec<RepoState>,
    pub active_repo: Option<RepoId>,
    /// Unacknowledged open failures, shared by snapshots until they change.
    /// Window routing releases its reservations before acknowledging these.
    pub repo_open_failures: Arc<FxHashMap<PathBuf, u64>>,
    /// Store-wide sequence; acknowledgements must not reset retry baselines.
    pub repo_open_failure_revision: u64,
    pub clone: Option<CloneOpState>,
    pub notifications: Vec<AppNotification>,
    pub auth_prompt: Option<AuthPromptState>,
    pub branch_exists_prompt: Option<BranchExistsPromptState>,
    pub submodule_trust_prompt: Option<SubmoduleTrustPromptState>,
    /// A submodule trust check is running in the background. Set the moment the
    /// add/update/load is triggered and cleared when the check resolves, so the
    /// UI can show a pending/spinner state instead of a dead gap before the
    /// trust dialog (or a silent proceed) appears.
    pub submodule_trust_check_pending: Option<SubmoduleTrustCheckState>,
    pub git_runtime: GitRuntimeState,
    /// The signature verifiers Git can run. Formats without one are not verified.
    pub signing_tools: SigningToolsState,
    /// Whether Git can run `git lfs` / `git annex`; gates their commands.
    pub large_file_tools: gitcomet_core::large_file_tools::LargeFileToolsState,
    pub large_file_settings: LargeFileSettings,
    pub remote_url_policy: RemoteUrlPolicy,
    pub git_log_settings: GitLogSettings,
    pub remote_settings: RemoteSettings,
    pub maintenance_settings: MaintenanceSettings,
    pub file_browser_settings: FileBrowserSettings,
    pub sidebar_mode: SidebarMode,
    pub default_tag_type: DefaultTagType,
    /// Outstanding [`WatchLease`](crate::store::WatchLease)s per open
    /// repository. A leased repository keeps its file watcher running while
    /// it is not the active one (a hosted view of a linked worktree, say).
    pub watch_leases: Arc<FxHashMap<RepoId, u32>>,
    /// Leases for linked worktrees, scoped to the owning repository lifetime.
    pub worktree_watch_leases: Arc<FxHashMap<(RepoId, u64, std::path::PathBuf), u32>>,
}

impl AppState {
    /// Deterministic fixture: tests opt into an available runtime without spawning Git.
    #[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
    pub fn test_default() -> Self {
        Self {
            git_runtime: GitRuntimeState {
                preference: gitcomet_core::process::GitExecutablePreference::SystemPath,
                availability: gitcomet_core::process::GitExecutableAvailability::Available {
                    version_output: "git version 2.55.0 (test)".into(),
                },
            },
            ..Self::default()
        }
    }

    /// The signature formats to verify: none when the preference is off,
    /// otherwise those whose verifier was not found missing.
    pub fn signature_verification_formats(&self) -> SignatureFormats {
        if self.git_log_settings.verify_commit_signatures {
            self.signing_tools.usable_formats()
        } else {
            SignatureFormats::NONE
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FilesystemState {
    pub pending: std::collections::BTreeMap<
        gitcomet_core::filesystem::OperationId,
        gitcomet_core::filesystem::Request,
    >,
    pub progress: Option<gitcomet_core::filesystem::Progress>,
    pub completed: std::collections::VecDeque<gitcomet_core::filesystem::OperationResult>,
    pub undo_available: bool,
    pub redo_available: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum BranchExistsPromptOperation {
    CreateBranch,
    CheckoutRemoteBranch { remote: String, branch: String },
    RenameBranch { old_name: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchExistsPromptState {
    pub repo_id: RepoId,
    pub name: String,
    pub target: String,
    pub operation: BranchExistsPromptOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthPromptKind {
    UsernamePassword,
    Passphrase,
    HostVerification,
}

impl AuthPromptKind {
    pub fn requires_username(self) -> bool {
        matches!(self, Self::UsernamePassword)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthRetryOperation {
    RepoCommand {
        repo_id: RepoId,
        command: RepoCommandKind,
    },
    SafePushAfterCommit {
        repo_id: RepoId,
        context: SafePushAfterCommitContext,
    },
    Commit {
        repo_id: RepoId,
        message: String,
        amend: bool,
        push_after_commit: bool,
    },
    Clone {
        url: String,
        dest: PathBuf,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthPromptState {
    pub kind: AuthPromptKind,
    pub reason: String,
    pub operation: AuthRetryOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmoduleTrustPromptOperation {
    Add {
        url: String,
        path: PathBuf,
        branch: Option<String>,
        name: Option<String>,
        force: bool,
    },
    Update,
    Load {
        path: PathBuf,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmoduleTrustPromptState {
    pub repo_id: RepoId,
    pub operation: SubmoduleTrustPromptOperation,
    pub sources: Vec<SubmoduleTrustTarget>,
}

/// Which pending action a background trust check belongs to. Mirrors the
/// operation so the spinner's title matches the trust dialog that may follow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmoduleTrustCheckOperation {
    Add,
    Update,
    Load,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubmoduleTrustCheckState {
    pub repo_id: RepoId,
    pub operation: SubmoduleTrustCheckOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppNotification {
    pub time: SystemTime,
    pub kind: AppNotificationKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppNotificationKind {
    Info,
    Success,
    Warning,
    Error,
}
