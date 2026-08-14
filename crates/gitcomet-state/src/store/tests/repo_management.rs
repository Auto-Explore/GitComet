use super::*;
use crate::model::{
    GitLogSettings, GitLogTagFetchMode, RepoLoadsInFlight, SidebarDataRequest, SidebarMode,
};

fn mark_repo_switch_secondary_metadata_ready(repo: &mut RepoState) {
    repo.branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.remotes = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.rebase_in_progress = Loadable::Ready(false);
    repo.merge_commit_message = Loadable::Ready(None);
}

fn has_full_refresh_only_effects(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadRemotes { repo_id: candidate }
                | Effect::LoadRemoteBranches { repo_id: candidate }
                if *candidate == repo_id
        )
    })
}

fn has_worktree_refresh_effect(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadWorktrees { repo_id: candidate } if *candidate == repo_id
        )
    })
}

fn has_cancel_repo_loads_effect(effects: &[Effect], repo_id: RepoId, load_epoch: u64) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::CancelRepoLoads {
                repo_id: candidate,
                load_epoch: candidate_epoch,
            } if *candidate == repo_id && *candidate_epoch == load_epoch
        )
    })
}

fn has_submodule_load_effect(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadSubmodules { repo_id: candidate } if *candidate == repo_id
        )
    })
}

fn has_stash_load_effect(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadStashes {
                repo_id: candidate,
                limit: 50
            } if *candidate == repo_id
        )
    })
}

#[test]
fn worker_command_prioritizes_close_repo_over_queued_background_result() {
    let repo_id = RepoId(7);
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
        crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Ok(Vec::new()),
        },
    ))))
    .expect("send background result");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CloseRepo {
        repo_id,
    })))
    .expect("send close");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::CloseRepo { repo_id: got } if got == repo_id));
        }
        _ => panic!("expected close repo command first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("deferred command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(
                *msg,
                Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
                    repo_id: got,
                    ..
                }) if got == repo_id
            ));
        }
        _ => panic!("expected deferred tags result"),
    }
}

#[test]
fn worker_command_prioritizes_close_repo_over_queued_open_error() {
    let repo_id = RepoId(7);
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
        crate::msg::InternalMsg::RepoOpenedErr {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/not-a-repo"),
            },
            error: Error::new(ErrorKind::NotARepository),
        },
    ))))
    .expect("send open error");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CloseRepo {
        repo_id,
    })))
    .expect("send close");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::CloseRepo { repo_id: got } if got == repo_id));
        }
        _ => panic!("expected close repo command first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("deferred open error");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(
                *msg,
                Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
                    repo_id: got,
                    ..
                }) if got == repo_id
            ));
        }
        _ => panic!("expected deferred open error"),
    }
}

#[test]
fn worker_command_prioritizes_close_repos_over_queued_background_result() {
    let repo_id = RepoId(7);
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
        crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Ok(Vec::new()),
        },
    ))))
    .expect("send background result");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CloseRepos {
        repo_ids: vec![repo_id],
        activate_after: None,
    })))
    .expect("send close repos");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(
                *msg,
                Msg::CloseRepos {
                    repo_ids,
                    activate_after: None,
                } if repo_ids == vec![repo_id]
            ));
        }
        _ => panic!("expected close repos command first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("deferred tags result");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(
                *msg,
                Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
                    repo_id: got,
                    ..
                }) if got == repo_id
            ));
        }
        _ => panic!("expected deferred tags result"),
    }
}

#[test]
fn worker_command_prioritizes_tab_switch_over_queued_background_result() {
    let repo_id = RepoId(7);
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
        crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Ok(Vec::new()),
        },
    ))))
    .expect("send background result");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::SetActiveRepo {
        repo_id,
    })))
    .expect("send tab switch");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::SetActiveRepo { repo_id: got } if got == repo_id));
        }
        _ => panic!("expected tab switch first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("deferred tags result");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(
                *msg,
                Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
                    repo_id: got,
                    ..
                }) if got == repo_id
            ));
        }
        _ => panic!("expected deferred tags result"),
    }
}

#[test]
fn worker_command_keeps_queued_open_repo_before_close_repo() {
    let repo_id = RepoId(7);
    let path = PathBuf::from("/tmp/repo");
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::OpenRepo(
        path.clone(),
    ))))
    .expect("send open");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CloseRepo {
        repo_id,
    })))
    .expect("send close");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::OpenRepo(got) if got == path));
        }
        _ => panic!("expected open repo command first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("queued close");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::CloseRepo { repo_id: got } if got == repo_id));
        }
        _ => panic!("expected close repo command second"),
    }
}

#[test]
fn worker_command_prioritizes_open_repo_over_queued_background_result() {
    let repo_id = RepoId(7);
    let path = PathBuf::from("/tmp/repo");
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
        crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Ok(Vec::new()),
        },
    ))))
    .expect("send background result");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::OpenRepo(
        path.clone(),
    ))))
    .expect("send open");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CloseRepo {
        repo_id,
    })))
    .expect("send close");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("queued open");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::OpenRepo(got) if got == path));
        }
        _ => panic!("expected open repo command first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("queued close");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::CloseRepo { repo_id: got } if got == repo_id));
        }
        _ => panic!("expected close repo command second"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("background result");
    assert!(matches!(
        command,
        StoreWorkerCommand::Msg(msg)
            if matches!(
                *msg,
                Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
                    repo_id: got,
                    ..
                }) if got == repo_id
            )
    ));
}

#[test]
fn guarded_effect_sender_wraps_repository_load_messages() {
    let repo_id = RepoId(7);
    let (tx, rx) = std::sync::mpsc::channel();
    let sender = StoreWorkerSender::new(
        tx,
        Arc::new(std::sync::atomic::AtomicBool::new(true)),
        StoreInstanceId::next(),
    );
    let guarded = sender.with_repo_load_guard(repo_id, 3, CancellationToken::new());

    guarded.send_effect_or_log(
        Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
        "guarded effect sender test",
    );

    let command = rx.recv_timeout(Duration::from_secs(1)).expect("message");
    match command {
        StoreWorkerCommand::Msg(msg) => match *msg {
            Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
                repo_id: got_repo_id,
                load_epoch,
                message,
            }) => {
                assert_eq!(got_repo_id, repo_id);
                assert_eq!(load_epoch, 3);
                assert!(matches!(
                    *message,
                    crate::msg::InternalMsg::TagsLoaded {
                        repo_id: got_inner_repo_id,
                        ..
                    } if got_inner_repo_id == repo_id
                ));
            }
            other => panic!("expected guarded load message, got {other:?}"),
        },
        _ => panic!("expected worker message"),
    }
}

fn has_effect_for_repo(
    effects: &[Effect],
    repo_id: RepoId,
    matches_effect: impl Fn(&Effect, RepoId) -> bool,
) -> bool {
    effects.iter().any(|effect| matches_effect(effect, repo_id))
}

fn mark_repo_open_ready(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) {
    let workdir = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .expect("repo exists")
        .spec
        .workdir
        .to_string_lossy()
        .into_owned();
    repos.insert(repo_id, Arc::new(DummyRepo::new(&workdir)));

    let repo_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id)
        .expect("repo exists");
    repo_state.set_open(Loadable::Ready(()));
    repo_state.missing_on_disk = false;
}

fn open_repo_ready(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    path: impl Into<PathBuf>,
) -> RepoId {
    reduce(repos, id_alloc, state, Msg::OpenRepo(path.into()));
    let repo_id = state.active_repo.expect("open repo should become active");
    mark_repo_open_ready(repos, state, repo_id);
    repo_id
}

fn assert_open_repo_history_mode_resolution(
    seed_session: impl FnOnce(&Path, &Path),
    expected: LogScope,
) {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");
    let session_file = dir.path().join("session.json");
    std::fs::create_dir_all(&repo_path).expect("create repo path");
    let normalized_repo_path = super::reducer::normalize_repo_path(repo_path.clone());

    let _session_file_override =
        crate::session::push_test_session_file_path_override(Some(session_file.clone()));
    seed_session(&normalized_repo_path, &session_file);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_path.clone()),
    );

    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert_eq!(state.repos[0].history_state.history_scope, expected);

    let spec = state.repos[0].spec.clone();
    let workdir = spec.workdir.to_string_lossy().into_owned();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec,
            repo: Arc::new(DummyRepo::new(&workdir)),
        }),
    );

    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog {
                repo_id,
                scope,
                ..
            } if *repo_id == RepoId(1) && *scope == expected
        )),
        "expected RepoOpenedOk to request LoadLog({expected:?}), got {effects:?}"
    );
}

#[test]
fn open_repo_sets_opening_and_emits_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    assert_eq!(state.active_repo, Some(RepoId(1)));
    let repo_state = state.repos.first().expect("repo state to be set");
    assert_eq!(repo_state.id.0, 1);
    assert!(repo_state.open.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(1)))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PersistSession { .. }))
    );
}

#[test]
fn open_repo_focuses_existing_repo_instead_of_opening_duplicate() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(2)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );

    assert!(
        has_status_refresh_effects(&effects, RepoId(1)),
        "expected status refresh when focusing an already open repo"
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    let repo1 = super::reducer::normalize_repo_path(PathBuf::from("/tmp/repo1"));
    assert_eq!(
        state
            .repos
            .iter()
            .filter(|r| r.spec.workdir == repo1)
            .count(),
        1
    );
}

#[test]
fn open_repo_allows_same_basename_in_different_folders() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = std::env::temp_dir().join(format!(
        "gitcomet-open-repo-same-basename-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let repo_a = dir.join("a").join("repo");
    let repo_b = dir.join("b").join("repo");
    let _ = std::fs::create_dir_all(&repo_a);
    let _ = std::fs::create_dir_all(&repo_b);

    open_repo_ready(&mut repos, &id_alloc, &mut state, repo_a.clone());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_b.clone()),
    );
    mark_repo_open_ready(&mut repos, &mut state, RepoId(2));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(2)))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PersistSession { .. }))
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(2)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_a.clone()),
    );
    assert!(
        has_status_refresh_effects(&effects, RepoId(1)),
        "expected status refresh when re-focusing repo by path"
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert_eq!(
        state
            .repos
            .iter()
            .filter(|r| r.spec.workdir == super::reducer::normalize_repo_path(repo_a.clone()))
            .count(),
        1
    );
    assert_eq!(
        state
            .repos
            .iter()
            .filter(|r| r.spec.workdir == super::reducer::normalize_repo_path(repo_b.clone()))
            .count(),
        1
    );
}

#[test]
fn open_repo_refreshes_when_repo_is_already_active() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo");
    state.repos[0].missing_on_disk = true;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert!(
        has_status_refresh_effects(&effects, RepoId(1)),
        "expected status refresh when re-opening active repo"
    );
}

#[test]
fn open_repo_prefers_saved_history_mode_over_legacy_scope_and_default() {
    assert_open_repo_history_mode_resolution(
        |repo_path, session_file| {
            crate::session::persist_ui_settings_to_path(
                crate::session::UiSettings {
                    default_history_mode: Some(LogScope::MergesOnly),
                    ..Default::default()
                },
                session_file,
            )
            .expect("persist default history mode");
            crate::session::persist_repo_history_scope_to_path(
                repo_path,
                LogScope::AllBranches,
                session_file,
            )
            .expect("persist legacy history scope");
            crate::session::persist_repo_history_mode_to_path(
                repo_path,
                LogScope::NoMerges,
                session_file,
            )
            .expect("persist repo history mode");
        },
        LogScope::NoMerges,
    );
}

#[test]
fn open_repo_falls_back_to_legacy_history_scope_when_saved_mode_is_missing() {
    assert_open_repo_history_mode_resolution(
        |repo_path, session_file| {
            crate::session::persist_ui_settings_to_path(
                crate::session::UiSettings {
                    default_history_mode: Some(LogScope::MergesOnly),
                    ..Default::default()
                },
                session_file,
            )
            .expect("persist default history mode");
            crate::session::persist_repo_history_scope_to_path(
                repo_path,
                LogScope::CurrentBranch,
                session_file,
            )
            .expect("persist legacy history scope");
        },
        LogScope::FirstParent,
    );
}

#[test]
fn open_repo_falls_back_to_default_history_mode_when_repo_settings_are_missing() {
    assert_open_repo_history_mode_resolution(
        |_repo_path, session_file| {
            crate::session::persist_ui_settings_to_path(
                crate::session::UiSettings {
                    default_history_mode: Some(LogScope::AllBranches),
                    ..Default::default()
                },
                session_file,
            )
            .expect("persist default history mode");
        },
        LogScope::AllBranches,
    );
}

#[test]
fn open_repo_uses_builtin_default_history_mode_without_saved_preferences() {
    assert_open_repo_history_mode_resolution(|_, _| {}, LogScope::default());
}

#[test]
fn open_repo_persists_resolved_history_mode_and_keeps_it_sticky() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");
    let session_file = dir.path().join("session.json");
    std::fs::create_dir_all(&repo_path).expect("create repo path");
    let normalized_repo_path = super::reducer::normalize_repo_path(repo_path.clone());

    crate::session::persist_ui_settings_to_path(
        crate::session::UiSettings {
            default_history_mode: Some(LogScope::AllBranches),
            ..Default::default()
        },
        &session_file,
    )
    .expect("persist initial default history mode");

    let _session_file_override =
        crate::session::push_test_session_file_path_override(Some(session_file.clone()));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_path.clone()),
    );

    assert_eq!(
        state.repos[0].history_state.history_scope,
        LogScope::AllBranches
    );
    let persist_history = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistRepoHistoryMode { workdir, mode, .. } => Some((workdir, mode)),
            _ => None,
        })
        .expect("expected async history mode persist effect");
    assert_eq!(persist_history.0, &normalized_repo_path);
    assert_eq!(*persist_history.1, LogScope::AllBranches);
    crate::session::persist_repo_history_mode_to_path(
        persist_history.0,
        *persist_history.1,
        &session_file,
    )
    .expect("apply async history mode persist effect");
    assert_eq!(
        crate::session::load_repo_history_mode_from_path(&normalized_repo_path, &session_file),
        Some(LogScope::AllBranches)
    );

    crate::session::persist_ui_settings_to_path(
        crate::session::UiSettings {
            default_history_mode: Some(LogScope::NoMerges),
            ..Default::default()
        },
        &session_file,
    )
    .expect("persist updated default history mode");

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    reduce(&mut repos, &id_alloc, &mut state, Msg::OpenRepo(repo_path));

    assert_eq!(
        state.repos[0].history_state.history_scope,
        LogScope::AllBranches
    );
}

#[test]
fn clone_repo_sets_running_state_and_emits_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: PathBuf::from("/tmp/example"),
        },
    );

    let op = state.clone.as_ref().expect("clone op set");
    assert!(matches!(op.status, CloneOpStatus::Running));
    assert_eq!(op.progress.stage, CloneProgressStage::Loading);
    assert_eq!(op.progress.percent, 0);
    assert_eq!(op.seq, 0);
    assert!(matches!(effects.as_slice(), [Effect::CloneRepo { .. }]));
}

#[test]
fn clone_repo_progress_trims_tail_and_skips_blank_lines() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(dest.clone()),
            line: "   ".to_string(),
        }),
    );
    for i in 0..84 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
                dest: Arc::new(dest.clone()),
                line: format!("line-{i}"),
            }),
        );
    }

    let op = state.clone.as_ref().expect("clone op set");
    assert_eq!(op.seq, 85);
    assert_eq!(op.output_tail.len(), 80);
    assert_eq!(op.output_tail.front().map(String::as_str), Some("line-4"));
    assert_eq!(op.output_tail.back().map(String::as_str), Some("line-83"));
    assert_eq!(op.progress.stage, CloneProgressStage::Loading);
    assert_eq!(op.progress.percent, 0);
}

#[test]
fn clone_repo_progress_tracks_loading_and_remote_object_phases() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(dest.clone()),
            line: "Receiving objects:  42% (52/123), 1.23 MiB | 2.00 MiB/s".to_string(),
        }),
    );
    {
        let op = state.clone.as_ref().expect("clone op set");
        assert_eq!(op.progress.stage, CloneProgressStage::Loading);
        assert_eq!(op.progress.percent, 42);
    }

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(dest),
            line: "Resolving deltas:  17% (5/29)".to_string(),
        }),
    );

    let op = state.clone.as_ref().expect("clone op set");
    assert_eq!(op.progress.stage, CloneProgressStage::RemoteObjects);
    assert_eq!(op.progress.percent, 17);
}

#[test]
fn clone_repo_progress_ignores_mismatched_or_non_running_operation() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(PathBuf::from("/tmp/other")),
            line: "ignored".to_string(),
        }),
    );
    {
        let op = state.clone.as_ref().expect("clone op set");
        assert_eq!(op.seq, 0);
        assert!(op.output_tail.is_empty());
    }

    if let Some(op) = state.clone.as_mut() {
        op.status = CloneOpStatus::FinishedOk;
    }
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(dest.clone()),
            line: "ignored-too".to_string(),
        }),
    );
    {
        let op = state.clone.as_ref().expect("clone op set");
        assert_eq!(op.seq, 0);
        assert!(op.output_tail.is_empty());
    }

    state.clone = None;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(dest),
            line: "no-op".to_string(),
        }),
    );
    assert!(state.clone.is_none());
}

#[test]
fn abort_clone_repo_marks_operation_cancelling_and_emits_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AbortCloneRepo { dest: dest.clone() },
    );

    let op = state.clone.as_ref().expect("clone op set");
    assert!(matches!(op.status, CloneOpStatus::Cancelling));
    assert_eq!(op.seq, 1);
    assert!(
        matches!(effects.as_slice(), [Effect::AbortCloneRepo { dest: effect_dest }] if effect_dest == &dest)
    );
}

#[test]
fn clone_repo_finished_updates_existing_operation_for_success_and_error() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: "file:///tmp/success.git".to_string(),
            dest: dest.clone(),
            result: Ok(CommandOutput::empty_success("git clone")),
        }),
    );
    {
        let op = state.clone.as_ref().expect("clone op set");
        assert_eq!(&*op.url, "file:///tmp/success.git");
        assert_eq!(op.dest.as_ref(), &dest);
        assert!(matches!(op.status, CloneOpStatus::FinishedOk));
        assert_eq!(op.seq, 1);
    }

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: "file:///tmp/failure.git".to_string(),
            dest: PathBuf::from("/tmp/example"),
            result: Err(Error::new(ErrorKind::Backend("boom".to_string()))),
        }),
    );
    let op = state.clone.as_ref().expect("clone op set");
    assert_eq!(&*op.url, "file:///tmp/failure.git");
    assert_eq!(op.seq, 2);
    match &op.status {
        CloneOpStatus::FinishedErr(message) => {
            assert!(message.contains("Clone failed"));
            assert!(message.contains("boom"));
        }
        other => panic!("expected clone error status, got {other:?}"),
    }
}

#[test]
fn clone_repo_finished_maps_cancelling_error_to_cancelled() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AbortCloneRepo { dest: dest.clone() },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: "file:///tmp/example.git".to_string(),
            dest,
            result: Err(Error::new(ErrorKind::Backend("clone aborted".to_string()))),
        }),
    );

    let op = state.clone.as_ref().expect("clone op set");
    assert!(matches!(op.status, CloneOpStatus::Cancelled));
    assert_eq!(op.seq, 2);
}

#[test]
fn clone_repo_finished_preserves_cleanup_failure_when_cancelling() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AbortCloneRepo { dest: dest.clone() },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: "file:///tmp/example.git".to_string(),
            dest,
            result: Err(Error::new(ErrorKind::Backend(
                "clone aborted, but failed to remove partially created destination `/tmp/example`: permission denied"
                    .to_string(),
            ))),
        }),
    );

    let op = state.clone.as_ref().expect("clone op set");
    match &op.status {
        CloneOpStatus::FinishedErr(message) => {
            assert!(message.contains("Clone failed"));
            assert!(message.contains("failed to remove partially created destination"));
        }
        other => panic!("expected cleanup failure to remain visible, got {other:?}"),
    }
    assert_eq!(op.seq, 2);
}

#[test]
fn clone_repo_finished_replaces_state_when_destination_differs() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/original.git".to_string(),
            dest: PathBuf::from("/tmp/original"),
        },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: "file:///tmp/replacement.git".to_string(),
            dest: PathBuf::from("/tmp/replacement"),
            result: Ok(CommandOutput::empty_success("git clone")),
        }),
    );

    let op = state.clone.as_ref().expect("clone op set");
    assert_eq!(&*op.url, "file:///tmp/replacement.git");
    assert_eq!(op.dest.as_ref(), &PathBuf::from("/tmp/replacement"));
    assert!(matches!(op.status, CloneOpStatus::FinishedOk));
    assert_eq!(op.seq, 1);
    assert!(op.output_tail.is_empty());
}

#[test]
fn close_repo_removes_and_moves_active() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(10);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(11)));
    let old_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(11))
        .expect("repo 11 exists")
        .load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo {
            repo_id: RepoId(11),
        },
    );

    assert!(has_cancel_repo_loads_effect(
        &effects,
        RepoId(11),
        old_epoch
    ));
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(10))
    ));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistSession { .. }))
    );
    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.active_repo, Some(RepoId(10)));
}

fn recent_repo_effect_workdirs(effects: &[Effect]) -> Vec<PathBuf> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::PersistRecentRepo { workdir, .. } => Some(workdir.clone()),
            _ => None,
        })
        .collect()
}

/// Recording the close here rather than at the affordance that asked for it is
/// what keeps the Recently Closed order the same whichever way a repository was
/// closed — the repo tab's `x`, its menu, or the picker's row menu.
#[test]
fn close_repo_records_the_closed_repository_as_recent() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for name in ["repo1", "repo2"] {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/{name}"))),
        );
    }
    let closed_workdir = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(2))
        .expect("repo 2 exists")
        .spec
        .workdir
        .clone();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id: RepoId(2) },
    );

    assert_eq!(recent_repo_effect_workdirs(&effects), vec![closed_workdir]);

    // Closing something that is not open leaves the recents alone: there is no
    // workdir to name, and re-running a close must not reorder the list.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id: RepoId(2) },
    );
    assert!(recent_repo_effect_workdirs(&effects).is_empty());
}

/// Bulk closes walk the tab strip left to right rather than the `HashSet` of
/// ids, so the Recently Closed order they leave behind is the same on every run.
#[test]
fn close_repos_records_recents_in_tab_order() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for ix in 1..=3 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/repo{ix}"))),
        );
    }
    let workdir_of = |state: &AppState, repo_id: RepoId| {
        state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .expect("repo exists")
            .spec
            .workdir
            .clone()
    };
    let first = workdir_of(&state, RepoId(1));
    let third = workdir_of(&state, RepoId(3));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(3), RepoId(999), RepoId(1)],
            activate_after: None,
        },
    );

    assert_eq!(recent_repo_effect_workdirs(&effects), vec![first, third]);
}

#[test]
fn close_repo_selects_right_neighbor_when_closing_first_active_tab() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(20);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo {
            repo_id: RepoId(20),
        },
    );
    let old_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(20))
        .expect("repo 20 exists")
        .load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo {
            repo_id: RepoId(20),
        },
    );

    assert!(has_cancel_repo_loads_effect(
        &effects,
        RepoId(20),
        old_epoch
    ));
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(21))
    ));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistSession { .. }))
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(21)));
}

#[test]
fn close_repos_ignores_unknown_ids_and_persists_once() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );
    let old_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(1))
        .expect("repo 1 exists")
        .load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(999), RepoId(1), RepoId(1)],
            activate_after: None,
        },
    );

    assert!(has_cancel_repo_loads_effect(&effects, RepoId(1), old_epoch));
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::PersistSession { .. }))
            .count(),
        1
    );
    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        vec![RepoId(2), RepoId(3)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));
}

#[test]
fn close_repos_selects_left_neighbor_when_active_repo_is_closed() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for ix in 1..=3 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/repo{ix}"))),
        );
    }
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: RepoId(2) },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(2)],
            activate_after: None,
        },
    );

    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        vec![RepoId(1), RepoId(3)]
    );
    assert_eq!(state.active_repo, Some(RepoId(1)));
}

#[test]
fn close_repos_uses_requested_active_repo_after_batch_close() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for ix in 1..=3 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/repo{ix}"))),
        );
    }
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: RepoId(1) },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(1), RepoId(3)],
            activate_after: Some(RepoId(2)),
        },
    );

    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        vec![RepoId(2)]
    );
    assert_eq!(state.active_repo, Some(RepoId(2)));
}

#[test]
fn close_repos_noops_when_no_existing_repos_match() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for ix in 1..=2 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/repo{ix}"))),
        );
    }
    let original_repo_ids = state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>();
    let original_active = state.active_repo;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(999)],
            activate_after: Some(RepoId(1)),
        },
    );

    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        original_repo_ids
    );
    assert_eq!(state.active_repo, original_active);
}

#[test]
fn close_repos_closing_all_repos_clears_active_and_persists_once() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for ix in 1..=2 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/repo{ix}"))),
        );
    }

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(1), RepoId(2)],
            activate_after: Some(RepoId(1)),
        },
    );

    assert!(state.repos.is_empty());
    assert_eq!(state.active_repo, None);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::PersistSession { .. }))
            .count(),
        1
    );
}

#[test]
fn reorder_repo_tabs_moves_repo_and_keeps_active() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );

    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![RepoId(1), RepoId(2), RepoId(3)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(3),
            insert_before: Some(RepoId(1)),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistSession { .. }]
    ));
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![RepoId(3), RepoId(1), RepoId(2)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(3),
            insert_before: None,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistSession { .. }]
    ));
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![RepoId(1), RepoId(2), RepoId(3)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));
}

#[test]
fn reorder_repo_tabs_noops_for_invalid_or_already_stable_ordering() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let original = state.repos.iter().map(|r| r.id).collect::<Vec<_>>();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(1),
            insert_before: None,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );
    let original = state.repos.iter().map(|r| r.id).collect::<Vec<_>>();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(999),
            insert_before: Some(RepoId(1)),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(2),
            insert_before: Some(RepoId(2)),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(1),
            insert_before: Some(RepoId(2)),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(3),
            insert_before: None,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );
}

#[test]
fn remote_branches_loaded_sets_state() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded {
            repo_id: RepoId(1),
            result: Ok(vec![RemoteBranch {
                remote: "origin".to_string(),
                name: "main".to_string(),
                target: CommitId("deadbeef".into()),
            }]),
        }),
    );

    let repo = state.repos.iter().find(|r| r.id == RepoId(1)).unwrap();
    match &repo.remote_branches {
        Loadable::Ready(branches) => {
            assert_eq!(branches.len(), 1);
            assert_eq!(branches[0].remote, "origin");
            assert_eq!(branches[0].name, "main");
        }
        other => panic!("expected Ready remote_branches, got {other:?}"),
    }
}

#[test]
fn restore_session_opens_only_active_repo_and_selects_active_repo() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = std::env::temp_dir().join(format!(
        "gitcomet-restore-session-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);

    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");
    let _ = std::fs::create_dir_all(&repo_a);
    let _ = std::fs::create_dir_all(&repo_b);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a.clone(), repo_b],
            active_repo: Some(repo_a.clone()),
        },
    );

    assert_eq!(state.repos.len(), 2);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::OpenRepo { .. }))
            .count(),
        1
    );
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::PersistSession { .. }))
            .count(),
        1
    );

    let active_repo_id = state.active_repo.expect("active repo is set");
    let active_workdir = state
        .repos
        .iter()
        .find(|r| r.id == active_repo_id)
        .expect("active repo exists")
        .spec
        .workdir
        .clone();

    assert_eq!(active_workdir, super::reducer::normalize_repo_path(repo_a));
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == active_repo_id)
            .expect("active repo exists")
            .open,
        Loadable::Loading
    ));
    assert!(
        state
            .repos
            .iter()
            .filter(|repo| repo.id != active_repo_id)
            .all(|repo| matches!(repo.open, Loadable::NotLoaded))
    );
}

#[test]
fn selecting_inactive_restored_repo_cancels_previous_load_and_starts_open() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("create repo-a");
    std::fs::create_dir_all(&repo_b).expect("create repo-b");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a, repo_b],
            active_repo: None,
        },
    );

    let previous_active = state.active_repo.expect("active repo exists");
    let previous_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == previous_active)
        .expect("previous active repo exists")
        .load_epoch;
    let inactive_repo = state
        .repos
        .iter()
        .find(|repo| repo.id != previous_active)
        .expect("inactive repo exists")
        .id;
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == inactive_repo)
            .expect("inactive repo exists")
            .open,
        Loadable::NotLoaded
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo {
            repo_id: inactive_repo,
        },
    );

    assert_eq!(state.active_repo, Some(inactive_repo));
    assert!(has_cancel_repo_loads_effect(
        &effects,
        previous_active,
        previous_epoch
    ));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == inactive_repo
    )));
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == inactive_repo)
            .expect("inactive repo exists")
            .open,
        Loadable::Loading
    ));
}

#[test]
fn selecting_third_restored_repo_while_second_is_opening_cancels_second_open() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    let repo_c = dir.path().join("repo-c");
    std::fs::create_dir_all(&repo_a).expect("create repo-a");
    std::fs::create_dir_all(&repo_b).expect("create repo-b");
    std::fs::create_dir_all(&repo_c).expect("create repo-c");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a.clone(), repo_b, repo_c],
            active_repo: Some(repo_a),
        },
    );

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    let repo3 = RepoId(3);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );
    assert_eq!(state.active_repo, Some(repo2));
    assert!(has_cancel_repo_loads_effect(&effects, repo1, 0));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == repo2
    )));

    let repo2_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == repo2)
        .expect("repo2 exists")
        .load_epoch;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo3 },
    );

    assert_eq!(state.active_repo, Some(repo3));
    assert!(has_cancel_repo_loads_effect(&effects, repo2, repo2_epoch));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == repo3
    )));
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == repo2)
            .expect("repo2 exists")
            .open,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == repo3)
            .expect("repo3 exists")
            .open,
        Loadable::Loading
    ));
}

#[test]
fn restore_session_resolves_history_mode_precedence_per_repository() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let session_file = dir.path().join("session.json");
    let repo_mode = dir.path().join("repo-mode");
    let repo_legacy = dir.path().join("repo-legacy");
    let repo_default = dir.path().join("repo-default");
    std::fs::create_dir_all(&repo_mode).expect("create repo-mode");
    std::fs::create_dir_all(&repo_legacy).expect("create repo-legacy");
    std::fs::create_dir_all(&repo_default).expect("create repo-default");
    let normalized_repo_mode = super::reducer::normalize_repo_path(repo_mode.clone());
    let normalized_repo_legacy = super::reducer::normalize_repo_path(repo_legacy.clone());
    let normalized_repo_default = super::reducer::normalize_repo_path(repo_default.clone());

    crate::session::persist_ui_settings_to_path(
        crate::session::UiSettings {
            default_history_mode: Some(LogScope::MergesOnly),
            ..Default::default()
        },
        &session_file,
    )
    .expect("persist default history mode");
    crate::session::persist_repo_history_mode_to_path(
        &normalized_repo_mode,
        LogScope::NoMerges,
        &session_file,
    )
    .expect("persist repo mode");
    crate::session::persist_repo_history_scope_to_path(
        &normalized_repo_legacy,
        LogScope::CurrentBranch,
        &session_file,
    )
    .expect("persist legacy scope");

    let _session_file_override =
        crate::session::push_test_session_file_path_override(Some(session_file.clone()));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_mode.clone(), repo_legacy.clone(), repo_default.clone()],
            active_repo: Some(repo_default.clone()),
        },
    );

    let by_workdir = state
        .repos
        .iter()
        .map(|repo| (repo.spec.workdir.clone(), repo.history_state.history_scope))
        .collect::<HashMap<_, _>>();

    assert_eq!(
        by_workdir.get(&normalized_repo_mode),
        Some(&LogScope::NoMerges)
    );
    assert_eq!(
        by_workdir.get(&normalized_repo_legacy),
        Some(&LogScope::FirstParent)
    );
    assert_eq!(
        by_workdir.get(&normalized_repo_default),
        Some(&LogScope::MergesOnly)
    );
    assert_eq!(
        state.active_repo.and_then(|repo_id| state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| repo.spec.workdir.clone())),
        Some(normalized_repo_default.clone())
    );
    let updates = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistRepoHistoryModesBatch { updates, .. } => Some(updates),
            _ => None,
        })
        .expect("expected async history mode batch persist effect");
    assert!(updates.contains(&(normalized_repo_legacy.clone(), LogScope::FirstParent)));
    assert!(updates.contains(&(normalized_repo_default.clone(), LogScope::MergesOnly)));
    crate::session::persist_repo_history_modes_batch_to_path(updates, &session_file)
        .expect("apply async history mode batch persist effect");
    assert_eq!(
        crate::session::load_repo_history_mode_from_path(&normalized_repo_mode, &session_file),
        Some(LogScope::NoMerges)
    );
    assert_eq!(
        crate::session::load_repo_history_mode_from_path(&normalized_repo_legacy, &session_file),
        Some(LogScope::FirstParent)
    );
    assert_eq!(
        crate::session::load_repo_history_mode_from_path(&normalized_repo_default, &session_file),
        Some(LogScope::MergesOnly)
    );
}

#[test]
fn set_active_repo_waits_for_repo_open_before_refreshing() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1 = RepoId(1);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert_eq!(state.active_repo, Some(repo1));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == repo1
    )));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistSession { .. }))
    );
    assert!(
        !effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadWorktreeStatus { .. }
                | Effect::LoadStagedStatus { .. }
                | Effect::LoadBranches { .. }
                | Effect::LoadWorktrees { .. }
                | Effect::LoadSelectedDiff { .. }
        )),
        "expected no handle-dependent refreshes before RepoOpenedOk"
    );
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == repo1)
            .expect("repo1 exists")
            .worktrees,
        Loadable::NotLoaded
    ));
}

#[test]
fn switching_away_from_opening_repo_cancels_loading_and_restarts_on_return() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo1 = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    let repo2 = RepoId(2);
    assert_eq!(state.active_repo, Some(repo2));
    assert!(state.repos[1].open.is_loading());

    let old_epoch = state.repos[1].load_epoch;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    let repo2_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo2)
        .expect("repo2 exists");
    assert_eq!(repo2_state.load_epoch, old_epoch.wrapping_add(1));
    assert!(matches!(repo2_state.open, Loadable::NotLoaded));
    assert!(has_cancel_repo_loads_effect(&effects, repo2, old_epoch));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );

    let repo2_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo2)
        .expect("repo2 exists");
    assert!(repo2_state.open.is_loading());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == repo2
    )));
}

#[test]
fn opening_another_repo_cancels_previous_active_repo_loads() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo1 = RepoId(1);
    let old_epoch = state.repos[0].load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(matches!(repo1_state.open, Loadable::NotLoaded));
    assert_eq!(repo1_state.load_epoch, old_epoch.wrapping_add(1));
    assert!(has_cancel_repo_loads_effect(&effects, repo1, old_epoch));
    assert_eq!(state.active_repo, Some(RepoId(2)));
}

#[test]
fn closing_active_repo_refreshes_open_neighbor_with_cancelled_loads() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo1 = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    {
        let repo1_state = state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo1)
            .expect("repo1 exists");
        repo1_state.set_branches(Loadable::Loading);
        repo1_state.set_status(Loadable::Loading);
        repo1_state.set_log(Loadable::Loading);
        assert!(
            repo1_state
                .loads_in_flight
                .request(RepoLoadsInFlight::BRANCHES)
        );
        assert!(
            repo1_state
                .loads_in_flight
                .request(RepoLoadsInFlight::WORKTREE_STATUS)
        );
        assert!(
            repo1_state
                .loads_in_flight
                .request(RepoLoadsInFlight::STAGED_STATUS)
        );
        let log_request = crate::model::PendingLogLoad {
            scope: repo1_state.history_state.history_scope,
            author: None,
            limit: 50,
            cursor: None,
        };
        assert!(
            repo1_state
                .loads_in_flight
                .request_log(log_request)
                .is_some()
        );
    }

    let repo2 = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");
    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(matches!(repo1_state.open, Loadable::Ready(())));
    assert!(matches!(repo1_state.branches, Loadable::NotLoaded));
    assert!(matches!(repo1_state.status, Loadable::NotLoaded));
    assert!(matches!(repo1_state.log, Loadable::NotLoaded));
    assert!(!repo1_state.loads_in_flight.any_in_flight());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id: repo2 },
    );

    assert_eq!(state.active_repo, Some(repo1));
    assert!(
        has_status_refresh_effects(&effects, repo1),
        "expected status refresh when close selects already-open repo"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadLog { repo_id, .. } if *repo_id == repo1)),
        "expected log refresh when close selects already-open repo"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadBranches { repo_id } if *repo_id == repo1)),
        "expected branch refresh when close selects already-open repo"
    );
}

#[test]
fn stale_open_result_after_cancel_is_ignored() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo1 = RepoId(1);
    let old_epoch = state.repos[0].load_epoch;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
            repo_id: repo1,
            load_epoch: old_epoch,
            message: Box::new(crate::msg::InternalMsg::RepoOpenedOk {
                repo_id: repo1,
                spec: RepoSpec {
                    workdir: PathBuf::from("/tmp/repo1"),
                },
                repo: Arc::new(DummyRepo::new("/tmp/repo1")),
            }),
        }),
    );

    assert!(effects.is_empty());
    assert!(!repos.contains_key(&repo1));
    assert!(matches!(state.repos[0].open, Loadable::NotLoaded));
}

#[test]
fn stale_load_result_after_cancel_is_ignored() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo1 = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    repo1_state.set_status(Loadable::Loading);
    assert!(
        repo1_state
            .loads_in_flight
            .request(RepoLoadsInFlight::WORKTREE_STATUS)
    );
    let old_epoch = repo1_state.load_epoch;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
            repo_id: repo1,
            load_epoch: old_epoch,
            message: Box::new(crate::msg::InternalMsg::StatusLoaded {
                repo_id: repo1,
                result: Ok(RepoStatus::default()),
            }),
        }),
    );

    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(effects.is_empty());
    assert!(matches!(repo1_state.status, Loadable::NotLoaded));
    assert!(!repo1_state.loads_in_flight.any_in_flight());
}

#[test]
fn inactive_open_result_does_not_schedule_refresh_or_tags() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    let inactive_repo = RepoId(1);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: inactive_repo,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo1"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo1")),
        }),
    );

    assert!(effects.is_empty());
    assert!(repos.contains_key(&inactive_repo));
    let repo_state = state
        .repos
        .iter()
        .find(|repo| repo.id == inactive_repo)
        .expect("inactive repo exists");
    assert!(matches!(repo_state.open, Loadable::Ready(())));
    assert!(matches!(repo_state.tags, Loadable::NotLoaded));
    assert!(matches!(repo_state.remote_tags, Loadable::NotLoaded));
    assert!(!repo_state.loads_in_flight.any_in_flight());
}

#[test]
fn closing_loading_active_repo_cancels_and_opens_neighbor() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("create repo-a");
    std::fs::create_dir_all(&repo_b).expect("create repo-b");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a, repo_b],
            active_repo: None,
        },
    );

    let active_repo = state.active_repo.expect("active repo exists");
    let neighbor_repo = state
        .repos
        .iter()
        .find(|repo| repo.id != active_repo)
        .expect("neighbor repo exists")
        .id;
    let old_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == active_repo)
        .expect("active repo exists")
        .load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo {
            repo_id: active_repo,
        },
    );

    assert!(state.repos.iter().all(|repo| repo.id != active_repo));
    assert_eq!(state.active_repo, Some(neighbor_repo));
    assert!(has_cancel_repo_loads_effect(
        &effects,
        active_repo,
        old_epoch
    ));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == neighbor_repo
    )));
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == neighbor_repo)
            .expect("neighbor exists")
            .open,
        Loadable::Loading
    ));
}

#[test]
fn closing_loading_inactive_repo_cancels_without_changing_active_repo() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("create repo-a");
    std::fs::create_dir_all(&repo_b).expect("create repo-b");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a, repo_b],
            active_repo: None,
        },
    );

    let active_repo = state.active_repo.expect("active repo exists");
    let inactive_repo = state
        .repos
        .iter()
        .find(|repo| repo.id != active_repo)
        .expect("inactive repo exists")
        .id;
    let inactive_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == inactive_repo)
        .expect("inactive repo exists");
    inactive_state.set_open(Loadable::Loading);
    let old_epoch = inactive_state.load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo {
            repo_id: inactive_repo,
        },
    );

    assert_eq!(state.active_repo, Some(active_repo));
    assert!(state.repos.iter().all(|repo| repo.id != inactive_repo));
    assert!(has_cancel_repo_loads_effect(
        &effects,
        inactive_repo,
        old_epoch
    ));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == active_repo
    )));
}

#[test]
fn pre_open_worktree_lazy_load_retries_after_repo_opened() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let repo_id = RepoId(1);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadWorktrees { repo_id },
    );
    assert!(effects.is_empty());
    assert!(matches!(state.repos[0].worktrees, Loadable::NotLoaded));
    assert!(
        !state.repos[0]
            .loads_in_flight
            .is_in_flight(RepoLoadsInFlight::WORKTREES)
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    assert!(state.repos[0].worktrees.is_loading());
    assert!(
        effects.iter().any(
            |effect| matches!(effect, Effect::LoadWorktrees { repo_id: rid } if *rid == repo_id)
        )
    );
}

#[test]
fn load_ref_metadata_emits_effect_and_result_builds_the_lookup_map() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadRefMetadata { repo_id },
    );
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::LoadRefMetadata { repo_id: rid } if *rid == repo_id)
    ));
    assert!(state.repos[0].ref_metadata.is_loading());

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
            repo_id,
            result: Ok(vec![(
                "main".to_string(),
                gitcomet_core::domain::RefMetadata {
                    author: "Ada".to_string(),
                    committed_at: 1_754_870_400,
                    summary: "first".to_string(),
                },
            )]),
        }),
    );

    let Loadable::Ready(map) = &state.repos[0].ref_metadata else {
        panic!("expected ref metadata to be ready");
    };
    assert_eq!(map.get("main").map(|m| m.summary.as_str()), Some("first"));
}

#[test]
fn ref_metadata_load_failure_records_no_diagnostic() {
    // Decorative data: a backend that cannot supply it must not raise an error
    // banner every time a picker opens.
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);
    let diagnostics_before = state.repos[0].diagnostics.len();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
            repo_id,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("git blew up".to_string()),
            )),
        }),
    );

    assert!(matches!(state.repos[0].ref_metadata, Loadable::Error(_)));
    assert_eq!(state.repos[0].diagnostics.len(), diagnostics_before);
}

#[test]
fn unsupported_ref_metadata_latches_instead_of_retrying_forever() {
    // Callers refetch on `Error`, so storing `Error` for a backend that can
    // never supply this would re-schedule a doomed load on every picker open.
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
            repo_id,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Unsupported("nope"),
            )),
        }),
    );

    let Loadable::Ready(map) = &state.repos[0].ref_metadata else {
        panic!("expected Unsupported to latch as an empty Ready map");
    };
    assert!(map.is_empty());
}

#[test]
fn transient_ref_metadata_failure_stays_retryable() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
            repo_id,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("git blew up".to_string()),
            )),
        }),
    );

    assert!(
        matches!(state.repos[0].ref_metadata, Loadable::Error(_)),
        "a transient failure must stay retryable"
    );
}

#[test]
fn branch_change_during_an_in_flight_metadata_load_schedules_a_refetch() {
    // Otherwise the in-flight result (read from the *old* refs) lands as
    // `Ready` and, since callers only refetch on NotLoaded/Error, is never
    // corrected.
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    // Start a metadata load, then let the branch list change underneath it.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadRefMetadata { repo_id },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![]),
        }),
    );

    // The stale result arrives; it must trigger another load rather than stick.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
            repo_id,
            result: Ok(vec![]),
        }),
    );

    assert!(
        effects.iter().any(
            |effect| matches!(effect, Effect::LoadRefMetadata { repo_id: rid } if *rid == repo_id)
        ),
        "expected a refetch to be scheduled, got {effects:?}"
    );
}

#[test]
fn pre_open_submodule_load_auto_starts_after_repo_opened() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let repo_id = RepoId(1);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadSubmodules { repo_id },
    );
    assert!(effects.is_empty());
    assert!(matches!(state.repos[0].submodules, Loadable::NotLoaded));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::LoadSubmodules { repo_id: rid } if *rid == repo_id)
    ));
    assert!(state.repos[0].submodules.is_loading());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadSubmodules { repo_id },
    );
    assert!(effects.is_empty());
    assert!(state.repos[0].submodules.is_loading());
}

#[test]
fn pre_open_stash_lazy_load_can_retry_after_repo_opened() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let repo_id = RepoId(1);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadStashes { repo_id },
    );
    assert!(effects.is_empty());
    assert!(matches!(state.repos[0].stashes, Loadable::NotLoaded));
    assert!(
        !state.repos[0]
            .loads_in_flight
            .is_in_flight(RepoLoadsInFlight::STASHES)
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadStashes {
            repo_id: rid,
            limit: 50
        } if *rid == repo_id
    )));
    assert!(matches!(state.repos[0].stashes, Loadable::NotLoaded));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadStashes { repo_id },
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadStashes {
            repo_id: rid,
            limit: 50
        } if *rid == repo_id
    )));
    assert!(state.repos[0].stashes.is_loading());
}

#[test]
fn ensure_sidebar_data_retries_requested_sections_after_repo_opened() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let repo_id = RepoId(1);
    let request = SidebarDataRequest {
        worktrees: true,
        submodules: true,
        stashes: true,
    };
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::EnsureSidebarData { repo_id, request },
    );
    assert!(effects.is_empty());
    assert_eq!(state.repos[0].sidebar_data_request, request);
    assert!(matches!(state.repos[0].worktrees, Loadable::NotLoaded));
    assert!(matches!(state.repos[0].submodules, Loadable::NotLoaded));
    assert!(matches!(state.repos[0].stashes, Loadable::NotLoaded));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    assert!(has_worktree_refresh_effect(&effects, repo_id));
    assert!(has_submodule_load_effect(&effects, repo_id));
    assert!(has_stash_load_effect(&effects, repo_id));
    assert!(state.repos[0].worktrees.is_loading());
    assert!(state.repos[0].submodules.is_loading());
    assert!(state.repos[0].stashes.is_loading());
}

#[test]
fn set_active_repo_replays_stored_sidebar_data_request() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    assert_eq!(state.active_repo, Some(repo2));

    let request = SidebarDataRequest {
        worktrees: true,
        submodules: true,
        stashes: true,
    };
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    repo1_state.set_sidebar_data_request(request);
    repo1_state.set_worktrees(Loadable::NotLoaded);
    repo1_state.set_submodules(Loadable::NotLoaded);
    repo1_state.set_stashes(Loadable::NotLoaded);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert_eq!(state.active_repo, Some(repo1));
    assert!(has_worktree_refresh_effect(&effects, repo1));
    assert!(has_submodule_load_effect(&effects, repo1));
    assert!(has_stash_load_effect(&effects, repo1));
    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(repo1_state.worktrees.is_loading());
    assert!(repo1_state.submodules.is_loading());
    assert!(repo1_state.stashes.is_loading());
}

#[test]
fn set_active_repo_full_refresh_with_sidebar_request_and_selected_diff_does_not_panic() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    assert_eq!(state.active_repo, Some(repo2));

    let request = SidebarDataRequest {
        worktrees: true,
        submodules: true,
        stashes: true,
    };
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    repo1_state.set_sidebar_data_request(request);
    repo1_state.set_worktrees(Loadable::NotLoaded);
    repo1_state.set_submodules(Loadable::NotLoaded);
    repo1_state.set_stashes(Loadable::NotLoaded);
    repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert_eq!(state.active_repo, Some(repo1));
    assert!(
        has_full_refresh_only_effects(&effects, repo1),
        "expected cold repo switch to use full refresh"
    );
    assert!(has_worktree_refresh_effect(&effects, repo1));
    assert!(has_submodule_load_effect(&effects, repo1));
    assert!(has_stash_load_effect(&effects, repo1));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSelectedDiff {
            repo_id,
            load_patch_diff: true,
            load_file_text: true,
            load_file_image: false,
            load_submodule_summary: false,
            preview_text_side: None,
        } if *repo_id == repo1
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::PersistSession { repo_id, .. } if *repo_id == Some(repo1)
    )));

    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(repo1_state.worktrees.is_loading());
    assert!(repo1_state.submodules.is_loading());
    assert!(repo1_state.stashes.is_loading());
}

#[test]
fn set_active_repo_refreshes_repo_state_and_selected_diff() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    assert_eq!(state.active_repo, Some(repo2));

    let repo1_state = state
        .repos
        .iter_mut()
        .find(|r| r.id == repo1)
        .expect("repo1 exists");
    repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert_eq!(state.active_repo, Some(repo1));

    let has_status = has_status_refresh_effects(&effects, repo1);
    let has_log = effects
        .iter()
        .any(|e| matches!(e, Effect::LoadLog { repo_id, .. } if *repo_id == repo1));
    let has_selected_diff_reload = effects.iter().any(|e| {
        matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id,
                load_patch_diff: true,
                load_file_text: true,
                load_file_image: false,
                load_submodule_summary: false,
                preview_text_side: None,
            } if *repo_id == repo1
        )
    });
    let has_persist = effects
        .iter()
        .any(|e| matches!(e, Effect::PersistSession { .. }));

    assert!(has_status, "expected status refresh on activation");
    assert!(has_log, "expected log refresh on activation");
    assert!(
        has_selected_diff_reload,
        "expected combined selected-diff reload on activation"
    );
    assert!(
        matches!(
            state
                .repos
                .iter()
                .find(|repo| repo.id == repo1)
                .and_then(|repo| repo.diff_state.diff_target.as_ref()),
            Some(DiffTarget::WorkingTree { path, .. }) if path == &PathBuf::from("src/lib.rs")
        ),
        "expected the selected diff target to remain available on repo state for scheduling"
    );
    assert!(
        has_persist,
        "expected session persist when active repo changes"
    );
}

#[test]
fn set_active_repo_reloads_cancelled_history_panes_for_existing_selection() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    let history_path = PathBuf::from("src/lib.rs");
    let blame_path = PathBuf::from("src/main.rs");
    let selected_commit = CommitId("deadbeef".into());

    mark_repo_switch_secondary_metadata_ready(
        state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo1)
            .expect("repo1 exists"),
    );
    mark_repo_switch_secondary_metadata_ready(
        state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo2)
            .expect("repo2 exists"),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    {
        let repo1_state = state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo1)
            .expect("repo1 exists");
        repo1_state.history_state.file_history_path = Some(history_path.clone());
        repo1_state.history_state.file_history = Loadable::Loading;
        repo1_state.history_state.blame_path = Some(blame_path.clone());
        repo1_state.history_state.blame_source = Some(
            gitcomet_core::domain::BlameSource::Revision(Some("HEAD~1".to_string())),
        );
        repo1_state.history_state.blame = Loadable::Loading;
        repo1_state.set_selected_commit(Some(selected_commit.clone()));
        repo1_state.set_commit_details(Loadable::Loading);
    }
    let repo1_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists")
        .load_epoch;

    let deactivate_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );
    assert!(
        has_cancel_repo_loads_effect(&deactivate_effects, repo1, repo1_epoch),
        "expected repo switch to cancel in-flight repo1 loads"
    );
    {
        let repo1_state = state
            .repos
            .iter()
            .find(|repo| repo.id == repo1)
            .expect("repo1 exists");
        assert!(matches!(
            repo1_state.history_state.file_history,
            Loadable::NotLoaded
        ));
        assert!(matches!(
            repo1_state.history_state.blame,
            Loadable::NotLoaded
        ));
        assert!(matches!(
            repo1_state.history_state.commit_details,
            Loadable::NotLoaded
        ));
        assert_eq!(
            repo1_state.history_state.file_history_path.as_ref(),
            Some(&history_path)
        );
        assert_eq!(
            repo1_state.history_state.blame_path.as_ref(),
            Some(&blame_path)
        );
        assert_eq!(
            repo1_state.history_state.selected_commit.as_ref(),
            Some(&selected_commit)
        );
    }

    let reactivate_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(reactivate_effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadFileHistory {
            repo_id,
            path,
            limit: 200,
        } if *repo_id == repo1 && path == &history_path
    )));
    assert!(reactivate_effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadBlame { repo_id, path, source: gitcomet_core::domain::BlameSource::Revision(Some(rev)) }
            if *repo_id == repo1
                && path == &blame_path
                && rev == "HEAD~1"
    )));
    assert!(reactivate_effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadCommitDetails { repo_id, commit_id }
            if *repo_id == repo1 && commit_id == &selected_commit
    )));

    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(repo1_state.history_state.file_history.is_loading());
    assert!(repo1_state.history_state.blame.is_loading());
    assert!(repo1_state.history_state.commit_details.is_loading());
}

#[test]
fn set_active_repo_reloads_selected_image_diff_via_image_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|r| r.id == repo1)
        .expect("repo1 exists");
    repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("icon.png"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadSelectedDiff {
            repo_id,
            load_patch_diff: true,
            load_file_text: false,
            load_file_image: true,
            load_submodule_summary: false,
            preview_text_side: None,
        } if *repo_id == repo1
    )));
}

#[test]
fn set_active_repo_png_diff_enqueues_image_preview_only() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|r| r.id == repo1)
        .expect("repo1 exists");
    repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("image.png"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id,
                load_patch_diff: true,
                load_file_text: false,
                load_file_image: true,
                ..
            } if *repo_id == repo1
        )),
        "expected combined selected-diff reload with image preview only for png target"
    );
}

#[test]
fn set_active_repo_svg_diff_enqueues_image_and_text_previews() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|r| r.id == repo1)
        .expect("repo1 exists");
    repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("vector.svg"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id,
                load_patch_diff: true,
                load_file_text: true,
                load_file_image: true,
                ..
            } if *repo_id == repo1
        )),
        "expected combined selected-diff reload with both image and text previews for svg target"
    );
}

#[test]
fn set_active_repo_selected_conflict_target_reuses_existing_conflict_state() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let conflict_path = PathBuf::from("src/conflict.rs");
    let before_rev = {
        let repo1_state = state
            .repos
            .iter_mut()
            .find(|r| r.id == repo1)
            .expect("repo1 exists");
        repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
            path: conflict_path.clone(),
            area: gitcomet_core::domain::DiffArea::Unstaged,
        });
        repo1_state.conflict_state.conflict_file_path = Some(conflict_path.clone());
        let content: Arc<str> = Arc::from("conflict contents");
        repo1_state.conflict_state.conflict_file =
            Loadable::Ready(Some(crate::model::ConflictFile {
                path: conflict_path.clone().into(),
                base_bytes: None,
                ours_bytes: None,
                theirs_bytes: None,
                current_bytes: None,
                base: Some(Arc::clone(&content)),
                ours: Some(Arc::clone(&content)),
                theirs: Some(Arc::clone(&content)),
                current: Some(content),
            }));
        repo1_state.conflict_state.conflict_rev = 41;
        repo1_state.conflict_state.conflict_rev
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    let repo1_state = state
        .repos
        .iter()
        .find(|r| r.id == repo1)
        .expect("repo1 exists");
    assert_eq!(
        repo1_state.conflict_state.conflict_file_path.as_ref(),
        Some(&conflict_path)
    );
    assert!(repo1_state.conflict_state.conflict_file.is_loading());
    assert!(repo1_state.conflict_state.conflict_session.is_none());
    assert_eq!(repo1_state.conflict_state.conflict_rev, before_rev + 1);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSelectedConflictFile {
            repo_id,
            mode: crate::model::ConflictFileLoadMode::CurrentOnly,
        } if *repo_id == repo1
    )));
}

#[test]
fn set_active_repo_hot_switch_skips_secondary_refresh_when_metadata_is_ready() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    mark_repo_switch_secondary_metadata_ready(repo1_state);
    repo1_state.last_active_at = Some(SystemTime::now());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        !has_full_refresh_only_effects(&effects, repo1),
        "hot repo switches with ready metadata should stay on the primary refresh path"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadBranches { repo_id } if *repo_id == repo1)),
        "expected local branches refresh on activation"
    );
    assert!(
        has_worktree_refresh_effect(&effects, repo1),
        "expected worktrees refresh on activation"
    );
    assert!(has_status_refresh_effects(&effects, repo1));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadLog { repo_id, .. } if *repo_id == repo1))
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadRebaseAndMergeState { repo_id } if *repo_id == repo1
    )));
}

#[test]
fn set_active_repo_uses_full_refresh_when_hot_switch_metadata_is_incomplete() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    mark_repo_switch_secondary_metadata_ready(repo1_state);
    repo1_state.remotes = Loadable::NotLoaded;
    repo1_state.last_active_at = Some(SystemTime::now());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        has_full_refresh_only_effects(&effects, repo1),
        "missing secondary metadata should force the full refresh path"
    );
    assert!(
        has_worktree_refresh_effect(&effects, repo1),
        "expected worktrees refresh even on the full refresh path"
    );
}

#[test]
fn set_active_repo_uses_full_refresh_when_hot_switch_window_expires() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    mark_repo_switch_secondary_metadata_ready(repo1_state);
    repo1_state.last_active_at = Some(SystemTime::now() - Duration::from_secs(6));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        has_full_refresh_only_effects(&effects, repo1),
        "stale repo switches should fall back to the full refresh path"
    );
    assert!(
        has_worktree_refresh_effect(&effects, repo1),
        "expected worktrees refresh even when the hot-switch window expires"
    );
}

#[test]
fn set_fetch_prune_deleted_remote_tracking_branches_updates_and_noops() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let initial = state.repos[0].fetch_prune_deleted_remote_tracking_branches;
    let target = !initial;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFetchPruneDeletedRemoteTrackingBranches {
            repo_id: RepoId(1),
            enabled: target,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos[0].fetch_prune_deleted_remote_tracking_branches,
        target
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFetchPruneDeletedRemoteTrackingBranches {
            repo_id: RepoId(1),
            enabled: target,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos[0].fetch_prune_deleted_remote_tracking_branches,
        target
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFetchPruneDeletedRemoteTrackingBranches {
            repo_id: RepoId(999),
            enabled: !target,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos[0].fetch_prune_deleted_remote_tracking_branches,
        target
    );
}

#[test]
fn repo_opened_ok_sets_loading_and_emits_refresh_effects() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    state.repos[0].missing_on_disk = true;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    let repo_state = state.repos.first().unwrap();
    assert!(matches!(repo_state.open, Loadable::Ready(())));
    assert!(!repo_state.missing_on_disk);
    assert!(repo_state.head_branch.is_loading());
    assert!(repo_state.branches.is_loading());
    assert!(repo_state.tags.is_loading());
    assert!(repo_state.remote_tags.is_loading());
    assert!(repo_state.remotes.is_loading());
    assert!(repo_state.remote_branches.is_loading());
    assert!(repo_state.status.is_loading());
    assert!(repo_state.worktree_status_is_loading());
    assert!(repo_state.staged_status_is_loading());
    assert!(repo_state.log.is_loading());
    assert!(matches!(repo_state.stashes, Loadable::NotLoaded));
    assert!(matches!(repo_state.reflog, Loadable::NotLoaded));
    assert!(repo_state.upstream_divergence.is_loading());
    assert!(repo_state.rebase_in_progress.is_loading());
    assert!(repo_state.merge_commit_message.is_loading());
    assert!(repo_state.worktrees.is_loading());
    assert!(repo_state.submodules.is_loading());
    assert!(matches!(
        repo_state.history_state.file_history,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        repo_state.history_state.blame,
        Loadable::NotLoaded
    ));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(effect, Effect::LoadHeadBranch { repo_id: candidate } if *candidate == repo_id)
        }
    ));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(
                effect,
                Effect::LoadUpstreamDivergence {
                    repo_id: candidate
                } if *candidate == repo_id
            )
        }
    ));
    assert!(has_status_refresh_effects(&effects, RepoId(1)));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(effect, Effect::LoadLog { repo_id: candidate, .. } if *candidate == repo_id)
        }
    ));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(effect, Effect::LoadBranches { repo_id: candidate } if *candidate == repo_id)
        }
    ));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadTags { repo_id } if *repo_id == RepoId(1)
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadRemoteTags { repo_id } if *repo_id == RepoId(1)
    )));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(effect, Effect::LoadRemotes { repo_id: candidate } if *candidate == repo_id)
        }
    ));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(
                effect,
                Effect::LoadRemoteBranches {
                    repo_id: candidate
                } if *candidate == repo_id
            )
        }
    ));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(
                effect,
                Effect::LoadRebaseAndMergeState {
                    repo_id: candidate
                } if *candidate == repo_id
            )
        }
    ));
    assert!(has_worktree_refresh_effect(&effects, RepoId(1)));
    assert!(has_submodule_load_effect(&effects, RepoId(1)));
}

#[test]
fn repo_opened_ok_auto_loads_tags_when_enabled() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState {
        git_log_settings: GitLogSettings {
            show_history_tags: true,
            tag_fetch_mode: GitLogTagFetchMode::OnRepositoryActivation,
        },
        ..AppState::default()
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    let repo_state = state.repos.first().unwrap();
    assert!(repo_state.tags.is_loading());
    assert!(repo_state.remote_tags.is_loading());
    assert!(repo_state.submodules.is_loading());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadTags { repo_id } if *repo_id == RepoId(1)
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadRemoteTags { repo_id } if *repo_id == RepoId(1)
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSubmodules { repo_id } if *repo_id == RepoId(1)
    )));
}

#[test]
fn repo_opened_ok_for_closed_repo_is_ignored() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id: RepoId(1) },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    assert!(effects.is_empty());
    assert!(state.repos.is_empty());
    assert!(!repos.contains_key(&RepoId(1)));
}

#[test]
fn repo_opened_err_for_closed_repo_is_ignored() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/not-a-repo")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id: RepoId(1) },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/not-a-repo"),
            },
            error: Error::new(ErrorKind::NotARepository),
        }),
    );

    assert!(effects.is_empty());
    assert!(state.repos.is_empty());
    assert_eq!(state.active_repo, None);
    assert!(
        state.notifications.is_empty(),
        "stale open errors for a closed repo must not surface notifications"
    );
    assert!(!repos.contains_key(&RepoId(1)));
}

#[test]
fn repo_action_finished_clears_error_and_refreshes() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));
    state.repos[0].last_error = Some("boom".to_string());
    state.banner_error = Some(crate::model::BannerErrorState {
        repo_id: Some(RepoId(1)),
        message: "boom".to_string(),
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id: RepoId(1),
            action: RepoActionKind::CheckoutBranch,
            result: Ok(()),
        }),
    );

    assert!(state.repos[0].last_error.is_none());
    assert!(state.banner_error.is_none());
    assert!(has_status_refresh_effects(&effects, RepoId(1)));
}

#[test]
fn repo_action_finished_err_records_diagnostic() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let error = Error::new(ErrorKind::Backend("boom".to_string()));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id: RepoId(1),
            action: RepoActionKind::CheckoutBranch,
            result: Err(error),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(
        repo_state
            .last_error
            .as_deref()
            .is_some_and(|s| s.contains("boom"))
    );
    assert!(
        repo_state
            .diagnostics
            .iter()
            .any(|d| d.message.contains("boom"))
    );
}

#[test]
fn cherry_pick_error_completion_refreshes_status_log_and_sequencer_state() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);
    state.repos[0].local_actions_in_flight = 1;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::CherryPickCommit,
            result: Err(Error::new(ErrorKind::Backend("conflict".to_string()))),
        }),
    );

    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert!(
        state.repos[0]
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("conflict"))
    );
    assert!(
        has_status_refresh_effects(&effects, repo_id),
        "cherry-pick errors should refresh status so conflicts are visible, got {effects:?}"
    );
    assert!(
        effects.iter().any(
            |effect| matches!(effect, Effect::LoadLog { repo_id: candidate, .. } if *candidate == repo_id)
        ),
        "cherry-pick errors should refresh the log, got {effects:?}"
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadRebaseAndMergeState { repo_id: candidate } if *candidate == repo_id
        )),
        "cherry-pick errors should refresh merge/rebase/cherry-pick state, got {effects:?}"
    );
}

#[test]
fn repo_action_finished_bumps_load_epoch_and_forces_fresh_status_load_when_stale_in_flight() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREE_STATUS);
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::STAGED_STATUS);
    let old_epoch = state.repos[0].load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    assert!(
        state.repos[0].load_epoch > old_epoch,
        "load_epoch should be bumped to invalidate stale load results"
    );
    assert!(
        has_status_refresh_effects(&effects, repo_id),
        "a fresh status load should be dispatched even when a stale one was in-flight"
    );
    assert!(
        has_cancel_repo_loads_effect(&effects, repo_id, old_epoch),
        "the stale in-flight loads should be cancelled at the pre-bump epoch"
    );
}

#[test]
fn repo_action_finished_reissues_inflight_non_status_loads() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    // A primary refresh plus a branch refresh are in flight when the action completes. The epoch
    // bump invalidates all of them, so they must be re-issued, not left stuck in `in_flight`.
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREE_STATUS);
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::STAGED_STATUS);
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::HEAD_BRANCH);
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::BRANCHES);
    state.repos[0].branches = Loadable::Loading;
    let old_epoch = state.repos[0].load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    assert!(state.repos[0].load_epoch > old_epoch);
    assert!(has_cancel_repo_loads_effect(&effects, repo_id, old_epoch));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadHeadBranch { repo_id: r } if *r == repo_id)),
        "head branch should be re-loaded, not stranded in flight"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadBranches { repo_id: r } if *r == repo_id)),
        "branch list should be re-loaded, not stranded in flight"
    );
    assert!(has_status_refresh_effects(&effects, repo_id));
}

#[test]
fn repo_action_finished_reissues_inflight_sidebar_data_loads() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);
    state.repos[0].open = Loadable::Ready(());

    state.repos[0].set_sidebar_data_request(SidebarDataRequest {
        worktrees: true,
        submodules: true,
        stashes: true,
    });

    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREES);
    state.repos[0].worktrees = Loadable::Loading;

    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::SUBMODULES);
    state.repos[0].submodules = Loadable::Loading;

    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::STASHES);
    state.repos[0].stashes = Loadable::Loading;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    assert!(
        has_worktree_refresh_effect(&effects, repo_id),
        "worktrees should be re-loaded after a repo action, not stranded in NotLoaded"
    );
    assert!(
        has_submodule_load_effect(&effects, repo_id),
        "submodules should be re-loaded after a repo action, not stranded in NotLoaded"
    );
    assert!(
        has_stash_load_effect(&effects, repo_id),
        "stashes should be re-loaded after a repo action, not stranded in NotLoaded"
    );
}

#[test]
fn repo_action_finished_reissues_inflight_blame_and_commit_details() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    // The user has a blame and a commit-details view open and still loading.
    state.repos[0].history_state.blame_path = Some(PathBuf::from("src/main.rs"));
    state.repos[0].history_state.blame_source = Some(gitcomet_core::domain::BlameSource::Revision(
        Some("HEAD".to_string()),
    ));
    state.repos[0].history_state.blame = Loadable::Loading;
    state.repos[0].history_state.selected_commit = Some(CommitId("abc123".into()));
    state.repos[0].history_state.commit_details = Loadable::Loading;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    assert!(
        state.repos[0].history_state.blame.is_loading(),
        "blame should be reset and re-loaded, not stranded on a spinner"
    );
    assert!(
        state.repos[0].history_state.commit_details.is_loading(),
        "commit details should be reset and re-loaded, not stranded on a spinner"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadBlame { repo_id: r, .. } if *r == repo_id)),
        "a fresh blame load should be dispatched"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadCommitDetails { repo_id: r, .. } if *r == repo_id)),
        "a fresh commit-details load should be dispatched"
    );
}

#[test]
fn repo_action_finished_reissues_selected_commit_diff() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    // A historical commit's diff (a non-WorkingTree target) is open and loading. The old code only
    // re-issued WorkingTree diffs, leaving this one stranded.
    state.repos[0].diff_state.diff_target = Some(DiffTarget::Commit {
        commit_id: CommitId("abc123".into()),
        path: Some(PathBuf::from("src/main.rs")),
    });
    state.repos[0].diff_state.diff = Loadable::Loading;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadDiff {
                repo_id: r,
                target: DiffTarget::Commit { .. }
            } if *r == repo_id
        )),
        "a commit diff in flight should be re-loaded when its action completes"
    );
}

#[test]
fn repo_action_finished_invalidates_but_does_not_reissue_views_for_non_active_repo() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let background = RepoId(1);
    let active = RepoId(2);
    state.repos.push(RepoState::new_opening(
        background,
        RepoSpec {
            workdir: PathBuf::from("/tmp/bg"),
        },
    ));
    state.repos.push(RepoState::new_opening(
        active,
        RepoSpec {
            workdir: PathBuf::from("/tmp/active"),
        },
    ));
    state.active_repo = Some(active);

    // The background repo had a branch load and a blame load in flight when its action completed.
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::BRANCHES);
    state.repos[0].branches = Loadable::Loading;
    state.repos[0].history_state.blame_path = Some(PathBuf::from("src/main.rs"));
    state.repos[0].history_state.blame_source = Some(gitcomet_core::domain::BlameSource::Revision(
        Some("HEAD".to_string()),
    ));
    state.repos[0].history_state.blame = Loadable::Loading;
    let old_epoch = state.repos[0].load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id: background,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    // The background repo's stale loads are still invalidated, so nothing is left stranded ...
    assert!(state.repos[0].load_epoch > old_epoch);
    assert!(has_cancel_repo_loads_effect(
        &effects, background, old_epoch
    ));
    assert!(matches!(state.repos[0].branches, Loadable::NotLoaded));
    assert!(matches!(
        state.repos[0].history_state.blame,
        Loadable::NotLoaded
    ));
    // ... but its view-specific data is not eagerly re-issued; it reloads when next activated.
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadBlame { repo_id: r, .. } if *r == background)),
        "a non-active repo should not eagerly re-load blame"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadBranches { repo_id: r } if *r == background)),
        "a non-active repo should not eagerly re-load its branch list"
    );
}

#[test]
fn stale_status_result_after_repo_action_finished_is_dropped() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo");
    let repo_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id)
        .expect("repo exists");
    repo_state.set_status(Loadable::Loading);
    assert!(
        repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::WORKTREE_STATUS)
    );
    let old_epoch = repo_state.load_epoch;

    // The action completes and bumps the epoch, invalidating the in-flight status load.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    // The stale (pre-action) status result then arrives stamped with the old epoch.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
            repo_id,
            load_epoch: old_epoch,
            message: Box::new(crate::msg::InternalMsg::StatusLoaded {
                repo_id,
                result: Ok(RepoStatus::default()),
            }),
        }),
    );

    let repo_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .expect("repo exists");
    // It is dropped by the epoch gate: no effects, and it does not clobber the reset status ...
    assert!(effects.is_empty());
    assert!(matches!(repo_state.status, Loadable::NotLoaded));
    assert_ne!(repo_state.load_epoch, old_epoch);
    // ... while the fresh post-action status load is live (its flag belongs to the new epoch).
    assert!(
        repo_state
            .loads_in_flight
            .is_in_flight(RepoLoadsInFlight::WORKTREE_STATUS)
    );
}

#[test]
fn repo_opened_err_records_diagnostic() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let error = Error::new(ErrorKind::Backend("nope".to_string()));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            error,
        }),
    );

    let repo_state = &state.repos[0];
    assert!(
        repo_state
            .last_error
            .as_deref()
            .is_some_and(|s| s.contains("nope"))
    );
    assert!(
        repo_state
            .diagnostics
            .iter()
            .any(|d| d.message.contains("nope"))
    );
    assert!(!repo_state.missing_on_disk);
}

#[test]
fn repo_opened_err_not_found_marks_repo_missing_without_banner_error() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/missing-repo")),
    );

    let error = Error::new(ErrorKind::Io(std::io::ErrorKind::NotFound));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/missing-repo"),
            },
            error,
        }),
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.missing_on_disk);
    assert!(repo_state.last_error.is_none());
    assert!(repo_state.diagnostics.is_empty());
    assert!(matches!(repo_state.open, Loadable::Error(_)));
}

#[test]
fn repo_opened_err_not_a_repository_shows_notification_and_does_not_add_repo() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/not-a-repo")),
    );

    let error = Error::new(ErrorKind::NotARepository);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/not-a-repo"),
            },
            error,
        }),
    );

    assert!(state.repos.is_empty());
    assert_eq!(state.active_repo, None);
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("not a git repository"))
    );
}

#[test]
fn repo_opened_err_not_a_repository_opens_restored_fallback_tab() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let invalid_repo = dir.path().join("invalid");
    let fallback_repo = dir.path().join("fallback");
    std::fs::create_dir_all(&invalid_repo).expect("create invalid repo dir");
    std::fs::create_dir_all(&fallback_repo).expect("create fallback repo dir");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![invalid_repo.clone(), fallback_repo],
            active_repo: Some(invalid_repo.clone()),
        },
    );
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert!(matches!(state.repos[1].open, Loadable::NotLoaded));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: invalid_repo,
            },
            error: Error::new(ErrorKind::NotARepository),
        }),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.active_repo, Some(RepoId(2)));
    assert_eq!(state.repos[0].id, RepoId(2));
    assert!(matches!(state.repos[0].open, Loadable::Loading));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(2)
    )));
}

#[test]
fn repo_opened_err_not_a_repository_allows_opening_another_repo_afterwards() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/not-a-repo")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/not-a-repo"),
            },
            error: Error::new(ErrorKind::NotARepository),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.repos[0].id, RepoId(2));
    assert_eq!(
        state.repos[0].spec.workdir,
        super::reducer::normalize_repo_path(PathBuf::from("/tmp/repo"))
    );
    assert!(state.repos[0].open.is_loading());
    assert_eq!(state.active_repo, Some(RepoId(2)));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(2)))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PersistSession { .. }))
    );
}

#[test]
fn set_active_repo_ignores_unknown_repo() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    assert_eq!(state.active_repo, Some(RepoId(2)));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo {
            repo_id: RepoId(999),
        },
    );
    assert_eq!(state.active_repo, Some(RepoId(2)));
}

#[test]
fn diagnostics_are_capped() {
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );

    for i in 0..205 {
        super::reducer::push_diagnostic(&mut repo_state, DiagnosticKind::Error, format!("err-{i}"));
    }

    assert_eq!(repo_state.diagnostics.len(), 200);
    assert_eq!(repo_state.diagnostics[0].message, "err-5");
    assert_eq!(repo_state.diagnostics.last().unwrap().message, "err-204");
}

#[test]
fn session_persist_error_reports_notification_and_repo_diagnostic() {
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    super::reducer::handle_session_persist_result(
        &mut state,
        Some(RepoId(1)),
        "opening a repository",
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        )),
    );

    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("Failed to persist session state"))
    );
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("permission denied"))
    );
    assert!(
        state.repos[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("permission denied"))
    );
}

#[test]
fn session_persist_error_without_repo_still_reports_notification() {
    let mut state = AppState::default();

    super::reducer::handle_session_persist_result(
        &mut state,
        Some(RepoId(999)),
        "closing a repository",
        Err(std::io::Error::other("disk full")),
    );

    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("disk full"))
    );
    assert!(state.repos.is_empty());
}

#[test]
fn session_persist_failed_msg_reports_notification_and_repo_diagnostic() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SessionPersistFailed {
            repo_id: Some(RepoId(1)),
            action: "opening a repository",
            error: "disk full".to_string(),
        }),
    );

    assert!(effects.is_empty());
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("Failed to persist session state"))
    );
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("disk full"))
    );
    assert!(
        state.repos[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("disk full"))
    );
}

#[test]
fn set_active_repo_loads_file_browser_when_files_mode_is_active() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    mark_repo_open_ready(&mut repos, &mut state, repo1);
    mark_repo_open_ready(&mut repos, &mut state, repo2);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetSidebarMode {
            mode: SidebarMode::Files,
        },
    );

    // Activating a repo whose listing never loaded must kick the file
    // browser while the Files sidebar is showing, or the tree is stuck on
    // "Loading files..." until the user toggles the sidebar tabs.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadFileBrowser { repo_id, .. } if *repo_id == repo2
        )),
        "expected activation to load the file browser, got {effects:?}"
    );

    // An already-loaded listing must not reload on every activation.
    state
        .repos
        .iter_mut()
        .find(|r| r.id == repo1)
        .expect("repo1 exists")
        .file_browser
        .entries = Loadable::Ready(Arc::new(Vec::new()));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadFileBrowser { .. })),
        "expected no file browser reload for a loaded listing, got {effects:?}"
    );
}

#[test]
fn set_active_repo_skips_file_browser_load_in_branches_mode() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    mark_repo_open_ready(&mut repos, &mut state, repo1);
    mark_repo_open_ready(&mut repos, &mut state, repo2);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadFileBrowser { .. })),
        "expected no file browser load while the Branches sidebar is showing"
    );
}

#[test]
fn repo_opened_ok_loads_file_browser_for_active_repo_in_files_mode() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo1 = RepoId(1);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetSidebarMode {
            mode: SidebarMode::Files,
        },
    );

    // The repo was activated before its open completed; the open completing
    // must kick the file browser listing for the Files sidebar.
    let spec = state.repos[0].spec.clone();
    let workdir = spec.workdir.to_string_lossy().into_owned();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: repo1,
            spec,
            repo: Arc::new(DummyRepo::new(&workdir)),
        }),
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadFileBrowser { repo_id, .. } if *repo_id == repo1
        )),
        "expected RepoOpenedOk to load the file browser, got {effects:?}"
    );
}

/// The flat entry list the backend produces, as a small tree:
///
/// ```text
/// src/            src/nested/       other/
/// src/a.rs        src/nested/b.rs   other/c.rs
/// ```
fn file_browser_tree_entries() -> Vec<gitcomet_core::domain::FileEntry> {
    use gitcomet_core::domain::{FileEntry, FileEntryKind};

    let entry = |path: &str, kind, depth| FileEntry {
        name: PathBuf::from(path)
            .file_name()
            .expect("named entry")
            .to_string_lossy()
            .into_owned(),
        path: Arc::new(PathBuf::from(path)),
        kind,
        depth,
    };

    vec![
        entry("other", FileEntryKind::Directory, 0),
        entry("other/c.rs", FileEntryKind::File, 1),
        entry("src", FileEntryKind::Directory, 0),
        entry("src/nested", FileEntryKind::Directory, 1),
        entry("src/nested/b.rs", FileEntryKind::File, 2),
        entry("src/a.rs", FileEntryKind::File, 1),
    ]
}

fn state_with_file_browser_tree() -> (
    HashMap<RepoId, Arc<dyn GitRepository>>,
    AtomicU64,
    AppState,
    RepoId,
) {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo_id = RepoId(1);
    mark_repo_open_ready(&mut repos, &mut state, repo_id);
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(file_browser_tree_entries()));

    (repos, id_alloc, state, repo_id)
}

#[test]
fn recursive_expand_opens_the_folder_and_every_directory_under_it() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    let rev_before = state.repos[0].file_browser.file_browser_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::from("src"),
            expanded: true,
        },
    );

    let expanded = &state.repos[0].file_browser.expanded_dirs;
    // The invoked folder itself has to open too, or "Expand all under here"
    // would leave the subtree it just expanded hidden behind a closed row.
    assert!(expanded.contains(&Arc::new(PathBuf::from("src"))));
    assert!(expanded.contains(&Arc::new(PathBuf::from("src/nested"))));
    // Siblings outside the subtree are untouched, and files are not directories.
    assert!(!expanded.contains(&Arc::new(PathBuf::from("other"))));
    assert!(!expanded.contains(&Arc::new(PathBuf::from("src/a.rs"))));
    assert_eq!(expanded.len(), 2);
    assert_ne!(
        state.repos[0].file_browser.file_browser_rev, rev_before,
        "the tree has to repaint after a recursive expand"
    );
}

#[test]
fn recursive_collapse_closes_exactly_the_subtree_it_opened() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    for path in ["other", "src", "src/nested"] {
        state.repos[0]
            .file_browser
            .expanded_dirs
            .insert(Arc::new(PathBuf::from(path)));
    }

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::from("src"),
            expanded: false,
        },
    );

    let expanded = &state.repos[0].file_browser.expanded_dirs;
    assert_eq!(
        expanded,
        &[Arc::new(PathBuf::from("other"))]
            .into_iter()
            .collect::<std::collections::HashSet<_>>(),
        "collapsing a subtree must not disturb folders outside it"
    );
}

/// `starts_with` on a `Path` compares whole components, so a sibling whose name
/// merely begins with the same characters is a different folder. String-prefix
/// matching here would collapse `src_generated` along with `src`.
#[test]
fn recursive_expand_does_not_match_name_prefixes_of_sibling_folders() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![
        gitcomet_core::domain::FileEntry {
            name: "src".to_string(),
            path: Arc::new(PathBuf::from("src")),
            kind: gitcomet_core::domain::FileEntryKind::Directory,
            depth: 0,
        },
        gitcomet_core::domain::FileEntry {
            name: "src_generated".to_string(),
            path: Arc::new(PathBuf::from("src_generated")),
            kind: gitcomet_core::domain::FileEntryKind::Directory,
            depth: 0,
        },
    ]));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::from("src"),
            expanded: true,
        },
    );

    let expanded = &state.repos[0].file_browser.expanded_dirs;
    assert!(expanded.contains(&Arc::new(PathBuf::from("src"))));
    assert!(!expanded.contains(&Arc::new(PathBuf::from("src_generated"))));
}

/// A no-op must not bump the rev: the file browser's row cache is keyed on it,
/// so a bump on every right-click would throw away the memoized row list for
/// nothing.
#[test]
fn recursive_expand_of_an_already_expanded_subtree_does_not_bump_the_rev() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    for path in ["src", "src/nested"] {
        state.repos[0]
            .file_browser
            .expanded_dirs
            .insert(Arc::new(PathBuf::from(path)));
    }
    let rev_before = state.repos[0].file_browser.file_browser_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::from("src"),
            expanded: true,
        },
    );

    assert_eq!(state.repos[0].file_browser.file_browser_rev, rev_before);
}

/// A filtered tree force-expands every directory and never reads
/// `expanded_dirs`, so a toggle would move nothing on screen and then reshape
/// the tree the moment the search was cleared.
#[test]
fn folder_toggles_are_frozen_while_a_search_filters_the_tree() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    state.repos[0].file_browser.search_query = "a.rs".to_string();
    let rev_before = state.repos[0].file_browser.file_browser_rev;

    for msg in [
        Msg::ToggleFileBrowserDir {
            repo_id,
            path: PathBuf::from("src"),
        },
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::from("src"),
            expanded: true,
        },
    ] {
        reduce(&mut repos, &id_alloc, &mut state, msg);
    }

    assert!(
        state.repos[0].file_browser.expanded_dirs.is_empty(),
        "a filtered tree must keep the shape the user left it in"
    );
    assert_eq!(state.repos[0].file_browser.file_browser_rev, rev_before);
}

/// The search input is multiline and stores what was typed verbatim, so a lone
/// space is a non-empty query that filters nothing — the toggles stay live.
#[test]
fn folder_toggles_stay_live_for_a_whitespace_only_query() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    state.repos[0].file_browser.search_query = "   \n".to_string();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ToggleFileBrowserDir {
            repo_id,
            path: PathBuf::from("src"),
        },
    );

    assert!(
        state.repos[0]
            .file_browser
            .expanded_dirs
            .contains(&Arc::new(PathBuf::from("src")))
    );
}

#[test]
fn delete_branches_emits_one_effect_carrying_every_name() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo_id = RepoId(1);
    mark_repo_open_ready(&mut repos, &mut state, repo_id);

    let names = vec!["feat/a".to_string(), "feat/b".to_string()];
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteBranches {
            repo_id,
            names: names.clone(),
            force: true,
        },
    );

    let batched = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::DeleteBranches {
                repo_id: candidate,
                names,
                force,
            } if *candidate == repo_id => Some((names.clone(), *force)),
            _ => None,
        })
        .expect("expected a batched delete effect");
    // One effect for the whole batch, not one per branch: the scheduler needs
    // the full list to summarise partial failures.
    assert_eq!(batched.0, names);
    assert!(batched.1, "the force choice has to survive to the effect");
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::DeleteBranches { .. }))
            .count(),
        1
    );
}

#[test]
fn delete_branches_with_an_empty_list_does_nothing() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo_id = RepoId(1);
    mark_repo_open_ready(&mut repos, &mut state, repo_id);
    let busy_before = state.repos[0].local_actions_in_flight;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteBranches {
            repo_id,
            names: Vec::new(),
            force: false,
        },
    );

    assert!(effects.is_empty());
    // Crucially it must not mark the repo busy, or the UI would sit disabled
    // waiting on an action that never runs.
    assert_eq!(state.repos[0].local_actions_in_flight, busy_before);
}

#[test]
fn delete_remote_branches_keeps_the_batch_under_one_remote() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo_id = RepoId(1);
    mark_repo_open_ready(&mut repos, &mut state, repo_id);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteRemoteBranches {
            repo_id,
            remote: "origin".to_string(),
            branches: vec!["feat/a".to_string(), "feat/b".to_string()],
        },
    );

    let batched = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::DeleteRemoteBranches {
                remote, branches, ..
            } => Some((remote.clone(), branches.clone())),
            _ => None,
        })
        .expect("expected a batched remote delete effect");
    assert_eq!(batched.0, "origin");
    assert_eq!(batched.1.len(), 2);
}
