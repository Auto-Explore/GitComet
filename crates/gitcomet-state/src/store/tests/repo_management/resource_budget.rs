use super::*;
use crate::msg::InternalMsg;
use crate::store::reducer::park_repository;

#[test]
fn commit_to_checked_push_keeps_its_backend_through_both_handoffs() {
    use crate::msg::{RepoCommandEffect, RepoCommandKind};
    use gitcomet_core::services::{
        CommitOperationOutcome, SafePushAfterCommitDecision, SafePushAfterCommitTarget,
    };
    for set_upstream in [false, true] {
        let mut repos = RepositoryRuntimes::default();
        let mut state = AppState::test_default();
        let ids = AtomicU64::new(1);
        let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/commit-background");
        state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::Commit {
                repo_id: id,
                message: "message".into(),
                push_after_commit: true,
            },
        );
        assert_eq!(state.repos[0].commit_in_flight, 1);
        for path in [
            "/tmp/commit-other-1",
            "/tmp/commit-other-2",
            "/tmp/commit-other-3",
        ] {
            open_repo_ready(&mut repos, &ids, &mut state, path);
            crate::store::repository_budget::park_inactive(&mut state, &mut repos);
        }
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::Internal(InternalMsg::CommitFinished {
                repo_id: id,
                result: Ok(CommitOperationOutcome::default()),
            }),
        );
        let context = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::RepoCommand(RepoCommandEffect::SafePushAfterCommit { context, .. }) => {
                    Some(context.clone())
                }
                _ => None,
            })
            .expect("commit schedules its safety check");
        assert_eq!(state.repos[0].commit_in_flight, 0);
        assert_eq!(state.repos[0].local_actions_in_flight, 1);
        assert!(repos.begin_descriptor_recovery(std::time::Instant::now()));
        crate::store::repository_budget::park_inactive(&mut state, &mut repos);
        assert!(
            repos.has_repository(&id),
            "preflight takes over the commit pin"
        );
        let target = SafePushAfterCommitTarget {
            remote: "origin".into(),
            branch: "main".into(),
            local_branch: "main".into(),
            local_head: gitcomet_core::domain::CommitId("a".repeat(40).into()),
        };
        let decision = if set_upstream {
            SafePushAfterCommitDecision::PushSetUpstream {
                target: target.clone(),
            }
        } else {
            SafePushAfterCommitDecision::Push {
                target: target.clone(),
            }
        };
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::Internal(InternalMsg::SafePushAfterCommitFinished {
                repo_id: id,
                context,
                auth: None,
                result: Ok(decision),
            }),
        );
        assert_eq!(state.repos[0].local_actions_in_flight, 0);
        assert_eq!(state.repos[0].push_in_flight, 1);
        assert!(effects.iter().any(|effect| matches!(effect,
            Effect::RepoCommand(RepoCommandEffect::PushAfterCommit { repo_id, set_upstream: actual, .. })
                if *repo_id == id && *actual == set_upstream
        )));
        crate::store::repository_budget::park_inactive(&mut state, &mut repos);
        assert!(
            repos.has_repository(&id),
            "checked push takes over the preflight pin"
        );
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::Internal(InternalMsg::RepoCommandFinished {
                repo_id: id,
                command: RepoCommandKind::PushAfterCommit {
                    target,
                    set_upstream,
                },
                result: Ok(gitcomet_core::services::CommandOutput::default()),
            }),
        );
        crate::store::repository_budget::park_inactive(&mut state, &mut repos);
        assert!(!repos.has_repository(&id), "finished push releases its pin");
    }
}

#[test]
fn safe_push_auth_retry_releases_preflight_on_all_terminal_outcomes() {
    use gitcomet_core::services::{SafePushAfterCommitContext, SafePushAfterCommitDecision};
    let _lock = super::super::staged_auth_test_lock();
    for blocked in [false, true] {
        let mut repos = RepositoryRuntimes::default();
        let mut state = AppState::test_default();
        let ids = AtomicU64::new(1);
        let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/preflight-background");
        state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
        let context = SafePushAfterCommitContext {
            amend: false,
            local_branch: Some("main".into()),
            pre_head: None,
            post_head: None,
        };
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::SafePushAfterCommit {
                repo_id: id,
                context: context.clone(),
            },
        );
        assert_eq!(state.repos[0].local_actions_in_flight, 1);
        for path in [
            "/tmp/preflight-other-1",
            "/tmp/preflight-other-2",
            "/tmp/preflight-other-3",
        ] {
            open_repo_ready(&mut repos, &ids, &mut state, path);
            crate::store::repository_budget::park_inactive(&mut state, &mut repos);
        }
        reduce(&mut repos, &ids, &mut state,
            Msg::Internal(InternalMsg::SafePushAfterCommitFinished {
                repo_id: id, context: context.clone(), auth: None,
                result: Err(Error::new(ErrorKind::Backend(
                    "fatal: could not read Password for 'https://alice@example.com': terminal prompts disabled".into(),
                ))),
            }));
        assert_eq!(state.repos[0].local_actions_in_flight, 0);
        let generation = state
            .auth_prompt
            .as_ref()
            .expect("preflight auth prompt")
            .generation;
        crate::store::repository_budget::park_inactive(&mut state, &mut repos);
        assert!(
            repos.has_repository(&id),
            "prompt takes over the preflight pin"
        );
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::SubmitAuthPrompt {
                generation,
                username: Some("alice".into()),
                secret: "test-password".into(),
            },
        );
        assert!(state.auth_prompt.is_none());
        assert_eq!(state.repos[0].local_actions_in_flight, 1);
        crate::store::repository_budget::park_inactive(&mut state, &mut repos);
        assert!(
            repos.has_repository(&id),
            "auth retry takes over the prompt pin"
        );
        let result = if blocked {
            Ok(SafePushAfterCommitDecision::Blocked {
                summary: "remote moved".into(),
                lease: None,
            })
        } else {
            Err(Error::new(ErrorKind::Backend("preflight failed".into())))
        };
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::Internal(InternalMsg::SafePushAfterCommitFinished {
                repo_id: id,
                context,
                auth: None,
                result,
            }),
        );
        assert_eq!(state.repos[0].local_actions_in_flight, 0);
        assert!(state.auth_prompt.is_none());
        assert_eq!(state.repos[0].push_in_flight, 0);
        crate::store::repository_budget::park_inactive(&mut state, &mut repos);
        assert!(
            !repos.has_repository(&id),
            "terminal preflight releases its pin"
        );
    }
}

#[test]
fn background_authentication_continuation_survives_leaving_the_warm_set() {
    use crate::msg::{RepoCommandEffect, RepoCommandKind};
    let _lock = super::super::staged_auth_test_lock();
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/auth-background");
    state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    reduce(&mut repos, &ids, &mut state, Msg::Push { repo_id: id });
    assert_eq!(state.repos[0].push_in_flight, 1);
    for path in [
        "/tmp/auth-other-1",
        "/tmp/auth-other-2",
        "/tmp/auth-other-3",
    ] {
        open_repo_ready(&mut repos, &ids, &mut state, path);
        crate::store::repository_budget::park_inactive(&mut state, &mut repos);
    }
    assert!(
        repos.has_repository(&id),
        "in-flight command pins its backend"
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id: id,
            command: RepoCommandKind::Push,
            result: Err(Error::new(ErrorKind::Backend(
                "fatal: could not read Password for 'https://alice@example.com': terminal prompts disabled".into(),
            ))),
        }),
    );
    assert_eq!(state.repos[0].push_in_flight, 0);
    let generation = state
        .auth_prompt
        .as_ref()
        .expect("authentication prompt")
        .generation;
    crate::store::repository_budget::park_inactive(&mut state, &mut repos);
    assert!(
        repos.has_repository(&id),
        "prompt takes over the command's pin"
    );
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SubmitAuthPrompt {
            generation,
            username: Some("alice".into()),
            secret: "test-password".into(),
        },
    );
    assert!(state.auth_prompt.is_none());
    assert_eq!(state.repos[0].push_in_flight, 1);
    assert!(effects.iter().any(|effect| matches!(effect,
        Effect::RepoCommand(RepoCommandEffect::Push { repo_id, .. }) if *repo_id == id
    )));
    crate::store::repository_budget::park_inactive(&mut state, &mut repos);
    assert!(
        repos.has_repository(&id),
        "retry takes over the prompt's pin"
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id: id,
            command: RepoCommandKind::Push,
            result: Ok(gitcomet_core::services::CommandOutput::default()),
        }),
    );
    crate::store::repository_budget::park_inactive(&mut state, &mut repos);
    assert!(
        !repos.has_repository(&id),
        "finished continuation releases its pin"
    );
}

#[test]
fn hosted_requests_wait_for_cold_or_initial_background_open_and_keep_the_latest_target() {
    use crate::diff_session::{ChangeSource, DiffSessionMsg, DiffSessionWork, DiffViewId};
    use gitcomet_core::domain::{DiffArea, DiffTarget};
    for initial in [false, true] {
        let mut repos = RepositoryRuntimes::default();
        let mut state = AppState::test_default();
        let ids = AtomicU64::new(1);
        let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/hosted-cold");
        state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
        let active = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/hosted-active");
        park_repository(&mut state.repos[0], &mut repos);
        repos.release_retired();
        if initial {
            state.repos[0].backend_parked = false;
            state.repos[0].repository_mode = None;
            state.repos[0].open = Loadable::NotLoaded;
        }
        let lifetime = state.repos[0].lifetime();
        let view = DiffViewId::next();
        let mut opens = 0;
        for path in ["old.rs", "latest.rs"] {
            let effects = reduce(
                &mut repos,
                &ids,
                &mut state,
                Msg::DiffSession(DiffSessionMsg::Open {
                    repo_id: id,
                    lifetime,
                    view,
                    target: DiffTarget::working_tree(path.into(), DiffArea::Unstaged),
                }),
            );
            opens += effects
                .iter()
                .filter(|effect| matches!(effect, Effect::OpenRepo { .. }))
                .count();
            assert!(
                !effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::DiffSession(_)))
            );
        }
        let list = DiffViewId::next();
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::DiffSession(DiffSessionMsg::OpenChanges {
                repo_id: id,
                lifetime,
                view: list,
                source: ChangeSource::worktree(DiffArea::Unstaged, true),
            }),
        );
        assert!(
            effects.is_empty(),
            "the pending backend is opened only once"
        );
        assert_eq!(opens, 1);
        assert!(!state.repos[0].diff_sessions[&view].is_loading());
        assert!(!state.repos[0].change_lists[&list].is_loading());
        let recent = state.repos[0].last_active_at;
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::DiffSession(DiffSessionMsg::Close {
                repo_id: id,
                lifetime: lifetime.wrapping_add(1),
                view,
            }),
        );
        assert!(effects.is_empty());
        assert_eq!(state.repos[0].last_active_at, recent);
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::Internal(InternalMsg::RepoOpenedOk {
                repo_id: id,
                spec: RepoSpec {
                    workdir: "/tmp/hosted-cold".into(),
                },
                repo: Arc::new(DummyRepo::new("/tmp/hosted-cold")),
                preferences: None,
                repository_mode: gitcomet_core::domain::RepositoryMode::WorkingTree,
                head_commit: None,
            }),
        );
        assert_eq!(state.active_repo, Some(active));
        assert!(effects.iter().any(|effect| matches!(effect, Effect::DiffSession(work)
            if work.view == view && matches!(&work.work, DiffSessionWork::Content { target, .. } if target.file_path() == Some(std::path::Path::new("latest.rs"))))));
        assert!(effects.iter().any(|effect| matches!(effect, Effect::DiffSession(work) if work.view == list && matches!(work.work, DiffSessionWork::Changes { .. }))));
    }
}

#[test]
fn parked_tab_reopens_without_resetting_its_lifetime_or_navigation() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/parked-repo");
    let repo = &mut state.repos[0];
    let lifetime = repo.lifetime_id();
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    repo.suggested_commit_message = Some("keep this draft".into());
    repo.file_browser.search_query = "needle".into();
    let snapshot = repo.main_view_snapshot();
    repo.navigation.main_history.reconcile(snapshot, true);
    park_repository(repo, &mut repos);
    repos.release_retired();

    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SetActiveRepo { repo_id: id },
    );
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::OpenRepo { .. }))
            .count(),
        1
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadWorktrees { .. }))
    );
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoOpenedOk {
            repo_id: id,
            spec: RepoSpec {
                workdir: "/tmp/parked-repo".into(),
            },
            repo: Arc::new(DummyRepo::new("/tmp/parked-repo")),
            preferences: None,
            repository_mode: gitcomet_core::domain::RepositoryMode::WorkingTree,
            head_commit: None,
        }),
    );
    let repo = &state.repos[0];
    assert_eq!(repo.lifetime_id(), lifetime);
    assert!(!repo.backend_parked);
    assert_eq!(
        repo.suggested_commit_message.as_deref(),
        Some("keep this draft")
    );
    assert_eq!(repo.file_browser.search_query, "needle");
    assert!(!repo.navigation.main_history.entries.is_empty());
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadWorktrees { .. }))
    );
}

#[cfg(unix)]
fn exhausted() -> Error {
    std::io::Error::from_raw_os_error(rustix::io::Errno::MFILE.raw_os_error()).into()
}

#[test]
fn a_file_selected_during_cold_reopen_reloads_its_decoding_attributes() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/cold-editor");
    state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    park_repository(&mut state.repos[0], &mut repos);
    repos.release_retired();
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SetActiveRepo { repo_id: id },
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::OpenFileEditor {
            repo_id: id,
            path: "file.rs".into(),
        },
    );
    assert!(matches!(
        state.repos[0].diff_state.text_attributes,
        Loadable::Loading
    ));
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoOpenedOk {
            repo_id: id,
            spec: RepoSpec {
                workdir: "/tmp/cold-editor".into(),
            },
            repo: Arc::new(DummyRepo::new("/tmp/cold-editor")),
            preferences: None,
            repository_mode: gitcomet_core::domain::RepositoryMode::WorkingTree,
            head_commit: None,
        }),
    );
    assert!(state.repos[0].diff_state.edit_mode);
    assert!(
        effects.iter().any(
            |effect| matches!(effect, Effect::LoadSelectedDiff { repo_id, .. } if *repo_id == id)
        ),
        "content-only resume effects cannot finish the editor's attributes read"
    );
}

#[cfg(unix)]
#[test]
fn exhausted_open_retries_once_and_keeps_an_unvalidated_drop_tab() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let path = PathBuf::from("/tmp/descriptor-limited-repo");
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::OpenRepoFromExternalDrop(path.clone()),
    );
    let id = state.active_repo.unwrap();
    let failure = || {
        Msg::Internal(InternalMsg::RepoOpenedErr {
            repo_id: id,
            spec: RepoSpec {
                workdir: path.clone(),
            },
            error: exhausted(),
        })
    };
    let effects = reduce(&mut repos, &ids, &mut state, failure());
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { .. }))
    );
    let effects = reduce(&mut repos, &ids, &mut state, failure());
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { .. }))
    );
    assert_eq!(state.repos.len(), 1);
    assert!(state.repos[0].is_provisional_external_drop_open());
    assert!(matches!(state.repos[0].open, Loadable::Error(_)));
    assert_eq!(
        state.repos[0].descriptor_retry_epoch,
        Some(state.repos[0].load_epoch)
    );
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SetActiveRepo { repo_id: id },
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { .. })),
        "explicit activation can retry after the automatic attempt is exhausted"
    );
}

#[test]
fn acquiring_a_document_lease_wakes_a_parked_tab_without_activating_it() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/document-repo");
    let lifetime = state.repos[0].lifetime();
    park_repository(&mut state.repos[0], &mut repos);
    state.active_repo = None;
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::AcquireWatchLease {
            repo_id: id,
            lifetime,
        },
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { repo_id, .. } if *repo_id == id))
    );
    assert_eq!(state.active_repo, None);
    assert_eq!(state.watch_leases.get(&id), Some(&1));
}

#[test]
fn explicit_background_reload_reopens_and_refreshes_a_cold_tab() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let cold = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/cold-reload");
    state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    let active = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/active-reload");
    park_repository(&mut state.repos[0], &mut repos);
    repos.release_retired();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::ReloadRepo { repo_id: cold },
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { repo_id, .. } if *repo_id == cold))
    );
    assert_eq!(state.active_repo, Some(active));
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoOpenedOk {
            repo_id: cold,
            spec: RepoSpec {
                workdir: "/tmp/cold-reload".into(),
            },
            repo: Arc::new(DummyRepo::new("/tmp/cold-reload")),
            preferences: None,
            repository_mode: gitcomet_core::domain::RepositoryMode::WorkingTree,
            head_commit: None,
        }),
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadWorktrees { repo_id } if *repo_id == cold))
    );
    crate::store::repository_budget::park_inactive(&mut state, &mut repos);
    assert!(
        repos.has_repository(&cold),
        "explicit use keeps the resumed handle warm"
    );
    assert_eq!(state.active_repo, Some(active));
}

#[cfg(unix)]
#[test]
fn descriptor_pressure_keeps_inactive_document_leases_attached() {
    for worktree_lease in [false, true] {
        let mut repos = RepositoryRuntimes::default();
        let mut state = AppState::test_default();
        let ids = AtomicU64::new(1);
        let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/pinned-document");
        let lifetime = state.repos[0].lifetime();
        if worktree_lease {
            Arc::make_mut(&mut state.worktree_watch_leases)
                .insert((id, lifetime, PathBuf::from("/tmp/pinned-document")), 1);
        } else {
            Arc::make_mut(&mut state.watch_leases).insert(id, 1);
        }
        open_repo_ready(&mut repos, &ids, &mut state, "/tmp/other-document");
        let epoch = state.repos[0].load_epoch;
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::Internal(InternalMsg::RepoLoadFinished {
                repo_id: id,
                load_epoch: epoch,
                message: Box::new(InternalMsg::StatusLoaded {
                    repo_id: id,
                    result: Err(exhausted()),
                }),
            }),
        );
        assert!(repos.has_repository(&id));
        assert!(!state.repos[0].is_backend_parked());
        assert_eq!(state.repos[0].load_epoch, epoch);
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::OpenRepo { .. }))
        );
    }
}

#[cfg(unix)]
#[test]
fn stale_exhaustion_does_not_reopen_or_evict_a_newer_runtime() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/current-repo");
    let old_epoch = state.repos[0].bump_load_epoch();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoLoadFinished {
            repo_id: id,
            load_epoch: old_epoch,
            message: Box::new(InternalMsg::StatusLoaded {
                repo_id: id,
                result: Err(exhausted()),
            }),
        }),
    );
    assert!(effects.is_empty());
    assert!(repos.has_repository(&id));
    assert_eq!(repos.warm_repository_count(), 2);
}
