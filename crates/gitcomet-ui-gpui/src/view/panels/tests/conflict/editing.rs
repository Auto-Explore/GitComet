//! Editing resolved output: edits, staging, splits and joins, view-mode persistence.

use super::*;

/// The input columns stream a whole-file conflict, but the resolved output is
/// editable at any size — the two gates are independent. `StreamedLargeFile`
/// describes how the A/B/C columns render; it never demotes the output pane to
/// a read-only projection.
#[gpui::test]
fn whole_file_conflict_bootstrap_streams_input_but_keeps_output_editable(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(169);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "whole_file_conflict_streamed",
        "fixtures/whole_file_conflict.html",
        crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1_000,
    );
    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "whole-file conflict streamed bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ) == 1
                && pane.conflict_resolver.rendering_mode()
                    == crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile
                && pane.conflict_resolver.split_row_index().is_some()
                && pane.conflict_resolver.two_way_split_projection().is_some()
                && pane.conflict_resolved_output_projection.is_none()
        },
        |pane| {
            format!(
                "path={:?} conflicts={} rendering_mode={:?} split_row_index={} projection={} output_projection={} three_way_len={}",
                pane.conflict_resolver.path.clone(),
                crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ),
                pane.conflict_resolver.rendering_mode(),
                pane.conflict_resolver.split_row_index().is_some(),
                pane.conflict_resolver.two_way_split_projection().is_some(),
                pane.conflict_resolved_output_projection.is_some(),
                pane.conflict_resolver.three_way_len,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            this.main_pane.update(_cx, |pane, _cx| {
                assert_streamed_whole_file_two_way_state(pane, fixture.line_count);
                assert!(
                    pane.conflict_resolved_output_projection.is_none(),
                    "whole-file bootstrap should materialize the resolved output, not stream it",
                );
                assert!(
                    !pane.conflict_resolved_output_is_streamed(),
                    "a materialized output must report itself editable so the edit \
                     affordances gated on this are enabled",
                );
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let expected = crate::view::conflict_resolver::generate_resolved_text(
            &pane.conflict_resolver.marker_segments,
        );
        assert_eq!(
            pane.conflict_resolver_input.read(app).text(),
            expected.as_str(),
            "the editable buffer should hold the full merged text of a whole-file conflict",
        );
    });

    fixture.cleanup();
}

/// There is no line-count ceiling on editing the resolved output.
///
/// An unresolved whole-file conflict collapses to a one-line placeholder, so the
/// size only materializes once a side is picked — which is precisely what the
/// old upper-bound guard refused to do. Pick a side to expand the output past
/// the old limit, then type into it. If a size gate is reintroduced anywhere on
/// the materialize path, the expanded output stays read-only and this fails.
#[gpui::test]
fn a_resolved_output_past_the_old_editable_ceiling_still_accepts_edits(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(173);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "whole_file_conflict_editable_no_ceiling",
        "fixtures/whole_file_conflict_editable.html",
        crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1_000,
    );
    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large resolved output materialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && !pane.conflict_resolved_output_is_streamed()
                && pane.conflict_resolved_preview_line_count > 1
        },
        |pane| {
            format!(
                "path={:?} streamed={} preview_lines={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_output_is_streamed(),
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());

    // Take "ours" for the single whole-file conflict. This is the step the old
    // ceiling refused: it expands a one-line placeholder into the full file.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_resolver_pick_at(
                0,
                crate::view::conflict_resolver::ConflictChoice::Ours,
                cx,
            );
        });
    });
    cx.run_until_parked();

    let before = cx.update(|_window, app| {
        main_pane
            .read(app)
            .conflict_resolver_input
            .read(app)
            .text()
            .to_string()
    });
    assert!(
        before.lines().count()
            > crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES,
        "picking a side should expand the output past the old ceiling, got {} lines",
        before.lines().count()
    );
    assert!(
        cx.update(|_window, app| !main_pane.read(app).conflict_resolved_output_is_streamed()),
        "an output expanded past the old ceiling must stay editable, not fall back to streamed",
    );

    // Append at the very end, which is never inside a protected marker range.
    let at = before.len();
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_resolver_input.update(cx, |input, cx| {
                input.replace_utf8_range(at..at, "edited", cx);
            });
        });
    });
    cx.run_until_parked();

    let after = cx.update(|_window, app| {
        main_pane
            .read(app)
            .conflict_resolver_input
            .read(app)
            .text()
            .to_string()
    });
    assert_eq!(
        after,
        format!("{before}edited"),
        "a keystroke in a large resolved output should land and persist"
    );

    fixture.cleanup();
}

/// Stage-anyway on a whole-file conflict must serialize the merged text the user
/// is actually looking at. The output is materialized at this size now, so this
/// guards the buffer-backed save path rather than the projection one.
#[gpui::test]
fn whole_file_conflict_stage_anyway_serializes_the_materialized_output(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(172);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "whole_file_conflict_stage_anyway_streamed",
        "fixtures/whole_file_conflict_stage_anyway.html",
        crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1_000,
    );
    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "whole-file conflict streamed stage-anyway bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.rendering_mode()
                    == crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile
                && pane.conflict_resolved_output_projection.is_none()
        },
        |pane| {
            format!(
                "path={:?} rendering_mode={:?} output_projection={} preview_lines={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.rendering_mode(),
                pane.conflict_resolved_output_projection.is_some(),
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    let (expected, actual, input_before, input_after, projection_after) =
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    let expected = crate::view::conflict_resolver::generate_resolved_text(
                        &pane.conflict_resolver.marker_segments,
                    );
                    let input_before = pane.conflict_resolver_input.read(cx).text().to_string();
                    // Mirrors the production save path in conflict_resolver_view.
                    let output_text = pane.current_conflict_resolved_output_text(cx);
                    let actual = pane.conflict_resolver_save_contents_from_text(output_text);
                    let input_after = pane.conflict_resolver_input.read(cx).text().to_string();
                    (
                        expected,
                        actual,
                        input_before,
                        input_after,
                        pane.conflict_resolved_output_projection.is_some(),
                    )
                })
            })
        });

    assert_eq!(
        input_before, expected,
        "a whole-file conflict should already hold its merged text in the editable buffer"
    );
    assert_eq!(
        actual, expected,
        "stage confirmation should serialize the resolved output the user is editing"
    );
    assert!(
        !actual.is_empty(),
        "stage-confirm contents should contain the resolved output text"
    );
    assert_eq!(
        input_after, input_before,
        "stage confirmation should read the editor buffer, not rewrite it"
    );
    assert!(
        !projection_after,
        "stage confirmation should not push the output back into projection mode"
    );

    fixture.cleanup();
}

#[gpui::test]
fn structured_conflict_edit_reuses_stashed_outline_base_while_background_recompute_is_pending(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(168);
    let fixture = SyntheticLargeConflictFixture::new(
        "resolved_outline_pending_incremental",
        "fixtures/resolved_outline_pending.html",
        20_000,
        4,
    );
    fixture.write();

    let expected_resolved_line_count = crate::view::conflict_resolver::generate_resolved_text(
        crate::view::conflict_resolver::parse_conflict_markers(&fixture.current_text).as_slice(),
    )
    .split('\n')
    .count();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = app_state_with_repo(fixture.repo_state(repo_id), repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolved outline pending incremental initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel),
        |pane| {
            format!(
                "path={:?} preview_lines={} meta={} markers={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolver.resolved_outline.meta.len(),
                pane.conflict_resolver.resolved_outline.markers.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_conflict_resolved_output_materialized(cx);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolved outline pending incremental materialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolved_output_projection.is_none()
                && pane.conflict_resolved_preview_line_count == expected_resolved_line_count
        },
        |pane| {
            format!(
                "path={:?} projection_present={} preview_lines={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_output_projection.is_some(),
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.recompute_conflict_resolved_outline_for_tests(cx);
                pane.conflict_resolver.resolver_pending_recompute_seq = pane
                    .conflict_resolver
                    .resolver_pending_recompute_seq
                    .wrapping_add(1);
                pane.set_conflict_resolved_outline_background_delay_override_for_tests(
                    std::time::Duration::from_millis(1_000),
                );
                assert_eq!(
                    pane.conflict_resolver.resolved_outline.meta.len(),
                    expected_resolved_line_count,
                    "forced outline recompute should seed current metadata before the pending fallback test starts",
                );
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = fixture.repo_state(repo_id);
            repo.conflict_state.conflict_hide_resolved = true;
            repo.conflict_state.conflict_rev = repo.conflict_state.conflict_rev.wrapping_add(1);

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolved outline state sync clears visible metadata while delayed background recompute is pending",
        std::time::Duration::from_millis(500),
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolved_preview_line_count == expected_resolved_line_count
                && pane.conflict_resolver.resolved_outline.meta.is_empty()
                && pane.conflict_resolver.resolved_outline.markers.is_empty()
        },
        |pane| {
            format!(
                "hide_resolved={} preview_lines={} meta={} markers={} stash={} pending_seq={}",
                pane.conflict_resolver.hide_resolved,
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolver.resolved_outline.meta.len(),
                pane.conflict_resolver.resolved_outline.markers.len(),
                pane.conflict_resolved_outline_stash.is_some(),
                pane.conflict_resolver.resolver_pending_recompute_seq,
            )
        },
    );

    cx.update(|_window, app| {
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
                    .expect("fixture should contain at least one conflict block");
                first_block.choice = crate::view::conflict_resolver::ConflictChoice::Theirs;
                first_block.resolved = true;

                let resolved = crate::view::conflict_resolver::generate_resolved_text(
                    &pane.conflict_resolver.marker_segments,
                );
                pane.conflict_resolver_set_output(resolved, cx);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "structured edit incrementally restores outline metadata from stashed base before delayed background fallback completes",
        std::time::Duration::from_millis(500),
        |pane| {
            pane.conflict_resolver.resolved_outline.meta.len() == expected_resolved_line_count
                && pane.conflict_resolver.resolved_outline.markers.len()
                    == expected_resolved_line_count
                && pane
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .iter()
                    .flatten()
                    .any(|marker| marker.conflict_ix == 0 && !marker.unresolved)
                && pane
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .iter()
                    .flatten()
                    .any(|marker| marker.conflict_ix == 1 && marker.unresolved)
        },
        |pane| {
            let first_markers: Vec<(usize, bool, bool)> = pane
                .conflict_resolver
                .resolved_outline
                .markers
                .iter()
                .flatten()
                .take(8)
                .map(|marker| (marker.conflict_ix, marker.unresolved, marker.is_start))
                .collect();
            format!(
                "meta={} markers={} stash={} first_markers={first_markers:?} preview_revision={:?}",
                pane.conflict_resolver.resolved_outline.meta.len(),
                pane.conflict_resolver.resolved_outline.markers.len(),
                pane.conflict_resolved_outline_stash.is_some(),
                pane.conflict_resolved_preview_source_revision,
            )
        },
    );

    fixture.cleanup();
}

#[gpui::test]
fn giant_two_way_resync_rebuilds_split_index_after_manual_session_edit(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(172);
    let fixture = SyntheticLargeConflictFixture::new(
        "giant_two_way_resync_manual_edit",
        "fixtures/resync_manual_edit.html",
        20_001,
        4,
    );
    fixture.write();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

            let next_state = app_state_with_repo(fixture.repo_state(repo_id), repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "giant two-way resync bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} split_row_index={} conflict_rev={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.split_row_index().is_some(),
                pane.conflict_resolver.conflict_rev,
            )
        },
    );

    let initial_visible_len = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                pane.conflict_resolver.two_way_split_visible_len()
            })
        })
    });

    let manual_text = "<article id=\"manual-0\">manual block 0</article>\n<article id=\"manual-1\">manual block 1</article>\n";
    let (
        updated_repo,
        expected_rev,
        expected_conflict_count,
        expected_total_rows,
        expected_visible_len,
    ) = {
        let mut repo = fixture.repo_state(repo_id);
        let session = repo
            .conflict_state
            .conflict_session
            .as_mut()
            .expect("fixture should include a text conflict session");
        session.regions[0].resolution =
            gitcomet_core::conflict_session::ConflictRegionResolution::ManualEdit(
                manual_text.to_string(),
            );

        let mut expected_segments =
            crate::view::conflict_resolver::parse_conflict_markers(&fixture.current_text);
        crate::view::conflict_resolver::apply_session_region_resolutions_with_index_map(
            &mut expected_segments,
            &session.regions,
        );
        let expected_conflict_count =
            crate::view::conflict_resolver::conflict_count(&expected_segments);
        let expected_index = crate::view::conflict_resolver::ConflictSplitRowIndex::new(
            &expected_segments,
            crate::view::conflict_resolver::BLOCK_LOCAL_DIFF_CONTEXT_LINES,
        );
        let expected_projection = crate::view::conflict_resolver::TwoWaySplitProjection::new(
            &expected_index,
            &expected_segments,
            false,
        );
        repo.conflict_state.conflict_rev = repo.conflict_state.conflict_rev.wrapping_add(1);
        let expected_rev = repo.conflict_state.conflict_rev;
        (
            repo,
            expected_rev,
            expected_conflict_count,
            expected_index.total_rows(),
            expected_projection.visible_len(),
        )
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = app_state_with_repo(updated_repo.clone(), repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "giant two-way resync applied manual session edit",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.conflict_rev == expected_rev
                && crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ) == expected_conflict_count
        },
        |pane| {
            format!(
                "path={:?} conflict_rev={} conflicts={} visible_len={} split_rows={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.conflict_rev,
                crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ),
                pane.conflict_resolver.two_way_split_visible_len(),
                pane.conflict_resolver
                    .split_row_index()
                    .map(|index| index.total_rows())
                    .unwrap_or_default(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);

                assert_eq!(
                    pane.conflict_resolver.rendering_mode(),
                    crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile,
                    "large fixture should remain in streamed large-file mode after re-sync",
                );
                assert_eq!(
                    crate::view::conflict_resolver::conflict_count(
                        &pane.conflict_resolver.marker_segments,
                    ),
                    expected_conflict_count,
                    "manual session edit should materialize one conflict block into text during re-sync",
                );
                assert_eq!(
                    pane.conflict_resolver.conflict_region_indices.len(),
                    expected_conflict_count,
                    "visible region indices should shrink with the remaining conflict blocks",
                );

                let index = pane
                    .conflict_resolver
                    .split_row_index()
                    .expect("re-sync should rebuild the giant split row index");
                assert_eq!(
                    index.total_rows(),
                    expected_total_rows,
                    "split row index should be rebuilt from the updated marker structure",
                );
                assert_eq!(
                    pane.conflict_resolver.two_way_split_visible_len(),
                    expected_visible_len,
                    "two-way projection should reflect the rebuilt split index",
                );
                assert_ne!(
                    pane.conflict_resolver.two_way_split_visible_len(),
                    initial_visible_len,
                    "manual materialization should change the visible giant split layout",
                );

                assert!(
                    index.first_row_for_conflict(expected_conflict_count).is_none(),
                    "rebuilt split index should drop the removed conflict block entirely",
                );
                let first_conflict_row_ix = index
                    .first_row_for_conflict(0)
                    .expect("remaining first conflict should still have rows after re-sync");
                let first_conflict_row = index
                    .row_at(
                        &pane.conflict_resolver.marker_segments,
                        first_conflict_row_ix,
                    )
                    .expect("remaining first conflict row should be generatable after re-sync");
                let row_has_shifted_conflict = first_conflict_row
                    .old
                    .as_deref()
                    .is_some_and(|text| text.contains("choice-1"))
                    || first_conflict_row
                        .new
                        .as_deref()
                        .is_some_and(|text| text.contains("choice-1"));
                assert!(
                    row_has_shifted_conflict,
                    "re-synced first remaining conflict row should now point at the old second block",
                );
                let first_conflict_visible_ix = pane
                    .conflict_resolver
                    .two_way_split_projection()
                    .and_then(|projection| projection.source_to_visible(first_conflict_row_ix));
                assert!(
                    first_conflict_visible_ix
                        .and_then(|visible_ix| {
                            pane.conflict_resolver.two_way_split_visible_row(visible_ix)
                        })
                        .is_some(),
                    "rebuilt projection should resolve the shifted first-conflict row as visible",
                );
            });
        });
    });

    fixture.cleanup();
}

#[gpui::test]
fn large_conflict_resolved_output_above_the_old_line_gate_is_highlighted(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(62);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_conflict_resolved_output_background_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/large_conflict_resolved_bg.rs");
    let abs_path = workdir.join(&file_rel);
    let comment_line = "still inside block comment";
    let fixture_line_count = 20_001usize;

    let mut base_lines = vec![
        "/* start block comment".to_string(),
        comment_line.to_string(),
        "end */".to_string(),
        "let chosen = 0;".to_string(),
    ];
    base_lines.extend(
        (base_lines.len()..fixture_line_count).map(|ix| format!("let base_bg_{ix}: usize = {ix};")),
    );
    // Carry classes the heuristic tokenizer cannot produce: a `type_identifier`,
    // a `field_identifier` and a method call. `syntax/heuristic.rs` colours
    // keywords, strings, numbers and comments and nothing else, so these are the
    // only probes that can tell a real tree-sitter parse from the fallback.
    let discriminating_lines = [
        "struct Stage { retries: usize }".to_string(),
        "fn bump(stage: &mut Stage) { stage.retries = stage.retries.wrapping_add(1); }".to_string(),
    ];
    base_lines.extend(discriminating_lines.iter().cloned());
    let base_text = base_lines.join("\n");

    let mut ours_lines = base_lines.clone();
    ours_lines[3] = "let chosen = 1;".to_string();
    let ours_text = ours_lines.join("\n");

    let mut theirs_lines = base_lines.clone();
    theirs_lines[3] = "let chosen = 2;".to_string();
    let theirs_text = theirs_lines.join("\n");

    let mut current_lines = vec![
        "/* start block comment".to_string(),
        comment_line.to_string(),
        "end */".to_string(),
        "<<<<<<< ours".to_string(),
        "let chosen = 1;".to_string(),
        "=======".to_string(),
        "let chosen = 2;".to_string(),
        ">>>>>>> theirs".to_string(),
    ];
    current_lines.extend(
        (current_lines.len()..fixture_line_count)
            .map(|ix| format!("let resolved_bg_{ix}: usize = {ix};")),
    );
    current_lines.extend(discriminating_lines.iter().cloned());
    let current_text = current_lines.join("\n");
    let resolved_output = crate::view::conflict_resolver::generate_resolved_text(
        crate::view::conflict_resolver::parse_conflict_markers(&current_text).as_slice(),
    );
    let line_count = resolved_output.lines().count();
    assert!(
        fixture_line_count > rows::MAX_LINES_FOR_SYNTAX_HIGHLIGHTING,
        "fixture should stay above the old syntax gate"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create conflict resolver fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write conflict resolver fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_secs(1),
                });
            });

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

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large conflict resolved output initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| {
            format!(
                "path={:?} line_count={} syntax_language={:?} live_syntax={} source_revision={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolved_preview_syntax_language,
                pane.conflict_resolved_output_live_syntax.is_some(),
                pane.conflict_resolved_preview_source_revision,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.recompute_conflict_resolved_outline_for_tests(cx);
                pane.conflict_resolver.resolver_pending_recompute_seq = pane
                    .conflict_resolver
                    .resolver_pending_recompute_seq
                    .wrapping_add(1);
                assert_eq!(
                    pane.conflict_resolved_preview_line_count, line_count,
                    "forced recompute should materialize the expected resolved output line count"
                );
                assert_eq!(
                    pane.conflict_resolved_preview_syntax_language,
                    Some(rows::DiffSyntaxLanguage::Rust),
                    "resolved output should still use the file-derived Rust syntax language"
                );
                assert!(
                    pane.conflict_resolved_output_live_syntax.is_some(),
                    "a 20k-line output should get a live syntax document: the old 4000-line \
                     `MAX_LINES_FOR_SYNTAX_HIGHLIGHTING` gate no longer applies to this view"
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let target_ix = 1usize;
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver_input.read_with(app, |input, _| {
                input.text().lines().nth(target_ix).map(ToOwned::to_owned)
            }),
            Some(comment_line.to_owned()),
            "the editable resolved-output buffer should expose the multiline comment text",
        );
        assert!(
            pane.conflict_resolved_output_projection.is_none(),
            "resolver bootstrap should materialize the editable output buffer"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_conflict_resolved_output_materialized(cx);
                assert!(
                    pane.conflict_resolved_output_projection.is_none(),
                    "explicit materialization should be idempotent"
                );
                assert_eq!(
                    pane.conflict_resolved_preview_line_count, line_count,
                    "materialized preview should preserve the output line count"
                );
                assert_eq!(
                    pane.conflict_resolved_preview_syntax_language,
                    Some(rows::DiffSyntaxLanguage::Rust),
                    "materialized resolved output should keep the path-derived syntax language"
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    // The row is the *continuation* of a block comment opened on the line
    // above, so getting it right requires the whole-document tree — a per-line
    // parse would read it as bare identifiers.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let comment_color = pane.theme.syntax.comment;
                let text = pane.conflict_resolver_input.read(cx).text().to_string();
                let line_start = text
                    .find(comment_line)
                    .expect("fixture should contain the comment continuation line");
                let line_end = line_start + comment_line.len();
                let highlights = pane.conflict_resolver_input.update(cx, |input, _| {
                    input.debug_effective_highlights_for_range(0..line_end + 1)
                });
                assert!(
                    highlights.iter().any(|(range, style)| {
                        range.start <= line_start
                            && range.end >= line_end
                            && style.color == Some(comment_color.into_color())
                    }),
                    "row {target_ix} continues a block comment and should be comment-coloured \
                     straight away: {highlights:?}"
                );

                // Do not assert on a keyword here. `syntax/heuristic.rs` colours
                // keywords too, so a `let`-shaped assertion passes in exactly the
                // broken state this test exists to catch -- the pane silently
                // falling back to the tokenizer because it never got a live
                // tree-sitter document. Only classes the tokenizer cannot
                // produce can tell the two engines apart.
                let all = pane.conflict_resolver_input.update(cx, |input, _| {
                    input.debug_effective_highlights_for_range(0..text.len())
                });
                assert_resolved_output_carries_treesitter_classes(&text, &all, pane.theme);
            });
        });
    });

    // The real regression this guards: a cold parse of a ~10KB output does not
    // fit the 1ms live foreground budget, so the first `LiveSyntaxDocument::new`
    // returns None. There is no tree to reparse incrementally, so unless the
    // build is finished off-thread the view stays on heuristic tokens forever --
    // which loses exactly the classes tree-sitter adds over a tokenizer: method
    // calls and field accesses.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                // Drop the document so the next refresh takes the *first parse*
                // path. An edit alone would not do: `sync` still has the old
                // tree to fall back on, so it recovers through the ordinary
                // deferred-reparse route and the bug stays hidden.
                pane.conflict_resolved_output_live_syntax = None;
                pane.conflict_resolved_output_live_syntax_source = None;
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
                pane.conflict_resolver_input.update(cx, |input, cx| {
                    let at = input.text().len();
                    input.replace_utf8_range(at..at, "\n", cx);
                });
            });
        });
    });
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolved output recovers a live document after a budget-exhausted first parse",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolved_output_live_syntax.is_some(),
        |pane| {
            format!(
                "live_syntax={} building={:?}",
                pane.conflict_resolved_output_live_syntax.is_some(),
                pane.conflict_resolved_output_live_syntax_building,
            )
        },
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_secs(1),
                });
                let snapshot = pane.conflict_resolver_input.read(cx).text_snapshot();
                let text: Arc<str> = snapshot.as_shared_string().into();
                let live = pane
                    .conflict_resolved_output_live_syntax
                    .as_ref()
                    .expect("recovered live document")
                    .snapshot(pane.theme)
                    .highlights_for_byte_range(0..text.len());
                let cold = rows::LiveSyntaxDocument::new(
                    rows::DiffSyntaxLanguage::Rust,
                    snapshot.rope(),
                    resolved_output_placeholder_protected_ranges_for_test(&text),
                    None,
                )
                .expect("cold parse")
                .snapshot(pane.theme)
                .highlights_for_byte_range(0..text.len());
                assert!(!live.is_empty(), "the recovered document must highlight");
                assert_eq!(
                    live, cold,
                    "the off-thread build must produce the same tree as an unbudgeted parse"
                );

                // And the recovered document must reach the *pane*, not just sit
                // in the field: the input is still showing whatever the earlier
                // fallback installed until the provider is rebound over it.
                let effective = pane.conflict_resolver_input.update(cx, |input, _| {
                    input.debug_effective_highlights_for_range(0..text.len())
                });
                assert_resolved_output_carries_treesitter_classes(&text, &effective, pane.theme);
            });
        });
    });

    // Switching theme must actually re-colour the output. The syntax palette is
    // baked into LiveSyntaxSnapshot at build time, and `set_highlight_provider_with_key`
    // early-returns on an unchanged key -- so if the key does not move on a theme
    // change, the old palette stays installed and the text keeps its old colours.
    let (dark_runs, light_runs) = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let len = pane.conflict_resolver_input.read(cx).text().len();
                let dark = pane.conflict_resolver_input.update(cx, |input, _| {
                    input.debug_effective_highlights_for_range(0..len.min(400))
                });
                // Deliberately another *dark* theme that differs only in its
                // syntax palette. A key built from sampled theme colours (or
                // from `is_dark`) would collide here and silently keep the old
                // palette; only a theme epoch catches it.
                pane.set_theme(other_dark_theme(), cx);
                let light = pane.conflict_resolver_input.update(cx, |input, _| {
                    input.debug_effective_highlights_for_range(0..len.min(400))
                });
                pane.set_theme(crate::theme::AppTheme::gitcomet_dark(), cx);
                (dark, light)
            })
        })
    });
    assert!(!dark_runs.is_empty() && !light_runs.is_empty());
    assert_ne!(
        dark_runs, light_runs,
        "a theme change must rebind the provider so the new syntax palette is used"
    );
    assert!(
        dark_runs
            .iter()
            .zip(light_runs.iter())
            .any(|((_, a), (_, b))| a.color != b.color),
        "the difference must be in the colours themselves, not just run boundaries"
    );

    // Settling must be idempotent. Installing a highlight provider notifies the
    // input, which re-enters the `cx.observe` that installed it; if a quiet
    // cycle still reparsed and rebound, that notify would trigger another, and
    // the pane would spin forever instead of ever finishing a frame.
    let settled_version = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .conflict_resolved_output_live_syntax
            .as_ref()
            .map(|document| document.version())
            .expect("a materialized Rust output has a live syntax document")
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .main_pane
                .read(app)
                .conflict_resolved_output_live_syntax
                .as_ref()
                .map(|document| document.version()),
            Some(settled_version),
            "an idle frame must not reparse or rebind the resolved output"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict resolver fixture");
}

#[gpui::test]
fn edited_conflict_resolved_output_highlights_multiline_comment_on_the_keystroke(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(63);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_edited_conflict_resolved_output_background_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/edited_conflict_resolved_bg.rs");
    let abs_path = workdir.join(&file_rel);
    let inserted_comment_line = "still inside block comment";
    let inserted_prefix = format!("/* start block comment\n{inserted_comment_line}\nend */\n");
    let fixture_line_count = 20_001usize;

    let mut base_lines = vec![
        "fn large_demo() {".to_string(),
        "    let chosen = 0;".to_string(),
        "    let tail = 9;".to_string(),
        "}".to_string(),
    ];
    base_lines.extend(
        (base_lines.len()..fixture_line_count).map(|ix| format!("let base_bg_{ix}: usize = {ix};")),
    );
    let base_text = base_lines.join("\n");

    let mut ours_lines = base_lines.clone();
    ours_lines[1] = "    let chosen = 1;".to_string();
    let ours_text = ours_lines.join("\n");

    let mut theirs_lines = base_lines.clone();
    theirs_lines[1] = "    let chosen = 2;".to_string();
    let theirs_text = theirs_lines.join("\n");

    let mut current_lines = vec![
        "fn large_demo() {".to_string(),
        "<<<<<<< ours".to_string(),
        "    let chosen = 1;".to_string(),
        "=======".to_string(),
        "    let chosen = 2;".to_string(),
        ">>>>>>> theirs".to_string(),
        "    let tail = 9;".to_string(),
        "}".to_string(),
    ];
    current_lines.extend(
        (current_lines.len()..fixture_line_count)
            .map(|ix| format!("let resolved_bg_{ix}: usize = {ix};")),
    );
    let current_text = current_lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create conflict resolver fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write conflict resolver fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_secs(1),
                });
            });

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

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited conflict resolved output initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| {
            format!(
                "path={:?} line_count={} source_revision={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolved_preview_source_revision,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_conflict_resolved_output_materialized(cx);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited conflict resolved output materialized for editing",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolved_output_projection.is_none(),
        |pane| {
            format!(
                "projection_present={} line_count={} live_syntax={}",
                pane.conflict_resolved_output_projection.is_some(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolved_output_live_syntax.is_some(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.recompute_conflict_resolved_outline_for_tests(cx);
                pane.conflict_resolver.resolver_pending_recompute_seq = pane
                    .conflict_resolver
                    .resolver_pending_recompute_seq
                    .wrapping_add(1);
            });
        });
    });

    // Under the live engine there is no plain-then-upgrade window to wait for:
    // the tree is parsed on materialization and edited in place afterwards.
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.conflict_resolved_output_live_syntax.is_some(),
            "materializing an editable Rust output should build a live syntax document"
        );
        assert_eq!(
            pane.conflict_resolved_preview_syntax_language,
            Some(rows::DiffSyntaxLanguage::Rust)
        );
    });

    // Insert a block comment whose body runs onto the next row. Getting that row
    // right needs the reparse to have happened — `tree.edit` alone only shifts
    // existing nodes, it cannot invent a comment node — so this is a test that
    // the keystroke path reparses synchronously within its budget. (The
    // budget-exhausted path is covered by `syntax::live`'s own tests, where the
    // deferred tree keeps painting until a background pass catches up.)
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, &inserted_prefix, cx);
                });
            });
        });
    });

    // No `wait_for_*`: the assertion is that this is already true, on the very
    // next look, with no background pass and no debounce elapsed.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let comment_color = pane.theme.syntax.comment;
                let line_start = inserted_prefix
                    .find(inserted_comment_line)
                    .expect("fixture prefix should contain the continuation line");
                let line_end = line_start + inserted_comment_line.len();
                let highlights = pane.conflict_resolver_input.update(cx, |input, _| {
                    input.debug_effective_highlights_for_range(0..inserted_prefix.len())
                });
                assert!(
                    highlights.iter().any(|(range, style)| {
                        range.start <= line_start
                            && range.end >= line_end
                            && style.color == Some(comment_color.into_color())
                    }),
                    "the row inside the inserted block comment should be comment-coloured \
                     on the keystroke, not after a background upgrade: {highlights:?}"
                );
            });
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict resolver fixture");
}

#[gpui::test]
fn conflict_resolver_fresh_open_uses_persisted_view_mode_and_toasts_once(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(171);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_view_mode_persist",
        std::process::id()
    ));

    let base_text = "base line\ncontext\ntail".to_string();
    let ours_text = "ours line\ncontext\ntail".to_string();
    let theirs_text = "theirs line\ncontext\ntail".to_string();
    let current_text =
        format!("<<<<<<< ours\n{ours_text}\n=======\n{theirs_text}\n>>>>>>> theirs\n");

    let file_a = std::path::PathBuf::from("fixtures/view_mode_persist_a.txt");
    let file_b = std::path::PathBuf::from("fixtures/view_mode_persist_b.txt");
    let _ = std::fs::remove_dir_all(&workdir);
    for file_rel in [&file_a, &file_b] {
        let abs_path = workdir.join(file_rel);
        std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
            .expect("create view-mode fixture dir");
        std::fs::write(&abs_path, &current_text).expect("write view-mode fixture");
    }

    let repo_with_conflict = |file_rel: &std::path::PathBuf,
                              base_text: &String,
                              ours_text: &String,
                              theirs_text: &String,
                              current_text: &String| {
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
            current_text,
        ));
        repo
    };

    // Persisted preference says the user last used two-way mode.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.mergetool_view_three_way = false;
            });
            let repo =
                repo_with_conflict(&file_a, &base_text, &ours_text, &theirs_text, &current_text);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "view-mode fixture A open summary announced",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_a)
                && pane.conflict_resolver.open_summary_announced
        },
        |pane| {
            format!(
                "path={:?} announced={} auto={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.open_summary_announced,
                pane.conflict_resolver.open_summary_counts,
            )
        },
    );

    cx.update(|_window, app| {
        let this = view.read(app);
        let pane = this.main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver.view_mode,
            ConflictResolverViewMode::TwoWayDiff,
            "base-present fresh open should honor the persisted two-way preference",
        );
        assert_eq!(
            this.toast_host.read(app).toast_count_for_tests(),
            1,
            "fresh open should push exactly one summary toast",
        );
    });

    // Re-syncing the same conflict must not announce again.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo =
                repo_with_conflict(&file_a, &base_text, &ours_text, &theirs_text, &current_text);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let this = view.read(app);
        assert_eq!(
            this.toast_host.read(app).toast_count_for_tests(),
            1,
            "same-conflict re-sync must not push another summary toast",
        );
    });

    // Toggling to three-way persists the preference.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                assert!(
                    pane.mergetool_view_three_way,
                    "switching to three-way should persist the preference",
                );
            });
        });
    });

    // A different conflict file is a fresh open: it honors the new preference
    // and announces its own summary.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo =
                repo_with_conflict(&file_b, &base_text, &ours_text, &theirs_text, &current_text);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "view-mode fixture B open summary announced",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_b)
                && pane.conflict_resolver.open_summary_announced
        },
        |pane| {
            format!(
                "path={:?} announced={} auto={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.open_summary_announced,
                pane.conflict_resolver.open_summary_counts,
            )
        },
    );

    cx.update(|_window, app| {
        let this = view.read(app);
        let pane = this.main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver.view_mode,
            ConflictResolverViewMode::ThreeWay,
            "fresh open after toggling should default to the persisted three-way mode",
        );
        assert_eq!(
            this.toast_host.read(app).toast_count_for_tests(),
            2,
            "a different conflict file is a fresh open and gets its own toast",
        );
    });

    // Returning to the first conflict file in this window must not announce it
    // a second time.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo =
                repo_with_conflict(&file_a, &base_text, &ours_text, &theirs_text, &current_text);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let this = view.read(app);
        let pane = this.main_pane.read(app);
        assert_eq!(pane.conflict_resolver.path.as_ref(), Some(&file_a));
        assert!(pane.conflict_resolver.open_summary_announced);
        assert_eq!(
            this.toast_host.read(app).toast_count_for_tests(),
            2,
            "reopening a previously announced conflict file must not push another toast",
        );
    });

    cx.run_until_parked();
    std::fs::remove_dir_all(&workdir).expect("cleanup view-mode persist fixture");
}

#[gpui::test]
fn conflict_resolver_split_selection_and_join_dispatch_and_rebuild_blocks(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(172);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_split_selection",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/split_selection.txt");
    let base = "ctx\nb1\nb2\nb3\ntail\n".to_string();
    let ours = "ctx\no1\no2\no3\ntail\n".to_string();
    let theirs = "ctx\nt1\nt2\nt3\ntail\n".to_string();
    let current = concat!(
        "ctx\n",
        "<<<<<<< ours\n",
        "o1\n",
        "o2\n",
        "o3\n",
        "=======\n",
        "t1\n",
        "t2\n",
        "t3\n",
        ">>>>>>> theirs\n",
        "tail\n",
    )
    .to_string();

    let mut repo = opening_repo_state(repo_id, &workdir);
    set_test_conflict_status(
        &mut repo,
        file_rel.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    set_test_conflict_file(
        &mut repo,
        file_rel.clone(),
        base.clone(),
        ours.clone(),
        theirs.clone(),
        current.clone(),
    );
    repo.conflict_state.conflict_file_load_mode = gitcomet_state::model::ConflictFileLoadMode::Full;
    repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
        file_rel.clone(),
        gitcomet_core::domain::FileConflictKind::BothModified,
        ConflictPayload::Text(base.into()),
        ConflictPayload::Text(ours.into()),
        ConflictPayload::Text(theirs.into()),
        &current,
    ));
    let state = app_state_with_repo(repo, repo_id);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.store.replace_snapshot_for_test(Arc::clone(&state));
            push_test_state(this, Arc::clone(&state), cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "split-ready conflict alignment",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.conflict_row_selection_enabled()
                && pane
                    .conflict_resolver
                    .three_way_block_aligned_range(0)
                    .is_some_and(|range| range.len() >= 3)
                && pane.conflict_resolver.conflict_region_indices == vec![0]
        },
        |pane| {
            format!(
                "path={:?} enabled={} range={:?} regions={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.conflict_row_selection_enabled(),
                pane.conflict_resolver.three_way_block_aligned_range(0),
                pane.conflict_resolver.conflict_region_indices,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let range = pane
                    .conflict_resolver
                    .three_way_block_aligned_range(0)
                    .expect("aligned block");
                pane.conflict_resolver_begin_row_selection(0, 0, cx);
                let selection = pane.conflict_resolver.row_selection.expect("selection");
                assert_eq!(selection.anchor_row, range.start);
                assert!(selection.selecting);
                pane.conflict_resolver_extend_row_selection(99, usize::MAX, cx);
                assert_eq!(
                    pane.conflict_resolver.row_selection.unwrap().head_row,
                    range.end - 1,
                    "dragging into another block clamps to the anchored block",
                );
                pane.conflict_resolver_extend_row_selection(99, 0, cx);
                assert_eq!(
                    pane.conflict_resolver.row_selection.unwrap().head_row,
                    range.start,
                );
                pane.conflict_resolver_end_row_selection(cx);
                assert!(!pane.conflict_resolver.row_selection.unwrap().selecting);

                let middle_row = range.start + 1;
                pane.conflict_resolver_begin_row_selection(0, middle_row, cx);
                pane.conflict_resolver_end_row_selection(cx);
                assert_eq!(pane.conflict_resolver_split_selection_row_count(0), Some(1),);

                pane.conflict_resolver_click_row_selection(
                    0,
                    range.end - 1,
                    gpui::Modifiers {
                        shift: true,
                        ..Default::default()
                    },
                    cx,
                );
                let selection = pane.conflict_resolver.row_selection.unwrap();
                assert_eq!(selection.anchor_row, middle_row);
                assert_eq!(selection.head_row, range.end - 1);
                assert!(!selection.selecting);
                assert_eq!(pane.conflict_resolver_split_selection_row_count(0), Some(2));

                pane.conflict_resolver_click_row_selection(
                    0,
                    range.start,
                    gpui::Modifiers {
                        control: true,
                        ..Default::default()
                    },
                    cx,
                );
                let selection = pane.conflict_resolver.row_selection.unwrap();
                assert_eq!(selection.anchor_row, middle_row);
                assert_eq!(selection.head_row, range.start);
                assert_eq!(pane.conflict_resolver_split_selection_row_count(0), Some(2));

                pane.conflict_resolver_begin_row_selection(0, middle_row, cx);
                pane.conflict_resolver_end_row_selection(cx);
                pane.conflict_resolver_split_selection(cx);
                assert!(
                    pane.conflict_resolver.row_selection.is_some(),
                    "selection stays available until the split is accepted"
                );
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "split dispatch to reach the store",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |_pane| {
            store_for_assert
                .snapshot()
                .repos
                .first()
                .and_then(|repo| repo.conflict_state.conflict_session.as_ref())
                .is_some_and(|session| session.regions.len() == 3)
        },
        |_pane| {
            let snapshot = store_for_assert.snapshot();
            snapshot
                .repos
                .first()
                .map(|repo| {
                    (
                        repo.conflict_state.conflict_rev,
                        repo.conflict_state
                            .conflict_session
                            .as_ref()
                            .map(|session| session.regions.len()),
                    )
                })
                .unwrap_or_default()
        },
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "split reducer round-trip",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            crate::view::conflict_resolver::conflict_count(&pane.conflict_resolver.marker_segments)
                == 3
                && pane.conflict_resolver.conflict_region_indices == vec![0, 1, 2]
                && pane.conflict_resolver.row_selection.is_none()
                && pane.conflict_resolver.active_conflict == Some(0)
                && pane.conflict_resolver.nav_anchor.is_some_and(|anchor| {
                    anchor.id == crate::view::conflict_resolver::ConflictNavTargetId::Region(0)
                })
        },
        |pane| {
            format!(
                "blocks={} regions={:?} rev={}",
                crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ),
                pane.conflict_resolver.conflict_region_indices,
                pane.conflict_resolver.conflict_rev,
            )
        },
    );

    let snapshot = store_for_assert.snapshot();
    let session = snapshot.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("split session");
    assert_eq!(session.regions.len(), 3);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let target = ConflictResolverJoinTarget {
                    repo_id,
                    path: file_rel.clone().into(),
                    conflict_rev: pane.conflict_resolver.conflict_rev,
                    first_region_index: 0,
                };
                let mut stale = target.clone();
                stale.conflict_rev = stale.conflict_rev.wrapping_add(1);
                pane.conflict_resolver_join_regions(stale, cx);
                pane.conflict_resolver_join_regions(target, cx);
            });
        });
    });
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "join dispatch to reach the store",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |_pane| {
            store_for_assert
                .snapshot()
                .repos
                .first()
                .and_then(|repo| repo.conflict_state.conflict_session.as_ref())
                .is_some_and(|session| session.regions.len() == 2)
        },
        |_pane| {
            store_for_assert
                .snapshot()
                .repos
                .first()
                .and_then(|repo| repo.conflict_state.conflict_session.as_ref())
                .map(|session| session.regions.len())
        },
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
    });
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "join reducer round-trip",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            crate::view::conflict_resolver::conflict_count(&pane.conflict_resolver.marker_segments)
                == 2
                && pane.conflict_resolver.conflict_region_indices == vec![0, 1]
                && pane.conflict_resolver.active_conflict == Some(0)
                && pane.conflict_resolver.nav_anchor.is_some_and(|anchor| {
                    anchor.id == crate::view::conflict_resolver::ConflictNavTargetId::Region(0)
                })
        },
        |pane| {
            format!(
                "blocks={} regions={:?} rev={}",
                crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ),
                pane.conflict_resolver.conflict_region_indices,
                pane.conflict_resolver.conflict_rev,
            )
        },
    );

    let before_reset = store_for_assert.snapshot();
    let before_reset_repo = &before_reset.repos[0];
    let before_reset_rev = before_reset_repo.conflict_state.conflict_rev;
    let session_current = before_reset_repo
        .conflict_state
        .conflict_session
        .as_ref()
        .and_then(|session| session.marker_projection_text())
        .expect("joined session marker projection")
        .to_string();
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                assert_eq!(
                    pane.conflict_resolver.current.as_deref(),
                    Some(session_current.as_str()),
                    "lightweight resync must retain the same authoritative marker snapshot",
                );
                pane.conflict_resolver_reset_output_from_markers(cx);
            });
        });
    });
    cx.run_until_parked();
    let after_reset = store_for_assert.snapshot();
    assert_eq!(
        after_reset.repos[0].conflict_state.conflict_rev, before_reset_rev,
        "Reset is a no-op in the reducer while every joined region is unresolved",
    );
    assert_eq!(
        after_reset.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("session after no-op Reset")
            .regions
            .len(),
        2,
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
    });
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "no-op reset keeps joined geometry",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            crate::view::conflict_resolver::conflict_count(&pane.conflict_resolver.marker_segments)
                == 2
                && pane.conflict_resolver.conflict_region_indices == vec![0, 1]
        },
        |pane| {
            format!(
                "blocks={} regions={:?} current_markers={}",
                crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ),
                pane.conflict_resolver.conflict_region_indices,
                pane.conflict_resolver
                    .current
                    .as_deref()
                    .map_or(0, |text| text.matches("<<<<<<<").count()),
            )
        },
    );
}

#[gpui::test]
fn conflict_resolver_current_only_then_full_keeps_mode_and_edited_worktree_output(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(173);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_current_only_mode",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/current_only_mode.txt");
    let base = "ctx\nbase\ntail\n".to_string();
    let ours = "ctx\nours\ntail\n".to_string();
    let theirs = "ctx\ntheirs\ntail\n".to_string();
    let current = "ctx\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\ntail\n";

    let mut current_only_repo = opening_repo_state(repo_id, &workdir);
    set_test_conflict_status(
        &mut current_only_repo,
        file_rel.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    current_only_repo.conflict_state.conflict_file_path = Some(file_rel.clone());
    current_only_repo.conflict_state.conflict_file_load_mode =
        gitcomet_state::model::ConflictFileLoadMode::CurrentOnly;
    current_only_repo.conflict_state.conflict_file =
        gitcomet_state::model::Loadable::Ready(Some(gitcomet_state::model::ConflictFile {
            path: file_rel.clone().into(),
            base_bytes: None,
            ours_bytes: None,
            theirs_bytes: None,
            current_bytes: None,
            base: None,
            ours: None,
            theirs: None,
            current: Some(current.to_string().into()),
        }));
    current_only_repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
        file_rel.clone(),
        gitcomet_core::domain::FileConflictKind::BothModified,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        current,
    ));

    let current_only_state = app_state_with_repo(current_only_repo, repo_id);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.mergetool_view_three_way = true;
            });
            this.store
                .replace_snapshot_for_test(Arc::clone(&current_only_state));
            push_test_state(this, Arc::clone(&current_only_state), cx);
        });
    });
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "current-only persisted three-way mode",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane
                    .conflict_resolver
                    .loaded_file
                    .as_ref()
                    .is_some_and(|file| file.base.is_none())
        },
        |pane| {
            format!(
                "path={:?} mode={:?} has_base={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.view_mode,
                pane.conflict_resolver
                    .loaded_file
                    .as_ref()
                    .is_some_and(|file| file.base.is_some()),
            )
        },
    );
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .main_pane
                .read(app)
                .conflict_resolver
                .view_mode,
            ConflictResolverViewMode::ThreeWay,
            "a BothModified CurrentOnly first paint should honor persisted three-way mode",
        );
    });

    let full_current = "ctx\nmanually resolved during load\ntail\n".to_string();
    let mut full_repo = opening_repo_state(repo_id, &workdir);
    set_test_conflict_status(
        &mut full_repo,
        file_rel.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    set_test_conflict_file(
        &mut full_repo,
        file_rel.clone(),
        base.clone(),
        ours.clone(),
        theirs.clone(),
        full_current.clone(),
    );
    full_repo.conflict_state.conflict_file_load_mode =
        gitcomet_state::model::ConflictFileLoadMode::Full;
    // The reducer bumps this when the CurrentOnly request upgrades to Full;
    // mirror that notification boundary in this direct-state test fixture.
    full_repo.conflict_state.conflict_rev = current_only_state.repos[0]
        .conflict_state
        .conflict_rev
        .wrapping_add(1);
    full_repo.conflict_state.conflict_session =
        Some(ConflictSession::from_stage_inputs_with_current(
            file_rel.clone(),
            gitcomet_core::domain::FileConflictKind::BothModified,
            ConflictPayload::Text(base.clone().into()),
            ConflictPayload::Text(ours.into()),
            ConflictPayload::Text(theirs.into()),
            Some(ConflictPayload::Text(full_current.clone().into())),
        ));
    let full_state = app_state_with_repo(full_repo, repo_id);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.store
                .replace_snapshot_for_test(Arc::clone(&full_state));
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
    });
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "full-side upgrade preserves three-way mode",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.three_way_text.base.as_ref() == base
                && !pane.conflict_resolver.three_way_aligned.is_identity()
                && pane.conflict_resolver.output_is_protected
                && pane.conflict_resolved_output_projection.is_none()
        },
        |pane| {
            format!(
                "path={:?} mode={:?} base_len={} identity={} protected={} projected={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.view_mode,
                pane.conflict_resolver.three_way_text.base.len(),
                pane.conflict_resolver.three_way_aligned.is_identity(),
                pane.conflict_resolver.output_is_protected,
                pane.conflict_resolved_output_projection.is_some(),
            )
        },
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver.view_mode,
            ConflictResolverViewMode::ThreeWay
        );
        assert_eq!(
            pane.conflict_resolver_input.read(app).text(),
            full_current,
            "the Full upgrade must not replace a manual worktree result with stage markers",
        );
    });
}
