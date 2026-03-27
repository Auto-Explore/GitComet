use super::*;

#[test]
fn split_row_index_single_block_plain_rows() {
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "a\nb\nc\n".into(),
        theirs: "x\ny\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);
    // Row count = max(3, 2) = 3
    assert_eq!(index.total_rows(), 3);

    // Row 0: both sides present
    let r0 = index.row_at(&segments, 0).unwrap();
    assert_eq!(r0.old, Some("a".into()));
    assert_eq!(r0.new, Some("x".into()));
    assert_eq!(r0.old_line, Some(1));
    assert_eq!(r0.new_line, Some(1));
    assert_eq!(r0.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);

    // Row 1: both present, text differs
    let r1 = index.row_at(&segments, 1).unwrap();
    assert_eq!(r1.old, Some("b".into()));
    assert_eq!(r1.new, Some("y".into()));
    assert_eq!(r1.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);

    // Row 2: only ours (theirs shorter)
    let r2 = index.row_at(&segments, 2).unwrap();
    assert_eq!(r2.old, Some("c".into()));
    assert_eq!(r2.new, None);
    assert_eq!(r2.kind, gitcomet_core::file_diff::FileDiffRowKind::Remove);

    // Row 3: out of bounds
    assert!(index.row_at(&segments, 3).is_none());
}

#[test]
fn split_row_index_context_plus_block() {
    let segments = vec![
        ConflictSegment::Text("line1\nline2\nline3\nline4\nline5\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".into(),
            theirs: "b\nc\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: false,
        }),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 2);
    // Context: last 2 lines of text (boundary before block) = "line4", "line5"
    // Block: max(1, 2) = 2
    // Total = 4
    assert_eq!(index.total_rows(), 4);

    // Context rows
    let c0 = index.row_at(&segments, 0).unwrap();
    assert_eq!(c0.old, Some("line4".into()));
    assert_eq!(c0.new, Some("line4".into()));
    assert_eq!(c0.old_line, Some(4)); // line4 is at 1-based index 4
    assert_eq!(c0.kind, gitcomet_core::file_diff::FileDiffRowKind::Context);

    let c1 = index.row_at(&segments, 1).unwrap();
    assert_eq!(c1.old, Some("line5".into()));

    // Block rows (text differs → Modify)
    let b0 = index.row_at(&segments, 2).unwrap();
    assert_eq!(b0.old, Some("a".into()));
    assert_eq!(b0.new, Some("b".into()));
    assert_eq!(b0.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);
    assert_eq!(b0.old_line, Some(6));
    assert_eq!(b0.new_line, Some(6));

    let b1 = index.row_at(&segments, 3).unwrap();
    assert_eq!(b1.old, None);
    assert_eq!(b1.new, Some("c".into()));
    assert_eq!(b1.kind, gitcomet_core::file_diff::FileDiffRowKind::Add);
}

#[test]
fn split_row_index_conflict_ix_lookup() {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".into(),
            theirs: "b\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "c\n".into(),
            theirs: "d\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: false,
        }),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 1);

    // First block
    assert_eq!(
        index.conflict_ix_for_row(index.first_row_for_conflict(0).unwrap()),
        Some(0)
    );
    // Second block
    assert_eq!(
        index.conflict_ix_for_row(index.first_row_for_conflict(1).unwrap()),
        Some(1)
    );
}

#[test]
fn split_row_index_page_cache_reuses_requested_page() {
    let line_count = CONFLICT_SPLIT_PAGE_SIZE * 3;
    let text: String = (0..line_count).map(|ix| format!("line_{ix}\n")).collect();
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: text.clone().into(),
        theirs: text.into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 0);

    assert!(index.cached_page_indices().is_empty());

    let first_page_one_row = CONFLICT_SPLIT_PAGE_SIZE + 7;
    assert!(index.row_at(&segments, first_page_one_row).is_some());
    assert_eq!(index.cached_page_indices(), vec![1]);

    assert!(index.row_at(&segments, first_page_one_row + 13).is_some());
    assert_eq!(
        index.cached_page_indices(),
        vec![1],
        "re-reading a row from the same page should not materialize new pages"
    );

    assert!(
        index
            .row_at(&segments, CONFLICT_SPLIT_PAGE_SIZE * 2)
            .is_some()
    );
    assert_eq!(index.cached_page_indices(), vec![1, 2]);
}

#[test]
fn split_row_index_page_cache_stays_bounded() {
    let total_pages = CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES + 2;
    let line_count = CONFLICT_SPLIT_PAGE_SIZE * total_pages;
    let text: String = (0..line_count).map(|ix| format!("line_{ix}\n")).collect();
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: text.clone().into(),
        theirs: text.into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 0);

    for page_ix in 0..total_pages {
        let row_ix = page_ix * CONFLICT_SPLIT_PAGE_SIZE;
        assert!(
            index.row_at(&segments, row_ix).is_some(),
            "page {page_ix} should be materialized"
        );
    }

    let expected: Vec<usize> =
        (total_pages - CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES..total_pages).collect();
    assert_eq!(
        index.cached_page_count(),
        CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES
    );
    assert_eq!(
        index.cached_page_indices(),
        expected,
        "cache should evict the oldest pages once it reaches its bounded capacity"
    );
}

#[test]
fn split_row_index_matches_eager_for_small_segments() {
    // For small segments, verify that the paged index produces the same rows
    // as the eager `block_local_two_way_diff_rows` path (minus diff alignment).
    let segments = vec![
        ConflictSegment::Text("header\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "same\n".into(),
            theirs: "same\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("footer\n".into()),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 3);
    // Should have rows: context header (1) + block (1) + context footer (1) = 3
    assert_eq!(index.total_rows(), 3);

    // All rows should be accessible
    for i in 0..index.total_rows() {
        assert!(
            index.row_at(&segments, i).is_some(),
            "row {i} should be accessible"
        );
    }
}

#[test]
fn two_way_split_projection_no_hide() {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\nb\n".into(),
            theirs: "x\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 1);
    let proj = TwoWaySplitProjection::new(&index, &segments, false);

    // Total = context(1) + block(max(2,1)) = 3
    assert_eq!(proj.visible_len(), index.total_rows());

    // All visible indices map to source rows
    for vi in 0..proj.visible_len() {
        let (source_ix, _conflict_ix) = proj.get(vi).unwrap();
        assert!(index.row_at(&segments, source_ix).is_some());
    }
}

#[test]
fn two_way_split_projection_hide_resolved() {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\nb\n".into(),
            theirs: "x\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true, // resolved!
        }),
        ConflictSegment::Text("end\n".into()),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 1);
    let proj_no_hide = TwoWaySplitProjection::new(&index, &segments, false);
    let proj_hide = TwoWaySplitProjection::new(&index, &segments, true);

    // Without hide: context(1) + block(2) + context(1) = 4
    assert_eq!(proj_no_hide.visible_len(), 4);
    // With hide: context(1) + context(1) = 2 (block hidden)
    assert_eq!(proj_hide.visible_len(), 2);
}

#[test]
fn widest_source_rows_ignore_hidden_middle_context_lines() {
    let segments = vec![
        ConflictSegment::Text("ctx\nTHIS_LINE_IS_HIDDEN_BUT_VERY_WIDE\nvisible_tail\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".into(),
            theirs: "b\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 1);

    let [ours_row, theirs_row] = index.widest_source_rows_by_text_len(&segments, false);
    let ours_row = index
        .row_at(&segments, ours_row.expect("expected a visible ours row"))
        .expect("resolved widest ours row");
    let theirs_row = index
        .row_at(
            &segments,
            theirs_row.expect("expected a visible theirs row"),
        )
        .expect("resolved widest theirs row");

    assert_eq!(ours_row.old.as_deref(), Some("visible_tail"));
    assert_eq!(theirs_row.new.as_deref(), Some("visible_tail"));
}

#[test]
fn widest_source_rows_skip_hidden_resolved_blocks() {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "THIS_RESOLVED_ROW_IS_HIDDEN_AND_VERY_WIDE\n".into(),
            theirs: "THIS_RESOLVED_ROW_IS_HIDDEN_AND_VERY_WIDE\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("visible_tail_is_widest\n".into()),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 1);

    let [ours_row, theirs_row] = index.widest_source_rows_by_text_len(&segments, true);
    let ours_row = index
        .row_at(&segments, ours_row.expect("expected a visible ours row"))
        .expect("resolved widest ours row");
    let theirs_row = index
        .row_at(
            &segments,
            theirs_row.expect("expected a visible theirs row"),
        )
        .expect("resolved widest theirs row");

    assert_eq!(ours_row.old.as_deref(), Some("visible_tail_is_widest"));
    assert_eq!(theirs_row.new.as_deref(), Some("visible_tail_is_widest"));
}

#[test]
fn two_way_split_projection_visible_ix_for_conflict() {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".into(),
            theirs: "b\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("mid\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "c\n".into(),
            theirs: "d\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: false,
        }),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 1);
    let proj = TwoWaySplitProjection::new(&index, &segments, false);

    let vis0 = proj.visible_index_for_conflict(0).unwrap();
    let vis1 = proj.visible_index_for_conflict(1).unwrap();
    assert!(vis0 < vis1);

    // The conflict_ix at the visible position should match
    let (_source, ci) = proj.get(vis0).unwrap();
    assert_eq!(ci, Some(0));
    let (_source, ci) = proj.get(vis1).unwrap();
    assert_eq!(ci, Some(1));
}

#[test]
fn split_row_index_empty_segments() {
    let segments: Vec<ConflictSegment> = vec![];
    let index = ConflictSplitRowIndex::new(&segments, 3);
    assert_eq!(index.total_rows(), 0);
    assert!(index.row_at(&segments, 0).is_none());
}

#[test]
fn split_row_index_large_block_no_diff_computation() {
    // Simulate a giant conflict block; verify the index builds without
    // running any whole-block diff computation.
    let big_ours: String = (0..1000).map(|i| format!("ours_line_{i}\n")).collect();
    let big_theirs: String = (0..800).map(|i| format!("theirs_line_{i}\n")).collect();
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: big_ours.into(),
        theirs: big_theirs.into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);
    assert_eq!(index.total_rows(), 1000); // max(1000, 800)

    // Spot-check first, middle, and last rows
    let first = index.row_at(&segments, 0).unwrap();
    assert_eq!(first.old, Some("ours_line_0".into()));
    assert_eq!(first.new, Some("theirs_line_0".into()));

    let mid = index.row_at(&segments, 500).unwrap();
    assert_eq!(mid.old, Some("ours_line_500".into()));
    assert_eq!(mid.new, Some("theirs_line_500".into()));

    let last_ours = index.row_at(&segments, 999).unwrap();
    assert_eq!(last_ours.old, Some("ours_line_999".into()));
    assert_eq!(last_ours.new, None); // theirs only has 800 lines
    assert_eq!(
        last_ours.kind,
        gitcomet_core::file_diff::FileDiffRowKind::Remove
    );
}

#[test]
fn split_row_index_positional_alignment_handles_shifted_insertions() {
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "header\nbody\ntail\n".into(),
        theirs: "header\ninserted\nbody\ntail\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);
    assert_eq!(index.total_rows(), 4);

    let header = index.row_at(&segments, 0).unwrap();
    assert_eq!(header.old.as_deref(), Some("header"));
    assert_eq!(header.new.as_deref(), Some("header"));
    assert_eq!(header.old_line, Some(1));
    assert_eq!(header.new_line, Some(1));

    let inserted = index.row_at(&segments, 1).unwrap();
    assert_eq!(inserted.old.as_deref(), Some("body"));
    assert_eq!(inserted.new.as_deref(), Some("inserted"));
    assert_eq!(inserted.old_line, Some(2));
    assert_eq!(inserted.new_line, Some(2));
    assert_eq!(
        inserted.kind,
        gitcomet_core::file_diff::FileDiffRowKind::Modify
    );

    let body = index.row_at(&segments, 2).unwrap();
    assert_eq!(body.old.as_deref(), Some("tail"));
    assert_eq!(body.new.as_deref(), Some("body"));
    assert_eq!(body.old_line, Some(3));
    assert_eq!(body.new_line, Some(3));
    assert_eq!(body.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);

    let tail = index.row_at(&segments, 3).unwrap();
    assert_eq!(tail.old, None);
    assert_eq!(tail.new.as_deref(), Some("tail"));
    assert_eq!(tail.old_line, None);
    assert_eq!(tail.new_line, Some(4));
    assert_eq!(tail.kind, gitcomet_core::file_diff::FileDiffRowKind::Add);
}

#[test]
fn split_row_index_positional_alignment_keeps_repeated_lines_reviewable() {
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "repeat\na\nrepeat\na\nrepeat\n".into(),
        theirs: "x\nrepeat\na\nrepeat\na\nrepeat\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);
    assert_eq!(index.total_rows(), 6);

    let expected_rows = [
        (
            0,
            Some("repeat"),
            Some("x"),
            Some(1),
            Some(1),
            gitcomet_core::file_diff::FileDiffRowKind::Modify,
        ),
        (
            1,
            Some("a"),
            Some("repeat"),
            Some(2),
            Some(2),
            gitcomet_core::file_diff::FileDiffRowKind::Modify,
        ),
        (
            2,
            Some("repeat"),
            Some("a"),
            Some(3),
            Some(3),
            gitcomet_core::file_diff::FileDiffRowKind::Modify,
        ),
        (
            3,
            Some("a"),
            Some("repeat"),
            Some(4),
            Some(4),
            gitcomet_core::file_diff::FileDiffRowKind::Modify,
        ),
        (
            4,
            Some("repeat"),
            Some("a"),
            Some(5),
            Some(5),
            gitcomet_core::file_diff::FileDiffRowKind::Modify,
        ),
        (
            5,
            None,
            Some("repeat"),
            None,
            Some(6),
            gitcomet_core::file_diff::FileDiffRowKind::Add,
        ),
    ];
    for (row_ix, old, new, old_line, new_line, kind) in expected_rows {
        let row = index.row_at(&segments, row_ix).unwrap();
        assert_eq!(
            row.old.as_deref(),
            old,
            "unexpected old text at row {row_ix}"
        );
        assert_eq!(
            row.new.as_deref(),
            new,
            "unexpected new text at row {row_ix}"
        );
        assert_eq!(
            row.old_line, old_line,
            "unexpected old line at row {row_ix}"
        );
        assert_eq!(
            row.new_line, new_line,
            "unexpected new line at row {row_ix}"
        );
        assert_eq!(row.kind, kind, "unexpected row kind at row {row_ix}");
    }
}

#[test]
fn split_row_index_classifies_modify_vs_context_kinds() {
    // Rows with identical text on both sides should be Context;
    // rows with differing text should be Modify.
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "same\ndiff_a\nsame2\n".into(),
        theirs: "same\ndiff_b\nsame2\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);
    assert_eq!(index.total_rows(), 3);

    let r0 = index.row_at(&segments, 0).unwrap();
    assert_eq!(r0.kind, gitcomet_core::file_diff::FileDiffRowKind::Context);
    assert_eq!(r0.old.as_deref(), Some("same"));
    assert_eq!(r0.new.as_deref(), Some("same"));

    let r1 = index.row_at(&segments, 1).unwrap();
    assert_eq!(r1.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);
    assert_eq!(r1.old.as_deref(), Some("diff_a"));
    assert_eq!(r1.new.as_deref(), Some("diff_b"));

    let r2 = index.row_at(&segments, 2).unwrap();
    assert_eq!(r2.kind, gitcomet_core::file_diff::FileDiffRowKind::Context);
    assert_eq!(r2.old.as_deref(), Some("same2"));
    assert_eq!(r2.new.as_deref(), Some("same2"));
}

#[test]
fn split_row_index_positional_alignment_keeps_inserted_block_reviewable() {
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "alpha\nbeta\ngamma\n".into(),
        theirs: "alpha\nnew1\nnew2\nbeta\ngamma\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);

    assert_eq!(index.total_rows(), 5);

    let r0 = index.row_at(&segments, 0).unwrap();
    assert_eq!(r0.kind, gitcomet_core::file_diff::FileDiffRowKind::Context);
    assert_eq!(r0.old.as_deref(), Some("alpha"));
    assert_eq!(r0.new.as_deref(), Some("alpha"));

    let r1 = index.row_at(&segments, 1).unwrap();
    assert_eq!(r1.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);
    assert_eq!(r1.old.as_deref(), Some("beta"));
    assert_eq!(r1.new.as_deref(), Some("new1"));

    let r2 = index.row_at(&segments, 2).unwrap();
    assert_eq!(r2.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);
    assert_eq!(r2.old.as_deref(), Some("gamma"));
    assert_eq!(r2.new.as_deref(), Some("new2"));

    let r3 = index.row_at(&segments, 3).unwrap();
    assert_eq!(r3.kind, gitcomet_core::file_diff::FileDiffRowKind::Add);
    assert_eq!(r3.old, None);
    assert_eq!(r3.new.as_deref(), Some("beta"));

    let r4 = index.row_at(&segments, 4).unwrap();
    assert_eq!(r4.kind, gitcomet_core::file_diff::FileDiffRowKind::Add);
    assert_eq!(r4.old, None);
    assert_eq!(r4.new.as_deref(), Some("gamma"));
}

#[test]
fn split_row_index_gap_diff_no_common_lines_falls_back_to_positional() {
    // When no lines match between ours and theirs in a gap, the diff
    // has no Context rows, so we fall back to positional pairing.
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "aaa\nbbb\n".into(),
        theirs: "xxx\nyyy\nzzz\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);

    // Positional: max(2, 3) = 3 rows
    assert_eq!(index.total_rows(), 3);

    let r0 = index.row_at(&segments, 0).unwrap();
    assert_eq!(r0.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);
    assert_eq!(r0.old.as_deref(), Some("aaa"));
    assert_eq!(r0.new.as_deref(), Some("xxx"));

    let r1 = index.row_at(&segments, 1).unwrap();
    assert_eq!(r1.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);
    assert_eq!(r1.old.as_deref(), Some("bbb"));
    assert_eq!(r1.new.as_deref(), Some("yyy"));

    let r2 = index.row_at(&segments, 2).unwrap();
    assert_eq!(r2.kind, gitcomet_core::file_diff::FileDiffRowKind::Add);
    assert_eq!(r2.old, None);
    assert_eq!(r2.new.as_deref(), Some("zzz"));
}

#[test]
fn split_row_index_gap_diff_produces_more_rows_than_positional() {
    // Verify that the diff alignment can produce more rows than max(N, M)
    // when matching lines interleave with unmatched lines.
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "ctx\nold_only\nctx2\n".into(),
        theirs: "ctx\nnew_only\nctx2\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);

    // Diff: ctx|ctx (Context), old_only|new_only (Modify), ctx2|ctx2 (Context) = 3 rows
    // Same as positional max(3, 3) = 3, but properly classified.
    assert_eq!(index.total_rows(), 3);

    let r0 = index.row_at(&segments, 0).unwrap();
    assert_eq!(r0.kind, gitcomet_core::file_diff::FileDiffRowKind::Context);

    let r1 = index.row_at(&segments, 1).unwrap();
    assert_eq!(r1.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);
    assert_eq!(r1.old.as_deref(), Some("old_only"));
    assert_eq!(r1.new.as_deref(), Some("new_only"));

    let r2 = index.row_at(&segments, 2).unwrap();
    assert_eq!(r2.kind, gitcomet_core::file_diff::FileDiffRowKind::Context);
}

#[test]
fn split_row_index_gap_diff_with_anchors_refines_each_gap() {
    // Verify that each gap between anchors is independently refined.
    // Here we set up a block with a unique anchor in the middle,
    // and insertions on both sides.
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "start\nunique_anchor\nend\n".into(),
        theirs: "added_before\nstart\nunique_anchor\nadded_after\nend\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);

    // Verify all content is reachable and the total row count is correct.
    // The unique_anchor should anchor the alignment.
    let mut all_old = Vec::new();
    let mut all_new = Vec::new();
    for i in 0..index.total_rows() {
        let row = index.row_at(&segments, i).unwrap();
        all_old.push(row.old.clone());
        all_new.push(row.new.clone());
    }
    // All ours lines must appear
    assert!(all_old.iter().any(|t| t.as_deref() == Some("start")));
    assert!(
        all_old
            .iter()
            .any(|t| t.as_deref() == Some("unique_anchor"))
    );
    assert!(all_old.iter().any(|t| t.as_deref() == Some("end")));
    // All theirs lines must appear
    assert!(all_new.iter().any(|t| t.as_deref() == Some("added_before")));
    assert!(all_new.iter().any(|t| t.as_deref() == Some("start")));
    assert!(
        all_new
            .iter()
            .any(|t| t.as_deref() == Some("unique_anchor"))
    );
    assert!(all_new.iter().any(|t| t.as_deref() == Some("added_after")));
    assert!(all_new.iter().any(|t| t.as_deref() == Some("end")));
}

#[test]
fn compute_word_highlights_for_modify_row() {
    use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

    // Modify row with differing text should produce word highlights
    let modify_row = FileDiffRow {
        kind: FileDiffRowKind::Modify,
        old_line: Some(1),
        new_line: Some(1),
        old: Some("let x = 1;".into()),
        new: Some("let x = 2;".into()),
        eof_newline: None,
    };
    let highlights = compute_word_highlights_for_row(&modify_row);
    assert!(
        highlights.is_some(),
        "Modify row with differing text should have word highlights"
    );
    let (old_ranges, new_ranges) = highlights.unwrap();
    assert!(
        !old_ranges.is_empty(),
        "should highlight the changed character in old text"
    );
    assert!(
        !new_ranges.is_empty(),
        "should highlight the changed character in new text"
    );

    // Context row should not produce highlights
    let context_row = FileDiffRow {
        kind: FileDiffRowKind::Context,
        old_line: Some(1),
        new_line: Some(1),
        old: Some("same".into()),
        new: Some("same".into()),
        eof_newline: None,
    };
    assert!(compute_word_highlights_for_row(&context_row).is_none());

    // Add row should not produce highlights
    let add_row = FileDiffRow {
        kind: FileDiffRowKind::Add,
        old_line: None,
        new_line: Some(1),
        old: None,
        new: Some("new line".into()),
        eof_newline: None,
    };
    assert!(compute_word_highlights_for_row(&add_row).is_none());
}

#[test]
fn split_row_index_modify_rows_get_word_highlights() {
    // End-to-end: build paged index, verify that Modify rows produce
    // word highlights via compute_word_highlights_for_row.
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "let x = 1;\nlet y = 2;\n".into(),
        theirs: "let x = 99;\nlet y = 2;\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);

    let r0 = index.row_at(&segments, 0).unwrap();
    assert_eq!(r0.kind, gitcomet_core::file_diff::FileDiffRowKind::Modify);
    let hl = compute_word_highlights_for_row(&r0);
    assert!(
        hl.is_some(),
        "Modify row from paged index should produce word highlights"
    );

    let r1 = index.row_at(&segments, 1).unwrap();
    assert_eq!(r1.kind, gitcomet_core::file_diff::FileDiffRowKind::Context);
    assert!(
        compute_word_highlights_for_row(&r1).is_none(),
        "Context row should not produce word highlights"
    );
}

#[test]
fn search_matching_rows_finds_text_in_block() {
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "alpha\nbeta\ngamma\n".into(),
        theirs: "delta\nepsilon\nzeta\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 3);
    assert_eq!(index.total_rows(), 3);

    // Search for "beta" — only ours line 1 matches.
    let matches = index.search_matching_rows(&segments, |text| text.contains("beta"));
    assert_eq!(matches, vec![1]);

    // Search for "epsilon" — only theirs line 1 matches.
    let matches = index.search_matching_rows(&segments, |text| text.contains("epsilon"));
    assert_eq!(matches, vec![1]);

    // Search for "a" — matches alpha (row 0), beta (row 1), gamma (row 2) on ours,
    // plus delta (row 0), zeta (row 2) on theirs. All rows match.
    let matches = index.search_matching_rows(&segments, |text| text.contains("a"));
    assert_eq!(matches, vec![0, 1, 2]);
}

#[test]
fn search_matching_rows_finds_text_in_context() {
    let segments = vec![
        ConflictSegment::Text("first\nsecond\nthird\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 2);

    // Context lines: trailing 2 lines of the text segment (second, third) since it precedes a block.
    // Block lines: 1 row for the block.
    // Search for "second" — should match the context row.
    let matches = index.search_matching_rows(&segments, |text| text.contains("second"));
    assert_eq!(matches.len(), 1);

    // "first" is outside the context window (only trailing 2 lines).
    let matches = index.search_matching_rows(&segments, |text| text.contains("first"));
    assert_eq!(matches.len(), 0);
}

#[test]
fn search_matching_rows_equivalence_with_row_at() {
    // For a multi-segment conflict, verify that search_matching_rows returns
    // the same set of source rows as manually iterating row_at.
    let segments = vec![
        ConflictSegment::Text("ctx_a\nctx_b\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "needle_ours\nplain\n".into(),
            theirs: "plain\nneedle_theirs\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("ctx_c\nctx_d\n".into()),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 1);

    let query = "needle";
    let via_search = index.search_matching_rows(&segments, |text| text.contains(query));

    let mut via_row_at = Vec::new();
    for row_ix in 0..index.total_rows() {
        if let Some(row) = index.row_at(&segments, row_ix) {
            let old_match = row.old.as_deref().is_some_and(|s| s.contains(query));
            let new_match = row.new.as_deref().is_some_and(|s| s.contains(query));
            if old_match || new_match {
                via_row_at.push(row_ix);
            }
        }
    }

    assert_eq!(
        via_search, via_row_at,
        "search_matching_rows must match row_at iteration"
    );
}

#[test]
fn search_matching_rows_does_not_materialize_split_pages() {
    let line_count = CONFLICT_SPLIT_PAGE_SIZE * 2;
    let ours: String = (0..line_count)
        .map(|ix| format!("ours_line_{ix}\n"))
        .collect();
    let theirs: String = (0..line_count)
        .map(|ix| format!("theirs_line_{ix}\n"))
        .collect();
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: ours.into(),
        theirs: theirs.into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, line_count);

    assert!(
        index.cached_page_indices().is_empty(),
        "search should start with an empty page cache"
    );

    let matches = index.search_matching_rows(&segments, |text| text.contains("theirs_line_300"));
    assert_eq!(matches, vec![300]);
    assert!(
        index.cached_page_indices().is_empty(),
        "source-text search should not materialize split pages"
    );

    assert!(index.row_at(&segments, matches[0]).is_some());
    assert_eq!(
        index.cached_page_count(),
        1,
        "requesting the matched row should materialize exactly one split page"
    );
}

#[test]
fn search_text_matching_rows_matches_predicate_path() {
    let line_count = CONFLICT_SPLIT_PAGE_SIZE * 4;
    let ours: String = (0..line_count)
        .map(|ix| format!("ours_line_{ix}\n"))
        .collect();
    let theirs: String = (0..line_count)
        .map(|ix| format!("theirs_line_{ix}\n"))
        .collect();
    let context: String = (0..line_count)
        .map(|ix| format!("context_line_{ix}\n"))
        .collect();
    let segments = vec![
        ConflictSegment::Text(context.into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: ours.into(),
            theirs: theirs.into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
    ];
    let index = ConflictSplitRowIndex::new(&segments, line_count);

    // Test several patterns against both paths.
    for needle in &[
        "ours_line_300",
        "theirs_line_5",
        "context_line_0",
        "_line_99",
    ] {
        let via_pred = index.search_matching_rows(&segments, |text| text.contains(needle));
        let via_simd = index.search_text_matching_rows(&segments, needle.as_bytes());
        assert_eq!(
            via_pred, via_simd,
            "search_text_matching_rows must match search_matching_rows for needle {needle:?}"
        );
    }
}

#[test]
fn search_ascii_case_insensitive_matching_rows_matches_predicate_path() {
    let line_count = CONFLICT_SPLIT_PAGE_SIZE * 4;
    let ours: String = (0..line_count)
        .map(|ix| format!("Ours_Line_{ix}\n"))
        .collect();
    let theirs: String = (0..line_count)
        .map(|ix| format!("THEIRS_line_{ix}\n"))
        .collect();
    let context: String = (0..line_count)
        .map(|ix| format!("Context_LINE_{ix}\n"))
        .collect();
    let segments = vec![
        ConflictSegment::Text(context.into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: ours.into(),
            theirs: theirs.into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
    ];
    let index = ConflictSplitRowIndex::new(&segments, line_count);

    for needle in &[
        "ours_line_300",
        "THEIRS_LINE_5",
        "context_line_0",
        "_LiNe_99",
    ] {
        let via_pred = index.search_matching_rows(&segments, |text| {
            text.as_bytes()
                .windows(needle.len())
                .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
        });
        let via_specialized =
            index.search_ascii_case_insensitive_matching_rows(&segments, needle.as_bytes());
        assert_eq!(
            via_pred, via_specialized,
            "case-insensitive search must match predicate path for needle {needle:?}"
        );
    }
}

#[test]
fn split_row_index_sparse_checkpoint_rows_resolve_far_from_start() {
    let line_count = CONFLICT_SPLIT_PAGE_SIZE * 4;
    let ours: String = (0..line_count)
        .map(|ix| format!("ours_line_{ix}\n"))
        .collect();
    let theirs: String = (0..line_count)
        .map(|ix| format!("theirs_line_{ix}\n"))
        .collect();
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: ours.into(),
        theirs: theirs.into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 0);

    let row_ix = CONFLICT_SPLIT_PAGE_SIZE * 3 + 17;
    let row = index
        .row_at(&segments, row_ix)
        .expect("far row should materialize from sparse checkpoints");

    assert_eq!(row.old_line, Some(u32::try_from(row_ix + 1).unwrap()));
    assert_eq!(row.new_line, Some(u32::try_from(row_ix + 1).unwrap()));
    assert_eq!(row.old.as_deref(), Some("ours_line_785"));
    assert_eq!(row.new.as_deref(), Some("theirs_line_785"));
}

#[test]
fn split_row_index_metadata_stays_sparse_for_large_block() {
    let line_count = CONFLICT_SPLIT_PAGE_SIZE * 16;
    let ours: String = (0..line_count)
        .map(|ix| format!("ours_line_{ix}\n"))
        .collect();
    let theirs: String = (0..line_count)
        .map(|ix| format!("theirs_line_{ix}\n"))
        .collect();
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: ours.into(),
        theirs: theirs.into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let index = ConflictSplitRowIndex::new(&segments, 0);

    assert!(
        index.metadata_byte_size() < line_count * std::mem::size_of::<usize>() / 4,
        "split-row metadata should stay sparse instead of storing per-line starts",
    );
}

fn resolved_output_projection_lines(
    projection: &ResolvedOutputProjection,
    segments: &[ConflictSegment],
) -> Vec<String> {
    (0..projection.len())
        .map(|line_ix| {
            projection
                .line_text(segments, line_ix)
                .expect("projection should return every visible output line")
                .into_owned()
        })
        .collect()
}

#[test]
fn resolved_output_projection_matches_generated_text_lines() {
    let segments = vec![
        ConflictSegment::Text("alpha\nprefix ".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base middle\n".into()),
            ours: "ours middle\n".into(),
            theirs: "theirs middle\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("suffix\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "left".into(),
            theirs: "right\n".into(),
            choice: ConflictChoice::Both,
            resolved: true,
        }),
    ];

    let projection = ResolvedOutputProjection::from_segments(&segments);
    let actual_lines = resolved_output_projection_lines(&projection, &segments);
    let expected_lines: Vec<String> = generate_resolved_text(&segments)
        .split('\n')
        .map(str::to_string)
        .collect();

    assert_eq!(projection.len(), expected_lines.len());
    assert_eq!(actual_lines, expected_lines);
}

#[test]
fn resolved_output_projection_merges_adjacent_segments_without_newlines() {
    let segments = vec![
        ConflictSegment::Text("prefix ".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "middle".into(),
            theirs: "remote".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text(" suffix\nnext".into()),
    ];

    let projection = ResolvedOutputProjection::from_segments(&segments);
    let lines = resolved_output_projection_lines(&projection, &segments);

    assert_eq!(
        lines,
        vec!["prefix middle suffix".to_string(), "next".to_string()]
    );
    assert_eq!(projection.conflict_line_range(0), Some(0..1));
}

#[test]
fn resolved_output_projection_tracks_conflict_line_ranges() {
    let segments = vec![
        ConflictSegment::Text("head\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a0\na1\n".into(),
            theirs: "x\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("middle\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b-left\n".into(),
            theirs: "b-right\n".into(),
            choice: ConflictChoice::Both,
            resolved: true,
        }),
        ConflictSegment::Text("tail\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base-last".into()),
            ours: "ignored\n".into(),
            theirs: "ignored\n".into(),
            choice: ConflictChoice::Base,
            resolved: true,
        }),
    ];

    let projection = ResolvedOutputProjection::from_segments(&segments);

    assert_eq!(projection.len(), 8);
    assert_eq!(projection.conflict_line_ranges(), &[1..3, 4..6, 7..8]);
    assert_eq!(
        projection
            .line_text(&segments, 7)
            .expect("final base line should be projected")
            .as_ref(),
        "base-last"
    );
}

#[test]
fn derive_region_resolution_updates_from_segments_uses_block_choices() {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let segments = vec![
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours unresolved\n".into(),
            theirs: "theirs unresolved\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: false,
        }),
        ConflictSegment::Text("between\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "left\n".into(),
            theirs: "right\n".into(),
            choice: ConflictChoice::Both,
            resolved: true,
        }),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".into()),
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::Base,
            resolved: true,
        }),
    ];

    let updates = derive_region_resolution_updates_from_segments(&segments, &[7, 11, 19]);

    assert_eq!(
        updates,
        vec![(7, R::Unresolved), (11, R::PickBoth), (19, R::PickBase)]
    );
}
