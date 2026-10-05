use crate::{
    FontId, GlyphId, Pixels, PlatformTextSystem, Point, SharedString, Size, TextAlign, point, px,
};
use collections::FxHashMap;
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};
use smallvec::SmallVec;
use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
    ops::Range,
    sync::Arc,
};

use super::LineWrapper;

/// A laid out and styled line of text
#[derive(Default, Debug)]
pub struct LineLayout {
    /// The font size for this line
    pub font_size: Pixels,
    /// The width of the line
    pub width: Pixels,
    /// The ascent of the line
    pub ascent: Pixels,
    /// The descent of the line
    pub descent: Pixels,
    /// The shaped runs that make up this line
    pub runs: Vec<ShapedRun>,
    /// The length of the line in utf-8 bytes
    pub len: usize,
}

/// A run of text that has been shaped .
#[derive(Debug, Clone)]
pub struct ShapedRun {
    /// The font id for this run
    pub font_id: FontId,
    /// The glyphs that make up this run
    pub glyphs: Vec<ShapedGlyph>,
}

/// A single glyph, ready to paint.
#[derive(Clone, Debug)]
pub struct ShapedGlyph {
    /// The ID for this glyph, as determined by the text system.
    pub id: GlyphId,

    /// The position of this glyph in its containing line.
    pub position: Point<Pixels>,

    /// The index of this glyph in the original text.
    pub index: usize,

    /// Whether this glyph is an emoji
    pub is_emoji: bool,
}

impl LineLayout {
    /// The index for the character at the given x coordinate
    pub fn index_for_x(&self, x: Pixels) -> Option<usize> {
        if x >= self.width {
            None
        } else {
            for run in self.runs.iter().rev() {
                for glyph in run.glyphs.iter().rev() {
                    if glyph.position.x <= x {
                        return Some(glyph.index);
                    }
                }
            }
            Some(0)
        }
    }

    /// closest_index_for_x returns the character boundary closest to the given x coordinate
    /// (e.g. to handle aligning up/down arrow keys)
    pub fn closest_index_for_x(&self, x: Pixels) -> usize {
        let mut prev_index = 0;
        let mut prev_x = px(0.);

        for run in self.runs.iter() {
            for glyph in run.glyphs.iter() {
                if glyph.position.x >= x {
                    if glyph.position.x - x < x - prev_x {
                        return glyph.index;
                    } else {
                        return prev_index;
                    }
                }
                prev_index = glyph.index;
                prev_x = glyph.position.x;
            }
        }

        if self.len == 1 {
            if x > self.width / 2. {
                return 1;
            } else {
                return 0;
            }
        }

        self.len
    }

    /// The x position of the character at the given index
    pub fn x_for_index(&self, index: usize) -> Pixels {
        for run in &self.runs {
            for glyph in &run.glyphs {
                if glyph.index >= index {
                    return glyph.position.x;
                }
            }
        }
        self.width
    }

    /// The corresponding Font at the given index
    pub fn font_id_for_index(&self, index: usize) -> Option<FontId> {
        for run in &self.runs {
            for glyph in &run.glyphs {
                if glyph.index >= index {
                    return Some(run.font_id);
                }
            }
        }
        None
    }

    /// Split this layout at a byte index, returning `(prefix, suffix)`.
    ///
    /// - `prefix` contains glyphs for bytes `[0, byte_index)` with original positions.
    ///   Its width equals the x-advance up to the split point.
    /// - `suffix` contains glyphs for bytes `[byte_index, len)` with positions
    ///   shifted left so the first glyph starts at x=0, and byte indices rebased to 0.
    /// - `font_size`, `ascent`, and `descent` are copied to both halves.
    pub fn split_at(&self, byte_index: usize) -> (LineLayout, LineLayout) {
        let x_offset = self.x_for_index(byte_index);

        // Partition glyph runs. A single run may contribute glyphs to both halves.
        let mut left_runs = Vec::new();
        let mut right_runs = Vec::new();

        for run in &self.runs {
            let split_pos = run.glyphs.partition_point(|g| g.index < byte_index);

            if split_pos > 0 {
                left_runs.push(ShapedRun {
                    font_id: run.font_id,
                    glyphs: run.glyphs[..split_pos].to_vec(),
                });
            }

            if split_pos < run.glyphs.len() {
                let right_glyphs = run.glyphs[split_pos..]
                    .iter()
                    .map(|g| ShapedGlyph {
                        id: g.id,
                        position: point(g.position.x - x_offset, g.position.y),
                        index: g.index - byte_index,
                        is_emoji: g.is_emoji,
                    })
                    .collect();
                right_runs.push(ShapedRun {
                    font_id: run.font_id,
                    glyphs: right_glyphs,
                });
            }
        }

        let left = LineLayout {
            font_size: self.font_size,
            width: x_offset,
            ascent: self.ascent,
            descent: self.descent,
            runs: left_runs,
            len: byte_index,
        };

        let right = LineLayout {
            font_size: self.font_size,
            width: self.width - x_offset,
            ascent: self.ascent,
            descent: self.descent,
            runs: right_runs,
            len: self.len - byte_index,
        };

        (left, right)
    }

    fn compute_wrap_boundaries(
        &self,
        text: &str,
        wrap_width: Pixels,
        max_lines: Option<usize>,
    ) -> SmallVec<[WrapBoundary; 1]> {
        let mut boundaries = SmallVec::new();
        let mut first_non_whitespace_ix = None;
        let mut last_candidate_ix = None;
        let mut last_candidate_x = px(0.);
        // Glyphs are addressed as `(run_ix, glyph_ix)`, which orders them as the text does.
        let mut last_boundary = (0, 0);
        let mut last_boundary_x = px(0.);
        let mut prev_ch = '\0';
        let mut glyphs = self
            .runs
            .iter()
            .enumerate()
            .flat_map(move |(run_ix, run)| {
                run.glyphs.iter().enumerate().map(move |(glyph_ix, glyph)| {
                    let character = text[glyph.index..].chars().next().unwrap();
                    ((run_ix, glyph_ix), character, glyph.position.x)
                })
            })
            .peekable();

        while let Some((glyph, ch, x)) = glyphs.next() {
            if ch == '\n' {
                continue;
            }

            // Here is very similar to `LineWrapper::wrap_line` to determine text wrapping,
            // but there are some differences, so we have to duplicate the code here.
            if LineWrapper::is_word_char(ch) {
                if prev_ch == ' ' && ch != ' ' && first_non_whitespace_ix.is_some() {
                    last_candidate_ix = Some(glyph);
                    last_candidate_x = x;
                }
            } else {
                if ch != ' ' && first_non_whitespace_ix.is_some() {
                    last_candidate_ix = Some(glyph);
                    last_candidate_x = x;
                }
            }

            if ch != ' ' && first_non_whitespace_ix.is_none() {
                first_non_whitespace_ix = Some(glyph);
            }

            let next_x = glyphs.peek().map_or(self.width, |(_, _, x)| *x);
            let width = next_x - last_boundary_x;

            if width > wrap_width && glyph > last_boundary {
                // When used line_clamp, we should limit the number of lines.
                if let Some(max_lines) = max_lines
                    && boundaries.len() >= max_lines.saturating_sub(1)
                {
                    break;
                }

                let line_start = last_boundary;
                (last_boundary, last_boundary_x) = last_candidate_ix
                    .take()
                    .map_or((glyph, x), |candidate| (candidate, last_candidate_x));
                let (run_ix, glyph_ix) = last_boundary;
                boundaries.push(WrapBoundary {
                    run_ix,
                    glyph_ix,
                    trailing_whitespace_x: self.trailing_whitespace_x(
                        text,
                        line_start,
                        last_boundary,
                    ),
                });
            }
            prev_ch = ch;
        }

        boundaries
    }

    /// Where the spaces a line was wrapped at begin: the x of the first of the `' '` glyphs
    /// running up to the glyph at `boundary`, or that glyph's own x when the line does not end
    /// in a space. The spaces are only followed back to the glyph at `line_start`, so a line of
    /// nothing but spaces hangs whole.
    fn trailing_whitespace_x(
        &self,
        text: &str,
        line_start: (usize, usize),
        boundary: (usize, usize),
    ) -> Pixels {
        let (run_ix, glyph_ix) = boundary;
        self.runs[..=run_ix]
            .iter()
            .enumerate()
            .rev()
            .flat_map(|(ix, run)| {
                let end = if ix == run_ix {
                    glyph_ix
                } else {
                    run.glyphs.len()
                };
                run.glyphs[..end]
                    .iter()
                    .enumerate()
                    .rev()
                    .map(move |(glyph_ix, glyph)| ((ix, glyph_ix), glyph))
            })
            .take_while(|(ix, glyph)| *ix >= line_start && text[glyph.index..].starts_with(' '))
            .last()
            .map_or_else(
                || self.runs[run_ix].glyphs[glyph_ix].position.x,
                |(_, glyph)| glyph.position.x,
            )
    }

    /// The glyph that starts the line after `boundary`.
    pub(crate) fn glyph_at(&self, boundary: &WrapBoundary) -> &ShapedGlyph {
        &self.runs[boundary.run_ix].glyphs[boundary.glyph_ix]
    }

    /// Where the line before `boundary` ends, as `(visible_end, end)`; the last line, which no
    /// boundary follows, ends with the layout.
    fn wrapped_line_end(&self, boundary: Option<&WrapBoundary>) -> (Pixels, Pixels) {
        match boundary {
            Some(boundary) => (
                boundary.trailing_whitespace_x,
                self.glyph_at(boundary).position.x,
            ),
            None => (self.width, self.width),
        }
    }

    /// Where visual line `line_ix` of this layout lies once it is wrapped at `wrap_boundaries`.
    /// `line_ix` is at most `wrap_boundaries.len()`, the index of the last line.
    pub(crate) fn wrapped_line_extent(
        &self,
        wrap_boundaries: &[WrapBoundary],
        line_ix: usize,
    ) -> WrappedLineExtent {
        debug_assert!(line_ix <= wrap_boundaries.len(), "no line {line_ix}");
        let start = match line_ix.checked_sub(1) {
            Some(previous) => self.glyph_at(&wrap_boundaries[previous]).position.x,
            None => Pixels::ZERO,
        };
        let (visible_end, end) = self.wrapped_line_end(wrap_boundaries.get(line_ix));
        WrappedLineExtent {
            start,
            visible_end,
            end,
        }
    }

    /// The extents of every visual line of this layout wrapped at `wrap_boundaries`, first line
    /// first. Each line starts where the one before it ended, so every boundary is looked up once.
    pub(crate) fn wrapped_line_extents<'a>(
        &'a self,
        wrap_boundaries: &'a [WrapBoundary],
    ) -> impl Iterator<Item = WrappedLineExtent> + 'a {
        let mut start = Pixels::ZERO;
        wrap_boundaries
            .iter()
            .map(Some)
            .chain([None])
            .map(move |boundary| {
                let (visible_end, end) = self.wrapped_line_end(boundary);
                let extent = WrappedLineExtent {
                    start,
                    visible_end,
                    end,
                };
                start = end;
                extent
            })
    }

    /// How far `align` shifts visual line `line_ix` right of this layout's origin when it is
    /// painted in a box `align_width` wide, wrapped at `wrap_boundaries`. Painting and hit
    /// testing share it, so a position resolves to the glyph painted there. A line the layout
    /// does not have is not shifted.
    pub(crate) fn wrapped_line_offset(
        &self,
        wrap_boundaries: &[WrapBoundary],
        line_ix: usize,
        align: TextAlign,
        align_width: Pixels,
    ) -> Pixels {
        // Left-aligned text, by far the common case, never shifts: painting calls
        // this once per soft wrap every frame, so skip the extent lookups for it.
        if align == TextAlign::Left || line_ix > wrap_boundaries.len() {
            return Pixels::ZERO;
        }
        self.wrapped_line_extent(wrap_boundaries, line_ix)
            .alignment_offset(align, align_width)
    }
}

/// Where one visual line of a wrapped layout lies within the unwrapped layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WrappedLineExtent {
    /// The x of the glyph that starts the line.
    pub(crate) start: Pixels,
    /// Where alignment measures the line to: the whitespace it was wrapped at hangs past this.
    pub(crate) visible_end: Pixels,
    /// The x of the glyph that starts the next line, or the layout's width for the last line.
    pub(crate) end: Pixels,
}

impl WrappedLineExtent {
    /// The line's width, the whitespace it was wrapped at included.
    pub(crate) fn width(&self) -> Pixels {
        self.end - self.start
    }

    /// How far `align` shifts the line right of the layout's origin in a box `align_width`
    /// wide. A line wider than the box shifts left, past the box's edge.
    pub(crate) fn alignment_offset(&self, align: TextAlign, align_width: Pixels) -> Pixels {
        let visible_width = self.visible_end - self.start;
        match align {
            TextAlign::Left => Pixels::ZERO,
            TextAlign::Center => (align_width - visible_width) / 2.,
            TextAlign::Right => align_width - visible_width,
        }
    }
}

/// A line of text that has been wrapped to fit a given width
#[derive(Default, Debug)]
pub struct WrappedLineLayout {
    /// The line layout, pre-wrapping.
    pub unwrapped_layout: Arc<LineLayout>,

    /// The boundaries at which the line was wrapped
    pub wrap_boundaries: SmallVec<[WrapBoundary; 1]>,

    /// The width of the line, if it was wrapped
    pub wrap_width: Option<Pixels>,
}

/// A boundary at which a line was wrapped
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WrapBoundary {
    /// The index in the run just before the line was wrapped
    pub run_ix: usize,
    /// The index of the glyph just before the line was wrapped
    pub glyph_ix: usize,
    /// Where the spaces the line was wrapped at begin, in the unwrapped layout: the glyph's own
    /// x when the line does not end in a space. Alignment measures the line to here, so those
    /// spaces hang past its aligned edge instead of pushing its text away from it. Whitespace at
    /// the end of the text is not wrapped at and so aligns like any other glyph.
    pub trailing_whitespace_x: Pixels,
}

impl WrapBoundary {
    /// Whether the line after this boundary starts with glyph `glyph_ix` of run `run_ix`.
    pub(crate) fn is_at(&self, run_ix: usize, glyph_ix: usize) -> bool {
        (self.run_ix, self.glyph_ix) == (run_ix, glyph_ix)
    }
}

impl WrappedLineLayout {
    /// The length of the underlying text, in utf8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.unwrapped_layout.len
    }

    /// The width of this line, in pixels, whether or not it was wrapped.
    pub fn width(&self) -> Pixels {
        self.wrap_width
            .unwrap_or(Pixels::MAX)
            .min(self.unwrapped_layout.width)
    }

    /// The size of the whole wrapped text, for the given line_height.
    /// can span multiple lines if there are multiple wrap boundaries.
    pub fn size(&self, line_height: Pixels) -> Size<Pixels> {
        Size {
            width: self.width(),
            height: line_height * (self.wrap_boundaries.len() + 1),
        }
    }

    /// The ascent of a line in this layout
    pub fn ascent(&self) -> Pixels {
        self.unwrapped_layout.ascent
    }

    /// The descent of a line in this layout
    pub fn descent(&self) -> Pixels {
        self.unwrapped_layout.descent
    }

    /// The wrap boundaries in this layout
    pub fn wrap_boundaries(&self) -> &[WrapBoundary] {
        &self.wrap_boundaries
    }

    /// The font size of this layout
    pub fn font_size(&self) -> Pixels {
        self.unwrapped_layout.font_size
    }

    /// The runs in this layout, sans wrapping
    pub fn runs(&self) -> &[ShapedRun] {
        &self.unwrapped_layout.runs
    }

    /// How far `align` moves visual line `wrapped_line_ix` right of the layout's origin when it
    /// is painted in a box `align_width` wide: the shift painting applies, so callers can place
    /// things on aligned text. The whitespace a line was wrapped at hangs past its aligned edge
    /// rather than counting toward its width. A line past the last is not moved.
    pub fn wrapped_line_offset(
        &self,
        wrapped_line_ix: usize,
        align: TextAlign,
        align_width: Pixels,
    ) -> Pixels {
        self.unwrapped_layout.wrapped_line_offset(
            &self.wrap_boundaries,
            wrapped_line_ix,
            align,
            align_width,
        )
    }

    /// The index corresponding to a given position in this layout for the given line height.
    ///
    /// See also [`Self::closest_index_for_position`].
    pub fn index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        self._index_for_position(position, line_height, false, TextAlign::Left, Pixels::ZERO)
    }

    /// [`Self::index_for_position`] for text painted with `align` in a box
    /// `align_width` wide.
    pub fn index_for_position_aligned(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Pixels,
    ) -> Result<usize, usize> {
        self._index_for_position(position, line_height, false, align, align_width)
    }

    /// The closest index to a given position in this layout for the given line height.
    ///
    /// Closest means the character boundary closest to the given position.
    ///
    /// See also [`LineLayout::closest_index_for_x`].
    pub fn closest_index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        self._index_for_position(position, line_height, true, TextAlign::Left, Pixels::ZERO)
    }

    /// [`Self::closest_index_for_position`] for text painted with `align` in
    /// a box `align_width` wide.
    pub fn closest_index_for_position_aligned(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Pixels,
    ) -> Result<usize, usize> {
        self._index_for_position(position, line_height, true, align, align_width)
    }

    fn _index_for_position(
        &self,
        mut position: Point<Pixels>,
        line_height: Pixels,
        closest: bool,
        align: TextAlign,
        align_width: Pixels,
    ) -> Result<usize, usize> {
        let wrapped_line_ix = (position.y / line_height) as usize;

        let wrapped_line_start_index;
        let wrapped_line_start_x;
        if wrapped_line_ix > 0 {
            let Some(line_start_boundary) = self.wrap_boundaries.get(wrapped_line_ix - 1) else {
                return Err(0);
            };
            let glyph = self.unwrapped_layout.glyph_at(line_start_boundary);
            wrapped_line_start_index = glyph.index;
            wrapped_line_start_x = glyph.position.x;
        } else {
            wrapped_line_start_index = 0;
            wrapped_line_start_x = Pixels::ZERO;
        };

        let wrapped_line_end_index;
        let wrapped_line_end_x;
        if wrapped_line_ix < self.wrap_boundaries.len() {
            let next_wrap_boundary_ix = wrapped_line_ix;
            let next_wrap_boundary = &self.wrap_boundaries[next_wrap_boundary_ix];
            let glyph = self.unwrapped_layout.glyph_at(next_wrap_boundary);
            wrapped_line_end_index = glyph.index;
            wrapped_line_end_x = glyph.position.x;
        } else {
            wrapped_line_end_index = self.unwrapped_layout.len;
            wrapped_line_end_x = self.unwrapped_layout.width;
        };

        position.x -= self.wrapped_line_offset(wrapped_line_ix, align, align_width);
        let mut position_in_unwrapped_line = position;
        position_in_unwrapped_line.x += wrapped_line_start_x;
        if position_in_unwrapped_line.x < wrapped_line_start_x {
            Err(wrapped_line_start_index)
        } else if position_in_unwrapped_line.x >= wrapped_line_end_x {
            Err(wrapped_line_end_index)
        } else {
            if closest {
                Ok(self
                    .unwrapped_layout
                    .closest_index_for_x(position_in_unwrapped_line.x))
            } else {
                Ok(self
                    .unwrapped_layout
                    .index_for_x(position_in_unwrapped_line.x)
                    .unwrap())
            }
        }
    }

    /// Returns the pixel position for the given byte index.
    pub fn position_for_index(&self, index: usize, line_height: Pixels) -> Option<Point<Pixels>> {
        self.position_for_index_aligned(index, line_height, TextAlign::Left, Pixels::ZERO)
    }

    /// [`Self::position_for_index`] for text painted with `align` in a box
    /// `align_width` wide. An index on a wrap boundary stays at the end of
    /// the line above, as it does unaligned.
    pub fn position_for_index_aligned(
        &self,
        index: usize,
        line_height: Pixels,
        align: TextAlign,
        align_width: Pixels,
    ) -> Option<Point<Pixels>> {
        let mut line_start_ix = 0;
        let mut line_end_indices = self
            .wrap_boundaries
            .iter()
            .map(|wrap_boundary| self.unwrapped_layout.glyph_at(wrap_boundary).index)
            .chain([self.len()])
            .enumerate();
        for (ix, line_end_ix) in line_end_indices {
            let line_y = ix as f32 * line_height;
            if index < line_start_ix {
                break;
            } else if index > line_end_ix {
                line_start_ix = line_end_ix;
                continue;
            } else {
                let line_start_x = self.unwrapped_layout.x_for_index(line_start_ix);
                let x = self.unwrapped_layout.x_for_index(index) - line_start_x
                    + self.wrapped_line_offset(ix, align, align_width);
                return Some(point(x, line_y));
            }
        }

        None
    }
}

pub(crate) struct LineLayoutCache {
    previous_frame: Mutex<FrameCache>,
    current_frame: RwLock<FrameCache>,
    platform_text_system: Arc<dyn PlatformTextSystem>,
}

#[derive(Default)]
struct FrameCache {
    lines: FxHashMap<Arc<CacheKey>, Arc<LineLayout>>,
    wrapped_lines: FxHashMap<Arc<CacheKey>, Arc<WrappedLineLayout>>,
    used_lines: Vec<Arc<CacheKey>>,
    used_wrapped_lines: Vec<Arc<CacheKey>>,

    // Content-addressable caches keyed by caller-provided text hash + layout params.
    // These allow cache hits without materializing a contiguous `SharedString`.
    //
    // IMPORTANT: To support allocation-free lookups, we store these maps using a key type
    // (`HashedCacheKeyRef`) that can be computed without building a contiguous `&str`/`SharedString`.
    // On miss, we allocate once and store under an owned `HashedCacheKey`.
    lines_by_hash: FxHashMap<Arc<HashedCacheKey>, Arc<LineLayout>>,
    wrapped_lines_by_hash: FxHashMap<Arc<HashedCacheKey>, Arc<WrappedLineLayout>>,
    used_lines_by_hash: Vec<Arc<HashedCacheKey>>,
    used_wrapped_lines_by_hash: Vec<Arc<HashedCacheKey>>,
}

#[derive(Clone, Default)]
pub(crate) struct LineLayoutIndex {
    lines_index: usize,
    wrapped_lines_index: usize,
    lines_by_hash_index: usize,
    wrapped_lines_by_hash_index: usize,
}

impl LineLayoutCache {
    pub fn new(platform_text_system: Arc<dyn PlatformTextSystem>) -> Self {
        Self {
            previous_frame: Mutex::default(),
            current_frame: RwLock::default(),
            platform_text_system,
        }
    }

    pub fn layout_index(&self) -> LineLayoutIndex {
        let frame = self.current_frame.read();
        LineLayoutIndex {
            lines_index: frame.used_lines.len(),
            wrapped_lines_index: frame.used_wrapped_lines.len(),
            lines_by_hash_index: frame.used_lines_by_hash.len(),
            wrapped_lines_by_hash_index: frame.used_wrapped_lines_by_hash.len(),
        }
    }

    pub fn reuse_layouts(&self, range: Range<LineLayoutIndex>) {
        let mut previous_frame = &mut *self.previous_frame.lock();
        let mut current_frame = &mut *self.current_frame.write();

        for key in &previous_frame.used_lines[range.start.lines_index..range.end.lines_index] {
            if let Some((key, line)) = previous_frame.lines.remove_entry(key) {
                current_frame.lines.insert(key, line);
            }
            current_frame.used_lines.push(key.clone());
        }

        for key in &previous_frame.used_wrapped_lines
            [range.start.wrapped_lines_index..range.end.wrapped_lines_index]
        {
            if let Some((key, line)) = previous_frame.wrapped_lines.remove_entry(key) {
                current_frame.wrapped_lines.insert(key, line);
            }
            current_frame.used_wrapped_lines.push(key.clone());
        }

        for key in &previous_frame.used_lines_by_hash
            [range.start.lines_by_hash_index..range.end.lines_by_hash_index]
        {
            if let Some((key, line)) = previous_frame.lines_by_hash.remove_entry(key) {
                current_frame.lines_by_hash.insert(key, line);
            }
            current_frame.used_lines_by_hash.push(key.clone());
        }

        for key in &previous_frame.used_wrapped_lines_by_hash
            [range.start.wrapped_lines_by_hash_index..range.end.wrapped_lines_by_hash_index]
        {
            if let Some((key, line)) = previous_frame.wrapped_lines_by_hash.remove_entry(key) {
                current_frame.wrapped_lines_by_hash.insert(key, line);
            }
            current_frame.used_wrapped_lines_by_hash.push(key.clone());
        }
    }

    pub fn truncate_layouts(&self, index: LineLayoutIndex) {
        let mut current_frame = &mut *self.current_frame.write();
        current_frame.used_lines.truncate(index.lines_index);
        current_frame
            .used_wrapped_lines
            .truncate(index.wrapped_lines_index);
        current_frame
            .used_lines_by_hash
            .truncate(index.lines_by_hash_index);
        current_frame
            .used_wrapped_lines_by_hash
            .truncate(index.wrapped_lines_by_hash_index);
    }

    pub fn finish_frame(&self) {
        let mut prev_frame = self.previous_frame.lock();
        let mut curr_frame = self.current_frame.write();
        std::mem::swap(&mut *prev_frame, &mut *curr_frame);
        curr_frame.lines.clear();
        curr_frame.wrapped_lines.clear();
        curr_frame.used_lines.clear();
        curr_frame.used_wrapped_lines.clear();

        curr_frame.lines_by_hash.clear();
        curr_frame.wrapped_lines_by_hash.clear();
        curr_frame.used_lines_by_hash.clear();
        curr_frame.used_wrapped_lines_by_hash.clear();
    }

    pub fn layout_wrapped_line<Text>(
        &self,
        text: Text,
        font_size: Pixels,
        runs: &[FontRun],
        wrap_width: Option<Pixels>,
        max_lines: Option<usize>,
    ) -> Arc<WrappedLineLayout>
    where
        Text: AsRef<str>,
        SharedString: From<Text>,
    {
        let key = &CacheKeyRef {
            text: text.as_ref(),
            font_size,
            runs,
            wrap_width,
            force_width: None,
        } as &dyn AsCacheKeyRef;

        let current_frame = self.current_frame.upgradable_read();
        if let Some(layout) = current_frame.wrapped_lines.get(key) {
            return layout.clone();
        }

        let previous_frame_entry = self.previous_frame.lock().wrapped_lines.remove_entry(key);
        if let Some((key, layout)) = previous_frame_entry {
            let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);
            current_frame
                .wrapped_lines
                .insert(key.clone(), layout.clone());
            current_frame.used_wrapped_lines.push(key);
            layout
        } else {
            drop(current_frame);
            let text = SharedString::from(text);
            let unwrapped_layout = self.layout_line::<&SharedString>(&text, font_size, runs, None);
            let wrap_boundaries = if let Some(wrap_width) = wrap_width {
                unwrapped_layout.compute_wrap_boundaries(text.as_ref(), wrap_width, max_lines)
            } else {
                SmallVec::new()
            };
            let layout = Arc::new(WrappedLineLayout {
                unwrapped_layout,
                wrap_boundaries,
                wrap_width,
            });
            let key = Arc::new(CacheKey {
                text,
                font_size,
                runs: SmallVec::from(runs),
                wrap_width,
                force_width: None,
            });

            let mut current_frame = self.current_frame.write();
            current_frame
                .wrapped_lines
                .insert(key.clone(), layout.clone());
            current_frame.used_wrapped_lines.push(key);

            layout
        }
    }

    pub fn layout_line<Text>(
        &self,
        text: Text,
        font_size: Pixels,
        runs: &[FontRun],
        force_width: Option<Pixels>,
    ) -> Arc<LineLayout>
    where
        Text: AsRef<str>,
        SharedString: From<Text>,
    {
        let key = &CacheKeyRef {
            text: text.as_ref(),
            font_size,
            runs,
            wrap_width: None,
            force_width,
        } as &dyn AsCacheKeyRef;

        let current_frame = self.current_frame.upgradable_read();
        if let Some(layout) = current_frame.lines.get(key) {
            return layout.clone();
        }

        let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);
        if let Some((key, layout)) = self.previous_frame.lock().lines.remove_entry(key) {
            current_frame.lines.insert(key.clone(), layout.clone());
            current_frame.used_lines.push(key);
            layout
        } else {
            let text = SharedString::from(text);
            let mut layout = self
                .platform_text_system
                .layout_line(&text, font_size, runs);

            if let Some(force_width) = force_width {
                apply_force_width_to_layout(&mut layout, force_width);
            }

            let key = Arc::new(CacheKey {
                text,
                font_size,
                runs: SmallVec::from(runs),
                wrap_width: None,
                force_width,
            });
            let layout = Arc::new(layout);
            current_frame.lines.insert(key.clone(), layout.clone());
            current_frame.used_lines.push(key);
            layout
        }
    }

    /// Try to retrieve a previously-shaped line layout using a caller-provided content hash.
    ///
    /// This is a *non-allocating* cache probe: it does not materialize any text. If the layout
    /// is not already cached in either the current frame or previous frame, returns `None`.
    ///
    /// Contract (caller enforced):
    /// - Same `text_hash` implies identical text content (collision risk accepted by caller).
    /// - `text_len` should be the UTF-8 byte length of the text (helps reduce accidental collisions).
    pub fn try_layout_line_by_hash(
        &self,
        text_hash: u64,
        text_len: usize,
        font_size: Pixels,
        runs: &[FontRun],
        force_width: Option<Pixels>,
    ) -> Option<Arc<LineLayout>> {
        let key_ref = HashedCacheKeyRef {
            text_hash,
            text_len,
            font_size,
            runs,
            wrap_width: None,
            force_width,
        };

        let current_frame = self.current_frame.read();
        if let Some((_, layout)) = current_frame.lines_by_hash.iter().find(|(key, _)| {
            HashedCacheKeyRef {
                text_hash: key.text_hash,
                text_len: key.text_len,
                font_size: key.font_size,
                runs: key.runs.as_slice(),
                wrap_width: key.wrap_width,
                force_width: key.force_width,
            } == key_ref
        }) {
            return Some(layout.clone());
        }

        let previous_frame = self.previous_frame.lock();
        if let Some((_, layout)) = previous_frame.lines_by_hash.iter().find(|(key, _)| {
            HashedCacheKeyRef {
                text_hash: key.text_hash,
                text_len: key.text_len,
                font_size: key.font_size,
                runs: key.runs.as_slice(),
                wrap_width: key.wrap_width,
                force_width: key.force_width,
            } == key_ref
        }) {
            return Some(layout.clone());
        }

        None
    }

    /// Layout a line of text using a caller-provided content hash as the cache key.
    ///
    /// This enables cache hits without materializing a contiguous `SharedString` for `text`.
    /// If the cache misses, `materialize_text` is invoked to produce the `SharedString` for shaping.
    ///
    /// Contract (caller enforced):
    /// - Same `text_hash` implies identical text content (collision risk accepted by caller).
    /// - `text_len` should be the UTF-8 byte length of the text (helps reduce accidental collisions).
    pub fn layout_line_by_hash(
        &self,
        text_hash: u64,
        text_len: usize,
        font_size: Pixels,
        runs: &[FontRun],
        force_width: Option<Pixels>,
        materialize_text: impl FnOnce() -> SharedString,
    ) -> Arc<LineLayout> {
        let key_ref = HashedCacheKeyRef {
            text_hash,
            text_len,
            font_size,
            runs,
            wrap_width: None,
            force_width,
        };

        // Fast path: already cached (no allocation).
        let current_frame = self.current_frame.upgradable_read();
        if let Some((_, layout)) = current_frame.lines_by_hash.iter().find(|(key, _)| {
            HashedCacheKeyRef {
                text_hash: key.text_hash,
                text_len: key.text_len,
                font_size: key.font_size,
                runs: key.runs.as_slice(),
                wrap_width: key.wrap_width,
                force_width: key.force_width,
            } == key_ref
        }) {
            return layout.clone();
        }

        let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);

        // Try to reuse from previous frame without allocating; do a linear scan to find a matching key.
        // (We avoid `drain()` here because it would eagerly move all entries.)
        let mut previous_frame = self.previous_frame.lock();
        if let Some(existing_key) = previous_frame
            .used_lines_by_hash
            .iter()
            .find(|key| {
                HashedCacheKeyRef {
                    text_hash: key.text_hash,
                    text_len: key.text_len,
                    font_size: key.font_size,
                    runs: key.runs.as_slice(),
                    wrap_width: key.wrap_width,
                    force_width: key.force_width,
                } == key_ref
            })
            .cloned()
        {
            if let Some((key, layout)) = previous_frame.lines_by_hash.remove_entry(&existing_key) {
                current_frame
                    .lines_by_hash
                    .insert(key.clone(), layout.clone());
                current_frame.used_lines_by_hash.push(key);
                return layout;
            }
        }

        let text = materialize_text();
        let mut layout = self
            .platform_text_system
            .layout_line(&text, font_size, runs);

        if let Some(force_width) = force_width {
            apply_force_width_to_layout(&mut layout, force_width);
        }

        let key = Arc::new(HashedCacheKey {
            text_hash,
            text_len,
            font_size,
            runs: SmallVec::from(runs),
            wrap_width: None,
            force_width,
        });
        let layout = Arc::new(layout);
        current_frame
            .lines_by_hash
            .insert(key.clone(), layout.clone());
        current_frame.used_lines_by_hash.push(key);
        layout
    }
}

// Combining marks (e.g. Thai vowel signs, Arabic diacritics) are shaped by
// HarfBuzz at the same x position as their base character. The force-width
// loop must not advance the cell counter for these zero-advance glyphs,
// otherwise they get displaced into the next cell. We detect them by checking
// whether shaped x has advanced by at least half a cell beyond the last base.
fn apply_force_width_to_layout(layout: &mut LineLayout, force_width: Pixels) {
    let mut glyph_pos: usize = 0;
    // NEG_INFINITY ensures the first glyph is always classified as a base.
    let mut last_base_shaped_x = px(f32::NEG_INFINITY);
    let mut last_base_actual_x = px(0.);
    let mut last_base_index = None;

    for run in layout.runs.iter_mut() {
        for glyph in run.glyphs.iter_mut() {
            let shaped_x = glyph.position.x;
            let shaped_x_advanced = shaped_x > last_base_shaped_x;
            let starts_new_cluster = last_base_index != Some(glyph.index);
            let advanced_far_enough = shaped_x > last_base_shaped_x + force_width * 0.5;

            if shaped_x_advanced && (starts_new_cluster || advanced_far_enough) {
                let forced_x = glyph_pos * force_width;
                if (shaped_x - forced_x).abs() > px(1.) {
                    glyph.position.x = forced_x;
                }
                last_base_shaped_x = shaped_x;
                last_base_actual_x = glyph.position.x;
                last_base_index = Some(glyph.index);
                glyph_pos += 1;
            } else {
                glyph.position.x = last_base_actual_x + (shaped_x - last_base_shaped_x);
            }
        }
    }
}

/// A run of text with a single font.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[expect(missing_docs)]
pub struct FontRun {
    pub len: usize,
    pub font_id: FontId,
    pub letter_spacing: Option<Pixels>,
}

impl Hash for FontRun {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len.hash(state);
        self.font_id.hash(state);
        self.letter_spacing
            .map(Pixels::as_f32)
            .map(|value| {
                if value == 0.0 {
                    0.0f32.to_bits()
                } else {
                    value.to_bits()
                }
            })
            .hash(state);
    }
}

trait AsCacheKeyRef {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_>;
}

#[derive(Clone, Debug, Eq)]
struct CacheKey {
    text: SharedString,
    font_size: Pixels,
    runs: SmallVec<[FontRun; 1]>,
    wrap_width: Option<Pixels>,
    force_width: Option<Pixels>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
struct CacheKeyRef<'a> {
    text: &'a str,
    font_size: Pixels,
    runs: &'a [FontRun],
    wrap_width: Option<Pixels>,
    force_width: Option<Pixels>,
}

#[derive(Clone, Debug)]
struct HashedCacheKey {
    text_hash: u64,
    text_len: usize,
    font_size: Pixels,
    runs: SmallVec<[FontRun; 1]>,
    wrap_width: Option<Pixels>,
    force_width: Option<Pixels>,
}

#[derive(Copy, Clone)]
struct HashedCacheKeyRef<'a> {
    text_hash: u64,
    text_len: usize,
    font_size: Pixels,
    runs: &'a [FontRun],
    wrap_width: Option<Pixels>,
    force_width: Option<Pixels>,
}

impl PartialEq for dyn AsCacheKeyRef + '_ {
    fn eq(&self, other: &dyn AsCacheKeyRef) -> bool {
        self.as_cache_key_ref() == other.as_cache_key_ref()
    }
}

impl PartialEq for HashedCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.text_hash == other.text_hash
            && self.text_len == other.text_len
            && self.font_size == other.font_size
            && self.runs.as_slice() == other.runs.as_slice()
            && self.wrap_width == other.wrap_width
            && self.force_width == other.force_width
    }
}

impl Eq for HashedCacheKey {}

impl Hash for HashedCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text_hash.hash(state);
        self.text_len.hash(state);
        self.font_size.hash(state);
        self.runs.as_slice().hash(state);
        self.wrap_width.hash(state);
        self.force_width.hash(state);
    }
}

impl PartialEq for HashedCacheKeyRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.text_hash == other.text_hash
            && self.text_len == other.text_len
            && self.font_size == other.font_size
            && self.runs == other.runs
            && self.wrap_width == other.wrap_width
            && self.force_width == other.force_width
    }
}

impl Eq for HashedCacheKeyRef<'_> {}

impl Hash for HashedCacheKeyRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text_hash.hash(state);
        self.text_len.hash(state);
        self.font_size.hash(state);
        self.runs.hash(state);
        self.wrap_width.hash(state);
        self.force_width.hash(state);
    }
}

impl Eq for dyn AsCacheKeyRef + '_ {}

impl Hash for dyn AsCacheKeyRef + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_cache_key_ref().hash(state)
    }
}

impl AsCacheKeyRef for CacheKey {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_> {
        CacheKeyRef {
            text: &self.text,
            font_size: self.font_size,
            runs: self.runs.as_slice(),
            wrap_width: self.wrap_width,
            force_width: self.force_width,
        }
    }
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.as_cache_key_ref().eq(&other.as_cache_key_ref())
    }
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_cache_key_ref().hash(state);
    }
}

impl<'a> Borrow<dyn AsCacheKeyRef + 'a> for Arc<CacheKey> {
    fn borrow(&self) -> &(dyn AsCacheKeyRef + 'a) {
        self.as_ref() as &dyn AsCacheKeyRef
    }
}

impl AsCacheKeyRef for CacheKeyRef<'_> {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_> {
        *self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GlyphId, NoopTextSystem};
    use gpui_macros::perf;
    use std::{hint::black_box, sync::Arc};

    fn glyph_at(x: f32, index: usize) -> ShapedGlyph {
        ShapedGlyph {
            id: GlyphId(0),
            position: point(px(x), px(0.)),
            index,
            is_emoji: false,
        }
    }

    /// Ten 10px glyphs, "abc efg ij", wrapped at its two spaces: lines of 40,
    /// 40 and 20px, which alignment measures as 30, 30 and 20px since the
    /// spaces the first two lines were wrapped at hang.
    fn wrapped_ten_glyphs() -> WrappedLineLayout {
        let mut layout = make_layout((0..10).map(|ix| glyph_at(ix as f32 * 10., ix)).collect());
        layout.len = 10;
        WrappedLineLayout {
            unwrapped_layout: Arc::new(layout),
            wrap_boundaries: SmallVec::from_slice(&[
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
            ]),
            wrap_width: Some(px(60.)),
        }
    }

    /// `text` in 10px glyphs, wrapped to `wrap_width`.
    fn wrapped(text: &str, wrap_width: f32) -> WrappedLineLayout {
        let glyphs = text
            .char_indices()
            .map(|(ix, _)| glyph_at(ix as f32 * 10., ix))
            .collect();
        let mut layout = make_layout(glyphs);
        layout.width = px(text.len() as f32 * 10.);
        layout.len = text.len();
        let wrap_boundaries = layout.compute_wrap_boundaries(text, px(wrap_width), None);
        WrappedLineLayout {
            unwrapped_layout: Arc::new(layout),
            wrap_boundaries,
            wrap_width: Some(px(wrap_width)),
        }
    }

    #[test]
    fn wrap_boundaries_record_where_the_spaces_wrapped_at_begin() {
        let boundary = |glyph_ix, trailing_whitespace_x| WrapBoundary {
            run_ix: 0,
            glyph_ix,
            trailing_whitespace_x: px(trailing_whitespace_x),
        };
        // Breaking before "def" and "gh" leaves the space before each at the end of its line.
        assert_eq!(
            wrapped("abc def gh", 45.).wrap_boundaries.as_slice(),
            &[boundary(4, 30.), boundary(8, 70.)]
        );
        // A line broken inside a word ends where the next line starts.
        assert_eq!(
            wrapped("abcdefgh", 35.).wrap_boundaries.as_slice(),
            &[boundary(3, 30.), boundary(6, 60.)]
        );
        // Spaces hang back no further than the start of their own line.
        assert_eq!(
            wrapped("a      b", 35.).wrap_boundaries.as_slice(),
            &[boundary(3, 10.), boundary(6, 30.)]
        );
    }

    #[test]
    fn trailing_spaces_are_followed_back_across_shaped_runs() {
        // "ab  cd" with a run break between the two spaces: the first space
        // ends run 0, the second starts run 1 along with "cd".
        let text = "ab  cd";
        let glyphs: Vec<_> = text
            .char_indices()
            .map(|(ix, _)| glyph_at(ix as f32 * 10., ix))
            .collect();
        let (first_run, second_run) = glyphs.split_at(3);
        let layout = LineLayout {
            font_size: px(16.),
            width: px(60.),
            ascent: px(12.),
            descent: px(4.),
            runs: vec![
                ShapedRun {
                    font_id: FontId(0),
                    glyphs: first_run.to_vec(),
                },
                ShapedRun {
                    font_id: FontId(1),
                    glyphs: second_run.to_vec(),
                },
            ],
            len: text.len(),
        };
        assert_eq!(
            layout
                .compute_wrap_boundaries(text, px(45.), None)
                .as_slice(),
            &[WrapBoundary {
                run_ix: 1,
                glyph_ix: 1,
                trailing_whitespace_x: px(20.),
            }]
        );
    }

    #[test]
    fn line_extents_iterate_as_they_index() {
        let layout = wrapped_ten_glyphs();
        let by_index: Vec<_> = (0..3)
            .map(|line_ix| {
                layout
                    .unwrapped_layout
                    .wrapped_line_extent(&layout.wrap_boundaries, line_ix)
            })
            .collect();
        let iterated: Vec<_> = layout
            .unwrapped_layout
            .wrapped_line_extents(&layout.wrap_boundaries)
            .collect();
        assert_eq!(iterated, by_index);
        assert_eq!(
            iterated[1],
            WrappedLineExtent {
                start: px(40.),
                visible_end: px(70.),
                end: px(80.),
            }
        );
    }

    #[test]
    fn wrapped_line_offsets_follow_the_alignment() {
        let layout = wrapped_ten_glyphs();
        let offsets = |align, width| {
            (0..4)
                .map(|line| layout.wrapped_line_offset(line, align, px(width)))
                .collect::<Vec<_>>()
        };
        assert_eq!(offsets(TextAlign::Left, 60.), vec![px(0.); 4]);
        // The first two lines align as 30px: the spaces they wrap at hang.
        assert_eq!(
            offsets(TextAlign::Center, 60.),
            vec![px(15.), px(15.), px(20.), px(0.)]
        );
        assert_eq!(
            offsets(TextAlign::Right, 60.),
            vec![px(30.), px(30.), px(40.), px(0.)]
        );
        // A line wider than its box overhangs it on both sides.
        assert_eq!(
            layout.wrapped_line_offset(0, TextAlign::Center, px(20.)),
            px(-5.)
        );
    }

    #[test]
    fn aligned_hit_testing_finds_the_glyph_painted_there() {
        let layout = wrapped_ten_glyphs();
        let line_height = px(20.);
        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            for index in 0..=10 {
                let position = layout
                    .position_for_index_aligned(index, line_height, align, px(60.))
                    .expect("every index has a position");
                let hit = layout.index_for_position_aligned(
                    point(position.x, position.y + line_height / 2.),
                    line_height,
                    align,
                    px(60.),
                );
                assert_eq!(
                    hit.unwrap_or_else(|clamped| clamped),
                    index,
                    "{align:?}: index {index} at {position:?} resolves to {hit:?}"
                );
            }
        }

        // The second glyph of the centred middle line is painted at its
        // offset plus one glyph.
        assert_eq!(
            layout.position_for_index_aligned(5, line_height, TextAlign::Center, px(60.)),
            Some(point(px(25.), px(20.)))
        );
        assert_eq!(
            layout.index_for_position_aligned(
                point(px(30.), px(30.)),
                line_height,
                TextAlign::Center,
                px(60.)
            ),
            Ok(5)
        );
        // The closest boundary to a position can be the glyph after it.
        assert_eq!(
            layout.closest_index_for_position_aligned(
                point(px(32.), px(30.)),
                line_height,
                TextAlign::Center,
                px(60.)
            ),
            Ok(6)
        );
        // A wrap boundary stays at the end of the line above, past the space
        // that line hangs.
        assert_eq!(
            layout.position_for_index_aligned(4, line_height, TextAlign::Center, px(60.)),
            Some(point(px(55.), px(0.)))
        );
    }

    #[test]
    fn clicks_beside_an_aligned_line_clamp_to_its_ends() {
        let layout = wrapped_ten_glyphs();
        let hit = |x: f32, y: f32| {
            layout.index_for_position_aligned(
                point(px(x), px(y)),
                px(20.),
                TextAlign::Center,
                px(60.),
            )
        };
        // The middle line is painted from 15 to 55, its hanging space included.
        assert_eq!(hit(10., 30.), Err(4));
        assert_eq!(hit(50., 30.), Ok(7));
        assert_eq!(hit(56., 30.), Err(8));
        assert_eq!(hit(200., 200.), Err(0));
        // Unaligned hit-testing is unchanged.
        assert_eq!(
            layout.index_for_position(point(px(5.), px(30.)), px(20.)),
            Ok(4)
        );
    }

    fn make_layout(glyphs: Vec<ShapedGlyph>) -> LineLayout {
        LineLayout {
            font_size: px(16.),
            width: px(100.),
            ascent: px(12.),
            descent: px(4.),
            runs: vec![ShapedRun {
                font_id: FontId(0),
                glyphs,
            }],
            len: 0,
        }
    }

    #[perf(important)]
    fn perf_hashed_line_layout_previous_frame_cache_hits() {
        const LINE_COUNT: usize = 4_096;
        const REPEATS: usize = 8;
        const TEXT_SUFFIX: &str = "abcdefghijklmnop";

        let cache = LineLayoutCache::new(Arc::new(NoopTextSystem::new()));
        let text_len = "line-0000-abcdefghijklmnop".len();
        let runs = [FontRun {
            len: text_len,
            font_id: FontId(0),
            letter_spacing: None,
        }];

        for ix in 0..LINE_COUNT {
            cache.layout_line_by_hash(ix as u64, text_len, px(16.), &runs, None, || {
                SharedString::from(format!("line-{ix:04}-{TEXT_SUFFIX}"))
            });
        }

        let mut total_width = px(0.);
        for _ in 0..REPEATS {
            cache.finish_frame();
            for ix in 0..LINE_COUNT {
                let layout =
                    cache.layout_line_by_hash(ix as u64, text_len, px(16.), &runs, None, || {
                        panic!("hashed line layout cache should hit")
                    });
                total_width += layout.width;
            }
        }

        black_box(total_width);
    }

    fn glyph_x_positions(layout: &LineLayout) -> Vec<f32> {
        layout.runs[0]
            .glyphs
            .iter()
            .map(|g| f32::from(g.position.x))
            .collect()
    }

    /// Lays out and wraps 1024 distinct lines of about 230 glyphs into 200px,
    /// so every line is a cache miss that computes its wrap boundaries.
    #[perf(important)]
    fn perf_wrap_many_distinct_lines() {
        const LINE_COUNT: usize = 1_024;
        let cache = LineLayoutCache::new(Arc::new(NoopTextSystem::new()));
        let mut total_boundaries = 0;
        for ix in 0..LINE_COUNT {
            let text = format!("line {ix:04} {}", "lorem ipsum dolor sit amet ".repeat(8));
            let runs = [FontRun {
                len: text.len(),
                font_id: FontId(0),
                letter_spacing: None,
            }];
            let layout = cache.layout_wrapped_line(text, px(16.), &runs, Some(px(200.)), None);
            total_boundaries += layout.wrap_boundaries.len();
        }
        black_box(total_boundaries);
    }

    #[perf(important)]
    fn perf_wrapped_line_position_for_index_many_soft_wraps() {
        const LINE_LEN: usize = 8_192;
        const WRAP_STEP: usize = 8;

        let glyphs = (0..LINE_LEN)
            .map(|ix| glyph_at(ix as f32, ix))
            .collect::<Vec<_>>();
        let wrap_boundaries = (WRAP_STEP..LINE_LEN)
            .step_by(WRAP_STEP)
            .map(|glyph_ix| WrapBoundary {
                run_ix: 0,
                glyph_ix,
                trailing_whitespace_x: px(glyph_ix as f32),
            })
            .collect();
        let layout = WrappedLineLayout {
            unwrapped_layout: Arc::new(LineLayout {
                font_size: px(16.),
                width: px(LINE_LEN as f32),
                ascent: px(12.),
                descent: px(4.),
                runs: vec![ShapedRun {
                    font_id: FontId(0),
                    glyphs,
                }],
                len: LINE_LEN,
            }),
            wrap_boundaries,
            wrap_width: Some(px(WRAP_STEP as f32)),
        };

        let mut total_x = px(0.);
        for ix in (0..LINE_LEN).step_by(17) {
            let position = layout.position_for_index(ix, px(16.)).unwrap();
            total_x += position.x;
        }

        black_box(total_x);
    }

    #[test]
    fn test_force_width_latin_unchanged() {
        let cell_width = px(8.);
        let mut layout = make_layout(vec![glyph_at(0., 0), glyph_at(8., 1), glyph_at(16., 2)]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 8., 16.]);
    }

    #[test]
    fn test_force_width_combining_marks_not_advanced() {
        let cell_width = px(8.);
        // Simulates Thai "กี" — base consonant at x=0, combining vowel also at x=0
        let mut layout = make_layout(vec![
            glyph_at(0., 0), // ก (base)
            glyph_at(0., 3), // ี (combining mark, same x)
        ]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 0.]);
    }

    #[test]
    fn test_force_width_base_after_combining_mark() {
        let cell_width = px(8.);
        let mut layout = make_layout(vec![glyph_at(0., 0), glyph_at(0., 3), glyph_at(8., 6)]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 0., 8.]);
    }

    #[test]
    fn test_force_width_multiple_combining_marks() {
        let cell_width = px(8.);
        // Simulates "ก้" — base + vowel + tone mark (two combining marks stacked)
        let mut layout = make_layout(vec![
            glyph_at(0., 0), // ก (base)
            glyph_at(0., 3), // vowel (combining)
            glyph_at(0., 6), // tone mark (combining)
            glyph_at(8., 9), // next base
        ]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 0., 0., 8.]);
    }

    #[test]
    fn test_force_width_corrects_drifted_base_positions() {
        let cell_width = px(8.);
        // Font metrics don't perfectly match cell grid — glyphs drift >1px from cell boundary
        let mut layout = make_layout(vec![
            glyph_at(0.5, 0),  // within 1px tolerance, kept as-is
            glyph_at(10.2, 1), // >1px off from 8.0, corrected
            glyph_at(19.8, 2), // >1px off from 16.0, corrected
        ]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0.5, 8., 16.]);
    }

    #[test]
    fn test_force_width_combining_mark_after_within_tolerance_base() {
        let cell_width = px(8.);
        // Base glyph is within 1px of grid so it keeps its shaped position.
        // The combining mark must align to the base's actual position, not the grid slot.
        let mut layout = make_layout(vec![glyph_at(0.5, 0), glyph_at(0.5, 3)]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0.5, 0.5]);
    }
}
