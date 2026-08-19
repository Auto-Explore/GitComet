//! A persistent rope: the storage layer for editable text.
//!
//! The design follows Zed's `rope` crate — a B-tree of small text chunks whose
//! summaries form a monoid, so every question we ask about a document is
//! answered by a tree descent rather than a scan. The implementation here is
//! our own, written against the Apache-2.0 `sum_tree` that `gpui` re-exports;
//! no Zed code is copied.
//!
//! Why this shape, in terms of what it replaces:
//!
//! - **No line-start array.** `TextSummary::lines` is a dimension of the tree,
//!   so `offset_to_point`/`point_to_offset` are O(log n) descents. The piece
//!   table's `LineIndex` had to be rebuilt on essentially every edit because a
//!   live snapshot kept its `Arc` shared.
//! - **No whole-document materialization.** Text is read through
//!   [`Rope::chunks_in_range`], which touches only the bytes asked for. Edits
//!   rebuild O(log n) nodes and share every untouched subtree, so a keystroke
//!   costs the same in a 100 MB buffer as in a 1 KB one.
//! - **O(1) widest line.** `longest_row` is carried in the summary and merged
//!   across chunk boundaries, so the horizontal scroll bound is a root read
//!   instead of a per-line measurement.
//! - **O(1) clone.** `SumTree` is `Arc`-backed and copy-on-write, so snapshots
//!   are an atomic increment.
//!
//! Offsets are byte offsets throughout. A [`Point`] is a zero-based row plus a
//! *byte* column within that row.

// A storage primitive: it deliberately offers the full complement of
// operations a rope is expected to have, a few of which have no caller in
// the tree yet. They are covered by this module's own tests.
#![allow(dead_code)]

use gpui::sum_tree::{Bias, ContextLessSummary, Dimension, Dimensions, Item, SumTree};
use memchr::{memchr, memrchr};
use std::fmt;
use std::ops::{Add, AddAssign, Range};
use std::sync::Arc;

/// Largest chunk we will store. Chunks are the granularity at which an edit
/// rewrites text, so this bounds the memcpy an edit performs; it is also the
/// granularity of subtree sharing, so smaller means more nodes.
pub(crate) const MAX_CHUNK_BYTES: usize = 512;
/// Appends top up the final chunk until it reaches [`MAX_CHUNK_BYTES`], so a
/// run of small pushes cannot leave a trail of one-byte chunks.
const CHUNK_TOP_UP_LIMIT: usize = MAX_CHUNK_BYTES;

/// A zero-based row, plus a byte offset within that row.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Point {
    pub row: u32,
    pub column: u32,
}

impl Point {
    pub const ZERO: Self = Self { row: 0, column: 0 };

    pub fn new(row: u32, column: u32) -> Self {
        Self { row, column }
    }
}

impl Add for Point {
    type Output = Self;

    fn add(mut self, other: Self) -> Self {
        self += other;
        self
    }
}

impl AddAssign for Point {
    fn add_assign(&mut self, other: Self) {
        self.row += other.row;
        // A column is only meaningful within its own row: crossing a newline
        // restarts it, staying on the same row extends it.
        if other.row == 0 {
            self.column += other.column;
        } else {
            self.column = other.column;
        }
    }
}

/// An offset counted in UTF-16 code units.
///
/// Carried because the platform input handler speaks UTF-16: without it every
/// caret query has to walk the document from byte zero counting `len_utf16`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OffsetUtf16(pub usize);

/// The monoid summarizing a run of text.
///
/// Every field must be computable from the concatenation of two summaries
/// without re-reading the text — that requirement is what forces
/// `first_line_len`/`last_line_len` to exist alongside `longest_row`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextSummary {
    /// Length in bytes.
    pub len: usize,
    /// Length in UTF-16 code units.
    pub len_utf16: usize,
    /// Rows spanned, plus the byte length of the final (possibly partial) row.
    pub lines: Point,
    /// Byte length of the first row, needed to join across a boundary.
    pub first_line_len: u32,
    /// Byte length of the last row, needed to join across a boundary.
    pub last_line_len: u32,
    /// Row with the greatest byte length, relative to the start of this run.
    pub longest_row: u32,
    /// That row's byte length.
    pub longest_row_len: u32,
}

impl TextSummary {
    fn from_str(text: &str) -> Self {
        let mut summary = Self {
            len: text.len(),
            len_utf16: utf16_len(text),
            ..Default::default()
        };

        let bytes = text.as_bytes();
        let mut line_start = 0usize;
        let mut row = 0u32;
        let mut search_from = 0usize;
        while let Some(found) = memchr(b'\n', &bytes[search_from..]) {
            let newline = search_from + found;
            let line_len = (newline - line_start) as u32;
            if row == 0 {
                summary.first_line_len = line_len;
            }
            if line_len > summary.longest_row_len {
                summary.longest_row = row;
                summary.longest_row_len = line_len;
            }
            row += 1;
            line_start = newline + 1;
            search_from = line_start;
        }

        let last_line_len = (text.len() - line_start) as u32;
        if row == 0 {
            summary.first_line_len = last_line_len;
        }
        if last_line_len > summary.longest_row_len {
            summary.longest_row = row;
            summary.longest_row_len = last_line_len;
        }
        summary.last_line_len = last_line_len;
        summary.lines = Point::new(row, last_line_len);
        summary
    }
}

impl ContextLessSummary for TextSummary {
    fn zero() -> Self {
        Self::default()
    }

    fn add_summary(&mut self, other: &Self) {
        // Order matters: the joined-row and first-line updates both read state
        // that the `lines` update below invalidates.
        let joined_len = self.last_line_len + other.first_line_len;
        if joined_len > self.longest_row_len {
            self.longest_row = self.lines.row;
            self.longest_row_len = joined_len;
        }
        if other.longest_row_len > self.longest_row_len {
            self.longest_row = self.lines.row + other.longest_row;
            self.longest_row_len = other.longest_row_len;
        }

        if self.lines.row == 0 {
            self.first_line_len += other.first_line_len;
        }
        if other.lines.row == 0 {
            self.last_line_len += other.last_line_len;
        } else {
            self.last_line_len = other.last_line_len;
        }

        self.lines += other.lines;
        self.len += other.len;
        self.len_utf16 += other.len_utf16;
    }
}

impl Dimension<'_, TextSummary> for usize {
    fn zero(_: ()) -> Self {
        0
    }

    fn add_summary(&mut self, summary: &TextSummary, _: ()) {
        *self += summary.len;
    }
}

impl Dimension<'_, TextSummary> for Point {
    fn zero(_: ()) -> Self {
        Point::ZERO
    }

    fn add_summary(&mut self, summary: &TextSummary, _: ()) {
        *self += summary.lines;
    }
}

impl Dimension<'_, TextSummary> for OffsetUtf16 {
    fn zero(_: ()) -> Self {
        OffsetUtf16(0)
    }

    fn add_summary(&mut self, summary: &TextSummary, _: ()) {
        self.0 += summary.len_utf16;
    }
}

// `sum_tree` already provides `SeekTarget<S, Dimensions<D1, ..>> for D1`, so a
// `usize`/`Point`/`OffsetUtf16` target seeks a paired dimension out of the box.

/// One leaf of the tree. `Arc<str>` rather than inline storage so that the
/// copy-on-write clone a `SumTree` performs on every edit is an atomic bump
/// instead of a memcpy of the whole leaf.
#[derive(Clone, Debug)]
struct Chunk(Arc<str>);

impl Item for Chunk {
    type Summary = TextSummary;

    fn summary(&self, _: ()) -> TextSummary {
        TextSummary::from_str(&self.0)
    }
}

/// A persistent, copy-on-write text buffer.
#[derive(Clone, Default)]
pub struct Rope {
    chunks: SumTree<Chunk>,
}

impl Rope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_str(text: &str) -> Self {
        let mut rope = Self::new();
        rope.push(text);
        rope
    }

    pub fn summary(&self) -> &TextSummary {
        self.chunks.summary()
    }

    pub fn len(&self) -> usize {
        self.summary().len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len_utf16(&self) -> usize {
        self.summary().len_utf16
    }

    /// Last position in the document: the final row, and its byte length.
    pub fn max_point(&self) -> Point {
        self.summary().lines
    }

    /// Number of rows, which is one more than the number of newlines.
    pub fn line_count(&self) -> u32 {
        self.summary().lines.row + 1
    }

    /// The row with the greatest byte length. O(1) — a root summary read.
    pub fn longest_row(&self) -> u32 {
        self.summary().longest_row
    }

    /// Byte length of the longest row. O(1).
    pub fn longest_row_len(&self) -> u32 {
        self.summary().longest_row_len
    }

    /// Append `text`, topping up the trailing chunk first so that repeated
    /// small appends do not fragment the tree.
    pub fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let mut remainder = text;
        let trailing = self.chunks.last().map(|chunk| chunk.0.clone());
        if let Some(trailing) = trailing.filter(|text| text.len() < CHUNK_TOP_UP_LIMIT) {
            let spare = CHUNK_TOP_UP_LIMIT - trailing.len();
            let take = floor_char_boundary(remainder, spare.min(remainder.len()));
            if take > 0 {
                let mut merged = String::with_capacity(trailing.len() + take);
                merged.push_str(&trailing);
                merged.push_str(&remainder[..take]);
                remainder = &remainder[take..];

                let head_len = self.len() - trailing.len();
                let mut cursor = self.chunks.cursor::<usize>(());
                let mut head = cursor.slice(&head_len, Bias::Right);
                drop(cursor);
                head.push(Chunk(merged.into()), ());
                self.chunks = head;
            }
        }

        if remainder.is_empty() {
            return;
        }
        self.chunks.extend(
            split_into_chunks(remainder).map(|text| Chunk(Arc::from(text))),
            (),
        );
    }

    pub fn append(&mut self, other: Rope) {
        if other.is_empty() {
            return;
        }
        if self.is_empty() {
            *self = other;
            return;
        }
        // Re-push the seam chunk so the join cannot leave an undersized chunk
        // in the middle of the tree, then splice the rest by subtree.
        let mut chunks = other.chunks.cursor::<usize>(());
        if let Some(first) = other.chunks.first() {
            self.push(&first.0);
            chunks.seek(&first.0.len(), Bias::Right);
        }
        let rest = chunks.suffix();
        drop(chunks);
        self.chunks.append(rest, ());
    }

    /// Replace `range` with `text`. O(log n) plus the cost of the two chunks
    /// straddling the range: every untouched subtree is shared, not copied.
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        let range = self.clip_range(range);
        if range.is_empty() && text.is_empty() {
            return;
        }

        // Everything before the chunk that straddles `range.start` carries over
        // as whole subtrees.
        let mut head_cursor = self.chunks.cursor::<usize>(());
        let mut new_chunks = head_cursor.slice(&range.start, Bias::Right);

        // Rebuild only the straddled text: the head of the chunk containing
        // `range.start`, the replacement, and the tail of the chunk containing
        // `range.end`. A separate cursor for the tail rather than seeking the
        // first one forward — the two can land on the same chunk, and a
        // forward-only seek has no defined behaviour when it does not move.
        let mut seam = String::new();
        if let Some(chunk) = head_cursor.item() {
            seam.push_str(&chunk.0[..range.start - *head_cursor.start()]);
        }
        drop(head_cursor);
        seam.push_str(text);

        let mut tail_cursor = self.chunks.cursor::<usize>(());
        tail_cursor.seek(&range.end, Bias::Left);
        if let Some(chunk) = tail_cursor.item() {
            seam.push_str(&chunk.0[range.end - *tail_cursor.start()..]);
            tail_cursor.next();
        }

        new_chunks.extend(
            split_into_chunks(&seam).map(|text| Chunk(Arc::from(text))),
            (),
        );
        let suffix = tail_cursor.suffix();
        drop(tail_cursor);
        new_chunks.append(suffix, ());
        self.chunks = new_chunks;
    }

    /// The text in `range`, as a sequence of borrowed chunk slices.
    pub fn chunks_in_range(&self, range: Range<usize>) -> Chunks<'_> {
        let range = self.clip_range(range);
        Chunks::new(&self.chunks, range)
    }

    pub fn chunks(&self) -> Chunks<'_> {
        self.chunks_in_range(0..self.len())
    }

    /// Materialize `range`. Callers on a hot path should prefer
    /// [`Rope::chunks_in_range`]; this exists for the many places that need a
    /// small, bounded slice (one visible row, a word under the caret).
    pub fn text_for_range(&self, range: Range<usize>) -> String {
        let range = self.clip_range(range);
        let mut out = String::with_capacity(range.len());
        for chunk in self.chunks_in_range(range) {
            out.push_str(chunk);
        }
        out
    }

    /// Byte offset of every row start, including row 0.
    ///
    /// A rope answers line questions by descent and has no such array; this
    /// builds one for consumers that still take `&[usize]`. O(document), so it
    /// is something to migrate callers *off*, not a primitive to reach for.
    pub fn line_start_offsets(&self) -> Vec<usize> {
        let mut starts = Vec::with_capacity(self.line_count() as usize);
        starts.push(0);
        let mut base = 0usize;
        for chunk in self.chunks() {
            for newline in memchr::memchr_iter(b'\n', chunk.as_bytes()) {
                starts.push(base + newline + 1);
            }
            base += chunk.len();
        }
        starts
    }

    /// Byte range of `row`, excluding its line terminator.
    pub fn line_range(&self, row: u32) -> Range<usize> {
        let start = self.point_to_offset(Point::new(row, 0));
        let end = start + self.line_len(row) as usize;
        start..end
    }

    /// Byte length of `row`, excluding its line terminator.
    pub fn line_len(&self, row: u32) -> u32 {
        let max_point = self.max_point();
        if row >= max_point.row {
            return if row == max_point.row {
                max_point.column
            } else {
                0
            };
        }
        let start = self.point_to_offset(Point::new(row, 0));
        let next = self.point_to_offset(Point::new(row + 1, 0));
        // `next` includes the newline that ended this row.
        (next - start).saturating_sub(1) as u32
    }

    pub fn line_text(&self, row: u32) -> String {
        self.text_for_range(self.line_range(row))
    }

    pub fn offset_to_point(&self, offset: usize) -> Point {
        let offset = offset.min(self.len());
        let (start, _, chunk) =
            self.chunks
                .find::<Dimensions<usize, Point>, _>((), &offset, Bias::Left);
        let mut point = start.1;
        if let Some(chunk) = chunk {
            point += str_offset_to_point(&chunk.0, offset - start.0);
        }
        point
    }

    /// Offset of `point`, clamped into the document.
    ///
    /// Deliberately does not call [`Rope::clip_point`]: that would recurse
    /// through `line_len` back into here. Clamping falls out anyway — a row
    /// past the end seeks to the tree end, and a column past the row end is
    /// clamped against the newline inside the landing chunk.
    pub fn point_to_offset(&self, point: Point) -> usize {
        let (start, _, chunk) =
            self.chunks
                .find::<Dimensions<Point, usize>, _>((), &point, Bias::Left);
        let mut offset = start.1;
        if let Some(chunk) = chunk {
            offset += str_point_to_offset(&chunk.0, point_sub(point, start.0));
        }
        offset
    }

    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        // Clip to a boundary first: the chunk slice below is a `&str` index, so
        // a mid-character offset would panic rather than answer. The caret can
        // legitimately arrive here mid-character (an IME marked range, an
        // offset carried across an edit), so this has to be total.
        let offset = self.clip_offset(offset.min(self.len()), Bias::Left);
        let (start, _, chunk) =
            self.chunks
                .find::<Dimensions<usize, OffsetUtf16>, _>((), &offset, Bias::Left);
        let mut utf16 = start.1.0;
        if let Some(chunk) = chunk {
            utf16 += utf16_len(&chunk.0[..offset - start.0]);
        }
        utf16
    }

    pub fn offset_from_utf16(&self, target: usize) -> usize {
        let target = OffsetUtf16(target.min(self.len_utf16()));
        let (start, _, chunk) =
            self.chunks
                .find::<Dimensions<OffsetUtf16, usize>, _>((), &target, Bias::Left);
        let mut offset = start.1;
        if let Some(chunk) = chunk {
            offset += utf16_to_utf8_offset(&chunk.0, target.0 - start.0.0);
        }
        offset
    }

    pub fn is_char_boundary(&self, offset: usize) -> bool {
        if offset == 0 || offset == self.len() {
            return true;
        }
        if offset > self.len() {
            return false;
        }
        let (start, _, chunk) = self.chunks.find::<usize, _>((), &offset, Bias::Left);
        chunk.is_some_and(|chunk| chunk.0.is_char_boundary(offset - start))
    }

    /// Move `offset` to the nearest char boundary in the direction of `bias`.
    pub fn clip_offset(&self, offset: usize, bias: Bias) -> usize {
        let mut offset = offset.min(self.len());
        if self.is_char_boundary(offset) {
            return offset;
        }
        match bias {
            Bias::Left => {
                while offset > 0 && !self.is_char_boundary(offset) {
                    offset -= 1;
                }
                offset
            }
            Bias::Right => {
                let len = self.len();
                while offset < len && !self.is_char_boundary(offset) {
                    offset += 1;
                }
                offset
            }
        }
    }

    /// Clamp a point into the document, and its column onto the row.
    pub fn clip_point(&self, point: Point) -> Point {
        let max_point = self.max_point();
        if point.row > max_point.row {
            return max_point;
        }
        let line_len = self.line_len(point.row);
        Point::new(point.row, point.column.min(line_len))
    }

    /// The chunk containing `offset`, from `offset` up to `limit` or the end of
    /// that chunk, whichever comes first.
    ///
    /// This is the primitive tree-sitter's chunked reader wants: it asks for
    /// bytes at an offset and is happy with however many it gets, so a parse
    /// never needs the document as one buffer.
    /// Bytes starting at exactly `offset`, at most up to `limit`, from a single
    /// chunk. Empty when `offset >= limit`.
    ///
    /// Byte-exact on both ends, which `chunks_in_range` deliberately is not:
    /// that widens a range to whole characters so it can hand back `&str`. A
    /// byte reader must never be given bytes it has already consumed — feeding
    /// tree-sitter a slice that begins before `offset` double-counts them and
    /// shifts every node offset in the resulting tree — nor bytes past `limit`,
    /// which for the masked reader would leak real text into a blanked span.
    pub fn bytes_at(&self, offset: usize, limit: usize) -> &[u8] {
        let limit = limit.min(self.len());
        if offset >= limit {
            return &[];
        }
        let start = self.clip_offset(offset, Bias::Left);
        let chunk = self.chunks_in_range(start..limit).next().unwrap_or("");
        let bytes = &chunk.as_bytes()[(offset - start).min(chunk.len())..];
        &bytes[..bytes.len().min(limit - offset)]
    }

    /// Identity of each chunk's backing allocation, for tests that assert
    /// copy-on-write sharing rather than timing it.
    #[cfg(test)]
    fn chunk_sizes_for_test(&self) -> Vec<usize> {
        self.chunks.iter().map(|chunk| chunk.0.len()).collect()
    }

    #[cfg(test)]
    fn chunk_identities(&self) -> Vec<*const u8> {
        self.chunks.iter().map(|chunk| chunk.0.as_ptr()).collect()
    }

    /// Clamp `range` to the document *and* to character boundaries.
    ///
    /// Every chunk read hands out `&str`, so a range that splits a multi-byte
    /// character has no valid answer — slicing it would panic. Widening is the
    /// only coherent repair: the start moves left and the end moves right, so
    /// the range covers whole characters and never less than what was asked
    /// for. `replace` inherits the same rule, because half a character cannot
    /// be replaced either.
    ///
    /// Callers that hold boundary-aligned offsets (all of them today —
    /// [`crate::kit::text_model::TextModel`] clips before it gets here) are
    /// unaffected; this only turns a panic into a sane result for the ones that
    /// do not.
    fn clip_range(&self, range: Range<usize>) -> Range<usize> {
        let len = self.len();
        let start = self.clip_offset(range.start.min(len), Bias::Left);
        let end = self.clip_offset(range.end.min(len).max(start), Bias::Right);
        start..end.max(start)
    }
}

impl fmt::Debug for Rope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rope")
            .field("len", &self.len())
            .field("lines", &self.max_point())
            .finish()
    }
}

impl fmt::Display for Rope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

impl From<&str> for Rope {
    fn from(text: &str) -> Self {
        Self::from_str(text)
    }
}

impl PartialEq<str> for Rope {
    fn eq(&self, other: &str) -> bool {
        if self.len() != other.len() {
            return false;
        }
        let mut rest = other;
        for chunk in self.chunks() {
            let Some((head, tail)) = rest.split_at_checked(chunk.len()) else {
                return false;
            };
            if head != chunk {
                return false;
            }
            rest = tail;
        }
        rest.is_empty()
    }
}

impl PartialEq<Rope> for Rope {
    fn eq(&self, other: &Rope) -> bool {
        if self.len() != other.len() {
            return false;
        }
        let mut ours = self.chunks();
        let mut theirs = other.chunks();
        let mut left = ours.next().unwrap_or("");
        let mut right = theirs.next().unwrap_or("");
        loop {
            match (left.is_empty(), right.is_empty()) {
                (true, true) => {
                    let (Some(next_left), Some(next_right)) = (ours.next(), theirs.next()) else {
                        return ours.next().is_none() && theirs.next().is_none();
                    };
                    left = next_left;
                    right = next_right;
                }
                (true, false) => match ours.next() {
                    Some(next) => left = next,
                    None => return false,
                },
                (false, true) => match theirs.next() {
                    Some(next) => right = next,
                    None => return false,
                },
                (false, false) => {
                    let shared = left.len().min(right.len());
                    if left.as_bytes()[..shared] != right.as_bytes()[..shared] {
                        return false;
                    }
                    left = &left[shared..];
                    right = &right[shared..];
                }
            }
        }
    }
}

impl Eq for Rope {}

/// Borrowed chunk slices covering a byte range.
pub struct Chunks<'a> {
    cursor: gpui::sum_tree::Cursor<'a, 'static, Chunk, usize>,
    range: Range<usize>,
    done: bool,
}

impl<'a> Chunks<'a> {
    fn new(chunks: &'a SumTree<Chunk>, range: Range<usize>) -> Self {
        let mut cursor = chunks.cursor::<usize>(());
        cursor.seek(&range.start, Bias::Right);
        Self {
            cursor,
            range,
            done: false,
        }
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        loop {
            if self.done || self.range.start >= self.range.end {
                return None;
            }
            let chunk_start = *self.cursor.start();
            let Some(chunk) = self.cursor.item() else {
                self.done = true;
                return None;
            };
            let chunk_end = chunk_start + chunk.0.len();
            let slice_start = self.range.start.saturating_sub(chunk_start);
            let slice_end = (self.range.end.min(chunk_end)) - chunk_start;
            self.cursor.next();
            self.range.start = chunk_end.min(self.range.end);
            if slice_end > slice_start {
                return Some(&chunk.0[slice_start..slice_end]);
            }
        }
    }
}

/// Split `text` into pieces of at most [`MAX_CHUNK_BYTES`], never mid-character.
/// Split `text` into chunks of at most [`MAX_CHUNK_BYTES`], balanced so the
/// last one is not a sliver.
///
/// Taking a greedy 512 bytes at a time would leave the remainder, and an edit
/// rebuilds the straddled chunk plus the inserted text — so a one-character
/// insert into a full chunk produces `[512][1]` and strands that 1-byte chunk
/// in the tree forever. Typing then fragments the rope in proportion to
/// keystrokes: 500 single-character inserts took a 44-chunk document to 544,
/// 500 of them under 64 bytes, each costing an `Arc` allocation and a full
/// `TextSummary` and slowing every descent.
///
/// Splitting into `ceil(len / MAX)` near-equal pieces instead keeps every chunk
/// of a multi-chunk seam at or above ~`MAX / 2`, so repeated edits in one place
/// settle into a stable band rather than accumulating debris.
fn split_into_chunks(text: &str) -> impl Iterator<Item = &str> {
    let pieces = text.len().div_ceil(MAX_CHUNK_BYTES).max(1);
    let target = text.len().div_ceil(pieces).max(1);
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let take = floor_char_boundary(rest, target.min(rest.len()));
        // A single character wider than the target cannot happen (4 bytes max),
        // but guard rather than loop forever if that ever changes.
        let take = take.max(1).min(rest.len());
        let (chunk, remainder) = rest.split_at(take);
        rest = remainder;
        Some(chunk)
    })
}

/// Largest index `<= limit` that sits on a character boundary.
fn floor_char_boundary(text: &str, limit: usize) -> usize {
    let mut index = limit.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub(crate) fn utf16_len(text: &str) -> usize {
    if text.is_ascii() {
        return text.len();
    }
    text.chars().map(char::len_utf16).sum()
}

pub(crate) fn utf16_to_utf8_offset(text: &str, target_utf16: usize) -> usize {
    if text.is_ascii() {
        return target_utf16.min(text.len());
    }
    let mut utf16 = 0;
    for (offset, ch) in text.char_indices() {
        if utf16 >= target_utf16 {
            return offset;
        }
        utf16 += ch.len_utf16();
    }
    text.len()
}

fn str_offset_to_point(text: &str, offset: usize) -> Point {
    let head = &text.as_bytes()[..offset.min(text.len())];
    let row = memchr::memchr_iter(b'\n', head).count() as u32;
    let column = match memrchr(b'\n', head) {
        Some(newline) => (head.len() - newline - 1) as u32,
        None => head.len() as u32,
    };
    Point::new(row, column)
}

fn str_point_to_offset(text: &str, point: Point) -> usize {
    let bytes = text.as_bytes();
    let mut offset = 0;
    for _ in 0..point.row {
        match memchr(b'\n', &bytes[offset..]) {
            Some(found) => offset += found + 1,
            None => return text.len(),
        }
    }
    let line_end = match memchr(b'\n', &bytes[offset..]) {
        Some(found) => offset + found,
        None => text.len(),
    };
    (offset + point.column as usize).min(line_end)
}

/// `left - right`, in the same row-restarts-the-column arithmetic as `AddAssign`.
fn point_sub(left: Point, right: Point) -> Point {
    debug_assert!(left >= right, "point subtraction must not go backwards");
    let row = left.row - right.row;
    let column = if row == 0 {
        left.column - right.column
    } else {
        left.column
    };
    Point::new(row, column)
}

#[cfg(test)]
mod tests;
