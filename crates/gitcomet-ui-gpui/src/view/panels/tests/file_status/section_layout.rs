//! Section layout: resizing, scroll viewports, and density sizing.

use super::*;

#[gpui::test]
fn status_section_drag_updates_saved_height(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(46);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_resize_drag",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("unstaged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let mut initial_status_sections_bounds = None;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        initial_status_sections_bounds = pane.current_status_sections_bounds();
        assert!(
            initial_status_sections_bounds.is_some(),
            "expected status sections to report measured bounds after draw"
        );
        assert_eq!(
            pane.saved_status_section_heights().0,
            None,
            "status resize height should start unset before dragging"
        );
    });

    let initial_handle_bounds = cx
        .debug_bounds("status_resize_change_tracking_staged")
        .expect("expected status resize handle bounds");
    let handle_center = initial_handle_bounds.center();
    let drag_target = gpui::point(handle_center.x, handle_center.y + px(48.0));
    let initial_change_tracking_height = initial_handle_bounds.top()
        - initial_status_sections_bounds
            .expect("expected status section bounds while computing drag start height")
            .top();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.status_section_resize = Some(StatusSectionResizeState {
                handle: StatusSectionResizeHandle::ChangeTrackingAndStaged,
                start_y: handle_center.y,
                start_height: initial_change_tracking_height,
            });
            assert!(
                pane.update_status_section_resize(drag_target.y, cx),
                "expected direct resize update to change the saved change-tracking height"
            );
            assert!(
                pane.finish_status_section_resize(cx),
                "expected direct resize finish to persist the updated change-tracking height"
            );
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert!(
            pane.change_tracking_height.is_some(),
            "expected dragging the resize handle to store a height"
        );
        assert!(
            pane.saved_status_section_heights().0.is_some(),
            "expected dragging the resize handle to persist a saved change-tracking height"
        );
    });
    let updated_handle_bounds = cx
        .debug_bounds("status_resize_change_tracking_staged")
        .expect("expected updated status resize handle bounds after dragging");
    assert!(
        updated_handle_bounds.top() > initial_handle_bounds.top(),
        "expected resizing the outer divider downward to move the staged section downward"
    );
}

#[gpui::test]
fn staged_section_remains_visible_after_window_resize_with_saved_split_height(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(51);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_resize_window_shrink",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(
                        (0..30)
                            .map(|ix| gitcomet_core::domain::FileStatus {
                                path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                                kind: gitcomet_core::domain::FileStatusKind::Modified,
                                conflict: None,
                            })
                            .collect(),
                    ),
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let mut initial_window_size = gpui::size(px(0.0), px(0.0));
    let mut initial_status_height = px(0.0);
    cx.update(|window, app| {
        initial_window_size = window.viewport_size();
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        initial_status_height = pane
            .current_status_sections_bounds()
            .expect("expected status section bounds before shrinking the window")
            .size
            .height;
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.change_tracking_height = Some(initial_status_height);
            cx.notify();
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

    cx.simulate_resize(gpui::size(
        initial_window_size.width,
        initial_window_size.height - px(120.0),
    ));

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let staged_header_bounds = cx
        .debug_bounds("staged_header")
        .expect("expected staged header bounds after shrinking the window");

    let mut staged_viewport_height = 0.0f32;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        staged_viewport_height = pane
            .staged_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected staged list viewport after shrinking the window")
            .item
            .height
            .into();
    });

    assert!(
        staged_viewport_height > 0.0,
        "expected staged section to keep a visible list viewport after shrinking the window (staged_header={staged_header_bounds:?}, staged_viewport_height={staged_viewport_height})"
    );
}

#[gpui::test]
fn split_status_section_resize_moves_untracked_section(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(47);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_status_resize_drag",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(vec![
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from("new.txt"),
                            kind: gitcomet_core::domain::FileStatusKind::Untracked,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from("tracked.txt"),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ]),
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.set_change_tracking_view(ChangeTrackingView::SplitUntracked, cx);
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

    let mut initial_stack_bounds = None;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            crate::view::test_support::change_tracking_view(view.read(app)),
            ChangeTrackingView::SplitUntracked,
            "expected the root view to store split change-tracking mode"
        );
        assert_eq!(
            pane.change_tracking_view,
            ChangeTrackingView::SplitUntracked,
            "expected the details pane to store split change-tracking mode"
        );
        assert!(
            pane.current_change_tracking_stack_bounds().is_some(),
            "expected split change-tracking stack bounds after initial draw"
        );
        initial_stack_bounds = pane.current_change_tracking_stack_bounds();
    });
    assert!(
        cx.debug_bounds("status_resize_change_tracking_staged")
            .is_some(),
        "expected the outer status resize handle to still be present in split mode"
    );
    let initial_split_unstaged_header_bounds = cx
        .debug_bounds("split_unstaged_header")
        .expect("expected split unstaged header bounds in split change-tracking view");

    let initial_handle_bounds = cx
        .debug_bounds("status_resize_untracked_unstaged")
        .expect("expected inner status resize handle bounds in split change-tracking view");
    let initial_untracked_wrapper_bounds = cx
        .debug_bounds("status_untracked_wrapper")
        .expect("expected untracked wrapper bounds in split change-tracking view");
    let handle_center = initial_handle_bounds.center();
    let drag_target = gpui::point(handle_center.x, handle_center.y + px(48.0));
    let initial_top_height = initial_handle_bounds.top()
        - initial_stack_bounds.expect(
            "expected initial split change-tracking stack bounds while computing drag start height",
        )
        .top();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.status_section_resize = Some(StatusSectionResizeState {
                handle: StatusSectionResizeHandle::UntrackedAndUnstaged,
                start_y: handle_center.y,
                start_height: initial_top_height,
            });
            assert!(
                pane.update_status_section_resize(drag_target.y, cx),
                "expected direct resize update to change the untracked height"
            );
            assert!(
                pane.finish_status_section_resize(cx),
                "expected direct resize finish to persist the updated height"
            );
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let mut updated_untracked_height = None;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            crate::view::test_support::change_tracking_view(view.read(app)),
            ChangeTrackingView::SplitUntracked,
            "expected split change-tracking view to remain active while resizing"
        );
        assert!(
            pane.untracked_height.is_some(),
            "expected dragging the inner resize handle to store an untracked height"
        );
        updated_untracked_height = pane.untracked_height;
    });
    let updated_handle_bounds = cx
        .debug_bounds("status_resize_untracked_unstaged")
        .expect("expected updated inner status resize handle bounds after dragging");
    let updated_split_unstaged_header_bounds = cx
        .debug_bounds("split_unstaged_header")
        .expect("expected updated split unstaged header bounds after dragging");
    assert!(
        updated_split_unstaged_header_bounds.top() > initial_split_unstaged_header_bounds.top(),
        "expected resizing the inner divider downward to move the split unstaged section downward (initial_header_top={:?}, updated_header_top={:?}, initial_untracked_wrapper={:?}, updated_untracked_height={:?})",
        initial_split_unstaged_header_bounds.top(),
        updated_split_unstaged_header_bounds.top(),
        initial_untracked_wrapper_bounds,
        updated_untracked_height,
    );
    assert!(
        updated_handle_bounds.center().y > initial_handle_bounds.center().y,
        "expected the inner divider to move downward after resizing (initial_handle_y={:?}, updated_handle_y={:?}, updated_untracked_height={:?})",
        initial_handle_bounds.center().y,
        updated_handle_bounds.center().y,
        updated_untracked_height,
    );
}

#[gpui::test]
fn unstaged_scroll_viewport_tracks_resized_section_height(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(48);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unstaged_scroll_viewport",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(
                        (0..30)
                            .map(|ix| gitcomet_core::domain::FileStatus {
                                path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                                kind: gitcomet_core::domain::FileStatusKind::Modified,
                                conflict: None,
                            })
                            .collect(),
                    ),
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.change_tracking_height = Some(px(160.0));
            cx.notify();
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

    let unstaged_wrapper_bounds = cx
        .debug_bounds("status_change_tracking_wrapper")
        .expect("expected unstaged section bounds after resizing");
    let unstaged_header_bounds = cx
        .debug_bounds("unstaged_header")
        .expect("expected unstaged header bounds after resizing");

    let mut is_scrollable = false;
    let mut viewport_height = 0.0f32;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        is_scrollable = pane.unstaged_scroll.is_scrollable();
        viewport_height = pane
            .unstaged_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected unstaged uniform list size after draw")
            .item
            .height
            .into();
    });

    let visible_height: f32 =
        (unstaged_wrapper_bounds.bottom() - unstaged_header_bounds.bottom()).into();
    assert!(
        is_scrollable,
        "expected unstaged list to become scrollable after shrinking the unstaged section"
    );
    assert!(
        (viewport_height - visible_height).abs() <= 1.0,
        "expected unstaged uniform list viewport to match visible container height after resize (viewport_height={viewport_height}, visible_height={visible_height})"
    );
}

#[gpui::test]
fn split_unstaged_scroll_viewport_tracks_resized_section_height(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(49);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_unstaged_scroll_viewport",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(
                        (0..30)
                            .map(|ix| gitcomet_core::domain::FileStatus {
                                path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                                kind: gitcomet_core::domain::FileStatusKind::Modified,
                                conflict: None,
                            })
                            .collect(),
                    ),
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.set_change_tracking_view(ChangeTrackingView::SplitUntracked, cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.change_tracking_height = Some(px(240.0));
            cx.notify();
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

    let change_tracking_wrapper_bounds = cx
        .debug_bounds("status_change_tracking_wrapper")
        .expect("expected change-tracking section bounds after resizing");
    let split_unstaged_wrapper_bounds = cx
        .debug_bounds("status_split_unstaged_wrapper")
        .expect("expected split unstaged section bounds after resizing");
    let split_unstaged_header_bounds = cx
        .debug_bounds("split_unstaged_header")
        .expect("expected split unstaged header bounds after resizing");

    let mut is_scrollable = false;
    let mut viewport_height = 0.0f32;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        is_scrollable = pane.unstaged_scroll.is_scrollable();
        viewport_height = pane
            .unstaged_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected split unstaged uniform list size after draw")
            .item
            .height
            .into();
    });

    let visible_bottom = split_unstaged_wrapper_bounds
        .bottom()
        .min(change_tracking_wrapper_bounds.bottom());
    let visible_height: f32 = (visible_bottom - split_unstaged_header_bounds.bottom())
        .max(px(0.0))
        .into();
    assert!(
        is_scrollable,
        "expected split unstaged list to become scrollable after shrinking the outer change-tracking section"
    );
    assert!(
        (viewport_height - visible_height).abs() <= 1.0,
        "expected split unstaged uniform list viewport to match visible container height after resize (viewport_height={viewport_height}, visible_height={visible_height})"
    );
}

#[gpui::test]
fn split_unstaged_scroll_viewport_updates_after_outer_resize_shrink(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(50);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_unstaged_outer_resize",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(
                        (0..30)
                            .map(|ix| gitcomet_core::domain::FileStatus {
                                path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                                kind: gitcomet_core::domain::FileStatusKind::Modified,
                                conflict: None,
                            })
                            .collect(),
                    ),
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.set_change_tracking_view(ChangeTrackingView::SplitUntracked, cx);
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.change_tracking_height = Some(px(360.0));
            cx.notify();
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

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.change_tracking_height = Some(px(180.0));
            cx.notify();
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

    let change_tracking_wrapper_bounds = cx
        .debug_bounds("status_change_tracking_wrapper")
        .expect("expected change-tracking section bounds after shrinking the outer resize");
    let split_unstaged_wrapper_bounds = cx
        .debug_bounds("status_split_unstaged_wrapper")
        .expect("expected split unstaged section bounds after shrinking the outer resize");
    let split_unstaged_header_bounds = cx
        .debug_bounds("split_unstaged_header")
        .expect("expected split unstaged header bounds after shrinking the outer resize");

    let mut is_scrollable = false;
    let mut viewport_height = 0.0f32;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        is_scrollable = pane.unstaged_scroll.is_scrollable();
        viewport_height = pane
            .unstaged_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected split unstaged uniform list size after outer resize shrink")
            .item
            .height
            .into();
    });

    let visible_bottom = split_unstaged_wrapper_bounds
        .bottom()
        .min(change_tracking_wrapper_bounds.bottom());
    let visible_height: f32 = (visible_bottom - split_unstaged_header_bounds.bottom())
        .max(px(0.0))
        .into();

    assert!(
        split_unstaged_wrapper_bounds.bottom() <= change_tracking_wrapper_bounds.bottom() + px(1.0),
        "expected split unstaged section to stay within the visible change-tracking area after shrinking the outer resize (split_unstaged_bottom={:?}, change_tracking_bottom={:?})",
        split_unstaged_wrapper_bounds.bottom(),
        change_tracking_wrapper_bounds.bottom(),
    );
    assert!(
        is_scrollable,
        "expected split unstaged list to become scrollable after shrinking the outer resize"
    );
    assert!(
        (viewport_height - visible_height).abs() <= 1.0,
        "expected split unstaged uniform list viewport to match the visible clipped height after shrinking the outer resize (viewport_height={viewport_height}, visible_height={visible_height})"
    );
}

/// The section header's action labels collapse to initials once the details
/// pane is too narrow to hold the full wording. Verified by width because the
/// test text system has no glyph metrics — but it does give every glyph the
/// same advance, so a label with fewer characters is reliably narrower.
#[gpui::test]
fn stage_selected_label_shrinks_when_the_details_pane_is_narrow(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1600.0), px(900.0)));

    let repo_id = gitcomet_state::model::RepoId(63);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_header_shrink",
        std::process::id()
    ));
    let a = std::path::PathBuf::from("a.txt");
    let b = std::path::PathBuf::from("b.txt");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.open = gitcomet_state::model::Loadable::Ready(());
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(Vec::new()),
                    unstaged: std::sync::Arc::new(vec![
                        gitcomet_core::domain::FileStatus {
                            path: a.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: b.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ]),
                }
                .into(),
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.insert(
                    repo_id,
                    StatusMultiSelection {
                        unstaged: vec![a.clone(), b.clone()],
                        unstaged_anchor: Some(a.clone()),
                        ..Default::default()
                    },
                );
                cx.notify();
            });
        });
    });

    let set_details_width = |cx: &mut gpui::VisualTestContext, width: f32| {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.details_width = px(width);
                this.details_render_width = px(width);
                cx.notify();
            });
            window.refresh();
            let _ = window.draw(app);
        });
        // The header reads a width the probe measured while painting, so the
        // switch lands on the frame after the one that resized the pane.
        draw_and_drain_test_window(cx);
    };

    set_details_width(cx, 700.0);
    let wide = cx
        .debug_bounds("stage_selected_button")
        .expect("expected the Stage (n) button while files are selected")
        .size
        .width;

    set_details_width(cx, 300.0);
    let narrow = cx
        .debug_bounds("stage_selected_button")
        .expect("expected the Stage (n) button to survive the narrower pane")
        .size
        .width;

    assert!(
        narrow < wide,
        "expected `Stage (2)` to collapse to `S (2)` in a narrow pane (wide={wide:?}, narrow={narrow:?})"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Menu-opening chips across the workspace: click targets that stayed fixed
/// pills while Comfortable grew everything around them. The title bar is the
/// deliberate exception -- it shares its row with OS window controls that never
/// resize, so its own chips must not move.
#[gpui::test]
fn comfortable_chrome_chips_grow_with_the_density(cx: &mut gpui::TestAppContext) {
    use crate::appearance::{Appearance, UiDensity};

    fn heights(
        cx: &mut gpui::VisualTestContext,
        selectors: &[&'static str],
        density: UiDensity,
    ) -> Vec<gpui::Pixels> {
        selectors
            .iter()
            .map(|selector| {
                cx.debug_bounds(selector)
                    .unwrap_or_else(|| panic!("missing {selector} at {density:?} density"))
                    .size
                    .height
            })
            .collect()
    }

    let _guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(83);
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/comfortable-chips"));
            repo.open = Loadable::Ready(());
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        })
    });

    let selectors = [
        "sidebar_tab_branches",
        "sidebar_tab_files",
        "history_mode_header",
        "change_tracking_unstaged_header",
    ];
    // Everything in the title bar. `add_repo_menu` and the app menu are the two
    // `Button::unscaled()` call sites, and `add_repo_menu` is the one whose
    // height actually comes from that pin -- the others carry explicit sizes.
    let leak = |selector: String| -> &'static str { Box::leak(selector.into_boxed_str()) };
    let tab_selector = leak(format!("repo_tab_{}", repo_id.0));
    let plate_selector = leak(format!("repo_tab_hover_box_{}", repo_id.0));
    let mut fixed_selectors = vec![
        "repo_picker_toggle",
        "add_repo_menu",
        tab_selector,
        plate_selector,
    ];
    // macOS supplies its own application menu and caption buttons.
    if !cfg!(target_os = "macos") {
        fixed_selectors.extend(["app_menu", "titlebar_win_close"]);
    }

    let mut per_density: Vec<Vec<gpui::Pixels>> = Vec::new();
    let mut fixed_per_density: Vec<Vec<gpui::Pixels>> = Vec::new();

    for density in UiDensity::ALL {
        cx.update(|_, app| {
            app.set_global(Appearance {
                density,
                ..Appearance::default()
            });
            view.update(app, |this, cx| {
                this.notify_font_preferences_changed(cx);
            });
        });
        draw_and_drain_test_window(cx);
        per_density.push(heights(cx, &selectors, density));
        fixed_per_density.push(heights(cx, &fixed_selectors, density));

        // The plate behind the label and the tab around it are both fixed, so
        // the plate has to stay inside its tab at every density.
        let tab = cx
            .debug_bounds(tab_selector)
            .expect("expected repository tab bounds");
        let plate = cx
            .debug_bounds(plate_selector)
            .expect("expected repository tab label plate bounds");

        assert!(
            plate.top() > tab.top() && plate.bottom() < tab.bottom(),
            "{density:?}: the label plate {plate:?} must stay inside its tab {tab:?}"
        );
    }

    for (step, pair) in per_density.windows(2).enumerate() {
        for (ix, selector) in selectors.into_iter().enumerate() {
            assert!(
                pair[1][ix] > pair[0][ix],
                "{selector} must grow from {:?} to {:?}, stayed {:?}",
                UiDensity::ALL[step],
                UiDensity::ALL[step + 1],
                pair[1][ix]
            );
        }
    }

    for (step, pair) in fixed_per_density.windows(2).enumerate() {
        for (ix, selector) in fixed_selectors.iter().enumerate() {
            assert_eq!(
                pair[1][ix],
                pair[0][ix],
                "{selector} sits in the title bar and must not move from {:?} to {:?}",
                UiDensity::ALL[step],
                UiDensity::ALL[step + 1],
            );
        }
    }
}

#[gpui::test]
fn comfortable_stage_and_commit_targets_fit_rows_at_laptop_and_4k_sizes(
    cx: &mut gpui::TestAppContext,
) {
    use crate::appearance::{Appearance, UiDensity};
    use gitcomet_core::domain::{FileStatus, FileStatusKind, RepoStatus};
    let _guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(79);
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/comfortable-targets"));
            repo.open = Loadable::Ready(());
            repo.status = Loadable::Ready(
                RepoStatus {
                    staged: Arc::new(vec![FileStatus {
                        path: "staged.txt".into(),
                        kind: FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: Arc::new(vec![FileStatus {
                        path: "unstaged.txt".into(),
                        kind: FileStatusKind::Modified,
                        conflict: None,
                    }]),
                }
                .into(),
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        })
    });
    for (width, height, percent) in [
        (1280.0, 900.0, 80),
        (1440.0, 1000.0, 100),
        (3840.0, 2160.0, 200),
    ] {
        cx.simulate_resize(gpui::size(px(width), px(height)));
        for ui_font_size_px in [14, 24] {
            cx.update(|_, app| {
                app.set_global(Appearance {
                    density: UiDensity::Comfortable,
                    ui_font_size_px,
                    ..Appearance::default()
                });
                crate::ui_scale::set_default(app, percent);
                view.update(app, |this, cx| {
                    this.notify_font_preferences_changed(cx);
                });
            });
            draw_and_drain_test_window(cx);
            let minimum = px(32.0 * percent as f32 / 100.0);
            for selector in [
                "stage_all_button",
                "commit_button",
                "status_row_79_unstaged_0",
                "status_row_79_staged_0",
            ] {
                let bounds = cx
                    .debug_bounds(selector)
                    .unwrap_or_else(|| panic!("missing {selector} at {percent}%"));
                assert!(
                    bounds.size.height + px(0.5) >= minimum,
                    "{selector} target {:?} must reach {minimum:?} at UI font {ui_font_size_px}",
                    bounds.size
                );
            }
            let row = cx.debug_bounds("status_row_79_unstaged_0").unwrap();
            cx.simulate_mouse_move(row.center(), None, Modifiers::default());
            draw_and_drain_test_window(cx);
            let button = cx
                .debug_bounds("status_stage_button_79_unstaged_0")
                .unwrap();
            assert!(
                button.top() >= row.top() - px(0.5) && button.bottom() <= row.bottom() + px(0.5),
                "stage target must fit inside its row"
            );
        }
    }
}

#[gpui::test]
fn commit_details_signature_icon_follows_ui_scale(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (_view, cx) = commit_details_signature_fixture(
        cx,
        Some(test_signature(gitcomet_core::domain::SignatureStatus::Good)),
    );
    cx.update(|window, app| {
        crate::ui_scale::apply_to_window(window, 100);
        window.refresh();
        let _ = window.draw(app);
    });
    let normal = cx
        .debug_bounds("commit_details_signature_icon")
        .expect("signature icon")
        .size;
    cx.update(|window, app| {
        crate::ui_scale::apply_to_window(window, 200);
        window.refresh();
        let _ = window.draw(app);
    });
    let enlarged = cx
        .debug_bounds("commit_details_signature_icon")
        .expect("scaled signature icon")
        .size;
    assert_eq!(enlarged.width, normal.width * 2.0);
    assert_eq!(enlarged.height, normal.height * 2.0);
}

/// Unlike the signature icon, the close icon is sized in design px, so it
/// only zooms through the pane's UI scale rather than the window's rem size.
#[gpui::test]
fn commit_details_close_icon_follows_ui_scale(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) = commit_details_signature_fixture(cx, None);
    let normal = cx
        .debug_bounds("commit_details_close_icon")
        .expect("close icon")
        .size;

    set_ui_scale_percent_for_test(cx, &view, 200);
    draw_and_drain_test_window(cx);
    let zoomed = cx
        .debug_bounds("commit_details_close_icon")
        .expect("zoomed close icon")
        .size;
    assert_eq!(zoomed.width, normal.width * 2.0);
    assert_eq!(zoomed.height, normal.height * 2.0);
}
