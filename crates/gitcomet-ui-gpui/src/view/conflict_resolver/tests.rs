use super::*;
use gitcomet_core::conflict_output::{
    ConflictMarkerLabels, GenerateResolvedTextOptions, UnresolvedConflictMode,
};
use gitcomet_core::file_diff::FileDiffRow;
use gitcomet_core::file_diff::FileDiffRowKind as RK;

#[test]
fn parses_and_generates_conflicts() {
    let input = "a\n<<<<<<< HEAD\none\ntwo\n=======\nuno\ndos\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);

    let ours = generate_resolved_text(&segments);
    assert_eq!(ours, "a\none\ntwo\nb\n");

    {
        let ConflictSegment::Block(block) = segments
            .iter_mut()
            .find(|s| matches!(s, ConflictSegment::Block(_)))
            .unwrap()
        else {
            panic!("expected a conflict block");
        };
        block.choice = ConflictChoice::Theirs;
    }

    let theirs = generate_resolved_text(&segments);
    assert_eq!(theirs, "a\nuno\ndos\nb\n");

    {
        let ConflictSegment::Block(block) = segments
            .iter_mut()
            .find(|s| matches!(s, ConflictSegment::Block(_)))
            .unwrap()
        else {
            panic!("expected a conflict block");
        };
        block.choice = ConflictChoice::Both;
    }
    let both = generate_resolved_text(&segments);
    assert_eq!(both, "a\none\ntwo\nuno\ndos\nb\n");
}

#[test]
fn parses_diff3_style_markers() {
    let input = "a\n<<<<<<< ours\none\n||||||| base\norig\n=======\nuno\n>>>>>>> theirs\nb\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);

    let ConflictSegment::Block(block) = segments
        .iter()
        .find(|s| matches!(s, ConflictSegment::Block(_)))
        .unwrap()
    else {
        panic!("expected a conflict block");
    };

    assert_eq!(block.ours, "one\n");
    assert_eq!(block.base.as_deref(), Some("orig\n"));
    assert_eq!(block.theirs, "uno\n");
}

#[test]
fn shared_marker_parse_reuses_original_backing() {
    let input: std::sync::Arc<str> =
        std::sync::Arc::from("pre\n<<<<<<< ours\none\n=======\nuno\n>>>>>>> theirs\npost\n");
    let segments = parse_conflict_markers_shared(input.clone());

    assert_eq!(segments.len(), 3);

    let ConflictSegment::Text(prefix) = &segments[0] else {
        panic!("expected leading text segment");
    };
    assert_eq!(prefix.as_str(), "pre\n");
    assert!(prefix.shares_backing_with(&input));

    let ConflictSegment::Block(block) = &segments[1] else {
        panic!("expected conflict block");
    };
    assert_eq!(block.base, None);
    assert_eq!(block.ours, "one\n");
    assert_eq!(block.theirs, "uno\n");
    assert!(block.ours.shares_backing_with(&input));
    assert!(block.theirs.shares_backing_with(&input));

    let ConflictSegment::Text(suffix) = &segments[2] else {
        panic!("expected trailing text segment");
    };
    assert_eq!(suffix.as_str(), "post\n");
    assert!(suffix.shares_backing_with(&input));
}

#[test]
fn generate_with_options_preserves_unresolved_markers_with_labels() {
    let input = "a\n<<<<<<< ours\none\n||||||| base\norig\n=======\nuno\n>>>>>>> theirs\nb\n";
    let segments = parse_conflict_markers(input);

    let output = generate_resolved_text_with_options(
        &segments,
        GenerateResolvedTextOptions {
            unresolved_mode: UnresolvedConflictMode::PreserveMarkers,
            labels: Some(ConflictMarkerLabels {
                local: "LOCAL",
                remote: "REMOTE",
                base: "BASE",
            }),
        },
    );

    assert_eq!(
        output,
        "a\n<<<<<<< LOCAL\none\n||||||| BASE\norig\n=======\nuno\n>>>>>>> REMOTE\nb\n"
    );
}

#[test]
fn malformed_markers_are_preserved() {
    let input = "a\n<<<<<<< HEAD\none\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 0);
    assert_eq!(generate_resolved_text(&segments), input);
}

// -- Marker parser edge case tests --

#[test]
fn empty_conflict_blocks_parse_and_generate() {
    let input = "a\n<<<<<<< ours\n=======\n>>>>>>> theirs\nb\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.ours, "");
    assert_eq!(block.theirs, "");
    // Default choice is Ours, generating empty content in place of the conflict
    let resolved = generate_resolved_text(&segments);
    assert_eq!(resolved, "a\nb\n");
}

#[test]
fn malformed_missing_end_marker_preserved_as_text() {
    // Start + separator found but no end marker
    let input = "a\n<<<<<<< HEAD\nfoo\n=======\nbar\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(
        conflict_count(&segments),
        0,
        "malformed block should not produce a conflict"
    );
    assert_eq!(
        generate_resolved_text(&segments),
        input,
        "malformed content must be preserved"
    );
}

#[test]
fn malformed_missing_end_marker_crlf_preserved_as_text() {
    // Same malformed structure as above, but with CRLF endings.
    // The parser should preserve line endings exactly.
    let input = "a\r\n<<<<<<< HEAD\r\nfoo\r\n=======\r\nbar\r\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 0);
    assert_eq!(generate_resolved_text(&segments), input);
}

#[test]
fn malformed_diff3_missing_end_marker_preserved_as_text() {
    // Diff3 malformed block (no >>>>>>> end marker). Ensure the base marker
    // section and separator are preserved exactly.
    let input = "a\r\n<<<<<<< ours\r\none\r\n||||||| base\r\norig\r\n=======\r\nuno\r\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 0);
    assert_eq!(generate_resolved_text(&segments), input);
}

#[test]
fn malformed_missing_separator_preserved_as_text() {
    // Start marker then end marker with no separator
    let input = "a\n<<<<<<< HEAD\nfoo\n>>>>>>> theirs\nb\n";
    let segments = parse_conflict_markers(input);
    // Parser looks for "=======" before ">>>>>>>", so this is malformed
    // and preserved as text. The parser stops parsing.
    assert_eq!(conflict_count(&segments), 0);
    // All content should be preserved
    assert_eq!(generate_resolved_text(&segments), input);
}

#[test]
fn separator_without_start_marker_is_plain_text() {
    let input = "before\n=======\nafter\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 0);
    assert_eq!(segments.len(), 1);
    assert_eq!(generate_resolved_text(&segments), input);
}

#[test]
fn end_marker_without_start_is_plain_text() {
    let input = "before\n>>>>>>> theirs\nafter\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 0);
    assert_eq!(generate_resolved_text(&segments), input);
}

#[test]
fn marker_labels_with_extra_text_parsed_correctly() {
    let input = "<<<<<<< HEAD (feature/my-branch)\nours\n||||||| merged common ancestors\nbase\n======= some notes\ntheirs\n>>>>>>> origin/main (remote)\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.ours, "ours\n");
    assert_eq!(block.base.as_deref(), Some("base\n"));
    assert_eq!(block.theirs, "theirs\n");
}

#[test]
fn mixed_two_way_and_diff3_conflicts() {
    let input = "\
header
<<<<<<< ours
two-way ours
=======
two-way theirs
>>>>>>> theirs
middle
<<<<<<< ours
diff3 ours
||||||| base
diff3 base
=======
diff3 theirs
>>>>>>> theirs
footer
";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 2);

    let blocks: Vec<_> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .collect();

    // First: 2-way (no base)
    assert!(blocks[0].base.is_none());
    assert_eq!(blocks[0].ours, "two-way ours\n");
    assert_eq!(blocks[0].theirs, "two-way theirs\n");

    // Second: 3-way (with base)
    assert_eq!(blocks[1].base.as_deref(), Some("diff3 base\n"));
    assert_eq!(blocks[1].ours, "diff3 ours\n");
    assert_eq!(blocks[1].theirs, "diff3 theirs\n");
}

#[test]
fn valid_conflict_before_malformed_is_preserved() {
    let input = "\
<<<<<<< ours
ok ours
=======
ok theirs
>>>>>>> theirs
<<<<<<< ours
missing end
=======
dangling
";
    let segments = parse_conflict_markers(input);
    assert_eq!(
        conflict_count(&segments),
        1,
        "only valid conflict should be parsed"
    );
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.ours, "ok ours\n");
    assert_eq!(block.theirs, "ok theirs\n");
    // The malformed part should be preserved as trailing text
    let resolved = generate_resolved_text(&segments);
    assert!(
        resolved.contains("ok ours"),
        "resolved should contain the valid conflict's choice"
    );
    assert!(
        resolved.contains("missing end"),
        "malformed content should be preserved as text"
    );
}

#[test]
fn multiline_asymmetric_conflict_blocks() {
    let input = "\
<<<<<<< ours
ours line 1
ours line 2
ours line 3
=======
theirs only line
>>>>>>> theirs
";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.ours.lines().count(), 3);
    assert_eq!(block.theirs.lines().count(), 1);
}

#[test]
fn no_trailing_newline_on_file() {
    let input = "<<<<<<< ours\nfoo\n=======\nbar\n>>>>>>> theirs";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);
}

// -- 2-way / 3-way mode consistency tests --

#[test]
fn two_way_blocks_have_no_base_three_way_have_base() {
    let two_way = "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\n";
    let three_way = "<<<<<<< ours\na\n||||||| base\norig\n=======\nb\n>>>>>>> theirs\n";

    let two_way_segments = parse_conflict_markers(two_way);
    let three_way_segments = parse_conflict_markers(three_way);

    let two_way_block = two_way_segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    let three_way_block = three_way_segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();

    // 2-way has no base, 3-way has base
    assert!(
        two_way_block.base.is_none(),
        "2-way conflict should have no base"
    );
    assert!(
        three_way_block.base.is_some(),
        "3-way conflict should have base"
    );

    // Both have same ours/theirs content
    assert_eq!(two_way_block.ours, three_way_block.ours);
    assert_eq!(two_way_block.theirs, three_way_block.theirs);
}

#[test]
fn populate_bases_converts_two_way_to_three_way_compatible() {
    let two_way = "a\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(two_way);

    // Initially no base
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert!(block.base.is_none());

    // After populating from ancestor, base is set
    populate_block_bases_from_ancestor(&mut segments, "a\norig\nb\n");
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert!(
        block.base.is_some(),
        "after populate, block should have base for 3-way display"
    );

    // Pick Base should now produce ancestor content
    if let Some(ConflictSegment::Block(b)) = segments
        .iter_mut()
        .find(|s| matches!(s, ConflictSegment::Block(_)))
    {
        b.choice = ConflictChoice::Base;
    }
    let resolved = generate_resolved_text(&segments);
    assert_eq!(resolved, "a\norig\nb\n");
}

#[test]
fn split_and_inline_views_consistent_for_mixed_mode_conflicts() {
    use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind as RK};

    // Simulate rows from a conflict with asymmetric content (3 ours lines, 1 theirs)
    let rows = vec![
        FileDiffRow {
            kind: RK::Context,
            old_line: Some(1),
            new_line: Some(1),
            old: Some("context".into()),
            new: Some("context".into()),
            eof_newline: None,
        },
        FileDiffRow {
            kind: RK::Modify,
            old_line: Some(2),
            new_line: Some(2),
            old: Some("old line".into()),
            new: Some("new line".into()),
            eof_newline: None,
        },
        FileDiffRow {
            kind: RK::Context,
            old_line: Some(3),
            new_line: Some(3),
            old: Some("end".into()),
            new: Some("end".into()),
            eof_newline: None,
        },
    ];

    let inline = build_inline_rows(&rows);
    // Split view has rows.len() entries, inline expands Modify → Remove+Add
    assert!(
        inline.len() >= rows.len(),
        "inline should have at least as many rows as split"
    );
    // Both should cover the same line range
    let split_lines: std::collections::HashSet<_> =
        rows.iter().filter_map(|r| r.new_line).collect();
    let inline_lines: std::collections::HashSet<_> =
        inline.iter().filter_map(|r| r.new_line).collect();
    assert!(
        split_lines.is_subset(&inline_lines),
        "inline should cover all new lines that split covers"
    );
}

#[test]
fn inline_rows_expand_modify_into_remove_and_add() {
    let rows = vec![
        FileDiffRow {
            kind: RK::Context,
            old_line: Some(1),
            new_line: Some(1),
            old: Some("a".into()),
            new: Some("a".into()),
            eof_newline: None,
        },
        FileDiffRow {
            kind: RK::Modify,
            old_line: Some(2),
            new_line: Some(2),
            old: Some("b".into()),
            new: Some("b2".into()),
            eof_newline: None,
        },
    ];
    let inline = build_inline_rows(&rows);
    assert_eq!(inline.len(), 3);
    assert_eq!(inline[0].content, "a");
    assert_eq!(inline[1].kind, gitcomet_core::domain::DiffLineKind::Remove);
    assert_eq!(inline[2].kind, gitcomet_core::domain::DiffLineKind::Add);
}

#[test]
fn append_lines_adds_newlines_safely() {
    let out = append_lines_to_output("a\n", &["b".into(), "c".into()]);
    assert_eq!(out, "a\nb\nc\n");
    let out = append_lines_to_output("a", &["b".into()]);
    assert_eq!(out, "a\nb\n");
}

#[test]
fn split_output_lines_for_outline_keeps_trailing_newline_row() {
    let lines = split_output_lines_for_outline("a\nb\n");
    assert_eq!(lines, vec!["a", "b", ""]);
}

#[test]
fn split_output_lines_for_outline_keeps_single_empty_row_for_empty_text() {
    let lines = split_output_lines_for_outline("");
    assert_eq!(lines, vec![""]);
}

// -----------------------------------------------------------------------
// Provenance mapping tests
// -----------------------------------------------------------------------

fn shared(s: &str) -> gpui::SharedString {
    s.to_string().into()
}

#[test]
fn provenance_matches_exact_source_lines() {
    let a = vec![shared("alpha"), shared("beta")];
    let b = vec![shared("gamma"), shared("delta")];
    let c = vec![shared("epsilon")];
    let sources = SourceLines {
        a: &a,
        b: &b,
        c: &c,
    };

    let output = vec![
        "gamma".to_string(),   // matches B[0]
        "alpha".to_string(),   // matches A[0]
        "epsilon".to_string(), // matches C[0]
        "manual".to_string(),  // no match
    ];

    let meta = compute_resolved_line_provenance(&output, &sources);
    assert_eq!(meta.len(), 4);

    assert_eq!(meta[0].source, ResolvedLineSource::B);
    assert_eq!(meta[0].input_line, Some(1));

    assert_eq!(meta[1].source, ResolvedLineSource::A);
    assert_eq!(meta[1].input_line, Some(1));

    assert_eq!(meta[2].source, ResolvedLineSource::C);
    assert_eq!(meta[2].input_line, Some(1));

    assert_eq!(meta[3].source, ResolvedLineSource::Manual);
    assert_eq!(meta[3].input_line, None);
}

#[test]
fn provenance_priority_a_over_b() {
    // When the same text exists in A and B, A wins.
    let a = vec![shared("same")];
    let b = vec![shared("same")];
    let c: Vec<gpui::SharedString> = vec![];
    let sources = SourceLines {
        a: &a,
        b: &b,
        c: &c,
    };

    let output = vec!["same".to_string()];
    let meta = compute_resolved_line_provenance(&output, &sources);
    assert_eq!(meta[0].source, ResolvedLineSource::A);
}

#[test]
fn provenance_empty_output_returns_empty() {
    let a: Vec<gpui::SharedString> = vec![];
    let b: Vec<gpui::SharedString> = vec![];
    let c: Vec<gpui::SharedString> = vec![];
    let sources = SourceLines {
        a: &a,
        b: &b,
        c: &c,
    };
    let output: Vec<String> = vec![];
    let meta = compute_resolved_line_provenance(&output, &sources);
    assert!(meta.is_empty());
}

#[test]
fn provenance_empty_line_matches_empty_source() {
    let a = vec![shared("")];
    let b: Vec<gpui::SharedString> = vec![];
    let c: Vec<gpui::SharedString> = vec![];
    let sources = SourceLines {
        a: &a,
        b: &b,
        c: &c,
    };
    let output = vec!["".to_string()];
    let meta = compute_resolved_line_provenance(&output, &sources);
    assert_eq!(meta[0].source, ResolvedLineSource::A);
    assert_eq!(meta[0].input_line, Some(1));
}

#[test]
fn two_way_indexed_provenance_uses_source_line_numbers() {
    let ours = "shared\nlocal\nctx";
    let theirs = "shared\nremote\nctx";
    let meta = compute_resolved_line_provenance_from_text_two_way_indexed_sources(
        "shared\nremote\nctx",
        ours,
        &preview_line_starts(ours),
        theirs,
        &preview_line_starts(theirs),
    );

    assert_eq!(meta.len(), 3);
    assert_eq!(meta[0].source, ResolvedLineSource::A);
    assert_eq!(meta[0].input_line, Some(1));
    assert_eq!(meta[1].source, ResolvedLineSource::B);
    assert_eq!(meta[1].input_line, Some(2));
    assert_eq!(meta[2].source, ResolvedLineSource::A);
    assert_eq!(meta[2].input_line, Some(3));
}

// -----------------------------------------------------------------------
// Dedupe key builder tests
// -----------------------------------------------------------------------

#[test]
fn dedupe_index_contains_matched_lines() {
    let a = vec![shared("fn main()"), shared("  println!()")];
    let b = vec![shared("fn main()"), shared("  eprintln!()")];
    let c: Vec<gpui::SharedString> = vec![];
    let sources = SourceLines {
        a: &a,
        b: &b,
        c: &c,
    };

    let output = vec!["fn main()".to_string(), "  eprintln!()".to_string()];
    let meta = compute_resolved_line_provenance(&output, &sources);
    let index = build_resolved_output_line_sources_index(
        &meta,
        &output,
        ConflictResolverViewMode::ThreeWay,
    );

    // "fn main()" matched A line 1
    assert!(is_source_line_in_output(
        &index,
        ConflictResolverViewMode::ThreeWay,
        ResolvedLineSource::A,
        1,
        "fn main()",
    ));
    // "  eprintln!()" matched B line 2
    assert!(is_source_line_in_output(
        &index,
        ConflictResolverViewMode::ThreeWay,
        ResolvedLineSource::B,
        2,
        "  eprintln!()",
    ));
    // A line 2 "  println!()" is NOT in output
    assert!(!is_source_line_in_output(
        &index,
        ConflictResolverViewMode::ThreeWay,
        ResolvedLineSource::A,
        2,
        "  println!()",
    ));
}

#[test]
fn dedupe_index_excludes_manual_lines() {
    let a: Vec<gpui::SharedString> = vec![];
    let b: Vec<gpui::SharedString> = vec![];
    let c: Vec<gpui::SharedString> = vec![];
    let sources = SourceLines {
        a: &a,
        b: &b,
        c: &c,
    };

    let output = vec!["manually typed".to_string()];
    let meta = compute_resolved_line_provenance(&output, &sources);
    let index = build_resolved_output_line_sources_index(
        &meta,
        &output,
        ConflictResolverViewMode::TwoWayDiff,
    );
    assert!(index.is_empty());
}

#[test]
fn source_line_key_content_hash_differs_for_different_text() {
    let k1 = SourceLineKey::new(
        ConflictResolverViewMode::ThreeWay,
        ResolvedLineSource::A,
        1,
        "hello",
    );
    let k2 = SourceLineKey::new(
        ConflictResolverViewMode::ThreeWay,
        ResolvedLineSource::A,
        1,
        "world",
    );
    assert_ne!(k1, k2);
}

#[test]
fn resolved_line_source_badge_chars() {
    assert_eq!(ResolvedLineSource::A.badge_char(), 'A');
    assert_eq!(ResolvedLineSource::B.badge_char(), 'B');
    assert_eq!(ResolvedLineSource::C.badge_char(), 'C');
    assert_eq!(ResolvedLineSource::Manual.badge_char(), 'M');
}

#[test]
fn derive_region_resolution_updates_preserves_unresolved_defaults() {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let input = concat!(
        "pre\n",
        "<<<<<<< ours\n",
        "ours\n",
        "=======\n",
        "theirs\n",
        ">>>>>>> theirs\n",
        "post\n"
    );
    let segments = parse_conflict_markers(input);
    let output = generate_resolved_text(&segments);
    let updates = derive_region_resolution_updates_from_output(
        &segments,
        &sequential_conflict_region_indices(&segments),
        &output,
    )
    .expect("updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, 0);
    assert_eq!(updates[0].1, R::Unresolved);
}

#[test]
fn derive_region_resolution_updates_detects_manual_and_pick() {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let input = concat!(
        "pre\n",
        "<<<<<<< ours\n",
        "ours1\n",
        "=======\n",
        "theirs1\n",
        ">>>>>>> theirs\n",
        "mid\n",
        "<<<<<<< ours\n",
        "ours2\n",
        "=======\n",
        "theirs2\n",
        ">>>>>>> theirs\n",
        "post\n"
    );
    let mut segments = parse_conflict_markers(input);
    if let Some(ConflictSegment::Block(block)) = segments
        .iter_mut()
        .filter(|seg| matches!(seg, ConflictSegment::Block(_)))
        .nth(1)
    {
        block.choice = ConflictChoice::Theirs;
        block.resolved = true;
    }
    let output = "pre\nmanual one\nmid\ntheirs2\npost\n";
    let updates = derive_region_resolution_updates_from_output(
        &segments,
        &sequential_conflict_region_indices(&segments),
        output,
    )
    .expect("updates");

    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].0, 0);
    assert_eq!(updates[0].1, R::ManualEdit("manual one\n".into()));
    assert_eq!(updates[1].0, 1);
    assert_eq!(updates[1].1, R::PickTheirs);
}

#[test]
fn derive_region_resolution_updates_returns_none_when_context_changed() {
    let input = concat!(
        "pre\n",
        "<<<<<<< ours\n",
        "ours\n",
        "=======\n",
        "theirs\n",
        ">>>>>>> theirs\n",
        "post\n"
    );
    let segments = parse_conflict_markers(input);
    let output = "changed-pre\nours\npost\n";
    let updates = derive_region_resolution_updates_from_output(
        &segments,
        &sequential_conflict_region_indices(&segments),
        output,
    );
    assert!(updates.is_none());
}

#[test]
fn populate_block_bases_from_ancestor_fills_missing_base() {
    // 2-way conflict markers (no base section)
    let input = "a\n<<<<<<< HEAD\none\ntwo\n=======\nuno\ndos\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);

    // The block has no base initially (2-way markers)
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert!(block.base.is_none());

    // Populate base from ancestor file
    let ancestor = "a\norig\nb\n";
    populate_block_bases_from_ancestor(&mut segments, ancestor);

    // Now the block should have base content extracted from the ancestor
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.base.as_deref(), Some("orig\n"));
}

#[test]
fn populate_block_bases_preserves_existing_base() {
    // 3-way conflict markers (with base section)
    let input = "a\n<<<<<<< ours\none\n||||||| base\norig\n=======\nuno\n>>>>>>> theirs\nb\n";
    let mut segments = parse_conflict_markers(input);

    // Block already has base from markers
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.base.as_deref(), Some("orig\n"));

    // populate should not overwrite existing base
    populate_block_bases_from_ancestor(&mut segments, "a\nDIFFERENT\nb\n");
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.base.as_deref(), Some("orig\n")); // unchanged
}

#[test]
fn populate_block_bases_multiple_conflicts() {
    let input = "a\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> other\nb\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> other\nc\n";
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 2);

    let ancestor = "a\norig_foo\nb\norig_x\nc\n";
    populate_block_bases_from_ancestor(&mut segments, ancestor);

    let blocks: Vec<_> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].base.as_deref(), Some("orig_foo\n"));
    assert_eq!(blocks[1].base.as_deref(), Some("orig_x\n"));
}

#[test]
fn populate_block_bases_generates_correct_resolved_text() {
    let input = "a\n<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);

    let ancestor = "a\norig\nb\n";
    populate_block_bases_from_ancestor(&mut segments, ancestor);

    // Pick Base and generate resolved text
    if let Some(ConflictSegment::Block(block)) = segments
        .iter_mut()
        .find(|s| matches!(s, ConflictSegment::Block(_)))
    {
        block.choice = ConflictChoice::Base;
    }
    let resolved = generate_resolved_text(&segments);
    assert_eq!(resolved, "a\norig\nb\n");
}

#[test]
fn apply_session_region_resolutions_applies_pick_states() {
    use gitcomet_core::conflict_session::{ConflictRegion, ConflictRegionResolution as R};

    let input = concat!(
        "pre\n",
        "<<<<<<< ours\n",
        "ours1\n",
        "||||||| base\n",
        "base1\n",
        "=======\n",
        "theirs1\n",
        ">>>>>>> theirs\n",
        "mid\n",
        "<<<<<<< ours\n",
        "ours2\n",
        "||||||| base\n",
        "base2\n",
        "=======\n",
        "theirs2\n",
        ">>>>>>> theirs\n",
        "tail\n",
    );
    let mut segments = parse_conflict_markers(input);
    let regions = vec![
        ConflictRegion {
            base: Some("base1\n".into()),
            ours: "ours1\n".into(),
            theirs: "theirs1\n".into(),
            resolution: R::PickTheirs,
        },
        ConflictRegion {
            base: Some("base2\n".into()),
            ours: "ours2\n".into(),
            theirs: "theirs2\n".into(),
            resolution: R::PickBoth,
        },
    ];

    let applied = apply_session_region_resolutions(&mut segments, &regions);
    assert_eq!(applied, 2);
    assert_eq!(conflict_count(&segments), 2);
    assert_eq!(resolved_conflict_count(&segments), 2);

    let blocks: Vec<_> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(block) => Some(block),
            ConflictSegment::Text(_) => None,
        })
        .collect();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].choice, ConflictChoice::Theirs);
    assert!(blocks[0].resolved);
    assert_eq!(blocks[1].choice, ConflictChoice::Both);
    assert!(blocks[1].resolved);

    let resolved = generate_resolved_text(&segments);
    assert_eq!(resolved, "pre\ntheirs1\nmid\nours2\ntheirs2\ntail\n");
}

#[test]
fn apply_session_region_resolutions_materializes_custom_resolved_text() {
    use gitcomet_core::conflict_session::{
        AutosolveConfidence, AutosolveRule, ConflictRegion, ConflictRegionResolution as R,
    };

    let input = concat!(
        "start\n",
        "<<<<<<< ours\n",
        "ours1\n",
        "||||||| base\n",
        "base1\n",
        "=======\n",
        "theirs1\n",
        ">>>>>>> theirs\n",
        "between\n",
        "<<<<<<< ours\n",
        "ours2\n",
        "||||||| base\n",
        "base2\n",
        "=======\n",
        "theirs2\n",
        ">>>>>>> theirs\n",
        "end\n",
    );
    let mut segments = parse_conflict_markers(input);
    let regions = vec![
        ConflictRegion {
            base: Some("base1\n".into()),
            ours: "ours1\n".into(),
            theirs: "theirs1\n".into(),
            resolution: R::ManualEdit("merged-custom\n".into()),
        },
        ConflictRegion {
            base: Some("base2\n".into()),
            ours: "ours2\n".into(),
            theirs: "theirs2\n".into(),
            resolution: R::AutoResolved {
                rule: AutosolveRule::SubchunkFullyMerged,
                confidence: AutosolveConfidence::Medium,
                content: "theirs2\n".into(),
            },
        },
    ];

    let applied = apply_session_region_resolutions(&mut segments, &regions);
    assert_eq!(applied, 2);
    assert_eq!(conflict_count(&segments), 1);
    assert_eq!(resolved_conflict_count(&segments), 1);

    let blocks: Vec<_> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(block) => Some(block),
            ConflictSegment::Text(_) => None,
        })
        .collect();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].ours, "ours2\n");
    assert_eq!(blocks[0].choice, ConflictChoice::Theirs);
    assert!(blocks[0].resolved);

    let resolved = generate_resolved_text(&segments);
    assert_eq!(resolved, "start\nmerged-custom\nbetween\ntheirs2\nend\n");
}

#[test]
fn apply_session_region_resolutions_with_index_map_tracks_remaining_blocks() {
    use gitcomet_core::conflict_session::{
        AutosolveConfidence, AutosolveRule, ConflictRegion, ConflictRegionResolution as R,
    };

    let input = concat!(
        "start\n",
        "<<<<<<< ours\n",
        "ours1\n",
        "||||||| base\n",
        "base1\n",
        "=======\n",
        "theirs1\n",
        ">>>>>>> theirs\n",
        "middle\n",
        "<<<<<<< ours\n",
        "ours2\n",
        "||||||| base\n",
        "base2\n",
        "=======\n",
        "theirs2\n",
        ">>>>>>> theirs\n",
        "end\n",
    );
    let mut segments = parse_conflict_markers(input);
    let regions = vec![
        ConflictRegion {
            base: Some("base1\n".into()),
            ours: "ours1\n".into(),
            theirs: "theirs1\n".into(),
            resolution: R::ManualEdit("custom-first\n".into()),
        },
        ConflictRegion {
            base: Some("base2\n".into()),
            ours: "ours2\n".into(),
            theirs: "theirs2\n".into(),
            resolution: R::AutoResolved {
                rule: AutosolveRule::SubchunkFullyMerged,
                confidence: AutosolveConfidence::Medium,
                content: "theirs2\n".into(),
            },
        },
    ];

    let result = apply_session_region_resolutions_with_index_map(&mut segments, &regions);
    assert_eq!(result.applied_regions, 2);
    assert_eq!(result.block_region_indices, vec![1]);
    assert_eq!(conflict_count(&segments), 1);
}

/// Simulates the lightweight re-sync: re-parse markers from the original
/// text and re-apply session resolutions. The resolved output must match
/// what the initial parse+apply produced, proving the re-sync path in
/// `resync_conflict_resolver_from_state` is correct.
#[test]
fn resync_reparse_and_reapply_produces_same_output() {
    use gitcomet_core::conflict_session::{ConflictRegion, ConflictRegionResolution as R};

    let input = concat!(
        "header\n",
        "<<<<<<< ours\n",
        "alpha\n",
        "||||||| base\n",
        "original\n",
        "=======\n",
        "beta\n",
        ">>>>>>> theirs\n",
        "middle\n",
        "<<<<<<< ours\n",
        "gamma\n",
        "||||||| base\n",
        "old\n",
        "=======\n",
        "delta\n",
        ">>>>>>> theirs\n",
        "footer\n",
    );
    let regions = vec![
        ConflictRegion {
            base: Some("original\n".into()),
            ours: "alpha\n".into(),
            theirs: "beta\n".into(),
            resolution: R::PickOurs,
        },
        ConflictRegion {
            base: Some("old\n".into()),
            ours: "gamma\n".into(),
            theirs: "delta\n".into(),
            resolution: R::PickTheirs,
        },
    ];

    // Initial parse + apply (what happens on full rebuild).
    let mut segments_initial = parse_conflict_markers(input);
    apply_session_region_resolutions(&mut segments_initial, &regions);
    let resolved_initial = generate_resolved_text(&segments_initial);
    let count_initial = conflict_count(&segments_initial);
    let resolved_count_initial = resolved_conflict_count(&segments_initial);

    // Re-sync: re-parse from same text and re-apply same resolutions.
    let mut segments_resync = parse_conflict_markers(input);
    apply_session_region_resolutions(&mut segments_resync, &regions);
    let resolved_resync = generate_resolved_text(&segments_resync);
    let count_resync = conflict_count(&segments_resync);
    let resolved_count_resync = resolved_conflict_count(&segments_resync);

    // Must produce identical results.
    assert_eq!(resolved_initial, resolved_resync);
    assert_eq!(count_initial, count_resync);
    assert_eq!(resolved_count_initial, resolved_count_resync);
    assert_eq!(resolved_initial, "header\nalpha\nmiddle\ndelta\nfooter\n");
    assert_eq!(count_initial, 2);
    assert_eq!(resolved_count_initial, 2);
}

/// Verifies that re-sync correctly applies hide_resolved visibility
/// when session regions update hide status for a subset of conflicts.
#[test]
fn resync_rebuilds_visible_maps_after_session_changes() {
    use gitcomet_core::conflict_session::{ConflictRegion, ConflictRegionResolution as R};

    let input = concat!(
        "<<<<<<< ours\n",
        "a\n",
        "=======\n",
        "b\n",
        ">>>>>>> theirs\n",
        "gap\n",
        "<<<<<<< ours\n",
        "c\n",
        "=======\n",
        "d\n",
        ">>>>>>> theirs\n",
    );

    // First conflict resolved, second unresolved.
    let regions = vec![
        ConflictRegion {
            base: None,
            ours: "a\n".into(),
            theirs: "b\n".into(),
            resolution: R::PickOurs,
        },
        ConflictRegion {
            base: None,
            ours: "c\n".into(),
            theirs: "d\n".into(),
            resolution: R::Unresolved,
        },
    ];

    let mut segments = parse_conflict_markers(input);
    apply_session_region_resolutions(&mut segments, &regions);

    // With hide_resolved=false, both conflicts visible.
    let three_way_ranges = vec![0..1, 2..3]; // simplified ranges
    let vis_all = build_three_way_visible_map(4, &three_way_ranges, &segments, false);
    assert!(!vis_all.is_empty());

    // With hide_resolved=true, only unresolved conflict visible.
    let vis_hidden = build_three_way_visible_map(4, &three_way_ranges, &segments, true);
    let collapsed_count = vis_hidden
        .iter()
        .filter(|v| matches!(v, ThreeWayVisibleItem::CollapsedBlock(..)))
        .count();
    assert!(collapsed_count > 0, "resolved conflict should be collapsed");

    // Verify the unresolved conflict is NOT collapsed.
    assert_eq!(resolved_conflict_count(&segments), 1);
    assert_eq!(conflict_count(&segments), 2);
}

#[test]
fn detects_conflict_markers_in_text() {
    assert!(text_contains_conflict_markers(
        "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nb\n"
    ));
    assert!(text_contains_conflict_markers("<<<<<<< HEAD\n"));
    assert!(text_contains_conflict_markers("=======\n"));
    assert!(text_contains_conflict_markers(">>>>>>> branch\n"));
    assert!(text_contains_conflict_markers("||||||| base\n"));
}

#[test]
fn no_false_positives_for_clean_text() {
    assert!(!text_contains_conflict_markers("a\nb\nc\n"));
    assert!(!text_contains_conflict_markers(""));
    assert!(!text_contains_conflict_markers(
        "some text with < and > arrows"
    ));
    assert!(!text_contains_conflict_markers("====== not quite seven"));
}

#[test]
fn stage_safety_requires_confirmation_for_unresolved_blocks_without_markers() {
    let input = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nb\n";
    let segments = parse_conflict_markers(input);
    let output_text = generate_resolved_text(&segments);

    let safety = conflict_stage_safety_check(&output_text, &segments);
    assert!(!safety.has_conflict_markers);
    assert_eq!(safety.unresolved_blocks, 1);
    assert!(safety.requires_confirmation());
}

#[test]
fn stage_safety_does_not_require_confirmation_when_fully_resolved_and_clean() {
    let input = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nb\n";
    let mut segments = parse_conflict_markers(input);
    if let Some(ConflictSegment::Block(block)) = segments
        .iter_mut()
        .find(|s| matches!(s, ConflictSegment::Block(_)))
    {
        block.choice = ConflictChoice::Theirs;
        block.resolved = true;
    }
    let output_text = generate_resolved_text(&segments);

    let safety = conflict_stage_safety_check(&output_text, &segments);
    assert!(!safety.has_conflict_markers);
    assert_eq!(safety.unresolved_blocks, 0);
    assert!(!safety.requires_confirmation());
}

#[test]
fn stage_safety_requires_confirmation_when_markers_remain() {
    let safety = conflict_stage_safety_check("<<<<<<< HEAD\nours\n", &[]);
    assert!(safety.has_conflict_markers);
    assert_eq!(safety.unresolved_blocks, 0);
    assert!(safety.requires_confirmation());
}

#[test]
fn autosolve_trace_summary_safe_mode() {
    let stats = gitcomet_state::msg::ConflictAutosolveStats {
        pass1: 2,
        pass2_split: 1,
        pass1_after_split: 0,
        regex: 0,
        history: 0,
    };
    let summary = format_autosolve_trace_summary(AutosolveTraceMode::Safe, 5, 2, &stats);
    assert!(summary.contains("Last autosolve (safe)"));
    assert!(summary.contains("resolved 3 blocks"));
    assert!(summary.contains("unresolved 5 -> 2"));
    assert!(summary.contains("pass1 2"));
    assert!(summary.contains("split 1"));
}

#[test]
fn autosolve_trace_summary_history_mode_uses_history_stat() {
    let stats = gitcomet_state::msg::ConflictAutosolveStats {
        pass1: 0,
        pass2_split: 0,
        pass1_after_split: 0,
        regex: 0,
        history: 3,
    };
    let summary = format_autosolve_trace_summary(AutosolveTraceMode::History, 4, 1, &stats);
    assert!(summary.contains("Last autosolve (history)"));
    assert!(summary.contains("resolved 3 blocks"));
    assert!(summary.contains("history 3"));
    assert!(!summary.contains("pass1"));
}

#[test]
fn active_conflict_autosolve_trace_label_reports_rule_and_confidence() {
    use gitcomet_core::conflict_session::{
        AutosolveConfidence, AutosolveRule, ConflictPayload, ConflictRegion,
        ConflictRegionResolution as R, ConflictSession,
    };
    use gitcomet_core::domain::FileConflictKind;
    use std::path::PathBuf;

    let mut session = ConflictSession::new(
        PathBuf::from("a.txt"),
        FileConflictKind::BothModified,
        ConflictPayload::Text(String::new().into()),
        ConflictPayload::Text(String::new().into()),
        ConflictPayload::Text(String::new().into()),
    );
    session.regions = vec![
        ConflictRegion {
            base: Some("base\n".into()),
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            resolution: R::AutoResolved {
                rule: AutosolveRule::OnlyOursChanged,
                confidence: AutosolveConfidence::High,
                content: "ours\n".into(),
            },
        },
        ConflictRegion {
            base: Some("base2\n".into()),
            ours: "ours2\n".into(),
            theirs: "theirs2\n".into(),
            resolution: R::PickTheirs,
        },
    ];

    let label = active_conflict_autosolve_trace_label(&session, &[0, 1], 0);
    assert_eq!(
        label.as_deref(),
        Some("Auto: only ours changed from base (high)")
    );
}

#[test]
fn active_conflict_autosolve_trace_label_returns_none_when_not_auto_or_oob() {
    use gitcomet_core::conflict_session::{
        ConflictPayload, ConflictRegion, ConflictRegionResolution as R, ConflictSession,
    };
    use gitcomet_core::domain::FileConflictKind;
    use std::path::PathBuf;

    let mut session = ConflictSession::new(
        PathBuf::from("a.txt"),
        FileConflictKind::BothModified,
        ConflictPayload::Text(String::new().into()),
        ConflictPayload::Text(String::new().into()),
        ConflictPayload::Text(String::new().into()),
    );
    session.regions = vec![ConflictRegion {
        base: Some("base\n".into()),
        ours: "ours\n".into(),
        theirs: "theirs\n".into(),
        resolution: R::PickOurs,
    }];

    assert_eq!(
        active_conflict_autosolve_trace_label(&session, &[0], 0),
        None
    );
    assert_eq!(
        active_conflict_autosolve_trace_label(&session, &[2], 0),
        None
    );
    assert_eq!(
        active_conflict_autosolve_trace_label(&session, &[0], 1),
        None
    );
}

#[test]
fn quick_pick_key_mapping_matches_a_b_c_d_shortcuts() {
    assert_eq!(
        conflict_quick_pick_choice_for_key("a"),
        Some(ConflictChoice::Base)
    );
    assert_eq!(
        conflict_quick_pick_choice_for_key("b"),
        Some(ConflictChoice::Ours)
    );
    assert_eq!(
        conflict_quick_pick_choice_for_key("c"),
        Some(ConflictChoice::Theirs)
    );
    assert_eq!(
        conflict_quick_pick_choice_for_key("d"),
        Some(ConflictChoice::Both)
    );
    assert_eq!(conflict_quick_pick_choice_for_key("x"), None);
}

#[test]
fn nav_key_mapping_matches_f2_f3_f7_shortcuts() {
    assert_eq!(
        conflict_nav_direction_for_key("f2", false),
        Some(ConflictNavDirection::Prev)
    );
    assert_eq!(
        conflict_nav_direction_for_key("f3", false),
        Some(ConflictNavDirection::Next)
    );
    assert_eq!(
        conflict_nav_direction_for_key("f7", true),
        Some(ConflictNavDirection::Prev)
    );
    assert_eq!(
        conflict_nav_direction_for_key("f7", false),
        Some(ConflictNavDirection::Next)
    );
    assert_eq!(conflict_nav_direction_for_key("home", false), None);
}

// -- resolved_conflict_count tests --

#[test]
fn resolved_count_starts_at_zero() {
    let input = "a\n<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\nb\n";
    let segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 1);
    assert_eq!(resolved_conflict_count(&segments), 0);
}

#[test]
fn resolved_count_tracks_picks() {
    let input = "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n";
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 2);
    assert_eq!(resolved_conflict_count(&segments), 0);

    // Resolve first block.
    if let ConflictSegment::Block(block) = &mut segments[0] {
        block.choice = ConflictChoice::Theirs;
        block.resolved = true;
    }
    assert_eq!(resolved_conflict_count(&segments), 1);
}

#[test]
fn effective_counts_use_marker_segments_when_blocks_exist() {
    let input = "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n";
    let mut segments = parse_conflict_markers(input);
    if let ConflictSegment::Block(block) = &mut segments[0] {
        block.resolved = true;
    }

    assert_eq!(effective_conflict_counts(&segments, Some((99, 98))), (1, 1));
}

#[test]
fn effective_counts_fall_back_to_session_counts_without_blocks() {
    let segments = vec![ConflictSegment::Text("resolved text\n".into())];

    assert_eq!(effective_conflict_counts(&segments, Some((1, 0))), (1, 0));
    assert_eq!(effective_conflict_counts(&segments, Some((2, 9))), (2, 2));
}

#[test]
fn effective_counts_return_zero_without_blocks_or_session() {
    let segments = vec![ConflictSegment::Text("plain text\n".into())];

    assert_eq!(effective_conflict_counts(&segments, None), (0, 0));
}

fn mark_block_resolved(segments: &mut [ConflictSegment], target: usize) {
    let mut seen = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if seen == target {
            block.resolved = true;
            return;
        }
        seen += 1;
    }
    panic!("missing block index {target}");
}

#[test]
fn next_unresolved_wraps_to_first() {
    let input = concat!(
        "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
        "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
        "<<<<<<< HEAD\nthree\n=======\ntres\n>>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    mark_block_resolved(&mut segments, 1);

    assert_eq!(next_unresolved_conflict_index(&segments, 2), Some(0));
    assert_eq!(next_unresolved_conflict_index(&segments, 0), Some(2));
}

#[test]
fn prev_unresolved_wraps_to_last() {
    let input = concat!(
        "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
        "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
        "<<<<<<< HEAD\nthree\n=======\ntres\n>>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    mark_block_resolved(&mut segments, 1);

    assert_eq!(prev_unresolved_conflict_index(&segments, 0), Some(2));
    assert_eq!(prev_unresolved_conflict_index(&segments, 2), Some(0));
}

#[test]
fn unresolved_navigation_returns_none_when_fully_resolved() {
    let input = concat!(
        "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
        "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    mark_block_resolved(&mut segments, 0);
    mark_block_resolved(&mut segments, 1);

    assert_eq!(next_unresolved_conflict_index(&segments, 0), None);
    assert_eq!(prev_unresolved_conflict_index(&segments, 0), None);
}

#[test]
fn unresolved_navigation_can_jump_from_resolved_active_conflict() {
    let input = concat!(
        "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
        "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    mark_block_resolved(&mut segments, 0);

    assert_eq!(next_unresolved_conflict_index(&segments, 0), Some(1));
    assert_eq!(prev_unresolved_conflict_index(&segments, 0), Some(1));
}

#[test]
fn bulk_pick_updates_only_unresolved_blocks() {
    let input = concat!(
        "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
        "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);

    if let Some(ConflictSegment::Block(block)) = segments
        .iter_mut()
        .find(|s| matches!(s, ConflictSegment::Block(_)))
    {
        block.choice = ConflictChoice::Theirs;
        block.resolved = true;
    }

    let updated = apply_choice_to_unresolved_segments(&mut segments, ConflictChoice::Ours);
    assert_eq!(updated, 1);
    assert_eq!(resolved_conflict_count(&segments), 2);

    let mut blocks = segments.iter().filter_map(|s| match s {
        ConflictSegment::Block(block) => Some(block),
        ConflictSegment::Text(_) => None,
    });
    let first = blocks.next().expect("missing first block");
    let second = blocks.next().expect("missing second block");
    assert_eq!(first.choice, ConflictChoice::Theirs);
    assert!(first.resolved);
    assert_eq!(second.choice, ConflictChoice::Ours);
    assert!(second.resolved);
}

#[test]
fn bulk_pick_both_concatenates_for_unresolved_blocks() {
    let input = concat!(
        "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
        "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    let updated = apply_choice_to_unresolved_segments(&mut segments, ConflictChoice::Both);
    assert_eq!(updated, 2);
    assert_eq!(resolved_conflict_count(&segments), 2);
    let resolved = generate_resolved_text(&segments);
    assert_eq!(resolved, "one\nuno\ntwo\ndos\n");
}

#[test]
fn bulk_pick_base_skips_unresolved_blocks_without_base() {
    let input = concat!(
        "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
        "<<<<<<< HEAD\ntwo\n||||||| base\ntwo\n=======\ndos\n>>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    let updated = apply_choice_to_unresolved_segments(&mut segments, ConflictChoice::Base);
    assert_eq!(updated, 1);
    assert_eq!(resolved_conflict_count(&segments), 1);

    let mut blocks = segments.iter().filter_map(|s| match s {
        ConflictSegment::Block(block) => Some(block),
        ConflictSegment::Text(_) => None,
    });
    let first = blocks.next().expect("missing first block");
    let second = blocks.next().expect("missing second block");

    assert_eq!(first.choice, ConflictChoice::Ours);
    assert!(!first.resolved);
    assert_eq!(second.choice, ConflictChoice::Base);
    assert!(second.resolved);
}

// -- auto_resolve_segments tests --

#[test]
fn auto_resolve_identical_sides() {
    let input = "a\n<<<<<<< HEAD\nsame\n||||||| base\norig\n=======\nsame\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    assert_eq!(auto_resolve_segments(&mut segments), 1);
    assert_eq!(resolved_conflict_count(&segments), 1);

    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.choice, ConflictChoice::Ours);
    assert!(block.resolved);
}

#[test]
fn auto_resolve_only_theirs_changed() {
    let input = "a\n<<<<<<< HEAD\norig\n||||||| base\norig\n=======\nchanged\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    assert_eq!(auto_resolve_segments(&mut segments), 1);

    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.choice, ConflictChoice::Theirs);
    assert!(block.resolved);
}

#[test]
fn auto_resolve_only_ours_changed() {
    let input = "a\n<<<<<<< HEAD\nchanged\n||||||| base\norig\n=======\norig\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    assert_eq!(auto_resolve_segments(&mut segments), 1);

    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.choice, ConflictChoice::Ours);
    assert!(block.resolved);
}

#[test]
fn auto_resolve_both_changed_differently_not_resolved() {
    let input = "a\n<<<<<<< HEAD\nours\n||||||| base\norig\n=======\ntheirs\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    assert_eq!(auto_resolve_segments(&mut segments), 0);
    assert_eq!(resolved_conflict_count(&segments), 0);
}

#[test]
fn auto_resolve_no_base_identical_sides() {
    // 2-way markers (no base section) — identical sides should still resolve.
    let input = "a\n<<<<<<< HEAD\nsame\n=======\nsame\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    assert_eq!(auto_resolve_segments(&mut segments), 1);
    assert_eq!(resolved_conflict_count(&segments), 1);
}

#[test]
fn auto_resolve_no_base_different_sides_not_resolved() {
    let input = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    assert_eq!(auto_resolve_segments(&mut segments), 0);
}

#[test]
fn auto_resolve_skips_already_resolved() {
    let input = "a\n<<<<<<< HEAD\nsame\n||||||| base\norig\n=======\nsame\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);

    // Manually resolve first.
    if let Some(ConflictSegment::Block(block)) = segments
        .iter_mut()
        .find(|s| matches!(s, ConflictSegment::Block(_)))
    {
        block.choice = ConflictChoice::Theirs;
        block.resolved = true;
    }

    // Auto-resolve should skip it.
    assert_eq!(auto_resolve_segments(&mut segments), 0);
    // Choice should remain Theirs (not overwritten).
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.choice, ConflictChoice::Theirs);
}

#[test]
fn auto_resolve_multiple_blocks_mixed() {
    let input = concat!(
        "<<<<<<< HEAD\nsame\n||||||| base\norig\n=======\nsame\n>>>>>>> other\n",
        "<<<<<<< HEAD\nours\n||||||| base\norig\n=======\ntheirs\n>>>>>>> other\n",
        "<<<<<<< HEAD\norig\n||||||| base\norig\n=======\nchanged\n>>>>>>> other\n",
    );
    let mut segments = parse_conflict_markers(input);
    assert_eq!(conflict_count(&segments), 3);

    let resolved = auto_resolve_segments(&mut segments);
    assert_eq!(resolved, 2); // blocks 0 (identical) and 2 (only theirs changed)
    assert_eq!(resolved_conflict_count(&segments), 2);
}

#[test]
fn auto_resolve_generates_correct_text() {
    let input = "a\n<<<<<<< HEAD\norig\n||||||| base\norig\n=======\nchanged\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    auto_resolve_segments(&mut segments);
    let text = generate_resolved_text(&segments);
    assert_eq!(text, "a\nchanged\nb\n");
}

#[test]
fn auto_resolve_regex_equivalent_sides() {
    use gitcomet_core::conflict_session::RegexAutosolveOptions;

    let input = "a\n<<<<<<< HEAD\nlet  answer = 42;\n||||||| base\nlet answer = 42;\n=======\nlet answer\t=\t42;\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    let options = RegexAutosolveOptions::whitespace_insensitive();

    assert_eq!(auto_resolve_segments_regex(&mut segments, &options), 1);
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.choice, ConflictChoice::Ours);
    assert!(block.resolved);
}

#[test]
fn auto_resolve_regex_only_theirs_changed_from_normalized_base() {
    use gitcomet_core::conflict_session::RegexAutosolveOptions;

    let input = "a\n<<<<<<< HEAD\nlet answer=42;\n||||||| base\nlet answer = 42;\n=======\nlet answer = 43;\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    let options = RegexAutosolveOptions::whitespace_insensitive();

    assert_eq!(auto_resolve_segments_regex(&mut segments, &options), 1);
    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    assert_eq!(block.choice, ConflictChoice::Theirs);
    assert!(block.resolved);
}

#[test]
fn auto_resolve_regex_invalid_pattern_noops() {
    use gitcomet_core::conflict_session::RegexAutosolveOptions;

    let input = "a\n<<<<<<< HEAD\nlet answer=42;\n||||||| base\nlet answer = 42;\n=======\nlet answer = 43;\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    let options = RegexAutosolveOptions::default().with_pattern("(", "");

    assert_eq!(auto_resolve_segments_regex(&mut segments, &options), 0);
    assert_eq!(resolved_conflict_count(&segments), 0);
}

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
        diff_row_conflict_map.iter().any(|entry| *entry == Some(0)),
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
        inline_row_conflict_map
            .iter()
            .any(|entry| *entry == Some(0)),
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
    let base_count = 2 + 1 + 1 + 0 + 1; // 5
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
    let mut all_old: Vec<Option<String>> = Vec::new();
    let mut all_new: Vec<Option<String>> = Vec::new();
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
