//! Repository and per-file state for large-file extensions (Git LFS and
//! git-annex). Computed from git objects, config and the local object stores
//! without running either tool.

use crate::annex::AnnexKey;
use crate::lfs::LfsPointer;
use rustc_hash::FxHashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum LargeFilePointer {
    Lfs(LfsPointer),
    Annex(AnnexKey),
}

impl LargeFilePointer {
    /// Recorded content size, when the pointer carries one.
    pub fn size(&self) -> Option<u64> {
        match self {
            Self::Lfs(pointer) => Some(pointer.size),
            Self::Annex(key) => key.size,
        }
    }

    pub fn is_lfs(&self) -> bool {
        matches!(self, Self::Lfs(_))
    }
}

/// What the working tree holds at a large-file path.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LargeFileWorktree {
    /// Pointer text or a dangling annex link: content not checked out.
    Pointer,
    Content,
    Missing,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LargeFileState {
    pub pointer: LargeFilePointer,
    /// Content exists in the local store; `None` when it cannot be told
    /// without running the tool (annex unlocked files).
    pub in_local_store: Option<bool>,
    /// `None` for rows that do not describe the working tree (commits, index).
    pub worktree: Option<LargeFileWorktree>,
    /// Matches an LFS `lockable` attribute (read-only until locked).
    pub lockable: bool,
}

impl LargeFileState {
    /// The row is managed but its content is known to be absent here.
    pub fn content_missing(&self) -> bool {
        self.in_local_store == Some(false) && self.worktree != Some(LargeFileWorktree::Content)
    }
}

/// Per-path state for status rows, keyed like `UncommittedLineStats`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UncommittedLargeFiles {
    pub staged: FxHashMap<PathBuf, LargeFileState>,
    pub unstaged: FxHashMap<PathBuf, LargeFileState>,
}

impl UncommittedLargeFiles {
    pub fn is_empty(&self) -> bool {
        self.staged.is_empty() && self.unstaged.is_empty()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LfsTrackedPattern {
    pub pattern: String,
    pub lockable: bool,
    /// `.gitattributes` file the line came from, relative to the workdir.
    pub source: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LfsRepoInfo {
    /// `filter.lfs.process` or `filter.lfs.clean` is configured.
    pub filter_configured: bool,
    pub filter_required: bool,
    pub tracked_patterns: Vec<LfsTrackedPattern>,
    pub storage_dir: PathBuf,
    pub has_local_store: bool,
    /// Smudge is skipped (`filter.lfs.smudge` runs `--skip`, or the env var).
    pub skip_smudge: bool,
    pub locks_verify: Option<bool>,
}

impl LfsRepoInfo {
    /// The repository uses LFS: patterns are tracked or objects are stored.
    pub fn in_use(&self) -> bool {
        !self.tracked_patterns.is_empty() || self.has_local_store
    }

    pub fn has_lockable_patterns(&self) -> bool {
        self.tracked_patterns.iter().any(|pattern| pattern.lockable)
    }
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct AnnexRepoInfo {
    /// `.git/annex` exists.
    pub has_annex_dir: bool,
    /// A local `git-annex` branch exists (e.g. a fresh clone of an annex repo).
    pub has_annex_branch: bool,
    /// `annex.uuid`: this clone has run `git annex init`.
    pub uuid: Option<String>,
    pub crippled_filesystem: bool,
    /// `.git/annex/restage.log` lists files whose content git-annex replaced
    /// without refreshing Git's index (an interrupted or failed command). They
    /// read as modified until `git annex restage` runs.
    pub restage_pending: bool,
    /// The git-annex assistant (started by the webapp) is running here.
    pub assistant_running: bool,
    /// Known repositories and special remotes, from `git annex info`; empty
    /// when git-annex is not installed or the clone is not initialized.
    pub repositories: Vec<AnnexRepository>,
    /// Desired number of copies, when set.
    pub numcopies: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AnnexTrust {
    Trusted,
    Semitrusted,
    Untrusted,
}

impl AnnexTrust {
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Trusted => "trust",
            Self::Semitrusted => "semitrust",
            Self::Untrusted => "untrust",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Semitrusted => "semitrusted",
            Self::Untrusted => "untrusted",
        }
    }
}

/// A repository git-annex knows about: this clone, another clone reachable as
/// a git remote, or a special remote (directory, S3, rsync, ...).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AnnexRepository {
    pub uuid: String,
    pub description: String,
    /// Local git remote name, when this clone can reach it directly.
    pub remote_name: Option<String>,
    /// Special remote type (`directory`, `S3`, ...); `None` for git repositories.
    pub special_type: Option<String>,
    /// A special remote's name from `remote.log`: what `enableremote` takes.
    pub special_name: Option<String>,
    pub trust: AnnexTrust,
    pub here: bool,
}

impl AnnexRepository {
    /// Built-in pseudo-remotes git-annex always lists (web, bittorrent).
    pub fn is_builtin(&self) -> bool {
        self.uuid.starts_with("00000000-0000-0000-0000-00000000000")
    }

    /// Name for menus: the remote name, else the description.
    pub fn display_name(&self) -> &str {
        self.remote_name.as_deref().unwrap_or(&self.description)
    }
}

/// Where an annexed file's content is, from `git annex whereis`. It reflects
/// the location log, not a fresh check of each remote.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct AnnexWhereis {
    pub key: String,
    pub copies: Vec<AnnexLocation>,
    pub untrusted: Vec<AnnexLocation>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AnnexLocation {
    pub uuid: String,
    pub description: String,
    pub here: bool,
}

/// Local content no file refers to any more, from `git annex unused`.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct AnnexUnused {
    pub entries: Vec<AnnexUnusedEntry>,
}

impl AnnexUnused {
    /// Sum of the sizes the keys record; keys without one are left out.
    pub fn known_bytes(&self) -> u64 {
        self.entries
            .iter()
            .filter_map(|entry| crate::annex::parse_key(&entry.key)?.size)
            .sum()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AnnexUnusedEntry {
    pub key: String,
    pub kind: AnnexUnusedKind,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AnnexUnusedKind {
    /// Old versions no branch or tag uses.
    Unused,
    /// Content `fsck` found corrupt and moved aside.
    Bad,
    /// Partial downloads.
    Temporary,
}

/// Adjusted-branch modes GitComet offers (`git annex adjust --<mode>`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AnnexAdjustMode {
    Unlock,
    Lock,
    HideMissing,
}

impl AnnexAdjustMode {
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Unlock => "--unlock",
            Self::Lock => "--lock",
            Self::HideMissing => "--hide-missing",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unlock => "unlocked",
            Self::Lock => "locked",
            Self::HideMissing => "hide missing",
        }
    }
}

impl AnnexRepoInfo {
    pub fn initialized(&self) -> bool {
        self.uuid.is_some()
    }

    pub fn in_use(&self) -> bool {
        self.has_annex_dir || self.has_annex_branch || self.uuid.is_some()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LargeFileSupport {
    pub lfs: LfsRepoInfo,
    pub annex: AnnexRepoInfo,
}

impl LargeFileSupport {
    /// Per-file state is worth computing for this repository.
    pub fn is_active(&self) -> bool {
        self.lfs.in_use() || self.annex.in_use()
    }
}

/// Whether the real content behind one side of a diff can be shown.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LargeFileContent {
    /// Present here; the diff side reads the real content.
    Available,
    /// Not in the local store or working tree; only the pointer is known.
    MissingLocally,
    /// Only the tool can tell (git-annex unlocked content).
    Unknown,
    /// Present, but above the text-diff size cap; the side keeps the pointer.
    TooLarge { bytes: u64 },
}

/// One side of a diff whose git form is a Git LFS pointer or git-annex key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LargeFileSide {
    pub pointer: LargeFilePointer,
    pub content: LargeFileContent,
}

impl LargeFileSide {
    pub fn is_available(&self) -> bool {
        self.content == LargeFileContent::Available
    }
}

/// The real diff is worth drawing only when every managed side has content.
pub fn large_file_sides_show_content(
    old: Option<&LargeFileSide>,
    new: Option<&LargeFileSide>,
) -> bool {
    old.is_none_or(LargeFileSide::is_available) && new.is_none_or(LargeFileSide::is_available)
}

/// A Git LFS or git-annex operation run through the tool itself. One command
/// kind carries them all, so each new operation is one variant here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LargeFileCommand {
    /// Download and check out the content of these paths.
    LfsPull {
        paths: Vec<PathBuf>,
    },
    /// Fetch the versions displayed by a diff, without checking out files.
    LfsFetchForDiff {
        target: crate::domain::DiffTarget,
    },
    /// Download objects for every ref without touching the worktree.
    LfsFetchAll,
    /// Upload every local object to the remote.
    LfsPushAll {
        remote: String,
    },
    /// Delete old local objects that are no longer needed.
    LfsPrune,
    /// Check the local objects for consistency.
    LfsFsck,
    /// Install the LFS filters and hooks for this repository only.
    LfsInstall,
    LfsLock {
        paths: Vec<PathBuf>,
    },
    LfsUnlock {
        paths: Vec<PathBuf>,
        force: bool,
    },
    /// Track patterns in `.gitattributes`, then re-add files so they become pointers.
    LfsTrack {
        patterns: Vec<String>,
        /// Treat each argument as a literal filename rather than a glob.
        filename: bool,
        lockable: bool,
        renormalize: Vec<PathBuf>,
    },
    /// Get content from whichever repository has it, or from `from`.
    AnnexGet {
        paths: Vec<PathBuf>,
        from: Option<String>,
    },
    /// Get content by key: a diff's historical versions, which the current
    /// file path no longer names.
    AnnexGetKeys {
        keys: Vec<String>,
    },
    /// Drop local content (or content on `from`). git-annex refuses when the
    /// required copies cannot be verified elsewhere, unless `force`.
    AnnexDrop {
        paths: Vec<PathBuf>,
        from: Option<String>,
        force: bool,
    },
    AnnexCopy {
        paths: Vec<PathBuf>,
        to: String,
    },
    AnnexMove {
        paths: Vec<PathBuf>,
        to: String,
    },
    /// Make files editable (pointer files) or read-only (symlinks) again.
    AnnexUnlock {
        paths: Vec<PathBuf>,
    },
    AnnexLock {
        paths: Vec<PathBuf>,
    },
    /// Add files to the annex rather than to git.
    AnnexAdd {
        paths: Vec<PathBuf>,
    },
    /// Fetch and merge from remotes, including the `git-annex` branch.
    AnnexPull {
        content: bool,
    },
    AnnexPush {
        content: bool,
    },
    /// Pull then push. Never commits: GitComet owns commits.
    AnnexSync {
        content: bool,
    },
    AnnexInit,
    AnnexAdjust {
        mode: AnnexAdjustMode,
    },
    /// Check out the base branch of the current adjusted branch.
    AnnexLeaveAdjusted {
        base: String,
    },
    /// Enable another clone's special remote here. Some types need local
    /// settings again (`directory=` for a directory remote).
    AnnexEnableRemote {
        name: String,
        params: Vec<String>,
    },
    /// Create a special remote: `params` are `key=value` pairs after the type.
    AnnexInitRemote {
        name: String,
        special_type: String,
        params: Vec<String>,
    },
    AnnexTrust {
        repository: String,
        trust: AnnexTrust,
    },
    AnnexDescribe {
        repository: String,
        description: String,
    },
    AnnexNumcopies {
        copies: u32,
    },
    AnnexFsck,
    /// Refresh Git's index for files git-annex changed but could not record.
    AnnexRestage,
    /// Drop everything `git annex unused` currently lists. git-annex refuses
    /// keys without enough verified copies elsewhere, unless `force`.
    AnnexDropUnused {
        force: bool,
    },
    /// Start git-annex's own web interface (and its assistant) in the background.
    AnnexWebapp,
    AnnexStopAssistant,
}

impl LargeFileCommand {
    /// Short name for activity rows and the command log.
    pub fn label(&self) -> &'static str {
        match self {
            Self::LfsPull { .. } | Self::LfsFetchForDiff { .. } => "LFS download",
            Self::LfsFetchAll => "LFS fetch",
            Self::LfsPushAll { .. } => "LFS push",
            Self::LfsPrune => "LFS prune",
            Self::LfsFsck => "LFS check",
            Self::LfsInstall => "Enable Git LFS",
            Self::LfsLock { .. } => "LFS lock",
            Self::LfsUnlock { .. } => "LFS unlock",
            Self::LfsTrack { .. } => "Track in LFS",
            Self::AnnexGet { .. } | Self::AnnexGetKeys { .. } => "annex get",
            Self::AnnexDrop { .. } => "annex drop",
            Self::AnnexCopy { .. } => "annex copy",
            Self::AnnexMove { .. } => "annex move",
            Self::AnnexUnlock { .. } => "annex unlock",
            Self::AnnexLock { .. } => "annex lock",
            Self::AnnexAdd { .. } => "annex add",
            Self::AnnexPull { .. } => "annex pull",
            Self::AnnexPush { .. } => "annex push",
            Self::AnnexSync { .. } => "annex sync",
            Self::AnnexInit => "Initialize git-annex",
            Self::AnnexAdjust { .. } => "annex adjust",
            Self::AnnexLeaveAdjusted { .. } => "Leave adjusted branch",
            Self::AnnexEnableRemote { .. } => "Enable special remote",
            Self::AnnexInitRemote { .. } => "Add special remote",
            Self::AnnexTrust { .. } => "annex trust",
            Self::AnnexDescribe { .. } => "annex describe",
            Self::AnnexNumcopies { .. } => "annex numcopies",
            Self::AnnexFsck => "annex check",
            Self::AnnexRestage => "Refresh annexed files",
            Self::AnnexDropUnused { .. } => "annex drop unused",
            Self::AnnexWebapp => "Open git-annex webapp",
            Self::AnnexStopAssistant => "Stop git-annex assistant",
        }
    }

    pub fn is_annex(&self) -> bool {
        !matches!(
            self,
            Self::LfsPull { .. }
                | Self::LfsFetchForDiff { .. }
                | Self::LfsFetchAll
                | Self::LfsPushAll { .. }
                | Self::LfsPrune
                | Self::LfsFsck
                | Self::LfsInstall
                | Self::LfsLock { .. }
                | Self::LfsUnlock { .. }
                | Self::LfsTrack { .. }
        )
    }

    /// Content was added to or removed from a store, so presence shown for
    /// the selected diff and rows is stale.
    pub fn changes_object_store(&self) -> bool {
        matches!(
            self,
            Self::LfsPull { .. }
                | Self::LfsFetchForDiff { .. }
                | Self::LfsFetchAll
                | Self::LfsPrune
                | Self::LfsFsck
                | Self::AnnexGet { .. }
                | Self::AnnexGetKeys { .. }
                | Self::AnnexDrop { .. }
                | Self::AnnexCopy { .. }
                | Self::AnnexMove { .. }
                | Self::AnnexUnlock { .. }
                | Self::AnnexLock { .. }
                | Self::AnnexPull { .. }
                | Self::AnnexPush { .. }
                | Self::AnnexSync { .. }
                | Self::AnnexFsck
                | Self::AnnexRestage
                | Self::AnnexDropUnused { .. }
        )
    }

    /// git-annex restages the index at the end of these; a failed or cancelled
    /// run skips that and leaves files reading as modified.
    pub fn restages_after(&self) -> bool {
        matches!(
            self,
            Self::AnnexGet { .. }
                | Self::AnnexGetKeys { .. }
                | Self::AnnexDrop { .. }
                | Self::AnnexCopy { .. }
                | Self::AnnexMove { .. }
                | Self::AnnexUnlock { .. }
                | Self::AnnexLock { .. }
                | Self::AnnexAdd { .. }
                | Self::AnnexPull { .. }
                | Self::AnnexPush { .. }
                | Self::AnnexSync { .. }
        )
    }

    /// Talks to a server, so it may need credentials.
    pub fn uses_network(&self) -> bool {
        matches!(
            self,
            Self::LfsPull { .. }
                | Self::LfsFetchForDiff { .. }
                | Self::LfsFetchAll
                | Self::LfsPushAll { .. }
                | Self::LfsLock { .. }
                | Self::LfsUnlock { .. }
                | Self::AnnexGet { .. }
                | Self::AnnexGetKeys { .. }
                | Self::AnnexDrop { .. }
                | Self::AnnexCopy { .. }
                | Self::AnnexMove { .. }
                | Self::AnnexPull { .. }
                | Self::AnnexPush { .. }
                | Self::AnnexSync { .. }
                | Self::AnnexEnableRemote { .. }
                | Self::AnnexInitRemote { .. }
        )
    }

    /// Rewrites files in the checkout: content replaces pointers or the
    /// reverse, a branch is checked out or merged, `.gitattributes` or file
    /// permissions change.
    pub fn writes_worktree(&self) -> bool {
        match self {
            Self::LfsPull { .. }
            | Self::LfsLock { .. }
            | Self::LfsUnlock { .. }
            | Self::LfsTrack { .. }
            | Self::AnnexGet { .. }
            | Self::AnnexGetKeys { .. }
            | Self::AnnexDrop { .. }
            | Self::AnnexMove { .. }
            | Self::AnnexUnlock { .. }
            | Self::AnnexLock { .. }
            | Self::AnnexAdd { .. }
            | Self::AnnexPull { .. }
            | Self::AnnexPush { .. }
            | Self::AnnexSync { .. }
            | Self::AnnexAdjust { .. }
            | Self::AnnexLeaveAdjusted { .. }
            | Self::AnnexFsck => true,
            Self::LfsFetchForDiff { .. }
            | Self::LfsFetchAll
            | Self::LfsPushAll { .. }
            | Self::LfsPrune
            | Self::LfsFsck
            | Self::LfsInstall
            | Self::AnnexCopy { .. }
            | Self::AnnexInit
            | Self::AnnexEnableRemote { .. }
            | Self::AnnexInitRemote { .. }
            | Self::AnnexTrust { .. }
            | Self::AnnexDescribe { .. }
            | Self::AnnexNumcopies { .. }
            | Self::AnnexRestage
            | Self::AnnexDropUnused { .. }
            | Self::AnnexWebapp
            | Self::AnnexStopAssistant => false,
        }
    }

    /// Changes lock state, so the lock list must be reloaded afterwards.
    pub fn changes_locks(&self) -> bool {
        matches!(self, Self::LfsLock { .. } | Self::LfsUnlock { .. })
    }
}

/// A Git LFS lock as reported by `git lfs locks --json`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LfsLock {
    pub id: String,
    pub path: PathBuf,
    pub owner: Option<String>,
    pub locked_at: Option<String>,
}

/// Escape a repository path for git-lfs `--include`, which takes
/// comma-separated gitignore-style patterns. A comma cannot be escaped.
pub fn lfs_include_pattern(path: &std::path::Path) -> Option<String> {
    let text = path.to_str()?;
    #[cfg(windows)]
    let text = text.replace('\\', "/");
    if text.contains(',') || text.is_empty() {
        return None;
    }
    let mut escaped = String::with_capacity(text.len() + 1);
    // Anchor at the root so `a.bin` does not also fetch `dir/a.bin`.
    escaped.push('/');
    for ch in text.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '\\' | '!' | '#') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    Some(escaped)
}

#[cfg(test)]
mod command_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn include_patterns_anchor_and_escape_paths() {
        assert_eq!(
            lfs_include_pattern(Path::new("a.bin")).as_deref(),
            Some("/a.bin")
        );
        assert_eq!(
            lfs_include_pattern(Path::new("art/[v2] *.psd")).as_deref(),
            Some("/art/\\[v2\\] \\*.psd")
        );
        assert_eq!(lfs_include_pattern(Path::new("a,b.bin")), None);
        #[cfg(unix)]
        assert_eq!(
            lfs_include_pattern(Path::new(r"a\b.bin")).as_deref(),
            Some(r"/a\\b.bin")
        );
    }

    #[test]
    fn network_and_lock_classification() {
        assert!(LargeFileCommand::LfsFetchAll.uses_network());
        assert!(!LargeFileCommand::LfsPrune.uses_network());
        let lock = LargeFileCommand::LfsLock { paths: vec![] };
        assert!(lock.uses_network() && lock.changes_locks());
    }
}
