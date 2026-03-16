use super::*;

fn make_region(base: Option<&str>, ours: &str, theirs: &str) -> ConflictRegion {
    ConflictRegion {
        base: base.map(ConflictRegionText::from),
        ours: ours.into(),
        theirs: theirs.into(),
        resolution: ConflictRegionResolution::Unresolved,
    }
}

fn make_session(regions: Vec<ConflictRegion>) -> ConflictSession {
    ConflictSession {
        path: PathBuf::from("test.txt"),
        conflict_kind: FileConflictKind::BothModified,
        strategy: ConflictResolverStrategy::FullTextResolver,
        base: ConflictPayload::Text("base\n".into()),
        ours: ConflictPayload::Text("ours\n".into()),
        theirs: ConflictPayload::Text("theirs\n".into()),
        current: None,
        regions,
    }
}

// -- ConflictPayload tests --

#[test]
fn payload_from_bytes_utf8() {
    let p = ConflictPayload::from_bytes(b"hello".to_vec());
    assert_eq!(p.as_text(), Some("hello"));
    assert_eq!(p.as_bytes(), Some("hello".as_bytes()));
    assert_eq!(p.byte_len(), Some(5));
    assert!(!p.is_binary());
    assert!(!p.is_absent());
}

#[test]
fn payload_from_bytes_binary() {
    let bytes = vec![0xFF, 0xFE, 0x00];
    let p = ConflictPayload::from_bytes(bytes.clone());
    assert!(p.is_binary());
    assert!(p.as_text().is_none());
    assert_eq!(p.as_bytes(), Some(bytes.as_slice()));
    assert_eq!(p.byte_len(), Some(bytes.len()));
}

#[test]
fn payload_absent() {
    let p = ConflictPayload::Absent;
    assert!(p.is_absent());
    assert!(p.as_text().is_none());
    assert!(p.as_bytes().is_none());
    assert_eq!(p.byte_len(), None);
    assert!(!p.is_binary());
}

#[test]
fn stage_parts_round_trip_text() {
    let text: Arc<str> = Arc::from("hello");
    let p = ConflictPayload::from_stage_parts(None, Some(text.clone()));
    assert_eq!(p.as_text(), Some("hello"));
    let (bytes, text_out) = p.into_stage_parts();
    assert!(bytes.is_none());
    assert_eq!(text_out.as_deref(), Some("hello"));
}

#[test]
fn stage_parts_round_trip_binary() {
    let bytes: Arc<[u8]> = Arc::from(vec![0xFF, 0xFE]);
    let p = ConflictPayload::from_stage_parts(Some(bytes.clone()), None);
    assert!(p.is_binary());
    let (bytes_out, text_out) = p.into_stage_parts();
    assert_eq!(bytes_out.as_deref(), Some([0xFF, 0xFE].as_slice()));
    assert!(text_out.is_none());
}

#[test]
fn stage_parts_round_trip_absent() {
    let p = ConflictPayload::from_stage_parts(None, None);
    assert!(p.is_absent());
    let (bytes, text) = p.into_stage_parts();
    assert!(bytes.is_none());
    assert!(text.is_none());
}

#[test]
fn stage_parts_text_preferred_over_bytes() {
    let text: Arc<str> = Arc::from("hi");
    let bytes: Arc<[u8]> = Arc::from(b"hi".to_vec());
    let p = ConflictPayload::from_stage_parts(Some(bytes), Some(text));
    assert_eq!(p.as_text(), Some("hi"));
    assert!(!p.is_binary());
}

// -- ConflictRegionResolution tests --

#[test]
fn unresolved_is_not_resolved() {
    assert!(!ConflictRegionResolution::Unresolved.is_resolved());
}

#[test]
fn all_pick_variants_are_resolved() {
    assert!(ConflictRegionResolution::PickBase.is_resolved());
    assert!(ConflictRegionResolution::PickOurs.is_resolved());
    assert!(ConflictRegionResolution::PickTheirs.is_resolved());
    assert!(ConflictRegionResolution::PickBoth.is_resolved());
    assert!(ConflictRegionResolution::ManualEdit("x".into()).is_resolved());
    assert!(
        ConflictRegionResolution::AutoResolved {
            rule: AutosolveRule::IdenticalSides,
            confidence: AutosolveRule::IdenticalSides.confidence(),
            content: "x".into(),
        }
        .is_resolved()
    );
}

// -- ConflictRegion tests --

#[test]
fn resolved_text_for_picks() {
    let mut r = make_region(Some("base\n"), "ours\n", "theirs\n");

    r.resolution = ConflictRegionResolution::PickBase;
    assert_eq!(r.resolved_text(), Some("base\n"));

    r.resolution = ConflictRegionResolution::PickOurs;
    assert_eq!(r.resolved_text(), Some("ours\n"));

    r.resolution = ConflictRegionResolution::PickTheirs;
    assert_eq!(r.resolved_text(), Some("theirs\n"));

    r.resolution = ConflictRegionResolution::ManualEdit("custom\n".into());
    assert_eq!(r.resolved_text(), Some("custom\n"));
}

#[test]
fn resolved_text_both_concatenates() {
    let r = make_region(Some("base\n"), "ours\n", "theirs\n");
    assert_eq!(r.resolved_text_both(), "ours\ntheirs\n");
}

#[test]
fn resolved_text_unresolved_returns_none() {
    let r = make_region(Some("base\n"), "ours\n", "theirs\n");
    assert!(r.resolved_text().is_none());
}

// -- ConflictResolverStrategy tests --

#[test]
fn strategy_for_both_modified() {
    assert_eq!(
        ConflictResolverStrategy::for_conflict(FileConflictKind::BothModified, false),
        ConflictResolverStrategy::FullTextResolver,
    );
}

#[test]
fn strategy_for_both_added() {
    assert_eq!(
        ConflictResolverStrategy::for_conflict(FileConflictKind::BothAdded, false),
        ConflictResolverStrategy::FullTextResolver,
    );
}

#[test]
fn strategy_for_deleted_by_us() {
    assert_eq!(
        ConflictResolverStrategy::for_conflict(FileConflictKind::DeletedByUs, false),
        ConflictResolverStrategy::TwoWayKeepDelete,
    );
}

#[test]
fn strategy_for_deleted_by_them() {
    assert_eq!(
        ConflictResolverStrategy::for_conflict(FileConflictKind::DeletedByThem, false),
        ConflictResolverStrategy::TwoWayKeepDelete,
    );
}

#[test]
fn strategy_for_added_by_us() {
    assert_eq!(
        ConflictResolverStrategy::for_conflict(FileConflictKind::AddedByUs, false),
        ConflictResolverStrategy::TwoWayKeepDelete,
    );
}

#[test]
fn strategy_for_added_by_them() {
    assert_eq!(
        ConflictResolverStrategy::for_conflict(FileConflictKind::AddedByThem, false),
        ConflictResolverStrategy::TwoWayKeepDelete,
    );
}

#[test]
fn strategy_for_both_deleted() {
    assert_eq!(
        ConflictResolverStrategy::for_conflict(FileConflictKind::BothDeleted, false),
        ConflictResolverStrategy::DecisionOnly,
    );
}

#[test]
fn strategy_binary_overrides_kind() {
    assert_eq!(
        ConflictResolverStrategy::for_conflict(FileConflictKind::BothModified, true),
        ConflictResolverStrategy::BinarySidePick,
    );
    assert_eq!(
        ConflictResolverStrategy::for_conflict(FileConflictKind::DeletedByUs, true),
        ConflictResolverStrategy::BinarySidePick,
    );
}

#[test]
fn strategy_for_both_deleted_stays_decision_only_when_binary() {
    assert_eq!(
        ConflictResolverStrategy::for_conflict(FileConflictKind::BothDeleted, true),
        ConflictResolverStrategy::DecisionOnly,
    );
}

// -- Marker parsing tests --

fn slice_range<'a>(text: &'a str, range: &std::ops::Range<usize>) -> &'a str {
    text.get(range.clone())
        .expect("parser produced invalid byte range")
}

#[test]
fn parse_regions_two_way_markers() {
    let merged = "before\n<<<<<<< ours\nlocal 1\n=======\nremote 1\n>>>>>>> theirs\nafter\n";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].base, None);
    assert_eq!(regions[0].ours, "local 1\n");
    assert_eq!(regions[0].theirs, "remote 1\n");
    assert_eq!(regions[0].resolution, ConflictRegionResolution::Unresolved);
}

#[test]
fn parse_regions_diff3_markers() {
    let merged = "\
<<<<<<< ours
local line
||||||| base
base line
=======
remote line
>>>>>>> theirs
";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].base.as_deref(), Some("base line\n"));
    assert_eq!(regions[0].ours, "local line\n");
    assert_eq!(regions[0].theirs, "remote line\n");
}

#[test]
fn parse_regions_stops_on_malformed_block() {
    let merged = "\
<<<<<<< ours
local ok
=======
remote ok
>>>>>>> theirs
middle
<<<<<<< ours
unterminated
";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].ours, "local ok\n");
    assert_eq!(regions[0].theirs, "remote ok\n");
}

#[test]
fn session_from_merged_text_populates_regions_and_navigation() {
    let merged = "\
start
<<<<<<< ours
local one
=======
remote one
>>>>>>> theirs
mid
<<<<<<< ours
local two
=======
remote two
>>>>>>> theirs
end
";
    let mut session = ConflictSession::from_merged_text(
        PathBuf::from("file.txt"),
        FileConflictKind::BothModified,
        ConflictPayload::Text("base\n".into()),
        ConflictPayload::Text("ours\n".into()),
        ConflictPayload::Text("theirs\n".into()),
        merged,
    );

    assert_eq!(session.total_regions(), 2);
    assert_eq!(session.solved_count(), 0);
    assert_eq!(session.unsolved_count(), 2);
    assert!(!session.is_fully_resolved());
    assert_eq!(session.next_unresolved_after(0), Some(1));
    assert_eq!(session.prev_unresolved_before(0), Some(1));

    session.regions[0].resolution = ConflictRegionResolution::PickOurs;
    assert_eq!(session.solved_count(), 1);
    assert_eq!(session.unsolved_count(), 1);
    assert_eq!(session.next_unresolved_after(0), Some(1));
}

#[test]
fn session_from_merged_shared_text_reuses_region_backing() {
    let merged: Arc<str> = "\
start
<<<<<<< ours
local one
=======
remote one
>>>>>>> theirs
end
"
    .into();
    let session = ConflictSession::from_merged_shared_text(
        PathBuf::from("file.txt"),
        FileConflictKind::BothModified,
        ConflictPayload::Text("base\n".into()),
        ConflictPayload::Text("ours\n".into()),
        ConflictPayload::Text("theirs\n".into()),
        merged.clone(),
    );

    assert_eq!(session.regions.len(), 1);
    assert_eq!(session.regions[0].ours, "local one\n");
    assert_eq!(session.regions[0].theirs, "remote one\n");
    assert!(
        session.regions[0].ours.shares_backing_with(&merged),
        "ours slice should point into the original merged buffer",
    );
    assert!(
        session.regions[0].theirs.shares_backing_with(&merged),
        "theirs slice should point into the original merged buffer",
    );
}

#[test]
fn parse_regions_from_merged_text_replaces_existing_regions() {
    let mut session = make_session(vec![make_region(Some("b"), "o", "t")]);
    assert_eq!(session.total_regions(), 1);
    let parsed = session.parse_regions_from_merged_text("plain text without markers\n");
    assert_eq!(parsed, 0);
    assert!(session.regions.is_empty());
}

// -- Marker parser edge case tests --

#[test]
fn parse_regions_empty_conflict_blocks() {
    // Both sides empty
    let merged = "before\n<<<<<<< ours\n=======\n>>>>>>> theirs\nafter\n";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].ours, "");
    assert_eq!(regions[0].theirs, "");
    assert_eq!(regions[0].base, None);
}

#[test]
fn parse_regions_empty_ours_nonempty_theirs() {
    let merged = "<<<<<<< ours\n=======\ntheirs line\n>>>>>>> theirs\n";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].ours, "");
    assert_eq!(regions[0].theirs, "theirs line\n");
}

#[test]
fn parse_regions_nonempty_ours_empty_theirs() {
    let merged = "<<<<<<< ours\nours line\n=======\n>>>>>>> theirs\n";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].ours, "ours line\n");
    assert_eq!(regions[0].theirs, "");
}

#[test]
fn parse_regions_diff3_empty_base() {
    let merged = "<<<<<<< ours\nours\n||||||| base\n=======\ntheirs\n>>>>>>> theirs\n";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].ours, "ours\n");
    assert_eq!(regions[0].base.as_deref(), Some(""));
    assert_eq!(regions[0].theirs, "theirs\n");
}

#[test]
fn parse_regions_nested_marker_like_content() {
    // Content that looks like markers but is inside a conflict block
    let merged = "\
<<<<<<< ours
<<<<<<< nested-fake
some text
=======
other text
>>>>>>> theirs
";
    let regions = parse_conflict_regions_from_markers(merged);
    // The inner <<<<<<< starts a new parse attempt; the first block's ours
    // only gets "<<<<<<< nested-fake\nsome text\n" before seeing "======="
    // The parser should handle this gracefully.
    assert!(!regions.is_empty());
}

#[test]
fn parse_regions_separator_without_start_marker() {
    // Lone ======= should not create any regions
    let merged = "before\n=======\nafter\n";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 0);
}

#[test]
fn parse_regions_end_marker_without_start() {
    let merged = "before\n>>>>>>> theirs\nafter\n";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 0);
}

#[test]
fn parse_regions_marker_labels_with_extra_text() {
    // Marker lines can have arbitrary text after the marker
    let merged = "\
<<<<<<< HEAD (feature/my-branch)
ours content
||||||| merged common ancestors
base content
======= some extra text
theirs content
>>>>>>> origin/main (remote tracking)
";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].ours, "ours content\n");
    assert_eq!(regions[0].base.as_deref(), Some("base content\n"));
    assert_eq!(regions[0].theirs, "theirs content\n");
}

#[test]
fn parse_conflict_marker_ranges_two_way_markers() {
    let merged = "before\n<<<<<<< ours\nlocal 1\n=======\nremote 1\n>>>>>>> theirs\nafter\n";
    let ranges = parse_conflict_marker_ranges(merged);

    assert_eq!(ranges.len(), 3);
    match &ranges[0] {
        ParsedConflictSegmentRanges::Text(range) => {
            assert_eq!(slice_range(merged, range), "before\n");
        }
        other => panic!("expected leading text segment, got {other:?}"),
    }

    match &ranges[1] {
        ParsedConflictSegmentRanges::Conflict(block) => {
            assert_eq!(
                &merged[block.marker_start..block.marker_end],
                "<<<<<<< ours\nlocal 1\n=======\nremote 1\n>>>>>>> theirs\n"
            );
            assert_eq!(slice_range(merged, &block.ours), "local 1\n");
            assert_eq!(block.base, None);
            assert_eq!(slice_range(merged, &block.theirs), "remote 1\n");
        }
        other => panic!("expected conflict segment, got {other:?}"),
    }

    match &ranges[2] {
        ParsedConflictSegmentRanges::Text(range) => {
            assert_eq!(slice_range(merged, range), "after\n");
        }
        other => panic!("expected trailing text segment, got {other:?}"),
    }
}

#[test]
fn parse_conflict_marker_ranges_diff3_markers() {
    let merged = "\
<<<<<<< ours
local line
||||||| base
base line
=======
remote line
>>>>>>> theirs
";
    let ranges = parse_conflict_marker_ranges(merged);

    assert_eq!(ranges.len(), 1);
    match &ranges[0] {
        ParsedConflictSegmentRanges::Conflict(block) => {
            assert_eq!(
                &merged[block.marker_start..block.marker_end],
                "<<<<<<< ours\nlocal line\n||||||| base\nbase line\n=======\nremote line\n>>>>>>> theirs\n"
            );
            assert_eq!(slice_range(merged, &block.ours), "local line\n");
            assert_eq!(
                block.base.as_ref().map(|range| slice_range(merged, range)),
                Some("base line\n")
            );
            assert_eq!(slice_range(merged, &block.theirs), "remote line\n");
        }
        other => panic!("expected conflict segment, got {other:?}"),
    }
}

#[test]
fn parse_conflict_marker_ranges_preserve_malformed_no_separator_as_text() {
    let merged = "before\n<<<<<<< ours\nours line 1\nours line 2\n";
    let ranges = parse_conflict_marker_ranges(merged);

    assert_eq!(ranges.len(), 2);
    match &ranges[0] {
        ParsedConflictSegmentRanges::Text(range) => {
            assert_eq!(slice_range(merged, range), "before\n");
        }
        other => panic!("expected leading text segment, got {other:?}"),
    }
    match &ranges[1] {
        ParsedConflictSegmentRanges::Text(range) => {
            assert_eq!(
                slice_range(merged, range),
                "<<<<<<< ours\nours line 1\nours line 2\n"
            );
        }
        other => panic!("expected malformed block to remain text, got {other:?}"),
    }
}

#[test]
fn parse_conflict_marker_ranges_preserve_malformed_diff3_missing_end_as_text() {
    let merged = "\
before
<<<<<<< ours
ours
||||||| base
base
=======
theirs
";
    let ranges = parse_conflict_marker_ranges(merged);

    assert_eq!(ranges.len(), 2);
    match &ranges[0] {
        ParsedConflictSegmentRanges::Text(range) => {
            assert_eq!(slice_range(merged, range), "before\n");
        }
        other => panic!("expected leading text segment, got {other:?}"),
    }
    match &ranges[1] {
        ParsedConflictSegmentRanges::Text(range) => {
            assert_eq!(
                slice_range(merged, range),
                "<<<<<<< ours\nours\n||||||| base\nbase\n=======\ntheirs\n"
            );
        }
        other => panic!("expected malformed diff3 block to remain text, got {other:?}"),
    }
}

#[test]
fn parse_regions_missing_end_after_separator() {
    // Start + ours + separator found, but no end marker (EOF)
    let merged = "<<<<<<< ours\nours line\n=======\ntheirs line\n";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(
        regions.len(),
        0,
        "missing end marker should yield no regions"
    );
}

#[test]
fn parse_regions_missing_separator_in_diff3() {
    // diff3 base section started but no separator before EOF
    let merged = "<<<<<<< ours\nours line\n||||||| base\nbase line\n";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(
        regions.len(),
        0,
        "missing separator after base should yield no regions"
    );
}

#[test]
fn parse_regions_multiple_conflicts_with_varied_styles() {
    // Mix of 2-way and 3-way conflicts in same file
    let merged = "\
header
<<<<<<< ours
two-way ours
=======
two-way theirs
>>>>>>> theirs
middle
<<<<<<< ours
three-way ours
||||||| base
three-way base
=======
three-way theirs
>>>>>>> theirs
footer
";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].base, None);
    assert_eq!(regions[0].ours, "two-way ours\n");
    assert_eq!(regions[0].theirs, "two-way theirs\n");
    assert!(regions[1].base.is_some());
    assert_eq!(regions[1].ours, "three-way ours\n");
    assert_eq!(regions[1].base.as_deref(), Some("three-way base\n"));
    assert_eq!(regions[1].theirs, "three-way theirs\n");
}

#[test]
fn parse_regions_multiline_content_in_all_sections() {
    let merged = "\
<<<<<<< ours
ours line 1
ours line 2
ours line 3
||||||| base
base line 1
base line 2
=======
theirs line 1
theirs line 2
theirs line 3
theirs line 4
>>>>>>> theirs
";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].ours.lines().count(), 3);
    assert_eq!(regions[0].base.as_deref().unwrap().lines().count(), 2);
    assert_eq!(regions[0].theirs.lines().count(), 4);
}

#[test]
fn parse_regions_valid_then_truly_malformed_preserves_valid() {
    // First valid conflict followed by one with no separator or end — truly malformed
    let merged = "\
<<<<<<< ours
ok ours
=======
ok theirs
>>>>>>> theirs
<<<<<<< ours
unterminated content with no separator
";
    let regions = parse_conflict_regions_from_markers(merged);
    assert_eq!(
        regions.len(),
        1,
        "only the first valid conflict should be parsed"
    );
    assert_eq!(regions[0].ours, "ok ours\n");
    assert_eq!(regions[0].theirs, "ok theirs\n");
}

#[test]
fn parse_regions_no_trailing_newline_on_file() {
    let merged = "<<<<<<< ours\nfoo\n=======\nbar\n>>>>>>> theirs";
    let regions = parse_conflict_regions_from_markers(merged);
    // The end marker line ">>>>>>> theirs" has no trailing newline but still
    // starts with ">>>>>>>" so it should be recognized
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].ours, "foo\n");
    assert_eq!(regions[0].theirs, "bar\n");
}

// -- ConflictSession counter & navigation tests --

#[test]
fn counters_all_unresolved() {
    let session = make_session(vec![
        make_region(Some("b"), "a", "c"),
        make_region(Some("b"), "x", "y"),
        make_region(Some("b"), "p", "q"),
    ]);
    assert_eq!(session.total_regions(), 3);
    assert_eq!(session.solved_count(), 0);
    assert_eq!(session.unsolved_count(), 3);
    assert!(!session.is_fully_resolved());
}

#[test]
fn counters_mixed_resolved() {
    let mut session = make_session(vec![
        make_region(Some("b"), "a", "c"),
        make_region(Some("b"), "x", "y"),
        make_region(Some("b"), "p", "q"),
    ]);
    session.regions[1].resolution = ConflictRegionResolution::PickOurs;
    assert_eq!(session.solved_count(), 1);
    assert_eq!(session.unsolved_count(), 2);
    assert!(!session.is_fully_resolved());
}

#[test]
fn counters_all_resolved() {
    let mut session = make_session(vec![
        make_region(Some("b"), "a", "c"),
        make_region(Some("b"), "x", "y"),
    ]);
    session.regions[0].resolution = ConflictRegionResolution::PickOurs;
    session.regions[1].resolution = ConflictRegionResolution::PickTheirs;
    assert_eq!(session.solved_count(), 2);
    assert_eq!(session.unsolved_count(), 0);
    assert!(session.is_fully_resolved());
}

#[test]
fn next_unresolved_wraps_around() {
    let mut session = make_session(vec![
        make_region(Some("b"), "a", "c"),
        make_region(Some("b"), "x", "y"),
        make_region(Some("b"), "p", "q"),
    ]);
    // Resolve regions 0 and 1, leave 2 unresolved.
    session.regions[0].resolution = ConflictRegionResolution::PickOurs;
    session.regions[1].resolution = ConflictRegionResolution::PickOurs;

    // From position 0, next unresolved should be 2.
    assert_eq!(session.next_unresolved_after(0), Some(2));
    // From position 2, should wrap to find none (2 is the current, only it's unresolved).
    // Actually from 2 it wraps: tries 0 (resolved), 1 (resolved), 2 (unresolved) -> Some(2).
    assert_eq!(session.next_unresolved_after(2), Some(2));
}

#[test]
fn next_unresolved_returns_none_when_all_resolved() {
    let mut session = make_session(vec![
        make_region(Some("b"), "a", "c"),
        make_region(Some("b"), "x", "y"),
    ]);
    session.regions[0].resolution = ConflictRegionResolution::PickOurs;
    session.regions[1].resolution = ConflictRegionResolution::PickTheirs;
    assert_eq!(session.next_unresolved_after(0), None);
}

#[test]
fn prev_unresolved_wraps_around() {
    let mut session = make_session(vec![
        make_region(Some("b"), "a", "c"),
        make_region(Some("b"), "x", "y"),
        make_region(Some("b"), "p", "q"),
    ]);
    session.regions[1].resolution = ConflictRegionResolution::PickOurs;
    session.regions[2].resolution = ConflictRegionResolution::PickOurs;

    // From position 1, previous unresolved wraps to 0.
    assert_eq!(session.prev_unresolved_before(1), Some(0));
    // From position 0, should wrap: tries 2 (resolved), 1 (resolved), 0 (unresolved) -> Some(0).
    assert_eq!(session.prev_unresolved_before(0), Some(0));
}

#[test]
fn navigation_empty_regions() {
    let session = make_session(vec![]);
    assert_eq!(session.next_unresolved_after(0), None);
    assert_eq!(session.prev_unresolved_before(0), None);
}

// -- Auto-resolve tests --

#[test]
fn auto_resolve_identical_sides() {
    let region = make_region(Some("base\n"), "same\n", "same\n");
    let result = safe_auto_resolve(&region, false);
    assert!(result.is_some());
    let (rule, content) = result.unwrap();
    assert_eq!(rule, AutosolveRule::IdenticalSides);
    assert_eq!(content, "same\n");

    // Verify it works via session.
    let mut session = make_session(vec![region.clone()]);
    assert_eq!(session.auto_resolve_safe(), 1);
    assert!(session.is_fully_resolved());
}

#[test]
fn auto_resolve_only_ours_changed() {
    let region = make_region(Some("base\n"), "changed\n", "base\n");
    let result = safe_auto_resolve(&region, false);
    assert!(result.is_some());
    let (rule, content) = result.unwrap();
    assert_eq!(rule, AutosolveRule::OnlyOursChanged);
    assert_eq!(content, "changed\n");
}

#[test]
fn auto_resolve_only_theirs_changed() {
    let region = make_region(Some("base\n"), "base\n", "changed\n");
    let result = safe_auto_resolve(&region, false);
    assert!(result.is_some());
    let (rule, content) = result.unwrap();
    assert_eq!(rule, AutosolveRule::OnlyTheirsChanged);
    assert_eq!(content, "changed\n");
}

#[test]
fn auto_resolve_both_changed_differently_returns_none() {
    let region = make_region(Some("base\n"), "ours\n", "theirs\n");
    assert!(safe_auto_resolve(&region, false).is_none());
}

#[test]
fn auto_resolve_no_base_both_different_returns_none() {
    let region = make_region(None, "ours\n", "theirs\n");
    assert!(safe_auto_resolve(&region, false).is_none());
}

#[test]
fn auto_resolve_no_base_identical_sides() {
    let region = make_region(None, "same\n", "same\n");
    let result = safe_auto_resolve(&region, false);
    assert!(result.is_some());
    assert_eq!(result.unwrap().0, AutosolveRule::IdenticalSides);
}

#[test]
fn auto_resolve_whitespace_only_diff_resolves_when_enabled() {
    let region = make_region(Some("base\n"), "let  x = 1;\n", "let x  =  1;\n");
    // Without whitespace normalization, should not resolve.
    assert!(safe_auto_resolve(&region, false).is_none());
    // With whitespace normalization, should resolve picking ours.
    let result = safe_auto_resolve(&region, true);
    assert!(result.is_some());
    let (rule, content) = result.unwrap();
    assert_eq!(rule, AutosolveRule::WhitespaceOnly);
    assert_eq!(content, "let  x = 1;\n");
}

#[test]
fn auto_resolve_whitespace_only_no_base_resolves_when_enabled() {
    // 2-way conflict (no base) with whitespace-only diff.
    let region = make_region(None, "hello  world\n", "hello world\n");
    assert!(safe_auto_resolve(&region, false).is_none());
    let result = safe_auto_resolve(&region, true);
    assert!(result.is_some());
    assert_eq!(result.unwrap().0, AutosolveRule::WhitespaceOnly);
}

#[test]
fn auto_resolve_whitespace_session_with_options() {
    let mut session = make_session(vec![make_region(
        Some("base\n"),
        "let  x = 1;\n",
        "let x  =  1;\n",
    )]);
    // Without whitespace toggle, nothing resolves.
    assert_eq!(session.auto_resolve_safe(), 0);
    // With whitespace toggle, it resolves.
    assert_eq!(session.auto_resolve_safe_with_options(true), 1);
    assert!(session.is_fully_resolved());
}

#[test]
fn auto_resolve_session_multiple_regions() {
    let mut session = make_session(vec![
        make_region(Some("base\n"), "same\n", "same\n"), // identical → auto
        make_region(Some("base\n"), "ours\n", "theirs\n"), // both changed → no auto
        make_region(Some("base\n"), "changed\n", "base\n"), // only ours → auto
    ]);
    let resolved = session.auto_resolve_safe();
    assert_eq!(resolved, 2);
    assert_eq!(session.solved_count(), 2);
    assert_eq!(session.unsolved_count(), 1);
    assert!(!session.is_fully_resolved());

    // Region 0: auto-resolved
    assert!(matches!(
        session.regions[0].resolution,
        ConflictRegionResolution::AutoResolved {
            rule: AutosolveRule::IdenticalSides,
            ..
        }
    ));
    // Region 1: still unresolved
    assert!(matches!(
        session.regions[1].resolution,
        ConflictRegionResolution::Unresolved
    ));
    // Region 2: auto-resolved
    assert!(matches!(
        session.regions[2].resolution,
        ConflictRegionResolution::AutoResolved {
            rule: AutosolveRule::OnlyOursChanged,
            ..
        }
    ));
}

#[test]
fn auto_resolve_skips_already_resolved() {
    let mut session = make_session(vec![make_region(Some("base\n"), "same\n", "same\n")]);
    // Manually resolve first.
    session.regions[0].resolution = ConflictRegionResolution::PickOurs;
    // Auto-resolve should skip it.
    let resolved = session.auto_resolve_safe();
    assert_eq!(resolved, 0);
    // Still manually resolved, not overwritten.
    assert!(matches!(
        session.regions[0].resolution,
        ConflictRegionResolution::PickOurs
    ));
}

#[test]
fn regex_auto_resolve_equivalent_sides() {
    let options = RegexAutosolveOptions::whitespace_insensitive();
    let decision = regex_assisted_auto_resolve_pick(
        Some("let answer = 42;\n"),
        "let  answer = 42;\n",
        "let answer\t=\t42;\n",
        &options,
    );
    assert_eq!(
        decision,
        Some((AutosolveRule::RegexEquivalentSides, AutosolvePickSide::Ours))
    );
}

#[test]
fn regex_auto_resolve_only_theirs_changed_from_normalized_base() {
    let options = RegexAutosolveOptions::whitespace_insensitive();
    let decision = regex_assisted_auto_resolve_pick(
        Some("let answer = 42;\n"),
        "let answer=42;\n",
        "let answer = 43;\n",
        &options,
    );
    assert_eq!(
        decision,
        Some((
            AutosolveRule::RegexOnlyTheirsChanged,
            AutosolvePickSide::Theirs
        ))
    );
}

#[test]
fn regex_auto_resolve_only_ours_changed_from_normalized_base() {
    let options = RegexAutosolveOptions::whitespace_insensitive();
    let decision = regex_assisted_auto_resolve_pick(
        Some("let answer = 42;\n"),
        "let answer = 43;\n",
        "let\tanswer=42;\n",
        &options,
    );
    assert_eq!(
        decision,
        Some((AutosolveRule::RegexOnlyOursChanged, AutosolvePickSide::Ours))
    );
}

#[test]
fn regex_auto_resolve_invalid_pattern_is_ignored() {
    let options = RegexAutosolveOptions::default().with_pattern("(", "");
    let decision = regex_assisted_auto_resolve_pick(Some("base\n"), "ours\n", "theirs\n", &options);
    assert!(decision.is_none());
}

#[test]
fn session_auto_resolve_regex_applies_to_unresolved_regions() {
    let mut session = make_session(vec![
        make_region(
            Some("let answer = 42;\n"),
            "let  answer = 42;\n",
            "let answer\t=\t42;\n",
        ),
        make_region(Some("base\n"), "ours\n", "theirs\n"),
    ]);
    let options = RegexAutosolveOptions::whitespace_insensitive();

    assert_eq!(session.auto_resolve_regex(&options), 1);
    assert_eq!(session.solved_count(), 1);
    assert_eq!(session.unsolved_count(), 1);
    match &session.regions[0].resolution {
        ConflictRegionResolution::AutoResolved {
            rule,
            confidence,
            content,
        } => {
            assert_eq!(*rule, AutosolveRule::RegexEquivalentSides);
            assert_eq!(*confidence, AutosolveConfidence::Medium);
            assert_eq!(content, "let  answer = 42;\n");
        }
        other => panic!("expected regex auto-resolved region, got {:?}", other),
    }
    assert!(matches!(
        session.regions[1].resolution,
        ConflictRegionResolution::Unresolved
    ));
}

// -- ConflictSession::new tests --

#[test]
fn session_new_text_conflict() {
    let session = ConflictSession::new(
        PathBuf::from("file.txt"),
        FileConflictKind::BothModified,
        ConflictPayload::Text("base".into()),
        ConflictPayload::Text("ours".into()),
        ConflictPayload::Text("theirs".into()),
    );
    assert_eq!(session.strategy, ConflictResolverStrategy::FullTextResolver);
    assert_eq!(session.total_regions(), 0); // No regions parsed yet
}

#[test]
fn session_side_byte_accessors_expose_all_payload_bytes() {
    let session = ConflictSession::new_with_current(
        PathBuf::from("file.bin"),
        FileConflictKind::BothModified,
        ConflictPayload::Binary(vec![0x00, 0x01].into()),
        ConflictPayload::Text("ours\n".into()),
        ConflictPayload::Absent,
        ConflictPayload::Binary(vec![0x02, 0x03].into()),
    );
    assert_eq!(session.base_bytes(), Some([0x00_u8, 0x01].as_slice()));
    assert_eq!(session.ours_bytes(), Some("ours\n".as_bytes()));
    assert_eq!(session.theirs_bytes(), None);
    assert_eq!(session.current_bytes(), Some([0x02_u8, 0x03].as_slice()));
    assert_eq!(session.current_text(), None);
}

#[test]
fn session_new_with_absent_current_preserves_loaded_absence() {
    let session = ConflictSession::new_with_current(
        PathBuf::from("deleted.txt"),
        FileConflictKind::BothDeleted,
        ConflictPayload::Text("base\n".into()),
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
    );
    assert!(matches!(
        session.current.as_ref(),
        Some(ConflictPayload::Absent)
    ));
    assert_eq!(session.current_bytes(), None);
    assert_eq!(session.current_text(), None);
}

#[test]
fn session_new_binary_conflict() {
    let session = ConflictSession::new(
        PathBuf::from("image.png"),
        FileConflictKind::BothModified,
        ConflictPayload::Binary(vec![0xFF].into()),
        ConflictPayload::Text("ours".into()),
        ConflictPayload::Text("theirs".into()),
    );
    assert_eq!(session.strategy, ConflictResolverStrategy::BinarySidePick);
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.solved_count(), 0);
    assert_eq!(session.unsolved_count(), 1);
    assert!(!session.is_fully_resolved());
    assert!(session.regions.is_empty());
}

#[test]
fn session_new_deleted_by_us() {
    let session = ConflictSession::new(
        PathBuf::from("file.txt"),
        FileConflictKind::DeletedByUs,
        ConflictPayload::Text("base".into()),
        ConflictPayload::Absent,
        ConflictPayload::Text("theirs".into()),
    );
    assert_eq!(session.strategy, ConflictResolverStrategy::TwoWayKeepDelete);
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.regions[0].base.as_deref(), Some("base"));
    assert_eq!(session.regions[0].ours, "");
    assert_eq!(session.regions[0].theirs, "theirs");
    assert!(matches!(
        session.regions[0].resolution,
        ConflictRegionResolution::Unresolved
    ));
}

#[test]
fn session_new_both_deleted() {
    let session = ConflictSession::new(
        PathBuf::from("file.txt"),
        FileConflictKind::BothDeleted,
        ConflictPayload::Text("base".into()),
        ConflictPayload::Absent,
        ConflictPayload::Absent,
    );
    assert_eq!(session.strategy, ConflictResolverStrategy::DecisionOnly);
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.regions[0].base.as_deref(), Some("base"));
    assert_eq!(session.regions[0].ours, "");
    assert_eq!(session.regions[0].theirs, "");
}

#[test]
fn from_merged_text_without_markers_keeps_synthetic_two_way_region() {
    let session = ConflictSession::from_merged_text(
        PathBuf::from("file.txt"),
        FileConflictKind::AddedByUs,
        ConflictPayload::Absent,
        ConflictPayload::Text("ours\n".into()),
        ConflictPayload::Absent,
        "ours\n",
    );
    assert_eq!(session.strategy, ConflictResolverStrategy::TwoWayKeepDelete);
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.regions[0].base, None);
    assert_eq!(session.regions[0].ours, "ours\n");
    assert_eq!(session.regions[0].theirs, "");
    assert_eq!(session.current_text(), Some("ours\n"));
}

#[test]
fn has_unresolved_markers_reflects_unsolved() {
    let mut session = make_session(vec![make_region(Some("b"), "a", "c")]);
    assert!(session.has_unresolved_markers());
    session.regions[0].resolution = ConflictRegionResolution::PickOurs;
    assert!(!session.has_unresolved_markers());
}

// -- AutosolveRule description test --

#[test]
fn autosolve_rule_descriptions() {
    assert!(!AutosolveRule::IdenticalSides.description().is_empty());
    assert!(!AutosolveRule::OnlyOursChanged.description().is_empty());
    assert!(!AutosolveRule::OnlyTheirsChanged.description().is_empty());
    assert!(!AutosolveRule::RegexEquivalentSides.description().is_empty());
    assert!(
        !AutosolveRule::RegexOnlyTheirsChanged
            .description()
            .is_empty()
    );
    assert!(!AutosolveRule::RegexOnlyOursChanged.description().is_empty());
    assert!(!AutosolveRule::SubchunkFullyMerged.description().is_empty());
    assert!(!AutosolveRule::HistoryMerged.description().is_empty());
}

#[test]
fn autosolve_rule_confidence_levels() {
    assert_eq!(
        AutosolveRule::IdenticalSides.confidence(),
        AutosolveConfidence::High
    );
    assert_eq!(
        AutosolveRule::OnlyOursChanged.confidence(),
        AutosolveConfidence::High
    );
    assert_eq!(
        AutosolveRule::WhitespaceOnly.confidence(),
        AutosolveConfidence::Medium
    );
    assert_eq!(
        AutosolveRule::RegexEquivalentSides.confidence(),
        AutosolveConfidence::Medium
    );
    assert_eq!(
        AutosolveRule::SubchunkFullyMerged.confidence(),
        AutosolveConfidence::Medium
    );
    assert_eq!(
        AutosolveRule::HistoryMerged.confidence(),
        AutosolveConfidence::Low
    );
}

#[test]
fn autosolve_confidence_labels() {
    assert_eq!(AutosolveConfidence::High.label(), "high");
    assert_eq!(AutosolveConfidence::Medium.label(), "medium");
    assert_eq!(AutosolveConfidence::Low.label(), "low");
}

// -- Pass 2: subchunk splitting tests --

#[test]
fn subchunk_split_identical_sides_returns_none() {
    // Pass 1 handles this — don't split.
    assert!(split_conflict_into_subchunks("base\n", "same\n", "same\n").is_none());
}

#[test]
fn subchunk_split_ours_equals_base_returns_none() {
    // Pass 1 handles this.
    assert!(split_conflict_into_subchunks("base\n", "base\n", "changed\n").is_none());
}

#[test]
fn subchunk_split_theirs_equals_base_returns_none() {
    // Pass 1 handles this.
    assert!(split_conflict_into_subchunks("base\n", "changed\n", "base\n").is_none());
}

#[test]
fn subchunk_split_single_line_conflict_returns_none() {
    // Both sides changed the only line — no way to split meaningfully.
    assert!(split_conflict_into_subchunks("original\n", "ours\n", "theirs\n").is_none());
}

#[test]
fn subchunk_split_mixed_lines() {
    // Base has 3 lines. Ours changes line 1, theirs changes line 3.
    // Line 2 is the same across all three → context.
    let base = "aaa\nbbb\nccc\n";
    let ours = "AAA\nbbb\nccc\n";
    let theirs = "aaa\nbbb\nCCC\n";

    let subchunks = split_conflict_into_subchunks(base, ours, theirs);
    assert!(subchunks.is_some(), "should split into subchunks");
    let subchunks = subchunks.unwrap();

    // All subchunks should be resolved because changes don't overlap.
    assert!(
        subchunks.iter().all(|c| matches!(c, Subchunk::Resolved(_))),
        "non-overlapping changes should all auto-merge"
    );

    // Concatenated resolved text should be the merged result.
    let merged: String = subchunks
        .iter()
        .map(|c| match c {
            Subchunk::Resolved(t) => t.as_str(),
            _ => panic!("unexpected conflict"),
        })
        .collect();
    assert_eq!(merged, "AAA\nbbb\nCCC\n");
}

#[test]
fn subchunk_split_with_remaining_conflict() {
    // Both sides change the same line (line 2), different changes on line 1.
    let base = "aaa\nbbb\nccc\n";
    let ours = "AAA\nBBB\nccc\n";
    let theirs = "XXX\nYYY\nccc\n";

    let subchunks = split_conflict_into_subchunks(base, ours, theirs);
    assert!(subchunks.is_some(), "should split");
    let subchunks = subchunks.unwrap();

    let has_resolved = subchunks.iter().any(|c| matches!(c, Subchunk::Resolved(_)));
    let has_conflict = subchunks
        .iter()
        .any(|c| matches!(c, Subchunk::Conflict { .. }));
    assert!(has_resolved, "should have resolved parts (line 3)");
    assert!(has_conflict, "should have conflicting parts (lines 1-2)");
}

#[test]
fn subchunk_split_only_one_side_adds_lines() {
    // Ours adds a line, theirs doesn't change anything.
    // But theirs != base overall, so this is a genuine 3-way conflict.
    let base = "aaa\nccc\n";
    let ours = "aaa\nbbb\nccc\n";
    let theirs = "aaa\nCCC\n";

    let subchunks = split_conflict_into_subchunks(base, ours, theirs);
    assert!(subchunks.is_some());
    let subchunks = subchunks.unwrap();

    // Should have context "aaa\n" resolved, then a conflict for the rest.
    let first = &subchunks[0];
    assert!(
        matches!(first, Subchunk::Resolved(t) if t == "aaa\n"),
        "first subchunk should be resolved context"
    );
}

#[test]
fn subchunk_split_both_change_same_line_identically() {
    // Both sides change line 2 the same way.
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nBBB\nccc\n";
    let theirs = "aaa\nBBB\nccc\n";

    // This would be caught by Pass 1 (ours == theirs), returns None.
    assert!(split_conflict_into_subchunks(base, ours, theirs).is_none());
}

#[test]
fn subchunk_split_nonoverlapping_changes_fully_merge() {
    // Ours changes line 1, theirs changes line 3. Line 2 is context.
    let base = "line1\nline2\nline3\n";
    let ours = "LINE1\nline2\nline3\n";
    let theirs = "line1\nline2\nLINE3\n";

    let subchunks = split_conflict_into_subchunks(base, ours, theirs).unwrap();

    // Should be fully resolved.
    assert!(subchunks.iter().all(|c| matches!(c, Subchunk::Resolved(_))));

    let merged: String = subchunks
        .iter()
        .map(|c| match c {
            Subchunk::Resolved(t) => t.as_str(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(merged, "LINE1\nline2\nLINE3\n");
}

#[test]
fn subchunk_split_overlapping_different_changes_conflict() {
    // Both sides change the same line differently.
    let base = "ctx\noriginal\nctx2\n";
    let ours = "ctx\nours_version\nctx2\n";
    let theirs = "ctx\ntheirs_version\nctx2\n";

    let subchunks = split_conflict_into_subchunks(base, ours, theirs).unwrap();

    // Should have context + conflict + context.
    assert_eq!(subchunks.len(), 3);
    assert!(matches!(&subchunks[0], Subchunk::Resolved(t) if t == "ctx\n"));
    assert!(
        matches!(&subchunks[1], Subchunk::Conflict { base, ours, theirs }
            if base == "original\n" && ours == "ours_version\n" && theirs == "theirs_version\n"
        )
    );
    assert!(matches!(&subchunks[2], Subchunk::Resolved(t) if t == "ctx2\n"));
}

#[test]
fn subchunk_session_pass2_fully_merges() {
    let mut session = make_session(vec![ConflictRegion {
        base: Some("line1\nline2\nline3\n".into()),
        ours: "LINE1\nline2\nline3\n".into(),
        theirs: "line1\nline2\nLINE3\n".into(),
        resolution: ConflictRegionResolution::Unresolved,
    }]);

    // Pass 1 can't resolve this (both sides changed differently from base).
    assert_eq!(session.auto_resolve_safe(), 0);

    // Pass 2 should fully merge it (non-overlapping changes).
    assert_eq!(session.auto_resolve_pass2(), 1);
    assert!(session.is_fully_resolved());

    match &session.regions[0].resolution {
        ConflictRegionResolution::AutoResolved {
            rule,
            confidence,
            content,
        } => {
            assert_eq!(*rule, AutosolveRule::SubchunkFullyMerged);
            assert_eq!(*confidence, AutosolveConfidence::Medium);
            assert_eq!(content, "LINE1\nline2\nLINE3\n");
        }
        other => panic!("expected AutoResolved, got {:?}", other),
    }
}

#[test]
fn subchunk_session_pass2_skips_partial_conflicts() {
    let mut session = make_session(vec![ConflictRegion {
        base: Some("ctx\noriginal\nctx2\n".into()),
        ours: "ctx\nours_version\nctx2\n".into(),
        theirs: "ctx\ntheirs_version\nctx2\n".into(),
        resolution: ConflictRegionResolution::Unresolved,
    }]);

    // Pass 2 can't fully merge (overlap on line 2), so region stays unresolved.
    assert_eq!(session.auto_resolve_pass2(), 0);
    assert!(!session.is_fully_resolved());
}

#[test]
fn subchunk_split_empty_base() {
    // Empty base, both sides have content.
    let base = "";
    let ours = "aaa\n";
    let theirs = "bbb\n";

    // Both sides differ from base and from each other.
    let result = split_conflict_into_subchunks(base, ours, theirs);
    // Can't meaningfully split an empty base with different insertions.
    assert!(result.is_none());
}

#[test]
fn subchunk_split_with_deletions() {
    // Ours deletes line 2, theirs changes line 3.
    let base = "aaa\nbbb\nccc\n";
    let ours = "aaa\nccc\n";
    let theirs = "aaa\nbbb\nCCC\n";

    let subchunks = split_conflict_into_subchunks(base, ours, theirs);
    assert!(subchunks.is_some());
    let subchunks = subchunks.unwrap();

    // Should be fully resolved: non-overlapping changes.
    assert!(subchunks.iter().all(|c| matches!(c, Subchunk::Resolved(_))));

    let merged: String = subchunks
        .iter()
        .map(|c| match c {
            Subchunk::Resolved(t) => t.as_str(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(merged, "aaa\nCCC\n");
}

#[test]
fn subchunk_split_skips_large_inputs() {
    // Inputs exceeding SUBCHUNK_MAX_LINES (500) should return None.
    let large = (0..501).map(|i| format!("line {i}\n")).collect::<String>();
    let base = &large;
    let ours = &large.replace("line 0", "LINE 0");
    let theirs = &large.replace("line 1", "LINE 1");

    let result = split_conflict_into_subchunks(base, ours, theirs);
    assert!(
        result.is_none(),
        "inputs exceeding SUBCHUNK_MAX_LINES should return None"
    );

    // Just under the limit should still work.
    let medium = (0..500).map(|i| format!("line {i}\n")).collect::<String>();
    let base = &medium;
    let ours = &medium.replace("line 0", "LINE 0");
    let theirs = &medium.replace("line 1", "LINE 1");

    let result = split_conflict_into_subchunks(base, ours, theirs);
    assert!(
        result.is_some(),
        "inputs at SUBCHUNK_MAX_LINES boundary should still split"
    );
}

// -- History-aware auto-resolve tests --

#[test]
fn history_merge_deduplicates_bullet_entries() {
    let options = HistoryAutosolveOptions::bullet_list();
    let base = "# Changelog\n- Added foo\n- Fixed bar\n";
    let ours = "# Changelog\n- Added foo\n- Fixed bar\n- Added baz\n";
    let theirs = "# Changelog\n- Added foo\n- Fixed bar\n- Fixed qux\n";

    let result = history_merge_region(Some(base), ours, theirs, &options);
    assert!(result.is_some(), "should merge changelog entries");
    let merged = result.unwrap();

    // Both new entries should be present.
    assert!(
        merged.contains("- Added baz"),
        "should contain ours' new entry"
    );
    assert!(
        merged.contains("- Fixed qux"),
        "should contain theirs' new entry"
    );
    // Common entries should appear exactly once.
    assert_eq!(
        merged.matches("- Added foo").count(),
        1,
        "deduped: Added foo"
    );
    assert_eq!(
        merged.matches("- Fixed bar").count(),
        1,
        "deduped: Fixed bar"
    );
}

#[test]
fn history_merge_no_section_marker_returns_none() {
    let options = HistoryAutosolveOptions::bullet_list();
    // Text without any changelog section header.
    let ours = "let x = 1;\nlet y = 2;\n";
    let theirs = "let x = 3;\nlet y = 4;\n";

    let result = history_merge_region(None, ours, theirs, &options);
    assert!(result.is_none(), "should not match non-changelog text");
}

#[test]
fn history_merge_invalid_options_returns_none() {
    let options = HistoryAutosolveOptions::default(); // empty patterns
    assert!(!options.is_valid());

    let result = history_merge_region(None, "a\n", "b\n", &options);
    assert!(result.is_none());
}

#[test]
fn history_merge_keepachangelog_style() {
    let options = HistoryAutosolveOptions::keepachangelog();
    let base = "## [1.0.0] - 2024-01-01\n- Initial release\n";
    let ours =
        "## [1.1.0] - 2024-02-01\n- Added feature A\n## [1.0.0] - 2024-01-01\n- Initial release\n";
    let theirs =
        "## [1.0.1] - 2024-01-15\n- Fixed bug B\n## [1.0.0] - 2024-01-01\n- Initial release\n";

    let result = history_merge_region(Some(base), ours, theirs, &options);
    assert!(result.is_some(), "should merge keepachangelog entries");
    let merged = result.unwrap();

    assert!(merged.contains("## [1.1.0]"), "should contain ours' entry");
    assert!(
        merged.contains("## [1.0.1]"),
        "should contain theirs' entry"
    );
    assert!(merged.contains("## [1.0.0]"), "should contain base entry");
    // The base entry should appear only once (deduped).
    assert_eq!(
        merged.matches("## [1.0.0]").count(),
        1,
        "deduped base entry"
    );
}

#[test]
fn history_merge_identical_additions_deduped() {
    let options = HistoryAutosolveOptions::bullet_list();
    let base = "# Changes\n- Existing\n";
    let ours = "# Changes\n- Existing\n- New feature\n";
    let theirs = "# Changes\n- Existing\n- New feature\n";

    let result = history_merge_region(Some(base), ours, theirs, &options);
    assert!(result.is_some());
    let merged = result.unwrap();
    assert_eq!(
        merged.matches("- New feature").count(),
        1,
        "identical additions should be deduped"
    );
}

#[test]
fn history_merge_with_sort() {
    let mut options = HistoryAutosolveOptions::bullet_list();
    options.sort_entries = true;

    let base = "# Changes\n";
    let ours = "# Changes\n- B entry\n- D entry\n";
    let theirs = "# Changes\n- A entry\n- C entry\n";

    let result = history_merge_region(Some(base), ours, theirs, &options);
    assert!(result.is_some());
    let merged = result.unwrap();

    // With sorting, entries should be in alphabetical order.
    let a_pos = merged.find("- A entry").unwrap();
    let b_pos = merged.find("- B entry").unwrap();
    let c_pos = merged.find("- C entry").unwrap();
    let d_pos = merged.find("- D entry").unwrap();
    assert!(a_pos < b_pos, "A before B");
    assert!(b_pos < c_pos, "B before C");
    assert!(c_pos < d_pos, "C before D");
}

#[test]
fn history_merge_with_max_entries() {
    let mut options = HistoryAutosolveOptions::bullet_list();
    options.max_entries = Some(2);

    let base = "# Changes\n";
    let ours = "# Changes\n- Entry 1\n- Entry 2\n- Entry 3\n";
    let theirs = "# Changes\n- Entry 4\n";

    let result = history_merge_region(Some(base), ours, theirs, &options);
    assert!(result.is_some());
    let merged = result.unwrap();

    // Should only have 2 entries (truncated).
    let entry_count = merged.matches("\n- ").count();
    assert!(
        entry_count <= 2,
        "should be truncated to max 2 entries, got {}",
        entry_count
    );
}

#[test]
fn history_merge_session_method() {
    let options = HistoryAutosolveOptions::bullet_list();
    let base_text = "# Changelog\n- Original\n";
    let ours_text = "# Changelog\n- Original\n- Added by ours\n";
    let theirs_text = "# Changelog\n- Original\n- Added by theirs\n";

    let mut session = make_session(vec![ConflictRegion {
        base: Some(base_text.into()),
        ours: ours_text.into(),
        theirs: theirs_text.into(),
        resolution: ConflictRegionResolution::Unresolved,
    }]);

    let resolved = session.auto_resolve_history(&options);
    assert_eq!(resolved, 1);
    assert!(session.is_fully_resolved());
    match &session.regions[0].resolution {
        ConflictRegionResolution::AutoResolved {
            rule,
            confidence,
            content,
        } => {
            assert_eq!(*rule, AutosolveRule::HistoryMerged);
            assert_eq!(*confidence, AutosolveConfidence::Low);
            assert!(content.contains("- Added by ours"));
            assert!(content.contains("- Added by theirs"));
        }
        other => panic!("expected HistoryMerged, got {:?}", other),
    }
}

#[test]
fn history_merge_skips_already_resolved() {
    let options = HistoryAutosolveOptions::bullet_list();
    let mut session = make_session(vec![ConflictRegion {
        base: Some("# Changelog\n- Original\n".into()),
        ours: "# Changelog\n- Original\n- New\n".into(),
        theirs: "# Changelog\n- Original\n- Other\n".into(),
        resolution: ConflictRegionResolution::PickOurs,
    }]);

    let resolved = session.auto_resolve_history(&options);
    assert_eq!(resolved, 0);
}

#[test]
fn history_merge_no_base_still_works() {
    let options = HistoryAutosolveOptions::bullet_list();
    let ours = "# Changes\n- Feature A\n- Feature B\n";
    let theirs = "# Changes\n- Feature B\n- Feature C\n";

    let result = history_merge_region(None, ours, theirs, &options);
    assert!(result.is_some());
    let merged = result.unwrap();
    assert!(merged.contains("- Feature A"));
    assert!(merged.contains("- Feature B"));
    assert!(merged.contains("- Feature C"));
    assert_eq!(merged.matches("- Feature B").count(), 1, "deduped");
}

#[test]
fn history_autosolve_rule_description() {
    assert!(!AutosolveRule::HistoryMerged.description().is_empty());
}

// -- history merge trailing content --

#[test]
fn history_merge_preserves_trailing_content() {
    let options = HistoryAutosolveOptions::bullet_list();
    let base = "# Changelog\n- Entry 1\n\n## License\nMIT\n";
    let ours = "# Changelog\n- Entry 1\n- Entry 2\n\n## License\nMIT\n";
    let theirs = "# Changelog\n- Entry 1\n- Entry 3\n\n## License\nMIT\n";

    let result = history_merge_region(Some(base), ours, theirs, &options);
    assert!(
        result.is_some(),
        "should merge changelog with trailing content"
    );
    let merged = result.unwrap();

    assert!(merged.contains("- Entry 2"), "should have ours entry");
    assert!(merged.contains("- Entry 3"), "should have theirs entry");
    assert!(
        merged.contains("## License\nMIT\n"),
        "should preserve trailing content"
    );
    // Trailing content should appear exactly once.
    assert_eq!(
        merged.matches("## License").count(),
        1,
        "trailing content should not be duplicated"
    );
}

#[test]
fn history_merge_preserves_trailing_blank_lines() {
    let options = HistoryAutosolveOptions::bullet_list();
    let ours = "# Changes\n- Entry A\n\n";
    let theirs = "# Changes\n- Entry B\n\n";

    let result = history_merge_region(None, ours, theirs, &options);
    assert!(result.is_some());
    let merged = result.unwrap();
    assert!(merged.contains("- Entry A"));
    assert!(merged.contains("- Entry B"));
    // Should preserve trailing blank line.
    assert!(
        merged.ends_with("\n\n"),
        "should preserve trailing blank lines"
    );
}

#[test]
fn history_merge_no_trailing_content_still_works() {
    // Entries go to end of text, no trailing section.
    let options = HistoryAutosolveOptions::bullet_list();
    let base = "# Changelog\n- Old entry\n";
    let ours = "# Changelog\n- Old entry\n- New ours\n";
    let theirs = "# Changelog\n- Old entry\n- New theirs\n";

    let result = history_merge_region(Some(base), ours, theirs, &options);
    assert!(result.is_some());
    let merged = result.unwrap();
    assert!(merged.contains("- New ours"));
    assert!(merged.contains("- New theirs"));
    // No trailing content should be present.
    let last_line = merged.trim_end().lines().last().unwrap_or("");
    assert!(last_line.starts_with("- "), "should end with an entry line");
}

#[test]
fn history_section_suffix_extracts_trailing_section() {
    let entry_re = Regex::new(r"^[-*]\s+").unwrap();
    let text = "- Entry 1\n- Entry 2\n\n## Footer\nSome text\n";

    let suffix = history_section_suffix(text, &entry_re);
    assert_eq!(suffix, "\n## Footer\nSome text\n");
}

#[test]
fn history_section_suffix_returns_empty_when_no_trailing() {
    let entry_re = Regex::new(r"^[-*]\s+").unwrap();
    let text = "- Entry 1\n- Entry 2\n";

    let suffix = history_section_suffix(text, &entry_re);
    assert!(suffix.is_empty());
}

#[test]
fn history_section_suffix_captures_trailing_blank_lines() {
    let entry_re = Regex::new(r"^[-*]\s+").unwrap();
    let text = "- Entry 1\n\n";

    let suffix = history_section_suffix(text, &entry_re);
    assert_eq!(suffix, "\n", "trailing blank line should be the suffix");
}

// -- counter/navigation correctness after sequential region picks --

#[test]
fn counters_track_sequential_region_resolution() {
    // Resolve 4 regions one at a time, verify counters at each step.
    let regions = vec![
        make_region(Some("b1"), "o1", "t1"),
        make_region(Some("b2"), "o2", "t2"),
        make_region(Some("b3"), "o3", "t3"),
        make_region(None, "o4", "t4"),
    ];
    let mut session = make_session(regions);
    assert_eq!(session.total_regions(), 4);
    assert_eq!(session.solved_count(), 0);
    assert_eq!(session.unsolved_count(), 4);
    assert!(!session.is_fully_resolved());

    // Resolve region 0
    session.regions[0].resolution = ConflictRegionResolution::PickOurs;
    assert_eq!(session.solved_count(), 1);
    assert_eq!(session.unsolved_count(), 3);

    // Resolve region 2 (skip 1)
    session.regions[2].resolution = ConflictRegionResolution::PickTheirs;
    assert_eq!(session.solved_count(), 2);
    assert_eq!(session.unsolved_count(), 2);

    // Resolve region 1
    session.regions[1].resolution = ConflictRegionResolution::PickBase;
    assert_eq!(session.solved_count(), 3);
    assert_eq!(session.unsolved_count(), 1);

    // Resolve region 3
    session.regions[3].resolution = ConflictRegionResolution::PickBoth;
    assert_eq!(session.solved_count(), 4);
    assert_eq!(session.unsolved_count(), 0);
    assert!(session.is_fully_resolved());
}

#[test]
fn navigation_skips_resolved_regions_correctly() {
    // 5 regions, resolve 0, 2, 4 → only 1 and 3 remain unresolved.
    let regions = vec![
        make_region(Some("b"), "o1", "t1"),
        make_region(Some("b"), "o2", "t2"),
        make_region(Some("b"), "o3", "t3"),
        make_region(Some("b"), "o4", "t4"),
        make_region(Some("b"), "o5", "t5"),
    ];
    let mut session = make_session(regions);

    session.regions[0].resolution = ConflictRegionResolution::PickOurs;
    session.regions[2].resolution = ConflictRegionResolution::PickTheirs;
    session.regions[4].resolution = ConflictRegionResolution::PickBase;

    // Next from 0 → 1 (first unresolved)
    assert_eq!(session.next_unresolved_after(0), Some(1));
    // Next from 1 → 3 (skips resolved 2)
    assert_eq!(session.next_unresolved_after(1), Some(3));
    // Next from 3 → wraps to 1 (skips resolved 4, 0)
    assert_eq!(session.next_unresolved_after(3), Some(1));

    // Prev from 3 → 1 (skips resolved 2)
    assert_eq!(session.prev_unresolved_before(3), Some(1));
    // Prev from 1 → wraps to 3 (skips resolved 0, 4)
    assert_eq!(session.prev_unresolved_before(1), Some(3));

    // Resolve remaining
    session.regions[1].resolution = ConflictRegionResolution::PickOurs;
    session.regions[3].resolution = ConflictRegionResolution::PickTheirs;
    assert!(session.is_fully_resolved());
    assert_eq!(session.next_unresolved_after(0), None);
    assert_eq!(session.prev_unresolved_before(0), None);
}

#[test]
fn autosolve_updates_counters_and_navigation() {
    // Verify counters are correct after auto_resolve_safe runs.
    let regions = vec![
        // Region 0: identical sides → auto-resolve
        make_region(Some("base"), "same", "same"),
        // Region 1: both changed differently → stays unresolved
        make_region(Some("base"), "ours_change", "theirs_change"),
        // Region 2: only ours changed → auto-resolve
        make_region(Some("base"), "changed", "base"),
    ];
    let mut session = make_session(regions);
    assert_eq!(session.unsolved_count(), 3);

    let resolved_count = session.auto_resolve_safe();
    assert_eq!(resolved_count, 2); // regions 0 and 2

    assert_eq!(session.solved_count(), 2);
    assert_eq!(session.unsolved_count(), 1);
    assert!(!session.is_fully_resolved());

    // Navigation should only find region 1
    assert_eq!(session.next_unresolved_after(0), Some(1));
    assert_eq!(session.next_unresolved_after(1), Some(1));
    assert_eq!(session.prev_unresolved_before(2), Some(1));
}

// ── try_autosolve_merged_text tests ──────────────────────────────

#[test]
fn autosolve_no_conflicts_returns_text_as_is() {
    let text = "clean\nfile\ncontent\n";
    let result = try_autosolve_merged_text(text);
    assert_eq!(result, Some(text.to_string()));
}

#[test]
fn autosolve_identical_sides_resolves() {
    let text = "before\n\
            <<<<<<< ours\n\
            same change\n\
            =======\n\
            same change\n\
            >>>>>>> theirs\n\
            after\n";
    let result = try_autosolve_merged_text(text);
    assert_eq!(result, Some("before\nsame change\nafter\n".to_string()));
}

#[test]
fn autosolve_whitespace_only_diff_resolves() {
    let text = "before\n\
            <<<<<<< ours\n\
            hello  world\n\
            =======\n\
            hello world\n\
            >>>>>>> theirs\n\
            after\n";
    let result = try_autosolve_merged_text(text);
    // Whitespace-only: picks ours.
    assert_eq!(result, Some("before\nhello  world\nafter\n".to_string()));
}

#[test]
fn autosolve_diff3_single_side_change_resolves() {
    let text = "before\n\
            <<<<<<< ours\n\
            unchanged\n\
            ||||||| base\n\
            unchanged\n\
            =======\n\
            modified\n\
            >>>>>>> theirs\n\
            after\n";
    let result = try_autosolve_merged_text(text);
    // Ours == base, only theirs changed → pick theirs.
    assert_eq!(result, Some("before\nmodified\nafter\n".to_string()));
}

#[test]
fn autosolve_diff3_subchunk_split_resolves() {
    // Two non-overlapping changes within a single conflict block.
    let text = "ctx\n\
            <<<<<<< ours\n\
            aaa\n\
            BBB\n\
            ccc\n\
            ||||||| base\n\
            aaa\n\
            bbb\n\
            ccc\n\
            =======\n\
            AAA\n\
            bbb\n\
            ccc\n\
            >>>>>>> theirs\n\
            end\n";
    let result = try_autosolve_merged_text(text);
    // Ours changed line 2, theirs changed line 1 → subchunk merge.
    assert_eq!(result, Some("ctx\nAAA\nBBB\nccc\nend\n".to_string()));
}

#[test]
fn autosolve_true_conflict_returns_none() {
    let text = "before\n\
            <<<<<<< ours\n\
            completely different\n\
            =======\n\
            totally different\n\
            >>>>>>> theirs\n\
            after\n";
    let result = try_autosolve_merged_text(text);
    assert!(result.is_none());
}

#[test]
fn autosolve_partial_resolve_returns_none() {
    // Two conflicts: first resolvable, second not.
    let text = "start\n\
            <<<<<<< ours\n\
            same\n\
            =======\n\
            same\n\
            >>>>>>> theirs\n\
            middle\n\
            <<<<<<< ours\n\
            foo\n\
            =======\n\
            bar\n\
            >>>>>>> theirs\n\
            end\n";
    let result = try_autosolve_merged_text(text);
    // Second conflict is unresolvable → None.
    assert!(result.is_none());
}

#[test]
fn autosolve_multiple_resolvable_conflicts() {
    let text = "a\n\
            <<<<<<< ours\n\
            X\n\
            =======\n\
            X\n\
            >>>>>>> theirs\n\
            b\n\
            <<<<<<< ours\n\
            Y Y\n\
            =======\n\
            Y  Y\n\
            >>>>>>> theirs\n\
            c\n";
    let result = try_autosolve_merged_text(text);
    // First: identical sides. Second: whitespace-only (picks ours "Y Y").
    assert_eq!(result, Some("a\nX\nb\nY Y\nc\n".to_string()));
}

#[test]
fn autosolve_preserves_context_between_conflicts() {
    let text = "line1\nline2\n\
            <<<<<<< ours\n\
            same\n\
            =======\n\
            same\n\
            >>>>>>> theirs\n\
            line3\nline4\nline5\n";
    let result = try_autosolve_merged_text(text);
    assert_eq!(
        result,
        Some("line1\nline2\nsame\nline3\nline4\nline5\n".to_string())
    );
}

// -----------------------------------------------------------------------
// CRLF preservation through subchunk splitting path
// -----------------------------------------------------------------------

#[test]
fn subchunk_split_preserves_crlf_per_line_merge() {
    // CRLF content going through per_line_merge must preserve \r\n endings.
    let base = "aaa\r\nbbb\r\nccc\r\n";
    let ours = "aaa\r\nBBB\r\nccc\r\n";
    let theirs = "AAA\r\nbbb\r\nccc\r\n";
    let subchunks = split_conflict_into_subchunks(base, ours, theirs).unwrap();

    // Both changes are non-overlapping: ours changed line 2, theirs changed line 1.
    // Should fully resolve.
    assert!(
        subchunks.iter().all(|c| matches!(c, Subchunk::Resolved(_))),
        "expected all resolved subchunks, got: {subchunks:?}"
    );

    let merged: String = subchunks
        .iter()
        .map(|c| match c {
            Subchunk::Resolved(t) => t.as_str(),
            _ => unreachable!(),
        })
        .collect();

    // The reconstructed text must preserve CRLF endings.
    assert_eq!(merged, "AAA\r\nBBB\r\nccc\r\n");
    assert!(
        !merged.contains("\r\n") || merged.matches("\r\n").count() == merged.matches('\n').count(),
        "line endings should be consistently CRLF"
    );
}

#[test]
fn subchunk_split_preserves_crlf_diff_based_merge() {
    // CRLF content with different line counts goes through merge_line_hunks.
    let base = "aaa\r\nbbb\r\nccc\r\n";
    let ours = "aaa\r\nBBB\r\nXXX\r\nccc\r\n"; // inserted line
    let theirs = "AAA\r\nbbb\r\nccc\r\n"; // changed first line
    let subchunks = split_conflict_into_subchunks(base, ours, theirs);

    // Should produce some resolved content.
    assert!(
        subchunks.is_some(),
        "expected subchunks from diff-based merge"
    );
    let subchunks = subchunks.unwrap();

    // Collect all text from resolved subchunks.
    for chunk in &subchunks {
        match chunk {
            Subchunk::Resolved(text) => {
                // Every line in resolved subchunks must end with \r\n.
                for line in text.split_inclusive('\n') {
                    if !line.is_empty() {
                        assert!(
                            line.ends_with("\r\n"),
                            "resolved line should end with CRLF, got: {line:?}"
                        );
                    }
                }
            }
            Subchunk::Conflict { base, ours, theirs } => {
                // Conflict subchunk content should also preserve CRLF.
                for (label, text) in [("base", base), ("ours", ours), ("theirs", theirs)] {
                    for line in text.split_inclusive('\n') {
                        if !line.is_empty() {
                            assert!(
                                line.ends_with("\r\n"),
                                "{label} conflict line should end with CRLF, got: {line:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn autosolve_diff3_subchunk_split_preserves_crlf() {
    // Full autosolve pipeline with CRLF content and diff3 markers.
    let text = "ctx\r\n\
            <<<<<<< ours\r\n\
            aaa\r\n\
            BBB\r\n\
            ccc\r\n\
            ||||||| base\r\n\
            aaa\r\n\
            bbb\r\n\
            ccc\r\n\
            =======\r\n\
            AAA\r\n\
            bbb\r\n\
            ccc\r\n\
            >>>>>>> theirs\r\n\
            end\r\n";
    let result = try_autosolve_merged_text(text);
    // Ours changed line 2, theirs changed line 1 → subchunk merge.
    // Result must preserve CRLF line endings.
    assert_eq!(
        result,
        Some("ctx\r\nAAA\r\nBBB\r\nccc\r\nend\r\n".to_string())
    );
}

#[test]
fn autosolve_crlf_identical_sides_preserves_endings() {
    // Identical sides with CRLF — simplest auto-resolve path.
    let text = "before\r\n\
            <<<<<<< ours\r\n\
            same\r\n\
            =======\r\n\
            same\r\n\
            >>>>>>> theirs\r\n\
            after\r\n";
    let result = try_autosolve_merged_text(text);
    assert_eq!(result, Some("before\r\nsame\r\nafter\r\n".to_string()));
}

#[test]
fn autosolve_crlf_whitespace_only_diff_preserves_endings() {
    // Whitespace-only difference with CRLF — picks ours.
    let text = "start\r\n\
            <<<<<<< ours\r\n\
            foo  bar\r\n\
            =======\r\n\
            foo bar\r\n\
            >>>>>>> theirs\r\n\
            end\r\n";
    let result = try_autosolve_merged_text(text);
    assert_eq!(result, Some("start\r\nfoo  bar\r\nend\r\n".to_string()));
}

#[test]
fn detect_subchunk_line_ending_crlf_dominant() {
    assert_eq!(
        detect_line_ending_from_texts(
            ["a\r\nb\r\n", "c\r\n"],
            LineEndingDetectionMode::DominantCrlfVsLf
        ),
        "\r\n"
    );
}

#[test]
fn detect_subchunk_line_ending_lf_dominant() {
    assert_eq!(
        detect_line_ending_from_texts(["a\nb\n", "c\n"], LineEndingDetectionMode::DominantCrlfVsLf),
        "\n"
    );
}

#[test]
fn detect_subchunk_line_ending_mixed_prefers_majority() {
    // 2 CRLF vs 1 LF → CRLF wins.
    assert_eq!(
        detect_line_ending_from_texts(["a\r\nb\r\nc\n"], LineEndingDetectionMode::DominantCrlfVsLf),
        "\r\n"
    );
    // 1 CRLF vs 2 LF → LF wins.
    assert_eq!(
        detect_line_ending_from_texts(["a\r\nb\nc\n"], LineEndingDetectionMode::DominantCrlfVsLf),
        "\n"
    );
}

#[test]
fn detect_subchunk_line_ending_empty_defaults_to_lf() {
    assert_eq!(
        detect_line_ending_from_texts([""], LineEndingDetectionMode::DominantCrlfVsLf),
        "\n"
    );
    assert_eq!(
        detect_line_ending_from_texts([], LineEndingDetectionMode::DominantCrlfVsLf),
        "\n"
    );
}

// ── parse_conflict_marker_segments malformed-marker preservation tests ──────

#[test]
fn autosolve_malformed_no_separator_preserves_all_content() {
    // A <<<<<<< marker with no matching ======= — all consumed
    // content should be preserved as context text in the output.
    let text = "before\n<<<<<<< ours\nours line 1\nours line 2\n";
    let result = try_autosolve_merged_text(text);
    // No valid conflicts to resolve, so the function returns
    // Some(original) since there are zero conflicts.
    assert_eq!(result.as_deref(), Some(text));
}

#[test]
fn autosolve_malformed_no_end_marker_preserves_all_content() {
    // A <<<<<<< and ======= but no matching >>>>>>> — all consumed
    // content should be preserved as context text in the output.
    let text = "before\n<<<<<<< ours\nours line\n=======\ntheirs line\n";
    let result = try_autosolve_merged_text(text);
    assert_eq!(result.as_deref(), Some(text));
}

#[test]
fn autosolve_malformed_diff3_no_separator_preserves_base_content() {
    // A <<<<<<< marker followed by ||||||| base section but no =======.
    let text = "before\n<<<<<<< ours\nours line\n||||||| base\nbase line\n";
    let result = try_autosolve_merged_text(text);
    assert_eq!(result.as_deref(), Some(text));
}

#[test]
fn autosolve_malformed_diff3_no_end_preserves_all_sections() {
    // A full diff3 conflict header (<<<, |||, ===) but no >>>>>>>.
    let text = "before\n<<<<<<< ours\nours\n||||||| base\nbase\n=======\ntheirs\n";
    let result = try_autosolve_merged_text(text);
    assert_eq!(result.as_deref(), Some(text));
}
