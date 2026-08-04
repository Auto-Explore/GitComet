use super::*;
use gitcomet_core::conflict_session::{ConflictPayload, ConflictRegionResolution, ConflictSession};
use gitcomet_core::domain::FileConflictKind;
use gitcomet_core::merge::{MergeBlockId, MergeSource};
use std::path::PathBuf;

fn text_payload(text: &str) -> ConflictPayload {
    ConflictPayload::Text(text.into())
}

fn session_with_automatic_deltas_between_conflicts() -> ConflictSession {
    ConflictSession::from_stage_inputs(
        PathBuf::from("file.txt"),
        FileConflictKind::BothModified,
        text_payload(
            "start\nold-a\nsep-1\nold-conflict-1\nsep-2\nold-b\nsep-3\nold-conflict-2\nend\n",
        ),
        text_payload(
            "start\nnew-a\nsep-1\nours-conflict-1\nsep-2\nold-b\nsep-3\nours-conflict-2\nend\n",
        ),
        text_payload(
            "start\nold-a\nsep-1\ntheirs-conflict-1\nsep-2\nnew-b\nsep-3\ntheirs-conflict-2\nend\n",
        ),
    )
}

fn displayed_session_targets(
    session: &ConflictSession,
) -> (Vec<ConflictSegment>, Vec<ConflictNavTarget>) {
    let current = session
        .marker_projection_text()
        .expect("plan-backed session should retain its marker projection");
    let mut segments = parse_conflict_markers(current);
    let applied = apply_session_region_resolutions_with_index_map(&mut segments, &session.regions);
    let region_ranges = conflict_nav_region_aligned_ranges(session, &[]);
    let display_plan_blocks = session
        .merge_plan
        .as_ref()
        .expect("plan")
        .unresolved_blocks
        .clone();
    let display_ranges = merge_plan_aligned_conflict_ranges(
        session,
        &applied.block_region_indices,
        &display_plan_blocks,
    )
    .expect("plan-backed display ranges");
    let display_ranges: Vec<_> = display_ranges.into_iter().map(Some).collect();
    let targets = build_conflict_nav_targets(
        Some(session),
        &region_ranges,
        &applied.block_region_indices,
        &display_ranges,
        &segments,
    );
    (segments, targets)
}

fn target(
    id: ConflictNavTargetId,
    order: usize,
    rows: Option<std::ops::Range<usize>>,
    region_index: Option<usize>,
    is_delta: bool,
    original_conflict: bool,
    unresolved: bool,
) -> ConflictNavTarget {
    ConflictNavTarget {
        id,
        order,
        aligned_rows: rows,
        region_index,
        display_conflict_index: None,
        is_delta,
        original_conflict,
        unresolved,
    }
}

#[test]
fn plan_targets_include_automatic_deltas_in_source_order() {
    let session = session_with_automatic_deltas_between_conflicts();
    let (_, targets) = displayed_session_targets(&session);

    assert_eq!(targets.len(), 4);
    assert_eq!(
        targets
            .iter()
            .map(|target| (
                target.is_delta,
                target.original_conflict,
                target.unresolved,
                target.display_conflict_index,
            ))
            .collect::<Vec<_>>(),
        vec![
            (true, false, false, None),
            (true, true, true, Some(0)),
            (true, false, false, None),
            (true, true, true, Some(1)),
        ]
    );
    assert!(targets.windows(2).all(|pair| {
        pair[0].order < pair[1].order
            && pair[0].aligned_rows.as_ref().unwrap().end
                <= pair[1].aligned_rows.as_ref().unwrap().start
    }));
}

#[test]
fn filtered_navigation_and_availability_share_the_semantic_anchor() {
    let session = session_with_automatic_deltas_between_conflicts();
    let (_, targets) = displayed_session_targets(&session);

    let fresh = fresh_conflict_nav_target_index(&targets).unwrap();
    assert_eq!(fresh, 1, "fresh open selects the first unresolved target");
    let conflict_anchor = Some(targets[fresh].anchor());
    assert_eq!(
        previous_conflict_nav_target_index(
            &targets,
            conflict_anchor,
            ConflictNavTargetFilter::Delta,
        ),
        Some(0)
    );
    assert_eq!(
        next_conflict_nav_target_index(
            &targets,
            conflict_anchor,
            ConflictNavTargetFilter::OriginalConflict,
        ),
        Some(3)
    );

    let automatic_anchor = Some(targets[0].anchor());
    assert_eq!(targets[0].display_conflict_index, None);
    assert_eq!(
        next_conflict_nav_target_index(
            &targets,
            automatic_anchor,
            ConflictNavTargetFilter::OriginalConflict,
        ),
        Some(1)
    );
    assert_eq!(
        previous_conflict_nav_target_index(
            &targets,
            automatic_anchor,
            ConflictNavTargetFilter::OriginalConflict,
        ),
        None
    );

    let last_anchor = Some(targets[3].anchor());
    assert_eq!(
        previous_conflict_nav_target_index(&targets, last_anchor, ConflictNavTargetFilter::Delta,),
        Some(2)
    );
    assert_eq!(
        next_conflict_nav_target_index(&targets, last_anchor, ConflictNavTargetFilter::Delta,),
        None
    );
}

#[test]
fn resolved_original_conflicts_remain_conflict_targets_but_not_unresolved_targets() {
    let mut session = session_with_automatic_deltas_between_conflicts();
    assert!(session.replace_region_selection(0, MergeSource::B.into()));
    let (_, targets) = displayed_session_targets(&session);

    assert!(targets[1].original_conflict);
    assert!(!targets[1].unresolved);
    assert_eq!(
        next_conflict_nav_target_index(
            &targets,
            Some(targets[0].anchor()),
            ConflictNavTargetFilter::OriginalConflict,
        ),
        Some(1)
    );
    assert_eq!(
        next_conflict_nav_target_index(
            &targets,
            Some(targets[0].anchor()),
            ConflictNavTargetFilter::Unresolved,
        ),
        Some(3)
    );
}

#[test]
fn materialized_original_conflict_retains_rows_and_semantic_navigation() {
    let mut session = session_with_automatic_deltas_between_conflicts();
    session.regions[0].resolution =
        ConflictRegionResolution::ManualEdit("custom result\n".to_string());
    session.sync_merge_plan_from_regions();

    let (segments, targets) = displayed_session_targets(&session);
    assert_eq!(conflict_count(&segments), 1);
    assert!(targets[1].original_conflict);
    assert!(!targets[1].unresolved);
    assert_eq!(targets[1].display_conflict_index, None);
    assert!(targets[1].aligned_rows.is_some());
    assert_eq!(
        next_conflict_nav_target_index(
            &targets,
            Some(targets[0].anchor()),
            ConflictNavTargetFilter::OriginalConflict,
        ),
        Some(1)
    );
    assert_eq!(fresh_conflict_nav_target_index(&targets), Some(3));
}

#[test]
fn planless_sessions_use_all_regions_and_display_only_is_the_final_fallback() {
    let session = ConflictSession::from_merged_shared_text(
        PathBuf::from("legacy.txt"),
        FileConflictKind::BothModified,
        text_payload("base\n"),
        text_payload("ours\n"),
        text_payload("theirs\n"),
        "<<<<<<< ours\none\n=======\ntwo\n>>>>>>> theirs\nbetween\n<<<<<<< ours\nthree\n=======\nfour\n>>>>>>> theirs\n"
            .into(),
    );
    assert!(session.merge_plan.is_none());
    let segments = parse_conflict_markers(session.marker_projection_text().unwrap());
    let display_region_indices = sequential_conflict_region_indices(&segments);
    let region_ranges = vec![Some(2..4), Some(8..11)];
    let targets = build_conflict_nav_targets(
        Some(&session),
        &region_ranges,
        &display_region_indices,
        &[Some(2..4), Some(8..11)],
        &segments,
    );
    assert_eq!(
        targets.iter().map(|target| target.id).collect::<Vec<_>>(),
        vec![
            ConflictNavTargetId::Region(0),
            ConflictNavTargetId::Region(1)
        ]
    );
    assert!(
        targets
            .iter()
            .all(|target| target.is_delta && target.original_conflict)
    );

    let mut materialized = session.clone();
    materialized.regions[0].resolution =
        ConflictRegionResolution::ManualEdit("legacy manual result\n".to_string());
    let mut materialized_segments = parse_conflict_markers(materialized.current_text().unwrap());
    let applied = apply_session_region_resolutions_with_index_map(
        &mut materialized_segments,
        &materialized.regions,
    );
    let materialized_targets = build_conflict_nav_targets(
        Some(&materialized),
        &region_ranges,
        &applied.block_region_indices,
        &[Some(8..11)],
        &materialized_segments,
    );
    assert_eq!(conflict_count(&materialized_segments), 1);
    assert_eq!(materialized_targets[0].display_conflict_index, None);
    assert_eq!(materialized_targets[0].aligned_rows, Some(2..4));
    assert!(!materialized_targets[0].unresolved);
    assert_eq!(
        next_conflict_nav_target_index(
            &materialized_targets,
            Some(materialized_targets[0].anchor()),
            ConflictNavTargetFilter::OriginalConflict,
        ),
        Some(1)
    );

    let fallback = build_conflict_nav_targets(
        None,
        &[],
        &display_region_indices,
        &[Some(2..4), Some(8..11)],
        &segments,
    );
    assert_eq!(
        fallback.iter().map(|target| target.id).collect::<Vec<_>>(),
        vec![
            ConflictNavTargetId::DisplayBlock(0),
            ConflictNavTargetId::DisplayBlock(1),
        ]
    );
}

#[test]
fn anchor_reconciliation_uses_the_documented_precedence() {
    let exact_id = ConflictNavTargetId::PlanBlock(MergeBlockId {
        fingerprint: 10,
        occurrence: 0,
    });
    let previous = vec![target(exact_id, 2, Some(20..30), Some(4), true, true, true)];
    let exact_targets = vec![
        target(
            ConflictNavTargetId::Region(8),
            0,
            Some(20..30),
            Some(8),
            true,
            true,
            true,
        ),
        target(exact_id, 1, Some(50..60), Some(4), true, true, true),
    ];
    let anchor = ConflictNavAnchor {
        id: exact_id,
        order_hint: 0,
        aligned_row_hint: Some(22),
    };
    assert_eq!(
        reconcile_conflict_nav_target_index(Some(anchor), &previous, &exact_targets),
        Some(1),
        "exact identity wins over the row hint"
    );

    let region_anchor = target(
        ConflictNavTargetId::Region(4),
        0,
        Some(20..30),
        Some(4),
        true,
        true,
        true,
    )
    .anchor();
    assert_eq!(
        reconcile_conflict_nav_target_index(Some(region_anchor), &[], &exact_targets),
        Some(1),
        "region-to-plan mapping is the second choice"
    );

    let row_anchor = ConflictNavAnchor {
        id: ConflictNavTargetId::DisplayBlock(99),
        order_hint: 0,
        aligned_row_hint: Some(22),
    };
    assert_eq!(
        reconcile_conflict_nav_target_index(Some(row_anchor), &[], &exact_targets),
        Some(0),
        "containing aligned rows are the third choice"
    );

    let order_anchor = ConflictNavAnchor {
        id: ConflictNavTargetId::DisplayBlock(99),
        order_hint: 99,
        aligned_row_hint: None,
    };
    assert_eq!(
        reconcile_conflict_nav_target_index(Some(order_anchor), &[], &exact_targets),
        Some(1),
        "the clamped nearest order is the final reconciliation fallback"
    );
}

#[test]
fn fresh_target_priority_is_unresolved_then_original_then_delta() {
    let targets = vec![
        target(
            ConflictNavTargetId::DisplayBlock(0),
            0,
            Some(0..1),
            None,
            true,
            false,
            false,
        ),
        target(
            ConflictNavTargetId::DisplayBlock(1),
            1,
            Some(1..2),
            None,
            true,
            true,
            false,
        ),
        target(
            ConflictNavTargetId::DisplayBlock(2),
            2,
            Some(2..3),
            None,
            true,
            true,
            true,
        ),
    ];
    assert_eq!(fresh_conflict_nav_target_index(&targets), Some(2));

    let resolved: Vec<_> = targets
        .iter()
        .cloned()
        .map(|mut target| {
            target.unresolved = false;
            target
        })
        .collect();
    assert_eq!(fresh_conflict_nav_target_index(&resolved), Some(1));

    let deltas: Vec<_> = resolved
        .iter()
        .cloned()
        .map(|mut target| {
            target.original_conflict = false;
            target
        })
        .collect();
    assert_eq!(fresh_conflict_nav_target_index(&deltas), Some(0));
}
