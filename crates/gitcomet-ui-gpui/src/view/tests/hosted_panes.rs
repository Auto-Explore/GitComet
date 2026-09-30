//! Hosted panes over a real repository: two diff panes and a file list,
//! each independent of the others and of History.

use super::*;
use gitcomet_extension_api::{
    ChangeSource, DiffLineSide, DiffPane, DiffPaneOptions, DiffPanePolicy, DiffRowDecor,
    DiffSnapshot, Registry,
};
use gitcomet_state::model::Loadable;

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
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "T"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // Same bytes under any user or system config (Windows CI sets autocrlf).
    git(root, &["config", "core.autocrlf", "false"]);
    std::fs::write(root.join("a.rs"), numbered("a", 30, None)).unwrap();
    std::fs::write(root.join("b.rs"), numbered("b", 30, None)).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "init"]);
    std::fs::write(root.join("a.rs"), numbered("a", 30, Some(3))).unwrap();
    std::fs::write(root.join("b.rs"), numbered("b", 30, Some(20))).unwrap();

    install_example(cx);
    let repo = gitcomet_git_gix::GixBackend.open(root).expect("open repo");
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let _guard = crate::test_support::lock_visual_test();
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
                ChangeSource::Worktree {
                    area,
                    include_untracked,
                },
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
    let _visual_guard = crate::test_support::lock_visual_test();
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
                ChangeSource::Comparison {
                    from: gitcomet_core::domain::CommitId("HEAD".into()),
                    to: None,
                    options: Default::default(),
                },
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
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_dir, store, view, cx) = open_repository(cx);
    let (host, repository) = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        (host, repository)
    });
    let target = |path: &str| DiffTarget::working_tree(path.into(), DiffArea::Unstaged);
    let source = ChangeSource::Comparison {
        from: CommitId("HEAD".into()),
        to: None,
        options: Default::default(),
    };
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
    // These panes have no dispatcher: only the explicitly published snapshots
    // can advance them, making the notification-before-Open race deterministic.
    let (pane_entity, list_entity) = cx.update(|_, app| {
        (
            app.new(|cx| {
                DiffPaneView::new(
                    host.clone(),
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
    let _visual_guard = crate::test_support::lock_visual_test();
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

#[gpui::test]
fn snapshot_panes_diff_texts_without_a_repository(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
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
    let _visual_guard = crate::test_support::lock_visual_test();
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

#[gpui::test]
fn pane_contributions_annotate_act_and_inset_without_touching_file_lines(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_extension_api::{
        DiffAnnotation, DiffAnnotations, DiffInset, DiffLegendItem, DiffLineRange,
        DiffSelectionAction,
    };
    let _visual_guard = crate::test_support::lock_visual_test();
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
    let (id, marker_builds) = app_cx.update(|_window, app| {
        (
            pane_view.read(app).view_id(),
            pane_view.read(app).marker_builds,
        )
    });

    let view_any = pane.view();
    let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(view_any));
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
        cx.update(|_window, app| pane_view.read(app).marker_builds),
        marker_builds,
        "drawing never places markers"
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
    let _visual_guard = crate::test_support::lock_visual_test();
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
    let _visual_guard = crate::test_support::lock_visual_test();
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
