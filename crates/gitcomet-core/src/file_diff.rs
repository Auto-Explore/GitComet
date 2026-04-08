use crate::domain::SharedLineText;
use rustc_hash::FxHasher;
use std::borrow::Cow;
use std::cell::OnceCell;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDiffRowKind {
    Context,
    Add,
    Remove,
    Modify,
}

const REPLACEMENT_ALIGN_CELL_BUDGET: usize = 50_000;
const REPLACEMENT_GAP_COST: u32 = 100;
const REPLACEMENT_PAIR_BASE_COST: u32 = 80;
const REPLACEMENT_PAIR_SCALE_COST: u32 = 120;
const REPLACEMENT_DISSIMILAR_PENALTY_COST: u32 = 40;
const REPLACEMENT_DISSIMILAR_PENALTY_MIN_LEN: usize = 4;
const ASCII_BITPARALLEL_MAX_PATTERN_LEN: usize = 128;
const SIDE_BY_SIDE_HISTOGRAM_LINE_THRESHOLD: usize = 1_024;
const SIDE_BY_SIDE_LINEAR_FALLBACK_LINE_THRESHOLD: usize = 100_000;
const PATIENCE_POSITIONAL_FALLBACK_LINE_THRESHOLD: usize = 2_048;
const SIDE_BY_SIDE_SPARSE_POSITIONAL_MAX_CHANGED_RATIO_DENOMINATOR: usize = 4;
const SIDE_BY_SIDE_SPARSE_POSITIONAL_MAX_BLOCK_LEN: usize = 1;
// UTF-8 code points are at most 4 bytes wide, so 3 bytes of lookaround is
// enough to recover the nearest character boundary around any requested slice.
const UTF8_SUBSLICE_BOUNDARY_LOOKAROUND_BYTES: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDiffEofNewline {
    MissingInOld,
    MissingInNew,
}

#[derive(Clone, Debug)]
enum FileDiffLineTextStorage {
    Owned(Arc<str>),
    SharedSlice { text: Arc<str>, range: Range<usize> },
    SharedLine(SharedLineText),
    FileSlice(Arc<FileDiffLineFileSlice>),
}

#[derive(Clone, Debug)]
pub struct FileDiffLineText {
    storage: FileDiffLineTextStorage,
}

#[derive(Debug)]
struct FileDiffLineFileSlice {
    path: Arc<PathBuf>,
    range: Range<usize>,
    ascii_only: bool,
    has_tabs: bool,
    text: OnceLock<Arc<str>>,
}

fn read_file_bytes(path: &PathBuf, range: Range<usize>) -> Option<Vec<u8>> {
    if range.start > range.end {
        return None;
    }

    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(u64::try_from(range.start).ok()?))
        .ok()?;
    let mut bytes = vec![0u8; range.end.saturating_sub(range.start)];
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

fn read_utf8_file_slice(path: &PathBuf, range: Range<usize>) -> Option<Arc<str>> {
    let bytes = read_file_bytes(path, range)?;
    let text = String::from_utf8(bytes).ok()?;
    Some(Arc::from(text))
}

fn read_utf8_file_subslice(
    path: &PathBuf,
    base_range: &Range<usize>,
    subrange: Range<usize>,
    ascii_only: bool,
) -> Option<Arc<str>> {
    read_utf8_file_subslice_with_range(path, base_range, subrange, ascii_only).map(|(text, _)| text)
}

fn clamp_byte_up_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset < text.len() && !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

fn clamp_byte_down_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn resolved_utf8_text_subslice_range(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    if range.start > range.end || range.end > text.len() {
        return None;
    }

    let start = clamp_byte_up_to_char_boundary(text, range.start);
    let end = clamp_byte_down_to_char_boundary(text, range.end).max(start);
    Some(start..end)
}

fn read_utf8_file_subslice_with_range(
    path: &PathBuf,
    base_range: &Range<usize>,
    subrange: Range<usize>,
    ascii_only: bool,
) -> Option<(Arc<str>, Range<usize>)> {
    if subrange.start > subrange.end
        || subrange.end > base_range.end.saturating_sub(base_range.start)
    {
        return None;
    }

    let start = base_range.start.saturating_add(subrange.start);
    let end = base_range.start.saturating_add(subrange.end);
    if ascii_only {
        return read_utf8_file_slice(path, start..end).map(|text| (text, subrange));
    }

    let read_start = start
        .saturating_sub(UTF8_SUBSLICE_BOUNDARY_LOOKAROUND_BYTES)
        .max(base_range.start);
    let read_end = end
        .saturating_add(UTF8_SUBSLICE_BOUNDARY_LOOKAROUND_BYTES)
        .min(base_range.end);
    let bytes = read_file_bytes(path, read_start..read_end)?;
    let requested_start = start.saturating_sub(read_start).min(bytes.len());
    let requested_end = end.saturating_sub(read_start).min(bytes.len());

    let mut local_start = requested_start;
    while local_start < bytes.len() && (bytes[local_start] & 0b1100_0000) == 0b1000_0000 {
        local_start += 1;
    }

    let mut local_end = requested_end.max(local_start).min(bytes.len());
    while local_end > local_start && std::str::from_utf8(&bytes[local_start..local_end]).is_err() {
        local_end -= 1;
    }

    let resolved_range = read_start
        .saturating_add(local_start)
        .saturating_sub(base_range.start)
        ..read_start
            .saturating_add(local_end)
            .saturating_sub(base_range.start);
    if local_end <= local_start {
        return Some((Arc::from(""), resolved_range));
    }

    String::from_utf8(bytes[local_start..local_end].to_vec())
        .ok()
        .map(Arc::from)
        .map(|text| (text, resolved_range))
}

impl FileDiffLineText {
    pub fn shared(text: Arc<str>) -> Self {
        Self {
            storage: FileDiffLineTextStorage::Owned(text),
        }
    }

    pub fn shared_slice(text: Arc<str>, range: Range<usize>) -> Self {
        debug_assert!(
            text.get(range.clone()).is_some(),
            "shared file-diff line range should stay within bounds"
        );
        Self {
            storage: FileDiffLineTextStorage::SharedSlice { text, range },
        }
    }

    pub fn shared_line(text: SharedLineText) -> Self {
        Self {
            storage: FileDiffLineTextStorage::SharedLine(text),
        }
    }

    pub fn file_slice(
        path: Arc<PathBuf>,
        range: Range<usize>,
        ascii_only: bool,
        has_tabs: bool,
    ) -> Self {
        Self {
            storage: FileDiffLineTextStorage::FileSlice(Arc::new(FileDiffLineFileSlice {
                path,
                range,
                ascii_only,
                has_tabs,
                text: OnceLock::new(),
            })),
        }
    }

    pub fn as_str(&self) -> &str {
        match &self.storage {
            FileDiffLineTextStorage::Owned(text) => text.as_ref(),
            FileDiffLineTextStorage::SharedSlice { text, range } => text
                .get(range.clone())
                .expect("shared file-diff line range should stay valid"),
            FileDiffLineTextStorage::SharedLine(text) => text.as_ref(),
            FileDiffLineTextStorage::FileSlice(slice) => slice
                .text
                .get_or_init(|| {
                    read_utf8_file_slice(slice.path.as_ref(), slice.range.clone())
                        .unwrap_or_else(|| Arc::<str>::from(""))
                })
                .as_ref(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    pub fn len(&self) -> usize {
        match &self.storage {
            FileDiffLineTextStorage::Owned(text) => text.len(),
            FileDiffLineTextStorage::SharedSlice { range, .. } => {
                range.end.saturating_sub(range.start)
            }
            FileDiffLineTextStorage::SharedLine(text) => text.len(),
            FileDiffLineTextStorage::FileSlice(slice) => {
                slice.range.end.saturating_sub(slice.range.start)
            }
        }
    }

    pub fn is_ascii_without_loading(&self) -> bool {
        match &self.storage {
            FileDiffLineTextStorage::Owned(text) => text.is_ascii(),
            FileDiffLineTextStorage::SharedSlice { text, range } => text
                .as_bytes()
                .get(range.clone())
                .is_some_and(|bytes| bytes.is_ascii()),
            FileDiffLineTextStorage::SharedLine(text) => text.as_ref().is_ascii(),
            FileDiffLineTextStorage::FileSlice(slice) => slice.ascii_only,
        }
    }

    pub fn has_tabs_without_loading(&self) -> bool {
        match &self.storage {
            FileDiffLineTextStorage::Owned(text) => text.contains('\t'),
            FileDiffLineTextStorage::SharedSlice { text, range } => text
                .as_bytes()
                .get(range.clone())
                .is_some_and(|bytes| bytes.contains(&b'\t')),
            FileDiffLineTextStorage::SharedLine(text) => text.as_ref().contains('\t'),
            FileDiffLineTextStorage::FileSlice(slice) => slice.has_tabs,
        }
    }

    pub fn slice_bytes(&self, range: Range<usize>) -> Option<Cow<'_, [u8]>> {
        if range.start > range.end || range.end > self.len() {
            return None;
        }

        match &self.storage {
            FileDiffLineTextStorage::Owned(text) => Some(Cow::Borrowed(
                text.as_bytes().get(range).unwrap_or_default(),
            )),
            FileDiffLineTextStorage::SharedSlice { text, range: base } => {
                let start = base.start.saturating_add(range.start);
                let end = base.start.saturating_add(range.end);
                Some(Cow::Borrowed(
                    text.as_bytes().get(start..end).unwrap_or_default(),
                ))
            }
            FileDiffLineTextStorage::SharedLine(text) => Some(Cow::Borrowed(
                text.as_ref().as_bytes().get(range).unwrap_or_default(),
            )),
            FileDiffLineTextStorage::FileSlice(slice) => {
                let start = slice.range.start.saturating_add(range.start);
                let end = slice.range.start.saturating_add(range.end);
                read_file_bytes(slice.path.as_ref(), start..end).map(Cow::Owned)
            }
        }
    }

    pub fn slice_text(&self, range: Range<usize>) -> Option<Cow<'_, str>> {
        if range.start > range.end || range.end > self.len() {
            return None;
        }

        match &self.storage {
            FileDiffLineTextStorage::Owned(text) => {
                Some(Cow::Borrowed(text.get(range).unwrap_or_default()))
            }
            FileDiffLineTextStorage::SharedSlice { text, range: base } => {
                let start = base.start.saturating_add(range.start);
                let end = base.start.saturating_add(range.end);
                Some(Cow::Borrowed(text.get(start..end).unwrap_or_default()))
            }
            FileDiffLineTextStorage::SharedLine(text) => {
                Some(Cow::Borrowed(text.as_ref().get(range).unwrap_or_default()))
            }
            FileDiffLineTextStorage::FileSlice(slice) => {
                read_utf8_file_subslice(slice.path.as_ref(), &slice.range, range, slice.ascii_only)
                    .map(|text| Cow::Owned(text.as_ref().to_string()))
            }
        }
    }

    pub fn slice_text_resolved(&self, range: Range<usize>) -> Option<(Cow<'_, str>, Range<usize>)> {
        if range.start > range.end || range.end > self.len() {
            return None;
        }

        match &self.storage {
            FileDiffLineTextStorage::Owned(text) => {
                let resolved_range = resolved_utf8_text_subslice_range(text.as_ref(), range)?;
                Some((
                    Cow::Borrowed(text.get(resolved_range.clone()).unwrap_or_default()),
                    resolved_range,
                ))
            }
            FileDiffLineTextStorage::SharedSlice { text, range: base } => {
                let start = base.start.saturating_add(range.start);
                let end = base.start.saturating_add(range.end);
                let resolved_absolute =
                    resolved_utf8_text_subslice_range(text.as_ref(), start..end)?;
                let resolved_relative = resolved_absolute.start.saturating_sub(base.start)
                    ..resolved_absolute.end.saturating_sub(base.start);
                Some((
                    Cow::Borrowed(text.get(resolved_absolute).unwrap_or_default()),
                    resolved_relative,
                ))
            }
            FileDiffLineTextStorage::SharedLine(text) => {
                let resolved_range = resolved_utf8_text_subslice_range(text.as_ref(), range)?;
                Some((
                    Cow::Borrowed(
                        text.as_ref()
                            .get(resolved_range.clone())
                            .unwrap_or_default(),
                    ),
                    resolved_range,
                ))
            }
            FileDiffLineTextStorage::FileSlice(slice) => read_utf8_file_subslice_with_range(
                slice.path.as_ref(),
                &slice.range,
                range,
                slice.ascii_only,
            )
            .map(|(text, resolved_range)| (Cow::Owned(text.as_ref().to_string()), resolved_range)),
        }
    }

    pub fn shares_backing_with(&self, other: &Self) -> bool {
        match (&self.storage, &other.storage) {
            (FileDiffLineTextStorage::Owned(a), FileDiffLineTextStorage::Owned(b))
            | (
                FileDiffLineTextStorage::Owned(a),
                FileDiffLineTextStorage::SharedSlice { text: b, .. },
            )
            | (
                FileDiffLineTextStorage::SharedSlice { text: a, .. },
                FileDiffLineTextStorage::Owned(b),
            )
            | (
                FileDiffLineTextStorage::SharedSlice { text: a, .. },
                FileDiffLineTextStorage::SharedSlice { text: b, .. },
            ) => Arc::ptr_eq(a, b),
            (FileDiffLineTextStorage::SharedLine(a), FileDiffLineTextStorage::SharedLine(b)) => {
                a.shares_storage_with(b)
            }
            (FileDiffLineTextStorage::FileSlice(a), FileDiffLineTextStorage::FileSlice(b)) => {
                Arc::ptr_eq(a, b)
            }
            _ => false,
        }
    }

    pub fn identity_hash_without_loading(&self) -> u64 {
        let mut hasher = FxHasher::default();
        match &self.storage {
            FileDiffLineTextStorage::Owned(text) => {
                0u8.hash(&mut hasher);
                (text.as_ptr() as usize).hash(&mut hasher);
                text.len().hash(&mut hasher);
            }
            FileDiffLineTextStorage::SharedSlice { text, range } => {
                1u8.hash(&mut hasher);
                (text.as_ptr() as usize).hash(&mut hasher);
                text.len().hash(&mut hasher);
                range.start.hash(&mut hasher);
                range.end.hash(&mut hasher);
            }
            FileDiffLineTextStorage::SharedLine(text) => {
                2u8.hash(&mut hasher);
                text.as_ref().hash(&mut hasher);
            }
            FileDiffLineTextStorage::FileSlice(slice) => {
                3u8.hash(&mut hasher);
                slice.path.hash(&mut hasher);
                slice.range.start.hash(&mut hasher);
                slice.range.end.hash(&mut hasher);
                slice.ascii_only.hash(&mut hasher);
                slice.has_tabs.hash(&mut hasher);
            }
        }
        hasher.finish()
    }
}

impl AsRef<str> for FileDiffLineText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::ops::Deref for FileDiffLineText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl PartialEq for FileDiffLineText {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for FileDiffLineText {}

impl Hash for FileDiffLineText {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl From<&str> for FileDiffLineText {
    fn from(value: &str) -> Self {
        Self::shared(Arc::from(value))
    }
}

impl From<String> for FileDiffLineText {
    fn from(value: String) -> Self {
        Self::shared(value.into())
    }
}

impl From<Arc<str>> for FileDiffLineText {
    fn from(value: Arc<str>) -> Self {
        Self::shared(value)
    }
}

impl From<SharedLineText> for FileDiffLineText {
    fn from(value: SharedLineText) -> Self {
        Self::shared_line(value)
    }
}

impl From<FileDiffLineText> for Arc<str> {
    fn from(value: FileDiffLineText) -> Self {
        match value.storage {
            FileDiffLineTextStorage::Owned(text) => text,
            FileDiffLineTextStorage::SharedSlice { text, range } => Arc::from(
                text.get(range)
                    .expect("shared file-diff line range should stay valid"),
            ),
            FileDiffLineTextStorage::SharedLine(text) => text.to_arc(),
            FileDiffLineTextStorage::FileSlice(slice) => slice
                .text
                .get_or_init(|| {
                    read_utf8_file_slice(slice.path.as_ref(), slice.range.clone())
                        .unwrap_or_else(|| Arc::<str>::from(""))
                })
                .clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffRow {
    pub kind: FileDiffRowKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub old: Option<FileDiffLineText>,
    pub new: Option<FileDiffLineText>,
    pub eof_newline: Option<FileDiffEofNewline>,
}

/// Stable anchor metadata for a rendered side-by-side diff row.
///
/// `region_id`/`ordinal_in_region` are only populated for non-context rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileDiffRowAnchor {
    pub row_index: usize,
    pub region_id: Option<u32>,
    pub ordinal_in_region: Option<u32>,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

/// Stable anchor metadata for one contiguous changed region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileDiffRegionAnchor {
    pub region_id: u32,
    pub row_start: usize,
    pub row_end_exclusive: usize,
    pub old_start_line: Option<u32>,
    pub old_end_line: Option<u32>,
    pub new_start_line: Option<u32>,
    pub new_end_line: Option<u32>,
}

/// Anchors for all rows and change regions in a side-by-side diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffAnchors {
    pub row_anchors: Vec<FileDiffRowAnchor>,
    pub region_anchors: Vec<FileDiffRegionAnchor>,
}

/// Side-by-side diff rows along with stable row/region anchors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffRowsWithAnchors {
    pub rows: Vec<FileDiffRow>,
    pub anchors: FileDiffAnchors,
}

/// Compact plan for a streamed side-by-side diff.
///
/// Runs carry only line-index spans into the old/new source documents. UI code
/// can materialize rows page-by-page without cloning the entire file into a
/// `Vec<FileDiffRow>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileDiffPlanRun {
    Context {
        old_start: usize,
        new_start: usize,
        len: usize,
    },
    Remove {
        old_start: usize,
        len: usize,
    },
    Add {
        new_start: usize,
        len: usize,
    },
    Modify {
        old_start: usize,
        new_start: usize,
        len: usize,
    },
}

impl FileDiffPlanRun {
    pub fn row_len(&self) -> usize {
        match self {
            Self::Context { len, .. }
            | Self::Remove { len, .. }
            | Self::Add { len, .. }
            | Self::Modify { len, .. } => *len,
        }
    }

    pub fn inline_row_len(&self) -> usize {
        match self {
            Self::Modify { len, .. } => len.saturating_mul(2),
            _ => self.row_len(),
        }
    }

    pub fn kind(&self) -> FileDiffRowKind {
        match self {
            Self::Context { .. } => FileDiffRowKind::Context,
            Self::Remove { .. } => FileDiffRowKind::Remove,
            Self::Add { .. } => FileDiffRowKind::Add,
            Self::Modify { .. } => FileDiffRowKind::Modify,
        }
    }
}

/// Compact whole-file plan used by the streamed UI runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffPlan {
    pub runs: Vec<FileDiffPlanRun>,
    pub row_count: usize,
    pub inline_row_count: usize,
    pub eof_newline: Option<FileDiffEofNewline>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditKind {
    Equal,
    Insert,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Edit<'a> {
    pub(crate) kind: EditKind,
    pub(crate) old: Option<&'a str>,
    pub(crate) new: Option<&'a str>,
}

/// A contiguous edit span relative to base lines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiffHunk<T> {
    pub(crate) base_start: usize,
    pub(crate) base_end: usize,
    pub(crate) new_lines: Vec<T>,
}

/// Convert an edit script into base-relative change hunks.
pub(crate) fn edits_to_hunks_with<'a, T, F>(
    edits: &[Edit<'a>],
    mut map_insert: F,
) -> Vec<DiffHunk<T>>
where
    F: FnMut(&'a str) -> T,
{
    let mut hunks = Vec::new();
    let mut base_ix = 0usize;
    let mut i = 0usize;

    while i < edits.len() {
        if edits[i].kind == EditKind::Equal {
            base_ix += 1;
            i += 1;
            continue;
        }

        let hunk_base_start = base_ix;
        let mut new_lines = Vec::new();

        while i < edits.len() && edits[i].kind != EditKind::Equal {
            match edits[i].kind {
                EditKind::Delete => {
                    base_ix += 1;
                }
                EditKind::Insert => {
                    new_lines.push(map_insert(edits[i].new.unwrap_or_default()));
                }
                EditKind::Equal => unreachable!(),
            }
            i += 1;
        }

        hunks.push(DiffHunk {
            base_start: hunk_base_start,
            base_end: base_ix,
            new_lines,
        });
    }

    hunks
}

/// Reconstruct one side's sequence for a base range by applying hunks.
pub(crate) fn reconstruct_side_with<'a, T, FBase>(
    base_lines: &'a [&'a str],
    range: std::ops::Range<usize>,
    hunks: &[DiffHunk<T>],
    output: &mut Vec<T>,
    mut map_base_line: FBase,
) where
    T: Clone,
    FBase: FnMut(&'a str) -> T,
{
    let range_end = range.end.min(base_lines.len());
    let mut pos = range.start.min(range_end);

    for hunk in hunks {
        let base_limit = hunk.base_start.min(range_end).max(pos);
        for &line in &base_lines[pos..base_limit] {
            output.push(map_base_line(line));
        }
        output.extend(hunk.new_lines.iter().cloned());
        pos = hunk.base_end.min(range_end).max(pos);
    }

    for &line in &base_lines[pos..range_end] {
        output.push(map_base_line(line));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplacementAlignStep {
    None,
    Pair,
    Delete,
    Insert,
}

#[cfg(feature = "benchmarks")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BenchmarkReplacementDistanceBackend {
    Scratch,
    Strsim,
}

pub fn side_by_side_plan(old: &str, new: &str) -> FileDiffPlan {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    side_by_side_plan_from_lines(old, new, old_lines.as_slice(), new_lines.as_slice())
}

/// Build a side-by-side diff plan from precomputed `str::lines()` slices.
///
/// Callers must ensure `old_lines` and `new_lines` were derived from
/// `old_text`/`new_text` using the same line-splitting semantics as
/// [`str::lines`], because EOF newline handling still comes from the full
/// source texts.
pub fn side_by_side_plan_from_lines(
    old_text: &str,
    new_text: &str,
    old_lines: &[&str],
    new_lines: &[&str],
) -> FileDiffPlan {
    build_side_by_side_plan_with_pair_cost(
        old_text,
        new_text,
        old_lines,
        new_lines,
        replacement_pair_cost,
    )
}

#[cfg(feature = "benchmarks")]
pub fn benchmark_side_by_side_plan_with_replacement_backend(
    old: &str,
    new: &str,
    backend: BenchmarkReplacementDistanceBackend,
) -> FileDiffPlan {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    match backend {
        BenchmarkReplacementDistanceBackend::Scratch => build_side_by_side_plan_with_pair_cost(
            old,
            new,
            old_lines.as_slice(),
            new_lines.as_slice(),
            replacement_pair_cost_with_scratch,
        ),
        BenchmarkReplacementDistanceBackend::Strsim => build_side_by_side_plan_with_pair_cost(
            old,
            new,
            old_lines.as_slice(),
            new_lines.as_slice(),
            replacement_pair_cost_with_strsim,
        ),
    }
}

pub fn side_by_side_rows(old: &str, new: &str) -> Vec<FileDiffRow> {
    let old_text: Arc<str> = Arc::from(old);
    let new_text: Arc<str> = Arc::from(new);
    let old_lines = split_lines(old_text.as_ref());
    let new_lines = split_lines(new_text.as_ref());
    let plan = build_side_by_side_plan_with_pair_cost(
        old_text.as_ref(),
        new_text.as_ref(),
        old_lines.as_slice(),
        new_lines.as_slice(),
        replacement_pair_cost,
    );
    materialize_rows_from_plan(
        &plan,
        &old_text,
        old_lines.as_slice(),
        &new_text,
        new_lines.as_slice(),
    )
}

pub fn append_side_by_side_rows_with_offsets(
    rows: &mut Vec<FileDiffRow>,
    old: &str,
    new: &str,
    old_line_offset: u32,
    new_line_offset: u32,
) {
    let old_text: Arc<str> = Arc::from(old);
    let new_text: Arc<str> = Arc::from(new);
    let old_lines = split_lines(old_text.as_ref());
    let new_lines = split_lines(new_text.as_ref());
    let plan = build_side_by_side_plan_with_pair_cost(
        old_text.as_ref(),
        new_text.as_ref(),
        old_lines.as_slice(),
        new_lines.as_slice(),
        replacement_pair_cost,
    );
    materialize_rows_from_plan_into(
        rows,
        &plan,
        &old_text,
        old_lines.as_slice(),
        &new_text,
        new_lines.as_slice(),
        old_line_offset,
        new_line_offset,
    )
}

pub fn side_by_side_rows_with_anchors(old: &str, new: &str) -> FileDiffRowsWithAnchors {
    let rows = side_by_side_rows(old, new);
    let anchors = compute_row_region_anchors(&rows);
    FileDiffRowsWithAnchors { rows, anchors }
}

pub fn plan_row_region_anchors(plan: &FileDiffPlan) -> FileDiffAnchors {
    let mut builder = FileDiffAnchorBuilder::with_capacity(plan.row_count);
    for_each_plan_row_meta(plan, |row_index, row| builder.push(row_index, row));
    builder.finish(plan.row_count)
}

pub fn plan_emitted_line_prefix_counts(plan: &FileDiffPlan) -> (Vec<usize>, Vec<usize>) {
    let mut old_prefix = Vec::with_capacity(plan.row_count.saturating_add(1));
    let mut new_prefix = Vec::with_capacity(plan.row_count.saturating_add(1));
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    old_prefix.push(0);
    new_prefix.push(0);

    for_each_plan_row_meta(plan, |_row_index, row| {
        if row.old_line.is_some() {
            old_count = old_count.saturating_add(1);
        }
        if row.new_line.is_some() {
            new_count = new_count.saturating_add(1);
        }
        old_prefix.push(old_count);
        new_prefix.push(new_count);
    });

    (old_prefix, new_prefix)
}

pub fn plan_changed_line_masks(
    plan: &FileDiffPlan,
    old_line_count: usize,
    new_line_count: usize,
) -> (Vec<bool>, Vec<bool>) {
    let mut old_mask = vec![false; old_line_count];
    let mut new_mask = vec![false; new_line_count];

    for_each_plan_row_meta(plan, |_row_index, row| match row.kind {
        FileDiffRowKind::Context => {}
        FileDiffRowKind::Remove => mark_changed_line(old_mask.as_mut_slice(), row.old_line),
        FileDiffRowKind::Add => mark_changed_line(new_mask.as_mut_slice(), row.new_line),
        FileDiffRowKind::Modify => {
            mark_changed_line(old_mask.as_mut_slice(), row.old_line);
            mark_changed_line(new_mask.as_mut_slice(), row.new_line);
        }
    });

    (old_mask, new_mask)
}

pub fn plan_line_to_row_maps(
    plan: &FileDiffPlan,
    old_line_count: usize,
    new_line_count: usize,
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let mut old_line_to_row = vec![None; old_line_count];
    let mut new_line_to_row = vec![None; new_line_count];

    for_each_plan_row_meta(plan, |row_index, row| {
        assign_line_to_row(old_line_to_row.as_mut_slice(), row.old_line, row_index);
        assign_line_to_row(new_line_to_row.as_mut_slice(), row.new_line, row_index);
    });

    (old_line_to_row, new_line_to_row)
}

/// A borrowed view of a single side-by-side diff row.
///
/// This is the zero-allocation equivalent of iterating `side_by_side_rows()`:
/// text references point directly into the source line slices instead of being
/// cloned into owned `String`s.
#[derive(Clone, Copy, Debug)]
pub enum PlanRowView<'a> {
    Context {
        old_line: u32,
        new_line: u32,
        text: &'a str,
    },
    Remove {
        old_line: u32,
        text: &'a str,
    },
    Add {
        new_line: u32,
        text: &'a str,
    },
    Modify {
        old_line: u32,
        new_line: u32,
        old_text: &'a str,
        new_text: &'a str,
    },
}

impl PlanRowView<'_> {
    pub fn kind(&self) -> FileDiffRowKind {
        match self {
            Self::Context { .. } => FileDiffRowKind::Context,
            Self::Remove { .. } => FileDiffRowKind::Remove,
            Self::Add { .. } => FileDiffRowKind::Add,
            Self::Modify { .. } => FileDiffRowKind::Modify,
        }
    }
}

/// Iterate over side-by-side diff rows with borrowed text, avoiding the
/// `Vec<FileDiffRow>` materialization that `side_by_side_rows()` performs.
///
/// Internally computes the diff plan and walks it, yielding `PlanRowView`
/// references into the source texts.
pub fn for_each_side_by_side_row<'a>(
    old: &'a str,
    new: &'a str,
    mut f: impl FnMut(PlanRowView<'a>),
) {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let plan = build_side_by_side_plan_with_pair_cost(
        old,
        new,
        old_lines.as_slice(),
        new_lines.as_slice(),
        replacement_pair_cost,
    );

    for run in &plan.runs {
        match *run {
            FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    if let (Some(ol), Some(nl)) =
                        (one_based_line_number(old_ix), one_based_line_number(new_ix))
                    {
                        let text = old_lines.get(old_ix).copied().unwrap_or_default();
                        f(PlanRowView::Context {
                            old_line: ol,
                            new_line: nl,
                            text,
                        });
                    }
                }
            }
            FileDiffPlanRun::Remove { old_start, len } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    if let Some(ol) = one_based_line_number(old_ix) {
                        let text = old_lines.get(old_ix).copied().unwrap_or_default();
                        f(PlanRowView::Remove { old_line: ol, text });
                    }
                }
            }
            FileDiffPlanRun::Add { new_start, len } => {
                for offset in 0..len {
                    let new_ix = new_start.saturating_add(offset);
                    if let Some(nl) = one_based_line_number(new_ix) {
                        let text = new_lines.get(new_ix).copied().unwrap_or_default();
                        f(PlanRowView::Add { new_line: nl, text });
                    }
                }
            }
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    if let (Some(ol), Some(nl)) =
                        (one_based_line_number(old_ix), one_based_line_number(new_ix))
                    {
                        let old_text = old_lines.get(old_ix).copied().unwrap_or_default();
                        let new_text = new_lines.get(new_ix).copied().unwrap_or_default();
                        f(PlanRowView::Modify {
                            old_line: ol,
                            new_line: nl,
                            old_text,
                            new_text,
                        });
                    }
                }
            }
        }
    }
}

pub(crate) fn compute_row_region_anchors(rows: &[FileDiffRow]) -> FileDiffAnchors {
    let mut builder = FileDiffAnchorBuilder::with_capacity(rows.len());
    for (row_index, row) in rows.iter().enumerate() {
        builder.push(row_index, row.into());
    }
    builder.finish(rows.len())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiffRowMeta {
    kind: FileDiffRowKind,
    old_line: Option<u32>,
    new_line: Option<u32>,
}

impl From<&FileDiffRow> for DiffRowMeta {
    fn from(row: &FileDiffRow) -> Self {
        Self {
            kind: row.kind,
            old_line: row.old_line,
            new_line: row.new_line,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ActiveRegion {
    region_id: u32,
    row_start: usize,
    old_start_line: Option<u32>,
    old_end_line: Option<u32>,
    new_start_line: Option<u32>,
    new_end_line: Option<u32>,
    next_ordinal: u32,
}

impl ActiveRegion {
    fn new(region_id: u32, row_start: usize) -> Self {
        Self {
            region_id,
            row_start,
            old_start_line: None,
            old_end_line: None,
            new_start_line: None,
            new_end_line: None,
            next_ordinal: 0,
        }
    }

    fn update_lines(&mut self, row: DiffRowMeta) {
        if let Some(old_line) = row.old_line {
            self.old_start_line = Some(
                self.old_start_line
                    .map_or(old_line, |line| line.min(old_line)),
            );
            self.old_end_line = Some(
                self.old_end_line
                    .map_or(old_line, |line| line.max(old_line)),
            );
        }
        if let Some(new_line) = row.new_line {
            self.new_start_line = Some(
                self.new_start_line
                    .map_or(new_line, |line| line.min(new_line)),
            );
            self.new_end_line = Some(
                self.new_end_line
                    .map_or(new_line, |line| line.max(new_line)),
            );
        }
    }

    fn as_region_anchor(self, row_end_exclusive: usize) -> FileDiffRegionAnchor {
        FileDiffRegionAnchor {
            region_id: self.region_id,
            row_start: self.row_start,
            row_end_exclusive,
            old_start_line: self.old_start_line,
            old_end_line: self.old_end_line,
            new_start_line: self.new_start_line,
            new_end_line: self.new_end_line,
        }
    }
}

struct FileDiffAnchorBuilder {
    row_anchors: Vec<FileDiffRowAnchor>,
    region_anchors: Vec<FileDiffRegionAnchor>,
    active_region: Option<ActiveRegion>,
}

impl FileDiffAnchorBuilder {
    fn with_capacity(row_count_hint: usize) -> Self {
        Self {
            row_anchors: Vec::with_capacity(row_count_hint),
            region_anchors: Vec::new(),
            active_region: None,
        }
    }

    fn push(&mut self, row_index: usize, row: DiffRowMeta) {
        if row.kind == FileDiffRowKind::Context {
            if let Some(region) = self.active_region.take() {
                self.region_anchors.push(region.as_region_anchor(row_index));
            }
            self.row_anchors.push(FileDiffRowAnchor {
                row_index,
                region_id: None,
                ordinal_in_region: None,
                old_line: row.old_line,
                new_line: row.new_line,
            });
            return;
        }

        let region = self.active_region.get_or_insert_with(|| {
            let region_id = self.region_anchors.len() as u32;
            ActiveRegion::new(region_id, row_index)
        });
        region.update_lines(row);
        let ordinal_in_region = region.next_ordinal;
        region.next_ordinal = region.next_ordinal.saturating_add(1);

        self.row_anchors.push(FileDiffRowAnchor {
            row_index,
            region_id: Some(region.region_id),
            ordinal_in_region: Some(ordinal_in_region),
            old_line: row.old_line,
            new_line: row.new_line,
        });
    }

    fn finish(mut self, row_count: usize) -> FileDiffAnchors {
        if let Some(region) = self.active_region.take() {
            self.region_anchors.push(region.as_region_anchor(row_count));
        }
        FileDiffAnchors {
            row_anchors: self.row_anchors,
            region_anchors: self.region_anchors,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlannedReplacementOp {
    Pair,
    Delete,
    Insert,
}

struct PreparedReplacementLine<'a> {
    text: &'a str,
    ascii_bytes: Option<&'a [u8]>,
    chars: OnceCell<Box<[char]>>,
}

impl<'a> PreparedReplacementLine<'a> {
    fn new(text: &'a str) -> Self {
        if text.is_ascii() {
            Self {
                text,
                ascii_bytes: Some(text.as_bytes()),
                chars: OnceCell::new(),
            }
        } else {
            let prepared_chars = text.chars().collect::<Vec<_>>().into_boxed_slice();
            let chars = OnceCell::new();
            assert!(
                chars.set(prepared_chars).is_ok(),
                "fresh OnceCell should accept prepared chars"
            );
            Self {
                text,
                ascii_bytes: None,
                chars,
            }
        }
    }

    fn ascii_bytes(&self) -> Option<&[u8]> {
        self.ascii_bytes
    }

    fn chars(&self) -> &[char] {
        self.chars
            .get_or_init(|| self.text.chars().collect::<Vec<_>>().into_boxed_slice())
            .as_ref()
    }
}

#[cfg(feature = "benchmarks")]
struct CharSlice<'a>(&'a [char]);

#[cfg(feature = "benchmarks")]
impl<'b> IntoIterator for &CharSlice<'b> {
    type Item = char;
    type IntoIter = std::iter::Copied<std::slice::Iter<'b, char>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().copied()
    }
}

#[cfg(all(feature = "benchmarks", test))]
struct ByteSlice<'a>(&'a [u8]);

#[cfg(all(feature = "benchmarks", test))]
impl<'b> IntoIterator for &ByteSlice<'b> {
    type Item = u8;
    type IntoIter = std::iter::Copied<std::slice::Iter<'b, u8>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().copied()
    }
}

#[derive(Default)]
struct LevenshteinScratch {
    cache: Vec<usize>,
}

impl LevenshteinScratch {
    fn distance<T: Eq>(&mut self, a: &[T], b: &[T]) -> usize {
        if a == b {
            return 0;
        }
        if a.is_empty() {
            return b.len();
        }
        if b.is_empty() {
            return a.len();
        }

        self.distance_non_empty_unequal(a, b)
    }

    fn distance_non_empty_unequal<T: Eq>(&mut self, a: &[T], b: &[T]) -> usize {
        distance_non_empty_unequal_with_cache(&mut self.cache, a, b)
    }

    fn distance_bytes(&mut self, a: &[u8], b: &[u8]) -> usize {
        if a == b {
            return 0;
        }
        if a.is_empty() {
            return b.len();
        }
        if b.is_empty() {
            return a.len();
        }

        if let Some(distance) = bitparallel_levenshtein_bytes(a, b) {
            return distance;
        }

        distance_non_empty_unequal_with_cache(&mut self.cache, a, b)
    }
}

fn distance_non_empty_unequal_with_cache<T: Eq>(cache: &mut Vec<usize>, a: &[T], b: &[T]) -> usize {
    debug_assert!(!a.is_empty());
    debug_assert!(!b.is_empty());

    let (a, b) = if b.len() > a.len() { (a, b) } else { (b, a) };
    debug_assert!(a != b);

    let b_len = b.len();
    cache.resize(b_len, 0);
    let cache = &mut cache[..b_len];
    for (ix, slot) in cache.iter_mut().enumerate() {
        *slot = ix + 1;
    }

    let mut result = b_len;
    for (i, a_ch) in a.iter().enumerate() {
        result = i + 1;
        let mut distance_b = i;
        for (j, b_ch) in b.iter().enumerate() {
            let cost = usize::from(a_ch != b_ch);
            let distance_a = distance_b + cost;
            distance_b = cache[j];
            result = (result + 1).min(distance_a).min(distance_b + 1);
            cache[j] = result;
        }
    }

    result
}

fn bitparallel_levenshtein_bytes(a: &[u8], b: &[u8]) -> Option<usize> {
    let (pattern, text) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let pattern_len = pattern.len();
    if pattern_len == 0 {
        return Some(text.len());
    }
    if pattern_len > ASCII_BITPARALLEL_MAX_PATTERN_LEN {
        return None;
    }

    let mut eq_masks = [0u128; 256];
    for (ix, &byte) in pattern.iter().enumerate() {
        eq_masks[byte as usize] |= 1u128 << ix;
    }

    let mask = if pattern_len == ASCII_BITPARALLEL_MAX_PATTERN_LEN {
        u128::MAX
    } else {
        (1u128 << pattern_len) - 1
    };
    let high_bit = 1u128 << (pattern_len - 1);
    let mut positive = mask;
    let mut negative = 0u128;
    let mut distance = pattern_len;

    for &byte in text {
        let eq = eq_masks[byte as usize];
        let xv = eq | negative;
        let xh = (((eq & positive).wrapping_add(positive)) ^ positive) | eq;
        let mut positive_h = negative | !(xh | positive);
        let mut negative_h = positive & xh;

        if (positive_h & high_bit) != 0 {
            distance += 1;
        } else if (negative_h & high_bit) != 0 {
            distance -= 1;
        }

        positive_h = ((positive_h << 1) | 1) & mask;
        negative_h = (negative_h << 1) & mask;
        positive = (negative_h | !(xv | positive_h)) & mask;
        negative = positive_h & xv;
    }

    Some(distance)
}

fn prepare_replacement_lines<'a>(lines: &[&'a str]) -> Vec<PreparedReplacementLine<'a>> {
    lines
        .iter()
        .map(|line| PreparedReplacementLine::new(line))
        .collect()
}

struct ReplacementTextCacheIds {
    ids: Vec<usize>,
    unique_texts: usize,
    has_duplicates: bool,
}

fn prepare_replacement_text_cache_ids(
    lines: &[PreparedReplacementLine<'_>],
) -> ReplacementTextCacheIds {
    use rustc_hash::FxHashMap;

    let mut text_ids = FxHashMap::default();
    text_ids.reserve(lines.len());
    let mut ids = Vec::with_capacity(lines.len());

    for line in lines {
        let next_id = text_ids.len();
        let id = *text_ids.entry(line.text).or_insert(next_id);
        ids.push(id);
    }

    let unique_texts = text_ids.len();
    ReplacementTextCacheIds {
        ids,
        unique_texts,
        has_duplicates: unique_texts < lines.len(),
    }
}

#[cfg(test)]
fn replacement_alignment_ops(
    deletes: &[PreparedReplacementLine<'_>],
    inserts: &[PreparedReplacementLine<'_>],
) -> Vec<PlannedReplacementOp> {
    replacement_alignment_ops_with_pair_cost(deletes, inserts, replacement_pair_cost)
}

fn replacement_alignment_ops_with_pair_cost<'a, F>(
    deletes: &[PreparedReplacementLine<'a>],
    inserts: &[PreparedReplacementLine<'a>],
    pair_cost_fn: F,
) -> Vec<PlannedReplacementOp>
where
    F: Copy
        + Fn(
            &PreparedReplacementLine<'a>,
            &PreparedReplacementLine<'a>,
            &mut LevenshteinScratch,
        ) -> u32,
{
    let n = deletes.len();
    let m = inserts.len();
    let width = m + 1;
    let mut prev_costs = vec![0; width];
    let mut curr_costs = vec![0; width];
    let mut step = vec![ReplacementAlignStep::None; (n + 1) * width];
    #[allow(clippy::default_constructed_unit_structs)]
    let mut scratch = LevenshteinScratch::default();
    let delete_text_ids = prepare_replacement_text_cache_ids(deletes);
    let insert_text_ids = prepare_replacement_text_cache_ids(inserts);
    // Cache pair costs only when either side actually repeats line text within
    // this replacement block; otherwise every pair is unique and the cache
    // adds extra hashing/allocation work without any reuse.
    let mut pair_cost_cache = (delete_text_ids.has_duplicates || insert_text_ids.has_duplicates)
        .then(|| vec![u32::MAX; delete_text_ids.unique_texts * insert_text_ids.unique_texts]);
    for j in 1..=m {
        prev_costs[j] = (j as u32) * REPLACEMENT_GAP_COST;
        step[j] = ReplacementAlignStep::Insert;
    }

    for i in 1..=n {
        curr_costs[0] = (i as u32) * REPLACEMENT_GAP_COST;
        step[i * width] = ReplacementAlignStep::Delete;
        for j in 1..=m {
            let idx = i * width + j;
            let pair_cost_value = if let Some(pair_cost_cache) = pair_cost_cache.as_mut() {
                let pair_cache_idx = delete_text_ids.ids[i - 1] * insert_text_ids.unique_texts
                    + insert_text_ids.ids[j - 1];
                let cached_pair_cost = &mut pair_cost_cache[pair_cache_idx];
                if *cached_pair_cost == u32::MAX {
                    let computed = pair_cost_fn(&deletes[i - 1], &inserts[j - 1], &mut scratch);
                    *cached_pair_cost = computed;
                    computed
                } else {
                    *cached_pair_cost
                }
            } else {
                pair_cost_fn(&deletes[i - 1], &inserts[j - 1], &mut scratch)
            };
            let pair_cost = prev_costs[j - 1].saturating_add(pair_cost_value);
            let insert_cost = curr_costs[j - 1].saturating_add(REPLACEMENT_GAP_COST);
            let delete_cost = prev_costs[j].saturating_add(REPLACEMENT_GAP_COST);

            let mut best_cost = pair_cost;
            let mut best_step = ReplacementAlignStep::Pair;

            if insert_cost < best_cost {
                best_cost = insert_cost;
                best_step = ReplacementAlignStep::Insert;
            }
            if delete_cost < best_cost {
                best_cost = delete_cost;
                best_step = ReplacementAlignStep::Delete;
            }

            curr_costs[j] = best_cost;
            step[idx] = best_step;
        }
        std::mem::swap(&mut prev_costs, &mut curr_costs);
    }

    let mut i = n;
    let mut j = m;
    let mut aligned_rev = Vec::with_capacity(n + m);
    while i > 0 || j > 0 {
        let idx = i * width + j;
        match step[idx] {
            ReplacementAlignStep::Pair if i > 0 && j > 0 => {
                aligned_rev.push(PlannedReplacementOp::Pair);
                i -= 1;
                j -= 1;
            }
            ReplacementAlignStep::Insert if j > 0 => {
                aligned_rev.push(PlannedReplacementOp::Insert);
                j -= 1;
            }
            ReplacementAlignStep::Delete if i > 0 => {
                aligned_rev.push(PlannedReplacementOp::Delete);
                i -= 1;
            }
            _ if j > 0 => {
                aligned_rev.push(PlannedReplacementOp::Insert);
                j -= 1;
            }
            _ if i > 0 => {
                aligned_rev.push(PlannedReplacementOp::Delete);
                i -= 1;
            }
            _ => break,
        }
    }

    aligned_rev.reverse();
    aligned_rev
}

fn select_side_by_side_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    let combined = old.len().saturating_add(new.len());
    if combined >= SIDE_BY_SIDE_LINEAR_FALLBACK_LINE_THRESHOLD {
        return myers_fallback_edits(old, new);
    }
    if combined >= SIDE_BY_SIDE_HISTOGRAM_LINE_THRESHOLD {
        return histogram_edits(old, new);
    }
    myers_edits(old, new)
}

fn push_plan_run(runs: &mut Vec<FileDiffPlanRun>, run: FileDiffPlanRun) {
    let len = run.row_len();
    if len == 0 {
        return;
    }

    let merged = match (runs.last_mut(), &run) {
        (
            Some(FileDiffPlanRun::Context {
                old_start: last_old_start,
                new_start: last_new_start,
                len: last_len,
            }),
            FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            },
        ) if last_old_start.saturating_add(*last_len) == *old_start
            && last_new_start.saturating_add(*last_len) == *new_start =>
        {
            *last_len = last_len.saturating_add(*len);
            true
        }
        (
            Some(FileDiffPlanRun::Remove {
                old_start: last_old_start,
                len: last_len,
            }),
            FileDiffPlanRun::Remove { old_start, len },
        ) if last_old_start.saturating_add(*last_len) == *old_start => {
            *last_len = last_len.saturating_add(*len);
            true
        }
        (
            Some(FileDiffPlanRun::Add {
                new_start: last_new_start,
                len: last_len,
            }),
            FileDiffPlanRun::Add { new_start, len },
        ) if last_new_start.saturating_add(*last_len) == *new_start => {
            *last_len = last_len.saturating_add(*len);
            true
        }
        (
            Some(FileDiffPlanRun::Modify {
                old_start: last_old_start,
                new_start: last_new_start,
                len: last_len,
            }),
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            },
        ) if last_old_start.saturating_add(*last_len) == *old_start
            && last_new_start.saturating_add(*last_len) == *new_start =>
        {
            *last_len = last_len.saturating_add(*len);
            true
        }
        _ => false,
    };

    if !merged {
        runs.push(run);
    }
}

fn push_plan_run_with_counts(
    runs: &mut Vec<FileDiffPlanRun>,
    row_count: &mut usize,
    inline_row_count: &mut usize,
    run: FileDiffPlanRun,
) {
    let len = run.row_len();
    if len == 0 {
        return;
    }
    *row_count = row_count.saturating_add(len);
    *inline_row_count = inline_row_count.saturating_add(run.inline_row_len());
    push_plan_run(runs, run);
}

fn apply_eof_newline_to_plan(
    runs: &mut Vec<FileDiffPlanRun>,
    eof_newline: Option<FileDiffEofNewline>,
) -> bool {
    if eof_newline.is_none() {
        return false;
    }

    let Some(last_run) = runs.pop() else {
        return false;
    };
    match last_run {
        FileDiffPlanRun::Context {
            old_start,
            new_start,
            len,
        } if len > 1 => {
            runs.push(FileDiffPlanRun::Context {
                old_start,
                new_start,
                len: len.saturating_sub(1),
            });
            runs.push(FileDiffPlanRun::Modify {
                old_start: old_start.saturating_add(len.saturating_sub(1)),
                new_start: new_start.saturating_add(len.saturating_sub(1)),
                len: 1,
            });
            true
        }
        FileDiffPlanRun::Context {
            old_start,
            new_start,
            ..
        } => {
            runs.push(FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len: 1,
            });
            true
        }
        other => {
            runs.push(other);
            false
        }
    }
}

fn build_sparse_positional_side_by_side_plan(
    old_text: &str,
    new_text: &str,
    old_lines: &[&str],
    new_lines: &[&str],
) -> Option<FileDiffPlan> {
    if old_lines.len() != new_lines.len() {
        return None;
    }
    let line_count = old_lines.len();
    if line_count == 0
        || line_count.saturating_add(line_count) < SIDE_BY_SIDE_HISTOGRAM_LINE_THRESHOLD
    {
        return None;
    }

    let mut runs = Vec::with_capacity(line_count / 8 + 1);
    let mut row_count = 0usize;
    let mut inline_row_count = 0usize;
    let mut changed_lines = 0usize;
    let mut ix = 0usize;

    while ix < line_count {
        if old_lines[ix] == new_lines[ix] {
            let start = ix;
            ix += 1;
            while ix < line_count && old_lines[ix] == new_lines[ix] {
                ix += 1;
            }
            push_plan_run_with_counts(
                &mut runs,
                &mut row_count,
                &mut inline_row_count,
                FileDiffPlanRun::Context {
                    old_start: start,
                    new_start: start,
                    len: ix.saturating_sub(start),
                },
            );
            continue;
        }

        let block_start = ix;
        ix += 1;
        while ix < line_count && old_lines[ix] != new_lines[ix] {
            ix += 1;
        }
        let block_len = ix.saturating_sub(block_start);
        changed_lines = changed_lines.saturating_add(block_len);
        if block_len > SIDE_BY_SIDE_SPARSE_POSITIONAL_MAX_BLOCK_LEN
            || changed_lines
                .saturating_mul(SIDE_BY_SIDE_SPARSE_POSITIONAL_MAX_CHANGED_RATIO_DENOMINATOR)
                > line_count
        {
            return None;
        }

        push_plan_run_with_counts(
            &mut runs,
            &mut row_count,
            &mut inline_row_count,
            FileDiffPlanRun::Modify {
                old_start: block_start,
                new_start: block_start,
                len: block_len,
            },
        );
    }

    let eof_newline = eof_newline_delta(old_text, new_text);
    if apply_eof_newline_to_plan(&mut runs, eof_newline) {
        inline_row_count = inline_row_count.saturating_add(1);
    }

    Some(FileDiffPlan {
        runs,
        row_count,
        inline_row_count,
        eof_newline,
    })
}

fn push_paired_replacement_runs_by_position_to_plan(
    old_start: usize,
    old_len: usize,
    new_start: usize,
    new_len: usize,
    runs: &mut Vec<FileDiffPlanRun>,
    row_count: &mut usize,
    inline_row_count: &mut usize,
) {
    let paired = old_len.min(new_len);
    if paired > 0 {
        push_plan_run_with_counts(
            runs,
            row_count,
            inline_row_count,
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len: paired,
            },
        );
    }
    if old_len > paired {
        push_plan_run_with_counts(
            runs,
            row_count,
            inline_row_count,
            FileDiffPlanRun::Remove {
                old_start: old_start.saturating_add(paired),
                len: old_len.saturating_sub(paired),
            },
        );
    }
    if new_len > paired {
        push_plan_run_with_counts(
            runs,
            row_count,
            inline_row_count,
            FileDiffPlanRun::Add {
                new_start: new_start.saturating_add(paired),
                len: new_len.saturating_sub(paired),
            },
        );
    }
}

fn push_aligned_replacement_runs_to_plan_with_pair_cost<F>(
    old_lines: &[&str],
    new_lines: &[&str],
    old_range: std::ops::Range<usize>,
    new_range: std::ops::Range<usize>,
    runs: &mut Vec<FileDiffPlanRun>,
    row_count: &mut usize,
    inline_row_count: &mut usize,
    pair_cost_fn: F,
) where
    F: Copy
        + for<'a> Fn(
            &PreparedReplacementLine<'a>,
            &PreparedReplacementLine<'a>,
            &mut LevenshteinScratch,
        ) -> u32,
{
    let old_start = old_range.start;
    let new_start = new_range.start;
    let deletes = &old_lines[old_range.start..old_range.end];
    let inserts = &new_lines[new_range.start..new_range.end];

    if deletes.is_empty() {
        push_plan_run_with_counts(
            runs,
            row_count,
            inline_row_count,
            FileDiffPlanRun::Add {
                new_start,
                len: inserts.len(),
            },
        );
        return;
    }
    if inserts.is_empty() {
        push_plan_run_with_counts(
            runs,
            row_count,
            inline_row_count,
            FileDiffPlanRun::Remove {
                old_start,
                len: deletes.len(),
            },
        );
        return;
    }

    if deletes.len().saturating_mul(inserts.len()) > REPLACEMENT_ALIGN_CELL_BUDGET {
        push_paired_replacement_runs_by_position_to_plan(
            old_start,
            deletes.len(),
            new_start,
            inserts.len(),
            runs,
            row_count,
            inline_row_count,
        );
        return;
    }

    let deletes = prepare_replacement_lines(deletes);
    let inserts = prepare_replacement_lines(inserts);

    let mut local_old = 0usize;
    let mut local_new = 0usize;
    for op in replacement_alignment_ops_with_pair_cost(&deletes, &inserts, pair_cost_fn) {
        match op {
            PlannedReplacementOp::Pair => {
                push_plan_run_with_counts(
                    runs,
                    row_count,
                    inline_row_count,
                    FileDiffPlanRun::Modify {
                        old_start: old_start.saturating_add(local_old),
                        new_start: new_start.saturating_add(local_new),
                        len: 1,
                    },
                );
                local_old += 1;
                local_new += 1;
            }
            PlannedReplacementOp::Delete => {
                push_plan_run_with_counts(
                    runs,
                    row_count,
                    inline_row_count,
                    FileDiffPlanRun::Remove {
                        old_start: old_start.saturating_add(local_old),
                        len: 1,
                    },
                );
                local_old += 1;
            }
            PlannedReplacementOp::Insert => {
                push_plan_run_with_counts(
                    runs,
                    row_count,
                    inline_row_count,
                    FileDiffPlanRun::Add {
                        new_start: new_start.saturating_add(local_new),
                        len: 1,
                    },
                );
                local_new += 1;
            }
        }
    }
}

fn build_linear_fallback_side_by_side_plan_with_pair_cost<F>(
    old_text: &str,
    new_text: &str,
    old_lines: &[&str],
    new_lines: &[&str],
    pair_cost_fn: F,
) -> FileDiffPlan
where
    F: Copy
        + for<'a> Fn(
            &PreparedReplacementLine<'a>,
            &PreparedReplacementLine<'a>,
            &mut LevenshteinScratch,
        ) -> u32,
{
    let mut prefix = 0usize;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while prefix + suffix < old_lines.len()
        && prefix + suffix < new_lines.len()
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid_end = old_lines.len().saturating_sub(suffix);
    let new_mid_end = new_lines.len().saturating_sub(suffix);
    let mut runs = Vec::with_capacity(3);
    let mut row_count = 0usize;
    let mut inline_row_count = 0usize;

    push_plan_run_with_counts(
        &mut runs,
        &mut row_count,
        &mut inline_row_count,
        FileDiffPlanRun::Context {
            old_start: 0,
            new_start: 0,
            len: prefix,
        },
    );

    push_aligned_replacement_runs_to_plan_with_pair_cost(
        old_lines,
        new_lines,
        prefix..old_mid_end,
        prefix..new_mid_end,
        &mut runs,
        &mut row_count,
        &mut inline_row_count,
        pair_cost_fn,
    );

    push_plan_run_with_counts(
        &mut runs,
        &mut row_count,
        &mut inline_row_count,
        FileDiffPlanRun::Context {
            old_start: old_mid_end,
            new_start: new_mid_end,
            len: suffix,
        },
    );

    let eof_newline = eof_newline_delta(old_text, new_text);
    if apply_eof_newline_to_plan(&mut runs, eof_newline) {
        inline_row_count = inline_row_count.saturating_add(1);
    }
    FileDiffPlan {
        runs,
        row_count,
        inline_row_count,
        eof_newline,
    }
}

fn build_side_by_side_plan_with_pair_cost<F>(
    old_text: &str,
    new_text: &str,
    old_lines: &[&str],
    new_lines: &[&str],
    pair_cost_fn: F,
) -> FileDiffPlan
where
    F: Copy
        + for<'a> Fn(
            &PreparedReplacementLine<'a>,
            &PreparedReplacementLine<'a>,
            &mut LevenshteinScratch,
        ) -> u32,
{
    if old_lines.len().saturating_add(new_lines.len())
        >= SIDE_BY_SIDE_LINEAR_FALLBACK_LINE_THRESHOLD
    {
        return build_linear_fallback_side_by_side_plan_with_pair_cost(
            old_text,
            new_text,
            old_lines,
            new_lines,
            pair_cost_fn,
        );
    }
    if let Some(plan) =
        build_sparse_positional_side_by_side_plan(old_text, new_text, old_lines, new_lines)
    {
        return plan;
    }

    let edits = select_side_by_side_edits(old_lines, new_lines);
    let mut runs = Vec::with_capacity(edits.len());
    let mut row_count = 0usize;
    let mut inline_row_count = 0usize;
    let mut old_ix = 0usize;
    let mut new_ix = 0usize;
    let mut i = 0usize;

    while i < edits.len() {
        match edits[i].kind {
            EditKind::Equal => {
                let run_old_start = old_ix;
                let run_new_start = new_ix;
                while i < edits.len() && edits[i].kind == EditKind::Equal {
                    old_ix += 1;
                    new_ix += 1;
                    i += 1;
                }
                push_plan_run_with_counts(
                    &mut runs,
                    &mut row_count,
                    &mut inline_row_count,
                    FileDiffPlanRun::Context {
                        old_start: run_old_start,
                        new_start: run_new_start,
                        len: old_ix.saturating_sub(run_old_start),
                    },
                );
            }
            EditKind::Delete => {
                let delete_start = old_ix;
                while i < edits.len() && edits[i].kind == EditKind::Delete {
                    old_ix += 1;
                    i += 1;
                }

                let insert_start = new_ix;
                while i < edits.len() && edits[i].kind == EditKind::Insert {
                    new_ix += 1;
                    i += 1;
                }

                if insert_start == new_ix {
                    push_plan_run_with_counts(
                        &mut runs,
                        &mut row_count,
                        &mut inline_row_count,
                        FileDiffPlanRun::Remove {
                            old_start: delete_start,
                            len: old_ix.saturating_sub(delete_start),
                        },
                    );
                } else {
                    push_aligned_replacement_runs_to_plan_with_pair_cost(
                        old_lines,
                        new_lines,
                        delete_start..old_ix,
                        insert_start..new_ix,
                        &mut runs,
                        &mut row_count,
                        &mut inline_row_count,
                        pair_cost_fn,
                    );
                }
            }
            EditKind::Insert => {
                let insert_start = new_ix;
                while i < edits.len() && edits[i].kind == EditKind::Insert {
                    new_ix += 1;
                    i += 1;
                }
                push_plan_run_with_counts(
                    &mut runs,
                    &mut row_count,
                    &mut inline_row_count,
                    FileDiffPlanRun::Add {
                        new_start: insert_start,
                        len: new_ix.saturating_sub(insert_start),
                    },
                );
            }
        }
    }

    let eof_newline = eof_newline_delta(old_text, new_text);
    if apply_eof_newline_to_plan(&mut runs, eof_newline) {
        inline_row_count = inline_row_count.saturating_add(1);
    }
    FileDiffPlan {
        runs,
        row_count,
        inline_row_count,
        eof_newline,
    }
}

fn one_based_line_number(line_ix: usize) -> Option<u32> {
    line_ix
        .checked_add(1)
        .and_then(|line| u32::try_from(line).ok())
}

fn mark_changed_line(mask: &mut [bool], line: Option<u32>) {
    let Some(line) = line else {
        return;
    };
    let line_ix = line.saturating_sub(1) as usize;
    if let Some(slot) = mask.get_mut(line_ix) {
        *slot = true;
    }
}

fn assign_line_to_row(line_to_row: &mut [Option<usize>], line: Option<u32>, row_index: usize) {
    let Some(line) = line else {
        return;
    };
    let line_ix = line.saturating_sub(1) as usize;
    if let Some(slot) = line_to_row.get_mut(line_ix) {
        *slot = Some(row_index);
    }
}

fn for_each_plan_row_meta(plan: &FileDiffPlan, mut f: impl FnMut(usize, DiffRowMeta)) {
    let mut row_index = 0usize;

    for run in &plan.runs {
        match *run {
            FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    f(
                        row_index,
                        DiffRowMeta {
                            kind: FileDiffRowKind::Context,
                            old_line: one_based_line_number(old_ix),
                            new_line: one_based_line_number(new_ix),
                        },
                    );
                    row_index = row_index.saturating_add(1);
                }
            }
            FileDiffPlanRun::Remove { old_start, len } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    f(
                        row_index,
                        DiffRowMeta {
                            kind: FileDiffRowKind::Remove,
                            old_line: one_based_line_number(old_ix),
                            new_line: None,
                        },
                    );
                    row_index = row_index.saturating_add(1);
                }
            }
            FileDiffPlanRun::Add { new_start, len } => {
                for offset in 0..len {
                    let new_ix = new_start.saturating_add(offset);
                    f(
                        row_index,
                        DiffRowMeta {
                            kind: FileDiffRowKind::Add,
                            old_line: None,
                            new_line: one_based_line_number(new_ix),
                        },
                    );
                    row_index = row_index.saturating_add(1);
                }
            }
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    f(
                        row_index,
                        DiffRowMeta {
                            kind: FileDiffRowKind::Modify,
                            old_line: one_based_line_number(old_ix),
                            new_line: one_based_line_number(new_ix),
                        },
                    );
                    row_index = row_index.saturating_add(1);
                }
            }
        }
    }

    debug_assert_eq!(row_index, plan.row_count);
}

fn materialize_rows_from_plan(
    plan: &FileDiffPlan,
    old_text: &Arc<str>,
    old_lines: &[&str],
    new_text: &Arc<str>,
    new_lines: &[&str],
) -> Vec<FileDiffRow> {
    let mut rows = Vec::with_capacity(plan.row_count);
    materialize_rows_from_plan_into(
        &mut rows, plan, old_text, old_lines, new_text, new_lines, 0, 0,
    );
    rows
}

fn shared_line_text(text: &Arc<str>, line: &str) -> FileDiffLineText {
    let base_ptr = text.as_ptr() as usize;
    let line_ptr = line.as_ptr() as usize;
    let start = line_ptr.saturating_sub(base_ptr);
    let end = start.saturating_add(line.len());
    debug_assert_eq!(text.get(start..end), Some(line));
    FileDiffLineText::shared_slice(Arc::clone(text), start..end)
}

fn materialize_rows_from_plan_into(
    rows: &mut Vec<FileDiffRow>,
    plan: &FileDiffPlan,
    old_text: &Arc<str>,
    old_lines: &[&str],
    new_text: &Arc<str>,
    new_lines: &[&str],
    old_line_offset: u32,
    new_line_offset: u32,
) {
    let old_line_delta = old_line_offset.saturating_sub(1);
    let new_line_delta = new_line_offset.saturating_sub(1);
    rows.reserve(plan.row_count);
    let row_start = rows.len();

    for run in &plan.runs {
        match run {
            FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..*len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    let text = shared_line_text(
                        old_text,
                        old_lines.get(old_ix).copied().unwrap_or_default(),
                    );
                    rows.push(FileDiffRow {
                        kind: FileDiffRowKind::Context,
                        old_line: one_based_line_number(old_ix)
                            .map(|line| line.saturating_add(old_line_delta)),
                        new_line: one_based_line_number(new_ix)
                            .map(|line| line.saturating_add(new_line_delta)),
                        old: Some(text.clone()),
                        new: Some(text),
                        eof_newline: None,
                    });
                }
            }
            FileDiffPlanRun::Remove { old_start, len } => {
                for offset in 0..*len {
                    let old_ix = old_start.saturating_add(offset);
                    rows.push(FileDiffRow {
                        kind: FileDiffRowKind::Remove,
                        old_line: one_based_line_number(old_ix)
                            .map(|line| line.saturating_add(old_line_delta)),
                        new_line: None,
                        old: Some(shared_line_text(
                            old_text,
                            old_lines.get(old_ix).copied().unwrap_or_default(),
                        )),
                        new: None,
                        eof_newline: None,
                    });
                }
            }
            FileDiffPlanRun::Add { new_start, len } => {
                for offset in 0..*len {
                    let new_ix = new_start.saturating_add(offset);
                    rows.push(FileDiffRow {
                        kind: FileDiffRowKind::Add,
                        old_line: None,
                        new_line: one_based_line_number(new_ix)
                            .map(|line| line.saturating_add(new_line_delta)),
                        old: None,
                        new: Some(shared_line_text(
                            new_text,
                            new_lines.get(new_ix).copied().unwrap_or_default(),
                        )),
                        eof_newline: None,
                    });
                }
            }
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                for offset in 0..*len {
                    let old_ix = old_start.saturating_add(offset);
                    let new_ix = new_start.saturating_add(offset);
                    rows.push(FileDiffRow {
                        kind: FileDiffRowKind::Modify,
                        old_line: one_based_line_number(old_ix)
                            .map(|line| line.saturating_add(old_line_delta)),
                        new_line: one_based_line_number(new_ix)
                            .map(|line| line.saturating_add(new_line_delta)),
                        old: Some(shared_line_text(
                            old_text,
                            old_lines.get(old_ix).copied().unwrap_or_default(),
                        )),
                        new: Some(shared_line_text(
                            new_text,
                            new_lines.get(new_ix).copied().unwrap_or_default(),
                        )),
                        eof_newline: None,
                    });
                }
            }
        }
    }

    if let Some(marker) = plan.eof_newline {
        if let Some(last) = rows[row_start..].last_mut() {
            last.eof_newline = Some(marker);
        } else {
            rows.push(FileDiffRow {
                kind: FileDiffRowKind::Modify,
                old_line: None,
                new_line: None,
                old: None,
                new: None,
                eof_newline: Some(marker),
            });
        }
    }
}

#[cfg(test)]
fn pair_replacements(rows: Vec<FileDiffRow>) -> Vec<FileDiffRow> {
    let mut out = Vec::with_capacity(rows.len());
    let mut ix = 0usize;

    while ix < rows.len() {
        if rows[ix].kind != FileDiffRowKind::Remove {
            out.push(rows[ix].clone());
            ix += 1;
            continue;
        }

        let del_start = ix;
        while ix < rows.len() && rows[ix].kind == FileDiffRowKind::Remove {
            ix += 1;
        }
        let del_end = ix;

        let ins_start = ix;
        while ix < rows.len() && rows[ix].kind == FileDiffRowKind::Add {
            ix += 1;
        }
        let ins_end = ix;

        if ins_start == ins_end {
            out.extend(rows[del_start..del_end].iter().cloned());
            continue;
        }

        out.extend(align_replacement_runs(
            &rows[del_start..del_end],
            &rows[ins_start..ins_end],
        ));
    }

    out
}

#[cfg(test)]
fn align_replacement_runs(deletes: &[FileDiffRow], inserts: &[FileDiffRow]) -> Vec<FileDiffRow> {
    if deletes.is_empty() {
        return inserts.to_vec();
    }
    if inserts.is_empty() {
        return deletes.to_vec();
    }

    if deletes.len().saturating_mul(inserts.len()) > REPLACEMENT_ALIGN_CELL_BUDGET {
        return pair_replacement_runs_by_position(deletes, inserts);
    }

    let delete_meta: Vec<_> = deletes
        .iter()
        .map(|row| PreparedReplacementLine::new(row.old.as_deref().unwrap_or_default()))
        .collect();
    let insert_meta: Vec<_> = inserts
        .iter()
        .map(|row| PreparedReplacementLine::new(row.new.as_deref().unwrap_or_default()))
        .collect();

    let mut out = Vec::with_capacity(deletes.len() + inserts.len());
    let mut delete_ix = 0usize;
    let mut insert_ix = 0usize;
    for op in replacement_alignment_ops(&delete_meta, &insert_meta) {
        match op {
            PlannedReplacementOp::Pair => {
                out.push(make_modify_row(&deletes[delete_ix], &inserts[insert_ix]));
                delete_ix += 1;
                insert_ix += 1;
            }
            PlannedReplacementOp::Insert => {
                out.push(inserts[insert_ix].clone());
                insert_ix += 1;
            }
            PlannedReplacementOp::Delete => {
                out.push(deletes[delete_ix].clone());
                delete_ix += 1;
            }
        }
    }

    out
}

#[cfg(test)]
fn pair_replacement_runs_by_position(
    deletes: &[FileDiffRow],
    inserts: &[FileDiffRow],
) -> Vec<FileDiffRow> {
    let paired = deletes.len().min(inserts.len());
    let mut out = Vec::with_capacity(deletes.len() + inserts.len());

    for i in 0..paired {
        out.push(make_modify_row(&deletes[i], &inserts[i]));
    }
    if deletes.len() > paired {
        out.extend(deletes[paired..].iter().cloned());
    }
    if inserts.len() > paired {
        out.extend(inserts[paired..].iter().cloned());
    }
    out
}

#[cfg(test)]
fn make_modify_row(delete: &FileDiffRow, insert: &FileDiffRow) -> FileDiffRow {
    FileDiffRow {
        kind: FileDiffRowKind::Modify,
        old_line: delete.old_line,
        new_line: insert.new_line,
        old: delete.old.clone(),
        new: insert.new.clone(),
        eof_newline: None,
    }
}

fn replacement_pair_cost(
    old: &PreparedReplacementLine<'_>,
    new: &PreparedReplacementLine<'_>,
    scratch: &mut LevenshteinScratch,
) -> u32 {
    if old.text == new.text {
        return 0;
    }

    if let (Some(old_bytes), Some(new_bytes)) = (old.ascii_bytes(), new.ascii_bytes()) {
        let (shared_prefix, shared_suffix) = shared_boundary_bytes(old_bytes, new_bytes);
        return replacement_pair_cost_with_shared_boundary(
            old_bytes,
            new_bytes,
            shared_prefix,
            shared_suffix,
            |old_trimmed, new_trimmed| scratch.distance_bytes(old_trimmed, new_trimmed) as u32,
        );
    }

    replacement_pair_cost_with_distance(old.chars(), new.chars(), |old_trimmed, new_trimmed| {
        scratch.distance(old_trimmed, new_trimmed) as u32
    })
}

#[cfg(feature = "benchmarks")]
fn replacement_pair_cost_with_scratch(
    old: &PreparedReplacementLine<'_>,
    new: &PreparedReplacementLine<'_>,
    scratch: &mut LevenshteinScratch,
) -> u32 {
    replacement_pair_cost(old, new, scratch)
}

#[cfg(feature = "benchmarks")]
fn replacement_pair_cost_with_strsim(
    old: &PreparedReplacementLine<'_>,
    new: &PreparedReplacementLine<'_>,
    scratch: &mut LevenshteinScratch,
) -> u32 {
    if old.text == new.text {
        return 0;
    }

    if let (Some(old_bytes), Some(new_bytes)) = (old.ascii_bytes(), new.ascii_bytes()) {
        let (shared_prefix, shared_suffix) = shared_boundary_bytes(old_bytes, new_bytes);
        return replacement_pair_cost_with_shared_boundary(
            old_bytes,
            new_bytes,
            shared_prefix,
            shared_suffix,
            |old_trimmed, new_trimmed| {
                u32::try_from(scratch.distance_bytes(old_trimmed, new_trimmed)).unwrap_or(u32::MAX)
            },
        );
    }

    replacement_pair_cost_with_distance(old.chars(), new.chars(), |old_trimmed, new_trimmed| {
        let old_trimmed_wrapper = CharSlice(old_trimmed);
        let new_trimmed_wrapper = CharSlice(new_trimmed);
        u32::try_from(strsim::generic_levenshtein(
            &old_trimmed_wrapper,
            &new_trimmed_wrapper,
        ))
        .unwrap_or(u32::MAX)
    })
}

fn replacement_pair_cost_with_distance<T: Eq>(
    old_units: &[T],
    new_units: &[T],
    distance_fn: impl FnOnce(&[T], &[T]) -> u32,
) -> u32 {
    let (shared_prefix, shared_suffix) = shared_boundary_units(old_units, new_units);
    replacement_pair_cost_with_shared_boundary(
        old_units,
        new_units,
        shared_prefix,
        shared_suffix,
        distance_fn,
    )
}

fn replacement_pair_cost_with_shared_boundary<T: Eq>(
    old_units: &[T],
    new_units: &[T],
    shared_prefix: usize,
    shared_suffix: usize,
    distance_fn: impl FnOnce(&[T], &[T]) -> u32,
) -> u32 {
    let max_len_usize = old_units.len().max(new_units.len()).max(1);
    let max_len = max_len_usize as u32;
    let old_trimmed = &old_units[shared_prefix..old_units.len().saturating_sub(shared_suffix)];
    let new_trimmed = &new_units[shared_prefix..new_units.len().saturating_sub(shared_suffix)];
    let trimmed_cells = old_trimmed.len().saturating_mul(new_trimmed.len());

    // Fast path: if either trimmed side is empty, the distance is exactly
    // the length of the other side — skip the O(n*m) Levenshtein DP.
    let distance = if old_trimmed.is_empty() || new_trimmed.is_empty() {
        (old_trimmed.len() + new_trimmed.len()) as u32
    } else if trimmed_cells > REPLACEMENT_ALIGN_CELL_BUDGET {
        u32::try_from(old_trimmed.len().max(new_trimmed.len())).unwrap_or(u32::MAX)
    } else {
        distance_fn(old_trimmed, new_trimmed)
    };

    let mut cost = REPLACEMENT_PAIR_BASE_COST
        + distance
            .min(max_len)
            .saturating_mul(REPLACEMENT_PAIR_SCALE_COST)
            / max_len;
    if shared_prefix == 0
        && shared_suffix == 0
        && max_len_usize >= REPLACEMENT_DISSIMILAR_PENALTY_MIN_LEN
    {
        cost = cost.saturating_add(REPLACEMENT_DISSIMILAR_PENALTY_COST);
    }

    cost
}

fn shared_boundary_bytes(a: &[u8], b: &[u8]) -> (usize, usize) {
    let min_len = a.len().min(b.len());
    let word_bytes = std::mem::size_of::<usize>();
    let mut prefix = 0usize;
    while prefix.saturating_add(word_bytes) <= min_len
        && a[prefix..prefix + word_bytes] == b[prefix..prefix + word_bytes]
    {
        prefix += word_bytes;
    }
    while prefix < min_len && a[prefix] == b[prefix] {
        prefix += 1;
    }

    let max_suffix = min_len.saturating_sub(prefix);
    let mut suffix = 0usize;
    while suffix < max_suffix && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix] {
        suffix += 1;
    }

    (prefix, suffix)
}

fn shared_boundary_units<T: Eq>(a: &[T], b: &[T]) -> (usize, usize) {
    let mut prefix = 0usize;
    while prefix < a.len() && prefix < b.len() && a[prefix] == b[prefix] {
        prefix += 1;
    }

    let max_suffix = a.len().min(b.len()).saturating_sub(prefix);
    let mut suffix = 0usize;
    while suffix < max_suffix && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix] {
        suffix += 1;
    }

    (prefix, suffix)
}

#[cfg(test)]
fn shared_boundary_chars(a: &[char], b: &[char]) -> (usize, usize) {
    shared_boundary_units(a, b)
}

/// Patience/histogram diff algorithm.
///
/// Uses unique lines as anchors via the longest increasing subsequence,
/// then recursively diffs the regions between anchors. Falls back to
/// Myers for regions with no unique lines. This produces cleaner diffs
/// for code with repetitive structural tokens (braces, returns, etc.)
/// by preferring semantically unique lines (function signatures) as
/// alignment points.
/// Maximum recursion depth for histogram/patience diff before falling back to Myers.
const PATIENCE_MAX_DEPTH: usize = 32;

pub(crate) fn histogram_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    patience_recurse(old, new, 0, old.len(), 0, new.len(), 0)
}

fn patience_recurse<'a>(
    old: &[&'a str],
    new: &[&'a str],
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
    depth: usize,
) -> Vec<Edit<'a>> {
    // Fall back to Myers if recursion is too deep.
    if depth >= PATIENCE_MAX_DEPTH {
        return myers_edits(&old[old_start..old_end], &new[new_start..new_end]);
    }

    // Strip common prefix.
    let mut prefix = 0;
    while old_start + prefix < old_end
        && new_start + prefix < new_end
        && old[old_start + prefix] == new[new_start + prefix]
    {
        prefix += 1;
    }

    // Strip common suffix.
    let mut suffix = 0;
    while old_start + prefix + suffix < old_end
        && new_start + prefix + suffix < new_end
        && old[old_end - 1 - suffix] == new[new_end - 1 - suffix]
    {
        suffix += 1;
    }

    let inner_old_start = old_start + prefix;
    let inner_old_end = old_end - suffix;
    let inner_new_start = new_start + prefix;
    let inner_new_end = new_end - suffix;

    let mut edits = Vec::new();

    // Emit prefix equals.
    for i in 0..prefix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[old_start + i]),
            new: Some(new[new_start + i]),
        });
    }

    if inner_old_start == inner_old_end && inner_new_start == inner_new_end {
        // Nothing between prefix and suffix.
    } else if inner_old_start == inner_old_end {
        // Pure insertions.
        for &item in &new[inner_new_start..inner_new_end] {
            edits.push(Edit {
                kind: EditKind::Insert,
                old: None,
                new: Some(item),
            });
        }
    } else if inner_new_start == inner_new_end {
        // Pure deletions.
        for &item in &old[inner_old_start..inner_old_end] {
            edits.push(Edit {
                kind: EditKind::Delete,
                old: Some(item),
                new: None,
            });
        }
    } else {
        // Find unique-line anchors via patience matching.
        let anchors = find_patience_anchors(
            old,
            new,
            inner_old_start,
            inner_old_end,
            inner_new_start,
            inner_new_end,
        );

        if anchors.is_empty() {
            let old_inner = &old[inner_old_start..inner_old_end];
            let new_inner = &new[inner_new_start..inner_new_end];
            // Large anchorless regions with matching line counts tend to be
            // structurally aligned already; preserve same-position context
            // lines linearly instead of paying for a full Myers trace.
            if should_use_patience_positional_fallback(old_inner, new_inner) {
                edits.extend(positional_fallback_edits(old_inner, new_inner));
            } else {
                // No unique anchors — fall back to Myers for this region.
                edits.extend(myers_edits(old_inner, new_inner));
            }
        } else {
            // Recursively diff between anchors.
            let mut oi = inner_old_start;
            let mut ni = inner_new_start;

            for &(old_idx, new_idx) in &anchors {
                if oi < old_idx || ni < new_idx {
                    edits.extend(patience_recurse(
                        old,
                        new,
                        oi,
                        old_idx,
                        ni,
                        new_idx,
                        depth + 1,
                    ));
                }
                edits.push(Edit {
                    kind: EditKind::Equal,
                    old: Some(old[old_idx]),
                    new: Some(new[new_idx]),
                });
                oi = old_idx + 1;
                ni = new_idx + 1;
            }

            // Region after the last anchor.
            if oi < inner_old_end || ni < inner_new_end {
                edits.extend(patience_recurse(
                    old,
                    new,
                    oi,
                    inner_old_end,
                    ni,
                    inner_new_end,
                    depth + 1,
                ));
            }
        }
    }

    // Emit suffix equals.
    for i in 0..suffix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[inner_old_end + i]),
            new: Some(new[inner_new_end + i]),
        });
    }

    edits
}

fn should_use_patience_positional_fallback(old: &[&str], new: &[&str]) -> bool {
    old.len() == new.len()
        && old.len().saturating_add(new.len()) >= PATIENCE_POSITIONAL_FALLBACK_LINE_THRESHOLD
}

/// Find lines that are unique in both old and new within the given ranges,
/// then compute the longest increasing subsequence of their positions to
/// produce patience anchors.
fn find_patience_anchors(
    old: &[&str],
    new: &[&str],
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
) -> Vec<(usize, usize)> {
    use rustc_hash::FxHashMap as HashMap;

    // Count occurrences and record position for old lines.
    let mut old_info: HashMap<&str, (usize, usize)> = HashMap::default();
    for (i, &line) in old.iter().enumerate().take(old_end).skip(old_start) {
        let entry = old_info.entry(line).or_insert((0, i));
        entry.0 += 1;
        entry.1 = i;
    }

    // Count occurrences and record position for new lines.
    let mut new_info: HashMap<&str, (usize, usize)> = HashMap::default();
    for (j, &line) in new.iter().enumerate().take(new_end).skip(new_start) {
        let entry = new_info.entry(line).or_insert((0, j));
        entry.0 += 1;
        entry.1 = j;
    }

    // Collect lines that appear exactly once in both old and new.
    let mut unique_pairs: Vec<(usize, usize)> = Vec::new();
    for (line, &(old_count, old_idx)) in &old_info {
        if old_count != 1 {
            continue;
        }
        if let Some(&(new_count, new_idx)) = new_info.get(line)
            && new_count == 1
        {
            unique_pairs.push((old_idx, new_idx));
        }
    }

    // Sort by position in old.
    unique_pairs.sort_by_key(|&(oi, _)| oi);

    // Find longest increasing subsequence by new-index.
    patience_lis(&unique_pairs)
}

/// Longest increasing subsequence by the second element (new-index).
fn patience_lis(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if pairs.is_empty() {
        return Vec::new();
    }

    let n = pairs.len();
    // `tails[i]` stores the index in `pairs` of the smallest tail element
    // for an increasing subsequence of length `i+1`.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; n];

    for i in 0..n {
        let new_idx = pairs[i].1;
        let pos = tails.partition_point(|&t| pairs[t].1 < new_idx);
        if pos == tails.len() {
            tails.push(i);
        } else {
            tails[pos] = i;
        }
        if pos > 0 {
            prev[i] = Some(tails[pos - 1]);
        }
    }

    // Reconstruct.
    let mut result = Vec::with_capacity(tails.len());
    // SAFETY: loop above runs at least once (n >= 1) and always pushes to `tails`.
    let mut idx = *tails
        .last()
        .expect("tails is non-empty after processing pairs");
    loop {
        result.push(pairs[idx]);
        match prev[idx] {
            Some(p) => idx = p,
            None => break,
        }
    }
    result.reverse();
    result
}

pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }

    // Keep row tokenization line-oriented; EOF newline deltas are annotated separately.
    text.lines().collect()
}

fn eof_newline_delta(old_text: &str, new_text: &str) -> Option<FileDiffEofNewline> {
    let old_has_newline = old_text.ends_with('\n');
    let new_has_newline = new_text.ends_with('\n');
    match (old_has_newline, new_has_newline) {
        (false, true) => Some(FileDiffEofNewline::MissingInOld),
        (true, false) => Some(FileDiffEofNewline::MissingInNew),
        _ => None,
    }
}

fn myers_fallback_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    // Keep fallback linear by only preserving common prefix/suffix and
    // representing the interior as delete/insert spans.
    let mut prefix = 0usize;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while prefix + suffix < old.len()
        && prefix + suffix < new.len()
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid_end = old.len().saturating_sub(suffix);
    let new_mid_end = new.len().saturating_sub(suffix);

    let mut edits = Vec::new();
    for i in 0..prefix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[i]),
            new: Some(new[i]),
        });
    }
    for &line in &old[prefix..old_mid_end] {
        edits.push(Edit {
            kind: EditKind::Delete,
            old: Some(line),
            new: None,
        });
    }
    for &line in &new[prefix..new_mid_end] {
        edits.push(Edit {
            kind: EditKind::Insert,
            old: None,
            new: Some(line),
        });
    }
    for i in 0..suffix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[old_mid_end + i]),
            new: Some(new[new_mid_end + i]),
        });
    }
    edits
}

fn positional_fallback_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    let mut prefix = 0usize;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while prefix + suffix < old.len()
        && prefix + suffix < new.len()
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid_end = old.len().saturating_sub(suffix);
    let new_mid_end = new.len().saturating_sub(suffix);
    let old_mid = &old[prefix..old_mid_end];
    let new_mid = &new[prefix..new_mid_end];
    let paired = old_mid.len().min(new_mid.len());

    let mut edits = Vec::new();
    for i in 0..prefix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[i]),
            new: Some(new[i]),
        });
    }

    for i in 0..paired {
        if old_mid[i] == new_mid[i] {
            edits.push(Edit {
                kind: EditKind::Equal,
                old: Some(old_mid[i]),
                new: Some(new_mid[i]),
            });
        } else {
            edits.push(Edit {
                kind: EditKind::Delete,
                old: Some(old_mid[i]),
                new: None,
            });
            edits.push(Edit {
                kind: EditKind::Insert,
                old: None,
                new: Some(new_mid[i]),
            });
        }
    }

    for &line in &old_mid[paired..] {
        edits.push(Edit {
            kind: EditKind::Delete,
            old: Some(line),
            new: None,
        });
    }
    for &line in &new_mid[paired..] {
        edits.push(Edit {
            kind: EditKind::Insert,
            old: None,
            new: Some(line),
        });
    }

    for i in 0..suffix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[old_mid_end + i]),
            new: Some(new[new_mid_end + i]),
        });
    }

    edits
}

pub(crate) fn myers_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    // Guard against overflow: if n + m exceeds isize::MAX, use linear fallback.
    let Some(sum) = old.len().checked_add(new.len()) else {
        return myers_fallback_edits(old, new);
    };
    if sum > isize::MAX as usize {
        return myers_fallback_edits(old, new);
    }
    if sum > u32::MAX as usize {
        return myers_fallback_edits(old, new);
    }

    let n = old.len() as isize;
    let m = new.len() as isize;
    let max = (n + m) as usize;
    let offset = max as isize;

    let Some(v_size) = max.checked_mul(2).and_then(|v| v.checked_add(1)) else {
        return myers_fallback_edits(old, new);
    };
    let mut v = vec![0u32; v_size];

    // Compact trace: store only active diagonals per depth.
    // At depth d, active diagonals are -d, -d+2, ..., d (d+1 values).
    // Depth d starts at flat index d*(d+1)/2.
    // This eliminates per-depth Vec allocations and reduces trace memory from
    // d * (2*(n+m)+1) to d*(d+1)/2 elements.
    let mut trace = Vec::<u32>::new();

    {
        let mut x = 0isize;
        let mut y = 0isize;
        while x < n && y < m && old[x as usize] == new[y as usize] {
            x += 1;
            y += 1;
        }
        v[offset as usize] = x as u32;
    }
    // Store depth 0: only diagonal 0
    trace.push(v[offset as usize]);

    let mut last_d = 0usize;
    if v[offset as usize] >= n as u32 && v[offset as usize] >= m as u32 {
        last_d = 0;
    } else {
        'outer: for d in 1..=max {
            let d_isize = d as isize;

            // Update v in-place: at depth d, writes go to diagonals with
            // the same parity as d, reads come from the opposite parity
            // (depth d-1 values), so there is no aliasing.
            for k in (-d_isize..=d_isize).step_by(2) {
                let k_idx = (offset + k) as usize;

                let x = if k == -d_isize
                    || (k != d_isize && v[(offset + k - 1) as usize] < v[(offset + k + 1) as usize])
                {
                    v[(offset + k + 1) as usize]
                } else {
                    v[(offset + k - 1) as usize] + 1
                };

                let mut x = x as isize;
                let mut y = x - k;
                while x < n && y < m && old[x as usize] == new[y as usize] {
                    x += 1;
                    y += 1;
                }
                v[k_idx] = x as u32;

                if x >= n && y >= m {
                    for k2 in (-d_isize..=d_isize).step_by(2) {
                        trace.push(v[(offset + k2) as usize]);
                    }
                    last_d = d;
                    break 'outer;
                }
            }

            for k2 in (-d_isize..=d_isize).step_by(2) {
                trace.push(v[(offset + k2) as usize]);
            }
        }
    }

    if n == 0 && m == 0 {
        return Vec::new();
    }

    if last_d == 0 && n == m && v[offset as usize] == n as u32 {
        return old
            .iter()
            .map(|&s| Edit {
                kind: EditKind::Equal,
                old: Some(s),
                new: Some(s),
            })
            .collect();
    }

    let mut x = n;
    let mut y = m;
    let mut rev: Vec<Edit<'a>> = Vec::with_capacity(last_d + (n + m) as usize);

    for d in (1..=last_d).rev() {
        let prev_depth = d - 1;
        let d_isize = d as isize;
        let k = x - y;

        let prev_k = if k == -d_isize
            || (k != d_isize
                && compact_trace_get(&trace, prev_depth, k - 1)
                    < compact_trace_get(&trace, prev_depth, k + 1))
        {
            k + 1
        } else {
            k - 1
        };

        let prev_x = compact_trace_get(&trace, prev_depth, prev_k) as isize;
        let prev_y = prev_x - prev_k;

        while x > prev_x && y > prev_y {
            rev.push(Edit {
                kind: EditKind::Equal,
                old: Some(old[(x - 1) as usize]),
                new: Some(new[(y - 1) as usize]),
            });
            x -= 1;
            y -= 1;
        }

        if x == prev_x {
            rev.push(Edit {
                kind: EditKind::Insert,
                old: None,
                new: Some(new[(y - 1) as usize]),
            });
            y -= 1;
        } else {
            rev.push(Edit {
                kind: EditKind::Delete,
                old: Some(old[(x - 1) as usize]),
                new: None,
            });
            x -= 1;
        }
    }

    while x > 0 && y > 0 {
        rev.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[(x - 1) as usize]),
            new: Some(new[(y - 1) as usize]),
        });
        x -= 1;
        y -= 1;
    }
    while x > 0 {
        rev.push(Edit {
            kind: EditKind::Delete,
            old: Some(old[(x - 1) as usize]),
            new: None,
        });
        x -= 1;
    }
    while y > 0 {
        rev.push(Edit {
            kind: EditKind::Insert,
            old: None,
            new: Some(new[(y - 1) as usize]),
        });
        y -= 1;
    }

    rev.reverse();
    rev
}

/// Access diagonal `k` at `depth` in the compact trace buffer.
/// At depth d, active diagonals are -d, -d+2, ..., d (d+1 values).
/// Base offset = d*(d+1)/2, index within depth = (k+d)/2.
#[inline]
fn compact_trace_get(trace: &[u32], depth: usize, k: isize) -> u32 {
    let d = depth as isize;
    let base = depth * (depth + 1) / 2;
    let idx = ((k + d) / 2) as usize;
    trace[base + idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remove_row(old_line: u32, old: &str) -> FileDiffRow {
        FileDiffRow {
            kind: FileDiffRowKind::Remove,
            old_line: Some(old_line),
            new_line: None,
            old: Some(old.into()),
            new: None,
            eof_newline: None,
        }
    }

    fn add_row(new_line: u32, new: &str) -> FileDiffRow {
        FileDiffRow {
            kind: FileDiffRowKind::Add,
            old_line: None,
            new_line: Some(new_line),
            old: None,
            new: Some(new.into()),
            eof_newline: None,
        }
    }

    fn changed_line_masks_from_rows(
        rows: &[FileDiffRow],
        old_line_count: usize,
        new_line_count: usize,
    ) -> (Vec<bool>, Vec<bool>) {
        let mut old_mask = vec![false; old_line_count];
        let mut new_mask = vec![false; new_line_count];

        for row in rows {
            match row.kind {
                FileDiffRowKind::Context => {}
                FileDiffRowKind::Remove => mark_changed_line(old_mask.as_mut_slice(), row.old_line),
                FileDiffRowKind::Add => mark_changed_line(new_mask.as_mut_slice(), row.new_line),
                FileDiffRowKind::Modify => {
                    mark_changed_line(old_mask.as_mut_slice(), row.old_line);
                    mark_changed_line(new_mask.as_mut_slice(), row.new_line);
                }
            }
        }

        (old_mask, new_mask)
    }

    fn line_to_row_maps_from_rows(
        rows: &[FileDiffRow],
        old_line_count: usize,
        new_line_count: usize,
    ) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
        let mut old_line_to_row = vec![None; old_line_count];
        let mut new_line_to_row = vec![None; new_line_count];

        for (row_index, row) in rows.iter().enumerate() {
            assign_line_to_row(old_line_to_row.as_mut_slice(), row.old_line, row_index);
            assign_line_to_row(new_line_to_row.as_mut_slice(), row.new_line, row_index);
        }

        (old_line_to_row, new_line_to_row)
    }

    #[test]
    fn file_slice_text_resolved_clamps_to_utf8_char_boundaries() {
        let text = "aÄ日z";
        let temp_path = std::env::temp_dir().join(format!(
            "gitcomet_file_diff_utf8_slice_{}_{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be monotonic enough for test temp path")
                .as_nanos()
        ));
        std::fs::write(&temp_path, text.as_bytes()).expect("write UTF-8 file slice fixture");

        let raw_text =
            FileDiffLineText::file_slice(Arc::new(temp_path.clone()), 0..text.len(), false, false);

        let (slice_text, resolved_range) = raw_text
            .slice_text_resolved(2..6)
            .expect("UTF-8 file slice should resolve");
        assert_eq!(slice_text.as_ref(), "日");
        assert_eq!(resolved_range, 3..6);

        let (empty_slice, empty_range) = raw_text
            .slice_text_resolved(2..5)
            .expect("partial UTF-8 slice should still resolve");
        assert!(empty_slice.is_empty());
        assert_eq!(empty_range, 3..3);

        let _ = std::fs::remove_file(&temp_path);
    }

    #[test]
    fn side_by_side_rows_share_backing_for_context_and_reuse_row_storage_on_clone() {
        let rows = side_by_side_rows("alpha\nbeta\n", "alpha\nbeta changed\n");
        assert_eq!(rows.len(), 2);

        let context = &rows[0];
        assert!(
            context
                .old
                .as_ref()
                .unwrap()
                .shares_backing_with(context.new.as_ref().unwrap())
        );

        let cloned = rows[1].clone();
        assert!(
            rows[1]
                .old
                .as_ref()
                .unwrap()
                .shares_backing_with(cloned.old.as_ref().unwrap())
        );
        assert!(
            rows[1]
                .new
                .as_ref()
                .unwrap()
                .shares_backing_with(cloned.new.as_ref().unwrap())
        );
    }

    fn emitted_line_prefix_counts_from_rows(rows: &[FileDiffRow]) -> (Vec<usize>, Vec<usize>) {
        let mut old_prefix = Vec::with_capacity(rows.len().saturating_add(1));
        let mut new_prefix = Vec::with_capacity(rows.len().saturating_add(1));
        let mut old_count = 0usize;
        let mut new_count = 0usize;
        old_prefix.push(0);
        new_prefix.push(0);

        for row in rows {
            if row.old_line.is_some() {
                old_count = old_count.saturating_add(1);
            }
            if row.new_line.is_some() {
                new_count = new_count.saturating_add(1);
            }
            old_prefix.push(old_count);
            new_prefix.push(new_count);
        }

        (old_prefix, new_prefix)
    }

    #[test]
    fn edits_to_hunks_with_builds_base_relative_hunks() {
        let inserted = String::from("inserted");
        let edits = vec![
            Edit {
                kind: EditKind::Equal,
                old: Some("ctx"),
                new: Some("ctx"),
            },
            Edit {
                kind: EditKind::Delete,
                old: Some("old"),
                new: None,
            },
            Edit {
                kind: EditKind::Insert,
                old: None,
                new: Some(inserted.as_str()),
            },
            Edit {
                kind: EditKind::Equal,
                old: Some("tail"),
                new: Some("tail"),
            },
        ];

        let hunks = edits_to_hunks_with(&edits, |line| line.to_string());
        assert_eq!(
            hunks,
            vec![DiffHunk {
                base_start: 1,
                base_end: 2,
                new_lines: vec!["inserted".to_string()],
            }]
        );
    }

    #[test]
    fn reconstruct_side_with_applies_hunks_and_preserves_context() {
        let base_lines = split_lines("a\nb\nc\n");
        let hunks = vec![
            DiffHunk {
                base_start: 1,
                base_end: 1,
                new_lines: vec!["ins".to_string()],
            },
            DiffHunk {
                base_start: 2,
                base_end: 3,
                new_lines: vec!["c2".to_string()],
            },
        ];
        let mut output: Vec<String> = Vec::new();

        reconstruct_side_with(&base_lines, 0..3, &hunks, &mut output, |line| {
            line.to_string()
        });

        assert_eq!(
            output,
            vec![
                "a".to_string(),
                "ins".to_string(),
                "b".to_string(),
                "c2".to_string()
            ]
        );
    }

    #[test]
    fn pairs_delete_insert_into_modify_rows() {
        let old = "a\nb\nc\n";
        let new = "a\nb2\nc\n";

        let rows = side_by_side_rows(old, new);
        assert_eq!(
            rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
            vec![
                FileDiffRowKind::Context,
                FileDiffRowKind::Modify,
                FileDiffRowKind::Context
            ]
        );

        let mid = &rows[1];
        assert_eq!(mid.old.as_deref(), Some("b"));
        assert_eq!(mid.new.as_deref(), Some("b2"));
        assert_eq!(mid.old_line, Some(2));
        assert_eq!(mid.new_line, Some(2));
        assert_eq!(mid.eof_newline, None);
    }

    #[test]
    fn handles_additions_and_deletions() {
        let old = "a\nb\n";
        let new = "a\nb\nc\n";
        let rows = side_by_side_rows(old, new);
        assert!(rows.iter().any(|r| r.kind == FileDiffRowKind::Add));

        let old = "a\nb\nc\n";
        let new = "a\nc\n";
        let rows = side_by_side_rows(old, new);
        assert!(rows.iter().any(|r| r.kind == FileDiffRowKind::Remove));
    }

    #[test]
    fn marks_missing_newline_in_new_file() {
        let old = "a\n";
        let new = "a";

        let rows = side_by_side_rows(old, new);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, FileDiffRowKind::Modify);
        assert_eq!(rows[0].old.as_deref(), Some("a"));
        assert_eq!(rows[0].new.as_deref(), Some("a"));
        assert_eq!(rows[0].eof_newline, Some(FileDiffEofNewline::MissingInNew));
    }

    #[test]
    fn marks_missing_newline_in_old_file() {
        let old = "a";
        let new = "a\n";

        let rows = side_by_side_rows(old, new);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, FileDiffRowKind::Modify);
        assert_eq!(rows[0].old.as_deref(), Some("a"));
        assert_eq!(rows[0].new.as_deref(), Some("a"));
        assert_eq!(rows[0].eof_newline, Some(FileDiffEofNewline::MissingInOld));
    }

    #[test]
    fn preserves_existing_modify_rows_when_eof_newline_differs() {
        let old = "a\nb\n";
        let new = "a\nc";

        let rows = side_by_side_rows(old, new);
        assert_eq!(
            rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
            vec![FileDiffRowKind::Context, FileDiffRowKind::Modify]
        );
        assert_eq!(rows[1].old.as_deref(), Some("b"));
        assert_eq!(rows[1].new.as_deref(), Some("c"));
        assert_eq!(rows[1].eof_newline, Some(FileDiffEofNewline::MissingInNew));
    }

    #[test]
    fn asymmetric_replacement_pairs_best_matching_lines() {
        let rows = vec![
            remove_row(10, "alpha"),
            remove_row(11, "beta"),
            add_row(20, "intro"),
            add_row(21, "alpha changed"),
            add_row(22, "beta changed"),
        ];

        let paired = pair_replacements(rows);
        assert_eq!(
            paired.iter().map(|row| row.kind).collect::<Vec<_>>(),
            vec![
                FileDiffRowKind::Add,
                FileDiffRowKind::Modify,
                FileDiffRowKind::Modify
            ]
        );
        assert_eq!(paired[0].new.as_deref(), Some("intro"));
        assert_eq!(paired[1].old.as_deref(), Some("alpha"));
        assert_eq!(paired[1].new.as_deref(), Some("alpha changed"));
        assert_eq!(paired[2].old.as_deref(), Some("beta"));
        assert_eq!(paired[2].new.as_deref(), Some("beta changed"));
    }

    #[test]
    fn dissimilar_single_line_replacement_stays_add_remove() {
        let rows = vec![remove_row(1, "aaaaaaaa"), add_row(1, "zzzzzzzz")];
        let paired = pair_replacements(rows);

        assert_eq!(
            paired.iter().map(|row| row.kind).collect::<Vec<_>>(),
            vec![FileDiffRowKind::Remove, FileDiffRowKind::Add]
        );
    }

    #[test]
    fn ascii_prepared_replacement_line_defers_char_allocation_until_needed() {
        let line = PreparedReplacementLine::new("plain-ascii");

        assert_eq!(line.ascii_bytes(), Some("plain-ascii".as_bytes()));
        assert!(line.chars.get().is_none());

        let chars = line.chars();
        assert_eq!(
            chars,
            ['p', 'l', 'a', 'i', 'n', '-', 'a', 's', 'c', 'i', 'i']
        );
        assert!(line.chars.get().is_some());
    }

    #[test]
    fn prepare_replacement_text_cache_ids_dedups_duplicate_texts() {
        let lines = prepare_replacement_lines(&["alpha", "beta", "alpha", "gamma", "beta"]);
        let cache_ids = prepare_replacement_text_cache_ids(&lines);

        assert_eq!(cache_ids.ids, vec![0, 1, 0, 2, 1]);
        assert_eq!(cache_ids.unique_texts, 3);
        assert!(cache_ids.has_duplicates);
    }

    #[test]
    fn shared_boundary_counts_unicode_codepoints() {
        let old = PreparedReplacementLine::new("prefix-é-suffix");
        let new = PreparedReplacementLine::new("prefix-ê-suffix");

        assert_eq!(
            shared_boundary_chars(old.chars(), new.chars()),
            ("prefix-".chars().count(), "-suffix".chars().count())
        );
    }

    #[test]
    fn shared_boundary_bytes_matches_generic_ascii_boundaries() {
        let old = b"prefix-before_source_001-suffix";
        let new = b"prefix-after_source_002-suffix";

        assert_eq!(
            shared_boundary_bytes(old, new),
            shared_boundary_units(old, new)
        );
    }

    #[test]
    fn replacement_pair_cost_reuses_unicode_boundaries() {
        let old = PreparedReplacementLine::new("prefix-é-suffix");
        let new = PreparedReplacementLine::new("prefix-ê-suffix");
        let unrelated = PreparedReplacementLine::new("xxxxxxxxxxxxxxx");
        let mut scratch = LevenshteinScratch::default();

        let shared_edge_cost = replacement_pair_cost(&old, &new, &mut scratch);
        let unrelated_cost = replacement_pair_cost(&old, &unrelated, &mut scratch);

        assert!(
            shared_edge_cost < unrelated_cost,
            "shared prefix/suffix should keep a unicode substitution cheaper than an unrelated line"
        );
    }

    #[test]
    fn side_by_side_aligns_asymmetric_replacement_in_context() {
        let old = "start\nalpha\nbeta\nend\n";
        let new = "start\nintro\nalpha changed\nbeta changed\nend\n";

        let rows = side_by_side_rows(old, new);
        assert_eq!(
            rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
            vec![
                FileDiffRowKind::Context,
                FileDiffRowKind::Add,
                FileDiffRowKind::Modify,
                FileDiffRowKind::Modify,
                FileDiffRowKind::Context
            ]
        );
    }

    #[test]
    fn anchor_groups_contiguous_changes_into_regions() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nx\nc\ny\nd\n";

        let rows = side_by_side_rows(old, new);
        assert_eq!(
            rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
            vec![
                FileDiffRowKind::Context,
                FileDiffRowKind::Modify,
                FileDiffRowKind::Context,
                FileDiffRowKind::Add,
                FileDiffRowKind::Context,
            ]
        );

        let anchors = compute_row_region_anchors(&rows);
        assert_eq!(anchors.row_anchors.len(), rows.len());
        assert_eq!(anchors.region_anchors.len(), 2);

        assert_eq!(
            anchors.region_anchors[0],
            FileDiffRegionAnchor {
                region_id: 0,
                row_start: 1,
                row_end_exclusive: 2,
                old_start_line: Some(2),
                old_end_line: Some(2),
                new_start_line: Some(2),
                new_end_line: Some(2),
            }
        );
        assert_eq!(
            anchors.region_anchors[1],
            FileDiffRegionAnchor {
                region_id: 1,
                row_start: 3,
                row_end_exclusive: 4,
                old_start_line: None,
                old_end_line: None,
                new_start_line: Some(4),
                new_end_line: Some(4),
            }
        );
        assert_eq!(anchors.row_anchors[0].region_id, None);
        assert_eq!(anchors.row_anchors[1].region_id, Some(0));
        assert_eq!(anchors.row_anchors[1].ordinal_in_region, Some(0));
        assert_eq!(anchors.row_anchors[2].region_id, None);
        assert_eq!(anchors.row_anchors[3].region_id, Some(1));
        assert_eq!(anchors.row_anchors[3].ordinal_in_region, Some(0));
        assert_eq!(anchors.row_anchors[4].region_id, None);
    }

    #[test]
    fn anchor_keeps_ordinals_within_single_region() {
        let old = "start\nalpha\nbeta\nend\n";
        let new = "start\nintro\nalpha changed\nbeta changed\nend\n";

        let rows = side_by_side_rows(old, new);
        let anchors = compute_row_region_anchors(&rows);

        assert_eq!(anchors.region_anchors.len(), 1);
        assert_eq!(
            anchors.region_anchors[0],
            FileDiffRegionAnchor {
                region_id: 0,
                row_start: 1,
                row_end_exclusive: 4,
                old_start_line: Some(2),
                old_end_line: Some(3),
                new_start_line: Some(2),
                new_end_line: Some(4),
            }
        );
        assert_eq!(anchors.row_anchors[1].region_id, Some(0));
        assert_eq!(anchors.row_anchors[1].ordinal_in_region, Some(0));
        assert_eq!(anchors.row_anchors[2].ordinal_in_region, Some(1));
        assert_eq!(anchors.row_anchors[3].ordinal_in_region, Some(2));
    }

    #[test]
    fn anchor_handles_rows_without_line_numbers() {
        let rows = vec![FileDiffRow {
            kind: FileDiffRowKind::Modify,
            old_line: None,
            new_line: None,
            old: None,
            new: None,
            eof_newline: Some(FileDiffEofNewline::MissingInNew),
        }];

        let anchors = compute_row_region_anchors(&rows);
        assert_eq!(anchors.row_anchors.len(), 1);
        assert_eq!(anchors.row_anchors[0].region_id, Some(0));
        assert_eq!(anchors.row_anchors[0].ordinal_in_region, Some(0));
        assert_eq!(
            anchors.region_anchors,
            vec![FileDiffRegionAnchor {
                region_id: 0,
                row_start: 0,
                row_end_exclusive: 1,
                old_start_line: None,
                old_end_line: None,
                new_start_line: None,
                new_end_line: None,
            }]
        );
    }

    #[test]
    fn side_by_side_with_anchors_is_deterministic() {
        let old = "a\nb\nc\n";
        let new = "a\nb changed\nc\n";

        let first = side_by_side_rows_with_anchors(old, new);
        let second = side_by_side_rows_with_anchors(old, new);
        assert_eq!(first, second);
        assert_eq!(first.rows.len(), first.anchors.row_anchors.len());
    }

    #[test]
    fn append_side_by_side_rows_with_offsets_matches_materialized_rows() {
        let old = "keep\nold-only\nchange-me\n";
        let new = "keep\nchange-you\nnew-only\n";
        let mut appended = vec![FileDiffRow {
            kind: FileDiffRowKind::Context,
            old_line: Some(1),
            new_line: Some(1),
            old: Some("prefix".into()),
            new: Some("prefix".into()),
            eof_newline: None,
        }];

        append_side_by_side_rows_with_offsets(&mut appended, old, new, 10, 20);

        let mut expected = vec![FileDiffRow {
            kind: FileDiffRowKind::Context,
            old_line: Some(1),
            new_line: Some(1),
            old: Some("prefix".into()),
            new: Some("prefix".into()),
            eof_newline: None,
        }];
        expected.extend(side_by_side_rows(old, new).into_iter().map(|row| {
            FileDiffRow {
                kind: row.kind,
                old_line: row
                    .old_line
                    .map(|line| line.saturating_add(10).saturating_sub(1)),
                new_line: row
                    .new_line
                    .map(|line| line.saturating_add(20).saturating_sub(1)),
                old: row.old,
                new: row.new,
                eof_newline: row.eof_newline,
            }
        }));

        assert_eq!(appended, expected);
    }

    #[test]
    fn plan_metadata_helpers_match_materialized_rows() {
        let old = "keep\nremove only\nbefore change\nshared tail\n";
        let new = "keep\ninsert only\nafter change\nshared tail\nextra add\n";

        let plan = side_by_side_plan(old, new);
        let rows = side_by_side_rows(old, new);
        let inline_row_count = rows.len()
            + rows
                .iter()
                .filter(|row| row.kind == FileDiffRowKind::Modify)
                .count();
        let old_line_count = old.lines().count();
        let new_line_count = new.lines().count();

        assert_eq!(plan.row_count, rows.len());
        assert_eq!(plan.inline_row_count, inline_row_count);
        assert_eq!(
            plan_row_region_anchors(&plan),
            compute_row_region_anchors(&rows)
        );
        assert_eq!(
            plan_changed_line_masks(&plan, old_line_count, new_line_count),
            changed_line_masks_from_rows(&rows, old_line_count, new_line_count)
        );
        assert_eq!(
            plan_line_to_row_maps(&plan, old_line_count, new_line_count),
            line_to_row_maps_from_rows(&rows, old_line_count, new_line_count)
        );
        assert_eq!(
            plan_emitted_line_prefix_counts(&plan),
            emitted_line_prefix_counts_from_rows(&rows)
        );
    }

    #[test]
    fn for_each_side_by_side_row_matches_materialized_rows() {
        let old = "keep\nremove only\nbefore change\nshared tail\n";
        let new = "keep\ninsert only\nafter change\nshared tail\nextra add\n";

        type PlanRow = (
            FileDiffRowKind,
            Option<u32>,
            Option<u32>,
            Option<String>,
            Option<String>,
        );
        let rows = side_by_side_rows(old, new);
        let mut plan_rows: Vec<PlanRow> = Vec::new();
        for_each_side_by_side_row(old, new, |view| {
            let (kind, old_line, new_line, old_text, new_text) = match view {
                PlanRowView::Context {
                    old_line,
                    new_line,
                    text,
                } => (
                    FileDiffRowKind::Context,
                    Some(old_line),
                    Some(new_line),
                    Some(text.to_string()),
                    Some(text.to_string()),
                ),
                PlanRowView::Remove { old_line, text } => (
                    FileDiffRowKind::Remove,
                    Some(old_line),
                    None,
                    Some(text.to_string()),
                    None,
                ),
                PlanRowView::Add { new_line, text } => (
                    FileDiffRowKind::Add,
                    None,
                    Some(new_line),
                    None,
                    Some(text.to_string()),
                ),
                PlanRowView::Modify {
                    old_line,
                    new_line,
                    old_text,
                    new_text,
                } => (
                    FileDiffRowKind::Modify,
                    Some(old_line),
                    Some(new_line),
                    Some(old_text.to_string()),
                    Some(new_text.to_string()),
                ),
            };
            plan_rows.push((kind, old_line, new_line, old_text, new_text));
        });

        assert_eq!(plan_rows.len(), rows.len());
        for (i, row) in rows.iter().enumerate() {
            let (kind, old_line, new_line, old_text, new_text) = &plan_rows[i];
            assert_eq!(*kind, row.kind, "row {i} kind mismatch");
            assert_eq!(*old_line, row.old_line, "row {i} old_line mismatch");
            assert_eq!(*new_line, row.new_line, "row {i} new_line mismatch");
            assert_eq!(
                old_text.as_deref(),
                row.old.as_deref(),
                "row {i} old text mismatch"
            );
            assert_eq!(
                new_text.as_deref(),
                row.new.as_deref(),
                "row {i} new text mismatch"
            );
        }
    }

    #[test]
    fn for_each_side_by_side_row_empty_inputs() {
        let mut count = 0;
        for_each_side_by_side_row("", "", |_| count += 1);
        assert_eq!(count, 0);
    }

    #[test]
    fn for_each_side_by_side_row_replacement_block() {
        let old = "start\nalpha\nbeta\nend\n";
        let new = "start\nintro\nalpha changed\nbeta changed\nend\n";

        let rows = side_by_side_rows(old, new);
        let mut kinds = Vec::new();
        for_each_side_by_side_row(old, new, |view| kinds.push(view.kind()));
        assert_eq!(kinds, rows.iter().map(|r| r.kind).collect::<Vec<_>>());
    }

    #[test]
    fn myers_fallback_preserves_common_prefix_and_suffix() {
        let old = ["keep-1", "keep-2", "old-middle", "keep-3"];
        let new = ["keep-1", "keep-2", "new-middle", "keep-3"];
        let edits = myers_fallback_edits(&old, &new);

        assert_eq!(
            edits.iter().map(|edit| edit.kind).collect::<Vec<_>>(),
            vec![
                EditKind::Equal,
                EditKind::Equal,
                EditKind::Delete,
                EditKind::Insert,
                EditKind::Equal
            ]
        );
    }

    #[test]
    fn positional_fallback_preserves_same_position_context_lines() {
        let old = ["repeat", "old-a", "repeat", "old-b", "repeat"];
        let new = ["repeat", "new-a", "repeat", "new-b", "repeat"];
        let edits = positional_fallback_edits(&old, &new);

        assert_eq!(
            edits.iter().map(|edit| edit.kind).collect::<Vec<_>>(),
            vec![
                EditKind::Equal,
                EditKind::Delete,
                EditKind::Insert,
                EditKind::Equal,
                EditKind::Delete,
                EditKind::Insert,
                EditKind::Equal,
            ]
        );
    }

    #[test]
    fn large_anchorless_repeated_regions_keep_context_localized() {
        let line_count = 700usize;
        let mut old_lines = Vec::with_capacity(line_count);
        let mut new_lines = Vec::with_capacity(line_count);

        for ix in 0..line_count {
            if ix % 2 == 0 {
                old_lines.push("repeat".to_string());
                new_lines.push("repeat".to_string());
            } else {
                old_lines.push(format!("before-{ix:04}"));
                new_lines.push(format!("after-{ix:04}"));
            }
        }

        let old = format!("{}\n", old_lines.join("\n"));
        let new = format!("{}\n", new_lines.join("\n"));
        let rows = side_by_side_rows(&old, &new);

        assert_eq!(rows.len(), line_count);
        for (ix, row) in rows.iter().enumerate() {
            let expected_kind = if ix % 2 == 0 {
                FileDiffRowKind::Context
            } else {
                FileDiffRowKind::Modify
            };
            assert_eq!(row.kind, expected_kind, "unexpected row kind at index {ix}");
        }
    }

    #[test]
    fn patience_lis_handles_empty_and_descending_inputs_without_panicking() {
        assert!(patience_lis(&[]).is_empty());

        let descending = [(0usize, 3usize), (1, 2), (2, 1)];
        let lis = patience_lis(&descending);
        assert_eq!(lis.len(), 1);
        assert!(descending.contains(&lis[0]));
    }

    #[test]
    fn side_by_side_large_files_keep_distant_changes_localized() {
        let line_count = 6_000;
        let mut old_lines: Vec<String> = (0..line_count).map(|i| format!("line-{i:05}")).collect();
        let mut new_lines = old_lines.clone();

        let first_change_ix = 137usize;
        let second_change_ix = line_count - 201;
        old_lines[first_change_ix] = "alpha-old".to_string();
        new_lines[first_change_ix] = "alpha-new".to_string();
        old_lines[second_change_ix] = "omega-old".to_string();
        new_lines[second_change_ix] = "omega-new".to_string();

        let old = format!("{}\n", old_lines.join("\n"));
        let new = format!("{}\n", new_lines.join("\n"));
        let rows = side_by_side_rows(&old, &new);

        let changed: Vec<&FileDiffRow> = rows
            .iter()
            .filter(|row| row.kind != FileDiffRowKind::Context)
            .collect();

        assert_eq!(
            changed.len(),
            2,
            "large files should not collapse distant edits into one huge changed block"
        );
        assert!(
            changed
                .iter()
                .all(|row| row.kind == FileDiffRowKind::Modify),
            "both changes should remain localized modify rows"
        );
        assert_eq!(changed[0].old_line, Some((first_change_ix + 1) as u32));
        assert_eq!(changed[0].new_line, Some((first_change_ix + 1) as u32));
        assert_eq!(changed[1].old_line, Some((second_change_ix + 1) as u32));
        assert_eq!(changed[1].new_line, Some((second_change_ix + 1) as u32));
    }

    #[test]
    fn plan_inline_row_count_tracks_eof_newline_rewrite() {
        let old = "keep\nshared tail\n";
        let new = "keep\nshared tail";

        let plan = side_by_side_plan(old, new);
        let rows = side_by_side_rows(old, new);
        let inline_row_count = rows.len()
            + rows
                .iter()
                .filter(|row| row.kind == FileDiffRowKind::Modify)
                .count();

        assert_eq!(plan.eof_newline, Some(FileDiffEofNewline::MissingInNew));
        assert_eq!(plan.row_count, rows.len());
        assert_eq!(plan.inline_row_count, inline_row_count);
    }

    #[test]
    fn direct_linear_fallback_plan_merges_prefix_middle_and_suffix_runs() {
        let old_text = "keep-1\nkeep-2\nold-a\nold-b\nkeep-3\n";
        let new_text = "keep-1\nkeep-2\nnew-a\nnew-b\nkeep-3\n";
        let old_lines = split_lines(old_text);
        let new_lines = split_lines(new_text);
        let plan = build_linear_fallback_side_by_side_plan_with_pair_cost(
            old_text,
            new_text,
            old_lines.as_slice(),
            new_lines.as_slice(),
            replacement_pair_cost,
        );

        assert_eq!(
            plan.runs,
            vec![
                FileDiffPlanRun::Context {
                    old_start: 0,
                    new_start: 0,
                    len: 2,
                },
                FileDiffPlanRun::Modify {
                    old_start: 2,
                    new_start: 2,
                    len: 2,
                },
                FileDiffPlanRun::Context {
                    old_start: 4,
                    new_start: 4,
                    len: 1,
                },
            ]
        );
    }

    #[cfg(feature = "benchmarks")]
    #[test]
    fn benchmark_backends_match_current_plan() {
        let cases = [
            (
                "start\nprefix-only-change\nshared tail\nend\n",
                "start\nprefix-only-change extended\nshared tail\nend\n",
            ),
            (
                "alpha\nrepeated\nrepeated\nomega\n",
                "alpha\nrepeated changed\nrepeated changed\nomega\n",
            ),
            (
                "context\nfn café() {\n    return old_value;\n}\n",
                "context\nfn café() {\n    return new_value;\n}\n",
            ),
        ];

        for (old, new) in cases {
            let current = side_by_side_plan(old, new);
            let scratch = benchmark_side_by_side_plan_with_replacement_backend(
                old,
                new,
                BenchmarkReplacementDistanceBackend::Scratch,
            );
            let strsim = benchmark_side_by_side_plan_with_replacement_backend(
                old,
                new,
                BenchmarkReplacementDistanceBackend::Strsim,
            );
            assert_eq!(
                current, scratch,
                "scratch backend parity mismatch for old={old:?} new={new:?}"
            );
            assert_eq!(
                current, strsim,
                "backend parity mismatch for old={old:?} new={new:?}"
            );
        }
    }

    #[cfg(feature = "benchmarks")]
    #[test]
    fn levenshtein_scratch_matches_strsim_generic() {
        let mut scratch = LevenshteinScratch::default();

        for (old, new) in [
            ("before_source_001", "after_source_002"),
            ("prefix-only-change", "prefix-only-change extended"),
            ("short", "considerably-longer-string-for-levenshtein"),
            ("", "abc"),
        ] {
            let old_bytes = old.as_bytes();
            let new_bytes = new.as_bytes();
            let old_wrapper = ByteSlice(old_bytes);
            let new_wrapper = ByteSlice(new_bytes);
            assert_eq!(
                scratch.distance(old_bytes, new_bytes),
                strsim::generic_levenshtein(&old_wrapper, &new_wrapper),
                "ascii mismatch for old={old:?} new={new:?}"
            );
        }

        for (old, new) in [("café", "caff"), ("prefix-é-suffix", "prefix-ê-suffix")] {
            let old_chars = old.chars().collect::<Vec<_>>();
            let new_chars = new.chars().collect::<Vec<_>>();
            let old_wrapper = CharSlice(old_chars.as_slice());
            let new_wrapper = CharSlice(new_chars.as_slice());
            assert_eq!(
                scratch.distance(old_chars.as_slice(), new_chars.as_slice()),
                strsim::generic_levenshtein(&old_wrapper, &new_wrapper),
                "unicode mismatch for old={old:?} new={new:?}"
            );
        }
    }

    #[cfg(feature = "benchmarks")]
    #[test]
    fn bitparallel_ascii_levenshtein_matches_strsim_generic() {
        fn generate_cases(alphabet: &[u8], max_len: usize) -> Vec<Vec<u8>> {
            let mut cases = vec![Vec::new()];
            let mut frontier = vec![Vec::new()];

            for _ in 0..max_len {
                let mut next = Vec::new();
                for prefix in &frontier {
                    for &byte in alphabet {
                        let mut candidate = prefix.clone();
                        candidate.push(byte);
                        next.push(candidate);
                    }
                }
                cases.extend(next.iter().cloned());
                frontier = next;
            }

            cases
        }

        let cases = generate_cases(b"ab_", 4);
        let mut scratch = LevenshteinScratch::default();

        for old in &cases {
            for new in &cases {
                let old_wrapper = ByteSlice(old.as_slice());
                let new_wrapper = ByteSlice(new.as_slice());
                let expected = strsim::generic_levenshtein(&old_wrapper, &new_wrapper);

                assert_eq!(
                    bitparallel_levenshtein_bytes(old.as_slice(), new.as_slice()),
                    Some(expected),
                    "bitparallel mismatch for old={:?} new={:?}",
                    String::from_utf8_lossy(old),
                    String::from_utf8_lossy(new)
                );
                assert_eq!(
                    scratch.distance_bytes(old.as_slice(), new.as_slice()),
                    expected,
                    "scratch ascii mismatch for old={:?} new={:?}",
                    String::from_utf8_lossy(old),
                    String::from_utf8_lossy(new)
                );
            }
        }
    }
}
