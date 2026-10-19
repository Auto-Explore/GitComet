use super::super::tests::test_bounds;
use super::*;
use gpui::{point, px};

#[test]
fn diff_row_text_regions_single_only_hits_inside_text() {
    let regions =
        DiffRowTextRegions::single(DiffTextRegion::Inline, test_bounds(5.0, 5.0, 20.0, 10.0));

    assert_eq!(
        regions.region_at(point(px(10.0), px(10.0))),
        Some(DiffTextRegion::Inline)
    );
    assert_eq!(regions.region_at(point(px(1.0), px(10.0))), None);
}

#[test]
fn diff_row_text_regions_split_maps_left_and_right_regions() {
    let regions = DiffRowTextRegions::split(
        test_bounds(0.0, 0.0, 40.0, 20.0),
        test_bounds(41.0, 0.0, 40.0, 20.0),
    );

    assert_eq!(
        regions.region_at(point(px(10.0), px(10.0))),
        Some(DiffTextRegion::SplitLeft)
    );
    assert_eq!(
        regions.region_at(point(px(60.0), px(10.0))),
        Some(DiffTextRegion::SplitRight)
    );
    assert_eq!(regions.region_at(point(px(40.5), px(10.0))), None);
}
