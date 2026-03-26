use crate::model::{
    AppNotification, AppNotificationKind, AppState, AuthPromptKind, CommandLogEntry,
    ConflictFileLoadMode, DiagnosticEntry, DiagnosticKind, Loadable, RepoId, RepoLoadsInFlight,
    RepoState,
};
use crate::msg::{ConflictAutosolveMode, ConflictAutosolveStats, Effect, RepoCommandKind};
#[cfg(test)]
use gitcomet_core::auth::stage_git_auth;
use gitcomet_core::auth::{GitAuthKind, StagedGitAuth, clear_staged_git_auth};
use gitcomet_core::domain::{DiffArea, DiffTarget, FileStatusKind};
use gitcomet_core::error::{Error, ErrorKind, GitFailure};
use gitcomet_core::services::CommandOutput;
use rustc_hash::FxHashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Default page size for log fetches.
pub(super) const DEFAULT_LOG_PAGE_SIZE: usize = 200;

fn is_supported_image_path(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    ext.eq_ignore_ascii_case("png")
        || ext.eq_ignore_ascii_case("jpg")
        || ext.eq_ignore_ascii_case("jpeg")
        || ext.eq_ignore_ascii_case("gif")
        || ext.eq_ignore_ascii_case("webp")
        || ext.eq_ignore_ascii_case("bmp")
        || ext.eq_ignore_ascii_case("ico")
        || ext.eq_ignore_ascii_case("svg")
        || ext.eq_ignore_ascii_case("tif")
        || ext.eq_ignore_ascii_case("tiff")
}

fn is_svg_path(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
}

pub(super) fn diff_target_wants_image_preview(target: &DiffTarget) -> bool {
    match target {
        DiffTarget::WorkingTree { path, .. } => is_supported_image_path(path),
        DiffTarget::Commit {
            path: Some(path), ..
        } => is_supported_image_path(path),
        _ => false,
    }
}

pub(super) fn diff_target_is_svg(target: &DiffTarget) -> bool {
    match target {
        DiffTarget::WorkingTree { path, .. } => is_svg_path(path),
        DiffTarget::Commit {
            path: Some(path), ..
        } => is_svg_path(path),
        _ => false,
    }
}

pub(super) fn selected_conflict_target_path(
    repo_state: &RepoState,
    target: &DiffTarget,
) -> Option<PathBuf> {
    let DiffTarget::WorkingTree { path, area } = target else {
        return None;
    };
    if *area != DiffArea::Unstaged {
        return None;
    }

    if repo_state.conflict_state.conflict_file_path.as_deref() == Some(path.as_path()) {
        return Some(path.clone());
    }

    let Loadable::Ready(status) = &repo_state.status else {
        return None;
    };
    status
        .unstaged
        .iter()
        .find(|entry| entry.path == *path && entry.kind == FileStatusKind::Conflicted)
        .map(|_| path.clone())
}

pub(super) fn current_conflict_load_mode(repo_state: &RepoState) -> ConflictFileLoadMode {
    repo_state.conflict_state.conflict_file_load_mode
}

pub(super) fn start_conflict_target_reload(
    repo_state: &mut RepoState,
    path: PathBuf,
) -> Vec<Effect> {
    let mode = current_conflict_load_mode(repo_state);
    start_conflict_target_reload_with_mode(repo_state, path, mode)
}

pub(super) fn start_conflict_target_reload_with_mode(
    repo_state: &mut RepoState,
    path: PathBuf,
    mode: ConflictFileLoadMode,
) -> Vec<Effect> {
    repo_state.set_conflict_file_path(Some(path.clone()));
    repo_state.set_conflict_file_load_mode(mode);
    repo_state.set_conflict_file(Loadable::Loading);
    repo_state.set_conflict_session(None);
    repo_state.set_conflict_hide_resolved(false);
    vec![Effect::LoadConflictFile {
        repo_id: repo_state.id,
        path,
        mode,
    }]
}

pub(super) fn diff_reload_effects(repo_id: RepoId, target: DiffTarget) -> Vec<Effect> {
    let supports_file = matches!(
        &target,
        DiffTarget::WorkingTree { .. } | DiffTarget::Commit { path: Some(_), .. }
    );
    let wants_image = diff_target_wants_image_preview(&target);
    let is_svg = diff_target_is_svg(&target);

    let mut effects = vec![Effect::LoadDiff {
        repo_id,
        target: target.clone(),
    }];
    if supports_file {
        if wants_image {
            effects.push(Effect::LoadDiffFileImage {
                repo_id,
                target: target.clone(),
            });
        }
        if !wants_image || is_svg {
            effects.push(Effect::LoadDiffFile { repo_id, target });
        }
    }

    effects
}

pub(super) fn refresh_primary_effects(repo_state: &mut RepoState) -> Vec<Effect> {
    let repo_id = repo_state.id;
    let mut effects = Vec::new();

    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::HEAD_BRANCH)
    {
        effects.push(Effect::LoadHeadBranch { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::UPSTREAM_DIVERGENCE)
    {
        effects.push(Effect::LoadUpstreamDivergence { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::REBASE_STATE)
    {
        effects.push(Effect::LoadRebaseState { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::MERGE_COMMIT_MESSAGE)
    {
        effects.push(Effect::LoadMergeCommitMessage { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::STATUS)
    {
        effects.push(Effect::LoadStatus { repo_id });
    }
    if repo_state.loads_in_flight.request_log(
        repo_state.history_state.history_scope,
        DEFAULT_LOG_PAGE_SIZE,
        None,
    ) {
        // Block pagination while a refresh log load is in flight, to avoid concurrent LogLoaded
        // merges with different cursors.
        repo_state.set_log_loading_more(false);
        effects.push(Effect::LoadLog {
            repo_id,
            scope: repo_state.history_state.history_scope,
            limit: DEFAULT_LOG_PAGE_SIZE,
            cursor: None,
        });
    }

    effects
}

pub(super) fn refresh_full_effects(repo_state: &mut RepoState) -> Vec<Effect> {
    let repo_id = repo_state.id;
    let mut effects = Vec::new();

    // Prioritize UI-critical loads (status + log) early. The executor is a FIFO queue, so this
    // ordering can materially impact perceived responsiveness when switching repositories.
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::HEAD_BRANCH)
    {
        effects.push(Effect::LoadHeadBranch { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::UPSTREAM_DIVERGENCE)
    {
        effects.push(Effect::LoadUpstreamDivergence { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::STATUS)
    {
        effects.push(Effect::LoadStatus { repo_id });
    }
    if repo_state.loads_in_flight.request_log(
        repo_state.history_state.history_scope,
        DEFAULT_LOG_PAGE_SIZE,
        None,
    ) {
        repo_state.set_log_loading_more(false);
        effects.push(Effect::LoadLog {
            repo_id,
            scope: repo_state.history_state.history_scope,
            limit: DEFAULT_LOG_PAGE_SIZE,
            cursor: None,
        });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::BRANCHES)
    {
        effects.push(Effect::LoadBranches { repo_id });
    }
    if repo_state.loads_in_flight.request(RepoLoadsInFlight::TAGS) {
        effects.push(Effect::LoadTags { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::REMOTE_TAGS)
    {
        effects.push(Effect::LoadRemoteTags { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::REMOTES)
    {
        effects.push(Effect::LoadRemotes { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::REMOTE_BRANCHES)
    {
        effects.push(Effect::LoadRemoteBranches { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::REBASE_STATE)
    {
        effects.push(Effect::LoadRebaseState { repo_id });
    }
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::MERGE_COMMIT_MESSAGE)
    {
        effects.push(Effect::LoadMergeCommitMessage { repo_id });
    }

    effects
}

pub(super) fn dedup_paths_in_order(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(paths.len());
    let mut seen: FxHashSet<PathBuf> = FxHashSet::default();
    for p in paths {
        if !seen.insert(p.clone()) {
            continue;
        }
        out.push(p);
    }
    out
}

pub(super) fn normalize_repo_path(path: PathBuf) -> PathBuf {
    let path = if path.is_relative() {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    } else {
        path
    };

    canonicalize_path(path)
}

pub(super) fn canonicalize_path(path: PathBuf) -> PathBuf {
    super::super::canonicalize_path(path)
}

pub(super) fn push_notification(state: &mut AppState, kind: AppNotificationKind, message: String) {
    const MAX_NOTIFICATIONS: usize = 200;
    state.notifications.push(AppNotification {
        time: SystemTime::now(),
        kind,
        message,
    });
    if state.notifications.len() > MAX_NOTIFICATIONS {
        let extra = state.notifications.len() - MAX_NOTIFICATIONS;
        state.notifications.drain(0..extra);
    }
}

pub(super) fn clear_banner_error_for_repo(state: &mut AppState, repo_id: RepoId) {
    if state
        .banner_error
        .as_ref()
        .is_some_and(|banner| banner.repo_id == Some(repo_id))
    {
        state.banner_error = None;
    }
}

pub(super) fn push_diagnostic(repo_state: &mut RepoState, kind: DiagnosticKind, message: String) {
    const MAX_DIAGNOSTICS: usize = 200;
    repo_state.diagnostics.push(DiagnosticEntry {
        time: SystemTime::now(),
        kind,
        message,
    });
    if repo_state.diagnostics.len() > MAX_DIAGNOSTICS {
        let extra = repo_state.diagnostics.len() - MAX_DIAGNOSTICS;
        repo_state.diagnostics.drain(0..extra);
    }
}

pub(super) fn handle_session_persist_result(
    state: &mut AppState,
    repo_id: Option<RepoId>,
    action: &'static str,
    result: io::Result<()>,
) {
    let Err(error) = result else {
        return;
    };
    let message = format!("Failed to persist session state while {action}: {error}");
    push_notification(state, AppNotificationKind::Error, message.clone());
    if let Some(repo_id) = repo_id
        && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
    {
        push_diagnostic(repo_state, DiagnosticKind::Error, message);
    }
}

pub(super) fn push_command_log(
    repo_state: &mut RepoState,
    ok: bool,
    command: &RepoCommandKind,
    output: &CommandOutput,
    error: Option<&Error>,
) {
    const MAX_COMMAND_LOG: usize = 200;

    let (command_text, summary) = summarize_command(command, output, ok, error);

    repo_state.command_log.push(CommandLogEntry {
        time: SystemTime::now(),
        ok,
        command: command_text,
        summary,
        stdout: output.stdout.clone(),
        stderr: if output.stderr.is_empty() {
            error.map(format_error_for_user).unwrap_or_default()
        } else {
            output.stderr.clone()
        },
    });
    if repo_state.command_log.len() > MAX_COMMAND_LOG {
        let extra = repo_state.command_log.len() - MAX_COMMAND_LOG;
        repo_state.command_log.drain(0..extra);
    }
}

pub(super) fn push_action_log(
    repo_state: &mut RepoState,
    ok: bool,
    command: String,
    summary: String,
    error: Option<&Error>,
) {
    const MAX_COMMAND_LOG: usize = 200;

    repo_state.command_log.push(CommandLogEntry {
        time: SystemTime::now(),
        ok,
        command,
        summary,
        stdout: String::new(),
        stderr: error.map(format_error_for_user).unwrap_or_default(),
    });
    if repo_state.command_log.len() > MAX_COMMAND_LOG {
        let extra = repo_state.command_log.len() - MAX_COMMAND_LOG;
        repo_state.command_log.drain(0..extra);
    }
}

pub(super) fn conflict_autosolve_telemetry_command(
    mode: ConflictAutosolveMode,
    path: Option<&Path>,
) -> String {
    let mut command = format!("telemetry.conflict_autosolve.{}", mode.as_str());
    if let Some(path) = path {
        command.push(' ');
        command.push_str(
            &path
                .to_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("{path:?}")),
        );
    }
    command
}

pub(super) fn conflict_autosolve_telemetry_summary(
    mode: ConflictAutosolveMode,
    path: Option<&Path>,
    total_conflicts_before: usize,
    total_conflicts_after: usize,
    unresolved_before: usize,
    unresolved_after: usize,
    stats: ConflictAutosolveStats,
) -> String {
    let resolved = stats.total_resolved();
    let mode_label = match mode {
        ConflictAutosolveMode::Safe => "safe",
        ConflictAutosolveMode::Regex => "regex",
        ConflictAutosolveMode::History => "history",
    };

    let path_label = path
        .map(|p| format!(" in {}", p.display()))
        .unwrap_or_default();

    let mut details = Vec::new();
    if stats.pass1 > 0 {
        details.push(format!("pass1={}", stats.pass1));
    }
    if stats.pass2_split > 0 {
        details.push(format!("pass2_split={}", stats.pass2_split));
    }
    if stats.pass1_after_split > 0 {
        details.push(format!("pass1_after_split={}", stats.pass1_after_split));
    }
    if stats.regex > 0 {
        details.push(format!("regex={}", stats.regex));
    }
    if stats.history > 0 {
        details.push(format!("history={}", stats.history));
    }
    let details = if details.is_empty() {
        "details=none".to_string()
    } else {
        details.join(", ")
    };

    format!(
        "Conflict autosolve ({mode_label}): resolved {resolved}; unresolved {unresolved_before} -> {unresolved_after}; conflicts {total_conflicts_before} -> {total_conflicts_after}{path_label} ({details})"
    )
}

fn summarize_command(
    command: &RepoCommandKind,
    output: &CommandOutput,
    ok: bool,
    error: Option<&Error>,
) -> (String, String) {
    use gitcomet_core::services::ConflictSide;

    if !ok {
        let label = match command {
            RepoCommandKind::FetchAll => "Fetch",
            RepoCommandKind::PruneMergedBranches => "Prune merged branches",
            RepoCommandKind::PruneLocalTags => "Prune local tags",
            RepoCommandKind::Pull { .. } => "Pull",
            RepoCommandKind::PullBranch { .. } => "Pull",
            RepoCommandKind::MergeRef { .. } => "Merge",
            RepoCommandKind::SquashRef { .. } => "Squash",
            RepoCommandKind::Push => "Push",
            RepoCommandKind::ForcePush => "Force push",
            RepoCommandKind::PushSetUpstream { .. } => "Push",
            RepoCommandKind::SetUpstreamBranch { .. } => "Set as tracking upstream",
            RepoCommandKind::UnsetUpstreamBranch { .. } => "Unlink upstream branch",
            RepoCommandKind::DeleteRemoteBranch { .. } => "Delete remote branch",
            RepoCommandKind::PushTag { .. } => "Push tag",
            RepoCommandKind::DeleteRemoteTag { .. } => "Delete remote tag",
            RepoCommandKind::Reset { .. } => "Reset",
            RepoCommandKind::Rebase { .. } => "Rebase",
            RepoCommandKind::RebaseContinue => "Rebase",
            RepoCommandKind::RebaseAbort => "Rebase",
            RepoCommandKind::MergeAbort => "Merge",
            RepoCommandKind::CreateTag { .. } => "Tag",
            RepoCommandKind::DeleteTag { .. } => "Tag",
            RepoCommandKind::AddRemote { .. } => "Remote",
            RepoCommandKind::RemoveRemote { .. } => "Remote",
            RepoCommandKind::SetRemoteUrl { .. } => "Remote",
            RepoCommandKind::CheckoutConflict { side, .. } => match side {
                ConflictSide::Ours => "Checkout ours",
                ConflictSide::Theirs => "Checkout theirs",
            },
            RepoCommandKind::AcceptConflictDeletion { .. } => "Accept deletion",
            RepoCommandKind::CheckoutConflictBase { .. } => "Checkout base",
            RepoCommandKind::LaunchMergetool { .. } => "Mergetool",
            RepoCommandKind::SaveWorktreeFile { .. } => "Save file",
            RepoCommandKind::ExportPatch { .. } | RepoCommandKind::ApplyPatch { .. } => "Patch",
            RepoCommandKind::AddWorktree { .. }
            | RepoCommandKind::RemoveWorktree { .. }
            | RepoCommandKind::ForceRemoveWorktree { .. } => "Worktree",
            RepoCommandKind::AddSubmodule { .. }
            | RepoCommandKind::UpdateSubmodules
            | RepoCommandKind::RemoveSubmodule { .. } => "Submodule",
            RepoCommandKind::StageHunk | RepoCommandKind::UnstageHunk => "Hunk",
            RepoCommandKind::ApplyWorktreePatch { reverse } => {
                if *reverse {
                    "Discard"
                } else {
                    "Patch"
                }
            }
        };
        if let Some(error) = error
            && let Some((git_command, details)) = try_format_git_backend_error(error)
        {
            return (git_command, format!("{label} failed:\n\n{details}"));
        }

        return (
            output.command.clone().if_empty_else(|| label.to_string()),
            error
                .map(|e| format!("{label} failed:\n\n{}", format_error_for_user(e)))
                .unwrap_or_else(|| format!("{label} failed")),
        );
    }

    let summary = match command {
        RepoCommandKind::FetchAll => {
            if output.stderr.trim().is_empty() && output.stdout.trim().is_empty() {
                "Fetch: Already up to date".to_string()
            } else {
                "Fetch: Synchronized".to_string()
            }
        }
        RepoCommandKind::PruneMergedBranches => "Prune merged branches: Completed".to_string(),
        RepoCommandKind::PruneLocalTags => "Prune local tags: Completed".to_string(),
        RepoCommandKind::Pull { .. } => {
            if output.stdout.contains("Already up to date") {
                "Pull: Already up to date".to_string()
            } else if output.stdout.starts_with("Updating") {
                "Pull: Fast-forwarded".to_string()
            } else if output.stdout.starts_with("Merge") {
                "Pull: Merged".to_string()
            } else if output.stdout.contains("Successfully rebased") {
                "Pull: Rebasing complete".to_string()
            } else {
                "Pull: Completed".to_string()
            }
        }
        RepoCommandKind::PullBranch { remote, branch } => {
            let base = if output.stdout.contains("Already up to date") {
                "Already up to date"
            } else if output.stdout.starts_with("Updating") {
                "Fast-forwarded"
            } else if output.stdout.starts_with("Merge") {
                "Merged"
            } else {
                "Completed"
            };
            format!("Pull {remote}/{branch}: {base}")
        }
        RepoCommandKind::MergeRef { reference } => {
            let base = if output.stdout.contains("Already up to date") {
                "Already up to date"
            } else if output.stdout.contains("Fast-forward")
                || output.stdout.starts_with("Updating")
            {
                "Fast-forwarded"
            } else if output.stdout.contains("Merge made by") {
                "Merged"
            } else {
                "Completed"
            };
            format!("Merge {reference}: {base}")
        }
        RepoCommandKind::SquashRef { reference } => {
            let base = if output.stdout.contains("Already up to date") {
                "Already up to date"
            } else if output.stdout.contains("Squash commit -- not updating HEAD")
                || output
                    .stdout
                    .contains("Automatic merge went well; stopped before committing as requested")
            {
                "Staged"
            } else {
                "Completed"
            };
            format!("Squash {reference}: {base}")
        }
        RepoCommandKind::Push => {
            if output.stderr.contains("Everything up-to-date") {
                "Push: Everything up-to-date".to_string()
            } else {
                "Push: Completed".to_string()
            }
        }
        RepoCommandKind::ForcePush => {
            if output.stderr.contains("Everything up-to-date") {
                "Force push: Everything up-to-date".to_string()
            } else {
                "Force push: Completed".to_string()
            }
        }
        RepoCommandKind::PushSetUpstream { remote, branch } => {
            let base = if output.stderr.contains("Everything up-to-date") {
                "Everything up-to-date"
            } else {
                "Completed"
            };
            format!("Push -u {remote}/{branch}: {base}")
        }
        RepoCommandKind::SetUpstreamBranch { branch, upstream } => {
            format!("Branch {branch}: Upstream set to {upstream}")
        }
        RepoCommandKind::UnsetUpstreamBranch { branch } => {
            format!("Branch {branch}: Upstream unlinked")
        }
        RepoCommandKind::DeleteRemoteBranch { remote, branch } => {
            format!("Remote branch {remote}/{branch}: Deleted")
        }
        RepoCommandKind::PushTag { remote, name } => {
            if output.stderr.contains("Everything up-to-date") {
                format!("Tag {name} → {remote}: Already up-to-date")
            } else {
                format!("Tag {name} → {remote}: Pushed")
            }
        }
        RepoCommandKind::DeleteRemoteTag { remote, name } => {
            format!("Tag {name} on {remote}: Deleted")
        }
        RepoCommandKind::CheckoutConflict { side, .. } => match side {
            ConflictSide::Ours => "Resolved using ours".to_string(),
            ConflictSide::Theirs => "Resolved using theirs".to_string(),
        },
        RepoCommandKind::AcceptConflictDeletion { path } => {
            format!("Resolved by accepting deletion → {}", path.display())
        }
        RepoCommandKind::CheckoutConflictBase { path } => {
            format!("Resolved using base → {}", path.display())
        }
        RepoCommandKind::LaunchMergetool { path } => {
            format!("Mergetool: Resolved {}", path.display())
        }
        RepoCommandKind::SaveWorktreeFile { path, stage } => {
            if *stage {
                format!("Saved and staged → {}", path.display())
            } else {
                format!("Saved → {}", path.display())
            }
        }
        RepoCommandKind::Reset { mode, target } => {
            let mode = match mode {
                gitcomet_core::services::ResetMode::Soft => "soft",
                gitcomet_core::services::ResetMode::Mixed => "mixed",
                gitcomet_core::services::ResetMode::Hard => "hard",
            };
            format!("Reset (--{mode}) {target}: Completed")
        }
        RepoCommandKind::Rebase { onto } => format!("Rebase onto {onto}: Completed"),
        RepoCommandKind::RebaseContinue => "Rebase: Continued".to_string(),
        RepoCommandKind::RebaseAbort => "Rebase: Aborted".to_string(),
        RepoCommandKind::MergeAbort => "Merge: Aborted".to_string(),
        RepoCommandKind::CreateTag { name, target } => format!("Tag {name} → {target}: Created"),
        RepoCommandKind::DeleteTag { name } => format!("Tag {name}: Deleted"),
        RepoCommandKind::AddRemote { name, .. } => format!("Remote {name}: Added"),
        RepoCommandKind::RemoveRemote { name } => format!("Remote {name}: Removed"),
        RepoCommandKind::SetRemoteUrl { name, kind, .. } => {
            let kind = match kind {
                gitcomet_core::services::RemoteUrlKind::Fetch => "fetch",
                gitcomet_core::services::RemoteUrlKind::Push => "push",
            };
            format!("Remote {name} ({kind}): URL updated")
        }
        RepoCommandKind::ExportPatch { dest, .. } => {
            format!("Patch exported → {}", dest.display())
        }
        RepoCommandKind::ApplyPatch { patch } => format!("Patch applied → {}", patch.display()),
        RepoCommandKind::AddWorktree { path, reference } => {
            if let Some(reference) = reference {
                format!("Worktree added → {} ({reference})", path.display())
            } else {
                format!("Worktree added → {}", path.display())
            }
        }
        RepoCommandKind::RemoveWorktree { path } => {
            format!("Worktree removed → {}", path.display())
        }
        RepoCommandKind::ForceRemoveWorktree { path } => {
            format!("Worktree force removed → {}", path.display())
        }
        RepoCommandKind::AddSubmodule { path, .. } => {
            format!("Submodule added → {}", path.display())
        }
        RepoCommandKind::UpdateSubmodules => "Submodules: Updated".to_string(),
        RepoCommandKind::RemoveSubmodule { path } => {
            format!("Submodule removed → {}", path.display())
        }
        RepoCommandKind::StageHunk => "Hunk staged".to_string(),
        RepoCommandKind::UnstageHunk => "Hunk unstaged".to_string(),
        RepoCommandKind::ApplyWorktreePatch { reverse } => {
            if *reverse {
                "Changes discarded".to_string()
            } else {
                "Patch applied".to_string()
            }
        }
    };

    (output.command.clone(), summary)
}

pub(super) fn format_error_for_user(error: &Error) -> String {
    match error.kind() {
        ErrorKind::Git(failure) => failure.to_string(),
        ErrorKind::Backend(message) => message.clone(),
        _ => error.to_string(),
    }
}

pub(super) fn format_failure_summary(label: &str, error: &Error) -> String {
    if let Some((_git_command, details)) = try_format_git_backend_error(error) {
        return format!("{label} failed:\n\n{details}");
    }
    format!("{label} failed:\n\n{}", format_error_for_user(error))
}

pub(super) fn detect_auth_prompt_kind(error: &Error) -> Option<AuthPromptKind> {
    match error.kind() {
        ErrorKind::Git(failure) => detect_auth_prompt_kind_from_git_failure(failure),
        ErrorKind::Backend(message) => detect_auth_prompt_kind_from_message(message),
        _ => None,
    }
}

pub(super) fn detect_auth_prompt_kind_from_message(message: &str) -> Option<AuthPromptKind> {
    let lower = message.to_ascii_lowercase();

    let host_verification = lower.contains("host key verification failed")
        || lower.contains("the authenticity of host")
        || lower.contains("this key is not known by any other names")
        || (lower.contains("are you sure you want to continue connecting")
            && lower.contains("yes/no"));
    if host_verification {
        return Some(AuthPromptKind::HostVerification);
    }

    let passphrase = lower.contains("could not read passphrase")
        || lower.contains("enter passphrase for key")
        || lower.contains("read_passphrase")
        || lower.contains("passphrase for key")
        || (lower.contains("passphrase") && lower.contains("terminal prompts disabled"));
    let ssh_publickey = lower.contains("permission denied (publickey")
        || (lower.contains("could not read from remote repository") && lower.contains("publickey"));
    if passphrase || ssh_publickey {
        return Some(AuthPromptKind::Passphrase);
    }

    let user_password = lower.contains("could not read username")
        || lower.contains("could not read password")
        || lower.contains("authentication failed")
        || lower.contains("invalid username or password")
        || lower.contains("http basic: access denied")
        || (lower.contains("terminal prompts disabled")
            && (lower.contains("https://")
                || lower.contains("http://")
                || lower.contains("username")
                || lower.contains("password")));
    if user_password {
        return Some(AuthPromptKind::UsernamePassword);
    }

    None
}

pub(super) fn clear_staged_git_auth_env() {
    clear_staged_git_auth();
}

pub(super) fn prepare_staged_git_auth(
    kind: AuthPromptKind,
    username: Option<&str>,
    secret: &str,
) -> Result<StagedGitAuth, Error> {
    let normalized_secret = match kind {
        AuthPromptKind::HostVerification => {
            let trimmed = secret.trim();
            if trimmed.eq_ignore_ascii_case("yes") {
                "yes".to_string()
            } else {
                trimmed.to_string()
            }
        }
        AuthPromptKind::UsernamePassword | AuthPromptKind::Passphrase => secret.to_string(),
    };

    if normalized_secret.trim().is_empty() {
        return Err(Error::new(ErrorKind::Backend(
            "credential/passphrase/confirmation cannot be empty".to_string(),
        )));
    }
    if kind.requires_username() && username.unwrap_or_default().trim().is_empty() {
        return Err(Error::new(ErrorKind::Backend(
            "username cannot be empty".to_string(),
        )));
    }

    Ok(StagedGitAuth {
        kind: match kind {
            AuthPromptKind::UsernamePassword => GitAuthKind::UsernamePassword,
            AuthPromptKind::Passphrase => GitAuthKind::Passphrase,
            AuthPromptKind::HostVerification => GitAuthKind::HostVerification,
        },
        username: username.map(ToOwned::to_owned),
        secret: normalized_secret,
    })
}

#[cfg(test)]
pub(super) fn stage_git_auth_env(
    kind: AuthPromptKind,
    username: Option<&str>,
    secret: &str,
) -> Result<(), Error> {
    stage_git_auth(prepare_staged_git_auth(kind, username, secret)?);
    Ok(())
}

fn try_format_git_backend_error(error: &Error) -> Option<(String, String)> {
    match error.kind() {
        ErrorKind::Git(failure) => try_format_structured_git_failure(failure),
        ErrorKind::Backend(message) => try_format_git_backend_error_message(message),
        _ => None,
    }
}

fn try_format_structured_git_failure(failure: &GitFailure) -> Option<(String, String)> {
    let command = failure.command().trim().to_string();
    if !command.starts_with("git ") {
        return None;
    }
    let rendered = render_command_and_output(&command, failure.detail());
    Some((command, rendered))
}

fn detect_auth_prompt_kind_from_git_failure(failure: &GitFailure) -> Option<AuthPromptKind> {
    let stderr = String::from_utf8_lossy(failure.stderr());
    detect_auth_prompt_kind_from_message(&stderr)
        .or_else(|| {
            detect_auth_prompt_kind_from_message(&String::from_utf8_lossy(failure.stdout()))
        })
        .or_else(|| detect_auth_prompt_kind_from_message(&failure.to_string()))
}

fn try_format_git_backend_error_message(message: &str) -> Option<(String, String)> {
    let (command, output) = parse_failed_command_message(message)?;
    if !command.trim_start().starts_with("git ") {
        return None;
    }

    let rendered = render_command_and_output(&command, output.as_deref());
    Some((command, rendered))
}

fn parse_failed_command_message(message: &str) -> Option<(String, Option<String>)> {
    if let Some(idx) = message.find(" failed:") {
        let command = message[..idx].trim_end().to_string();
        let mut output = &message[(idx + " failed:".len())..];
        if output.starts_with(' ') {
            output = &output[1..];
        }
        let output = output.trim_end_matches(['\r', '\n']).to_string();
        return Some((command, (!output.is_empty()).then_some(output)));
    }

    let trimmed = message.trim_end_matches(['\r', '\n']);
    if let Some(command) = trimmed.strip_suffix(" failed") {
        return Some((command.trim_end().to_string(), None));
    }

    None
}

fn render_command_and_output(command: &str, output: Option<&str>) -> String {
    let command = command.replace(['\n', '\r'], " ");
    let command = command.trim();

    let output_len = output.map_or(0, |s| s.len());
    let mut rendered = String::with_capacity(command.len() + output_len + 16);
    append_code_block(&mut rendered, command);

    if let Some(output) = output {
        let output = output.trim_end_matches(['\r', '\n']);
        if !output.is_empty() {
            rendered.push_str("\n\n");
            append_code_block(&mut rendered, output);
        }
    }

    rendered
}

fn append_code_block(out: &mut String, text: &str) {
    for (ix, line) in text.lines().enumerate() {
        if ix > 0 {
            out.push('\n');
        }
        out.push_str("    ");
        out.push_str(line);
    }
}

trait IfEmptyElse {
    fn if_empty_else(self, f: impl FnOnce() -> String) -> String;
}

impl IfEmptyElse for String {
    fn if_empty_else(self, f: impl FnOnce() -> String) -> String {
        if self.trim().is_empty() { f() } else { self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AppNotificationKind, DiagnosticKind};
    use crate::msg::RepoCommandKind;
    use gitcomet_core::domain::{CommitId, DiffArea, DiffTarget, RepoSpec};
    use gitcomet_core::error::{GitFailure, GitFailureId};
    use gitcomet_core::services::{PullMode, RemoteUrlKind, ResetMode};
    use std::path::Path;

    fn repo_state(id: u64) -> RepoState {
        RepoState::new_opening(
            RepoId(id),
            RepoSpec {
                workdir: PathBuf::from("/tmp/gitcomet-state-util-tests"),
            },
        )
    }

    fn command_output(command: &str, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            command: command.to_string(),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code: Some(0),
        }
    }

    fn dummy_log_entry(ix: usize) -> CommandLogEntry {
        CommandLogEntry {
            time: SystemTime::UNIX_EPOCH,
            ok: true,
            command: format!("cmd-{ix}"),
            summary: String::new(),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    #[test]
    fn diff_reload_effects_cover_image_svg_and_non_file_targets() {
        let repo_id = RepoId(7);
        let png = DiffTarget::WorkingTree {
            path: PathBuf::from("img.PNG"),
            area: DiffArea::Unstaged,
        };
        let png_effects = diff_reload_effects(repo_id, png.clone());
        assert!(diff_target_wants_image_preview(&png));
        assert!(!diff_target_is_svg(&png));
        assert_eq!(png_effects.len(), 2);
        assert!(matches!(png_effects[0], Effect::LoadDiff { .. }));
        assert!(matches!(png_effects[1], Effect::LoadDiffFileImage { .. }));

        let svg = DiffTarget::WorkingTree {
            path: PathBuf::from("diagram.svg"),
            area: DiffArea::Unstaged,
        };
        let svg_effects = diff_reload_effects(repo_id, svg.clone());
        assert!(diff_target_wants_image_preview(&svg));
        assert!(diff_target_is_svg(&svg));
        assert_eq!(svg_effects.len(), 3);
        assert!(matches!(svg_effects[2], Effect::LoadDiffFile { .. }));

        let text_no_ext = DiffTarget::WorkingTree {
            path: PathBuf::from("README"),
            area: DiffArea::Unstaged,
        };
        assert!(!diff_target_wants_image_preview(&text_no_ext));
        assert_eq!(diff_reload_effects(repo_id, text_no_ext).len(), 2);

        let commit_without_path = DiffTarget::Commit {
            commit_id: CommitId("abc123".into()),
            path: None,
        };
        assert!(!diff_target_wants_image_preview(&commit_without_path));
        assert!(!diff_target_is_svg(&commit_without_path));
        assert_eq!(diff_reload_effects(repo_id, commit_without_path).len(), 1);
    }

    #[test]
    fn refresh_effects_request_expected_loads_and_reset_log_loading_more() {
        let mut primary = repo_state(1);
        primary.set_log_loading_more(true);
        let primary_effects = refresh_primary_effects(&mut primary);
        assert_eq!(primary_effects.len(), 6);
        assert!(!primary.log_loading_more);
        assert!(matches!(primary_effects[0], Effect::LoadHeadBranch { .. }));
        assert!(matches!(
            primary_effects[5],
            Effect::LoadLog {
                limit: DEFAULT_LOG_PAGE_SIZE,
                ..
            }
        ));

        let mut full = repo_state(2);
        full.set_log_loading_more(true);
        let full_effects = refresh_full_effects(&mut full);
        assert_eq!(full_effects.len(), 11);
        assert!(!full.log_loading_more);
        assert!(
            full_effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadRemoteTags { .. }))
        );
        assert!(
            !full_effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadStashes { .. })),
            "stashes should now lazy-load from the sidebar instead of refresh_full_effects"
        );
        assert!(
            full_effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadMergeCommitMessage { .. }))
        );
    }

    #[test]
    fn dedup_and_normalize_path_cover_duplicate_and_relative_branches() {
        let deduped = dedup_paths_in_order(vec![
            PathBuf::from("a"),
            PathBuf::from("b"),
            PathBuf::from("a"),
        ]);
        assert_eq!(deduped, vec![PathBuf::from("a"), PathBuf::from("b")]);

        let normalized = normalize_repo_path(PathBuf::from("."));
        assert!(normalized.is_absolute());
    }

    #[test]
    fn push_notification_and_diagnostic_cap_old_entries() {
        let mut state = AppState::default();
        for ix in 0..205 {
            push_notification(
                &mut state,
                AppNotificationKind::Info,
                format!("notification-{ix}"),
            );
        }
        assert_eq!(state.notifications.len(), 200);
        assert_eq!(state.notifications[0].message, "notification-5");

        let mut repo = repo_state(3);
        for ix in 0..205 {
            push_diagnostic(&mut repo, DiagnosticKind::Info, format!("diagnostic-{ix}"));
        }
        assert_eq!(repo.diagnostics.len(), 200);
        assert_eq!(repo.diagnostics[0].message, "diagnostic-5");
    }

    #[test]
    fn command_and_action_logs_use_expected_stderr_and_trim_history() {
        let mut repo = repo_state(4);
        repo.command_log = (0..200).map(dummy_log_entry).collect();
        push_command_log(
            &mut repo,
            true,
            &RepoCommandKind::FetchAll,
            &command_output("git fetch", "", "stderr from git"),
            None,
        );
        assert_eq!(repo.command_log.len(), 200);
        assert_eq!(repo.command_log[0].command, "cmd-1");
        assert_eq!(
            repo.command_log
                .last()
                .expect("last command log entry")
                .stderr,
            "stderr from git"
        );

        repo.command_log = (0..200).map(dummy_log_entry).collect();
        push_action_log(
            &mut repo,
            false,
            "manual action".to_string(),
            "action failed".to_string(),
            Some(&Error::new(ErrorKind::Backend(
                "backend failure".to_string(),
            ))),
        );
        assert_eq!(repo.command_log.len(), 200);
        assert_eq!(repo.command_log[0].command, "cmd-1");
    }

    #[test]
    fn conflict_autosolve_summary_covers_mode_and_detail_variants() {
        let history_summary = conflict_autosolve_telemetry_summary(
            ConflictAutosolveMode::History,
            Some(Path::new("conflict.txt")),
            6,
            3,
            4,
            1,
            ConflictAutosolveStats {
                history: 2,
                ..ConflictAutosolveStats::default()
            },
        );
        assert!(history_summary.contains("(history)"));
        assert!(history_summary.contains("history=2"));
        assert!(history_summary.contains("in conflict.txt"));

        let safe_summary = conflict_autosolve_telemetry_summary(
            ConflictAutosolveMode::Safe,
            None,
            1,
            1,
            1,
            1,
            ConflictAutosolveStats::default(),
        );
        assert!(safe_summary.contains("(safe)"));
        assert!(safe_summary.contains("details=none"));
    }

    #[test]
    fn summarize_command_failure_covers_error_labels() {
        let failing_cases = vec![
            (RepoCommandKind::FetchAll, "Fetch"),
            (
                RepoCommandKind::PruneMergedBranches,
                "Prune merged branches",
            ),
            (RepoCommandKind::PruneLocalTags, "Prune local tags"),
            (
                RepoCommandKind::Pull {
                    mode: PullMode::Default,
                },
                "Pull",
            ),
            (
                RepoCommandKind::PullBranch {
                    remote: "origin".into(),
                    branch: "main".into(),
                },
                "Pull",
            ),
            (
                RepoCommandKind::MergeRef {
                    reference: "feature".into(),
                },
                "Merge",
            ),
            (
                RepoCommandKind::SquashRef {
                    reference: "feature".into(),
                },
                "Squash",
            ),
            (RepoCommandKind::Push, "Push"),
            (RepoCommandKind::ForcePush, "Force push"),
            (
                RepoCommandKind::PushSetUpstream {
                    remote: "origin".into(),
                    branch: "main".into(),
                },
                "Push",
            ),
            (
                RepoCommandKind::SetUpstreamBranch {
                    branch: "main".into(),
                    upstream: "origin/main".into(),
                },
                "Set as tracking upstream",
            ),
            (
                RepoCommandKind::UnsetUpstreamBranch {
                    branch: "main".into(),
                },
                "Unlink upstream branch",
            ),
            (
                RepoCommandKind::DeleteRemoteBranch {
                    remote: "origin".into(),
                    branch: "old".into(),
                },
                "Delete remote branch",
            ),
            (
                RepoCommandKind::PushTag {
                    remote: "origin".into(),
                    name: "v1".into(),
                },
                "Push tag",
            ),
            (
                RepoCommandKind::DeleteRemoteTag {
                    remote: "origin".into(),
                    name: "v1".into(),
                },
                "Delete remote tag",
            ),
            (
                RepoCommandKind::Reset {
                    mode: ResetMode::Hard,
                    target: "HEAD~1".into(),
                },
                "Reset",
            ),
            (
                RepoCommandKind::Rebase {
                    onto: "main".into(),
                },
                "Rebase",
            ),
            (RepoCommandKind::RebaseContinue, "Rebase"),
            (RepoCommandKind::RebaseAbort, "Rebase"),
            (RepoCommandKind::MergeAbort, "Merge"),
            (
                RepoCommandKind::CreateTag {
                    name: "v2".into(),
                    target: "HEAD".into(),
                },
                "Tag",
            ),
            (RepoCommandKind::DeleteTag { name: "v2".into() }, "Tag"),
            (
                RepoCommandKind::AddRemote {
                    name: "origin".into(),
                    url: "https://example.com/repo.git".into(),
                },
                "Remote",
            ),
            (
                RepoCommandKind::RemoveRemote {
                    name: "origin".into(),
                },
                "Remote",
            ),
            (
                RepoCommandKind::SetRemoteUrl {
                    name: "origin".into(),
                    url: "https://example.com/repo.git".into(),
                    kind: RemoteUrlKind::Fetch,
                },
                "Remote",
            ),
        ];

        for (command, label) in failing_cases {
            let (rendered_command, summary) =
                summarize_command(&command, &CommandOutput::default(), false, None);
            assert_eq!(rendered_command, label);
            assert_eq!(summary, format!("{label} failed"));
        }
    }

    #[test]
    fn summarize_command_success_covers_status_variants() {
        let (_, fetch_summary) = summarize_command(
            &RepoCommandKind::FetchAll,
            &command_output("git fetch", "synced", ""),
            true,
            None,
        );
        assert_eq!(fetch_summary, "Fetch: Synchronized");

        let (_, pull_up_to_date) = summarize_command(
            &RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            &command_output("git pull", "Already up to date", ""),
            true,
            None,
        );
        assert_eq!(pull_up_to_date, "Pull: Already up to date");

        let (_, pull_fast_forward) = summarize_command(
            &RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            &command_output("git pull", "Updating abc..def", ""),
            true,
            None,
        );
        assert_eq!(pull_fast_forward, "Pull: Fast-forwarded");

        let (_, pull_merged) = summarize_command(
            &RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            &command_output("git pull", "Merge branch 'feature'", ""),
            true,
            None,
        );
        assert_eq!(pull_merged, "Pull: Merged");

        let (_, pull_rebased) = summarize_command(
            &RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            &command_output(
                "git pull",
                "Successfully rebased and updated refs/heads/main.",
                "",
            ),
            true,
            None,
        );
        assert_eq!(pull_rebased, "Pull: Rebasing complete");

        let (_, pull_branch_summary) = summarize_command(
            &RepoCommandKind::PullBranch {
                remote: "origin".into(),
                branch: "main".into(),
            },
            &command_output("git pull origin main", "Updating abc..def", ""),
            true,
            None,
        );
        assert_eq!(pull_branch_summary, "Pull origin/main: Fast-forwarded");

        let (_, merge_ref_summary) = summarize_command(
            &RepoCommandKind::MergeRef {
                reference: "feature".into(),
            },
            &command_output("git merge feature", "Fast-forward", ""),
            true,
            None,
        );
        assert_eq!(merge_ref_summary, "Merge feature: Fast-forwarded");

        let (_, squash_ref_summary) = summarize_command(
            &RepoCommandKind::SquashRef {
                reference: "feature".into(),
            },
            &command_output(
                "git merge --squash feature",
                "Squash commit -- not updating HEAD\nAutomatic merge went well; stopped before committing as requested",
                "",
            ),
            true,
            None,
        );
        assert_eq!(squash_ref_summary, "Squash feature: Staged");

        let (_, push_uptodate) = summarize_command(
            &RepoCommandKind::Push,
            &command_output("git push", "", "Everything up-to-date"),
            true,
            None,
        );
        assert_eq!(push_uptodate, "Push: Everything up-to-date");

        let (_, force_push_uptodate) = summarize_command(
            &RepoCommandKind::ForcePush,
            &command_output("git push --force", "", "Everything up-to-date"),
            true,
            None,
        );
        assert_eq!(force_push_uptodate, "Force push: Everything up-to-date");

        let (_, push_upstream_uptodate) = summarize_command(
            &RepoCommandKind::PushSetUpstream {
                remote: "origin".into(),
                branch: "main".into(),
            },
            &command_output("git push -u origin main", "", "Everything up-to-date"),
            true,
            None,
        );
        assert_eq!(
            push_upstream_uptodate,
            "Push -u origin/main: Everything up-to-date"
        );

        let (_, set_upstream_summary) = summarize_command(
            &RepoCommandKind::SetUpstreamBranch {
                branch: "feature".into(),
                upstream: "origin/feature".into(),
            },
            &command_output(
                "git branch --set-upstream-to origin/feature feature",
                "",
                "",
            ),
            true,
            None,
        );
        assert_eq!(
            set_upstream_summary,
            "Branch feature: Upstream set to origin/feature"
        );

        let (_, unset_upstream_summary) = summarize_command(
            &RepoCommandKind::UnsetUpstreamBranch {
                branch: "feature".into(),
            },
            &command_output("git branch --unset-upstream feature", "", ""),
            true,
            None,
        );
        assert_eq!(unset_upstream_summary, "Branch feature: Upstream unlinked");

        let (_, push_tag_uptodate) = summarize_command(
            &RepoCommandKind::PushTag {
                remote: "origin".into(),
                name: "v1".into(),
            },
            &command_output("git push origin v1", "", "Everything up-to-date"),
            true,
            None,
        );
        assert_eq!(push_tag_uptodate, "Tag v1 → origin: Already up-to-date");

        let (_, reset_soft) = summarize_command(
            &RepoCommandKind::Reset {
                mode: ResetMode::Soft,
                target: "HEAD~1".into(),
            },
            &command_output("git reset --soft HEAD~1", "", ""),
            true,
            None,
        );
        assert_eq!(reset_soft, "Reset (--soft) HEAD~1: Completed");

        let (_, reset_mixed) = summarize_command(
            &RepoCommandKind::Reset {
                mode: ResetMode::Mixed,
                target: "HEAD~1".into(),
            },
            &command_output("git reset --mixed HEAD~1", "", ""),
            true,
            None,
        );
        assert_eq!(reset_mixed, "Reset (--mixed) HEAD~1: Completed");

        let (_, reset_hard) = summarize_command(
            &RepoCommandKind::Reset {
                mode: ResetMode::Hard,
                target: "HEAD~1".into(),
            },
            &command_output("git reset --hard HEAD~1", "", ""),
            true,
            None,
        );
        assert_eq!(reset_hard, "Reset (--hard) HEAD~1: Completed");

        let (_, rebase_summary) = summarize_command(
            &RepoCommandKind::Rebase {
                onto: "origin/main".into(),
            },
            &command_output("git rebase origin/main", "", ""),
            true,
            None,
        );
        assert_eq!(rebase_summary, "Rebase onto origin/main: Completed");

        let (_, rebase_continue_summary) = summarize_command(
            &RepoCommandKind::RebaseContinue,
            &command_output("git rebase --continue", "", ""),
            true,
            None,
        );
        assert_eq!(rebase_continue_summary, "Rebase: Continued");

        let (_, rebase_abort_summary) = summarize_command(
            &RepoCommandKind::RebaseAbort,
            &command_output("git rebase --abort", "", ""),
            true,
            None,
        );
        assert_eq!(rebase_abort_summary, "Rebase: Aborted");

        let (_, merge_abort_summary) = summarize_command(
            &RepoCommandKind::MergeAbort,
            &command_output("git merge --abort", "", ""),
            true,
            None,
        );
        assert_eq!(merge_abort_summary, "Merge: Aborted");

        let (_, create_tag_summary) = summarize_command(
            &RepoCommandKind::CreateTag {
                name: "v2".into(),
                target: "HEAD".into(),
            },
            &command_output("git tag v2 HEAD", "", ""),
            true,
            None,
        );
        assert_eq!(create_tag_summary, "Tag v2 → HEAD: Created");

        let (_, delete_tag_summary) = summarize_command(
            &RepoCommandKind::DeleteTag { name: "v2".into() },
            &command_output("git tag -d v2", "", ""),
            true,
            None,
        );
        assert_eq!(delete_tag_summary, "Tag v2: Deleted");

        let (_, add_remote_summary) = summarize_command(
            &RepoCommandKind::AddRemote {
                name: "origin".into(),
                url: "https://example.com/repo.git".into(),
            },
            &command_output("git remote add origin ...", "", ""),
            true,
            None,
        );
        assert_eq!(add_remote_summary, "Remote origin: Added");

        let (_, remove_remote_summary) = summarize_command(
            &RepoCommandKind::RemoveRemote {
                name: "origin".into(),
            },
            &command_output("git remote remove origin", "", ""),
            true,
            None,
        );
        assert_eq!(remove_remote_summary, "Remote origin: Removed");

        let (_, set_remote_url_summary) = summarize_command(
            &RepoCommandKind::SetRemoteUrl {
                name: "origin".into(),
                url: "https://example.com/repo.git".into(),
                kind: RemoteUrlKind::Push,
            },
            &command_output("git remote set-url --push origin ...", "", ""),
            true,
            None,
        );
        assert_eq!(set_remote_url_summary, "Remote origin (push): URL updated");
    }

    #[test]
    fn error_format_helpers_cover_non_git_and_failed_suffix_cases() {
        let git_error = Error::new(ErrorKind::Git(GitFailure::new(
            "git fetch --all",
            GitFailureId::CommandFailed,
            Some(128),
            Vec::new(),
            b"fatal: network down\n".to_vec(),
            Some("fatal: network down".to_string()),
        )));
        let formatted = format_failure_summary("Fetch", &git_error);
        assert!(formatted.contains("Fetch failed"));
        assert!(formatted.contains("git fetch --all"));
        assert!(formatted.contains("fatal: network down"));
        assert_eq!(
            format_error_for_user(&git_error),
            "git fetch --all failed: fatal: network down"
        );

        let backend_error = Error::new(ErrorKind::Backend(
            "git fetch --all failed: fatal: network down".to_string(),
        ));
        assert!(format_failure_summary("Fetch", &backend_error).contains("git fetch --all"));

        let io_error = Error::new(ErrorKind::Io(io::ErrorKind::Other));
        let io_rendered = format_error_for_user(&io_error);
        assert_eq!(io_rendered, io_error.to_string());
        assert!(!io_rendered.is_empty());
        assert!(try_format_git_backend_error(&io_error).is_none());
        assert!(try_format_git_backend_error_message("curl failed: timeout").is_none());
        assert_eq!(
            parse_failed_command_message("git status failed"),
            Some(("git status".to_string(), None))
        );

        let rendered = render_command_and_output("git status", Some(""));
        assert!(rendered.contains("    git status"));
        assert!(!rendered.contains("\n\n    "));

        assert_eq!(
            "value".to_string().if_empty_else(|| "fallback".to_string()),
            "value"
        );
    }

    #[test]
    fn detect_auth_prompt_kind_classifies_username_password_passphrase_and_host_verification() {
        assert_eq!(
            detect_auth_prompt_kind_from_message(
                "git pull failed: fatal: could not read Username for 'https://example.com': terminal prompts disabled"
            ),
            Some(crate::model::AuthPromptKind::UsernamePassword)
        );
        assert_eq!(
            detect_auth_prompt_kind_from_message(
                "git push failed: Enter passphrase for key '/home/user/.ssh/id_ed25519': terminal prompts disabled"
            ),
            Some(crate::model::AuthPromptKind::Passphrase)
        );
        assert_eq!(
            detect_auth_prompt_kind_from_message(
                "git clone --progress git@github.com:org/repo.git C:\\git\\repo failed: git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository."
            ),
            Some(crate::model::AuthPromptKind::Passphrase)
        );
        assert_eq!(
            detect_auth_prompt_kind_from_message(
                "git pull --no-rebase origin main failed: Host key verification failed.\nfatal: Could not read from remote repository."
            ),
            Some(crate::model::AuthPromptKind::HostVerification)
        );
        assert_eq!(
            detect_auth_prompt_kind_from_message(
                "git fetch origin failed: The authenticity of host 'github.com (140.82.121.3)' can't be established.\nED25519 key fingerprint is: SHA256:+DiY...\nAre you sure you want to continue connecting (yes/no/[fingerprint])?"
            ),
            Some(crate::model::AuthPromptKind::HostVerification)
        );
        assert!(detect_auth_prompt_kind_from_message("git status failed").is_none());

        let structured = Error::new(ErrorKind::Git(GitFailure::new(
            "git fetch origin",
            GitFailureId::CommandFailed,
            Some(128),
            Vec::new(),
            b"Host key verification failed.\nfatal: Could not read from remote repository.\n"
                .to_vec(),
            None,
        )));
        assert_eq!(
            detect_auth_prompt_kind(&structured),
            Some(crate::model::AuthPromptKind::HostVerification)
        );
    }

    #[test]
    fn stage_git_auth_env_stages_and_clears_shared_auth_slot() {
        let _lock = crate::store::tests::staged_auth_test_lock();
        gitcomet_core::auth::clear_staged_git_auth();
        stage_git_auth_env(
            crate::model::AuthPromptKind::UsernamePassword,
            Some("alice"),
            "secret-token",
        )
        .expect("staging auth");

        let staged = gitcomet_core::auth::take_staged_git_auth().expect("staged auth to exist");
        assert_eq!(staged.username.as_deref(), Some("alice"));
        assert_eq!(staged.secret, "secret-token");
        assert_eq!(
            staged.kind,
            gitcomet_core::auth::GitAuthKind::UsernamePassword
        );

        stage_git_auth_env(
            crate::model::AuthPromptKind::Passphrase,
            None,
            "ssh-passphrase",
        )
        .expect("staging passphrase");

        let staged =
            gitcomet_core::auth::take_staged_git_auth().expect("staged passphrase to exist");
        assert_eq!(staged.kind, gitcomet_core::auth::GitAuthKind::Passphrase);

        stage_git_auth_env(
            crate::model::AuthPromptKind::HostVerification,
            None,
            " YES ",
        )
        .expect("staging host verification");

        let staged =
            gitcomet_core::auth::take_staged_git_auth().expect("staged host verification to exist");
        assert_eq!(
            staged.kind,
            gitcomet_core::auth::GitAuthKind::HostVerification
        );
        assert_eq!(staged.secret, "yes");

        clear_staged_git_auth_env();
        assert!(gitcomet_core::auth::take_staged_git_auth().is_none());
    }
}
