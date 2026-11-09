use super::*;
use crate::msg::InternalMsg;
use crate::store::reducer::park_repository;

fn park(state: &mut AppState, repos: &mut RepositoryRuntimes) {
    crate::store::repository_budget::park_inactive(state, repos, std::time::SystemTime::now());
}

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
                request: crate::model::CommitRequest {
                    amend: false,
                    message: "message".into(),
                    push_after_commit: true,
                    hooks: gitcomet_core::services::GitHooks::Run,
                },
            },
        );
        assert_eq!(state.repos[0].commit_in_flight, 1);
        for path in [
            "/tmp/commit-other-1",
            "/tmp/commit-other-2",
            "/tmp/commit-other-3",
        ] {
            open_repo_ready(&mut repos, &ids, &mut state, path);
            park(&mut state, &mut repos);
        }
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::Internal(InternalMsg::CommitFinished {
                amend: false,
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
        assert_eq!(state.repos[0].push_preflights_in_flight, 1);
        assert_eq!(state.repos[0].local_actions_in_flight, 0);
        assert!(!state.repos[0].history_rewrite_busy());
        assert!(repos.begin_descriptor_recovery(std::time::SystemTime::now()));
        park(&mut state, &mut repos);
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
        assert_eq!(state.repos[0].push_preflights_in_flight, 0);
        assert_eq!(state.repos[0].push_in_flight, 1);
        assert!(effects.iter().any(|effect| matches!(effect,
            Effect::RepoCommand(RepoCommandEffect::PushAfterCommit { repo_id, set_upstream: actual, .. })
                if *repo_id == id && *actual == set_upstream
        )));
        park(&mut state, &mut repos);
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
                    hooks: gitcomet_core::services::GitHooks::Run,
                },
                result: Ok(gitcomet_core::services::CommandOutput::default()),
            }),
        );
        park(&mut state, &mut repos);
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
            hooks: gitcomet_core::services::GitHooks::Run,
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
        assert_eq!(state.repos[0].push_preflights_in_flight, 1);
        for path in [
            "/tmp/preflight-other-1",
            "/tmp/preflight-other-2",
            "/tmp/preflight-other-3",
        ] {
            open_repo_ready(&mut repos, &ids, &mut state, path);
            park(&mut state, &mut repos);
        }
        reduce(&mut repos, &ids, &mut state,
            Msg::Internal(InternalMsg::SafePushAfterCommitFinished {
                repo_id: id, context: context.clone(), auth: None,
                result: Err(Error::new(ErrorKind::Backend(
                    "fatal: could not read Password for 'https://alice@example.com': terminal prompts disabled".into(),
                ))),
            }));
        assert_eq!(state.repos[0].push_preflights_in_flight, 0);
        let generation = state
            .auth_prompt
            .as_ref()
            .expect("preflight auth prompt")
            .generation;
        park(&mut state, &mut repos);
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
        assert_eq!(state.repos[0].push_preflights_in_flight, 1);
        park(&mut state, &mut repos);
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
        assert_eq!(state.repos[0].push_preflights_in_flight, 0);
        assert!(state.auth_prompt.is_none());
        assert_eq!(state.repos[0].push_in_flight, 0);
        park(&mut state, &mut repos);
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
        park(&mut state, &mut repos);
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
    park(&mut state, &mut repos);
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
    park(&mut state, &mut repos);
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
    park(&mut state, &mut repos);
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
                    request: None,
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
        assert!(state.repos[0].diff_sessions[&view].is_loading());
        assert!(state.repos[0].change_lists[&list].is_loading());
        let recent = state.repos[0].last_background_use_at;
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
        assert_eq!(state.repos[0].last_background_use_at, recent);
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
    park(&mut state, &mut repos);
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
    assert_eq!(repos.warm_repository_count(std::time::SystemTime::now()), 2);
}

#[cfg(unix)]
#[test]
fn a_push_finishing_after_its_tab_goes_cold_parks_without_failing_its_refresh() {
    use super::super::worktree_redirect::{open_repo_and_wait, wait_until};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = gitcomet_core::path_utils::canonicalize_or_original(dir.path().to_path_buf());
    let remote = root.join("remote.git");
    std::fs::create_dir(&remote).unwrap();
    super::super::run_git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let tabs: Vec<PathBuf> = ["pushing", "b", "c", "d"]
        .into_iter()
        .map(|name| {
            let repo = root.join(name);
            std::fs::create_dir(&repo).unwrap();
            super::super::run_git(&repo, &["init", "-q", "-b", "main"]);
            for (key, value) in [
                ("user.name", "Test User"),
                ("user.email", "test@example.com"),
                ("commit.gpgsign", "false"),
            ] {
                super::super::run_git(&repo, &["config", key, value]);
            }
            super::super::run_git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
            repo
        })
        .collect();
    let pushing = &tabs[0];
    super::super::run_git(
        pushing,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    super::super::run_git(pushing, &["push", "-q", "-u", "origin", "main"]);
    super::super::run_git(pushing, &["commit", "-q", "--allow-empty", "-m", "second"]);
    let hooks = root.join("hooks");
    std::fs::create_dir(&hooks).unwrap();
    let release = root.join("release");
    let hook = hooks.join("pre-push");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\nwhile [ ! -e '{}' ]; do sleep 0.02; done\n",
            release.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    super::super::run_git(
        pushing,
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );

    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let id = open_repo_and_wait(&store, pushing);
    let tab = |store: &AppStore| {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == id)
            .cloned()
            .expect("pushing tab")
    };
    store.dispatch(Msg::Push { repo_id: id });
    wait_until("push to start", || tab(&store).push_in_flight == 1);
    for path in &tabs[1..] {
        open_repo_and_wait(&store, path);
    }
    assert!(
        !tab(&store).is_backend_parked(),
        "the push pins its backend"
    );
    std::fs::write(&release, "").unwrap();
    wait_until("push to finish and its tab to park", || {
        let repo = tab(&store);
        repo.push_in_flight == 0 && repo.is_backend_parked()
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    let errors: Vec<_> = tab(&store)
        .feedback
        .diagnostics
        .iter()
        .filter(|entry| entry.kind == DiagnosticKind::Error)
        .map(|entry| entry.message.clone())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    store.shutdown_for_test().recv().unwrap();
}

#[cfg(unix)]
#[test]
fn an_exhausted_read_on_the_active_tab_parks_idle_tabs_and_reopens_once() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let idle = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/recovery-idle");
    let active = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/recovery-active");
    for repo in &mut state.repos {
        repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    }
    let status_failed = |state: &AppState| {
        Msg::Internal(InternalMsg::RepoLoadFinished {
            repo_id: active,
            load_epoch: state.repos[1].load_epoch,
            message: Box::new(InternalMsg::StatusLoaded {
                repo_id: active,
                result: Err(exhausted()),
            }),
        })
    };
    let reopens = |effects: &[Effect]| {
        effects
            .iter()
            .filter(
                |effect| matches!(effect, Effect::OpenRepo { repo_id, .. } if *repo_id == active),
            )
            .count()
    };
    let failure = status_failed(&state);
    let effects = reduce(&mut repos, &ids, &mut state, failure);
    assert_eq!(reopens(&effects), 1);
    assert!(!repos.has_repository(&idle), "pressure parks the warm tab");
    assert!(state.repos[1].is_backend_parked());
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoOpenedOk {
            repo_id: active,
            spec: RepoSpec {
                workdir: "/tmp/recovery-active".into(),
            },
            repo: Arc::new(DummyRepo::new("/tmp/recovery-active")),
            preferences: None,
            repository_mode: gitcomet_core::domain::RepositoryMode::WorkingTree,
            head_commit: None,
        }),
    );
    let failure = status_failed(&state);
    let effects = reduce(&mut repos, &ids, &mut state, failure);
    assert_eq!(reopens(&effects), 0, "the retry's failure is not retried");
    assert!(matches!(state.repos[1].status, Loadable::Error(_)));
}

fn open_hosted_reads(
    repos: &mut RepositoryRuntimes,
    ids: &AtomicU64,
    state: &mut AppState,
    id: RepoId,
) -> (
    Vec<Effect>,
    crate::diff_session::DiffViewId,
    crate::diff_session::DiffViewId,
) {
    use crate::diff_session::{ChangeSource, DiffSessionMsg, DiffViewId};
    use gitcomet_core::domain::{DiffArea, DiffTarget};
    let lifetime = state
        .repos
        .iter()
        .find(|repo| repo.id == id)
        .unwrap()
        .lifetime();
    let (view, list) = (DiffViewId::next(), DiffViewId::next());
    let mut effects = reduce(
        repos,
        ids,
        state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id: id,
            lifetime,
            view,
            request: None,
            target: DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged),
        }),
    );
    effects.extend(reduce(
        repos,
        ids,
        state,
        Msg::DiffSession(DiffSessionMsg::OpenChanges {
            repo_id: id,
            lifetime,
            view: list,
            source: ChangeSource::worktree(DiffArea::Unstaged, true),
        }),
    ));
    (effects, view, list)
}

#[test]
fn a_failed_open_answers_the_hosted_reads_waiting_for_it() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/hosted-open-fails");
    open_repo_ready(
        &mut repos,
        &ids,
        &mut state,
        "/tmp/hosted-open-fails-active",
    );
    park_repository(&mut state.repos[0], &mut repos);
    repos.release_retired();
    state.repos[0].backend_parked = false;
    state.repos[0].open = Loadable::NotLoaded;
    let (effects, view, list) = open_hosted_reads(&mut repos, &ids, &mut state, id);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { .. }))
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoOpenedErr {
            repo_id: id,
            spec: RepoSpec {
                workdir: "/tmp/hosted-open-fails".into(),
            },
            error: std::io::Error::from(std::io::ErrorKind::PermissionDenied).into(),
        }),
    );
    let repo = &state.repos[0];
    assert!(!repo.diff_sessions[&view].is_loading());
    assert!(matches!(repo.diff_sessions[&view].diff, Loadable::Error(_)));
    assert!(!repo.change_lists[&list].is_loading());
    assert!(matches!(repo.change_lists[&list].files, Loadable::Error(_)));
}

#[test]
fn hosted_reads_on_a_tab_whose_open_failed_are_answered_at_once() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/hosted-failed-tab");
    open_repo_ready(
        &mut repos,
        &ids,
        &mut state,
        "/tmp/hosted-failed-tab-active",
    );
    park_repository(&mut state.repos[0], &mut repos);
    repos.release_retired();
    state.repos[0].backend_parked = false;
    state.repos[0].open = Loadable::Error("permission denied".into());
    let (effects, view, list) = open_hosted_reads(&mut repos, &ids, &mut state, id);
    assert!(effects.is_empty());
    let repo = &state.repos[0];
    assert!(
        matches!(&repo.diff_sessions[&view].diff, Loadable::Error(message) if message == "permission denied")
    );
    assert!(matches!(repo.change_lists[&list].files, Loadable::Error(_)));
}

#[test]
fn a_cancelled_open_settles_waiting_hosted_reads_until_the_next_open() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::OpenRepo("/tmp/hosted-cancelled".into()),
    );
    let id = state.active_repo.unwrap();
    let (_, view, list) = open_hosted_reads(&mut repos, &ids, &mut state, id);
    open_repo_ready(&mut repos, &ids, &mut state, "/tmp/hosted-cancelled-other");
    let repo = &state.repos[0];
    assert!(matches!(repo.open, Loadable::NotLoaded));
    assert!(!repo.diff_sessions[&view].is_loading());
    assert!(matches!(
        repo.diff_sessions[&view].diff,
        Loadable::NotLoaded
    ));
    assert!(!repo.change_lists[&list].is_loading());
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SetActiveRepo { repo_id: id },
    );
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoOpenedOk {
            repo_id: id,
            spec: RepoSpec {
                workdir: "/tmp/hosted-cancelled".into(),
            },
            repo: Arc::new(DummyRepo::new("/tmp/hosted-cancelled")),
            preferences: None,
            repository_mode: gitcomet_core::domain::RepositoryMode::WorkingTree,
            head_commit: None,
        }),
    );
    for expected in [view, list] {
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::DiffSession(work) if work.view == expected)),
            "{expected:?} reloads on the next open"
        );
    }
}

#[test]
fn a_maintenance_recommendation_for_a_parked_tab_reopens_it() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/maintenance-parked");
    state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    state.repos[0].common_dir = Some(Arc::from(std::path::Path::new(
        "/tmp/maintenance-parked/.git",
    )));
    open_repo_ready(
        &mut repos,
        &ids,
        &mut state,
        "/tmp/maintenance-parked-active",
    );
    park_repository(&mut state.repos[0], &mut repos);
    repos.release_retired();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoMaintenanceChecked {
            repo_id: id,
            needed: true,
        }),
    );
    assert!(state.repos[0].maintenance.recommended);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { repo_id, .. } if *repo_id == id))
    );
}

#[test]
fn background_hosted_use_keeps_the_budget_warm_without_skipping_the_switch_refresh() {
    use crate::diff_session::{DiffSessionMsg, DiffViewId};
    use gitcomet_core::domain::{DiffArea, DiffTarget};
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let background = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/hot-switch-background");
    open_repo_ready(&mut repos, &ids, &mut state, "/tmp/hot-switch-active");
    let repo = &mut state.repos[0];
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    repo.set_branches(Loadable::Ready(Vec::new()));
    repo.set_tags(Loadable::Ready(Vec::new()));
    repo.set_remotes(Loadable::Ready(Vec::new()));
    repo.set_remote_branches(Loadable::Ready(Vec::new()));
    repo.set_stashes(Loadable::Ready(Vec::new()));
    repo.set_rebase_in_progress(Loadable::Ready(false));
    repo.set_merge_commit_message(Loadable::Ready(None));
    let an_hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    repo.last_active_at = Some(an_hour_ago);
    let lifetime = repo.lifetime();
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id: background,
            lifetime,
            view: DiffViewId::next(),
            request: None,
            target: DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged),
        }),
    );
    assert_eq!(state.repos[0].last_active_at, Some(an_hour_ago));
    assert!(state.repos[0].last_used_at() > Some(an_hour_ago));
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SetActiveRepo {
            repo_id: background,
        },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::References(ReferenceEffect::LoadRemotes { .. })
        )),
        "a tab last viewed an hour ago refreshes its remotes, tags and stashes"
    );
}

#[test]
fn a_resumed_history_walk_streams_into_a_loading_log() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/resumed-log");
    state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    open_repo_ready(&mut repos, &ids, &mut state, "/tmp/resumed-log-active");
    park_repository(&mut state.repos[0], &mut repos);
    repos.release_retired();
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SetActiveRepo { repo_id: id },
    );
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoOpenedOk {
            repo_id: id,
            spec: RepoSpec {
                workdir: "/tmp/resumed-log".into(),
            },
            repo: Arc::new(DummyRepo::new("/tmp/resumed-log")),
            preferences: None,
            repository_mode: gitcomet_core::domain::RepositoryMode::WorkingTree,
            head_commit: None,
        }),
    );
    let seq = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadLog { seq, .. } => Some(*seq),
            _ => None,
        })
        .expect("the resume walks history");
    assert!(state.repos[0].log.is_loading());
    assert!(state.repos[0].status.is_loading());
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::LogChunkLoaded {
            repo_id: id,
            seq,
            commits: Vec::new(),
            scanned: 42,
        }),
    );
    assert_eq!(state.repos[0].history_state.log_scan_progress, Some(42));
}

#[cfg(unix)]
#[test]
fn descriptor_recovery_reloads_a_comparison_that_was_loading() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/recovery-comparison");
    let repo = &mut state.repos[0];
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    repo.set_range_selection(Some(crate::model::RangeSelection::new(
        gitcomet_core::domain::CommitId("a".repeat(40).into()),
        None,
        "base".into(),
        "Working tree".into(),
    )));
    repo.set_range_files(Loadable::Loading);
    let stranded = repo.begin_range_files_load();
    let failure = Msg::Internal(InternalMsg::RepoLoadFinished {
        repo_id: id,
        load_epoch: repo.load_epoch,
        message: Box::new(InternalMsg::StatusLoaded {
            repo_id: id,
            result: Err(exhausted()),
        }),
    });
    reduce(&mut repos, &ids, &mut state, failure);
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoOpenedOk {
            repo_id: id,
            spec: RepoSpec {
                workdir: "/tmp/recovery-comparison".into(),
            },
            repo: Arc::new(DummyRepo::new("/tmp/recovery-comparison")),
            preferences: None,
            repository_mode: gitcomet_core::domain::RepositoryMode::WorkingTree,
            head_commit: None,
        }),
    );
    let history = &state.repos[0].history_state;
    let reissued = effects.iter().find_map(|effect| match effect {
        Effect::LoadRangeFiles { request, .. } => Some(*request),
        _ => None,
    });
    assert_eq!(reissued, Some(history.range_files_request));
    assert_ne!(reissued, Some(stranded));
    assert!(history.range_files_in_flight);
    assert!(history.range_files.is_loading());
}

#[test]
fn a_parked_tab_whose_repository_is_gone_stays_open_with_its_error() {
    let mut repos = RepositoryRuntimes::default();
    let mut state = AppState::test_default();
    let ids = AtomicU64::new(1);
    let id = open_repo_ready(&mut repos, &ids, &mut state, "/tmp/gone-repo");
    state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    open_repo_ready(&mut repos, &ids, &mut state, "/tmp/gone-repo-active");
    park_repository(&mut state.repos[0], &mut repos);
    repos.release_retired();
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SetActiveRepo { repo_id: id },
    );
    let failure = || {
        Msg::Internal(InternalMsg::RepoOpenedErr {
            repo_id: id,
            spec: RepoSpec {
                workdir: "/tmp/gone-repo".into(),
            },
            error: Error::new(ErrorKind::NotARepository),
        })
    };
    reduce(&mut repos, &ids, &mut state, failure());
    let repo = state
        .repos
        .iter()
        .find(|repo| repo.id == id)
        .expect("the tab stays open");
    assert!(matches!(repo.open, Loadable::Error(_)));
    assert_eq!(state.active_repo, Some(id));
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SetActiveRepo { repo_id: id },
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { repo_id, .. } if *repo_id == id)),
        "activation retries the open"
    );
}

#[test]
fn reading_a_parked_tab_opens_a_handle_without_waking_it() {
    use super::super::worktree_redirect::{open_repo_and_wait, wait_until};
    let dir = tempfile::tempdir().unwrap();
    let root = gitcomet_core::path_utils::canonicalize_or_original(dir.path().to_path_buf());
    let paths: Vec<PathBuf> = ["parked", "b", "c", "d"]
        .into_iter()
        .map(|name| {
            let repo = root.join(name);
            std::fs::create_dir(&repo).unwrap();
            super::super::run_git(&repo, &["init", "-q", "-b", "main"]);
            repo
        })
        .collect();
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let id = open_repo_and_wait(&store, &paths[0]);
    for path in &paths[1..] {
        open_repo_and_wait(&store, path);
    }
    let tab = |store: &AppStore| {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == id)
            .cloned()
            .expect("parked tab")
    };
    wait_until("the oldest tab to park", || tab(&store).is_backend_parked());
    let handle = store
        .repository(id, tab(&store).lifetime())
        .expect("a parked tab is still open");
    assert_eq!(handle.spec().workdir, paths[0]);
    assert!(tab(&store).is_backend_parked(), "reading does not wake it");
    store.shutdown_for_test().recv().unwrap();
}
