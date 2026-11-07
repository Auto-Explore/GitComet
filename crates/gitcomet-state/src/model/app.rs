//! App-wide state: settings, prompts, and notifications.

use super::{CloneOpState, RepoId, RepoState};
use crate::msg::RepoCommandKind;
use gitcomet_core::domain::SignatureFormats;
use gitcomet_core::process::GitRuntimeState;
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::services::{HistoryRefFilter, SafePushAfterCommitContext, SubmoduleTrustTarget};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RemoteSettings {
    pub prune_deleted_remote_branches_on_fetch: bool,
    pub auto_fetch: super::AutoFetchSettings,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            prune_deleted_remote_branches_on_fetch: true,
            auto_fetch: super::AutoFetchSettings::default(),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubmoduleSettings {
    /// After a checkout, pull, reset or stash pop moves a first-level
    /// submodule's recorded commit, check the submodule out there too when it
    /// was in sync before. Git's own `submodule.recurse` takes precedence.
    pub follow_pointers: bool,
}

impl Default for SubmoduleSettings {
    fn default() -> Self {
        Self {
            follow_pointers: true,
        }
    }
}

/// What a submodule checkout prompt will run once the user picks how to keep
/// the work the guard found.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SubmoduleCheckoutOperation {
    /// Discard these paths in the parent; at least one restores a pointer.
    Discard { paths: Vec<PathBuf> },
    /// Check a submodule out at the commit its parent records.
    UpdateToRecorded { path: PathBuf },
}

/// A checkout the guard stopped because it could lose work in a submodule.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SubmoduleCheckoutPrompt {
    pub generation: u64,
    pub repo_id: RepoId,
    pub operation: SubmoduleCheckoutOperation,
    pub risks: Vec<gitcomet_core::domain::SubmoduleCheckoutRisk>,
}

/// Where a commit made in a submodule tab, recorded in its parent, stands.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum LinkedCommitStage {
    /// Committing in the submodule.
    CommitChild,
    /// Recording the new submodule commit in the parent.
    RecordInParent,
    /// Pushing the submodule; the parent waits for it.
    PushChild,
    /// Pushing the parent after the submodule.
    PushParent,
    /// The submodule push was refused; waiting for the user's choice.
    PushStopped,
    /// Pulling the submodule before pushing again.
    ReconcileChild { rebase: bool },
    /// Recording the rebased submodule commit in the parent's commit.
    RerecordInParent,
}

/// A commit in a first-level submodule tab that the parent records too, and
/// pushes after the submodule, so the parent never records a commit nobody
/// else can fetch.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LinkedCommit {
    pub child: RepoId,
    pub parent: RepoId,
    /// The submodule's path in the parent.
    pub path: PathBuf,
    pub parent_message: String,
    pub push: bool,
    pub stage: LinkedCommitStage,
    /// The parent commit that records the submodule, once made.
    pub parent_commit: Option<gitcomet_core::domain::CommitId>,
}

/// The message a parent's commit recording a submodule starts with: the
/// submodule's path and the first line of the submodule's message.
pub fn default_parent_message(path: &std::path::Path, child_message: &str) -> String {
    let subject = child_message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if subject.is_empty() {
        format!("Update {}", path.display())
    } else {
        format!("Update {}: {subject}", path.display())
    }
}

/// The submodule push of a linked commit was refused, so the parent's push
/// is held back.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LinkedPushPrompt {
    pub generation: u64,
    pub child: RepoId,
    pub parent: RepoId,
    pub error: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LinkedPushChoice {
    /// Leave both commits local and push later.
    KeepLocal,
    /// Pull the submodule with a rebase, record the rebased commit in the
    /// parent's unpushed commit, and push both again.
    Rebase,
    /// Pull the submodule with a merge and push both again.
    Merge,
}

/// The push the unpushed-pointer guard stopped, so that "Push anyway" and
/// "Push submodules first" run it again as it was asked for.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum BlockedPush {
    Push,
    AfterCommit {
        target: gitcomet_core::services::SafePushAfterCommitTarget,
        set_upstream: bool,
    },
    /// The first push of a branch, which also sets its upstream.
    SetUpstream {
        remote: String,
        branch: String,
    },
    /// A force push, with the lease it was asked with if any.
    Force {
        lease: Option<gitcomet_core::services::ForcePushLease>,
    },
    WithTags {
        request: gitcomet_core::tag_push::TagPushRequest,
    },
}

/// A push refused because outgoing commits record submodule commits that
/// the submodules' remotes do not have.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct UnpushedPointerPrompt {
    pub generation: u64,
    pub repo_id: RepoId,
    pub pointers: Vec<gitcomet_core::domain::SubmoduleCheckoutUpdate>,
    pub push: BlockedPush,
}

/// Changes to tracked files in first-level submodules that a commit in the
/// parent stashes first, under `message`, instead of leaving them behind.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SubmoduleShelve {
    pub paths: Vec<PathBuf>,
    pub message: String,
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
    /// Notifications the window has not acknowledged yet
    /// (`Msg::AcknowledgeNotifications`).
    pub notifications: Vec<AppNotification>,
    /// Number pushed, including notifications already acknowledged and dropped.
    pub notifications_seq: u64,
    pub auth_prompt: Option<AuthPromptState>,
    /// Every new authentication request has a distinct identity, including
    /// repeated requests for the same key and operation.
    pub auth_prompt_generation: u64,
    /// Cancels local key validation; submitted secrets live only in its effect.
    pub auth_validation_cancellation: Option<gitcomet_core::services::CancellationToken>,
    pub branch_exists_prompt: Option<BranchExistsPromptState>,
    pub force_delete_branch_prompt: Option<ForceDeleteBranchPrompt>,
    /// Every force-delete prompt has a distinct generation, so a refusal that
    /// repeats the last one is still a new prompt.
    pub force_delete_branch_prompt_generation: u64,
    pub force_remove_worktree_prompt: Option<ForceRemoveWorktreePrompt>,
    /// Every force prompt has a distinct generation, so a refusal that
    /// repeats the last one is still a new prompt.
    pub force_remove_worktree_prompt_generation: u64,
    pub submodule_trust_prompt: Option<SubmoduleTrustPromptState>,
    /// A discard or update the submodule guard stopped, waiting for a choice.
    pub submodule_checkout_prompt: Option<SubmoduleCheckoutPrompt>,
    pub submodule_checkout_prompt_generation: u64,
    pub submodule_settings: SubmoduleSettings,
    /// The linked commit in progress; one at a time.
    pub linked_commit: Option<LinkedCommit>,
    pub linked_push_prompt: Option<LinkedPushPrompt>,
    pub linked_push_prompt_generation: u64,
    pub unpushed_pointer_prompt: Option<UnpushedPointerPrompt>,
    pub unpushed_pointer_prompt_generation: u64,
    /// The next push of this repository skips the unpushed-submodule check:
    /// the user chose to push anyway, or the submodules were pushed first.
    pub unpushed_pointer_override: Option<RepoId>,
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
    /// Ref namespaces every all-branches walk leaves out, resolved from
    /// [`HiddenRefSettings`] against the product's presets.
    pub history_hidden_refs: HistoryRefFilter,
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

    /// Open first-level submodule tabs of the worktree tab `repo_id`, in tab
    /// order.
    pub fn submodule_children(&self, repo_id: RepoId) -> Vec<RepoId> {
        self.repos
            .iter()
            .filter(|repo| repo.submodule_parent_tab() == Some(repo_id))
            .map(|repo| repo.id)
            .collect()
    }

    /// The folder name the tab of `repo_id` shows, for prompts and notices.
    pub fn repo_name(&self, repo_id: RepoId) -> String {
        self.repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.spec.workdir.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "the repository".to_string())
    }

    /// The row the parent tab lists for the first-level submodule tab
    /// `repo_id`: what the parent records for it and how far it has moved.
    pub fn submodule_row_in_parent(
        &self,
        repo_id: RepoId,
    ) -> Option<&gitcomet_core::domain::Submodule> {
        let child = self.repos.iter().find(|repo| repo.id == repo_id)?;
        let link = child.submodule_parent.as_ref()?;
        let parent = self
            .repos
            .iter()
            .find(|repo| Some(repo.id) == link.repo_id)?;
        let crate::model::Loadable::Ready(rows) = &parent.submodules else {
            return None;
        };
        rows.iter().find(|row| !row.nested && row.path == link.path)
    }

    /// Queues a notification for the window.
    pub fn push_notification(&mut self, notification: AppNotification) {
        self.notifications_seq = self.notifications_seq.wrapping_add(1);
        self.notifications.push(notification);
    }

    /// Notifications queued after notification number `seq`.
    pub fn notifications_after(&self, seq: u64) -> &[AppNotification] {
        super::feedback_after(&self.notifications, self.notifications_seq, seq)
    }

    /// Releases the notifications the window has shown, up to number
    /// `through_seq`.
    pub fn acknowledge_notifications(&mut self, through_seq: u64) {
        super::acknowledge_feedback(&mut self.notifications, self.notifications_seq, through_seq);
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
    pub pending: std::collections::BTreeSet<gitcomet_core::filesystem::OperationId>,
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

/// `git branch -d` refused a branch that is not fully merged.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ForceDeleteBranchPrompt {
    pub generation: u64,
    pub repo_id: RepoId,
    pub name: String,
}

/// A plain worktree removal Git refused for a reason `--force` overrides.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ForceRemoveWorktreePrompt {
    pub generation: u64,
    pub repo_id: RepoId,
    pub path: PathBuf,
    /// The local branch to delete once the worktree is gone.
    pub delete_branch: Option<String>,
    pub blocker: gitcomet_core::error::WorktreeRemovalBlocker,
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
    pub generation: u64,
    pub challenge: Option<gitcomet_core::auth::AuthChallenge>,
    pub status: AuthPromptStatus,
    pub kind: AuthPromptKind,
    pub reason: String,
    pub operation: AuthRetryOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthPromptStatus {
    AwaitingInput,
    Validating,
    Rejected,
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
