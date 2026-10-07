use crate::text_format::TextFormat;
use memchr::memchr;
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use std::{
    hash::{Hash, Hasher},
    ops::Deref,
};

#[cfg(test)]
use rustc_hash::FxHashMap;
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

impl CommitId {
    /// A fixed seven-character prefix for compact labels.
    pub fn short(&self) -> &str {
        short_commit_id(&self.0)
    }

    /// Whether this id is git's "not committed yet" marker rather than a real
    /// commit. See [`is_uncommitted_commit_id`].
    pub fn is_uncommitted(&self) -> bool {
        is_uncommitted_commit_id(&self.0)
    }
}

/// A fixed seven-character prefix for labels that do not require a unique
/// abbreviation. Accepts a borrowed string to avoid allocating a `CommitId`.
pub fn short_commit_id(id: &str) -> &str {
    &id[..id
        .char_indices()
        .nth(7)
        .map_or(id.len(), |(index, _)| index)]
}

/// Whether `id` is git's all-zero "not committed yet" object id, emitted by
/// `git blame`/`git diff` for working-tree lines that have no commit. Works for
/// any hash length (40-char SHA-1, 64-char SHA-256). An empty id is also treated
/// as uncommitted: it never names a real commit (it only arises from an
/// unparsed/unknown id), so callers that gate navigation or commit attribution
/// on a real commit must exclude it the same way.
pub fn is_uncommitted_commit_id(id: &str) -> bool {
    id.is_empty() || id.bytes().all(|b| b == b'0')
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commit {
    pub id: CommitId,
    pub parent_ids: CommitParentIds,
    pub summary: Arc<str>,
    pub author: Arc<str>,
    pub time: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentCommitMessage {
    pub id: CommitId,
    pub summary: Arc<str>,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum HistoryMode {
    #[default]
    FullReachable,
    FirstParent,
    NoMerges,
    MergesOnly,
    AllBranches,
}

impl HistoryMode {
    #[allow(non_upper_case_globals)]
    pub const CurrentBranch: Self = Self::FirstParent;

    pub fn is_all_branches(self) -> bool {
        matches!(self, Self::AllBranches)
    }

    pub fn is_current_branch_mode(self) -> bool {
        !self.is_all_branches()
    }

    pub fn guarantees_head_visibility(self) -> bool {
        matches!(self, Self::FullReachable | Self::FirstParent)
    }

    pub fn uses_first_parent_pagination(self) -> bool {
        matches!(self, Self::FirstParent)
    }
}

pub type LogScope = HistoryMode;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitDetails {
    pub id: CommitId,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    /// Author timestamp in unix seconds; 0 when unavailable.
    pub authored_at_unix: i64,
    pub committed_at: String,
    /// Committer timestamp in unix seconds; 0 when unavailable. Lets the UI
    /// format the date per user preference instead of showing the raw
    /// `committed_at` string.
    pub committed_at_unix: i64,
    pub parent_ids: Vec<CommitId>,
    pub files: Vec<CommitFileChange>,
}

/// Verification outcome for a commit signature, from `git log %G?`.
///
/// `E` (key missing) and `N` (unsigned) have no variant: both mean "no badge",
/// and are represented by the absence of a [`CommitSignature`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignatureStatus {
    /// `G` — good signature from a certified key.
    Good,
    /// `U` — good signature, key not certified in the local web of trust.
    GoodUncertified,
    /// `X` — good signature that has expired.
    Expired,
    /// `Y` — good signature made by a key that has expired.
    ExpiredKey,
    /// `B` — signature does not match the commit.
    Bad,
    /// `R` — good signature made by a revoked key.
    Revoked,
}

impl SignatureStatus {
    /// `None` for the codes that carry no badge (`E`, `N`) and anything unknown.
    pub fn from_git_code(code: u8) -> Option<Self> {
        match code {
            b'G' => Some(Self::Good),
            b'U' => Some(Self::GoodUncertified),
            b'X' => Some(Self::Expired),
            b'Y' => Some(Self::ExpiredKey),
            b'B' => Some(Self::Bad),
            b'R' => Some(Self::Revoked),
            _ => None,
        }
    }

    /// Whether the signature checked out with a trusted signing identity.
    pub fn is_verified(self) -> bool {
        matches!(self, Self::Good)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignatureFormat {
    OpenPgp,
    Ssh,
    X509,
}

impl SignatureFormat {
    /// Reads the format off the armor header of a `gpgsig` payload.
    pub fn from_armor(signature: &[u8]) -> Option<Self> {
        // Git matches raw prefixes, including PGP MESSAGE armor. Leading
        // whitespace is invalid and must not be sent to `git log` as signed.
        [
            (b"-----BEGIN PGP SIGNATURE-----".as_slice(), Self::OpenPgp),
            (b"-----BEGIN PGP MESSAGE-----".as_slice(), Self::OpenPgp),
            (b"-----BEGIN SSH SIGNATURE-----".as_slice(), Self::Ssh),
            (b"-----BEGIN SIGNED MESSAGE-----".as_slice(), Self::X509),
        ]
        .into_iter()
        .find_map(|(prefix, format)| signature.starts_with(prefix).then_some(format))
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenPgp => "GPG",
            Self::Ssh => "SSH",
            Self::X509 => "X.509",
        }
    }
}

/// A set of [`SignatureFormat`]s, such as the formats whose verifier is installed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SignatureFormats(u8);

impl SignatureFormats {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(0b111);

    const fn bit(format: SignatureFormat) -> u8 {
        match format {
            SignatureFormat::OpenPgp => 0b001,
            SignatureFormat::Ssh => 0b010,
            SignatureFormat::X509 => 0b100,
        }
    }

    #[must_use]
    pub const fn with(self, format: SignatureFormat) -> Self {
        Self(self.0 | Self::bit(format))
    }

    pub const fn contains(self, format: SignatureFormat) -> bool {
        self.0 & Self::bit(format) != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// A verified commit signature. Only built for commits that earn a badge, so
/// "absent from the signature map" and "no badge" are the same thing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitSignature {
    pub status: SignatureStatus,
    pub format: SignatureFormat,
    /// Signer identity (`%GS`); `None` when git could not name one.
    pub signer: Option<Arc<str>>,
    /// Key id or fingerprint (`%GK`).
    pub key_id: Option<Arc<str>>,
}

/// A Git object id in hex: the blob holding a file's content, or the commit
/// a submodule's gitlink points at.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ObjectHash(pub Arc<str>);

impl ObjectHash {
    /// Git's all-zero id, which stands for "no object" (for example the
    /// working-tree side of a comparison), is `None`.
    pub fn from_hex(hex: &str) -> Option<Self> {
        (!hex.is_empty() && !hex.bytes().all(|byte| byte == b'0')).then(|| Self(hex.into()))
    }
}

impl AsRef<str> for ObjectHash {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A tree entry's mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FileMode {
    Regular,
    Executable,
    Symlink,
    /// A submodule pointer.
    Gitlink,
}

impl FileMode {
    /// Parses Git's octal mode (`100644`, `100755`, `120000`, `160000`); other
    /// modes, including "absent" (`000000`) and trees, are `None`.
    pub fn from_octal(mode: &[u8]) -> Option<Self> {
        match mode {
            b"100644" | b"100664" => Some(Self::Regular),
            b"100755" => Some(Self::Executable),
            b"120000" => Some(Self::Symlink),
            b"160000" => Some(Self::Gitlink),
            _ => None,
        }
    }
}

/// One file a commit or comparison changes. Build it with
/// [`CommitFileChange::new`]: it is non-exhaustive so fields can be added
/// without breaking callers.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CommitFileChange {
    /// The file's path on the newer side (for a deletion, its last path).
    pub path: PathBuf,
    pub kind: FileStatusKind,
    pub is_submodule: bool,
    /// Added line count; `None` for binary files, submodules, or when the
    /// commit was too large to compute per-file stats.
    pub additions: Option<u32>,
    /// Removed line count; `None` under the same conditions as `additions`.
    pub deletions: Option<u32>,
    /// Set when the new side (or the old side of a deletion) is a Git LFS
    /// pointer or git-annex key.
    pub large_file: Option<crate::large_files::LargeFileState>,
    /// Where a renamed or copied file came from.
    pub old_path: Option<PathBuf>,
    /// The content on each side; `None` where the side is absent or is the
    /// working tree, or when the backend does not report ids.
    pub old_id: Option<ObjectHash>,
    pub new_id: Option<ObjectHash>,
    /// The mode on each side; `None` where the side is absent or unknown.
    pub old_mode: Option<FileMode>,
    pub new_mode: Option<FileMode>,
    /// The edit as text, for ordering files changed the same way together;
    /// `None` under the same conditions as `additions`, or when nothing in
    /// the file's text changed.
    pub edit: Option<crate::edit_signature::EditSignature>,
}

impl CommitFileChange {
    pub fn new(path: PathBuf, kind: FileStatusKind) -> Self {
        Self {
            path,
            kind,
            is_submodule: false,
            additions: None,
            deletions: None,
            old_path: None,
            old_id: None,
            new_id: None,
            old_mode: None,
            new_mode: None,
            large_file: None,
            edit: None,
        }
    }

    pub fn with_submodule(mut self, is_submodule: bool) -> Self {
        self.is_submodule = is_submodule;
        self
    }

    pub fn with_line_counts(mut self, additions: Option<u32>, deletions: Option<u32>) -> Self {
        self.additions = additions;
        self.deletions = deletions;
        self
    }

    /// The counts and the edit together, as a line diff reports them.
    pub fn with_line_stats(self, stats: LineStats) -> Self {
        self.with_line_counts(stats.additions, stats.deletions)
            .with_edit(stats.edit)
    }

    pub fn with_edit(mut self, edit: Option<crate::edit_signature::EditSignature>) -> Self {
        self.edit = edit;
        self
    }

    /// Records a rename or copy source. The same path is not a rename.
    pub fn with_old_path(mut self, old_path: Option<PathBuf>) -> Self {
        self.old_path = old_path.filter(|old| *old != self.path);
        self
    }

    pub fn with_ids(mut self, old_id: Option<ObjectHash>, new_id: Option<ObjectHash>) -> Self {
        self.old_id = old_id;
        self.new_id = new_id;
        self
    }

    pub fn with_modes(mut self, old_mode: Option<FileMode>, new_mode: Option<FileMode>) -> Self {
        self.old_mode = old_mode;
        self.new_mode = new_mode;
        self
    }

    pub fn with_large_file(
        mut self,
        large_file: Option<crate::large_files::LargeFileState>,
    ) -> Self {
        self.large_file = large_file;
        self
    }

    /// Only the mode changed (for example the executable bit).
    pub fn is_mode_change_only(&self) -> bool {
        self.old_id.is_some()
            && self.old_id == self.new_id
            && self.old_mode.is_some()
            && self.new_mode.is_some()
            && self.old_mode != self.new_mode
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Branch {
    pub name: String,
    pub target: CommitId,
    pub upstream: Option<Upstream>,
    pub divergence: Option<UpstreamDivergence>,
}

/// Decorative tip-commit details for a ref, keyed by short refname (`main`,
/// `origin/main`). Deliberately kept out of [`Branch`]: branch listing has a
/// loose-ref fast path that never reads commit objects, and this data is only
/// needed when a picker that displays it is opened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefMetadata {
    pub author: String,
    /// Committer date as a Unix timestamp in seconds.
    pub committed_at: i64,
    pub summary: String,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileEntryKind {
    File,
    Directory,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Default)]
pub enum FileSource {
    #[default]
    WorkingDirectory,
    Commit(CommitId),
    Branch(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub path: Arc<PathBuf>,
    pub kind: FileEntryKind,
    pub depth: usize,
    /// Excluded by gitignore; only the working-tree listing's ignored pass sets it.
    pub ignored: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<CommitId>,
    pub branch: Option<String>,
    pub detached: bool,
}

/// Uncommitted-change counts for one linked worktree, gathered by opening a
/// throwaway handle at its path. Counts follow the same rules as the history
/// pane's working-tree row: staged and unstaged are summed, so a file that is
/// both staged and dirty counts twice.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorktreeDirtySummary {
    pub path: PathBuf,
    /// Commit this worktree has checked out. Carried here rather than looked up
    /// in the repo's worktree list, because that list is loaded lazily (only
    /// when the sidebar's Worktrees section is opened) and the history rows must
    /// not depend on the sidebar having been expanded.
    pub head: Option<CommitId>,
    pub branch: Option<String>,
    pub detached: bool,
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
    /// The changed files themselves, kept split the way git reports them.
    ///
    /// Only populated for the worktree whose row is selected. Every other
    /// worktree carries counts alone: the lists are read by one pane, for one
    /// worktree at a time, while an un-ignored `target/` or `node_modules/` in
    /// *any* linked worktree would otherwise park tens of thousands of
    /// `FileStatus` values in application state and re-compare them on every
    /// scan. Empty lists next to non-zero counts mean "not loaded yet", not
    /// "no files" -- the scan that carries them is requested when the row is
    /// selected.
    pub staged: Vec<FileStatus>,
    pub unstaged: Vec<FileStatus>,
    /// Filled only for the selected worktree, like the lists above. Empty means
    /// "not loaded", not "no counts".
    pub line_stats: UncommittedLineStats,
}

impl WorktreeDirtySummary {
    pub fn is_dirty(&self) -> bool {
        self.added > 0 || self.modified > 0 || self.deleted > 0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    pub recorded_head: CommitId,
    pub checked_out_head: Option<CommitId>,
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

/// `None` means unknown: binary, over the size cap, or not reported by git at
/// all — an untracked file is in neither index lane.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LineStats {
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    /// The edit as text; `None` when the counts are unknown or nothing in
    /// the text changed.
    pub edit: Option<crate::edit_signature::EditSignature>,
}

impl LineStats {
    pub const UNKNOWN: Self = Self {
        additions: None,
        deletions: None,
        edit: None,
    };
}

impl From<(Option<u32>, Option<u32>)> for LineStats {
    fn from((additions, deletions): (Option<u32>, Option<u32>)) -> Self {
        Self {
            additions,
            deletions,
            edit: None,
        }
    }
}

/// Keyed the way `FileStatus` reports paths, so the join is a map lookup. An
/// absent key means git has no counts for that file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UncommittedLineStats {
    pub staged: rustc_hash::FxHashMap<PathBuf, LineStats>,
    pub unstaged: rustc_hash::FxHashMap<PathBuf, LineStats>,
}

impl UncommittedLineStats {
    pub fn for_area(&self, area: DiffArea) -> &rustc_hash::FxHashMap<PathBuf, LineStats> {
        match area {
            DiffArea::Staged => &self.staged,
            DiffArea::Unstaged => &self.unstaged,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SubmoduleInnerChange {
    pub path: PathBuf,
    pub kind: FileStatusKind,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    /// Where a renamed file came from inside the submodule.
    pub old_path: Option<PathBuf>,
}

impl SubmoduleInnerChange {
    pub fn new(path: PathBuf, kind: FileStatusKind) -> Self {
        Self {
            path,
            kind,
            additions: None,
            deletions: None,
            old_path: None,
        }
    }

    pub fn with_line_counts(mut self, additions: Option<u32>, deletions: Option<u32>) -> Self {
        self.additions = additions;
        self.deletions = deletions;
        self
    }

    /// Records a rename source. The same path is not a rename.
    pub fn with_old_path(mut self, old_path: Option<PathBuf>) -> Self {
        self.old_path = old_path.filter(|old| *old != self.path);
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmoduleDiffSummaryMode {
    Worktree,
    CommitHistory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmoduleDiffRangeKind {
    StagedPointer,
    UnstagedPointer,
    CommitHistory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmoduleDiffRange {
    pub kind: SubmoduleDiffRangeKind,
    pub from: Option<CommitId>,
    pub to: Option<CommitId>,
    pub changes: Vec<SubmoduleInnerChange>,
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmoduleDiffSummary {
    pub path: PathBuf,
    pub mode: SubmoduleDiffSummaryMode,
    pub status: Option<SubmoduleStatus>,
    pub checkout_available: bool,
    pub commit_id: Option<CommitId>,
    pub parent_commit_id: Option<CommitId>,
    pub checked_out_head: Option<CommitId>,
    pub ranges: Vec<SubmoduleDiffRange>,
    pub live_staged: Vec<SubmoduleInnerChange>,
    pub live_unstaged: Vec<SubmoduleInnerChange>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoStatus {
    /// Shared: the backend's staged-status cache and the app model both keep
    /// these lists, and every refresh copied them in and out.
    pub staged: Arc<Vec<FileStatus>>,
    pub unstaged: Arc<Vec<FileStatus>>,
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

/// Bucket a status list into `(added, modified, deleted)` the way the history
/// pane's uncommitted-changes rows report it.
///
/// Shared so the working-tree row and the per-worktree scan can never drift
/// apart. Callers that want a combined figure sum the staged and unstaged
/// results, which double-counts a file that is both staged and dirty in the
/// worktree — that is the long-standing behaviour of the working-tree row.
pub fn count_file_statuses(entries: &[FileStatus]) -> (usize, usize, usize) {
    let mut added = 0usize;
    let mut modified = 0usize;
    let mut deleted = 0usize;
    for entry in entries {
        match entry.kind {
            FileStatusKind::Untracked | FileStatusKind::Added => added += 1,
            FileStatusKind::Deleted => deleted += 1,
            FileStatusKind::Modified | FileStatusKind::Renamed | FileStatusKind::Conflicted => {
                modified += 1
            }
        }
    }
    (added, modified, deleted)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DiffArea {
    Staged,
    Unstaged,
}

/// What a blame computation runs against: a committed revision (or HEAD when
/// `None`), or the working-tree content shown on the new side of a
/// staged/unstaged diff. Working-tree blame attributes uncommitted lines to a
/// synthetic "Not Committed Yet" entry.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum BlameSource {
    Revision(Option<String>),
    WorkingTree(DiffArea),
}

/// Git's canonical empty tree object. Usable anywhere a diff wants a base with
/// no content — comparing against it is how the changes a root commit
/// *introduces* are expressed, since a root commit has no parent to diff from.
pub const EMPTY_TREE_ID_SHA1: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
pub const EMPTY_TREE_ID_SHA256: &str =
    "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321";

/// The empty tree in the same object format as a complete hexadecimal ID.
/// Abbreviations and revision names do not identify an object format.
pub fn empty_tree_id_like(sibling: &CommitId) -> Option<CommitId> {
    if !sibling
        .as_ref()
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    let id = match sibling.as_ref().len() {
        40 => EMPTY_TREE_ID_SHA1,
        64 => EMPTY_TREE_ID_SHA256,
        _ => return None,
    };
    Some(CommitId(id.into()))
}

pub fn is_empty_tree_id(id: &str) -> bool {
    id.eq_ignore_ascii_case(EMPTY_TREE_ID_SHA1) || id.eq_ignore_ascii_case(EMPTY_TREE_ID_SHA256)
}

/// What a diff shows. Build targets with the constructors
/// ([`DiffTarget::working_tree`], [`DiffTarget::commit`],
/// [`DiffTarget::commit_range`]): the variants are non-exhaustive so fields
/// can be added without breaking callers, and patterns end in `..`.
///
/// Equality ignores a file's `old_path`: it is derived from the commit and the
/// path, so two targets naming the same file are the same target whether or
/// not one carries its rename source.
#[derive(Clone, Debug)]
pub enum DiffTarget {
    #[non_exhaustive]
    WorkingTree {
        path: PathBuf,
        area: DiffArea,
        /// A linked worktree the path is in; `None` is the repository's own.
        worktree: Option<PathBuf>,
    },
    /// One file's change in a commit, against its first parent.
    #[non_exhaustive]
    Commit {
        commit_id: CommitId,
        path: PathBuf,
        /// Where `path` was renamed or copied from in this commit, so the old
        /// side loads from there.
        old_path: Option<PathBuf>,
    },
    #[non_exhaustive]
    CommitRange {
        from_commit_id: CommitId,
        /// The newer side of the comparison. `Some(id)` compares two commits
        /// (`git diff from to`); `None` compares `from` against the live working
        /// tree (`git diff from`), so the tip tracks uncommitted changes.
        to_commit_id: Option<CommitId>,
        path: Option<PathBuf>,
        /// Where `path` was renamed or copied from across the range.
        old_path: Option<PathBuf>,
    },
}

impl PartialEq for DiffTarget {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::WorkingTree {
                    path,
                    area,
                    worktree,
                },
                Self::WorkingTree {
                    path: other_path,
                    area: other_area,
                    worktree: other_worktree,
                },
            ) => path == other_path && area == other_area && worktree == other_worktree,
            (
                Self::Commit {
                    commit_id, path, ..
                },
                Self::Commit {
                    commit_id: other_id,
                    path: other_path,
                    ..
                },
            ) => commit_id == other_id && path == other_path,
            (
                Self::CommitRange {
                    from_commit_id,
                    to_commit_id,
                    path,
                    ..
                },
                Self::CommitRange {
                    from_commit_id: other_from,
                    to_commit_id: other_to,
                    path: other_path,
                    ..
                },
            ) => from_commit_id == other_from && to_commit_id == other_to && path == other_path,
            _ => false,
        }
    }
}

impl Eq for DiffTarget {}

impl DiffTarget {
    /// A working-tree file in `area` (unstaged or staged).
    pub fn working_tree(path: PathBuf, area: DiffArea) -> Self {
        Self::WorkingTree {
            path,
            area,
            worktree: None,
        }
    }

    /// The same working-tree file in the linked worktree at `worktree`;
    /// other targets are unchanged.
    pub fn in_worktree(mut self, worktree: PathBuf) -> Self {
        if let Self::WorkingTree { worktree: slot, .. } = &mut self {
            *slot = Some(normalize_worktree_path(&worktree));
        }
        self
    }

    /// The linked worktree a working-tree target reads, if not the
    /// repository's own.
    pub fn worktree(&self) -> Option<&std::path::Path> {
        match self {
            Self::WorkingTree { worktree, .. } => worktree.as_deref(),
            Self::Commit { .. } | Self::CommitRange { .. } => None,
        }
    }

    /// What `commit_id` changed in one file.
    pub fn commit(commit_id: CommitId, path: PathBuf) -> Self {
        Self::Commit {
            commit_id,
            path,
            old_path: None,
        }
    }

    /// One file of a commit's change list, carrying its rename source.
    pub fn commit_change(commit_id: CommitId, change: &CommitFileChange) -> Self {
        Self::commit(commit_id, change.path.clone()).with_old_path(change.old_path.clone())
    }

    /// `from` to `to`, or to the working tree when `to` is `None`.
    pub fn commit_range(
        from_commit_id: CommitId,
        to_commit_id: Option<CommitId>,
        path: Option<PathBuf>,
    ) -> Self {
        Self::CommitRange {
            from_commit_id,
            to_commit_id,
            path,
            old_path: None,
        }
    }

    /// The file's rename or copy source, for commit and range targets; the
    /// same path is not a rename. Working-tree targets are unchanged.
    pub fn with_old_path(mut self, source: Option<PathBuf>) -> Self {
        match &mut self {
            Self::Commit { path, old_path, .. } => {
                *old_path = source.filter(|source| source != path);
            }
            Self::CommitRange { path, old_path, .. } => {
                *old_path = source.filter(|source| Some(source) != path.as_ref());
            }
            Self::WorkingTree { .. } => {}
        }
        self
    }

    /// The target for one file of a commit's or comparison's change list,
    /// carrying its rename source.
    pub fn for_change(self, change: &CommitFileChange) -> Self {
        match self {
            Self::Commit { commit_id, .. } => Self::commit(commit_id, change.path.clone()),
            Self::CommitRange {
                from_commit_id,
                to_commit_id,
                ..
            } => Self::commit_range(from_commit_id, to_commit_id, Some(change.path.clone())),
            working_tree @ Self::WorkingTree { .. } => return working_tree,
        }
        .with_old_path(change.old_path.clone())
    }

    /// Where the file shown was renamed or copied from, if it was.
    pub fn old_file_path(&self) -> Option<&std::path::Path> {
        match self {
            Self::Commit { old_path, .. } | Self::CommitRange { old_path, .. } => {
                old_path.as_deref()
            }
            Self::WorkingTree { .. } => None,
        }
    }

    /// The single file this target shows, if it shows one.
    pub fn file_path(&self) -> Option<&std::path::Path> {
        match self {
            Self::WorkingTree { path, .. } | Self::Commit { path, .. } => Some(path),
            Self::CommitRange { path, .. } => path.as_deref(),
        }
    }
}

/// A worktree path as targets, sessions and watches compare it: lexical
/// (no `.` components or trailing separator), so equal spellings match
/// without touching the file system.
pub fn normalize_worktree_path(path: &std::path::Path) -> PathBuf {
    path.components().collect()
}

/// Where the change "Apply change" applies comes from.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum ApplyChangeSource {
    /// A commit's change against its first parent.
    Commit(CommitId),
    /// The difference between two commits.
    Range { from: CommitId, to: CommitId },
}

/// Files whose change "Apply change" takes from one source, named as they
/// are in the checkout.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ApplyChangeTarget {
    pub source: ApplyChangeSource,
    pub paths: Vec<PathBuf>,
}

impl ApplyChangeTarget {
    pub fn commit(commit_id: CommitId, path: PathBuf) -> Self {
        Self {
            source: ApplyChangeSource::Commit(commit_id),
            paths: vec![path],
        }
    }

    pub fn range(from: CommitId, to: CommitId, paths: Vec<PathBuf>) -> Self {
        Self {
            source: ApplyChangeSource::Range { from, to },
            paths,
        }
    }

    /// The target for one file diff; `None` for a working-tree diff or a
    /// comparison to the working tree, which have no change to apply.
    pub fn from_diff_target(target: &DiffTarget) -> Option<Self> {
        match target {
            DiffTarget::Commit {
                commit_id, path, ..
            } => Some(Self::commit(commit_id.clone(), path.clone())),
            DiffTarget::CommitRange {
                from_commit_id,
                to_commit_id: Some(to_commit_id),
                path: Some(path),
                ..
            } => Some(Self::range(
                from_commit_id.clone(),
                to_commit_id.clone(),
                vec![path.clone()],
            )),
            _ => None,
        }
    }
}

/// The exact result of an apply whose commit step failed. Carried by the
/// retried command so reopening a repository neither loses it nor leaves
/// hidden state that can make a later, independent apply commit staged work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyFileChangeRetry {
    pub target: ApplyChangeTarget,
    pub head: Option<CommitId>,
    pub index: Vec<ApplyFileChangeIndexEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyFileChangeIndexEntry {
    pub path: PathBuf,
    /// Git tree mode and object id, or `None` for a deleted path.
    pub version: Option<(u16, CommitId)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffPreviewTextSide {
    Old,
    New,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffPreviewTextFile {
    pub path: PathBuf,
    pub side: DiffPreviewTextSide,
    pub large_file: Option<crate::large_files::LargeFileSide>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diff {
    pub target: DiffTarget,
    pub lines: Vec<DiffLine>,
}

/// How the two sides of one file section of a patch decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffSectionFormats {
    pub old: TextFormat,
    pub new: TextFormat,
}

impl DiffSectionFormats {
    pub const UTF_8: Self = Self {
        old: TextFormat::UTF_8,
        new: TextFormat::UTF_8,
    };

    pub fn is_plain_utf8(&self) -> bool {
        self.old.is_plain_utf8() && self.new.is_plain_utf8()
    }

    /// Guess from the section's own bytes, for callers that know nothing
    /// about the file. Valid UTF-8 stays UTF-8.
    pub fn sniff(section: &[u8]) -> Self {
        Self::resolve(
            section,
            &crate::text_format::TextAttributes::default(),
            None,
        )
    }

    /// Decode patch content per side without loading or transcoding the full
    /// files. Textconv output can use a different encoding from those files.
    pub fn resolve(
        section: &[u8],
        attributes: &crate::text_format::TextAttributes,
        encoding: Option<crate::text_format::TextEncoding>,
    ) -> Self {
        use crate::text_format::{ContentSniffer, SideKind};
        let mut old = ContentSniffer::new();
        let mut new = ContentSniffer::new();
        let mut headers = ContentSniffer::new();
        for line in section.split_inclusive(|&byte| byte == b'\n') {
            match Diff::classify_unified_line_bytes(line) {
                DiffLineKind::Remove => old.feed(&line[1..]),
                DiffLineKind::Add => new.feed(&line[1..]),
                DiffLineKind::Context if line.starts_with(b" ") => {
                    old.feed(&line[1..]);
                    new.feed(&line[1..]);
                }
                DiffLineKind::Hunk => {
                    if let Some(offset) = line
                        .get(2..)
                        .and_then(|rest| memchr::memmem::find(rest, b"@@"))
                    {
                        // A truncated function name cannot override complete
                        // file content. It only supplies evidence when the old
                        // content is ASCII and encoding has no effect on it.
                        headers.feed(&line[offset + 4..]);
                    }
                }
                _ => {}
            }
        }
        let resolve = |sniff: crate::text_format::ContentSniff| TextFormat {
            bom: false,
            ..sniff
                .resolve(SideKind::GitInternal, attributes, encoding)
                .format
        };
        let old = old.finish();
        let plain_ascii = old.ascii_only
            && old
                .resolve(SideKind::GitInternal, attributes, encoding)
                .source
                == crate::text_format::FormatSource::Utf8;
        Self {
            old: resolve(if plain_ascii { headers.finish() } else { old }),
            new: resolve(new.finish()),
        }
    }
}

#[derive(Debug)]
struct SharedLineTextStorage {
    text: String,
    /// Original bytes of lines decoded from another encoding; `None` when
    /// `text` holds them verbatim.
    raw: Option<Box<RawLineBytes>>,
}

#[derive(Debug)]
struct RawLineBytes {
    bytes: Vec<u8>,
    /// One entry per parsed line, sorted by `text_start`.
    lines: Vec<RawLineRange>,
}

#[derive(Clone, Copy, Debug)]
struct RawLineRange {
    text_start: u32,
    text_len: u32,
    raw_start: u32,
    /// Includes a trailing CR.
    raw_len: u32,
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
            storage: Arc::new(SharedLineTextStorage { text, raw: None }),
            start: 0,
            len: u32::try_from(len).unwrap_or(u32::MAX),
        }
    }

    /// The bytes this line had in `git diff` output, trailing CR included —
    /// what a patch must contain for `git apply` to match. Slices of a line
    /// give their decoded text.
    pub fn raw_bytes(&self) -> &[u8] {
        let start = self.start as usize;
        let end = start.saturating_add(self.len as usize);
        if let Some(raw) = self.storage.raw.as_deref() {
            if let Ok(ix) = raw
                .lines
                .binary_search_by_key(&self.start, |line| line.text_start)
            {
                let line = raw.lines[ix];
                if line.text_len == self.len {
                    let raw_start = line.raw_start as usize;
                    return &raw.bytes[raw_start..raw_start + line.raw_len as usize];
                }
            }
            return self.as_ref().as_bytes();
        }
        // Parsed UTF-8 keeps the stripped CR right after the line's range.
        let bytes = self.storage.text.as_bytes();
        let whole_line = start == 0 || bytes.get(start - 1) == Some(&b'\n');
        if whole_line
            && bytes.get(end) == Some(&b'\r')
            && bytes.get(end + 1).is_none_or(|next| *next == b'\n')
        {
            return &bytes[start..end + 1];
        }
        &bytes[start..end]
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
    /// Raw bytes count too: a line re-read as CRLF, or in another encoding,
    /// must not be mistaken for the old one.
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref() && self.raw_bytes() == other.raw_bytes()
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
pub struct FileDiffTextSource {
    /// The original, content-addressed bytes before transcoding.
    pub raw_path: Option<PathBuf>,
    /// UTF-8 content, transcoded from the file's encoding when needed.
    pub path: PathBuf,
    pub identity: Arc<str>,
    /// How the original content was read; `None` when not resolved.
    pub format: Option<crate::text_format::SideTextFormat>,
}

impl FileDiffTextSource {
    pub fn new(path: PathBuf) -> Self {
        let identity = Self::filesystem_identity(&path);
        Self {
            raw_path: None,
            path,
            identity,
            format: None,
        }
    }

    pub fn with_identity(path: PathBuf, identity: impl Into<Arc<str>>) -> Self {
        Self {
            raw_path: None,
            path,
            identity: identity.into(),
            format: None,
        }
    }

    pub fn with_raw_path(mut self, path: PathBuf) -> Self {
        self.raw_path = Some(path);
        self
    }

    pub fn with_format(mut self, format: crate::text_format::SideTextFormat) -> Self {
        self.format = Some(format);
        self
    }

    fn filesystem_identity(path: &std::path::Path) -> Arc<str> {
        let mut hasher = FxHasher::default();
        path.hash(&mut hasher);
        if let Ok(metadata) = std::fs::metadata(path) {
            metadata.len().hash(&mut hasher);
            metadata.is_file().hash(&mut hasher);
            if let Ok(modified) = metadata.modified()
                && let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH)
            {
                duration.as_secs().hash(&mut hasher);
                duration.subsec_nanos().hash(&mut hasher);
            }
        }
        Arc::from(format!("{:016x}", hasher.finish()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffText {
    pub path: PathBuf,
    pub old_source: Option<FileDiffTextSource>,
    pub new_source: Option<FileDiffTextSource>,
    pub old: Option<Arc<str>>,
    pub new: Option<Arc<str>>,
    /// Set when a side's git form is a Git LFS pointer or git-annex key.
    pub old_large: Option<crate::large_files::LargeFileSide>,
    pub new_large: Option<crate::large_files::LargeFileSide>,
    content_signature: u64,
}

impl FileDiffText {
    pub fn new(path: PathBuf, old: Option<String>, new: Option<String>) -> Self {
        Self::new_shared(path, old.map(Arc::<str>::from), new.map(Arc::<str>::from))
    }

    pub fn new_shared(path: PathBuf, old: Option<Arc<str>>, new: Option<Arc<str>>) -> Self {
        let content_signature =
            Self::content_signature_for_parts(&path, None, None, old.as_deref(), new.as_deref());
        Self {
            path,
            old_source: None,
            new_source: None,
            old,
            new,
            old_large: None,
            new_large: None,
            content_signature,
        }
    }

    pub fn new_sources(
        path: PathBuf,
        old_source: Option<FileDiffTextSource>,
        new_source: Option<FileDiffTextSource>,
    ) -> Self {
        let content_signature = Self::content_signature_for_parts(
            &path,
            old_source.as_ref(),
            new_source.as_ref(),
            None,
            None,
        );
        Self {
            path,
            old_source,
            new_source,
            old: None,
            new: None,
            old_large: None,
            new_large: None,
            content_signature,
        }
    }

    /// Attach large-file sides, folding them into the content signature.
    pub fn with_large_sides(
        mut self,
        old_large: Option<crate::large_files::LargeFileSide>,
        new_large: Option<crate::large_files::LargeFileSide>,
    ) -> Self {
        let mut hasher = FxHasher::default();
        self.content_signature.hash(&mut hasher);
        old_large.hash(&mut hasher);
        new_large.hash(&mut hasher);
        self.content_signature = hasher.finish();
        self.old_large = old_large;
        self.new_large = new_large;
        self
    }

    pub fn content_signature(&self) -> u64 {
        self.content_signature
    }

    fn content_signature_for_parts(
        path: &std::path::Path,
        old_source: Option<&FileDiffTextSource>,
        new_source: Option<&FileDiffTextSource>,
        old: Option<&str>,
        new: Option<&str>,
    ) -> u64 {
        let mut hasher = FxHasher::default();
        path.hash(&mut hasher);
        old_source
            .map(|source| (&source.path, &source.identity, &source.format))
            .hash(&mut hasher);
        new_source
            .map(|source| (&source.path, &source.identity, &source.format))
            .hash(&mut hasher);
        old.hash(&mut hasher);
        new.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileDiffImage {
    pub path: PathBuf,
    pub old: Option<Vec<u8>>,
    pub new: Option<Vec<u8>>,
    pub old_large: Option<crate::large_files::LargeFileSide>,
    pub new_large: Option<crate::large_files::LargeFileSide>,
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
    pages: Mutex<FxHashMap<usize, Arc<[DiffLine]>>>,
}

#[cfg(test)]
impl PagedDiffLineProvider {
    pub(crate) fn new(lines: Arc<[DiffLine]>, page_size: usize) -> Self {
        Self {
            lines,
            page_size: page_size.max(1),
            pages: Mutex::new(FxHashMap::default()),
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
    pub const MAX_UNIFIED_BYTES: u64 = 64 * 1024 * 1024;
    pub const MAX_UNIFIED_LINES: usize = 1_000_000;
    /// Start of the row appended when a diff was cut at a display limit.
    pub const TRUNCATION_NOTICE_PREFIX: &'static str = "... diff truncated";

    fn line_capacity_from_bytes(bytes: &[u8]) -> usize {
        if bytes.is_empty() {
            return 0;
        }

        memchr::memchr_iter(b'\n', bytes).count() + usize::from(!bytes.ends_with(b"\n"))
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
        let storage = Arc::new(SharedLineTextStorage { text, raw: None });
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

    /// Parse `git diff` output bytes. `formats` is asked once per file
    /// section (from one `diff --git` line to the next) with that section's
    /// bytes. When every section is plain UTF-8 and the bytes are valid, this
    /// is [`Self::from_unified_owned`] without a copy; otherwise each line is
    /// decoded (removed lines as the old side, the rest as the new side) and
    /// the original bytes are kept for [`SharedLineText::raw_bytes`].
    pub fn from_unified_bytes(
        target: DiffTarget,
        bytes: Vec<u8>,
        mut formats: impl FnMut(&[u8]) -> DiffSectionFormats,
    ) -> Self {
        let sections = Self::unified_sections(&bytes);
        let section_formats: Vec<DiffSectionFormats> = sections
            .iter()
            .map(|section| formats(&bytes[section.clone()]))
            .collect();
        let bytes = if section_formats
            .iter()
            .all(DiffSectionFormats::is_plain_utf8)
        {
            match String::from_utf8(bytes) {
                Ok(text) => return Self::from_unified_owned(target, text),
                Err(err) => err.into_bytes(),
            }
        } else {
            bytes
        };

        let mut text = String::with_capacity(bytes.len() + bytes.len() / 4);
        let mut ranges = Vec::with_capacity(Self::line_capacity_from_bytes(&bytes));
        let mut kinds = Vec::with_capacity(ranges.capacity());
        for (section, section_format) in sections.into_iter().zip(section_formats) {
            let mut start = section.start;
            while start < section.end {
                let line_end = memchr(b'\n', &bytes[start..section.end])
                    .map_or(section.end, |offset| start + offset);
                let line = Self::trim_unified_line_bytes(&bytes[start..line_end]);
                let kind = Self::classify_unified_line_bytes(line);
                let text_start = text.len();
                Self::decode_unified_line(line, kind, section_format, &mut text);
                ranges.push(RawLineRange {
                    text_start: u32::try_from(text_start).unwrap_or(u32::MAX),
                    text_len: u32::try_from(text.len() - text_start).unwrap_or(u32::MAX),
                    raw_start: u32::try_from(start).unwrap_or(u32::MAX),
                    raw_len: u32::try_from(line_end - start).unwrap_or(u32::MAX),
                });
                kinds.push(kind);
                // Separates lines so a range never runs into the next one.
                text.push('\n');
                start = line_end + 1;
            }
        }

        let storage = Arc::new(SharedLineTextStorage {
            text,
            raw: Some(Box::new(RawLineBytes {
                bytes,
                lines: ranges.clone(),
            })),
        });
        let lines = ranges
            .iter()
            .zip(kinds)
            .map(|(range, kind)| {
                let start = range.text_start as usize;
                DiffLine {
                    kind,
                    text: SharedLineText::from_storage(
                        &storage,
                        start..start + range.text_len as usize,
                    ),
                }
            })
            .collect();
        Self { target, lines }
    }

    /// Byte ranges of each file section; output before the first `diff `
    /// line is a section of its own.
    fn unified_sections(bytes: &[u8]) -> Vec<std::ops::Range<usize>> {
        let mut sections = Vec::new();
        let mut section_start = 0usize;
        let mut line_start = 0usize;
        while line_start < bytes.len() {
            let rest = &bytes[line_start..];
            if line_start > section_start
                && (rest.starts_with(b"diff --git ") || rest.starts_with(b"diff --cc "))
            {
                sections.push(section_start..line_start);
                section_start = line_start;
            }
            match memchr(b'\n', rest) {
                Some(offset) => line_start += offset + 1,
                None => break,
            }
        }
        if section_start < bytes.len() || sections.is_empty() {
            sections.push(section_start..bytes.len());
        }
        sections
    }

    fn decode_unified_line(
        line: &[u8],
        kind: DiffLineKind,
        formats: DiffSectionFormats,
        out: &mut String,
    ) {
        let decode_into = |bytes: &[u8], format: TextFormat, out: &mut String| {
            out.push_str(&crate::text_format::decode(bytes, format).text);
        };
        match kind {
            DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                if matches!(line.first(), Some(b'+' | b'-' | b' ')) =>
            {
                let format = if kind == DiffLineKind::Remove {
                    formats.old
                } else {
                    formats.new
                };
                out.push(char::from(line[0]));
                decode_into(&line[1..], format, out);
            }
            DiffLineKind::Hunk => {
                // Git takes function context from the old side.
                let context_start = line
                    .get(2..)
                    .and_then(|rest| memchr::memmem::find(rest, b"@@"))
                    .map_or(line.len(), |offset| offset + 4);
                let context_start = context_start.min(line.len());
                decode_into(&line[..context_start], TextFormat::UTF_8, out);
                decode_into(&line[context_start..], formats.old, out);
            }
            _ => decode_into(line, TextFormat::UTF_8, out),
        }
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

    /// Read a unified diff, cutting it at the display limits rather than failing.
    fn read_unified_text_with_limits<R: std::io::Read>(
        reader: R,
        max_bytes: u64,
        max_lines: usize,
    ) -> std::io::Result<(String, Option<String>)> {
        let (bytes, notice) = Self::read_unified_bytes_with_limits(reader, max_bytes, max_lines)?;
        let text = String::from_utf8(bytes).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unified diff is not valid UTF-8: {err}"),
            )
        })?;
        Ok((text, notice))
    }

    /// Raw `git diff` output cut at the display limits, and the notice to show
    /// when it was cut.
    pub fn read_unified_bytes<R: std::io::Read>(
        reader: R,
    ) -> std::io::Result<(Vec<u8>, Option<String>)> {
        Self::read_unified_bytes_with_limits(
            reader,
            Self::MAX_UNIFIED_BYTES,
            Self::MAX_UNIFIED_LINES,
        )
    }

    fn read_unified_bytes_with_limits<R: std::io::Read>(
        mut reader: R,
        max_bytes: u64,
        max_lines: usize,
    ) -> std::io::Result<(Vec<u8>, Option<String>)> {
        let mut bytes = Vec::new();
        (&mut reader)
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;

        let mut notice = None;
        if bytes.len() as u64 > max_bytes {
            // Drain to the end so Git exits normally rather than by SIGPIPE,
            // whose non-zero status would outrank the diff we just read. A
            // failure here must not cost us that diff either.
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
            bytes.truncate(Self::line_boundary_at_or_before(&bytes, max_bytes as usize));
            notice = Some(Self::truncation_notice(&format!("{max_bytes}-byte")));
        }

        if Self::line_capacity_from_bytes(&bytes) > max_lines {
            bytes.truncate(Self::line_boundary_after_lines(&bytes, max_lines));
            notice = Some(Self::truncation_notice(&format!("{max_lines}-line")));
        }
        Ok((bytes, notice))
    }

    fn truncation_notice(limit: &str) -> String {
        format!(
            "{} at the {limit} display limit",
            Self::TRUNCATION_NOTICE_PREFIX
        )
    }

    /// Largest cut at or before `limit` on a line boundary, else a UTF-8 one.
    fn line_boundary_at_or_before(bytes: &[u8], limit: usize) -> usize {
        let head = &bytes[..limit.min(bytes.len())];
        if let Some(index) = memchr::memrchr(b'\n', head) {
            return index + 1;
        }
        let mut cut = head.len();
        while cut > 0 && bytes.get(cut).is_some_and(|byte| byte & 0xC0 == 0x80) {
            cut -= 1;
        }
        cut
    }

    /// The byte just past the `count`-th line.
    fn line_boundary_after_lines(bytes: &[u8], count: usize) -> usize {
        let mut start = 0usize;
        for _ in 0..count {
            match memchr(b'\n', &bytes[start..]) {
                Some(offset) => start += offset + 1,
                None => return bytes.len(),
            }
        }
        start
    }

    /// Whether a prospective unified diff fits the display limits.
    pub fn fits_unified_limits(byte_count: u64, line_count: usize) -> bool {
        byte_count <= Self::MAX_UNIFIED_BYTES && line_count <= Self::MAX_UNIFIED_LINES
    }

    pub fn read_unified_text<R: std::io::Read>(
        reader: R,
    ) -> std::io::Result<(String, Option<String>)> {
        Self::read_unified_text_with_limits(
            reader,
            Self::MAX_UNIFIED_BYTES,
            Self::MAX_UNIFIED_LINES,
        )
    }

    fn from_unified_reader_with_limits<R: std::io::Read>(
        target: DiffTarget,
        reader: R,
        max_bytes: u64,
        max_lines: usize,
    ) -> std::io::Result<Self> {
        let (bytes, notice) = Self::read_unified_bytes_with_limits(reader, max_bytes, max_lines)?;
        let mut diff = Self::from_unified_bytes(target, bytes, DiffSectionFormats::sniff);
        if let Some(notice) = notice {
            diff.lines.push(DiffLine {
                kind: DiffLineKind::Header,
                text: SharedLineText::from(notice.as_str()),
            });
        }
        Ok(diff)
    }

    pub fn from_unified_reader<R: std::io::Read>(
        target: DiffTarget,
        reader: R,
    ) -> std::io::Result<Self> {
        Self::from_unified_reader_with_limits(
            target,
            reader,
            Self::MAX_UNIFIED_BYTES,
            Self::MAX_UNIFIED_LINES,
        )
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
    /// The committer name recorded on this reflog line (git stores one
    /// committer identity per entry, distinct from the commit's own author).
    /// Empty when the backend could not resolve one.
    pub author: Arc<str>,
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
    /// Optional backend-provided opaque token for resuming more complex walks.
    /// Consumers must treat this as an implementation detail and fall back to
    /// `last_seen` semantics when it is absent or stale.
    pub resume_token: Option<Arc<str>>,
}

#[cfg(test)]
mod tests {
    mod comparison;

    use super::*;
    use rustc_hash::FxHashSet;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    #[test]
    fn working_tree_targets_name_their_worktree() {
        let main = DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged);
        let linked = main.clone().in_worktree("/repos/feature/./".into());
        assert_eq!(
            linked.worktree(),
            Some(std::path::Path::new("/repos/feature"))
        );
        assert_eq!(main.worktree(), None);
        assert_ne!(main, linked, "the same file in another worktree differs");
        assert_eq!(
            linked,
            main.clone().in_worktree("/repos/feature".into()),
            "spellings of one path agree"
        );
        let commit = DiffTarget::commit(CommitId("abc".into()), "a.rs".into());
        assert_eq!(commit.clone().in_worktree("/repos/feature".into()), commit);
    }

    #[test]
    fn empty_tree_uses_the_complete_sibling_object_format() {
        for (len, expected) in [(40, EMPTY_TREE_ID_SHA1), (64, EMPTY_TREE_ID_SHA256)] {
            assert_eq!(
                empty_tree_id_like(&CommitId("a".repeat(len).into()))
                    .unwrap()
                    .as_ref(),
                expected
            );
            assert!(is_empty_tree_id(expected));
        }
        for invalid in ["main", "abc1234", "", &"x".repeat(40)] {
            assert!(empty_tree_id_like(&CommitId(invalid.into())).is_none());
            assert!(!is_empty_tree_id(invalid));
        }
    }

    #[test]
    fn review_truncated_hunk_context_does_not_change_side_encoding() {
        let patch = b"@@ -1 +1 @@ function \xc3\n-\xc3\xa4\n+\xc3\xb6\n";
        assert_eq!(DiffSectionFormats::sniff(patch), DiffSectionFormats::UTF_8);
    }

    #[test]
    fn uncommitted_commit_id_detects_zero_and_empty_for_any_hash_length() {
        // git's all-zero sentinel, SHA-1 (40) and SHA-256 (64) widths.
        assert!(is_uncommitted_commit_id(&"0".repeat(40)));
        assert!(is_uncommitted_commit_id(&"0".repeat(64)));
        // An empty/unparsed id is treated as uncommitted (names no real commit).
        assert!(is_uncommitted_commit_id(""));
        // Real commit ids of either width are committed.
        assert!(!is_uncommitted_commit_id("deadbeef"));
        assert!(!is_uncommitted_commit_id(&format!("{}1", "0".repeat(63))));
        // CommitId delegates to the same rule.
        assert!(CommitId("0".repeat(40).into()).is_uncommitted());
        assert!(CommitId("".into()).is_uncommitted());
        assert!(!CommitId("abc123".into()).is_uncommitted());
    }

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
        let target = DiffTarget::working_tree(PathBuf::from("src/main.rs"), DiffArea::Unstaged);
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
        let target = DiffTarget::working_tree(PathBuf::from("README.md"), DiffArea::Unstaged);
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

    fn target(path: &str) -> DiffTarget {
        DiffTarget::working_tree(PathBuf::from(path), DiffArea::Unstaged)
    }

    #[test]
    fn crlf_lines_keep_their_cr_in_raw_bytes() {
        let unified = "@@ -1 +1 @@\r\n-old\r\n+new\n context\r\r\n";
        let diff =
            Diff::from_unified_reader(target("a.txt"), Cursor::new(unified.as_bytes())).unwrap();
        assert_eq!(diff.lines[1].text.raw_bytes(), b"-old\r");
        assert_eq!(diff.lines[2].text.raw_bytes(), b"+new");
        // Only the final CR was stripped; one CR stays in the text.
        assert_eq!(diff.lines[3].text.as_ref(), " context\r");
        assert_eq!(diff.lines[3].text.raw_bytes(), b" context\r\r");
        // A slice is not a whole line and gets its text.
        let slice = diff.lines[1].text.slice(1..4).unwrap();
        assert_eq!(slice.raw_bytes(), b"old");
    }

    #[test]
    fn crlf_and_lf_versions_of_a_line_are_not_equal() {
        let crlf = Diff::from_unified(target("a.txt"), "+x\r\n");
        let lf = Diff::from_unified(target("a.txt"), "+x\n");
        assert_eq!(crlf.lines[0].text.as_ref(), lf.lines[0].text.as_ref());
        assert_ne!(crlf, lf);
    }

    #[test]
    fn non_utf8_output_decodes_per_side_and_keeps_raw_bytes() {
        let raw: &[u8] = b"diff --git a/a.txt b/a.txt\nindex 1..2 100644\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@ caf\xe9\n caf\xe9\r\n-na\xefve\n+na\xc3\xafve\n";
        let latin1 = TextFormat {
            encoding: crate::text_format::TextEncoding::WINDOWS_1252,
            bom: false,
        };
        let diff =
            Diff::from_unified_bytes(target("a.txt"), raw.to_vec(), |_| DiffSectionFormats {
                old: latin1,
                new: TextFormat::UTF_8,
            });
        let texts: Vec<&str> = diff.lines.iter().map(|line| line.text.as_ref()).collect();
        // The context line decodes as the new side (UTF-8), so 0xE9 is invalid there.
        assert_eq!(texts[4], "@@ -1,2 +1,2 @@ café");
        assert_eq!(texts[5], " caf\u{fffd}");
        assert_eq!(texts[6], "-naïve");
        assert_eq!(texts[7], "+naïve");
        assert_eq!(diff.lines[5].text.raw_bytes(), b" caf\xe9\r");
        assert_eq!(diff.lines[6].text.raw_bytes(), b"-na\xefve");
        assert_eq!(diff.lines[6].kind, DiffLineKind::Remove);
        assert_eq!(diff.lines[0].kind, DiffLineKind::Header);
    }

    #[test]
    fn reader_decodes_a_latin1_patch_instead_of_failing() {
        let raw = b"diff --git a/a.txt b/a.txt\n@@ -1 +1 @@\n-caf\xe9 cr\xe8me br\xfbl\xe9e\n+caf\xe9 cr\xe8me br\xfbl\xe9e!\n";
        let diff = Diff::from_unified_reader(target("a.txt"), Cursor::new(&raw[..])).unwrap();
        assert_eq!(diff.lines[2].text.as_ref(), "-café crème brûlée");
        assert_eq!(
            diff.lines[3].text.raw_bytes(),
            b"+caf\xe9 cr\xe8me br\xfbl\xe9e!"
        );
    }

    #[test]
    fn hunk_context_supplies_old_side_encoding_when_changed_lines_are_ascii() {
        let diff = Diff::from_unified_bytes(
            target("a.txt"),
            b"@@ -6 +6 @@ caf\xe9\n-old\n+\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\n".to_vec(),
            DiffSectionFormats::sniff,
        );
        assert_eq!(diff.lines[0].text.as_ref(), "@@ -6 +6 @@ café");
        assert_eq!(diff.lines[2].text.as_ref(), "+日本語");
    }

    #[test]
    fn sections_split_on_diff_headers() {
        let raw = b"diff --git a/a b/a\n+1\ndiff --git a/b b/b\n+2\n";
        let mut seen = Vec::new();
        let _ = Diff::from_unified_bytes(target("a"), raw.to_vec(), |section| {
            seen.push(section.to_vec());
            DiffSectionFormats::UTF_8
        });
        assert_eq!(
            seen,
            vec![
                b"diff --git a/a b/a\n+1\n".to_vec(),
                b"diff --git a/b b/b\n+2\n".to_vec()
            ]
        );
    }

    #[test]
    fn unified_reader_handles_small_buffer_chunks_without_extra_newline_bytes() {
        let target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
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
        let target = DiffTarget::working_tree(PathBuf::from("README.md"), DiffArea::Unstaged);
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
    fn unified_reader_keeps_the_limit_result_when_the_drain_fails() {
        // A killed `git diff` leaves the pipe erroring; keep what we read.
        struct FailsWhenDrained(Cursor<&'static [u8]>);
        impl std::io::Read for FailsWhenDrained {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                match self.0.read(buf)? {
                    0 => Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "child went away",
                    )),
                    read => Ok(read),
                }
            }
        }

        let (text, notice) = Diff::read_unified_text_with_limits(
            FailsWhenDrained(Cursor::new(b"aa\nbb\ncc\n")),
            5,
            100,
        )
        .expect("a failing drain must not discard the diff");
        assert_eq!(text, "aa\n");
        assert!(notice.expect("notice").contains("5-byte"));
    }

    #[test]
    fn unified_reader_drains_the_whole_reader_after_truncating() {
        // The reader is a child Git pipe. Closing it early kills Git with
        // SIGPIPE, and the non-zero exit outranks the diff we just parsed.
        let mut reader = Cursor::new(vec![b'x'; 100]);
        let (text, notice) = Diff::read_unified_text_with_limits(&mut reader, 4, 100).unwrap();

        assert_eq!(text.len(), 4);
        assert!(notice.is_some());
        assert_eq!(
            reader.position(),
            100,
            "Git must reach the end of its output and exit normally"
        );
    }

    #[test]
    fn unified_reader_truncates_instead_of_failing() {
        let (text, notice) =
            Diff::read_unified_text_with_limits(Cursor::new(b"aa\nbb\ncc\n"), 100, 2).unwrap();
        assert_eq!(text, "aa\nbb\n");
        assert!(notice.expect("notice").contains("2-line"));

        // A single line longer than the limit is cut on a UTF-8 boundary.
        let (text, notice) =
            Diff::read_unified_text_with_limits(Cursor::new("aaaä".as_bytes()), 4, 100).unwrap();
        assert_eq!(text, "aaa");
        assert!(notice.expect("notice").contains("4-byte"));
    }

    #[test]
    fn truncated_diffs_end_with_a_notice_row() {
        let target = DiffTarget::working_tree(PathBuf::from("a.txt"), DiffArea::Unstaged);
        let diff =
            Diff::from_unified_reader_with_limits(target, Cursor::new(b"aa\nbb\ncc\n"), 100, 2)
                .unwrap();
        let last = diff.lines.last().expect("notice row");
        assert_eq!(last.kind, DiffLineKind::Header);
        assert!(
            last.text
                .as_ref()
                .starts_with(Diff::TRUNCATION_NOTICE_PREFIX),
            "{:?}",
            last.text.as_ref()
        );
        assert_eq!(diff.lines.len(), 3);
    }

    #[test]
    fn unified_reader_leaves_diffs_within_the_limits_untouched() {
        for (text, max_bytes, max_lines) in [("1234", 4, 1), ("a\nb\n", 4, 2)] {
            let (read, notice) =
                Diff::read_unified_text_with_limits(Cursor::new(text), max_bytes, max_lines)
                    .unwrap();
            assert_eq!(read, text);
            assert_eq!(notice, None);
        }
    }

    #[test]
    fn paged_provider_loads_pages_on_demand() {
        let target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
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
            resume_token: Some(Arc::from("cursor-token")),
        };
        assert_eq!(cursor.last_seen.as_ref(), "deadbeef");
        assert_eq!(
            cursor.resume_from.as_ref().map(AsRef::as_ref),
            Some("feedface")
        );
        assert_eq!(cursor.resume_token.as_deref(), Some("cursor-token"));
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
        let target = DiffTarget::working_tree(PathBuf::from("a.txt"), DiffArea::Unstaged);

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
        let target = DiffTarget::working_tree(PathBuf::from("a.txt"), DiffArea::Unstaged);

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
            author: "Jane Doe".into(),
        };
        let reflog_clone = reflog.clone();
        assert!(Arc::ptr_eq(&reflog.message, &reflog_clone.message));
        assert!(Arc::ptr_eq(&reflog.selector, &reflog_clone.selector));
    }
}

#[cfg(test)]
mod file_status_count_tests {
    use super::*;

    fn status(kind: FileStatusKind) -> FileStatus {
        FileStatus {
            path: PathBuf::from("a.txt"),
            kind,
            conflict: None,
        }
    }

    #[test]
    fn counts_bucket_untracked_and_added_together() {
        let entries = [
            status(FileStatusKind::Untracked),
            status(FileStatusKind::Added),
        ];
        assert_eq!(count_file_statuses(&entries), (2, 0, 0));
    }

    #[test]
    fn counts_bucket_renamed_and_conflicted_as_modified() {
        let entries = [
            status(FileStatusKind::Modified),
            status(FileStatusKind::Renamed),
            status(FileStatusKind::Conflicted),
        ];
        assert_eq!(count_file_statuses(&entries), (0, 3, 0));
    }

    #[test]
    fn counts_are_zero_for_an_empty_status_list() {
        assert_eq!(count_file_statuses(&[]), (0, 0, 0));
    }

    #[test]
    fn a_summary_is_dirty_only_when_some_bucket_is_non_zero() {
        let clean = WorktreeDirtySummary {
            path: PathBuf::from("/tmp/wt"),
            head: None,
            branch: None,
            detached: false,
            added: 0,
            modified: 0,
            deleted: 0,
            staged: Vec::new(),
            unstaged: Vec::new(),
            line_stats: UncommittedLineStats::default(),
        };
        assert!(!clean.is_dirty());
        assert!(
            WorktreeDirtySummary {
                deleted: 1,
                ..clean
            }
            .is_dirty()
        );
    }
}
