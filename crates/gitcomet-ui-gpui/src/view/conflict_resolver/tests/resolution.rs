use super::*;

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
