use super::*;

#[test]
fn history_columns_available_width_reserves_scrollbar_gutter() {
    let gutter = history_scrollbar_gutter();
    assert_eq!(
        history_columns_available_width(px(200.0)),
        px(200.0) - gutter
    );
    assert_eq!(history_columns_available_width(gutter), px(0.0));
}

#[test]
fn history_column_drag_clamp_respects_static_maximums() {
    let available = history_columns_available_width(px(1436.0));
    let layout = all_columns_visible_drag_layout();
    let next = history_column_drag_clamped_width(
        HistoryColResizeHandle::Branch,
        px(900.0),
        available,
        layout,
        100,
    );
    assert_eq!(next, px(HISTORY_COL_BRANCH_MAX_PX));
}

#[test]
fn history_column_drag_clamp_preserves_message_space() {
    let available = history_columns_available_width(px(836.0));
    let layout = all_columns_visible_drag_layout();
    let next = history_column_drag_clamped_width(
        HistoryColResizeHandle::Branch,
        px(500.0),
        available,
        layout,
        100,
    );

    let next_f: f32 = next.into();
    assert!((next_f - 132.0).abs() < 1e-3);
}

#[test]
fn history_column_drag_clamp_never_goes_below_minimum() {
    let available = history_columns_available_width(px(1436.0));
    let layout = all_columns_visible_drag_layout();
    let next = history_column_drag_clamped_width(
        HistoryColResizeHandle::Sha,
        px(0.0),
        available,
        layout,
        100,
    );
    assert_eq!(next, px(HISTORY_COL_SHA_MIN_PX));
}

#[test]
fn history_column_widths_recompute_from_design_units_with_ui_scale_percent() {
    let widths = scaled_history_column_widths(
        default_history_column_design_widths(),
        ui_scale::UiScale::from_percent(200),
    );
    assert_eq!(
        widths,
        HistoryColumnWidths {
            branch: px(HISTORY_COL_BRANCH_PX * 2.0),
            graph: px(HISTORY_COL_GRAPH_PX * 2.0),
            author: px(HISTORY_COL_AUTHOR_PX * 2.0),
            date: px(HISTORY_COL_DATE_PX * 2.0),
            sha: px(HISTORY_COL_SHA_PX * 2.0),
        }
    );
}

#[test]
fn graph_drag_ignores_auto_hidden_optional_columns() {
    let available = history_columns_available_width(px(500.0));
    let widths = default_history_column_widths(100);
    let preferred = (true, true, true);

    assert_eq!(
        history_visible_columns_for_width(available, true, preferred, widths, 100),
        (false, false, false)
    );

    let next = history_column_drag_next_width(
        HistoryColResizeHandle::Graph,
        px(90.0),
        available,
        true,
        preferred,
        widths,
        100,
    );

    assert_eq!(next, px(90.0));
}

#[test]
fn reset_widths_clamp_default_graph_in_narrow_windows() {
    let widths = history_reset_widths_for_available_width(
        history_columns_available_width(px(396.0)),
        true,
        (true, true, true),
        100,
    );

    assert_eq!(widths.branch, px(116.0));
    assert_eq!(widths.graph, px(HISTORY_COL_GRAPH_MIN_PX));
}

#[test]
fn reset_widths_clamp_branch_after_graph_reaches_minimum() {
    let widths = history_reset_widths_for_available_width(
        history_columns_available_width(px(360.0)),
        true,
        (true, true, true),
        100,
    );

    assert_eq!(widths.graph, px(HISTORY_COL_GRAPH_MIN_PX));
    assert_eq!(widths.branch, px(80.0));
}

#[test]
fn history_resize_state_uses_actual_visible_columns_in_narrow_windows() {
    let available = history_columns_available_width(px(500.0));
    let layout = all_columns_visible_drag_layout();
    let state = history_column_resize_state(
        HistoryColResizeHandle::Graph,
        px(0.0),
        available,
        layout,
        100,
    );

    assert_eq!(
        history_resize_state_visible_columns(available, Some(&state)),
        Some((false, false, false))
    );
}

#[test]
fn history_resize_state_preserves_visible_columns_within_drag_bounds() {
    let available = history_columns_available_width(px(836.0));
    let layout = all_columns_visible_drag_layout();
    let state = history_column_resize_state(
        HistoryColResizeHandle::Graph,
        px(0.0),
        available,
        layout,
        100,
    );

    assert!(history_resize_state_preserves_visible_columns(
        available,
        layout,
        Some(&state)
    ));
    assert_eq!(
        history_visible_columns_for_layout_with_resize_state(available, layout, Some(&state), 100,),
        (true, true, true)
    );
}

#[test]
fn history_resize_state_visibility_fast_path_falls_back_for_out_of_bounds_layout() {
    let available = history_columns_available_width(px(836.0));
    let state = history_column_resize_state(
        HistoryColResizeHandle::Graph,
        px(0.0),
        available,
        all_columns_visible_drag_layout(),
        100,
    );
    let layout = HistoryColumnDragLayout {
        graph_w: px(140.0),
        ..all_columns_visible_drag_layout()
    };

    assert!(!history_resize_state_preserves_visible_columns(
        available,
        layout,
        Some(&state)
    ));
    assert_eq!(
        history_visible_columns_for_layout_with_resize_state(available, layout, Some(&state), 100,),
        history_visible_columns_for_layout(available, layout, 100)
    );
}

#[test]
fn history_resize_state_visible_columns_fast_path_rejects_stale_current_width() {
    let available = history_columns_available_width(px(836.0));
    let layout = all_columns_visible_drag_layout();
    let state = history_column_resize_state(
        HistoryColResizeHandle::Date,
        px(0.0),
        available,
        layout,
        100,
    );

    assert_eq!(
        history_resize_state_visible_columns_for_current_width(
            available,
            px(HISTORY_COL_DATE_PX),
            Some(&state),
        ),
        Some((true, true, true))
    );
    assert_eq!(
        history_resize_state_visible_columns_for_current_width(
            available,
            px(HISTORY_COL_DATE_PX + 1.0),
            Some(&state),
        ),
        None
    );
}
