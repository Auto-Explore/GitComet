use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

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
    pub parent_ids: Vec<CommitId>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffText {
    pub path: PathBuf,
    pub old: Option<String>,
    pub new: Option<String>,
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
    pub text: Arc<str>,
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
    fn classify_unified_line(raw: &str) -> DiffLineKind {
        match raw.as_bytes().first().copied() {
            Some(b'@') if raw.starts_with("@@") => DiffLineKind::Hunk,
            Some(b'd') if raw.starts_with("diff ") || raw.starts_with("deleted file mode ") => {
                DiffLineKind::Header
            }
            Some(b'i') if raw.starts_with("index ") => DiffLineKind::Header,
            Some(b'-') if raw.starts_with("--- ") => DiffLineKind::Header,
            Some(b'-') => DiffLineKind::Remove,
            Some(b'+') if raw.starts_with("+++ ") => DiffLineKind::Header,
            Some(b'+') => DiffLineKind::Add,
            Some(b'n') if raw.starts_with("new file mode ") => DiffLineKind::Header,
            Some(b's') if raw.starts_with("similarity index ") => DiffLineKind::Header,
            Some(b'r') if raw.starts_with("rename from ") || raw.starts_with("rename to ") => {
                DiffLineKind::Header
            }
            Some(b'B') if raw.starts_with("Binary files ") => DiffLineKind::Header,
            _ => DiffLineKind::Context,
        }
    }

    pub fn from_unified_iter<'a>(
        target: DiffTarget,
        lines: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let mut out = Vec::new();
        for raw in lines {
            out.push(DiffLine {
                kind: Self::classify_unified_line(raw),
                text: raw.into(),
            });
        }
        Self { target, lines: out }
    }

    #[cfg(test)]
    pub(crate) fn from_unified_reader<R: std::io::BufRead>(
        target: DiffTarget,
        mut reader: R,
    ) -> std::io::Result<Self> {
        let mut lines = Vec::new();
        let mut buf = String::new();
        loop {
            buf.clear();
            let read = reader.read_line(&mut buf)?;
            if read == 0 {
                break;
            }
            let raw = buf.trim_end_matches(['\n', '\r']);
            lines.push(DiffLine {
                kind: Self::classify_unified_line(raw),
                text: raw.into(),
            });
        }
        Ok(Self { target, lines })
    }

    pub fn from_unified(target: DiffTarget, text: &str) -> Self {
        Self::from_unified_iter(target, text.lines())
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
            parent_ids: vec![CommitId("0".into())],
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
