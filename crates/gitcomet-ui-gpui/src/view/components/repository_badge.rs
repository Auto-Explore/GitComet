use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, FontWeight, SharedString, TextRun, div, point, px};

pub const REPOSITORY_BADGE_SIZE_PX: f32 = 18.0;
const REPOSITORY_BADGE_FONT_SIZE_PX: f32 = 8.5;
const REPOSITORY_BADGE_RADIUS_PX: f32 = 4.0;

/// Prefer word/camel-case boundaries (`GitComet`, `git-comet` -> `GC`), then
/// fill from the remaining letters so a single-word name still gets two
/// characters (`repository` -> `RE`).
pub fn repository_initials(name: &str) -> String {
    let mut preferred = [(0, '\0'); 2];
    let mut preferred_len = 0;
    let mut previous = None;
    for (ix, character) in name.char_indices() {
        if !character.is_alphanumeric() {
            previous = Some(character);
            continue;
        }
        if previous.is_none_or(|previous| !previous.is_alphanumeric())
            || previous.is_some_and(|previous| previous.is_lowercase() && character.is_uppercase())
        {
            preferred[preferred_len] = (ix, character);
            preferred_len += 1;
            if preferred_len == preferred.len() {
                break;
            }
        }
        previous = Some(character);
    }

    if preferred_len < preferred.len() {
        for (ix, character) in name.char_indices() {
            let already_selected = preferred[..preferred_len]
                .iter()
                .any(|&(selected_ix, _)| selected_ix == ix);
            if character.is_alphanumeric() && !already_selected {
                preferred[preferred_len] = (ix, character);
                preferred_len += 1;
                if preferred_len == preferred.len() {
                    break;
                }
            }
        }
    }

    let mut initials = String::new();
    let mut initials_len = 0;
    for &(_, character) in &preferred[..preferred_len] {
        for uppercase in character.to_uppercase() {
            initials.push(uppercase);
            initials_len += 1;
            if initials_len == 2 {
                return initials;
            }
        }
    }
    if initials.is_empty() {
        initials.push('?');
    }
    initials
}

/// Shared repository mark used by tabs and repository-picker rows. Capital
/// ink is centered by cap height, and a slight radius keeps the silhouette a
/// compact box rather than an avatar-like circle.
pub fn repository_initials_box(
    theme: AppTheme,
    scale: impl Into<UiScale>,
    initials: SharedString,
    active: bool,
) -> Div {
    let scale = scale.into();
    let foreground = if active {
        theme.colors.accent
    } else {
        with_alpha(theme.colors.text, if theme.is_dark { 0.72 } else { 0.62 })
    };
    let background = if active {
        with_alpha(theme.colors.accent, if theme.is_dark { 0.28 } else { 0.18 })
    } else {
        with_alpha(theme.colors.text, if theme.is_dark { 0.16 } else { 0.11 })
    };
    let size = scale.px(REPOSITORY_BADGE_SIZE_PX);
    let font_size = scale.px(REPOSITORY_BADGE_FONT_SIZE_PX);

    div()
        .flex_none()
        .size(size)
        .rounded(scale.px(REPOSITORY_BADGE_RADIUS_PX))
        .bg(background)
        .child(
            gpui::canvas(
                |_bounds, _window, _cx| {},
                move |bounds, (), window, cx| {
                    let mut font = window.text_style().font();
                    font.weight = FontWeight::SEMIBOLD;
                    let run = TextRun {
                        len: initials.len(),
                        font,
                        color: foreground.into(),
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
                    let origin = point(
                        bounds.left() + (bounds.size.width - shaped.width).max(px(0.0)) * 0.5,
                        super::initials_paint_origin_y(
                            bounds.top(),
                            bounds.size.height,
                            bounds.size.height,
                            shaped.ascent,
                            shaped.descent,
                            cap_height,
                        ),
                    );
                    let _ = shaped.paint(
                        origin,
                        bounds.size.height,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                },
            )
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_initials_use_name_boundaries_then_fill_from_the_name() {
        assert_eq!(repository_initials("GitComet"), "GC");
        assert_eq!(repository_initials("git-comet"), "GC");
        assert_eq!(repository_initials("repository"), "RE");
        assert_eq!(repository_initials("x"), "X");
        assert_eq!(repository_initials("***"), "?");
    }
}
