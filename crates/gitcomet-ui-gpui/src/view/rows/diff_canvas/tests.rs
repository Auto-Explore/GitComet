use super::*;

fn rgba(r: f32, g: f32, b: f32) -> gpui::Rgba {
    gpui::Rgba::new(r, g, b, 1.0)
}

/// `gpui` shapes a line by splitting the text at each run boundary, so a
/// run that ends inside a multi-byte character aborts the process in
/// `str::split_at`. This pins the diff canvas to the shared guard: without
/// it, a highlight pointing into a character reaches `shape_line` as a
/// run length that splits it.
#[test]
fn compute_runs_never_splits_a_multibyte_char() {
    let text = "— dash — end";
    let style = TextStyle::default();
    let bold = HighlightStyle {
        font_weight: Some(gpui::FontWeight::BOLD),
        ..HighlightStyle::default()
    };

    for highlights in [
        // Inside the leading em dash, from both sides.
        vec![(0..1, bold)],
        vec![(1..3, bold)],
        vec![(2..4, bold)],
        // Inside the second em dash, after valid text.
        vec![(0..2, bold), (7..9, bold)],
        // Past the end, overlapping, and out of order.
        vec![(5..99, bold)],
        vec![(0..6, bold), (2..4, bold)],
        vec![(6..9, bold), (0..3, bold)],
        vec![],
    ] {
        let runs = compute_runs(text, &style, &highlights);
        let total: usize = runs.iter().map(|run| run.len).sum();
        assert_eq!(
            total,
            text.len(),
            "runs must tile the text for {highlights:?}"
        );

        let mut rest = text;
        for run in &runs {
            assert!(
                rest.is_char_boundary(run.len),
                "run of {} bytes splits a character in {rest:?} for {highlights:?}",
                run.len
            );
            rest = &rest[run.len..];
        }
    }
}

pub(super) fn test_bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

#[test]
fn row_bg_fill_bounds_overdraws_bottom_without_changing_origin_or_width() {
    let bounds = test_bounds(4.0, 8.0, 120.0, 20.0);
    let painted = row_bg_fill_bounds(bounds);

    assert_eq!(painted.origin, bounds.origin);
    assert_eq!(painted.size.width, bounds.size.width);
    assert_eq!(
        painted.size.height,
        bounds.size.height + px(DIFF_ROW_BACKGROUND_OVERDRAW_PX)
    );
}

#[test]
fn inline_row_canvas_revision_key_tracks_rendered_payload() {
    let base = inline_row_canvas_revision_key(
        &"1".into(),
        &"2".into(),
        rgba(0.0, 0.0, 0.0),
        rgba(1.0, 1.0, 1.0),
        rgba(1.0, 1.0, 1.0),
        11,
        17,
    );

    assert_eq!(
        base,
        inline_row_canvas_revision_key(
            &"1".into(),
            &"2".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            11,
            17,
        )
    );
    assert_ne!(
        base,
        inline_row_canvas_revision_key(
            &"1".into(),
            &"3".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            11,
            17,
        )
    );
    assert_ne!(
        base,
        inline_row_canvas_revision_key(
            &"1".into(),
            &"2".into(),
            rgba(1.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            11,
            17,
        )
    );
    assert_ne!(
        base,
        inline_row_canvas_revision_key(
            &"1".into(),
            &"2".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            12,
            17,
        )
    );
}

#[test]
fn split_row_canvas_revision_key_tracks_both_sides() {
    let base = split_row_canvas_revision_key(
        &"10".into(),
        &"20".into(),
        rgba(0.0, 0.0, 0.0),
        rgba(1.0, 1.0, 1.0),
        rgba(1.0, 1.0, 1.0),
        rgba(0.0, 0.0, 0.0),
        rgba(1.0, 1.0, 1.0),
        rgba(1.0, 1.0, 1.0),
        3,
        5,
        7,
        11,
    );

    assert_ne!(
        base,
        split_row_canvas_revision_key(
            &"10".into(),
            &"20".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            4,
            5,
            7,
            11,
        )
    );
    assert_ne!(
        base,
        split_row_canvas_revision_key(
            &"10".into(),
            &"20".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            3,
            5,
            7,
            11,
        )
    );
    assert_ne!(
        base,
        split_row_canvas_revision_key(
            &"10".into(),
            &"21".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            3,
            5,
            7,
            11,
        )
    );
}

#[test]
fn patch_split_row_canvas_revision_key_tracks_line_number_and_style() {
    let base = patch_split_row_canvas_revision_key(
        &"42".into(),
        rgba(0.0, 0.0, 0.0),
        rgba(1.0, 1.0, 1.0),
        rgba(1.0, 1.0, 1.0),
        13,
        17,
    );

    assert_ne!(
        base,
        patch_split_row_canvas_revision_key(
            &"43".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            13,
            17,
        )
    );
    assert_ne!(
        base,
        patch_split_row_canvas_revision_key(
            &"42".into(),
            rgba(0.0, 1.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            13,
            17,
        )
    );
    assert_ne!(
        base,
        patch_split_row_canvas_revision_key(
            &"42".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            14,
            17,
        )
    );
}
