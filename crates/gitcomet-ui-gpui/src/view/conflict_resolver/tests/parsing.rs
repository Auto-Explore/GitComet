use super::*;

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
fn shared_nonempty_marker_parse_skips_clean_text() {
    let input: std::sync::Arc<str> = std::sync::Arc::from("pre\nplain html\npost\n");
    let segments = parse_conflict_markers_shared_nonempty(input);
    assert!(segments.is_empty());
}

#[test]
fn bootstrap_resolved_output_text_reuses_clean_current_arc() {
    let current: std::sync::Arc<str> = std::sync::Arc::from("pre\nplain html\npost\n");
    let output = bootstrap_resolved_output_text(&[], Some(&current), None, None);

    let ResolvedOutputText::Shared(shared) = &output else {
        panic!("clean bootstrap should reuse the shared current text");
    };

    assert!(std::sync::Arc::ptr_eq(&current, shared));
    assert_eq!(output.line_count(), 3);
}

#[test]
fn bootstrap_resolved_output_text_materializes_conflicted_output() {
    let segments =
        parse_conflict_markers("a\n<<<<<<< ours\none\n=======\nuno\n>>>>>>> theirs\nb\n");
    let output = bootstrap_resolved_output_text(&segments, None, None, None);

    assert!(matches!(output, ResolvedOutputText::Owned(_)));
    assert_eq!(output.as_str(), "a\none\nb\n");
    assert_eq!(output.line_count(), 3);
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
    let split_lines: rustc_hash::FxHashSet<_> = rows.iter().filter_map(|r| r.new_line).collect();
    let inline_lines: rustc_hash::FxHashSet<_> = inline.iter().filter_map(|r| r.new_line).collect();
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
    assert_eq!(resolved_output_outline_line_count("a\nb\n"), lines.len());
}

#[test]
fn split_output_lines_for_outline_keeps_single_empty_row_for_empty_text() {
    let lines = split_output_lines_for_outline("");
    assert_eq!(lines, vec![""]);
    assert_eq!(resolved_output_outline_line_count(""), lines.len());
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
fn resolved_output_gutter_row_packs_marker_bits() {
    let manual = ResolvedOutputGutterRow::default();
    assert_eq!(manual.source(), ResolvedLineSource::Manual);
    assert_eq!(manual.badge_char(), 'M');
    assert!(manual.manual_without_marker());
    assert_eq!(manual.marker_conflict_ix(), None);

    let marker = ResolvedOutputGutterRow::new(ResolvedLineSource::B, Some(17), true, false, true);
    assert_eq!(marker.source(), ResolvedLineSource::B);
    assert_eq!(marker.badge_char(), 'B');
    assert_eq!(marker.marker_conflict_ix(), Some(17));
    assert!(marker.has_marker());
    assert!(marker.is_start());
    assert!(!marker.is_end());
    assert!(marker.unresolved());
    assert!(!marker.manual_without_marker());

    let source_only = ResolvedOutputGutterRow::new(ResolvedLineSource::C, None, true, true, true);
    assert_eq!(source_only.source(), ResolvedLineSource::C);
    assert_eq!(source_only.badge_char(), 'C');
    assert_eq!(source_only.marker_conflict_ix(), None);
    assert!(!source_only.has_marker());
    assert!(!source_only.is_start());
    assert!(!source_only.is_end());
    assert!(!source_only.unresolved());
}
