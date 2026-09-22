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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnnexRepoInfo {
    /// `.git/annex` exists.
    pub has_annex_dir: bool,
    /// A local `git-annex` branch exists (e.g. a fresh clone of an annex repo).
    pub has_annex_branch: bool,
    /// `annex.uuid`: this clone has run `git annex init`.
    pub uuid: Option<String>,
    /// HEAD is an adjusted branch: `(base branch, mode)`.
    pub adjusted: Option<(String, String)>,
    pub crippled_filesystem: bool,
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
        lockable: bool,
        renormalize: Vec<PathBuf>,
    },
}

impl LargeFileCommand {
    /// Short name for activity rows and the command log.
    pub fn label(&self) -> &'static str {
        match self {
            Self::LfsPull { .. } => "LFS download",
            Self::LfsFetchAll => "LFS fetch",
            Self::LfsPushAll { .. } => "LFS push",
            Self::LfsPrune => "LFS prune",
            Self::LfsFsck => "LFS check",
            Self::LfsInstall => "Enable Git LFS",
            Self::LfsLock { .. } => "LFS lock",
            Self::LfsUnlock { .. } => "LFS unlock",
            Self::LfsTrack { .. } => "Track in LFS",
        }
    }

    /// Talks to a server, so it may need credentials.
    pub fn uses_network(&self) -> bool {
        matches!(
            self,
            Self::LfsPull { .. }
                | Self::LfsFetchAll
                | Self::LfsPushAll { .. }
                | Self::LfsLock { .. }
                | Self::LfsUnlock { .. }
        )
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
    let text = path.to_str()?.replace('\\', "/");
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
    }

    #[test]
    fn network_and_lock_classification() {
        assert!(LargeFileCommand::LfsFetchAll.uses_network());
        assert!(!LargeFileCommand::LfsPrune.uses_network());
        let lock = LargeFileCommand::LfsLock { paths: vec![] };
        assert!(lock.uses_network() && lock.changes_locks());
    }
}
