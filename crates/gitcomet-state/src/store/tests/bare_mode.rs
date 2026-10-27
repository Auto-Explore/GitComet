use super::*;
use crate::model::{BareModeConversionState, SidebarMode};
use crate::msg::{ControlEffect, InternalMsg, RepoCommandEffect, RepoPathList};
use gitcomet_core::domain::{FileSource, RepositoryMode};
use gitcomet_core::large_files::LargeFileCommand;
use gitcomet_core::services::ResetMode;

fn fixture(mode: RepositoryMode) -> (RepositoryRuntimes, AtomicU64, AppState, RepoId) {
    let repo_id = RepoId(1);
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: "/bare-state-test".into(),
        },
    );
    repo.repository_mode = Some(mode);
    repo.open = Loadable::Ready(());
    let mut state = AppState::test_default();
    state.repos.push(repo);
    state.active_repo = Some(repo_id);
    let mut repos = RepositoryRuntimes::default();
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/bare-state-test")));
    repos.bind_test_state(&state);
    (repos, AtomicU64::new(2), state, repo_id)
}

#[test]
fn bare_fast_stage_and_unstage_refuse_before_counters_or_effects() {
    for mode in [RepositoryMode::Bare, RepositoryMode::WorkingTree] {
        for action in 0..4 {
            let (_, _, mut state, repo_id) = fixture(mode);
            let mut effects = reducer::SinglePathActionEffects::new();
            let path = PathBuf::from("config");
            match action {
                0 => reducer::fill_stage_path_inline(&mut state, repo_id, path, &mut effects),
                1 => reducer::fill_stage_paths_inline(
                    &mut state,
                    repo_id,
                    RepoPathList::from(vec![path]),
                    &mut effects,
                ),
                2 => reducer::fill_unstage_path_inline(&mut state, repo_id, path, &mut effects),
                3 => reducer::fill_unstage_paths_inline(
                    &mut state,
                    repo_id,
                    RepoPathList::from(vec![path]),
                    &mut effects,
                ),
                _ => unreachable!(),
            }
            assert_eq!(
                effects.len(),
                usize::from(mode == RepositoryMode::WorkingTree)
            );
            assert_eq!(
                state.repos[0].local_actions_in_flight,
                u32::from(mode == RepositoryMode::WorkingTree)
            );
            if mode == RepositoryMode::Bare {
                assert_eq!(
                    state.repos[0].feedback.last_error.as_deref(),
                    Some("Requires a working tree; this repository is bare.")
                );
            }
        }
    }
}

#[test]
fn bare_worktree_commands_refuse_without_mutating_operation_counters() {
    let (mut repos, ids, mut state, repo_id) = fixture(RepositoryMode::Bare);
    for message in [
        Msg::Commit {
            repo_id,
            message: "message".into(),
            push_after_commit: false,
        },
        Msg::DropStash { repo_id, index: 0 },
        Msg::Reset {
            repo_id,
            mode: ResetMode::Mixed,
            target: "HEAD".into(),
        },
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
        Msg::OpenFileEditor {
            repo_id,
            path: "config".into(),
        },
        Msg::StagePath {
            repo_id,
            path: "config".into(),
        },
        Msg::SelectWorktreeUncommitted {
            request_id: None,
            repo_id,
            path: "/bare-state-test".into(),
        },
        Msg::DiffSession(crate::diff_session::DiffSessionMsg::OpenChanges {
            repo_id,
            lifetime: state.repos[0].lifetime(),
            view: crate::diff_session::DiffViewId::next(),
            source: crate::diff_session::ChangeSource::linked_worktree(
                "/bare-state-test".into(),
                DiffArea::Unstaged,
                true,
            ),
        }),
    ] {
        assert!(reduce(&mut repos, &ids, &mut state, message).is_empty());
        assert_eq!(state.repos[0].local_actions_in_flight, 0);
        assert_eq!(state.repos[0].pull_in_flight, 0);
        assert_eq!(state.repos[0].commit_in_flight, 0);
    }
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Reset {
            repo_id,
            mode: ResetMode::Soft,
            target: "HEAD".into(),
        },
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RepoCommand(RepoCommandEffect::Reset {
            mode: ResetMode::Soft,
            ..
        })
    )));
}

#[test]
fn bare_large_file_admission_preserves_paths_and_force_choice() {
    for (command, allowed) in [
        (
            LargeFileCommand::AnnexGet {
                paths: Vec::new(),
                from: None,
            },
            true,
        ),
        (
            LargeFileCommand::AnnexGet {
                paths: vec!["config".into()],
                from: None,
            },
            false,
        ),
        (
            LargeFileCommand::LfsUnlock {
                paths: vec!["file".into()],
                force: false,
            },
            false,
        ),
        (
            LargeFileCommand::LfsUnlock {
                paths: vec!["file".into()],
                force: true,
            },
            true,
        ),
        (
            LargeFileCommand::LfsFetchForDiff {
                target: DiffTarget::working_tree("file".into(), DiffArea::Staged),
            },
            false,
        ),
        (
            LargeFileCommand::LfsFetchForDiff {
                target: DiffTarget::commit(CommitId("abc".into()), "file".into()),
            },
            true,
        ),
    ] {
        let (mut repos, ids, mut state, repo_id) = fixture(RepositoryMode::Bare);
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::RunLargeFileCommand {
                repo_id,
                command: command.clone(),
            },
        );
        assert_eq!(effects.iter().any(|effect| matches!(effect, Effect::RepoCommand(RepoCommandEffect::RunLargeFileCommand { command: actual, .. }) if actual == &command)), allowed);
        assert_eq!(state.repos[0].local_actions_in_flight, u32::from(allowed));
    }
}

#[test]
fn bare_annex_pull_routes_before_worktree_gate() {
    let (mut repos, ids, mut state, repo_id) = fixture(RepositoryMode::Bare);
    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support.annex.uuid = Some("uuid".into());
    support.annex.has_annex_branch = true;
    state.repos[0].large_file_support = Loadable::Ready(Arc::new(support));
    state.repos[0].head_branch = Loadable::Ready("main".into());
    state.large_file_settings.annex_pull_push = true;
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(
            RepoCommandEffect::RunLargeFileCommand {
                command: LargeFileCommand::AnnexPull { .. },
                ..
            }
        )]
    ));
}

#[test]
fn authentication_replay_rechecks_bare_mode_and_preserves_supported_fetch() {
    for command in [
        RepoCommandKind::Pull {
            mode: PullMode::Default,
        },
        RepoCommandKind::FetchAll,
    ] {
        let blocked = matches!(command, RepoCommandKind::Pull { .. });
        let (mut repos, ids, mut state, repo_id) = fixture(RepositoryMode::Bare);
        state.auth_prompt = Some(crate::model::AuthPromptState {
            generation: 1,
            challenge: None,
            status: crate::model::AuthPromptStatus::AwaitingInput,
            kind: crate::model::AuthPromptKind::UsernamePassword,
            reason: "authentication required".into(),
            operation: crate::model::AuthRetryOperation::RepoCommand { repo_id, command },
        });
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::SubmitAuthPrompt {
                generation: 1,
                username: Some("fixture".into()),
                secret: "fixture-token".into(),
            },
        );
        assert_eq!(effects.is_empty(), blocked);
        assert_eq!(state.repos[0].pull_in_flight, u32::from(!blocked));
        assert_eq!(state.repos[0].worktree_pull_in_flight, 0);
    }
}

#[test]
fn stale_native_mode_handle_reopens_instead_of_retaining_a_failed_handle() {
    let (mut repos, ids, mut state, repo_id) = fixture(RepositoryMode::WorkingTree);
    let lifetime = state.repos[0].lifetime();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepositoryModeLoaded {
            repo_id,
            result: Err(Error::new(ErrorKind::RepositoryChanged)),
        }),
    );
    assert!(matches!(effects.as_slice(), [Effect::OpenRepo { .. }]));
    assert_ne!(state.repos[0].lifetime(), lifetime);
    assert!(state.repos[0].open.is_loading());
    assert!(!repos.has_repository(&repo_id));
}

#[test]
fn bare_open_is_ready_with_committed_browser_and_no_worktree_loads() {
    for head in [None, Some(CommitId("abc".into()))] {
        let (mut repos, ids, mut state, repo_id) = fixture(RepositoryMode::Bare);
        state.sidebar_mode = SidebarMode::Files;
        let spec = state.repos[0].spec.clone();
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::Internal(InternalMsg::RepoOpenedOk {
                repository_mode: RepositoryMode::Bare,
                head_commit: head.clone(),
                preferences: None,
                repo_id,
                spec,
                repo: Arc::new(DummyRepo::new("/bare-state-test")),
            }),
        );
        assert!(state.repos[0].is_bare());
        assert!(matches!(state.repos[0].open, Loadable::Ready(())));
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            Effect::Status(_)
                | Effect::LoadRebaseState { .. }
                | Effect::LoadRebaseAndMergeState { .. }
                | Effect::LoadMergeCommitMessage { .. }
        )));
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadFileBrowser {
                source: FileSource::WorkingDirectory,
                ..
            }
        )));
        match head {
            Some(head) => assert_eq!(state.repos[0].file_browser.source, FileSource::Commit(head)),
            None => assert!(
                matches!(&state.repos[0].file_browser.entries, Loadable::Ready(entries) if entries.is_empty())
            ),
        }
    }
}

#[test]
fn conversion_quiescence_retires_same_common_directory_and_rejects_stale_loads() {
    let (mut repos, ids, mut state, repo_id) = fixture(RepositoryMode::Bare);
    let common: Arc<Path> = Arc::from(Path::new("/bare-state-test"));
    state.repos[0].common_dir = Some(Arc::clone(&common));
    let old_lifetime = state.repos[0].lifetime();
    let old_epoch = state.repos[0].load_epoch;
    let mut linked = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: "/linked-state-test".into(),
        },
    );
    linked.common_dir = Some(common);
    linked.repository_mode = Some(RepositoryMode::WorkingTree);
    state.repos.push(linked);
    Arc::make_mut(&mut state.watch_leases).insert(repo_id, 1);
    Arc::make_mut(&mut state.worktree_watch_leases)
        .insert((repo_id, old_lifetime, "/linked-state-test".into()), 1);
    let (reply, _) = std::sync::mpsc::channel();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::QuiesceRepository {
            common_dir: "/bare-state-test".into(),
            cancellation: CancellationToken::new(),
            reply,
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::Control(ControlEffect::RepositoryQuiesced { .. })]
    ));
    assert!(state.repos.iter().all(|repo| matches!(
        repo.bare_mode_conversion,
        BareModeConversionState::Converting
    )));
    assert_ne!(state.repos[0].lifetime(), old_lifetime);
    assert!(!repos.has_repository(&repo_id));
    assert!(state.watch_leases.is_empty());
    assert!(state.worktree_watch_leases.is_empty());
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::RepoLoadFinished {
            repo_id,
            load_epoch: old_epoch,
            message: Box::new(InternalMsg::HeadBranchLoaded {
                repo_id,
                result: Ok("stale".into()),
            }),
        }),
    );
    assert!(!matches!(&state.repos[0].head_branch, Loadable::Ready(branch) if branch == "stale"));
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::StagePath {
            repo_id: RepoId(2),
            path: "file".into(),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.repos[1].local_actions_in_flight, 0);
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::BareModeDisabled {
            common_dir: "/bare-state-test".into(),
            result: Err(Error::new(ErrorKind::Cancelled)),
        }),
    );
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::OpenRepo { .. }))
            .count(),
        2
    );
    assert!(state.repos.iter().all(|repo| matches!(
        repo.bare_mode_conversion,
        BareModeConversionState::Failed(_)
    )));
}

#[test]
fn cancelled_quiesce_does_not_pause_a_window_after_conversion_aborts() {
    let (mut repos, ids, mut state, repo_id) = fixture(RepositoryMode::Bare);
    state.repos[0].common_dir = Some(Arc::from(Path::new("/bare-state-test")));
    let lifetime = state.repos[0].lifetime();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let (reply, _) = std::sync::mpsc::channel();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(InternalMsg::QuiesceRepository {
            common_dir: "/bare-state-test".into(),
            cancellation,
            reply,
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::Control(ControlEffect::RepositoryQuiesced { .. })]
    ));
    assert!(repos.has_repository(&repo_id));
    assert_eq!(state.repos[0].lifetime(), lifetime);
    assert!(matches!(
        state.repos[0].bare_mode_conversion,
        BareModeConversionState::Idle
    ));
}

#[test]
fn queued_stage_rechecks_native_mode_before_touching_index() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "main"]);
    fs::write(dir.path().join("tracked"), "content").unwrap();
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let handle = backend.open(dir.path()).unwrap();
    let (mut repos, _, mut state, repo_id) = fixture(RepositoryMode::WorkingTree);
    state.repos[0].set_spec(handle.spec().clone());
    repos.attach_repository_for_test(repo_id, handle);
    run_git(dir.path(), &["config", "core.bare", "true"]);
    let executor = executor::TaskExecutor::new(1);
    let (sender, receiver) = std::sync::mpsc::channel();
    super::effects::schedule_effect_with_state_for_test(
        &executor,
        &executor,
        &backend,
        &mut repos,
        state,
        sender,
        Effect::StagePath {
            repo_id,
            path: "tracked".into(),
        },
    );
    executor.join();
    assert!(!dir.path().join(".git/index").exists());
    assert!(receiver.try_iter().any(|message| matches!(message, Msg::Internal(InternalMsg::RepoActionFinished { result: Err(error), .. }) if matches!(error.kind(), ErrorKind::RepositoryChanged))));
}

#[test]
fn queued_editor_save_rechecks_live_bare_handle_before_filesystem_write() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "main"]);
    fs::write(dir.path().join("tracked"), "original").unwrap();
    run_git(dir.path(), &["config", "core.bare", "true"]);
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let handle = backend.open(dir.path()).unwrap();
    let (mut repos, _, mut state, repo_id) = fixture(RepositoryMode::WorkingTree);
    state.repos[0].set_spec(handle.spec().clone());
    repos.attach_repository_for_test(repo_id, handle);
    let executor = executor::TaskExecutor::new(1);
    let (sender, receiver) = std::sync::mpsc::channel();
    let (completion, saved) = smol::channel::bounded(1);
    super::effects::schedule_effect_with_state_for_test(
        &executor,
        &executor,
        &backend,
        &mut repos,
        state,
        sender,
        Effect::RepoCommand(RepoCommandEffect::SaveWorktreeFile {
            repo_id,
            path: "tracked".into(),
            contents: crate::msg::ContentBytes::from(b"replacement".to_vec()),
            expected_contents: Some(Arc::from(b"original".as_slice())),
            stage: false,
            completion: Some(completion),
        }),
    );
    executor.join();
    let result = loop {
        let message = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        if let Msg::Internal(InternalMsg::GitOperationFinished { message, .. }) = message
            && let InternalMsg::RepoCommandFinished { result, .. } = *message
        {
            break result;
        }
    };
    assert_eq!(fs::read(dir.path().join("tracked")).unwrap(), b"original");
    assert!(matches!(
        result.unwrap_err().kind(),
        ErrorKind::BareUnsupported
    ));
    assert!(!saved.try_recv().unwrap());
}

#[test]
fn bare_root_keeps_linked_checkout_rows_and_loads_the_linked_files() {
    let (_directory, root, linked) = repo_with_linked_worktree();
    run_git(&root, &["config", "core.bare", "true"]);
    fs::write(linked.join("a.txt"), "linked edit\n").unwrap();
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    store.disable_repo_monitors_for_test();
    let repo_id = super::worktree_redirect::open_repo_and_wait(&store, &root);
    assert!(store.snapshot().repos[0].is_bare());
    store.dispatch(Msg::LoadWorktreeDirty { repo_id });
    super::worktree_redirect::wait_until(
        "linked dirty row in bare repository",
        || matches!(&store.snapshot().repos[0].worktree_dirty, Loadable::Ready(rows) if rows.iter().any(|row| row.path == linked)),
    );
    let lifetime = store.snapshot().repos[0].lifetime();
    let view = crate::diff_session::DiffViewId::next();
    store.dispatch(Msg::DiffSession(
        crate::diff_session::DiffSessionMsg::OpenChanges {
            repo_id,
            lifetime,
            view,
            source: crate::diff_session::ChangeSource::linked_worktree(
                linked.clone(),
                DiffArea::Unstaged,
                true,
            ),
        },
    ));
    super::worktree_redirect::wait_until("linked file list through bare repository", || {
        store.snapshot().repos[0].change_lists.get(&view).is_some_and(|session| matches!(&session.files, Loadable::Ready(files) if files.iter().any(|file| file.path == Path::new("a.txt"))))
    });
    assert!(
        matches!(&store.snapshot().repos[0].status, Loadable::Ready(status) if status.staged.is_empty() && status.unstaged.is_empty())
    );
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}

#[test]
fn configuration_repair_reopens_main_and_linked_windows_and_preserves_index() {
    let (_directory, root, linked) = repo_with_linked_worktree();
    fs::write(root.join("a.txt"), "staged edit\n").unwrap();
    run_git(&root, &["add", "a.txt"]);
    fs::write(root.join("untracked.txt"), "untracked\n").unwrap();
    let index = fs::read(root.join(".git/index")).unwrap();
    run_git(&root, &["config", "core.bare", "true"]);
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (main, _events) = AppStore::new_test(Arc::clone(&backend));
    let (other, _other_events) = AppStore::new_test_sharing_preferences(backend, &main);
    main.disable_repo_monitors_for_test();
    other.disable_repo_monitors_for_test();
    let repo_id = super::worktree_redirect::open_repo_and_wait(&main, &root);
    let other_id = super::worktree_redirect::open_repo_and_wait(&other, &linked);
    let main_lifetime = main.snapshot().repos[0].lifetime();
    let other_lifetime = other.snapshot().repos[0].lifetime();
    let old = main.repository(repo_id, main_lifetime).unwrap();
    main.dispatch(Msg::PreviewDisableBareMode { repo_id });
    super::worktree_redirect::wait_until("configuration repair and main reopen", || {
        main.snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| {
                if let BareModeConversionState::Failed(error) = &repo.bare_mode_conversion {
                    panic!("configuration repair failed: {error:?}");
                }
                repo.has_working_tree()
                    && repo.lifetime() != main_lifetime
                    && matches!(repo.open, Loadable::Ready(()))
            })
    });
    super::worktree_redirect::wait_until("linked window reopen", || {
        other
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == other_id)
            .is_some_and(|repo| {
                repo.has_working_tree()
                    && repo.lifetime() != other_lifetime
                    && matches!(repo.open, Loadable::Ready(()))
            })
    });
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"staged edit\n");
    assert_eq!(
        fs::read(root.join("untracked.txt")).unwrap(),
        b"untracked\n"
    );
    assert!(matches!(
        old.repository_mode().unwrap_err().kind(),
        ErrorKind::RepositoryChanged
    ));
    assert_eq!(main.snapshot().repos[0].spec.workdir, root);
    assert_eq!(other.snapshot().repos[0].spec.workdir, linked);
    main.shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    other
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}

#[test]
fn focus_refresh_observes_external_bare_mode_changes() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "main"]);
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    store.disable_repo_monitors_for_test();
    let repo_id = super::worktree_redirect::open_repo_and_wait(&store, dir.path());
    assert!(store.snapshot().repos[0].has_working_tree());
    run_git(dir.path(), &["config", "core.bare", "true"]);
    store.dispatch(Msg::RepoActivated { repo_id });
    super::worktree_redirect::wait_until("external bare toggle", || {
        store.snapshot().repos[0].is_bare()
    });
    super::worktree_redirect::wait_until("external bare reopen", || {
        matches!(store.snapshot().repos[0].open, Loadable::Ready(()))
    });
    run_git(dir.path(), &["config", "core.bare", "false"]);
    store.dispatch(Msg::RepoActivated { repo_id });
    super::worktree_redirect::wait_until("external worktree toggle", || {
        store.snapshot().repos[0].has_working_tree()
    });
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}

fn filesystem_result(
    store: &AppStore,
    operation: gitcomet_core::filesystem::Operation,
) -> gitcomet_core::filesystem::OperationResult {
    let request = gitcomet_core::filesystem::Request::new(operation);
    let id = request.id;
    store.dispatch(Msg::FilesystemRequest(request));
    super::worktree_redirect::wait_until("filesystem result", || {
        store
            .snapshot()
            .filesystem
            .completed
            .iter()
            .any(|result| result.id == id)
    });
    store
        .snapshot()
        .filesystem
        .completed
        .iter()
        .find(|result| result.id == id)
        .unwrap()
        .clone()
}

#[cfg(unix)]
#[test]
fn transfers_through_directory_aliases_check_the_repository_owned_by_another_window() {
    use gitcomet_core::filesystem::{ItemOutcome, Operation, TransferIntent};

    fn file_bytes(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        let mut bytes = std::collections::BTreeMap::new();
        for entry in fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                bytes.extend(file_bytes(&entry.path()));
            } else {
                bytes.insert(entry.path(), fs::read(entry.path()).unwrap());
            }
        }
        bytes
    }

    let directory = tempfile::tempdir().unwrap();
    let root = gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let bare = root.join("bare");
    let normal = root.join("normal");
    let detached = root.join("detached");
    for target in [&bare, &normal, &detached] {
        fs::create_dir(target).unwrap();
    }
    run_git(&bare, &["init", "--bare", "-q", "-b", "main"]);
    run_git(&normal, &["init", "-q", "-b", "main"]);
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (owner, _owner_events) = AppStore::new_test(Arc::clone(&backend));
    let (other, _other_events) = AppStore::new_test(backend);
    owner.disable_repo_monitors_for_test();
    super::worktree_redirect::open_repo_and_wait(&owner, &bare);
    super::worktree_redirect::open_repo_and_wait(&owner, &normal);
    let original = file_bytes(&bare);
    for target in [&bare, &normal, &detached] {
        let alias = root.join(format!(
            "{}_alias",
            target.file_name().unwrap().to_str().unwrap()
        ));
        std::os::unix::fs::symlink(target, &alias).unwrap();
        for intent in [TransferIntent::Copy, TransferIntent::Move] {
            let name = if intent == TransferIntent::Copy {
                "copy.txt"
            } else {
                "move.txt"
            };
            let source = root.join(name);
            fs::write(&source, b"transferred contents").unwrap();
            let result = filesystem_result(
                &other,
                Operation::Transfer {
                    sources: vec![source.clone()],
                    destination: alias.clone(),
                    intent,
                },
            );
            if target == &bare {
                assert!(
                    matches!(&result.items[0].outcome, ItemOutcome::Failed(message) if message == "Requires a working tree; this repository is bare.")
                );
                assert!(result.changes.is_empty());
                assert_eq!(file_bytes(&bare), original);
                assert_eq!(fs::read(&source).unwrap(), b"transferred contents");
            } else {
                assert!(result.succeeded(), "{result:?}");
                assert_eq!(
                    fs::read(target.join(name)).unwrap(),
                    b"transferred contents"
                );
                assert_eq!(source.exists(), intent == TransferIntent::Copy);
            }
        }
    }
    owner
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    other
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}

#[test]
fn deleted_parent_replacement_checks_repository_mode_before_creating_directories() {
    use gitcomet_core::filesystem::{DiskVersion, ItemOutcome, Operation};
    let directory = tempfile::tempdir().unwrap();
    let root = gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let bare = root.join("bare");
    let normal = root.join("normal");
    let detached = root.join("detached");
    for target in [&bare, &normal, &detached] {
        fs::create_dir(target).unwrap();
    }
    run_git(&bare, &["init", "--bare", "-q", "-b", "main"]);
    run_git(&normal, &["init", "-q", "-b", "main"]);
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (owner, _owner_events) = AppStore::new_test(Arc::clone(&backend));
    let (other, _other_events) = AppStore::new_test(backend);
    owner.disable_repo_monitors_for_test();
    super::worktree_redirect::open_repo_and_wait(&owner, &bare);
    super::worktree_redirect::open_repo_and_wait(&owner, &normal);
    for target in [&bare, &normal, &detached] {
        let deleted = target.join("deleted");
        let path = deleted.join("nested/file.txt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"loaded").unwrap();
        let expected = DiskVersion::read(&path).unwrap();
        fs::remove_dir_all(&deleted).unwrap();
        let save = |overwrite| Operation::Save {
            path: path.clone(),
            worktree: (target != &detached).then(|| target.clone()),
            contents: Arc::from(b"retained edits".as_slice()),
            expected: Some(expected.clone()),
            overwrite,
        };
        assert!(!filesystem_result(&other, save(false)).succeeded());
        assert!(!deleted.exists());
        let result = filesystem_result(&other, save(true));
        if target == &bare {
            assert!(
                matches!(&result.items[0].outcome, ItemOutcome::Failed(message) if message == "Requires a working tree; this repository is bare.")
            );
            assert!(result.changes.is_empty());
            assert!(!deleted.exists());
        } else {
            assert!(result.succeeded(), "{result:?}");
            assert_eq!(fs::read(&path).unwrap(), b"retained edits");
            assert_eq!(
                result.saved_version,
                Some(DiskVersion::read(&path).unwrap())
            );
        }
    }
    owner
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    other
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}

#[test]
fn detached_filesystem_work_does_not_wait_for_an_unrelated_conversion_gate() {
    use gitcomet_core::filesystem::Operation;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("repo");
    fs::create_dir(&root).unwrap();
    run_git(&root, &["init", "--bare", "-q", "-b", "main"]);
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    store.disable_repo_monitors_for_test();
    let repo_id = super::worktree_redirect::open_repo_and_wait(&store, &root);
    let repo = store
        .repository(repo_id, store.snapshot().repos[0].lifetime())
        .unwrap();
    let gate = super::super::mutation_gate::for_repo(repo.as_ref());
    let mutation = gate.write().unwrap();
    let path = directory.path().join("document.txt");
    assert!(filesystem_result(&store, Operation::CreateFile { path: path.clone() }).succeeded());
    assert!(path.exists());
    drop(mutation);
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}

#[test]
fn filesystem_repository_lease_lives_through_the_actual_write() {
    struct LeaseObserver {
        path: PathBuf,
        outcome: Arc<AtomicU64>,
    }
    impl Drop for LeaseObserver {
        fn drop(&mut self) {
            self.outcome.store(
                if fs::read(&self.path).ok().as_deref() == Some(b"replacement") {
                    1
                } else {
                    2
                },
                Ordering::SeqCst,
            );
        }
    }
    struct LeaseRepo {
        inner: DummyRepo,
        path: PathBuf,
        outcome: Arc<AtomicU64>,
    }
    impl GitRepository for LeaseRepo {
        fn spec(&self) -> &RepoSpec {
            self.inner.spec()
        }
        fn log_head_page(
            &self,
            limit: usize,
            cursor: Option<&LogCursor>,
            cancellation: &CancellationToken,
        ) -> Result<Arc<LogPage>> {
            self.inner.log_head_page(limit, cursor, cancellation)
        }
        fn commit_details(&self, id: &CommitId) -> Result<CommitDetails> {
            self.inner.commit_details(id)
        }
        fn reflog_head(&self, limit: usize) -> Result<Vec<ReflogEntry>> {
            self.inner.reflog_head(limit)
        }
        fn current_branch(&self, cancellation: &CancellationToken) -> Result<String> {
            self.inner.current_branch(cancellation)
        }
        fn list_branches(&self, cancellation: &CancellationToken) -> Result<Vec<Branch>> {
            self.inner.list_branches(cancellation)
        }
        fn list_remotes(&self, cancellation: &CancellationToken) -> Result<Vec<Remote>> {
            self.inner.list_remotes(cancellation)
        }
        fn list_remote_branches(
            &self,
            cancellation: &CancellationToken,
        ) -> Result<Vec<RemoteBranch>> {
            self.inner.list_remote_branches(cancellation)
        }
        fn status(&self, cancellation: &CancellationToken) -> Result<RepoStatus> {
            self.inner.status(cancellation)
        }
        fn diff_unified(&self, target: &DiffTarget) -> Result<String> {
            self.inner.diff_unified(target)
        }
        fn create_branch(&self, name: &str, target: &CommitId) -> Result<()> {
            self.inner.create_branch(name, target)
        }
        fn delete_branch(&self, name: &str) -> Result<()> {
            self.inner.delete_branch(name)
        }
        fn checkout_branch(&self, name: &str) -> Result<()> {
            self.inner.checkout_branch(name)
        }
        fn checkout_commit(&self, id: &CommitId) -> Result<()> {
            self.inner.checkout_commit(id)
        }
        fn cherry_pick(&self, id: &CommitId) -> Result<()> {
            self.inner.cherry_pick(id)
        }
        fn stash_create(&self, message: &str, include_untracked: bool) -> Result<()> {
            self.inner.stash_create(message, include_untracked)
        }
        fn stash_list(&self, cancellation: &CancellationToken) -> Result<Vec<StashEntry>> {
            self.inner.stash_list(cancellation)
        }
        fn stash_apply(&self, index: usize) -> Result<()> {
            self.inner.stash_apply(index)
        }
        fn stash_drop(&self, index: usize) -> Result<()> {
            self.inner.stash_drop(index)
        }
        fn stage(&self, paths: &[&Path]) -> Result<()> {
            self.inner.stage(paths)
        }
        fn unstage(&self, paths: &[&Path]) -> Result<()> {
            self.inner.unstage(paths)
        }
        fn commit(&self, message: &str) -> Result<()> {
            self.inner.commit(message)
        }
        fn fetch_all(&self, prune: bool) -> Result<CommandOutput> {
            self.inner.fetch_all(prune)
        }
        fn pull(&self, mode: PullMode, prune: bool) -> Result<CommandOutput> {
            self.inner.pull(mode, prune)
        }
        fn push(&self) -> Result<()> {
            self.inner.push()
        }
        fn discard_worktree_changes(&self, paths: &[&Path]) -> Result<()> {
            self.inner.discard_worktree_changes(paths)
        }
        fn acquire_operation_lease(
            &self,
        ) -> Result<gitcomet_core::services::RepositoryOperationLease> {
            Ok(gitcomet_core::services::RepositoryOperationLease::new(
                LeaseObserver {
                    path: self.path.clone(),
                    outcome: Arc::clone(&self.outcome),
                },
            ))
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("repo");
    fs::create_dir(&root).unwrap();
    let root = canonicalize_or_original(root);
    let path = root.join("file");
    fs::write(&path, b"original").unwrap();
    let operations = vec![gitcomet_core::filesystem::Operation::Save {
        path: path.clone(),
        worktree: Some(root.clone()),
        contents: Arc::from(b"replacement".as_slice()),
        expected: None,
        overwrite: true,
    }];
    #[cfg(unix)]
    let operations = {
        use gitcomet_core::filesystem::{Operation, TransferIntent};
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        let mut operations = operations;
        for intent in [TransferIntent::Copy, TransferIntent::Move] {
            let parent = directory.path().join(if intent == TransferIntent::Copy {
                "copy"
            } else {
                "move"
            });
            fs::create_dir(&parent).unwrap();
            let source = parent.join("file");
            fs::write(&source, b"replacement").unwrap();
            operations.push(Operation::Transfer {
                sources: vec![source],
                destination: alias.clone(),
                intent,
            });
        }
        operations
    };
    for operation in operations {
        let outcome = Arc::new(AtomicU64::new(0));
        let repository: Arc<dyn GitRepository> = Arc::new(LeaseRepo {
            inner: DummyRepo::new(&root),
            path: path.clone(),
            outcome: Arc::clone(&outcome),
        });
        let (mut repos, _, mut state, repo_id) = fixture(RepositoryMode::WorkingTree);
        state.repos[0].set_spec(repository.spec().clone());
        repos.attach_repository_for_test(repo_id, repository);
        let executor = executor::TaskExecutor::new(1);
        let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
        let (sender, receiver) = std::sync::mpsc::channel();
        super::effects::schedule_effect_with_state_for_test(
            &executor,
            &executor,
            &backend,
            &mut repos,
            state,
            sender,
            Effect::Filesystem(gitcomet_core::filesystem::Request::new(operation)),
        );
        loop {
            if let Msg::FilesystemFinished(result) =
                receiver.recv_timeout(Duration::from_secs(10)).unwrap()
            {
                assert!(result.succeeded(), "{result:?}");
                break;
            }
        }
        let (completed, acknowledged) = std::sync::mpsc::channel();
        executor::FILESYSTEM_QUEUE.spawn(move || completed.send(()).unwrap());
        acknowledged.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(outcome.load(Ordering::SeqCst), 1);
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        fs::remove_file(&path).unwrap();
    }
}

#[test]
fn undo_in_another_window_checks_the_owned_repository_after_external_mode_change() {
    use gitcomet_core::filesystem::Operation;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("repo");
    fs::create_dir(&root).unwrap();
    run_git(&root, &["init", "-q", "-b", "main"]);
    fs::write(root.join("a.txt"), "preserved").unwrap();
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (owner, _owner_events) = AppStore::new_test(Arc::clone(&backend));
    let (other, _other_events) = AppStore::new_test(backend);
    owner.disable_repo_monitors_for_test();
    let repo_id = super::worktree_redirect::open_repo_and_wait(&owner, &root);
    assert!(
        filesystem_result(
            &other,
            Operation::Rename {
                source: root.join("a.txt"),
                name: "b.txt".into(),
            }
        )
        .succeeded()
    );
    run_git(&root, &["config", "core.bare", "true"]);
    assert!(!filesystem_result(&other, Operation::Undo).succeeded());
    assert!(!root.join("a.txt").exists());
    assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"preserved");
    owner.dispatch(Msg::RepoActivated { repo_id });
    super::worktree_redirect::wait_until("bare owned root in other window", || {
        owner.snapshot().repos[0].is_bare()
            && matches!(owner.snapshot().repos[0].open, Loadable::Ready(()))
    });
    let refused = filesystem_result(&other, Operation::Undo);
    assert!(
        matches!(&refused.items[0].outcome, gitcomet_core::filesystem::ItemOutcome::Failed(message) if message == "Requires a working tree; this repository is bare.")
    );
    assert!(refused.undo_available);
    let detached = directory.path().join("document.txt");
    assert!(
        filesystem_result(
            &other,
            Operation::CreateFile {
                path: detached.clone()
            }
        )
        .succeeded()
    );
    assert!(detached.exists());
    assert!(filesystem_result(&other, Operation::Undo).succeeded());
    assert!(!detached.exists());
    run_git(&root, &["config", "core.bare", "false"]);
    owner.dispatch(Msg::RepoActivated { repo_id });
    super::worktree_redirect::wait_until("working tree restored in other window", || {
        owner.snapshot().repos[0].has_working_tree()
            && matches!(owner.snapshot().repos[0].open, Loadable::Ready(()))
    });
    assert!(filesystem_result(&other, Operation::Undo).succeeded());
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"preserved");
    owner
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    other
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}

#[test]
fn bare_directory_conversion_rekeys_preferences_and_rebuilds_watching() {
    let (directory, source, _linked) = repo_with_linked_worktree();
    let root = directory.path().join("bare");
    run_git(
        directory.path(),
        &[
            "clone",
            "--bare",
            "-q",
            source.to_str().unwrap(),
            root.to_str().unwrap(),
        ],
    );
    let root = canonicalize_or_original(root);
    let session = directory.path().join("session.json");
    let _session = crate::session::push_test_session_file_path_override(Some(session.clone()));
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let repo_id = super::worktree_redirect::open_repo_and_wait(&store, &root);
    let lifetime = store.snapshot().repos[0].lifetime();
    store.dispatch(Msg::UpdateRepositoryPreference {
        repo_id,
        update: crate::model::RepositoryPreferenceUpdate::Pin {
            key: "local:main".into(),
            pinned: true,
        },
    });
    let old = crate::model::RepositoryKey::CommonDir(root.clone());
    super::worktree_redirect::wait_until("bare preference persisted", || {
        fs::read(&session)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|stored| {
                stored["repository_preferences"][old.storage_key()]["pinned_items"]
                    == serde_json::json!(["local:main"])
            })
    });
    store.dispatch(Msg::PreviewDisableBareMode { repo_id });
    super::worktree_redirect::wait_until("bare directory confirmation", || {
        let snapshot = store.snapshot();
        if let BareModeConversionState::Failed(error) = &snapshot.repos[0].bare_mode_conversion {
            panic!("bare preview failed: {error:?}");
        }
        matches!(
            snapshot.repos[0].bare_mode_conversion,
            BareModeConversionState::AwaitingConfirmation(_)
        )
    });
    let BareModeConversionState::AwaitingConfirmation(preview) =
        store.snapshot().repos[0].bare_mode_conversion.clone()
    else {
        unreachable!();
    };
    store.dispatch(Msg::DisableBareMode { repo_id, preview });
    super::worktree_redirect::wait_until("bare directory reopened", || {
        let snapshot = store.snapshot();
        let repo = &snapshot.repos[0];
        if let BareModeConversionState::Failed(error) = &repo.bare_mode_conversion {
            panic!("bare conversion failed: {error:?}");
        }
        repo.has_working_tree()
            && repo.lifetime() != lifetime
            && matches!(repo.open, Loadable::Ready(()))
    });
    let snapshot = store.snapshot();
    let repo = &snapshot.repos[0];
    assert_eq!(repo.spec.workdir, root);
    assert_eq!(
        repo.common_dir.as_deref(),
        Some(root.join(".git").as_path())
    );
    assert!(
        repo.shared_preferences
            .as_ref()
            .unwrap()
            .preferences
            .pinned_items
            .contains("local:main")
    );
    let stored: serde_json::Value = serde_json::from_slice(&fs::read(session).unwrap()).unwrap();
    let new = crate::model::RepositoryKey::CommonDir(root.join(".git"));
    assert!(
        stored["repository_preferences"]
            .get(old.storage_key())
            .is_none()
    );
    assert_eq!(
        stored["repository_preferences"][new.storage_key()]["pinned_items"],
        serde_json::json!(["local:main"])
    );
    store
        .msg_tx
        .repo_monitor_barrier_for_test(repo_id)
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    fs::write(root.join("a.txt"), "watched change\n").unwrap();
    super::worktree_redirect::wait_until("new checkout watcher", || {
        store.snapshot().repos[0]
            .worktree_status_entries()
            .is_some_and(|files| files.iter().any(|file| file.path == Path::new("a.txt")))
    });
    run_git(&root, &["config", "core.bare", "true"]);
    super::worktree_redirect::wait_until("watched external bare toggle", || {
        let snapshot = store.snapshot();
        snapshot.repos[0].is_bare() && matches!(snapshot.repos[0].open, Loadable::Ready(()))
    });
    store
        .msg_tx
        .repo_monitor_barrier_for_test(repo_id)
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    run_git(&root, &["config", "core.bare", "false"]);
    super::worktree_redirect::wait_until("watched external worktree toggle", || {
        let snapshot = store.snapshot();
        snapshot.repos[0].has_working_tree()
            && matches!(snapshot.repos[0].open, Loadable::Ready(()))
    });
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}
