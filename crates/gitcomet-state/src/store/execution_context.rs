use crate::model::{AppState, LoadKind, RepoId};
use crate::msg::{Effect, ReferenceEffect, StatusEffect};
use gitcomet_core::git_execution_log::context::{self, ExecutionContext};

pub(super) struct ScheduledEffect {
    pub(super) effect: Effect,
    pub(super) context: Option<ExecutionContext>,
}

impl ScheduledEffect {
    pub(super) fn capture(state: &AppState, effect: Effect) -> Self {
        let context = effect_context(state, &effect);
        Self { effect, context }
    }
}

fn effect_context(state: &AppState, effect: &Effect) -> Option<ExecutionContext> {
    let loads = |id: RepoId| {
        state
            .repos
            .iter()
            .find(|repo| repo.id == id)
            .map(|repo| &repo.loads_in_flight)
    };
    let ordinary = |id, kind| loads(id).and_then(|loads| loads.active_context(kind));
    match effect {
        Effect::References(effect) => {
            let kind = match effect {
                ReferenceEffect::LoadBranches { .. } => LoadKind::Branches,
                ReferenceEffect::LoadRemotes { .. } => LoadKind::Remotes,
                ReferenceEffect::LoadRemoteBranches { .. } => LoadKind::RemoteBranches,
                ReferenceEffect::LoadHeadBranch { .. } => LoadKind::HeadBranch,
                ReferenceEffect::LoadUpstreamDivergence { .. } => LoadKind::UpstreamDivergence,
                ReferenceEffect::LoadTags { .. } => LoadKind::Tags,
                ReferenceEffect::LoadRemoteTags { .. } => LoadKind::RemoteTags,
                ReferenceEffect::LoadRefMetadata { .. } => LoadKind::RefMetadata,
            };
            ordinary(effect.repo_id(), kind)
        }
        Effect::Status(effect) => match effect {
            StatusEffect::LoadStatus { repo_id } => {
                let worktree = ordinary(*repo_id, LoadKind::WorktreeStatus);
                debug_assert_eq!(worktree, ordinary(*repo_id, LoadKind::StagedStatus));
                worktree
            }
            StatusEffect::LoadWorktreeStatus { repo_id } => {
                ordinary(*repo_id, LoadKind::WorktreeStatus)
            }
            StatusEffect::LoadStagedStatus { repo_id } => {
                ordinary(*repo_id, LoadKind::StagedStatus)
            }
            StatusEffect::LoadUncommittedLineStats {
                repo_id,
                generation,
                ..
            } => loads(*repo_id).and_then(|loads| loads.line_stats_context(*generation)),
        },
        Effect::LoadLog { repo_id, seq, .. } => {
            loads(*repo_id).and_then(|loads| loads.log_context(*seq))
        }
        Effect::LoadWorktreeDirty { repo_id, .. } => {
            loads(*repo_id).and_then(|loads| loads.worktree_context())
        }
        Effect::LoadStashes { repo_id, .. } => ordinary(*repo_id, LoadKind::Stashes),
        Effect::LoadReflog { repo_id, .. } => ordinary(*repo_id, LoadKind::Reflog),
        Effect::LoadWorktrees { repo_id } => ordinary(*repo_id, LoadKind::Worktrees),
        Effect::LoadSubmodules { repo_id } => ordinary(*repo_id, LoadKind::Submodules),
        Effect::LoadLargeFileSupport { repo_id } => ordinary(*repo_id, LoadKind::LargeFileSupport),
        Effect::LoadLfsLocks { repo_id } => ordinary(*repo_id, LoadKind::LfsLocks),
        Effect::LoadAnnexUnused { repo_id } => ordinary(*repo_id, LoadKind::AnnexUnused),
        Effect::LoadFileBrowser { repo_id, .. } => ordinary(*repo_id, LoadKind::FileBrowser),
        Effect::LoadRebaseState { repo_id } => ordinary(*repo_id, LoadKind::RebaseState),
        Effect::LoadMergeCommitMessage { repo_id } => {
            ordinary(*repo_id, LoadKind::MergeCommitMessage)
        }
        Effect::LoadRebaseAndMergeState { repo_id } => {
            let rebase = ordinary(*repo_id, LoadKind::RebaseState);
            debug_assert_eq!(rebase, ordinary(*repo_id, LoadKind::MergeCommitMessage));
            rebase
        }
        _ => context::current(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RepoState;
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::git_execution_log::context::UiSurface;

    #[test]
    fn scheduled_effect_freezes_promoted_context_before_later_requests() {
        let mut state = AppState::test_default();
        let id = RepoId(9);
        let mut repo = RepoState::new_opening(
            id,
            RepoSpec {
                workdir: std::path::PathBuf::from("/example"),
            },
        );
        let a = ExecutionContext::ui(UiSurface::Button, "a");
        let b = ExecutionContext::ui(UiSurface::Button, "b");
        let c = ExecutionContext::ui(UiSurface::Button, "c");
        {
            let _scope = a.attach();
            assert!(repo.loads_in_flight.request(LoadKind::WorktreeStatus));
        }
        {
            let _scope = b.attach();
            assert!(!repo.loads_in_flight.request(LoadKind::WorktreeStatus));
        }
        assert!(repo.loads_in_flight.finish(LoadKind::WorktreeStatus));
        state.repos.push(repo);
        let scheduled = {
            let _scope = a.attach();
            ScheduledEffect::capture(
                &state,
                Effect::Status(StatusEffect::LoadWorktreeStatus { repo_id: id }),
            )
        };
        {
            let _scope = c.attach();
            state.repos[0]
                .loads_in_flight
                .request(LoadKind::WorktreeStatus);
        }
        assert_eq!(scheduled.context, Some(b));
    }

    #[test]
    fn filesystem_and_visibility_refreshes_admit_their_own_load_context() {
        use crate::model::Loadable;
        use crate::msg::Msg;
        use gitcomet_core::filesystem::PathChange;
        use std::sync::{Arc, atomic::AtomicU64};
        for busy in [false, true] {
            for visibility in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let mut state = AppState::test_default();
                let id = RepoId(9);
                let mut repo = RepoState::new_opening(
                    id,
                    RepoSpec {
                        workdir: root.path().to_path_buf(),
                    },
                );
                repo.open = Loadable::Ready(());
                repo.file_browser.entries = Loadable::Ready(Arc::new(Vec::new()));
                let a = ExecutionContext::ui(UiSurface::Button, "a");
                let b = ExecutionContext::ui(UiSurface::Button, "b");
                if busy {
                    let _scope = a.attach();
                    repo.loads_in_flight.request(LoadKind::FileBrowser);
                    repo.loads_in_flight.request(LoadKind::WorktreeStatus);
                }
                state.repos.push(repo);
                let _scope = b.attach();
                let message = if visibility {
                    Msg::SetExplorerVisibility {
                        repo_id: id,
                        hidden: Some(true),
                        ignored: Some(true),
                    }
                } else {
                    Msg::FilesystemPathsChanged(vec![PathChange {
                        old: Some(root.path().join("before")),
                        new: Some(root.path().join("after")),
                    }])
                };
                let effects = crate::store::reducer::reduce(
                    &mut Default::default(),
                    &AtomicU64::new(10),
                    &mut state,
                    message,
                );
                for effect in effects {
                    let scheduled = ScheduledEffect::capture(&state, effect);
                    assert_eq!(scheduled.context, Some(b.clone()));
                }
                if busy {
                    assert_eq!(
                        state.repos[0]
                            .loads_in_flight
                            .active_context(LoadKind::FileBrowser),
                        Some(a)
                    );
                    assert!(state.repos[0].loads_in_flight.finish(LoadKind::FileBrowser));
                }
                assert_eq!(
                    state.repos[0]
                        .loads_in_flight
                        .active_context(LoadKind::FileBrowser),
                    Some(b)
                );
            }
        }
    }
}
