//! Panes extensions host: independently owned diff panes and file lists over
//! their own sessions. History keeps its own main pane.

use super::*;

pub(crate) mod diff_pane;
pub(in crate::view) mod environment;
pub(crate) mod file_list;
pub(crate) mod markdown;

pub(crate) mod raw_lines;

/// An extension's row glyph in `color`: an icon `icon_size` square, or its
/// text at the parent's size. A glyph kind added later draws nothing until
/// it is handled here.
pub(in crate::view) fn row_glyph(
    glyph: &gitcomet_extension_api::RowGlyph,
    color: gpui::Hsla,
    icon_size: Pixels,
) -> AnyElement {
    match glyph {
        gitcomet_extension_api::RowGlyph::Icon(path) => gpui::svg()
            .path(path.clone())
            .flex_none()
            .size(icon_size)
            .text_color(color)
            .into_any_element(),
        gitcomet_extension_api::RowGlyph::Text(text) => div()
            .text_color(color)
            .child(text.clone())
            .into_any_element(),
        _ => gpui::Empty.into_any_element(),
    }
}
