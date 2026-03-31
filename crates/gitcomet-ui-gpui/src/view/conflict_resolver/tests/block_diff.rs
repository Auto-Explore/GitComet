use super::*;

#[test]
fn three_way_word_highlights_align_shifted_local_and_remote_rows() {
    fn line_starts(text: &str) -> Vec<usize> {
        let mut starts =
            Vec::with_capacity(text.as_bytes().iter().filter(|&&b| b == b'\n').count() + 1);
        starts.push(0);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                starts.push(ix + 1);
            }
        }
        starts
    }

    let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "alpha\nbeta changed\ngamma\n".into(),
        theirs: "alpha\ninserted\nbeta remote\ngamma\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let base_text = "";
    let ours_text = "alpha\nbeta changed\ngamma\n";
    let theirs_text = "alpha\ninserted\nbeta remote\ngamma\n";

    let (_base_hl, ours_hl, theirs_hl) = compute_three_way_word_highlights(
        base_text,
        &line_starts(base_text),
        ours_text,
        &line_starts(ours_text),
        theirs_text,
        &line_starts(theirs_text),
        &marker_segments,
    );

    assert!(
        ours_hl.contains_key(&1),
        "local modified line should be highlighted even when remote line is shifted"
    );
    assert!(
        !ours_hl.contains_key(&0),
        "unchanged local line should not be highlighted"
    );
    assert!(
        !ours_hl.contains_key(&2),
        "unchanged local line should not be highlighted"
    );

    assert!(
        theirs_hl.contains_key(&1),
        "remote added line should be highlighted"
    );
    assert!(
        theirs_hl.contains_key(&2),
        "remote modified line should be highlighted at its aligned row"
    );
    assert!(
        !theirs_hl.contains_key(&3),
        "unchanged remote line should not be highlighted"
    );
}

#[test]
fn three_way_word_highlights_keep_global_offsets_per_column() {
    fn line_starts(text: &str) -> Vec<usize> {
        let mut starts =
            Vec::with_capacity(text.as_bytes().iter().filter(|&&b| b == b'\n').count() + 1);
        starts.push(0);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                starts.push(ix + 1);
            }
        }
        starts
    }

    let marker_segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "same\n".into(),
            theirs: "added\nsame\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("tail\n".into()),
    ];
    let base_text = "";
    let ours_text = "ctx\nsame\ntail\n";
    let theirs_text = "ctx\nadded\nsame\ntail\n";

    let (_base_hl, ours_hl, theirs_hl) = compute_three_way_word_highlights(
        base_text,
        &line_starts(base_text),
        ours_text,
        &line_starts(ours_text),
        theirs_text,
        &line_starts(theirs_text),
        &marker_segments,
    );

    assert!(
        !ours_hl.contains_key(&1),
        "local unchanged block line should stay unhighlighted"
    );
    assert!(
        theirs_hl.contains_key(&1),
        "remote inserted block line should map to its own global row"
    );
    assert!(
        !theirs_hl.contains_key(&2),
        "remote aligned context line should not be highlighted"
    );
}

#[test]
fn block_local_two_way_diff_rows_produces_correct_rows_and_line_numbers() {
    let input = "ctx-0\nctx-1\nctx-2\nctx-3\n<<<<<<< HEAD\nours-line\n=======\ntheirs-line\n>>>>>>> other\nctx-4\nctx-5\nctx-6\nctx-7\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);

    let rows = block_local_two_way_diff_rows(&segments);

    assert!(
        !rows.is_empty(),
        "block-local diff should produce rows for the conflict block"
    );

    let context_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.kind == RK::Context)
        .map(|row| row.old.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(
        context_rows,
        vec!["ctx-1", "ctx-2", "ctx-3", "ctx-4", "ctx-5", "ctx-6"],
        "block-local rows should keep only the configured boundary context window",
    );

    assert!(
        rows.iter()
            .any(|row| row.old_line == Some(5) || row.new_line == Some(5)),
        "block-local diff rows should still reference the conflict's global line number",
    );
}

#[test]
fn block_local_two_way_diff_rows_stats_keep_large_blocks_out_of_whole_block_diff() {
    let repeated = "ours-line\n".repeat(LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 17);
    let input = format!("pre\n<<<<<<< HEAD\n{repeated}=======\n{repeated}>>>>>>> other\npost\n");
    let segments = parse_conflict_markers(&input);
    assert_eq!(conflict_count(&segments), 1);

    let (rows, stats) = block_local_two_way_diff_rows_with_stats(&segments);

    assert!(
        !rows.is_empty(),
        "large conflict preview should still emit bounded rows"
    );
    assert!(
        !stats.whole_block_diff_ran,
        "large conflict previews should never run a whole-block side-by-side diff"
    );
}

#[test]
fn block_local_two_way_diff_rows_handles_multiple_blocks() {
    let input = "top\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> other\nmid\n<<<<<<< HEAD\nx\ny\n=======\nz\n>>>>>>> other\nend\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 2);

    let rows = block_local_two_way_diff_rows(&segments);

    let mid_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.old.as_deref() == Some("mid") && row.new.as_deref() == Some("mid"))
        .collect();
    assert_eq!(
        mid_rows.len(),
        1,
        "shared middle context should not be duplicated when boundary windows overlap",
    );
    assert_eq!(mid_rows[0].kind, RK::Context);
    assert_eq!(mid_rows[0].old_line, Some(3));
    assert_eq!(mid_rows[0].new_line, Some(3));

    let has_line_2 = rows
        .iter()
        .any(|r| r.old_line == Some(2) || r.new_line == Some(2));
    let has_line_4_or_5 = rows
        .iter()
        .any(|r| r.old_line.is_some_and(|l| l >= 4) || r.new_line.is_some_and(|l| l >= 4));
    assert!(has_line_2, "should have rows referencing block 1 at line 2");
    assert!(
        has_line_4_or_5,
        "should have rows referencing block 2 at line 4+"
    );

    let end_row = rows
        .iter()
        .find(|row| row.old.as_deref() == Some("end") && row.new.as_deref() == Some("end"))
        .expect("expected trailing boundary context row");
    assert_eq!(end_row.kind, RK::Context);
    assert_eq!(end_row.old_line, Some(6));
    assert_eq!(end_row.new_line, Some(5));
}

#[test]
fn block_local_two_way_diff_rows_preserves_tiny_block_prefix_alignment() {
    let input = "<<<<<<< HEAD\nsame\nours-only\n=======\nsame\n>>>>>>> other\n";
    let rows = block_local_two_way_diff_rows(&parse_conflict_markers(input));

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].kind, RK::Context);
    assert_eq!(rows[0].old_line, Some(1));
    assert_eq!(rows[0].new_line, Some(1));
    assert_eq!(rows[0].old.as_deref(), Some("same"));
    assert_eq!(rows[0].new.as_deref(), Some("same"));

    assert_eq!(rows[1].kind, RK::Remove);
    assert_eq!(rows[1].old_line, Some(2));
    assert_eq!(rows[1].new_line, None);
    assert_eq!(rows[1].old.as_deref(), Some("ours-only"));
    assert_eq!(rows[1].new.as_deref(), None);
}

#[test]
fn block_local_two_way_diff_rows_preserves_tiny_block_suffix_alignment() {
    let input = "<<<<<<< HEAD\nours-only\nsame\n=======\nsame\n>>>>>>> other\n";
    let rows = block_local_two_way_diff_rows(&parse_conflict_markers(input));

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].kind, RK::Remove);
    assert_eq!(rows[0].old_line, Some(1));
    assert_eq!(rows[0].new_line, None);
    assert_eq!(rows[0].old.as_deref(), Some("ours-only"));
    assert_eq!(rows[0].new.as_deref(), None);

    assert_eq!(rows[1].kind, RK::Context);
    assert_eq!(rows[1].old_line, Some(2));
    assert_eq!(rows[1].new_line, Some(1));
    assert_eq!(rows[1].old.as_deref(), Some("same"));
    assert_eq!(rows[1].new.as_deref(), Some("same"));
}

#[test]
fn block_local_two_way_diff_rows_keeps_trailing_context_without_terminal_newline() {
    let input = "ctx-0\n<<<<<<< HEAD\nours-line\n=======\ntheirs-line\n>>>>>>> other\nctx-1";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);

    let rows = block_local_two_way_diff_rows(&segments);
    let trailing_row = rows
        .iter()
        .find(|row| row.old.as_deref() == Some("ctx-1") && row.new.as_deref() == Some("ctx-1"))
        .expect("expected trailing boundary context row without a terminal newline");

    assert_eq!(trailing_row.kind, RK::Context);
    assert_eq!(trailing_row.old_line, Some(3));
    assert_eq!(trailing_row.new_line, Some(3));
}

#[test]
fn block_local_two_way_diff_rows_empty_for_no_conflicts() {
    let segments = parse_conflict_markers("just plain text\nno conflicts\n");
    assert_eq!(conflict_count(&segments), 0);

    let rows = block_local_two_way_diff_rows(&segments);
    assert!(rows.is_empty(), "no conflict blocks means no diff rows");
}

#[test]
fn block_local_two_way_conflict_maps_leave_boundary_context_unmapped() {
    let input = "ctx-a\nctx-b\nctx-c\n<<<<<<< HEAD\nours-line\n=======\ntheirs-line\n>>>>>>> other\nctx-d\nctx-e\nctx-f\n";
    let segments = parse_conflict_markers(input);
    let rows =
        block_local_two_way_diff_rows_with_context(&segments, BLOCK_LOCAL_DIFF_CONTEXT_LINES);
    let inline_rows = build_inline_rows(&rows);

    let (diff_row_conflict_map, inline_row_conflict_map) =
        map_two_way_rows_to_conflicts(&segments, &rows, &inline_rows);

    assert!(
        diff_row_conflict_map.iter().any(|entry| entry.is_none()),
        "boundary context rows should stay outside any conflict mapping",
    );
    assert!(
        diff_row_conflict_map.contains(&Some(0)),
        "conflict-local rows should still map back to the conflict block",
    );
    for (row, conflict_ix) in rows.iter().zip(diff_row_conflict_map.iter()) {
        if conflict_ix.is_none() {
            assert_eq!(
                row.kind,
                RK::Context,
                "only shared context rows should remain unmapped",
            );
        }
    }

    assert!(
        inline_row_conflict_map.iter().any(|entry| entry.is_none()),
        "inline boundary context rows should stay outside any conflict mapping",
    );
    assert!(
        inline_row_conflict_map.contains(&Some(0)),
        "inline conflict rows should still map back to the conflict block",
    );
    for (row, conflict_ix) in inline_rows.iter().zip(inline_row_conflict_map.iter()) {
        if conflict_ix.is_none() {
            assert_eq!(
                row.kind,
                gitcomet_core::domain::DiffLineKind::Context,
                "only inline context rows should remain unmapped",
            );
        }
    }
}

// -- binary search conflict_index_for_line tests --

#[test]
fn conflict_index_for_line_finds_correct_range() {
    let ranges = vec![5..10, 20..30, 50..55];
    assert_eq!(conflict_index_for_line(&ranges, 0), None);
    assert_eq!(conflict_index_for_line(&ranges, 4), None);
    assert_eq!(conflict_index_for_line(&ranges, 5), Some(0));
    assert_eq!(conflict_index_for_line(&ranges, 9), Some(0));
    assert_eq!(conflict_index_for_line(&ranges, 10), None);
    assert_eq!(conflict_index_for_line(&ranges, 15), None);
    assert_eq!(conflict_index_for_line(&ranges, 20), Some(1));
    assert_eq!(conflict_index_for_line(&ranges, 29), Some(1));
    assert_eq!(conflict_index_for_line(&ranges, 30), None);
    assert_eq!(conflict_index_for_line(&ranges, 50), Some(2));
    assert_eq!(conflict_index_for_line(&ranges, 54), Some(2));
    assert_eq!(conflict_index_for_line(&ranges, 55), None);
    assert_eq!(conflict_index_for_line(&ranges, 100), None);
}

#[test]
fn conflict_index_for_line_empty_ranges() {
    let ranges: Vec<std::ops::Range<usize>> = Vec::new();
    assert_eq!(conflict_index_for_line(&ranges, 0), None);
    assert_eq!(conflict_index_for_line(&ranges, 100), None);
}

#[test]
fn conflict_index_for_line_single_range() {
    let ranges = vec![10..20];
    assert_eq!(conflict_index_for_line(&ranges, 9), None);
    assert_eq!(conflict_index_for_line(&ranges, 10), Some(0));
    assert_eq!(conflict_index_for_line(&ranges, 19), Some(0));
    assert_eq!(conflict_index_for_line(&ranges, 20), None);
}

// -- per-side conflict ranges tests --

#[test]
fn build_three_way_conflict_maps_includes_per_side_ranges() {
    let segments = vec![
        ConflictSegment::Text("a\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base1\nbase2\n".into()),
            ours: "ours1\n".into(),
            theirs: "theirs1\ntheirs2\ntheirs3\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("mid\n".into()),
    ];
    let maps = build_three_way_conflict_maps(&segments, 3, 2, 4);
    // [base, ours, theirs]
    // base: 1 text line + 2 conflict lines = 3 total. conflict at 1..3
    assert_eq!(maps.conflict_ranges[0], vec![1..3]);
    // ours: 1 text line + 1 conflict line = 2 total. conflict at 1..2
    assert_eq!(maps.conflict_ranges[1], vec![1..2]);
    // theirs: 1 text line + 3 conflict lines = 4 total. conflict at 1..4
    assert_eq!(maps.conflict_ranges[2], vec![1..4]);
}

#[test]
fn per_side_ranges_binary_search_matches_per_line_maps() {
    // Two conflicts with different side lengths.
    let segments = vec![
        ConflictSegment::Text("a\nb\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("B1\n".into()),
            ours: "O1\nO2\n".into(),
            theirs: "T1\nT2\nT3\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "x\n".into(),
            theirs: "y\nz\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        }),
        ConflictSegment::Text("end\n".into()),
    ];
    let base_count = 2 + 1 + 1 + 1; // 5
    let ours_count = 2 + 2 + 1 + 1 + 1; // 7
    let theirs_count = 2 + 3 + 1 + 2 + 1; // 9
    let maps = build_three_way_conflict_maps(&segments, base_count, ours_count, theirs_count);

    // Verify binary search matches per-line map for each side [base, ours, theirs].
    for (side_ix, count) in [(0, base_count), (1, ours_count), (2, theirs_count)] {
        for line in 0..count {
            let from_map = maps.line_conflict_maps[side_ix][line];
            let from_search = conflict_index_for_line(&maps.conflict_ranges[side_ix], line);
            assert_eq!(from_map, from_search, "side {side_ix} line {line}");
        }
    }
}

// -- ThreeWayVisibleProjection tests --

#[test]
fn projection_matches_visible_map_identity() {
    let segments = vec![
        ConflictSegment::Text("a\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".into(),
            theirs: "x\ny\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("d\ne\n".into()),
    ];
    let ranges = vec![1..3];
    let map = build_three_way_visible_map(5, &ranges, &segments, false);
    let proj = build_three_way_visible_projection(5, &ranges, &segments, false);
    assert_eq!(proj.len(), map.len());
    for (i, item) in map.iter().enumerate() {
        assert_eq!(proj.get(i), Some(*item), "mismatch at visible index {i}");
    }
}

#[test]
fn projection_matches_visible_map_with_collapsed() {
    let segments = vec![
        ConflictSegment::Text("a\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".into(),
            theirs: "x\ny\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("d\ne\n".into()),
    ];
    let ranges = vec![1..3];
    let map = build_three_way_visible_map(5, &ranges, &segments, true);
    let proj = build_three_way_visible_projection(5, &ranges, &segments, true);
    assert_eq!(proj.len(), map.len());
    for (i, item) in map.iter().enumerate() {
        assert_eq!(proj.get(i), Some(*item), "mismatch at visible index {i}");
    }
}

#[test]
fn projection_matches_visible_map_with_large_block_gap() {
    // Build a conflict block larger than LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES.
    let big_block_lines = LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 100;
    let big_text: String = (0..big_block_lines).map(|i| format!("line{i}\n")).collect();
    let segments = vec![
        ConflictSegment::Text("before\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: big_text.clone().into(),
            theirs: big_text.into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("after\n".into()),
    ];
    let total_lines = 1 + big_block_lines + 1;
    let ranges = vec![1..(1 + big_block_lines)];
    let map = build_three_way_visible_map(total_lines, &ranges, &segments, false);
    let proj = build_three_way_visible_projection(total_lines, &ranges, &segments, false);
    assert_eq!(
        proj.len(),
        map.len(),
        "projection and map should have same length"
    );
    // Both builders should show all lines for a large unresolved block.
    assert_eq!(proj.len(), total_lines, "all lines should be visible");
    for (i, item) in map.iter().enumerate() {
        assert_eq!(proj.get(i), Some(*item), "mismatch at visible index {i}");
    }
}

#[test]
fn projection_visible_index_for_conflict() {
    let segments = vec![
        ConflictSegment::Text("a\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".into(),
            theirs: "x\ny\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("d\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "e\n".into(),
            theirs: "z\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: false,
        }),
    ];
    let ranges = vec![1..3, 4..5];
    let proj = build_three_way_visible_projection(5, &ranges, &segments, true);
    let map = build_three_way_visible_map(5, &ranges, &segments, true);
    // First conflict should be collapsed.
    let proj_vi = proj.visible_index_for_conflict(&ranges, 0);
    let map_vi = visible_index_for_conflict(&map, &ranges, 0);
    assert_eq!(proj_vi, map_vi);
    // Second conflict should be expanded.
    let proj_vi = proj.visible_index_for_conflict(&ranges, 1);
    let map_vi = visible_index_for_conflict(&map, &ranges, 1);
    assert_eq!(proj_vi, map_vi);
}

#[test]
fn projection_out_of_bounds_returns_none() {
    let proj = ThreeWayVisibleProjection::default();
    assert_eq!(proj.get(0), None);
    assert_eq!(proj.get(100), None);
    assert_eq!(proj.len(), 0);
}

#[test]
fn projection_multiple_collapsed_blocks() {
    let segments = vec![
        ConflictSegment::Text("a\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".into(),
            theirs: "x\ny\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "d\ne\nf\n".into(),
            theirs: "p\nq\nr\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        }),
        ConflictSegment::Text("end\n".into()),
    ];
    let ranges = vec![1..3, 4..7];
    let total = 8;
    let map = build_three_way_visible_map(total, &ranges, &segments, true);
    let proj = build_three_way_visible_projection(total, &ranges, &segments, true);
    assert_eq!(proj.len(), map.len());
    for (i, item) in map.iter().enumerate() {
        assert_eq!(proj.get(i), Some(*item), "mismatch at visible index {i}");
    }
}

// ---------------------------------------------------------------------------
// Phase 3: ConflictSplitRowIndex and TwoWaySplitProjection tests
// ---------------------------------------------------------------------------
