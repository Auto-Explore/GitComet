//! Hosted panes over a real repository: two diff panes and a file list,
//! each independent of the others and of History.

use super::*;
use gitcomet_extension_api::{
    ChangeSource, DiffLineSide, DiffPane, DiffPaneOptions, DiffPanePolicy, DiffRowDecor,
    DiffSnapshot, Registry,
};
use gitcomet_state::model::Loadable;

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
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
        let registry = Registry::build(&[Box::new(
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
fn the_example_changes_view_shows_picks_in_two_panes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_dir, store, view, cx) = open_repository(cx);
    click_debug_selector(cx, "repository_view_1");
    let list_id = {
        let mut id = None;
        settle(cx, &view, &store, "the example's file list", |cx| {
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
        id.unwrap().0
    };
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

    click_debug_selector(
        cx,
        selector(format!("hosted_file_list_{list_id}_file_b.rs")),
    );
    settle(cx, &view, &store, "the second pick", |cx| {
        let sessions = store.snapshot().repos[0].diff_sessions.len() == 2;
        let drawn = store.snapshot().repos[0].diff_sessions.keys().all(|id| {
            cx.debug_bounds(selector(format!("hosted_diff_{}_row_0", id.0)))
                .is_some()
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
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
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
