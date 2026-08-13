use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, FontWeight, Pixels, Rgba, TextRun, div, px};

/// Deterministic identity color for an author name. Uses the same hue recipe
/// as the history graph lanes (hash-driven hue, fixed saturation,
/// theme-dependent lightness) so avatars read as part of the same palette.
pub fn author_color(theme: AppTheme, name: &str) -> Rgba {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    name.hash(&mut hasher);
    let hue = (hasher.finish() % 360) as f32 / 360.0;
    let light = if theme.is_dark { 0.62 } else { 0.34 };
    gpui::hsla(hue, 0.60, light, 1.0).into()
}

/// Up to two uppercase initials for an author display name: first letters of
/// the first and last word, or the first two letters of a single word.
pub fn author_initials(name: &str) -> String {
    let word_initials: Vec<char> = name
        .split_whitespace()
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .collect();

    match word_initials.as_slice() {
        [] => "?".to_string(),
        [_] => {
            // A single usable word: take its first two letters.
            let word = name
                .split_whitespace()
                .find(|word| word.chars().any(|c| c.is_alphanumeric()))
                .unwrap_or("");
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .take(2)
                .flat_map(char::to_uppercase)
                .collect()
        }
        [first, .., last] => [*first, *last]
            .into_iter()
            .flat_map(char::to_uppercase)
            .collect(),
    }
}

pub const AVATAR_DIAMETER_PX: f32 = 16.0;
pub const AVATAR_FONT_PX: f32 = 7.5;

/// Y origin to hand `ShapedLine::paint` so capital-letter initials sit
/// visually centered in a circle. gpui places the baseline at
/// `origin_y + (line_height - ascent - descent) / 2 + ascent`, while the ink
/// of capitals spans `[baseline - cap_height, baseline]` — so the baseline
/// must land at `circle_center + cap_height / 2`, not at the line-box center.
pub fn initials_paint_origin_y(
    circle_top: Pixels,
    diameter: Pixels,
    line_height: Pixels,
    ascent: Pixels,
    descent: Pixels,
    cap_height: Pixels,
) -> Pixels {
    let baseline_in_line = (line_height - ascent - descent) * 0.5 + ascent;
    circle_top + diameter * 0.5 + cap_height * 0.5 - baseline_in_line
}

/// Tinted circle + initials, for retained-mode UIs (commit details, popovers).
/// The initials are painted by hand with cap-height centering so they sit
/// visually centered in the circle; the history table paints the same design
/// directly on its row canvas.
pub fn author_avatar(theme: AppTheme, scale: impl Into<UiScale>, name: &str) -> Div {
    let scale = scale.into();
    let color = author_color(theme, name);
    let diameter = scale.px(AVATAR_DIAMETER_PX);
    let font_size = scale.px(AVATAR_FONT_PX);
    let initials: gpui::SharedString = author_initials(name).into();
    div()
        .flex_none()
        .w(diameter)
        .h(diameter)
        .rounded(diameter * 0.5)
        .bg(with_alpha(color, 0.22))
        .child(
            gpui::canvas(
                |_bounds, _window, _cx| {},
                move |bounds, (), window, cx| {
                    let mut font = window.text_style().font();
                    font.weight = FontWeight::SEMIBOLD;
                    let run = TextRun {
                        len: initials.len(),
                        font,
                        color: color.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let shaped =
                        window
                            .text_system()
                            .shape_line(initials.clone(), font_size, &[run], None);
                    let cap_height = shaped
                        .runs
                        .first()
                        .map(|run| window.text_system().cap_height(run.font_id, font_size))
                        .unwrap_or(font_size * 0.7);
                    let line_height = bounds.size.height;
                    let origin = gpui::point(
                        bounds.left() + (bounds.size.width - shaped.width).max(px(0.0)) * 0.5,
                        initials_paint_origin_y(
                            bounds.top(),
                            bounds.size.height,
                            line_height,
                            shaped.ascent,
                            shaped.descent,
                            cap_height,
                        ),
                    );
                    let _ =
                        shaped.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx);
                },
            )
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_take_first_and_last_word() {
        assert_eq!(author_initials("Sampo Kivistö"), "SK");
        assert_eq!(author_initials("Roope Von Airinen"), "RA");
    }

    #[test]
    fn initials_from_single_word_take_two_chars() {
        assert_eq!(author_initials("dependabot[bot]"), "DE");
        assert_eq!(author_initials("x"), "X");
    }

    #[test]
    fn initials_fall_back_for_empty_or_symbolic_names() {
        assert_eq!(author_initials(""), "?");
        assert_eq!(author_initials("***"), "?");
    }

    #[test]
    fn author_color_is_deterministic_and_name_sensitive() {
        let theme = AppTheme::gitcomet_dark();
        assert_eq!(
            author_color(theme, "Sampo Kivistö"),
            author_color(theme, "Sampo Kivistö")
        );
        assert_ne!(
            author_color(theme, "Sampo Kivistö"),
            author_color(theme, "Roope Airinen")
        );
    }

    #[test]
    fn initials_origin_places_baseline_at_center_plus_half_cap_height() {
        // line box 16px, ascent 8, descent 2, cap height 6, circle top 0 / d 16.
        let origin_y =
            initials_paint_origin_y(px(0.0), px(16.0), px(16.0), px(8.0), px(2.0), px(6.0));
        // gpui paints the baseline at origin + (lh - ascent - descent)/2 + ascent.
        let baseline = origin_y + (px(16.0) - px(8.0) - px(2.0)) * 0.5 + px(8.0);
        // Circle center (8) + cap/2 (3): capital ink [5, 11] centers on 8.
        assert_eq!(baseline, px(11.0));
    }

    #[test]
    fn initials_origin_is_line_height_invariant() {
        // The same visual placement must come out regardless of the line box
        // the style happens to resolve (the term cancels out of the math).
        let a = initials_paint_origin_y(px(0.0), px(16.0), px(12.0), px(8.0), px(2.0), px(6.0))
            + (px(12.0) - px(8.0) - px(2.0)) * 0.5
            + px(8.0);
        let b = initials_paint_origin_y(px(0.0), px(16.0), px(20.0), px(8.0), px(2.0), px(6.0))
            + (px(20.0) - px(8.0) - px(2.0)) * 0.5
            + px(8.0);
        assert_eq!(a, b);
    }
}
