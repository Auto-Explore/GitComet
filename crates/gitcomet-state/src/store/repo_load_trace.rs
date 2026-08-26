use crate::msg::{Effect, InternalMsg, Msg};
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use super::RepoId;

const TRACE_FILE_ENV: &str = "GITCOMET_REPO_LOAD_TRACE";
const TRACE_STDERR_ENV: &str = "GITCOMET_TRACE_REPO_LOADS";

enum TraceTarget {
    Disabled,
    Stderr,
    File(Mutex<std::fs::File>),
}

static STARTED_AT: LazyLock<Instant> = LazyLock::new(Instant::now);
static TARGET: LazyLock<TraceTarget> = LazyLock::new(|| {
    if let Some(path) = std::env::var_os(TRACE_FILE_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => return TraceTarget::File(Mutex::new(file)),
            Err(error) => {
                eprintln!(
                    "gitcomet-state: failed to open repo load trace file {}: {error}",
                    path.display()
                );
            }
        }
    }

    if std::env::var_os(TRACE_STDERR_ENV).is_some_and(|value| !value.is_empty() && value != "0") {
        TraceTarget::Stderr
    } else {
        TraceTarget::Disabled
    }
});

pub(super) fn enabled() -> bool {
    !matches!(&*TARGET, TraceTarget::Disabled)
}

pub(super) fn record(args: std::fmt::Arguments<'_>) {
    if !enabled() {
        return;
    }

    let elapsed = STARTED_AT.elapsed();
    let line = format!(
        "[repo-load-trace +{}.{:03}s thread={:?}] {}\n",
        elapsed.as_secs(),
        elapsed.subsec_millis(),
        std::thread::current().id(),
        args
    );
    match &*TARGET {
        TraceTarget::Disabled => {}
        TraceTarget::Stderr => eprint!("{line}"),
        TraceTarget::File(file) => {
            if let Ok(mut file) = file.lock() {
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
            }
        }
    }
}

macro_rules! trace {
    ($($arg:tt)*) => {
        $crate::store::repo_load_trace::record(format_args!($($arg)*))
    };
}

pub(super) use trace;

pub(super) fn msg_name(msg: &Msg) -> &'static str {
    match msg {
        Msg::OpenRepo(_) => "OpenRepo",
        Msg::OpenRepoFromExternalDrop(_) => "OpenRepoFromExternalDrop",
        Msg::RestoreSession { .. } => "RestoreSession",
        Msg::CloseRepo { .. } => "CloseRepo",
        Msg::CloseRepos { .. } => "CloseRepos",
        Msg::SetActiveRepo { .. } => "SetActiveRepo",
        Msg::ReorderRepoTabs { .. } => "ReorderRepoTabs",
        Msg::ReloadRepo { .. } => "ReloadRepo",
        Msg::RepoActivated { .. } => "RepoActivated",
        Msg::RepoExternallyChanged { .. } => "RepoExternallyChanged",
        Msg::Internal(message) => internal_msg_name(message),
        _ => "Msg",
    }
}

pub(super) fn msg_repo_id(msg: &Msg) -> Option<RepoId> {
    match msg {
        Msg::CloseRepo { repo_id }
        | Msg::SetActiveRepo { repo_id }
        | Msg::ReorderRepoTabs { repo_id, .. }
        | Msg::ReloadRepo { repo_id }
        | Msg::RepoActivated { repo_id }
        | Msg::RepoExternallyChanged { repo_id, .. } => Some(*repo_id),
        _ => None,
    }
}

pub(super) fn msg_external_change(msg: &Msg) -> Option<crate::msg::RepoExternalChange> {
    match msg {
        Msg::RepoExternallyChanged { change, .. } => Some(*change),
        _ => None,
    }
}

pub(super) fn internal_msg_name(msg: &InternalMsg) -> &'static str {
    match msg {
        InternalMsg::RepoLoadFinished { .. } => "RepoLoadFinished",
        InternalMsg::RepoOpenedOk { .. } => "RepoOpenedOk",
        InternalMsg::RepoOpenedErr { .. } => "RepoOpenedErr",
        InternalMsg::BranchesLoaded { .. } => "BranchesLoaded",
        InternalMsg::RemotesLoaded { .. } => "RemotesLoaded",
        InternalMsg::RemoteBranchesLoaded { .. } => "RemoteBranchesLoaded",
        InternalMsg::WorktreeStatusLoaded { .. } => "WorktreeStatusLoaded",
        InternalMsg::StagedStatusLoaded { .. } => "StagedStatusLoaded",
        InternalMsg::StatusLoaded { .. } => "StatusLoaded",
        InternalMsg::HeadBranchLoaded { .. } => "HeadBranchLoaded",
        InternalMsg::UpstreamDivergenceLoaded { .. } => "UpstreamDivergenceLoaded",
        InternalMsg::LogLoaded { .. } => "LogLoaded",
        InternalMsg::TagsLoaded { .. } => "TagsLoaded",
        InternalMsg::RemoteTagsLoaded { .. } => "RemoteTagsLoaded",
        InternalMsg::StashesLoaded { .. } => "StashesLoaded",
        InternalMsg::WorktreesLoaded { .. } => "WorktreesLoaded",
        InternalMsg::WorktreeDirtyLoaded { .. } => "WorktreeDirtyLoaded",
        InternalMsg::RefMetadataLoaded { .. } => "RefMetadataLoaded",
        InternalMsg::SubmodulesLoaded { .. } => "SubmodulesLoaded",
        InternalMsg::RebaseStateLoaded { .. } => "RebaseStateLoaded",
        InternalMsg::MergeCommitMessageLoaded { .. } => "MergeCommitMessageLoaded",
        _ => "InternalMsg",
    }
}

pub(super) fn effect_name(effect: &Effect) -> &'static str {
    match effect {
        Effect::OpenRepo { .. } => "OpenRepo",
        Effect::CancelRepoLoads { .. } => "CancelRepoLoads",
        Effect::LoadBranches { .. } => "LoadBranches",
        Effect::LoadRemotes { .. } => "LoadRemotes",
        Effect::LoadRemoteBranches { .. } => "LoadRemoteBranches",
        Effect::LoadWorktreeStatus { .. } => "LoadWorktreeStatus",
        Effect::LoadStagedStatus { .. } => "LoadStagedStatus",
        Effect::LoadStatus { .. } => "LoadStatus",
        Effect::LoadHeadBranch { .. } => "LoadHeadBranch",
        Effect::LoadUpstreamDivergence { .. } => "LoadUpstreamDivergence",
        Effect::LoadLog { .. } => "LoadLog",
        Effect::LoadTags { .. } => "LoadTags",
        Effect::LoadRemoteTags { .. } => "LoadRemoteTags",
        Effect::LoadStashes { .. } => "LoadStashes",
        Effect::LoadWorktrees { .. } => "LoadWorktrees",
        Effect::LoadWorktreeDirty { .. } => "LoadWorktreeDirty",
        Effect::LoadRefMetadata { .. } => "LoadRefMetadata",
        Effect::LoadSubmodules { .. } => "LoadSubmodules",
        Effect::LoadRebaseAndMergeState { .. } => "LoadRebaseAndMergeState",
        Effect::LoadRebaseState { .. } => "LoadRebaseState",
        Effect::LoadMergeCommitMessage { .. } => "LoadMergeCommitMessage",
        Effect::PersistSession { .. } => "PersistSession",
        Effect::PersistRecentRepo { .. } => "PersistRecentRepo",
        _ => "Effect",
    }
}

pub(super) fn effect_repo_id(effect: &Effect) -> Option<RepoId> {
    match effect {
        Effect::OpenRepo { repo_id, .. }
        | Effect::CancelRepoLoads { repo_id, .. }
        | Effect::LoadBranches { repo_id }
        | Effect::LoadRemotes { repo_id }
        | Effect::LoadRemoteBranches { repo_id }
        | Effect::LoadWorktreeStatus { repo_id }
        | Effect::LoadStagedStatus { repo_id }
        | Effect::LoadStatus { repo_id }
        | Effect::LoadHeadBranch { repo_id }
        | Effect::LoadUpstreamDivergence { repo_id }
        | Effect::LoadLog { repo_id, .. }
        | Effect::LoadTags { repo_id }
        | Effect::LoadRemoteTags { repo_id }
        | Effect::LoadStashes { repo_id, .. }
        | Effect::LoadWorktrees { repo_id }
        | Effect::LoadWorktreeDirty { repo_id, .. }
        | Effect::LoadRefMetadata { repo_id }
        | Effect::LoadSubmodules { repo_id }
        | Effect::LoadRebaseAndMergeState { repo_id }
        | Effect::LoadRebaseState { repo_id }
        | Effect::LoadMergeCommitMessage { repo_id } => Some(*repo_id),
        Effect::PersistSession { repo_id, .. } => *repo_id,
        Effect::PersistRecentRepo { repo_id, .. } => *repo_id,
        Effect::PersistRepoHistoryMode { repo_id, .. } => *repo_id,
        Effect::PersistRepoHistoryModesBatch { repo_id, .. } => *repo_id,
        Effect::PersistRepoHistoryAuthorFilter { repo_id, .. } => *repo_id,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worktree-dirty scan is the most expensive recurring background load,
    /// so the trace that exists to explain slow repo loads has to name it. Both
    /// matches above end in a catch-all, so a missing arm is not a compile error
    /// -- it just makes every one of these scans log as an anonymous `Effect`
    /// with no repo, which `store/mod.rs` then cannot resolve a workdir for.
    #[test]
    fn the_worktree_dirty_scan_is_traced_against_its_repo() {
        let effect = Effect::LoadWorktreeDirty {
            repo_id: RepoId(7),
            workdir: PathBuf::from("/tmp/repo"),
            files_for: None,
        };

        assert_eq!(effect_name(&effect), "LoadWorktreeDirty");
        assert_eq!(effect_repo_id(&effect), Some(RepoId(7)));
    }

    /// The reply side is half the trace: a load with no matching completion is
    /// exactly the shape a stall takes, and an anonymous `InternalMsg` hides it.
    #[test]
    fn the_worktree_dirty_reply_is_named() {
        assert_eq!(
            internal_msg_name(&InternalMsg::WorktreeDirtyLoaded {
                repo_id: RepoId(7),
                result: Ok(Vec::new()),
            }),
            "WorktreeDirtyLoaded"
        );
    }
}
