//! Git LFS and git-annex detection and per-row state, from git objects,
//! config and the local object stores. Never runs either tool.

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
fn read_pointer_candidate(path: &Path) -> Option<Vec<u8>> {
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

/// A pointer read from git, plus the link text when it came from a symlink.
struct Classified {
    pointer: LargeFilePointer,
    link_target: Option<Vec<u8>>,
}

fn classify_bytes(bytes: &[u8], is_symlink: bool) -> Option<Classified> {
    if is_symlink {
        let key = annex::key_from_symlink_target(bytes)?;
        return Some(Classified {
            pointer: LargeFilePointer::Annex(key),
            link_target: Some(bytes.to_vec()),
        });
    }
    let pointer = lfs::parse_pointer(bytes)
        .map(LargeFilePointer::Lfs)
        .or_else(|| annex::key_from_pointer(bytes).map(LargeFilePointer::Annex))?;
    Some(Classified {
        pointer,
        link_target: None,
    })
}

fn classify_blob(
    repo: &gix::Repository,
    id: gix::ObjectId,
    is_symlink: bool,
) -> Option<Classified> {
    if repo.find_header(id).ok()?.size() > MAX_POINTER_BYTES {
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

/// Large-file state of a committed blob, for commit file rows. `None` for
/// ordinary files; costs one object-header lookup for those.
pub(super) fn committed_large_file_state(
    repo: &gix::Repository,
    id: gix::ObjectId,
    is_symlink: bool,
    logical_path: &Path,
) -> Option<LargeFileState> {
    let classified = classify_blob(repo, id, is_symlink)?;
    let in_local_store = repo.workdir().and_then(|workdir| {
        in_local_store(&classified, &lfs_storage_dir(repo), workdir, logical_path)
    });
    Some(LargeFileState {
        pointer: classified.pointer,
        in_local_store,
        worktree: None,
        lockable: false,
    })
}

/// Whether the content is here. Unlocked annex files need the tool to know.
fn in_local_store(
    classified: &Classified,
    storage_dir: &Path,
    workdir: &Path,
    path: &Path,
) -> Option<bool> {
    match (&classified.pointer, &classified.link_target) {
        (LargeFilePointer::Lfs(pointer), _) => Some(
            storage_dir
                .join(lfs::object_relative_path(&pointer.oid))
                .is_file(),
        ),
        (LargeFilePointer::Annex(_), Some(target)) => {
            let target = gix::path::try_from_byte_slice(target).ok()?;
            let link_dir = workdir.join(path);
            Some(link_dir.parent()?.join(target).is_file())
        }
        (LargeFilePointer::Annex(_), None) => None,
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

impl super::GixRepo {
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
        let classified = classify_git_form(&read_pointer_candidate(&source.path)?)?;
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
            None => match (&classified.pointer, &classified.link_target) {
                (LargeFilePointer::Lfs(pointer), _) => {
                    let path = lfs_storage_dir(repo).join(lfs::object_relative_path(&pointer.oid));
                    (
                        path.is_file().then_some(path),
                        Some(format!("lfs:{}", pointer.oid)),
                    )
                }
                (LargeFilePointer::Annex(key), Some(target)) => {
                    let path = gix::path::try_from_byte_slice(target)
                        .ok()
                        .and_then(|target| Some(full.parent()?.join(target)))
                        .filter(|path| path.is_file());
                    (path, Some(format!("annex:{}", key.raw)))
                }
                (LargeFilePointer::Annex(_), None) => (None, None),
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
            None if !worktree
                && classified.link_target.is_none()
                && !classified.pointer.is_lfs() =>
            {
                LargeFileContent::Unknown
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
        logical_path: &Path,
        max_bytes: u64,
    ) -> Option<(LargeFileSide, Option<Vec<u8>>)> {
        if git_form.len() as u64 > MAX_POINTER_BYTES || !may_be_pointer(git_form) {
            return None;
        }
        let classified = classify_git_form(git_form)?;
        let path = match (&classified.pointer, &classified.link_target) {
            (LargeFilePointer::Lfs(pointer), _) => {
                lfs_storage_dir(repo).join(lfs::object_relative_path(&pointer.oid))
            }
            (LargeFilePointer::Annex(_), Some(target)) => self
                .spec
                .workdir
                .join(logical_path)
                .parent()?
                .join(gix::path::try_from_byte_slice(target).ok()?),
            (LargeFilePointer::Annex(_), None) => {
                return Some((
                    LargeFileSide {
                        pointer: classified.pointer,
                        content: LargeFileContent::Unknown,
                    },
                    None,
                ));
            }
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
        let repo = self.repo();
        let config = repo.config_snapshot();
        let storage_dir = lfs_storage_dir(&repo);
        let skip_flag = |key: &str| {
            config
                .string(key)
                .is_some_and(|value| value.contains_str("--skip"))
        };
        cancellation.check_cancelled()?;
        let tracked_patterns = self.lfs_tracked_patterns(&repo, cancellation)?;
        let lfs = LfsRepoInfo {
            filter_configured: lfs_filter_configured(&config),
            filter_required: config.boolean("filter.lfs.required").unwrap_or(false),
            tracked_patterns,
            has_local_store: storage_dir.join("objects").is_dir(),
            storage_dir,
            skip_smudge: truthy_env("GIT_LFS_SKIP_SMUDGE")
                || skip_flag("filter.lfs.smudge")
                || skip_flag("filter.lfs.process"),
            locks_verify: config.boolean("lfs.locksverify"),
        };

        let head_short = repo
            .head_name()
            .ok()
            .flatten()
            .map(|name| name.shorten().to_str_lossy().into_owned());
        let annex = AnnexRepoInfo {
            has_annex_dir: repo.common_dir().join("annex").is_dir(),
            has_annex_branch: repo
                .try_find_reference("refs/heads/git-annex")
                .ok()
                .flatten()
                .is_some(),
            uuid: config
                .string("annex.uuid")
                .map(|value| value.to_str_lossy().into_owned())
                .filter(|value| !value.is_empty()),
            adjusted: head_short.as_deref().and_then(|head| {
                annex::adjusted_branch(head).map(|(base, mode)| (base.to_owned(), mode.to_owned()))
            }),
            crippled_filesystem: config.boolean("annex.crippledfilesystem").unwrap_or(false),
        };
        Ok(LargeFileSupport { lfs, annex })
    }

    /// `filter=lfs` patterns from every tracked `.gitattributes`, an untracked
    /// root one, and `info/attributes`. Macros are not expanded here; per-file
    /// state uses the real attribute stack.
    fn lfs_tracked_patterns(
        &self,
        repo: &gix::Repository,
        cancellation: &CancellationToken,
    ) -> Result<Vec<LfsTrackedPattern>> {
        let workdir = &self.spec.workdir;
        let mut sources: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        let index = repo
            .index_or_empty()
            .map_err(|e| backend_error("read index for LFS patterns", e))?;
        let mut saw_root = false;
        for entry in index.entries() {
            let path = entry.path(&index);
            if path != ".gitattributes" && !path.ends_with(b"/.gitattributes") {
                continue;
            }
            cancellation.check_cancelled()?;
            saw_root |= path == ".gitattributes";
            let Ok(relative) = gix::path::try_from_bstr(path) else {
                continue;
            };
            let relative = relative.into_owned();
            let bytes = std::fs::read(workdir.join(&relative)).ok().or_else(|| {
                repo.find_object(entry.id)
                    .ok()
                    .map(|object| object.data.clone())
            });
            if let Some(bytes) = bytes {
                sources.push((relative, bytes));
            }
        }
        if !saw_root && let Ok(bytes) = std::fs::read(workdir.join(".gitattributes")) {
            sources.push((PathBuf::from(".gitattributes"), bytes));
        }
        let info = repo.common_dir().join("info").join("attributes");
        if let Ok(bytes) = std::fs::read(&info) {
            sources.push((PathBuf::from(".git/info/attributes"), bytes));
        }

        let mut patterns = Vec::new();
        for (source, bytes) in sources {
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
                        ("lockable", gix::attrs::StateRef::Set) => lockable = true,
                        _ => {}
                    }
                }
                if lfs {
                    patterns.push(LfsTrackedPattern {
                        pattern: pattern.to_string(),
                        lockable,
                        source: source.clone(),
                    });
                }
            }
        }
        Ok(patterns)
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
        let repo = self.repo();
        let index = repo
            .index_or_empty()
            .map_err(|e| backend_error("read index for large files", e))?;
        let storage_dir = lfs_storage_dir(&repo);
        let workdir = self.spec.workdir.clone();
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
            let in_store = in_local_store(&classified, &storage_dir, &workdir, path);
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
            if let Some(classified) = index_classified(&repo, &index, &entry.path) {
                let state = finish(classified, None, &entry.path, &mut lockable);
                result.staged.insert(entry.path.clone(), state);
            }
        }
        for entry in status.unstaged.iter() {
            cancellation.check_cancelled()?;
            if let Some((classified, worktree)) = self.classify_unstaged_row(&repo, &index, entry) {
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
        if metadata.is_file()
            && metadata.len() <= MAX_POINTER_BYTES
            && let Ok(bytes) = std::fs::read(&full)
            && let Some(classified) = classify_bytes(&bytes, false)
        {
            return Some((classified, LargeFileWorktree::Pointer));
        }
        index_classified(repo, index, &entry.path)
            .map(|classified| (classified, LargeFileWorktree::Content))
    }
}
