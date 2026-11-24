use crate::rope::{Point, Rope};
use gpui_shared_string::SharedString;
use std::borrow::Cow;
use std::ops::{Deref, Range};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// `slice` calls that had to copy a range spanning rope chunks.
    pub static OWNED_SLICES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

static NEXT_MODEL_ID: AtomicU64 = AtomicU64::new(1);

/// The document, plus the caches that let legacy `&str`/`&[usize]` callers keep
/// working while they are migrated onto the windowed accessors.
///
/// Both caches are lazy and both are dropped on every edit. That is deliberate:
/// an edit is O(log n) in the rope, and anything that re-warms a whole-document
/// cache afterwards is a caller that has not been migrated yet. The
/// `no_materialization_tests` suite exists to keep the hot paths off them.
#[derive(Debug)]
struct TextModelCore {
    model_id: u64,
    revision: u64,
    rope: Rope,
    line_starts: OnceLock<Arc<[usize]>>,
    materialized: OnceLock<SharedString>,
}

impl Clone for TextModelCore {
    fn clone(&self) -> Self {
        Self {
            model_id: self.model_id,
            revision: self.revision,
            // O(1): the tree is persistent and shares every node.
            rope: self.rope.clone(),
            // A clone exists because a mutation is coming, which would
            // invalidate both caches anyway, or to be kept as history, which
            // must not pin them.
            line_starts: OnceLock::new(),
            materialized: OnceLock::new(),
        }
    }
}

impl TextModelCore {
    fn materialized(&self) -> &SharedString {
        self.materialized
            .get_or_init(|| SharedString::from(self.rope.to_string()))
    }

    fn materialized_clone(&self) -> SharedString {
        self.materialized().clone()
    }

    fn line_starts(&self) -> &Arc<[usize]> {
        self.line_starts
            .get_or_init(|| Arc::from(self.rope.line_start_offsets()))
    }

    /// True when every byte is a single-byte character, so byte and UTF-16
    /// offsets coincide. O(1) from the summary rather than a stored flag.
    fn is_ascii(&self) -> bool {
        self.rope.len() == self.rope.len_utf16()
    }
}

#[derive(Clone, Debug)]
pub struct TextModel {
    core: Arc<TextModelCore>,
}

#[derive(Clone, Debug)]
pub struct TextModelSnapshot {
    core: Arc<TextModelCore>,
}

impl Default for TextModel {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for TextModelSnapshot {
    /// An empty document with its own identity, so it never compares equal to a
    /// snapshot of real content.
    fn default() -> Self {
        TextModel::new().snapshot()
    }
}

impl TextModel {
    pub fn new() -> Self {
        Self::from_large_text("")
    }

    pub fn from_large_text(text: &str) -> Self {
        let model_id = NEXT_MODEL_ID.fetch_add(1, Ordering::Relaxed).max(1);
        Self {
            core: Arc::new(TextModelCore {
                model_id,
                revision: 1,
                rope: Rope::from_text(text),
                line_starts: OnceLock::new(),
                materialized: OnceLock::new(),
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.core.rope.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[cfg(feature = "benchmarks")]
    pub fn model_id(&self) -> u64 {
        self.core.model_id
    }

    #[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
    pub fn revision(&self) -> u64 {
        self.core.revision
    }

    pub fn as_str(&self) -> &str {
        self.core.materialized().as_ref()
    }

    #[cfg(feature = "benchmarks")]
    pub fn as_shared_string(&self) -> SharedString {
        self.core.materialized_clone()
    }

    pub fn line_starts(&self) -> &[usize] {
        self.core.line_starts().as_ref()
    }

    pub fn snapshot(&self) -> TextModelSnapshot {
        TextModelSnapshot {
            core: Arc::clone(&self.core),
        }
    }

    /// A snapshot sharing the text but none of the whole-document caches, for
    /// history kept across many edits. A stack of [`TextModel::snapshot`]s pins
    /// every revision's flattened copy and line-start table.
    pub fn snapshot_without_caches(&self) -> TextModelSnapshot {
        TextModelSnapshot {
            core: Arc::new(TextModelCore::clone(&self.core)),
        }
    }

    /// Replace the whole document. Mints a fresh `model_id`, so snapshots taken
    /// of the previous contents never compare equal to the new ones.
    pub fn set_text(&mut self, text: &str) {
        *self = Self::from_large_text(text);
    }

    #[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
    pub fn append_large(&mut self, text: &str) -> Range<usize> {
        let start = self.len();
        self.replace_range(start..start, text)
    }

    pub fn is_char_boundary(&self, offset: usize) -> bool {
        self.core.rope.is_char_boundary(offset)
    }

    pub fn clamp_to_char_boundary(&self, offset: usize) -> usize {
        self.core
            .rope
            .clip_offset(offset, gpui_sum_tree::Bias::Left)
    }

    /// Replace `range` with `new_text`, returning the byte range the inserted
    /// text now occupies.
    ///
    /// O(log n) plus the replaced text: the rope shares every untouched subtree,
    /// so this does not scale with the document.
    pub fn replace_range(&mut self, range: Range<usize>, new_text: &str) -> Range<usize> {
        let len = self.len();
        let start = self.clamp_to_char_boundary(range.start.min(len));
        let end = self.clamp_to_char_boundary(range.end.min(len));
        let range = if end < start { end..start } else { start..end };
        if range.is_empty() && new_text.is_empty() {
            return range.start..range.start;
        }

        let core = Arc::make_mut(&mut self.core);
        core.rope.replace(range.clone(), new_text);
        core.revision = core.revision.wrapping_add(1).max(1);
        core.materialized = OnceLock::new();
        core.line_starts = OnceLock::new();

        range.start..range.start.saturating_add(new_text.len())
    }
}

impl AsRef<str> for TextModel {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for TextModel {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl From<&str> for TextModel {
    fn from(value: &str) -> Self {
        Self::from_large_text(value)
    }
}

impl From<String> for TextModel {
    fn from(value: String) -> Self {
        Self::from_large_text(value.as_str())
    }
}

impl From<TextModelSnapshot> for TextModel {
    fn from(snapshot: TextModelSnapshot) -> Self {
        Self {
            core: snapshot.core,
        }
    }
}

impl TextModelSnapshot {
    pub fn len(&self) -> usize {
        self.core.rope.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn model_id(&self) -> u64 {
        self.core.model_id
    }

    pub fn revision(&self) -> u64 {
        self.core.revision
    }

    pub fn as_str(&self) -> &str {
        self.core.materialized().as_ref()
    }

    pub fn as_shared_string(&self) -> SharedString {
        self.core.materialized_clone()
    }

    #[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
    pub fn line_starts(&self) -> &[usize] {
        self.core.line_starts().as_ref()
    }

    pub fn shared_line_starts(&self) -> Arc<[usize]> {
        Arc::clone(self.core.line_starts())
    }

    /// Whether the document has been flattened into one contiguous `String`.
    /// The windowed accessors exist precisely so that hot paths never trip
    /// this, and that is only assertable by observing it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_materialized(&self) -> bool {
        self.core.materialized.get().is_some()
    }

    /// Whether the whole-document line-start array has been built.
    ///
    /// The quieter of the two O(document) caches, and the one worth asserting
    /// separately: it allocates an index proportional to the line count without
    /// copying any text, so a caller that trips it looks innocent next to
    /// [`TextModelSnapshot::is_materialized`] while costing the same order.
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_line_index_built(&self) -> bool {
        self.core.line_starts.get().is_some()
    }

    /// The document itself, for consumers that need to read arbitrary spans
    /// without a contiguous buffer — tree-sitter's chunked parser and query
    /// text provider, above all.
    ///
    /// Cloning is an atomic increment: the returned rope is a snapshot, immune
    /// to later edits, and costs nothing to hold across frames.
    pub fn rope(&self) -> Rope {
        self.core.rope.clone()
    }

    /// Number of rows, which is one more than the number of line breaks. O(1).
    pub fn line_count(&self) -> usize {
        self.core.rope.line_count() as usize
    }

    /// The row containing `offset`. O(log n), and the windowed replacement for
    /// binary-searching a materialized line-start array.
    pub fn row_for_offset(&self, offset: usize) -> usize {
        self.core.rope.offset_to_point(offset.min(self.len())).row as usize
    }

    /// Byte range of `row` *including* its line terminator, i.e. exactly
    /// `line_starts[row]..line_starts[row + 1]`.
    ///
    /// Callers measuring how wide a row is count the terminator as a column, so
    /// they need the span between consecutive line starts rather than
    /// [`TextModelSnapshot::line_range`], which stops before it.
    /// Two descents, not six: only the row *starts* are wanted, so this seeks
    /// them directly instead of asking [`TextModelSnapshot::line_range`] twice
    /// and discarding the row lengths it measures on the way. This is the inner
    /// loop of the content-width cache, which runs once per row.
    pub fn line_range_with_terminator(&self, row: usize) -> Range<usize> {
        let Ok(row) = u32::try_from(row) else {
            let end = self.len();
            return end..end;
        };
        let start = self.core.rope.point_to_offset(Point::new(row, 0));
        let end = if u64::from(row) + 1 < self.core.rope.line_count() as u64 {
            self.core.rope.point_to_offset(Point::new(row + 1, 0))
        } else {
            self.len()
        };
        start..end.max(start)
    }

    /// Byte range of `row`, excluding its line terminator.
    ///
    /// The windowed read primitive: a renderer that only needs the rows on
    /// screen asks for those rows, instead of materializing the document and
    /// indexing into it. O(log n).
    pub fn line_range(&self, row: usize) -> Range<usize> {
        let Ok(row) = u32::try_from(row) else {
            let end = self.len();
            return end..end;
        };
        self.core.rope.line_range(row)
    }

    /// Text of `row`, excluding its line terminator. Borrowed when the row does
    /// not straddle a chunk boundary.
    pub fn line_text(&self, row: usize) -> Cow<'_, str> {
        self.slice(self.line_range(row))
    }

    /// UTF-16 code unit offset for a byte offset.
    ///
    /// The platform input handler speaks UTF-16, so this runs on essentially
    /// every caret query. All-ASCII documents — nearly all of them — answer in
    /// O(1); the rest are an O(log n) descent.
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        if self.core.is_ascii() {
            return offset.min(self.len());
        }
        self.core.rope.offset_to_utf16(offset)
    }

    /// Byte offset for a UTF-16 code unit offset. Inverse of
    /// [`TextModelSnapshot::offset_to_utf16`].
    pub fn offset_from_utf16(&self, target: usize) -> usize {
        if self.core.is_ascii() {
            return target.min(self.len());
        }
        self.core.rope.offset_from_utf16(target)
    }

    fn clamp_offset_to_char_boundary(&self, offset: usize) -> usize {
        self.core
            .rope
            .clip_offset(offset, gpui_sum_tree::Bias::Left)
    }

    fn normalized_char_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clamp_offset_to_char_boundary(range.start.min(self.len()));
        let end = self.clamp_offset_to_char_boundary(range.end.min(self.len()));
        if end < start { end..start } else { start..end }
    }

    /// The characters from `offset` to the end, read chunk by chunk: a walk
    /// that stops early never copies or touches the rest of the document. Both
    /// walks split the text where [`TextModelSnapshot::slice`] does, so an
    /// offset inside a character counts as that character's start.
    pub fn chars_at(&self, offset: usize) -> impl Iterator<Item = char> + '_ {
        self.core
            .rope
            .chunks_in_range(self.normalized_char_range(offset..self.len()))
            .flat_map(str::chars)
    }

    /// The characters before `offset`, nearest first, read the same way.
    pub fn reversed_chars_at(&self, offset: usize) -> impl Iterator<Item = char> + '_ {
        self.core
            .rope
            .reversed_chunks_in_range(self.normalized_char_range(0..offset))
            .flat_map(|chunk| chunk.chars().rev())
    }

    /// The text in `range`. Borrowed when it lives inside a single chunk, which
    /// covers a typical row; owned only when it straddles one.
    pub fn slice(&self, range: Range<usize>) -> Cow<'_, str> {
        let range = self.normalized_char_range(range);
        let mut chunks = self.core.rope.chunks_in_range(range.clone());
        match (chunks.next(), chunks.next()) {
            (None, _) => Cow::Borrowed(""),
            (Some(only), None) => Cow::Borrowed(only),
            _ => {
                #[cfg(any(test, feature = "test-support"))]
                OWNED_SLICES.with(|count| count.set(count.get() + 1));
                Cow::Owned(self.core.rope.text_for_range(range))
            }
        }
    }

    #[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
    pub fn slice_to_string(&self, range: Range<usize>) -> String {
        self.slice(range).into_owned()
    }
}

impl AsRef<str> for TextModelSnapshot {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for TextModelSnapshot {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl PartialEq for TextModelSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.model_id() == other.model_id() && self.revision() == other.revision()
    }
}

impl Eq for TextModelSnapshot {}

#[cfg(test)]
mod tests;

/// Differential tests for the model's observable behaviour.
///
/// Written to cross-validate the piece table against [`crate::rope::Rope`]
/// while the storage swap was in flight, and kept afterwards: the `String`
/// oracle is what actually pins the semantics, and driving a bare `Rope`
/// alongside the model still catches a `TextModel` that mis-clamps or
/// mis-normalizes a range before handing it down.
#[cfg(test)]
mod storage_equivalence_tests;

/// Guards on what the windowed accessors are *not* allowed to do.
///
/// Materializing the document is the cost these APIs exist to avoid, and it is
/// silent — nothing fails, the frame just gets slower in proportion to the file.
/// Observing the materialization cache directly is the only way to keep that
/// honest as the code changes.
#[cfg(test)]
mod no_materialization_tests;

/// What the rope-backed storage is *for*: an edit costs the edit, not the
/// document.
#[cfg(test)]
mod edit_cost_tests;
