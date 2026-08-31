use super::*;

fn build_multi_conflict_sides() -> (String, String, String, String) {
    let mut base = Vec::new();
    let mut ours = Vec::new();
    let mut theirs = Vec::new();
    let mut current = Vec::new();
    for block in 0..8 {
        for ctx in 0..12 {
            let line = format!("context {block:02}/{ctx:02} shared text");
            base.push(line.clone());
            ours.push(line.clone());
            theirs.push(line.clone());
            current.push(line);
        }
        // Asymmetric block sizes: ours grows with the block index, theirs
        // shrinks, so no single global ratio maps the two row spaces.
        let ours_len = 2 + block;
        let theirs_len = 10 - block;
        let settled = block % 2 == 1;
        base.push(format!("base block {block:02}"));
        if !settled {
            current.push("<<<<<<< ours".to_string());
        }
        for line in 0..ours_len {
            let text = format!("ours {block:02}/{line:02}");
            ours.push(text.clone());
            current.push(text);
        }
        if !settled {
            current.push("=======".to_string());
        }
        for line in 0..theirs_len {
            let text = format!("theirs {block:02}/{line:02}");
            theirs.push(text.clone());
            if !settled {
                current.push(text);
            }
        }
        if !settled {
            current.push(">>>>>>> theirs".to_string());
        }
    }
    let join = |lines: Vec<String>| format!("{}\n", lines.join("\n"));
    (join(base), join(ours), join(theirs), join(current))
}

/// The resolved output scrolls entirely on its own: walking a source column
/// down must leave it exactly where it was, and vice versa.
///
/// This is the KDiff3 behaviour — its merge result window owns a scrollbar the
/// diff windows are not connected to. Offsets are never mapped between the two
/// documents, because on a real conflict, where a changed block can occur every
/// few rows, no continuous mapping between them exists. Navigation is what
/// brings the two panes onto the same block.
#[gpui::test]
fn conflict_resolver_output_scrolls_independently_of_the_columns(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(192);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_independent_output_scroll",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_independent_output_scroll.txt");
    let abs_path = workdir.join(&file_rel);
    let (base_text, ours_text, theirs_text, current_text) = build_multi_conflict_sides();

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver independence fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver independence fixture");

    seed_unresolved_conflict_state(
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
        "resolver independence fixture initialized",
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.three_way_visible_len() >= 4
                && pane.conflict_resolved_preview_line_count >= 1
        },
        |pane| {
            format!(
                "path={:?} visible={} lines={}",
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
        "resolver independence overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).height > px(400.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).height
                    > px(400.0)
        },
        |pane| {
            format!(
                "base_max={:?} output_max={:?}",
                uniform_list_max_offset(&pane.conflict_resolver_diff_scroll),
                scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll),
            )
        },
    );

    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::Both);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                // Output scroll sync on, which is the demanding case: even
                // then the vertical axis carries no relationship between the
                // resolved output and the columns.
                pane.mergetool_output_scroll_sync = true;
                reset_conflict_scroll_matrix_offsets(pane);
                // Park the output partway down so a stray coupling would show
                // up as movement in either direction.
                set_scroll_handle_offset(
                    &pane.conflict_resolved_output_editor_scroll,
                    point(px(0.0), px(-400.0)),
                );
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    let parked_output = cx.update(|_window, app| {
        scroll_handle_offset(
            &view
                .read(app)
                .main_pane
                .read(app)
                .conflict_resolved_output_editor_scroll,
        )
        .y
    });

    // Walk the base column the length of the file. The output must not budge,
    // and the other two columns must track the base exactly.
    let column_max = cx.update(|_window, app| {
        uniform_list_max_offset(
            &view
                .read(app)
                .main_pane
                .read(app)
                .conflict_resolver_diff_scroll,
        )
        .height
    });
    let mut row = 0.0f32;
    while px(row * 20.0) < column_max {
        let target = point(px(0.0), px(-row * 20.0));
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    set_uniform_list_offset(&pane.conflict_resolver_diff_scroll, target);
                    pane.record_conflict_vertical_wheel_master(0);
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);

        let snapshot = read_conflict_scroll_snapshot(cx, &view);
        assert!(
            (f32::from(snapshot.output) - f32::from(parked_output)).abs() < 0.5,
            "scrolling the base column to row {row} moved the resolved output from \
             {parked_output:?} to {:?}",
            snapshot.output,
        );
        assert!(
            (f32::from(snapshot.ours) - f32::from(snapshot.base)).abs() < 0.5
                && (f32::from(snapshot.theirs) - f32::from(snapshot.base)).abs() < 0.5,
            "the aligned columns share one row space and must stay together: {snapshot:?}",
        );
        row += 1.0;
    }

    // And the reverse: scrolling the output leaves the columns alone.
    let parked_columns = read_conflict_scroll_snapshot(cx, &view).base;
    let output_max = cx.update(|_window, app| {
        scroll_handle_max_offset(
            &view
                .read(app)
                .main_pane
                .read(app)
                .conflict_resolved_output_editor_scroll,
        )
        .height
    });
    let mut row = 0.0f32;
    while px(row * 20.0) < output_max {
        let target = point(px(0.0), px(-row * 20.0));
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    set_scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll, target);
                    pane.record_conflict_vertical_wheel_master(3);
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);

        let snapshot = read_conflict_scroll_snapshot(cx, &view);
        assert!(
            (f32::from(snapshot.base) - f32::from(parked_columns)).abs() < 0.5,
            "scrolling the resolved output to row {row} moved the base column from \
             {parked_columns:?} to {:?}",
            snapshot.base,
        );
        row += 1.0;
    }

    std::fs::remove_dir_all(&workdir).expect("cleanup resolver independence fixture");
}

/// A freshly materialized resolved output must not leave the caret parked at
/// end-of-document: the pane opens at the top, so the first arrow key would
/// autoscroll the whole coupled group to the bottom of the file.
#[gpui::test]
fn conflict_resolver_materialized_output_parks_caret_at_the_start(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(193);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_caret_park",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_caret_park.txt");
    let abs_path = workdir.join(&file_rel);
    let (base_text, ours_text, theirs_text, current_text) = build_multi_conflict_sides();

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver caret-park fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver caret-park fixture");

    seed_unresolved_conflict_state(
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
        "resolver caret-park fixture initialized",
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
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let input = pane.conflict_resolver_input.read(app);
        assert!(
            input.text().len() > 100,
            "fixture should have materialized a multi-line output",
        );
        assert_eq!(
            input.selected_range(),
            0..0,
            "a freshly materialized resolved output should park the caret at the start",
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup resolver caret-park fixture");
}

/// Seed a conflict session with every region left unresolved, so the resolved
/// output still renders conflict markers and the column/output anchor list is
/// non-trivial.
fn seed_unresolved_conflict_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &std::path::Path,
    file_rel: &std::path::Path,
    base_text: &str,
    ours_text: &str,
    theirs_text: &str,
    current_text: &str,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.to_path_buf(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            set_test_conflict_file(
                &mut repo,
                file_rel.to_path_buf(),
                base_text.to_string(),
                ours_text.to_string(),
                theirs_text.to_string(),
                current_text.to_string(),
            );
            // Plan-backed, the way the app builds a full-text conflict:
            // `from_merged_text` derives geometry from whatever markers happen
            // to be in the worktree and leaves `merge_plan` empty, which would
            // silently exercise only the marker-only anchor fallback.
            repo.conflict_state.conflict_session =
                Some(ConflictSession::from_stage_inputs_with_current(
                    file_rel.to_path_buf(),
                    gitcomet_core::domain::FileConflictKind::BothModified,
                    ConflictPayload::Text(base_text.to_string().into()),
                    ConflictPayload::Text(ours_text.to_string().into()),
                    ConflictPayload::Text(theirs_text.to_string().into()),
                    Some(ConflictPayload::Text(current_text.to_string().into())),
                ));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
}

/// Conflict navigation centers the aligned row in the source columns and the
/// output line in the resolved output, independently. Those two panes are the
/// halves of the vsplit and therefore have different heights, while the
/// column/output scroll sync aligns their *top* rows. Two centerings cannot
/// both survive that, so the sync drags one onto the other and the loser jumps.
#[gpui::test]
fn conflict_navigation_settles_without_a_second_jump(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(194);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_nav_center_jump",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_nav_center_jump.txt");
    let abs_path = workdir.join(&file_rel);
    let (base_text, ours_text, theirs_text, current_text) = build_multi_conflict_sides();

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver nav-center fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver nav-center fixture");

    seed_unresolved_conflict_state(
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
        "resolver nav-center fixture initialized",
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.three_way_visible_len() >= 4
                && pane.conflict_resolved_preview_line_count >= 1
        },
        |pane| {
            format!(
                "path={:?} visible={} lines={}",
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
        "resolver nav-center overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).height > px(400.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).height
                    > px(400.0)
        },
        |pane| {
            format!(
                "base_max={:?} output_max={:?}",
                uniform_list_max_offset(&pane.conflict_resolver_diff_scroll),
                scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll),
            )
        },
    );

    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::Both);

    for target in 1..5usize {
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
                    pane.conflict_jump_to_nav_target(target, cx);
                });
            });
        });
        // Let the deferred item scrolls land and the synchronizer observe them.
        draw_and_drain_test_window(cx);
        draw_and_drain_test_window(cx);
        let settled = read_conflict_scroll_snapshot(cx, &view);

        for frame in 1..=3 {
            draw_and_drain_test_window(cx);
            let idle = read_conflict_scroll_snapshot(cx, &view);
            let output_jump = f32::from(idle.output) - f32::from(settled.output);
            let column_jump = f32::from(idle.base) - f32::from(settled.base);
            assert!(
                output_jump.abs() < 1.0 && column_jump.abs() < 1.0,
                "target {target}, idle frame {frame}: navigation did not settle — output \
                 moved {output_jump}px ({:.1} lines), columns moved {column_jump}px \
                 ({:.1} lines); settled={settled:?} idle={idle:?}",
                output_jump / 20.0,
                column_jump / 20.0,
            );
        }
    }

    std::fs::remove_dir_all(&workdir).expect("cleanup resolver nav-center fixture");
}

/// The resolved output washes the conflict being resolved in yellow, and the
/// wash has to follow conflict navigation.
///
/// Navigating moves no text and touches no tree, so none of the paths that
/// normally reinstall the output's highlights fire — the pane only reassigns
/// `active_conflict`. Without the render pass noticing that, the wash stays
/// parked on whichever conflict the file opened on, which is worse than no wash
/// at all: it points at the wrong row.
#[gpui::test]
fn the_resolved_output_wash_follows_conflict_navigation(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(197);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_active_conflict_wash",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/active_conflict_wash.txt");
    let abs_path = workdir.join(&file_rel);
    let base = "head\nbase one\nmiddle\nbase two\ntail\n";
    let ours = "head\nours one\nmiddle\nours two\ntail\n";
    let theirs = "head\ntheirs one\nmiddle\ntheirs two\ntail\n";
    let current = "head\n\
                   <<<<<<< ours\nours one\n=======\ntheirs one\n>>>>>>> theirs\n\
                   middle\n\
                   <<<<<<< ours\nours two\n=======\ntheirs two\n>>>>>>> theirs\n\
                   tail\n";

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create active-conflict wash fixture dir");
    std::fs::write(&abs_path, current).expect("write active-conflict wash fixture");

    seed_unresolved_conflict_state(
        cx, &view, repo_id, &workdir, &file_rel, base, ours, theirs, current,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "two-conflict wash fixture initialized",
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ) == 2
                && !pane.conflict_resolved_output_is_streamed()
        },
        |pane| {
            format!(
                "path={:?} blocks={} streamed={}",
                pane.conflict_resolver.path.clone(),
                crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ),
                pane.conflict_resolved_output_is_streamed(),
            )
        },
    );

    // Both placeholder rows read `<Merge Conflict>`, so only their offsets can
    // say which one is washed.
    let placeholder = crate::view::conflict_resolver::UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER;
    let output = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .conflict_resolver_input
            .read(app)
            .text()
            .to_string()
    });
    let first = output.find(placeholder).expect("first placeholder row");
    let second = output[first + placeholder.len()..]
        .find(placeholder)
        .expect("second placeholder row")
        + first
        + placeholder.len();

    let washed_ranges = |cx: &mut gpui::VisualTestContext| -> Vec<std::ops::Range<usize>> {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    let wash = crate::view::panes::main::resolved_output_active_conflict_background(
                        pane.theme,
                    );
                    let len = pane.conflict_resolver_input.read(cx).text().len();
                    pane.conflict_resolver_input
                        .update(cx, |input, _| {
                            input.debug_effective_highlights_for_range(0..len)
                        })
                        .into_iter()
                        .filter(|(_, style)| style.background_color == Some(wash.into_color()))
                        .map(|(range, _)| range)
                        .collect()
                })
            })
        })
    };

    for (conflict_ix, expected_start) in [(0usize, first), (1, second), (0, first)] {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.conflict_resolver_select_conflict(conflict_ix, cx);
                });
            });
        });
        draw_and_drain_test_window(cx);

        assert_eq!(
            washed_ranges(cx),
            vec![expected_start..expected_start + placeholder.len()],
            "selecting conflict {conflict_ix} must wash its row and only its row"
        );
    }

    // A pick can settle on a block that renders no marker, leaving nothing
    // selected. The wash has to come off then too, rather than staying on the
    // row the last selection put it on.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver.active_conflict = None;
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        washed_ranges(cx).is_empty(),
        "with no conflict selected there is nothing for the wash to point at"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup active-conflict wash fixture");
}

/// An HTML-shaped fixture with the structure that exposed the bug: a repeated
/// card, most of which the planner settles on its own, and a handful of real
/// conflicts spread through the file.
///
/// The repetition matters — it is what makes the marker-projection estimate
/// drift, because the same text appears in every card.
fn build_repetitive_card_conflict_sides() -> (String, String, String, String) {
    let mut base = Vec::new();
    let mut ours = Vec::new();
    let mut theirs = Vec::new();

    base.push("<html>".to_string());
    ours.push("<html>".to_string());
    theirs.push("<html>".to_string());
    for card in 0..24 {
        let head = [
            format!("  <article class=\"card\" id=\"card-{card:02}\">"),
            "    <header>".to_string(),
            format!("      <h2>Card {card:02}</h2>"),
        ];
        for line in &head {
            base.push(line.clone());
            ours.push(line.clone());
            theirs.push(line.clone());
        }

        // Every third card is a real conflict; the ones between it are edits
        // both sides made the same way, which the planner resolves by itself.
        match card % 3 {
            0 => {
                base.push("      <span>Healthy</span>".to_string());
                ours.push("      <span>Local override</span>".to_string());
                theirs.push("      <span>Remote canary</span>".to_string());
            }
            1 => {
                base.push("      <span>Healthy</span>".to_string());
                ours.push("      <span>Shared rollout</span>".to_string());
                theirs.push("      <span>Shared rollout</span>".to_string());
            }
            _ => {
                for side in [&mut base, &mut ours, &mut theirs] {
                    side.push("      <span>Healthy</span>".to_string());
                }
            }
        }

        let tail = [
            "    </header>".to_string(),
            "    <div class=\"body\">".to_string(),
            "      <p>Nominal traffic across all production cells.</p>".to_string(),
            "    </div>".to_string(),
            "  </article>".to_string(),
        ];
        for line in &tail {
            base.push(line.clone());
            ours.push(line.clone());
            theirs.push(line.clone());
        }
    }
    base.push("</html>".to_string());
    ours.push("</html>".to_string());
    theirs.push("</html>".to_string());

    let join = |lines: Vec<String>| format!("{}\n", lines.join("\n"));
    let (base, ours, theirs) = (join(base), join(ours), join(theirs));
    let current = gitcomet_core::merge::merge_file_with_optional_base(
        Some(base.as_str()),
        &ours,
        &theirs,
        &gitcomet_core::merge::MergeOptions::default(),
    )
    .output;
    (base, ours, theirs, current)
}

/// Every conflict highlight must stay inside the block it belongs to.
///
/// The highlight is driven by `three_way_conflict_ranges` (the chunk bar and
/// the active-conflict tint) and by the nav targets' `aligned_rows`. Both are
/// exact when the merge plan describes them; the marker-projection estimate
/// used to be able to hand a block a range running to the end of the file,
/// which painted the whole tail as one enormous selected conflict.
fn assert_conflict_highlight_ranges_are_bounded(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    stage: &str,
) {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let aligned_len = pane.conflict_resolver.three_way_len;
        let ranges =
            &pane.conflict_resolver.three_way_conflict_ranges[crate::view::ThreeWayColumn::Ours];
        assert!(
            !ranges.is_empty(),
            "{stage}: the fixture must still have conflicts to highlight",
        );

        // No block may claim more than a modest slice of the file. The fixture's
        // conflicts are a line or two; anything spanning a quarter of the aligned
        // rows is the runaway range this guards against.
        let budget = (aligned_len / 4).max(8);
        for (ix, range) in ranges.iter().enumerate() {
            assert!(
                range.end <= aligned_len,
                "{stage}: conflict {ix} range {range:?} leaves the aligned space \
                 (len {aligned_len})",
            );
            assert!(
                range.len() <= budget,
                "{stage}: conflict {ix} spans {} of {aligned_len} aligned rows ({range:?})",
                range.len(),
            );
        }
        for pair in ranges.windows(2) {
            assert!(
                pair[0].end <= pair[1].start,
                "{stage}: conflict ranges overlap or go backwards: {pair:?}",
            );
        }

        for (ix, target) in pane.conflict_resolver.nav_targets.iter().enumerate() {
            let Some(rows) = target.aligned_rows.as_ref() else {
                continue;
            };
            assert!(
                rows.end <= aligned_len && rows.len() <= budget,
                "{stage}: nav target {ix} spans {rows:?} of {aligned_len} aligned rows",
            );
        }

        // The painted highlight itself: every aligned row the source columns
        // mark as the active conflict has to belong to a conflict.
        let highlighted = (0..aligned_len)
            .filter(|row| {
                let conflict_ix = pane
                    .conflict_resolver
                    .conflict_index_for_side_line(crate::view::ThreeWayColumn::Ours, *row);
                pane.conflict_resolver.conflict_is_active(conflict_ix)
                    || pane
                        .conflict_resolver
                        .selected_nav_target_contains_aligned_row(*row)
            })
            .count();
        assert!(
            highlighted <= budget,
            "{stage}: {highlighted} of {aligned_len} aligned rows are painted as the \
             active conflict (active={:?})",
            pane.conflict_resolver.active_conflict,
        );
    });
}

/// The active-conflict highlight must never cover more than the conflict it
/// belongs to — not on open, not after a pick, and above all not when nothing
/// is selected, which is where it used to swallow the rest of the file.
///
/// It guards the two range sources (the aligned conflict ranges behind the
/// chunk bar, and the nav targets' `aligned_rows`) as well as the predicate the
/// source columns actually paint with.
#[gpui::test]
fn the_conflict_highlight_stays_inside_the_conflict_it_belongs_to(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(193);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_highlight_bounds",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_highlight_bounds.html");
    let abs_path = workdir.join(&file_rel);
    let (base_text, ours_text, theirs_text, current_text) = build_repetitive_card_conflict_sides();

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create conflict highlight fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write conflict highlight fixture");

    seed_unresolved_conflict_state(
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
        "conflict highlight fixture initialized",
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && !pane.conflict_resolver.three_way_aligned.is_identity()
                && pane.conflict_resolver.three_way_conflict_ranges
                    [crate::view::ThreeWayColumn::Ours]
                    .len()
                    >= 4
        },
        |pane| {
            format!(
                "path={:?} identity={} ranges={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.three_way_aligned.is_identity(),
                pane.conflict_resolver.three_way_conflict_ranges[crate::view::ThreeWayColumn::Ours]
                    .len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
            });
        });
    });
    draw_and_drain_test_window(cx);
    assert_conflict_highlight_ranges_are_bounded(cx, &view, "on open");

    // Pick a source on a conflict partway down the file — the case the user hit.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_select_conflict(3, cx);
                pane.conflict_resolver_pick_active_conflict(
                    crate::view::conflict_resolver::ConflictChoice::Theirs,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);
    assert_conflict_highlight_ranges_are_bounded(cx, &view, "after picking a source");

    // Nothing selected is the state a pick can settle into: the anchor lands on
    // a block that renders no marker, so there is no displayed conflict index.
    // Rows outside every conflict must stay unmarked — comparing the row's
    // `Option` conflict index against the equally-`None` selection is what used
    // to light up the whole file below the conflict that was just resolved.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver.active_conflict = None;
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    assert_conflict_highlight_ranges_are_bounded(cx, &view, "with nothing selected");

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict highlight fixture");
}

#[gpui::test]
fn measure_resolved_output_typing_rerenders(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(917);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_typing_rerender",
        std::process::id()
    ));
    // A real source file, so the syntax-highlighting path a user actually hits
    // is in the measurement.
    let file_rel = std::path::PathBuf::from("fixtures/conflict_typing_rerender.rs");
    let abs_path = workdir.join(&file_rel);
    let side_lines: usize = std::env::var("GITCOMET_MEASURE_LINES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2000);
    let big = |label: &str, fill: char| -> String {
        (0..side_lines)
            .map(|ix| {
                format!(
                    "fn {label}_{ix:05}(value: usize) -> String {{ format!(\"{}{ix}\", value) }}",
                    fill.to_string().repeat(20)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let base_text = big("base", 'B');
    let ours_text = big("ours", 'O');
    let theirs_text = big("theirs", 'T');
    let current_text = build_conflict_scroll_matrix_current_text(&ours_text, &theirs_text);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create typing fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write typing fixture");

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
        "typing fixture initialized",
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolved_preview_line_count >= 1
        },
        |pane| format!("path={:?}", pane.conflict_resolver.path.clone()),
    );
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    // Confirm the resolver columns really render in this window before trusting
    // any of the numbers below.
    crate::view::perf::reset();
    cx.update(|window, app| {
        main_pane.update(app, |_pane, cx| cx.notify());
        let _ = window.draw(app);
    });
    eprintln!(
        "MEASURE cold full draw perf: {:?}",
        crate::view::perf::snapshot()
    );
    cx.run_until_parked();

    let main_notifies = Arc::new(AtomicUsize::new(0));
    let _main_notify_sub = cx.update(|_window, app| {
        let main_notifies = Arc::clone(&main_notifies);
        main_pane.update(app, |_pane, cx| {
            cx.observe_self(move |_pane, _cx| {
                main_notifies.fetch_add(1, Ordering::Relaxed);
            })
        })
    });

    let streamed =
        cx.update(|_window, app| main_pane.read(app).conflict_resolved_output_is_streamed());
    eprintln!("MEASURE streamed={streamed}");
    eprintln!(
        "MEASURE size_of::<ShapedLine>()={} lines={}",
        std::mem::size_of::<gpui::ShapedLine>(),
        cx.update(|_window, app| main_pane
            .read(app)
            .conflict_resolver_input
            .read(app)
            .text()
            .lines()
            .count()),
    );

    cx.update(|_window, app| {
        main_pane.update(app, |pane, _cx| {
            pane.set_conflict_resolved_outline_background_delay_override_for_tests(
                std::time::Duration::from_millis(500),
            );
        });
    });

    // Idle baseline: draws with no edits at all.
    main_notifies.store(0, Ordering::Relaxed);
    let idle_started = std::time::Instant::now();
    for _ in 0..5 {
        draw_and_drain_test_window(cx);
    }
    eprintln!(
        "MEASURE idle: 5 draws in {:?}, main notifies={}",
        idle_started.elapsed(),
        main_notifies.load(Ordering::Relaxed)
    );

    // Cost of a bare main-pane re-render (notify, no edit).
    main_notifies.store(0, Ordering::Relaxed);
    for ix in 0..5usize {
        cx.update(|_window, app| {
            main_pane.update(app, |_pane, cx| cx.notify());
        });
        let draw_started = std::time::Instant::now();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        eprintln!(
            "MEASURE notify-only draw {ix}: {:?}",
            draw_started.elapsed()
        );
        cx.run_until_parked();
    }

    // Typing: one character at a time, each followed by a frame.
    main_notifies.store(0, Ordering::Relaxed);
    let typing_started = std::time::Instant::now();
    for ix in 0..5usize {
        let before = main_notifies.load(Ordering::Relaxed);
        crate::view::perf::reset();
        let keystroke_started = std::time::Instant::now();
        let buffer_elapsed = cx.update(|_window, app| {
            main_pane.update(app, |pane, cx| {
                pane.conflict_resolver_input.update(cx, |input, cx| {
                    let at = input.text().len().min(40);
                    let started = std::time::Instant::now();
                    input.replace_utf8_range(at..at, "x", cx);
                    started.elapsed()
                })
            })
        });
        // Everything after the buffer edit but before the frame: the
        // `cx.observe(conflict_resolver_input)` closure and any other flushed
        // effects.
        let effects_elapsed = keystroke_started.elapsed() - buffer_elapsed;
        // `flush_effects` auto-draws dirty windows in test builds, so the frame
        // the keystroke causes is already inside `effects_elapsed`.
        let effects_perf = crate::view::perf::snapshot();
        let after_edit = main_notifies.load(Ordering::Relaxed);
        crate::view::perf::reset();
        let draw_started = std::time::Instant::now();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let draw_elapsed = draw_started.elapsed();
        let perf = crate::view::perf::snapshot();
        // Debounced follow-up work (outline recompute, syntax refresh) plus the
        // frame it schedules.
        crate::view::perf::reset();
        let settle_started = std::time::Instant::now();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let settle_elapsed = settle_started.elapsed();
        let settle_perf = crate::view::perf::snapshot();
        cx.run_until_parked();
        eprintln!(
            "MEASURE keystroke {ix}: buffer={buffer_elapsed:?} effects={effects_elapsed:?} (notifies {}) draw={draw_elapsed:?} settle={settle_elapsed:?} (notifies {})",
            after_edit - before,
            main_notifies.load(Ordering::Relaxed) - before,
        );
        eprintln!("MEASURE keystroke {ix} effects perf: {effects_perf:?}");
        eprintln!("MEASURE keystroke {ix} draw perf: {perf:?}");
        eprintln!("MEASURE keystroke {ix} settle perf: {settle_perf:?}");
    }
    eprintln!(
        "MEASURE typing: 5 keystrokes in {:?}, main notifies={}",
        typing_started.elapsed(),
        main_notifies.load(Ordering::Relaxed)
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup typing fixture");
}

/// Assert the resolved output is coloured by tree-sitter rather than by the
/// heuristic tokenizer.
///
/// The two engines agree on keywords, strings, numbers and comments, so an
/// assertion built from those classes cannot see the difference. These four
/// probes can: `syntax/heuristic.rs` has no notion of a `primitive_type`, a
/// `type_identifier`, a `field_identifier` or a method call, and leaves all of
/// them plain. If any comes back uncoloured, the pane is on the fallback --
/// which is what the diff panes above it are *not* on, hence the mismatch.
pub(super) fn assert_resolved_output_carries_treesitter_classes(
    text: &str,
    highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    theme: crate::theme::AppTheme,
) {
    for (needle, class, expected) in [
        ("usize", "primitive_type", theme.syntax.type_builtin),
        ("Stage {", "type_identifier", theme.syntax.type_name),
        ("retries: usize", "field_identifier", theme.syntax.property),
        ("wrapping_add", "method call", theme.syntax.function_method),
    ] {
        let at = text
            .find(needle)
            .unwrap_or_else(|| panic!("fixture should contain {needle:?}"));
        let found = highlights
            .iter()
            .find(|(range, _)| range.start <= at && range.end > at)
            .and_then(|(_, style)| style.color);
        assert_eq!(
            found,
            Some(expected.into_color()),
            "{needle:?} at {at} is a {class} and must carry its tree-sitter colour; \
             the heuristic tokenizer leaves it plain, so a mismatch here means the \
             resolved output never got a live document"
        );
    }
}

/// A dark theme that differs from `gitcomet_dark` only in its syntax palette.
pub(super) fn other_dark_theme() -> crate::theme::AppTheme {
    crate::theme::AppTheme::from_json_str(&crate::theme::test_theme_json_with_syntax(
        "gitcomet_dark",
        r##"{
            "keyword": "#112233ff",
            "comment": "#445566ff"
        }"##,
    ))
    .expect("fixture theme JSON should parse")
}
/// The placeholder mask for a resolved-output text, matching what the pane
/// derives: the placeholder rows minus their line terminator.
pub(super) fn resolved_output_placeholder_protected_ranges_for_test(
    text: &str,
) -> Arc<[std::ops::Range<usize>]> {
    let mut mask = Vec::new();
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if conflict_resolver::line_is_unresolved_conflict_placeholder(trimmed) {
            mask.push(offset..offset + trimmed.len());
        }
        offset += line.len();
    }
    mask.into()
}

/// Conflict navigation must move the editable output in the frame it happens,
/// not leave it parked until some unrelated event repaints the pane.
///
/// The columns and the gutter are lists with their own deferred scroll; the
/// output is a `TextInput` that used to be dragged along only by a prepaint
/// mirror. The assertion is made *inside* the update that navigates — before
/// any draw — so it can only pass if navigation placed the editor itself.
#[gpui::test]
fn conflict_navigation_places_the_editable_output_without_waiting_for_a_frame(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(953);
    let fixture = SyntheticLargeConflictFixture::new(
        "resolver_nav_places_output",
        "fixtures/resolver_nav_places_output.rs",
        900,
        12,
    );
    fixture.write();

    seed_unresolved_conflict_state(
        cx,
        &view,
        repo_id,
        &fixture.workdir,
        &fixture.file_rel,
        &fixture.base_text,
        &fixture.ours_text,
        &fixture.theirs_text,
        &fixture.current_text,
    );

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "nav placement fixture initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && !pane.conflict_resolver.nav_targets.is_empty()
                && !pane.conflict_resolved_output_is_streamed()
        },
        |pane| {
            format!(
                "targets={} streamed={}",
                pane.conflict_resolver.nav_targets.len(),
                pane.conflict_resolved_output_is_streamed(),
            )
        },
    );
    // Two draws: the first gives the gutter and the editor their bounds, which
    // is what the offset arithmetic reads.
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    let before = cx.update(|_window, app| {
        main_pane
            .read(app)
            .conflict_resolved_output_editor_scroll
            .offset()
            .y
    });

    // Jump far enough down that the target cannot already be on screen.
    let after = cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            for _ in 0..6 {
                pane.conflict_jump_next(cx);
            }
            pane.conflict_resolved_output_editor_scroll.offset().y
        })
    });

    assert!(
        after < before,
        "navigating six conflicts down must scroll the editable output immediately \
         (before={before:?} after={after:?})"
    );

    // And the placement has to be the one the gutter lands on, or the mirror
    // that runs on the next prepaint would jerk the view a second time.
    draw_and_drain_test_window(cx);
    let settled = cx.update(|_window, app| {
        main_pane
            .read(app)
            .conflict_resolved_output_editor_scroll
            .offset()
            .y
    });
    assert_eq!(
        settled, after,
        "the drawn frame must agree with the offset navigation placed"
    );

    fixture.cleanup();
}

/// Conflict navigation must not re-run the resolved output's edit pipeline.
///
/// Moving the yellow wash rebinds the highlight provider, which notifies the
/// input, which re-enters the observe that refreshes syntax. Without an
/// early-out that refresh rescans the whole document — two line walks and a
/// materialization — on every F3. Materialization is the observable half, so
/// that is what this pins.
#[gpui::test]
fn conflict_navigation_does_not_rescan_the_resolved_output(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(983);
    let fixture = SyntheticLargeConflictFixture::new(
        "nav_no_rescan",
        "fixtures/nav_no_rescan.html",
        4_000,
        24,
    );
    fixture.write();

    seed_unresolved_conflict_state(
        cx,
        &view,
        repo_id,
        &fixture.workdir,
        &fixture.file_rel,
        &fixture.base_text,
        &fixture.ours_text,
        &fixture.theirs_text,
        &fixture.current_text,
    );

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "nav rescan fixture initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && !pane.conflict_resolver.nav_targets.is_empty()
                && !pane.conflict_resolved_output_is_streamed()
        },
        |pane| format!("targets={}", pane.conflict_resolver.nav_targets.len()),
    );
    draw_and_drain_test_window(cx);

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());

    // An edit legitimately rescans. Navigation must not add any.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            let at = pane.conflict_resolver_input.read(cx).text().len();
            pane.conflict_resolver_input.update(cx, |input, cx| {
                input.replace_utf8_range(at..at, "\n", cx);
            });
        });
    });
    draw_and_drain_test_window(cx);

    let before = cx.update(|_window, app| main_pane.read(app).conflict_resolved_output_full_scans);

    for _ in 0..4 {
        cx.update(|_window, app| {
            main_pane.update(app, |pane, cx| {
                pane.conflict_jump_next(cx);
            });
        });
        draw_and_drain_test_window(cx);
    }

    let after = cx.update(|_window, app| main_pane.read(app).conflict_resolved_output_full_scans);
    assert_eq!(
        after, before,
        "four conflict jumps changed no text, so none of them may rescan the document"
    );

    fixture.cleanup();
}

/// Shift+F2/F3 step between *unresolved* conflicts, in both focus states.
///
/// The chord replaces Ctrl+PgUp/PgDn, which collided with repository-tab
/// switching. Two things have to hold that plain F2/F3 does not give you: the
/// jump *skips over* conflicts already resolved, and it still fires while the
/// resolved-output editor has focus — resolving a merge means typing in that
/// editor, so a shortcut that dies there is the one you need most.
///
/// The resolved conflict is deliberately placed *between* the starting point
/// and the expected destination. Resolving the conflict you are standing on
/// and then stepping forward proves nothing: unfiltered navigation leaves it
/// too, simply by moving.
#[gpui::test]
fn shift_f2_and_f3_step_over_resolved_conflicts(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(991);
    let fixture =
        SyntheticLargeConflictFixture::new("shift_f3_nav", "fixtures/shift_f3_nav.html", 400, 6);
    fixture.write();
    seed_unresolved_conflict_state(
        cx,
        &view,
        repo_id,
        &fixture.workdir,
        &fixture.file_rel,
        &fixture.base_text,
        &fixture.ours_text,
        &fixture.theirs_text,
        &fixture.current_text,
    );
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "shift-f3 nav fixture initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver
                .nav_targets
                .iter()
                .filter(|target| target.unresolved)
                .count()
                >= 3
        },
        |pane| format!("targets={}", pane.conflict_resolver.nav_targets.len()),
    );
    draw_and_drain_test_window(cx);

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());

    // Nav-target positions of the first three still-open conflicts.
    let open: Vec<usize> = cx.update(|_window, app| {
        main_pane
            .read(app)
            .conflict_resolver
            .nav_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.unresolved)
            .map(|(ix, _)| ix)
            .collect()
    });
    let (start, middle, beyond) = (open[0], open[1], open[2]);

    // Resolve the middle one, so it sits between the caret and the next open
    // conflict. This is the conflict the chord must step over.
    let middle_display = cx.update(|_window, app| {
        main_pane.read(app).conflict_resolver.nav_targets[middle]
            .display_conflict_index
            .expect("an unresolved conflict target has a display index")
    });
    // Resolve the middle conflict for real — select it and pick a side, the way
    // a user does — so the chord is exercised against genuine resolution state.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_resolver_select_conflict(middle_display, cx);
            pane.conflict_resolver_pick_active_conflict(
                crate::view::conflict_resolver::ConflictChoice::Ours,
                cx,
            );
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_jump_to_nav_target(start, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let order_of = |cx: &mut gpui::VisualTestContext, target_ix: usize| -> usize {
        cx.update(|_window, app| main_pane.read(app).conflict_resolver.nav_targets[target_ix].order)
    };
    let anchor = |cx: &mut gpui::VisualTestContext| -> Option<usize> {
        cx.update(|_window, app| {
            main_pane
                .read(app)
                .conflict_resolver
                .nav_anchor
                .map(|anchor| anchor.order_hint)
        })
    };
    let press = |cx: &mut gpui::VisualTestContext, chord: &str| -> bool {
        let keystroke = gpui::Keystroke::parse(chord).expect("valid chord");
        cx.update(|window, app| {
            main_pane.update(app, |pane, cx| {
                pane.handle_diff_shortcut(&keystroke, window, cx)
            })
        })
    };

    let (middle_order, beyond_order) = (order_of(cx, middle), order_of(cx, beyond));
    assert!(
        cx.update(|_window, app| {
            !main_pane.read(app).conflict_resolver.nav_targets[middle].unresolved
        }),
        "the middle conflict must be marked resolved for this test to mean anything"
    );
    assert_eq!(
        anchor(cx),
        Some(order_of(cx, start)),
        "should start at the first open conflict"
    );

    assert!(press(cx, "shift-f3"), "shift-f3 should be handled");
    draw_and_drain_test_window(cx);
    assert_ne!(
        anchor(cx),
        Some(middle_order),
        "shift-f3 landed on the conflict that was just resolved: it is navigating \
         conflicts, not unresolved conflicts"
    );
    assert_eq!(
        anchor(cx),
        Some(beyond_order),
        "shift-f3 should skip the resolved conflict and land on the next open one"
    );

    // Shift+F2 comes back the same way, skipping the same resolved conflict.
    assert!(press(cx, "shift-f2"), "shift-f2 should be handled");
    draw_and_drain_test_window(cx);
    assert_ne!(
        anchor(cx),
        Some(middle_order),
        "shift-f2 must skip the resolved conflict too"
    );

    // The chord must survive focus being in the resolved-output editor, which
    // is where a merge is actually resolved.
    cx.update(|window, app| {
        main_pane.update(app, |pane, cx| {
            let handle = pane.conflict_resolver_input.read(cx).focus_handle();
            handle.focus(window, cx);
        });
    });
    draw_and_drain_test_window(cx);
    let before_focused = anchor(cx);
    assert!(
        press(cx, "shift-f3"),
        "shift-f3 must still be handled while the resolved-output editor has focus"
    );
    draw_and_drain_test_window(cx);
    assert_ne!(
        anchor(cx),
        before_focused,
        "shift-f3 should navigate while the editor has focus"
    );
    assert_ne!(
        anchor(cx),
        Some(middle_order),
        "the focused-editor path must filter by resolution state as well"
    );

    fixture.cleanup();
}

/// The last conflict standing must still be reachable from itself.
///
/// Once everything else is decided there is nothing strictly past the anchor in
/// either direction, so both chords went dead and both toolbar arrows greyed
/// out — at exactly the point where the user has scrolled off somewhere else and
/// wants the one remaining decision back on screen.
#[gpui::test]
fn shift_f2_and_f3_still_reach_the_last_unresolved_conflict(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(992);
    let fixture =
        SyntheticLargeConflictFixture::new("last_open_nav", "fixtures/last_open_nav.html", 400, 6);
    fixture.write();
    seed_unresolved_conflict_state(
        cx,
        &view,
        repo_id,
        &fixture.workdir,
        &fixture.file_rel,
        &fixture.base_text,
        &fixture.ours_text,
        &fixture.theirs_text,
        &fixture.current_text,
    );
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "last-open nav fixture initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver
                .nav_targets
                .iter()
                .filter(|target| target.unresolved)
                .count()
                >= 3
        },
        |pane| format!("targets={}", pane.conflict_resolver.nav_targets.len()),
    );
    draw_and_drain_test_window(cx);

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    let open_display_indices = |cx: &mut gpui::VisualTestContext| -> Vec<usize> {
        cx.update(|_window, app| {
            main_pane
                .read(app)
                .conflict_resolver
                .nav_targets
                .iter()
                .filter(|target| target.unresolved)
                .filter_map(|target| target.display_conflict_index)
                .collect()
        })
    };

    // Resolve every conflict but the last, the way a user does, so the one left
    // is genuinely the only unresolved target.
    let keep_open = *open_display_indices(cx).last().expect("an open conflict");
    while let Some(display) = open_display_indices(cx)
        .into_iter()
        .find(|display| *display != keep_open)
    {
        cx.update(|_window, app| {
            main_pane.update(app, |pane, cx| {
                pane.conflict_resolver_select_conflict(display, cx);
                pane.conflict_resolver_pick_active_conflict(
                    crate::view::conflict_resolver::ConflictChoice::Ours,
                    cx,
                );
            });
        });
        draw_and_drain_test_window(cx);
    }
    assert_eq!(
        open_display_indices(cx),
        vec![keep_open],
        "exactly one conflict must be left open for this test to mean anything"
    );

    let sole_target = cx.update(|_window, app| {
        main_pane
            .read(app)
            .conflict_resolver
            .nav_targets
            .iter()
            .position(|target| target.unresolved)
            .expect("the remaining open conflict has a nav target")
    });
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_jump_to_nav_target(sole_target, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let anchor = |cx: &mut gpui::VisualTestContext| -> Option<usize> {
        cx.update(|_window, app| {
            main_pane
                .read(app)
                .conflict_resolver
                .nav_anchor
                .map(|anchor| anchor.order_hint)
        })
    };
    let press = |cx: &mut gpui::VisualTestContext, chord: &str| -> bool {
        let keystroke = gpui::Keystroke::parse(chord).expect("valid chord");
        cx.update(|window, app| {
            main_pane.update(app, |pane, cx| {
                pane.handle_diff_shortcut(&keystroke, window, cx)
            })
        })
    };

    let sole_order = cx.update(|_window, app| {
        main_pane.read(app).conflict_resolver.nav_targets[sole_target].order
    });
    assert_eq!(anchor(cx), Some(sole_order));

    // The toolbar reads the same predicates the chords do, so both arrows have
    // to come back with them.
    cx.update(|_window, app| {
        let pane = main_pane.read(app);
        assert!(
            pane.conflict_has_next_unresolved(),
            "next-unresolved must be offered while a conflict is still open"
        );
        assert!(
            pane.conflict_has_prev_unresolved(),
            "previous-unresolved must be offered too"
        );
    });

    for chord in ["shift-f3", "shift-f2"] {
        assert!(press(cx, chord), "{chord} should be handled");
        draw_and_drain_test_window(cx);
        assert_eq!(
            anchor(cx),
            Some(sole_order),
            "{chord} must keep the last open conflict selected rather than going dead"
        );
    }

    fixture.cleanup();
}

/// *Reset conflict markers* has to outlive the round-trip it starts.
///
/// The button clears protection and then dispatches, and the resync that comes
/// back re-derived protection from the same unchanged worktree payload — so the
/// flag went straight back on and every pick and Unresolve greyed out again.
/// From the user's side the button did nothing.
///
/// The payload here is one git left conflicted but that no longer carries
/// markers, which is what an editor-side resolution looks like: protection is
/// right to fire on it, and the reset is the user overriding that.
#[gpui::test]
fn resetting_the_markers_survives_the_resync_it_triggers(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(996);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_reset_sticks",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/reset_sticks.txt");
    let base = "head\nB\ntail\n";
    let ours = "head\nB1\ntail\n";
    let theirs = "head\nB2\ntail\n";
    let resolved_by_hand = "head\nB1\ntail\n";

    seed_unresolved_conflict_state(
        cx,
        &view,
        repo_id,
        &workdir,
        &file_rel,
        base,
        ours,
        theirs,
        resolved_by_hand,
    );
    draw_and_drain_test_window(cx);
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    let protected = |cx: &mut gpui::VisualTestContext| -> bool {
        cx.update(|_window, app| main_pane.read(app).conflict_resolver.output_is_protected)
    };

    assert!(
        protected(cx),
        "a payload with no conflict block left reads as resolved by hand"
    );

    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_resolver_reset_output_from_markers(cx);
        });
    });
    draw_and_drain_test_window(cx);
    assert!(!protected(cx), "the reset must clear protection");

    // Now resolve something. That dispatches, which bumps `conflict_rev`, which
    // is what drives the resync — and the resync re-derives protection from the
    // same unchanged worktree payload that still reads as hand-resolved. Only
    // the waiver keeps it off.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_resolver_select_conflict(0, cx);
            pane.conflict_resolver_pick_active_conflict(
                crate::view::conflict_resolver::ConflictChoice::Ours,
                cx,
            );
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        !protected(cx),
        "protection came back after the first pick, so the reset did nothing"
    );
    assert!(
        cx.update(|_window, app| main_pane
            .read(app)
            .conflict_resolver_active_pick_state()
            .is_some()),
        "the pick controls must be usable after the reset"
    );
}

/// The resolver's rows are shaped from `window.rem_size()`, which UI scale moves, so
/// the row boxes have to move with it. A flat 20px row holds a ~31px line box at 150%
/// and the text spills into the row below -- the "lines break" symptom.
#[gpui::test]
fn conflict_resolver_row_geometry_follows_ui_scale(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(191);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_ui_scale",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_resolver_ui_scale.txt");
    let abs_path = workdir.join(&file_rel);

    // Enough lines that the lists virtualize and the output gutter has room to drift.
    let context = (0..40)
        .map(|ix| format!("context line {ix}"))
        .collect::<Vec<_>>();
    let base_text = context.join("\n");
    let ours_text = context
        .iter()
        .enumerate()
        .map(|(ix, line)| {
            if ix == 20 {
                "ours change".to_string()
            } else {
                line.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let theirs_text = context
        .iter()
        .enumerate()
        .map(|(ix, line)| {
            if ix == 20 {
                "theirs change".to_string()
            } else {
                line.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let head = context[..20].join("\n");
    let tail = context[21..].join("\n");
    let current_text = format!(
        "{head}\n<<<<<<< ours\nours change\n=======\ntheirs change\n>>>>>>> theirs\n{tail}\n"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver ui-scale fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver ui-scale fixture");

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
        "resolver ui-scale fixture initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.three_way_visible_len() >= 40
        },
        |pane| {
            format!(
                "path={:?} three_way_visible={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.three_way_visible_len(),
            )
        },
    );

    cx.simulate_resize(gpui::size(px(1280.0), px(720.0)));
    draw_and_drain_test_window(cx);

    // Source rows have a canvas path and a div path, and which one runs is an env
    // toggle -- pin it rather than inheriting the ambient default.
    let set_canvas_rows = |cx: &mut gpui::VisualTestContext, enabled: bool| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.conflict_canvas_rows_enabled = enabled;
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);
    };

    /// Total laid-out height of every row in a virtualized list. `item` is the
    /// viewport, `contents` is the full row stack, so this is `row_height * rows`.
    fn measured_content_height(handle: &gpui::UniformListScrollHandle, label: &str) -> f32 {
        handle
            .0
            .borrow()
            .last_item_size
            .unwrap_or_else(|| panic!("expected rendered item size for {label}"))
            .contents
            .height
            .into()
    }

    struct Sample {
        base_contents: f32,
        gutter_contents: f32,
        gutter_rows: usize,
        gutter_row_height: Pixels,
        editor_line_height: Pixels,
    }

    let sample = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            Sample {
                base_contents: measured_content_height(
                    &pane.conflict_resolver_diff_scroll,
                    "three-way base column",
                ),
                gutter_contents: measured_content_height(
                    &pane.conflict_resolved_preview_gutter_scroll,
                    "resolved output gutter",
                ),
                gutter_rows: pane.resolved_output_visible_len(),
                gutter_row_height: pane.conflict_resolved_gutter_row_height,
                editor_line_height: pane
                    .conflict_resolver_input
                    .read(app)
                    .line_height_override()
                    .expect("resolved output editor should carry an explicit line height"),
            }
        })
    };

    for canvas_rows in [true, false] {
        let path = if canvas_rows { "canvas" } else { "div" };
        set_canvas_rows(cx, canvas_rows);
        set_ui_scale_percent_for_test(cx, &view, 100);
        draw_and_drain_test_window(cx);
        let at_100 = sample(cx);

        set_ui_scale_percent_for_test(cx, &view, 200);
        draw_and_drain_test_window(cx);
        let at_200 = sample(cx);

        let base_ratio = at_200.base_contents / at_100.base_contents;
        assert!(
            (base_ratio - 2.0).abs() < 0.05,
            "{path} source column rows should double at 200% (100%={} 200%={})",
            at_100.base_contents,
            at_200.base_contents,
        );
    }

    set_canvas_rows(cx, true);
    set_ui_scale_percent_for_test(cx, &view, 100);
    draw_and_drain_test_window(cx);

    let at_100 = sample(cx);

    // The gutter list and the editable buffer it labels must advance at the same
    // rate, or the numbers walk off their lines as you scroll down the file.
    assert_eq!(
        at_100.gutter_row_height, at_100.editor_line_height,
        "at 100% the output gutter row height must match the editor line height"
    );
    // ...and the height the render pass recorded has to be the one the list really
    // laid out, since navigation centres the editor on a row using it.
    assert!(at_100.gutter_rows > 0);
    let laid_out_100 = at_100.gutter_contents / at_100.gutter_rows as f32;
    assert!(
        (laid_out_100 - f32::from(at_100.gutter_row_height)).abs() < 0.01,
        "recorded gutter row height {:?} disagrees with the {laid_out_100} the list laid out",
        at_100.gutter_row_height,
    );

    set_ui_scale_percent_for_test(cx, &view, 200);
    draw_and_drain_test_window(cx);

    let at_200 = sample(cx);

    assert_eq!(
        at_200.gutter_row_height, at_200.editor_line_height,
        "at 200% the output gutter row height must match the editor line height \
         (gutter={:?} editor={:?})",
        at_200.gutter_row_height, at_200.editor_line_height,
    );
    assert_eq!(at_200.gutter_rows, at_100.gutter_rows);
    let laid_out_200 = at_200.gutter_contents / at_200.gutter_rows as f32;
    assert!(
        (laid_out_200 - f32::from(at_200.gutter_row_height)).abs() < 0.01,
        "recorded gutter row height {:?} disagrees with the {laid_out_200} the list laid out",
        at_200.gutter_row_height,
    );

    let base_ratio = at_200.base_contents / at_100.base_contents;
    assert!(
        (base_ratio - 2.0).abs() < 0.05,
        "source column rows should double at 200% (100%={} 200%={})",
        at_100.base_contents,
        at_200.base_contents,
    );
    let gutter_ratio = at_200.gutter_contents / at_100.gutter_contents;
    assert!(
        (gutter_ratio - 2.0).abs() < 0.05,
        "output gutter rows should double at 200% (100%={} 200%={})",
        at_100.gutter_contents,
        at_200.gutter_contents,
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup resolver ui-scale fixture");
}

/// Ctrl+F in the merge tool must bring the hit into view — in the input
/// columns *and* in the resolved output.
///
/// The scroll dispatch (`diff_search_scroll_to_visible_ix`) hands conflict
/// targets to `conflict_resolver_scroll_all_columns`, which knows only the
/// three column lists. The resolved output rides handles of its own, so a
/// match far down the file left it parked at the top.
fn assert_conflict_search_reveals_match(
    cx: &mut gpui::TestAppContext,
    view_mode: ConflictResolverViewMode,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{fixture_name}",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from(format!("fixtures/{fixture_name}.txt"));
    let abs_path = workdir.join(&file_rel);
    let base_text = build_conflict_scroll_matrix_text("base", 'B');
    let ours_text = build_conflict_scroll_matrix_text("ours", 'O');
    let theirs_text = build_conflict_scroll_matrix_text("theirs", 'T');
    let current_text = build_conflict_scroll_matrix_current_text(&ours_text, &theirs_text);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver search fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver search fixture");

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
        "resolver search fixture initialized",
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
                pane.conflict_resolver_set_view_mode(view_mode, cx);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolver search vertical overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.view_mode == view_mode
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).height > px(120.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).height
                    > px(120.0)
        },
        |pane| {
            format!(
                "view_mode={:?} left_max={:?} output_max={:?}",
                pane.conflict_resolver.view_mode,
                uniform_list_max_offset(&pane.conflict_resolver_diff_scroll),
                scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                reset_conflict_scroll_matrix_offsets(pane);
                pane.diff_search_active = true;
                pane.diff_search_query = "line 150".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("line 150", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.diff_search_matches.is_empty(),
            "expected the merge tool search to find `line 150`"
        );

        // The two-way view renders only left/right, so `ours` stays untracked
        // there and must not be asserted on.
        let mut columns = vec![
            (
                "left/base",
                uniform_list_offset(&pane.conflict_resolver_diff_scroll).y,
            ),
            (
                "right/theirs",
                uniform_list_offset(&pane.conflict_preview_theirs_scroll).y,
            ),
        ];
        if view_mode == ConflictResolverViewMode::ThreeWay {
            columns.push((
                "ours",
                uniform_list_offset(&pane.conflict_preview_ours_scroll).y,
            ));
        }
        for (label, offset) in &columns {
            assert!(
                *offset < px(0.0),
                "expected the {label} column to scroll to the match, got {offset:?} \
                 (columns={columns:?} matches={:?} current={:?})",
                pane.diff_search_matches,
                pane.diff_search_match_ix,
            );
        }

        // Proves the output moved because search resolved the hit's row to an
        // output line, not because a scroll-sync pass happened to drag it.
        let current_row = pane
            .diff_search_current_match_row()
            .expect("expected a current search match row");
        let output_line = pane
            .conflict_resolver
            .output_line_for_visible_row(current_row)
            .expect("expected the matched column row to map to an output line");
        assert!(
            output_line > 100,
            "expected the mapped output line to be far down the file, got {output_line}"
        );

        let output_y = scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll).y;
        let gutter_y = uniform_list_offset(&pane.conflict_resolved_preview_gutter_scroll).y;
        assert!(
            output_y < px(0.0) && gutter_y < px(0.0),
            "expected the resolved output and its gutter to scroll to the match, got \
             output={output_y:?} gutter={gutter_y:?} matches={:?} current={:?}",
            pane.diff_search_matches,
            pane.diff_search_match_ix,
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn conflict_resolver_three_way_search_reveals_match_in_columns_and_output(
    cx: &mut gpui::TestAppContext,
) {
    assert_conflict_search_reveals_match(
        cx,
        ConflictResolverViewMode::ThreeWay,
        gitcomet_state::model::RepoId(1631),
        "resolver_search_reveal_three_way",
    );
}

#[gpui::test]
fn conflict_resolver_two_way_search_reveals_match_in_columns_and_output(
    cx: &mut gpui::TestAppContext,
) {
    assert_conflict_search_reveals_match(
        cx,
        ConflictResolverViewMode::TwoWayDiff,
        gitcomet_state::model::RepoId(1632),
        "resolver_search_reveal_two_way",
    );
}

/// The three-way merge tool columns had no search wash at all — only the
/// two-way split columns built a query overlay — so a Ctrl+F hit scrolled into
/// view with nothing marking it.
///
/// Also pins the mechanism that keeps the *current* match distinguishable: its
/// row is built per frame with `DiffSearchMatchEmphasis::Current` and
/// deliberately kept out of the cache, so the wash follows the search cursor
/// instead of being left behind on the row it stepped off.
#[gpui::test]
fn conflict_resolver_three_way_columns_paint_the_search_wash(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(1633);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_search_wash",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/resolver_search_wash.txt");
    let abs_path = workdir.join(&file_rel);
    let base_text = build_conflict_scroll_matrix_text("base", 'B');
    let ours_text = build_conflict_scroll_matrix_text("ours", 'O');
    let theirs_text = build_conflict_scroll_matrix_text("theirs", 'T');
    let current_text = build_conflict_scroll_matrix_current_text(&ours_text, &theirs_text);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver search wash fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver search wash fixture");

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
        "resolver search wash fixture initialized",
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| format!("path={:?}", pane.conflict_resolver.path.clone()),
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

    // `line 00` matches the first ten rows, so the top of the file holds both a
    // current match and several others without any scrolling.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = "line 00".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("line 00", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_search_matches.len() > 1,
            "expected several matches, got {:?}",
            pane.diff_search_matches
        );
        let current_row = pane
            .diff_search_current_match_row()
            .expect("expected a current search match row");
        let other_row = pane
            .diff_search_matches
            .iter()
            .copied()
            .find(|row| *row != current_row)
            .expect("expected a non-current match row");

        let side_line = |row: usize| {
            pane.conflict_resolver
                .three_way_side_line_for_row(ThreeWayColumn::Ours, row)
                .expect("expected the matched row to have an ours-side line")
        };

        let other = pane
            .conflict_three_way_query_segments_cache
            .get(&(side_line(other_row), ThreeWayColumn::Ours))
            .unwrap_or_else(|| {
                panic!(
                    "expected a search overlay for the non-current match row {other_row}, \
                     cache holds {} entries",
                    pane.conflict_three_way_query_segments_cache.len()
                )
            });
        assert!(
            !other.highlights.is_empty(),
            "expected the non-current match row to carry highlight ranges"
        );

        assert!(
            !pane
                .conflict_three_way_query_segments_cache
                .contains_key(&(side_line(current_row), ThreeWayColumn::Ours)),
            "expected the current match row {current_row} to be built per frame, not cached"
        );
    });

    // A new query throws the old wash away rather than leaving stale ranges
    // behind: entries are only ever inserted for rows the *current* query
    // matches, so a surviving `line 00` row would mean the cache was not cleared.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = "line 09".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("line 09", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.conflict_three_way_query_segments_cache.is_empty(),
            "expected the new query to paint its own wash"
        );
        for ((line, column), styled) in &pane.conflict_three_way_query_segments_cache {
            assert!(
                styled.text.contains("line 09"),
                "stale wash left over from the previous query on {column:?} line {line}: {:?}",
                styled.text
            );
        }
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The merge tool columns scroll sideways to a match too.
///
/// They are their own canvases and register nothing in the diff hitbox map, so
/// they record where they painted their text separately; the columns share a
/// horizontal scroll sync, so moving the one that matched carries the rest.
fn assert_conflict_search_scrolls_sideways(
    cx: &mut gpui::TestAppContext,
    view_mode: ConflictResolverViewMode,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    reveal_whitespace: bool,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{fixture_name}",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from(format!("fixtures/{fixture_name}.txt"));
    let abs_path = workdir.join(&file_rel);
    let base_text = build_conflict_scroll_matrix_text("base", 'B');
    // Only `ours` carries the needle, and it sits past the fold on a long line.
    let ours_text = format!(
        "{}\n{}needle tail",
        build_conflict_scroll_matrix_text("ours", 'O'),
        "pad ".repeat(200)
    );
    let theirs_text = build_conflict_scroll_matrix_text("theirs", 'T');
    let current_text = build_conflict_scroll_matrix_current_text(&ours_text, &theirs_text);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver hscroll fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver hscroll fixture");

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
        "resolver hscroll fixture initialized",
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| format!("path={:?}", pane.conflict_resolver.path.clone()),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(view_mode, cx);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    // Two-way reuses the three-way handles for a two-column layout: its left
    // (Ours) list is tracked by `conflict_resolver_diff_scroll`, and the ours
    // handle is never laid out there.
    fn ours_list(
        pane: &MainPaneView,
        view_mode: ConflictResolverViewMode,
    ) -> &gpui::UniformListScrollHandle {
        match view_mode {
            ConflictResolverViewMode::ThreeWay => &pane.conflict_preview_ours_scroll,
            ConflictResolverViewMode::TwoWayDiff => &pane.conflict_resolver_diff_scroll,
        }
    }

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolver hscroll horizontal overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.view_mode == view_mode
                && uniform_list_max_offset(ours_list(pane, view_mode)).width > px(120.0)
        },
        |pane| {
            format!(
                "view_mode={:?} ours_max={:?}",
                pane.conflict_resolver.view_mode,
                uniform_list_max_offset(ours_list(pane, view_mode)),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                reset_conflict_scroll_matrix_offsets(pane);
                pane.reveal_whitespace_chars = reveal_whitespace;
                pane.diff_search_active = true;
                // The space is the point of the whitespace variant: with
                // whitespace revealed every space in the painted text is `·`,
                // so the query has to be looked for in its painted form.
                pane.diff_search_query = "needle tail".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("needle tail", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });
    // The vertical jump lands and the row paints, then the sideways reveal reads
    // what that paint recorded.
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.diff_search_matches.is_empty(),
            "expected the merge tool to find the needle"
        );
        assert!(
            uniform_list_offset(ours_list(pane, view_mode)).x < px(0.0),
            "expected the ours column to scroll right to the match in {view_mode:?}, \
             x stayed at {:?}",
            uniform_list_offset(ours_list(pane, view_mode)),
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn conflict_resolver_three_way_search_scrolls_columns_sideways_to_a_match(
    cx: &mut gpui::TestAppContext,
) {
    assert_conflict_search_scrolls_sideways(
        cx,
        ConflictResolverViewMode::ThreeWay,
        gitcomet_state::model::RepoId(1634),
        "resolver_search_hscroll_three_way",
        false,
    );
}

/// Two-way renders Ours in the list tracked by `conflict_resolver_diff_scroll`,
/// not by `conflict_preview_ours_scroll` — writing to the latter scrolls a
/// handle that mode never lays out.
#[gpui::test]
fn conflict_resolver_two_way_search_scrolls_columns_sideways_to_a_match(
    cx: &mut gpui::TestAppContext,
) {
    assert_conflict_search_scrolls_sideways(
        cx,
        ConflictResolverViewMode::TwoWayDiff,
        gitcomet_state::model::RepoId(1635),
        "resolver_search_hscroll_two_way",
        false,
    );
}

/// With whitespace revealed, the merge tool paints `·` for every space, so a
/// query containing one is only findable again in its painted form — otherwise
/// the sideways reveal silently gives up.
#[gpui::test]
fn conflict_resolver_search_scrolls_sideways_with_whitespace_revealed(
    cx: &mut gpui::TestAppContext,
) {
    assert_conflict_search_scrolls_sideways(
        cx,
        ConflictResolverViewMode::ThreeWay,
        gitcomet_state::model::RepoId(1636),
        "resolver_search_hscroll_ws",
        true,
    );
}
