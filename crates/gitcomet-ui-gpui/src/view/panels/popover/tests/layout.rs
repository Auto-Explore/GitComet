use super::*;

#[test]
fn popover_width_spec_scales_with_zoom() {
    let spec = popover_width_spec(&PopoverKind::RepoPicker).expect("repo picker width");
    let default_scale = ui_scale::UiScale::from_percent(100);
    let zoomed_scale = ui_scale::UiScale::from_percent(200);

    assert_eq!(spec.preferred_px(default_scale), px(420.0));
    assert_eq!(spec.preferred_px(zoomed_scale), px(840.0));
    assert_eq!(spec.max_px(zoomed_scale), px(1640.0));
}

#[test]
fn choose_popover_anchor_corner_prefers_side_with_more_space() {
    assert_eq!(
        choose_popover_anchor_corner(Corner::TopRight, px(260.0), px(640.0), px(420.0),),
        Corner::TopLeft,
    );
    assert_eq!(
        choose_popover_anchor_corner(Corner::BottomLeft, px(500.0), px(260.0), px(420.0),),
        Corner::BottomRight,
    );
}
