//! Hosted panes over a real repository: two diff panes and a file list,
//! each independent of the others and of History.

use super::*;
use gitcomet_extension_api::{
    ChangeSource, DiffLineSide, DiffPane, DiffPaneOptions, DiffPanePolicy, DiffRowDecor,
    DiffSnapshot, Registry,
};
use gitcomet_state::model::Loadable;
use gitcomet_state::msg::WorktreeSaveBaseline;

fn git(dir: &Path, args: &[&str]) {
    crate::test_support::git(dir, args);
}

fn head(dir: &Path) -> String {
    let out = crate::test_support::git(dir, &["rev-parse", "HEAD"]);
    String::from_utf8(out).unwrap().trim().to_string()
}

fn numbered(prefix: &str, count: usize, edit_at: Option<usize>) -> String {
    (0..count)
        .map(|n| {
            if Some(n) == edit_at {
                format!("{prefix} edited {n}\n")
            } else {
                format!("{prefix} {n}\n")
            }
        })
        .collect()
}

fn install_example(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        let registry = Registry::build(vec![Box::new(
            gitcomet_extension_example::review::ReviewExtension,
        )])
        .unwrap();
        crate::view::extension_host::install(registry, app);
    });
}

fn repository_with_edits() -> tempfile::TempDir {
    repository_with_edits_of(30, [3, 20])
}

fn repository_with_edits_of(lines: usize, edits: [usize; 2]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "T"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // Same bytes under any user or system config (Windows CI sets autocrlf).
    git(root, &["config", "core.autocrlf", "false"]);
    std::fs::write(root.join("a.rs"), numbered("a", lines, None)).unwrap();
    std::fs::write(root.join("b.rs"), numbered("b", lines, None)).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "init"]);
    std::fs::write(root.join("a.rs"), numbered("a", lines, Some(edits[0]))).unwrap();
    std::fs::write(root.join("b.rs"), numbered("b", lines, Some(edits[1]))).unwrap();

    dir
}

/// A repository with unstaged edits to `a.rs` and `b.rs`, open in `view`'s
/// store through the real backend.
fn open_repository(
    cx: &mut gpui::TestAppContext,
) -> (
    tempfile::TempDir,
    AppStore,
    gpui::Entity<GitCometView>,
    &mut gpui::VisualTestContext,
) {
    open_repository_with(cx, Arc::new(TestBackend))
}

/// [`open_repository`] with `backend` opening any other repository (such
/// as a linked worktree).
fn open_repository_with(
    cx: &mut gpui::TestAppContext,
    backend: Arc<dyn gitcomet_core::services::GitBackend>,
) -> (
    tempfile::TempDir,
    AppStore,
    gpui::Entity<GitCometView>,
    &mut gpui::VisualTestContext,
) {
    open_repository_of(cx, backend, 30, [3, 20])
}

/// [`open_repository_with`] with `lines` lines in each file, `a.rs` edited
/// at `edits[0]` and `b.rs` at `edits[1]`.
fn open_repository_of(
    cx: &mut gpui::TestAppContext,
    backend: Arc<dyn gitcomet_core::services::GitBackend>,
    lines: usize,
    edits: [usize; 2],
) -> (
    tempfile::TempDir,
    AppStore,
    gpui::Entity<GitCometView>,
    &mut gpui::VisualTestContext,
) {
    open_repository_in(cx, backend, repository_with_edits_of(lines, edits))
}

/// A repository whose `a.rs` gains `pub` on line 1 and a new line 2, and
/// whose `b.rs` loses `pub` on line 1, in Rust Difftastic parses.
fn rust_repository() -> tempfile::TempDir {
    let dir = repository_with_edits_of(1, [0, 0]);
    let root = dir.path();
    std::fs::write(root.join("a.rs"), "fn a() -> i32 {\n    1\n}\n").unwrap();
    std::fs::write(root.join("b.rs"), "pub fn b() -> i32 {\n    1\n}\n").unwrap();
    git(root, &["commit", "-qam", "rust"]);
    std::fs::write(
        root.join("a.rs"),
        "pub fn a() -> i32 {\n    let two = 2;\n    1\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("b.rs"), "fn b() -> i32 {\n    1\n}\n").unwrap();
    dir
}

/// `dir`'s repository, open in a window through the real backend.
fn open_repository_in(
    cx: &mut gpui::TestAppContext,
    backend: Arc<dyn gitcomet_core::services::GitBackend>,
    dir: tempfile::TempDir,
) -> (
    tempfile::TempDir,
    AppStore,
    gpui::Entity<GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let root = dir.path();

    install_example(cx);
    let repo = gitcomet_git_gix::GixBackend.open(root).expect("open repo");
    let (store, events) = AppStore::new_test(backend);
    // Watching is not under test. Windows reports the `.git/logs` and
    // `.git/hooks` directory timestamps setup left dirty only once a watch
    // exists (NTFS flushes them late), FSEvents replays recent history, and
    // that git-state refresh reloads every worktree pane mid-assertion.
    store.disable_repo_monitors_for_test();
    let repo_id = RepoId(1);
    let mut repo_state = RepoState::new_opening(repo_id, repo.spec().clone());
    repo_state.open = Loadable::Ready(());
    let state = Arc::new(AppState {
        active_repo: Some(repo_id),
        git_runtime: available_git_runtime_state(),
        repos: vec![repo_state],
        ..AppState::test_default()
    });
    store.replace_snapshot_for_test(Arc::clone(&state));
    store.insert_repo_for_test(repo_id, repo);
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    publish(cx, &view, state);
    (dir, store, view, cx)
}

fn publish(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    state: Arc<AppState>,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.ui_model
                .update(cx, |model, cx| model.set_state(state, cx))
        });
    });
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
}

/// Hands the store's state to the window until `ready` holds for it.
fn settle(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    what: &str,
    mut ready: impl FnMut(&mut gpui::VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        publish(cx, view, store.snapshot());
        if ready(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn rows_with(cx: &mut gpui::VisualTestContext, pane: &DiffPane, needle: &str) -> usize {
    let needle = needle.to_string();
    cx.update(|_window, app| {
        pane.set_search(needle, app);
        pane.search_matches(app)
    })
}

#[gpui::test]
fn worktree_lists_respect_the_index_and_untracked_option(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx) = open_repository(cx);
    crate::test_support::git(dir.path(), &["add", "a.rs"]);
    std::fs::write(dir.path().join("untracked.rs"), "untracked\n").unwrap();
    let lists = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        [
            (DiffArea::Staged, true),
            (DiffArea::Unstaged, false),
            (DiffArea::Unstaged, true),
        ]
        .map(|(area, include_untracked)| {
            host.create_file_list(
                &repository,
                ChangeSource::worktree(area, include_untracked),
                |_, _, _| {},
                app,
            )
            .unwrap()
        })
    });
    settle(cx, &view, &store, "worktree lists", |cx| {
        cx.update(|_, app| lists.iter().all(|list| !list.is_loading(app)))
    });
    let paths = cx.update(|_, app| {
        lists.each_ref().map(|list| {
            list.files(app)
                .into_iter()
                .map(|file| file.path)
                .collect::<Vec<_>>()
        })
    });
    assert_eq!(paths[0], vec![PathBuf::from("a.rs")]);
    assert_eq!(paths[1], vec![PathBuf::from("b.rs")]);
    assert_eq!(
        paths[2],
        vec![PathBuf::from("b.rs"), PathBuf::from("untracked.rs")]
    );
}

#[gpui::test]
fn two_panes_and_a_list_change_cancel_and_drop_independently(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository(cx);
    let (host, repository) = cx.update(|_window, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        (host, repository)
    });
    let target = |path: &str| {
        gitcomet_core::domain::DiffTarget::working_tree(path.into(), DiffArea::Unstaged)
    };

    let picked = std::rc::Rc::new(std::cell::RefCell::new(None));
    let seen = std::rc::Rc::clone(&picked);
    let decor = std::rc::Rc::new(|side: DiffLineSide, line: u32| {
        (side == DiffLineSide::New && line == 4).then(|| DiffRowDecor::gutter("●"))
    });
    let (left, right, list) = cx.update(|_window, app| {
        let left = host
            .create_diff_pane(
                &repository,
                target("a.rs"),
                DiffPaneOptions {
                    decor: Some(decor),
                    ..DiffPaneOptions::default()
                },
                app,
            )
            .unwrap();
        let right = host
            .create_diff_pane(
                &repository,
                target("b.rs"),
                DiffPaneOptions {
                    policy: DiffPanePolicy::read_only(),
                    ..DiffPaneOptions::default()
                },
                app,
            )
            .unwrap();
        let list = host
            .create_file_list(
                &repository,
                ChangeSource::comparison(
                    gitcomet_core::domain::CommitId("HEAD".into()),
                    None,
                    Default::default(),
                ),
                move |change, target, _| {
                    *seen.borrow_mut() = Some((change.path.clone(), target));
                },
                app,
            )
            .unwrap();
        (left, right, list)
    });

    settle(cx, &view, &store, "both panes and the list", |cx| {
        cx.update(|_window, app| {
            !left.is_loading(app) && !right.is_loading(app) && !list.is_loading(app)
        })
    });
    assert_eq!(rows_with(cx, &left, "a edited 3"), 1);
    assert_eq!(
        rows_with(cx, &right, "b edited 20"),
        0,
        "read-only panes do not search"
    );
    let files: Vec<_> = cx.update(|_window, app| {
        list.files(app)
            .into_iter()
            .map(|change| change.path)
            .collect()
    });
    assert_eq!(files, vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")]);

    // Picking a file hands its target to the extension.
    cx.update(|_window, app| assert!(list.select_path(Path::new("b.rs"), app)));
    cx.run_until_parked();
    let (path, picked_target) = picked.borrow_mut().take().expect("a pick");
    assert_eq!(path, PathBuf::from("b.rs"));
    assert_eq!(picked_target.file_path(), Some(Path::new("b.rs")));

    // Retargeting the left pane leaves the right one and History alone.
    // Before the retarget only the right pane shows b.rs.
    let right_view = *store.snapshot().repos[0]
        .diff_sessions
        .iter()
        .find(|(_, session)| session.target == target("b.rs"))
        .expect("the right pane's session")
        .0;
    let right_session = |store: &AppStore| {
        store.snapshot().repos[0]
            .diff_sessions
            .get(&right_view)
            .map(|session| (session.generation, session.rev))
    };
    let right_generation = right_session(&store);
    cx.update(|_window, app| left.set_target(target("b.rs"), app));
    settle(cx, &view, &store, "the retarget", |cx| {
        cx.update(|_window, app| !left.is_loading(app))
    });
    assert_eq!(rows_with(cx, &left, "b edited 20"), 1);
    assert_eq!(right_session(&store), right_generation);
    assert!(store.snapshot().repos[0].diff_state.diff_target.is_none());

    // Dropping the right pane closes its session; the left one stays.
    drop(right);
    settle(cx, &view, &store, "the right session to close", |_| {
        store.snapshot().repos[0].diff_sessions.len() == 1
    });
    assert_eq!(rows_with(cx, &left, "b edited 20"), 1);
    drop(list);
    settle(cx, &view, &store, "the list session to close", |_| {
        store.snapshot().repos[0].change_lists.is_empty()
    });
}

fn selector(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn hosted_editor(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
) -> (DiffPane, gpui::Entity<DiffRenderer>) {
    hosted_editor_at(cx, view, store, "a.rs")
}

fn hosted_editor_at(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    path: &str,
) -> (DiffPane, gpui::Entity<DiffRenderer>) {
    let pane = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        host.create_diff_pane(
            &repository,
            DiffTarget::working_tree(path.into(), DiffArea::Unstaged),
            DiffPaneOptions::default(),
            app,
        )
        .unwrap()
    });
    settle(cx, view, store, "hosted file", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    let renderer = cx.update(|window, app| {
        let entity = pane
            .view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap_or_else(|_| panic!("hosted pane"));
        let renderer = entity.update(app, |pane, cx| pane.renderer_for_test(window, cx));
        renderer.update(app, |renderer, _| {
            renderer.auto_save_file_edits = false;
            renderer.store.dispatch(Msg::OpenFileEditor {
                repo_id: RepoId(1),
                path: path.into(),
            });
        });
        renderer
    });
    settle(cx, view, store, "hosted editor", |cx| {
        cx.update(|_, app| {
            renderer.update(app, |pane, cx| {
                pane.ensure_file_editor_loaded(cx);
                pane.file_editor_key().is_some() && !pane.file_editor_loading()
            })
        })
    });
    (pane, renderer)
}

fn edit_hosted(
    renderer: &gpui::Entity<DiffRenderer>,
    text: &str,
    cx: &mut gpui::VisualTestContext,
) {
    cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            pane.file_editor_input.update(cx, |input, cx| {
                input.replace_utf8_range(0..input.text().len(), text, cx);
            });
        })
    });
    cx.run_until_parked();
}

#[gpui::test]
fn hosted_parent_saves_keep_their_binding_when_a_nested_repository_is_open(
    cx: &mut gpui::TestAppContext,
) {
    let (dir, store, view, cx) = open_repository(cx);
    let nested = dir.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let nested = canonicalize_or_original(nested);
    git(&nested, &["init", "-q", "-b", "main"]);
    let path = nested.join("child.rs");
    std::fs::write(&path, "nested original\n").unwrap();
    let nested_repo = gitcomet_git_gix::GixBackend.open(&nested).unwrap();
    let mut state = (*store.snapshot()).clone();
    let mut nested_state = RepoState::new_opening(RepoId(2), nested_repo.spec().clone());
    nested_state.open = Loadable::Ready(());
    state.repos.push(nested_state);
    store.replace_snapshot_for_test(Arc::new(state));
    store.insert_repo_for_test(RepoId(2), nested_repo);
    publish(cx, &view, store.snapshot());
    let (pane, renderer) = hosted_editor_at(cx, &view, &store, "nested/child.rs");
    let identity = gitcomet_core::filesystem::DocumentIdentity(path.clone());
    cx.update(|_, app| {
        assert_eq!(
            view.read(app)
                .diff_renderer
                .read(app)
                .document_repo_path(&identity),
            Some((RepoId(2), "child.rs".into())),
            "the primary renderer still chooses the innermost open repository"
        );
    });
    edit_hosted(&renderer, "saved through parent binding\n", cx);
    let owner = cx.update(|_, app| view.read(app).ui_model.read(app).documents.clone());
    cx.update(|_, app| {
        owner.resolve(None, None, true, app);
        assert!(
            !owner.saves_drained(None),
            "Save all admitted the hosted write"
        );
        assert_eq!(
            renderer.read(app).document_repo_path(&identity),
            Some((RepoId(1), "nested/child.rs".into())),
            "hosted ownership stays within its parent repository"
        );
    });
    settle(cx, &view, &store, "nested hosted save", |_| {
        owner.saves_drained(None) && owner.unsaved_documents().is_empty()
    });
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "saved through parent binding\n"
    );
    drop(pane);
    drop(renderer);
    cx.run_until_parked();

    store.dispatch(Msg::SetActiveRepo { repo_id: RepoId(2) });
    store.dispatch(Msg::OpenFileEditor {
        repo_id: RepoId(2),
        path: "child.rs".into(),
    });
    let primary = cx.update(|_, app| view.read(app).diff_renderer.clone());
    settle(cx, &view, &store, "primary nested editor", |cx| {
        cx.update(|_, app| {
            primary.update(app, |pane, cx| {
                pane.auto_save_file_edits = false;
                pane.ensure_file_editor_loaded(cx);
                pane.file_editor_key().as_ref() == Some(&identity) && !pane.file_editor_loading()
            })
        })
    });
    edit_hosted(&primary, "saved through nested primary\n", cx);
    cx.update(|_, app| owner.resolve(None, None, true, app));
    settle(cx, &view, &store, "nested primary save", |_| {
        owner.saves_drained(None) && owner.unsaved_documents().is_empty()
    });
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "saved through nested primary\n"
    );
}

#[gpui::test]
fn hosted_save_admission_follows_policy_and_keeps_admitted_receipts(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx) = open_repository(cx);
    let (pane, renderer) = hosted_editor(cx, &view, &store);
    let path = dir.path().join("a.rs");
    let original = std::fs::read_to_string(&path).unwrap();
    let owner = cx.update(|_, app| view.read(app).ui_model.read(app).documents.clone());
    edit_hosted(&renderer, "admitted before policy changed\n", cx);

    cx.update(|_, app| {
        pane.set_policy(DiffPanePolicy::read_only(), app);
        owner.resolve(None, None, true, app);
        assert!(
            owner.saves_drained(None),
            "read-only policy refused the write"
        );
        assert!(renderer.read(app).file_editor_dirty());
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

    let held = gitcomet_core::filesystem::global()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    cx.update(|_, app| {
        pane.set_policy(DiffPanePolicy::default(), app);
        owner.resolve(None, None, true, app);
        assert!(
            !owner.saves_drained(None),
            "editable policy admitted the write"
        );
        pane.set_policy(DiffPanePolicy::read_only(), app);
    });
    drop(held);
    settle(cx, &view, &store, "receipt after policy changed", |_| {
        owner.saves_drained(None) && owner.unsaved_documents().is_empty()
    });
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "admitted before policy changed\n"
    );
}

#[gpui::test]
fn hosted_save_admission_checks_linked_worktree_before_renderer_sync(
    cx: &mut gpui::TestAppContext,
) {
    let (dir, store, view, cx) = open_repository(cx);
    let (pane, renderer) = hosted_editor(cx, &view, &store);
    let (_linked_dir, linked) = add_linked_worktree(dir.path());
    let path = dir.path().join("a.rs");
    let original = std::fs::read_to_string(&path).unwrap();
    let owner = cx.update(|_, app| view.read(app).ui_model.read(app).documents.clone());
    edit_hosted(&renderer, "main checkout only\n", cx);
    let before_retarget = store.snapshot();
    let binding = cx.update(|_, app| renderer.read(app).store.binding.unwrap());
    let mut retargeted = (*before_retarget).clone();
    let session = Arc::make_mut(&mut retargeted.repos[0].diff_sessions)
        .get_mut(&binding.view)
        .unwrap();
    session.target = session.target.clone().in_worktree(linked.clone());
    store.replace_snapshot_for_test(Arc::new(retargeted));

    cx.update(|_, app| {
        assert!(renderer.read(app).store.policy.allow_edit);
        owner.resolve(None, None, true, app);
        assert!(
            owner.saves_drained(None),
            "live linked session refused the write"
        );
        assert!(renderer.read(app).file_editor_dirty());
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(linked.join("a.rs")).unwrap(),
        "linked edit\n"
    );

    store.replace_snapshot_for_test(before_retarget);
    cx.update(|_, app| owner.resolve(None, None, true, app));
    settle(cx, &view, &store, "main checkout save", |_| {
        owner.saves_drained(None) && owner.unsaved_documents().is_empty()
    });
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "main checkout only\n"
    );
    assert_eq!(
        std::fs::read_to_string(linked.join("a.rs")).unwrap(),
        "linked edit\n"
    );
    drop(pane);
}

#[gpui::test]
fn hosted_save_admission_rejects_a_reopened_repository(cx: &mut gpui::TestAppContext) {
    hosted_save_admission_rejects_stale_binding(cx, false);
}

#[gpui::test]
fn hosted_save_admission_rejects_a_removed_session(cx: &mut gpui::TestAppContext) {
    hosted_save_admission_rejects_stale_binding(cx, true);
}

fn hosted_save_admission_rejects_stale_binding(
    cx: &mut gpui::TestAppContext,
    remove_session: bool,
) {
    let (dir, store, view, cx) = open_repository(cx);
    let (_pane, renderer) = hosted_editor(cx, &view, &store);
    let path = dir.path().join("a.rs");
    let original = std::fs::read_to_string(&path).unwrap();
    let owner = cx.update(|_, app| view.read(app).ui_model.read(app).documents.clone());
    edit_hosted(&renderer, "current binding only\n", cx);
    let current = store.snapshot();
    let binding = cx.update(|_, app| renderer.read(app).store.binding.unwrap());
    let mut stale = (*current).clone();
    if remove_session {
        Arc::make_mut(&mut stale.repos[0].diff_sessions).remove(&binding.view);
    } else {
        let mut replacement = RepoState::new_opening(RepoId(1), current.repos[0].spec.clone());
        replacement.open = Loadable::Ready(());
        replacement.diff_sessions = current.repos[0].diff_sessions.clone();
        assert_ne!(replacement.lifetime(), binding.lifetime);
        stale.repos[0] = replacement;
    }
    store.replace_snapshot_for_test(Arc::new(stale));
    cx.update(|_, app| {
        owner.resolve(None, None, true, app);
        assert!(owner.saves_drained(None), "stale binding refused the write");
        assert!(renderer.read(app).file_editor_dirty());
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

    store.replace_snapshot_for_test(current);
    cx.update(|_, app| owner.resolve(None, None, true, app));
    settle(cx, &view, &store, "current binding save", |_| {
        owner.saves_drained(None) && owner.unsaved_documents().is_empty()
    });
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "current binding only\n"
    );
}

#[gpui::test]
fn hosted_edits_are_saved_and_discarded_by_the_window_close_guard(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx) = open_repository(cx);
    let (_pane, renderer) = hosted_editor(cx, &view, &store);
    edit_hosted(&renderer, "saved by close\n", cx);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            assert!(view.request_quit_unsaved_file_edits_prompt(cx));
            assert!(
                view.pending_unsaved_file_edits_prompt
                    .as_ref()
                    .is_some_and(|prompt| prompt.files == vec![SharedString::from("a.rs")])
            );
            view.resolve_unsaved_file_edits(UnsavedFileEditsAction::QuitApp, true, cx);
        })
    });
    settle(cx, &view, &store, "hosted close save", |cx| {
        cx.update(|_, app| {
            let owner = &view.read(app).ui_model.read(app).documents;
            owner.saves_drained(None) && owner.unsaved_documents().is_empty()
        })
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
        "saved by close\n"
    );
    edit_hosted(&renderer, "discarded by close\n", cx);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.resolve_unsaved_file_edits(UnsavedFileEditsAction::QuitApp, false, cx);
        })
    });
    settle(cx, &view, &store, "hosted close discard", |cx| {
        cx.update(|_, app| {
            !renderer.read(app).file_editor_loading()
                && view
                    .read(app)
                    .ui_model
                    .read(app)
                    .documents
                    .unsaved_documents()
                    .is_empty()
        })
    });
    cx.update(|_, app| {
        assert_eq!(
            renderer.read(app).file_editor_input.read(app).text(),
            "saved by close\n"
        )
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
        "saved by close\n"
    );
}

#[gpui::test]
fn filesystem_resolution_and_rename_reach_hosted_editors(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::filesystem::{Operation, Request};
    let (dir, store, view, cx) = open_repository(cx);
    let (_pane, renderer) = hosted_editor(cx, &view, &store);
    let path = canonicalize_or_original(dir.path().join("a.rs"));
    let moved = path.with_file_name("moved.rs");
    edit_hosted(&renderer, "saved before rename\n", cx);
    cx.update(|_, app| {
        assert!(crate::app::filesystem_has_unsaved_buffers(
            std::slice::from_ref(&path),
            app
        ));
        crate::app::resolve_filesystem_buffers(std::slice::from_ref(&path), true, app);
    });
    settle(cx, &view, &store, "hosted filesystem save", |cx| {
        cx.update(|_, app| {
            crate::app::filesystem_saves_drained(app)
                && !crate::app::filesystem_has_unsaved_buffers(std::slice::from_ref(&path), app)
        })
    });
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "saved before rename\n"
    );
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.submit_filesystem_operation(
                Request::new(Operation::Rename {
                    source: path.clone(),
                    name: "moved.rs".into(),
                }),
                None,
                window,
                cx,
            );
        })
    });
    settle(cx, &view, &store, "hosted rename", |cx| {
        cx.update(|_, app| {
            renderer
                .read(app)
                .file_editor_key()
                .is_some_and(|identity| identity.0 == moved)
                && !view.read(app).file_operations.has_pending()
        })
    });
    edit_hosted(&renderer, "discarded after rename\n", cx);
    cx.update(|_, app| {
        crate::app::resolve_filesystem_buffers(std::slice::from_ref(&moved), false, app)
    });
    settle(cx, &view, &store, "hosted filesystem discard", |cx| {
        cx.update(|_, app| {
            !renderer.read(app).file_editor_loading()
                && !crate::app::filesystem_has_unsaved_buffers(std::slice::from_ref(&moved), app)
        })
    });
    assert_eq!(
        std::fs::read_to_string(&moved).unwrap(),
        "saved before rename\n"
    );
    assert!(!path.exists());
}

#[gpui::test]
fn disposing_a_hosted_editor_transfers_dirty_text_and_releases_clean_records(
    cx: &mut gpui::TestAppContext,
) {
    let (dir, store, view, cx) = open_repository(cx);
    let (pane, renderer) = hosted_editor(cx, &view, &store);
    let path = canonicalize_or_original(dir.path().join("a.rs"));
    drop(renderer);
    drop(pane);
    cx.update(|_, _| {});
    cx.run_until_parked();
    cx.update(|_, app| {
        assert!(
            view.read(app)
                .ui_model
                .read(app)
                .documents
                .at_path(&path)
                .is_empty()
        )
    });

    let (pane, renderer) = hosted_editor(cx, &view, &store);
    edit_hosted(&renderer, "retained after disposal\n", cx);
    let id = cx.update(|_, app| renderer.read(app).editor_documents.active().unwrap().id());
    drop(renderer);
    drop(pane);
    cx.update(|_, _| {});
    cx.run_until_parked();
    cx.update(|_, app| {
        let owner = view.read(app).ui_model.read(app).documents.clone();
        let buffers = owner.at_path(&path);
        assert_eq!(buffers.len(), 1);
        assert_eq!(buffers[0].id(), id);
        assert_eq!(buffers[0].read().repository(), None);
        assert_eq!(
            buffers[0].stashed().unwrap().text.as_ref(),
            "retained after disposal\n"
        );
        assert!(owner.resolve(None, None, true, app));
    });
    settle(cx, &view, &store, "detached hosted save", |cx| {
        cx.update(|_, app| {
            let owner = &view.read(app).ui_model.read(app).documents;
            owner.saves_drained(None) && owner.unsaved_documents().is_empty()
        })
    });
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "retained after disposal\n"
    );
}

#[gpui::test]
fn disposing_a_hosted_editor_keeps_its_pending_write_receipt(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx) = open_repository(cx);
    let (pane, renderer) = hosted_editor(cx, &view, &store);
    let path = canonicalize_or_original(dir.path().join("a.rs"));
    edit_hosted(&renderer, "pending at disposal\n", cx);
    let held = gitcomet_core::filesystem::global()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    cx.update(|_, app| renderer.update(app, |pane, cx| pane.save_file_editor_buffer(cx)));
    drop(renderer);
    drop(pane);
    cx.update(|_, _| {});
    cx.run_until_parked();
    cx.update(|_, app| {
        let owner = &view.read(app).ui_model.read(app).documents;
        assert!(!owner.saves_drained(None));
        let buffers = owner.at_path(&path);
        assert_eq!(buffers.len(), 1);
        assert_eq!(buffers[0].read().repository(), None);
        assert_eq!(
            buffers[0].stashed().unwrap().text.as_ref(),
            "pending at disposal\n"
        );
    });
    drop(held);
    settle(cx, &view, &store, "released hosted save", |cx| {
        cx.update(|_, app| {
            let owner = &view.read(app).ui_model.read(app).documents;
            owner.saves_drained(None) && owner.unsaved_documents().is_empty()
        })
    });
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "pending at disposal\n"
    );
}

#[gpui::test]
fn retargeting_ignores_old_notifications_and_clears_files_before_loading(
    cx: &mut gpui::TestAppContext,
) {
    use crate::view::hosted::{
        diff_pane::{DiffPaneView, HostedDiffPane},
        file_list::{FileListView, HostedFileList},
    };
    use gitcomet_state::diff_session::DiffViewId;
    use std::rc::Rc;
    let (_dir, store, view, cx) = open_repository(cx);
    let (host, repository) = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        (host, repository)
    });
    let target = |path: &str| DiffTarget::working_tree(path.into(), DiffArea::Unstaged);
    let source = ChangeSource::comparison(CommitId("HEAD".into()), None, Default::default());
    let (seed_pane, seed_list) = cx.update(|_, app| {
        (
            host.create_diff_pane(&repository, target("a.rs"), DiffPaneOptions::default(), app)
                .unwrap(),
            host.create_file_list(&repository, source.clone(), |_, _, _| {}, app)
                .unwrap(),
        )
    });
    settle(cx, &view, &store, "seed content", |cx| {
        cx.update(|_, app| !seed_pane.is_loading(app) && !seed_list.is_loading(app))
    });
    let mut state = (*store.snapshot()).clone();
    let session = state.repos[0]
        .diff_sessions
        .values()
        .next()
        .unwrap()
        .clone();
    let change_list = state.repos[0].change_lists.values().next().unwrap().clone();
    let environment = cx.update(|window, app| {
        use crate::view::hosted::environment::{DiffPaneEnvironment, DiffPreferences};
        use crate::view::panes::diff::services::{DiffCommands, DiffServices};
        let root = view.read(app);
        DiffPaneEnvironment {
            services: DiffServices {
                store: crate::view::pane_store::PaneStore::read_only(),
                commands: DiffCommands::default(),
                documents: root.ui_model.read(app).documents.clone(),
                tooltip: root.tooltip_host.downgrade(),
            },
            preferences: DiffPreferences::Window(root.ui_model.downgrade()),
            window: window.window_handle(),
        }
    });
    // These panes have no dispatcher: only the explicitly published snapshots
    // can advance them, making the notification-before-Open race deterministic.
    let (pane_entity, list_entity) = cx.update(|_, app| {
        (
            app.new(|cx| {
                DiffPaneView::new(
                    host.clone(),
                    environment,
                    std::sync::Weak::new(),
                    repository.clone(),
                    target("a.rs"),
                    DiffPaneOptions::default(),
                    cx,
                )
            }),
            app.new(|cx| {
                FileListView::new(
                    host.clone(),
                    std::sync::Weak::new(),
                    repository.clone(),
                    source,
                    Rc::new(|_, _, _| {}),
                    cx,
                )
            }),
        )
    });
    let pane = DiffPane::new(Rc::new(HostedDiffPane {
        entity: pane_entity.clone(),
    }));
    let list = gitcomet_extension_api::FileList::new(Rc::new(HostedFileList {
        entity: list_entity.clone(),
    }));
    let (pane_id, list_id) = cx.update(|_, app| {
        (
            DiffViewId(pane_entity.read(app).view_id()),
            DiffViewId(list_entity.read(app).test_parts().0),
        )
    });
    Arc::make_mut(&mut state.repos[0].diff_sessions).insert(pane_id, session);
    Arc::make_mut(&mut state.repos[0].change_lists).insert(list_id, change_list);
    publish(cx, &view, Arc::new(state.clone()));
    assert_eq!(rows_with(cx, &pane, "a edited 3"), 1);
    assert!(!cx.update(|_, app| list.files(app)).is_empty());
    let new_source = ChangeSource::Commit(CommitId("other".into()));
    cx.update(|_, app| {
        pane.set_target(target("b.rs"), app);
        list.set_source(new_source.clone(), app);
        assert!(list.files(app).is_empty());
        assert!(!list.select_path(Path::new("a.rs"), app));
    });
    // A newer revision of the OLD target/source arrives before Open is reduced.
    Arc::make_mut(&mut state.repos[0].diff_sessions)
        .get_mut(&pane_id)
        .unwrap()
        .rev += 1;
    Arc::make_mut(&mut state.repos[0].change_lists)
        .get_mut(&list_id)
        .unwrap()
        .rev += 1;
    publish(cx, &view, Arc::new(state.clone()));
    assert_eq!(rows_with(cx, &pane, "a edited 3"), 0);
    cx.update(|_, app| {
        assert!(pane.is_loading(app));
        assert!(list.is_loading(app));
        assert!(list.files(app).is_empty());
    });
    let session = Arc::make_mut(&mut state.repos[0].diff_sessions)
        .get_mut(&pane_id)
        .unwrap();
    session.target = target("b.rs");
    session.rev += 1;
    session.diff = Loadable::Loading;
    session.diff_file = Loadable::Loading;
    let change_list = Arc::make_mut(&mut state.repos[0].change_lists)
        .get_mut(&list_id)
        .unwrap();
    change_list.source = new_source;
    change_list.rev += 1;
    change_list.files = Loadable::Error("Could not load comparison".into());
    change_list.base = None;
    publish(cx, &view, Arc::new(state));
    assert_eq!(rows_with(cx, &pane, "a edited 3"), 0);
    cx.update(|_, app| {
        assert!(list.files(app).is_empty());
        assert!(!list.is_loading(app));
        assert_eq!(
            list_entity.read(app).load_error(),
            Some("Could not load comparison")
        );
    });
}

/// Opens the example's Changes view and waits for its file list.
fn open_changes_view(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
) -> (
    u64,
    gpui::Entity<gitcomet_extension_example::changes::ChangesView>,
) {
    click_debug_selector(cx, "repository_view_1");
    let mut id = None;
    settle(cx, view, store, "the example's file list", |cx| {
        id = store.snapshot().repos[0]
            .change_lists
            .keys()
            .next()
            .copied();
        id.is_some_and(|id| {
            cx.debug_bounds(selector(format!("hosted_file_list_{}_file_b.rs", id.0)))
                .is_some()
        })
    });
    let changes = cx.update(|_window, app| {
        let this = view.read(app);
        let repo = this.active_repo().unwrap();
        let router = this.repository_views.as_ref().unwrap();
        router
            .active_view(repo)
            .unwrap()
            .downcast::<gitcomet_extension_example::changes::ChangesView>()
            .unwrap_or_else(|_| panic!("the Changes view"))
    });
    (id.unwrap().0, changes)
}

#[gpui::test]
fn the_example_changes_view_shows_picks_in_two_panes(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository(cx);
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    let shown = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let changes = changes.read(app);
            let target = |pane: Option<&DiffPane>| {
                pane.and_then(|pane| pane.target(app))
                    .and_then(|target| target.file_path().map(Path::to_path_buf))
            };
            (target(changes.current()), target(changes.previous()))
        })
    };

    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_a.rs")),
    );
    settle(cx, &view, &store, "the first pick", |_| {
        store.snapshot().repos[0].diff_sessions.len() == 1
    });
    assert_eq!(shown(cx), (Some(PathBuf::from("a.rs")), None));

    // The example flags a line from the current pane's gutter.
    let current_id = store.snapshot().repos[0]
        .diff_sessions
        .keys()
        .next()
        .unwrap()
        .0;
    settle(cx, &view, &store, "the current pane's rows", |cx| {
        cx.debug_bounds(selector(format!("hosted_diff_{current_id}_gutter_0")))
            .is_some()
    });
    click_debug_selector(cx, selector(format!("hosted_diff_{current_id}_gutter_0")));
    publish(cx, &view, store.snapshot());
    let flags = cx.update(|_window, app| changes.read(app).flags().clone());
    assert_eq!(
        flags.into_iter().collect::<Vec<_>>(),
        vec![(DiffLineSide::New, 1)]
    );
    assert!(
        cx.debug_bounds(selector(format!("hosted_diff_{current_id}_markers")))
            .is_some()
    );
    // The list groups by role and flags the file; its chip shows only it.
    for part in ["group_Code", "glyph_a.rs"] {
        assert!(
            cx.debug_bounds(selector(format!("hosted_file_list_{list_id}_{part}")))
                .is_some(),
            "{part}"
        );
    }
    click_debug_selector(cx, selector(format!("file_filter_{list_id}_0")));
    publish(cx, &view, store.snapshot());
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{list_id}_file_b.rs")))
            .is_none()
    );
    click_debug_selector(cx, selector(format!("file_filter_{list_id}_0")));
    publish(cx, &view, store.snapshot());

    // The flag's own click removes it, and with it the file's flag.
    click_debug_selector(cx, selector(format!("hosted_diff_{current_id}_gutter_0")));
    publish(cx, &view, store.snapshot());
    assert!(cx.update(|_window, app| changes.read(app).flags().is_empty()));
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{list_id}_glyph_a.rs")))
            .is_none()
    );

    // A note on the selected line goes under it; clicking it removes it.
    click_debug_selector(cx, selector(format!("hosted_diff_{current_id}_row_0")));
    publish(cx, &view, store.snapshot());
    click_debug_selector(cx, "diff_bar_note");
    publish(cx, &view, store.snapshot());
    assert_eq!(cx.update(|_window, app| changes.read(app).notes().len()), 1);
    click_debug_selector(cx, selector(format!("hosted_diff_{current_id}_row_1")));
    publish(cx, &view, store.snapshot());
    assert!(cx.update(|_window, app| changes.read(app).notes().is_empty()));

    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_b.rs")),
    );
    settle(cx, &view, &store, "the second pick", |cx| {
        let sessions = store.snapshot().repos[0].diff_sessions.len() == 2;
        let drawn = store.snapshot().repos[0].diff_sessions.keys().all(|id| {
            // The shared renderer initially reveals the first changed line.
            // Row zero need not be inside either pane's visible window.
            (0..64).any(|row| {
                cx.debug_bounds(selector(format!("hosted_diff_{}_row_{row}", id.0)))
                    .is_some()
            })
        });
        sessions && drawn
    });
    assert_eq!(
        shown(cx),
        (Some(PathBuf::from("b.rs")), Some(PathBuf::from("a.rs")))
    );
    assert!(store.snapshot().repos[0].diff_state.diff_target.is_none());

    // Back to History: the example's panes keep their sessions.
    click_debug_selector(cx, "repository_view_history");
    publish(cx, &view, store.snapshot());
    assert_eq!(store.snapshot().repos[0].diff_sessions.len(), 2);
}

/// Whether display row `row` of pane `id` is drawn: in its visible window.
fn row_drawn(cx: &mut gpui::VisualTestContext, id: u64, row: usize) -> bool {
    cx.debug_bounds(selector(format!("hosted_diff_{id}_row_{row}")))
        .is_some()
}

/// A pane opened on a long file, and the same pane given another one, each
/// show the file's first change rather than its first line, as History's
/// diff does.
#[gpui::test]
fn a_pane_shows_the_first_change_of_each_file_it_is_given(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository_of(cx, Arc::new(TestBackend), 400, [300, 250]);
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    let current = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            changes
                .read(app)
                .current()
                .and_then(|pane| pane.target(app))
                .and_then(|target| target.file_path().map(Path::to_path_buf))
        })
    };
    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_a.rs")),
    );
    settle(cx, &view, &store, "a.rs at its change", |cx| {
        store.snapshot().repos[0].diff_sessions.len() == 1
            && store.snapshot().repos[0]
                .diff_sessions
                .keys()
                .all(|id| row_drawn(cx, id.0, 300))
    });
    let first = store.snapshot().repos[0]
        .diff_sessions
        .keys()
        .next()
        .unwrap()
        .0;
    assert!(!row_drawn(cx, first, 0), "not at the top");
    // Opening at a change is not selecting it: the extension is told of
    // no selection, so it offers no line actions.
    let selection = cx.update(|_window, app| {
        changes
            .read(app)
            .current()
            .and_then(|pane| pane.selection(app))
    });
    assert_eq!(selection, None);

    // The same pane, given b.rs.
    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_b.rs")),
    );
    settle(cx, &view, &store, "b.rs in the first pane", |_| {
        store.snapshot().repos[0].diff_sessions.len() == 2
    });
    assert_eq!(current(cx), Some(PathBuf::from("b.rs")));
    settle(cx, &view, &store, "b.rs at its change", |cx| {
        row_drawn(cx, first, 250)
    });
    assert!(!row_drawn(cx, first, 300), "not where a.rs was");

    // A line asked for once the file is in wins over its first change.
    let pane = cx.update(|_window, app| changes.read(app).current().cloned().unwrap());
    cx.update(|_window, app| {
        pane.set_target(
            DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged),
            app,
        );
        let pane_view = pane
            .view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap();
        let renderer = pane_view.read(app).renderer().unwrap().read(app);
        assert!(
            renderer.rendered_diff_target().is_some(),
            "the loading frame keeps the diff and its bar mounted"
        );
    });
    settle(cx, &view, &store, "a.rs again, at its change", |cx| {
        row_drawn(cx, first, 300)
    });
    cx.update(|_window, app| pane.reveal(DiffLineSide::New, 60, app));
    settle(cx, &view, &store, "a.rs at the line asked for", |cx| {
        row_drawn(cx, first, 59)
    });
    for _ in 0..5 {
        publish(cx, &view, store.snapshot());
    }
    assert!(row_drawn(cx, first, 59));
    assert!(
        !row_drawn(cx, first, 300),
        "the first change did not take over"
    );
}

/// A file that is not text shows the renderer's "Binary file" body; the pane
/// adds no line with the file diff's own error above it.
#[gpui::test]
fn a_pane_over_a_file_that_is_not_text_draws_no_error_line(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx) = open_repository(cx);
    std::fs::write(
        dir.path().join("a.rs"),
        b"\x00\x01\x02\xff\xfe\x00 binary \x80\x81\n",
    )
    .unwrap();
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_a.rs")),
    );
    let pane_view = |app: &App| {
        changes
            .read(app)
            .current()
            .unwrap()
            .view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap()
    };
    settle(cx, &view, &store, "a.rs found not to be text", |cx| {
        cx.update(|_window, app| {
            pane_view(app).read(app).renderer().is_some_and(|renderer| {
                matches!(
                    renderer.read(app).file_diff_cache_error,
                    Some(crate::view::panes::diff::diff_cache::FileDiffCacheError::NotText { .. })
                )
            })
        })
    });
    let statuses = cx.update(|_window, app| pane_view(app).read(app).rendered_statuses().to_vec());
    assert_eq!(statuses.last(), Some(&None), "{statuses:?}");
}

/// A patch that fails while its file loads is the one error the renderer,
/// drawing the file, does not show, so the pane draws it.
#[gpui::test]
fn a_pane_draws_a_patch_error_its_file_does_not_show(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository(cx);
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_a.rs")),
    );
    settle(cx, &view, &store, "a.rs loaded", |_| {
        store.snapshot().repos[0]
            .diff_sessions
            .values()
            .any(|session| matches!(session.diff_state.diff_file, Loadable::Ready(Some(_))))
    });
    let mut state = (*store.snapshot()).clone();
    let sessions = Arc::make_mut(&mut state.repos[0].diff_sessions);
    for session in sessions.values_mut() {
        session.diff_state.diff = Loadable::Error("patch failed".into());
        session.rev += 1;
    }
    publish(cx, &view, Arc::new(state));
    let statuses = cx.update(|_window, app| {
        changes
            .read(app)
            .current()
            .unwrap()
            .view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap()
            .read(app)
            .rendered_statuses()
            .to_vec()
    });
    assert_eq!(
        statuses.last(),
        Some(&Some(SharedString::from("patch failed"))),
        "{statuses:?}"
    );
}

/// A pane whose body is a patch has its renderer draw the patch's error, so
/// the pane draws none of its own above it.
#[gpui::test]
fn a_patch_bodied_pane_leaves_its_patch_error_to_the_renderer(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository(cx);
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_a.rs")),
    );
    let pane = cx.update(|_window, app| changes.read(app).current().cloned().unwrap());
    let range = DiffTarget::commit_range(
        gitcomet_core::domain::CommitId("a".repeat(40).into()),
        Some(gitcomet_core::domain::CommitId("b".repeat(40).into())),
        None,
    );
    cx.update(|_window, app| pane.set_target(range.clone(), app));
    // The commits do not exist, so the patch fails to load.
    settle(cx, &view, &store, "the range's patch failed", |_| {
        store.snapshot().repos[0]
            .diff_sessions
            .values()
            .any(|session| {
                session.target == range && matches!(session.diff_state.diff, Loadable::Error(_))
            })
    });
    cx.update(|_window, app| {
        let pane_view = pane
            .view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap();
        let pane_view = pane_view.read(app);
        assert_eq!(
            pane_view.renderer().unwrap().read(app).diff_surface().body,
            crate::view::panes::diff::DiffBody::Patch
        );
        let statuses = pane_view.rendered_statuses();
        assert_eq!(statuses.last(), Some(&None), "{statuses:?}");
    });
}

/// `rust_repository`'s `path` in a Changes pane on its unstaged diff, Inline,
/// compared by Difftastic into `rows` rows.
fn difftastic_file<'a>(
    cx: &'a mut gpui::TestAppContext,
    path: &str,
    rows: usize,
) -> (
    tempfile::TempDir,
    AppStore,
    gpui::Entity<GitCometView>,
    &'a mut gpui::VisualTestContext,
    gpui::Entity<crate::view::panes::diff::DiffRenderer>,
) {
    difftastic_file_in(cx, path, rows, gitcomet_extension_api::DiffLayout::Inline)
}

fn difftastic_file_in<'a>(
    cx: &'a mut gpui::TestAppContext,
    path: &str,
    rows: usize,
    layout: gitcomet_extension_api::DiffLayout,
) -> (
    tempfile::TempDir,
    AppStore,
    gpui::Entity<GitCometView>,
    &'a mut gpui::VisualTestContext,
    gpui::Entity<crate::view::panes::diff::DiffRenderer>,
) {
    let (dir, store, view, cx) = open_repository_in(cx, Arc::new(TestBackend), rust_repository());
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_{path}")),
    );
    let pane = cx.update(|_window, app| changes.read(app).current().cloned().unwrap());
    cx.update(|_window, app| {
        pane.set_target(
            DiffTarget::working_tree(path.into(), DiffArea::Unstaged),
            app,
        );
        pane.set_layout(layout, app);
        pane.set_engine_override(
            Some(gitcomet_core::diff_engine::DiffEngine::Difftastic),
            app,
        );
    });
    let renderer = cx.update(|_window, app| {
        pane.view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap()
            .read(app)
            .renderer()
            .cloned()
            .unwrap()
    });
    settle(cx, &view, &store, "the file compared by Difftastic", |cx| {
        cx.update(|_window, app| {
            let renderer = renderer.read(app);
            renderer.structural_diff_active()
                && renderer
                    .rendered_diff_target()
                    .and_then(DiffTarget::file_path)
                    == Some(Path::new(path))
                && renderer.diff_visible_len() == rows
        })
    });
    (dir, store, view, cx, renderer)
}

/// The visible row of `renderer` whose line numbers are `(old, new)`.
fn row_with_lines(
    renderer: &gpui::Entity<crate::view::panes::diff::DiffRenderer>,
    lines: (Option<u32>, Option<u32>),
    cx: &mut gpui::VisualTestContext,
) -> usize {
    cx.update(|_window, app| {
        let renderer = renderer.read(app);
        (0..renderer.diff_visible_len())
            .find(|&visible| {
                renderer
                    .diff_mapped_ix_for_visible_ix(visible)
                    .and_then(|row| renderer.file_diff_inline_row(row))
                    .is_some_and(|row| (row.old_line, row.new_line) == lines)
            })
            .unwrap_or_else(|| panic!("a row for lines {lines:?}"))
    })
}

fn stage_row(
    renderer: &gpui::Entity<crate::view::panes::diff::DiffRenderer>,
    visible: usize,
    app: &mut App,
) {
    renderer.update(app, |pane, cx| {
        pane.stage_or_unstage_diff_line(visible, gitcomet_core::domain::DiffLineKind::Add, cx)
    })
}

fn staged(dir: &tempfile::TempDir) -> String {
    staged_file(dir, "a.rs")
}

fn staged_file(dir: &tempfile::TempDir, path: &str) -> String {
    String::from_utf8(crate::test_support::git(
        dir.path(),
        &["diff", "--cached", path],
    ))
    .unwrap()
}

/// Staging a line of a Difftastic diff applies its patch at once; there is no
/// review screen to confirm it in.
#[gpui::test]
fn staging_a_difftastic_line_applies_it_without_a_review(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx, renderer) = difftastic_file(cx, "a.rs", 4);
    let added = row_with_lines(&renderer, (None, Some(2)), cx);
    cx.update(|_window, app| stage_row(&renderer, added, app));
    settle(cx, &view, &store, "the added line in the index", |_| {
        staged(&dir).contains("+    let two = 2;")
    });
    assert!(!staged(&dir).contains("pub fn"), "{}", staged(&dir));
}

/// A line changed on one side is one row carrying both line numbers, so
/// staging it replaces the line in the index.
#[gpui::test]
fn staging_a_one_row_difftastic_pair_replaces_its_line(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx, renderer) = difftastic_file(cx, "a.rs", 4);
    let pair = row_with_lines(&renderer, (Some(1), Some(1)), cx);
    cx.update(|_window, app| stage_row(&renderer, pair, app));
    settle(cx, &view, &store, "line 1 replaced in the index", |_| {
        staged(&dir).contains("+pub fn a() -> i32 {")
    });
    assert!(
        staged(&dir).contains("-fn a() -> i32 {"),
        "{}",
        staged(&dir)
    );
    assert!(!staged(&dir).contains("let two"), "{}", staged(&dir));
}

/// Stages line 1 of `path` in Split through the gutter button of `column`,
/// the only one Split draws on a pair changed on one side.
fn stage_split_line_one(
    cx: &mut gpui::TestAppContext,
    path: &str,
    rows: usize,
    column: gitcomet_core::domain::DiffLineKind,
) -> String {
    let (dir, store, view, cx, renderer) =
        difftastic_file_in(cx, path, rows, gitcomet_extension_api::DiffLayout::Split);
    let pair = cx.update(|_window, app| {
        let renderer = renderer.read(app);
        assert_eq!(renderer.diff_view, crate::view::DiffViewMode::Split);
        (0..renderer.diff_visible_len())
            .find(|&visible| {
                renderer
                    .diff_mapped_ix_for_visible_ix(visible)
                    .and_then(|row| renderer.file_diff_split_row(row))
                    .is_some_and(|row| (row.old_line, row.new_line) == (Some(1), Some(1)))
            })
            .expect("the split row of line 1")
    });
    cx.update(|_window, app| {
        renderer.update(app, |pane, cx| {
            pane.stage_or_unstage_diff_line(pair, column, cx)
        })
    });
    settle(cx, &view, &store, "line 1 staged", |_| {
        !staged_file(&dir, path).is_empty()
    });
    staged_file(&dir, path)
}

/// Split draws a pair changed on one side with one gutter button, in the
/// changed side's column. It staged only the new line, so the index kept the
/// old line beside it.
#[gpui::test]
fn staging_a_split_pair_changed_on_the_new_side_replaces_its_line(cx: &mut gpui::TestAppContext) {
    let staged = stage_split_line_one(cx, "a.rs", 4, gitcomet_core::domain::DiffLineKind::Add);
    assert!(staged.contains("-fn a() -> i32 {"), "{staged}");
    assert!(staged.contains("+pub fn a() -> i32 {"), "{staged}");
}

/// The old-side twin: its button staged only the removal, so the index lost
/// the line.
#[gpui::test]
fn staging_a_split_pair_changed_on_the_old_side_replaces_its_line(cx: &mut gpui::TestAppContext) {
    let staged = stage_split_line_one(cx, "b.rs", 3, gitcomet_core::domain::DiffLineKind::Remove);
    assert!(staged.contains("-pub fn b() -> i32 {"), "{staged}");
    assert!(staged.contains("+fn b() -> i32 {"), "{staged}");
}

/// An extension's selected text is the side it asks for, even on a one-row
/// pair that draws only the other side.
#[gpui::test]
fn hosted_selected_text_reads_the_side_asked_for_on_a_one_row_pair(cx: &mut gpui::TestAppContext) {
    let (_dir, _store, _view, cx, renderer) = difftastic_file(cx, "a.rs", 4);
    cx.update(|_window, app| {
        let renderer = renderer.read(app);
        let line_one = |side| gitcomet_extension_api::DiffLineRange {
            side,
            start: 1,
            end: 1,
        };
        assert_eq!(
            renderer.hosted_selected_text(line_one(DiffLineSide::Old)),
            "fn a() -> i32 {"
        );
        assert_eq!(
            renderer.hosted_selected_text(line_one(DiffLineSide::New)),
            "pub fn a() -> i32 {"
        );
    });
}

/// A decor tint asked for without a side follows the side a row draws: a
/// one-row pair drawing its old line takes the old line's tint.
#[gpui::test]
fn a_sideless_decor_tint_follows_the_side_a_row_draws(cx: &mut gpui::TestAppContext) {
    let (_dir, _store, _view, cx, renderer) = difftastic_file(cx, "b.rs", 3);
    let pair = row_with_lines(&renderer, (Some(1), Some(1)), cx);
    cx.update(|_window, app| {
        renderer.update(app, |pane, _| {
            pane.hosted_decor.as_mut().unwrap().options.decor =
                Some(std::rc::Rc::new(|side: DiffLineSide, line: u32| {
                    (side == DiffLineSide::Old && line == 1).then(|| {
                        DiffRowDecor::gutter("●").with_tint(gpui::hsla(0.5, 0.5, 0.5, 1.0))
                    })
                }));
        });
        // Only the old line is tinted, so a tint means the old side was read.
        assert!(renderer.read(app).hosted_row_tint(pair, None).is_some());
    });
}

/// A click while another action is still applying is dropped quietly, as the
/// line engine drops it: the rows still describe the index before that action.
#[gpui::test]
fn a_difftastic_action_while_another_applies_is_dropped_quietly(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx, renderer) = difftastic_file(cx, "a.rs", 4);
    let added = row_with_lines(&renderer, (None, Some(2)), cx);
    let mut busy = (*store.snapshot()).clone();
    busy.repos[0].local_actions_in_flight = 1;
    publish(cx, &view, Arc::new(busy));
    cx.update(|_window, app| stage_row(&renderer, added, app));
    cx.run_until_parked();
    let toasts =
        cx.update(|_window, app| view.read(app).toast_host.read(app).toasts_for_tests(app));
    assert!(toasts.is_empty(), "{toasts:?}");
    assert_eq!(staged(&dir), "");
}

/// F1/F4 and Alt+Left/Right step the hosted pane the user is in, as its
/// own arrows do, and never move History's diff behind the view.
#[gpui::test]
fn the_file_keys_in_an_extension_view_step_its_pane_not_history(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository(cx);
    cx.update(|_window, app| {
        crate::app::bind_app_keys_for_test(app);
        crate::app::install_global_diff_navigation_actions_for_test(app);
    });
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    let current = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            changes
                .read(app)
                .current()
                .and_then(|pane| pane.target(app))
                .and_then(|target| target.file_path().map(Path::to_path_buf))
        })
    };
    // No pane yet: the keys have nothing to act on.
    for key in ["f4", "alt-right", "alt-down"] {
        cx.simulate_keystrokes(key);
        publish(cx, &view, store.snapshot());
    }
    assert!(store.snapshot().repos[0].diff_state.diff_target.is_none());

    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_a.rs")),
    );
    settle(cx, &view, &store, "a.rs in the pane", |cx| {
        store.snapshot().repos[0].diff_sessions.len() == 1
            && row_drawn(cx, first_session(&store), 0)
    });
    assert_eq!(current(cx), Some(PathBuf::from("a.rs")));
    let id = first_session(&store);
    // File arrows work from the selected view's root, outside the diff.
    cx.update(|window, app| {
        let focus = view.read(app).repository_view_focus.clone();
        window.focus(&focus, app);
    });
    for (key, path) in [("alt-right", "b.rs"), ("alt-left", "a.rs")] {
        cx.simulate_keystrokes(key);
        settle(cx, &view, &store, key, |cx| {
            current(cx) == Some(PathBuf::from(path))
        });
    }
    cx.update(|window, app| {
        let focus = app.focus_handle();
        window.focus(&focus, app);
    });
    for (key, path) in [("alt-right", "b.rs"), ("alt-left", "a.rs")] {
        cx.simulate_keystrokes(key);
        settle(cx, &view, &store, key, |cx| {
            current(cx) == Some(PathBuf::from(path))
        });
    }
    // The example also mounts a previous pane after a file step. Keep
    // clicking the original current pane, and wait for its loaded rows.
    settle(cx, &view, &store, "a.rs rows after file steps", |cx| {
        row_drawn(cx, id, 0)
    });
    // Into the diff, then F4 and F1.
    click_debug_selector(cx, selector(format!("hosted_diff_{id}_row_0")));
    publish(cx, &view, store.snapshot());
    cx.simulate_keystrokes("f4");
    settle(cx, &view, &store, "F4 to b.rs", |cx| {
        current(cx) == Some(PathBuf::from("b.rs"))
    });
    cx.simulate_keystrokes("f1");
    settle(cx, &view, &store, "F1 back to a.rs", |cx| {
        current(cx) == Some(PathBuf::from("a.rs"))
    });
    // The owner says where a.rs is, so the bar counts it and the first
    // file's previous arrow is disabled.
    let pane = cx.update(|_window, app| changes.read(app).current().cloned().unwrap());
    let nav = hosted_bar_nav(cx, &pane);
    assert!(
        matches!(nav, Some((Some((0, count)), false, true)) if count >= 2),
        "{nav:?}"
    );
    assert!(
        store.snapshot().repos[0].diff_state.diff_target.is_none(),
        "History's diff did not move"
    );
    // The pane's file arrows are at its foot, in its bottom bar, not in its
    // header; History's Stage is not there.
    let pane = cx
        .debug_bounds(selector(format!("hosted_diff_{id}")))
        .expect("the current pane");
    let next = cx
        .debug_bounds("diff_next_file")
        .expect("the pane's next-file arrow");
    assert!(pane.contains(&next.center()));
    assert!(next.center().y > pane.center().y, "{next:?} in {pane:?}");
    assert!(cx.debug_bounds("diff_bar_stage").is_none());
}

#[gpui::test]
fn alt_change_keys_reach_the_visible_pane_without_diff_focus(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx) = open_repository(cx);
    std::fs::write(
        dir.path().join("a.rs"),
        numbered("a", 30, Some(3)).replace("a 20\n", "a edited 20\n"),
    )
    .unwrap();
    cx.update(|_, app| {
        crate::app::bind_app_keys_for_test(app);
        crate::app::install_global_diff_navigation_actions_for_test(app);
    });
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_a.rs")),
    );
    settle(cx, &view, &store, "the pane's rows", |cx| {
        store.snapshot().repos[0].diff_sessions.len() == 1
            && row_drawn(cx, first_session(&store), 0)
    });
    let pane = cx.update(|_, app| changes.read(app).current().cloned().unwrap());
    let renderer = hosted_renderer(cx, &pane);
    let stops = cx.update(|_, app| renderer.read(app).diff_nav_entries());
    assert_eq!(stops.len(), 2);
    for detached in [false, true] {
        let focus = cx.update(|window, app| {
            renderer.update(app, |pane, _| pane.diff_selection_anchor = Some(stops[0]));
            let focus = if detached {
                app.focus_handle()
            } else {
                view.read(app).repository_view_focus.clone()
            };
            window.focus(&focus, app);
            focus
        });
        publish(cx, &view, store.snapshot());
        for (key, stop) in [("alt-down", stops[1]), ("alt-up", stops[0])] {
            cx.simulate_keystrokes(key);
            publish(cx, &view, store.snapshot());
            assert_eq!(
                cx.update(|_, app| renderer.read(app).diff_selection_anchor),
                Some(stop),
                "{key}, detached focus: {detached}"
            );
            assert!(cx.update(|window, _| focus.is_focused(window)));
        }
    }
    assert!(store.snapshot().repos[0].diff_state.diff_target.is_none());
}

#[gpui::test]
fn comparison_blame_toggles_retries_and_follows_the_file(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::domain::BlameSource;
    let (dir, store, view, cx) = open_repository(cx);
    let from = head(dir.path());
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_a.rs")),
    );
    settle(cx, &view, &store, "the comparison", |cx| {
        store.snapshot().repos[0].diff_sessions.len() == 1
            && row_drawn(cx, first_session(&store), 0)
    });
    let id = gitcomet_state::diff_session::DiffViewId(first_session(&store));
    let pane = cx.update(|_, app| changes.read(app).current().cloned().unwrap());
    let renderer = hosted_renderer(cx, &pane);
    assert!(!cx.update(|_, app| renderer.read(app).annotate_enabled));
    click_debug_selector(cx, "diff_annotate");
    let ready = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            let pane = renderer.read(app);
            pane.annotation_active()
                && pane.blame_matches_rendered_target()
                && matches!(&pane.bound_diff_state(pane.active_repo().unwrap()).blame, Loadable::Ready(lines) if !lines.is_empty())
        })
    };
    settle(cx, &view, &store, "working-tree comparison blame", ready);
    assert_eq!(
        store.snapshot().repos[0].diff_sessions[&id].blame_source,
        Some(BlameSource::WorkingTree(DiffArea::Unstaged))
    );

    // A matching failed load must retry on an explicit off/on toggle.
    let mut failed = (*store.snapshot()).clone();
    let session = Arc::make_mut(&mut failed.repos[0].diff_sessions)
        .get_mut(&id)
        .unwrap();
    session.blame = Loadable::Error("temporary blame failure".into());
    session.rev += 1;
    let failed = Arc::new(failed);
    store.replace_snapshot_for_test(failed.clone());
    publish(cx, &view, failed);
    click_debug_selector(cx, "diff_annotate");
    click_debug_selector(cx, "diff_annotate");
    settle(cx, &view, &store, "blame retry", ready);

    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "edits"]);
    let to = head(dir.path());
    cx.update(|_, app| {
        pane.set_target(
            DiffTarget::commit_range(
                CommitId(from.into()),
                Some(CommitId(to.clone().into())),
                Some("b.rs".into()),
            ),
            app,
        );
    });
    settle(cx, &view, &store, "committed comparison blame", |cx| {
        ready(cx)
            && store.snapshot().repos[0].diff_sessions[&id]
                .blame_path
                .as_deref()
                == Some(Path::new("b.rs"))
    });
    assert_eq!(
        store.snapshot().repos[0].diff_sessions[&id].blame_source,
        Some(BlameSource::Revision(Some(to)))
    );
    assert!(matches!(
        store.snapshot().repos[0].diff_state.blame,
        Loadable::NotLoaded
    ));
}

fn hosted_renderer(
    cx: &mut gpui::VisualTestContext,
    pane: &DiffPane,
) -> gpui::Entity<DiffRenderer> {
    let view = pane
        .view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .unwrap();
    cx.update(|_, app| view.read(app).renderer().cloned().unwrap())
}

#[gpui::test]
fn image_preview_regression_hosted_panes_zoom_without_changing_window_scale(
    cx: &mut gpui::TestAppContext,
) {
    let (dir, store, view, cx) = open_repository(cx);
    let paths = ["first.png", "second.png"];
    for path in paths {
        image::RgbaImage::from_pixel(64, 64, image::Rgba([200, 100, 50, 255]))
            .save(dir.path().join(path))
            .unwrap();
    }
    git(dir.path(), &["add", "first.png", "second.png"]);
    git(dir.path(), &["commit", "-q", "-m", "image fixtures"]);
    for path in paths {
        image::RgbaImage::from_pixel(64, 64, image::Rgba([50, 100, 200, 255]))
            .save(dir.path().join(path))
            .unwrap();
    }
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    for path in paths {
        let file = selector(format!("hosted_file_list_{list_id}_file_{path}"));
        assert!(
            cx.debug_bounds(file).is_some(),
            "image fixture appears in the hosted list"
        );
        click_debug_selector(cx, file);
        // The second pick moves the first into a new previous pane, which
        // loads on its own; wait for both before wheeling over them.
        settle(cx, &view, &store, "hosted image previews", |cx| {
            cx.update(|_, app| {
                let changes = changes.read(app);
                changes
                    .current()
                    .into_iter()
                    .chain(changes.previous())
                    .all(|pane| {
                        pane.view()
                            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
                            .unwrap()
                            .read(app)
                            .renderer()
                            .is_some_and(|renderer| renderer.read(app).image_preview_active)
                    })
            })
        });
    }
    let panes = cx.update(|_, app| {
        let changes = changes.read(app);
        [
            changes.current().cloned().unwrap(),
            changes.previous().cloned().unwrap(),
        ]
    });
    let before = cx.update(|_, app| crate::ui_scale::current(app).percent);
    for (pane, modifiers) in panes.iter().zip([
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        gpui::Modifiers::secondary_key(),
    ]) {
        let renderer = hosted_renderer(cx, pane);
        cx.update(|_, app| {
            renderer.update(app, |pane, cx| {
                pane.image_zoom(crate::view::image_preview::Zoom::Scale(1.0), cx)
            });
        });
        crate::view::test_support::redraw(cx);
        let id = cx.update(|_, app| {
            pane.view()
                .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
                .unwrap()
                .read(app)
                .view_id()
        });
        let bounds = cx
            .debug_bounds(selector(format!("hosted_diff_{id}")))
            .unwrap();
        let position =
            bounds.origin + gpui::point(bounds.size.width * 0.75, bounds.size.height * 0.6);
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(60.0))),
            modifiers,
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.update(|_, app| {
            assert_eq!(
                crate::ui_scale::current(app).percent,
                before,
                "image wheel zoom must leave application scale unchanged"
            );
            assert_eq!(
                renderer
                    .read(app)
                    .image_preview_group
                    .read(app)
                    .zoom_label(),
                "125%"
            );
        });
    }
    // A retained hosted pane has old bounds after switching back to History;
    // those bounds must no longer claim wheel events.
    let id = cx.update(|_, app| {
        panes[0]
            .view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap()
            .read(app)
            .view_id()
    });
    let bounds = cx
        .debug_bounds(selector(format!("hosted_diff_{id}")))
        .unwrap();
    let hidden_position =
        bounds.origin + gpui::point(bounds.size.width * 0.75, bounds.size.height * 0.6);
    click_debug_selector(cx, "repository_view_history");
    crate::view::test_support::redraw(cx);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: hidden_position,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(60.0))),
        modifiers: gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|_, app| {
        assert_eq!(
            crate::ui_scale::current(app).percent,
            crate::ui_scale::step_up(before)
        )
    });
}

/// A bar's owner position, and whether each file arrow is enabled.
type BarNav = (Option<(usize, usize)>, bool, bool);

/// What a hosted pane's bar shows.
fn hosted_bar_nav(cx: &mut gpui::VisualTestContext, pane: &DiffPane) -> Option<BarNav> {
    let pane_view = pane
        .view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .ok()?;
    cx.update(|_window, app| {
        let renderer = pane_view.read(app).renderer()?.clone();
        renderer.update(app, |pane, cx| {
            let repo_id = pane.active_repo_id();
            pane.diff_bar_nav(repo_id, cx)
                .map(|nav| (nav.position, nav.can_prev, nav.can_next))
        })
    })
}

fn first_session(store: &AppStore) -> u64 {
    store.snapshot().repos[0]
        .diff_sessions
        .keys()
        .next()
        .map_or(0, |id| id.0)
}

#[gpui::test]
fn snapshot_panes_diff_texts_without_a_repository(cx: &mut gpui::TestAppContext) {
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    publish(cx, &view, Arc::new(AppState::test_default()));
    let host = cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let pane = cx.update(|_window, app| {
        host.create_snapshot_pane(
            DiffSnapshot::new("lib.rs", "fn a() {}\nold\n", "fn a() {}\nnew\nmore\n"),
            DiffPaneOptions::default(),
            app,
        )
        .unwrap()
    });
    cx.run_until_parked();
    assert!(!cx.update(|_window, app| pane.is_loading(app)));
    assert_eq!(cx.update(|_window, app| pane.target(app)), None);
    assert_eq!(rows_with(cx, &pane, "new"), 1);

    // Targets are for repository panes; the snapshot stays.
    cx.update(|_window, app| {
        pane.set_target(
            gitcomet_core::domain::DiffTarget::working_tree("x.rs".into(), DiffArea::Unstaged),
            app,
        )
    });
    cx.run_until_parked();
    assert_eq!(rows_with(cx, &pane, "more"), 1);

    cx.update(|_window, app| pane.set_snapshot(DiffSnapshot::new("lib.rs", "one\n", "two\n"), app));
    cx.run_until_parked();
    assert_eq!(rows_with(cx, &pane, "more"), 0);
    assert_eq!(rows_with(cx, &pane, "two"), 1);
}

#[gpui::test]
fn hosted_raw_bytes_finish_across_metadata_updates_and_reject_replaced_content(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_extension_api::{DiffLayout, DiffPaneEvent};
    let (_dir, store, view, cx) = open_repository(cx);
    let loaded = std::rc::Rc::new(std::cell::Cell::new(0));
    let on_loaded = loaded.clone();
    let pane = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        host.create_diff_pane(
            &repository,
            DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged),
            DiffPaneOptions {
                layout: DiffLayout::Inline,
                on_event: Some(std::rc::Rc::new(move |event, _| {
                    if matches!(event, DiffPaneEvent::Loaded) {
                        on_loaded.set(on_loaded.get() + 1);
                    }
                })),
                ..Default::default()
            },
            app,
        )
        .unwrap()
    });
    settle(cx, &view, &store, "initial byte index", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    let renderer = hosted_renderer(cx, &pane);
    let binding = cx.update(|_, app| renderer.read(app).store.binding.unwrap());
    let mut state = (*store.snapshot()).clone();
    let publish_without_running_tasks = |cx: &mut gpui::VisualTestContext, state: AppState| {
        cx.update(|_, app| {
            view.read(app).ui_model.clone().update(app, |model, cx| {
                model.set_state(Arc::new(state), cx);
            });
        });
    };
    let replace_content = |state: &mut AppState, text: &str| {
        let session = Arc::make_mut(&mut state.repos[0].diff_sessions)
            .get_mut(&binding.view)
            .unwrap();
        session.diff_file =
            Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
                "a.rs".into(),
                Some("before\n".into()),
                Some(text.into()),
            ))));
        session.diff = Loadable::NotLoaded;
        session.diff_file_rev += 1;
        session.diff_rev += 1;
        session.rev += 1;
    };
    loaded.set(0);
    replace_content(&mut state, "same cache\n");
    publish_without_running_tasks(cx, state.clone());
    let cache = cx.update(|_, app| {
        assert!(pane.is_loading(app), "raw indexing has not completed");
        assert!(pane.selected_bytes(app).is_none());
        renderer
            .read(app)
            .hosted_decor
            .as_ref()
            .unwrap()
            .file_cache
            .clone()
            .unwrap()
    });
    let session = Arc::make_mut(&mut state.repos[0].diff_sessions)
        .get_mut(&binding.view)
        .unwrap();
    session.blame = Loadable::Ready(Arc::new(Vec::new()));
    session.rev += 1;
    publish_without_running_tasks(cx, state.clone());
    cx.update(|_, app| {
        assert!(
            pane.is_loading(app),
            "metadata arrived before task completion"
        );
        assert!(Arc::ptr_eq(
            &cache,
            renderer
                .read(app)
                .hosted_decor
                .as_ref()
                .unwrap()
                .file_cache
                .as_ref()
                .unwrap(),
        ));
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        assert!(
            !pane.is_loading(app),
            "the same cache's byte index must finish"
        );
        assert_eq!(loaded.get(), 1);
        renderer.update(app, |pane, _| {
            pane.ensure_diff_visible_indices();
            assert!(pane.select_hosted_row(1, false));
        });
        assert_eq!(pane.selected_bytes(app).unwrap().as_ref(), b"same cache\n");
    });

    loaded.set(0);
    replace_content(&mut state, "stale content\n");
    publish_without_running_tasks(cx, state.clone());
    cx.update(|_, app| assert!(pane.is_loading(app)));
    replace_content(&mut state, "current content\n");
    publish_without_running_tasks(cx, state);
    cx.update(|_, app| {
        assert!(pane.is_loading(app));
        assert!(pane.selected_bytes(app).is_none());
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        assert!(!pane.is_loading(app));
        assert_eq!(loaded.get(), 1, "replaced content must never report Loaded");
        renderer.update(app, |pane, _| {
            pane.ensure_diff_visible_indices();
            assert!(pane.select_hosted_row(1, false));
        });
        assert_eq!(
            pane.selected_bytes(app).unwrap().as_ref(),
            b"current content\n"
        );
    });
}

#[gpui::test]
fn hosted_search_counts_raw_source_lines_without_changing_navigation(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::Diff;
    use gitcomet_extension_api::DiffLayout;
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let target = DiffTarget::working_tree("search.txt".into(), DiffArea::Unstaged);
    let snapshots = [
        DiffSnapshot::new(
            "search.txt",
            "ÉCLAIR\tneedle old\nsame same\rcarriage\r\n",
            "éclair\tneedle new\nsame same\rcarriage\r\n",
        ),
        DiffSnapshot::from_patch(
            "search.txt",
            Diff::from_unified(
                target,
                "diff --git a/search.txt b/search.txt\n@@ -1,2 +1,2 @@\n-ÉCLAIR\tneedle old\n+éclair\tneedle new\n same same\rcarriage\r\n",
            ),
        ),
    ];
    // The existing indexed file rows strip LF only; parsed patch rows already
    // stripped CRLF. Search keeps the bytes each source actually supplied.
    for (snapshot, terminal_cr_matches) in snapshots.into_iter().zip([1, 0]) {
        let pane = cx.update(|_, app| {
            view.read(app)
                .extension_window
                .as_ref()
                .unwrap()
                .host()
                .create_snapshot_pane(
                    snapshot,
                    DiffPaneOptions {
                        layout: DiffLayout::Split,
                        ..Default::default()
                    },
                    app,
                )
                .unwrap()
        });
        cx.run_until_parked();
        let renderer = hosted_renderer(cx, &pane);
        for (query, count) in [
            ("needle", 2),
            ("ÉCLAIR", 2),
            ("éclair", 2),
            ("\tneedle", 2),
            ("    needle", 0),
            ("+éclair", 0),
            ("same", 1),
            ("same\rcarriage", 1),
            ("same\ncarriage", 0),
            ("carriage\r", terminal_cr_matches),
            ("carriage\r\n", 0),
            ("old\néclair", 0),
            ("missing", 0),
            ("", 0),
        ] {
            cx.update(|_, app| {
                pane.set_search(query, app);
                let navigation = {
                    let renderer = renderer.read(app);
                    (
                        renderer.diff_search_matches.clone(),
                        renderer.diff_search_match_ix,
                        renderer.diff_search_query.clone(),
                        renderer
                            .diff_row_provider
                            .as_ref()
                            .map(|rows| rows.materialized_row_count()),
                    )
                };
                assert_eq!(pane.search_matches(app), count, "query {query:?}");
                let _ = gitcomet_core::text_search::take_text_search_matchers_built_for_tests();
                assert_eq!(pane.search_matches(app), count);
                assert_eq!(
                    gitcomet_core::text_search::take_text_search_matchers_built_for_tests(),
                    0,
                    "repeated public count reads reuse the source result"
                );
                let renderer = renderer.read(app);
                assert_eq!(renderer.diff_search_matches, navigation.0);
                assert_eq!(renderer.diff_search_match_ix, navigation.1);
                assert_eq!(renderer.diff_search_query, navigation.2);
                assert_eq!(
                    renderer
                        .diff_row_provider
                        .as_ref()
                        .map(|rows| rows.materialized_row_count()),
                    navigation.3,
                    "source counting does not materialize patch pages"
                );
            });
        }
        assert_eq!(rows_with(cx, &pane, "needle"), 2);
        cx.update(|_, app| {
            renderer
                .read(app)
                .diff_search_input
                .clone()
                .update(app, |input, cx| input.set_text("toolbar-only query", cx));
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(
                renderer.read(app).diff_search_query.as_ref(),
                "toolbar-only query"
            );
            assert_eq!(
                pane.search_matches(app),
                2,
                "the public count retains the public setter's query"
            );
        });
    }
}

#[gpui::test]
fn hosted_search_keeps_diff_source_while_editing_and_previewing(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx) = open_repository(cx);
    std::fs::write(dir.path().join("a.rs"), "needle needle\n").unwrap();
    let (pane, renderer) = hosted_editor(cx, &view, &store);
    assert!(cx.update(|_, app| renderer.read(app).is_file_editor_active()));
    assert_eq!(rows_with(cx, &pane, "needle"), 1, "one source line");
    assert_eq!(rows_with(cx, &pane, "a 0"), 1, "old-side source survives");
    edit_hosted(&renderer, "unsaved unsaved\n", cx);
    assert_eq!(rows_with(cx, &pane, "needle"), 1);
    assert_eq!(rows_with(cx, &pane, "unsaved"), 0);

    cx.update(|_, app| {
        renderer.read(app).store.dispatch(Msg::OpenFileContent {
            repo_id: RepoId(1),
            source: gitcomet_core::domain::FileSource::WorkingDirectory,
            path: "a.rs".into(),
        });
    });
    settle(cx, &view, &store, "content preview", |cx| {
        cx.update(|_, app| renderer.read(app).is_file_preview_active())
    });
    assert_eq!(rows_with(cx, &pane, "needle"), 1);
    assert_eq!(rows_with(cx, &pane, "a 0"), 1);
    assert_eq!(rows_with(cx, &pane, "unsaved"), 0);
}

#[gpui::test]
fn hosted_search_keeps_raw_markdown_while_navigation_uses_rendered_text(
    cx: &mut gpui::TestAppContext,
) {
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_, app| {
        view.read(app)
            .extension_window
            .as_ref()
            .unwrap()
            .host()
            .create_snapshot_pane(
                DiffSnapshot::new("notes.md", "# Before\n", "# After\n"),
                DiffPaneOptions::default(),
                app,
            )
            .unwrap()
    });
    cx.run_until_parked();
    let renderer = hosted_renderer(cx, &pane);
    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            renderer.rendered_preview_modes.set(
                crate::view::RenderedPreviewKind::Markdown,
                crate::view::RenderedPreviewMode::Rendered,
            );
            renderer.ensure_file_markdown_preview_cache(cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        let renderer = renderer.read(app);
        assert!(renderer.rendered_markdown_preview_owns_view());
        assert!(renderer.markdown_search_surface().is_some());
    });
    assert_eq!(rows_with(cx, &pane, "# After"), 1);
    cx.update(|_, app| {
        assert!(renderer.read(app).diff_search_matches.is_empty());
    });
    assert_eq!(rows_with(cx, &pane, "# Before"), 1);
    assert_eq!(rows_with(cx, &pane, "After"), 1);
}

#[gpui::test]
fn snapshot_markdown_images_resolve_relative_and_absolute_document_paths(
    test_cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(test_cx);
    let cwd = std::env::current_dir().unwrap();
    let directory = tempfile::tempdir_in(&cwd).unwrap();
    let docs = directory.path().join("docs");
    std::fs::create_dir(&docs).unwrap();
    let picture = docs.join("picture.svg");
    std::fs::write(&picture,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"26\" height=\"26\"><rect width=\"26\" height=\"26\"/></svg>",
    ).unwrap();
    let document = docs.join("readme.md");
    install_example(test_cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        test_cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let host = cx.update(|_, app| view.read(app).extension_window.as_ref().unwrap().host());
    let panes = [document.strip_prefix(&cwd).unwrap().to_path_buf(), document].map(|path| {
        let pane = cx.update(|_, app| {
            host.create_snapshot_pane(
                DiffSnapshot::new(path.clone(), "", "![local](picture.svg)\n"),
                DiffPaneOptions::default(),
                app,
            )
            .unwrap()
        });
        let renderer = cx.update(|window, app| {
            pane.view()
                .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
                .unwrap()
                .update(app, |pane, cx| pane.renderer_for_test(window, cx))
        });
        cx.update(|_, app| {
            assert!(
                renderer.read(app).markdown_preview_image_root().is_some(),
                "snapshot document image root exists before awaiting image work"
            );
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(
                renderer.read(app).file_diff_cache_path.as_ref(),
                Some(&path),
                "preparation and rendering use the same snapshot path identity"
            );
        });
        cx.update(|_, app| {
            renderer.update(app, |renderer, cx| {
                renderer
                    .rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                renderer.ensure_file_markdown_preview_cache(cx);
                cx.notify();
            })
        });
        (pane, renderer)
    });
    for (pane, renderer) in panes {
        let (_, pane_cx) = test_cx.add_window_view(|_, _| PaneHolder(pane.view()));
        for _ in 0..3 {
            pane_cx.run_until_parked();
            crate::view::test_support::redraw(pane_cx);
        }
        pane_cx.update(|_, app| {
            let renderer = renderer.read(app);
            assert!(renderer.active_repo().is_none());
            assert!(renderer.rendered_markdown_preview_owns_view());
            let root = renderer
                .markdown_preview_image_root()
                .expect("snapshot document image root");
            let resolved =
                crate::view::rows::markdown_preview_image_source(Some(&root), "picture.svg");
            let Some(crate::view::rows::MarkdownPreviewImageSource::File(resolved)) = resolved
            else {
                panic!("local snapshot image should resolve");
            };
            assert_eq!(
                resolved.canonicalize().unwrap(),
                picture.canonicalize().unwrap()
            );
            assert!(
                crate::view::rows::markdown_preview_image_source(Some(&root), ".git/picture.svg")
                    .is_none()
            );
        });
        let bounds = pane_cx
            .debug_bounds("markdown_preview_block_image_0")
            .expect("local snapshot image is drawn");
        assert!(bounds.size.width > px(0.0) && bounds.size.height > px(0.0));
    }
}

#[gpui::test]
fn whole_commit_panes_do_not_bind_contributions_to_ambiguous_file_lines(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_extension_api::{DiffAnnotation, DiffAnnotations, DiffInset, DiffLayout};
    let (dir, store, view, cx) = open_repository(cx);
    let pane = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        let pane = host
            .create_diff_pane(
                &repository,
                DiffTarget::commit_range(CommitId(head(dir.path()).into()), None, None),
                DiffPaneOptions {
                    layout: DiffLayout::Inline,
                    ..Default::default()
                },
                app,
            )
            .unwrap();
        pane.set_insets(
            vec![DiffInset::new(
                DiffLineSide::New,
                4,
                ["ambiguous note".into()],
            )],
            app,
        );
        pane.set_annotations(
            DiffAnnotations::new().with(DiffLineSide::New, 4, DiffAnnotation::new(gpui::red())),
            app,
        );
        pane
    });
    settle(cx, &view, &store, "whole-commit patch", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    assert_eq!(rows_with(cx, &pane, "a edited"), 1);
    assert_eq!(rows_with(cx, &pane, "b edited"), 1);
    let renderer = hosted_renderer(cx, &pane);
    cx.update(|_, app| {
        renderer.update(app, |renderer, _| {
            let rows = renderer.diff_visible_len();
            assert!(rows > 0);
            assert_eq!(
                renderer.diff_list_len(),
                rows,
                "an ambiguous file line admits no inset"
            );
            assert!(renderer.hosted_markers().is_empty());
            for row in 0..rows {
                assert_eq!(renderer.hosted_line_numbers(row), (None, None));
                renderer.select_hosted_row(row, false);
                assert!(renderer.hosted_selection().is_none());
            }
        });
        assert!(pane.selection(app).is_none());
        assert!(pane.selected_text(app).is_none());
        assert!(pane.selected_bytes(app).is_none());
    });
}

/// A pane left at the default layout opens in the user's Inline/Split
/// setting; an explicit layout wins, and `Preferred` reads the setting again.
#[gpui::test]
fn a_pane_at_the_default_layout_follows_the_diff_setting(cx: &mut gpui::TestAppContext) {
    use crate::view::DiffViewMode;
    use gitcomet_extension_api::DiffLayout;
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, app_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    publish(app_cx, &view, Arc::new(AppState::test_default()));
    // The setting's own default is Split; the user chose Inline.
    app_cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.set_diff_view_mode(DiffViewMode::Inline, cx)
        })
    });
    let host =
        app_cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let pane = app_cx.update(|_window, app| {
        host.create_snapshot_pane(
            DiffSnapshot::new("lib.rs", "old\n", "new\n"),
            DiffPaneOptions::default(),
            app,
        )
        .unwrap()
    });
    let view_any = pane.view();
    let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(view_any));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    let entity = pane
        .view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .unwrap();
    let mode = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| entity.read(app).view_mode_for_test(app))
    };
    assert_eq!(mode(cx), Some(DiffViewMode::Inline), "as the user set it");

    cx.update(|_window, app| pane.set_layout(DiffLayout::Split, app));
    assert_eq!(
        mode(cx),
        Some(DiffViewMode::Split),
        "an explicit layout wins"
    );
    cx.update(|_window, app| pane.set_layout(DiffLayout::Preferred, app));
    assert_eq!(mode(cx), Some(DiffViewMode::Inline));
}

/// Commits a rename with an edit, an addition, a deletion, and a binary
/// change on top of an initial commit; returns the second commit's id.
fn commit_every_kind_of_change(root: &Path) -> String {
    let lines = |edit: bool| -> String {
        (0..20)
            .map(|n| {
                if edit && n == 10 {
                    format!("row {n:02} moved\n")
                } else {
                    format!("row {n:02}\n")
                }
            })
            .collect()
    };
    std::fs::write(root.join("orig.rs"), lines(false)).unwrap();
    std::fs::write(root.join("gone.rs"), "gone 1\ngone 2\n").unwrap();
    std::fs::write(root.join("img.bin"), [0u8, 1, 2, 3, 0, 5]).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "before"]);
    git(root, &["mv", "orig.rs", "moved.rs"]);
    std::fs::write(root.join("moved.rs"), lines(true)).unwrap();
    git(root, &["rm", "-q", "gone.rs"]);
    std::fs::write(root.join("new.rs"), "fresh 1\nfresh 2\n").unwrap();
    std::fs::write(root.join("img.bin"), [0u8, 9, 9, 9, 0, 5]).unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "after"]);
    head(root)
}

#[gpui::test]
fn panes_show_renames_additions_deletions_and_binaries_while_history_keeps_its_diff(
    cx: &mut gpui::TestAppContext,
) {
    let (dir, store, view, cx) = open_repository(cx);
    let head = commit_every_kind_of_change(dir.path());
    let (host, repository) = cx.update(|_window, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        (host, repository)
    });

    // History shows a file of its own throughout.
    let history_target =
        gitcomet_core::domain::DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged);
    store.dispatch(Msg::SelectDiff {
        repo_id: RepoId(1),
        target: history_target.clone(),
    });
    settle(cx, &view, &store, "History's diff", |_| {
        store.snapshot().repos[0].diff_state.diff_target.as_ref() == Some(&history_target)
    });
    let history_rev = store.snapshot().repos[0].diff_state.diff_target_rev;

    let picked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = std::rc::Rc::clone(&picked);
    let list = cx.update(|_window, app| {
        host.create_file_list(
            &repository,
            ChangeSource::Commit(gitcomet_core::domain::CommitId(head.clone().into())),
            move |change, target, _| seen.borrow_mut().push((change.clone(), target)),
            app,
        )
        .unwrap()
    });
    settle(cx, &view, &store, "the commit's files", |cx| {
        cx.update(|_window, app| !list.is_loading(app))
    });
    let kinds: Vec<_> = cx.update(|_window, app| {
        list.files(app)
            .into_iter()
            .map(|change| (change.path.to_string_lossy().into_owned(), change.kind))
            .collect()
    });
    use gitcomet_core::domain::FileStatusKind as Kind;
    for expected in [
        ("gone.rs", Kind::Deleted),
        ("img.bin", Kind::Modified),
        ("moved.rs", Kind::Renamed),
        ("new.rs", Kind::Added),
    ] {
        assert!(
            kinds.contains(&(expected.0.to_string(), expected.1)),
            "{expected:?} in {kinds:?}"
        );
    }

    for path in ["gone.rs", "img.bin", "moved.rs", "new.rs"] {
        cx.update(|_window, app| assert!(list.select_path(Path::new(path), app)));
    }
    let picks = picked.borrow().clone();
    let panes: Vec<(String, DiffPane)> = cx.update(|_window, app| {
        picks
            .into_iter()
            .map(|(change, target)| {
                let pane = host
                    .create_diff_pane(&repository, target, DiffPaneOptions::default(), app)
                    .unwrap();
                (change.path.to_string_lossy().into_owned(), pane)
            })
            .collect()
    });
    settle(cx, &view, &store, "every pane", |cx| {
        cx.update(|_window, app| panes.iter().all(|(_, pane)| !pane.is_loading(app)))
    });
    let pane = |path: &str| &panes.iter().find(|(shown, _)| shown == path).unwrap().1;
    // The rename shows its edit: both sides of row 10, not a whole-file add.
    assert_eq!(rows_with(cx, pane("moved.rs"), "row 10"), 2);
    assert_eq!(rows_with(cx, pane("new.rs"), "fresh"), 2);
    assert_eq!(rows_with(cx, pane("gone.rs"), "gone"), 2);

    let history = &store.snapshot().repos[0].diff_state;
    assert_eq!(history.diff_target.as_ref(), Some(&history_target));
    assert_eq!(history.diff_target_rev, history_rev);
}

/// Mounts a pane's view as a window of its own.
struct PaneHolder(gpui::AnyView);

#[gpui::test]
fn diff_engine_override_is_local_and_survives_snapshot_navigation(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::diff_engine::DiffEngine;
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, app_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let (pane, other) = app_cx.update(|_, app| {
        let host = root.read(app).extension_window.as_ref().unwrap().host();
        let snapshot = || DiffSnapshot::from_patch("file.rs", gitcomet_core::domain::Diff::from_unified(
            DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged), "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new\n"));
        (host.create_snapshot_pane(snapshot(), DiffPaneOptions::default(), app).unwrap(),
         host.create_snapshot_pane(snapshot(), DiffPaneOptions::default(), app).unwrap())
    });
    let any = pane.view();
    let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(any));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        root.update(app, |root, cx| {
            root.set_diff_engine(DiffEngine::Difftastic, cx)
        });
    });
    cx.run_until_parked();
    let entity = pane
        .view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .unwrap();
    let engine = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            entity
                .read(app)
                .renderer()
                .unwrap()
                .read(app)
                .effective_diff_engine()
        })
    };
    assert_eq!(engine(cx), DiffEngine::Difftastic);
    cx.update(|_, app| pane.set_engine_override(Some(DiffEngine::LineByLine), app));
    cx.run_until_parked();
    assert_eq!(engine(cx), DiffEngine::LineByLine);
    cx.update(|_, app| {
        pane.set_snapshot(DiffSnapshot::from_patch("next.rs", gitcomet_core::domain::Diff::from_unified(
            DiffTarget::working_tree("next.rs".into(), DiffArea::Unstaged), "diff --git a/next.rs b/next.rs\n--- a/next.rs\n+++ b/next.rs\n@@ -1 +1 @@\n-a\n+b\n")), app);
        other.set_engine_override(Some(DiffEngine::Difftastic), app);
    });
    cx.run_until_parked();
    assert_eq!(engine(cx), DiffEngine::LineByLine);
    cx.update(|_, app| pane.set_engine_override(None, app));
    cx.run_until_parked();
    assert_eq!(engine(cx), DiffEngine::Difftastic);
}

impl Render for PaneHolder {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

fn click_with(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
    modifiers: gpui::Modifiers,
) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} to be rendered"))
        .center();
    cx.simulate_mouse_move(center, None, modifiers);
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, modifiers);
}

fn review_hosted_panes(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
    store: &AppStore,
) -> [DiffPane; 2] {
    let panes = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        let options = DiffPaneOptions {
            layout: gitcomet_extension_api::DiffLayout::Inline,
            ..Default::default()
        };
        [
            host.create_diff_pane(
                &repository,
                DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged),
                options.clone(),
                app,
            )
            .unwrap(),
            host.create_snapshot_pane(
                DiffSnapshot::new("snapshot.rs", "before\n", "after\n"),
                options,
                app,
            )
            .unwrap(),
        ]
    });
    settle(cx, view, store, "repository and snapshot panes", |cx| {
        cx.update(|_, app| panes.iter().all(|pane| !pane.is_loading(app)))
    });
    panes
}

#[gpui::test]
fn hosted_layout_changes_leave_window_preferences_alone(test_cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository(test_cx);
    cx.update(|_, app| {
        view.update(app, |root, cx| {
            root.set_diff_view_mode(DiffViewMode::Inline, cx)
        });
    });
    let panes = review_hosted_panes(cx, &view, &store);
    for pane in panes {
        let (_, cx) = test_cx.add_window_view(|_, _| PaneHolder(pane.view()));
        for _ in 0..3 {
            crate::view::test_support::redraw(cx);
            cx.run_until_parked();
        }
        click_with(cx, "diff_split", Default::default());
        cx.run_until_parked();
        let renderer = hosted_renderer(cx, &pane);
        cx.update(|_, app| {
            assert_eq!(renderer.read(app).diff_view, DiffViewMode::Split);
            let root = view.read(app);
            assert_eq!(root.diff_renderer.read(app).diff_view, DiffViewMode::Inline);
            assert_eq!(
                root.ui_model.read(app).preferences.diff.view_mode,
                DiffViewMode::Inline,
                "a hosted layout choice belongs to that pane"
            );
        });
    }
}

#[gpui::test]
fn hosted_settings_follow_the_invoker_and_restore_the_primary_pane(
    test_cx: &mut gpui::TestAppContext,
) {
    use crate::view::panels::ContextMenuAction;
    let (_dir, store, view, cx) = open_repository(test_cx);
    let panes = review_hosted_panes(cx, &view, &store);
    for pane in panes {
        let (_, cx) = test_cx.add_window_view(|_, _| PaneHolder(pane.view()));
        for _ in 0..3 {
            crate::view::test_support::redraw(cx);
            cx.run_until_parked();
        }
        let renderer = hosted_renderer(cx, &pane);
        let (primary, popover, primary_wrap) = cx.update(|_, app| {
            let root = view.read(app);
            (
                root.diff_renderer.clone(),
                root.popover_host.clone(),
                root.diff_renderer.read(app).diff_word_wrap,
            )
        });
        cx.update(|_, app| {
            renderer.update(app, |pane, _| pane.diff_word_wrap = primary_wrap);
        });
        click_with(cx, "diff_action_menu", Default::default());
        cx.run_until_parked();
        cx.update(|window, app| {
            popover.update(app, |host, cx| {
                host.context_menu_activate_action(
                    ContextMenuAction::SetDiffWordWrap {
                        enabled: !primary_wrap,
                    },
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(renderer.read(app).diff_word_wrap, !primary_wrap);
            assert_eq!(primary.read(app).diff_word_wrap, primary_wrap);
            popover.update(app, |host, cx| host.close_popover(cx));
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            // A closed host resumes its primary command destination.
            popover.update(app, |host, cx| {
                host.context_menu_activate_action(
                    ContextMenuAction::SetDiffWordWrap {
                        enabled: !primary_wrap,
                    },
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            assert_eq!(
                primary.read(app).diff_word_wrap,
                !primary_wrap,
                "close restored the primary destination"
            );
            renderer.update(app, |pane, cx| {
                pane.open_popover_for_bounds(
                    PopoverKind::DiffActionMenu,
                    gpui::Bounds::new(point(px(20.0), px(20.0)), gpui::size(px(20.0), px(20.0))),
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            popover.update(app, |host, cx| {
                host.context_menu_activate_action(
                    ContextMenuAction::SetDiffWordWrap {
                        enabled: primary_wrap,
                    },
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(
                renderer.read(app).diff_word_wrap,
                primary_wrap,
                "bounds routing reached the hosted renderer"
            );
            assert_eq!(primary.read(app).diff_word_wrap, !primary_wrap);
            popover.update(app, |host, cx| host.close_popover(cx));
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            popover.update(app, |host, cx| {
                host.context_menu_activate_action(
                    ContextMenuAction::SetDiffWordWrap {
                        enabled: primary_wrap,
                    },
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        cx.update(|_, app| assert_eq!(primary.read(app).diff_word_wrap, primary_wrap));
    }
}

#[gpui::test]
fn hosted_text_selection_obeys_the_live_policy_for_repository_and_snapshot_panes(
    test_cx: &mut gpui::TestAppContext,
) {
    let (_dir, store, view, cx) = open_repository_of(test_cx, Arc::new(TestBackend), 2, [0, 1]);
    let panes = review_hosted_panes(cx, &view, &store);
    for pane in panes {
        let (_, cx) = test_cx.add_window_view(|_, _| PaneHolder(pane.view()));
        for _ in 0..3 {
            crate::view::test_support::redraw(cx);
            cx.run_until_parked();
        }
        let renderer = hosted_renderer(cx, &pane);
        for select_lines in [false, true] {
            cx.update(|_, app| {
                let mut policy = DiffPanePolicy::read_only();
                policy.select_lines = select_lines;
                pane.set_policy(policy, app);
            });
            let click = cx.update(|_, app| {
                let renderer = renderer.read(app);
                let hit = renderer
                    .diff_text_hitboxes
                    .values()
                    .find(|hit| hit.text_len > 0)
                    .unwrap();
                point(hit.bounds.left() + px(1.0), hit.bounds.center().y)
            });
            cx.simulate_event(gpui::MouseDownEvent {
                position: click,
                modifiers: Default::default(),
                button: gpui::MouseButton::Left,
                click_count: 3,
                first_mouse: false,
            });
            cx.simulate_mouse_up(click, gpui::MouseButton::Left, Default::default());
            cx.update(|_, app| {
                assert_eq!(pane.selection(app).is_some(), select_lines);
                assert_eq!(pane.selected_text(app).is_some(), select_lines);
                renderer.update(app, |renderer, _| renderer.clear_diff_selection_state());
            });
            let eof = cx
                .debug_bounds("diff_text_empty_space_Inline")
                .unwrap()
                .center();
            cx.simulate_mouse_down(eof, gpui::MouseButton::Left, Default::default());
            cx.update(|_, app| {
                assert_eq!(renderer.read(app).diff_text_selecting, select_lines);
            });
            cx.simulate_mouse_up(eof, gpui::MouseButton::Left, Default::default());
            cx.update(|_, app| {
                renderer.update(app, |renderer, _| renderer.clear_diff_selection_state());
            });
        }
    }
}

#[gpui::test]
fn pane_contributions_annotate_act_and_inset_without_touching_file_lines(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_extension_api::{
        DiffAnnotation, DiffAnnotations, DiffInset, DiffLegendItem, DiffLineRange,
        DiffSelectionAction,
    };
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, app_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    publish(app_cx, &view, Arc::new(AppState::test_default()));
    let host =
        app_cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());

    let gutter_clicks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let acted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (on_gutter, on_action) = (
        std::rc::Rc::clone(&gutter_clicks),
        std::rc::Rc::clone(&acted),
    );
    let pane = app_cx.update(|_window, app| {
        host.create_snapshot_pane(
            DiffSnapshot::new("notes.txt", "one\ntwo\nthree\n", "one\nTWO\nthree\n"),
            DiffPaneOptions {
                on_gutter_click: Some(std::rc::Rc::new(move |side, line, _| {
                    on_gutter.borrow_mut().push((side, line))
                })),
                selection_actions: vec![DiffSelectionAction::new("Comment", move |range, _| {
                    on_action.borrow_mut().push(range)
                })],
                // Rows are counted inline, whatever the diff setting says.
                layout: gitcomet_extension_api::DiffLayout::Inline,
                ..DiffPaneOptions::default()
            },
            app,
        )
        .unwrap()
    });
    app_cx.run_until_parked();
    let red = gpui::red();
    app_cx.update(|_window, app| {
        pane.set_insets(
            vec![DiffInset::new(
                DiffLineSide::New,
                2,
                ["note one".into(), "note two".into()],
            )],
            app,
        );
        pane.set_annotations(
            DiffAnnotations::new().with(
                DiffLineSide::New,
                3,
                DiffAnnotation::new(red).with_label("flag"),
            ),
            app,
        );
        pane.set_legend(vec![DiffLegendItem::new("Flagged", red)], app);
    });
    let pane_view = pane
        .view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .unwrap_or_else(|_| panic!("a hosted diff pane"));
    let id = app_cx.update(|_window, app| pane_view.read(app).view_id());

    let view_any = pane.view();
    let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(view_any));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    let marker_builds = cx.update(|_, app| pane_view.read(app).marker_builds(app));
    assert!(marker_builds > 0, "the shared renderer placed the markers");
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
    assert!(cx.update(|_, app| pane_view.read(app).shares_renderer_rows(app)));
    // Rows: one, two (removed), TWO, two inset rows, three.
    for part in ["legend", "markers", "row_3", "row_4", "row_5"] {
        assert!(
            cx.debug_bounds(selector(format!("hosted_diff_{id}_{part}")))
                .is_some(),
            "{part} is drawn"
        );
    }
    assert_eq!(
        cx.update(|_window, app| pane_view.read(app).marker_builds(app)),
        marker_builds,
        "unchanged frames reuse the shared marker projection"
    );
    assert_eq!(rows_with(cx, &pane, "note"), 0, "insets are not file lines");
    assert_eq!(rows_with(cx, &pane, "three"), 1);
    rows_with(cx, &pane, "");

    let selection =
        |cx: &mut gpui::VisualTestContext| cx.update(|_window, app| pane.selection(app));
    click_with(
        cx,
        selector(format!("hosted_diff_{id}_row_0")),
        gpui::Modifiers::default(),
    );
    click_with(
        cx,
        selector(format!("hosted_diff_{id}_row_2")),
        gpui::Modifiers::shift(),
    );
    cx.run_until_parked();
    let expected = DiffLineRange {
        side: DiffLineSide::New,
        start: 1,
        end: 2,
    };
    assert_eq!(selection(cx), Some(expected));
    assert_eq!(
        cx.update(|_window, app| pane.selected_text(app)).as_deref(),
        Some("one\nTWO")
    );

    // Clicking an inset or a gutter leaves the selection alone.
    click_with(
        cx,
        selector(format!("hosted_diff_{id}_row_3")),
        gpui::Modifiers::default(),
    );
    click_with(
        cx,
        selector(format!("hosted_diff_{id}_gutter_5")),
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    assert_eq!(selection(cx), Some(expected));
    assert_eq!(*gutter_clicks.borrow(), vec![(DiffLineSide::New, 3)]);

    for _ in 0..2 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    click_with(
        cx,
        selector(format!("hosted_diff_{id}_action_0")),
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    assert_eq!(*acted.borrow(), vec![expected]);
}

#[gpui::test]
fn grouped_file_lists_pin_the_current_group_without_replanning(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, app_cx) = open_repository(cx);
    let root = dir.path();
    for n in 0..60 {
        std::fs::write(root.join(format!("m{n:02}.rs")), "before\n").unwrap();
    }
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "before"]);
    for n in 0..60 {
        std::fs::write(root.join(format!("m{n:02}.rs")), "after\n").unwrap();
        std::fs::write(root.join(format!("a{n:02}.rs")), "new\n").unwrap();
    }
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "after"]);
    let head = head(root);

    let (host, repository) = app_cx.update(|_window, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        (host, repository)
    });
    let list = app_cx.update(|_window, app| {
        let list = host
            .create_file_list(
                &repository,
                ChangeSource::Commit(gitcomet_core::domain::CommitId(head.into())),
                |_, _, _| {},
                app,
            )
            .unwrap();
        list.set_mode(gitcomet_extension_api::FileListMode::Grouped, app);
        list
    });
    settle(app_cx, &view, &store, "the commit's files", |cx| {
        cx.update(|_window, app| !list.is_loading(app))
    });
    let list_view = list
        .view()
        .downcast::<crate::view::hosted::file_list::FileListView>()
        .unwrap_or_else(|_| panic!("a hosted file list"));
    let view_any = list.view();
    let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(view_any));
    let draw = |cx: &mut gpui::VisualTestContext| {
        for _ in 0..2 {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
        }
    };
    draw(cx);
    let (id, scroll, builds) = cx.update(|_window, app| list_view.read(app).test_parts());
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_group_Added")))
            .is_some()
    );
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_sticky_Added")))
            .is_none(),
        "nothing to pin before scrolling"
    );

    // Rows: Added header, 60 files, Modified header, 60 files.
    let added = selector(format!("hosted_file_list_{id}_group_Added"));
    let center = cx.debug_bounds(added).unwrap().center();
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    draw(cx);
    assert_eq!(
        cx.update(|_, app| list_view.read(app).test_parts().2),
        builds,
        "a header never toggles on press"
    );
    cx.simulate_mouse_up(
        gpui::point(px(1000.0), px(1000.0)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    draw(cx);
    assert_eq!(
        cx.update(|_, app| list_view.read(app).test_parts().2),
        builds,
        "a cancelled click never toggles"
    );
    click_debug_selector(cx, added);
    draw(cx);
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_file_a00.rs")))
            .is_none()
    );
    cx.simulate_keystrokes("space");
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("space").unwrap(),
            }),
            app,
        );
    });
    draw(cx);
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_file_a00.rs")))
            .is_some(),
        "keyboard activation reopens the group"
    );
    let builds = cx.update(|_, app| list_view.read(app).test_parts().2);
    scroll.scroll_to_item(90, gpui::ScrollStrategy::Top);
    draw(cx);
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_sticky_Modified")))
            .is_some()
    );
    scroll.scroll_to_item(30, gpui::ScrollStrategy::Top);
    draw(cx);
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_sticky_Added")))
            .is_some()
    );
    assert_eq!(
        cx.update(|_window, app| list_view.read(app).test_parts().2),
        builds,
        "scrolling never regroups"
    );

    // The pinned header collapses its group like the row does.
    let sticky = selector(format!("hosted_file_list_{id}_sticky_Added"));
    let center = cx.debug_bounds(sticky).unwrap().center();
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    draw(cx);
    assert_eq!(
        cx.update(|_, app| list_view.read(app).test_parts().2),
        builds,
        "the sticky header also waits for release"
    );
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    draw(cx);
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_file_a00.rs")))
            .is_none()
    );
    assert!(
        cx.debug_bounds(selector(format!("hosted_file_list_{id}_group_Modified")))
            .is_some()
    );
}

#[gpui::test]
fn the_example_pops_its_pane_out_and_back(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository(cx);
    let (list_id, changes) = open_changes_view(cx, &view, &store);
    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_a.rs")),
    );
    settle(cx, &view, &store, "the first pick", |_| {
        store.snapshot().repos[0].diff_sessions.len() == 1
    });
    let pane_id = store.snapshot().repos[0]
        .diff_sessions
        .keys()
        .next()
        .unwrap()
        .0;
    let row = selector(format!("hosted_diff_{pane_id}_row_0"));
    settle(cx, &view, &store, "the pane's rows", |cx| {
        cx.debug_bounds(row).is_some()
    });
    // App-level: the last check runs after the main window has gone.
    let windows = |cx: &mut gpui::VisualTestContext| cx.cx.update(|app| app.windows());
    let before = windows(cx);

    click_debug_selector(cx, "example_changes_pop_out");
    cx.run_until_parked();
    let after = windows(cx);
    assert_eq!(after.len(), before.len() + 1);
    let popped = *after
        .iter()
        .find(|window| !before.contains(window))
        .unwrap();
    publish(cx, &view, store.snapshot());
    assert!(
        cx.debug_bounds(row).is_none(),
        "the pane left the main window"
    );
    let mut pop_cx = gpui::VisualTestContext::from_window(popped, cx);
    for _ in 0..2 {
        pop_cx.update(|window, app| {
            let _ = window.draw(app);
        });
        pop_cx.run_until_parked();
    }
    assert!(pop_cx.debug_bounds("extension_pop_out").is_some());
    assert!(
        pop_cx.debug_bounds(row).is_some(),
        "the pane draws in its window"
    );

    // Closing it from its handle returns the pane.
    cx.update(|_window, app| {
        let handle = changes.read(app).popped().cloned().unwrap();
        handle.close(app);
    });
    cx.run_until_parked();
    assert_eq!(windows(cx).len(), before.len());
    assert!(cx.update(|_window, app| changes.read(app).popped().is_none()));
    publish(cx, &view, store.snapshot());
    assert!(cx.debug_bounds(row).is_some());

    // A pop-out closes with the window that opened it.
    click_debug_selector(cx, "example_changes_pop_out");
    cx.run_until_parked();
    assert_eq!(windows(cx).len(), before.len() + 1);
    cx.update(|window, _app| window.remove_window());
    cx.run_until_parked();
    assert!(windows(cx).is_empty());
}

type Clicks = std::rc::Rc<std::cell::RefCell<Vec<(&'static str, DiffLineSide, u32)>>>;

/// A snapshot pane with a clickable note under new line 2 and an annotation
/// on new line 3, drawn by the same renderer as repository diffs.
fn clickable_pane(
    host: &gitcomet_extension_api::WindowHost,
    policy: DiffPanePolicy,
    clicks: &Clicks,
    app: &mut App,
) -> (
    DiffPane,
    gpui::Entity<crate::view::hosted::diff_pane::DiffPaneView>,
) {
    use crate::view::hosted::diff_pane::DiffPaneView;
    use gitcomet_extension_api::{DiffAnnotation, DiffAnnotations, DiffInset, HostedAction};
    let record = |kind: &'static str| {
        let clicks = clicks.clone();
        Some(std::rc::Rc::new(move |side, line, _: &mut App| {
            clicks.borrow_mut().push((kind, side, line))
        }) as gitcomet_extension_api::DiffGutterAction)
    };
    let options = DiffPaneOptions {
        policy,
        on_gutter_click: record("gutter"),
        on_annotation_click: record("annotation"),
        // Rows are counted inline, whatever the diff setting says.
        layout: gitcomet_extension_api::DiffLayout::Inline,
        ..DiffPaneOptions::default()
    };
    let snapshot = DiffSnapshot::new("notes.txt", "one\ntwo\nthree\n", "one\nTWO\nthree\n");
    let pane = host.create_snapshot_pane(snapshot, options, app).unwrap();
    let entity = pane.view().downcast::<DiffPaneView>().unwrap();
    let noted = clicks.clone();
    pane.set_insets(
        vec![
            DiffInset::new(DiffLineSide::New, 2, ["note".into()])
                .with_action(HostedAction::new("Open note", move |_| {
                    noted.borrow_mut().push(("inset", DiffLineSide::New, 2))
                })),
        ],
        app,
    );
    pane.set_annotations(
        DiffAnnotations::new().with(
            DiffLineSide::New,
            3,
            DiffAnnotation::new(gpui::red()).with_label("flag"),
        ),
        app,
    );
    (pane, entity)
}

/// A note's click runs its action and a click on an annotation runs the
/// annotation action, in the shared renderer and in the pane's own rows;
/// other lines' gutters still run the gutter action. A read-only pane runs
/// no line action, but its notes stay clickable.
#[gpui::test]
fn insets_and_annotations_run_their_actions(cx: &mut gpui::TestAppContext) {
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, app_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    publish(app_cx, &view, Arc::new(AppState::test_default()));
    let host =
        app_cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let (new2, new3) = ((DiffLineSide::New, 2), (DiffLineSide::New, 3));

    for policy in [DiffPanePolicy::default(), DiffPanePolicy::read_only()] {
        let clicks = Clicks::default();
        let (pane, entity) = cx.update(|app| clickable_pane(&host, policy, &clicks, app));
        cx.run_until_parked();
        let view_any = pane.view();
        let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(view_any));
        for _ in 0..3 {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
        }
        let (id, shared) = cx.update(|_, app| {
            let pane = entity.read(app);
            (pane.view_id(), pane.shares_renderer_rows(app))
        });
        assert!(shared);
        // Rows: one, two (removed), TWO, the note, three.
        let (note, two, three) = (3, 2, 4);
        let click = |cx: &mut gpui::VisualTestContext, part: String| {
            click_with(
                cx,
                selector(format!("hosted_diff_{id}_{part}")),
                Default::default(),
            );
            cx.run_until_parked();
        };
        click(cx, format!("row_{note}"));
        click(cx, format!("gutter_{two}"));
        // The shared lane draws the annotation.
        click(cx, format!("gutter_{three}"));
        let expected: Vec<_> = if policy.line_action {
            [("inset", new2), ("gutter", new2)]
                .into_iter()
                .chain(std::iter::once(("annotation", new3)))
                .map(|(kind, (side, line))| (kind, side, line))
                .collect()
        } else {
            vec![("inset", new2.0, new2.1)]
        };
        assert_eq!(
            *clicks.borrow(),
            expected,
            "line actions: {}",
            policy.line_action
        );
    }
}

/// A linked worktree of `root` on a new branch, with `a.rs` edited there.
fn add_linked_worktree(root: &Path) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let linked = dir.path().join("linked");
    git(
        root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );
    std::fs::write(linked.join("a.rs"), "linked edit\n").unwrap();
    (dir, linked)
}

struct TwoPanes(gpui::AnyView, gpui::AnyView);

impl Render for TwoPanes {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().child(self.0.clone()))
            .child(div().flex_1().child(self.1.clone()))
    }
}

/// Staging and saving act on the main checkout, so a pane on a linked
/// worktree's file offers neither, and its handlers refuse both even when
/// asked directly; the same file in the main worktree offers both.
#[gpui::test]
fn a_linked_worktree_pane_offers_and_runs_no_staging_or_editing(cx: &mut gpui::TestAppContext) {
    use crate::view::hosted::diff_pane::DiffPaneView;
    use gitcomet_core::domain::{DiffArea, DiffTarget};
    let (dir, store, view, app_cx) =
        open_repository_with(cx, Arc::new(gitcomet_git_gix::GixBackend));
    let root = dir.path().to_path_buf();
    let (_linked_dir, linked) = add_linked_worktree(&root);
    let (host, repository) = app_cx.update(|_window, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        (host, repository)
    });
    let target = DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged);
    let (main, linked_pane) = app_cx.update(|_window, app| {
        let mut pane = |target| {
            host.create_diff_pane(&repository, target, DiffPaneOptions::default(), app)
                .unwrap()
        };
        (
            pane(target.clone()),
            pane(target.clone().in_worktree(linked.clone())),
        )
    });
    settle(app_cx, &view, &store, "both panes", |cx| {
        cx.update(|_window, app| !main.is_loading(app) && !linked_pane.is_loading(app))
    });
    let entity = |pane: &DiffPane| {
        pane.view()
            .downcast::<DiffPaneView>()
            .unwrap_or_else(|_| panic!("a hosted diff pane"))
    };
    let (main_view, linked_view) = (entity(&main), entity(&linked_pane));
    let views = (main.view(), linked_pane.view());
    let (_holder, cx) = cx.add_window_view(move |_, _| TwoPanes(views.0, views.1));
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
    let offers = |cx: &mut gpui::VisualTestContext, pane: &gpui::Entity<DiffPaneView>| {
        cx.update(|_, app| pane.read(app).renderer_offers(app))
    };
    assert_eq!(offers(cx, &main_view), Some((true, true)));
    assert_eq!(offers(cx, &linked_view), Some((false, false)));
    // Previews and the editor read the linked checkout, not the main one.
    let workdir = |cx: &mut gpui::VisualTestContext, pane: &gpui::Entity<DiffPaneView>| {
        cx.update(|_, app| pane.read(app).renderer_workdir(app))
    };
    assert_eq!(
        workdir(cx, &linked_view).map(canonicalize_or_original),
        Some(canonicalize_or_original(linked.clone()))
    );
    assert_eq!(
        workdir(cx, &main_view).map(canonicalize_or_original),
        Some(canonicalize_or_original(root.clone()))
    );

    // Asked directly, the linked pane's handlers drop both requests; a
    // later request from the main pane shows they had their turn.
    let repo_id = repository.repo_id();
    let (saved_tx, saved_rx) = smol::channel::bounded(1);
    cx.update(|_, app| {
        linked_view.read(app).renderer_dispatch(
            Msg::StagePath {
                repo_id,
                path: "a.rs".into(),
            },
            app,
        );
        linked_view.read(app).renderer_dispatch(
            Msg::SaveWorktreeFile {
                repo_id,
                path: "a.rs".into(),
                contents: "overwritten\n".into(),
                expected_contents: WorktreeSaveBaseline::DeriveFromLoadedConflict,
                stage: false,
                completion: Some(saved_tx),
            },
            app,
        );
        main_view.read(app).renderer_dispatch(
            Msg::StagePath {
                repo_id,
                path: "b.rs".into(),
            },
            app,
        );
    });
    let staged = || {
        let out = crate::test_support::git(&root, &["diff", "--cached", "--name-only"]);
        String::from_utf8(out).unwrap().trim().to_string()
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while staged().is_empty() {
        assert!(Instant::now() < deadline, "the main pane's stage never ran");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(staged(), "b.rs", "the linked pane staged nothing");
    assert!(
        matches!(
            saved_rx.try_recv(),
            Err(smol::channel::TryRecvError::Closed)
        ),
        "the save was dropped, never run"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("a.rs")).unwrap(),
        numbered("a", 30, Some(3))
    );
    assert_eq!(
        std::fs::read_to_string(linked.join("a.rs")).unwrap(),
        "linked edit\n"
    );

    // The renderer's own retargets name the main checkout; the pane keeps
    // them in its worktree.
    cx.update(|_, app| {
        linked_view.read(app).renderer_dispatch(
            Msg::SelectDiff {
                repo_id,
                target: DiffTarget::working_tree("b.rs".into(), DiffArea::Unstaged),
            },
            app,
        )
    });
    let view_id = gitcomet_state::diff_session::DiffViewId(
        cx.update(|_, app| linked_view.read(app).view_id()),
    );
    let target = || {
        store.snapshot().repos[0]
            .diff_sessions
            .get(&view_id)
            .map(|session| session.target.clone())
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while target().and_then(|target| target.file_path().map(Path::to_path_buf))
        != Some(PathBuf::from("b.rs"))
    {
        assert!(Instant::now() < deadline, "the retarget never ran");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        target()
            .and_then(|target| target.worktree().map(Path::to_path_buf))
            .map(canonicalize_or_original),
        Some(canonicalize_or_original(linked))
    );
}

/// A save that stages is staging: a pane that may edit but not stage drops
/// it unrun, and still runs a plain save.
#[gpui::test]
fn a_pane_without_staging_drops_a_save_that_stages(cx: &mut gpui::TestAppContext) {
    use crate::view::hosted::diff_pane::DiffPaneView;
    use gitcomet_core::domain::{DiffArea, DiffTarget};
    let (dir, store, view, app_cx) =
        open_repository_with(cx, Arc::new(gitcomet_git_gix::GixBackend));
    let root = dir.path().to_path_buf();
    let (host, repository) = app_cx.update(|_window, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        (host, repository)
    });
    let mut policy = DiffPanePolicy::default();
    policy.allow_stage = false;
    let pane = app_cx.update(|_window, app| {
        host.create_diff_pane(
            &repository,
            DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged),
            DiffPaneOptions {
                policy,
                ..DiffPaneOptions::default()
            },
            app,
        )
        .unwrap()
    });
    settle(app_cx, &view, &store, "the pane", |cx| {
        cx.update(|_window, app| !pane.is_loading(app))
    });
    let entity = pane
        .view()
        .downcast::<DiffPaneView>()
        .unwrap_or_else(|_| panic!("a hosted diff pane"));
    let shown = pane.view();
    let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(shown));
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let repo_id = repository.repo_id();
    let save = |contents: &str, stage: bool, cx: &mut gpui::VisualTestContext| {
        let (saved_tx, saved_rx) = smol::channel::bounded(1);
        cx.update(|_, app| {
            entity.read(app).renderer_dispatch(
                Msg::SaveWorktreeFile {
                    repo_id,
                    path: "a.rs".into(),
                    contents: contents.to_string().into(),
                    expected_contents: WorktreeSaveBaseline::Present(
                        std::fs::read(root.join("a.rs")).unwrap().into(),
                    ),
                    stage,
                    completion: Some(saved_tx),
                },
                app,
            );
        });
        saved_rx
    };
    let staging = save("staged\n", true, cx);
    assert!(
        smol::block_on(staging.recv()).is_err(),
        "the staging save was dropped, never run"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("a.rs")).unwrap(),
        numbered("a", 30, Some(3))
    );
    let plain = save("saved\n", false, cx);
    assert_eq!(
        smol::block_on(plain.recv()),
        Ok(true),
        "the plain save runs"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("a.rs")).unwrap(),
        "saved\n"
    );
    let staged = crate::test_support::git(&root, &["diff", "--cached", "--name-only"]);
    assert!(staged.is_empty(), "nothing was staged");
}

/// A list on a linked worktree reloads when that worktree changes, as its
/// watch reports it.
#[gpui::test]
fn a_linked_worktree_list_reloads_when_its_worktree_changes(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::domain::DiffArea;
    let (dir, store, view, cx) = open_repository_with(cx, Arc::new(gitcomet_git_gix::GixBackend));
    let (_linked_dir, linked) = add_linked_worktree(dir.path());
    let (host, repository) = cx.update(|_window, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        (host, repository)
    });
    let (list, _watch) = cx.update(|_window, app| {
        let list = host
            .create_file_list(
                &repository,
                ChangeSource::linked_worktree(linked.clone(), DiffArea::Unstaged, true),
                |_, _, _| {},
                app,
            )
            .unwrap();
        let watch = host.watch_worktree(&repository, &linked, app).unwrap();
        (list, watch)
    });
    let listed = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            list.files(app)
                .into_iter()
                .map(|file| file.path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
    };
    settle(cx, &view, &store, "the linked worktree's changes", |cx| {
        listed(cx) == ["a.rs"]
    });

    std::fs::write(linked.join("b.rs"), "linked too\n").unwrap();
    let mut change = gitcomet_state::msg::RepoExternalChange::worktree();
    change.paths = gitcomet_state::msg::ChangedPaths::known(vec!["b.rs".into()]);
    store.dispatch(Msg::WorktreeExternallyChanged {
        repo_id: repository.repo_id(),
        lifetime: repository.lifetime(),
        path: linked.clone(),
        change,
    });
    settle(cx, &view, &store, "the reload", |cx| {
        listed(cx) == ["a.rs", "b.rs"]
    });
}

/// The example's Changes view lists a linked worktree on request: its own
/// changes, read-only, watched while shown.
#[gpui::test]
fn the_example_lists_a_linked_worktree(cx: &mut gpui::TestAppContext) {
    let (dir, store, view, cx) = open_repository_with(cx, Arc::new(gitcomet_git_gix::GixBackend));
    let (_linked_dir, linked) = add_linked_worktree(dir.path());
    let (_list_id, _changes) = open_changes_view(cx, &view, &store);
    store.dispatch(Msg::LoadWorktrees { repo_id: RepoId(1) });
    settle(cx, &view, &store, "the worktree buttons", |cx| {
        cx.debug_bounds("example_changes_worktree_0").is_some()
    });
    click_debug_selector(cx, "example_changes_worktree_0");
    let linked_files = |store: &AppStore| {
        let snapshot = store.snapshot();
        let list = snapshot.repos[0].change_lists.values().next()?.clone();
        list.source.linked_path()?;
        list.files.ready().map(|files| {
            files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>()
        })
    };
    settle(cx, &view, &store, "the linked worktree's changes", |_| {
        linked_files(&store).is_some()
    });
    assert_eq!(linked_files(&store), Some(vec![PathBuf::from("a.rs")]));
    let snapshot = store.snapshot();
    let source = &snapshot.repos[0]
        .change_lists
        .values()
        .next()
        .unwrap()
        .source;
    assert_eq!(
        source
            .linked_path()
            .map(|path| canonicalize_or_original(path.to_path_buf())),
        Some(canonicalize_or_original(linked))
    );
    assert_eq!(
        snapshot.worktree_watch_leases.len(),
        1,
        "the shown worktree is watched"
    );
}

fn open_linked_worktree_changes(
    cx: &mut gpui::TestAppContext,
) -> (
    tempfile::TempDir,
    PathBuf,
    AppStore,
    gpui::Entity<GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let dir = repository_with_edits();
    let linked = dir.path().join("linked");
    git(
        dir.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    );
    let staged = numbered("a", 30, None)
        .replace("a 3\n", "linked staged 3\n")
        .replace("a 14\n", "linked staged 14\n")
        .replace("a 25\n", "linked staged 25\n");
    std::fs::write(linked.join("a.rs"), &staged).unwrap();
    git(&linked, &["add", "a.rs"]);
    std::fs::write(
        linked.join("a.rs"),
        staged.replace("linked staged", "linked unstaged"),
    )
    .unwrap();
    std::fs::remove_file(linked.join("b.rs")).unwrap();
    std::fs::write(linked.join("c.rs"), "linked untracked\n").unwrap();
    let linked = canonicalize_or_original(linked);
    let (store, events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    store.disable_repo_monitors_for_test();
    store.dispatch(Msg::SetGitRuntimeState(available_git_runtime_state()));
    store.dispatch(Msg::OpenRepo(dir.path().to_path_buf()));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    settle(cx, &view, &store, "origin checkout to open", |cx| {
        cx.update(|_, app| {
            view.read(app).active_repo().is_some_and(|repo| {
                repo.id == RepoId(1)
                    && matches!(repo.open, Loadable::Ready(()))
                    && matches!(repo.status, Loadable::Ready(_))
            })
        })
    });
    store.dispatch(Msg::SelectWorktreeUncommitted {
        request_id: None,
        repo_id: RepoId(1),
        path: linked.clone(),
    });
    settle(cx, &view, &store, "linked worktree files", |cx| {
        cx.update(|_, app| {
            view.read(app)
                .details_pane
                .read(app)
                .selected_worktree_summary()
                .is_some_and(|summary| {
                    summary.path == linked
                        && summary.staged.len() == 1
                        && summary.unstaged.len() == 3
                })
        })
    });
    assert!(
        cx.debug_bounds("worktree_uncommitted_body").is_some(),
        "the worktree details body must be mounted"
    );
    (dir, linked, store, view, cx)
}

fn click_linked_file(cx: &mut gpui::VisualTestContext, row: usize, button: gpui::MouseButton) {
    let position = cx
        .debug_bounds(Box::leak(format!("worktree_file_1_{row}").into_boxed_str()))
        .expect("worktree file row visible")
        .center();
    cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(position, button, gpui::Modifiers::default());
    cx.simulate_mouse_up(position, button, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
}

fn main_diff_contains(pane: &DiffRenderer, target: &DiffTarget, needle: &str) -> bool {
    pane.rendered_patch_diff_loadable()
        .and_then(Loadable::ready)
        .is_some_and(|diff| diff.lines.iter().any(|line| line.text.contains(needle)))
        || (pane.file_diff_cache_target.as_ref() == Some(target)
            && (0..pane.file_diff_split_row_len())
                .filter_map(|ix| pane.file_diff_split_row(ix))
                .any(|row| {
                    row.old.as_deref().is_some_and(|text| text.contains(needle))
                        || row.new.as_deref().is_some_and(|text| text.contains(needle))
                }))
}

fn assert_linked_diff(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    linked: &Path,
    target: &DiffTarget,
    needle: &str,
) {
    settle(cx, view, store, "linked diff content", |cx| {
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            pane.rendered_diff_target() == Some(target)
                && pane
                    .rendered_patch_diff_loadable()
                    .is_some_and(|diff| matches!(diff, Loadable::Ready(_)))
                && pane
                    .rendered_file_diff_loadable()
                    .is_some_and(|file| matches!(file, Loadable::Ready(Some(_))))
                && main_diff_contains(pane, target, needle)
        })
    });
    let state = store.snapshot();
    assert_eq!(
        state.active_repo,
        Some(RepoId(1)),
        "inline viewing keeps the original tab"
    );
    let repo = &state.repos[0];
    assert!(
        repo.diff_state.diff_target.is_none(),
        "the actual click sets only the foreign target"
    );
    let inline = repo.diff_state.inline_submodule_diff.as_ref().unwrap();
    assert_eq!(inline.submodule_repo_path, linked);
    assert_eq!(&inline.target, target);
    let diff = inline.diff.ready().unwrap();
    assert!(
        cx.update(|_, app| main_diff_contains(
            view.read(app).diff_renderer.read(app),
            target,
            needle
        )),
        "the rendered diff must contain the linked checkout's bytes"
    );
    assert!(
        !diff.lines.iter().any(|line| line.text.contains("a edited")),
        "must not load the originating checkout's edits"
    );
    assert!(
        cx.debug_bounds("diff_body_container").is_some(),
        "loaded foreign diff must actually render"
    );
    assert!(cx.debug_bounds("inline_foreign_back").is_none());
}

#[gpui::test]
fn clicking_linked_worktree_files_renders_staged_and_unstaged_diffs(cx: &mut gpui::TestAppContext) {
    let (_dir, linked, store, view, cx) = open_linked_worktree_changes(cx);
    click_linked_file(cx, 0, gpui::MouseButton::Left);
    assert_linked_diff(
        cx,
        &view,
        &store,
        &linked,
        &DiffTarget::working_tree(PathBuf::from("a.rs"), DiffArea::Staged),
        "linked staged",
    );
    let close = cx.debug_bounds("diff_close").unwrap().center();
    cx.simulate_mouse_move(close, None, gpui::Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view).as_deref(),
        Some("Close diff (Esc)"),
    );
    cx.simulate_keystrokes("escape");
    settle(cx, &view, &store, "return to worktree history", |cx| {
        cx.update(|_, app| {
            view.read(app).main_pane.read(app).active_content(app)
                == crate::view::panes::main::MainPaneContent::History
        })
    });
    assert!(cx.debug_bounds("diff_body_container").is_none());
    assert_eq!(
        store.snapshot().repos[0]
            .history_state
            .worktree_selection
            .as_deref(),
        Some(linked.as_path())
    );
    click_linked_file(cx, 1, gpui::MouseButton::Left);
    assert_linked_diff(
        cx,
        &view,
        &store,
        &linked,
        &DiffTarget::working_tree(PathBuf::from("a.rs"), DiffArea::Unstaged),
        "linked unstaged",
    );
}

#[gpui::test]
fn linked_worktree_change_keys_use_the_displayed_diff(cx: &mut gpui::TestAppContext) {
    let (_dir, linked, store, view, cx) = open_linked_worktree_changes(cx);
    cx.update(|_, app| {
        crate::app::bind_app_keys_for_test(app);
        crate::app::install_global_diff_navigation_actions_for_test(app);
        crate::app::install_global_diff_shortcut_fallback_for_test(app);
    });
    click_linked_file(cx, 0, gpui::MouseButton::Left);
    assert_linked_diff(
        cx,
        &view,
        &store,
        &linked,
        &DiffTarget::working_tree(PathBuf::from("a.rs"), DiffArea::Staged),
        "linked staged",
    );

    for layout in [DiffViewMode::Inline, DiffViewMode::Split] {
        for content in [DiffContentMode::Full, DiffContentMode::Collapsed] {
            cx.update(|_, app| {
                view.update(app, |view, cx| {
                    view.set_diff_view_mode(layout, cx);
                    view.set_diff_content_mode(content, cx);
                });
            });
            publish(cx, &view, store.snapshot());
            let pane = cx.update(|_, app| view.read(app).diff_renderer.clone());
            let stops = cx.update(|_, app| pane.read(app).diff_nav_entries());
            assert_eq!(stops.len(), 3, "{layout:?}, {content:?}");

            for detached in [false, true] {
                let focus = cx.update(|window, app| {
                    pane.update(app, |pane, cx| {
                        pane.diff_selection_anchor = Some(stops[0]);
                        pane.diff_selection_range = None;
                        cx.notify();
                    });
                    let focus = if detached {
                        app.focus_handle()
                    } else {
                        pane.read(app).diff_panel_focus_handle.clone()
                    };
                    window.focus(&focus, app);
                    focus
                });
                publish(cx, &view, store.snapshot());
                for (key, stop) in [
                    ("f3", stops[1]),
                    ("f2", stops[0]),
                    ("shift-f3", stops[1]),
                    ("shift-f2", stops[0]),
                    ("f7", stops[1]),
                    ("shift-f7", stops[0]),
                    ("alt-down", stops[1]),
                    ("alt-up", stops[0]),
                ] {
                    cx.simulate_keystrokes(key);
                    publish(cx, &view, store.snapshot());
                    assert_eq!(
                        cx.update(|_, app| pane.read(app).diff_selection_anchor),
                        Some(stop),
                        "{key}, {layout:?}, {content:?}, detached focus: {detached}",
                    );
                    assert!(cx.update(|window, _| focus.is_focused(window)));
                }
            }
        }
    }

    for (key, area, needle) in [
        ("f4", DiffArea::Unstaged, "linked unstaged"),
        ("f1", DiffArea::Staged, "linked staged"),
    ] {
        cx.simulate_keystrokes(key);
        assert_linked_diff(
            cx,
            &view,
            &store,
            &linked,
            &DiffTarget::working_tree(PathBuf::from("a.rs"), area),
            needle,
        );
    }
}

#[gpui::test]
fn linked_worktree_file_selection_repaints_without_mouse_movement(cx: &mut gpui::TestAppContext) {
    let (_dir, _linked, store, view, cx) = open_linked_worktree_changes(cx);
    cx.update(|_, app| {
        crate::app::bind_app_keys_for_test(app);
        crate::app::install_global_diff_navigation_actions_for_test(app);
        crate::app::install_global_diff_shortcut_fallback_for_test(app);
    });
    // Cached frames do not replay debug selectors, so retain the bounds before
    // enabling the same cache boundaries used in the application.
    let staged_bounds = cx.debug_bounds("worktree_file_1_0").unwrap();
    let unstaged_bounds = cx.debug_bounds("worktree_file_1_1").unwrap();
    let paint = |cx: &mut gpui::VisualTestContext, bounds: gpui::Bounds<gpui::Pixels>| {
        cx.update(|window, _| {
            let scale = window.scale_factor();
            window
                .painted_quads()
                .into_iter()
                .filter(|quad| {
                    let rect = quad.bounds;
                    (rect.origin.x.0 - f32::from(bounds.origin.x) * scale).abs() < 1.0
                        && (rect.origin.y.0 - f32::from(bounds.origin.y) * scale).abs() < 1.0
                        && (rect.size.width.0 - f32::from(bounds.size.width) * scale).abs() < 1.0
                        && (rect.size.height.0 - f32::from(bounds.size.height) * scale).abs() < 1.0
                })
                .map(|quad| quad.background)
                .collect::<Vec<_>>()
        })
    };
    let staged_resting = paint(cx, staged_bounds);
    cx.simulate_mouse_move(staged_bounds.center(), None, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    let staged_idle = paint(cx, staged_bounds);
    let unstaged_idle = paint(cx, unstaged_bounds);
    assert!(!staged_idle.is_empty());
    assert!(!unstaged_idle.is_empty());
    let _cache_guard = enable_stable_cached_views_for_test();
    publish(cx, &view, store.snapshot());
    // The pointer stays over this row throughout selection, change navigation,
    // file navigation and closing. No mouse event can repair a stale highlight.
    cx.simulate_click(staged_bounds.center(), gpui::Modifiers::default());
    let pane = cx.update(|_, app| view.read(app).diff_renderer.clone());
    let selected_ix = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            view.read(app)
                .details_pane
                .read(app)
                .active_repo()
                .and_then(|repo| repo.diff_state.inline_submodule_diff.as_ref())
                .map(|inline| inline.selected_ix)
        })
    };
    settle(cx, &view, &store, "selected staged worktree file", |cx| {
        selected_ix(cx) == Some(0)
            && cx.update(|_, app| {
                let pane = pane.read(app);
                pane.rendered_patch_diff_loadable()
                    .is_some_and(|diff| matches!(diff, Loadable::Ready(_)))
                    && pane
                        .rendered_file_diff_loadable()
                        .is_some_and(|file| matches!(file, Loadable::Ready(Some(_))))
                    && main_diff_contains(
                        pane,
                        &DiffTarget::working_tree(PathBuf::from("a.rs"), DiffArea::Staged),
                        "linked staged",
                    )
            })
    });
    let staged_selected = paint(cx, staged_bounds);
    assert_ne!(
        staged_selected, staged_idle,
        "selection must paint immediately"
    );
    assert_eq!(paint(cx, unstaged_bounds), unstaged_idle);

    let stops = cx.update(|_, app| pane.read(app).diff_nav_entries());
    assert_eq!(stops.len(), 3);
    assert_eq!(
        cx.update(|_, app| pane.read(app).diff_selection_anchor),
        Some(stops[0]),
        "opening the file focuses its first change"
    );
    for (key, stop) in [
        ("f3", stops[1]),
        ("f2", stops[0]),
        ("shift-f3", stops[1]),
        ("shift-f2", stops[0]),
        ("f7", stops[1]),
        ("shift-f7", stops[0]),
        ("alt-down", stops[1]),
        ("alt-up", stops[0]),
    ] {
        cx.simulate_keystrokes(key);
        publish(cx, &view, store.snapshot());
        assert_eq!(
            cx.update(|_, app| pane.read(app).diff_selection_anchor),
            Some(stop)
        );
        assert_eq!(paint(cx, staged_bounds), staged_selected, "after {key}");
    }

    cx.simulate_keystrokes("f4");
    settle(cx, &view, &store, "selected unstaged worktree file", |cx| {
        selected_ix(cx) == Some(1)
    });
    // Keyboard input can clear hover feedback even with a stationary pointer.
    // Both unselected fills are valid; the persistent selection must disappear.
    let staged_after_f4 = paint(cx, staged_bounds);
    assert!(
        staged_after_f4 == staged_idle || staged_after_f4 == staged_resting,
        "F4 clears the old highlight: {staged_after_f4:?}"
    );
    assert_ne!(
        paint(cx, unstaged_bounds),
        unstaged_idle,
        "F4 paints the new highlight"
    );

    cx.simulate_keystrokes("f1");
    settle(cx, &view, &store, "reselected staged worktree file", |cx| {
        selected_ix(cx) == Some(0)
    });
    assert_eq!(
        paint(cx, staged_bounds),
        staged_selected,
        "F1 restores the highlight"
    );
    assert_eq!(paint(cx, unstaged_bounds), unstaged_idle);

    cx.simulate_keystrokes("escape");
    settle(cx, &view, &store, "closed worktree diff", |cx| {
        selected_ix(cx).is_none()
    });
    let staged_after_close = paint(cx, staged_bounds);
    assert!(
        staged_after_close == staged_idle || staged_after_close == staged_resting,
        "closing clears the highlight: {staged_after_close:?}"
    );
    assert_eq!(paint(cx, unstaged_bounds), unstaged_idle);
}

#[gpui::test]
fn linked_worktree_file_context_menu_opens_diff_and_uses_checkout_paths(
    cx: &mut gpui::TestAppContext,
) {
    let (_dir, linked, store, view, cx) = open_linked_worktree_changes(cx);
    click_linked_file(cx, 0, gpui::MouseButton::Right);
    assert!(cx.debug_bounds("app_popover").is_some());
    for entry in [
        "view_diff",
        "open_in_worktree_tab",
        "open_file",
        "open_file_location",
        "copy_absolute_path",
        "copy_relative_path",
    ] {
        assert!(
            cx.debug_bounds(Box::leak(format!("context_menu_{entry}").into_boxed_str()))
                .is_some(),
            "missing menu action {entry}"
        );
    }
    for entry in [
        "stage",
        "unstage",
        "discard_changes",
        "edit_file",
        "apply_changes",
    ] {
        assert!(
            cx.debug_bounds(Box::leak(format!("context_menu_{entry}").into_boxed_str()))
                .is_none(),
            "foreign menus allow viewing and navigation only"
        );
    }
    let copy = cx
        .debug_bounds("context_menu_copy_absolute_path")
        .unwrap()
        .center();
    cx.simulate_click(copy, gpui::Modifiers::default());
    let copied = cx.update(|_, app| app.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(
        copied,
        Some(linked.join("a.rs").to_string_lossy().into_owned())
    );
    click_linked_file(cx, 0, gpui::MouseButton::Right);
    let show = cx.debug_bounds("context_menu_view_diff").unwrap().center();
    cx.simulate_click(show, gpui::Modifiers::default());
    assert_linked_diff(
        cx,
        &view,
        &store,
        &linked,
        &DiffTarget::working_tree(PathBuf::from("a.rs"), DiffArea::Staged),
        "linked staged",
    );
    click_linked_file(cx, 1, gpui::MouseButton::Right);
    let open = cx
        .debug_bounds("context_menu_open_in_worktree_tab")
        .unwrap()
        .center();
    cx.simulate_click(open, gpui::Modifiers::default());
    settle(cx, &view, &store, "diff in linked tab", |cx| {
        cx.update(|_, app| {
            view.read(app).active_repo().is_some_and(|repo| {
                repo.spec.workdir == linked
                    && repo.diff_state.diff_target.as_ref()
                        == Some(&DiffTarget::working_tree(
                            PathBuf::from("a.rs"),
                            DiffArea::Unstaged,
                        ))
                    && matches!(repo.diff_state.diff, Loadable::Ready(_))
            })
        })
    });
}

#[gpui::test]
fn sorted_linked_worktree_file_clicks_open_untracked_and_deleted_content(
    cx: &mut gpui::TestAppContext,
) {
    let (_dir, linked, store, view, cx) = open_linked_worktree_changes(cx);
    for layout in [FileListLayout::Flat, FileListLayout::Tree] {
        cx.update(|_, app| {
            view.read(app).details_pane.clone().update(app, |pane, cx| {
                pane.set_file_list_sort(
                    crate::view::rows::FileListId::WorktreeFiles,
                    crate::view::rows::CommitFileSort::PathDescending,
                    cx,
                );
                pane.set_file_list_layout(layout, cx);
            })
        });
        settle(cx, &view, &store, "shared worktree sorting", |cx| {
            cx.update(|_, app| {
                view.read(app)
                    .details_pane
                    .read(app)
                    .file_list_sort_for(crate::view::rows::FileListId::WorktreeFiles)
                    == crate::view::rows::CommitFileSort::PathDescending
            })
        });
        for (row, path, content) in [(0, "c.rs", "linked untracked"), (1, "b.rs", "-b 0")] {
            click_linked_file(cx, row, gpui::MouseButton::Left);
            assert_linked_diff(
                cx,
                &view,
                &store,
                &linked,
                &DiffTarget::working_tree(PathBuf::from(path), DiffArea::Unstaged),
                content,
            );
        }
    }
}

/// An owner's bar: its buttons show in the pane's bottom bar, one that needs
/// a selection waits for it and is run with it, and `set_bar` replaces them.
/// A snapshot pane has no list to step through, so no arrows.
#[gpui::test]
fn a_pane_draws_its_owners_bar(cx: &mut gpui::TestAppContext) {
    use gitcomet_extension_api::{DiffBar, DiffBarItem, DiffFileNavigation, DiffLineRange};
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, app_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    publish(app_cx, &view, Arc::new(AppState::test_default()));
    let host =
        app_cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());

    let runs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let item = |id: &'static str| {
        let runs = std::rc::Rc::clone(&runs);
        DiffBarItem::new(id, id, move |range, _| runs.borrow_mut().push((id, range)))
    };
    let pane = app_cx.update(|_window, app| {
        host.create_snapshot_pane(
            DiffSnapshot::new("notes.txt", "one\ntwo\n", "one\nTWO\n"),
            DiffPaneOptions {
                file_navigation: DiffFileNavigation {
                    previous: Some(gitcomet_extension_api::HostedAction::new("Back", |_| {})),
                    next: Some(gitcomet_extension_api::HostedAction::new("On", |_| {})),
                },
                bar: DiffBar::new()
                    .with_position(0, 5)
                    .with_item(
                        item("note")
                            .enabled_when(Option::is_some)
                            .with_shortcut("c"),
                    )
                    .with_item(item("done").primary())
                    .with_item(item("disabled").enabled(false)),
                layout: gitcomet_extension_api::DiffLayout::Inline,
                ..DiffPaneOptions::default()
            },
            app,
        )
        .unwrap()
    });
    app_cx.run_until_parked();
    let id = app_cx.update(|_window, app| {
        pane.view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap_or_else(|_| panic!("a hosted diff pane"))
            .read(app)
            .view_id()
    });
    let view_any = pane.view();
    let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(view_any));
    let draw = |cx: &mut gpui::VisualTestContext| {
        for _ in 0..3 {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
        }
    };
    draw(cx);

    for part in ["diff_bottom_bar", "diff_bar_note", "diff_bar_done"] {
        assert!(cx.debug_bounds(part).is_some(), "{part} is drawn");
    }
    assert!(
        cx.debug_bounds("diff_prev_file").is_none(),
        "a snapshot pane steps through no list"
    );

    // No selection yet: the note waits; the other runs without one.
    click_with(cx, "diff_bar_note", gpui::Modifiers::default());
    click_with(cx, "diff_bar_done", gpui::Modifiers::default());
    click_with(cx, "diff_bar_disabled", gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(*runs.borrow(), vec![("done", None)]);

    click_with(
        cx,
        selector(format!("hosted_diff_{id}_row_0")),
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    draw(cx);
    click_with(cx, "diff_bar_note", gpui::Modifiers::default());
    cx.run_until_parked();
    let selected = DiffLineRange {
        side: DiffLineSide::New,
        start: 1,
        end: 1,
    };
    assert_eq!(runs.borrow().last(), Some(&("note", Some(selected))));

    struct CustomControls;
    impl gpui::Render for CustomControls {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            div()
                .debug_selector(|| "custom_diff_bar_controls".to_string())
                .w(px(40.0))
                .h(px(20.0))
        }
    }
    cx.update(|_window, app| {
        let content = app.new(|_| CustomControls).into();
        pane.set_bar(DiffBar::new().with_content(content), app);
    });
    draw(cx);
    assert!(cx.debug_bounds("custom_diff_bar_controls").is_some());
    assert!(cx.debug_bounds("diff_bar_note").is_none());
    cx.update(|_window, app| pane.set_bar(DiffBar::new().with_item(item("other")), app));
    draw(cx);
    assert!(cx.debug_bounds("custom_diff_bar_controls").is_none());
    assert!(cx.debug_bounds("diff_bar_note").is_none());
    assert!(cx.debug_bounds("diff_bar_other").is_some());
}

#[gpui::test]
fn list_observers_follow_sort_layout_and_filter(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository(cx);
    let (host, repository) = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        (host, repository)
    });
    let list = cx.update(|_, app| {
        host.create_file_list(
            &repository,
            ChangeSource::comparison(CommitId("HEAD".into()), None, Default::default()),
            |_, _, _| {},
            app,
        )
        .unwrap()
    });
    settle(cx, &view, &store, "observed list loaded", |cx| {
        cx.update(|_, app| !list.is_loading(app) && list.ordered_paths(app).len() == 2)
    });
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let subscription = cx.update(|_, app| {
        let paths = list.clone();
        let seen = seen.clone();
        list.observe(
            move |app| seen.borrow_mut().push(paths.ordered_paths(app)),
            app,
        )
        .unwrap()
    });
    for mode in [
        gitcomet_extension_api::FileListMode::Flat,
        gitcomet_extension_api::FileListMode::Tree,
    ] {
        seen.borrow_mut().clear();
        cx.update(|_, app| list.set_mode(mode, app));
        cx.run_until_parked();
        assert!(!seen.borrow().is_empty(), "layout changes notify the owner");
    }
    seen.borrow_mut().clear();
    cx.update(|_, app| list.set_sort(gitcomet_extension_api::FileListSort::PathDescending, app));
    cx.run_until_parked();
    assert_eq!(
        seen.borrow().last(),
        Some(&vec![PathBuf::from("b.rs"), PathBuf::from("a.rs")])
    );
    seen.borrow_mut().clear();
    cx.update(|_, app| list.set_filter("a.rs", app));
    cx.run_until_parked();
    assert_eq!(seen.borrow().last(), Some(&vec![PathBuf::from("a.rs")]));
    drop(subscription);
}
