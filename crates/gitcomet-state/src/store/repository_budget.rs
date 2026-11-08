//! Open tabs do not own backend resources indefinitely. Keep a small MRU set
//! and let document leases and unfinished operations explicitly pin a runtime.

use super::repository_runtime::RepositoryRuntimes;
use crate::model::{
    AppState, AuthRetryOperation, AutoFetchStatus, BareModeConversionState, Loadable, RepoId,
    RepoState,
};
use crate::msg::InternalMsg;
use std::time::{Duration, SystemTime};

pub(super) const WARM_INACTIVE_REPOSITORIES: usize = 2;
pub(super) const DESCRIPTOR_RECOVERY_COOLDOWN: Duration = Duration::from_secs(30);

pub(super) fn is_pinned(state: &AppState, repo: &RepoState) -> bool {
    !repo.diff_sessions.is_empty()
        || !repo.change_lists.is_empty()
        || state.watch_leases.contains_key(&repo.id)
        || state
            .worktree_watch_leases
            .keys()
            .any(|(id, lifetime, _)| *id == repo.id && *lifetime == repo.lifetime())
        || has_pending_continuation(state, repo.id)
        || has_unfinished_operation(repo)
}

fn has_pending_continuation(state: &AppState, repo_id: RepoId) -> bool {
    // Prompt acceptance resumes the existing operation directly, without a
    // tab activation that would reopen a parked backend first.
    state
        .auth_prompt
        .as_ref()
        .is_some_and(|prompt| match &prompt.operation {
            AuthRetryOperation::RepoCommand { repo_id: id, .. }
            | AuthRetryOperation::SafePushAfterCommit { repo_id: id, .. }
            | AuthRetryOperation::Commit { repo_id: id, .. } => *id == repo_id,
            AuthRetryOperation::Clone { .. } => false,
        })
        || state
            .branch_exists_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.repo_id == repo_id)
        || state
            .force_delete_branch_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.repo_id == repo_id)
        || state
            .force_remove_worktree_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.repo_id == repo_id)
        || state
            .submodule_trust_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.repo_id == repo_id)
        || state
            .submodule_trust_check_pending
            .as_ref()
            .is_some_and(|pending| pending.repo_id == repo_id)
}

pub(super) fn has_unfinished_operation(repo: &RepoState) -> bool {
    repo.pull_in_flight > 0
        || repo.push_in_flight > 0
        || repo.worktrees_in_flight > 0
        || repo.local_actions_in_flight > 0
        || repo.sequencer_actions_in_flight > 0
        || repo.commit_in_flight > 0
        || repo.submodule_add_in_flight.is_some()
        || repo.maintenance.running
        || repo.bare_mode_conversion.is_busy()
        || matches!(
            repo.bare_mode_conversion,
            BareModeConversionState::AwaitingConfirmation(_)
        )
        || matches!(repo.auto_fetch.status, AutoFetchStatus::Running)
        || repo.diff_state.edit_mode
        || repo.conflict_state.conflict_session.is_some()
}

/// Called under the state lock. Actual handle destruction is deferred until
/// the worker releases that lock, as it is when closing a tab.
pub(super) fn park_inactive(state: &mut AppState, repos: &mut RepositoryRuntimes, now: SystemTime) {
    let warm = super::warm_inactive_repos(state, repos.warm_repository_count(now));
    let cold: Vec<_> = state
        .repos
        .iter()
        .filter(|repo| {
            state.active_repo != Some(repo.id)
                && !warm.contains(&repo.id)
                && !is_pinned(state, repo)
                && matches!(repo.open, Loadable::Ready(()))
                && repos.has_repository(&repo.id)
        })
        .map(|repo| repo.id)
        .collect();
    for id in cold {
        let repo = state
            .repos
            .iter_mut()
            .find(|repo| repo.id == id)
            .expect("known tab");
        super::reducer::park_repository(repo, repos);
    }
}

/// Only ordinary idempotent repository reads may initiate a reopen. A push,
/// commit or other mutation's error is always delivered to its normal reducer.
/// Request-specific popovers/previews keep their normal errors too: replaying
/// a full activation would not reproduce those exact requests.
pub(super) fn descriptor_read_failure(message: &InternalMsg) -> Option<RepoId> {
    let (repo_id, error) = match message {
        InternalMsg::RepoOpenedErr { repo_id, error, .. }
        | InternalMsg::BranchesLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::RemotesLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::RemoteBranchesLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::StatusLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::WorktreeStatusLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::StagedStatusLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::HeadBranchLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::HeadTrackingLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::UpstreamDivergenceLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::LogLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::TagsLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::RemoteTagsLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::StashesLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::RebaseStateLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::MergeCommitMessageLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::WorktreesLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::WorktreeDirtyLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::SubmodulesLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::LargeFileSupportLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::DiffLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::DiffFileLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::DiffFileImageLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::DiffPreviewTextFileLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::SubmoduleSummaryLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::TextAttributesLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::FileHistoryLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::BlameLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::CommitDetailsLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::FileBrowserLoaded {
            repo_id,
            result: Err(error),
            ..
        } => (repo_id, error),
        _ => return None,
    };
    error.is_file_descriptor_exhaustion().then_some(*repo_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{RepoSpec, RepositoryMode};
    use gitcomet_core::services::GitRepository;
    use gitcomet_core::test_support::UnconfiguredRepository;
    use std::sync::{Arc, Weak};
    use std::time::{Duration, SystemTime};

    fn visited_repos(count: u64) -> (AppState, RepositoryRuntimes, Vec<Weak<dyn GitRepository>>) {
        let mut state = AppState::test_default();
        let mut runtimes = RepositoryRuntimes::default();
        let mut weak = Vec::new();
        for id in 1..=count {
            let spec = RepoSpec {
                workdir: format!("/repos/{id}").into(),
            };
            let mut repo = RepoState::new_opening(RepoId(id), spec.clone());
            repo.set_open(Loadable::Ready(()));
            repo.repository_mode = Some(RepositoryMode::WorkingTree);
            repo.last_active_at = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(id));
            let backend: Arc<dyn GitRepository> =
                Arc::new(UnconfiguredRepository::new(spec.workdir));
            weak.push(Arc::downgrade(&backend));
            runtimes.opened(repo.id, repo.lifetime_id(), backend);
            state.repos.push(repo);
        }
        state.active_repo = Some(RepoId(count));
        (state, runtimes, weak)
    }

    #[test]
    fn hundreds_of_visited_tabs_retain_only_active_and_two_recent_backends() {
        let (mut state, mut runtimes, weak) = visited_repos(264);
        let lifetime = state.repos[0].lifetime_id();
        let lease = runtimes.worktree_cache(RepoId(1), lifetime);
        let scope = runtimes.start_load_scope(RepoId(1), lifetime, state.repos[0].load_epoch);
        park_inactive(&mut state, &mut runtimes, SystemTime::now());
        // Destruction is deliberately deferred beyond the state write lock.
        assert!(weak[0].upgrade().is_some());
        runtimes.release_retired();
        assert!(scope.cancellation.is_cancelled());
        assert!(!lease.is_open());
        assert_eq!(
            weak.iter().filter(|repo| repo.upgrade().is_some()).count(),
            3
        );
        for id in [262, 263, 264] {
            assert!(runtimes.has_repository(&RepoId(id)));
        }
        assert_eq!(state.repos.len(), 264);
        assert_eq!(state.repos[0].lifetime_id(), lifetime);
        assert!(matches!(state.repos[0].open, Loadable::Ready(())));
        assert!(state.repos[0].backend_parked);
    }

    #[test]
    fn leases_and_mutations_pin_runtimes_until_released() {
        let (mut state, mut runtimes, _) = visited_repos(7);
        Arc::make_mut(&mut state.watch_leases).insert(RepoId(1), 1);
        let lifetime = state.repos[1].lifetime();
        Arc::make_mut(&mut state.worktree_watch_leases)
            .insert((RepoId(2), lifetime, "/linked".into()), 1);
        state.repos[2].push_in_flight = 1;
        state.repos[3].diff_state.edit_mode = true;
        park_inactive(&mut state, &mut runtimes, SystemTime::now());
        for id in 1..=7 {
            assert!(runtimes.has_repository(&RepoId(id)));
        }
        Arc::make_mut(&mut state.watch_leases).clear();
        Arc::make_mut(&mut state.worktree_watch_leases).clear();
        state.repos[2].push_in_flight = 0;
        state.repos[3].diff_state.edit_mode = false;
        park_inactive(&mut state, &mut runtimes, SystemTime::now());
        for id in 1..=4 {
            assert!(!runtimes.has_repository(&RepoId(id)));
        }
        for id in 5..=7 {
            assert!(runtimes.has_repository(&RepoId(id)));
        }
    }

    #[test]
    fn hosted_documents_pin_backends_until_closed_even_under_pressure() {
        use crate::diff_session::{ChangeListSession, ChangeSource, DiffSession, DiffViewId};
        use gitcomet_core::domain::{DiffArea, DiffTarget};
        let (mut state, mut runtimes, _) = visited_repos(6);
        Arc::make_mut(&mut state.repos[0].diff_sessions).insert(
            DiffViewId::next(),
            DiffSession::new(DiffTarget::working_tree(
                "file.rs".into(),
                DiffArea::Unstaged,
            )),
        );
        Arc::make_mut(&mut state.repos[1].change_lists).insert(
            DiffViewId::next(),
            ChangeListSession::new(ChangeSource::worktree(DiffArea::Unstaged, true)),
        );
        park_inactive(&mut state, &mut runtimes, SystemTime::now());
        assert!(runtimes.has_repository(&RepoId(1)));
        assert!(runtimes.has_repository(&RepoId(2)));
        assert!(runtimes.begin_descriptor_recovery(SystemTime::now()));
        park_inactive(&mut state, &mut runtimes, SystemTime::now());
        assert!(runtimes.has_repository(&RepoId(1)));
        assert!(runtimes.has_repository(&RepoId(2)));
        Arc::make_mut(&mut state.repos[0].diff_sessions).clear();
        Arc::make_mut(&mut state.repos[1].change_lists).clear();
        park_inactive(&mut state, &mut runtimes, SystemTime::now());
        assert!(!runtimes.has_repository(&RepoId(1)));
        assert!(!runtimes.has_repository(&RepoId(2)));
    }

    #[test]
    fn pending_operation_prompts_pin_only_their_repository_until_dismissed() {
        use crate::model::{
            AuthPromptKind, AuthPromptState, AuthPromptStatus, AuthRetryOperation,
            BranchExistsPromptOperation, BranchExistsPromptState, ForceDeleteBranchPrompt,
            ForceRemoveWorktreePrompt, SubmoduleTrustCheckOperation, SubmoduleTrustCheckState,
            SubmoduleTrustPromptOperation, SubmoduleTrustPromptState,
        };
        let id = RepoId(1);
        let mut cases = vec![
            AppState {
                branch_exists_prompt: Some(BranchExistsPromptState {
                    repo_id: id,
                    name: "topic".into(),
                    target: "HEAD".into(),
                    operation: BranchExistsPromptOperation::CreateBranch,
                }),
                ..AppState::test_default()
            },
            AppState {
                force_delete_branch_prompt: Some(ForceDeleteBranchPrompt {
                    generation: 1,
                    repo_id: id,
                    name: "topic".into(),
                }),
                ..AppState::test_default()
            },
            AppState {
                force_remove_worktree_prompt: Some(ForceRemoveWorktreePrompt {
                    generation: 1,
                    repo_id: id,
                    path: "/linked".into(),
                    delete_branch: None,
                    blocker: gitcomet_core::error::WorktreeRemovalBlocker::Changes,
                }),
                ..AppState::test_default()
            },
            AppState {
                submodule_trust_prompt: Some(SubmoduleTrustPromptState {
                    repo_id: id,
                    operation: SubmoduleTrustPromptOperation::Update,
                    sources: Vec::new(),
                }),
                ..AppState::test_default()
            },
            AppState {
                submodule_trust_check_pending: Some(SubmoduleTrustCheckState {
                    repo_id: id,
                    operation: SubmoduleTrustCheckOperation::Update,
                }),
                ..AppState::test_default()
            },
        ];
        for operation in [
            AuthRetryOperation::RepoCommand {
                repo_id: id,
                command: crate::msg::RepoCommandKind::Push,
            },
            AuthRetryOperation::SafePushAfterCommit {
                repo_id: id,
                context: gitcomet_core::services::SafePushAfterCommitContext {
                    amend: false,
                    local_branch: Some("main".into()),
                    pre_head: None,
                    post_head: None,
                },
            },
            AuthRetryOperation::Commit {
                repo_id: id,
                message: "message".into(),
                amend: false,
                push_after_commit: false,
            },
        ] {
            for status in [
                AuthPromptStatus::AwaitingInput,
                AuthPromptStatus::Validating,
                AuthPromptStatus::Rejected,
            ] {
                cases.push(AppState {
                    auth_prompt: Some(AuthPromptState {
                        generation: 1,
                        challenge: None,
                        status,
                        kind: AuthPromptKind::UsernamePassword,
                        reason: "authentication required".into(),
                        operation: operation.clone(),
                    }),
                    ..AppState::test_default()
                });
            }
        }
        for (case, mut state) in cases.into_iter().enumerate() {
            let (visited, mut runtimes, _) = visited_repos(5);
            state.repos = visited.repos;
            state.active_repo = visited.active_repo;
            assert!(runtimes.begin_descriptor_recovery(SystemTime::now()));
            park_inactive(&mut state, &mut runtimes, SystemTime::now());
            assert!(runtimes.has_repository(&id), "pending continuation {case}");
            assert!(!runtimes.has_repository(&RepoId(2)), "unrelated repository");

            state.auth_prompt = None;
            state.branch_exists_prompt = None;
            state.force_delete_branch_prompt = None;
            state.force_remove_worktree_prompt = None;
            state.submodule_trust_prompt = None;
            state.submodule_trust_check_pending = None;
            park_inactive(&mut state, &mut runtimes, SystemTime::now());
            assert!(
                !runtimes.has_repository(&id),
                "dismissed continuation {case}"
            );
        }
    }

    #[test]
    fn bare_conversion_confirmation_pins_until_dismissed() {
        use gitcomet_core::domain::{
            BareConversionHead, BareConversionLayout, BareConversionPreview,
        };
        let (mut state, mut runtimes, _) = visited_repos(5);
        state.repos[0].bare_mode_conversion =
            BareModeConversionState::AwaitingConfirmation(Arc::new(BareConversionPreview::new(
                state.repos[0].spec.workdir.clone(),
                BareConversionLayout::BareDirectory,
                BareConversionHead::Branch {
                    name: "main".into(),
                    commit: None,
                },
                Vec::new(),
            )));
        assert!(runtimes.begin_descriptor_recovery(SystemTime::now()));
        park_inactive(&mut state, &mut runtimes, SystemTime::now());
        assert!(runtimes.has_repository(&RepoId(1)));
        assert!(!runtimes.has_repository(&RepoId(2)));
        state.repos[0].bare_mode_conversion = BareModeConversionState::Idle;
        park_inactive(&mut state, &mut runtimes, SystemTime::now());
        assert!(!runtimes.has_repository(&RepoId(1)));
    }

    #[test]
    fn clone_authentication_does_not_pin_existing_repository_tabs() {
        let (mut state, mut runtimes, _) = visited_repos(4);
        state.auth_prompt = Some(crate::model::AuthPromptState {
            generation: 1,
            challenge: None,
            status: crate::model::AuthPromptStatus::AwaitingInput,
            kind: crate::model::AuthPromptKind::UsernamePassword,
            reason: "authentication required".into(),
            operation: AuthRetryOperation::Clone {
                url: "https://example.com/repo.git".into(),
                dest: "/clone".into(),
            },
        });
        park_inactive(&mut state, &mut runtimes, SystemTime::now());
        assert!(!runtimes.has_repository(&RepoId(1)));
    }

    #[test]
    fn pressure_releases_all_warm_slots_and_retries_are_bounded() {
        let (mut state, mut runtimes, _) = visited_repos(4);
        let now = SystemTime::now();
        assert!(runtimes.begin_descriptor_recovery(now));
        assert!(!runtimes.begin_descriptor_recovery(now + Duration::from_secs(1)));
        assert_eq!(runtimes.warm_repository_count(now), 0);
        park_inactive(&mut state, &mut runtimes, now);
        for id in 1..=3 {
            assert!(!runtimes.has_repository(&RepoId(id)));
        }
        assert!(runtimes.has_repository(&RepoId(4)));
        assert!(runtimes.begin_descriptor_recovery(now + DESCRIPTOR_RECOVERY_COOLDOWN));
    }

    #[test]
    fn the_warm_allowance_returns_after_the_cooldown_and_survives_clock_steps() {
        let mut runtimes = RepositoryRuntimes::default();
        let now = SystemTime::now();
        assert!(runtimes.begin_descriptor_recovery(now));
        let just_before = now + DESCRIPTOR_RECOVERY_COOLDOWN - Duration::from_millis(1);
        assert_eq!(runtimes.warm_repository_count(just_before), 0);
        assert_eq!(
            runtimes.warm_repository_count(now - Duration::from_secs(5)),
            0
        );
        assert_eq!(
            runtimes.warm_repository_count(now + DESCRIPTOR_RECOVERY_COOLDOWN),
            WARM_INACTIVE_REPOSITORIES
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_read_failures_with_typed_descriptor_errors_request_recovery() {
        let exhausted = || {
            gitcomet_core::error::Error::from(std::io::Error::from_raw_os_error(
                rustix::io::Errno::MFILE.raw_os_error(),
            ))
        };
        assert_eq!(
            descriptor_read_failure(&InternalMsg::StatusLoaded {
                repo_id: RepoId(1),
                result: Err(exhausted()),
            }),
            Some(RepoId(1))
        );
        assert_eq!(
            descriptor_read_failure(&InternalMsg::CloneRepoFinished {
                url: "example".into(),
                dest: "/clone".into(),
                result: Err(exhausted()),
            }),
            None
        );
        assert_eq!(
            descriptor_read_failure(&InternalMsg::StatusLoaded {
                repo_id: RepoId(1),
                result: Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()),
            }),
            None
        );
    }
}
