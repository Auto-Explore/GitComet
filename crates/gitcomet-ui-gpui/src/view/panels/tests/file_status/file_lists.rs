//! File lists: tree layout, ordering, filtering, and navigation order.

use super::*;

#[gpui::test]
fn commit_details_added_file_copy_path_works_after_left_clicking_menu_entry(
    cx: &mut gpui::TestAppContext,
) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(60);
    let commit_sha = "0123456789abcdef0123456789abcdef01234567".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_added_copy_path",
        std::process::id()
    ));
    let added_path = std::path::Path::new("src").join("added_from_commit.rs");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone().into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![gitcomet_core::domain::CommitId(
                        "89abcdef0123456789abcdef0123456789abcdef".into(),
                    )],
                    files: vec![gitcomet_core::domain::CommitFileChange::new(
                        added_path.clone(),
                        gitcomet_core::domain::FileStatusKind::Added,
                    )],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    cx.write_to_clipboard(gpui::ClipboardItem::new_string("initial".to_string()));

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let row_bounds = cx
        .debug_bounds("commit_file_60_0")
        .expect("expected added commit file row");
    let row_center = row_bounds.center();
    cx.simulate_mouse_move(row_center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        row_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        row_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let copy_bounds = cx
        .debug_bounds("context_menu_copy_absolute_path")
        .expect("expected Copy absolute path context menu row");
    let copy_center = copy_bounds.center();
    cx.simulate_mouse_move(copy_center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        copy_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("initial".to_string())
    );
    cx.simulate_mouse_up(
        copy_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(workdir.join(&added_path).display().to_string())
    );
}

#[gpui::test]
fn commit_details_file_right_click_only_opens_menu_for_added_modified_and_deleted(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(61);
    let commit_sha = "0123456789abcdef0123456789abcdef01234567".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_file_right_click_menu_only",
        std::process::id()
    ));
    let files = [
        (
            std::path::PathBuf::from("src/added.rs"),
            gitcomet_core::domain::FileStatusKind::Added,
        ),
        (
            std::path::PathBuf::from("src/deleted.rs"),
            gitcomet_core::domain::FileStatusKind::Deleted,
        ),
        (
            std::path::PathBuf::from("src/modified.rs"),
            gitcomet_core::domain::FileStatusKind::Modified,
        ),
    ];
    let initial_target = gitcomet_core::domain::DiffTarget::working_tree(
        std::path::PathBuf::from("src/current.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(initial_target.clone());
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone().into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![gitcomet_core::domain::CommitId(
                        "89abcdef0123456789abcdef0123456789abcdef".into(),
                    )],
                    files: files
                        .iter()
                        .map(|(path, kind)| {
                            gitcomet_core::domain::CommitFileChange::new(path.clone(), *kind)
                        })
                        .collect(),
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    for (ix, (path, _kind)) in files.iter().enumerate() {
        let row_selector = format!("commit_file_{}_{}", repo_id.0, ix);
        let row_bounds = cx
            .debug_bounds(Box::leak(row_selector.into_boxed_str()))
            .expect("expected commit file row");
        let row_center = row_bounds.center();
        cx.simulate_mouse_move(row_center, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(
            row_center,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            row_center,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );

        let (popover_kind, diff_target) = cx.update(|_window, app| {
            let view = view.read(app);
            let popover_kind = view.popover_host.read(app).popover_kind_for_tests();
            let diff_target = view
                .state
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .and_then(|repo| repo.diff_state.diff_target.clone());
            (popover_kind, diff_target)
        });

        assert_eq!(
            popover_kind,
            Some(PopoverKind::CommitFileMenu {
                repo_id,
                commit_id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                path: path.clone(),
            })
        );
        assert_eq!(diff_target, Some(initial_target.clone()));

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.close_popover(cx);
                });
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
    }
}

#[gpui::test]
fn commit_details_file_list_keeps_visible_viewport_when_overflowing(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(61);
    let commit_sha = "0123456789abcdef0123456789abcdef01234567".to_string();
    let files = (0..48)
        .map(|ix| {
            gitcomet_core::domain::CommitFileChange::new(
                std::path::PathBuf::from(format!("src/commit_details/dir_{ix}/file_{ix}.rs")),
                gitcomet_core::domain::FileStatusKind::Modified,
            )
        })
        .collect::<Vec<_>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-files-list"));
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone().into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![gitcomet_core::domain::CommitId(
                        "89abcdef0123456789abcdef0123456789abcdef".into(),
                    )],
                    files,
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.simulate_resize(gpui::size(px(1024.0), px(420.0)));

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let mut viewport_height = 0.0f32;
    let mut contents_height = 0.0f32;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        let item_size = pane
            .commit_files_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected commit details files list to report its measured viewport");
        viewport_height = item_size.item.height.into();
        contents_height = item_size.contents.height.into();
    });

    assert!(
        contents_height > viewport_height,
        "expected commit details file list to overflow so the scrollbar has content to represent (viewport_height={viewport_height}, contents_height={contents_height})",
    );
    assert!(
        viewport_height >= 24.0,
        "expected commit details file list to keep at least one visible row when overflowing (viewport_height={viewport_height}, contents_height={contents_height})",
    );
}

#[gpui::test]
fn commit_details_file_controls_render_filter_and_open_the_sort_menu(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(611);
    let commit_id = gitcomet_core::domain::CommitId("0123456789abcdef".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-file-controls"));
            repo.history_state.selected_commit = Some(commit_id.clone());
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: commit_id.clone(),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![
                        gitcomet_core::domain::CommitFileChange::new(
                            "src/modified.rs".into(),
                            gitcomet_core::domain::FileStatusKind::Modified,
                        )
                        .with_line_counts(Some(1), Some(2)),
                        gitcomet_core::domain::CommitFileChange::new(
                            "src/removed.rs".into(),
                            gitcomet_core::domain::FileStatusKind::Deleted,
                        )
                        .with_line_counts(Some(0), Some(4)),
                        gitcomet_core::domain::CommitFileChange::new(
                            "src/added.rs".into(),
                            gitcomet_core::domain::FileStatusKind::Added,
                        )
                        .with_line_counts(Some(5), Some(0)),
                        gitcomet_core::domain::CommitFileChange::new(
                            "src/renamed.rs".into(),
                            gitcomet_core::domain::FileStatusKind::Renamed,
                        )
                        .with_line_counts(Some(0), Some(0)),
                        gitcomet_core::domain::CommitFileChange::new(
                            "src/added-small.rs".into(),
                            gitcomet_core::domain::FileStatusKind::Added,
                        )
                        .with_line_counts(Some(1), Some(0)),
                    ],
                },
            ));
            repo.history_state.commit_details_rev = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    assert!(cx.debug_bounds("commit_file_sort_button").is_some());
    assert!(cx.debug_bounds("commit_file_filter_tabs").is_some());
    for selector in [
        "commit_file_filter_tab_0",
        "commit_file_filter_tab_1",
        "commit_file_filter_tab_2",
        "commit_file_filter_tab_3",
        "commit_file_filter_tab_4",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "expected {selector} to be rendered"
        );
    }

    let added = cx
        .debug_bounds("commit_file_filter_tab_3")
        .expect("expected Added filter tab");
    cx.simulate_click(added.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_file_filter,
            crate::view::rows::CommitFileFilter::Added
        );
        assert_eq!(
            pane.active_commit_file_source_indices(repo_id)
                .expect("expected commit-file projection")
                .as_ref(),
            &[4, 2]
        );
    });

    let sort = cx
        .debug_bounds("commit_file_sort_button")
        .expect("expected commit file sort button");
    cx.simulate_click(sort.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_window, app| view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()),
        Some(PopoverKind::CommitFileSortMenu {
            list: crate::view::rows::FileListId::CommitFiles
        })
    );

    draw_and_drain_test_window(cx);
    let largest = cx
        .debug_bounds("context_menu_edit_size_largest")
        .expect("expected Edit size: Largest menu entry");
    cx.simulate_click(largest.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_file_sort,
            crate::view::rows::CommitFileSort::EditSizeDescending
        );
        assert_eq!(
            pane.commit_file_filter,
            crate::view::rows::CommitFileFilter::Added,
            "changing sort keeps the selected filter"
        );
        assert_eq!(
            pane.active_commit_file_source_indices(repo_id)
                .expect("expected sorted commit-file projection")
                .as_ref(),
            &[2, 4]
        );
    });
}

#[gpui::test]
fn commit_file_filter_resets_on_commit_switch_while_sort_is_retained(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(612);
    let make_state = |suffix: &str| {
        let commit_id = gitcomet_core::domain::CommitId(format!("commit-{suffix}").into());
        let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-file-state"));
        repo.history_state.selected_commit = Some(commit_id.clone());
        repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
            gitcomet_core::domain::CommitDetails {
                id: commit_id,
                message: "subject".to_string(),
                author_name: String::new(),
                author_email: String::new(),
                authored_at_unix: 0,
                committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                committed_at_unix: 0,
                parent_ids: vec![],
                files: vec![],
            },
        ));
        app_state_with_repo(repo, repo_id)
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state("a"), cx);
            this.details_pane.update(cx, |pane, cx| {
                pane.set_commit_file_sort(
                    crate::view::rows::CommitFileSort::EditSizeDescending,
                    cx,
                );
                pane.set_commit_file_filter(crate::view::rows::CommitFileFilter::Removed, cx);
            });
        });
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| push_test_state(this, make_state("b"), cx));
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_file_sort,
            crate::view::rows::CommitFileSort::EditSizeDescending
        );
        assert_eq!(
            pane.commit_file_filter,
            crate::view::rows::CommitFileFilter::All
        );
    });
}

/// Commit-diff file navigation is rendered by the cached main pane but derives
/// its endpoints from the details pane's active file projection. Guard that
/// cross-entity invalidation boundary for both ways the projection can change.
#[gpui::test]
fn commit_file_projection_changes_notify_the_cached_main_pane(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let (details_pane, main_pane) = cx.update(|_window, app| {
        let view = view.read(app);
        (view.details_pane.clone(), view.main_pane.clone())
    });

    let main_notifies = Arc::new(AtomicUsize::new(0));
    let _main_notify_subscription = cx.update(|_window, app| {
        let main_notifies = Arc::clone(&main_notifies);
        main_pane.update(app, |_pane, cx| {
            cx.observe_self(move |_pane, _cx| {
                main_notifies.fetch_add(1, Ordering::Relaxed);
            })
        })
    });
    cx.run_until_parked();

    main_notifies.store(0, Ordering::Relaxed);
    cx.update(|_window, app| {
        details_pane.update(app, |pane, cx| {
            pane.set_commit_file_filter(crate::view::rows::CommitFileFilter::Added, cx);
        });
    });
    cx.run_until_parked();
    assert!(
        main_notifies.load(Ordering::Relaxed) > 0,
        "filtering the commit file projection must invalidate the cached main pane"
    );

    main_notifies.store(0, Ordering::Relaxed);
    cx.update(|_window, app| {
        details_pane.update(app, |pane, cx| {
            pane.set_commit_file_sort(crate::view::rows::CommitFileSort::EditSizeDescending, cx);
        });
    });
    cx.run_until_parked();
    assert!(
        main_notifies.load(Ordering::Relaxed) > 0,
        "sorting the commit file projection must invalidate the cached main pane"
    );
}

#[gpui::test]
fn ui_scale_commit_details_file_list_content_height_scales(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(62);
    let commit_sha = "fedcba9876543210fedcba9876543210fedcba98".to_string();
    let files = (0..48)
        .map(|ix| {
            gitcomet_core::domain::CommitFileChange::new(
                std::path::PathBuf::from(format!("src/commit_zoom/dir_{ix}/file_{ix}.rs")),
                gitcomet_core::domain::FileStatusKind::Modified,
            )
        })
        .collect::<Vec<_>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-files-zoom"));
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone().into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![gitcomet_core::domain::CommitId(
                        "89abcdef0123456789abcdef0123456789abcdef".into(),
                    )],
                    files,
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    cx.simulate_resize(gpui::size(px(1024.0), px(420.0)));
    draw_and_drain_test_window(cx);

    let default_contents_height = cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let item_size = pane
            .commit_files_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected commit details files list measurements at the default zoom");
        let height: f32 = item_size.contents.height.into();
        height
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.apply_ui_scale_percent(200, window, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let zoomed_contents_height = cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let item_size = pane
            .commit_files_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected commit details files list measurements after zooming");
        let height: f32 = item_size.contents.height.into();
        height
    });

    assert!(
        zoomed_contents_height > default_contents_height * 1.7,
        "expected the commit details file list content height to grow substantially with zoom (default={default_contents_height}, zoomed={zoomed_contents_height})",
    );
}

#[gpui::test]
fn details_row_renderers_begin_separate_alignment_groups_for_status_and_commit_files(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(631);
    let commit_id = gitcomet_core::domain::CommitId("0123456789abcdef".into());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo =
                opening_repo_state(repo_id, Path::new("/tmp/repo-details-path-alignment"));
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(
                                "staged/really_long_directory_name/files/staged_alpha.rs",
                            ),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(
                                "staged/another_super_long_directory_name/files/staged_beta.rs",
                            ),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ]),
                    unstaged: std::sync::Arc::new(vec![
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(
                                "src/components/really_long_directory_name/status/file_name_alpha.rs",
                            ),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(
                                "src/components/dir/another_super_long_directory_name/file_name_beta.rs",
                            ),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ]),
                }
                .into(),
            );
            repo.status_rev = repo.status_rev.wrapping_add(1);
            repo.history_state.selected_commit = Some(commit_id.clone());
            repo.history_state.selected_commit_rev =
                repo.history_state.selected_commit_rev.wrapping_add(1);
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: commit_id.clone(),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![
                        gitcomet_core::domain::CommitFileChange::new(std::path::PathBuf::from(
                                "history/really_long_commit_directory_name/files/commit_file_alpha.rs",
                            ), gitcomet_core::domain::FileStatusKind::Modified),
                        gitcomet_core::domain::CommitFileChange::new(std::path::PathBuf::from(
                                "history/dir/another_super_long_commit_directory_name/commit_file_beta.rs",
                            ), gitcomet_core::domain::FileStatusKind::Modified),
                    ],
                },
            ));
            repo.history_state.commit_details_rev =
                repo.history_state.commit_details_rev.wrapping_add(1);

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            let unstaged =
                crate::view::panes::DetailsPaneView::render_unstaged_rows(pane, 0..2, window, cx);
            let staged =
                crate::view::panes::DetailsPaneView::render_staged_rows(pane, 0..2, window, cx);
            let commit_files = crate::view::panes::DetailsPaneView::render_commit_file_rows(
                pane,
                0..2,
                window,
                cx,
            );

            assert_eq!(unstaged.len(), 2);
            assert_eq!(staged.len(), 2);
            assert_eq!(commit_files.len(), 2);
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let staged = pane.staged_path_alignment_group.snapshot_for_test();
        let unstaged = pane.unstaged_path_alignment_group.snapshot_for_test();
        let commit_files = pane.commit_files_path_alignment_group.snapshot_for_test();
        let untracked = pane.untracked_path_alignment_group.snapshot_for_test();

        assert!(staged.visible_signature.is_some());
        assert!(unstaged.visible_signature.is_some());
        assert!(commit_files.visible_signature.is_some());
        assert_eq!(untracked.visible_signature, None);
        assert_ne!(staged.visible_signature, unstaged.visible_signature);
        assert_ne!(unstaged.visible_signature, commit_files.visible_signature);
        assert_ne!(staged.visible_signature, commit_files.visible_signature);
    });
}

/// The worktree scan revision bumps per repo-wide rescan, not per worktree, so
/// two worktrees with the same file count and the same visible range would share
/// a path-truncation signature if the path were left out of it — and the second
/// one would render with the first one's measured alignment.
#[gpui::test]
fn worktree_file_alignment_signatures_differ_per_worktree(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let repo_id = gitcomet_state::model::RepoId(4);
        let signature = |path: &str| {
            pane.worktree_files_visible_signature(
                repo_id,
                7,
                std::path::Path::new(path),
                &(0..5),
                5,
            )
        };

        assert_ne!(
            signature("/wt/a"),
            signature("/wt/b"),
            "two worktrees scanned at the same revision must not share a signature"
        );
        assert_eq!(
            signature("/wt/a"),
            signature("/wt/a"),
            "the same worktree keeps its alignment across renders"
        );
    });
}

/// The worktree file list is virtualized, but its inputs are one entry per
/// changed file: building them inline made every layout pass O(all files). They
/// are derived once per scan instead, and keyed by worktree — the scan revision
/// alone bumps for the whole repo, so it cannot tell two worktrees apart.
#[gpui::test]
fn worktree_file_inputs_are_derived_once_per_scan_and_keyed_by_worktree(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let summary = |path: &str, files: usize| gitcomet_core::domain::WorktreeDirtySummary {
        path: std::path::PathBuf::from(path),
        head: Some(gitcomet_core::domain::CommitId("tip".into())),
        branch: Some("side".to_string()),
        detached: false,
        added: files,
        modified: 0,
        deleted: 0,
        staged: (0..files)
            .map(|ix| gitcomet_core::domain::FileStatus {
                path: std::path::PathBuf::from(format!("staged_{ix}.rs")),
                kind: gitcomet_core::domain::FileStatusKind::Added,
                conflict: None,
            })
            .collect(),
        unstaged: (0..files)
            .map(|ix| gitcomet_core::domain::FileStatus {
                path: std::path::PathBuf::from(format!("unstaged_{ix}.rs")),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            })
            .collect(),
        line_stats: Default::default(),
    };

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let repo_id = gitcomet_state::model::RepoId(9);
        let worktree_a = summary("/wt/a", 3);
        let worktree_b = summary("/wt/b", 3);

        let first = pane.cached_worktree_file_inputs(repo_id, 1, &worktree_a);
        let again = pane.cached_worktree_file_inputs(repo_id, 1, &worktree_a);
        assert!(
            Arc::ptr_eq(&first, &again),
            "a second frame at the same scan revision must reuse the derived inputs"
        );
        assert_eq!(first.files.len(), 6);
        assert_eq!(first.entries.len(), 6);

        // `selected_ix` indexes `entries` by the row's position in `files`, so the
        // two vectors have to stay in step -- staged first, then unstaged, each
        // entry pointing at its own file and carrying the section it came from.
        let staged_then_unstaged: Vec<_> =
            first.files.iter().map(|file| file.path.clone()).collect();
        assert_eq!(
            staged_then_unstaged,
            vec![
                std::path::PathBuf::from("staged_0.rs"),
                std::path::PathBuf::from("staged_1.rs"),
                std::path::PathBuf::from("staged_2.rs"),
                std::path::PathBuf::from("unstaged_0.rs"),
                std::path::PathBuf::from("unstaged_1.rs"),
                std::path::PathBuf::from("unstaged_2.rs"),
            ],
            "staged files come first, in scan order"
        );
        for (ix, entry) in first.entries.iter().enumerate() {
            assert_eq!(
                entry.path, first.files[ix].path,
                "entry {ix} must describe the file rendered at row {ix}"
            );
            let staged = ix < 3;
            assert_eq!(
                entry.section,
                if staged {
                    gitcomet_state::model::InlineSubmoduleDiffSection::LiveStaged
                } else {
                    gitcomet_state::model::InlineSubmoduleDiffSection::LiveUnstaged
                },
                "entry {ix} must open in the section its file was scanned in"
            );
            assert_eq!(
                entry.target,
                gitcomet_core::domain::DiffTarget::working_tree(
                    first.files[ix].path.clone(),
                    if staged {
                        gitcomet_core::domain::DiffArea::Staged
                    } else {
                        gitcomet_core::domain::DiffArea::Unstaged
                    }
                ),
                "entry {ix} must diff against the right side of the index"
            );
        }

        // Same repo, same revision, same file count: only the path tells them apart.
        let other = pane.cached_worktree_file_inputs(repo_id, 1, &worktree_b);
        assert!(
            !Arc::ptr_eq(&first, &other),
            "another worktree must not be served the first one's files"
        );

        let rescanned = pane.cached_worktree_file_inputs(repo_id, 2, &worktree_b);
        assert!(
            !Arc::ptr_eq(&other, &rescanned),
            "a new scan revision must rebuild them"
        );
    });
}

fn commit_details_state_with_paths(
    repo_id: gitcomet_state::model::RepoId,
    commit_id: &gitcomet_core::domain::CommitId,
    paths: &[&str],
    rev: u64,
) -> gitcomet_state::model::RepoState {
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-file-tree"));
    repo.history_state.selected_commit = Some(commit_id.clone());
    repo.history_state.commit_details =
        gitcomet_state::model::Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
            id: commit_id.clone(),
            message: "subject".to_string(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at_unix: 0,
            committed_at: "2026-03-08 12:34:56 +0200".to_string(),
            committed_at_unix: 0,
            parent_ids: vec![],
            files: paths
                .iter()
                .map(|path| {
                    gitcomet_core::domain::CommitFileChange::new(
                        (*path).into(),
                        gitcomet_core::domain::FileStatusKind::Modified,
                    )
                    .with_line_counts(Some(1), Some(1))
                })
                .collect(),
        }));
    repo.history_state.commit_details_rev = rev;
    // The details pane only re-reads a snapshot when its notify fingerprint
    // moves, and that hashes the rev, not the commit id.
    repo.history_state.selected_commit_rev = rev;
    repo
}

#[gpui::test]
fn commit_file_layout_toggle_groups_rows_into_folders(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(613);
    let commit_id = gitcomet_core::domain::CommitId("tree0123456789ab".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_details_state_with_paths(
                repo_id,
                &commit_id,
                &["src/view/a.rs", "src/view/b.rs", "root.rs"],
                1,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds(leaked(format!("commit_file_dir_{}_0", repo_id.0)))
            .is_none(),
        "flat is the default, so no folder rows"
    );

    let toggle = cx
        .debug_bounds("commit_file_layout_button")
        .expect("expected the layout toggle");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.file_list_layout_for(repo_id, crate::view::rows::FileListId::CommitFiles),
            crate::view::FileListLayout::Tree
        );
    });
    assert!(
        cx.debug_bounds(leaked(format!("commit_file_dir_{}_0", repo_id.0)))
            .is_some(),
        "the single-child chain src/view collapses into one folder row"
    );
}

/// Collapsing hides rows; it must not narrow what F1/F4 steps through, or the
/// prev/next arrows vanish the moment a folder is shut.
#[gpui::test]
fn collapsing_a_folder_keeps_every_file_navigable(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(614);
    let commit_id = gitcomet_core::domain::CommitId("collapse0123456a".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_details_state_with_paths(
                repo_id,
                &commit_id,
                &["src/view/a.rs", "src/view/b.rs", "root.rs"],
                1,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    // Drawn before the toggle: the snapshot lands on the next draw, and it is
    // what resets a per-list override.
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::CommitFiles,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    let before = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .active_commit_file_source_indices(repo_id)
            .expect("expected a projection")
    });
    assert_eq!(before.len(), 3);

    let folder = cx
        .debug_bounds(leaked(format!("commit_file_dir_{}_0", repo_id.0)))
        .expect("expected a folder row");
    cx.simulate_click(folder.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    let after = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .active_commit_file_source_indices(repo_id)
            .expect("expected a projection")
    });
    assert_eq!(
        after.as_ref(),
        before.as_ref(),
        "collapsing changes which rows render, not which files navigate"
    );
    // Rows are [folder, root.rs] now; before the collapse they were
    // [folder, a.rs, b.rs, root.rs].
    assert!(
        cx.debug_bounds(leaked(format!("commit_file_{}_3", repo_id.0)))
            .is_none(),
        "the collapsed folder's children stop rendering"
    );
}

/// The override is scoped to the commit it was made on.
#[gpui::test]
fn commit_file_layout_override_resets_on_commit_switch(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(615);
    let first = gitcomet_core::domain::CommitId("first01234567890".into());
    let second = gitcomet_core::domain::CommitId("second1234567890".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_details_state_with_paths(repo_id, &first, &["src/view/a.rs"], 1);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::CommitFiles,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.file_list_layout_for(repo_id, crate::view::rows::FileListId::CommitFiles),
            crate::view::FileListLayout::Tree,
            "the override applies while the commit is unchanged"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_details_state_with_paths(repo_id, &second, &["src/view/a.rs"], 2);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.file_list_layout_for(repo_id, crate::view::rows::FileListId::CommitFiles),
            crate::view::FileListLayout::Flat,
            "a different commit re-reads the global default"
        );
    });
}

#[gpui::test]
fn status_layout_toggle_groups_rows_into_folders(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(620);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo =
                repo_with_unstaged_paths(repo_id, &["src/view/a.rs", "src/view/b.rs", "root.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    let dir_selector = leaked(format!("status_dir_{}_unstaged_0", repo_id.0));
    assert!(
        cx.debug_bounds(dir_selector).is_none(),
        "flat is the default"
    );

    let toggle = cx
        .debug_bounds("status_unstaged_layout_button")
        .expect("expected the unstaged layout toggle");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds(dir_selector).is_some(),
        "src/view folds into one folder row"
    );
}

/// The anchor index is a position in the display order. Sorting reorders the
/// list without touching `status_rev`, so an index hint captured under the old
/// order must not be trusted — it would silently select the wrong range.
#[gpui::test]
fn status_shift_click_range_follows_the_sorted_display_order(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(621);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(repo_id, &["a.rs", "b.rs", "c.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_status_file_sort(
                    StatusSection::CombinedUnstaged,
                    crate::view::rows::CommitFileSort::PathDescending,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    // Displayed order is now c, b, a.
    let first = cx
        .debug_bounds(leaked(format!("status_row_{}_unstaged_0", repo_id.0)))
        .expect("expected the first status row");
    cx.simulate_click(first.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    let third = cx
        .debug_bounds(leaked(format!("status_row_{}_unstaged_2", repo_id.0)))
        .expect("expected the third status row");
    cx.simulate_click(
        third.center(),
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let selected =
            pane.status_selected_paths_for_area(repo_id, gitcomet_core::domain::DiffArea::Unstaged);
        let mut names: Vec<String> = selected
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()],
            "shift-clicking row 0 to row 2 spans the whole displayed range"
        );
    });
}

/// Counts arrive on their own effect, after the list is drawn and without
/// `worktree_status_rev` moving. Keyed on the status rev alone, the cache would
/// keep serving pre-stats rows until some unrelated change shifted the key.
#[gpui::test]
fn status_rows_pick_up_line_stats_that_arrive_without_a_status_change(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(650);
    let paths = ["src/a.rs", "src/b.rs"];

    let folder_additions = |cx: &mut gpui::VisualTestContext| -> Option<u64> {
        cx.update(|_window, app| {
            let pane = view.read(app).details_pane.read(app);
            let repo = pane.active_repo().expect("active repo");
            let plan = pane.status_file_plan(repo, StatusSection::CombinedUnstaged);
            match plan.row_at(crate::view::rows::RowIx(0)) {
                Some(crate::view::rows::FileListRow::Directory { additions, .. }) => additions,
                other => panic!("expected a folder row, got {other:?}"),
            }
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                app_state_with_repo(repo_with_unstaged_paths(repo_id, &paths), repo_id),
                cx,
            );
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);
    assert_eq!(folder_additions(cx), None, "no counts before they load");

    // Same status, same rev — only the stats lane moves.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = repo_with_unstaged_paths(repo_id, &paths);
            let mut unstaged = rustc_hash::FxHashMap::default();
            for (ix, path) in paths.iter().enumerate() {
                unstaged.insert(
                    std::path::PathBuf::from(path),
                    gitcomet_core::domain::LineStats::from((Some(ix as u32 + 1), Some(1))),
                );
            }
            repo.uncommitted_line_stats = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::UncommittedLineStats {
                    staged: Default::default(),
                    unstaged,
                },
            ));
            repo.unstaged_line_stats_rev = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    assert_eq!(
        folder_additions(cx),
        Some(3),
        "the cached tree must be rebuilt when only the line-stats rev moves"
    );
}

/// A tree hoists directories above files, so a range computed in ordinal space
/// skips files displayed between the clicks — and those files then get staged.
#[gpui::test]
fn status_shift_click_in_tree_layout_spans_the_displayed_rows(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(660);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            // Projection 0,1,2 = root.rs, a, b; tree order [src/, a, b, root.rs].
            let repo = repo_with_unstaged_paths(repo_id, &["root.rs", "src/a.rs", "src/b.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    let row = |cx: &mut gpui::VisualTestContext, ix: usize| {
        cx.debug_bounds(leaked(format!("status_row_{}_unstaged_{ix}", repo_id.0)))
            .unwrap_or_else(|| panic!("expected status row {ix}"))
    };

    // Row 1 is src/a.rs, row 3 is root.rs, with src/b.rs displayed between them.
    let first = row(cx, 1);
    cx.simulate_click(first.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);
    let last = row(cx, 3);
    cx.simulate_click(
        last.center(),
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let mut names: Vec<String> = pane
            .status_selected_paths_for_area(repo_id, gitcomet_core::domain::DiffArea::Unstaged)
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "root.rs".to_string(),
                "src/a.rs".to_string(),
                "src/b.rs".to_string()
            ],
            "the range must cover every row displayed between the two clicks"
        );
    });
}

/// Sorting reorders rows without moving any status or line-stats rev, so an
/// anchor captured under the old order must not be trusted.
#[gpui::test]
fn status_shift_click_drops_an_anchor_invalidated_by_a_sort_change(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(670);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(repo_id, &["a.rs", "b.rs", "c.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    // Anchor a.rs at row 0.
    let first = cx
        .debug_bounds(leaked(format!("status_row_{}_unstaged_0", repo_id.0)))
        .expect("expected the first row");
    cx.simulate_click(first.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    // Z-A puts a.rs at row 2; no status or stats rev moves.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_status_file_sort(
                    StatusSection::CombinedUnstaged,
                    crate::view::rows::CommitFileSort::PathDescending,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    let same_file_now_last = cx
        .debug_bounds(leaked(format!("status_row_{}_unstaged_2", repo_id.0)))
        .expect("expected the third row");
    cx.simulate_click(
        same_file_now_last.center(),
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let names: Vec<String> = pane
            .status_selected_paths_for_area(repo_id, gitcomet_core::domain::DiffArea::Unstaged)
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["a.rs".to_string()],
            "shift-clicking the anchor's own row selects only it, whatever the sort"
        );
    });
}

/// A "Renamed" filter carried to a worktree with no renames would empty the
/// list under a header still counting changes.
#[gpui::test]
fn worktree_filter_resets_when_the_shown_worktree_changes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(690);
    let list = crate::view::rows::FileListId::WorktreeFiles;
    let with_selection = |path: &str, rev: u64| {
        let mut repo = repo_with_unstaged_paths(repo_id, &["a.rs"]);
        repo.history_state.worktree_selection = Some(std::path::PathBuf::from(path));
        repo.history_state.worktree_selection_rev = rev;
        repo
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                app_state_with_repo(with_selection("/tmp/wt-a", 1), repo_id),
                cx,
            );
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_file_list_filter(list, crate::view::rows::CommitFileFilter::Renamed, cx);
                assert_eq!(
                    pane.file_list_filter_for(list),
                    crate::view::rows::CommitFileFilter::Renamed,
                    "the filter applies to the repo it was set on"
                );
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                app_state_with_repo(with_selection("/tmp/wt-b", 2), repo_id),
                cx,
            );
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.file_list_filter_for(list),
            crate::view::rows::CommitFileFilter::All,
            "a different worktree must not inherit the previous one's filter"
        );
    });
}

/// Navigation hands over a position among files, but `scroll_to_item_strict`
/// indexes rows — which a tree pads with directories and may be hiding.
#[gpui::test]
fn commit_file_scroll_resolves_a_file_position_to_its_row(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(700);
    let commit_id = gitcomet_core::domain::CommitId("scroll0123456789".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            // Tree order [src/, a, b, root.rs]: file 0 is row 1, file 2 row 3.
            let repo = commit_details_state_with_paths(
                repo_id,
                &commit_id,
                &["root.rs", "src/a.rs", "src/b.rs"],
                1,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::CommitFiles,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    let row_for = |cx: &mut gpui::VisualTestContext, position: usize| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.details_pane
                    .update(cx, |pane, cx| pane.reveal_commit_file_row(position, cx))
            })
        })
    };

    assert_eq!(
        row_for(cx, 0),
        Some(1),
        "the first file sits below its folder"
    );
    assert_eq!(row_for(cx, 2), Some(3));

    // Collapsed, so its files have no row until the reveal reopens it.
    let folder = cx
        .debug_bounds(leaked(format!("commit_file_dir_{}_0", repo_id.0)))
        .expect("expected the folder row");
    cx.simulate_click(folder.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds(leaked(format!("commit_file_{}_2", repo_id.0)))
            .is_none(),
        "the folder is collapsed"
    );

    assert_eq!(
        row_for(cx, 0),
        Some(1),
        "navigating to a hidden file reopens its folder rather than scrolling to a stranger"
    );
}

/// Prev/next-file must step through the rows as drawn. The order navigation
/// walks comes from `active_status_section_order`, which is the projection —
/// and a tree reorders that.
#[gpui::test]
fn status_navigation_order_matches_the_drawn_rows_in_a_tree(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(710);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(repo_id, &["root.rs", "src/a.rs", "src/b.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let repo = pane.active_repo().expect("active repo");
        let entries = repo.worktree_status_entries().expect("status entries");
        let navigated: Vec<&std::path::Path> = pane
            .active_status_section_order(repo_id, StatusSection::CombinedUnstaged)
            .expect("nav order")
            .iter()
            .filter_map(|source_ix| entries.get(*source_ix).map(|entry| entry.path.as_path()))
            .collect();
        let drawn = pane.status_display_order_paths(repo_id, StatusSection::CombinedUnstaged);

        assert_eq!(
            navigated,
            drawn.iter().map(|path| path.as_path()).collect::<Vec<_>>(),
            "prev/next must visit files in the order the rows are drawn"
        );
        assert_eq!(drawn.len(), 3, "and the comparison is not two empty lists");
        for (position, path) in drawn.iter().enumerate() {
            assert_eq!(
                pane.status_path_display_position(StatusSection::CombinedUnstaged, path),
                Some(position)
            );
        }
        assert_eq!(
            pane.status_path_display_position(
                StatusSection::CombinedUnstaged,
                std::path::Path::new("missing")
            ),
            None
        );
    });
}

/// Frames caused elsewhere must not re-render the cached details pane while
/// it lists status files. `changed_file_list` refreshed each list view with
/// an unconditional notify from inside the details render, so after the pane's
/// first re-render every window frame re-rendered it, and laid out the list's
/// visible rows again.
#[gpui::test]
fn status_file_list_does_not_rerender_the_details_pane_on_unrelated_frames(
    cx: &mut gpui::TestAppContext,
) {
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(35);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = super::repo_with_unstaged_paths(repo_id, &["a.rs", "b.rs", "c.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    // One legitimate re-render, as any status publication causes.
    cx.update(|_window, app| {
        let details = view.read(app).details_pane.clone();
        details.update(app, |_, cx| cx.notify());
    });
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    let render_count = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| view.read(app).details_pane.read(app).render_count)
    };
    let before = render_count(cx);

    for _ in 0..5 {
        cx.update(|_window, app| {
            let sidebar = view.read(app).sidebar_pane.clone();
            sidebar.update(app, |_, cx| cx.notify());
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }

    assert_eq!(
        render_count(cx),
        before,
        "an unrelated frame re-rendered the details pane"
    );
}
