use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, FontWeight, Rgba, div};

/// Deterministic identity color for an author name. Uses the same hue recipe
/// as the history graph lanes (hash-driven hue, fixed saturation,
/// theme-dependent lightness) so avatars read as part of the same palette.
pub fn author_color(theme: AppTheme, name: &str) -> Rgba {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    name.hash(&mut hasher);
    let hue = (hasher.finish() % 360) as f32 / 360.0;
    let light = if theme.is_dark { 0.62 } else { 0.45 };
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

/// Tinted circle + initials, for retained-mode UIs (commit details, popovers).
/// The history table paints the same design directly on its row canvas.
pub fn author_avatar(theme: AppTheme, scale: impl Into<UiScale>, name: &str) -> Div {
    let scale = scale.into();
    let color = author_color(theme, name);
    let diameter = scale.px(AVATAR_DIAMETER_PX);
    div()
        .flex_none()
        .w(diameter)
        .h(diameter)
        .rounded(diameter * 0.5)
        .bg(with_alpha(color, 0.22))
        .flex()
        .items_center()
        .justify_center()
        .text_size(scale.px(AVATAR_FONT_PX))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .line_height(diameter)
        .child(author_initials(name))
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
}
