use super::*;

fn block_map_for_output(segments: &[ConflictSegment], output_text: &str) -> ResolvedOutputBlockMap {
    let original = generate_resolved_text(segments);
    let mut map = ResolvedOutputBlockMap::from_segments(segments);
    if original != output_text {
        let old = original.as_bytes();
        let new = output_text.as_bytes();
        let mut prefix = 0usize;
        while prefix < old.len().min(new.len()) && old[prefix] == new[prefix] {
            prefix += 1;
        }
        while prefix > 0
            && (!original.is_char_boundary(prefix) || !output_text.is_char_boundary(prefix))
        {
            prefix -= 1;
        }
        let mut suffix = 0usize;
        while suffix < old.len().saturating_sub(prefix)
            && suffix < new.len().saturating_sub(prefix)
            && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
        {
            suffix += 1;
        }
        while suffix > 0
            && (!original.is_char_boundary(old.len().saturating_sub(suffix))
                || !output_text.is_char_boundary(new.len().saturating_sub(suffix)))
        {
            suffix -= 1;
        }
        assert!(map.apply_edit_delta(
            prefix..old.len().saturating_sub(suffix),
            prefix..new.len().saturating_sub(suffix),
        ));
    }
    assert!(map.is_valid_for(segments, output_text));
    map
}

fn apply_output_edit(
    map: &mut ResolvedOutputBlockMap,
    output: &mut String,
    old_range: Range<usize>,
    replacement: &str,
) {
    let new_range = old_range.start..old_range.start.saturating_add(replacement.len());
    assert!(map.apply_edit_delta(old_range.clone(), new_range));
    output.replace_range(old_range, replacement);
}

#[test]
fn deleting_conflict_output_to_empty_is_an_empty_manual_resolution() {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let segments =
        parse_conflict_markers("pre\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\npost\n");
    let mut output = generate_resolved_text(&segments);
    let mut block_map = ResolvedOutputBlockMap::from_segments(&segments);
    let owned = block_map.ranges()[0].clone();
    apply_output_edit(&mut block_map, &mut output, owned, "");

    let updates =
        derive_region_resolution_updates_from_output(&segments, &[0], &block_map, &output)
            .expect("mapped empty edit");
    assert_eq!(updates[0].1, R::ManualEdit(String::new()));
}

#[test]
fn deleting_placeholder_to_empty_does_not_implicitly_choose_an_empty_source() {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "".into(),
        theirs: "theirs\n".into(),
        choice: ConflictChoice::empty(),
        resolved: false,
        whitespace_only: false,
    })];
    let mut output = generate_resolved_text(&segments);
    let mut block_map = ResolvedOutputBlockMap::from_segments(&segments);
    let owned = block_map.ranges()[0].clone();
    apply_output_edit(&mut block_map, &mut output, owned, "");

    let updates =
        derive_region_resolution_updates_from_output(&segments, &[0], &block_map, &output)
            .expect("mapped empty edit");
    assert_eq!(updates[0].1, R::ManualEdit(String::new()));
}

#[test]
fn selected_empty_source_keeps_its_explicit_source_resolution() {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;
    use gitcomet_core::merge::MergeSource;

    let segments = vec![
        ConflictSegment::Text("pre\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::Ours,
            resolved: true,
            whitespace_only: false,
        }),
        ConflictSegment::Text("post\n".into()),
    ];
    let output = generate_resolved_text(&segments);
    let block_map = ResolvedOutputBlockMap::from_segments(&segments);
    assert!(block_map.ranges()[0].is_empty());

    let updates =
        derive_region_resolution_updates_from_output(&segments, &[0], &block_map, &output)
            .expect("empty source selection");
    assert_eq!(updates[0].1, R::Sources(MergeSource::A.into()));
}

#[test]
fn context_edits_before_between_and_after_conflicts_only_move_boundaries() {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let segments = parse_conflict_markers(concat!(
        "top\n",
        "<<<<<<< ours\nours-1\n=======\ntheirs-1\n>>>>>>> theirs\n",
        "middle\n",
        "<<<<<<< ours\nours-2\n=======\ntheirs-2\n>>>>>>> theirs\n",
        "bottom\n",
    ));
    let mut output = generate_resolved_text(&segments);
    let mut block_map = ResolvedOutputBlockMap::from_segments(&segments);

    apply_output_edit(&mut block_map, &mut output, 0..0, "before\n");
    let middle = output.find("middle\n").expect("middle context") + "middle".len();
    apply_output_edit(&mut block_map, &mut output, middle..middle, "-edited");
    let end = output.len();
    apply_output_edit(&mut block_map, &mut output, end..end, "after\n");

    let updates =
        derive_region_resolution_updates_from_output(&segments, &[0, 1], &block_map, &output)
            .expect("context edits preserve ownership");
    assert_eq!(updates[0].1, R::Unresolved);
    assert_eq!(updates[1].1, R::Unresolved);
}

#[test]
fn untouched_placeholder_stays_unresolved_while_another_block_is_edited() {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let segments = parse_conflict_markers(concat!(
        "top\n",
        "<<<<<<< ours\nours-1\n=======\ntheirs-1\n>>>>>>> theirs\n",
        "middle\n",
        "<<<<<<< ours\nours-2\n=======\ntheirs-2\n>>>>>>> theirs\n",
        "bottom\n",
    ));
    let mut output = generate_resolved_text(&segments);
    let mut block_map = ResolvedOutputBlockMap::from_segments(&segments);
    let second = block_map.ranges()[1].clone();
    apply_output_edit(&mut block_map, &mut output, second, "manual second\n");
    let middle = output.find("middle\n").expect("middle context");
    apply_output_edit(&mut block_map, &mut output, middle..middle, "context\n");

    let updates =
        derive_region_resolution_updates_from_output(&segments, &[0, 1], &block_map, &output)
            .expect("mixed edits");
    assert_eq!(updates[0].1, R::Unresolved);
    assert_eq!(updates[1].1, R::ManualEdit("manual second\n".into()));
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
    let block_map = ResolvedOutputBlockMap::from_segments(&segments);
    let updates = derive_region_resolution_updates_from_output(
        &segments,
        &sequential_conflict_region_indices(&segments),
        &block_map,
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
    use gitcomet_core::merge::MergeSource;

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
    let block_map = block_map_for_output(&segments, output);
    let updates = derive_region_resolution_updates_from_output(
        &segments,
        &sequential_conflict_region_indices(&segments),
        &block_map,
        output,
    )
    .expect("updates");

    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].0, 0);
    assert_eq!(updates[0].1, R::ManualEdit("manual one\n".into()));
    assert_eq!(updates[1].0, 1);
    assert_eq!(updates[1].1, R::Sources(MergeSource::B.into()));
}

#[test]
fn derive_region_resolution_updates_preserves_ownership_when_context_changed() {
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
    let output = "changed-pre\n<Merge Conflict>\npost\n";
    let block_map = block_map_for_output(&segments, output);
    let updates = derive_region_resolution_updates_from_output(
        &segments,
        &sequential_conflict_region_indices(&segments),
        &block_map,
        output,
    );
    assert_eq!(
        updates.expect("mapped context edit")[0].1,
        gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved,
    );
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
fn populate_block_bases_from_shared_ancestor_reuses_ancestor_storage() {
    let input = "a\n<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\nb\n";
    let mut segments = parse_conflict_markers(input);
    let ancestor = Arc::<str>::from("a\norig\nb\n");
    let ancestor_ptr = ancestor.as_ptr() as usize;
    let ancestor_end = ancestor_ptr + ancestor.len();

    populate_block_bases_from_shared_ancestor(&mut segments, Arc::clone(&ancestor));

    let block = segments
        .iter()
        .find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        })
        .unwrap();
    let base = block.base.as_deref().unwrap();
    let base_ptr = base.as_ptr() as usize;
    assert_eq!(base, "orig\n");
    assert!(base_ptr >= ancestor_ptr && base_ptr < ancestor_end);
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
        block.resolved = true;
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
fn apply_session_unresolved_clears_stale_source_selection() {
    use gitcomet_core::conflict_session::{ConflictRegion, ConflictRegionResolution as R};

    let input = "pre\n<<<<<<< ours\nlocal\n=======\nremote\n>>>>>>> theirs\npost\n";
    let mut segments = parse_conflict_markers(input);
    if let Some(ConflictSegment::Block(block)) = segments
        .iter_mut()
        .find(|segment| matches!(segment, ConflictSegment::Block(_)))
    {
        block.choice = ConflictChoice::Theirs;
        block.resolved = true;
    }
    let regions = [ConflictRegion {
        base: None,
        ours: "local\n".into(),
        theirs: "remote\n".into(),
        resolution: R::Unresolved,
    }];

    assert_eq!(apply_session_region_resolutions(&mut segments, &regions), 1);
    let block = segments
        .iter()
        .find_map(|segment| match segment {
            ConflictSegment::Block(block) => Some(block),
            ConflictSegment::Text(_) => None,
        })
        .expect("conflict block");
    assert!(block.choice.is_empty());
    assert!(!block.resolved);
    assert_eq!(
        generate_resolved_text(&segments),
        "pre\n<Merge Conflict>\npost\n"
    );
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
    assert!(text_contains_conflict_markers(
        "<<<<<<< HEAD\nours\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> branch\n"
    ));
}

#[test]
fn no_false_positives_for_clean_text() {
    assert!(!text_contains_conflict_markers("a\nb\nc\n"));
    assert!(!text_contains_conflict_markers(""));
    assert!(!text_contains_conflict_markers(
        "some text with < and > arrows"
    ));
    assert!(!text_contains_conflict_markers("====== not quite seven"));
    assert!(!text_contains_conflict_markers("<<<<<<< HEAD\n"));
    assert!(!text_contains_conflict_markers("=======\n"));
    assert!(!text_contains_conflict_markers(">>>>>>> branch\n"));
    assert!(!text_contains_conflict_markers("||||||| base\n"));
    assert!(!text_contains_conflict_markers(
        "Markdown heading\n=======\nbody\n"
    ));
}

#[test]
fn stage_safety_blocks_unresolved_blocks_without_markers() {
    let input = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nb\n";
    let segments = parse_conflict_markers(input);
    let output_text = generate_resolved_text(&segments);
    let block_map = ResolvedOutputBlockMap::from_segments(&segments);

    let safety = conflict_stage_safety_check(&output_text, &segments, &block_map);
    assert!(!safety.has_conflict_markers);
    assert_eq!(safety.unresolved_blocks, 1);
    assert!(safety.blocks_save());
}

#[test]
fn stage_safety_allows_fully_resolved_clean_output() {
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
    let block_map = ResolvedOutputBlockMap::from_segments(&segments);

    let safety = conflict_stage_safety_check(&output_text, &segments, &block_map);
    assert!(!safety.has_conflict_markers);
    assert_eq!(safety.unresolved_blocks, 0);
    assert!(!safety.blocks_save());
}

#[test]
fn stage_safety_allows_incomplete_marker_like_content() {
    let safety = conflict_stage_safety_check(
        "<<<<<<< HEAD\nours\n",
        &[],
        &ResolvedOutputBlockMap::default(),
    );
    assert!(!safety.has_conflict_markers);
    assert_eq!(safety.unresolved_blocks, 0);
    assert!(!safety.blocks_save());
}

#[test]
fn stage_safety_recognizes_manual_replacement_before_session_sync() {
    let input = "pre\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\npost\n";
    let segments = parse_conflict_markers(input);
    let output = "pre\nmanual resolution\npost\n";
    let block_map = block_map_for_output(&segments, output);
    let safety = conflict_stage_safety_check(output, &segments, &block_map);
    assert!(!safety.has_conflict_markers);
    assert_eq!(safety.unresolved_blocks, 0);
    assert!(!safety.blocks_save());
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
fn autosolve_trace_summary_on_open_mode() {
    let stats = gitcomet_state::msg::ConflictAutosolveStats {
        pass1: 2,
        pass2_split: 1,
        pass1_after_split: 0,
        regex: 1,
        history: 0,
    };
    let summary = format_autosolve_trace_summary(AutosolveTraceMode::OnOpen, 6, 2, &stats);
    assert!(summary.contains("Auto-solved on open"));
    assert!(summary.contains("resolved 4 blocks"));
    assert!(summary.contains("unresolved 6 -> 2"));
    assert!(summary.contains("regex 1"));
}

#[test]
fn open_summary_toast_reports_kdiff3_total_auto_and_unsolved() {
    let counts = |total, auto_solved, unsolved| ConflictSummaryCounts {
        total,
        auto_solved,
        unsolved,
        whitespace_conflicts: None,
    };
    assert_eq!(
        format_open_summary_toast(counts(21, 19, 2)).as_deref(),
        Some("Total 21 / auto-solved 19 / unsolved 2")
    );
    assert_eq!(
        format_open_summary_toast(counts(5, 0, 5)).as_deref(),
        Some("Total 5 / auto-solved 0 / unsolved 5")
    );
    assert_eq!(
        format_open_summary_toast(counts(1, 1, 0)).as_deref(),
        Some("Total 1 / auto-solved 1 / unsolved 0")
    );
    // Counts clamp to a valid partition and never underflow.
    assert_eq!(
        format_open_summary_toast(counts(3, 9, 9)).as_deref(),
        Some("Total 3 / auto-solved 0 / unsolved 3")
    );
    assert!(format_open_summary_toast(counts(0, 0, 0)).is_none());
}

#[test]
fn on_open_autosolve_summary_reconstructs_tier_breakdown_from_rules() {
    use gitcomet_core::conflict_session::{
        AutosolveConfidence, AutosolveRule, ConflictPayload, ConflictRegion,
        ConflictRegionResolution as R, ConflictSession,
    };
    use gitcomet_core::domain::FileConflictKind;

    let region = |resolution: R| ConflictRegion {
        base: None,
        ours: "ours\n".into(),
        theirs: "theirs\n".into(),
        resolution,
    };
    let auto = |rule: AutosolveRule| {
        region(R::AutoResolved {
            rule,
            confidence: rule.confidence(),
            content: String::new(),
        })
    };
    let _ = AutosolveConfidence::High;

    let mut session = ConflictSession::new(
        std::path::PathBuf::from("file.txt"),
        FileConflictKind::BothModified,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
    );
    session.regions = vec![
        auto(AutosolveRule::IdenticalSides),
        auto(AutosolveRule::SubchunkFullyMerged),
        auto(AutosolveRule::RegexEquivalentSides),
        region(R::Unresolved),
    ];

    let summary = on_open_autosolve_summary(&session).expect("summary for auto-resolved regions");
    assert!(summary.contains("Auto-solved on open"));
    assert!(summary.contains("resolved 3 blocks"));
    assert!(summary.contains("unresolved 4 -> 1"));
    assert!(summary.contains("pass1 1"));
    assert!(summary.contains("split 1"));
    assert!(summary.contains("regex 1"));
}

#[test]
fn on_open_autosolve_summary_is_none_without_auto_resolutions() {
    use gitcomet_core::conflict_session::{
        ConflictPayload, ConflictRegion, ConflictRegionResolution as R, ConflictSession,
    };
    use gitcomet_core::domain::FileConflictKind;

    let mut session = ConflictSession::new(
        std::path::PathBuf::from("file.txt"),
        FileConflictKind::BothModified,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
    );
    session.regions = vec![
        ConflictRegion {
            base: None,
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            resolution: R::PickOurs,
        },
        ConflictRegion {
            base: None,
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            resolution: R::Unresolved,
        },
    ];

    assert!(on_open_autosolve_summary(&session).is_none());
}

#[test]
fn conflict_session_summary_counts_include_materialized_auto_resolutions() {
    use gitcomet_core::conflict_session::{
        AutosolveRule, ConflictPayload, ConflictRegion, ConflictRegionResolution as R,
        ConflictSession,
    };
    use gitcomet_core::domain::FileConflictKind;

    let region = |resolution: R| ConflictRegion {
        base: None,
        ours: "ours\n".into(),
        theirs: "theirs\n".into(),
        resolution,
    };
    let mut session = ConflictSession::new(
        std::path::PathBuf::from("file.txt"),
        FileConflictKind::BothModified,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
    );
    session.regions = vec![
        region(R::AutoResolved {
            rule: AutosolveRule::IdenticalSides,
            confidence: AutosolveRule::IdenticalSides.confidence(),
            content: "custom merged output\n".to_string(),
        }),
        region(R::PickOurs),
        region(R::AutoResolved {
            rule: AutosolveRule::RegexEquivalentSides,
            confidence: AutosolveRule::RegexEquivalentSides.confidence(),
            content: String::new(),
        }),
        region(R::Unresolved),
    ];

    assert_eq!(
        conflict_session_summary_counts(&session),
        ConflictSummaryCounts {
            total: 4,
            auto_solved: 2,
            unsolved: 1,
            whitespace_conflicts: None,
        }
    );
}

#[test]
fn plan_summary_counts_deltas_conflicts_and_whitespace_like_kdiff3() {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
    use gitcomet_core::domain::FileConflictKind;
    use gitcomet_core::merge::MergeSource;

    let base = "start\nbase-local\nanchor-1\nbase-conflict\nanchor-2\nfoo(1);\nend\n";
    let ours = "start\nours-local\nanchor-1\nours-conflict\nanchor-2\nfoo( 1 );\nend\n";
    let theirs = "start\nbase-local\nanchor-1\ntheirs-conflict\nanchor-2\nfoo(1) ;\nend\n";
    let mut session = ConflictSession::from_stage_inputs(
        std::path::PathBuf::from("file.txt"),
        FileConflictKind::BothModified,
        ConflictPayload::Text(base.into()),
        ConflictPayload::Text(ours.into()),
        ConflictPayload::Text(theirs.into()),
    );

    assert_eq!(
        conflict_session_summary_counts(&session),
        ConflictSummaryCounts {
            total: 3,
            auto_solved: 1,
            unsolved: 2,
            whitespace_conflicts: Some(1),
        }
    );

    let plan = session
        .merge_plan
        .as_mut()
        .expect("stage-backed merge plan");
    let conflict = plan
        .blocks
        .iter_mut()
        .find(|block| block.original_conflict && !block.whitespace_conflict)
        .expect("non-whitespace conflict");
    conflict.replace_selection(MergeSource::B.into());
    plan.refresh_unresolved_blocks();

    assert_eq!(
        conflict_session_summary_counts(&session),
        ConflictSummaryCounts {
            total: 3,
            auto_solved: 2,
            unsolved: 1,
            whitespace_conflicts: Some(1),
        },
        "resolving a block changes the solved/unsolved partition, not the denominator",
    );
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
        conflict_quick_pick_choice_for_key("a", ConflictResolverViewMode::ThreeWay),
        Some(ConflictChoice::Base)
    );
    assert_eq!(
        conflict_quick_pick_choice_for_key("b", ConflictResolverViewMode::ThreeWay),
        Some(ConflictChoice::Ours)
    );
    assert_eq!(
        conflict_quick_pick_choice_for_key("c", ConflictResolverViewMode::ThreeWay),
        Some(ConflictChoice::Theirs)
    );
    assert_eq!(
        conflict_quick_pick_choice_for_key("d", ConflictResolverViewMode::ThreeWay),
        Some(ConflictChoice::Both)
    );
    assert_eq!(
        conflict_quick_pick_choice_for_key("x", ConflictResolverViewMode::ThreeWay),
        None
    );
}

#[test]
fn two_way_quick_pick_key_mapping_uses_a_b_c_without_base() {
    assert_eq!(
        conflict_quick_pick_choice_for_key("a", ConflictResolverViewMode::TwoWayDiff),
        Some(ConflictChoice::Ours)
    );
    assert_eq!(
        conflict_quick_pick_choice_for_key("b", ConflictResolverViewMode::TwoWayDiff),
        Some(ConflictChoice::Theirs)
    );
    assert_eq!(
        conflict_quick_pick_choice_for_key("c", ConflictResolverViewMode::TwoWayDiff),
        Some(ConflictChoice::Both)
    );
    assert_eq!(
        conflict_quick_pick_choice_for_key("d", ConflictResolverViewMode::TwoWayDiff),
        None
    );
}

#[test]
fn ctrl_pick_key_mapping_matches_kdiff3_1_2_3_aliases() {
    assert_eq!(
        conflict_ctrl_pick_choice_for_key("1", ConflictResolverViewMode::ThreeWay),
        Some(ConflictChoice::Base)
    );
    assert_eq!(
        conflict_ctrl_pick_choice_for_key("2", ConflictResolverViewMode::ThreeWay),
        Some(ConflictChoice::Ours)
    );
    assert_eq!(
        conflict_ctrl_pick_choice_for_key("3", ConflictResolverViewMode::ThreeWay),
        Some(ConflictChoice::Theirs)
    );
    assert_eq!(
        conflict_ctrl_pick_choice_for_key("4", ConflictResolverViewMode::ThreeWay),
        None
    );
    assert_eq!(
        conflict_ctrl_pick_choice_for_key("a", ConflictResolverViewMode::ThreeWay),
        None
    );
}

#[test]
fn two_way_ctrl_pick_key_mapping_uses_1_2_3_without_base() {
    assert_eq!(
        conflict_ctrl_pick_choice_for_key("1", ConflictResolverViewMode::TwoWayDiff),
        Some(ConflictChoice::Ours)
    );
    assert_eq!(
        conflict_ctrl_pick_choice_for_key("2", ConflictResolverViewMode::TwoWayDiff),
        Some(ConflictChoice::Theirs)
    );
    assert_eq!(
        conflict_ctrl_pick_choice_for_key("3", ConflictResolverViewMode::TwoWayDiff),
        Some(ConflictChoice::Both)
    );
    assert_eq!(
        conflict_ctrl_pick_choice_for_key("4", ConflictResolverViewMode::TwoWayDiff),
        None
    );
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

    assert!(first.choice.is_empty());
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
