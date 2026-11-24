use super::*;

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
                cx.update(|window, app| {
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
                });
                publish(cx, &view, store.snapshot());
                for (key, stop) in [
                    ("f3", stops[1]),
                    ("f2", stops[0]),
                    ("shift-f3", stops[1]),
                    ("shift-f2", stops[0]),
                    ("f7", stops[1]),
                    ("shift-f7", stops[0]),
                ] {
                    cx.simulate_keystrokes(key);
                    publish(cx, &view, store.snapshot());
                    assert_eq!(
                        cx.update(|_, app| pane.read(app).diff_selection_anchor),
                        Some(stop),
                        "{key}, {layout:?}, {content:?}, detached focus: {detached}",
                    );
                    // Landing on a change block focuses its diff, for S and U.
                    assert!(cx.update(|window, app| {
                        pane.read(app).diff_panel_focus_handle.is_focused(window)
                    }));
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
