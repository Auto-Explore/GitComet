use memchr::memchr;
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use std::{
    hash::{Hash, Hasher},
    ops::Deref,
};

#[cfg(test)]
use rustc_hash::FxHashMap as HashMap;
#[cfg(test)]
use std::sync::Mutex;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RepoSpec {
    pub workdir: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct CommitId(pub Arc<str>);

pub type CommitParentIds = SmallVec<[CommitId; 2]>;

impl AsRef<str> for CommitId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CommitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commit {
    pub id: CommitId,
    pub parent_ids: CommitParentIds,
    pub summary: Arc<str>,
    pub author: Arc<str>,
    pub time: SystemTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum LogScope {
    CurrentBranch,
    AllBranches,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitDetails {
    pub id: CommitId,
    pub message: String,
    pub committed_at: String,
    pub parent_ids: Vec<CommitId>,
    pub files: Vec<CommitFileChange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitFileChange {
    pub path: PathBuf,
    pub kind: FileStatusKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Branch {
    pub name: String,
    pub target: CommitId,
    pub upstream: Option<Upstream>,
    pub divergence: Option<UpstreamDivergence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tag {
    pub name: String,
    pub target: CommitId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteTag {
    pub remote: String,
    pub name: String,
    pub target: CommitId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Upstream {
    pub remote: String,
    pub branch: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UpstreamDivergence {
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Remote {
    pub name: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<CommitId>,
    pub branch: Option<String>,
    pub detached: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmoduleStatus {
    UpToDate,
    NotInitialized,
    HeadMismatch,
    MergeConflict,
    MissingMapping,
    Unknown(char),
}

impl SubmoduleStatus {
    #[cfg(test)]
    pub fn from_git_status_marker(marker: char) -> Self {
        match marker {
            ' ' => Self::UpToDate,
            '-' => Self::NotInitialized,
            '+' => Self::HeadMismatch,
            'U' => Self::MergeConflict,
            '!' => Self::MissingMapping,
            other => Self::Unknown(other),
        }
    }

    #[cfg(test)]
    pub fn git_status_marker(self) -> char {
        match self {
            Self::UpToDate => ' ',
            Self::NotInitialized => '-',
            Self::HeadMismatch => '+',
            Self::MergeConflict => 'U',
            Self::MissingMapping => '!',
            Self::Unknown(marker) => marker,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Submodule {
    pub path: PathBuf,
    pub head: CommitId,
    pub status: SubmoduleStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteBranch {
    pub remote: String,
    pub name: String,
    pub target: CommitId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileConflictKind {
    BothDeleted,
    AddedByUs,
    DeletedByThem,
    AddedByThem,
    DeletedByUs,
    BothAdded,
    BothModified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileStatus {
    pub path: PathBuf,
    pub kind: FileStatusKind,
    pub conflict: Option<FileConflictKind>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoStatus {
    pub staged: Vec<FileStatus>,
    pub unstaged: Vec<FileStatus>,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileStatusKind {
    Untracked,
    Modified,
    Added,
    Deleted,
    Renamed,
    Conflicted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffArea {
    Staged,
    Unstaged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiffTarget {
    WorkingTree {
        path: PathBuf,
        area: DiffArea,
    },
    Commit {
        commit_id: CommitId,
        path: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diff {
    pub target: DiffTarget,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug)]
struct SharedLineTextStorage {
    text: String,
}

#[derive(Clone, Debug)]
pub struct SharedLineText {
    storage: Arc<SharedLineTextStorage>,
    start: u32,
    len: u32,
}

impl SharedLineText {
    fn from_storage(storage: &Arc<SharedLineTextStorage>, range: std::ops::Range<usize>) -> Self {
        Self {
            storage: Arc::clone(storage),
            start: u32::try_from(range.start).unwrap_or(u32::MAX),
            len: u32::try_from(range.end.saturating_sub(range.start)).unwrap_or(u32::MAX),
        }
    }

    pub fn from_owned(text: impl Into<String>) -> Self {
        let text = text.into();
        let len = text.len();
        Self {
            storage: Arc::new(SharedLineTextStorage { text }),
            start: 0,
            len: u32::try_from(len).unwrap_or(u32::MAX),
        }
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn starts_with(&self, prefix: &str) -> bool {
        self.as_ref().starts_with(prefix)
    }

    pub fn to_arc(&self) -> Arc<str> {
        Arc::from(self.as_ref())
    }

    pub fn slice(&self, range: std::ops::Range<usize>) -> Option<Self> {
        if range.start > range.end || range.end > self.len() {
            return None;
        }

        let start = (self.start as usize).checked_add(range.start)?;
        Some(Self {
            storage: Arc::clone(&self.storage),
            start: u32::try_from(start).ok()?,
            len: u32::try_from(range.end.saturating_sub(range.start)).ok()?,
        })
    }

    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }
}

impl AsRef<str> for SharedLineText {
    fn as_ref(&self) -> &str {
        let start = self.start as usize;
        let end = start.saturating_add(self.len as usize);
        &self.storage.text[start..end]
    }
}

impl Deref for SharedLineText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl Eq for SharedLineText {}

impl PartialEq for SharedLineText {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Hash for SharedLineText {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

impl From<&str> for SharedLineText {
    fn from(value: &str) -> Self {
        Self::from_owned(value.to_owned())
    }
}

impl From<String> for SharedLineText {
    fn from(value: String) -> Self {
        Self::from_owned(value)
    }
}

impl From<SharedLineText> for Arc<str> {
    fn from(value: SharedLineText) -> Self {
        value.to_arc()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffText {
    pub path: PathBuf,
    pub old: Option<String>,
    pub new: Option<String>,
    content_signature: u64,
}

impl FileDiffText {
    pub fn new(path: PathBuf, old: Option<String>, new: Option<String>) -> Self {
        let content_signature =
            Self::content_signature_for_parts(&path, old.as_deref(), new.as_deref());
        Self {
            path,
            old,
            new,
            content_signature,
        }
    }

    pub fn content_signature(&self) -> u64 {
        self.content_signature
    }

    fn content_signature_for_parts(
        path: &std::path::Path,
        old: Option<&str>,
        new: Option<&str>,
    ) -> u64 {
        let mut hasher = FxHasher::default();
        path.hash(&mut hasher);
        old.hash(&mut hasher);
        new.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffImage {
    pub path: PathBuf,
    pub old: Option<Vec<u8>>,
    pub new: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: SharedLineText,
}

pub trait DiffRowProvider {
    type RowRef: Clone;
    type SliceIter<'a>: Iterator<Item = Self::RowRef> + 'a
    where
        Self: 'a;

    fn len_hint(&self) -> usize;
    fn row(&self, ix: usize) -> Option<Self::RowRef>;
    fn slice(&self, start: usize, end: usize) -> Self::SliceIter<'_>;
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct PagedDiffLineProvider {
    lines: Arc<[DiffLine]>,
    page_size: usize,
    pages: Mutex<HashMap<usize, Arc<[DiffLine]>>>,
}

#[cfg(test)]
impl PagedDiffLineProvider {
    pub(crate) fn new(lines: Arc<[DiffLine]>, page_size: usize) -> Self {
        Self {
            lines,
            page_size: page_size.max(1),
            pages: Mutex::new(HashMap::default()),
        }
    }

    pub fn cached_page_count(&self) -> usize {
        self.pages.lock().map(|pages| pages.len()).unwrap_or(0)
    }

    fn page_bounds(&self, page_ix: usize) -> Option<(usize, usize)> {
        let start = page_ix.saturating_mul(self.page_size);
        (start < self.lines.len()).then(|| {
            let end = start.saturating_add(self.page_size).min(self.lines.len());
            (start, end)
        })
    }

    fn load_page(&self, page_ix: usize) -> Option<Arc<[DiffLine]>> {
        if let Ok(pages) = self.pages.lock()
            && let Some(page) = pages.get(&page_ix)
        {
            return Some(Arc::clone(page));
        }

        let (start, end) = self.page_bounds(page_ix)?;
        let page = Arc::<[DiffLine]>::from(&self.lines[start..end]);
        if let Ok(mut pages) = self.pages.lock() {
            return Some(Arc::clone(
                pages.entry(page_ix).or_insert_with(|| Arc::clone(&page)),
            ));
        }
        Some(page)
    }
}

#[cfg(test)]
impl DiffRowProvider for PagedDiffLineProvider {
    type RowRef = DiffLine;
    type SliceIter<'a>
        = std::vec::IntoIter<DiffLine>
    where
        Self: 'a;

    fn len_hint(&self) -> usize {
        self.lines.len()
    }

    fn row(&self, ix: usize) -> Option<Self::RowRef> {
        if ix >= self.lines.len() {
            return None;
        }
        let page_ix = ix / self.page_size;
        let row_ix = ix % self.page_size;
        let page = self.load_page(page_ix)?;
        page.get(row_ix).cloned()
    }

    fn slice(&self, start: usize, end: usize) -> Self::SliceIter<'_> {
        if start >= end || start >= self.lines.len() {
            return Vec::new().into_iter();
        }
        let end = end.min(self.lines.len());
        let mut rows = Vec::with_capacity(end - start);
        let mut ix = start;
        while ix < end {
            let page_ix = ix / self.page_size;
            let page_row_ix = ix % self.page_size;
            let Some(page) = self.load_page(page_ix) else {
                break;
            };
            if let Some(line) = page.get(page_row_ix) {
                rows.push(line.clone());
                ix += 1;
            } else {
                break;
            }
        }
        rows.into_iter()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffLineKind {
    Header,
    Hunk,
    Add,
    Remove,
    Context,
}

impl Diff {
    fn line_capacity_from_bytes(bytes: &[u8]) -> usize {
        if bytes.is_empty() {
            return 0;
        }

        bytes.iter().filter(|&&byte| byte == b'\n').count() + usize::from(!bytes.ends_with(b"\n"))
    }

    fn classify_unified_line_bytes(raw: &[u8]) -> DiffLineKind {
        match raw.first().copied() {
            Some(b'@') if raw.starts_with(b"@@") => DiffLineKind::Hunk,
            Some(b'd') if raw.starts_with(b"diff ") || raw.starts_with(b"deleted file mode ") => {
                DiffLineKind::Header
            }
            Some(b'i') if raw.starts_with(b"index ") => DiffLineKind::Header,
            Some(b'-') if raw.starts_with(b"--- ") => DiffLineKind::Header,
            Some(b'-') => DiffLineKind::Remove,
            Some(b'+') if raw.starts_with(b"+++ ") => DiffLineKind::Header,
            Some(b'+') => DiffLineKind::Add,
            Some(b'n') if raw.starts_with(b"new file mode ") => DiffLineKind::Header,
            Some(b's') if raw.starts_with(b"similarity index ") => DiffLineKind::Header,
            Some(b'r') if raw.starts_with(b"rename from ") || raw.starts_with(b"rename to ") => {
                DiffLineKind::Header
            }
            Some(b'B') if raw.starts_with(b"Binary files ") => DiffLineKind::Header,
            _ => DiffLineKind::Context,
        }
    }

    fn parsed_unified_line(raw: &str) -> DiffLine {
        DiffLine {
            kind: Self::classify_unified_line_bytes(raw.as_bytes()),
            text: SharedLineText::from(raw),
        }
    }

    fn trim_unified_line_bytes(raw: &[u8]) -> &[u8] {
        raw.strip_suffix(b"\r").unwrap_or(raw)
    }

    pub fn from_unified_owned(target: DiffTarget, text: String) -> Self {
        let storage = Arc::new(SharedLineTextStorage { text });
        let bytes = storage.text.as_bytes();
        let mut lines = Vec::with_capacity(Self::line_capacity_from_bytes(bytes));

        let mut start = 0usize;
        while start < bytes.len() {
            let line_end = match memchr(b'\n', &bytes[start..]) {
                Some(offset) => start + offset,
                None => bytes.len(),
            };
            let raw_end = Self::trim_unified_line_bytes(&bytes[start..line_end]).len() + start;
            lines.push(DiffLine {
                kind: Self::classify_unified_line_bytes(&bytes[start..raw_end]),
                text: SharedLineText::from_storage(&storage, start..raw_end),
            });
            if line_end == bytes.len() {
                break;
            }
            start = line_end + 1;
        }

        Self { target, lines }
    }

    pub fn from_unified_iter<'a>(
        target: DiffTarget,
        lines: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let mut out = Vec::new();
        for raw in lines {
            out.push(Self::parsed_unified_line(raw));
        }
        Self { target, lines: out }
    }

    pub fn from_unified_reader<R: std::io::BufRead>(
        target: DiffTarget,
        mut reader: R,
    ) -> std::io::Result<Self> {
        let mut text = String::new();
        reader.read_to_string(&mut text)?;
        Ok(Self::from_unified_owned(target, text))
    }

    pub fn from_unified(target: DiffTarget, text: &str) -> Self {
        Self::from_unified_owned(target, text.to_owned())
    }

    #[cfg(test)]
    pub(crate) fn paged_lines(&self, page_size: usize) -> PagedDiffLineProvider {
        PagedDiffLineProvider::new(Arc::from(self.lines.clone()), page_size)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StashEntry {
    pub index: usize,
    pub id: CommitId,
    pub message: Arc<str>,
    pub created_at: Option<SystemTime>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflogEntry {
    pub index: usize,
    pub new_id: CommitId,
    pub message: Arc<str>,
    pub time: Option<SystemTime>,
    pub selector: Arc<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogPage {
    pub commits: Vec<Commit>,
    pub next_cursor: Option<LogCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogCursor {
    pub last_seen: CommitId,
    /// Optional backend-provided resume hint for the next page. Consumers should
    /// treat this as an opaque optimization and fall back to `last_seen`
    /// semantics when it is absent.
    pub resume_from: Option<CommitId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_hash::FxHashSet;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    #[test]
    fn submodule_status_maps_known_git_markers() {
        assert_eq!(
            SubmoduleStatus::from_git_status_marker(' '),
            SubmoduleStatus::UpToDate
        );
        assert_eq!(
            SubmoduleStatus::from_git_status_marker('-'),
            SubmoduleStatus::NotInitialized
        );
        assert_eq!(
            SubmoduleStatus::from_git_status_marker('+'),
            SubmoduleStatus::HeadMismatch
        );
        assert_eq!(
            SubmoduleStatus::from_git_status_marker('U'),
            SubmoduleStatus::MergeConflict
        );
        assert_eq!(
            SubmoduleStatus::from_git_status_marker('!'),
            SubmoduleStatus::MissingMapping
        );
    }

    #[test]
    fn submodule_status_round_trips_unknown_git_marker() {
        let status = SubmoduleStatus::from_git_status_marker('M');
        assert_eq!(status, SubmoduleStatus::Unknown('M'));
        assert_eq!(status.git_status_marker(), 'M');
    }

    #[test]
    fn unified_reader_matches_string_parser() {
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("src/main.rs"),
            area: DiffArea::Unstaged,
        };
        let unified = "\
diff --git a/src/main.rs b/src/main.rs\n\
index 1111111..2222222 100644\n\
--- a/src/main.rs\n\
+++ b/src/main.rs\n\
@@ -1,2 +1,3 @@\n\
 fn main() {\n\
-    println!(\"old\");\n\
+    println!(\"new\");\n\
+    println!(\"extra\");\n\
 }\n";

        let from_text = Diff::from_unified(target.clone(), unified);
        let from_reader = Diff::from_unified_reader(target, Cursor::new(unified.as_bytes()))
            .expect("reader parse should succeed");

        assert_eq!(from_reader, from_text);
        assert_eq!(from_reader.lines[0].kind, DiffLineKind::Header);
        assert_eq!(from_reader.lines[4].kind, DiffLineKind::Hunk);
        assert_eq!(from_reader.lines[6].kind, DiffLineKind::Remove);
        assert_eq!(from_reader.lines[7].kind, DiffLineKind::Add);
    }

    #[test]
    fn unified_reader_trims_crlf_line_endings() {
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("README.md"),
            area: DiffArea::Unstaged,
        };
        let unified = "\
@@ -1 +1 @@\r\n\
-old\r\n\
+new\r\n";

        let diff = Diff::from_unified_reader(target, Cursor::new(unified.as_bytes()))
            .expect("reader parse should succeed");
        assert_eq!(diff.lines.len(), 3);
        assert_eq!(diff.lines[0].kind, DiffLineKind::Hunk);
        assert_eq!(diff.lines[1].text.as_ref(), "-old");
        assert_eq!(diff.lines[2].text.as_ref(), "+new");
    }

    #[test]
    fn unified_reader_handles_small_buffer_chunks_without_extra_newline_bytes() {
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("src/lib.rs"),
            area: DiffArea::Unstaged,
        };
        let unified = "\
diff --git a/src/lib.rs b/src/lib.rs\r\n\
@@ -1,2 +1,2 @@\r\n\
-alpha beta gamma delta epsilon\r\n\
+omega beta gamma delta epsilon\r\n";

        let reader = std::io::BufReader::with_capacity(7, Cursor::new(unified.as_bytes()));
        let diff = Diff::from_unified_reader(target, reader).expect("reader parse should succeed");

        assert_eq!(diff.lines.len(), 4);
        assert_eq!(
            diff.lines[0].text.as_ref(),
            "diff --git a/src/lib.rs b/src/lib.rs"
        );
        assert_eq!(diff.lines[1].kind, DiffLineKind::Hunk);
        assert_eq!(
            diff.lines[2].text.as_ref(),
            "-alpha beta gamma delta epsilon"
        );
        assert_eq!(
            diff.lines[3].text.as_ref(),
            "+omega beta gamma delta epsilon"
        );
    }

    #[test]
    fn unified_reader_lines_share_backing_storage() {
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("README.md"),
            area: DiffArea::Unstaged,
        };
        let unified = "\
@@ -1 +1 @@\n\
-old\n\
+new\n";

        let diff = Diff::from_unified_reader(target, Cursor::new(unified.as_bytes()))
            .expect("reader parse should succeed");

        assert_eq!(diff.lines.len(), 3);
        assert!(diff.lines[0].text.shares_storage_with(&diff.lines[1].text));
        assert!(diff.lines[1].text.shares_storage_with(&diff.lines[2].text));
    }

    #[test]
    fn paged_provider_loads_pages_on_demand() {
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("src/lib.rs"),
            area: DiffArea::Unstaged,
        };
        let unified = "\
diff --git a/src/lib.rs b/src/lib.rs\n\
@@ -1,4 +1,4 @@\n\
 old1\n\
-old2\n\
+new2\n\
 old3\n";
        let diff = Diff::from_unified(target, unified);
        let provider = diff.paged_lines(2);

        assert_eq!(provider.cached_page_count(), 0);
        assert_eq!(provider.len_hint(), diff.lines.len());

        let line = provider.row(3).expect("line 3 should exist");
        assert_eq!(line.text.as_ref(), "-old2");
        assert_eq!(provider.cached_page_count(), 1);

        let line = provider.row(0).expect("line 0 should exist");
        assert_eq!(line.text.as_ref(), "diff --git a/src/lib.rs b/src/lib.rs");
        assert_eq!(provider.cached_page_count(), 2);

        let slice = provider
            .slice(2, 5)
            .map(|line| line.text.to_string())
            .collect::<Vec<_>>();
        assert_eq!(slice, vec!["old1", "-old2", "+new2"]);
        assert_eq!(provider.cached_page_count(), 3);
    }

    // --- Tests moved from tests/domain_smoke.rs ---

    #[test]
    fn commit_id_is_hashable() {
        let mut set = FxHashSet::default();
        set.insert(CommitId("a".into()));
        set.insert(CommitId("b".into()));
        assert!(set.contains(&CommitId("a".into())));
    }

    #[test]
    fn log_cursor_roundtrips() {
        let cursor = LogCursor {
            last_seen: CommitId("deadbeef".into()),
            resume_from: Some(CommitId("feedface".into())),
        };
        assert_eq!(cursor.last_seen.as_ref(), "deadbeef");
        assert_eq!(
            cursor.resume_from.as_ref().map(AsRef::as_ref),
            Some("feedface")
        );
    }

    #[test]
    fn commit_struct_is_constructible() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        let commit = Commit {
            id: CommitId("1".into()),
            parent_ids: smallvec::smallvec![CommitId("0".into())],
            summary: "test".into(),
            author: "me".into(),
            time: now,
        };
        assert_eq!(&*commit.summary, "test");
    }

    // --- Tests moved from tests/diff_from_unified.rs ---

    #[test]
    fn diff_from_unified_classifies_lines() {
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        };

        let text = "\
diff --git a/a.txt b/a.txt
index 0000000..1111111 100644
--- a/a.txt
+++ b/a.txt
@@ -0,0 +1,2 @@
+hello
 world
-bye
";

        let diff = Diff::from_unified(target, text);
        assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Header));
        assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Hunk));
        assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Add));
        assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Remove));
        assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Context));
    }

    #[test]
    fn diff_from_unified_treats_three_dash_content_as_removed_line() {
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        };

        let diff = Diff::from_unified(
            target,
            "\
@@ -1 +1 @@
----keep this as removed content
+++ header
",
        );

        assert_eq!(diff.lines[1].kind, DiffLineKind::Remove);
        assert_eq!(diff.lines[2].kind, DiffLineKind::Header);
    }

    #[test]
    fn stash_and_reflog_entries_share_arc_text_on_clone() {
        let stash = StashEntry {
            index: 0,
            id: CommitId("stash".into()),
            message: "stash message".into(),
            created_at: None,
        };
        let stash_clone = stash.clone();
        assert!(Arc::ptr_eq(&stash.message, &stash_clone.message));

        let reflog = ReflogEntry {
            index: 0,
            new_id: CommitId("head".into()),
            message: "reflog message".into(),
            time: None,
            selector: "HEAD@{0}".into(),
        };
        let reflog_clone = reflog.clone();
        assert!(Arc::ptr_eq(&reflog.message, &reflog_clone.message));
        assert!(Arc::ptr_eq(&reflog.selector, &reflog_clone.selector));
    }
}
