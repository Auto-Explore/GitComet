//! Image and diagram previews for GPUI: device-pixel image inspection with
//! tiled resampling, SVG rasterization with bundled fonts, asynchronous
//! diagram surfaces, and the source/rendered toggle they share. Hosts supply
//! the inputs and open the menus the toolbars ask for.

pub mod diagram_preview;
pub mod image_preview;
pub mod rendered_preview;

#[cfg(test)]
mod source_guards;
