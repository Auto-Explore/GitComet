use crate::theme::{AppTheme, with_alpha};
use gpui::prelude::*;
use gpui::{Div, div, px};

/// A stand-in for content that has not arrived yet.
///
/// It holds the shape of what is coming rather than describing it, so the page
/// does not reflow when the real thing lands and the reader is not asked to
/// read a sentence about waiting. The caller gives it its box; this only
/// decides how it looks.
///
/// Deliberately still: the pane it sits in lays out every row of the document
/// on every frame, and a picture can take seconds to decode, so a pulse would
/// mean seconds of full repaints behind it.
pub fn skeleton(theme: AppTheme) -> Div {
    div()
        .rounded(px(theme.radii.row))
        .bg(with_alpha(
            theme.colors.surface.raised,
            if theme.is_dark { 0.55 } else { 0.75 },
        ))
        .border_1()
        .border_color(with_alpha(
            theme.colors.stroke.default,
            if theme.is_dark { 0.70 } else { 0.60 },
        ))
}
