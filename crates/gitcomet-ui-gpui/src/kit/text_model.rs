use gpui::SharedString;
use std::ops::{Deref, Range};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

const LARGE_TEXT_CHUNK_BYTES: usize = 16 * 1024;
const LARGE_TEXT_PARALLEL_THRESHOLD: usize = 512 * 1024;
const LARGE_TEXT_PARALLEL_MIN_CHUNKS: usize = 8;
const LARGE_TEXT_MAX_THREADS: usize = 8;

static NEXT_MODEL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BufferId {
    Original,
    Add,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Piece {
    buffer: BufferId,
    chunk_index: usize,
    start: usize,
    len: usize,
}

impl Piece {
    fn prefix(&self, len: usize) -> Option<Self> {
        (len > 0).then_some(Self {
            buffer: self.buffer,
            chunk_index: self.chunk_index,
            start: self.start,
            len,
        })
    }

    fn suffix(&self, offset: usize) -> Option<Self> {
        let suffix_len = self.len.saturating_sub(offset);
        (suffix_len > 0).then_some(Self {
            buffer: self.buffer,
            chunk_index: self.chunk_index,
            start: self.start.saturating_add(offset),
            len: suffix_len,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LineIndex {
    starts: Arc<[usize]>,
}

impl LineIndex {
    fn from_text(text: &str) -> Self {
        let mut starts = Vec::with_capacity(text.bytes().filter(|&b| b == b'\n').count() + 1);
        starts.push(0);
        for (ix, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(ix + 1);
            }
        }
        Self {
            starts: Arc::<[usize]>::from(starts),
        }
    }

    fn starts(&self) -> &[usize] {
        self.starts.as_ref()
    }

    fn shared_starts(&self) -> Arc<[usize]> {
        Arc::clone(&self.starts)
    }

    fn apply_edit(&mut self, range: Range<usize>, inserted: &str) {
        let starts = self.starts.as_ref();
        debug_assert_eq!(starts.first().copied(), Some(0));
        debug_assert!(
            starts.windows(2).all(|window| window[0] < window[1]),
            "line starts must remain strictly increasing before edit"
        );

        let old_len = range.end.saturating_sub(range.start);
        let new_len = inserted.len();
        let delta = new_len as isize - old_len as isize;

        let prefix_len = starts.partition_point(|&start| start <= range.start);
        // For non-empty edits, a line start at `range.end` is produced by a
        // newline byte inside the replaced range and must be removed.
        let suffix_start = starts.partition_point(|&start| start <= range.end);

        let inserted_breaks = inserted.bytes().filter(|&b| b == b'\n').count();
        let mut updated = Vec::with_capacity(
            prefix_len
                .saturating_add(inserted_breaks)
                .saturating_add(starts.len().saturating_sub(suffix_start))
                .saturating_add(1),
        );
        updated.extend_from_slice(&starts[..prefix_len]);

        for (ix, byte) in inserted.bytes().enumerate() {
            if byte == b'\n' {
                updated.push(range.start.saturating_add(ix).saturating_add(1));
            }
        }

        for &start in &starts[suffix_start..] {
            let shifted = if delta >= 0 {
                start.saturating_add(delta as usize)
            } else {
                start.saturating_sub((-delta) as usize)
            };
            updated.push(shifted);
        }

        updated.sort_unstable();
        updated.dedup();
        if updated.first().copied() != Some(0) {
            updated.insert(0, 0);
        }
        self.starts = Arc::<[usize]>::from(updated);
        debug_assert_eq!(self.starts.as_ref().first().copied(), Some(0));
        debug_assert!(
            self.starts
                .as_ref()
                .windows(2)
                .all(|window| window[0] < window[1]),
            "line starts must remain strictly increasing after edit"
        );
    }
}

#[derive(Debug)]
struct TextModelCore {
    model_id: u64,
    revision: u64,
    original_chunks: Arc<Vec<Arc<str>>>,
    add_chunks: Arc<Vec<Arc<str>>>,
    pieces: Vec<Piece>,
    len: usize,
    line_index: LineIndex,
    materialized: OnceLock<SharedString>,
}

impl Clone for TextModelCore {
    fn clone(&self) -> Self {
        Self {
            model_id: self.model_id,
            revision: self.revision,
            original_chunks: Arc::clone(&self.original_chunks),
            add_chunks: Arc::clone(&self.add_chunks),
            pieces: self.pieces.clone(),
            len: self.len,
            line_index: self.line_index.clone(),
            // Do not clone materialized text into writable COW clones.
            materialized: OnceLock::new(),
        }
    }
}

impl TextModelCore {
    fn chunk_for_piece(&self, piece: &Piece) -> &str {
        match piece.buffer {
            BufferId::Original => self
                .original_chunks
                .get(piece.chunk_index)
                .map(|chunk| chunk.as_ref())
                .unwrap_or(""),
            BufferId::Add => self
                .add_chunks
                .get(piece.chunk_index)
                .map(|chunk| chunk.as_ref())
                .unwrap_or(""),
        }
    }

    fn piece_slice<'a>(&'a self, piece: &Piece) -> &'a str {
        let chunk = self.chunk_for_piece(piece);
        let start = piece.start.min(chunk.len());
        let end = piece.start.saturating_add(piece.len).min(chunk.len());
        chunk.get(start..end).unwrap_or("")
    }

    fn materialized(&self) -> &SharedString {
        self.materialized.get_or_init(|| {
            if self.pieces.is_empty() {
                return SharedString::default();
            }

            let mut text = String::with_capacity(self.len);
            for piece in &self.pieces {
                text.push_str(self.piece_slice(piece));
            }
            text.into()
        })
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
    fn default() -> Self {
        TextModel::default().snapshot()
    }
}

impl TextModel {
    pub fn new() -> Self {
        Self::from_large_text("")
    }

    pub fn from_large_text(text: &str) -> Self {
        let ranges = chunk_ranges(text, LARGE_TEXT_CHUNK_BYTES);
        let original_chunks = prepare_chunks(
            text,
            ranges.as_slice(),
            LARGE_TEXT_PARALLEL_THRESHOLD,
            LARGE_TEXT_PARALLEL_MIN_CHUNKS,
        );
        let pieces = original_chunks
            .iter()
            .enumerate()
            .filter_map(|(chunk_index, chunk)| {
                (!chunk.is_empty()).then_some(Piece {
                    buffer: BufferId::Original,
                    chunk_index,
                    start: 0,
                    len: chunk.len(),
                })
            })
            .collect::<Vec<_>>();

        let model_id = NEXT_MODEL_ID.fetch_add(1, Ordering::Relaxed).max(1);
        Self {
            core: Arc::new(TextModelCore {
                model_id,
                revision: 1,
                original_chunks: Arc::new(original_chunks),
                add_chunks: Arc::new(Vec::new()),
                len: text.len(),
                line_index: LineIndex::from_text(text),
                pieces,
                materialized: OnceLock::new(),
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.core.len
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
        self.core.materialized().clone()
    }

    pub fn line_starts(&self) -> &[usize] {
        self.core.line_index.starts()
    }

    pub fn snapshot(&self) -> TextModelSnapshot {
        TextModelSnapshot {
            core: Arc::clone(&self.core),
        }
    }

    pub fn set_text(&mut self, text: &str) {
        *self = Self::from_large_text(text);
    }

    pub fn append_large(&mut self, text: &str) -> Range<usize> {
        let start = self.len();
        self.replace_range(start..start, text)
    }

    pub fn is_char_boundary(&self, offset: usize) -> bool {
        self.snapshot().is_char_boundary(offset)
    }

    pub fn clamp_to_char_boundary(&self, mut offset: usize) -> usize {
        offset = offset.min(self.len());
        while offset > 0 && !self.is_char_boundary(offset) {
            offset = offset.saturating_sub(1);
        }
        offset
    }

    pub fn replace_range(&mut self, range: Range<usize>, new_text: &str) -> Range<usize> {
        let start = self.clamp_to_char_boundary(range.start.min(self.len()));
        let end = self.clamp_to_char_boundary(range.end.min(self.len()));
        let range = if end < start { end..start } else { start..end };
        if range.is_empty() && new_text.is_empty() {
            return range.start..range.start;
        }

        let core = Arc::make_mut(&mut self.core);
        let (mut left, right_from_start) = split_pieces_at(core.pieces.as_slice(), range.start);
        let (_removed, mut right) = split_pieces_at(
            right_from_start.as_slice(),
            range.end.saturating_sub(range.start),
        );

        let inserted_pieces = append_add_pieces(core, new_text);
        left.extend(inserted_pieces);
        left.append(&mut right);
        merge_adjacent_pieces(&mut left);

        core.pieces = left;
        core.len = core
            .len
            .saturating_sub(range.end.saturating_sub(range.start))
            .saturating_add(new_text.len());
        core.line_index.apply_edit(range.clone(), new_text);
        core.revision = core.revision.wrapping_add(1).max(1);
        core.materialized = OnceLock::new();

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
        self.core.len
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
        self.core.materialized().clone()
    }

    pub fn line_starts(&self) -> &[usize] {
        self.core.line_index.starts()
    }

    pub fn shared_line_starts(&self) -> Arc<[usize]> {
        self.core.line_index.shared_starts()
    }

    pub fn is_char_boundary(&self, offset: usize) -> bool {
        if offset == 0 || offset == self.core.len {
            return true;
        }
        if offset > self.core.len {
            return false;
        }

        let mut cursor = 0usize;
        for piece in &self.core.pieces {
            let next = cursor.saturating_add(piece.len);
            if offset == cursor || offset == next {
                return true;
            }
            if offset < next {
                let local = offset.saturating_sub(cursor);
                let chunk = self.core.chunk_for_piece(piece);
                let absolute = piece.start.saturating_add(local);
                return chunk.is_char_boundary(absolute);
            }
            cursor = next;
        }
        false
    }

    pub fn clamp_to_char_boundary(&self, mut offset: usize) -> usize {
        offset = offset.min(self.len());
        while offset > 0 && !self.is_char_boundary(offset) {
            offset = offset.saturating_sub(1);
        }
        offset
    }

    pub fn slice_to_string(&self, range: Range<usize>) -> String {
        let start = self.clamp_to_char_boundary(range.start.min(self.len()));
        let end = self.clamp_to_char_boundary(range.end.min(self.len()));
        let range = if end < start { end..start } else { start..end };
        if range.is_empty() {
            return String::new();
        }

        let mut out = String::with_capacity(range.end.saturating_sub(range.start));
        let mut cursor = 0usize;
        for piece in &self.core.pieces {
            let piece_start = cursor;
            let piece_end = cursor.saturating_add(piece.len);
            if piece_end <= range.start {
                cursor = piece_end;
                continue;
            }
            if piece_start >= range.end {
                break;
            }

            let local_start = range.start.saturating_sub(piece_start);
            let local_end = range.end.min(piece_end).saturating_sub(piece_start);
            if local_start < local_end {
                let chunk = self.core.chunk_for_piece(piece);
                let chunk_start = piece.start.saturating_add(local_start);
                let chunk_end = piece.start.saturating_add(local_end);
                if let Some(slice) = chunk.get(chunk_start..chunk_end) {
                    out.push_str(slice);
                }
            }
            cursor = piece_end;
        }
        out
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

fn chunk_ranges(text: &str, chunk_bytes: usize) -> Vec<Range<usize>> {
    if text.is_empty() {
        return Vec::new();
    }

    let chunk_bytes = chunk_bytes.max(1);
    let mut ranges = Vec::with_capacity(text.len() / chunk_bytes + 1);
    let mut start = 0usize;
    while start < text.len() {
        let mut end = (start + chunk_bytes).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        if end == start {
            end = text.len();
        }
        ranges.push(start..end);
        start = end;
    }
    ranges
}

fn prepare_chunks(
    text: &str,
    ranges: &[Range<usize>],
    parallel_threshold: usize,
    parallel_min_chunks: usize,
) -> Vec<Arc<str>> {
    if ranges.is_empty() {
        return Vec::new();
    }

    let should_parallelize = text.len() >= parallel_threshold
        && ranges.len() >= parallel_min_chunks
        && std::thread::available_parallelism()
            .map(|n| n.get() > 1)
            .unwrap_or(false);
    if !should_parallelize {
        return ranges
            .iter()
            .map(|range| Arc::<str>::from(&text[range.clone()]))
            .collect();
    }

    prepare_chunks_parallel(text, ranges)
}

fn prepare_chunks_parallel(text: &str, ranges: &[Range<usize>]) -> Vec<Arc<str>> {
    let thread_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, LARGE_TEXT_MAX_THREADS)
        .min(ranges.len());
    if thread_count <= 1 {
        return ranges
            .iter()
            .map(|range| Arc::<str>::from(&text[range.clone()]))
            .collect();
    }

    let mut assignments = vec![Vec::<(usize, Range<usize>)>::new(); thread_count];
    for (ix, range) in ranges.iter().cloned().enumerate() {
        assignments[ix % thread_count].push((ix, range));
    }

    let mut worker_results = vec![Vec::<(usize, Arc<str>)>::new(); thread_count];
    std::thread::scope(|scope| {
        for (result_slot, worker_ranges) in worker_results.iter_mut().zip(assignments.into_iter()) {
            let text_ref = text;
            scope.spawn(move || {
                result_slot.reserve(worker_ranges.len());
                for (ix, range) in worker_ranges {
                    result_slot.push((ix, Arc::<str>::from(&text_ref[range])));
                }
            });
        }
    });

    let mut chunks = std::iter::repeat_with(|| Arc::<str>::from(""))
        .take(ranges.len())
        .collect::<Vec<_>>();
    for worker in worker_results {
        for (ix, chunk) in worker {
            if let Some(slot) = chunks.get_mut(ix) {
                *slot = chunk;
            }
        }
    }
    chunks
}

fn split_pieces_at(pieces: &[Piece], offset: usize) -> (Vec<Piece>, Vec<Piece>) {
    if pieces.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if offset == 0 {
        return (Vec::new(), pieces.to_vec());
    }

    let mut left = Vec::with_capacity(pieces.len());
    let mut right = Vec::with_capacity(pieces.len());
    let mut consumed = 0usize;

    for piece in pieces {
        let piece_end = consumed.saturating_add(piece.len);
        if piece_end <= offset {
            left.push(piece.clone());
        } else if consumed >= offset {
            right.push(piece.clone());
        } else {
            let split_at = offset.saturating_sub(consumed).min(piece.len);
            if let Some(prefix) = piece.prefix(split_at) {
                left.push(prefix);
            }
            if let Some(suffix) = piece.suffix(split_at) {
                right.push(suffix);
            }
        }
        consumed = piece_end;
    }

    (left, right)
}

fn append_add_pieces(core: &mut TextModelCore, text: &str) -> Vec<Piece> {
    if text.is_empty() {
        return Vec::new();
    }

    let ranges = chunk_ranges(text, LARGE_TEXT_CHUNK_BYTES);
    let chunks = prepare_chunks(
        text,
        ranges.as_slice(),
        LARGE_TEXT_PARALLEL_THRESHOLD,
        LARGE_TEXT_PARALLEL_MIN_CHUNKS,
    );

    let add_chunks = Arc::make_mut(&mut core.add_chunks);
    add_chunks.reserve(chunks.len());
    let base = add_chunks.len();

    let mut pieces = Vec::with_capacity(chunks.len());
    for (ix, chunk) in chunks.into_iter().enumerate() {
        let len = chunk.len();
        add_chunks.push(chunk);
        pieces.push(Piece {
            buffer: BufferId::Add,
            chunk_index: base + ix,
            start: 0,
            len,
        });
    }
    pieces
}

fn merge_adjacent_pieces(pieces: &mut Vec<Piece>) {
    if pieces.len() < 2 {
        return;
    }

    let mut merged = Vec::with_capacity(pieces.len());
    let mut current = pieces[0].clone();
    for piece in pieces.iter().skip(1) {
        let contiguous = current.buffer == piece.buffer
            && current.chunk_index == piece.chunk_index
            && current.start.saturating_add(current.len) == piece.start;
        if contiguous {
            current.len = current.len.saturating_add(piece.len);
            continue;
        }
        merged.push(current);
        current = piece.clone();
    }
    merged.push(current);
    *pieces = merged;
}

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
    fn snapshot_shared_line_starts_reuse_index_storage() {
        let model = TextModel::from_large_text("alpha\nbeta\ngamma");
        let snapshot = model.snapshot();

        let model_starts = model.snapshot().shared_line_starts();
        let snapshot_starts = snapshot.shared_line_starts();

        assert!(
            Arc::ptr_eq(&model_starts, &snapshot_starts),
            "snapshots should share line-start storage with the source model"
        );
        assert_eq!(snapshot_starts.as_ref(), &[0, 6, 11]);
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
    fn append_large_uses_piece_table_insert_path() {
        let mut model = TextModel::new();
        let inserted = model.append_large("first\n");
        assert_eq!(inserted, 0..6);
        let inserted = model.append_large("second");
        assert_eq!(inserted, 6..12);
        assert_eq!(model.as_str(), "first\nsecond");
        assert_eq!(model.line_starts(), &[0, 6]);
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
