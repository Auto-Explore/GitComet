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
        ConflictPayload::Text(String::new()),
        ConflictPayload::Text(String::new()),
        ConflictPayload::Text(String::new()),
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
        ConflictPayload::Text(String::new()),
        ConflictPayload::Text(String::new()),
        ConflictPayload::Text(String::new()),
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

    assert_eq!(maps.conflict_ranges, vec![1..3, 4..5]);
    assert_eq!(
        maps.base_line_conflict_map,
        vec![None, Some(0), None, Some(1), Some(1), None]
    );
    assert_eq!(
        maps.ours_line_conflict_map,
        vec![None, Some(0), Some(0), None, Some(1), None]
    );
    assert_eq!(
        maps.theirs_line_conflict_map,
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

    assert_eq!(maps.conflict_ranges, vec![1..1]);
    assert_eq!(maps.base_line_conflict_map, vec![None, None, None]);
    assert_eq!(maps.ours_line_conflict_map, vec![None, None]);
    assert_eq!(
        maps.theirs_line_conflict_map,
        vec![None, Some(0), Some(0), None]
    );
    assert_eq!(maps.conflict_has_base, vec![false]);
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
    assert!(map
        .iter()
        .all(|item| matches!(item, ThreeWayVisibleItem::Line(_))));

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

    assert_eq!(
        unresolved_visible_nav_entries_for_three_way(&segments, &visible_map, &ranges),
        vec![1, 5]
    );
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
    fn shared_lines(text: &str) -> Vec<gpui::SharedString> {
        text.lines().map(|line| line.to_string().into()).collect()
    }

    let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "alpha\nbeta changed\ngamma\n".into(),
        theirs: "alpha\ninserted\nbeta remote\ngamma\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];
    let base_lines = Vec::new();
    let ours_lines = shared_lines("alpha\nbeta changed\ngamma\n");
    let theirs_lines = shared_lines("alpha\ninserted\nbeta remote\ngamma\n");

    let (_base_hl, ours_hl, theirs_hl) = compute_three_way_word_highlights(
        &base_lines,
        &ours_lines,
        &theirs_lines,
        &marker_segments,
    );

    assert!(
        ours_hl[1].is_some(),
        "local modified line should be highlighted even when remote line is shifted"
    );
    assert!(
        ours_hl[0].is_none(),
        "unchanged local line should not be highlighted"
    );
    assert!(
        ours_hl[2].is_none(),
        "unchanged local line should not be highlighted"
    );

    assert!(
        theirs_hl[1].is_some(),
        "remote added line should be highlighted"
    );
    assert!(
        theirs_hl[2].is_some(),
        "remote modified line should be highlighted at its aligned row"
    );
    assert!(
        theirs_hl[3].is_none(),
        "unchanged remote line should not be highlighted"
    );
}

#[test]
fn three_way_word_highlights_keep_global_offsets_per_column() {
    fn shared_lines(text: &str) -> Vec<gpui::SharedString> {
        text.lines().map(|line| line.to_string().into()).collect()
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
    let base_lines = Vec::new();
    let ours_lines = shared_lines("ctx\nsame\ntail\n");
    let theirs_lines = shared_lines("ctx\nadded\nsame\ntail\n");

    let (_base_hl, ours_hl, theirs_hl) = compute_three_way_word_highlights(
        &base_lines,
        &ours_lines,
        &theirs_lines,
        &marker_segments,
    );

    assert!(
        ours_hl[1].is_none(),
        "local unchanged block line should stay unhighlighted"
    );
    assert!(
        theirs_hl[1].is_some(),
        "remote inserted block line should map to its own global row"
    );
    assert!(
        theirs_hl[2].is_none(),
        "remote aligned context line should not be highlighted"
    );
}
