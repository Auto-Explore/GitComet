use super::*;

#[test]
fn map_two_way_rows_to_conflicts_tracks_conflict_indices() {
    let markers = concat!(
        "a\n",
        "<<<<<<< HEAD\n",
        "b\n",
        "=======\n",
        "B\n",
        ">>>>>>> other\n",
        "mid\n",
        "<<<<<<< HEAD\n",
        "c\n",
        "=======\n",
        "C\n",
        ">>>>>>> other\n",
        "z\n",
    );
    let segments = parse_conflict_markers(markers);
    let diff_rows =
        gitcomet_core::file_diff::side_by_side_rows("a\nb\nmid\nc\nz\n", "a\nB\nmid\nC\nz\n");
    let inline_rows = build_inline_rows(&diff_rows);
    let (split_map, inline_map) =
        map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);

    let split_conflicts: Vec<usize> = split_map.iter().flatten().copied().collect();
    let inline_conflicts: Vec<usize> = inline_map.iter().flatten().copied().collect();

    assert_eq!(split_conflicts, vec![0, 1]);
    assert_eq!(inline_conflicts, vec![0, 0, 1, 1]);
}

#[test]
fn map_two_way_rows_to_conflicts_maps_single_sided_rows() {
    let markers = "<<<<<<< HEAD\n=======\nadd\n>>>>>>> other\n";
    let segments = parse_conflict_markers(markers);
    let diff_rows = gitcomet_core::file_diff::side_by_side_rows("", "add\n");
    let inline_rows = build_inline_rows(&diff_rows);
    let (split_map, inline_map) =
        map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);

    assert_eq!(split_map, vec![Some(0)]);
    assert_eq!(inline_map, vec![Some(0)]);
}

#[test]
fn build_three_way_conflict_maps_tracks_column_conflict_indices() {
    let markers = concat!(
        "ctx\n",
        "<<<<<<< HEAD\n",
        "ours-a\nours-b\n",
        "||||||| base\n",
        "base-a\n",
        "=======\n",
        "theirs-a\n",
        ">>>>>>> other\n",
        "mid\n",
        "<<<<<<< HEAD\n",
        "ours-c\n",
        "||||||| base\n",
        "base-b\nbase-c\n",
        "=======\n",
        "theirs-b\ntheirs-c\n",
        ">>>>>>> other\n",
        "tail\n",
    );
    let segments = parse_conflict_markers(markers);
    let maps = build_three_way_conflict_maps(&segments, 6, 6, 6);

    // [base, ours, theirs]
    assert_eq!(maps.conflict_ranges[1], vec![1..3, 4..5]);
    assert_eq!(
        maps.line_conflict_maps[0],
        vec![None, Some(0), None, Some(1), Some(1), None]
    );
    assert_eq!(
        maps.line_conflict_maps[1],
        vec![None, Some(0), Some(0), None, Some(1), None]
    );
    assert_eq!(
        maps.line_conflict_maps[2],
        vec![None, Some(0), None, Some(1), Some(1), None]
    );
    assert_eq!(maps.conflict_has_base, vec![true, true]);
}

#[test]
fn build_three_way_conflict_maps_handles_single_sided_and_no_base_blocks() {
    let markers = concat!(
        "ctx\n",
        "<<<<<<< HEAD\n",
        "=======\n",
        "theirs-a\ntheirs-b\n",
        ">>>>>>> other\n",
        "tail\n",
    );
    let segments = parse_conflict_markers(markers);
    let maps = build_three_way_conflict_maps(&segments, 3, 2, 4);

    assert_eq!(maps.conflict_ranges[1], vec![1..1]);
    assert_eq!(maps.line_conflict_maps[0], vec![None, None, None]);
    assert_eq!(maps.line_conflict_maps[1], vec![None, None]);
    assert_eq!(
        maps.line_conflict_maps[2],
        vec![None, Some(0), Some(0), None]
    );
    assert_eq!(maps.conflict_has_base, vec![false]);
}

#[test]
fn build_three_way_conflict_maps_without_line_maps_keeps_only_compact_metadata() {
    let markers = concat!(
        "ctx\n",
        "<<<<<<< HEAD\n",
        "ours-a\nours-b\n",
        "||||||| base\n",
        "base-a\n",
        "=======\n",
        "theirs-a\n",
        ">>>>>>> other\n",
        "tail\n",
    );
    let segments = parse_conflict_markers(markers);
    let full = build_three_way_conflict_maps(&segments, 3, 4, 3);
    let compact = build_three_way_conflict_maps_without_line_maps(&segments, 3, 4, 3);

    assert_eq!(compact.conflict_ranges, full.conflict_ranges);
    assert_eq!(compact.conflict_has_base, full.conflict_has_base);
    assert!(compact.line_conflict_maps[0].is_empty());
    assert!(compact.line_conflict_maps[1].is_empty());
    assert!(compact.line_conflict_maps[2].is_empty());
}

#[test]
fn two_way_visible_indices_hide_only_resolved_conflict_rows() {
    let segments = vec![
        ConflictSegment::Text("a\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\n".into(),
            theirs: "B\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "c\n".into(),
            theirs: "C\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
    ];
    let row_conflict_map = vec![None, Some(0), Some(0), None, Some(1), Some(1)];

    assert_eq!(
        build_two_way_visible_indices(&row_conflict_map, &segments, false),
        vec![0, 1, 2, 3, 4, 5]
    );
    assert_eq!(
        build_two_way_visible_indices(&row_conflict_map, &segments, true),
        vec![0, 3, 4, 5]
    );
}

// -- hide-resolved visible map tests --

fn build_three_way_visible_map_legacy(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> Vec<ThreeWayVisibleItem> {
    if !hide_resolved {
        return (0..total_lines).map(ThreeWayVisibleItem::Line).collect();
    }

    let resolved_blocks: Vec<bool> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b.resolved),
            _ => None,
        })
        .collect();

    let mut visible = Vec::with_capacity(total_lines);
    let mut line = 0usize;
    while line < total_lines {
        if let Some((range_ix, range)) = conflict_ranges
            .iter()
            .enumerate()
            .find(|(_, r)| r.contains(&line))
            .filter(|(ri, _)| resolved_blocks.get(*ri).copied().unwrap_or(false))
        {
            visible.push(ThreeWayVisibleItem::CollapsedBlock(range_ix));
            line = range.end;
            continue;
        }
        visible.push(ThreeWayVisibleItem::Line(line));
        line += 1;
    }
    visible
}

#[test]
fn visible_map_identity_when_not_hiding() {
    // 3 lines of text, 1 conflict with 2 lines = 5 total lines
    // conflict range: 1..3
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
    let ranges = [1..3];
    let map = build_three_way_visible_map(5, &ranges, &segments, false);
    assert_eq!(map.len(), 5);
    for (i, item) in map.iter().enumerate() {
        assert_eq!(*item, ThreeWayVisibleItem::Line(i));
    }
}

#[test]
fn visible_map_collapses_resolved_block() {
    let segments = vec![
        ConflictSegment::Text("a\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".into(),
            theirs: "x\ny\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true, // resolved
        }),
        ConflictSegment::Text("d\ne\n".into()),
    ];
    let ranges = [1..3];
    let map = build_three_way_visible_map(5, &ranges, &segments, true);
    // Should be: Line(0), CollapsedBlock(0), Line(3), Line(4)
    assert_eq!(map.len(), 4);
    assert_eq!(map[0], ThreeWayVisibleItem::Line(0));
    assert_eq!(map[1], ThreeWayVisibleItem::CollapsedBlock(0));
    assert_eq!(map[2], ThreeWayVisibleItem::Line(3));
    assert_eq!(map[3], ThreeWayVisibleItem::Line(4));
}

#[test]
fn visible_map_keeps_unresolved_blocks_expanded() {
    let segments = vec![
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\nb\n".into(),
            theirs: "x\ny\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false, // unresolved — keep expanded
        }),
        ConflictSegment::Text("c\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "d\n".into(),
            theirs: "z\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: true, // resolved — collapse
        }),
    ];
    let ranges = vec![0..2, 3..4];
    let map = build_three_way_visible_map(4, &ranges, &segments, true);
    // Unresolved block: Line(0), Line(1)
    // Text: Line(2)
    // Resolved block: CollapsedBlock(1)
    assert_eq!(map.len(), 4);
    assert_eq!(map[0], ThreeWayVisibleItem::Line(0));
    assert_eq!(map[1], ThreeWayVisibleItem::Line(1));
    assert_eq!(map[2], ThreeWayVisibleItem::Line(2));
    assert_eq!(map[3], ThreeWayVisibleItem::CollapsedBlock(1));
}

#[test]
fn visible_map_matches_legacy_scan_with_empty_and_trailing_ranges() {
    let segments = vec![
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".into(),
            theirs: "A\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".into(),
            theirs: "B\nC\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "d\ne\n".into(),
            theirs: "D\nE\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        }),
    ];
    let ranges = vec![0..0, 1..3, 4..6, 7..9];
    let linear = build_three_way_visible_map(9, &ranges, &segments, true);
    let legacy = build_three_way_visible_map_legacy(9, &ranges, &segments, true);
    assert_eq!(linear, legacy);
    assert_eq!(
        linear,
        vec![
            ThreeWayVisibleItem::Line(0),
            ThreeWayVisibleItem::Line(1),
            ThreeWayVisibleItem::Line(2),
            ThreeWayVisibleItem::Line(3),
            ThreeWayVisibleItem::CollapsedBlock(2),
            ThreeWayVisibleItem::Line(6),
            ThreeWayVisibleItem::Line(7),
            ThreeWayVisibleItem::Line(8),
        ]
    );
}

#[test]
fn visible_map_linear_walk_outpaces_legacy_scan() {
    use std::time::Instant;

    let conflict_count = 5_000usize;
    let total_lines = conflict_count.saturating_mul(2).saturating_add(1);
    let ranges: Vec<std::ops::Range<usize>> = (0..conflict_count)
        .map(|ix| {
            let start = ix.saturating_mul(2).saturating_add(1);
            start..start.saturating_add(1)
        })
        .collect();
    let segments: Vec<ConflictSegment> = (0..conflict_count)
        .map(|ix| {
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "ours\n".into(),
                theirs: "theirs\n".into(),
                choice: ConflictChoice::Ours,
                resolved: ix % 3 != 0,
            })
        })
        .collect();

    let linear_map = build_three_way_visible_map(total_lines, &ranges, &segments, true);
    let legacy_map = build_three_way_visible_map_legacy(total_lines, &ranges, &segments, true);
    assert_eq!(linear_map, legacy_map);

    let iterations = 6usize;

    let linear_start = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(build_three_way_visible_map(
            total_lines,
            &ranges,
            &segments,
            true,
        ));
    }
    let linear_elapsed = linear_start.elapsed();

    let legacy_start = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(build_three_way_visible_map_legacy(
            total_lines,
            &ranges,
            &segments,
            true,
        ));
    }
    let legacy_elapsed = legacy_start.elapsed();

    let linear_ns = linear_elapsed.as_nanos().max(1);
    let legacy_ns = legacy_elapsed.as_nanos().max(1);
    assert!(
        linear_ns.saturating_mul(4) < legacy_ns,
        "expected linear walk to be >=4x faster than legacy scan, got linear={linear_elapsed:?}, legacy={legacy_elapsed:?}"
    );
}

#[test]
fn visible_index_for_conflict_finds_collapsed() {
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
    ];
    let ranges = [1..3];
    let map = build_three_way_visible_map(4, &ranges, &segments, true);
    // map: Line(0), CollapsedBlock(0), Line(3)
    let vi = visible_index_for_conflict(&map, &ranges, 0);
    assert_eq!(vi, Some(1)); // CollapsedBlock is at visible index 1
}

#[test]
fn visible_index_for_conflict_finds_expanded() {
    let segments = vec![
        ConflictSegment::Text("a\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".into(),
            theirs: "x\ny\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
    ];
    let ranges = [1..3];
    let map = build_three_way_visible_map(3, &ranges, &segments, false);
    // map: Line(0), Line(1), Line(2)
    let vi = visible_index_for_conflict(&map, &ranges, 0);
    assert_eq!(vi, Some(1)); // First line of conflict at visible index 1
}

// -- Pass 2 subchunk splitting tests --

#[test]
fn pass2_splits_block_with_nonoverlapping_changes() {
    // 3-way conflict: ours changes line 1, theirs changes line 3.
    // Line 2 is context. Should split into resolved parts.
    let input = concat!(
        "ctx\n",
        "<<<<<<< HEAD\n",
        "AAA\nbbb\nccc\n",
        "||||||| base\n",
        "aaa\nbbb\nccc\n",
        "=======\n",
        "aaa\nbbb\nCCC\n",
        ">>>>>>> other\n",
        "end\n",
    );
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);

    // Pass 1 can't resolve (both sides changed differently).
    assert_eq!(auto_resolve_segments(&mut segments), 0);

    // Pass 2 should split the block.
    let split = auto_resolve_segments_pass2(&mut segments);
    assert_eq!(split, 1);

    // Original 1-block conflict is now gone (split into text + smaller blocks or all text).
    // Since ours changes line 1 and theirs changes line 3, non-overlapping →
    // all subchunks resolved → no more Block segments.
    assert_eq!(conflict_count(&segments), 0);

    // Resolved text should be the merged result.
    let text = generate_resolved_text(&segments);
    assert_eq!(text, "ctx\nAAA\nbbb\nCCC\nend\n");
}

#[test]
fn pass2_splits_block_with_partial_conflict() {
    // Both sides change line 2, but line 1 and 3 are only changed by one side.
    let input = concat!(
        "<<<<<<< HEAD\n",
        "AAA\nBBB\nccc\n",
        "||||||| base\n",
        "aaa\nbbb\nccc\n",
        "=======\n",
        "aaa\nYYY\nCCC\n",
        ">>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);

    let split = auto_resolve_segments_pass2(&mut segments);
    assert_eq!(split, 1);

    // Should now have 1 smaller conflict block (line 2: BBB vs YYY)
    // and resolved text for lines 1 and 3.
    let blocks: Vec<_> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(blocks.len(), 1, "should have 1 remaining conflict");
    assert_eq!(blocks[0].ours, "BBB\n");
    assert_eq!(blocks[0].theirs, "YYY\n");
    assert_eq!(blocks[0].base.as_deref(), Some("bbb\n"));
}

#[test]
fn pass2_with_region_indices_preserves_parent_region_mapping() {
    let input = concat!(
        "<<<<<<< HEAD\n",
        "AAA\nBBB\nccc\n",
        "||||||| base\n",
        "aaa\nbbb\nccc\n",
        "=======\n",
        "aaa\nYYY\nCCC\n",
        ">>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    let mut region_indices = vec![42];

    let split = auto_resolve_segments_pass2_with_region_indices(&mut segments, &mut region_indices);
    assert_eq!(split, 1);
    assert_eq!(conflict_count(&segments), 1);
    assert_eq!(region_indices, vec![42]);
}

#[test]
fn pass2_no_base_skips_block() {
    // 2-way markers (no base) — Pass 2 can't split without a base.
    let input = "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\n";
    let mut segments = parse_conflict_markers(input);
    let split = auto_resolve_segments_pass2(&mut segments);
    assert_eq!(split, 0);
    assert_eq!(conflict_count(&segments), 1);
}

#[test]
fn pass2_skips_already_resolved() {
    let input = concat!(
        "<<<<<<< HEAD\n",
        "AAA\nbbb\nccc\n",
        "||||||| base\n",
        "aaa\nbbb\nccc\n",
        "=======\n",
        "aaa\nbbb\nCCC\n",
        ">>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);

    // Resolve manually first.
    if let Some(ConflictSegment::Block(block)) = segments
        .iter_mut()
        .find(|s| matches!(s, ConflictSegment::Block(_)))
    {
        block.resolved = true;
    }

    // Pass 2 should skip resolved blocks.
    let split = auto_resolve_segments_pass2(&mut segments);
    assert_eq!(split, 0);
}

#[test]
fn pass2_merges_adjacent_text_segments() {
    // After splitting, resolved subchunks adjacent to existing Text segments
    // should be merged for cleanliness.
    let input = concat!(
        "before\n",
        "<<<<<<< HEAD\n",
        "AAA\nbbb\n",
        "||||||| base\n",
        "aaa\nbbb\n",
        "=======\n",
        "aaa\nBBB\n",
        ">>>>>>> other\n",
        "after\n",
    );
    let mut segments = parse_conflict_markers(input);
    auto_resolve_segments_pass2(&mut segments);

    // Non-overlapping changes → fully merged → no blocks remain.
    assert_eq!(conflict_count(&segments), 0);

    // All text should be merged into as few Text segments as possible.
    let text_count = segments
        .iter()
        .filter(|s| matches!(s, ConflictSegment::Text(_)))
        .count();
    // "before\n" + merged subchunks + "after\n" — exact count depends on
    // merging, but should be compact.
    assert!(text_count <= 3, "should have at most 3 text segments");
}

// -- History-aware auto-resolve tests --

#[test]
fn history_auto_resolve_merges_changelog_block() {
    use gitcomet_core::conflict_session::HistoryAutosolveOptions;

    // Simulate a conflict in a changelog section.
    let input = concat!(
        "# README\n",
        "<<<<<<< HEAD\n",
        "# Changes\n",
        "- Added feature A\n",
        "- Existing entry\n",
        "||||||| base\n",
        "# Changes\n",
        "- Existing entry\n",
        "=======\n",
        "# Changes\n",
        "- Fixed bug B\n",
        "- Existing entry\n",
        ">>>>>>> other\n",
        "# Footer\n",
    );
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);

    let options = HistoryAutosolveOptions::bullet_list();
    let resolved = auto_resolve_segments_history(&mut segments, &options);
    assert_eq!(resolved, 1);
    assert_eq!(conflict_count(&segments), 0);

    let text = generate_resolved_text(&segments);
    assert!(text.contains("- Added feature A"), "ours' new entry");
    assert!(text.contains("- Fixed bug B"), "theirs' new entry");
    assert!(text.contains("- Existing entry"), "common entry");
    assert_eq!(
        text.matches("- Existing entry").count(),
        1,
        "deduped common entry"
    );
}

#[test]
fn history_auto_resolve_with_region_indices_drops_materialized_block_mapping() {
    use gitcomet_core::conflict_session::HistoryAutosolveOptions;

    let input = concat!(
        "<<<<<<< HEAD\n",
        "# Changes\n",
        "- Added feature A\n",
        "- Existing entry\n",
        "||||||| base\n",
        "# Changes\n",
        "- Existing entry\n",
        "=======\n",
        "# Changes\n",
        "- Fixed bug B\n",
        "- Existing entry\n",
        ">>>>>>> other\n",
        "middle\n",
        "<<<<<<< HEAD\n",
        "left\n",
        "||||||| base\n",
        "base\n",
        "=======\n",
        "right\n",
        ">>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    let mut region_indices = vec![11, 22];
    let options = HistoryAutosolveOptions::bullet_list();

    let resolved = auto_resolve_segments_history_with_region_indices(
        &mut segments,
        &options,
        &mut region_indices,
    );
    assert_eq!(resolved, 1);
    assert_eq!(conflict_count(&segments), 1);
    assert_eq!(region_indices, vec![22]);
}

#[test]
fn history_auto_resolve_skips_non_changelog_blocks() {
    use gitcomet_core::conflict_session::HistoryAutosolveOptions;

    // Regular code conflict, no changelog markers.
    let input = concat!(
        "<<<<<<< HEAD\n",
        "let x = 1;\n",
        "=======\n",
        "let x = 2;\n",
        ">>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    let options = HistoryAutosolveOptions::bullet_list();
    let resolved = auto_resolve_segments_history(&mut segments, &options);
    assert_eq!(resolved, 0);
    assert_eq!(conflict_count(&segments), 1);
}

#[test]
fn history_auto_resolve_skips_already_resolved() {
    use gitcomet_core::conflict_session::HistoryAutosolveOptions;

    let input = concat!(
        "<<<<<<< HEAD\n",
        "# Changes\n- New\n",
        "=======\n",
        "# Changes\n- Other\n",
        ">>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    // Resolve manually first.
    if let Some(ConflictSegment::Block(block)) = segments
        .iter_mut()
        .find(|s| matches!(s, ConflictSegment::Block(_)))
    {
        block.resolved = true;
    }

    let options = HistoryAutosolveOptions::bullet_list();
    let resolved = auto_resolve_segments_history(&mut segments, &options);
    assert_eq!(resolved, 0);
}

// -- bulk-pick + hide-resolved interaction tests --

#[test]
fn bulk_pick_then_three_way_visible_map_collapses_all_resolved() {
    // Scenario: 3 conflicts with context. Resolve block 0 manually, then bulk-pick
    // remaining. The three-way visible map should collapse all 3 blocks.
    let input = concat!(
        "ctx\n",                                    // line 0
        "<<<<<<< HEAD\nA\n=======\na\n>>>>>>> o\n", // conflict 0, lines 1..2
        "mid\n",                                    // line 3 (after conflict)
        "<<<<<<< HEAD\nB\n=======\nb\n>>>>>>> o\n", // conflict 1, lines 4..5
        "mid2\n",                                   // line 6
        "<<<<<<< HEAD\nC\n=======\nc\n>>>>>>> o\n", // conflict 2, lines 7..8
        "end\n",                                    // line 9
    );
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 3);

    // Manually resolve block 0
    mark_block_resolved(&mut segments, 0);

    // Bulk-pick remaining → blocks 1 and 2 become resolved
    let updated = apply_choice_to_unresolved_segments(&mut segments, ConflictChoice::Ours);
    assert_eq!(updated, 2);
    assert_eq!(resolved_conflict_count(&segments), 3);

    // Now rebuild the three-way visible map with hide_resolved=true.
    // Each conflict block is 2 lines (ours side), ranges are:
    //   block 0: 1..3, block 1: 4..6, block 2: 7..9
    // Total lines in the three-way view: 10
    let conflict_ranges = [1..3, 4..6, 7..9];
    let map = build_three_way_visible_map(10, &conflict_ranges, &segments, true);

    // Expect: Line(0), Collapsed(0), Line(3), Collapsed(1), Line(6), Collapsed(2), Line(9)
    assert_eq!(map.len(), 7);
    assert_eq!(map[0], ThreeWayVisibleItem::Line(0));
    assert_eq!(map[1], ThreeWayVisibleItem::CollapsedBlock(0));
    assert_eq!(map[2], ThreeWayVisibleItem::Line(3));
    assert_eq!(map[3], ThreeWayVisibleItem::CollapsedBlock(1));
    assert_eq!(map[4], ThreeWayVisibleItem::Line(6));
    assert_eq!(map[5], ThreeWayVisibleItem::CollapsedBlock(2));
    assert_eq!(map[6], ThreeWayVisibleItem::Line(9));
}

#[test]
fn bulk_pick_then_two_way_visible_indices_hides_all_resolved() {
    // Two-way variant: after bulk pick, all conflict rows should be hidden.
    let mut segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "A\n".into(),
            theirs: "a\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "B\n".into(),
            theirs: "b\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("end\n".into()),
    ];
    // row indices: 0=ctx, 1,2=block0(ours+theirs), 3=mid, 4,5=block1, 6=end
    let row_conflict_map: Vec<Option<usize>> =
        vec![None, Some(0), Some(0), None, Some(1), Some(1), None];

    // Before bulk pick: all rows visible
    assert_eq!(
        build_two_way_visible_indices(&row_conflict_map, &segments, true).len(),
        7
    );

    // Bulk pick resolves both blocks
    let updated = apply_choice_to_unresolved_segments(&mut segments, ConflictChoice::Theirs);
    assert_eq!(updated, 2);

    // After bulk pick with hide_resolved=true: conflict rows hidden
    let visible = build_two_way_visible_indices(&row_conflict_map, &segments, true);
    assert_eq!(visible, vec![0, 3, 6]); // only context rows
}

#[test]
fn autosolve_then_three_way_visible_map_collapses_autoresolved() {
    // Auto-resolve should cause the same collapse behavior as manual picks
    // when hide_resolved is active.
    let input = concat!(
        "ctx\n",
        "<<<<<<< HEAD\nsame\n||||||| base\norig\n=======\nsame\n>>>>>>> o\n",
        "mid\n",
        "<<<<<<< HEAD\nX\n||||||| base\norig2\n=======\nY\n>>>>>>> o\n",
        "end\n",
    );
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 2);

    // Block 0: ours==theirs → autosolve resolves it
    // Block 1: both changed differently → stays unresolved
    let resolved = auto_resolve_segments(&mut segments);
    assert_eq!(resolved, 1);
    assert_eq!(resolved_conflict_count(&segments), 1);

    // Three-way: ctx(0), block0(1), mid(2), block1(3), end(4) → total 5
    let conflict_ranges = [1..2, 3..4];
    let map = build_three_way_visible_map(5, &conflict_ranges, &segments, true);
    assert_eq!(map[0], ThreeWayVisibleItem::Line(0));
    assert_eq!(map[1], ThreeWayVisibleItem::CollapsedBlock(0)); // autoresolved
    assert_eq!(map[2], ThreeWayVisibleItem::Line(2)); // mid
    assert_eq!(map[3], ThreeWayVisibleItem::Line(3)); // unresolved block stays expanded
    assert_eq!(map[4], ThreeWayVisibleItem::Line(4)); // end
}

// -- counter/navigation correctness after sequential picks --

#[test]
fn navigation_updates_correctly_after_sequential_picks() {
    // Start with 3 unresolved blocks, resolve them one-by-one,
    // verify navigation at each step.
    let input = concat!(
        "<<<<<<< HEAD\nA\n=======\na\n>>>>>>> o\n",
        "<<<<<<< HEAD\nB\n=======\nb\n>>>>>>> o\n",
        "<<<<<<< HEAD\nC\n=======\nc\n>>>>>>> o\n",
    );
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 3);

    // All unresolved: next from 0 → 1, prev from 0 → 2 (wrap)
    assert_eq!(next_unresolved_conflict_index(&segments, 0), Some(1));
    assert_eq!(prev_unresolved_conflict_index(&segments, 0), Some(2));

    // Resolve block 1 (middle)
    mark_block_resolved(&mut segments, 1);
    assert_eq!(resolved_conflict_count(&segments), 1);
    // Next from 0 should skip block 1, go to 2
    assert_eq!(next_unresolved_conflict_index(&segments, 0), Some(2));
    // Prev from 2 should skip block 1, go to 0
    assert_eq!(prev_unresolved_conflict_index(&segments, 2), Some(0));

    // Resolve block 0 (first)
    mark_block_resolved(&mut segments, 0);
    assert_eq!(resolved_conflict_count(&segments), 2);
    // Only block 2 is unresolved
    assert_eq!(next_unresolved_conflict_index(&segments, 0), Some(2));
    assert_eq!(next_unresolved_conflict_index(&segments, 1), Some(2));
    assert_eq!(next_unresolved_conflict_index(&segments, 2), Some(2));
    assert_eq!(prev_unresolved_conflict_index(&segments, 0), Some(2));

    // Resolve last block
    mark_block_resolved(&mut segments, 2);
    assert_eq!(resolved_conflict_count(&segments), 3);
    assert_eq!(next_unresolved_conflict_index(&segments, 0), None);
    assert_eq!(prev_unresolved_conflict_index(&segments, 0), None);
}

#[test]
fn resolved_counter_consistent_with_visible_map_after_incremental_picks() {
    // Ensure the resolved count and visible map stay in sync as
    // conflicts are resolved one by one. Uses multi-line conflicts so
    // collapsing them visibly reduces the visible row count.
    let mut segments = vec![
        ConflictSegment::Text("pre\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("orig1\norig1b\n".into()),
            ours: "A\nA2\n".into(),
            theirs: "a\na2\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("orig2\norig2b\norig2c\n".into()),
            ours: "B\nB2\nB3\n".into(),
            theirs: "b\nb2\nb3\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("post\n".into()),
    ];
    // Layout: pre(0), block0(1..3), mid(3), block1(4..7), post(7) → total 8
    let conflict_ranges = [1..3, 4..7];
    let total_lines = 8;

    // Step 0: nothing resolved — all lines visible
    assert_eq!(resolved_conflict_count(&segments), 0);
    let map = build_three_way_visible_map(total_lines, &conflict_ranges, &segments, true);
    assert_eq!(map.len(), 8);
    assert!(
        map.iter()
            .all(|item| matches!(item, ThreeWayVisibleItem::Line(_)))
    );

    // Step 1: resolve block 0 (2 lines → 1 collapsed row)
    mark_block_resolved(&mut segments, 0);
    assert_eq!(resolved_conflict_count(&segments), 1);
    let map = build_three_way_visible_map(total_lines, &conflict_ranges, &segments, true);
    // pre(0), [collapsed0], mid(3), block1-lines(4,5,6), post(7) = 7 items
    assert_eq!(map.len(), 7);
    assert_eq!(map[0], ThreeWayVisibleItem::Line(0));
    assert_eq!(map[1], ThreeWayVisibleItem::CollapsedBlock(0));
    assert_eq!(map[2], ThreeWayVisibleItem::Line(3));

    // Step 2: resolve block 1 (3 lines → 1 collapsed row)
    mark_block_resolved(&mut segments, 1);
    assert_eq!(resolved_conflict_count(&segments), 2);
    let map = build_three_way_visible_map(total_lines, &conflict_ranges, &segments, true);
    // pre(0), [collapsed0], mid(3), [collapsed1], post(7) = 5 items
    assert_eq!(map.len(), 5);
    assert_eq!(map[1], ThreeWayVisibleItem::CollapsedBlock(0));
    assert_eq!(map[3], ThreeWayVisibleItem::CollapsedBlock(1));
}

#[test]
fn large_unresolved_three_way_visible_map_shows_all_lines() {
    let range_len = LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 17;
    let conflict_start = 1usize;
    let conflict_end = conflict_start + range_len;
    let total_lines = conflict_end + 1;
    let conflict_ranges = [conflict_start..conflict_end];
    let segments = vec![
        ConflictSegment::Text("pre\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".into()),
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("post\n".into()),
    ];

    // Visible map shows every line — no gaps.
    let map = build_three_way_visible_map(total_lines, &conflict_ranges, &segments, false);
    assert_eq!(map.len(), total_lines, "all lines should be visible");
    assert!(
        map.iter()
            .all(|item| matches!(item, ThreeWayVisibleItem::Line(_))),
        "all items should be Line variants, no gaps"
    );
    assert_eq!(
        visible_index_for_conflict(&map, &conflict_ranges, 0),
        Some(1)
    );
}

#[test]
fn three_way_projection_exposes_full_large_unresolved_block() {
    let range_len = LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 17;
    let conflict_start = 1usize;
    let conflict_end = conflict_start + range_len;
    let total_lines = conflict_end + 1;
    let conflict_ranges = [conflict_start..conflict_end];
    let segments = vec![
        ConflictSegment::Text("pre\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".into()),
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("post\n".into()),
    ];

    for hide_resolved in [false, true] {
        let projection = build_three_way_visible_projection(
            total_lines,
            &conflict_ranges,
            &segments,
            hide_resolved,
        );

        assert_eq!(
            projection.len(),
            total_lines,
            "streamed projection should keep every unresolved line visible",
        );
        assert_eq!(
            projection.visible_index_for_conflict(&conflict_ranges, 0),
            Some(conflict_start),
        );
        assert_eq!(projection.get(0), Some(ThreeWayVisibleItem::Line(0)));
        assert_eq!(
            projection.get(conflict_start),
            Some(ThreeWayVisibleItem::Line(conflict_start))
        );
        assert_eq!(
            projection.get(conflict_start + LARGE_CONFLICT_BLOCK_PREVIEW_LINES + 10),
            Some(ThreeWayVisibleItem::Line(
                conflict_start + LARGE_CONFLICT_BLOCK_PREVIEW_LINES + 10
            ))
        );
        assert_eq!(
            projection.get(total_lines - 1),
            Some(ThreeWayVisibleItem::Line(total_lines - 1))
        );
    }
}

#[test]
fn select_conflict_rendering_mode_streams_non_empty_conflicts() {
    let segments =
        parse_conflict_markers("pre\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\npost\n");

    assert_eq!(
        select_conflict_rendering_mode(&segments, 4),
        ConflictRenderingMode::StreamedLargeFile
    );
}

#[test]
fn select_conflict_rendering_mode_keeps_empty_inputs_eager() {
    assert_eq!(
        select_conflict_rendering_mode(&[], LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1),
        ConflictRenderingMode::EagerSmallFile
    );
}

#[test]
fn select_conflict_rendering_mode_streams_large_files_or_blocks() {
    let small_segments =
        parse_conflict_markers("pre\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\npost\n");
    assert_eq!(
        select_conflict_rendering_mode(&small_segments, LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1),
        ConflictRenderingMode::StreamedLargeFile
    );

    let repeated = "line\n".repeat(LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1);
    let large_input = format!("<<<<<<< HEAD\n{repeated}=======\n{repeated}>>>>>>> other\n");
    let large_segments = parse_conflict_markers(&large_input);
    assert_eq!(
        select_conflict_rendering_mode(&large_segments, 1),
        ConflictRenderingMode::StreamedLargeFile
    );
}

// -- split vs inline row list consistency --

#[test]
fn split_and_inline_views_have_consistent_conflict_counts() {
    // Verify that both split and inline row conflict maps produce the
    // same set of conflict indices (the same number of distinct conflicts).
    let markers = concat!(
        "ctx\n",
        "<<<<<<< HEAD\n",
        "alpha\nbeta\n",
        "=======\n",
        "ALPHA\nBETA\n",
        ">>>>>>> other\n",
        "mid\n",
        "<<<<<<< HEAD\n",
        "gamma\n",
        "=======\n",
        "GAMMA\nDELTA\n",
        ">>>>>>> other\n",
        "end\n",
    );
    let segments = parse_conflict_markers(markers);
    assert_eq!(conflict_count(&segments), 2);

    let ours_text = "ctx\nalpha\nbeta\nmid\ngamma\nend\n";
    let theirs_text = "ctx\nALPHA\nBETA\nmid\nGAMMA\nDELTA\nend\n";
    let diff_rows = gitcomet_core::file_diff::side_by_side_rows(ours_text, theirs_text);
    let inline_rows = build_inline_rows(&diff_rows);

    let (split_map, inline_map) =
        map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);

    // Both maps should contain the same set of distinct conflict indices
    let split_indices: std::collections::BTreeSet<usize> =
        split_map.iter().flatten().copied().collect();
    let inline_indices: std::collections::BTreeSet<usize> =
        inline_map.iter().flatten().copied().collect();
    assert_eq!(split_indices, inline_indices);

    // And that set should match the actual conflict count
    assert_eq!(split_indices.len(), 2);
    assert!(split_indices.contains(&0));
    assert!(split_indices.contains(&1));
}

#[test]
fn split_and_inline_hide_resolved_filter_same_conflicts() {
    // After resolving one conflict, both split and inline visible indices
    // should filter out the same conflict's rows.
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "A\nB\n".into(),
            theirs: "a\nb\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true, // resolved
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "C\n".into(),
            theirs: "c\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false, // unresolved
        }),
        ConflictSegment::Text("end\n".into()),
    ];

    // Build split and inline maps
    let ours_text = "ctx\nA\nB\nmid\nC\nend\n";
    let theirs_text = "ctx\na\nb\nmid\nc\nend\n";
    let diff_rows = gitcomet_core::file_diff::side_by_side_rows(ours_text, theirs_text);
    let inline_rows = build_inline_rows(&diff_rows);
    let (split_map, inline_map) =
        map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);

    // With hide_resolved=true, both views should hide block 0 rows
    let split_visible = build_two_way_visible_indices(&split_map, &segments, true);
    let inline_visible = build_two_way_visible_indices(&inline_map, &segments, true);

    // Split visible should not contain any rows mapped to conflict 0
    for &ix in &split_visible {
        if let Some(ci) = split_map[ix] {
            assert_ne!(ci, 0, "split view should hide resolved conflict 0 rows");
        }
    }
    // Inline visible should not contain any rows mapped to conflict 0
    for &ix in &inline_visible {
        if let Some(ci) = inline_map[ix] {
            assert_ne!(ci, 0, "inline view should hide resolved conflict 0 rows");
        }
    }

    // Both should still show the unresolved conflict 1 rows
    let split_has_conflict_1 = split_visible.iter().any(|&ix| split_map[ix] == Some(1));
    let inline_has_conflict_1 = inline_visible.iter().any(|&ix| inline_map[ix] == Some(1));
    assert!(
        split_has_conflict_1,
        "split should show unresolved conflict 1"
    );
    assert!(
        inline_has_conflict_1,
        "inline should show unresolved conflict 1"
    );
}

#[test]
fn unresolved_conflict_indices_match_queue_order() {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "A\n".into(),
            theirs: "a\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "B\n".into(),
            theirs: "b\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "C\n".into(),
            theirs: "c\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
    ];

    assert_eq!(unresolved_conflict_indices(&segments), vec![0, 2]);
}

#[test]
fn visible_index_for_two_way_conflict_respects_hide_resolved_filter() {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "A\n".into(),
            theirs: "a\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "B\n".into(),
            theirs: "b\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("end\n".into()),
    ];
    let ours_text = "ctx\nA\nmid\nB\nend\n";
    let theirs_text = "ctx\na\nmid\nb\nend\n";
    let diff_rows = gitcomet_core::file_diff::side_by_side_rows(ours_text, theirs_text);
    let inline_rows = build_inline_rows(&diff_rows);
    let (split_map, inline_map) =
        map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);

    let split_visible = build_two_way_visible_indices(&split_map, &segments, true);
    let inline_visible = build_two_way_visible_indices(&inline_map, &segments, true);

    assert_eq!(
        visible_index_for_two_way_conflict(&split_map, &split_visible, 0),
        None
    );
    assert_eq!(
        visible_index_for_two_way_conflict(&inline_map, &inline_visible, 0),
        None
    );
    assert!(
        visible_index_for_two_way_conflict(&split_map, &split_visible, 1).is_some(),
        "unresolved conflict should remain visible in split mode"
    );
    assert!(
        visible_index_for_two_way_conflict(&inline_map, &inline_visible, 1).is_some(),
        "unresolved conflict should remain visible in inline mode"
    );
}

#[test]
fn unresolved_visible_nav_entries_for_three_way_skip_resolved_blocks_even_when_visible() {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base-a\n".into()),
            ours: "ours-a\n".into(),
            theirs: "theirs-a\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base-b\n".into()),
            ours: "ours-b\n".into(),
            theirs: "theirs-b\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        }),
        ConflictSegment::Text("tail\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base-c\n".into()),
            ours: "ours-c\n".into(),
            theirs: "theirs-c\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("end\n".into()),
    ];
    let ranges = vec![1..2, 3..4, 5..6];
    let visible_map = build_three_way_visible_map(7, &ranges, &segments, false);

    let nav_entries: Vec<usize> = unresolved_conflict_indices(&segments)
        .into_iter()
        .filter_map(|ci| visible_index_for_conflict(&visible_map, &ranges, ci))
        .collect();
    assert_eq!(nav_entries, vec![1, 5]);
}

#[test]
fn unresolved_visible_nav_entries_for_two_way_skip_resolved_conflicts() {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "A\n".into(),
            theirs: "a\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "B\n".into(),
            theirs: "b\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("end\n".into()),
    ];
    let ours_text = "ctx\nA\nmid\nB\nend\n";
    let theirs_text = "ctx\na\nmid\nb\nend\n";
    let diff_rows = gitcomet_core::file_diff::side_by_side_rows(ours_text, theirs_text);
    let inline_rows = build_inline_rows(&diff_rows);
    let (split_map, _) = map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);
    let visible_rows = build_two_way_visible_indices(&split_map, &segments, false);

    let resolved_visible =
        visible_index_for_two_way_conflict(&split_map, &visible_rows, 0).expect("visible");
    let unresolved_visible =
        visible_index_for_two_way_conflict(&split_map, &visible_rows, 1).expect("visible");

    let nav_entries =
        unresolved_visible_nav_entries_for_two_way(&segments, &split_map, &visible_rows);
    assert_eq!(nav_entries, vec![unresolved_visible]);
    assert!(!nav_entries.contains(&resolved_visible));
}

#[test]
fn two_way_conflict_index_for_visible_row_maps_back_to_conflict() {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "A\n".into(),
            theirs: "a\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "B\n".into(),
            theirs: "b\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("end\n".into()),
    ];
    let ours_text = "ctx\nA\nmid\nB\nend\n";
    let theirs_text = "ctx\na\nmid\nb\nend\n";
    let diff_rows = gitcomet_core::file_diff::side_by_side_rows(ours_text, theirs_text);
    let inline_rows = build_inline_rows(&diff_rows);
    let (split_map, _) = map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);
    let visible_rows = build_two_way_visible_indices(&split_map, &segments, false);
    let conflict_1_visible =
        visible_index_for_two_way_conflict(&split_map, &visible_rows, 1).expect("visible");

    assert_eq!(
        two_way_conflict_index_for_visible_row(&split_map, &visible_rows, conflict_1_visible),
        Some(1)
    );
    assert_eq!(
        two_way_conflict_index_for_visible_row(&split_map, &visible_rows, usize::MAX),
        None
    );
}
