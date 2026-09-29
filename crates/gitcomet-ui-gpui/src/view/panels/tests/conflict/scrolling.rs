//! Projection and scrolling: sync matrices, horizontal overflow, visible rows, clicks.

use super::*;

#[gpui::test]
fn conflict_resolver_input_lists_measure_later_long_rows_for_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    fn assert_horizontal_overflow(handle: &gpui::UniformListScrollHandle, label: &str) {
        let size = handle
            .0
            .borrow()
            .last_item_size
            .expect("expected rendered list item size");
        assert!(
            size.contents.width > size.item.width,
            "{label} should report horizontal overflow, got item={:?} contents={:?}",
            size.item,
            size.contents,
        );
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(163);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_hscroll_measure",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_resolver_hscroll_measure.txt");
    let abs_path = workdir.join(&file_rel);

    let long_base = format!("base {}", "X".repeat(320));
    let long_ours = format!("ours {}", "Y".repeat(320));
    let long_theirs = format!("theirs {}", "Z".repeat(320));
    let base_text = ["short", "context", long_base.as_str(), "tail"].join("\n");
    let ours_text = ["short", "context", long_ours.as_str(), "tail"].join("\n");
    let theirs_text = ["short", "context", long_theirs.as_str(), "tail"].join("\n");
    let current_text =
        format!("<<<<<<< ours\n{ours_text}\n=======\n{theirs_text}\n>>>>>>> theirs\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver hscroll fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver hscroll fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            set_test_conflict_file(
                &mut repo,
                file_rel.clone(),
                base_text.clone(),
                ours_text.clone(),
                theirs_text.clone(),
                current_text.clone(),
            );
            repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
                file_rel.clone(),
                gitcomet_core::domain::FileConflictKind::BothModified,
                ConflictPayload::Text(base_text.clone().into()),
                ConflictPayload::Text(ours_text.clone().into()),
                ConflictPayload::Text(theirs_text.clone().into()),
                &current_text,
            ));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolver hscroll fixture initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.two_way_split_visible_len() >= 4
                && pane.conflict_resolver.three_way_visible_len() >= 4
        },
        |pane| {
            format!(
                "path={:?} two_way_visible={} three_way_visible={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.two_way_split_visible_len(),
                pane.conflict_resolver.three_way_visible_len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                assert!(
                    pane.conflict_resolver.two_way_horizontal_measure_row(
                        crate::view::conflict_resolver::ConflictPickSide::Ours,
                    ) > 0,
                    "two-way ours column should not measure only the first short row",
                );
                assert!(
                    pane.conflict_resolver.two_way_horizontal_measure_row(
                        crate::view::conflict_resolver::ConflictPickSide::Theirs,
                    ) > 0,
                    "two-way theirs column should not measure only the first short row",
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        let _ = window.draw(app);
        let pane = view.read(app).main_pane.read(app);
        assert_horizontal_overflow(&pane.conflict_resolver_diff_scroll, "two-way ours list");
        assert_horizontal_overflow(&pane.conflict_preview_theirs_scroll, "two-way theirs list");
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                assert!(
                    pane.conflict_resolver
                        .three_way_horizontal_measure_row(ThreeWayColumn::Base)
                        > 0,
                    "three-way base column should not measure only the first short row",
                );
                assert!(
                    pane.conflict_resolver
                        .three_way_horizontal_measure_row(ThreeWayColumn::Ours)
                        > 0,
                    "three-way ours column should not measure only the first short row",
                );
                assert!(
                    pane.conflict_resolver
                        .three_way_horizontal_measure_row(ThreeWayColumn::Theirs)
                        > 0,
                    "three-way theirs column should not measure only the first short row",
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        let _ = window.draw(app);
        let pane = view.read(app).main_pane.read(app);
        assert_horizontal_overflow(&pane.conflict_resolver_diff_scroll, "three-way base list");
        assert_horizontal_overflow(&pane.conflict_preview_ours_scroll, "three-way ours list");
        assert_horizontal_overflow(
            &pane.conflict_preview_theirs_scroll,
            "three-way theirs list",
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup resolver hscroll fixture");
}

#[gpui::test]
fn conflict_resolver_three_way_remote_horizontal_overflow_with_divergent_context(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    fn assert_horizontal_overflow(handle: &gpui::UniformListScrollHandle, renderer: &str) {
        let size = handle
            .0
            .borrow()
            .last_item_size
            .expect("expected rendered Remote list item size");
        assert!(
            size.contents.width > size.item.width,
            "Remote should report horizontal overflow with {renderer} rows, got item={:?} contents={:?}",
            size.item,
            size.contents,
        );
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(174);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_remote_divergent_hscroll",
        std::process::id()
    ));
    let file_rel =
        std::path::PathBuf::from("fixtures/conflict_resolver_remote_divergent_hscroll.txt");
    let abs_path = workdir.join(&file_rel);

    // The merged file carries Local's longer pre-conflict context, while the
    // Remote stage reaches the conflict much earlier. This makes Remote's side
    // line index differ from the shared aligned row used by the three-way list.
    let base_prefix = ["shared", "base context one", "base context two"].join("\n");
    let ours_prefix = [
        "shared",
        "local context one",
        "local context two",
        "local context three",
        "local context four",
        "local context five",
        "local context six",
    ]
    .join("\n");
    let theirs_prefix = "shared";
    let base_conflict = "base value";
    let ours_conflict = "local value";
    let long_remote = format!("remote value {}", "R".repeat(420));
    let base_text = format!("{base_prefix}\n{base_conflict}\ntail\n");
    let ours_text = format!("{ours_prefix}\n{ours_conflict}\ntail\n");
    let theirs_text = format!("{theirs_prefix}\n{long_remote}\ntail\n");
    let current_text = format!(
        "{ours_prefix}\n<<<<<<< ours\n{ours_conflict}\n||||||| base\n{base_conflict}\n=======\n{long_remote}\n>>>>>>> theirs\ntail\n"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create divergent-context hscroll fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write divergent-context hscroll fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            set_test_conflict_file(
                &mut repo,
                file_rel.clone(),
                base_text.clone(),
                ours_text.clone(),
                theirs_text.clone(),
                current_text.clone(),
            );
            repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
                file_rel.clone(),
                gitcomet_core::domain::FileConflictKind::BothModified,
                ConflictPayload::Text(base_text.clone().into()),
                ConflictPayload::Text(ours_text.clone().into()),
                ConflictPayload::Text(theirs_text.clone().into()),
                &current_text,
            ));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "divergent-context Remote hscroll fixture initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.three_way_visible_len() >= 3
        },
        |pane| {
            format!(
                "path={:?} three_way_visible={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.three_way_visible_len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                let remote_line = theirs_text
                    .lines()
                    .position(|line| line == long_remote)
                    .expect("long Remote line should exist in the Remote stage");
                let expected_row = pane
                    .conflict_resolver
                    .three_way_row_for_side_line(ThreeWayColumn::Theirs, remote_line);
                assert_eq!(
                    pane.conflict_resolver
                        .three_way_horizontal_measure_row(ThreeWayColumn::Theirs),
                    expected_row,
                    "Remote width measurement must translate its stage line to the aligned row",
                );
            });
        });
    });

    for canvas_rows_enabled in [true, false] {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.conflict_canvas_rows_enabled = canvas_rows_enabled;
                    cx.notify();
                });
            });
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
            let pane = view.read(app).main_pane.read(app);
            assert_horizontal_overflow(
                &pane.conflict_preview_theirs_scroll,
                if canvas_rows_enabled { "canvas" } else { "div" },
            );
        });
    }

    std::fs::remove_dir_all(&workdir).expect("cleanup divergent-context Remote hscroll fixture");
}

#[gpui::test]
fn conflict_resolver_output_gutter_tracks_output_scroll_when_diff_sync_is_disabled(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(163);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_output_gutter_scroll_sync",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_output_gutter_scroll_sync.txt");
    let abs_path = workdir.join(&file_rel);
    let base_text = build_conflict_scroll_matrix_text("base", 'B');
    let ours_text = build_conflict_scroll_matrix_text("ours", 'O');
    let theirs_text = build_conflict_scroll_matrix_text("theirs", 'T');
    let current_text = build_conflict_scroll_matrix_current_text(&ours_text, &theirs_text);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver output gutter fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver output gutter fixture");

    seed_conflict_scroll_matrix_state(
        cx,
        &view,
        repo_id,
        &workdir,
        &file_rel,
        &base_text,
        &ours_text,
        &theirs_text,
        &current_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "resolver output gutter fixture initialized",
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolved_preview_line_count >= 1
        },
        |pane| {
            format!(
                "path={:?} resolved_lines={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolver output gutter overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).width
                    > px(120.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).height
                    > px(120.0)
        },
        |pane| {
            format!(
                "view_mode={:?} output_offset={:?} output_max={:?} gutter_offset={:?} gutter_max={:?}",
                pane.conflict_resolver.view_mode,
                scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll),
                scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll),
                uniform_list_offset(&pane.conflict_resolved_preview_gutter_scroll),
                uniform_list_max_offset(&pane.conflict_resolved_preview_gutter_scroll),
            )
        },
    );

    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::None);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                reset_conflict_scroll_matrix_offsets(pane);
                set_scroll_handle_offset(
                    &pane.conflict_resolved_output_editor_scroll,
                    point(px(-72.0), px(-48.0)),
                );
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll),
            point(px(-72.0), px(-48.0)),
            "resolved output should keep its own scroll offset when diff sync is disabled",
        );
        assert_eq!(
            uniform_list_offset(&pane.conflict_resolved_preview_gutter_scroll),
            point(px(0.0), px(-48.0)),
            "resolved output gutter should follow only the editor's vertical scroll",
        );
        assert_eq!(
            uniform_list_offset(&pane.conflict_resolver_diff_scroll),
            point(px(0.0), px(0.0)),
            "base pane should remain independent when diff sync is disabled",
        );
        assert_eq!(
            uniform_list_offset(&pane.conflict_preview_ours_scroll),
            point(px(0.0), px(0.0)),
            "ours pane should remain independent when diff sync is disabled",
        );
        assert_eq!(
            uniform_list_offset(&pane.conflict_preview_theirs_scroll),
            point(px(0.0), px(0.0)),
            "theirs pane should remain independent when diff sync is disabled",
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let editor_max =
                    scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).height;
                set_scroll_handle_offset(
                    &pane.conflict_resolved_output_editor_scroll,
                    point(px(0.0), -editor_max),
                );
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let editor_offset = scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll).y;
        let gutter_offset = uniform_list_offset(&pane.conflict_resolved_preview_gutter_scroll).y;
        assert_eq!(
            gutter_offset, editor_offset,
            "line-number gutter should stop at the editor's bottom boundary; editor_max={:?} gutter_max={:?}",
            scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll),
            uniform_list_max_offset(&pane.conflict_resolved_preview_gutter_scroll),
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup resolver output gutter fixture");
}

#[gpui::test]
fn conflict_resolver_three_way_scroll_sync_matrix_covers_all_modes_and_axes(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(164);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_three_way_scroll_sync_matrix",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_scroll_sync_matrix.txt");
    let abs_path = workdir.join(&file_rel);
    let base_text = build_conflict_scroll_matrix_text("base", 'B');
    let ours_text = build_conflict_scroll_matrix_text("ours", 'O');
    let theirs_text = build_conflict_scroll_matrix_text("theirs", 'T');
    let current_text = build_conflict_scroll_matrix_current_text(&ours_text, &theirs_text);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver three-way matrix fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver three-way matrix fixture");

    seed_conflict_scroll_matrix_state(
        cx,
        &view,
        repo_id,
        &workdir,
        &file_rel,
        &base_text,
        &ours_text,
        &theirs_text,
        &current_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "resolver three-way matrix fixture initialized",
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.three_way_visible_len() >= 4
                && pane.conflict_resolved_preview_line_count >= 1
        },
        |pane| {
            format!(
                "path={:?} three_way_visible={} resolved_lines={} base_max={:?} ours_max={:?} theirs_max={:?} output_max={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.three_way_visible_len(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolver_diff_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
                pane.conflict_preview_ours_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
                pane.conflict_preview_theirs_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
                pane.conflict_resolved_output_editor_scroll.max_offset(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolver three-way matrix overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).width > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_ours_scroll).width > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_theirs_scroll).width > px(120.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).width
                    > px(120.0)
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).height > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_ours_scroll).height > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_theirs_scroll).height > px(120.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).height
                    > px(120.0)
        },
        |pane| {
            format!(
                "view_mode={:?} base_offset={:?} ours_offset={:?} theirs_offset={:?} output_offset={:?} base_max={:?} ours_max={:?} theirs_max={:?} output_max={:?}",
                pane.conflict_resolver.view_mode,
                uniform_list_offset(&pane.conflict_resolver_diff_scroll),
                uniform_list_offset(&pane.conflict_preview_ours_scroll),
                uniform_list_offset(&pane.conflict_preview_theirs_scroll),
                scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll),
                uniform_list_max_offset(&pane.conflict_resolver_diff_scroll),
                uniform_list_max_offset(&pane.conflict_preview_ours_scroll),
                uniform_list_max_offset(&pane.conflict_preview_theirs_scroll),
                scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll),
            )
        },
    );

    let reset_offsets =
        |cx: &mut gpui::VisualTestContext,
         view: &gpui::Entity<super::super::super::GitCometView>| {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        reset_conflict_scroll_matrix_offsets(pane);
                        cx.notify();
                    });
                });
            });
            draw_and_drain_test_window(cx);
        };

    for mode in ALL_DIFF_SCROLL_SYNC_MODES {
        set_diff_scroll_sync_for_test(cx, &view, mode);

        for axis in ScrollSyncAxis::ALL {
            let output_offset = axis.offset(px(72.0));
            reset_offsets(cx, &view);
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        set_scroll_handle_offset(
                            &pane.conflict_resolved_output_editor_scroll,
                            output_offset,
                        );
                        cx.notify();
                    });
                });
            });
            draw_and_drain_test_window(cx);

            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                // The resolved output is a different document from the
                // aligned columns and is only coupled to them horizontally,
                // where the correspondence is exact (same pixel column).
                // Vertically it scrolls on its own; see
                // `sync_conflict_preview_axis`.
                let output_coupled =
                    axis.includes(mode) && matches!(axis, ScrollSyncAxis::Horizontal);
                let expected = if output_coupled {
                    axis.component(output_offset)
                } else {
                    px(0.0)
                };
                assert_eq!(
                    axis.component(scroll_handle_offset(
                        &pane.conflict_resolved_output_editor_scroll,
                    )),
                    axis.component(output_offset),
                    "resolved output should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_resolver_diff_scroll)),
                    expected,
                    "three-way base pane should {} {} scrolling from resolved output in {:?} mode",
                    if output_coupled { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_preview_ours_scroll)),
                    expected,
                    "three-way ours pane should {} {} scrolling from resolved output in {:?} mode",
                    if output_coupled { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_preview_theirs_scroll)),
                    expected,
                    "three-way theirs pane should {} {} scrolling from resolved output in {:?} mode",
                    if output_coupled { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
            });

            let base_offset = axis.offset(px(96.0));
            reset_offsets(cx, &view);
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        set_uniform_list_offset(&pane.conflict_resolver_diff_scroll, base_offset);
                        cx.notify();
                    });
                });
            });
            draw_and_drain_test_window(cx);

            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let columns_expected = if axis.includes(mode) {
                    axis.component(base_offset)
                } else {
                    px(0.0)
                };
                let output_expected =
                    if axis.includes(mode) && matches!(axis, ScrollSyncAxis::Horizontal) {
                        axis.component(base_offset)
                    } else {
                        px(0.0)
                    };
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_resolver_diff_scroll)),
                    axis.component(base_offset),
                    "three-way base pane should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_preview_ours_scroll)),
                    columns_expected,
                    "three-way ours pane should {} {} scrolling from the base pane in {:?} mode",
                    if axis.includes(mode) {
                        "sync"
                    } else {
                        "not sync"
                    },
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_preview_theirs_scroll)),
                    columns_expected,
                    "three-way theirs pane should {} {} scrolling from the base pane in {:?} mode",
                    if axis.includes(mode) {
                        "sync"
                    } else {
                        "not sync"
                    },
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(scroll_handle_offset(
                        &pane.conflict_resolved_output_editor_scroll,
                    )),
                    output_expected,
                    "resolved output should {} {} scrolling from the base pane in {:?} mode",
                    if axis.includes(mode) {
                        "sync"
                    } else {
                        "not sync"
                    },
                    axis.label(),
                    mode,
                );
            });
        }
    }

    std::fs::remove_dir_all(&workdir).expect("cleanup resolver three-way matrix fixture");
}

#[gpui::test]
fn conflict_resolver_two_way_scroll_sync_matrix_covers_all_modes_and_axes(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(165);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_two_way_scroll_sync_matrix",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_scroll_sync_two_way.txt");
    let abs_path = workdir.join(&file_rel);
    let base_text = build_conflict_scroll_matrix_text("base", 'B');
    let ours_text = build_conflict_scroll_matrix_text("ours", 'O');
    let theirs_text = build_conflict_scroll_matrix_text("theirs", 'T');
    let current_text = build_conflict_scroll_matrix_current_text(&ours_text, &theirs_text);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver two-way matrix fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver two-way matrix fixture");

    seed_conflict_scroll_matrix_state(
        cx,
        &view,
        repo_id,
        &workdir,
        &file_rel,
        &base_text,
        &ours_text,
        &theirs_text,
        &current_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "resolver two-way matrix fixture initialized",
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.two_way_split_visible_len() >= 4
                && pane.conflict_resolved_preview_line_count >= 1
        },
        |pane| {
            format!(
                "path={:?} two_way_visible={} resolved_lines={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.two_way_split_visible_len(),
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolver two-way matrix overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.view_mode == ConflictResolverViewMode::TwoWayDiff
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).width > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_theirs_scroll).width > px(120.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).width
                    > px(120.0)
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).height > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_theirs_scroll).height > px(120.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).height
                    > px(120.0)
        },
        |pane| {
            format!(
                "view_mode={:?} left_offset={:?} right_offset={:?} output_offset={:?} left_max={:?} right_max={:?} output_max={:?}",
                pane.conflict_resolver.view_mode,
                uniform_list_offset(&pane.conflict_resolver_diff_scroll),
                uniform_list_offset(&pane.conflict_preview_theirs_scroll),
                scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll),
                uniform_list_max_offset(&pane.conflict_resolver_diff_scroll),
                uniform_list_max_offset(&pane.conflict_preview_theirs_scroll),
                scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll),
            )
        },
    );

    let reset_offsets =
        |cx: &mut gpui::VisualTestContext,
         view: &gpui::Entity<super::super::super::GitCometView>| {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        reset_conflict_scroll_matrix_offsets(pane);
                        cx.notify();
                    });
                });
            });
            draw_and_drain_test_window(cx);
        };

    // section 30 aligned two-way full mode (this fixture has a base, so ours/theirs
    // align onto the shared whole-file row space). The left/right columns
    // always couple as a pair; the resolved output couples with them only when
    // the merge-tool output-scroll-sync setting is on. Both are still gated by
    // the diff sync mode/axis.
    for output_sync_on in [true, false] {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    // Set the field directly: this test exercises the sync-group
                    // logic, not persistence, and the `_and_persist` variant
                    // re-enters the root view we're already updating.
                    pane.mergetool_output_scroll_sync = output_sync_on;
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);

        for mode in ALL_DIFF_SCROLL_SYNC_MODES {
            set_diff_scroll_sync_for_test(cx, &view, mode);

            for axis in ScrollSyncAxis::ALL {
                let coupled = axis.includes(mode);
                // The editable resolved output has a content-width horizontal
                // range, so it participates in both axes when output sync is on.
                // The resolved output is a different document from the
                // aligned columns and is only coupled to them horizontally,
                // where the correspondence is exact (same pixel column).
                // Vertically it scrolls on its own; see
                // `sync_conflict_preview_axis`.
                let output_coupled =
                    coupled && output_sync_on && matches!(axis, ScrollSyncAxis::Horizontal);

                let output_offset = axis.offset(px(72.0));
                reset_offsets(cx, &view);
                cx.update(|_window, app| {
                    view.update(app, |this, cx| {
                        this.main_pane.update(cx, |pane, cx| {
                            set_scroll_handle_offset(
                                &pane.conflict_resolved_output_editor_scroll,
                                output_offset,
                            );
                            cx.notify();
                        });
                    });
                });
                draw_and_drain_test_window(cx);

                cx.update(|_window, app| {
                    let pane = view.read(app).main_pane.read(app);
                    let expected = if output_coupled {
                        axis.component(output_offset)
                    } else {
                        px(0.0)
                    };
                    assert_eq!(
                        axis.component(scroll_handle_offset(
                        &pane.conflict_resolved_output_editor_scroll,
                    )),
                        axis.component(output_offset),
                        "two-way resolved output should keep its {} offset in {:?} mode (sync={output_sync_on})",
                        axis.label(),
                        mode,
                    );
                    assert_eq!(
                        axis.component(uniform_list_offset(&pane.conflict_resolver_diff_scroll)),
                        expected,
                        "two-way left pane should {} {} scrolling from resolved output in {:?} mode (sync={output_sync_on})",
                        if output_coupled { "sync" } else { "not sync" },
                        axis.label(),
                        mode,
                    );
                    assert_eq!(
                        axis.component(uniform_list_offset(&pane.conflict_preview_theirs_scroll)),
                        expected,
                        "two-way right pane should {} {} scrolling from resolved output in {:?} mode (sync={output_sync_on})",
                        if output_coupled { "sync" } else { "not sync" },
                        axis.label(),
                        mode,
                    );
                });

                let right_offset = axis.offset(px(96.0));
                reset_offsets(cx, &view);
                cx.update(|_window, app| {
                    view.update(app, |this, cx| {
                        this.main_pane.update(cx, |pane, cx| {
                            set_uniform_list_offset(
                                &pane.conflict_preview_theirs_scroll,
                                right_offset,
                            );
                            cx.notify();
                        });
                    });
                });
                draw_and_drain_test_window(cx);

                cx.update(|_window, app| {
                    let pane = view.read(app).main_pane.read(app);
                    let pair_expected = if coupled {
                        axis.component(right_offset)
                    } else {
                        px(0.0)
                    };
                    let output_expected = if output_coupled {
                        axis.component(right_offset)
                    } else {
                        px(0.0)
                    };
                    assert_eq!(
                        axis.component(uniform_list_offset(&pane.conflict_preview_theirs_scroll)),
                        axis.component(right_offset),
                        "two-way right pane should keep its {} offset in {:?} mode (sync={output_sync_on})",
                        axis.label(),
                        mode,
                    );
                    assert_eq!(
                        axis.component(uniform_list_offset(&pane.conflict_resolver_diff_scroll)),
                        pair_expected,
                        "two-way left pane should {} {} scrolling from the right pane in {:?} mode (sync={output_sync_on})",
                        if coupled { "sync" } else { "not sync" },
                        axis.label(),
                        mode,
                    );
                    assert_eq!(
                        axis.component(scroll_handle_offset(
                        &pane.conflict_resolved_output_editor_scroll,
                    )),
                        output_expected,
                        "two-way resolved output should {} {} scrolling from the right pane in {:?} mode (sync={output_sync_on})",
                        if output_coupled { "sync" } else { "not sync" },
                        axis.label(),
                        mode,
                    );
                });
            }
        }
    }

    // Exercise the real wheel path at EOF. The two-way source lists include
    // comfort overscroll while resolved output has a shorter maximum. Reaching
    // the source maximum must remain stable across subsequent render/sync
    // passes instead of letting the clamped output pull the sources backward.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.mergetool_output_scroll_sync = true;
                reset_conflict_scroll_matrix_offsets(pane);
                cx.notify();
            });
        });
    });
    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::Vertical);
    draw_and_drain_test_window(cx);

    let (right_max, right_bounds) = cx.update(|window, app| {
        let _ = window.draw(app);
        let pane = view.read(app).main_pane.read(app);
        let handle = pane.conflict_preview_theirs_scroll.0.borrow();
        (
            handle.base_handle.max_offset().y.max(px(0.0)),
            handle.base_handle.bounds(),
        )
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_uniform_list_offset(
                    &pane.conflict_preview_theirs_scroll,
                    point(px(0.0), -(right_max - px(40.0)).max(px(0.0))),
                );
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.simulate_event(gpui::ScrollWheelEvent {
        position: right_bounds.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-160.0))),
        ..Default::default()
    });
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    let after_wheel = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            uniform_list_offset(&pane.conflict_resolver_diff_scroll).y,
            uniform_list_offset(&pane.conflict_preview_theirs_scroll).y,
            scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll).y,
        )
    });
    draw_and_drain_test_window(cx);
    let after_idle_render = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            uniform_list_offset(&pane.conflict_resolver_diff_scroll).y,
            uniform_list_offset(&pane.conflict_preview_theirs_scroll).y,
            scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll).y,
        )
    });
    assert_eq!(
        after_wheel, after_idle_render,
        "two-way EOF offsets must remain stable after output clamps; right_max={right_max:?}",
    );
    assert_eq!(
        after_wheel.1, -right_max,
        "two-way right pane should retain its comfort-overscroll EOF position",
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup resolver two-way matrix fixture");
}

#[gpui::test]
fn conflict_compare_split_renderer_uses_streamed_visible_rows_for_large_conflicts(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(176);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "conflict_compare_split_streamed",
        "fixtures/conflict_compare_split_streamed.html",
        crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1,
    );
    fixture.write();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = app_state_with_repo(
                conflict_compare_repo_state(
                    repo_id,
                    &fixture.workdir,
                    &fixture.file_rel,
                    &fixture.base_text,
                    &fixture.ours_text,
                    &fixture.theirs_text,
                    &fixture.current_text,
                ),
                repo_id,
            );
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "streamed compare split bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.rendering_mode()
                    == crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} rendering_mode={:?} split_row_index={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.rendering_mode(),
                pane.conflict_resolver.split_row_index().is_some(),
            )
        },
    );

    crate::view::test_support::inspect_render(cx, |window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.conflict_diff_segments_cache_split.clear();
                pane.conflict_diff_query_segments_cache_split.clear();

                let visible_ix = pane.conflict_resolver.two_way_split_visible_len() / 2;
                let crate::view::conflict_resolver::TwoWaySplitVisibleRow {
                    source_row_ix: _source_ix,
                    row,
                    conflict_ix: _conflict_ix,
                } = pane
                    .conflict_resolver
                    .two_way_split_visible_row(visible_ix)
                    .expect("deep streamed compare row should resolve through the split provider");

                assert!(
                    pane.conflict_diff_segments_cache_split.is_empty(),
                    "compare split style cache should start empty for this focused render",
                );

                let elements = MainPaneView::render_conflict_compare_diff_rows(
                    pane,
                    visible_ix..visible_ix + 1,
                    window,
                    cx,
                );
                assert_eq!(elements.len(), 1);

                assert!(
                    pane.conflict_diff_segments_cache_split.is_empty(),
                    "large streamed compare render should skip per-row style caching and render plain text",
                );
                assert!(
                    row.old.is_some() || row.new.is_some(),
                    "deep streamed compare row should still expose real source text",
                );
            });
        });
    });

    fixture.cleanup();
}

#[gpui::test]
fn conflict_compare_split_renderer_uses_visible_projection_when_rows_are_hidden(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(177);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_compare_split_hidden",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/conflict_compare_split_hidden.rs");
    let abs_path = workdir.join(&file_rel);

    let base_text = [
        "fn main() {",
        "    let first = 0;",
        "    let between = 1;",
        "    let second = 2;",
        "}",
    ]
    .join("\n");
    let ours_text = [
        "fn main() {",
        "    let first = 10;",
        "    let between = 1;",
        "    let second = 20;",
        "}",
    ]
    .join("\n");
    let theirs_text = [
        "fn main() {",
        "    let first = 11;",
        "    let between = 1;",
        "    let second = 21;",
        "}",
    ]
    .join("\n");
    let current_text = [
        "fn main() {",
        "<<<<<<< ours",
        "    let first = 10;",
        "=======",
        "    let first = 11;",
        ">>>>>>> theirs",
        "    let between = 1;",
        "<<<<<<< ours",
        "    let second = 20;",
        "=======",
        "    let second = 21;",
        ">>>>>>> theirs",
        "}",
    ]
    .join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = app_state_with_repo(
                conflict_compare_repo_state(
                    repo_id,
                    &workdir,
                    &file_rel,
                    &base_text,
                    &ours_text,
                    &theirs_text,
                    &current_text,
                ),
                repo_id,
            );
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "split compare streamed bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.rendering_mode()
                    == crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} rendering_mode={:?} split_row_index={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.rendering_mode(),
                pane.conflict_resolver.split_row_index().is_some(),
            )
        },
    );

    crate::view::test_support::inspect_render(cx, |window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let first_block = pane
                    .conflict_resolver
                    .marker_segments
                    .iter_mut()
                    .find_map(|segment| match segment {
                        crate::view::conflict_resolver::ConflictSegment::Block(block) => {
                            Some(block)
                        }
                        crate::view::conflict_resolver::ConflictSegment::Text(_) => None,
                    })
                    .expect("fixture should contain a first conflict block");
                first_block.resolved = true;
                pane.conflict_resolver.hide_resolved = true;
                pane.conflict_resolver.rebuild_three_way_visible_state();
                pane.conflict_resolver.rebuild_two_way_visible_state();
                pane.diff_view = DiffViewMode::Split;
                pane.conflict_diff_segments_cache_split.clear();
                pane.conflict_diff_query_segments_cache_split.clear();

                let (visible_ix, source_ix, row) =
                    (0..pane.conflict_resolver.two_way_split_visible_len()).find_map(
                        |visible_ix| {
                        let crate::view::conflict_resolver::TwoWaySplitVisibleRow {
                            source_row_ix: source_ix,
                            row,
                            conflict_ix: _conflict_ix,
                        } = pane
                            .conflict_resolver
                            .two_way_split_visible_row(visible_ix)?;
                        (source_ix != visible_ix && (row.old.is_some() || row.new.is_some()))
                            .then_some((visible_ix, source_ix, row))
                    },
                    )
                    .expect("hide-resolved compare view should remap at least one split row");

                let elements = MainPaneView::render_conflict_compare_diff_rows(
                    pane,
                    visible_ix..visible_ix + 1,
                    window,
                    cx,
                );
                assert_eq!(elements.len(), 1);

                if let Some(expected_text) = row.old.as_deref() {
                    if let Some(styled) = pane.conflict_diff_segments_cache_split.get(&(
                        source_ix,
                        crate::view::conflict_resolver::ConflictPickSide::Ours,
                    )) {
                        assert_eq!(styled.text.as_ref(), expected_text);
                    }
                    assert!(
                        !pane.conflict_diff_segments_cache_split.contains_key(&(
                            visible_ix,
                            crate::view::conflict_resolver::ConflictPickSide::Ours,
                        )),
                        "compare split render should cache ours styling by source row index, not visible row index",
                    );
                }
                if let Some(expected_text) = row.new.as_deref() {
                    if let Some(styled) = pane.conflict_diff_segments_cache_split.get(&(
                        source_ix,
                        crate::view::conflict_resolver::ConflictPickSide::Theirs,
                    )) {
                        assert_eq!(styled.text.as_ref(), expected_text);
                    }
                    assert!(
                        !pane.conflict_diff_segments_cache_split.contains_key(&(
                            visible_ix,
                            crate::view::conflict_resolver::ConflictPickSide::Theirs,
                        )),
                        "compare split render should cache theirs styling by source row index, not visible row index",
                    );
                }
            });
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup fixture");
}

/// Verifies that giant two-way split mode uses the paged provider to generate
/// rows on demand instead of building an eager `diff_rows` array. Deep rows
/// should be accessible without materializing rows for earlier indices.
#[gpui::test]
fn giant_two_way_paged_provider_generates_rows_on_demand(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(170);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "giant_two_way_paged_on_demand",
        "fixtures/paged_on_demand.html",
        20_001,
    );
    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "giant two-way paged bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} split_row_index={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.split_row_index().is_some(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                let total = assert_streamed_whole_file_two_way_state(pane, fixture.line_count);

                // Generate a deep row on demand without touching earlier rows.
                let deep_ix = total / 2;
                let crate::view::conflict_resolver::TwoWaySplitVisibleRow {
                    source_row_ix: source_ix,
                    row,
                    conflict_ix: _conflict_ix,
                } = pane
                    .conflict_resolver
                    .two_way_split_visible_row(deep_ix)
                    .expect("deep visible row should be accessible on demand");
                assert!(
                    row.old.is_some() || row.new.is_some(),
                    "on-demand row at visible index {deep_ix} (source {source_ix}) should have text",
                );

                // Verify the first and last visible rows are accessible too.
                assert!(
                    pane.conflict_resolver.two_way_split_visible_row(0).is_some(),
                    "first visible row should be accessible",
                );
                assert!(
                    pane.conflict_resolver
                        .two_way_split_visible_row(total - 1)
                        .is_some(),
                    "last visible row should be accessible",
                );

                // Out-of-bounds returns None.
                assert!(
                    pane.conflict_resolver
                        .two_way_split_visible_row(total)
                        .is_none(),
                    "out-of-bounds visible row should return None",
                );
            });
        });
    });

    fixture.cleanup();
}

/// Once a scroll gesture has settled, further idle frames must not move any
/// pane. A jump here is the user-visible "the resolved output jumps after I
/// scroll" bug: some pane is treated as freshly changed on a frame with no
/// input, wins the master election, and drags the others onto it.
#[gpui::test]
fn conflict_resolver_scroll_positions_hold_across_idle_frames(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(191);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_idle_frame_scroll_hold",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_idle_frame_scroll_hold.txt");
    let abs_path = workdir.join(&file_rel);
    let base_text = build_conflict_scroll_matrix_text("base", 'B');
    let ours_text = build_conflict_scroll_matrix_text("ours", 'O');
    let theirs_text = build_conflict_scroll_matrix_text("theirs", 'T');
    let current_text = build_conflict_scroll_matrix_current_text(&ours_text, &theirs_text);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver idle-frame fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver idle-frame fixture");

    seed_conflict_scroll_matrix_state(
        cx,
        &view,
        repo_id,
        &workdir,
        &file_rel,
        &base_text,
        &ours_text,
        &theirs_text,
        &current_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "resolver idle-frame fixture initialized",
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.three_way_visible_len() >= 4
                && pane.conflict_resolved_preview_line_count >= 1
        },
        |pane| {
            format!(
                "path={:?} three_way_visible={} resolved_lines={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.three_way_visible_len(),
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolver idle-frame vertical overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).height > px(400.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).height
                    > px(400.0)
        },
        |pane| {
            format!(
                "view_mode={:?} base_max={:?} output_max={:?}",
                pane.conflict_resolver.view_mode,
                uniform_list_max_offset(&pane.conflict_resolver_diff_scroll),
                scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll),
            )
        },
    );

    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::Both);

    // A wheel over the resolved output: the editor handle moves natively and
    // the pane records the output as this gesture's master.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                reset_conflict_scroll_matrix_offsets(pane);
                set_scroll_handle_offset(
                    &pane.conflict_resolved_output_editor_scroll,
                    point(px(0.0), px(-240.0)),
                );
                pane.record_conflict_vertical_wheel_master(3);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    let settled = read_conflict_scroll_snapshot(cx, &view);
    for frame in 1..=3 {
        draw_and_drain_test_window(cx);
        let idle = read_conflict_scroll_snapshot(cx, &view);
        assert_eq!(
            idle, settled,
            "idle frame {frame} moved the resolver panes after an output wheel",
        );
    }

    // A minimap click/drag: every source column gets a deferred
    // `scroll_to_item_strict`, which lands during prepaint, after this frame's
    // synchronizer already ran.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                reset_conflict_scroll_matrix_offsets(pane);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_scroll_all_columns(90, gpui::ScrollStrategy::Center);
                cx.notify();
            });
        });
    });
    // Two frames: one for prepaint to consume the deferred scroll, one for the
    // synchronizer to observe it and remap the output.
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    let settled = read_conflict_scroll_snapshot(cx, &view);
    assert!(
        settled.base < px(0.0),
        "the minimap jump should have scrolled the columns, got {settled:?}",
    );
    for frame in 1..=3 {
        draw_and_drain_test_window(cx);
        let idle = read_conflict_scroll_snapshot(cx, &view);
        assert_eq!(
            idle, settled,
            "idle frame {frame} moved the resolver panes after a minimap jump",
        );
    }

    // Scrolling a source column: the columns drive, and the output is remapped
    // through the conflict anchors rather than copied 1:1.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                reset_conflict_scroll_matrix_offsets(pane);
                set_uniform_list_offset(
                    &pane.conflict_resolver_diff_scroll,
                    point(px(0.0), px(-320.0)),
                );
                pane.record_conflict_vertical_wheel_master(0);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    let settled = read_conflict_scroll_snapshot(cx, &view);
    for frame in 1..=3 {
        draw_and_drain_test_window(cx);
        let idle = read_conflict_scroll_snapshot(cx, &view);
        assert_eq!(
            idle, settled,
            "idle frame {frame} moved the resolver panes after a column wheel",
        );
    }

    // The bottom boundary: the source columns carry comfort overscroll rows the
    // resolved output does not, so the output clamps while the columns keep
    // going. The clamped follower must not then be promoted to master.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let deep = uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).height;
                set_uniform_list_offset(&pane.conflict_resolver_diff_scroll, point(px(0.0), -deep));
                pane.record_conflict_vertical_wheel_master(0);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    let settled = read_conflict_scroll_snapshot(cx, &view);
    for frame in 1..=3 {
        draw_and_drain_test_window(cx);
        let idle = read_conflict_scroll_snapshot(cx, &view);
        assert_eq!(
            idle, settled,
            "idle frame {frame} moved the resolver panes at the bottom clamp boundary",
        );
    }

    // Collapsed context folds the line-number gutter's row space but not the
    // editor's text, so the two are no longer the same number of rows.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                // Pre-match the persisted default so the toggle does not
                // schedule a settings persist (which re-enters the view).
                pane.mergetool_collapse_unchanged = true;
                pane.conflict_resolver_toggle_collapse_context(cx);
                reset_conflict_scroll_matrix_offsets(pane);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_scroll_handle_offset(
                    &pane.conflict_resolved_output_editor_scroll,
                    point(px(0.0), px(-240.0)),
                );
                pane.record_conflict_vertical_wheel_master(3);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    let settled = read_conflict_scroll_snapshot(cx, &view);
    for frame in 1..=3 {
        draw_and_drain_test_window(cx);
        let idle = read_conflict_scroll_snapshot(cx, &view);
        assert_eq!(
            idle, settled,
            "idle frame {frame} moved the resolver panes with collapsed context",
        );
    }

    std::fs::remove_dir_all(&workdir).expect("cleanup resolver idle-frame fixture");
}

#[gpui::test]
fn conflict_canvas_clicks_cannot_transfer_between_rows(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    let _guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(173);
    let path = std::path::PathBuf::from("conflict_clicks.txt");
    let base = "ctx\nb1\nb2\nb3\ntail\n";
    let ours = "ctx\no1\no2\no3\ntail\n";
    let theirs = "ctx\nt1\nt2\nt3\ntail\n";
    let current = "ctx\n<<<<<<< ours\no1\no2\no3\n=======\nt1\nt2\nt3\n>>>>>>> theirs\ntail\n";
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/conflict_completed_clicks"));
    set_test_conflict_status(
        &mut repo,
        path.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    set_test_conflict_file(&mut repo, path.clone(), base, ours, theirs, current);
    repo.conflict_state.conflict_file_load_mode = gitcomet_state::model::ConflictFileLoadMode::Full;
    repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
        path.clone(),
        gitcomet_core::domain::FileConflictKind::BothModified,
        ConflictPayload::Text(base.into()),
        ConflictPayload::Text(ours.into()),
        ConflictPayload::Text(theirs.into()),
        current,
    ));
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_canvas_rows_enabled = true;
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
            });
        })
    });
    cx.simulate_resize(gpui::size(px(1280.0), px(720.0)));
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "conflict canvas click fixture",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&path)
                && pane.conflict_resolver.manual_alignment_enabled()
                && pane
                    .conflict_text_hitboxes
                    .contains_key(&(2, ThreeWayColumn::Ours))
        },
        |pane| {
            format!(
                "path={:?} hitboxes={:?}",
                pane.conflict_resolver.path,
                pane.conflict_text_hitboxes.keys()
            )
        },
    );
    let [first, second] = cx.update(|_, app| {
        let pane = view.read(app).main_pane.read(app);
        [1, 2].map(|row| {
            pane.conflict_text_hitboxes[&(row, ThreeWayColumn::Ours)]
                .bounds
                .center()
        })
    });
    let menu_open = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| view.read(app).popover_host.read(app).is_open())
    };
    cx.simulate_mouse_down(first, MouseButton::Right, Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(!menu_open(cx), "a context menu waits for the release");
    cx.simulate_mouse_move(second, Some(MouseButton::Right), Modifiers::default());
    cx.simulate_mouse_up(second, MouseButton::Right, Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        !menu_open(cx),
        "a release on another row cannot open its menu"
    );
    cx.simulate_mouse_down(second, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(second, MouseButton::Right, Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        menu_open(cx),
        "the same row opens its menu on a completed click"
    );
    cx.update(|_, app| {
        view.read(app)
            .popover_host
            .clone()
            .update(app, |host, cx| host.close_popover(cx))
    });
    draw_and_drain_test_window(cx);

    let alt = Modifiers {
        alt: true,
        ..Default::default()
    };
    let marked_columns = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            view.read(app)
                .main_pane
                .read(app)
                .conflict_resolver_alignment_marked_columns()
        })
    };
    cx.simulate_mouse_down(first, MouseButton::Left, alt);
    draw_and_drain_test_window(cx);
    assert_eq!(
        marked_columns(cx),
        0,
        "alignment marking waits for the release"
    );
    cx.simulate_mouse_move(second, Some(MouseButton::Left), alt);
    cx.simulate_mouse_up(second, MouseButton::Left, alt);
    draw_and_drain_test_window(cx);
    assert_eq!(
        marked_columns(cx),
        0,
        "cancelled marking changes neither row"
    );
    cx.simulate_mouse_down(second, MouseButton::Left, alt);
    cx.simulate_mouse_up(second, MouseButton::Left, alt);
    draw_and_drain_test_window(cx);
    assert_eq!(marked_columns(cx), 1, "a completed Alt-click marks the row");
}
