use gpui::SharedString;
use memchr::memchr_iter;
use smallvec::SmallVec;
#[cfg(any(test, feature = "benchmarks"))]
use std::borrow::Cow;
use std::ops::{Deref, Range};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

const LARGE_TEXT_CHUNK_BYTES: usize = 16 * 1024;
const SMALL_ADD_CHUNK_BYTES: usize = 4 * 1024;
const PIECE_TABLE_COMPACTION_THRESHOLD: usize = 256;
static NEXT_MODEL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BufferId {
    Original,
    Add,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
        let bytes = text.as_bytes();
        let newline_count = memchr_iter(b'\n', bytes).count();
        let mut starts = Vec::with_capacity(newline_count + 1);
        starts.push(0);
        for pos in memchr_iter(b'\n', bytes) {
            starts.push(pos + 1);
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

    fn append_text(&mut self, offset: usize, inserted: &str) {
        if inserted.is_empty() {
            return;
        }

        let inserted_breaks = memchr_iter(b'\n', inserted.as_bytes()).count();
        if inserted_breaks == 0 {
            return;
        }

        let starts = self.starts.as_ref();
        let mut updated = Vec::with_capacity(starts.len().saturating_add(inserted_breaks));
        updated.extend_from_slice(starts);

        for pos in memchr_iter(b'\n', inserted.as_bytes()) {
            updated.push(offset.saturating_add(pos).saturating_add(1));
        }

        self.starts = Arc::<[usize]>::from(updated);
        debug_assert_eq!(self.starts.as_ref().first().copied(), Some(0));
        debug_assert!(
            self.starts
                .as_ref()
                .windows(2)
                .all(|window| window[0] < window[1]),
            "line starts must remain strictly increasing after append"
        );
    }

    fn apply_edit(&mut self, range: Range<usize>, inserted: &str) {
        {
            let starts = self.starts.as_ref();
            debug_assert_eq!(starts.first().copied(), Some(0));
            debug_assert!(
                starts.windows(2).all(|window| window[0] < window[1]),
                "line starts must remain strictly increasing before edit"
            );
        }

        let old_len = range.end.saturating_sub(range.start);
        let new_len = inserted.len();
        let delta = new_len as isize - old_len as isize;

        let (prefix_len, starts_len, suffix_start) = {
            let starts = self.starts.as_ref();
            (
                starts.partition_point(|&start| start <= range.start),
                starts.len(),
                starts.partition_point(|&start| start <= range.end),
            )
        };

        let inserted_breaks = memchr_iter(b'\n', inserted.as_bytes()).count();
        let removed_breaks = suffix_start.saturating_sub(prefix_len);
        if removed_breaks == inserted_breaks
            && let Some(updated) = Arc::get_mut(&mut self.starts)
        {
            let mut write_ix = prefix_len;
            for pos in memchr_iter(b'\n', inserted.as_bytes()) {
                updated[write_ix] = range.start.saturating_add(pos).saturating_add(1);
                write_ix += 1;
            }
            for start in &mut updated[suffix_start..] {
                *start = shift_offset_by_delta(*start, delta);
            }
            debug_assert_eq!(updated.first().copied(), Some(0));
            debug_assert!(
                updated.windows(2).all(|window| window[0] < window[1]),
                "line starts must remain strictly increasing after in-place edit"
            );
            return;
        }

        let mut updated = Vec::with_capacity(
            prefix_len
                .saturating_add(inserted_breaks)
                .saturating_add(starts_len.saturating_sub(suffix_start))
                .saturating_add(1),
        );
        let starts = self.starts.as_ref();
        updated.extend_from_slice(&starts[..prefix_len]);

        for pos in memchr_iter(b'\n', inserted.as_bytes()) {
            updated.push(range.start.saturating_add(pos).saturating_add(1));
        }

        for &start in &starts[suffix_start..] {
            updated.push(shift_offset_by_delta(start, delta));
        }

        // The three sections (prefix, inserted breaks, shifted suffix) are
        // already in strictly increasing order with non-overlapping ranges:
        //   prefix values        ≤ range.start
        //   inserted break values ∈ (range.start, range.start + new_len]
        //   shifted suffix values > range.start + new_len
        // so sort/dedup is unnecessary.
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

fn shift_offset_by_delta(start: usize, delta: isize) -> usize {
    if delta >= 0 {
        start.saturating_add(delta as usize)
    } else {
        start.saturating_sub((-delta) as usize)
    }
}

#[derive(Debug)]
struct TextModelCore {
    model_id: u64,
    revision: u64,
    original_chunks: Arc<Vec<Arc<str>>>,
    add_chunks: Arc<Vec<Arc<String>>>,
    pieces: Vec<Piece>,
    len: usize,
    ascii_only: bool,
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
            ascii_only: self.ascii_only,
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
                .map(|chunk| chunk.as_str())
                .unwrap_or(""),
        }
    }

    fn piece_slice<'a>(&'a self, piece: &Piece) -> &'a str {
        let chunk = self.chunk_for_piece(piece);
        let end = piece.start.saturating_add(piece.len);
        debug_assert!(
            end <= chunk.len(),
            "piece range must stay within backing chunk"
        );
        &chunk[piece.start..end]
    }

    fn original_text_shared_string(&self) -> Option<SharedString> {
        let first = *self.pieces.first()?;
        if first.buffer != BufferId::Original || first.start != 0 {
            return None;
        }

        let chunk = Arc::clone(self.original_chunks.get(first.chunk_index)?);
        let mut expected_start = 0usize;
        for piece in &self.pieces {
            if piece.buffer != BufferId::Original
                || piece.chunk_index != first.chunk_index
                || piece.start != expected_start
            {
                return None;
            }
            expected_start = expected_start.saturating_add(piece.len);
        }

        (expected_start == self.len && expected_start == chunk.len())
            .then_some(SharedString::from(chunk))
    }

    fn materialized_clone(&self) -> SharedString {
        self.materialized
            .get()
            .cloned()
            .unwrap_or_else(|| self.materialized().clone())
    }

    fn materialized(&self) -> &SharedString {
        self.materialized.get_or_init(|| {
            if self.pieces.is_empty() {
                return SharedString::default();
            }

            if let Some(original) = self.original_text_shared_string() {
                return original;
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
        let mut original_chunks = Vec::with_capacity(usize::from(!text.is_empty()));
        let pieces = if text.is_empty() {
            Vec::new()
        } else {
            let original = Arc::<str>::from(text);
            let pieces = build_original_pieces(original.as_ref(), LARGE_TEXT_CHUNK_BYTES);
            original_chunks.push(original);
            pieces
        };

        let model_id = NEXT_MODEL_ID.fetch_add(1, Ordering::Relaxed).max(1);
        Self {
            core: Arc::new(TextModelCore {
                model_id,
                revision: 1,
                original_chunks: Arc::new(original_chunks),
                add_chunks: Arc::new(Vec::new()),
                len: text.len(),
                ascii_only: text.is_ascii(),
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

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn as_shared_string(&self) -> SharedString {
        self.core.materialized_clone()
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

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn append_large(&mut self, text: &str) -> Range<usize> {
        let start = self.len();
        self.replace_range(start..start, text)
    }

    pub fn is_char_boundary(&self, offset: usize) -> bool {
        core_is_char_boundary(self.core.as_ref(), offset)
    }

    pub fn clamp_to_char_boundary(&self, offset: usize) -> usize {
        core_clamp_to_char_boundary(self.core.as_ref(), offset)
    }

    pub fn replace_range(&mut self, range: Range<usize>, new_text: &str) -> Range<usize> {
        let len = self.len();
        if range.start >= len && range.end >= len {
            if new_text.is_empty() {
                return len..len;
            }

            let core = Arc::make_mut(&mut self.core);
            return append_text_to_core(core, new_text);
        }

        let start = self.clamp_to_char_boundary(range.start.min(self.len()));
        let end = self.clamp_to_char_boundary(range.end.min(self.len()));
        let range = if end < start { end..start } else { start..end };
        if range.is_empty() && new_text.is_empty() {
            return range.start..range.start;
        }

        let core = Arc::make_mut(&mut self.core);
        core.ascii_only &= new_text.is_ascii();
        let inserted_piece = append_add_piece(core, new_text);
        let span = locate_piece_edit_span(core.pieces.as_slice(), range.clone());
        let mut replacement = SmallVec::<[Piece; 3]>::new();
        if let Some(prefix) = span.prefix_fragment {
            push_piece_merged_small(&mut replacement, prefix);
        }
        if let Some(inserted_piece) = inserted_piece {
            push_piece_merged_small(&mut replacement, inserted_piece);
        }
        if let Some(suffix) = span.suffix_fragment {
            push_piece_merged_small(&mut replacement, suffix);
        }

        let mut replacement_len = replacement.len();
        core.pieces
            .splice(span.replace_start..span.replace_end, replacement);
        let merged_left = span.replace_start > 0
            && merge_piece_at(&mut core.pieces, span.replace_start.saturating_sub(1));
        if replacement_len > 0 {
            if merged_left {
                replacement_len = replacement_len.saturating_sub(1);
            }
            let right_ix = span
                .replace_start
                .saturating_add(replacement_len.saturating_sub(1));
            if right_ix < core.pieces.len() {
                let _ = merge_piece_at(&mut core.pieces, right_ix);
            }
        }
        core.len = core
            .len
            .saturating_sub(range.end.saturating_sub(range.start))
            .saturating_add(new_text.len());
        core.line_index.apply_edit(range.clone(), new_text);
        core.revision = core.revision.wrapping_add(1).max(1);
        if !maybe_compact_piece_table(core) {
            core.materialized = OnceLock::new();
        }

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
        self.core.materialized_clone()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn line_starts(&self) -> &[usize] {
        self.core.line_index.starts()
    }

    pub fn shared_line_starts(&self) -> Arc<[usize]> {
        self.core.line_index.shared_starts()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn clamp_offset_to_char_boundary(&self, offset: usize) -> usize {
        core_clamp_to_char_boundary(self.core.as_ref(), offset)
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn normalized_char_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clamp_offset_to_char_boundary(range.start.min(self.len()));
        let end = self.clamp_offset_to_char_boundary(range.end.min(self.len()));
        if end < start { end..start } else { start..end }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn borrowed_slice_for_range(&self, range: Range<usize>) -> Option<&str> {
        if range.is_empty() {
            return Some("");
        }

        if let Some(text) = self.core.materialized.get() {
            return Some(&text[range]);
        }

        let mut cursor = 0usize;
        for piece in &self.core.pieces {
            let piece_start = cursor;
            let piece_end = cursor.saturating_add(piece.len);
            if piece_end <= range.start {
                cursor = piece_end;
                continue;
            }
            if range.end <= piece_end {
                let local_start = range.start.saturating_sub(piece_start);
                let local_end = range.end.saturating_sub(piece_start);
                let chunk = self.core.chunk_for_piece(piece);
                let chunk_start = piece.start.saturating_add(local_start);
                let chunk_end = piece.start.saturating_add(local_end);
                return chunk.get(chunk_start..chunk_end);
            }
            break;
        }
        None
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn collect_range_to_string(&self, range: Range<usize>) -> String {
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

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn slice(&self, range: Range<usize>) -> Cow<'_, str> {
        let range = self.normalized_char_range(range);
        self.borrowed_slice_for_range(range.clone())
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(self.collect_range_to_string(range)))
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn clamp_to_char_boundary(&self, offset: usize) -> usize {
        self.clamp_offset_to_char_boundary(offset)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PieceEditSpan {
    replace_start: usize,
    replace_end: usize,
    prefix_fragment: Option<Piece>,
    suffix_fragment: Option<Piece>,
}

fn build_original_pieces(text: &str, chunk_bytes: usize) -> Vec<Piece> {
    if text.is_empty() {
        return Vec::new();
    }

    let chunk_bytes = chunk_bytes.max(1);
    let mut pieces = Vec::with_capacity(text.len() / chunk_bytes + 1);
    let mut start = 0usize;
    while start < text.len() {
        let mut end = (start + chunk_bytes).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        if end == start {
            end = text.len();
        }

        pieces.push(Piece {
            buffer: BufferId::Original,
            chunk_index: 0,
            start,
            len: end.saturating_sub(start),
        });
        start = end;
    }

    pieces
}

fn append_text_to_core(core: &mut TextModelCore, text: &str) -> Range<usize> {
    let start = core.len;
    if text.is_empty() {
        return start..start;
    }

    if core.len == 0 && core.pieces.is_empty() {
        let add_chunks = Arc::make_mut(&mut core.add_chunks);
        let chunk_index = add_chunks.len();
        let chunk = Arc::new(String::from(text));
        let len = chunk.len();
        add_chunks.push(chunk);

        core.ascii_only = text.is_ascii();
        core.pieces.push(Piece {
            buffer: BufferId::Add,
            chunk_index,
            start: 0,
            len,
        });
        core.len = len;
        core.line_index = LineIndex::from_text(text);
        core.revision = core.revision.wrapping_add(1).max(1);
        core.materialized = OnceLock::new();
        return 0..len;
    }

    if let Some(inserted_piece) = append_add_piece(core, text) {
        core.ascii_only &= text.is_ascii();
        push_piece_merged(&mut core.pieces, inserted_piece);
        core.len = start.saturating_add(text.len());
        core.line_index.append_text(start, text);
        core.revision = core.revision.wrapping_add(1).max(1);
        core.materialized = OnceLock::new();
    }

    start..start.saturating_add(text.len())
}

fn append_add_piece(core: &mut TextModelCore, text: &str) -> Option<Piece> {
    if text.is_empty() {
        return None;
    }

    let add_chunks = Arc::make_mut(&mut core.add_chunks);
    if let Some(piece) = append_small_text_to_last_add_chunk(add_chunks, text) {
        return Some(piece);
    }

    let chunk_index = add_chunks.len();
    let mut chunk = String::with_capacity(if text.len() <= SMALL_ADD_CHUNK_BYTES {
        SMALL_ADD_CHUNK_BYTES
    } else {
        text.len()
    });
    chunk.push_str(text);
    let len = chunk.len();
    let chunk = Arc::new(chunk);
    add_chunks.push(chunk);
    Some(Piece {
        buffer: BufferId::Add,
        chunk_index,
        start: 0,
        len,
    })
}

fn append_small_text_to_last_add_chunk(
    add_chunks: &mut [Arc<String>],
    text: &str,
) -> Option<Piece> {
    if text.len() > SMALL_ADD_CHUNK_BYTES {
        return None;
    }

    let chunk_index = add_chunks.len().checked_sub(1)?;
    let chunk = Arc::get_mut(add_chunks.get_mut(chunk_index)?)?;
    let start = chunk.len();
    let new_len = start.checked_add(text.len())?;
    if new_len > SMALL_ADD_CHUNK_BYTES {
        return None;
    }

    chunk.push_str(text);
    Some(Piece {
        buffer: BufferId::Add,
        chunk_index,
        start,
        len: text.len(),
    })
}

fn maybe_compact_piece_table(core: &mut TextModelCore) -> bool {
    if core.pieces.len() <= PIECE_TABLE_COMPACTION_THRESHOLD {
        return false;
    }

    let mut text = String::with_capacity(core.len);
    for piece in &core.pieces {
        text.push_str(core.piece_slice(piece));
    }

    let original = Arc::<str>::from(text);
    let shared = SharedString::from(Arc::clone(&original));
    core.original_chunks = Arc::new(vec![original]);
    core.add_chunks = Arc::new(Vec::new());
    core.pieces = build_original_pieces(core.original_chunks[0].as_ref(), LARGE_TEXT_CHUNK_BYTES);
    core.ascii_only = core.original_chunks[0].is_ascii();

    let materialized = OnceLock::new();
    let _ = materialized.set(shared);
    core.materialized = materialized;
    true
}

fn core_clamp_to_char_boundary(core: &TextModelCore, mut offset: usize) -> usize {
    offset = offset.min(core.len);
    if core.ascii_only {
        return offset;
    }

    // Fast path: use the materialized string directly to avoid piece walks in
    // the non-ASCII clamping loop.
    if let Some(text) = core.materialized.get() {
        while offset > 0 && !text.is_char_boundary(offset) {
            offset -= 1;
        }
        return offset;
    }

    while offset > 0 && !core_is_char_boundary(core, offset) {
        offset = offset.saturating_sub(1);
    }
    offset
}

fn core_is_char_boundary(core: &TextModelCore, offset: usize) -> bool {
    if offset == 0 || offset == core.len {
        return true;
    }
    if offset > core.len {
        return false;
    }
    if core.ascii_only {
        return true;
    }

    if let Some(text) = core.materialized.get() {
        return text.is_char_boundary(offset);
    }

    let mut cursor = 0usize;
    for piece in &core.pieces {
        let next = cursor.saturating_add(piece.len);
        if offset == cursor || offset == next {
            return true;
        }
        if offset < next {
            let local = offset.saturating_sub(cursor);
            let chunk = core.chunk_for_piece(piece);
            let absolute = piece.start.saturating_add(local);
            return chunk.is_char_boundary(absolute);
        }
        cursor = next;
    }
    false
}

fn locate_piece_edit_span(pieces: &[Piece], range: Range<usize>) -> PieceEditSpan {
    let mut cursor = 0usize;
    let mut ix = 0usize;
    let mut replace_start = pieces.len();
    let mut prefix_fragment = None;

    while ix < pieces.len() {
        let piece = pieces[ix];
        let piece_start = cursor;
        let piece_end = cursor.saturating_add(piece.len);
        if piece_end <= range.start {
            cursor = piece_end;
            ix += 1;
            continue;
        }

        replace_start = ix;
        if range.start > piece_start {
            prefix_fragment = piece.prefix(range.start.saturating_sub(piece_start));
        }
        break;
    }

    if replace_start == pieces.len() {
        return PieceEditSpan {
            replace_start,
            replace_end: replace_start,
            prefix_fragment: None,
            suffix_fragment: None,
        };
    }

    while ix < pieces.len() {
        let piece = pieces[ix];
        let piece_start = cursor;
        let piece_end = cursor.saturating_add(piece.len);
        if range.end <= piece_start {
            return PieceEditSpan {
                replace_start,
                replace_end: ix,
                prefix_fragment,
                suffix_fragment: None,
            };
        }
        if range.end < piece_end {
            return PieceEditSpan {
                replace_start,
                replace_end: ix.saturating_add(1),
                prefix_fragment,
                suffix_fragment: piece.suffix(range.end.saturating_sub(piece_start)),
            };
        }
        if range.end == piece_end {
            return PieceEditSpan {
                replace_start,
                replace_end: ix.saturating_add(1),
                prefix_fragment,
                suffix_fragment: None,
            };
        }
        cursor = piece_end;
        ix += 1;
    }

    PieceEditSpan {
        replace_start,
        replace_end: pieces.len(),
        prefix_fragment,
        suffix_fragment: None,
    }
}

fn pieces_are_mergeable(left: Piece, right: Piece) -> bool {
    left.len > 0
        && right.len > 0
        && left.buffer == right.buffer
        && left.chunk_index == right.chunk_index
        && left.start.saturating_add(left.len) == right.start
}

fn merge_piece_at(pieces: &mut Vec<Piece>, index: usize) -> bool {
    let Some(right_ix) = index.checked_add(1) else {
        return false;
    };
    if right_ix >= pieces.len() || !pieces_are_mergeable(pieces[index], pieces[right_ix]) {
        return false;
    }

    let right_len = pieces[right_ix].len;
    pieces[index].len = pieces[index].len.saturating_add(right_len);
    pieces.remove(right_ix);
    true
}

fn push_piece_merged(pieces: &mut Vec<Piece>, piece: Piece) {
    if piece.len == 0 {
        return;
    }

    if let Some(last) = pieces.last_mut()
        && pieces_are_mergeable(*last, piece)
    {
        last.len = last.len.saturating_add(piece.len);
        return;
    }

    pieces.push(piece);
}

fn push_piece_merged_small(pieces: &mut SmallVec<[Piece; 3]>, piece: Piece) {
    if piece.len == 0 {
        return;
    }

    if let Some(last) = pieces.last_mut()
        && pieces_are_mergeable(*last, piece)
    {
        last.len = last.len.saturating_add(piece.len);
        return;
    }

    pieces.push(piece);
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
    fn snapshot_slice_borrows_single_piece_ranges_without_materializing() {
        let text = "a".repeat(LARGE_TEXT_CHUNK_BYTES + 32);
        let snapshot = TextModel::from_large_text(text.as_str()).snapshot();

        assert!(snapshot.core.materialized.get().is_none());
        assert_eq!(snapshot.clamp_to_char_boundary(96), 96);
        let prefix = snapshot.slice(0..96);

        assert!(matches!(prefix, Cow::Borrowed(_)));
        assert_eq!(prefix.as_ref(), &text[..96]);
        assert!(snapshot.core.materialized.get().is_none());
    }

    #[test]
    fn snapshot_slice_allocates_across_piece_boundaries_without_materializing() {
        let text = "a".repeat(LARGE_TEXT_CHUNK_BYTES + 32);
        let snapshot = TextModel::from_large_text(text.as_str()).snapshot();
        let range = (LARGE_TEXT_CHUNK_BYTES - 8)..(LARGE_TEXT_CHUNK_BYTES + 8);

        assert!(snapshot.core.materialized.get().is_none());
        let slice = snapshot.slice(range.clone());

        assert!(matches!(slice, Cow::Owned(_)));
        assert_eq!(slice.as_ref(), &text[range]);
        assert!(snapshot.core.materialized.get().is_none());
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
    fn small_unique_inserts_share_one_add_chunk() {
        let mut model = TextModel::from_large_text("body");

        let _ = model.replace_range(4..4, "-tail");
        let _ = model.replace_range(0..0, "head-");
        let _ = model.replace_range(5..5, "mid-");

        assert_eq!(model.as_str(), "head-mid-body-tail");
        assert_eq!(model.core.add_chunks.len(), 1);
        assert!(
            model
                .core
                .pieces
                .iter()
                .filter(|piece| piece.buffer == BufferId::Add)
                .all(|piece| piece.chunk_index == 0)
        );
    }

    #[test]
    fn later_insert_after_snapshot_uses_new_add_chunk() {
        let mut model = TextModel::from_large_text("body");
        let _ = model.replace_range(0..0, "head-");
        let snapshot = model.snapshot();

        let _ = model.replace_range(model.len()..model.len(), "-tail");

        assert_eq!(snapshot.as_str(), "head-body");
        assert_eq!(model.as_str(), "head-body-tail");
        assert_eq!(snapshot.core.add_chunks.len(), 1);
        assert_eq!(model.core.add_chunks.len(), 2);
    }

    #[test]
    fn ascii_fast_path_turns_off_after_unicode_insert() {
        let mut model = TextModel::from_large_text("alpha");
        let inserted = model.replace_range(5..5, "🙂");
        assert_eq!(inserted, 5..9);
        assert_eq!(model.as_str(), "alpha🙂");

        let replaced = model.replace_range(6..9, "!");
        assert_eq!(replaced, 5..6);
        assert_eq!(model.as_str(), "alpha!");
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
    fn from_large_text_uses_one_backing_chunk_with_logical_boundaries() {
        let text = "a".repeat(LARGE_TEXT_CHUNK_BYTES * 2 + 32);
        let model = TextModel::from_large_text(text.as_str());

        assert_eq!(model.core.original_chunks.len(), 1);
        assert_eq!(model.core.original_chunks[0].len(), text.len());
        assert!(model.core.pieces.len() >= 2);
        assert!(
            model
                .core
                .pieces
                .iter()
                .all(|piece| { piece.buffer == BufferId::Original && piece.chunk_index == 0 })
        );
        assert_eq!(
            model
                .core
                .pieces
                .iter()
                .map(|piece| piece.len)
                .sum::<usize>(),
            text.len()
        );
    }

    #[test]
    fn as_shared_string_reuses_original_chunk_when_unedited() {
        let text = "alpha\nbeta\n".repeat(2_048);
        let model = TextModel::from_large_text(text.as_str());

        let shared = model.as_shared_string();
        let shared_arc: Arc<str> = shared.into();

        assert!(Arc::ptr_eq(&shared_arc, &model.core.original_chunks[0]));
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
    fn replace_range_reuses_line_index_storage_when_unique_and_line_count_unchanged() {
        let mut model = TextModel::from_large_text("alpha\nbeta\ngamma");
        let before_ptr = Arc::as_ptr(&model.core.line_index.starts);

        model.replace_range(6..10, "BETA");

        let after_ptr = Arc::as_ptr(&model.core.line_index.starts);
        assert_eq!(model.as_str(), "alpha\nBETA\ngamma");
        assert_eq!(model.line_starts(), &[0, 6, 11]);
        assert_eq!(
            before_ptr, after_ptr,
            "unique edits with unchanged line counts should reuse line-index storage"
        );
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

    #[test]
    fn heavily_fragmented_unique_edits_compact_piece_table() {
        let mut control = "line\n".repeat(LARGE_TEXT_CHUNK_BYTES);
        let mut model = TextModel::from_large_text(control.as_str());

        for ix in 0..(PIECE_TABLE_COMPACTION_THRESHOLD + 32) {
            let offset = (ix * 97) % control.len();
            model.replace_range(offset..offset, "x");
            control.insert(offset, 'x');
        }

        assert_eq!(model.as_str(), control);
        assert!(
            model.core.pieces.len() <= PIECE_TABLE_COMPACTION_THRESHOLD,
            "fragmentation past the threshold should trigger table compaction"
        );
    }
}
