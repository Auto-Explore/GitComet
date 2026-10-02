//! Git LFS and git-annex detection and per-row state, from git objects,
//! config, git-annex's logs and the local object stores. Never runs either
//! tool.

use gitcomet_core::annex;
use gitcomet_core::domain::FileDiffTextSource;
use gitcomet_core::domain::{FileStatus, FileStatusKind, RepoStatus};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::large_files::{
    AnnexRepoInfo, LargeFileContent, LargeFilePointer, LargeFileSide, LargeFileState,
    LargeFileSupport, LargeFileWorktree, LfsRepoInfo, LfsTrackedPattern, UncommittedLargeFiles,
};
use gitcomet_core::lfs;
use gitcomet_core::services::{CancellationToken, Result};
use gix::bstr::ByteSlice;
use std::io::Read as _;
use std::path::{Path, PathBuf};

/// Larger status snapshots are not classified: each row costs an attribute
/// lookup and a small read, and such trees are rarely all large files.
pub(super) const LARGE_FILE_STATUS_ROW_LIMIT: usize = 5_000;

/// Largest pointer either tool writes; anything bigger is content.
const MAX_POINTER_BYTES: u64 = annex::POINTER_MAX_BYTES as u64;

/// Git LFS pointers are under 1 KiB; git-annex allows up to 32 KiB.
const MAX_LFS_POINTER_BYTES: u64 = 1024;

/// Real content above this stays behind its pointer in the text diff.
pub(super) const LARGE_FILE_TEXT_DIFF_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// Cheap check on a file's first bytes before reading it whole: an LFS
/// pointer, an annex pointer, or annex link text.
fn may_be_pointer(head: &[u8]) -> bool {
    lfs::looks_like_pointer(head)
        || head.starts_with(b"/annex/objects/")
        || head
            .windows(b"annex/objects/".len())
            .any(|w| w == b"annex/objects/")
}

/// Read a file only if it could be a pointer; `None` for ordinary content.
pub(super) fn read_pointer_candidate(path: &Path) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_POINTER_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.by_ref().take(160).read_to_end(&mut bytes).ok()?;
    if !may_be_pointer(&bytes) {
        return None;
    }
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Git-form bytes: pointer text, or annex link text as git stores a symlink.
fn classify_git_form(bytes: &[u8]) -> Option<Classified> {
    classify_bytes(bytes, false).or_else(|| classify_bytes(bytes, true))
}

/// Drop the git-annex filter driver from this handle's in-memory config (never
/// the file on disk). Background reads use it: git-annex's clean filter hashes
/// whole files and writes its keys database, and `git status` does not run it
/// for status either, reporting stale annexed files as modified instead.
pub(crate) fn strip_annex_filter(repo: &mut gix::Repository) {
    let has_annex_filter = repo
        .config_snapshot()
        .plumbing()
        .sections_by_name("filter")
        .into_iter()
        .flatten()
        .any(|section| section.header().subsection_name() == Some("annex".into()));
    if !has_annex_filter {
        return;
    }
    let mut config = repo.config_snapshot_mut();
    while config
        .remove_section("filter", Some(gix::bstr::BStr::new("annex")))
        .is_some()
    {}
}

pub(crate) fn lfs_filter_configured(config: &gix::config::File) -> bool {
    ["filter.lfs.process", "filter.lfs.clean"]
        .iter()
        .any(|key| config.string(*key).is_some_and(|value| !value.is_empty()))
}

fn lfs_storage_dir(repo: &gix::Repository) -> PathBuf {
    let config = repo.config_snapshot();
    let storage = config
        .string("lfs.storage")
        .and_then(|value| gix::path::try_from_bstring(value).ok());
    lfs::storage_dir(storage.as_deref(), repo.common_dir())
}

fn truthy_env(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn backend_error(context: &str, error: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Backend(format!("{context}: {error}")))
}

/// A pointer read from git. Content identity is independent of the link's path.
struct Classified {
    pointer: LargeFilePointer,
}

fn classify_bytes(bytes: &[u8], is_symlink: bool) -> Option<Classified> {
    if is_symlink {
        let key = annex::key_from_symlink_target(bytes)?;
        return Some(Classified {
            pointer: LargeFilePointer::Annex(key),
        });
    }
    let pointer = lfs::parse_pointer(bytes)
        .map(LargeFilePointer::Lfs)
        .or_else(|| annex::key_from_pointer(bytes).map(LargeFilePointer::Annex))?;
    Some(Classified { pointer })
}

fn classify_blob(
    repo: &gix::Repository,
    id: gix::ObjectId,
    is_symlink: bool,
) -> Option<Classified> {
    classify_blob_up_to(repo, id, is_symlink, MAX_POINTER_BYTES)
}

fn classify_blob_up_to(
    repo: &gix::Repository,
    id: gix::ObjectId,
    is_symlink: bool,
    max_bytes: u64,
) -> Option<Classified> {
    if repo.find_header(id).ok()?.size() > max_bytes {
        return None;
    }
    let object = repo.find_object(id).ok()?;
    classify_bytes(&object.data, is_symlink)
}

fn index_classified(
    repo: &gix::Repository,
    index: &gix::index::State,
    path: &Path,
) -> Option<Classified> {
    let key = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(path));
    let entry = index.entry_by_path(key.as_ref())?;
    let is_symlink = entry.mode == gix::index::entry::Mode::SYMLINK;
    classify_blob(repo, entry.id, is_symlink)
}

/// Where content lives locally: the LFS store, and git-annex's objects
/// directory, whose layout is hashed from each key.
struct LocalStores {
    lfs: PathBuf,
    annex_objects: PathBuf,
    annex_levels: usize,
}

impl LocalStores {
    fn of(repo: &gix::Repository) -> Self {
        let objecthash1 = repo
            .config_snapshot()
            .boolean("annex.tune.objecthash1")
            .unwrap_or(false);
        Self {
            lfs: lfs_storage_dir(repo),
            annex_objects: repo.common_dir().join("annex").join("objects"),
            annex_levels: if objecthash1 { 1 } else { 2 },
        }
    }

    /// The content of an annex key when it is here, found the way
    /// `git annex contentlocation` finds it, without running git-annex.
    fn annex_object(&self, key: &str) -> Option<PathBuf> {
        annex::object_paths(key, self.annex_levels)
            .into_iter()
            .map(|relative| self.annex_objects.join(relative))
            .find(|path| path.is_file())
    }
}

fn has_annex_branch(repo: &gix::Repository) -> bool {
    let Ok(refs) = crate::refs::view(repo) else {
        return false;
    };
    if refs.find("refs/heads/git-annex").ok().flatten().is_some() {
        return true;
    }
    let Ok(remote_branches) = refs.remote_branches() else {
        return false;
    };
    let mut remotes = None;
    remote_branches.flatten().any(|reference| {
        let Some(prefix) = reference
            .name()
            .as_bstr()
            .strip_prefix(b"refs/remotes/")
            .and_then(|name| name.strip_suffix(b"/git-annex"))
        else {
            return false;
        };
        // gix's tracking-ref mapping: a single-level prefix is the remote.
        // Remote names can contain slashes too, so a nested prefix must be a
        // configured remote; origin/feature/git-annex is not the bookkeeping
        // branch.
        !prefix.contains(&b'/')
            || remotes
                .get_or_insert_with(|| repo.remote_names())
                .contains(prefix.as_bstr())
    })
}

pub(super) enum AnnexWorktreeSide {
    /// Unchanged since `git annex add`: the indexed key is its key.
    Indexed(gix::ObjectId),
    /// Edited and too large to read as text: the index pointer stands in for
    /// the unknown new key, under an identity of its own.
    EditedTooLarge { id: gix::ObjectId, identity: String },
}

/// Commit rows are classified only in repositories that use either tool, and
/// only blobs small enough to be a pointer of the tools in use are read.
pub(super) struct CommittedPointerScan {
    stores: LocalStores,
    lfs: bool,
    annex: bool,
}

impl CommittedPointerScan {
    fn of(repo: &gix::Repository) -> Self {
        let config = repo.config_snapshot();
        let annex = repo.common_dir().join("annex").is_dir()
            || config
                .string("annex.uuid")
                .is_some_and(|uuid| !uuid.is_empty())
            || has_annex_branch(repo);
        let stores = LocalStores::of(repo);
        // Global filter config only means LFS is installed. Use the same
        // attribute sources as the support summary, including nested ones.
        let lfs = stores.lfs.join("objects").is_dir()
            || scan_lfs_attributes(repo, &CancellationToken::new(), |pattern, _| {
                pattern.is_some()
            })
            .unwrap_or(false);
        Self { stores, lfs, annex }
    }

    fn supports(&self, pointer: &LargeFilePointer) -> bool {
        match pointer {
            LargeFilePointer::Lfs(_) => self.lfs,
            LargeFilePointer::Annex(_) => self.annex,
        }
    }

    /// Large-file state of a committed blob, for commit file rows. `None` for
    /// ordinary files; costs one object-header lookup for those.
    pub(super) fn state(
        &self,
        repo: &gix::Repository,
        id: gix::ObjectId,
        is_symlink: bool,
    ) -> Option<LargeFileState> {
        let max_bytes = if self.annex {
            MAX_POINTER_BYTES
        } else if self.lfs {
            MAX_LFS_POINTER_BYTES
        } else {
            return None;
        };
        let classified = classify_blob_up_to(repo, id, is_symlink, max_bytes)?;
        if !self.supports(&classified.pointer) {
            return None;
        }
        let in_local_store = in_local_store(&classified, &self.stores);
        Some(LargeFileState {
            pointer: classified.pointer,
            in_local_store,
            worktree: None,
            lockable: false,
        })
    }
}

/// Whether the content is here.
fn in_local_store(classified: &Classified, stores: &LocalStores) -> Option<bool> {
    match &classified.pointer {
        LargeFilePointer::Lfs(pointer) => Some(
            stores
                .lfs
                .join(lfs::object_relative_path(&pointer.oid))
                .is_file(),
        ),
        LargeFilePointer::Annex(key) => Some(stores.annex_object(&key.raw).is_some()),
    }
}

struct LockableLookup<'repo> {
    stack: Option<gix::AttributeStack<'repo>>,
    outcome: gix::attrs::search::Outcome,
}

impl LockableLookup<'_> {
    fn is_lockable(&mut self, path: &Path) -> bool {
        let Some(stack) = self.stack.as_mut() else {
            return false;
        };
        let key = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(path));
        let Ok(platform) = stack.at_entry(key.as_ref(), None) else {
            return false;
        };
        platform.matching_attributes(&mut self.outcome);
        self.outcome.iter().any(|matched| {
            matched.assignment.name.as_str() == "lockable"
                && matches!(matched.assignment.state, gix::attrs::StateRef::Set)
        })
    }
}

/// Visit `filter=lfs` patterns and independent `lockable` rules in tracked
/// `.gitattributes`, an untracked root one, and `info/attributes`. Macros are
/// not expanded here; per-file state uses the real attribute stack.
/// Returning true stops the scan, so row classification only needs the first
/// LFS pattern and keeps no list of attribute-file contents in memory.
fn scan_lfs_attributes(
    repo: &gix::Repository,
    cancellation: &CancellationToken,
    mut found: impl FnMut(Option<LfsTrackedPattern>, bool) -> bool,
) -> Result<bool> {
    let workdir = repo.workdir().unwrap_or(repo.common_dir());
    let index = repo
        .index_or_empty()
        .map_err(|e| backend_error("read index for LFS patterns", e))?;
    let sources = index.entries().iter().filter_map(|entry| {
        let path = entry.path(&index);
        (path == ".gitattributes" || path.ends_with(b"/.gitattributes"))
            .then(|| {
                gix::path::try_from_bstr(path)
                    .ok()
                    .map(|path| (path.into_owned(), Some(entry.id)))
            })
            .flatten()
    });
    // Include the untracked root and the local attribute overrides too.
    let sources = sources.chain([
        (PathBuf::from(".gitattributes"), None),
        (PathBuf::from(".git/info/attributes"), None),
    ]);
    let mut saw_root = false;
    for (source, id) in sources {
        cancellation.check_cancelled()?;
        if source == Path::new(".gitattributes") {
            if saw_root {
                continue;
            }
            saw_root = true;
        }
        let full = if source == Path::new(".git/info/attributes") {
            repo.common_dir().join("info/attributes")
        } else {
            workdir.join(&source)
        };
        let bytes = std::fs::read(full)
            .ok()
            .or_else(|| repo.find_object(id?).ok().map(|object| object.data.clone()));
        let Some(bytes) = bytes else {
            continue;
        };
        for (kind, assignments, _line) in gix::attrs::parse(&bytes).flatten() {
            let gix::attrs::parse::Kind::Pattern(pattern) = kind else {
                continue;
            };
            let (mut lfs, mut lockable) = (false, false);
            for assignment in assignments.flatten() {
                match (assignment.name.as_str(), assignment.state) {
                    ("filter", gix::attrs::StateRef::Value(value)) => {
                        lfs = value.as_bstr() == "lfs";
                    }
                    ("filter", _) => lfs = false,
                    ("lockable", state) => lockable = matches!(state, gix::attrs::StateRef::Set),
                    _ => {}
                }
            }
            if (lfs || lockable)
                && found(
                    lfs.then(|| LfsTrackedPattern {
                        pattern: pattern.to_string(),
                        source: source.clone(),
                    }),
                    lockable,
                )
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

impl super::GixRepo {
    /// Share the ordinary config cache and object store with other readers.
    /// Support metadata must not retain a separately opened repository.
    pub(super) fn large_file_read_repo(&self) -> gix::Repository {
        self.repo_with_current_config()
            .unwrap_or_else(|_| self.repo())
    }

    pub(super) fn committed_pointer_scan(
        &self,
        repo: &gix::Repository,
    ) -> std::sync::Arc<CommittedPointerScan> {
        self.large_file_scan
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(|| std::sync::Arc::new(CommittedPointerScan::of(repo)))
            .clone()
    }

    /// Describe an unlocked annexed file's worktree side through its index
    /// entry, without hashing the worktree through git-annex. `None` reads
    /// the file normally: not unlocked annex content, or a small edit.
    pub(super) fn annex_worktree_side(
        &self,
        repo: &gix::Repository,
        path: &Path,
    ) -> Option<AnnexWorktreeSide> {
        if !self.committed_pointer_scan(repo).annex {
            return None;
        }
        let full = self.spec.workdir.join(path);
        let metadata = std::fs::symlink_metadata(&full).ok()?;
        if !metadata.is_file()
            || read_pointer_candidate(&full)
                .and_then(|bytes| classify_git_form(&bytes))
                .is_some()
        {
            return None;
        }
        let index = repo.index_or_empty().ok()?;
        let key = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(path));
        let entry = index.entry_by_path(key.as_ref())?;
        // A locked (symlink) entry replaced by a file is ordinary content.
        if entry.mode == gix::index::entry::Mode::SYMLINK
            || !matches!(
                classify_blob(repo, entry.id, false)?.pointer,
                LargeFilePointer::Annex(_)
            )
        {
            return None;
        }
        // The key describes the file only while it is as `git annex add` left it.
        let options = repo.stat_options().ok()?;
        let unchanged = gix::index::fs::Metadata::from_path_no_follow(&full)
            .ok()
            .and_then(|metadata| gix::index::entry::Stat::from_fs(&metadata).ok())
            .is_some_and(|stat| {
                entry.stat.matches(&stat, options)
                    && !entry.stat.is_racy(index.timestamp(), options)
            });
        if unchanged {
            Some(AnnexWorktreeSide::Indexed(entry.id))
        } else if metadata.len() > LARGE_FILE_TEXT_DIFF_MAX_BYTES {
            Some(AnnexWorktreeSide::EditedTooLarge {
                id: entry.id,
                identity: format!(
                    "annex-worktree:{}:{}:{:?}",
                    path.display(),
                    metadata.len(),
                    metadata.modified().ok()
                ),
            })
        } else {
            None
        }
    }

    /// Describe one side of a text diff whose git form is a pointer, and point
    /// it at the real content when that is here and small enough to diff.
    /// `worktree` sides may find content in the working tree itself.
    pub(super) fn large_file_side(
        &self,
        repo: &gix::Repository,
        source: &FileDiffTextSource,
        logical_path: &Path,
        worktree: bool,
    ) -> Option<(LargeFileSide, Option<FileDiffTextSource>)> {
        let scan = self.committed_pointer_scan(repo);
        if !scan.lfs && !scan.annex {
            return None;
        }
        let classified = classify_git_form(&read_pointer_candidate(&source.path)?)?;
        if !scan.supports(&classified.pointer) {
            return None;
        }
        let full = self.spec.workdir.join(logical_path);
        let worktree_content = worktree
            .then(|| match std::fs::symlink_metadata(&full) {
                Ok(meta) if meta.file_type().is_symlink() => std::fs::metadata(&full)
                    .is_ok_and(|m| m.is_file())
                    .then(|| full.clone()),
                // A pointer in the worktree is not content.
                Ok(meta) if meta.is_file() => read_pointer_candidate(&full)
                    .and_then(|bytes| classify_bytes(&bytes, false))
                    .is_none()
                    .then(|| full.clone()),
                _ => None,
            })
            .flatten();
        let (content_path, identity) = match worktree_content {
            Some(path) => (Some(path), None),
            None => match &classified.pointer {
                LargeFilePointer::Lfs(pointer) => {
                    let path = scan
                        .stores
                        .lfs
                        .join(lfs::object_relative_path(&pointer.oid));
                    (
                        path.is_file().then_some(path),
                        Some(format!("lfs:{}", pointer.oid)),
                    )
                }
                LargeFilePointer::Annex(key) => (
                    scan.stores.annex_object(&key.raw),
                    Some(format!("annex:{}", key.raw)),
                ),
            },
        };
        let content = match &content_path {
            Some(path) => {
                let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                if bytes > LARGE_FILE_TEXT_DIFF_MAX_BYTES {
                    LargeFileContent::TooLarge { bytes }
                } else {
                    LargeFileContent::Available
                }
            }
            None => LargeFileContent::MissingLocally,
        };
        let replacement = (content == LargeFileContent::Available)
            .then_some(content_path)
            .flatten()
            .map(|path| match identity {
                Some(identity) => FileDiffTextSource::with_identity(path, identity),
                None => FileDiffTextSource::new(path),
            });
        Some((
            LargeFileSide {
                pointer: classified.pointer,
                content,
            },
            replacement,
        ))
    }

    /// Describe managed images even when their bytes cannot be loaded. Pointer
    /// text must never reach the image decoder.
    pub(super) fn large_file_image_side(
        &self,
        repo: &gix::Repository,
        git_form: &[u8],
        max_bytes: u64,
    ) -> Option<(LargeFileSide, Option<Vec<u8>>)> {
        if git_form.len() as u64 > MAX_POINTER_BYTES || !may_be_pointer(git_form) {
            return None;
        }
        let classified = classify_git_form(git_form)?;
        let scan = self.committed_pointer_scan(repo);
        if !scan.supports(&classified.pointer) {
            return None;
        }
        let path = match &classified.pointer {
            LargeFilePointer::Lfs(pointer) => scan
                .stores
                .lfs
                .join(lfs::object_relative_path(&pointer.oid)),
            LargeFilePointer::Annex(key) => match scan.stores.annex_object(&key.raw) {
                Some(path) => path,
                None => {
                    return Some((
                        LargeFileSide {
                            pointer: classified.pointer,
                            content: LargeFileContent::MissingLocally,
                        },
                        None,
                    ));
                }
            },
        };
        let (content, bytes) = match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > max_bytes => {
                (LargeFileContent::TooLarge { bytes: meta.len() }, None)
            }
            Ok(meta) if meta.is_file() => match std::fs::read(path) {
                Ok(bytes) if bytes.len() as u64 <= max_bytes => {
                    (LargeFileContent::Available, Some(bytes))
                }
                Ok(bytes) => (
                    LargeFileContent::TooLarge {
                        bytes: bytes.len() as u64,
                    },
                    None,
                ),
                Err(_) => (LargeFileContent::MissingLocally, None),
            },
            _ => (LargeFileContent::MissingLocally, None),
        };
        Some((
            LargeFileSide {
                pointer: classified.pointer,
                content,
            },
            bytes,
        ))
    }

    pub(super) fn large_file_support_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<LargeFileSupport> {
        cancellation.check_cancelled()?;
        // Commands and external tools can change configuration on the same
        // open handle (annex init/enableremote, LFS install, custom storage).
        let repo = self.repo_with_current_config()?;
        let config = repo.config_snapshot();
        let storage_dir = lfs_storage_dir(&repo);
        let skip_flag = |key: &str| {
            config
                .string(key)
                .is_some_and(|value| value.contains_str("--skip"))
        };
        cancellation.check_cancelled()?;
        let mut lfs = LfsRepoInfo {
            filter_configured: lfs_filter_configured(&config),
            filter_required: config.boolean("filter.lfs.required").unwrap_or(false),
            has_local_store: storage_dir.join("objects").is_dir(),
            storage_dir,
            skip_smudge: truthy_env("GIT_LFS_SKIP_SMUDGE")
                || skip_flag("filter.lfs.smudge")
                || skip_flag("filter.lfs.process"),
            locks_verify: config.boolean("lfs.locksverify"),
            ..LfsRepoInfo::default()
        };
        scan_lfs_attributes(&repo, cancellation, |pattern, lockable| {
            lfs.tracked_patterns.extend(pattern);
            lfs.has_lockable_patterns |= lockable;
            false
        })?;

        let annex = AnnexRepoInfo {
            has_annex_dir: repo.common_dir().join("annex").is_dir(),
            has_annex_branch: has_annex_branch(&repo),
            uuid: config
                .string("annex.uuid")
                .map(|value| value.to_str_lossy().into_owned())
                .filter(|value| !value.is_empty()),
            crippled_filesystem: config.boolean("annex.crippledfilesystem").unwrap_or(false),
            restage_pending: std::fs::metadata(repo.common_dir().join("annex").join("restage.log"))
                .is_ok_and(|metadata| metadata.len() > 0),
            assistant_running: super::annex::assistant_running(&repo.common_dir().join("annex")),
            repositories: Vec::new(),
            numcopies: None,
        };
        let mut annex = annex;
        if annex.initialized() {
            cancellation.check_cancelled()?;
            (annex.repositories, annex.numcopies) = self.annex_repositories(&repo);
        }
        *self
            .large_file_scan
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(std::sync::Arc::new(CommittedPointerScan {
            stores: LocalStores::of(&repo),
            lfs: lfs.in_use(),
            annex: annex.in_use(),
        }));
        Ok(LargeFileSupport { lfs, annex })
    }

    pub(super) fn uncommitted_large_files_impl(
        &self,
        status: &RepoStatus,
        cancellation: &CancellationToken,
    ) -> Result<UncommittedLargeFiles> {
        let mut result = UncommittedLargeFiles::default();
        if status.staged.len() + status.unstaged.len() > LARGE_FILE_STATUS_ROW_LIMIT {
            return Ok(result);
        }
        let repo = self.large_file_read_repo();
        let scan = self.committed_pointer_scan(&repo);
        if !scan.lfs && !scan.annex {
            return Ok(result);
        }
        let index = repo
            .index_or_empty()
            .map_err(|e| backend_error("read index for large files", e))?;
        let mut lockable = LockableLookup {
            stack: repo
                .attributes_only(
                    &index,
                    gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
                )
                .ok(),
            outcome: gix::attrs::search::Outcome::default(),
        };
        let finish = |classified: Classified,
                      worktree: Option<LargeFileWorktree>,
                      path: &Path,
                      lockable: &mut LockableLookup<'_>| {
            let in_store = in_local_store(&classified, &scan.stores);
            let is_lfs = classified.pointer.is_lfs();
            LargeFileState {
                pointer: classified.pointer,
                in_local_store: in_store,
                worktree,
                lockable: is_lfs && lockable.is_lockable(path),
            }
        };

        for entry in status.staged.iter() {
            cancellation.check_cancelled()?;
            if entry.kind == FileStatusKind::Deleted {
                continue;
            }
            if let Some(classified) = index_classified(&repo, &index, &entry.path)
                && scan.supports(&classified.pointer)
            {
                let state = finish(classified, None, &entry.path, &mut lockable);
                result.staged.insert(entry.path.clone(), state);
            }
        }
        for entry in status.unstaged.iter() {
            cancellation.check_cancelled()?;
            if let Some((classified, worktree)) = self.classify_unstaged_row(&repo, &index, entry)
                && scan.supports(&classified.pointer)
            {
                let state = finish(classified, Some(worktree), &entry.path, &mut lockable);
                result.unstaged.insert(entry.path.clone(), state);
            }
        }
        Ok(result)
    }

    /// Worktree side of an unstaged row: a pointer or dangling annex link means
    /// content is not checked out; real content keeps the index's pointer.
    fn classify_unstaged_row(
        &self,
        repo: &gix::Repository,
        index: &gix::index::State,
        entry: &FileStatus,
    ) -> Option<(Classified, LargeFileWorktree)> {
        if entry.kind == FileStatusKind::Untracked {
            return None;
        }
        let full = self.spec.workdir.join(&entry.path);
        let Ok(metadata) = std::fs::symlink_metadata(&full) else {
            return index_classified(repo, index, &entry.path)
                .map(|classified| (classified, LargeFileWorktree::Missing));
        };
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&full).ok()?;
            let target = gix::path::into_bstr(target).into_owned();
            let classified = classify_bytes(target.as_ref(), true)?;
            let present = std::fs::metadata(&full).is_ok_and(|m| m.is_file());
            let worktree = if present {
                LargeFileWorktree::Content
            } else {
                LargeFileWorktree::Pointer
            };
            return Some((classified, worktree));
        }
        // A plain file can hold link text too (`core.symlinks=false`).
        if metadata.is_file()
            && metadata.len() <= MAX_POINTER_BYTES
            && let Some(bytes) = read_pointer_candidate(&full)
            && let Some(classified) = classify_git_form(&bytes)
        {
            return Some((classified, LargeFileWorktree::Pointer));
        }
        index_classified(repo, index, &entry.path)
            .map(|classified| (classified, LargeFileWorktree::Content))
    }
}
