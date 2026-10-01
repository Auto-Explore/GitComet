//! Line text for file diffs: shared, file-backed, and lazily read slices.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDiffEofNewline {
    MissingInOld,
    MissingInNew,
}

#[derive(Clone, Debug)]
pub(super) enum FileDiffLineTextStorage {
    Owned(Arc<str>),
    SharedSlice { text: Arc<str>, range: Range<usize> },
    SharedLine(SharedLineText),
    FileSlice(Arc<FileDiffLineFileSlice>),
}

#[derive(Clone, Debug)]
pub struct FileDiffLineText {
    pub(super) storage: FileDiffLineTextStorage,
}

#[derive(Debug)]
pub(super) struct FileDiffLineFileSlice {
    pub(super) path: Arc<PathBuf>,
    pub(super) range: Range<usize>,
    pub(super) ascii_only: bool,
    pub(super) has_tabs: bool,
    pub(super) text: OnceLock<Arc<str>>,
}

/// Largest file a batch keeps in memory; larger ones keep an open handle.
const SLICE_BATCH_MAX_FILE_BYTES: u64 = 64 << 20;
/// A split diff reads two files; a few more cover previews beside it.
const SLICE_BATCH_MAX_FILES: usize = 4;

thread_local! {
    static SLICE_BATCH: RefCell<SliceBatch> = const {
        RefCell::new(SliceBatch { depth: 0, sparse: false, files: Vec::new() })
    };
}

struct SliceBatch {
    depth: usize,
    /// Set by the outermost guard: keep handles and read each slice, never
    /// a whole file.
    sparse: bool,
    files: Vec<(PathBuf, BatchedFile)>,
}

enum BatchedFile {
    Bytes(Vec<u8>),
    Handle(File),
    Unreadable,
}

// Per thread, like the batch, so parallel tests cannot disturb a count.
#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static FILE_SLICE_OPENS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FILE_SLICE_BYTES_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Source-backed files this thread opened to read line slices since the
/// last call.
#[cfg(any(test, feature = "test-support"))]
pub fn take_file_slice_opens_for_tests() -> usize {
    FILE_SLICE_OPENS.with(|opens| opens.replace(0))
}

/// Bytes this thread read from source-backed files for line slices since
/// the last call.
#[cfg(any(test, feature = "test-support"))]
pub fn take_file_slice_bytes_read_for_tests() -> usize {
    FILE_SLICE_BYTES_READ.with(|read| read.replace(0))
}

fn count_slice_bytes_read(_bytes: usize) {
    #[cfg(any(test, feature = "test-support"))]
    FILE_SLICE_BYTES_READ.with(|read| read.set(read.get() + _bytes));
}

fn open_slice_file(path: &PathBuf) -> std::io::Result<File> {
    #[cfg(any(test, feature = "test-support"))]
    FILE_SLICE_OPENS.with(|opens| opens.set(opens.get() + 1));
    File::open(path)
}

/// While the guard lives, source-backed line slices read on this thread come
/// from one read of each file instead of an open, seek and read per line. A
/// pass over every row (search) otherwise spends nearly all its time in those
/// system calls. The files are released when the outermost guard drops.
pub fn batch_file_slice_reads() -> FileSliceBatch {
    begin_slice_batch(false)
}

/// [`batch_file_slice_reads`] for passes over a few rows: each file is opened
/// once and each slice is its own seek and read, so a handful of rows never
/// loads whole files.
pub fn batch_file_slice_handles() -> FileSliceBatch {
    begin_slice_batch(true)
}

fn begin_slice_batch(sparse: bool) -> FileSliceBatch {
    SLICE_BATCH.with(|batch| {
        let mut batch = batch.borrow_mut();
        if batch.depth == 0 {
            batch.sparse = sparse;
        }
        batch.depth += 1;
    });
    FileSliceBatch {
        _thread_bound: PhantomData,
    }
}

#[must_use = "reads are batched only while the guard lives"]
pub struct FileSliceBatch {
    _thread_bound: PhantomData<*const ()>,
}

impl Drop for FileSliceBatch {
    fn drop(&mut self) {
        SLICE_BATCH.with(|batch| {
            let mut batch = batch.borrow_mut();
            batch.depth -= 1;
            if batch.depth == 0 {
                batch.files.clear();
            }
        });
    }
}

/// `None` when no batch is active on this thread.
fn read_batched_file_bytes(path: &PathBuf, range: &Range<usize>) -> Option<Option<Vec<u8>>> {
    SLICE_BATCH.with(|batch| {
        let mut batch = batch.borrow_mut();
        if batch.depth == 0 {
            return None;
        }
        let ix = match batch.files.iter().position(|(cached, _)| cached == path) {
            Some(ix) => ix,
            None => {
                if batch.files.len() >= SLICE_BATCH_MAX_FILES {
                    batch.files.remove(0);
                }
                let sparse = batch.sparse;
                let file = match open_slice_file(path) {
                    Ok(mut file) => match file.metadata() {
                        Ok(metadata) if !sparse && metadata.len() <= SLICE_BATCH_MAX_FILE_BYTES => {
                            let mut bytes = Vec::new();
                            match file.read_to_end(&mut bytes) {
                                Ok(read) => {
                                    count_slice_bytes_read(read);
                                    BatchedFile::Bytes(bytes)
                                }
                                Err(_) => BatchedFile::Unreadable,
                            }
                        }
                        _ => BatchedFile::Handle(file),
                    },
                    Err(_) => BatchedFile::Unreadable,
                };
                batch.files.push((path.clone(), file));
                batch.files.len() - 1
            }
        };
        Some(match &mut batch.files[ix].1 {
            BatchedFile::Bytes(bytes) => bytes.get(range.clone()).map(<[u8]>::to_vec),
            BatchedFile::Handle(file) => read_range(file, range.clone()),
            BatchedFile::Unreadable => None,
        })
    })
}

fn read_range(file: &mut File, range: Range<usize>) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(u64::try_from(range.start).ok()?))
        .ok()?;
    let mut bytes = vec![0u8; range.end.saturating_sub(range.start)];
    file.read_exact(&mut bytes).ok()?;
    count_slice_bytes_read(bytes.len());
    Some(bytes)
}

pub(super) fn read_file_bytes(path: &PathBuf, range: Range<usize>) -> Option<Vec<u8>> {
    if range.start > range.end {
        return None;
    }
    if let Some(bytes) = read_batched_file_bytes(path, &range) {
        return bytes;
    }

    read_range(&mut open_slice_file(path).ok()?, range)
}

pub(super) fn read_utf8_file_slice(path: &PathBuf, range: Range<usize>) -> Option<Arc<str>> {
    let bytes = read_file_bytes(path, range)?;
    let text = String::from_utf8(bytes).ok()?;
    Some(Arc::from(text))
}

pub(super) fn read_utf8_file_subslice(
    path: &PathBuf,
    base_range: &Range<usize>,
    subrange: Range<usize>,
    ascii_only: bool,
) -> Option<Arc<str>> {
    read_utf8_file_subslice_with_range(path, base_range, subrange, ascii_only).map(|(text, _)| text)
}

pub(super) fn clamp_byte_up_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset < text.len() && !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

pub(super) fn clamp_byte_down_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

pub(super) fn resolved_utf8_text_subslice_range(
    text: &str,
    range: Range<usize>,
) -> Option<Range<usize>> {
    if range.start > range.end || range.end > text.len() {
        return None;
    }

    let start = clamp_byte_up_to_char_boundary(text, range.start);
    let end = clamp_byte_down_to_char_boundary(text, range.end).max(start);
    Some(start..end)
}

pub(super) fn read_utf8_file_subslice_with_range(
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
