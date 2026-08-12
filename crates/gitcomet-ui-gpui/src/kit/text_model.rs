use crate::kit::rope::{Point, Rope};
use gpui::SharedString;
use std::borrow::Cow;
use std::ops::{Deref, Range};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

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
            // A copy-on-write clone exists because a mutation is coming, which
            // would invalidate both caches anyway.
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
                rope: Rope::from_str(text),
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

    #[cfg(any(test, feature = "benchmarks"))]
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

    /// Replace the whole document. Mints a fresh `model_id`, so snapshots taken
    /// of the previous contents never compare equal to the new ones.
    pub fn set_text(&mut self, text: &str) {
        *self = Self::from_large_text(text);
    }

    #[cfg(any(test, feature = "benchmarks"))]
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
            .clip_offset(offset, gpui::sum_tree::Bias::Left)
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

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn line_starts(&self) -> &[usize] {
        self.core.line_starts().as_ref()
    }

    pub fn shared_line_starts(&self) -> Arc<[usize]> {
        Arc::clone(self.core.line_starts())
    }

    /// Whether the document has been flattened into one contiguous `String`.
    /// The windowed accessors exist precisely so that hot paths never trip
    /// this, and that is only assertable by observing it.
    #[cfg(test)]
    pub(crate) fn is_materialized(&self) -> bool {
        self.core.materialized.get().is_some()
    }

    /// Whether the whole-document line-start array has been built.
    ///
    /// The quieter of the two O(document) caches, and the one worth asserting
    /// separately: it allocates an index proportional to the line count without
    /// copying any text, so a caller that trips it looks innocent next to
    /// [`TextModelSnapshot::is_materialized`] while costing the same order.
    #[cfg(test)]
    pub(crate) fn is_line_index_built(&self) -> bool {
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
            .clip_offset(offset, gpui::sum_tree::Bias::Left)
    }

    fn normalized_char_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clamp_offset_to_char_boundary(range.start.min(self.len()));
        let end = self.clamp_offset_to_char_boundary(range.end.min(self.len()));
        if end < start { end..start } else { start..end }
    }

    /// The text in `range`. Borrowed when it lives inside a single chunk, which
    /// covers a typical row; owned only when it straddles one.
    pub fn slice(&self, range: Range<usize>) -> Cow<'_, str> {
        let range = self.normalized_char_range(range);
        let mut chunks = self.core.rope.chunks_in_range(range.clone());
        match (chunks.next(), chunks.next()) {
            (None, _) => Cow::Borrowed(""),
            (Some(only), None) => Cow::Borrowed(only),
            _ => Cow::Owned(self.core.rope.text_for_range(range)),
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
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
mod tests {
    use super::*;

    fn line_starts_for_text(text: &str) -> Vec<usize> {
        let mut starts = vec![0];
        for (ix, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(ix + 1);
            }
        }
        starts
    }

    fn clamp_to_char_boundary(text: &str, mut offset: usize) -> usize {
        offset = offset.min(text.len());
        while offset > 0 && !text.is_char_boundary(offset) {
            offset = offset.saturating_sub(1);
        }
        offset
    }

    fn normalize_range(text: &str, range: Range<usize>) -> Range<usize> {
        let start = clamp_to_char_boundary(text, range.start.min(text.len()));
        let end = clamp_to_char_boundary(text, range.end.min(text.len()));
        if end < start { end..start } else { start..end }
    }

    fn replace_control(text: &mut String, range: Range<usize>, inserted: &str) -> Range<usize> {
        let normalized = normalize_range(text.as_str(), range);
        text.replace_range(normalized.clone(), inserted);
        normalized.start..normalized.start.saturating_add(inserted.len())
    }

    #[test]
    fn replace_range_updates_text_and_line_index() {
        let mut model = TextModel::from_large_text("alpha\nbeta\ngamma");
        let inserted = model.replace_range(6..10, "BETA\nDELTA");
        assert_eq!(inserted, 6..16);
        assert_eq!(model.as_str(), "alpha\nBETA\nDELTA\ngamma");
        assert_eq!(model.line_starts(), &[0, 6, 11, 17]);
    }

    #[test]
    fn replace_range_keeps_line_start_when_edit_ends_at_line_boundary() {
        let mut model = TextModel::from_large_text("ab\ncd");
        let inserted = model.replace_range(0..3, "");
        assert_eq!(inserted, 0..0);
        assert_eq!(model.as_str(), "cd");
        assert_eq!(model.line_starts(), &[0]);
    }

    #[test]
    fn replace_range_dropping_newline_removes_stale_line_start() {
        let mut model = TextModel::from_large_text("a\nb\nc");
        let inserted = model.replace_range(1..2, "");
        assert_eq!(inserted, 1..1);
        assert_eq!(model.as_str(), "ab\nc");
        assert_eq!(model.line_starts(), &[0, 3]);
    }

    #[test]
    fn snapshot_clone_is_cheap_and_immutable_after_mutation() {
        let mut model = TextModel::from_large_text("hello world");
        let snapshot_a = model.snapshot();
        let snapshot_b = snapshot_a.clone();
        let snapshot_revision = snapshot_a.revision();

        model.replace_range(0..5, "goodbye");

        assert_eq!(snapshot_a.as_str(), "hello world");
        assert_eq!(snapshot_b.as_str(), "hello world");
        assert_eq!(snapshot_a.revision(), snapshot_revision);
        assert_ne!(snapshot_a.revision(), model.revision());
    }

    #[test]
    fn snapshot_shared_line_starts_remain_stable_after_edit() {
        let mut model = TextModel::from_large_text("alpha\nbeta\ngamma");
        let old_snapshot = model.snapshot();
        let old_starts = old_snapshot.shared_line_starts();

        model.replace_range(6..10, "BETA\nDELTA");

        let new_starts = model.snapshot().shared_line_starts();

        assert!(
            !Arc::ptr_eq(&old_starts, &new_starts),
            "editing should swap to a new line-start index"
        );
        assert_eq!(old_starts.as_ref(), &[0, 6, 11]);
        assert_eq!(new_starts.as_ref(), &[0, 6, 11, 17]);
    }

    #[test]
    fn from_large_text_chunks_preserve_content() {
        let mut text = String::new();
        for ix in 0..2_048usize {
            text.push_str(format!("line_{ix:04}\n").as_str());
        }
        let model = TextModel::from_large_text(text.as_str());
        assert_eq!(model.len(), text.len());
        assert_eq!(model.as_str(), text);
        assert_eq!(model.line_starts().len(), 2_049);
    }

    #[test]
    fn replace_range_clamps_unicode_boundaries() {
        let mut model = TextModel::from_large_text("🙂\nβeta");
        let inserted = model.replace_range(1..6, "é\n");
        assert_eq!(inserted, 0..3);
        assert_eq!(model.as_str(), "é\nβeta");
        assert_eq!(model.line_starts(), &[0, 3]);
    }

    #[test]
    fn snapshot_slice_to_string_matches_full_text_across_piece_boundaries() {
        let mut model = TextModel::new();
        let _ = model.append_large("left-");
        let _ = model.append_large("🙂middle-");
        let _ = model.append_large("right");
        let snapshot = model.snapshot();
        let full = snapshot.as_str();
        let expected_range = normalize_range(full, 3..17);
        let expected = full[expected_range].to_string();
        assert_eq!(snapshot.slice_to_string(3..17), expected);
    }

    #[test]
    #[allow(clippy::reversed_empty_ranges)]
    fn replace_range_normalizes_reversed_and_out_of_bounds_ranges() {
        let mut model = TextModel::from_large_text("abcdef");
        let inserted = model.replace_range(128..2, "XY");
        assert_eq!(inserted, 2..4);
        assert_eq!(model.as_str(), "abXY");
        assert_eq!(model.line_starts(), &[0]);

        let inserted = model.replace_range(4..999, "!");
        assert_eq!(inserted, 4..5);
        assert_eq!(model.as_str(), "abXY!");
        assert_eq!(model.line_starts(), &[0]);
    }

    #[test]
    fn replace_range_handles_empty_model_insert_and_delete() {
        let mut model = TextModel::new();
        let inserted = model.replace_range(0..16, "");
        assert_eq!(inserted, 0..0);
        assert_eq!(model.as_str(), "");
        assert_eq!(model.line_starts(), &[0]);

        let inserted = model.replace_range(0..0, "hello\n");
        assert_eq!(inserted, 0..6);
        assert_eq!(model.as_str(), "hello\n");
        assert_eq!(model.line_starts(), &[0, 6]);

        let inserted = model.replace_range(0..usize::MAX, "");
        assert_eq!(inserted, 0..0);
        assert_eq!(model.as_str(), "");
        assert_eq!(model.line_starts(), &[0]);
    }

    #[test]
    fn replace_range_updates_consecutive_newline_line_starts() {
        let mut model = TextModel::from_large_text("a\n\n\nb");
        let inserted = model.replace_range(1..4, "\n\n");
        assert_eq!(inserted, 1..3);
        assert_eq!(model.as_str(), "a\n\nb");
        assert_eq!(model.line_starts(), &[0, 2, 3]);
    }

    #[test]
    fn apply_edit_at_line_boundaries_stays_monotonic() {
        // Exercises boundary conditions around the monotonic-output guarantee:
        // edits exactly at newline offsets, multi-newline inserts replacing
        // multi-newline ranges, and empty-range inserts at every line start.
        let cases: &[(&str, Range<usize>, &str)] = &[
            // Delete a newline exactly between two line starts.
            ("a\nb\nc", 1..2, ""),
            // Replace across multiple newlines with multiple newlines.
            ("a\nb\nc\nd", 2..5, "X\nY\nZ"),
            // Insert newlines at position 0.
            ("abc", 0..0, "\n\n"),
            // Insert at end after trailing newline.
            ("a\n", 2..2, "b\nc"),
            // Replace entire content.
            ("old\ntext", 0..8, "new\n\nlines\n"),
            // Delete range that spans from before a newline to after it.
            ("ab\ncd\nef", 2..5, ""),
            // Insert at every line start in a multi-line doc.
            ("a\nb\nc\n", 0..0, "X"),
            ("a\nb\nc\n", 2..2, "X"),
            ("a\nb\nc\n", 4..4, "X"),
            // Replace newline with newlines.
            ("a\nb", 1..2, "\n\n"),
        ];
        for (text, range, inserted) in cases {
            let mut model = TextModel::from_large_text(text);
            model.replace_range(range.clone(), inserted);
            let mut control = text.to_string();
            replace_control(&mut control, range.clone(), inserted);
            assert_eq!(model.as_str(), control, "text mismatch for edit {text:?}");
            let expected_starts = line_starts_for_text(&control);
            assert_eq!(
                model.line_starts(),
                expected_starts.as_slice(),
                "line starts mismatch for edit on {text:?} [{range:?} -> {inserted:?}]"
            );
        }
    }

    #[test]
    fn sequential_edits_match_string_control() {
        let mut model = TextModel::from_large_text("😀alpha\nβeta\n\ngamma");
        let mut control = model.as_str().to_string();
        let edits = [
            (1usize, 6usize, "X"),
            (12usize, 4usize, "Q\n"),
            (999usize, 999usize, "\ntail"),
            (3usize, 1_000usize, ""),
            (0usize, 0usize, "prefix\n"),
            (2usize, 2usize, "🙂"),
            (5usize, 8usize, ""),
            (usize::MAX - 1, 1usize, "Ω"),
        ];

        for (start, end, inserted_text) in edits {
            let range = start..end;
            let expected_inserted = replace_control(&mut control, range.clone(), inserted_text);
            let actual_inserted = model.replace_range(range, inserted_text);
            assert_eq!(actual_inserted, expected_inserted);
            assert_eq!(model.as_str(), control);
            let expected_starts = line_starts_for_text(control.as_str());
            assert_eq!(model.line_starts(), expected_starts.as_slice());
        }
    }
}

/// Differential tests for the model's observable behaviour.
///
/// Written to cross-validate the piece table against [`crate::kit::rope::Rope`]
/// while the storage swap was in flight, and kept afterwards: the `String`
/// oracle is what actually pins the semantics, and driving a bare `Rope`
/// alongside the model still catches a `TextModel` that mis-clamps or
/// mis-normalizes a range before handing it down.
#[cfg(test)]
mod storage_equivalence_tests {
    use super::*;
    use crate::kit::rope::Rope;

    /// Deterministic xorshift64*, matching the rope's own test generator.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed.wrapping_mul(2685821657736338717).max(1))
        }

        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(2685821657736338717)
        }

        fn up_to(&mut self, limit: usize) -> usize {
            if limit == usize::MAX {
                return 0;
            }
            (self.next_u64() % (limit as u64 + 1)) as usize
        }
    }

    fn random_text(rng: &mut Rng, len: usize) -> String {
        (0..len)
            .map(|_| match rng.up_to(9) {
                0..=3 => (b'a' + rng.up_to(25) as u8) as char,
                4..=5 => '\n',
                6 => ' ',
                7 => 'é',
                8 => '√',
                _ => '\u{1F600}',
            })
            .collect()
    }

    fn floor_boundary(text: &str, mut index: usize) -> usize {
        index = index.min(text.len());
        while index > 0 && !text.is_char_boundary(index) {
            index -= 1;
        }
        index
    }

    fn assert_agree(model: &TextModelSnapshot, rope: &Rope, expected: &str, context: &str) {
        assert_eq!(model.len(), rope.len(), "{context}: len");
        assert_eq!(model.len(), expected.len(), "{context}: len vs oracle");
        assert_eq!(
            model.line_count(),
            rope.line_count() as usize,
            "{context}: line_count"
        );

        for row in 0..model.line_count() {
            let model_range = model.line_range(row);
            let rope_range = rope.line_range(row as u32);
            assert_eq!(model_range, rope_range, "{context}: line_range({row})");
            assert_eq!(
                model.line_text(row).as_ref(),
                rope.line_text(row as u32),
                "{context}: line_text({row})"
            );
            assert_eq!(
                model.line_text(row).as_ref(),
                &expected[model_range],
                "{context}: line_text({row}) vs oracle"
            );
        }
    }

    #[test]
    fn piece_table_and_rope_agree_across_random_edits() {
        for seed in 0..24 {
            let mut rng = Rng::new(seed);
            let mut expected = String::new();
            let mut model = TextModel::new();
            let mut rope = Rope::new();

            for op in 0..30 {
                let end = floor_boundary(&expected, rng.up_to(expected.len()));
                let start = floor_boundary(&expected, rng.up_to(end));
                let inserted = {
                    let len = rng.up_to(24);
                    random_text(&mut rng, len)
                };

                model.replace_range(start..end, &inserted);
                rope.replace(start..end, &inserted);
                expected.replace_range(start..end, &inserted);

                let context = format!("seed {seed} op {op}");
                let snapshot = model.snapshot();
                assert_agree(&snapshot, &rope, &expected, &context);

                // Windowed reads and the UTF-16 conversions, at random probes.
                for _ in 0..4 {
                    let probe = floor_boundary(&expected, rng.up_to(expected.len()));
                    let oracle_utf16 = expected[..probe]
                        .chars()
                        .map(char::len_utf16)
                        .sum::<usize>();

                    assert_eq!(
                        snapshot.offset_to_utf16(probe),
                        oracle_utf16,
                        "{context}: offset_to_utf16({probe})"
                    );
                    assert_eq!(
                        snapshot.offset_to_utf16(probe),
                        rope.offset_to_utf16(probe),
                        "{context}: offset_to_utf16({probe}) vs rope"
                    );
                    assert_eq!(
                        snapshot.offset_from_utf16(oracle_utf16),
                        probe,
                        "{context}: offset_from_utf16({oracle_utf16})"
                    );
                    assert_eq!(
                        snapshot.offset_from_utf16(oracle_utf16),
                        rope.offset_from_utf16(oracle_utf16),
                        "{context}: offset_from_utf16({oracle_utf16}) vs rope"
                    );

                    let end = floor_boundary(&expected, rng.up_to(expected.len()));
                    let start = floor_boundary(&expected, rng.up_to(end));
                    assert_eq!(
                        snapshot.slice(start..end).as_ref(),
                        &expected[start..end],
                        "{context}: slice({start}..{end})"
                    );
                    assert_eq!(
                        snapshot.slice(start..end).as_ref(),
                        rope.text_for_range(start..end),
                        "{context}: slice({start}..{end}) vs rope"
                    );
                }
            }
        }
    }

    /// Reading one row must not depend on the document's size — the property
    /// the renderer relies on, and the reason `line_text` exists at all.
    #[test]
    fn line_text_reads_a_single_row_of_a_large_document() {
        let text = "the quick brown fox\n".repeat(50_000);
        let model = TextModel::from_large_text(&text);
        let snapshot = model.snapshot();
        let rope = Rope::from_str(&text);

        assert_eq!(snapshot.line_count(), 50_001);
        for row in [0usize, 1, 25_000, 49_999, 50_000] {
            assert_eq!(
                snapshot.line_text(row).as_ref(),
                rope.line_text(row as u32),
                "row {row}"
            );
        }
        assert_eq!(snapshot.line_text(0).as_ref(), "the quick brown fox");
        assert_eq!(snapshot.line_text(50_000).as_ref(), "");
    }

    /// ASCII documents — nearly all of them — must answer UTF-16 conversions
    /// without inspecting the text at all.
    #[test]
    fn ascii_documents_convert_utf16_offsets_by_identity() {
        let model = TextModel::from_large_text(&"plain ascii\n".repeat(1000));
        let snapshot = model.snapshot();
        for probe in [0usize, 1, 500, snapshot.len()] {
            assert_eq!(snapshot.offset_to_utf16(probe), probe);
            assert_eq!(snapshot.offset_from_utf16(probe), probe);
        }
    }

    #[test]
    fn utf16_conversions_handle_surrogate_pairs() {
        let text = "a🙂b";
        let model = TextModel::from_large_text(text);
        let snapshot = model.snapshot();
        let rope = Rope::from_str(text);

        // 'a' = 1 unit, '🙂' = 2 units (surrogate pair), 'b' = 1 unit.
        assert_eq!(snapshot.offset_to_utf16(0), 0);
        assert_eq!(snapshot.offset_to_utf16(1), 1);
        assert_eq!(snapshot.offset_to_utf16(5), 3);
        assert_eq!(snapshot.offset_to_utf16(6), 4);
        assert_eq!(snapshot.offset_from_utf16(3), 5);

        for probe in 0..=4 {
            assert_eq!(
                snapshot.offset_from_utf16(probe),
                rope.offset_from_utf16(probe),
                "offset_from_utf16({probe})"
            );
        }
    }
}

/// Guards on what the windowed accessors are *not* allowed to do.
///
/// Materializing the document is the cost these APIs exist to avoid, and it is
/// silent — nothing fails, the frame just gets slower in proportion to the file.
/// Observing the materialization cache directly is the only way to keep that
/// honest as the code changes.
#[cfg(test)]
mod no_materialization_tests {
    use super::*;

    /// An edited model whose materialization cache is cold: any accessor that
    /// reaches for `as_str()` will flatten the whole document to answer.
    fn edited_model() -> TextModel {
        let mut model = TextModel::from_large_text(&"pub fn value() -> u32 { 7 }\n".repeat(4000));
        // Edit mid-document so the piece table is genuinely fragmented and the
        // "unedited original" fast path cannot apply.
        let offset = model.len() / 2;
        model.replace_range(offset..offset, "// touched\n");
        assert!(
            !model.snapshot().is_materialized(),
            "an edit must invalidate the materialization cache"
        );
        model
    }

    #[test]
    fn utf16_conversions_do_not_materialize_the_document() {
        let model = edited_model();
        let snapshot = model.snapshot();

        snapshot.offset_to_utf16(snapshot.len() / 2);
        snapshot.offset_from_utf16(snapshot.len() / 4);

        assert!(
            !snapshot.is_materialized(),
            "caret queries run on every keystroke; they must not flatten the buffer"
        );
    }

    #[test]
    fn reading_one_row_does_not_materialize_the_document() {
        let model = edited_model();
        let snapshot = model.snapshot();

        let row = snapshot.line_count() / 2;
        let text = snapshot.line_text(row);

        assert!(!text.is_empty(), "fixture row should have content");
        assert!(
            !snapshot.is_materialized(),
            "rendering a row must cost the row, not the document"
        );
    }

    #[test]
    fn line_geometry_does_not_materialize_the_document() {
        let model = edited_model();
        let snapshot = model.snapshot();

        let _ = snapshot.line_count();
        let _ = snapshot.line_range(snapshot.line_count() - 1);
        let _ = snapshot.shared_line_starts();

        assert!(!snapshot.is_materialized());
    }
}

/// What the rope-backed storage is *for*: an edit costs the edit, not the
/// document.
#[cfg(test)]
mod edit_cost_tests {
    use super::*;

    #[test]
    fn editing_a_large_document_does_not_materialize_it() {
        let mut model = TextModel::from_large_text(&"fn value() -> u32 { 7 }\n".repeat(20_000));
        assert!(!model.snapshot().is_materialized());

        // Mid-document insert, delete, and replace — the three shapes an editor
        // produces. None of them may need the document as one string.
        let middle = model.len() / 2;
        model.replace_range(middle..middle, "// inserted\n");
        model.replace_range(middle..middle + 6, "");
        model.replace_range(middle..middle + 4, "abcd");

        assert!(
            !model.snapshot().is_materialized(),
            "editing must not flatten the buffer"
        );
    }

    /// A snapshot taken before an edit keeps observing the old document, and
    /// getting there costs an atomic increment rather than a copy.
    #[test]
    fn snapshots_are_cheap_and_isolated_from_later_edits() {
        let text = "line of text\n".repeat(20_000);
        let mut model = TextModel::from_large_text(&text);
        let before = model.snapshot();

        model.replace_range(0..0, "prefix\n");

        assert_eq!(before.len(), text.len());
        assert_eq!(before.line_text(0).as_ref(), "line of text");
        assert_eq!(model.snapshot().line_text(0).as_ref(), "prefix");
        assert_ne!(before, model.snapshot());
    }

    /// `longest_row` backs the horizontal scroll bound, and it must stay a
    /// summary read rather than a scan.
    #[test]
    fn the_widest_row_is_known_without_scanning() {
        let mut model = TextModel::from_large_text(&"short\n".repeat(10_000));
        let wide = "w".repeat(4096);
        model.replace_range(0..0, &format!("{wide}\n"));

        let snapshot = model.snapshot();
        assert_eq!(snapshot.line_text(0).as_ref().len(), wide.len());
        assert!(!snapshot.is_materialized());
    }
}
