use super::*;
use gitcomet_core::merge::{DiffAlgorithm, OverviewMode, OverviewRowKind, align_three_way};

fn aligned(base: &str, ours: &str, theirs: &str) -> ThreeWayAlignedMap {
    ThreeWayAlignedMap::from_alignment(&align_three_way(base, ours, theirs, DiffAlgorithm::Myers))
}

/// A pass-through projection: every aligned row is visible, 1:1.
fn flat_projection(len: usize) -> ThreeWayVisibleProjection {
    build_three_way_visible_projection_with_resolved_flags(len, &[], &[], false)
}

#[test]
fn bands_mark_each_side_in_its_own_color() {
    let map = aligned("a\nb\nc\n", "a\nB\nc\n", "a\nb\nC\n");
    let bands = build_overview_bands(
        &map,
        &flat_projection(map.aligned_len()),
        OverviewMode::Merge,
        0,
    );

    assert_eq!(bands.len(), 3);
    assert_eq!(bands[0], OverviewRowKind::Unchanged);
    assert_eq!(bands[1], OverviewRowKind::LocalChanged);
    assert_eq!(bands[2], OverviewRowKind::RemoteChanged);
}

#[test]
fn identity_map_yields_no_bands() {
    // The identity fallback (unaligned/giant files) carries no classification.
    let map = ThreeWayAlignedMap::default();
    let bands = build_overview_bands(&map, &flat_projection(10), OverviewMode::Merge, 0);
    assert!(bands.is_empty());
}

#[test]
fn trailing_overscroll_rows_extend_the_band_range() {
    let map = aligned("a\nb\n", "a\nX\n", "a\nb\n");
    let without = build_overview_bands(
        &map,
        &flat_projection(map.aligned_len()),
        OverviewMode::Merge,
        0,
    );
    let with = build_overview_bands(
        &map,
        &flat_projection(map.aligned_len()),
        OverviewMode::Merge,
        8,
    );

    assert_eq!(without.len(), 2);
    assert_eq!(with.len(), 10);
    // The change stays where it was; the added range is blank.
    assert_eq!(with[1], OverviewRowKind::LocalChanged);
    assert!(
        with[2..]
            .iter()
            .all(|kind| *kind == OverviewRowKind::Unchanged)
    );
}

#[test]
fn pairwise_mode_drops_the_side_it_does_not_compare() {
    let map = aligned("a\nb\nc\n", "a\nB\nc\n", "a\nb\nC\n");
    let projection = flat_projection(map.aligned_len());

    let ab = build_overview_bands(&map, &projection, OverviewMode::BaseVsLocal, 0);
    assert_eq!(ab[1], OverviewRowKind::Conflict);
    assert_eq!(ab[2], OverviewRowKind::Unchanged);

    let ac = build_overview_bands(&map, &projection, OverviewMode::BaseVsRemote, 0);
    assert_eq!(ac[1], OverviewRowKind::Unchanged);
    assert_eq!(ac[2], OverviewRowKind::Conflict);
}

#[test]
fn bands_are_capped_and_merge_changes_that_share_one_band() {
    // More rows than bands: every band covers several rows, and a band holding
    // both a local and a remote change reads as a conflict.
    let rows = OVERVIEW_BAND_COUNT * 2;
    let mut base = String::new();
    let mut ours = String::new();
    let mut theirs = String::new();
    for ix in 0..rows {
        base.push_str(&format!("line {ix}\n"));
        // Adjacent local/remote edits land in the same band.
        if ix == 10 {
            ours.push_str("local edit\n");
            theirs.push_str(&format!("line {ix}\n"));
        } else if ix == 11 {
            ours.push_str(&format!("line {ix}\n"));
            theirs.push_str("remote edit\n");
        } else {
            ours.push_str(&format!("line {ix}\n"));
            theirs.push_str(&format!("line {ix}\n"));
        }
    }

    let map = aligned(&base, &ours, &theirs);
    let bands = build_overview_bands(
        &map,
        &flat_projection(map.aligned_len()),
        OverviewMode::Merge,
        0,
    );

    assert_eq!(bands.len(), OVERVIEW_BAND_COUNT);
    let band = 10 * OVERVIEW_BAND_COUNT / map.aligned_len();
    assert_eq!(bands[band], OverviewRowKind::Conflict);
}

#[test]
fn hidden_rows_fold_into_their_summary_band() {
    // Hide-resolved collapses a resolved conflict to one row; the overview has
    // to follow the same visible space or the frame drifts from the panes.
    let map = aligned("a\nb\nc\nd\ne\n", "a\nB\nC\nd\nE\n", "a\nb\nc\nd\ne\n");
    let ranges = vec![1..3];
    let projection = build_three_way_visible_projection_with_resolved_flags(
        map.aligned_len(),
        &ranges,
        &[true],
        true,
    );

    // The two-row conflict collapses to one summary row.
    assert_eq!(map.aligned_len(), 5);
    assert_eq!(projection.len(), 4);

    let bands = build_overview_bands(&map, &projection, OverviewMode::Merge, 0);
    assert_eq!(bands.len(), projection.len());
    assert_eq!(bands[0], OverviewRowKind::Unchanged);
    // The folded rows' change lands on the summary row's band...
    assert_eq!(bands[1], OverviewRowKind::LocalChanged);
    assert_eq!(bands[2], OverviewRowKind::Unchanged);
    // ...and the change below the fold shifts up with the rows it belongs to.
    assert_eq!(bands[3], OverviewRowKind::LocalChanged);
}
