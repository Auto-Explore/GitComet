//! Repository operations: stash, branches, checkout, clone, saves, attributes.

use super::*;

/// Exercise a worker effect and feed all its replies through the reducer.
fn apply_effect_with_state_for_test(
    executor: &super::super::executor::TaskExecutor,
    backend: &Arc<dyn GitBackend>,
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    effect: Effect,
) -> Vec<Effect> {
    let (tx, rx) = std::sync::mpsc::channel();
    schedule_effect_with_state_for_test(
        executor,
        executor,
        backend,
        repos,
        state.clone(),
        tx,
        effect,
    );
    let mut followups = Vec::new();
    loop {
        match recv_effect_message(&rx, Duration::from_secs(5)) {
            Ok(reply) => followups.extend(reduce(repos, &AtomicU64::new(9600), state, reply)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return followups,
            Err(error) => panic!("effect did not finish: {error}"),
        }
    }
}

fn selected_diff_effect(effects: Vec<Effect>) -> Effect {
    effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::LoadSelectedDiff { .. }))
        .expect("selected diff refresh")
}

#[test]
fn attribute_refresh_redecodes_staged_and_commit_diffs() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    run_git(workdir, &["init", "-q"]);
    run_git(workdir, &["config", "user.name", "Test"]);
    run_git(workdir, &["config", "user.email", "test@example.com"]);
    run_git(workdir, &["config", "commit.gpgsign", "false"]);
    fs::write(workdir.join("menu.txt"), b"\xf0\xd2\xc9\xd7\xc5\xd4\n").unwrap();
    run_git(workdir, &["add", "menu.txt"]);
    run_git(workdir, &["commit", "-qm", "base"]);
    fs::write(workdir.join("menu.txt"), b"\xf0\xd2\xc9\xd7\xc5\xd4!\n").unwrap();
    run_git(workdir, &["commit", "-qam", "change"]);
    fs::write(workdir.join("menu.txt"), b"\xf0\xd2\xc9\xd7\xc5\xd4!!\n").unwrap();
    run_git(workdir, &["add", "menu.txt"]);

    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let repo = backend.open(workdir).unwrap();
    let repo_id = RepoId(9530);
    let mut repos = FxHashMap::default();
    repos.insert(repo_id, repo.clone());
    let executor = super::super::executor::TaskExecutor::new(1);
    let resolve_commit = |revision: &str| {
        let output = Command::new("git")
            .arg("-C")
            .arg(workdir)
            .args(["rev-parse", revision])
            .output()
            .unwrap();
        assert!(output.status.success());
        CommitId(String::from_utf8(output.stdout).unwrap().trim().into())
    };
    let head = resolve_commit("HEAD");
    let base = resolve_commit("HEAD^");
    for (target, expected) in [
        (
            DiffTarget::working_tree("menu.txt".into(), DiffArea::Staged),
            "Привет!!\n",
        ),
        (
            DiffTarget::commit(head.clone(), "menu.txt".into()),
            "Привет!\n",
        ),
        (
            DiffTarget::commit_range(base, Some(head), Some("menu.txt".into())),
            "Привет!\n",
        ),
    ] {
        fs::write(
            workdir.join(".gitattributes"),
            "*.txt encoding=windows-1252\n",
        )
        .unwrap();
        let mut state = AppState::test_default();
        let mut repo_state = RepoState::new_opening(repo_id, repo.spec().clone());
        repo_state.diff_state.diff_target = Some(target.clone());
        repo_state.diff_state.text_attributes = Loadable::Ready(Arc::new(
            repo.text_attributes(Path::new("menu.txt")).unwrap(),
        ));
        let original = Arc::new(
            repo.diff_parsed_with_encoding_cancellable(&target, None, &CancellationToken::new())
                .unwrap(),
        );
        repo_state.diff_state.diff = Loadable::Ready(original.clone());
        repo_state.diff_state.diff_file = Loadable::Ready(
            repo.diff_file_text_with_encoding_cancellable(&target, None, &CancellationToken::new())
                .unwrap()
                .map(Arc::new),
        );
        state.repos.push(repo_state);
        state.active_repo = Some(repo_id);

        fs::write(workdir.join(".gitattributes"), "*.txt encoding=koi8-r\n").unwrap();
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(9531),
            &mut state,
            Msg::RepoExternallyChanged {
                repo_id,
                change: crate::msg::RepoExternalChange {
                    text_attributes: true,
                    ..crate::msg::RepoExternalChange::Worktree
                },
            },
        );
        let followups = apply_effect_with_state_for_test(
            &executor,
            &backend,
            &mut repos,
            &mut state,
            selected_diff_effect(effects),
        );
        assert!(
            matches!(&state.repos[0].diff_state.diff, Loadable::Ready(diff) if Arc::ptr_eq(diff, &original)),
            "keep content visible until the replacement arrives"
        );
        let reload = selected_diff_effect(followups);
        assert!(matches!(
            reload,
            Effect::LoadSelectedDiff {
                load_patch_diff: true,
                load_file_text: true,
                ..
            }
        ));
        assert!(
            apply_effect_with_state_for_test(&executor, &backend, &mut repos, &mut state, reload)
                .is_empty(),
            "an unchanged attribute reply must not reload again"
        );

        let diff_state = &state.repos[0].diff_state;
        assert_eq!(diff_state.diff_target.as_ref(), Some(&target));
        let Loadable::Ready(diff) = &diff_state.diff else {
            panic!("patch did not reload")
        };
        assert!(
            diff.lines
                .iter()
                .any(|line| line.kind == gitcomet_core::domain::DiffLineKind::Add
                    && line.text.as_ref() == format!("+{}", expected.trim_end()))
        );
        let Loadable::Ready(Some(file)) = &diff_state.diff_file else {
            panic!("file text did not reload")
        };
        assert_eq!(
            fs::read_to_string(&file.new_source.as_ref().unwrap().path).unwrap(),
            expected
        );
    }
}

#[test]
fn saving_attributes_preserves_choices_even_when_the_rule_is_shadowed() {
    use gitcomet_core::text_format::{TextEncoding, TextOverride};
    for shadow in [
        None,
        Some("sub/.gitattributes"),
        Some(".git/info/attributes"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        run_git(workdir, &["init", "-q"]);
        fs::create_dir(workdir.join("sub")).unwrap();
        fs::write(workdir.join("sub/menu.txt"), b"\xf0\xd2\xc9\xd7\xc5\xd4\n").unwrap();
        fs::write(
            workdir.join(".gitattributes"),
            "*.txt encoding=windows-1252\n",
        )
        .unwrap();
        if let Some(shadow) = shadow {
            fs::write(workdir.join(shadow), "*.txt encoding=windows-1252\n").unwrap();
        }
        let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
        let repo = backend.open(workdir).unwrap();
        let repo_id = RepoId(9540);
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo.clone());
        let mut state = AppState::test_default();
        let mut repo_state = RepoState::new_opening(repo_id, repo.spec().clone());
        let path = PathBuf::from("sub/menu.txt");
        repo_state.diff_state.diff_target =
            Some(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged));
        repo_state.diff_state.content_preview = true;
        repo_state.diff_state.text_attributes =
            Loadable::Ready(Arc::new(repo.text_attributes(&path).unwrap()));
        let chosen = TextOverride {
            encoding: TextEncoding::from_label("koi8-r"),
            tab_size: Some(3),
        };
        repo_state.diff_state.text_override = Some(crate::model::OpenFileTextOverride {
            path: path.clone(),
            value: chosen,
        });
        state.repos.push(repo_state);
        state.active_repo = Some(repo_id);
        let executor = super::super::executor::TaskExecutor::new(1);
        let rule = "/sub/menu.txt encoding=KOI8-R".to_string();
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(9541),
            &mut state,
            Msg::AppendGitattributesRule {
                repo_id,
                rule: rule.clone(),
            },
        );
        let followups = apply_effect_with_state_for_test(
            &executor,
            &backend,
            &mut repos,
            &mut state,
            effects.into_iter().next().unwrap(),
        );
        apply_effect_with_state_for_test(
            &executor,
            &backend,
            &mut repos,
            &mut state,
            selected_diff_effect(followups),
        );
        assert!(
            fs::read_to_string(workdir.join(".gitattributes"))
                .unwrap()
                .ends_with(&format!("{rule}\n"))
        );
        let diff = &state.repos[0].diff_state;
        assert_eq!(
            diff.text_override_for(&path),
            Some(chosen),
            "shadow: {shadow:?}"
        );
        assert_eq!(diff.text_override_rev, 0);
        let Loadable::Ready(attributes) = &diff.text_attributes else {
            panic!("attributes did not reload")
        };
        assert_eq!(
            attributes.encoding.as_ref().unwrap().encoding,
            if shadow.is_some() {
                Some(TextEncoding::WINDOWS_1252)
            } else {
                chosen.encoding
            }
        );
        let decoded = gitcomet_core::text_format::decode_bytes(
            &fs::read(workdir.join(&path)).unwrap(),
            gitcomet_core::text_format::SideKind::Worktree,
            attributes,
            diff.selected_encoding_override(),
        )
        .text
        .into_owned();
        assert_eq!(decoded, "Привет\n");
    }
}

#[test]
fn repository_refreshes_reload_selected_text_attributes_from_git() {
    use crate::msg::RepoExternalChange;
    use gitcomet_core::text_format::{TabWidthSource, TextEncoding};
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q"]);
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let repo = backend.open(dir.path()).unwrap();
    let repo_id = RepoId(9521);
    let path = PathBuf::from("menu.txt");
    let mut repos = FxHashMap::default();
    repos.insert(repo_id, repo.clone());
    let executor = super::super::executor::TaskExecutor::new(1);
    let id_alloc = AtomicU64::new(9522);
    for refresh in [
        Msg::ReloadRepo { repo_id },
        Msg::RepoExternallyChanged {
            repo_id,
            change: RepoExternalChange {
                text_attributes: true,
                ..RepoExternalChange::Worktree
            },
        },
        Msg::RepoExternallyChanged {
            repo_id,
            change: RepoExternalChange {
                text_attributes: true,
                ..RepoExternalChange::Index
            },
        },
        Msg::RepoExternallyChanged {
            repo_id,
            change: RepoExternalChange {
                text_attributes: true,
                ..RepoExternalChange::GitState
            },
        },
    ] {
        fs::write(
            dir.path().join(".gitattributes"),
            "/menu.txt encoding=windows-1252 whitespace=tabwidth=2\n",
        )
        .unwrap();
        let original = repo.text_attributes(&path).unwrap();
        let mut state = AppState::test_default();
        let mut repo_state = RepoState::new_opening(repo_id, repo.spec().clone());
        repo_state.diff_state.diff_target =
            Some(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged));
        // This preview reads from disk and does not otherwise load a diff.
        repo_state.diff_state.content_preview = true;
        repo_state.diff_state.text_attributes = Loadable::Ready(Arc::new(original.clone()));
        let original_rev = repo_state.diff_state.text_attributes_rev;
        state.repos.push(repo_state);
        state.active_repo = Some(repo_id);
        fs::write(
            dir.path().join(".gitattributes"),
            "/menu.txt encoding=koi8-r whitespace=tabwidth=8\n",
        )
        .unwrap();
        let effects = reduce(&mut repos, &id_alloc, &mut state, refresh);
        let load = effects
            .into_iter()
            .find(|effect| matches!(effect, Effect::LoadSelectedDiff { .. }))
            .expect("a refresh must request the selected file's attributes");
        let (tx, rx) = std::sync::mpsc::channel();
        schedule_effect_with_state_for_test(
            &executor,
            &executor,
            &backend,
            &repos,
            state.clone(),
            tx,
            load,
        );
        let reply = recv_effect_message(&rx, Duration::from_secs(5)).unwrap();
        assert!(matches!(
            reply,
            Msg::Internal(crate::msg::InternalMsg::TextAttributesLoaded { .. })
        ));
        reduce(&mut repos, &id_alloc, &mut state, reply);
        let diff = &state.repos[0].diff_state;
        let Loadable::Ready(attributes) = &diff.text_attributes else {
            panic!("attributes did not load")
        };
        assert_ne!(attributes.as_ref(), &original);
        assert_eq!(
            attributes.encoding.as_ref().and_then(|attr| attr.encoding),
            TextEncoding::from_label("koi8-r")
        );
        let tab = attributes.tab_width.unwrap();
        assert_eq!(tab.columns, 8);
        assert_eq!(tab.source, TabWidthSource::Attribute);
        assert!(diff.text_attributes_rev > original_rev);
        let revision = diff.text_attributes_rev;
        let attributes = attributes.as_ref().clone();
        // An unchanged refresh should not churn the editor's decoding key.
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::TextAttributesLoaded {
                repo_id,
                target: DiffTarget::working_tree(path.clone(), DiffArea::Unstaged),
                result: Ok(attributes),
            }),
        );
        assert_eq!(state.repos[0].diff_state.text_attributes_rev, revision);
    }
}

#[test]
fn file_save_receipts_wait_for_execution_and_report_success_or_failure() {
    let directory = tempfile::tempdir().unwrap();
    let repo_id = RepoId(1);
    let repos = [(
        repo_id,
        Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
            directory.path(),
        )) as Arc<dyn GitRepository>,
    )]
    .into_iter()
    .collect();
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
    for (path, succeeds) in [("file.txt", true), ("../outside.txt", false)] {
        let (release, wait) = std::sync::mpsc::channel();
        // Saves run on the shared filesystem queue, not the repo executor.
        super::super::executor::filesystem_executor().spawn(move || {
            let _ = wait.recv();
        });
        let (completion, received) = smol::channel::bounded(1);
        let (msg_tx, msg_rx) = std::sync::mpsc::channel();
        schedule_effect_for_test(
            &executor,
            &executor,
            &backend,
            &repos,
            msg_tx,
            Effect::SaveWorktreeFile {
                repo_id,
                path: PathBuf::from(path),
                contents: "saved contents".to_string().into(),
                expected_contents: None,
                stage: false,
                completion: Some(completion),
            },
        );
        assert_eq!(received.try_recv(), Err(smol::channel::TryRecvError::Empty));
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                result, ..
            })) = recv_effect_message(&msg_rx, Duration::from_millis(50))
            {
                assert_eq!(result.is_ok(), succeeds);
                break;
            }
            assert!(Instant::now() < deadline, "save completion");
        }
        assert_eq!(received.try_recv(), Ok(succeeds));
        if succeeds {
            assert_eq!(
                std::fs::read_to_string(directory.path().join(path)).unwrap(),
                "saved contents"
            );
        }
    }
}

#[test]
fn session_update_effects_persist_on_session_executor() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            panic!("session persistence effects should not open repositories")
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let session_file = dir.path().join("session.json");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    let _session_file_override =
        crate::session::push_test_session_file_path_override(Some(session_file.clone()));

    let executor = super::super::executor::TaskExecutor::new(1);
    let session_executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &session_executor,
        &backend,
        &repos,
        msg_tx.clone(),
        Effect::PersistRecentRepo {
            repo_id: Some(RepoId(1)),
            workdir: repo_a.clone(),
            action: "test recent",
        },
    );
    schedule_effect_for_test(
        &executor,
        &session_executor,
        &backend,
        &repos,
        msg_tx.clone(),
        Effect::PersistRepoHistoryMode {
            repo_id: Some(RepoId(1)),
            workdir: repo_a.clone(),
            mode: LogScope::NoMerges,
            action: "test history mode",
        },
    );
    schedule_effect_for_test(
        &executor,
        &session_executor,
        &backend,
        &repos,
        msg_tx,
        Effect::PersistRepoHistoryModesBatch {
            repo_id: Some(RepoId(1)),
            updates: vec![(repo_b.clone(), LogScope::FirstParent)],
            action: "test history batch",
        },
    );

    let (completed_tx, completed_rx) = std::sync::mpsc::channel();
    session_executor.spawn(move || {
        completed_tx
            .send(())
            .expect("session persistence completion receiver should remain open");
    });
    completed_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("session persistence effects did not complete before timeout");

    let persistence_failures = msg_rx.try_iter().collect::<Vec<_>>();
    assert!(
        persistence_failures.is_empty(),
        "session persistence effects reported failures: {persistence_failures:?}"
    );

    let session = crate::session::load_from_path(&session_file);
    assert_eq!(session.recent_repos.first(), Some(&repo_a));
    assert_eq!(
        crate::session::load_repo_history_mode_from_path(&repo_a, &session_file),
        Some(LogScope::NoMerges)
    );
    assert_eq!(
        crate::session::load_repo_history_mode_from_path(&repo_b, &session_file),
        Some(LogScope::FirstParent)
    );
}

#[test]
fn safe_push_after_commit_effect_carries_auth_to_finished_message() {
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo_id = RepoId(3);
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    repos.insert(
        repo_id,
        Arc::new(UnsupportedRepo {
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            delete_branch_calls: None,
            cancel_delete_branch: None,
        }),
    );
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    let context = gitcomet_core::services::SafePushAfterCommitContext {
        amend: false,
        local_branch: Some("main".to_string()),
        pre_head: None,
        post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
    };
    let auth = gitcomet_core::auth::StagedGitAuth {
        kind: gitcomet_core::auth::GitAuthKind::UsernamePassword,
        username: Some("alice".to_string()),
        secret: "token".to_string(),
    };

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::SafePushAfterCommit {
            repo_id,
            context: context.clone(),
            auth: Some(auth.clone()),
        },
    );

    let msg = msg_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected safe-push completion message");
    match msg {
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id: emitted_repo_id,
            context: emitted_context,
            auth: emitted_auth,
            result,
        }) => {
            assert_eq!(emitted_repo_id, repo_id);
            assert_eq!(emitted_context, context);
            assert_eq!(emitted_auth, Some(auth));
            let err = result.expect_err("unsupported test repo should fail safe push");
            assert!(err.to_string().contains("safe push after commit"));
        }
        other => panic!("unexpected message: {other:?}"),
    }
}

#[test]
fn clone_repo_effect_clones_local_repo_and_emits_finished_and_open_repo() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let base = std::env::temp_dir().join(format!(
        "gitcomet-clone-effect-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let src = base.join("src");
    let dest = base.join("dest");
    let _ = std::fs::create_dir_all(&src);

    run_git(&src, &["init"]);
    run_git(&src, &["config", "user.email", "you@example.com"]);
    run_git(&src, &["config", "user.name", "You"]);
    run_git(&src, &["config", "commit.gpgsign", "false"]);
    std::fs::write(src.join("a.txt"), "one\n").unwrap();
    run_git(&src, &["add", "a.txt"]);
    run_git(
        &src,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CloneRepo {
            url: src.display().to_string(),
            dest: dest.clone(),
            remote_url_policy: Default::default(),
            auth: None,
        },
    );

    let start = Instant::now();
    let mut saw_finished_ok = false;
    let mut saw_open_repo = false;
    while start.elapsed() < Duration::from_secs(15) {
        let msg = match msg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(m) => m,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                dest: finished_dest,
                result,
                ..
            }) if finished_dest == dest => {
                assert!(result.is_ok(), "clone failed: {result:?}");
                saw_finished_ok = true;
            }
            Msg::OpenRepo(path) if path == dest => {
                saw_open_repo = true;
            }
            _ => {}
        }

        if saw_finished_ok && saw_open_repo {
            break;
        }
    }

    assert!(saw_finished_ok, "did not observe CloneRepoFinished");
    assert!(saw_open_repo, "did not observe OpenRepo after clone");
    assert!(dest.join(".git").exists(), "expected .git at cloned dest");
}

#[test]
fn save_worktree_file_effect_writes_and_can_stage() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        staged: std::sync::Mutex<Vec<PathBuf>>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, paths: &[&Path]) -> Result<()> {
            let mut staged = self.staged.lock().unwrap();
            for p in paths {
                staged.push(p.to_path_buf());
            }
            Ok(())
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    let base = std::env::temp_dir().join(format!(
        "gitcomet-save-worktree-file-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let rel = PathBuf::from("dir/out.txt");
    std::fs::create_dir(base.join("dir")).unwrap();
    let contents = "hello\nworld\n";

    let repo_id = RepoId(1);
    let repo: Arc<Repo> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: base.clone(),
        },
        staged: std::sync::Mutex::new(Vec::new()),
    });
    let repo_trait: Arc<dyn GitRepository> = repo.clone();
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo_trait);
        repos
    };

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx.clone(),
        Effect::SaveWorktreeFile {
            repo_id,
            path: rel.clone(),
            expected_contents: None,
            contents: contents.to_string().into(),
            stage: true,
            completion: None,
        },
    );

    let mut saw_write_and_stage = false;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = recv_effect_message(&msg_rx, Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: rid,
                command,
                result,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert!(matches!(
                command,
                crate::msg::RepoCommandKind::SaveWorktreeFile { .. }
            ));
            assert!(result.is_ok());
            let on_disk = std::fs::read_to_string(base.join(&rel)).unwrap();
            assert_eq!(on_disk, contents);
            let staged = repo.staged.lock().unwrap().clone();
            assert_eq!(staged, vec![rel.clone()]);
            saw_write_and_stage = true;
            break;
        };
    }
    assert!(
        saw_write_and_stage,
        "timed out waiting for RepoCommandFinished"
    );

    let escaped_name = format!(
        "gitcomet-save-worktree-file-escape-{}-{}.txt",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let escaped_path = PathBuf::from("..").join(&escaped_name);
    let escaped_dest = base
        .parent()
        .expect("temp dir should have a parent")
        .join(&escaped_name);
    let _ = std::fs::remove_file(&escaped_dest);

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::SaveWorktreeFile {
            repo_id,
            path: escaped_path,
            expected_contents: None,
            contents: "escape".to_string().into(),
            stage: false,
            completion: None,
        },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = recv_effect_message(&msg_rx, Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: rid,
                command,
                result,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert!(matches!(
                command,
                crate::msg::RepoCommandKind::SaveWorktreeFile { .. }
            ));
            let err = result.expect_err("expected traversal write to fail");
            match err.kind() {
                ErrorKind::Backend(message) => {
                    assert!(
                        message.contains("outside repository workdir"),
                        "unexpected error message: {message}"
                    );
                }
                other => panic!("unexpected error kind: {other:?}"),
            }
            assert!(
                !escaped_dest.exists(),
                "unexpected file written outside workdir: {}",
                escaped_dest.display()
            );
            return;
        };
    }
    panic!("timed out waiting for RepoCommandFinished");
}

#[test]
fn append_gitignore_patterns_effect_creates_appends_and_dedupes() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    // `tempfile` rather than a hand-rolled directory: its `Drop` runs on unwind,
    // so a failing assertion below does not leave a stray repo in /tmp.
    let dir = tempfile::tempdir().expect("create tempdir");
    let base = dir.path().to_path_buf();
    let gitignore = base.join(".gitignore");

    let repo_id = RepoId(1);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let repo: Arc<dyn GitRepository> = Arc::new(Repo {
            spec: RepoSpec {
                workdir: base.clone(),
            },
        });
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);

    // Runs one append to completion and hands back the command's stdout, which
    // is how the worker reports the "nothing to add" short-circuit.
    let append = |patterns: Vec<String>| -> String {
        let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
        schedule_effect_for_test(
            &executor,
            &executor,
            &backend,
            &repos,
            msg_tx,
            Effect::AppendGitignorePatterns { repo_id, patterns },
        );
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if let Ok(msg) = recv_effect_message(&msg_rx, Duration::from_millis(50))
                && let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                    command,
                    result,
                    ..
                }) = msg
            {
                assert!(matches!(
                    command,
                    crate::msg::RepoCommandKind::AppendGitignorePatterns { .. }
                ));
                return result.expect("append should succeed").stdout;
            }
        }
        panic!("timed out waiting for RepoCommandFinished");
    };

    append(vec!["/a.log".to_string()]);
    assert_eq!(
        std::fs::read_to_string(&gitignore).unwrap(),
        "/a.log\n",
        "the file is created when absent"
    );

    append(vec!["/a.log".to_string(), "/b.log".to_string()]);
    assert_eq!(
        std::fs::read_to_string(&gitignore).unwrap(),
        "/a.log\n/b.log\n",
        "the already-present pattern is skipped, the new one appended"
    );

    let before = std::fs::read_to_string(&gitignore).unwrap();
    let stdout = append(vec!["/a.log".to_string()]);
    assert_eq!(
        std::fs::read_to_string(&gitignore).unwrap(),
        before,
        "re-running must not duplicate the line"
    );
    assert_eq!(
        stdout.trim(),
        gitcomet_core::gitignore::NOTHING_TO_ADD,
        "a fully redundant append must short-circuit before the write so it does \
         not bump the mtime and rebuild the filesystem watcher for nothing — and \
         it must say so with the marker `summarize_command` keys off, or the user \
         is told a write happened"
    );

    std::fs::write(&gitignore, "/target").unwrap();
    append(vec!["/c.log".to_string()]);
    assert_eq!(
        std::fs::read_to_string(&gitignore).unwrap(),
        "/target\n/c.log\n",
        "an unterminated last line must not fuse with the new pattern"
    );

    std::fs::write(&gitignore, "/target\r\n").unwrap();
    append(vec!["/d.log".to_string()]);
    assert_eq!(
        std::fs::read_to_string(&gitignore).unwrap(),
        "/target\r\n/d.log\r\n",
        "a CRLF file stays CRLF"
    );
}

#[test]
fn checkout_conflict_base_effect_calls_repo_and_emits_finished() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        checkout_base_calls: std::sync::Mutex<Vec<PathBuf>>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }

        fn checkout_conflict_base(&self, path: &Path) -> Result<CommandOutput> {
            self.checkout_base_calls
                .lock()
                .unwrap()
                .push(path.to_path_buf());
            Ok(CommandOutput::empty_success(format!(
                "git checkout :1:{}",
                path.display()
            )))
        }
    }

    let repo_id = RepoId(1);
    let rel = PathBuf::from("conflicted.txt");
    let repo: Arc<Repo> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: std::env::temp_dir(),
        },
        checkout_base_calls: std::sync::Mutex::new(Vec::new()),
    });
    let repo_trait: Arc<dyn GitRepository> = repo.clone();
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo_trait);
        repos
    };

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CheckoutConflictBase {
            repo_id,
            path: rel.clone(),
        },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = recv_effect_message(&msg_rx, Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: rid,
                command,
                result,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert!(matches!(
                command,
                crate::msg::RepoCommandKind::CheckoutConflictBase { path } if path == rel
            ));
            assert!(result.is_ok());
            assert_eq!(repo.checkout_base_calls.lock().unwrap().as_slice(), [rel]);
            return;
        };
    }
    panic!("timed out waiting for RepoCommandFinished");
}

#[test]
fn accept_conflict_deletion_effect_calls_repo_and_emits_finished() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        accepted_deletion_calls: std::sync::Mutex<Vec<PathBuf>>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }

        fn accept_conflict_deletion(&self, path: &Path) -> Result<CommandOutput> {
            self.accepted_deletion_calls
                .lock()
                .unwrap()
                .push(path.to_path_buf());
            Ok(CommandOutput::empty_success(format!(
                "git rm -- {}",
                path.display()
            )))
        }
    }

    let repo_id = RepoId(1);
    let rel = PathBuf::from("conflicted.txt");
    let repo: Arc<Repo> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: std::env::temp_dir(),
        },
        accepted_deletion_calls: std::sync::Mutex::new(Vec::new()),
    });
    let repo_trait: Arc<dyn GitRepository> = repo.clone();
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo_trait);
        repos
    };

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::AcceptConflictDeletion {
            repo_id,
            path: rel.clone(),
        },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(msg) = recv_effect_message(&msg_rx, Duration::from_millis(50))
            && let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id: rid,
                command,
                result,
            }) = msg
        {
            assert_eq!(rid, repo_id);
            assert!(matches!(
                command,
                crate::msg::RepoCommandKind::AcceptConflictDeletion { path } if path == rel
            ));
            assert!(result.is_ok());
            assert_eq!(
                repo.accepted_deletion_calls.lock().unwrap().as_slice(),
                [rel]
            );
            return;
        };
    }
    panic!("timed out waiting for RepoCommandFinished");
}

#[test]
fn load_stashes_effect_truncates_results_to_limit() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    struct Repo {
        spec: RepoSpec,
        stashes: Vec<StashEntry>,
    }

    impl GitRepository for Repo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            Ok(self.stashes.clone())
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    let base = std::env::temp_dir().join(format!(
        "gitcomet-stash-load-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&base);

    let stashes = (0..5)
        .map(|i| StashEntry {
            index: i,
            id: CommitId(format!("stash-{i}").into()),
            message: format!("stash message {i}").into(),
            created_at: None,
        })
        .collect::<Vec<_>>();

    let repo_id = RepoId(1);
    let repo: Arc<dyn GitRepository> = Arc::new(Repo {
        spec: RepoSpec {
            workdir: base.clone(),
        },
        stashes,
    });

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadStashes { repo_id, limit: 2 },
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match recv_effect_message(&msg_rx, Duration::from_millis(100)) {
            Ok(m) => m,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::Internal(crate::msg::InternalMsg::StashesLoaded {
                repo_id: got_repo_id,
                result,
            }) if got_repo_id == repo_id => {
                let entries = result.expect("expected stash list Ok");
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].index, 0);
                assert_eq!(entries[1].index, 1);
                return;
            }
            _ => {}
        }
    }

    panic!("did not observe StashesLoaded");
}

#[test]
fn stash_effect_requests_stash_reload_on_success() {
    use std::sync::Mutex;

    struct RecordingRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitRepository for RecordingRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }

        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, message: &str, include_untracked: bool) -> Result<()> {
            self.calls.lock().unwrap().push(format!(
                "stash {message} include_untracked={include_untracked}"
            ));
            Ok(())
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }

        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<RecordingRepo> = Arc::new(RecordingRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
        calls: Arc::clone(&calls),
    });

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    repos.insert(RepoId(1), repo);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::Stash {
            repo_id: RepoId(1),
            message: "wip".to_string(),
            include_untracked: true,
        },
    );

    let start = Instant::now();
    let mut saw_load_stashes = false;
    let mut saw_finished = false;
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match recv_effect_message(&msg_rx, Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::LoadStashes { repo_id: RepoId(1) } => saw_load_stashes = true,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                action: RepoActionKind::Stash,
                result: Ok(()),
            }) => saw_finished = true,
            _ => {}
        }

        if saw_load_stashes && saw_finished {
            break;
        }
    }

    assert!(
        saw_load_stashes,
        "expected stash effect to request stash reload"
    );
    assert!(saw_finished, "expected stash effect to complete");
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["stash wip include_untracked=true".to_string()]
    );
}

#[test]
fn pop_stash_effect_applies_and_drops_then_requests_stash_reload() {
    use std::sync::Mutex;

    struct RecordingRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitRepository for RecordingRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }

        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("apply {index}"));
            Ok(())
        }
        fn stash_drop(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("drop {index}"));
            Ok(())
        }

        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<RecordingRepo> = Arc::new(RecordingRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
        calls: Arc::clone(&calls),
    });

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    repos.insert(RepoId(1), repo);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::PopStash {
            repo_id: RepoId(1),
            index: 3,
        },
    );

    let start = Instant::now();
    let mut saw_load_stashes = false;
    let mut saw_finished = false;
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match recv_effect_message(&msg_rx, Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::LoadStashes { repo_id: RepoId(1) } => saw_load_stashes = true,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                action: RepoActionKind::PopStash,
                result: Ok(()),
            }) => saw_finished = true,
            _ => {}
        }

        if saw_load_stashes && saw_finished {
            break;
        }
    }

    assert!(
        saw_load_stashes,
        "expected pop stash effect to request stash reload"
    );
    assert!(saw_finished, "expected pop stash effect to complete");
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["apply 3".to_string(), "drop 3".to_string()]
    );
}

#[test]
fn pop_stash_effect_propagates_apply_error_without_drop_or_reload() {
    use std::sync::Mutex;

    struct FailingApplyRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitRepository for FailingApplyRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }

        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("apply {index}"));
            Err(Error::new(ErrorKind::Backend("apply failed".to_string())))
        }
        fn stash_drop(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("drop {index}"));
            Ok(())
        }

        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<FailingApplyRepo> = Arc::new(FailingApplyRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
        calls: Arc::clone(&calls),
    });

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    repos.insert(RepoId(1), repo);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::PopStash {
            repo_id: RepoId(1),
            index: 7,
        },
    );

    let start = Instant::now();
    let mut saw_load_stashes = false;
    let mut saw_finished_err = false;
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match recv_effect_message(&msg_rx, Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::LoadStashes { repo_id: RepoId(1) } => saw_load_stashes = true,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                action: RepoActionKind::PopStash,
                result: Err(_),
            }) => {
                saw_finished_err = true;
                break;
            }
            _ => {}
        }
    }

    assert!(
        !saw_load_stashes,
        "pop stash apply failure should not request stash reload"
    );
    assert!(
        saw_finished_err,
        "expected pop stash effect to emit apply error completion"
    );
    assert_eq!(*calls.lock().unwrap(), vec!["apply 7".to_string()]);
}

#[test]
fn drop_stash_effect_requests_stash_reload_on_success() {
    use std::sync::Mutex;

    struct RecordingRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitRepository for RecordingRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }

        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("drop {index}"));
            Ok(())
        }

        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<RecordingRepo> = Arc::new(RecordingRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
        calls: Arc::clone(&calls),
    });

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    repos.insert(RepoId(1), repo);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::DropStash {
            repo_id: RepoId(1),
            index: 3,
        },
    );

    let start = Instant::now();
    let mut saw_load_stashes = false;
    let mut saw_finished = false;
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match recv_effect_message(&msg_rx, Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::LoadStashes { repo_id: RepoId(1) } => saw_load_stashes = true,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                action: RepoActionKind::DropStash,
                result: Ok(()),
            }) => saw_finished = true,
            _ => {}
        }

        if saw_load_stashes && saw_finished {
            break;
        }
    }

    assert!(
        saw_load_stashes,
        "expected drop stash effect to request stash reload"
    );
    assert!(saw_finished, "expected drop stash effect to complete");
    assert_eq!(*calls.lock().unwrap(), vec!["drop 3".to_string()]);
}

#[test]
fn drop_stash_effect_requests_stash_reload_on_error() {
    use std::sync::Mutex;

    struct FailingRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl GitRepository for FailingRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }

        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, index: usize) -> Result<()> {
            self.calls.lock().unwrap().push(format!("drop {index}"));
            Err(Error::new(ErrorKind::Backend("drop failed".to_string())))
        }

        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            unimplemented!()
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<FailingRepo> = Arc::new(FailingRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
        calls: Arc::clone(&calls),
    });

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    repos.insert(RepoId(1), repo);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::DropStash {
            repo_id: RepoId(1),
            index: 4,
        },
    );

    let start = Instant::now();
    let mut saw_load_stashes = false;
    let mut saw_finished_err = false;
    while start.elapsed() < Duration::from_secs(5) {
        let msg = match recv_effect_message(&msg_rx, Duration::from_millis(100)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::LoadStashes { repo_id: RepoId(1) } => saw_load_stashes = true,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                action: RepoActionKind::DropStash,
                result: Err(_),
            }) => {
                saw_finished_err = true;
                break;
            }
            _ => {}
        }
    }

    assert!(
        saw_load_stashes,
        "drop stash failure should still request stash reload"
    );
    assert!(
        saw_finished_err,
        "expected drop stash effect to emit error completion"
    );
    assert_eq!(*calls.lock().unwrap(), vec!["drop 4".to_string()]);
}

/// A repo on `main` whose `feature` commit edits `a.txt`; returns that commit.
fn apply_file_change_fixture(repo: &Path) -> String {
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "user.name", "Test User"]);
    run_git(repo, &["config", "user.email", "test@example.com"]);
    fs::write(repo.join("a.txt"), "one\ntwo\n").expect("write base");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["commit", "-q", "-m", "base"]);
    run_git(repo, &["checkout", "-q", "-b", "feature"]);
    fs::write(repo.join("a.txt"), "one\nTWO\n").expect("write feature");
    run_git(repo, &["commit", "-q", "-am", "feature subject\n\nbody"]);
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse");
    run_git(repo, &["checkout", "-q", "main"]);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// Runs one `ApplyFileChange` effect and returns every message it sent, in
/// order, ending with its `RepoCommandFinished` unwrapped from the operation.
fn run_apply_file_change_effect(
    repo: &Path,
    target: gitcomet_core::domain::ApplyChangeTarget,
    commit: bool,
) -> Vec<Msg> {
    run_apply_file_change_effect_with_retry(repo, target, commit, None)
}

fn run_apply_file_change_effect_with_retry(
    repo: &Path,
    target: gitcomet_core::domain::ApplyChangeTarget,
    commit: bool,
    commit_retry: Option<gitcomet_core::domain::ApplyFileChangeRetry>,
) -> Vec<Msg> {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            panic!("open should not be called in this test")
        }
    }
    let repo_id = RepoId(9);
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    repos.insert(
        repo_id,
        gitcomet_git_gix::GixBackend
            .open(repo)
            .expect("open repository through backend"),
    );
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::ApplyFileChange {
            commit_retry,
            repo_id,
            target,
            commit,
            auth: None,
        },
    );
    let mut msgs = Vec::new();
    loop {
        match recv_effect_message(&msg_rx, Duration::from_secs(20)) {
            Ok(msg) => msgs.push(msg),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return msgs,
            Err(error) => panic!("apply file change did not finish: {error}"),
        }
    }
}

fn suggested_messages(msgs: &[Msg]) -> Vec<&str> {
    msgs.iter()
        .filter_map(|msg| match msg {
            Msg::Internal(crate::msg::InternalMsg::CommitMessageSuggested { message, .. }) => {
                Some(message.as_str())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn apply_file_change_offers_the_source_message_only_while_uncommitted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let picked = apply_file_change_fixture(dir.path());
    let target = gitcomet_core::domain::ApplyChangeTarget::commit(
        CommitId(picked.as_str().into()),
        PathBuf::from("a.txt"),
    );

    let staged = run_apply_file_change_effect(dir.path(), target.clone(), false);
    assert_eq!(suggested_messages(&staged), ["feature subject\n\nbody"]);
    assert!(matches!(
        staged.last(),
        Some(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished { result: Ok(_), .. }
        ))
    ));

    // Committing needs no message from the user.
    run_git(dir.path(), &["reset", "--hard", "HEAD"]);
    let committed = run_apply_file_change_effect(dir.path(), target, true);
    assert!(suggested_messages(&committed).is_empty(), "{committed:?}");
    assert!(matches!(
        committed.last(),
        Some(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished { result: Ok(_), .. }
        ))
    ));
}

#[test]
fn a_conflicted_apply_file_change_still_offers_the_message() {
    let dir = tempfile::tempdir().expect("tempdir");
    let picked = apply_file_change_fixture(dir.path());
    fs::write(dir.path().join("a.txt"), "one\nMAIN\n").expect("write main edit");
    run_git(dir.path(), &["commit", "-q", "-am", "main edit"]);
    let target = gitcomet_core::domain::ApplyChangeTarget::commit(
        CommitId(picked.as_str().into()),
        PathBuf::from("a.txt"),
    );

    let msgs = run_apply_file_change_effect(dir.path(), target, true);

    assert_eq!(suggested_messages(&msgs), ["feature subject\n\nbody"]);
    assert!(matches!(
        msgs.last(),
        Some(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished { result: Err(_), .. }
        ))
    ));
}

#[test]
fn an_already_applied_change_offers_no_message() {
    let dir = tempfile::tempdir().expect("tempdir");
    let picked = apply_file_change_fixture(dir.path());
    fs::write(dir.path().join("a.txt"), "one\nTWO\n").expect("write same edit");
    run_git(dir.path(), &["commit", "-q", "-am", "same edit"]);
    let target = gitcomet_core::domain::ApplyChangeTarget::commit(
        CommitId(picked.as_str().into()),
        PathBuf::from("a.txt"),
    );

    let msgs = run_apply_file_change_effect(dir.path(), target, false);

    assert!(suggested_messages(&msgs).is_empty(), "{msgs:?}");
}

/// The change stays staged when committing it fails, so the commit box gets
/// the message the user now has to commit with.
#[cfg(unix)]
#[test]
fn a_failed_commit_step_offers_the_message() {
    let dir = tempfile::tempdir().expect("tempdir");
    let picked = apply_file_change_fixture(dir.path());
    run_git(dir.path(), &["config", "commit.gpgsign", "true"]);
    run_git(dir.path(), &["config", "gpg.program", "false"]);
    let target = gitcomet_core::domain::ApplyChangeTarget::commit(
        CommitId(picked.as_str().into()),
        PathBuf::from("a.txt"),
    );

    let msgs = run_apply_file_change_effect(dir.path(), target.clone(), true);

    assert_eq!(suggested_messages(&msgs), ["feature subject\n\nbody"]);
    let Some(Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
        command:
            RepoCommandKind::ApplyFileChange {
                commit_retry: Some(retry),
                ..
            },
        result: Err(_),
        ..
    })) = msgs.last()
    else {
        panic!("failed commit must carry its checkpoint: {msgs:?}")
    };

    run_git(dir.path(), &["config", "commit.gpgsign", "false"]);
    let committed =
        run_apply_file_change_effect_with_retry(dir.path(), target, true, Some(retry.clone()));
    assert!(suggested_messages(&committed).is_empty());
    assert!(committed.iter().any(|msg| matches!(msg,
        Msg::Internal(crate::msg::InternalMsg::CommitMessageSuggestionConsumed { message, .. })
            if message == "feature subject\n\nbody"
    )));
    assert!(matches!(
        committed.last(),
        Some(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished { result: Ok(_), .. }
        ))
    ));
}

#[test]
fn push_lifecycle_uses_cached_tracking_branch_context() {
    let repo_id = RepoId(340);
    let spec = RepoSpec {
        workdir: unique_temp_path("gitcomet-push-hook-context"),
    };
    let repo: Arc<dyn GitRepository> = Arc::new(UnsupportedRepo {
        spec: spec.clone(),
        delete_branch_calls: None,
        cancel_delete_branch: None,
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let mut repo_state = RepoState::new_opening(repo_id, spec);
    repo_state.set_head_branch(Loadable::Ready("main".to_string()));
    repo_state.set_branches(Loadable::Ready(vec![Branch {
        name: "main".to_string(),
        target: CommitId("1111111111111111111111111111111111111111".into()),
        upstream: Some(gitcomet_core::domain::Upstream {
            remote: "origin".to_string(),
            branch: "main".to_string(),
        }),
        divergence: None,
    }]));

    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    schedule_effect_with_state_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        AppState {
            repos: vec![repo_state],
            ..AppState::test_default()
        },
        msg_tx,
        Effect::Push {
            repo_id,
            auth: None,
        },
    );

    match msg_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("push should start its hook activity lifecycle")
    {
        Msg::Internal(crate::msg::InternalMsg::GitOperationStarted {
            repo_id: started_repo_id,
            label,
            context,
            ..
        }) => {
            assert_eq!(started_repo_id, repo_id);
            assert_eq!(label, "Push");
            assert_eq!(context.as_deref(), Some("main → origin/main"));
        }
        other => panic!("unexpected first push lifecycle message: {other:?}"),
    }
    assert!(
        matches!(
            msg_rx.recv_timeout(Duration::from_secs(2)),
            Ok(Msg::Internal(
                crate::msg::InternalMsg::GitOperationFinished { .. }
            ))
        ),
        "push should finish its hook activity lifecycle"
    );
}

fn wait_for_checkout_refresh_messages(
    msg_rx: &std::sync::mpsc::Receiver<Msg>,
    repo_id: RepoId,
    expect_refresh_branches: bool,
    expect_load_worktrees: bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_refresh_branches = false;
    let mut saw_load_worktrees = false;
    let mut saw_finished = false;

    while Instant::now() < deadline {
        let msg = match recv_effect_message(msg_rx, Duration::from_millis(50)) {
            Ok(msg) => msg,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(err) => panic!("channel closed: {err:?}"),
        };

        match msg {
            Msg::RefreshBranches { repo_id: rid } if rid == repo_id => {
                saw_refresh_branches = true;
            }
            Msg::LoadWorktrees { repo_id: rid } if rid == repo_id => {
                saw_load_worktrees = true;
            }
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: rid,
                action: _,
                result: Ok(()),
            }) if rid == repo_id => {
                saw_finished = true;
            }
            _ => {}
        }

        if saw_finished
            && saw_refresh_branches == expect_refresh_branches
            && saw_load_worktrees == expect_load_worktrees
        {
            return;
        }
    }

    assert_eq!(
        saw_refresh_branches, expect_refresh_branches,
        "unexpected RefreshBranches emission for repo {repo_id:?}"
    );
    assert_eq!(
        saw_load_worktrees, expect_load_worktrees,
        "unexpected LoadWorktrees emission for repo {repo_id:?}"
    );
    assert!(
        saw_finished,
        "expected RepoActionFinished for repo {repo_id:?}"
    );
}

#[test]
fn checkout_branch_effect_requests_branch_and_worktree_reload_on_success() {
    let repo_id = RepoId(700);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingCheckoutRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-checkout-branch-effect"),
        },
        calls: Arc::clone(&calls),
        create_branch_already_exists: false,
        rename_branch_already_exists: false,
        other_worktree: None,
        current_branch: None,
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CheckoutBranch {
            repo_id,
            name: "feature".to_string(),
        },
    );

    wait_for_checkout_refresh_messages(&msg_rx, repo_id, true, true);
    assert_eq!(
        *calls.lock().expect("checkout recording mutex"),
        vec!["checkout feature".to_string()]
    );
}

#[test]
fn checkout_remote_branch_effect_requests_branch_and_worktree_reload_on_success() {
    let repo_id = RepoId(701);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingCheckoutRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-checkout-remote-branch-effect"),
        },
        calls: Arc::clone(&calls),
        create_branch_already_exists: false,
        rename_branch_already_exists: false,
        other_worktree: None,
        current_branch: None,
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CheckoutRemoteBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature".to_string(),
            local_branch: "feature".to_string(),
            mode: gitcomet_core::services::CheckoutRemoteBranchMode::Overwrite,
        },
    );

    wait_for_checkout_refresh_messages(&msg_rx, repo_id, true, true);
    assert_eq!(
        *calls.lock().expect("checkout recording mutex"),
        vec!["checkout_remote origin/feature -> feature (Overwrite)".to_string()]
    );
}

#[test]
fn checkout_commit_effect_requests_worktree_reload_on_success() {
    let repo_id = RepoId(702);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingCheckoutRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-checkout-commit-effect"),
        },
        calls: Arc::clone(&calls),
        create_branch_already_exists: false,
        rename_branch_already_exists: false,
        other_worktree: None,
        current_branch: None,
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    let commit_id = CommitId("deadbeef".into());

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CheckoutCommit {
            repo_id,
            commit_id: commit_id.clone(),
        },
    );

    wait_for_checkout_refresh_messages(&msg_rx, repo_id, false, true);
    assert_eq!(
        *calls.lock().expect("checkout recording mutex"),
        vec![format!("checkout_commit {}", commit_id.as_ref())]
    );
}

#[test]
fn create_branch_and_checkout_effect_requests_branch_and_worktree_reload_on_success() {
    let repo_id = RepoId(703);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingCheckoutRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-create-branch-and-checkout-effect"),
        },
        calls: Arc::clone(&calls),
        create_branch_already_exists: false,
        rename_branch_already_exists: false,
        other_worktree: None,
        current_branch: None,
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CreateBranchAndCheckout {
            repo_id,
            name: "feature".to_string(),
            target: "HEAD".to_string(),
            force: false,
        },
    );

    wait_for_checkout_refresh_messages(&msg_rx, repo_id, true, true);
    assert_eq!(
        *calls.lock().expect("checkout recording mutex"),
        vec![
            "create feature HEAD".to_string(),
            "checkout feature".to_string()
        ]
    );
}

#[test]
fn create_branch_and_checkout_force_effect_skips_separate_create_and_checkout() {
    let repo_id = RepoId(704);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingCheckoutRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-create-branch-and-checkout-force-effect"),
        },
        calls: Arc::clone(&calls),
        create_branch_already_exists: false,
        rename_branch_already_exists: false,
        other_worktree: None,
        current_branch: None,
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CreateBranchAndCheckout {
            repo_id,
            name: "feature".to_string(),
            target: "HEAD".to_string(),
            force: true,
        },
    );

    wait_for_checkout_refresh_messages(&msg_rx, repo_id, true, true);
    assert_eq!(
        *calls.lock().expect("checkout recording mutex"),
        vec!["force-create-and-checkout feature HEAD".to_string()]
    );
}

#[test]
fn create_branch_and_checkout_effect_routes_collision_with_original_target() {
    let repo_id = RepoId(705);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repo: Arc<dyn GitRepository> = Arc::new(RecordingCheckoutRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-create-branch-collision-effect"),
        },
        calls: Arc::clone(&calls),
        create_branch_already_exists: true,
        rename_branch_already_exists: false,
        other_worktree: None,
        current_branch: None,
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::CreateBranchAndCheckout {
            repo_id,
            name: "feature".to_string(),
            target: "origin/feature-one".to_string(),
            force: false,
        },
    );

    let first =
        recv_effect_message(&msg_rx, Duration::from_secs(2)).expect("expected collision refresh");
    assert!(matches!(first, Msg::RefreshBranches { repo_id: id } if id == repo_id));
    let second =
        recv_effect_message(&msg_rx, Duration::from_secs(2)).expect("expected semantic collision");
    assert!(matches!(
        second,
        Msg::Internal(crate::msg::InternalMsg::BranchAlreadyExists {
            action: RepoActionKind::CreateBranchAndCheckout,
            prompt: crate::model::BranchExistsPromptState {
                repo_id: id,
                name,
                target,
                operation: crate::model::BranchExistsPromptOperation::CreateBranch,
            },
        }) if id == repo_id && name == "feature" && target == "origin/feature-one"
    ));
    assert!(
        msg_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "collision must not emit checkout, worktree reload, or generic failure messages"
    );
    assert_eq!(
        *calls.lock().expect("checkout recording mutex"),
        vec!["create feature origin/feature-one".to_string()]
    );
}

fn assert_refreshes_branches_and_worktrees(others: &[Msg], repo_id: RepoId) {
    assert_eq!(others.len(), 3, "unexpected messages: {others:?}");
    assert!(
        others
            .iter()
            .any(|msg| matches!(msg, Msg::RefreshBranches { repo_id: id } if *id == repo_id))
    );
    assert!(
        others
            .iter()
            .any(|msg| matches!(msg, Msg::LoadWorktrees { repo_id: id } if *id == repo_id))
    );
    assert!(
        others
            .iter()
            .any(|msg| matches!(msg, Msg::LoadWorktreeDirty { repo_id: id } if *id == repo_id))
    );
}

#[test]
fn checkout_branch_effect_reports_other_worktree_without_running_checkout() {
    let repo_id = RepoId(710);
    let fixture =
        worktree_redirect_fixture(repo_id, "gitcomet-checkout-redirect", "feature", false);
    let msg_rx = run_effect_with_fixture(
        &fixture,
        Effect::CheckoutBranch {
            repo_id,
            name: "feature".to_string(),
        },
    );

    let (others, finished) = recv_until_action_finished(&msg_rx);
    assert!(
        others.is_empty(),
        "a redirected checkout runs nothing and refreshes nothing: {others:?}"
    );
    assert!(matches!(
        &finished,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinishedInWorktree {
            repo_id: id,
            action: RepoActionKind::CheckoutBranch,
            worktree_path,
            result: Ok(()),
        }) if *id == repo_id && worktree_path == &fixture.worktree
    ));
    assert!(fixture.origin_calls.lock().unwrap().is_empty());
    assert!(fixture.opened.lock().unwrap().is_empty());
}

#[test]
fn create_branch_and_checkout_force_effect_runs_in_other_worktree() {
    let repo_id = RepoId(711);
    let fixture =
        worktree_redirect_fixture(repo_id, "gitcomet-force-create-redirect", "feature", false);
    let msg_rx = run_effect_with_fixture(
        &fixture,
        Effect::CreateBranchAndCheckout {
            repo_id,
            name: "feature".to_string(),
            target: "origin/feature-one".to_string(),
            force: true,
        },
    );

    let (others, finished) = recv_until_action_finished(&msg_rx);
    assert_refreshes_branches_and_worktrees(&others, repo_id);
    assert!(matches!(
        &finished,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinishedInWorktree {
            repo_id: id,
            action: RepoActionKind::CreateBranchAndCheckout,
            worktree_path,
            result: Ok(()),
        }) if *id == repo_id && worktree_path == &fixture.worktree
    ));
    assert!(fixture.origin_calls.lock().unwrap().is_empty());
    assert_eq!(
        *fixture.worktree_calls.lock().unwrap(),
        vec!["force-create-and-checkout feature origin/feature-one".to_string()]
    );
    assert_eq!(
        *fixture.opened.lock().unwrap(),
        vec![fixture.worktree.clone()]
    );
}

#[test]
fn checkout_remote_branch_overwrite_effect_runs_in_other_worktree() {
    let repo_id = RepoId(712);
    let fixture = worktree_redirect_fixture(
        repo_id,
        "gitcomet-remote-overwrite-redirect",
        "feature",
        false,
    );
    let msg_rx = run_effect_with_fixture(
        &fixture,
        Effect::CheckoutRemoteBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature".to_string(),
            local_branch: "feature".to_string(),
            mode: gitcomet_core::services::CheckoutRemoteBranchMode::Overwrite,
        },
    );

    let (others, finished) = recv_until_action_finished(&msg_rx);
    assert_refreshes_branches_and_worktrees(&others, repo_id);
    assert!(matches!(
        &finished,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinishedInWorktree {
            repo_id: id,
            action: RepoActionKind::CheckoutRemoteBranch,
            worktree_path,
            result: Ok(()),
        }) if *id == repo_id && worktree_path == &fixture.worktree
    ));
    assert!(fixture.origin_calls.lock().unwrap().is_empty());
    assert_eq!(
        *fixture.worktree_calls.lock().unwrap(),
        vec!["checkout_remote origin/feature -> feature (Overwrite)".to_string()]
    );
}

#[test]
fn checkout_remote_branch_create_effect_runs_here_even_when_branch_is_elsewhere() {
    let repo_id = RepoId(713);
    let fixture =
        worktree_redirect_fixture(repo_id, "gitcomet-remote-create-here", "feature", false);
    let msg_rx = run_effect_with_fixture(
        &fixture,
        Effect::CheckoutRemoteBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature".to_string(),
            local_branch: "feature".to_string(),
            mode: gitcomet_core::services::CheckoutRemoteBranchMode::Create,
        },
    );

    let (others, finished) = recv_until_action_finished(&msg_rx);
    assert_refreshes_branches_and_worktrees(&others, repo_id);
    assert!(matches!(
        &finished,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id: id,
            action: RepoActionKind::CheckoutRemoteBranch,
            result: Ok(()),
        }) if *id == repo_id
    ));
    assert_eq!(
        *fixture.origin_calls.lock().unwrap(),
        vec!["checkout_remote origin/feature -> feature (Create)".to_string()]
    );
    assert!(fixture.opened.lock().unwrap().is_empty());
}

#[test]
fn rename_branch_effect_routes_collision_to_prompt() {
    let repo_id = RepoId(714);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut repo = RecordingCheckoutRepo::new(
        RepoSpec {
            workdir: unique_temp_path("gitcomet-rename-collision-effect"),
        },
        Arc::clone(&calls),
    );
    repo.rename_branch_already_exists = true;
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, Arc::new(repo) as Arc<dyn GitRepository>);
        repos
    };
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::RenameBranch {
            repo_id,
            old_name: "old".to_string(),
            new_name: "feature".to_string(),
            force: false,
        },
    );

    let (others, finished) = recv_until_action_finished(&msg_rx);
    assert!(matches!(
        others.as_slice(),
        [Msg::RefreshBranches { repo_id: id }] if *id == repo_id
    ));
    assert!(matches!(
        &finished,
        Msg::Internal(crate::msg::InternalMsg::BranchAlreadyExists {
            action: RepoActionKind::RenameBranch,
            prompt: crate::model::BranchExistsPromptState {
                repo_id: id,
                name,
                target,
                operation: crate::model::BranchExistsPromptOperation::RenameBranch { old_name },
            },
        }) if *id == repo_id && name == "feature" && target == "old" && old_name == "old"
    ));
    assert!(
        msg_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "a collision must not emit worktree reloads or a generic failure"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["rename old feature".to_string()]
    );
}

#[test]
fn rename_branch_effect_finishes_normally_without_collision() {
    let repo_id = RepoId(715);
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let repo = RecordingCheckoutRepo::new(
        RepoSpec {
            workdir: unique_temp_path("gitcomet-rename-effect"),
        },
        Arc::clone(&calls),
    );
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, Arc::new(repo) as Arc<dyn GitRepository>);
        repos
    };
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::RenameBranch {
            repo_id,
            old_name: "old".to_string(),
            new_name: "feature".to_string(),
            force: false,
        },
    );

    let (others, finished) = recv_until_action_finished(&msg_rx);
    assert_refreshes_branches_and_worktrees(&others, repo_id);
    assert!(matches!(
        &finished,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id: id,
            action: RepoActionKind::RenameBranch,
            result: Ok(()),
        }) if *id == repo_id
    ));
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["rename old feature".to_string()]
    );
}

#[test]
fn rename_branch_force_effect_runs_in_other_worktree() {
    let repo_id = RepoId(716);
    let fixture =
        worktree_redirect_fixture(repo_id, "gitcomet-rename-force-redirect", "feature", false);
    let msg_rx = run_effect_with_fixture(
        &fixture,
        Effect::RenameBranch {
            repo_id,
            old_name: "old".to_string(),
            new_name: "feature".to_string(),
            force: true,
        },
    );

    let (others, finished) = recv_until_action_finished(&msg_rx);
    assert_refreshes_branches_and_worktrees(&others, repo_id);
    assert!(matches!(
        &finished,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinishedInWorktree {
            repo_id: id,
            action: RepoActionKind::RenameBranch,
            worktree_path,
            result: Ok(()),
        }) if *id == repo_id && worktree_path == &fixture.worktree
    ));
    assert!(fixture.origin_calls.lock().unwrap().is_empty());
    assert_eq!(
        *fixture.worktree_calls.lock().unwrap(),
        vec!["rename-force old feature".to_string()]
    );
}

#[test]
fn pull_releases_the_object_store_before_its_refresh() {
    use std::sync::Mutex;

    struct RecordingRepo {
        spec: RepoSpec,
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    impl GitRepository for RecordingRepo {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }
        fn release_object_store(&self) {
            self.calls.lock().unwrap().push("release");
        }
        fn log_head_page(
            &self,
            _limit: usize,
            _cursor: Option<&LogCursor>,
        ) -> Result<std::sync::Arc<LogPage>> {
            unimplemented!()
        }
        fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
            unimplemented!()
        }
        fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
            unimplemented!()
        }
        fn current_branch(&self) -> Result<String> {
            unimplemented!()
        }
        fn list_branches(&self) -> Result<Vec<Branch>> {
            unimplemented!()
        }
        fn list_remotes(&self) -> Result<Vec<Remote>> {
            unimplemented!()
        }
        fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
            unimplemented!()
        }
        fn status(&self) -> Result<RepoStatus> {
            unimplemented!()
        }
        fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
            unimplemented!()
        }
        fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn delete_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            unimplemented!()
        }
        fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
            unimplemented!()
        }
        fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
            unimplemented!()
        }
        fn stash_list(&self) -> Result<Vec<StashEntry>> {
            unimplemented!()
        }
        fn stash_apply(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stash_drop(&self, _index: usize) -> Result<()> {
            unimplemented!()
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
        fn commit(&self, _message: &str) -> Result<()> {
            unimplemented!()
        }
        fn fetch_all(&self) -> Result<()> {
            unimplemented!()
        }
        fn pull(&self, _mode: PullMode) -> Result<()> {
            self.calls.lock().unwrap().push("pull");
            Ok(())
        }
        fn push(&self) -> Result<()> {
            unimplemented!()
        }
        fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
            unimplemented!()
        }
    }

    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _workdir: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    repos.insert(
        RepoId(1),
        Arc::new(RecordingRepo {
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            calls: Arc::clone(&calls),
        }),
    );
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::Pull {
            repo_id: RepoId(1),
            mode: PullMode::Default,
            prune: false,
            auth: None,
        },
    );

    let finished = loop {
        match recv_effect_message(&msg_rx, Duration::from_secs(5)).expect("pull finishes") {
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished { result, .. }) => {
                break result;
            }
            _ => continue,
        }
    };
    assert!(finished.is_ok(), "{finished:?}");
    // Released before the finish message, so the refresh it triggers reads
    // through a fresh store.
    assert_eq!(*calls.lock().unwrap(), vec!["pull", "release"]);
}
