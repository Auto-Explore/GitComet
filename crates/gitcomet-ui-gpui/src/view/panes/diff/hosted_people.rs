//! The people a hosted diff annotation names, drawn in the comment column
//! the pane makes for them before its line numbers.
use super::*;

/// How wide each person's circle is.
const DIAMETER: f32 = 16.0;
/// How far the second circle sits past the first: they overlap by two fifths.
const STEP: f32 = 0.6;
/// The space between the circles and the "+k" after them.
const GAP: f32 = 0.125;
/// The "+k" text size.
const MORE_TEXT: f32 = 0.625;
/// The room each character of "+k" gets, generous for any UI or editor font.
const MORE_ADVANCE: f32 = 0.7;
/// The space on each side of the widest glyph in the column.
const COLUMN_PADDING: f32 = 4.0;

/// How many people draw as circles; the rest count in the "+k".
const SHOWN: usize = 2;

fn more_text(diameter: Pixels) -> Pixels {
    diameter * MORE_TEXT
}

fn more_width(diameter: Pixels, extra: usize) -> Pixels {
    let chars = format!("+{extra}").len();
    more_text(diameter) * MORE_ADVANCE * chars as f32
}

/// How wide the glyph for `people` is with circles `diameter` across.
pub(super) fn people_glyph_width(diameter: Pixels, people: usize) -> Pixels {
    let shown = people.min(SHOWN);
    if shown == 0 {
        return px(0.0);
    }
    let circles = diameter + diameter * STEP * (shown - 1) as f32;
    match people - shown {
        0 => circles,
        extra => circles + diameter * GAP + more_width(diameter, extra),
    }
}

/// The comment column for a pane whose largest group is `most` people.
pub(super) fn people_column_width(scale: ui_scale::UiScale, most: usize) -> Pixels {
    people_glyph_width(scale.px(DIAMETER), most) + scale.px(COLUMN_PADDING) * 2.0
}

/// Where the glyph starts in its column, so the circles line up row to row.
pub(super) fn people_column_padding(scale: ui_scale::UiScale) -> Pixels {
    scale.px(COLUMN_PADDING)
}

/// The first two people as initials in circles, the first on top, then "+k"
/// for the rest in the annotation's colour.
pub(super) fn people_glyph(
    people: &[SharedString],
    color: gpui::Hsla,
    scale: ui_scale::UiScale,
    theme: AppTheme,
) -> AnyElement {
    let diameter = scale.px(DIAMETER);
    let shown = people.len().min(SHOWN);
    if shown == 0 {
        return gpui::Empty.into_any_element();
    }
    let step = diameter * STEP;
    // Drawn last to first so the first is on top. Each circle sits on the
    // canvas colour, so the one beneath does not show through its tint.
    let circles = people[..shown].iter().enumerate().rev().map(|(ix, name)| {
        div()
            .absolute()
            .top_0()
            .left(step * ix as f32)
            .rounded(diameter * 0.5)
            .bg(theme.colors.surface.canvas)
            .child(components::author_avatar_sized(theme, diameter, name))
    });
    let extra = people.len() - shown;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(diameter * GAP)
        .child(
            div()
                .relative()
                .flex_none()
                .w(diameter + step * (shown - 1) as f32)
                .h(diameter)
                .children(circles),
        )
        .when(extra > 0, |glyph| {
            glyph.child(
                div()
                    .flex_none()
                    .w(more_width(diameter, extra))
                    .whitespace_nowrap()
                    .text_size(more_text(diameter))
                    .text_color(color)
                    .child(format!("+{extra}")),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glyph_grows_with_its_circles_and_count() {
        let d = px(16.0);
        assert_eq!(people_glyph_width(d, 0), px(0.0));
        assert_eq!(people_glyph_width(d, 1), d);
        assert_eq!(people_glyph_width(d, 2), d * 1.6);
        assert!(people_glyph_width(d, 3) > people_glyph_width(d, 2));
        assert!(people_glyph_width(d, 14) > people_glyph_width(d, 3));
    }
}
