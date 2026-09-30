use super::*;

fn file_slice_lines(path: &Arc<PathBuf>, text: &str) -> Vec<FileDiffLineText> {
    let mut start = 0;
    text.split_inclusive('\n')
        .map(|line| {
            let range = start..start + line.trim_end_matches('\n').len();
            start += line.len();
            FileDiffLineText::file_slice(Arc::clone(path), range, line.is_ascii(), false)
        })
        .collect()
}

/// Search reads every row of a source-backed side. Batched, it must see
/// exactly what per-line reads see, opening the file once rather than
/// once per line, and keep nothing once the batch ends.
#[test]
fn batched_slice_reads_match_per_line_reads_and_open_each_file_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = Arc::new(dir.path().join("side.txt"));
    let text = "plain ascii line\nβeta γamma δelta\n\ttabbed\nlast line without newline";
    std::fs::write(&*path, text).expect("write side");
    let read_all = |lines: &[FileDiffLineText]| {
        lines
            .iter()
            .map(|line| {
                let len = line.len();
                (
                    line.slice_bytes(0..len).map(|b| b.into_owned()),
                    line.slice_text_resolved(1..len.min(7))
                        .map(|(text, range)| (text.into_owned(), range)),
                    line.as_str().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };

    let _ = take_file_slice_opens_for_tests();
    let unbatched = read_all(&file_slice_lines(&path, text));
    assert!(take_file_slice_opens_for_tests() >= 4);

    let batched = {
        let _batch = batch_file_slice_reads();
        let _nested = batch_file_slice_reads();
        read_all(&file_slice_lines(&path, text))
    };
    assert_eq!(batched, unbatched);
    assert_eq!(take_file_slice_opens_for_tests(), 1);

    // A sparse batch reads the same slices through one handle, and only
    // the bytes asked for; the outermost guard picks the mode.
    let _ = take_file_slice_bytes_read_for_tests();
    let sparse = {
        let _batch = batch_file_slice_handles();
        let _nested = batch_file_slice_reads();
        file_slice_lines(&path, text)[0].as_str().to_owned()
    };
    assert_eq!(sparse, "plain ascii line");
    assert_eq!(take_file_slice_opens_for_tests(), 1);
    assert_eq!(take_file_slice_bytes_read_for_tests(), sparse.len());

    // The batch ended: a changed file is read afresh, not from a copy.
    std::fs::write(&*path, text.replace("plain", "PLAIN")).expect("rewrite side");
    let after = file_slice_lines(&path, text);
    assert_eq!(after[0].as_str(), "PLAIN ascii line");
    assert_eq!(take_file_slice_opens_for_tests(), 1);

    // Out-of-range slices fail the same way with or without a batch.
    let missing = FileDiffLineText::file_slice(Arc::clone(&path), 10_000..10_010, true, false);
    let _batch = batch_file_slice_reads();
    assert!(missing.slice_bytes(0..10).is_none());
}

fn remove_row(old_line: u32, old: &str) -> FileDiffRow {
    FileDiffRow {
        kind: FileDiffRowKind::Remove,
        old_line: Some(old_line),
        new_line: None,
        old: Some(old.into()),
        new: None,
        eof_newline: None,
    }
}

fn add_row(new_line: u32, new: &str) -> FileDiffRow {
    FileDiffRow {
        kind: FileDiffRowKind::Add,
        old_line: None,
        new_line: Some(new_line),
        old: None,
        new: Some(new.into()),
        eof_newline: None,
    }
}

fn changed_line_masks_from_rows(
    rows: &[FileDiffRow],
    old_line_count: usize,
    new_line_count: usize,
) -> (Vec<bool>, Vec<bool>) {
    let mut old_mask = vec![false; old_line_count];
    let mut new_mask = vec![false; new_line_count];

    for row in rows {
        match row.kind {
            FileDiffRowKind::Context => {}
            FileDiffRowKind::Remove => mark_changed_line(old_mask.as_mut_slice(), row.old_line),
            FileDiffRowKind::Add => mark_changed_line(new_mask.as_mut_slice(), row.new_line),
            FileDiffRowKind::Modify => {
                mark_changed_line(old_mask.as_mut_slice(), row.old_line);
                mark_changed_line(new_mask.as_mut_slice(), row.new_line);
            }
        }
    }

    (old_mask, new_mask)
}

fn line_to_row_maps_from_rows(
    rows: &[FileDiffRow],
    old_line_count: usize,
    new_line_count: usize,
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut old_line_to_row = vec![None; old_line_count];
    let mut new_line_to_row = vec![None; new_line_count];

    for (row_index, row) in rows.iter().enumerate() {
        assign_line_to_row(old_line_to_row.as_mut_slice(), row.old_line, row_index);
        assign_line_to_row(new_line_to_row.as_mut_slice(), row.new_line, row_index);
    }

    (old_line_to_row, new_line_to_row)
}

#[test]
fn file_slice_text_resolved_clamps_to_utf8_char_boundaries() {
    let text = "aÄ日z";
    let temp_path = std::env::temp_dir().join(format!(
        "gitcomet_file_diff_utf8_slice_{}_{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be monotonic enough for test temp path")
            .as_nanos()
    ));
    std::fs::write(&temp_path, text.as_bytes()).expect("write UTF-8 file slice fixture");

    let raw_text =
        FileDiffLineText::file_slice(Arc::new(temp_path.clone()), 0..text.len(), false, false);

    let (slice_text, resolved_range) = raw_text
        .slice_text_resolved(2..6)
        .expect("UTF-8 file slice should resolve");
    assert_eq!(slice_text.as_ref(), "日");
    assert_eq!(resolved_range, 3..6);

    let (empty_slice, empty_range) = raw_text
        .slice_text_resolved(2..5)
        .expect("partial UTF-8 slice should still resolve");
    assert!(empty_slice.is_empty());
    assert_eq!(empty_range, 3..3);

    let _ = std::fs::remove_file(&temp_path);
}

#[test]
fn side_by_side_rows_share_backing_for_context_and_reuse_row_storage_on_clone() {
    let rows = side_by_side_rows("alpha\nbeta\n", "alpha\nbeta changed\n");
    assert_eq!(rows.len(), 2);

    let context = &rows[0];
    assert!(
        context
            .old
            .as_ref()
            .unwrap()
            .shares_backing_with(context.new.as_ref().unwrap())
    );

    let cloned = rows[1].clone();
    assert!(
        rows[1]
            .old
            .as_ref()
            .unwrap()
            .shares_backing_with(cloned.old.as_ref().unwrap())
    );
    assert!(
        rows[1]
            .new
            .as_ref()
            .unwrap()
            .shares_backing_with(cloned.new.as_ref().unwrap())
    );
}

fn emitted_line_prefix_counts_from_rows(rows: &[FileDiffRow]) -> (Vec<usize>, Vec<usize>) {
    let mut old_prefix = Vec::with_capacity(rows.len().saturating_add(1));
    let mut new_prefix = Vec::with_capacity(rows.len().saturating_add(1));
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    old_prefix.push(0);
    new_prefix.push(0);

    for row in rows {
        if row.old_line.is_some() {
            old_count = old_count.saturating_add(1);
        }
        if row.new_line.is_some() {
            new_count = new_count.saturating_add(1);
        }
        old_prefix.push(old_count);
        new_prefix.push(new_count);
    }

    (old_prefix, new_prefix)
}

#[test]
fn edits_to_hunks_with_builds_base_relative_hunks() {
    let inserted = String::from("inserted");
    let edits = vec![
        Edit {
            kind: EditKind::Equal,
            old: Some("ctx"),
            new: Some("ctx"),
        },
        Edit {
            kind: EditKind::Delete,
            old: Some("old"),
            new: None,
        },
        Edit {
            kind: EditKind::Insert,
            old: None,
            new: Some(inserted.as_str()),
        },
        Edit {
            kind: EditKind::Equal,
            old: Some("tail"),
            new: Some("tail"),
        },
    ];

    let hunks = edits_to_hunks_with(&edits, |line| line.to_string());
    assert_eq!(
        hunks,
        vec![DiffHunk {
            base_start: 1,
            base_end: 2,
            new_lines: vec!["inserted".to_string()],
        }]
    );
}

#[test]
fn reconstruct_side_with_applies_hunks_and_preserves_context() {
    let base_lines = split_lines("a\nb\nc\n");
    let hunks = vec![
        DiffHunk {
            base_start: 1,
            base_end: 1,
            new_lines: vec!["ins".to_string()],
        },
        DiffHunk {
            base_start: 2,
            base_end: 3,
            new_lines: vec!["c2".to_string()],
        },
    ];
    let mut output: Vec<String> = Vec::new();

    reconstruct_side_with(&base_lines, 0..3, &hunks, &mut output, |line| {
        line.to_string()
    });

    assert_eq!(
        output,
        vec![
            "a".to_string(),
            "ins".to_string(),
            "b".to_string(),
            "c2".to_string()
        ]
    );
}

#[test]
fn pairs_delete_insert_into_modify_rows() {
    let old = "a\nb\nc\n";
    let new = "a\nb2\nc\n";

    let rows = side_by_side_rows(old, new);
    assert_eq!(
        rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
        vec![
            FileDiffRowKind::Context,
            FileDiffRowKind::Modify,
            FileDiffRowKind::Context
        ]
    );

    let mid = &rows[1];
    assert_eq!(mid.old.as_deref(), Some("b"));
    assert_eq!(mid.new.as_deref(), Some("b2"));
    assert_eq!(mid.old_line, Some(2));
    assert_eq!(mid.new_line, Some(2));
    assert_eq!(mid.eof_newline, None);
}

#[test]
fn handles_additions_and_deletions() {
    let old = "a\nb\n";
    let new = "a\nb\nc\n";
    let rows = side_by_side_rows(old, new);
    assert!(rows.iter().any(|r| r.kind == FileDiffRowKind::Add));

    let old = "a\nb\nc\n";
    let new = "a\nc\n";
    let rows = side_by_side_rows(old, new);
    assert!(rows.iter().any(|r| r.kind == FileDiffRowKind::Remove));
}

#[test]
fn marks_missing_newline_in_new_file() {
    let old = "a\n";
    let new = "a";

    let rows = side_by_side_rows(old, new);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, FileDiffRowKind::Modify);
    assert_eq!(rows[0].old.as_deref(), Some("a"));
    assert_eq!(rows[0].new.as_deref(), Some("a"));
    assert_eq!(rows[0].eof_newline, Some(FileDiffEofNewline::MissingInNew));
}

#[test]
fn marks_missing_newline_in_old_file() {
    let old = "a";
    let new = "a\n";

    let rows = side_by_side_rows(old, new);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, FileDiffRowKind::Modify);
    assert_eq!(rows[0].old.as_deref(), Some("a"));
    assert_eq!(rows[0].new.as_deref(), Some("a"));
    assert_eq!(rows[0].eof_newline, Some(FileDiffEofNewline::MissingInOld));
}

#[test]
fn preserves_existing_modify_rows_when_eof_newline_differs() {
    let old = "a\nb\n";
    let new = "a\nc";

    let rows = side_by_side_rows(old, new);
    assert_eq!(
        rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
        vec![FileDiffRowKind::Context, FileDiffRowKind::Modify]
    );
    assert_eq!(rows[1].old.as_deref(), Some("b"));
    assert_eq!(rows[1].new.as_deref(), Some("c"));
    assert_eq!(rows[1].eof_newline, Some(FileDiffEofNewline::MissingInNew));
}

#[test]
fn asymmetric_replacement_pairs_best_matching_lines() {
    let rows = vec![
        remove_row(10, "alpha"),
        remove_row(11, "beta"),
        add_row(20, "intro"),
        add_row(21, "alpha changed"),
        add_row(22, "beta changed"),
    ];

    let paired = pair_replacements(rows);
    assert_eq!(
        paired.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            FileDiffRowKind::Add,
            FileDiffRowKind::Modify,
            FileDiffRowKind::Modify
        ]
    );
    assert_eq!(paired[0].new.as_deref(), Some("intro"));
    assert_eq!(paired[1].old.as_deref(), Some("alpha"));
    assert_eq!(paired[1].new.as_deref(), Some("alpha changed"));
    assert_eq!(paired[2].old.as_deref(), Some("beta"));
    assert_eq!(paired[2].new.as_deref(), Some("beta changed"));
}

#[test]
fn dissimilar_single_line_replacement_stays_add_remove() {
    let rows = vec![remove_row(1, "aaaaaaaa"), add_row(1, "zzzzzzzz")];
    let paired = pair_replacements(rows);

    assert_eq!(
        paired.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![FileDiffRowKind::Remove, FileDiffRowKind::Add]
    );
}

#[test]
fn ascii_prepared_replacement_line_defers_char_allocation_until_needed() {
    let line = PreparedReplacementLine::new("plain-ascii");

    assert_eq!(line.ascii_bytes(), Some("plain-ascii".as_bytes()));
    assert!(line.chars.get().is_none());

    let chars = line.chars();
    assert_eq!(
        chars,
        ['p', 'l', 'a', 'i', 'n', '-', 'a', 's', 'c', 'i', 'i']
    );
    assert!(line.chars.get().is_some());
}

#[test]
fn prepare_replacement_text_cache_ids_dedups_duplicate_texts() {
    let lines = prepare_replacement_lines(&["alpha", "beta", "alpha", "gamma", "beta"]);
    let cache_ids = prepare_replacement_text_cache_ids(&lines);

    assert_eq!(cache_ids.ids, vec![0, 1, 0, 2, 1]);
    assert_eq!(cache_ids.unique_texts, 3);
    assert!(cache_ids.has_duplicates);
}

#[test]
fn shared_boundary_counts_unicode_codepoints() {
    let old = PreparedReplacementLine::new("prefix-é-suffix");
    let new = PreparedReplacementLine::new("prefix-ê-suffix");

    assert_eq!(
        shared_boundary_chars(old.chars(), new.chars()),
        ("prefix-".chars().count(), "-suffix".chars().count())
    );
}

#[test]
fn shared_boundary_bytes_matches_generic_ascii_boundaries() {
    let old = b"prefix-before_source_001-suffix";
    let new = b"prefix-after_source_002-suffix";

    assert_eq!(
        shared_boundary_bytes(old, new),
        shared_boundary_units(old, new)
    );
}

#[test]
fn replacement_pair_cost_reuses_unicode_boundaries() {
    let old = PreparedReplacementLine::new("prefix-é-suffix");
    let new = PreparedReplacementLine::new("prefix-ê-suffix");
    let unrelated = PreparedReplacementLine::new("xxxxxxxxxxxxxxx");
    let mut scratch = LevenshteinScratch::default();

    let shared_edge_cost = replacement_pair_cost(&old, &new, &mut scratch);
    let unrelated_cost = replacement_pair_cost(&old, &unrelated, &mut scratch);

    assert!(
        shared_edge_cost < unrelated_cost,
        "shared prefix/suffix should keep a unicode substitution cheaper than an unrelated line"
    );
}

#[test]
fn side_by_side_aligns_asymmetric_replacement_in_context() {
    let old = "start\nalpha\nbeta\nend\n";
    let new = "start\nintro\nalpha changed\nbeta changed\nend\n";

    let rows = side_by_side_rows(old, new);
    assert_eq!(
        rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            FileDiffRowKind::Context,
            FileDiffRowKind::Add,
            FileDiffRowKind::Modify,
            FileDiffRowKind::Modify,
            FileDiffRowKind::Context
        ]
    );
}

#[test]
fn anchor_groups_contiguous_changes_into_regions() {
    let old = "a\nb\nc\nd\n";
    let new = "a\nx\nc\ny\nd\n";

    let rows = side_by_side_rows(old, new);
    assert_eq!(
        rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            FileDiffRowKind::Context,
            FileDiffRowKind::Modify,
            FileDiffRowKind::Context,
            FileDiffRowKind::Add,
            FileDiffRowKind::Context,
        ]
    );

    let anchors = compute_row_region_anchors(&rows);
    assert_eq!(anchors.row_anchors.len(), rows.len());
    assert_eq!(anchors.region_anchors.len(), 2);

    assert_eq!(
        anchors.region_anchors[0],
        FileDiffRegionAnchor {
            region_id: 0,
            row_start: 1,
            row_end_exclusive: 2,
            old_start_line: Some(2),
            old_end_line: Some(2),
            new_start_line: Some(2),
            new_end_line: Some(2),
        }
    );
    assert_eq!(
        anchors.region_anchors[1],
        FileDiffRegionAnchor {
            region_id: 1,
            row_start: 3,
            row_end_exclusive: 4,
            old_start_line: None,
            old_end_line: None,
            new_start_line: Some(4),
            new_end_line: Some(4),
        }
    );
    assert_eq!(anchors.row_anchors[0].region_id, None);
    assert_eq!(anchors.row_anchors[1].region_id, Some(0));
    assert_eq!(anchors.row_anchors[1].ordinal_in_region, Some(0));
    assert_eq!(anchors.row_anchors[2].region_id, None);
    assert_eq!(anchors.row_anchors[3].region_id, Some(1));
    assert_eq!(anchors.row_anchors[3].ordinal_in_region, Some(0));
    assert_eq!(anchors.row_anchors[4].region_id, None);
}

#[test]
fn anchor_keeps_ordinals_within_single_region() {
    let old = "start\nalpha\nbeta\nend\n";
    let new = "start\nintro\nalpha changed\nbeta changed\nend\n";

    let rows = side_by_side_rows(old, new);
    let anchors = compute_row_region_anchors(&rows);

    assert_eq!(anchors.region_anchors.len(), 1);
    assert_eq!(
        anchors.region_anchors[0],
        FileDiffRegionAnchor {
            region_id: 0,
            row_start: 1,
            row_end_exclusive: 4,
            old_start_line: Some(2),
            old_end_line: Some(3),
            new_start_line: Some(2),
            new_end_line: Some(4),
        }
    );
    assert_eq!(anchors.row_anchors[1].region_id, Some(0));
    assert_eq!(anchors.row_anchors[1].ordinal_in_region, Some(0));
    assert_eq!(anchors.row_anchors[2].ordinal_in_region, Some(1));
    assert_eq!(anchors.row_anchors[3].ordinal_in_region, Some(2));
}

#[test]
fn anchor_handles_rows_without_line_numbers() {
    let rows = vec![FileDiffRow {
        kind: FileDiffRowKind::Modify,
        old_line: None,
        new_line: None,
        old: None,
        new: None,
        eof_newline: Some(FileDiffEofNewline::MissingInNew),
    }];

    let anchors = compute_row_region_anchors(&rows);
    assert_eq!(anchors.row_anchors.len(), 1);
    assert_eq!(anchors.row_anchors[0].region_id, Some(0));
    assert_eq!(anchors.row_anchors[0].ordinal_in_region, Some(0));
    assert_eq!(
        anchors.region_anchors,
        vec![FileDiffRegionAnchor {
            region_id: 0,
            row_start: 0,
            row_end_exclusive: 1,
            old_start_line: None,
            old_end_line: None,
            new_start_line: None,
            new_end_line: None,
        }]
    );
}

#[test]
fn side_by_side_with_anchors_is_deterministic() {
    let old = "a\nb\nc\n";
    let new = "a\nb changed\nc\n";

    let first = side_by_side_rows_with_anchors(old, new);
    let second = side_by_side_rows_with_anchors(old, new);
    assert_eq!(first, second);
    assert_eq!(first.rows.len(), first.anchors.row_anchors.len());
}

#[test]
fn append_side_by_side_rows_with_offsets_matches_materialized_rows() {
    let old = "keep\nold-only\nchange-me\n";
    let new = "keep\nchange-you\nnew-only\n";
    let mut appended = vec![FileDiffRow {
        kind: FileDiffRowKind::Context,
        old_line: Some(1),
        new_line: Some(1),
        old: Some("prefix".into()),
        new: Some("prefix".into()),
        eof_newline: None,
    }];

    append_side_by_side_rows_with_offsets(&mut appended, old, new, 10, 20);

    let mut expected = vec![FileDiffRow {
        kind: FileDiffRowKind::Context,
        old_line: Some(1),
        new_line: Some(1),
        old: Some("prefix".into()),
        new: Some("prefix".into()),
        eof_newline: None,
    }];
    expected.extend(side_by_side_rows(old, new).into_iter().map(|row| {
        FileDiffRow {
            kind: row.kind,
            old_line: row
                .old_line
                .map(|line| line.saturating_add(10).saturating_sub(1)),
            new_line: row
                .new_line
                .map(|line| line.saturating_add(20).saturating_sub(1)),
            old: row.old,
            new: row.new,
            eof_newline: row.eof_newline,
        }
    }));

    assert_eq!(appended, expected);
}

#[test]
fn plan_metadata_helpers_match_materialized_rows() {
    let old = "keep\nremove only\nbefore change\nshared tail\n";
    let new = "keep\ninsert only\nafter change\nshared tail\nextra add\n";

    let plan = side_by_side_plan(old, new);
    let rows = side_by_side_rows(old, new);
    let inline_row_count = rows.len()
        + rows
            .iter()
            .filter(|row| row.kind == FileDiffRowKind::Modify)
            .count();
    let old_line_count = old.lines().count();
    let new_line_count = new.lines().count();

    assert_eq!(plan.row_count, rows.len());
    assert_eq!(plan.inline_row_count, inline_row_count);
    assert_eq!(
        plan_row_region_anchors(&plan),
        compute_row_region_anchors(&rows)
    );
    assert_eq!(
        plan_changed_line_masks(&plan, old_line_count, new_line_count),
        changed_line_masks_from_rows(&rows, old_line_count, new_line_count)
    );
    assert_eq!(
        plan_line_to_row_maps(&plan, old_line_count, new_line_count),
        line_to_row_maps_from_rows(&rows, old_line_count, new_line_count)
    );
    assert_eq!(
        plan_emitted_line_prefix_counts(&plan),
        emitted_line_prefix_counts_from_rows(&rows)
    );
}

#[test]
fn for_each_side_by_side_row_matches_materialized_rows() {
    let old = "keep\nremove only\nbefore change\nshared tail\n";
    let new = "keep\ninsert only\nafter change\nshared tail\nextra add\n";

    type PlanRow = (
        FileDiffRowKind,
        Option<u32>,
        Option<u32>,
        Option<String>,
        Option<String>,
    );
    let rows = side_by_side_rows(old, new);
    let mut plan_rows: Vec<PlanRow> = Vec::new();
    for_each_side_by_side_row(old, new, |view| {
        let (kind, old_line, new_line, old_text, new_text) = match view {
            PlanRowView::Context {
                old_line,
                new_line,
                text,
            } => (
                FileDiffRowKind::Context,
                Some(old_line),
                Some(new_line),
                Some(text.to_string()),
                Some(text.to_string()),
            ),
            PlanRowView::Remove { old_line, text } => (
                FileDiffRowKind::Remove,
                Some(old_line),
                None,
                Some(text.to_string()),
                None,
            ),
            PlanRowView::Add { new_line, text } => (
                FileDiffRowKind::Add,
                None,
                Some(new_line),
                None,
                Some(text.to_string()),
            ),
            PlanRowView::Modify {
                old_line,
                new_line,
                old_text,
                new_text,
            } => (
                FileDiffRowKind::Modify,
                Some(old_line),
                Some(new_line),
                Some(old_text.to_string()),
                Some(new_text.to_string()),
            ),
        };
        plan_rows.push((kind, old_line, new_line, old_text, new_text));
    });

    assert_eq!(plan_rows.len(), rows.len());
    for (i, row) in rows.iter().enumerate() {
        let (kind, old_line, new_line, old_text, new_text) = &plan_rows[i];
        assert_eq!(*kind, row.kind, "row {i} kind mismatch");
        assert_eq!(*old_line, row.old_line, "row {i} old_line mismatch");
        assert_eq!(*new_line, row.new_line, "row {i} new_line mismatch");
        assert_eq!(
            old_text.as_deref(),
            row.old.as_deref(),
            "row {i} old text mismatch"
        );
        assert_eq!(
            new_text.as_deref(),
            row.new.as_deref(),
            "row {i} new text mismatch"
        );
    }
}

#[test]
fn for_each_side_by_side_row_empty_inputs() {
    let mut count = 0;
    for_each_side_by_side_row("", "", |_| count += 1);
    assert_eq!(count, 0);
}

#[test]
fn for_each_side_by_side_row_replacement_block() {
    let old = "start\nalpha\nbeta\nend\n";
    let new = "start\nintro\nalpha changed\nbeta changed\nend\n";

    let rows = side_by_side_rows(old, new);
    let mut kinds = Vec::new();
    for_each_side_by_side_row(old, new, |view| kinds.push(view.kind()));
    assert_eq!(kinds, rows.iter().map(|r| r.kind).collect::<Vec<_>>());
}

#[test]
fn myers_fallback_preserves_common_prefix_and_suffix() {
    let old = ["keep-1", "keep-2", "old-middle", "keep-3"];
    let new = ["keep-1", "keep-2", "new-middle", "keep-3"];
    let edits = myers_fallback_edits(&old, &new);

    assert_eq!(
        edits.iter().map(|edit| edit.kind).collect::<Vec<_>>(),
        vec![
            EditKind::Equal,
            EditKind::Equal,
            EditKind::Delete,
            EditKind::Insert,
            EditKind::Equal
        ]
    );
}

#[test]
fn positional_fallback_preserves_same_position_context_lines() {
    let old = ["repeat", "old-a", "repeat", "old-b", "repeat"];
    let new = ["repeat", "new-a", "repeat", "new-b", "repeat"];
    let edits = positional_fallback_edits(&old, &new);

    assert_eq!(
        edits.iter().map(|edit| edit.kind).collect::<Vec<_>>(),
        vec![
            EditKind::Equal,
            EditKind::Delete,
            EditKind::Insert,
            EditKind::Equal,
            EditKind::Delete,
            EditKind::Insert,
            EditKind::Equal,
        ]
    );
}

#[test]
fn myers_fallback_reserves_exactly_the_edits_it_pushes() {
    // These builders are reached only once the combined line count crosses
    // the linear-fallback threshold, so a loose reservation costs the most
    // exactly where it is taken. `old.len() + new.len()` counts every
    // prefix and suffix line on both sides.
    let old = ["keep", "old-a", "old-b", "tail"];
    let new = ["keep", "new-a", "tail"];

    let edits = myers_fallback_edits(&old, &new);

    assert_eq!(
        edits.capacity(),
        edits.len(),
        "prefix and suffix lines produce one edit, not two"
    );
}

#[test]
fn positional_fallback_reserves_its_exact_upper_bound() {
    // Every paired line differs, so the bound is also the exact count.
    let old = ["keep", "old-a", "old-b", "tail"];
    let new = ["keep", "new-a", "new-b", "tail"];

    let edits = positional_fallback_edits(&old, &new);

    assert_eq!(edits.capacity(), edits.len());

    // An interior line that matches emits one edit where the bound allowed
    // two. That slack is the only slack the reservation may carry.
    let old = ["keep", "old-a", "same", "old-b", "tail"];
    let new = ["keep", "new-a", "same", "new-b", "tail"];

    let edits = positional_fallback_edits(&old, &new);

    assert_eq!(edits.capacity(), edits.len() + 1);
}

#[test]
fn large_anchorless_repeated_regions_keep_context_localized() {
    let line_count = 700usize;
    let mut old_lines = Vec::with_capacity(line_count);
    let mut new_lines = Vec::with_capacity(line_count);

    for ix in 0..line_count {
        if ix % 2 == 0 {
            old_lines.push("repeat".to_string());
            new_lines.push("repeat".to_string());
        } else {
            old_lines.push(format!("before-{ix:04}"));
            new_lines.push(format!("after-{ix:04}"));
        }
    }

    let old = format!("{}\n", old_lines.join("\n"));
    let new = format!("{}\n", new_lines.join("\n"));
    let rows = side_by_side_rows(&old, &new);

    assert_eq!(rows.len(), line_count);
    for (ix, row) in rows.iter().enumerate() {
        let expected_kind = if ix % 2 == 0 {
            FileDiffRowKind::Context
        } else {
            FileDiffRowKind::Modify
        };
        assert_eq!(row.kind, expected_kind, "unexpected row kind at index {ix}");
    }
}

#[test]
fn patience_lis_handles_empty_and_descending_inputs_without_panicking() {
    assert!(patience_lis(&[]).is_empty());

    let descending = [(0usize, 3usize), (1, 2), (2, 1)];
    let lis = patience_lis(&descending);
    assert_eq!(lis.len(), 1);
    assert!(descending.contains(&lis[0]));
}

#[test]
fn side_by_side_large_files_keep_distant_changes_localized() {
    let line_count = 6_000;
    let mut old_lines: Vec<String> = (0..line_count).map(|i| format!("line-{i:05}")).collect();
    let mut new_lines = old_lines.clone();

    let first_change_ix = 137usize;
    let second_change_ix = line_count - 201;
    old_lines[first_change_ix] = "alpha-old".to_string();
    new_lines[first_change_ix] = "alpha-new".to_string();
    old_lines[second_change_ix] = "omega-old".to_string();
    new_lines[second_change_ix] = "omega-new".to_string();

    let old = format!("{}\n", old_lines.join("\n"));
    let new = format!("{}\n", new_lines.join("\n"));
    let rows = side_by_side_rows(&old, &new);

    let changed: Vec<&FileDiffRow> = rows
        .iter()
        .filter(|row| row.kind != FileDiffRowKind::Context)
        .collect();

    assert_eq!(
        changed.len(),
        2,
        "large files should not collapse distant edits into one huge changed block"
    );
    assert!(
        changed
            .iter()
            .all(|row| row.kind == FileDiffRowKind::Modify),
        "both changes should remain localized modify rows"
    );
    assert_eq!(changed[0].old_line, Some((first_change_ix + 1) as u32));
    assert_eq!(changed[0].new_line, Some((first_change_ix + 1) as u32));
    assert_eq!(changed[1].old_line, Some((second_change_ix + 1) as u32));
    assert_eq!(changed[1].new_line, Some((second_change_ix + 1) as u32));
}

#[test]
fn plan_inline_row_count_tracks_eof_newline_rewrite() {
    let old = "keep\nshared tail\n";
    let new = "keep\nshared tail";

    let plan = side_by_side_plan(old, new);
    let rows = side_by_side_rows(old, new);
    let inline_row_count = rows.len()
        + rows
            .iter()
            .filter(|row| row.kind == FileDiffRowKind::Modify)
            .count();

    assert_eq!(plan.eof_newline, Some(FileDiffEofNewline::MissingInNew));
    assert_eq!(plan.row_count, rows.len());
    assert_eq!(plan.inline_row_count, inline_row_count);
}

#[test]
fn direct_linear_fallback_plan_merges_prefix_middle_and_suffix_runs() {
    let old_text = "keep-1\nkeep-2\nold-a\nold-b\nkeep-3\n";
    let new_text = "keep-1\nkeep-2\nnew-a\nnew-b\nkeep-3\n";
    let old_lines = split_lines(old_text);
    let new_lines = split_lines(new_text);
    let plan = build_linear_fallback_side_by_side_plan_with_pair_cost(
        old_text,
        new_text,
        old_lines.as_slice(),
        new_lines.as_slice(),
        replacement_pair_cost,
    );

    assert_eq!(
        plan.runs,
        vec![
            FileDiffPlanRun::Context {
                old_start: 0,
                new_start: 0,
                len: 2,
            },
            FileDiffPlanRun::Modify {
                old_start: 2,
                new_start: 2,
                len: 2,
            },
            FileDiffPlanRun::Context {
                old_start: 4,
                new_start: 4,
                len: 1,
            },
        ]
    );
}

#[cfg(feature = "benchmarks")]
#[test]
fn benchmark_backends_match_current_plan() {
    let cases = [
        (
            "start\nprefix-only-change\nshared tail\nend\n",
            "start\nprefix-only-change extended\nshared tail\nend\n",
        ),
        (
            "alpha\nrepeated\nrepeated\nomega\n",
            "alpha\nrepeated changed\nrepeated changed\nomega\n",
        ),
        (
            "context\nfn café() {\n    return old_value;\n}\n",
            "context\nfn café() {\n    return new_value;\n}\n",
        ),
    ];

    for (old, new) in cases {
        let current = side_by_side_plan(old, new);
        let scratch = benchmark_side_by_side_plan_with_replacement_backend(
            old,
            new,
            BenchmarkReplacementDistanceBackend::Scratch,
        );
        let strsim = benchmark_side_by_side_plan_with_replacement_backend(
            old,
            new,
            BenchmarkReplacementDistanceBackend::Strsim,
        );
        assert_eq!(
            current, scratch,
            "scratch backend parity mismatch for old={old:?} new={new:?}"
        );
        assert_eq!(
            current, strsim,
            "backend parity mismatch for old={old:?} new={new:?}"
        );
    }
}

#[cfg(feature = "benchmarks")]
#[test]
fn levenshtein_scratch_matches_strsim_generic() {
    let mut scratch = LevenshteinScratch::default();

    for (old, new) in [
        ("before_source_001", "after_source_002"),
        ("prefix-only-change", "prefix-only-change extended"),
        ("short", "considerably-longer-string-for-levenshtein"),
        ("", "abc"),
    ] {
        let old_bytes = old.as_bytes();
        let new_bytes = new.as_bytes();
        let old_wrapper = ByteSlice(old_bytes);
        let new_wrapper = ByteSlice(new_bytes);
        assert_eq!(
            scratch.distance(old_bytes, new_bytes),
            strsim::generic_levenshtein::<ByteSlice<'_>, ByteSlice<'_>, u8, u8>(
                &old_wrapper,
                &new_wrapper
            ),
            "ascii mismatch for old={old:?} new={new:?}"
        );
    }

    for (old, new) in [("café", "caff"), ("prefix-é-suffix", "prefix-ê-suffix")] {
        let old_chars = old.chars().collect::<Vec<_>>();
        let new_chars = new.chars().collect::<Vec<_>>();
        let old_wrapper = CharSlice(old_chars.as_slice());
        let new_wrapper = CharSlice(new_chars.as_slice());
        assert_eq!(
            scratch.distance(old_chars.as_slice(), new_chars.as_slice()),
            strsim::generic_levenshtein::<CharSlice<'_>, CharSlice<'_>, char, char>(
                &old_wrapper,
                &new_wrapper
            ),
            "unicode mismatch for old={old:?} new={new:?}"
        );
    }
}

#[cfg(feature = "benchmarks")]
#[test]
fn bitparallel_ascii_levenshtein_matches_strsim_generic() {
    fn generate_cases(alphabet: &[u8], max_len: usize) -> Vec<Vec<u8>> {
        let mut cases = vec![Vec::new()];
        let mut frontier = vec![Vec::new()];

        for _ in 0..max_len {
            let mut next = Vec::new();
            for prefix in &frontier {
                for &byte in alphabet {
                    let mut candidate = prefix.clone();
                    candidate.push(byte);
                    next.push(candidate);
                }
            }
            cases.extend(next.iter().cloned());
            frontier = next;
        }

        cases
    }

    let cases = generate_cases(b"ab_", 4);
    let mut scratch = LevenshteinScratch::default();

    for old in &cases {
        for new in &cases {
            let old_wrapper = ByteSlice(old.as_slice());
            let new_wrapper = ByteSlice(new.as_slice());
            let expected = strsim::generic_levenshtein::<ByteSlice<'_>, ByteSlice<'_>, u8, u8>(
                &old_wrapper,
                &new_wrapper,
            );

            assert_eq!(
                bitparallel_levenshtein_bytes(old.as_slice(), new.as_slice()),
                Some(expected),
                "bitparallel mismatch for old={:?} new={:?}",
                String::from_utf8_lossy(old),
                String::from_utf8_lossy(new)
            );
            assert_eq!(
                scratch.distance_bytes(old.as_slice(), new.as_slice()),
                expected,
                "scratch ascii mismatch for old={:?} new={:?}",
                String::from_utf8_lossy(old),
                String::from_utf8_lossy(new)
            );
        }
    }
}
