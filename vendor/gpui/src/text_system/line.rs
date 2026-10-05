use crate::{
    App, Bounds, DevicePixels, Half, Hsla, LineLayout, Pixels, Point, RenderGlyphParams, Result,
    SharedString, StrikethroughStyle, TextAlign, UnderlineStyle, Window, WrapBoundary,
    WrappedLineLayout, black, fill, point, px, size,
};
use derive_more::{Deref, DerefMut};
use smallvec::SmallVec;
use std::sync::Arc;

/// Pre-computed glyph data for efficient painting without per-glyph cache lookups.
///
/// This is produced by `ShapedLine::compute_glyph_raster_data` during prepaint
/// and consumed by `ShapedLine::paint_with_raster_data` during paint.
#[derive(Clone, Debug)]
pub struct GlyphRasterData {
    /// The raster bounds for each glyph, in paint order.
    pub bounds: Vec<Bounds<DevicePixels>>,
    /// The render params for each glyph (needed for sprite atlas lookup).
    pub params: Vec<RenderGlyphParams>,
}

/// Set the text decoration for a run of text.
#[derive(Debug, Clone)]
pub struct DecorationRun {
    /// The length of the run in utf-8 bytes.
    pub len: u32,

    /// The color for this run
    pub color: Hsla,

    /// The background color for this run
    pub background_color: Option<Hsla>,

    /// The underline style for this run
    pub underline: Option<UnderlineStyle>,

    /// The strikethrough style for this run
    pub strikethrough: Option<StrikethroughStyle>,
}

/// Most UI strings use one decoration. Keep that case allocation-free, while
/// cloning highlighted lines shares their arbitrarily long decoration array.
#[derive(Clone, Default, Debug)]
pub(crate) enum DecorationRuns {
    #[default]
    Empty,
    Single(DecorationRun),
    Multiple(Arc<[DecorationRun]>),
}

impl From<SmallVec<[DecorationRun; 1]>> for DecorationRuns {
    fn from(mut runs: SmallVec<[DecorationRun; 1]>) -> Self {
        match runs.len() {
            0 => Self::Empty,
            1 => Self::Single(runs.pop().unwrap()),
            _ => Self::Multiple(runs.into_vec().into()),
        }
    }
}

impl std::ops::Deref for DecorationRuns {
    type Target = [DecorationRun];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Empty => &[],
            Self::Single(run) => std::slice::from_ref(run),
            Self::Multiple(runs) => runs,
        }
    }
}

/// A line of text that has been shaped and decorated.
#[derive(Clone, Default, Debug, Deref, DerefMut)]
pub struct ShapedLine {
    #[deref]
    #[deref_mut]
    pub(crate) layout: Arc<LineLayout>,
    /// The text that was shaped for this line.
    pub text: SharedString,
    pub(crate) decoration_runs: DecorationRuns,
}

impl ShapedLine {
    /// The length of the line in utf-8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.len
    }

    /// The width of the shaped line in pixels.
    ///
    /// This is the glyph advance width computed by the text shaping system and is useful for
    /// incrementally advancing a "pen" when painting multiple fragments on the same row.
    pub fn width(&self) -> Pixels {
        self.layout.width
    }

    /// Override the len, useful if you're rendering text a
    /// as text b (e.g. rendering invisibles).
    pub fn with_len(mut self, len: usize) -> Self {
        let layout = self.layout.as_ref();
        self.layout = Arc::new(LineLayout {
            font_size: layout.font_size,
            width: layout.width,
            ascent: layout.ascent,
            descent: layout.descent,
            runs: layout.runs.clone(),
            len,
        });
        self
    }

    /// Paint the line of text to the window.
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_line(
            origin,
            &self.layout,
            line_height,
            align,
            align_width,
            &self.decoration_runs,
            &[],
            window,
            cx,
        )?;

        Ok(())
    }

    /// Paint the background of the line to the window.
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_line_background(
            origin,
            &self.layout,
            line_height,
            align,
            align_width,
            &self.decoration_runs,
            &[],
            window,
            cx,
        )?;

        Ok(())
    }

    /// Split this shaped line at a byte index, returning `(prefix, suffix)`.
    ///
    /// - `prefix` contains glyphs for bytes `[0, byte_index)` with original positions.
    ///   Its width equals the x-advance up to the split point.
    /// - `suffix` contains glyphs for bytes `[byte_index, len)` with positions
    ///   shifted left so the first glyph starts at x=0, and byte indices rebased to 0.
    /// - Decoration runs are partitioned at the boundary; a run that straddles it is
    ///   split into two with adjusted lengths.
    /// - `font_size`, `ascent`, and `descent` are copied to both halves.
    pub fn split_at(&self, byte_index: usize) -> (ShapedLine, ShapedLine) {
        let (left_layout, right_layout) = self.layout.split_at(byte_index);

        // Partition decoration runs. A run straddling the boundary is split into two.
        let mut left_decorations = SmallVec::new();
        let mut right_decorations = SmallVec::new();
        let mut decoration_offset = 0u32;
        let split_point = byte_index as u32;

        for decoration in self.decoration_runs.iter() {
            let run_end = decoration_offset + decoration.len;

            if run_end <= split_point {
                left_decorations.push(decoration.clone());
            } else if decoration_offset >= split_point {
                right_decorations.push(decoration.clone());
            } else {
                let left_len = split_point - decoration_offset;
                let right_len = run_end - split_point;
                left_decorations.push(DecorationRun {
                    len: left_len,
                    color: decoration.color,
                    background_color: decoration.background_color,
                    underline: decoration.underline,
                    strikethrough: decoration.strikethrough,
                });
                right_decorations.push(DecorationRun {
                    len: right_len,
                    color: decoration.color,
                    background_color: decoration.background_color,
                    underline: decoration.underline,
                    strikethrough: decoration.strikethrough,
                });
            }

            decoration_offset = run_end;
        }

        // Split text
        let left_text = if byte_index == self.text.len() {
            self.text.clone()
        } else {
            SharedString::new(&self.text[..byte_index])
        };
        let right_text = if byte_index == 0 {
            self.text.clone()
        } else {
            SharedString::new(&self.text[byte_index..])
        };

        let left = ShapedLine {
            layout: Arc::new(left_layout),
            text: left_text,
            decoration_runs: left_decorations.into(),
        };

        let right = ShapedLine {
            layout: Arc::new(right_layout),
            text: right_text,
            decoration_runs: right_decorations.into(),
        };

        (left, right)
    }
}

impl LineLayout {
    /// Paint this layout to the window, using the given decoration runs to color
    /// glyphs and draw underlines and strikethroughs.
    ///
    /// This is a lower-level alternative to [`ShapedLine::paint`] for callers that
    /// hold a bare layout and track decorations themselves.
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        decoration_runs: &[DecorationRun],
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_line(
            origin,
            self,
            line_height,
            align,
            align_width,
            decoration_runs,
            &[],
            window,
            cx,
        )
    }

    /// Paint the background of this layout to the window, using the given
    /// decoration runs to determine background colors.
    ///
    /// This is a lower-level alternative to [`ShapedLine::paint_background`] for
    /// callers that hold a bare layout and track decorations themselves.
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        decoration_runs: &[DecorationRun],
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_line_background(
            origin,
            self,
            line_height,
            align,
            align_width,
            decoration_runs,
            &[],
            window,
            cx,
        )
    }
}

/// A line of text that has been shaped, decorated, and wrapped by the text layout system.
#[derive(Default, Debug, Deref, DerefMut)]
pub struct WrappedLine {
    #[deref]
    #[deref_mut]
    pub(crate) layout: Arc<WrappedLineLayout>,
    /// The text that was shaped for this line.
    pub text: SharedString,
    pub(crate) decoration_runs: DecorationRuns,
}

impl WrappedLine {
    /// The length of the underlying, unwrapped layout, in utf-8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.len()
    }

    /// Paint this line of text to the window.
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        bounds: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        let align_width = match bounds {
            Some(bounds) => Some(bounds.size.width),
            None => self.layout.wrap_width,
        };

        paint_line(
            origin,
            &self.layout.unwrapped_layout,
            line_height,
            align,
            align_width,
            &self.decoration_runs,
            &self.wrap_boundaries,
            window,
            cx,
        )?;

        Ok(())
    }

    /// Paint the background of line of text to the window.
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        bounds: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        let align_width = match bounds {
            Some(bounds) => Some(bounds.size.width),
            None => self.layout.wrap_width,
        };

        paint_line_background(
            origin,
            &self.layout.unwrapped_layout,
            line_height,
            align,
            align_width,
            &self.decoration_runs,
            &self.wrap_boundaries,
            window,
            cx,
        )?;

        Ok(())
    }
}

fn paint_line(
    origin: Point<Pixels>,
    layout: &LineLayout,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    decoration_runs: &[DecorationRun],
    wrap_boundaries: &[WrapBoundary],
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    let lines = AlignedLines {
        origin,
        layout,
        wrap_boundaries,
        align,
        align_width: align_width.unwrap_or(layout.width),
    };
    window.paint_layer(lines.bounds(line_height), |window| {
        let padding_top = (line_height - layout.ascent - layout.descent) / 2.;
        let baseline_offset = point(px(0.), padding_top + layout.ascent);
        let mut decoration_runs = decoration_runs.iter();
        let mut wraps = wrap_boundaries.iter().peekable();
        let mut run_end = 0;
        let mut color = black();
        let mut current_underline: Option<(Point<Pixels>, UnderlineStyle)> = None;
        let mut current_strikethrough: Option<(Point<Pixels>, StrikethroughStyle)> = None;
        let text_system = cx.text_system().clone();
        let mut line_ix = 0;
        let mut glyph_origin = point(lines.start_x(0), origin.y);
        let mut prev_glyph_position = Point::default();
        let mut max_glyph_size = size(px(0.), px(0.));
        for (run_ix, run) in layout.runs.iter().enumerate() {
            max_glyph_size = text_system.bounding_box(run.font_id, layout.font_size).size;

            for (glyph_ix, glyph) in run.glyphs.iter().enumerate() {
                glyph_origin.x += glyph.position.x - prev_glyph_position.x;

                if wraps
                    .next_if(|boundary| boundary.is_at(run_ix, glyph_ix))
                    .is_some()
                {
                    // A decoration that runs on continues where alignment
                    // puts the next line, not at the origin.
                    line_ix += 1;
                    let next_line_x = lines.start_x(line_ix);
                    if let Some((underline_origin, underline_style)) = current_underline.as_mut() {
                        if glyph_origin.x == underline_origin.x {
                            underline_origin.x -= max_glyph_size.width.half();
                        };
                        window.paint_underline(
                            *underline_origin,
                            glyph_origin.x - underline_origin.x,
                            underline_style,
                        );
                        if glyph.index < run_end {
                            underline_origin.x = next_line_x;
                            underline_origin.y += line_height;
                        } else {
                            current_underline = None;
                        }
                    }
                    if let Some((strikethrough_origin, strikethrough_style)) =
                        current_strikethrough.as_mut()
                    {
                        if glyph_origin.x == strikethrough_origin.x {
                            strikethrough_origin.x -= max_glyph_size.width.half();
                        };
                        window.paint_strikethrough(
                            *strikethrough_origin,
                            glyph_origin.x - strikethrough_origin.x,
                            strikethrough_style,
                        );
                        if glyph.index < run_end {
                            strikethrough_origin.x = next_line_x;
                            strikethrough_origin.y += line_height;
                        } else {
                            current_strikethrough = None;
                        }
                    }

                    glyph_origin.x = next_line_x;
                    glyph_origin.y += line_height;
                }
                prev_glyph_position = glyph.position;

                let mut finished_underline: Option<(Point<Pixels>, UnderlineStyle)> = None;
                let mut finished_strikethrough: Option<(Point<Pixels>, StrikethroughStyle)> = None;
                if glyph.index >= run_end {
                    let mut style_run = decoration_runs.next();

                    // ignore style runs that apply to a partial glyph
                    while let Some(run) = style_run {
                        if glyph.index < run_end + (run.len as usize) {
                            break;
                        }
                        run_end += run.len as usize;
                        style_run = decoration_runs.next();
                    }

                    if let Some(style_run) = style_run {
                        if let Some((_, underline_style)) = &mut current_underline
                            && style_run.underline.as_ref() != Some(underline_style)
                        {
                            finished_underline = current_underline.take();
                        }
                        if let Some(run_underline) = style_run.underline.as_ref() {
                            current_underline.get_or_insert((
                                point(
                                    glyph_origin.x,
                                    glyph_origin.y + baseline_offset.y + (layout.descent * 0.618),
                                ),
                                UnderlineStyle {
                                    color: Some(run_underline.color.unwrap_or(style_run.color)),
                                    thickness: run_underline.thickness,
                                    wavy: run_underline.wavy,
                                },
                            ));
                        }
                        if let Some((_, strikethrough_style)) = &mut current_strikethrough
                            && style_run.strikethrough.as_ref() != Some(strikethrough_style)
                        {
                            finished_strikethrough = current_strikethrough.take();
                        }
                        if let Some(run_strikethrough) = style_run.strikethrough.as_ref() {
                            current_strikethrough.get_or_insert((
                                point(
                                    glyph_origin.x,
                                    glyph_origin.y
                                        + (((layout.ascent * 0.5) + baseline_offset.y) * 0.5),
                                ),
                                StrikethroughStyle {
                                    color: Some(run_strikethrough.color.unwrap_or(style_run.color)),
                                    thickness: run_strikethrough.thickness,
                                },
                            ));
                        }

                        run_end += style_run.len as usize;
                        color = style_run.color;
                    } else {
                        run_end = layout.len;
                        finished_underline = current_underline.take();
                        finished_strikethrough = current_strikethrough.take();
                    }
                }

                if let Some((mut underline_origin, underline_style)) = finished_underline {
                    if underline_origin.x == glyph_origin.x {
                        underline_origin.x -= max_glyph_size.width.half();
                    };
                    window.paint_underline(
                        underline_origin,
                        glyph_origin.x - underline_origin.x,
                        &underline_style,
                    );
                }

                if let Some((mut strikethrough_origin, strikethrough_style)) =
                    finished_strikethrough
                {
                    if strikethrough_origin.x == glyph_origin.x {
                        strikethrough_origin.x -= max_glyph_size.width.half();
                    };
                    window.paint_strikethrough(
                        strikethrough_origin,
                        glyph_origin.x - strikethrough_origin.x,
                        &strikethrough_style,
                    );
                }

                let max_glyph_bounds = Bounds {
                    origin: glyph_origin,
                    size: max_glyph_size,
                };

                let content_mask = window.content_mask();
                if max_glyph_bounds.intersects(&content_mask.bounds) {
                    let vertical_offset = point(px(0.0), glyph.position.y);
                    if glyph.is_emoji {
                        window.paint_emoji(
                            glyph_origin + baseline_offset + vertical_offset,
                            run.font_id,
                            glyph.id,
                            layout.font_size,
                        )?;
                    } else {
                        window.paint_glyph(
                            glyph_origin + baseline_offset + vertical_offset,
                            run.font_id,
                            glyph.id,
                            layout.font_size,
                            color,
                        )?;
                    }
                }
            }
        }

        let last_line_end_x = lines.end_x(wrap_boundaries.len());

        if let Some((mut underline_start, underline_style)) = current_underline.take() {
            if last_line_end_x == underline_start.x {
                underline_start.x -= max_glyph_size.width.half()
            };
            window.paint_underline(
                underline_start,
                last_line_end_x - underline_start.x,
                &underline_style,
            );
        }

        if let Some((mut strikethrough_start, strikethrough_style)) = current_strikethrough.take() {
            if last_line_end_x == strikethrough_start.x {
                strikethrough_start.x -= max_glyph_size.width.half()
            };
            window.paint_strikethrough(
                strikethrough_start,
                last_line_end_x - strikethrough_start.x,
                &strikethrough_style,
            );
        }

        Ok(())
    })
}

fn paint_line_background(
    origin: Point<Pixels>,
    layout: &LineLayout,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    decoration_runs: &[DecorationRun],
    wrap_boundaries: &[WrapBoundary],
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    let lines = AlignedLines {
        origin,
        layout,
        wrap_boundaries,
        align,
        align_width: align_width.unwrap_or(layout.width),
    };
    window.paint_layer(lines.bounds(line_height), |window| {
        let mut decoration_runs = decoration_runs.iter();
        let mut wraps = wrap_boundaries.iter().peekable();
        let mut run_end = 0;
        let mut current_background: Option<(Point<Pixels>, Hsla)> = None;
        let text_system = cx.text_system().clone();
        let mut line_ix = 0;
        let mut glyph_origin = point(lines.start_x(0), origin.y);
        let mut prev_glyph_position = Point::default();
        let mut max_glyph_size = size(px(0.), px(0.));
        for (run_ix, run) in layout.runs.iter().enumerate() {
            max_glyph_size = text_system.bounding_box(run.font_id, layout.font_size).size;

            for (glyph_ix, glyph) in run.glyphs.iter().enumerate() {
                glyph_origin.x += glyph.position.x - prev_glyph_position.x;

                if wraps
                    .next_if(|boundary| boundary.is_at(run_ix, glyph_ix))
                    .is_some()
                {
                    line_ix += 1;
                    let next_line_x = lines.start_x(line_ix);
                    if let Some((background_origin, background_color)) = current_background.as_mut()
                    {
                        if glyph_origin.x == background_origin.x {
                            background_origin.x -= max_glyph_size.width.half()
                        }
                        window.paint_quad(fill(
                            Bounds {
                                origin: *background_origin,
                                size: size(glyph_origin.x - background_origin.x, line_height),
                            },
                            *background_color,
                        ));
                        if glyph.index < run_end {
                            background_origin.x = next_line_x;
                            background_origin.y += line_height;
                        } else {
                            current_background = None;
                        }
                    }

                    glyph_origin.x = next_line_x;
                    glyph_origin.y += line_height;
                }
                prev_glyph_position = glyph.position;

                let mut finished_background: Option<(Point<Pixels>, Hsla)> = None;
                if glyph.index >= run_end {
                    let mut style_run = decoration_runs.next();

                    // ignore style runs that apply to a partial glyph
                    while let Some(run) = style_run {
                        if glyph.index < run_end + (run.len as usize) {
                            break;
                        }
                        run_end += run.len as usize;
                        style_run = decoration_runs.next();
                    }

                    if let Some(style_run) = style_run {
                        if let Some((_, background_color)) = &mut current_background
                            && style_run.background_color.as_ref() != Some(background_color)
                        {
                            finished_background = current_background.take();
                        }
                        if let Some(run_background) = style_run.background_color {
                            current_background.get_or_insert((
                                point(glyph_origin.x, glyph_origin.y),
                                run_background,
                            ));
                        }
                        run_end += style_run.len as usize;
                    } else {
                        run_end = layout.len;
                        finished_background = current_background.take();
                    }
                }

                if let Some((mut background_origin, background_color)) = finished_background {
                    let mut width = glyph_origin.x - background_origin.x;
                    if background_origin.x == glyph_origin.x {
                        background_origin.x -= max_glyph_size.width.half();
                    };
                    window.paint_quad(fill(
                        Bounds {
                            origin: background_origin,
                            size: size(width, line_height),
                        },
                        background_color,
                    ));
                }
            }
        }

        let last_line_end_x = lines.end_x(wrap_boundaries.len());

        if let Some((mut background_origin, background_color)) = current_background.take() {
            if last_line_end_x == background_origin.x {
                background_origin.x -= max_glyph_size.width.half()
            };
            window.paint_quad(fill(
                Bounds {
                    origin: background_origin,
                    size: size(last_line_end_x - background_origin.x, line_height),
                },
                background_color,
            ));
        }

        Ok(())
    })
}

/// Where a layout's lines are painted once aligned: visual line `line_ix`
/// starts at `start_x(line_ix)` and runs its own width from there.
struct AlignedLines<'a> {
    origin: Point<Pixels>,
    layout: &'a LineLayout,
    wrap_boundaries: &'a [WrapBoundary],
    align: TextAlign,
    align_width: Pixels,
}

impl AlignedLines<'_> {
    /// Where visual line `line_ix` starts.
    fn start_x(&self, line_ix: usize) -> Pixels {
        self.origin.x
            + self.layout.wrapped_line_offset(
                self.wrap_boundaries,
                line_ix,
                self.align,
                self.align_width,
            )
    }

    /// Where visual line `line_ix` ends, the whitespace it was wrapped at included.
    fn end_x(&self, line_ix: usize) -> Pixels {
        self.start_x(line_ix)
            + self
                .layout
                .wrapped_line_extent(self.wrap_boundaries, line_ix)
                .width()
    }

    /// Conservative paint-layer bounds, retaining the unwrapped layout's width
    /// at the original position and at each aligned line start. Wrapped advance
    /// widths do not include glyph ink overhangs, so tightening the layer to those
    /// widths can let later primitives draw underneath an overhanging glyph.
    fn bounds(&self, line_height: Pixels) -> Bounds<Pixels> {
        let line_count = self.wrap_boundaries.len() + 1;
        let height = line_height * line_count as f32;
        // Left-aligned lines all start at the origin and none is wider than the
        // unwrapped layout, so that width covers them without a pass over the
        // lines; the layer is clipped to the content mask anyway.
        if self.align == TextAlign::Left {
            return Bounds::new(self.origin, size(self.layout.width, height));
        }
        let (mut left, mut right) = (px(0.), self.layout.width);
        for extent in self.layout.wrapped_line_extents(self.wrap_boundaries) {
            let offset = extent.alignment_offset(self.align, self.align_width);
            left = left.min(offset);
            right = right.max(offset + self.layout.width);
        }
        Bounds::from_corners(
            point(self.origin.x + left, self.origin.y),
            point(self.origin.x + right, self.origin.y + height),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AtlasTextureId, AtlasTextureKind, AtlasTile, ContentMask, FontId, GlyphId,
        MonochromeSprite, Quad, Scene, ShapedGlyph, ShapedRun, TileId,
    };

    #[test]
    fn compact_decorations_share_many_runs_and_split_unicode() {
        let text = "é".repeat(65);
        let glyphs: Vec<_> = (0..65).map(|ix| (ix * 2, ix as f32 * 7.5)).collect();
        let runs: Vec<_> = (0..65)
            .map(|ix| DecorationRun {
                len: 2,
                color: if ix % 2 == 0 { black() } else { crate::white() },
                background_color: None,
                underline: None,
                strikethrough: None,
            })
            .collect();
        let line = make_shaped_line(&text, &glyphs, 65. * 7.5, &runs);
        let cloned = line.clone();
        let (DecorationRuns::Multiple(a), DecorationRuns::Multiple(b)) =
            (&line.decoration_runs, &cloned.decoration_runs)
        else {
            panic!("shared runs");
        };
        assert!(Arc::ptr_eq(a, b));
        for split in (0..=text.len()).step_by(2) {
            let (left, right) = line.split_at(split);
            assert_eq!(
                left.decoration_runs
                    .iter()
                    .map(|r| r.len as usize)
                    .sum::<usize>(),
                split
            );
            assert_eq!(
                right
                    .decoration_runs
                    .iter()
                    .map(|r| r.len as usize)
                    .sum::<usize>(),
                text.len() - split
            );
            assert_eq!(left.text.len() + right.text.len(), text.len());
        }
        let single = make_shaped_line("é", &[(0, 0.)], 7.5, &runs[..1]);
        assert!(matches!(single.decoration_runs, DecorationRuns::Single(_)));
        let empty = make_shaped_line("", &[], 0., &[]);
        assert!(matches!(empty.decoration_runs, DecorationRuns::Empty));
        assert!(std::mem::size_of::<ShapedLine>() < 192);
        eprintln!("ShapedLine: {} bytes", std::mem::size_of::<ShapedLine>());
    }

    /// Helper: build a ShapedLine from glyph descriptors without the platform text system.
    /// Each glyph is described as (byte_index, x_position).
    fn make_shaped_line(
        text: &str,
        glyphs: &[(usize, f32)],
        width: f32,
        decorations: &[DecorationRun],
    ) -> ShapedLine {
        let shaped_glyphs: Vec<ShapedGlyph> = glyphs
            .iter()
            .map(|&(index, x)| ShapedGlyph {
                id: GlyphId(0),
                position: point(px(x), px(0.0)),
                index,
                is_emoji: false,
            })
            .collect();

        ShapedLine {
            layout: Arc::new(LineLayout {
                font_size: px(16.0),
                width: px(width),
                ascent: px(12.0),
                descent: px(4.0),
                runs: vec![ShapedRun {
                    font_id: FontId(0),
                    glyphs: shaped_glyphs,
                }],
                len: text.len(),
            }),
            text: SharedString::new(text),
            decoration_runs: SmallVec::from(decorations.to_vec()).into(),
        }
    }

    #[test]
    fn aligned_lines_start_and_end_where_their_glyphs_are_painted() {
        // "abc efg ij" in 10px glyphs, wrapped at both spaces, painted in a 60px
        // box: lines of 40, 40 and 20px that align as 30, 30 and 20px.
        let glyphs: Vec<_> = (0..10).map(|ix| (ix, ix as f32 * 10.)).collect();
        let line = make_shaped_line("abc efg ij", &glyphs, 100., &[]);
        let wrap_boundaries = [
            WrapBoundary {
                run_ix: 0,
                glyph_ix: 4,
                trailing_whitespace_x: px(30.),
            },
            WrapBoundary {
                run_ix: 0,
                glyph_ix: 8,
                trailing_whitespace_x: px(70.),
            },
        ];
        let lines = |align| AlignedLines {
            origin: point(px(100.), px(50.)),
            layout: &line.layout,
            wrap_boundaries: &wrap_boundaries,
            align,
            align_width: px(60.),
        };
        let bounds = |left, right| {
            Bounds::from_corners(point(px(left), px(50.)), point(px(right), px(110.)))
        };

        // Left-aligned lines never shift; their layer spans the unwrapped
        // layout, which no line exceeds.
        assert_eq!(lines(TextAlign::Left).start_x(2), px(100.));
        assert_eq!(lines(TextAlign::Left).end_x(2), px(120.));
        assert_eq!(lines(TextAlign::Left).bounds(px(20.)), bounds(100., 200.));

        // Centred lines start at 15, 15 and 20; the first two hang a space
        // past their aligned end. Retain the unwrapped width at each start
        // rather than tightening the layer to glyph advances.
        assert_eq!(lines(TextAlign::Center).start_x(0), px(115.));
        assert_eq!(lines(TextAlign::Center).end_x(0), px(155.));
        assert_eq!(lines(TextAlign::Center).bounds(px(20.)), bounds(100., 220.));

        // Right-aligned lines start at 30, 30 and 40 and hang over the box.
        assert_eq!(lines(TextAlign::Right).start_x(2), px(140.));
        assert_eq!(lines(TextAlign::Right).end_x(2), px(160.));
        assert_eq!(lines(TextAlign::Right).bounds(px(20.)), bounds(100., 240.));

        // Lines wider than a 20px box overhang it on both sides.
        let overflowing = AlignedLines {
            align_width: px(20.),
            ..lines(TextAlign::Center)
        };
        assert_eq!(overflowing.start_x(0), px(95.));
        assert_eq!(overflowing.bounds(px(20.)), bounds(95., 200.));

        // An unwrapped line covers exactly its width and height.
        let unwrapped = AlignedLines {
            wrap_boundaries: &[],
            align_width: px(100.),
            ..lines(TextAlign::Right)
        };
        assert_eq!(
            unwrapped.bounds(px(20.)),
            Bounds::from_corners(point(px(100.), px(50.)), point(px(200.), px(70.)))
        );
    }

    fn assert_wrapped_glyph_overhang_stays_below_later_quad(align: TextAlign, align_width: Pixels) {
        // Two 40px lines in an alignment box. The last glyph on the first line has
        // a 10px advance but 15px of ink, like an italic f. Use explicit ink
        // bounds so this exercises scene ordering without platform fonts.
        let glyphs: Vec<_> = (0..8).map(|ix| (ix, ix as f32 * 10.)).collect();
        let line = make_shaped_line("ffffffff", &glyphs, 80., &[]);
        let wrap_boundaries = [WrapBoundary {
            run_ix: 0,
            glyph_ix: 4,
            trailing_whitespace_x: px(40.),
        }];
        let line_height = px(20.);
        let lines = AlignedLines {
            origin: Point::default(),
            layout: &line.layout,
            wrap_boundaries: &wrap_boundaries,
            align,
            align_width,
        };
        let glyph_bounds = Bounds::new(
            point(lines.start_x(0) + px(30.), px(0.)),
            size(px(15.), line_height),
        );
        // In a 60px box, right alignment puts this at x=61..65: outside the
        // advances, but overlapping the glyph's ink. Center alignment has
        // the same issue at x=51..55.
        let overlay_bounds = Bounds::new(
            point(lines.end_x(0) + px(1.), px(0.)),
            size(px(4.), line_height),
        );
        assert!(glyph_bounds.intersects(&overlay_bounds));
        let content_mask = ContentMask {
            bounds: Bounds::new(Point::default(), size(px(200.), px(200.))).scale(1.),
            ..Default::default()
        };

        let mut scene = Scene::default();
        scene.push_layer(lines.bounds(line_height).scale(1.));
        scene.insert_primitive(MonochromeSprite {
            order: 0,
            padding: 0,
            bounds: glyph_bounds.scale(1.),
            content_mask,
            color: black().into(),
            tile: AtlasTile {
                texture_id: AtlasTextureId {
                    index: 0,
                    kind: AtlasTextureKind::Monochrome,
                },
                tile_id: TileId(0),
                padding: 0,
                bounds: Bounds::new(Point::default(), size(DevicePixels(15), DevicePixels(20))),
            },
            transformation: Default::default(),
        });
        scene.pop_layer();
        scene.insert_primitive(Quad {
            bounds: overlay_bounds.scale(1.),
            content_mask,
            background: crate::white().into(),
            ..Default::default()
        });
        scene.finish();

        let glyph_order = scene.monochrome_sprites[0].order;
        let overlay_order = scene.quads[0].order;
        assert!(
            overlay_order > glyph_order,
            "{align:?} in {align_width:?}: the later quad (order {overlay_order}) must paint above the \
             overlapping glyph (order {glyph_order})"
        );
    }

    #[test]
    fn center_aligned_wrapped_glyph_overhang_stays_below_later_quad() {
        // Also cover a box wider than the unwrapped layout, where retaining
        // only the original, unshifted layer bounds would not cover the ink.
        for align_width in [px(60.), px(120.)] {
            assert_wrapped_glyph_overhang_stays_below_later_quad(TextAlign::Center, align_width);
        }
    }

    #[test]
    fn right_aligned_wrapped_glyph_overhang_stays_below_later_quad() {
        for align_width in [px(60.), px(120.)] {
            assert_wrapped_glyph_overhang_stays_below_later_quad(TextAlign::Right, align_width);
        }
    }

    #[test]
    fn test_split_at_invariants() {
        // Split "abcdef" at every possible byte index and verify structural invariants.
        let line = make_shaped_line(
            "abcdef",
            &[
                (0, 0.0),
                (1, 10.0),
                (2, 20.0),
                (3, 30.0),
                (4, 40.0),
                (5, 50.0),
            ],
            60.0,
            &[],
        );

        for i in 0..=6 {
            let (left, right) = line.split_at(i);

            assert_eq!(
                left.width() + right.width(),
                line.width(),
                "widths must sum at split={i}"
            );
            assert_eq!(
                left.len() + right.len(),
                line.len(),
                "lengths must sum at split={i}"
            );
            assert_eq!(
                format!("{}{}", left.text.as_ref(), right.text.as_ref()),
                "abcdef",
                "text must concatenate at split={i}"
            );
            assert_eq!(left.font_size, line.font_size, "font_size at split={i}");
            assert_eq!(right.ascent, line.ascent, "ascent at split={i}");
            assert_eq!(right.descent, line.descent, "descent at split={i}");
        }

        // Edge: split at 0 produces no left runs, full content on right
        let (left, right) = line.split_at(0);
        assert_eq!(left.runs.len(), 0);
        assert_eq!(right.runs[0].glyphs.len(), 6);

        // Edge: split at end produces full content on left, no right runs
        let (left, right) = line.split_at(6);
        assert_eq!(left.runs[0].glyphs.len(), 6);
        assert_eq!(right.runs.len(), 0);
    }

    #[test]
    fn test_split_at_glyph_rebasing() {
        // Two font runs (simulating a font fallback boundary at byte 3):
        //   run A (FontId 0): glyphs at bytes 0,1,2  positions 0,10,20
        //   run B (FontId 1): glyphs at bytes 3,4,5  positions 30,40,50
        // Successive splits simulate the incremental splitting done during wrap.
        let line = ShapedLine {
            layout: Arc::new(LineLayout {
                font_size: px(16.0),
                width: px(60.0),
                ascent: px(12.0),
                descent: px(4.0),
                runs: vec![
                    ShapedRun {
                        font_id: FontId(0),
                        glyphs: vec![
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(0.0), px(0.0)),
                                index: 0,
                                is_emoji: false,
                            },
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(10.0), px(0.0)),
                                index: 1,
                                is_emoji: false,
                            },
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(20.0), px(0.0)),
                                index: 2,
                                is_emoji: false,
                            },
                        ],
                    },
                    ShapedRun {
                        font_id: FontId(1),
                        glyphs: vec![
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(30.0), px(0.0)),
                                index: 3,
                                is_emoji: false,
                            },
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(40.0), px(0.0)),
                                index: 4,
                                is_emoji: false,
                            },
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(50.0), px(0.0)),
                                index: 5,
                                is_emoji: false,
                            },
                        ],
                    },
                ],
                len: 6,
            }),
            text: "abcdef".into(),
            decoration_runs: DecorationRuns::default(),
        };

        // First split at byte 2 — mid-run in run A
        let (first, remainder) = line.split_at(2);
        assert_eq!(first.text.as_ref(), "ab");
        assert_eq!(first.runs.len(), 1);
        assert_eq!(first.runs[0].font_id, FontId(0));

        // Remainder "cdef" should have two runs: tail of A (1 glyph) + all of B (3 glyphs)
        assert_eq!(remainder.text.as_ref(), "cdef");
        assert_eq!(remainder.runs.len(), 2);
        assert_eq!(remainder.runs[0].font_id, FontId(0));
        assert_eq!(remainder.runs[0].glyphs.len(), 1);
        assert_eq!(remainder.runs[0].glyphs[0].index, 0);
        assert_eq!(remainder.runs[0].glyphs[0].position.x, px(0.0));
        assert_eq!(remainder.runs[1].font_id, FontId(1));
        assert_eq!(remainder.runs[1].glyphs[0].index, 1);
        assert_eq!(remainder.runs[1].glyphs[0].position.x, px(10.0));

        // Second split at byte 2 within remainder — crosses the run boundary
        let (second, final_part) = remainder.split_at(2);
        assert_eq!(second.text.as_ref(), "cd");
        assert_eq!(final_part.text.as_ref(), "ef");
        assert_eq!(final_part.runs[0].glyphs[0].index, 0);
        assert_eq!(final_part.runs[0].glyphs[0].position.x, px(0.0));

        // Widths must sum across all three pieces
        assert_eq!(
            first.width() + second.width() + final_part.width(),
            line.width()
        );
    }

    #[test]
    fn test_split_at_decorations() {
        // Three decoration runs: red [0..2), green [2..5), blue [5..6).
        // Split at byte 3 — red goes entirely left, green straddles, blue goes entirely right.
        let red = Hsla::new(0., 1., 0.5, 1.);
        let green = Hsla::new(108., 1., 0.5, 1.);
        let blue = Hsla::new(216., 1., 0.5, 1.);

        let line = make_shaped_line(
            "abcdef",
            &[
                (0, 0.0),
                (1, 10.0),
                (2, 20.0),
                (3, 30.0),
                (4, 40.0),
                (5, 50.0),
            ],
            60.0,
            &[
                DecorationRun {
                    len: 2,
                    color: red,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                },
                DecorationRun {
                    len: 3,
                    color: green,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                },
                DecorationRun {
                    len: 1,
                    color: blue,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                },
            ],
        );

        let (left, right) = line.split_at(3);

        // Left: red(2) + green(1) — green straddled, left portion has len 1
        assert_eq!(left.decoration_runs.len(), 2);
        assert_eq!(left.decoration_runs[0].len, 2);
        assert_eq!(left.decoration_runs[0].color, red);
        assert_eq!(left.decoration_runs[1].len, 1);
        assert_eq!(left.decoration_runs[1].color, green);

        // Right: green(2) + blue(1) — green straddled, right portion has len 2
        assert_eq!(right.decoration_runs.len(), 2);
        assert_eq!(right.decoration_runs[0].len, 2);
        assert_eq!(right.decoration_runs[0].color, green);
        assert_eq!(right.decoration_runs[1].len, 1);
        assert_eq!(right.decoration_runs[1].color, blue);
    }
}
