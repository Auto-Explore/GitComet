use crate::msg::{ControlEffect, PersistenceEffect, ReferenceEffect, StatusEffect};
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
        // Gate before the arguments are evaluated: call sites format paths
        // per filesystem event, which must cost nothing while tracing is off.
        if $crate::store::repo_load_trace::enabled() {
            $crate::store::repo_load_trace::record(format_args!($($arg)*))
        }
    };
}

pub(super) use trace;

pub(super) fn msg_name(msg: &Msg) -> &'static str {
    match msg {
        Msg::Internal(message) => internal_msg_name(message),
        msg => msg.into(),
    }
}

pub(super) fn msg_repo_id(msg: &Msg) -> Option<RepoId> {
    match msg {
        Msg::CloseRepo { repo_id }
        | Msg::MoveRepoOut { repo_id }
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
        Msg::RepoExternallyChanged { change, .. } => Some(change.clone()),
        _ => None,
    }
}

pub(super) fn internal_msg_name(msg: &InternalMsg) -> &'static str {
    msg.into()
}

/// Operation-trace label: names the result a repository load delivered
/// rather than the `RepoLoadFinished` envelope every load arrives in.
pub(super) fn stage_label(msg: &Msg) -> &'static str {
    let mut message = match msg {
        Msg::Internal(message) => message,
        msg => return msg.into(),
    };
    while let InternalMsg::RepoLoadFinished { message: inner, .. } = message {
        message = inner;
    }
    message.into()
}

pub(super) fn effect_name(effect: &Effect) -> &'static str {
    match effect {
        Effect::Persistence(work) => work.into(),
        Effect::Control(work) => work.into(),
        Effect::Status(work) => work.into(),
        Effect::References(work) => work.into(),
        Effect::RepoCommand(work) => work.into(),
        effect => effect.into(),
    }
}

pub(super) fn effect_repo_id(effect: &Effect) -> Option<RepoId> {
    match effect {
        Effect::OpenRepo { repo_id, .. }
        | Effect::Control(ControlEffect::CancelRepoLoads { repo_id, .. })
        | Effect::Control(ControlEffect::CancelGitOperation { repo_id, .. })
        | Effect::References(ReferenceEffect::LoadBranches { repo_id })
        | Effect::References(ReferenceEffect::LoadRemotes { repo_id })
        | Effect::References(ReferenceEffect::LoadRemoteBranches { repo_id })
        | Effect::Status(StatusEffect::LoadWorktreeStatus { repo_id })
        | Effect::Status(StatusEffect::LoadStagedStatus { repo_id })
        | Effect::Status(StatusEffect::LoadUncommittedLineStats { repo_id, .. })
        | Effect::Status(StatusEffect::LoadStatus { repo_id })
        | Effect::References(ReferenceEffect::LoadHeadBranch { repo_id })
        | Effect::References(ReferenceEffect::LoadUpstreamDivergence { repo_id })
        | Effect::LoadLog { repo_id, .. }
        | Effect::References(ReferenceEffect::LoadTags { repo_id })
        | Effect::References(ReferenceEffect::LoadRemoteTags { repo_id })
        | Effect::LoadStashes { repo_id, .. }
        | Effect::LoadWorktrees { repo_id }
        | Effect::LoadWorktreeDirty { repo_id, .. }
        | Effect::References(ReferenceEffect::LoadRefMetadata { repo_id })
        | Effect::LoadSubmodules { repo_id }
        | Effect::LoadLargeFileSupport { repo_id }
        | Effect::LoadLfsLocks { repo_id }
        | Effect::LoadAnnexWhereis { repo_id, .. }
        | Effect::LoadAnnexUnused { repo_id }
        | Effect::LoadRebaseAndMergeState { repo_id }
        | Effect::LoadRebaseState { repo_id }
        | Effect::LoadMergeCommitMessage { repo_id } => Some(*repo_id),
        Effect::Persistence(PersistenceEffect::PersistSession { repo_id, .. }) => *repo_id,
        Effect::Persistence(PersistenceEffect::PersistRecentRepo { repo_id, .. }) => *repo_id,
        Effect::Persistence(PersistenceEffect::PersistRepoHistoryMode { repo_id, .. }) => *repo_id,
        Effect::Persistence(PersistenceEffect::PersistRepoHistoryModesBatch {
            repo_id, ..
        }) => *repo_id,
        Effect::Persistence(PersistenceEffect::PersistRepoHistoryAuthorFilter {
            repo_id, ..
        }) => *repo_id,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::RepoCommandEffect;

    #[test]
    fn effect_families_preserve_operation_trace_labels() {
        let cases = [
            (
                Effect::Persistence(PersistenceEffect::PersistSession {
                    repo_id: Some(RepoId(7)),
                    action: "save session",
                }),
                "PersistSession",
            ),
            (
                Effect::Control(ControlEffect::CancelRepoLoads {
                    repo_id: RepoId(7),
                    load_epoch: Default::default(),
                }),
                "CancelRepoLoads",
            ),
            (
                Effect::Status(StatusEffect::LoadWorktreeStatus { repo_id: RepoId(7) }),
                "LoadWorktreeStatus",
            ),
            (
                Effect::References(ReferenceEffect::LoadRefMetadata { repo_id: RepoId(7) }),
                "LoadRefMetadata",
            ),
            (
                Effect::RepoCommand(RepoCommandEffect::FetchAll {
                    repo_id: RepoId(7),
                    prune: true,
                    auth: None,
                }),
                "FetchAll",
            ),
        ];
        for (effect, expected) in cases {
            assert_eq!(effect_name(&effect), expected);
        }
    }

    /// The worktree-dirty scan is the most expensive recurring background load,
    /// so the trace that exists to explain slow repo loads has to name it. Both
    /// matches above end in a catch-all, so a missing arm is not a compile error
    /// -- it just makes every one of these scans log as an anonymous `Effect`
    /// with no repo, which `store/mod.rs` then cannot resolve a workdir for.
    #[test]
    fn the_worktree_dirty_scan_is_traced_against_its_repo() {
        let effect = Effect::LoadWorktreeDirty {
            repo_id: RepoId(7),
            scope: crate::model::WorktreeDirtyScope::All,
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
                scope: crate::model::WorktreeDirtyScope::All,
                result: Ok(Vec::new()),
            }),
            "WorktreeDirtyLoaded"
        );
    }
}
