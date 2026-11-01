use super::{
    ConflictResolverUiState, ConflictRowSelection, DeferredLineStarts, DiffWhitespaceMode,
    Loadable, ThreeWayColumn, ThreeWaySides, preview_render_image_bounds,
};
use crate::view::conflict_resolver::{
    self, ConflictBlock, ConflictChoice, ConflictNavTarget, ConflictNavTargetId,
    ConflictResolverViewMode, ConflictSegment, ConflictSplitRowIndex, ResolvedLineMeta,
    ResolvedLineSource, ThreeWayVisibleItem, TwoWaySplitProjection,
};

#[test]
fn preview_scale_down_compares_logical_image_dimensions() {
    let bounds = gpui::Bounds {
        origin: gpui::point(gpui::px(0.0), gpui::px(0.0)),
        size: gpui::size(gpui::px(100.0), gpui::px(100.0)),
    };
    let image_size = gpui::size(gpui::DevicePixels(16), gpui::DevicePixels(16));

    let scale_down = preview_render_image_bounds(bounds, image_size, 2.0, true);
    assert_eq!(scale_down.size, gpui::size(gpui::px(8.0), gpui::px(8.0)));
    assert_eq!(
        scale_down.origin,
        gpui::point(gpui::px(46.0), gpui::px(46.0))
    );

    let contain = preview_render_image_bounds(bounds, image_size, 2.0, false);
    assert_eq!(contain.size, bounds.size);
}

#[test]
pub(crate) fn default_groups_three_way_side_fields() {
    let state = ConflictResolverUiState::default();

    assert!(state.three_way_text.base.is_empty());
    assert!(state.three_way_text.ours.is_empty());
    assert!(state.three_way_text.theirs.is_empty());
    assert!(state.rendering_mode().is_streamed_large_file());
    assert!(state.three_way_line_starts.base.is_empty());
    assert!(state.three_way_line_starts.ours.is_empty());
    assert!(state.three_way_line_starts.theirs.is_empty());
    assert!(state.three_way_conflict_ranges.base.is_empty());
    assert!(state.three_way_word_highlights.base.is_empty());
    assert!(state.split_row_index().is_some());
    assert!(state.two_way_split_projection().is_some());
    assert!(matches!(
        state.markdown_preview.documents.base,
        Loadable::NotLoaded
    ));
}

#[test]
pub(crate) fn three_way_sides_keep_each_column_separate() {
    let mut sides = ThreeWaySides {
        base: vec![1],
        ours: vec![2],
        theirs: vec![3],
    };

    sides.base.push(10);
    sides.ours.push(20);
    sides.theirs.push(30);

    assert_eq!(sides.base, vec![1, 10]);
    assert_eq!(sides.ours, vec![2, 20]);
    assert_eq!(sides.theirs, vec![3, 30]);
}

#[test]
pub(crate) fn three_way_sides_index_by_column() {
    let mut sides = ThreeWaySides {
        base: 10,
        ours: 20,
        theirs: 30,
    };

    assert_eq!(sides[ThreeWayColumn::Base], 10);
    assert_eq!(sides[ThreeWayColumn::Ours], 20);
    assert_eq!(sides[ThreeWayColumn::Theirs], 30);

    sides[ThreeWayColumn::Ours] = 42;
    assert_eq!(sides.ours, 42);
}

#[test]
pub(crate) fn source_line_text_for_choice_reads_two_way_inputs_from_indexed_text() {
    let mut state = ConflictResolverUiState {
        view_mode: ConflictResolverViewMode::TwoWayDiff,
        ..Default::default()
    };
    state.three_way_text.ours = "o0\no1\n".into();
    state.three_way_text.theirs = "t0\nt1\n".into();
    state.three_way_line_starts.ours = vec![0, 3].into();
    state.three_way_line_starts.theirs = vec![0, 3].into();

    assert_eq!(
        state.source_line_text_for_choice(ConflictChoice::Ours, 1),
        Some("o1")
    );
    assert_eq!(
        state.source_line_text_for_choice(ConflictChoice::Theirs, 0),
        Some("t0")
    );
    assert_eq!(
        state.source_line_text_for_choice(ConflictChoice::Base, 0),
        None
    );
    assert_eq!(
        state.source_line_text_for_choice(ConflictChoice::Both, 0),
        None
    );
}

#[test]
pub(crate) fn source_line_text_for_choice_reads_base_only_in_three_way_mode() {
    let mut state = ConflictResolverUiState {
        view_mode: ConflictResolverViewMode::ThreeWay,
        ..Default::default()
    };
    state.three_way_text.base = "b0\nb1\n".into();
    state.three_way_text.ours = "o0\no1\n".into();
    state.three_way_text.theirs = "t0\nt1\n".into();
    state.three_way_line_starts.base = vec![0, 3].into();
    state.three_way_line_starts.ours = vec![0, 3].into();
    state.three_way_line_starts.theirs = vec![0, 3].into();

    assert_eq!(
        state.source_line_text_for_choice(ConflictChoice::Base, 1),
        Some("b1")
    );
    assert_eq!(
        state.source_line_text_for_choice(ConflictChoice::Ours, 0),
        Some("o0")
    );
    assert_eq!(
        state.source_line_text_for_choice(ConflictChoice::Theirs, 1),
        Some("t1")
    );
}

#[test]
pub(crate) fn apply_three_way_conflict_maps_distributes_ranges_and_flags() {
    let mut state = ConflictResolverUiState::default();
    state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: Some("base\n".into()),
        ours: "ours\n".into(),
        theirs: "theirs\n".into(),
        choice: ConflictChoice::Theirs,
        resolved: true,
        whitespace_only: false,
    })];
    let maps = conflict_resolver::ThreeWayConflictMaps {
        conflict_ranges: [vec![0..3], vec![0..5], vec![0..4]],
        line_conflict_maps: [vec![Some(0); 3], vec![Some(0); 5], vec![Some(0); 4]],
        conflict_has_base: vec![true],
        conflict_resolved: vec![true],
    };
    state.apply_three_way_conflict_maps(maps.clone());

    assert_eq!(
        state.three_way_conflict_ranges.base,
        maps.conflict_ranges[0]
    );
    assert_eq!(
        state.three_way_conflict_ranges.ours,
        maps.conflict_ranges[1]
    );
    assert_eq!(
        state.three_way_conflict_ranges.theirs,
        maps.conflict_ranges[2]
    );
    assert_eq!(state.conflict_has_base, maps.conflict_has_base);
    assert_eq!(state.conflict_choices, vec![ConflictChoice::Theirs]);
}

#[test]
pub(crate) fn merge_plan_ranges_override_marker_output_offset_estimates() {
    let block = |ours: &str, theirs: &str| {
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: ours.into(),
            theirs: theirs.into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        })
    };
    let exact_ranges = vec![1..3, 4..5, 6..8];
    let mut state = ConflictResolverUiState {
        // These text segments are merged-output projections whose line
        // counts do not represent positions in both immutable sources.
        marker_segments: vec![
            ConflictSegment::Text("one-sided resolved output\n".into()),
            block("local-a\nlocal-b\n", "remote-a\n"),
            ConflictSegment::Text("another selected-side line\n".into()),
            block("local-c\n", "remote-c\nremote-extra\n"),
            ConflictSegment::Text("selected output before final block\n".into()),
            block("local-d\nlocal-e\n", "remote-d\n"),
        ],
        three_way_len: 9,
        merge_plan_aligned_conflict_ranges: Some(exact_ranges.clone()),
        ..Default::default()
    };

    state.rebuild_three_way_visible_state();

    assert_eq!(state.three_way_conflict_ranges.base, exact_ranges);
    assert_eq!(
        state.three_way_conflict_ranges.ours,
        state.three_way_conflict_ranges.base
    );
    assert_eq!(
        state.three_way_conflict_ranges.theirs,
        state.three_way_conflict_ranges.base
    );
    assert_eq!(
        state.conflict_index_for_side_line(ThreeWayColumn::Ours, 1),
        Some(0)
    );
    assert_eq!(
        state.conflict_index_for_side_line(ThreeWayColumn::Ours, 3),
        None
    );
    assert_eq!(
        state.conflict_index_for_side_line(ThreeWayColumn::Ours, 4),
        Some(1)
    );
    assert_eq!(
        state.conflict_index_for_side_line(ThreeWayColumn::Ours, 7),
        Some(2)
    );
}

#[test]
pub(crate) fn refresh_conflict_has_base_from_segments_refreshes_choice_cache() {
    let mut state = ConflictResolverUiState::default();
    state.marker_segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".into()),
            ours: "ours2\n".into(),
            theirs: "theirs2\n".into(),
            choice: ConflictChoice::Both,
            resolved: true,
            whitespace_only: false,
        }),
    ];

    state.refresh_conflict_has_base_from_segments();

    assert_eq!(state.conflict_has_base, vec![false, true]);
    assert_eq!(
        state.conflict_choices,
        vec![ConflictChoice::Ours, ConflictChoice::Both]
    );
}

#[test]
pub(crate) fn ignored_whitespace_visual_kind_caches_entire_change_run() {
    use gitcomet_core::file_diff::FileDiffRowKind as RK;

    let mut state = ConflictResolverUiState::default();
    state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "let x = 1\nabc  \n".into(),
        theirs: "let x=1\nabc\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
        whitespace_only: false,
    })];
    state.rebuild_two_way_visible_state();

    let first_row = state.two_way_split_row_by_source(0).unwrap();
    assert_eq!(
        state.two_way_split_visual_kind_at(0, &first_row, DiffWhitespaceMode::Ignore),
        RK::Context
    );

    assert_eq!(state.two_way_split_visual_kind_cache.len(), 2);
    assert_eq!(
        state.two_way_split_visual_kind_cache.get(&1).copied(),
        Some(RK::Context)
    );
}

#[test]
pub(crate) fn giant_two_way_word_highlights_are_shared_between_column_renders() {
    let mut state = ConflictResolverUiState::default();
    state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "let local_name = value;\n".into(),
        theirs: "let remote_name = value;\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
        whitespace_only: false,
    })];
    state.rebuild_two_way_visible_state();
    let row = state.two_way_split_row_by_source(0).unwrap();

    let left = state
        .two_way_split_word_highlight_for_row(0, &row)
        .expect("modified row should have word highlights");
    let right = state
        .two_way_split_word_highlight_for_row(0, &row)
        .expect("second column should reuse word highlights");

    assert!(std::sync::Arc::ptr_eq(&left, &right));
}

#[test]
pub(crate) fn rebuild_three_way_visible_state_streamed_mode() {
    let mut state = ConflictResolverUiState::default();
    state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "a\nb\n".into(),
        theirs: "c\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
        whitespace_only: false,
    })];
    state.three_way_text.ours = "a\nb\n".into();
    state.three_way_text.theirs = "c\n".into();
    state.three_way_line_starts.ours = vec![0, 2].into();
    state.three_way_line_starts.theirs = vec![0].into();
    state.three_way_len = 2;

    state.rebuild_three_way_visible_state();

    assert!(!state.streamed().three_way_visible_projection.is_empty());
    assert_eq!(
        state.three_way_visible_len(),
        state.streamed().three_way_visible_projection.len()
    );
    assert!(!state.three_way_conflict_ranges.ours.is_empty());
}

#[test]
pub(crate) fn three_way_measure_rows_do_not_materialize_deferred_line_starts() {
    let mut state = ConflictResolverUiState::default();
    let base_text = "ctx\nbase 1234567890\nend\n";
    let ours_text = "ctx\nours abcdefghij\nend\n";
    let theirs_text = "ctx\ntheirs klmnopqrstuv\nend\n";

    state.marker_segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base 1234567890\n".into()),
            ours: "ours abcdefghij\n".into(),
            theirs: "theirs klmnopqrstuv\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("end\n".into()),
    ];
    state.three_way_text = ThreeWaySides {
        base: base_text.into(),
        ours: ours_text.into(),
        theirs: theirs_text.into(),
    };
    state.three_way_line_starts = ThreeWaySides {
        base: DeferredLineStarts::with_line_count(3),
        ours: DeferredLineStarts::with_line_count(3),
        theirs: DeferredLineStarts::with_line_count(3),
    };
    state.three_way_len = 3;

    state.rebuild_three_way_visible_state();

    assert_eq!(
        state.three_way_horizontal_measure_row(ThreeWayColumn::Base),
        1
    );
    assert_eq!(
        state.three_way_horizontal_measure_row(ThreeWayColumn::Ours),
        1
    );
    assert_eq!(
        state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs),
        1
    );
    assert!(
        !state.three_way_line_starts.base.is_materialized(),
        "base line starts should stay deferred when selecting measure rows"
    );
    assert!(
        !state.three_way_line_starts.ours.is_materialized(),
        "ours line starts should stay deferred when selecting measure rows"
    );
    assert!(
        !state.three_way_line_starts.theirs.is_materialized(),
        "theirs line starts should stay deferred when selecting measure rows"
    );
}

#[test]
pub(crate) fn three_way_measure_rows_use_each_stage_coordinates_when_clean_context_diverges() {
    let base = "ctx\nbase conflict\ntail\n";
    let ours = "ctx\nclean ours insertion\nours conflict\ntail\n";
    let long_theirs = "theirs conflict line that must drive the remote column width";
    let theirs = format!("ctx\n{long_theirs}\ntail\n");
    let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
        &gitcomet_core::merge::align_three_way(
            base,
            ours,
            &theirs,
            gitcomet_core::merge::DiffAlgorithm::Myers,
        ),
    );
    let mut state = ConflictResolverUiState {
        // The clean insertion is present in the merge result's context,
        // but not in the base or remote index stages.
        marker_segments: vec![
            ConflictSegment::Text("ctx\nclean ours insertion\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base conflict\n".into()),
                ours: "ours conflict\n".into(),
                theirs: format!("{long_theirs}\n").into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
            ConflictSegment::Text("tail\n".into()),
        ],
        three_way_text: ThreeWaySides {
            base: base.into(),
            ours: ours.into(),
            theirs: theirs.clone().into(),
        },
        three_way_line_starts: ThreeWaySides {
            base: super::deferred_line_starts_for_text(base).into(),
            ours: super::deferred_line_starts_for_text(ours).into(),
            theirs: super::deferred_line_starts_for_text(&theirs).into(),
        },
        three_way_len: aligned.aligned_len(),
        three_way_aligned: aligned,
        ..Default::default()
    };

    state.rebuild_three_way_visible_state();

    let measure_row = state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs);
    assert_eq!(
        state.three_way_row_text(ThreeWayColumn::Theirs, measure_row),
        Some(long_theirs),
        "remote width measurement must select the widest line in stage :3"
    );
}

#[test]
pub(crate) fn hidden_resolved_measure_row_is_not_remapped_as_a_side_line() {
    let base = "head\nb1\nb2\ntail\nbase widest visible line\n";
    let ours = "head\no1\nours insertion\no2\ntail\nours widest visible line\n";
    let long_theirs = "theirs widest visible line after a collapsed conflict";
    let theirs = format!("head\nt1\nt2\ntail\n{long_theirs}\n");
    let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
        &gitcomet_core::merge::align_three_way(
            base,
            ours,
            &theirs,
            gitcomet_core::merge::DiffAlgorithm::Myers,
        ),
    );
    let mut state = ConflictResolverUiState {
        marker_segments: vec![
            ConflictSegment::Text("head\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("b1\nb2\n".into()),
                ours: "o1\nours insertion\no2\n".into(),
                theirs: "t1\nt2\n".into(),
                choice: ConflictChoice::Ours,
                resolved: true,
                whitespace_only: false,
            }),
            ConflictSegment::Text(format!("tail\n{long_theirs}\n").into()),
        ],
        hide_resolved: true,
        three_way_text: ThreeWaySides {
            base: base.into(),
            ours: ours.into(),
            theirs: theirs.clone().into(),
        },
        three_way_line_starts: ThreeWaySides {
            base: super::deferred_line_starts_for_text(base).into(),
            ours: super::deferred_line_starts_for_text(ours).into(),
            theirs: super::deferred_line_starts_for_text(&theirs).into(),
        },
        three_way_len: aligned.aligned_len(),
        three_way_aligned: aligned,
        ..Default::default()
    };

    state.rebuild_three_way_visible_state();

    let measure_visible_ix = state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs);
    let Some(ThreeWayVisibleItem::Line(aligned_row)) =
        state.three_way_visible_item(measure_visible_ix)
    else {
        panic!("remote measure row should be a visible source line");
    };
    assert_eq!(
        state.three_way_row_text(ThreeWayColumn::Theirs, aligned_row),
        Some(long_theirs),
    );
}

#[test]
pub(crate) fn collapsed_context_measure_row_uses_the_compact_visible_index() {
    let prefix = (0..20)
        .map(|ix| format!("context {ix}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let base = format!("{prefix}base conflict\ntail\n");
    let ours = format!("{prefix}ours conflict\ntail\n");
    let long_theirs = "theirs conflict line wide enough to be the measurement row";
    let theirs = format!("{prefix}{long_theirs}\ntail\n");
    let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
        &gitcomet_core::merge::align_three_way(
            &base,
            &ours,
            &theirs,
            gitcomet_core::merge::DiffAlgorithm::Myers,
        ),
    );
    let mut state = ConflictResolverUiState {
        marker_segments: vec![
            ConflictSegment::Text(prefix.into()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base conflict\n".into()),
                ours: "ours conflict\n".into(),
                theirs: format!("{long_theirs}\n").into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
            ConflictSegment::Text("tail\n".into()),
        ],
        collapse_context: true,
        three_way_text: ThreeWaySides {
            base: base.clone().into(),
            ours: ours.clone().into(),
            theirs: theirs.clone().into(),
        },
        three_way_line_starts: ThreeWaySides {
            base: super::deferred_line_starts_for_text(&base).into(),
            ours: super::deferred_line_starts_for_text(&ours).into(),
            theirs: super::deferred_line_starts_for_text(&theirs).into(),
        },
        three_way_len: aligned.aligned_len(),
        three_way_aligned: aligned,
        ..Default::default()
    };

    state.rebuild_three_way_visible_state();

    let measure_visible_ix = state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs);
    let Some(ThreeWayVisibleItem::Line(aligned_row)) =
        state.three_way_visible_item(measure_visible_ix)
    else {
        panic!("remote measure row should survive context folding");
    };
    assert_eq!(
        state.three_way_row_text(ThreeWayColumn::Theirs, aligned_row),
        Some(long_theirs),
    );
    assert!(
        measure_visible_ix < aligned_row,
        "folded projection should compact the source row index"
    );
}

#[test]
pub(crate) fn streamed_conflict_index_for_side_line_uses_grouped_side_ranges() {
    let mut state = ConflictResolverUiState::default();
    state.three_way_conflict_ranges = ThreeWaySides {
        base: vec![0..1, 4..6],
        ours: vec![2..5, 8..9],
        theirs: vec![1..3, 7..10],
    };

    assert_eq!(
        state.conflict_index_for_side_line(ThreeWayColumn::Base, 4),
        Some(1)
    );
    assert_eq!(
        state.conflict_index_for_side_line(ThreeWayColumn::Ours, 3),
        Some(0)
    );
    assert_eq!(
        state.conflict_index_for_side_line(ThreeWayColumn::Theirs, 8),
        Some(1)
    );
    assert_eq!(
        state.conflict_index_for_side_line(ThreeWayColumn::Base, 2),
        None
    );
}

#[test]
pub(crate) fn streamed_mode_dispatch_uses_projection() {
    let mut state = ConflictResolverUiState::default();
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "a\nb\nc\nd\ne\n".into(),
        theirs: "a\nb\nc\nd\ne\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
        whitespace_only: false,
    })];
    let ranges = vec![0..5];
    state.streamed_mut().three_way_visible_projection =
        conflict_resolver::build_three_way_visible_projection(5, &ranges, &segments, false);

    assert_eq!(state.three_way_visible_len(), 5);
    assert_eq!(
        state.three_way_visible_item(2),
        Some(ThreeWayVisibleItem::Line(2))
    );
}

pub(crate) fn streamed_state_with_one_conflict() -> ConflictResolverUiState {
    let segments = vec![
        ConflictSegment::Text("ctx\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\nb\n".into(),
            theirs: "c\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        }),
    ];
    let index = ConflictSplitRowIndex::new(&segments, 3);
    let projection = TwoWaySplitProjection::new(&index, &segments, false);

    let mut state = ConflictResolverUiState::default();
    state.marker_segments = segments;
    state.mode_state = super::ConflictModeState::Streamed(super::StreamedConflictState {
        split_row_index: index,
        two_way_split_projection: projection,
        ..super::StreamedConflictState::default()
    });
    state
}

#[test]
pub(crate) fn two_way_row_counts_dispatch() {
    let streamed = streamed_state_with_one_conflict();
    let (diff_count, inline_count) = streamed.two_way_row_counts();
    assert!(diff_count > 0);
    assert_eq!(inline_count, 0);
}

#[test]
pub(crate) fn two_way_split_conflict_ix_for_visible_dispatch() {
    let streamed = streamed_state_with_one_conflict();
    let vis_len = streamed.two_way_split_visible_len();
    let mut found_conflict = false;
    for ix in 0..vis_len {
        if streamed.two_way_split_conflict_ix_for_visible(ix) == Some(0) {
            found_conflict = true;
            break;
        }
    }
    assert!(found_conflict);
}

#[test]
pub(crate) fn two_way_split_visible_row_dispatch() {
    let streamed = streamed_state_with_one_conflict();
    let visible_ix = streamed
        .two_way_visible_ix_for_conflict(0)
        .expect("streamed visible row should exist for the unresolved conflict");
    let visible_row = streamed
        .two_way_split_visible_row(visible_ix)
        .expect("streamed visible row should resolve through the projection");
    assert_eq!(visible_row.conflict_ix, Some(0));
    assert!(visible_row.row.old.is_some() || visible_row.row.new.is_some());
    assert!(visible_row.source_row_ix < streamed.two_way_row_counts().0);
}

#[test]
pub(crate) fn two_way_split_nav_entries_dispatch() {
    let streamed = streamed_state_with_one_conflict();
    assert_eq!(streamed.two_way_split_nav_entries().len(), 1);
}

#[test]
pub(crate) fn two_way_nav_entries_uses_split_projection() {
    let streamed = streamed_state_with_one_conflict();
    assert_eq!(streamed.two_way_nav_entries().len(), 1);
}

#[test]
pub(crate) fn two_way_conflict_ix_for_visible_dispatch() {
    let streamed = streamed_state_with_one_conflict();
    let vis_len = streamed.two_way_split_visible_len();
    let mut found = false;
    for ix in 0..vis_len {
        if streamed.two_way_conflict_ix_for_visible(ix) == Some(0) {
            found = true;
            break;
        }
    }
    assert!(found);
}

#[test]
pub(crate) fn two_way_visible_ix_for_conflict_dispatch() {
    let streamed = streamed_state_with_one_conflict();
    assert!(streamed.two_way_visible_ix_for_conflict(0).is_some());
    assert_eq!(streamed.two_way_visible_ix_for_conflict(99), None);
}

#[test]
pub(crate) fn default_mode_state_is_streamed() {
    let state = ConflictResolverUiState::default();
    assert!(state.rendering_mode().is_streamed_large_file());
    assert!(state.split_row_index().is_some());
}

pub(crate) fn split_ready_state() -> ConflictResolverUiState {
    let base = "ctx\nb1\nb2\nb3\ntail\n";
    let ours = "ctx\no1\no2\no3\ntail\n";
    let theirs = "ctx\nt1\nt2\nt3\ntail\n";
    let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
        &gitcomet_core::merge::align_three_way(
            base,
            ours,
            theirs,
            gitcomet_core::merge::DiffAlgorithm::Myers,
        ),
    );
    let mut state = ConflictResolverUiState {
        marker_segments: vec![
            ConflictSegment::Text("ctx\n".into()),
            // Display blocks may have a base populated from the ancestor
            // even though the raw marker block is two-sided.
            ConflictSegment::Block(ConflictBlock {
                base: Some("b1\nb2\nb3\n".into()),
                ours: "o1\no2\no3\n".into(),
                theirs: "t1\nt2\nt3\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
            ConflictSegment::Text("tail\n".into()),
        ],
        conflict_region_indices: vec![0],
        conflict_region_marker_has_base: vec![false],
        strategy: Some(gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver),
        three_way_text: ThreeWaySides {
            base: base.into(),
            ours: ours.into(),
            theirs: theirs.into(),
        },
        three_way_line_starts: ThreeWaySides {
            base: vec![0, 4, 7, 10, 13].into(),
            ours: vec![0, 4, 7, 10, 13].into(),
            theirs: vec![0, 4, 7, 10, 13].into(),
        },
        three_way_len: aligned.aligned_len(),
        three_way_aligned: aligned,
        ..Default::default()
    };
    state.rebuild_three_way_visible_state();
    assert_eq!(state.three_way_block_aligned_range(0), Some(1..4));
    state
}

pub(crate) fn split_ready_state_with_synthetic_base(
    base: &str,
    block_base: &str,
) -> ConflictResolverUiState {
    let ours = "ctx\nshared1\nshared2\ntail\n";
    let theirs = ours;
    let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
        &gitcomet_core::merge::align_three_way(
            base,
            ours,
            theirs,
            gitcomet_core::merge::DiffAlgorithm::Myers,
        ),
    );
    let mut state = ConflictResolverUiState {
        marker_segments: vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                // Synthetic display base populated from the ancestor; the
                // serialized marker remains the ordinary two-marker form.
                base: Some(block_base.to_string().into()),
                ours: "shared1\nshared2\n".into(),
                theirs: "shared1\nshared2\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
            ConflictSegment::Text("tail\n".into()),
        ],
        conflict_region_indices: vec![0],
        conflict_region_marker_has_base: vec![false],
        strategy: Some(gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver),
        three_way_text: ThreeWaySides {
            base: base.to_string().into(),
            ours: ours.into(),
            theirs: theirs.into(),
        },
        three_way_line_starts: ThreeWaySides {
            base: super::deferred_line_starts_for_text(base).into(),
            ours: super::deferred_line_starts_for_text(ours).into(),
            theirs: super::deferred_line_starts_for_text(theirs).into(),
        },
        three_way_len: aligned.aligned_len(),
        three_way_aligned: aligned,
        ..Default::default()
    };
    state.rebuild_three_way_visible_state();
    state
}

#[test]
pub(crate) fn conflict_row_selection_normalizes_and_clamps_to_its_block() {
    let state = split_ready_state();
    let reverse = ConflictRowSelection {
        conflict_ix: 0,
        anchor_row: 3,
        head_row: 1,
        selecting: true,
    };
    assert_eq!(reverse.row_range(), 1..=3);
    assert_eq!(state.clamp_row_to_conflict_block(0, 0), 1);
    assert_eq!(state.clamp_row_to_conflict_block(0, usize::MAX), 3);
}

#[test]
pub(crate) fn alignment_marks_are_independent_per_column_and_extend_from_their_anchor() {
    let mut state = split_ready_state();
    assert!(state.manual_alignment_enabled());
    assert!(!state.has_alignment_selection());

    state.set_alignment_selection(ThreeWayColumn::Ours, 2, false);
    state.set_alignment_selection(ThreeWayColumn::Theirs, 1, false);
    assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 2));
    assert!(state.alignment_line_is_selected(ThreeWayColumn::Theirs, 1));
    assert!(
        !state.alignment_line_is_selected(ThreeWayColumn::Ours, 1),
        "marking one column must not mark the same line in another"
    );

    // Extending backwards from the anchor normalizes the range.
    state.set_alignment_selection(ThreeWayColumn::Ours, 1, true);
    assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 1));
    assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 2));
    assert!(!state.alignment_line_is_selected(ThreeWayColumn::Ours, 3));

    // Without extend the mark restarts at the clicked line.
    state.set_alignment_selection(ThreeWayColumn::Ours, 3, false);
    assert!(!state.alignment_line_is_selected(ThreeWayColumn::Ours, 1));
    assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 3));

    assert!(state.clear_alignment_selections());
    assert!(!state.has_alignment_selection());
    assert!(!state.clear_alignment_selections());
}

#[test]
pub(crate) fn an_unmarked_column_pins_an_empty_range_at_its_aligned_position() {
    let mut state = split_ready_state();
    state.set_alignment_selection(ThreeWayColumn::Ours, 2, false);
    state.set_alignment_selection(ThreeWayColumn::Theirs, 1, false);

    let entry = state
        .manual_alignment_from_selections(true)
        .expect("two marked columns are enough to pin");
    assert_eq!(entry.local, 2..3);
    assert_eq!(entry.remote, 1..2);
    assert!(
        entry.base.is_empty(),
        "the unmarked base column pins nothing, not a guessed range"
    );
    assert_eq!(
        entry.base.start,
        state
            .three_way_aligned
            .side_line_lower_bound(ThreeWayColumn::Base.side_index(), 1),
        "its empty range still sits where the marked columns start"
    );
}

#[test]
pub(crate) fn a_two_input_pin_leaves_the_base_range_at_the_origin() {
    let mut state = split_ready_state();
    state.set_alignment_selection(ThreeWayColumn::Base, 2, false);
    state.set_alignment_selection(ThreeWayColumn::Ours, 2, false);

    let entry = state
        .manual_alignment_from_selections(false)
        .expect("marked columns are enough to pin");
    assert_eq!(
        entry.base,
        0..0,
        "without a base the plan maps ours/theirs onto A/B, so the base range must stay inert"
    );
    assert_eq!(entry.local, 2..3);
}

#[test]
pub(crate) fn nothing_marked_pins_nothing() {
    let state = split_ready_state();
    assert!(state.manual_alignment_from_selections(true).is_none());
}

#[test]
pub(crate) fn a_conflict_without_aligned_rows_cannot_be_pinned() {
    let mut state = ConflictResolverUiState {
        strategy: Some(gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver),
        ..Default::default()
    };
    assert!(
        !state.manual_alignment_enabled(),
        "the identity map has no shared row space to express a pin in"
    );
    state.set_alignment_selection(ThreeWayColumn::Ours, 0, false);
    assert!(state.manual_alignment_from_selections(true).is_none());
}

#[test]
pub(crate) fn split_boundaries_support_forward_reverse_and_single_row_selections() {
    let mut state = split_ready_state();

    state.row_selection = Some(ConflictRowSelection {
        conflict_ix: 0,
        anchor_row: 1,
        head_row: 2,
        selecting: false,
    });
    let (region_index, forward) = state.split_boundaries_for_selection().expect("forward");
    assert_eq!(region_index, 0);
    assert_eq!(forward.ours, [0, 2]);
    assert_eq!(forward.theirs, [0, 2]);
    assert_eq!(
        forward.base, None,
        "raw two-sided markers need no base cuts"
    );

    state.row_selection = Some(ConflictRowSelection {
        conflict_ix: 0,
        anchor_row: 2,
        head_row: 1,
        selecting: false,
    });
    assert_eq!(
        state.split_boundaries_for_selection().unwrap().1,
        forward,
        "reverse drags normalize to the same boundaries",
    );

    state.row_selection = Some(ConflictRowSelection {
        conflict_ix: 0,
        anchor_row: 3,
        head_row: 3,
        selecting: false,
    });
    let single = state
        .split_boundaries_for_selection()
        .expect("single row")
        .1;
    assert_eq!(single.ours, [2, 3]);
    assert_eq!(single.theirs, [2, 3]);
}

#[test]
pub(crate) fn split_boundaries_use_staged_positions_after_one_sided_clean_context() {
    let base = "ctx\nb1\nb2\ntail\n";
    let ours = "ctx\nours clean insertion\no1\no2\ntail\n";
    let theirs = "ctx\nt1\nt2\ntail\n";
    use gitcomet_core::merge::{AlignedRun, AlignedRunKind};
    let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(&[
        AlignedRun {
            base: 0..1,
            ours: 0..1,
            theirs: 0..1,
            kind: AlignedRunKind::Unchanged,
        },
        AlignedRun {
            base: 1..1,
            ours: 1..2,
            theirs: 1..1,
            kind: AlignedRunKind::OursChanged,
        },
        AlignedRun {
            base: 1..3,
            ours: 2..4,
            theirs: 1..3,
            kind: AlignedRunKind::Conflict,
        },
        AlignedRun {
            base: 3..4,
            ours: 4..5,
            theirs: 3..4,
            kind: AlignedRunKind::Unchanged,
        },
    ]);
    let mut state = ConflictResolverUiState {
        marker_segments: vec![
            ConflictSegment::Text("ctx\nours clean insertion\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("b1\nb2\n".into()),
                ours: "o1\no2\n".into(),
                theirs: "t1\nt2\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
            ConflictSegment::Text("tail\n".into()),
        ],
        conflict_region_indices: vec![0],
        conflict_region_marker_has_base: vec![true],
        strategy: Some(gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver),
        three_way_text: ThreeWaySides {
            base: base.into(),
            ours: ours.into(),
            theirs: theirs.into(),
        },
        three_way_line_starts: ThreeWaySides {
            base: super::deferred_line_starts_for_text(base).into(),
            ours: super::deferred_line_starts_for_text(ours).into(),
            theirs: super::deferred_line_starts_for_text(theirs).into(),
        },
        three_way_len: aligned.aligned_len(),
        three_way_aligned: aligned,
        ..Default::default()
    };
    state.rebuild_three_way_visible_state();
    let first_conflict_row = state
        .three_way_block_aligned_range(0)
        .unwrap()
        .find(|&row| {
            state.three_way_row_text(ThreeWayColumn::Base, row) == Some("b1")
                && state.three_way_row_text(ThreeWayColumn::Ours, row) == Some("o1")
                && state.three_way_row_text(ThreeWayColumn::Theirs, row) == Some("t1")
        })
        .expect("first conflict row");
    state.row_selection = Some(ConflictRowSelection {
        conflict_ix: 0,
        anchor_row: first_conflict_row,
        head_row: first_conflict_row,
        selecting: false,
    });

    let boundaries = state.split_boundaries_for_selection().unwrap().1;
    assert_eq!(boundaries.base, Some([0, 1]));
    assert_eq!(boundaries.ours, [0, 1]);
    assert_eq!(boundaries.theirs, [0, 1]);
}

#[test]
pub(crate) fn split_boundaries_reject_whole_block_and_ambiguous_region_maps() {
    let mut state = split_ready_state();
    state.row_selection = Some(ConflictRowSelection {
        conflict_ix: 0,
        anchor_row: 1,
        head_row: 3,
        selecting: false,
    });
    assert!(state.split_boundaries_for_selection().is_none());

    state.row_selection.as_mut().unwrap().head_row = 2;
    state.conflict_region_indices = vec![0, 0];
    assert!(state.split_boundaries_for_selection().is_none());

    state.conflict_region_indices.clear();
    assert!(state.split_boundaries_for_selection().is_none());

    state.conflict_region_indices = vec![1];
    assert!(state.split_boundaries_for_selection().is_none());
}

#[test]
pub(crate) fn split_boundaries_reject_synthetic_base_only_and_serialized_whole_block_selections() {
    let mut interior = split_ready_state_with_synthetic_base(
        "ctx\nshared1\nbase-only\nshared2\ntail\n",
        "shared1\nbase-only\nshared2\n",
    );
    let interior_range = interior.three_way_block_aligned_range(0).unwrap();
    let interior_padding = interior_range
        .clone()
        .find(|&row| {
            interior
                .three_way_side_line_for_row(ThreeWayColumn::Base, row)
                .is_some()
                && interior
                    .three_way_side_line_for_row(ThreeWayColumn::Ours, row)
                    .is_none()
                && interior
                    .three_way_side_line_for_row(ThreeWayColumn::Theirs, row)
                    .is_none()
        })
        .expect("base-only aligned row");
    interior.row_selection = Some(ConflictRowSelection {
        conflict_ix: 0,
        anchor_row: interior_padding,
        head_row: interior_padding,
        selecting: false,
    });
    assert!(
        interior.split_boundaries_for_selection().is_none(),
        "a row absent from every serialized marker side cannot become its own conflict",
    );

    let mut edge = split_ready_state_with_synthetic_base(
        "ctx\nbase-only\nshared1\nshared2\ntail\n",
        "base-only\nshared1\nshared2\n",
    );
    let edge_range = edge.three_way_block_aligned_range(0).unwrap();
    let serialized_rows: Vec<usize> = edge_range
        .clone()
        .filter(|&row| {
            edge.three_way_side_line_for_row(ThreeWayColumn::Ours, row)
                .is_some()
                || edge
                    .three_way_side_line_for_row(ThreeWayColumn::Theirs, row)
                    .is_some()
        })
        .collect();
    edge.row_selection = Some(ConflictRowSelection {
        conflict_ix: 0,
        anchor_row: *serialized_rows.first().expect("serialized row"),
        head_row: *serialized_rows.last().expect("serialized row"),
        selecting: false,
    });
    assert!(
        edge.split_boundaries_for_selection().is_none(),
        "selecting every serialized line remains a degenerate whole-block split",
    );
}

#[test]
pub(crate) fn joinable_context_rejects_marker_looking_text_between_blocks() {
    let block = || {
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        })
    };
    let mut state = ConflictResolverUiState {
        marker_segments: vec![
            block(),
            ConflictSegment::Text("clean context\n".into()),
            block(),
        ],
        ..Default::default()
    };
    assert!(state.conflict_blocks_have_joinable_context(0, 1));
    state.marker_segments[1] = ConflictSegment::Text("<<<<<<< malformed\n".into());
    assert!(!state.conflict_blocks_have_joinable_context(0, 1));
    assert!(!state.conflict_blocks_have_joinable_context(0, 2));
}

#[test]
pub(crate) fn semantic_selection_retains_automatic_target_when_no_marker_block_exists() {
    let automatic_id = ConflictNavTargetId::PlanBlock(gitcomet_core::merge::MergeBlockId {
        fingerprint: 1,
        occurrence: 0,
    });
    let conflict_id = ConflictNavTargetId::PlanBlock(gitcomet_core::merge::MergeBlockId {
        fingerprint: 2,
        occurrence: 0,
    });
    let mut state = ConflictResolverUiState {
        conflict_region_indices: vec![0],
        nav_targets: vec![
            ConflictNavTarget {
                id: automatic_id,
                order: 0,
                aligned_rows: Some(1..2),
                region_index: None,
                display_conflict_index: None,
                is_delta: true,
                original_conflict: false,
                unresolved: false,
            },
            ConflictNavTarget {
                id: conflict_id,
                order: 1,
                aligned_rows: Some(3..4),
                region_index: Some(0),
                display_conflict_index: Some(0),
                is_delta: true,
                original_conflict: true,
                unresolved: true,
            },
        ],
        ..Default::default()
    };

    assert!(state.select_nav_target(0));
    assert_eq!(state.nav_anchor.unwrap().id, automatic_id);
    assert_eq!(state.selected_nav_target_index(), Some(0));
    assert_eq!(state.active_conflict, None);

    assert!(state.select_display_conflict(0));
    assert_eq!(state.nav_anchor.unwrap().id, conflict_id);
    assert_eq!(state.active_conflict, Some(0));
}

#[test]
pub(crate) fn exact_provenance_projects_target_rows_after_output_line_shifts() {
    let target = ConflictNavTarget {
        id: ConflictNavTargetId::DisplayBlock(0),
        order: 0,
        aligned_rows: Some(2..4),
        region_index: None,
        display_conflict_index: None,
        is_delta: true,
        original_conflict: false,
        unresolved: false,
    };
    let anchor = target.anchor();
    let mut state = ConflictResolverUiState {
        view_mode: ConflictResolverViewMode::ThreeWay,
        resolved_outline: super::ResolvedOutlineData {
            meta: vec![ResolvedLineMeta {
                output_line: 5,
                source: ResolvedLineSource::B,
                input_line: Some(3),
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    assert_eq!(
        state.output_line_for_nav_target_provenance(&target),
        Some(5)
    );
    state.resolved_outline.meta[0].output_line = 11;
    assert_eq!(
        state.output_line_for_nav_target_provenance(&target),
        Some(11),
        "surrounding output insertions shift only the projection"
    );
    assert_eq!(target.anchor(), anchor, "the semantic anchor is unchanged");
}

#[test]
pub(crate) fn deletion_and_untraceable_manual_output_have_no_output_projection() {
    let deletion = ConflictNavTarget {
        id: ConflictNavTargetId::DisplayBlock(0),
        order: 0,
        aligned_rows: Some(8..9),
        region_index: None,
        display_conflict_index: None,
        is_delta: true,
        original_conflict: false,
        unresolved: false,
    };
    let state = ConflictResolverUiState {
        resolved_outline: super::ResolvedOutlineData {
            meta: vec![ResolvedLineMeta {
                output_line: 3,
                source: ResolvedLineSource::Manual,
                input_line: None,
            }],
            ..Default::default()
        },
        ..Default::default()
    };

    assert_eq!(state.output_line_for_nav_target_provenance(&deletion), None);
}
