use super::log::{
    COMMIT_STATS_BINARY_SNIFF_BYTES, COMMIT_STATS_MAX_BLOB_BYTES, CommitStatsScratch,
    commit_file_line_stats, commit_stats_looks_binary, line_stats_from_bytes,
    read_commit_stats_blob,
};
use crate::util::path_buf_from_git_bytes;
use gitcomet_core::domain::{
    DiffArea, FileStatus, FileStatusKind, LineStats, UncommittedLineStats,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use gix::error::ResultExt as _;
use gix::prelude::FindExt as _;
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash as _, Hasher as _};
use std::path::PathBuf;

type LineStatsKey = (DiffArea, Option<gix::ObjectId>, Option<gix::ObjectId>);

/// The two lanes have different rules for pointers. Keep counts separate,
/// and classify index blobs by id before touching any worktree payload.
#[derive(Default)]
pub(super) struct LineStatsMemo {
    counts: FxHashMap<LineStatsKey, LineStats>,
    pointer_index_blobs: FxHashMap<gix::ObjectId, IndexPointer>,
    /// Untracked files have no blob id, so the read identity keys them.
    untracked: FxHashMap<PathBuf, UntrackedCounts>,
}

impl LineStatsMemo {
    /// Adds `other`'s entries, for a scan that saw only part of the changes.
    fn extend(&mut self, other: Self) {
        let Self {
            counts,
            pointer_index_blobs,
            untracked,
        } = other;
        self.counts.extend(counts);
        self.pointer_index_blobs.extend(pointer_index_blobs);
        self.untracked.extend(untracked);
    }
}

#[derive(Clone, Copy)]
struct UntrackedCounts {
    identity: ReadIdentity,
    stats: LineStats,
}

/// A worktree file's stamp and how it converts (its conversion attributes
/// and config). For a path with no index entry the same identity reads the
/// same bytes; with one, autocrlf also reads the entry's blob.
#[derive(Clone, Copy, Eq, PartialEq)]
struct ReadIdentity {
    file: super::DiskFileStamp,
    conversion: u64,
}

/// What reading a worktree file found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadOutcome {
    Text,
    Binary,
    /// Gone, unopenable, or a conversion or read failed.
    Unreadable,
}

impl ReadOutcome {
    fn of(bytes: &[u8]) -> Self {
        if commit_stats_looks_binary(bytes) {
            Self::Binary
        } else {
            Self::Text
        }
    }
}

/// An untracked file counted as wholly added, as far as its read went.
enum AddedLines {
    Counted(LineStats),
    Binary,
    /// The read failed; another may not.
    Unreadable,
}

impl AddedLines {
    fn stats(&self) -> LineStats {
        match self {
            Self::Counted(stats) => *stats,
            Self::Binary | Self::Unreadable => LineStats::UNKNOWN,
        }
    }
}

/// What an index blob stands in for, decided from its first bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IndexPointer {
    /// The blob is the content itself.
    Content,
    Annex,
}

/// A blob pair's unknown is cheap to rediscover (size cap, binary sniff) or
/// may be transient (an unreadable object), so the memo never keeps it.
fn memoizable(stats: &LineStats) -> bool {
    stats.additions.is_some()
}

/// One scan's view of the memo: hits carry over into `next`, which replaces
/// the memo, so it only ever holds the current changes.
#[derive(Default)]
struct MemoScan {
    previous: LineStatsMemo,
    next: LineStatsMemo,
}

impl MemoScan {
    fn counts(&mut self, key: LineStatsKey, diff: impl FnOnce() -> LineStats) -> LineStats {
        if let Some(&entry) = self
            .previous
            .counts
            .get(&key)
            .or_else(|| self.next.counts.get(&key))
        {
            self.next.counts.insert(key, entry);
            return entry;
        }
        #[cfg(test)]
        LINE_STATS_DIFFS.with(|diffs| diffs.set(diffs.get() + 1));
        let entry = diff();
        if memoizable(&entry) {
            self.next.counts.insert(key, entry);
        }
        entry
    }

    fn index_pointer(
        &mut self,
        id: gix::ObjectId,
        read: impl FnOnce() -> Option<IndexPointer>,
    ) -> Option<IndexPointer> {
        let pointer = self
            .previous
            .pointer_index_blobs
            .get(&id)
            .or_else(|| self.next.pointer_index_blobs.get(&id))
            .copied()
            .or_else(read)?;
        self.next.pointer_index_blobs.insert(id, pointer);
        Some(pointer)
    }

    /// An untracked file's counts, counted again only when its read identity
    /// changed. `None` (no stamp to trust) always counts. A binary file's
    /// unknown is kept too: the same identity reads the same bytes, and
    /// sniffing every unchanged binary again costs more than the memo.
    fn untracked(
        &mut self,
        path: &std::path::Path,
        identity: Option<ReadIdentity>,
        count: impl FnOnce() -> AddedLines,
    ) -> LineStats {
        let Some(identity) = identity else {
            return count().stats();
        };
        if let Some((owned, entry)) = self.previous.untracked.remove_entry(path)
            && entry.identity == identity
        {
            self.next.untracked.insert(owned, entry);
            return entry.stats;
        }
        let added = count();
        let stats = added.stats();
        match added {
            AddedLines::Counted(_) | AddedLines::Binary => {
                self.next
                    .untracked
                    .insert(path.to_path_buf(), UntrackedCounts { identity, stats });
            }
            AddedLines::Unreadable => {}
        }
        stats
    }
}

#[cfg(test)]
thread_local! {
    static LINE_STATS_DIFFS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LINE_STATS_WORKTREE_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn take_line_stats_diffs_for_tests() -> usize {
    LINE_STATS_DIFFS.with(|diffs| diffs.replace(0))
}

#[cfg(test)]
fn take_worktree_reads_for_tests() -> usize {
    LINE_STATS_WORKTREE_READS.with(|reads| reads.replace(0))
}

#[cfg(test)]
thread_local! {
    static STAGED_WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn take_staged_walks_for_tests() -> usize {
    STAGED_WALKS.with(|walks| walks.replace(0))
}

type ContentPair = (Option<gix::ObjectId>, Option<gix::ObjectId>);

/// One staged change: its path, the content ids the memo keys on, its counts.
type StagedLineStatsRow = (PathBuf, ContentPair, LineStats);

/// The last staged walk. Its counts depend on HEAD and the index alone, and a
/// worktree save, the usual reason to recount, changes neither; the walk
/// itself costs a tree-vs-index comparison over the whole index.
pub(super) struct StagedLineStatsCache {
    head_oid: Option<gix::ObjectId>,
    index_stamp: super::RepoFileStamp,
    rows: Vec<StagedLineStatsRow>,
}

impl super::GixRepo {
    pub(super) fn uncommitted_line_stats_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<UncommittedLineStats> {
        let entries = self.worktree_status_cancellable_impl(cancellation)?;
        self.line_stats_for_entries_impl(&entries, cancellation)
    }

    pub(super) fn line_stats_for_entries_impl(
        &self,
        entries: &[FileStatus],
        cancellation: &CancellationToken,
    ) -> Result<UncommittedLineStats> {
        cancellation.check_cancelled()?;
        let repo = self.status_repo();
        // Taken, not held: a concurrent scan starts cold instead of waiting.
        let mut scan = MemoScan {
            previous: std::mem::take(&mut *self.line_stats_memo()),
            next: LineStatsMemo::default(),
        };
        let result =
            cached_staged_line_stats(self, &repo, &mut scan, cancellation).and_then(|staged| {
                cancellation.check_cancelled()?;
                let unstaged = unstaged_line_stats(self, &repo, entries, &mut scan, cancellation)?;
                Ok(UncommittedLineStats { staged, unstaged })
            });
        let MemoScan { mut previous, next } = scan;
        // A cancelled scan saw only part of the changes; keep both.
        *self.line_stats_memo() = if result.is_ok() {
            next
        } else {
            previous.extend(next);
            previous
        };
        result
    }

    /// What a commit-to-worktree file list lacks. Untracked files are counted
    /// as the status list counts them. `git diff --numstat` counted the rest
    /// without the contents, so they get only the edit, read under the caps
    /// the counts use and only for files those could count.
    pub(super) fn add_worktree_line_stats(
        &self,
        files: &mut [gitcomet_core::domain::CommitFileChange],
        cancellation: &CancellationToken,
    ) -> Result<()> {
        let edits = files.len() <= super::log::COMMIT_STATS_MAX_FILES;
        if !edits
            && files
                .iter()
                .all(|file| file.kind != FileStatusKind::Untracked)
        {
            return Ok(());
        }
        let repo = self.status_repo();
        let mut reader = WorktreeReader::new(&repo, &self.spec.workdir);
        let (mut old, mut new) = (Vec::new(), Vec::new());
        for file in files.iter_mut() {
            if file.kind == FileStatusKind::Untracked {
                cancellation.check_cancelled()?;
                file.line_stats = reader.find(&file.path).map_or(LineStats::UNKNOWN, |found| {
                    found.added_line_stats(&mut new).stats()
                });
                continue;
            }
            if !edits || file.line_stats.edit.is_some() || file.line_stats.additions.is_none() {
                continue;
            }
            cancellation.check_cancelled()?;
            let old_id = file
                .old_id
                .as_ref()
                .and_then(|id| gix::ObjectId::from_hex(id.0.as_bytes()).ok());
            if !read_commit_stats_blob(&repo, old_id, &mut old) {
                continue;
            }
            new.clear();
            if file.kind != FileStatusKind::Deleted && !reader.read(&file.path, &mut new) {
                continue;
            }
            file.line_stats.edit = line_stats_from_bytes(&old, &new).edit;
        }
        Ok(())
    }

    fn line_stats_memo(&self) -> std::sync::MutexGuard<'_, LineStatsMemo> {
        self.line_stats_memo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Staged counts, from the last walk while HEAD and the index are unchanged.
fn cached_staged_line_stats(
    gix_repo: &super::GixRepo,
    repo: &gix::Repository,
    memo: &mut MemoScan,
    cancellation: &CancellationToken,
) -> Result<FxHashMap<PathBuf, LineStats>> {
    let head_oid = super::history::gix_head_id_or_none(repo)?;
    let index_stamp = super::status::repo_index_stamp(repo);
    let cached = gix_repo
        .staged_line_stats_cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .filter(|cached| cached.head_oid == head_oid && cached.index_stamp == index_stamp);
    let rows = match cached {
        Some(cached) => {
            // Keep the memo as the walk would have left it, so the next walk
            // (after a stage or commit) still skips unchanged pairs.
            for (_, pair, stats) in &cached.rows {
                if memoizable(stats) {
                    memo.next
                        .counts
                        .insert((DiffArea::Staged, pair.0, pair.1), *stats);
                }
            }
            cached.rows
        }
        None => staged_line_stats(repo, memo, cancellation)?,
    };
    let out = rows
        .iter()
        .map(|(path, _, stats)| (path.clone(), *stats))
        .collect();
    // Kept only if the index did not move under the walk.
    if super::status::repo_index_stamp(repo) == index_stamp {
        *gix_repo
            .staged_line_stats_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(StagedLineStatsCache {
            head_oid,
            index_stamp,
            rows,
        });
    }
    Ok(out)
}

/// HEAD tree vs index. The walk hands us both blob ids, so this costs two
/// object reads per changed pair the memo lacks and no worktree traversal.
fn staged_line_stats(
    repo: &gix::Repository,
    memo: &mut MemoScan,
    cancellation: &CancellationToken,
) -> Result<Vec<StagedLineStatsRow>> {
    #[cfg(test)]
    STAGED_WALKS.with(|walks| walks.set(walks.get() + 1));
    let mut out = Vec::new();
    // `tree_index_status` wants a tree; an unborn HEAD measures against the
    // empty tree.
    let head_tree_id = match super::history::gix_head_id_or_none(repo)? {
        Some(head_oid) => super::status::tree_id_for_commit(repo, &head_oid)?,
        None => repo.empty_tree().id().detach(),
    };
    let index = repo
        .index_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    let mut scratch = CommitStatsScratch::default();

    let result = repo.tree_index_status(
        &head_tree_id,
        &index,
        None,
        // Without an on-disk index there can be no staged rename. Avoid
        // gix's diff-cache fallback to its files-only HEAD reader.
        if repo.try_index().map_err(crate::refs::failure)?.is_none() {
            gix::status::tree_index::TrackRenames::Disabled
        } else {
            gix::status::tree_index::TrackRenames::AsConfigured
        },
        |change, _, _| {
            use gix::diff::index::ChangeRef;
            cancellation.check_cancelled().or_error()?;
            let (location, old_id, new_id) = match change {
                ChangeRef::Addition { location, id, .. } => (location, None, Some(id.into_owned())),
                ChangeRef::Deletion { location, id, .. } => (location, Some(id.into_owned()), None),
                ChangeRef::Modification {
                    location,
                    previous_id,
                    id,
                    ..
                } => (
                    location,
                    Some(previous_id.into_owned()),
                    Some(id.into_owned()),
                ),
                // Both ids, so a rename counts the edit, not the whole file.
                ChangeRef::Rewrite {
                    location,
                    source_id,
                    id,
                    ..
                } => (
                    location,
                    Some(source_id.into_owned()),
                    Some(id.into_owned()),
                ),
            };
            let path = path_buf_from_git_bytes(location.as_ref(), "gix staged line stats path")
                .or_error()?;
            let stats = memo.counts((DiffArea::Staged, old_id, new_id), || {
                commit_file_line_stats(repo, old_id, new_id, &mut scratch)
            });
            out.push((path, (old_id, new_id), stats));
            Ok(std::ops::ControlFlow::Continue(()))
        },
    );
    // The walk wraps callback errors; preserve cancellation as its own error kind.
    cancellation.check_cancelled()?;
    result.map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix tree/index line stats: {e}"
        )))
    })?;

    Ok(out)
}

/// Index blob vs worktree file, the latter run through the worktree->git
/// filter pipeline so CRLF and clean filters apply before counting. An
/// untracked file has no blob, so all of it is added.
fn unstaged_line_stats(
    gix_repo: &super::GixRepo,
    repo: &gix::Repository,
    entries: &[FileStatus],
    memo: &mut MemoScan,
    cancellation: &CancellationToken,
) -> Result<FxHashMap<PathBuf, LineStats>> {
    let mut out = FxHashMap::default();
    if entries.is_empty() {
        return Ok(out);
    }

    let index = repo
        .index_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    // Only the old side is a blob, so `CommitStatsScratch` does not fit.
    let mut index_blob = Vec::new();
    let mut worktree = Vec::new();
    let mut reader = WorktreeReader::new(repo, &gix_repo.spec.workdir);

    for entry in entries.iter() {
        // Conflicted has no single before.
        if entry.kind == FileStatusKind::Conflicted {
            continue;
        }
        // Per file: this loop reads both sides of each one.
        cancellation.check_cancelled()?;
        let stats = unstaged_entry_line_stats(
            repo,
            &index,
            entry,
            &mut index_blob,
            &mut worktree,
            &mut reader,
            memo,
        );
        out.insert(entry.path.clone(), stats);
    }

    Ok(out)
}

fn unstaged_entry_line_stats(
    repo: &gix::Repository,
    index: &gix::index::File,
    entry: &FileStatus,
    index_blob: &mut Vec<u8>,
    worktree: &mut Vec<u8>,
    reader: &mut WorktreeReader<'_>,
    memo: &mut MemoScan,
) -> LineStats {
    let Ok(rel) = gix::path::into_bstr(entry.path.as_path()) else {
        return LineStats::UNKNOWN;
    };
    let Some(index_id) = index.entry_by_path(rel.as_ref()).map(|found| found.id) else {
        // Untracked: every line is added. With no blob to key the memo on,
        // the read identity does.
        return reader
            .find(&entry.path)
            .map_or(LineStats::UNKNOWN, |found| {
                memo.untracked(&entry.path, found.identity(), || {
                    found.added_line_stats(worktree)
                })
            });
    };

    let mut index_blob_loaded = false;
    let pointer = memo.index_pointer(index_id, || {
        let header = repo.find_header(index_id).ok()?;
        if header.kind() != gix::object::Kind::Blob {
            return None;
        }
        if header.size() > gitcomet_core::annex::POINTER_MAX_BYTES as u64 {
            return Some(IndexPointer::Content);
        }
        // A larger blob cannot be a pointer; leave its decompression to
        // a diff miss, after the worktree's size and binary checks.
        index_blob.clear();
        index_blob_loaded = repo.objects.find_blob(&index_id, index_blob).is_ok();
        index_blob_loaded.then(|| {
            if gitcomet_core::annex::key_from_pointer(index_blob).is_some() {
                IndexPointer::Annex
            } else {
                IndexPointer::Content
            }
        })
    });
    match pointer {
        // Unlocked content cannot be compared without annex's clean
        // filter. Its size and hash cannot make these counts known.
        Some(IndexPointer::Annex) | None => return LineStats::UNKNOWN,
        Some(IndexPointer::Content) => {}
    }

    worktree.clear();
    // Binary or unread on this side means no counts, whatever the index
    // holds: skip the hash, the index read and the memo.
    if entry.kind != FileStatusKind::Deleted && !reader.read(&entry.path, worktree) {
        return LineStats::UNKNOWN;
    }

    let mut diff = || {
        if !index_blob_loaded && !read_commit_stats_blob(repo, Some(index_id), index_blob) {
            return LineStats::UNKNOWN;
        }
        line_stats_from_bytes(index_blob.as_slice(), worktree.as_slice())
    };
    // Hashing costs a fraction of the diff an unchanged file then skips.
    match gix::objs::compute_hash(repo.object_hash(), gix::objs::Kind::Blob, worktree) {
        Ok(worktree_id) => memo.counts(
            (DiffArea::Unstaged, Some(index_id), Some(worktree_id)),
            diff,
        ),
        Err(_) => diff(),
    }
}

/// Reads worktree files as git would store them, keeping attribute caches
/// and filter processes across files. A file the LFS clean filter takes is
/// not read: cleaning would hash and store the payload on every refresh, only
/// to count changed pointer lines.
struct WorktreeReader<'repo> {
    repo: &'repo gix::Repository,
    workdir: &'repo std::path::Path,
    /// `None` when the filters or attributes cannot be loaded: nothing reads.
    filters: Option<Filters<'repo>>,
}

/// The attributes the worktree->git conversion reads; a file's states key
/// the untracked memo. `find` takes `filter` and `working-tree-encoding` by
/// position.
const CONVERSION_ATTRIBUTES: [&str; 6] = [
    "filter",
    "working-tree-encoding",
    "text",
    "crlf",
    "eol",
    "ident",
];

/// The worktree->git filters and the attributes a read decides on.
struct Filters<'repo> {
    pipeline: gix::filter::Pipeline<'repo>,
    index: gix::worktree::IndexPersistedOrInMemory,
    attributes: gix::AttributeStack<'repo>,
    /// [`CONVERSION_ATTRIBUTES`] as the last path matched them.
    selected: gix::attrs::search::Outcome,
    /// Filter drivers with a clean or process program configured.
    cleaning_drivers: Vec<gix::bstr::BString>,
    /// The values of [`super::diff::WORKTREE_CONVERSION_CONFIG`], hashed.
    config: u64,
    /// A file's raw sniff window, reused across files.
    window: Vec<u8>,
}

/// A worktree path stat'ed and matched against attributes, not yet read.
enum Found<'a, 'repo> {
    /// A symlink, stored as its target.
    Link(gix::bstr::BString),
    File(FoundFile<'a, 'repo>),
}

struct FoundFile<'a, 'repo> {
    repo: &'repo gix::Repository,
    filters: &'a mut Filters<'repo>,
    relative: &'a std::path::Path,
    full: PathBuf,
    /// A clean or process program converts it.
    cleans: bool,
    /// It has a `working-tree-encoding`.
    encoded: bool,
    /// `None` while racy, or when a program converts it: a program's output
    /// can change without its input changing.
    stamp: Option<super::DiskFileStamp>,
}

impl<'repo> WorktreeReader<'repo> {
    fn new(repo: &'repo gix::Repository, workdir: &'repo std::path::Path) -> Self {
        let filters = repo
            .filter_pipeline(None)
            .ok()
            .and_then(|(pipeline, index)| {
                let attributes = repo
                    .attributes_only(
                        &index,
                        gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
                    )
                    .ok()?;
                let cleaning_drivers = gix::filter::Pipeline::options(repo)
                    .ok()?
                    .drivers
                    .into_iter()
                    .filter(|driver| driver.clean.is_some() || driver.process.is_some())
                    .map(|driver| driver.name)
                    .collect();
                let snapshot = repo.config_snapshot();
                let mut config = FxHasher::default();
                for key in super::diff::WORKTREE_CONVERSION_CONFIG {
                    snapshot.string(key).hash(&mut config);
                }
                Some(Filters {
                    selected: attributes.selected_attribute_matches(CONVERSION_ATTRIBUTES),
                    pipeline,
                    index,
                    attributes,
                    cleaning_drivers,
                    config: config.finish(),
                    window: Vec::with_capacity(COMMIT_STATS_BINARY_SNIFF_BYTES),
                })
            });
        Self {
            repo,
            workdir,
            filters,
        }
    }

    /// Replaces `out` with the file at `relative`. `false` means gone, over
    /// the size cap, unreadable, binary, or left to the LFS clean filter.
    fn read(&mut self, relative: &std::path::Path, out: &mut Vec<u8>) -> bool {
        self.find(relative)
            .is_some_and(|found| found.read(out) == ReadOutcome::Text)
    }

    /// `None` means gone, over the size cap, unreadable, or left to the LFS
    /// clean filter.
    fn find<'a>(&'a mut self, relative: &'a std::path::Path) -> Option<Found<'a, 'repo>> {
        use gix::attrs::StateRef;

        let full = self.workdir.join(relative);
        // Before the stat: a pause after it must not age a racy stamp.
        let now = super::racy_check_now(&full);
        let metadata = std::fs::symlink_metadata(&full).ok()?;
        if metadata.file_type().is_symlink() {
            let target = gix::path::into_bstr(std::fs::read_link(&full).ok()?).ok()?;
            return Some(Found::Link(target.into_owned()));
        }
        if !metadata.is_file() || metadata.len() > COMMIT_STATS_MAX_BLOB_BYTES as u64 {
            return None;
        }
        let filters = self.filters.as_mut()?;
        let path = gix::path::into_bstr(relative).ok()?;
        filters
            .attributes
            .at_entry(path.as_ref(), None)
            .ok()?
            .matching_attributes(&mut filters.selected);
        let (filter, encoding) = {
            let mut states = filters
                .selected
                .iter_selected()
                .map(|matched| matched.assignment.state);
            (states.next(), states.next())
        };
        let driver = match filter {
            Some(StateRef::Value(name)) => filters
                .cleaning_drivers
                .iter()
                .find(|driver| **driver == name.as_bstr()),
            _ => None,
        };
        if driver.is_some_and(|driver| *driver == super::large_files::LFS_FILTER_DRIVER) {
            return None;
        }
        let cleans = driver.is_some();
        let encoded = matches!(encoding, Some(StateRef::Value(_)));
        // Unlike the diff panel's memo, no second stat after the read: a write
        // after this stat moves ctime, so counts from a torn read never match.
        let stamp = super::DiskFileStamp::from_metadata(&metadata)
            .filter(|stamp| !cleans && !stamp.is_racy_at(now));
        Some(Found::File(FoundFile {
            repo: self.repo,
            filters,
            relative,
            full,
            cleans,
            encoded,
            stamp,
        }))
    }
}

impl Found<'_, '_> {
    /// `None` when counts read from it must not be reused.
    fn identity(&self) -> Option<ReadIdentity> {
        let Self::File(file) = self else {
            return None;
        };
        let stamp = file.stamp?;
        let mut conversion = FxHasher::default();
        file.filters.config.hash(&mut conversion);
        for matched in file.filters.selected.iter_selected() {
            matched.assignment.state.hash(&mut conversion);
        }
        Some(ReadIdentity {
            file: stamp,
            conversion: conversion.finish(),
        })
    }

    /// A file with nothing stored for it: all of it is added. Reads into
    /// `buf`.
    fn added_line_stats(self, buf: &mut Vec<u8>) -> AddedLines {
        match self.read(buf) {
            ReadOutcome::Text => AddedLines::Counted(line_stats_from_bytes(&[], buf)),
            ReadOutcome::Binary => AddedLines::Binary,
            ReadOutcome::Unreadable => AddedLines::Unreadable,
        }
    }

    /// Replaces `out` with what git would store.
    fn read(self, out: &mut Vec<u8>) -> ReadOutcome {
        out.clear();
        match self {
            Self::Link(target) => {
                out.extend_from_slice(&target);
                ReadOutcome::Text
            }
            Self::File(file) => file.read(out),
        }
    }
}

impl FoundFile<'_, '_> {
    fn read(self, out: &mut Vec<u8>) -> ReadOutcome {
        use gix::filter::plumbing::pipeline::convert::ToGitOutcome;
        use std::io::Read as _;

        let Filters {
            pipeline,
            index,
            attributes,
            window,
            ..
        } = self.filters;
        #[cfg(test)]
        LINE_STATS_WORKTREE_READS.with(|reads| reads.set(reads.get() + 1));
        if self.encoded {
            let text = super::text_attributes::resolve_text_attributes_on(
                self.repo,
                attributes,
                self.relative,
            );
            if super::text_attributes::decodes_working_tree_encoding(&text) {
                let Ok(bytes) = super::diff::worktree_encoding_to_git(
                    self.repo,
                    index,
                    self.relative,
                    &self.full,
                    &text,
                ) else {
                    return ReadOutcome::Unreadable;
                };
                out.extend_from_slice(&bytes);
                return ReadOutcome::of(out);
            }
        }
        let Ok(mut file) = std::fs::File::open(&self.full) else {
            return ReadOutcome::Unreadable;
        };
        // Without a cleaning driver or working-tree-encoding, conversion only
        // drops CRs and shortens `$Id$`: a NUL in the raw sniff window stays
        // binary, so such a file is not read past it.
        window.clear();
        if !self.cleans && !self.encoded {
            if (&mut file)
                .take(COMMIT_STATS_BINARY_SNIFF_BYTES as u64)
                .read_to_end(window)
                .is_err()
            {
                return ReadOutcome::Unreadable;
            }
            if commit_stats_looks_binary(window) {
                return ReadOutcome::Binary;
            }
        }
        let source = std::io::Cursor::new(window.as_slice()).chain(file);
        let Ok(converted) = pipeline.convert_to_git(source, self.relative, index) else {
            return ReadOutcome::Unreadable;
        };
        let read = match converted {
            ToGitOutcome::Unchanged(mut source) => source.read_to_end(out).is_ok(),
            ToGitOutcome::Process(mut stream) => stream.read_to_end(out).is_ok(),
            ToGitOutcome::Buffer(bytes) => {
                out.extend_from_slice(bytes);
                true
            }
        };
        if !read {
            return ReadOutcome::Unreadable;
        }
        ReadOutcome::of(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{git_success, init_test_repo, open_repo, write_file};

    /// The counts alone: these tests are about counting, not the edit.
    fn counts(stats: Option<&LineStats>) -> Option<(Option<u32>, Option<u32>)> {
        stats.map(|stats| (stats.additions, stats.deletions))
    }

    #[test]
    fn review_unlocked_annex_stats_skip_payload_reads_on_every_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(
            dir,
            "asset.bin",
            "/annex/objects/WORM-s3000000-m1--payload\n",
        );
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "annex pointer"]);
        std::fs::write(dir.join("asset.bin"), vec![b'x'; 3_000_000]).unwrap();
        let repo = open_repo(dir);
        let entries = [FileStatus {
            path: "asset.bin".into(),
            kind: FileStatusKind::Modified,
            conflict: None,
        }];
        take_worktree_reads_for_tests();
        for _ in 0..2 {
            assert_eq!(
                repo.line_stats_for_entries_impl(&entries, &CancellationToken::new())
                    .unwrap()
                    .unstaged[std::path::Path::new("asset.bin")],
                LineStats::UNKNOWN
            );
        }
        assert_eq!(
            take_worktree_reads_for_tests(),
            0,
            "unknown annex counts must not read/hash the unlocked payload"
        );
    }

    #[test]
    fn unstaged_lfs_payloads_never_start_the_clean_filter() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        let pointer = |oid: char, size: usize| {
            format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {size}\n",
                oid.to_string().repeat(64)
            )
        };
        for path in ["staged.bin", "unstaged.bin"] {
            write_file(dir, path, &pointer('a', 5));
        }
        // Stored as content: committed before a driver was configured.
        write_file(dir, "content.bin", "content\n");
        write_file(dir, ".gitattributes", "*.bin filter=lfs\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "pointers"]);
        // A staged change is pointer against pointer, both already stored.
        write_file(dir, "staged.bin", &pointer('b', 7));
        git_success(dir, &["add", "staged.bin"]);
        write_file(dir, "intent.bin", "new payload\n");
        git_success(dir, &["add", "-N", "intent.bin"]);
        let ran = dir.join("clean-filter-ran");
        git_success(
            dir,
            &[
                "config",
                "filter.lfs.clean",
                &format!("touch '{}'; cat", ran.display()),
            ],
        );
        std::fs::write(dir.join("unstaged.bin"), b"smudged payload\n").unwrap();
        std::fs::write(dir.join("untracked.bin"), b"new payload\n").unwrap();
        write_file(dir, "content.bin", "content\nedited\n");
        // Listed by hand: the status walk itself re-reads the racily clean
        // `staged.bin` through the filter, which is not the counts' doing.
        let entries = [
            ("unstaged.bin", FileStatusKind::Modified),
            ("untracked.bin", FileStatusKind::Untracked),
            ("intent.bin", FileStatusKind::Added),
            ("content.bin", FileStatusKind::Modified),
        ]
        .map(|(path, kind)| FileStatus {
            path: path.into(),
            kind,
            conflict: None,
        });
        let repo = open_repo(dir);
        take_worktree_reads_for_tests();
        for _ in 0..2 {
            let stats = repo
                .line_stats_for_entries_impl(&entries, &CancellationToken::new())
                .unwrap();
            for path in ["unstaged.bin", "untracked.bin", "intent.bin", "content.bin"] {
                assert_eq!(
                    stats.unstaged[std::path::Path::new(path)],
                    LineStats::UNKNOWN,
                    "{path}"
                );
            }
            assert_eq!(
                counts(stats.staged.get(std::path::Path::new("staged.bin"))),
                Some((Some(2), Some(2)))
            );
        }
        assert_eq!(take_worktree_reads_for_tests(), 0);
        assert!(!ran.exists(), "the LFS clean filter must not run");
    }

    /// A `working-tree-encoding` file counts the lines git stores, decoded
    /// with git's (iconv) labels as the diff panel decodes them; gix knows
    /// only WHATWG labels.
    #[test]
    fn working_tree_encoded_files_count_their_decoded_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        let utf16 = |text: &str| -> Vec<u8> {
            [0xFF, 0xFE]
                .into_iter()
                .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
                .collect()
        };
        write_file(
            dir,
            ".gitattributes",
            "*.txt working-tree-encoding=UTF-16LE-BOM\n",
        );
        std::fs::write(dir.join("tracked.txt"), utf16("a\n")).unwrap();
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        std::fs::write(dir.join("tracked.txt"), utf16("a\nb\n")).unwrap();
        std::fs::write(dir.join("new.txt"), utf16("a\nb\nc\n")).unwrap();

        let stats = open_repo(dir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap();

        let unstaged = |path: &str| counts(stats.unstaged.get(std::path::Path::new(path)));
        assert_eq!(unstaged("tracked.txt"), Some((Some(1), Some(0))));
        assert_eq!(unstaged("new.txt"), Some((Some(3), Some(0))));
    }

    /// A cleaning driver can turn binary-looking bytes into text, so its
    /// paths are counted from what it stores, not sniffed raw.
    #[cfg(unix)]
    #[test]
    fn a_cleaning_driver_decides_binary_after_it_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, ".gitattributes", "*.dat filter=strip\n");
        write_file(dir, "data.dat", "one\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        git_success(dir, &["config", "filter.strip.clean", "tr -d '\\000'"]);
        std::fs::write(dir.join("data.dat"), b"one\n\0two\n").unwrap();

        let stats = open_repo(dir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap();

        assert_eq!(
            counts(stats.unstaged.get(std::path::Path::new("data.dat"))),
            Some((Some(1), Some(0)))
        );
    }

    /// `git lfs track` leaves `.gitattributes` uncommitted at first; the
    /// worktree copy already decides which reads would start the filter.
    #[test]
    fn an_uncommitted_lfs_track_keeps_the_clean_filter_from_running() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "asset.bin", "base\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        write_file(dir, ".gitattributes", "*.bin filter=lfs\n");
        let ran = dir.join("clean-filter-ran");
        git_success(
            dir,
            &[
                "config",
                "filter.lfs.clean",
                &format!("touch '{}'; cat", ran.display()),
            ],
        );
        write_file(dir, "asset.bin", "base\nedited\n");
        write_file(dir, "new.bin", "payload\n");

        let stats = open_repo(dir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap();

        for path in ["asset.bin", "new.bin"] {
            assert_eq!(
                stats.unstaged[std::path::Path::new(path)],
                LineStats::UNKNOWN,
                "{path}"
            );
        }
        assert_eq!(
            counts(stats.unstaged.get(std::path::Path::new(".gitattributes"))),
            Some((Some(1), Some(0)))
        );
        assert!(!ran.exists(), "the LFS clean filter must not run");
    }

    #[test]
    fn lfs_pointer_counts_stay_known_where_no_clean_filter_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        let pointer = format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 5\n",
            "a".repeat(64)
        );
        for path in ["deleted.bin", "plain.txt", "tracked.dat"] {
            write_file(dir, path, &pointer);
        }
        write_file(
            dir,
            ".gitattributes",
            "*.bin filter=lfs\n*.dat filter=lfs\n",
        );
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "pointers"]);
        std::fs::remove_file(dir.join("deleted.bin")).unwrap();
        write_file(dir, "plain.txt", "edited\n");
        write_file(dir, "tracked.dat", "smudged payload\n");
        write_file(dir, "new.txt", "new\n");
        write_file(dir, "new.bin", "new payload\n");
        let unstaged = |path: &str| {
            open_repo(dir)
                .uncommitted_line_stats_impl(&CancellationToken::new())
                .unwrap()
                .unstaged[std::path::Path::new(path)]
        };
        // No LFS driver is configured (git-lfs not installed).
        assert_eq!(
            counts(Some(&unstaged("deleted.bin"))),
            Some((Some(0), Some(3)))
        );
        assert_eq!(
            counts(Some(&unstaged("plain.txt"))),
            Some((Some(1), Some(3)))
        );
        assert_eq!(counts(Some(&unstaged("new.txt"))), Some((Some(1), Some(0))));

        let ran = dir.join("clean-filter-ran");
        git_success(
            dir,
            &[
                "config",
                "filter.lfs.clean",
                &format!("touch '{}'; cat", ran.display()),
            ],
        );
        // A deletion and a path outside `filter=lfs` still read no filter.
        assert_eq!(
            counts(Some(&unstaged("deleted.bin"))),
            Some((Some(0), Some(3)))
        );
        assert_eq!(
            counts(Some(&unstaged("plain.txt"))),
            Some((Some(1), Some(3)))
        );
        assert_eq!(counts(Some(&unstaged("new.txt"))), Some((Some(1), Some(0))));
        assert_eq!(unstaged("tracked.dat"), LineStats::UNKNOWN);
        assert_eq!(unstaged("new.bin"), LineStats::UNKNOWN);
        assert!(!ran.exists(), "the LFS clean filter must not run");
    }

    /// Times six runs of a probe's refresh and prints min, median and max.
    fn print_timing(label: &str, mut refresh: impl FnMut()) {
        let mut ms: Vec<f64> = (0..6)
            .map(|_| {
                let started = std::time::Instant::now();
                refresh();
                started.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        ms.sort_by(f64::total_cmp);
        eprintln!(
            "{label}: min {:.1} ms, median {:.1} ms, max {:.1} ms",
            ms[0],
            ms[ms.len() / 2],
            ms[ms.len() - 1]
        );
    }

    /// Refresh cost of counting untracked files: 4 KiB each, in 200 folders,
    /// text and binary. Files inside the racy window are read on every
    /// refresh; older ones only on the first.
    #[test]
    #[ignore = "timing probe"]
    fn perf_untracked_line_stats_refresh() {
        let body = |seed: usize| -> String {
            (0..64)
                .map(|line| format!("untracked {seed:06} line {line:02} representative text ...\n"))
                .collect()
        };
        for untracked in [2_000usize, 20_000] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path();
            init_test_repo(dir);
            write_file(dir, "seed.txt", "seed\n");
            git_success(dir, &["add", "."]);
            git_success(dir, &["commit", "-q", "-m", "seed"]);
            for ix in 0..untracked {
                write_file(
                    dir,
                    &format!("text/d{:03}/f{ix:06}.txt", ix % 200),
                    &body(ix),
                );
                let binary = dir.join(format!("binary/d{:03}/f{ix:06}.bin", ix % 200));
                std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
                let mut bytes = body(ix).into_bytes();
                bytes[10] = 0;
                std::fs::write(binary, bytes).unwrap();
            }
            let repo = open_repo(dir);
            let token = CancellationToken::new();
            let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
            assert_eq!(entries.len(), 2 * untracked);
            print_timing(&format!("untracked={untracked} x2: status walk"), || {
                std::hint::black_box(repo.worktree_status_cancellable_impl(&token).unwrap());
            });
            for kind in ["text", "binary"] {
                let files: Vec<FileStatus> = entries
                    .iter()
                    .filter(|entry| entry.path.starts_with(kind))
                    .cloned()
                    .collect();
                let _aged = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
                let refresh = || {
                    std::hint::black_box(repo.line_stats_for_entries_impl(&files, &token).unwrap());
                };
                print_timing(
                    &format!("untracked={untracked} {kind}: line stats, first count"),
                    || {
                        *repo.line_stats_memo() = LineStatsMemo::default();
                        refresh();
                    },
                );
                refresh();
                print_timing(
                    &format!("untracked={untracked} {kind}: line stats, unchanged"),
                    refresh,
                );
            }
        }
    }

    /// Refresh cost of untracked binaries: 200 of 2 MiB, read only to be
    /// reported unknown.
    #[test]
    #[ignore = "timing probe"]
    fn perf_untracked_binary_line_stats_refresh() {
        // End-of-line conversion may apply, as in most repositories; and a
        // filter naming no configured driver, like `git lfs track`'s line
        // where git-lfs is not installed.
        for attributes in [
            "* text=auto\n",
            "* text=auto\n*.bin filter=nodriver -text\n",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path();
            init_test_repo(dir);
            write_file(dir, "seed.txt", "seed\n");
            write_file(dir, ".gitattributes", attributes);
            git_success(dir, &["add", "."]);
            git_success(dir, &["commit", "-q", "-m", "seed"]);
            for ix in 0..200u32 {
                let payload: Vec<u8> = (0..2 * 1024 * 1024u32)
                    .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8 ^ ix as u8)
                    .collect();
                std::fs::write(dir.join(format!("blob{ix:03}.bin")), payload).unwrap();
            }
            let repo = open_repo(dir);
            let token = CancellationToken::new();
            let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
            assert_eq!(entries.len(), 200);
            print_timing(&format!("untracked binaries under {attributes:?}"), || {
                let stats = repo.line_stats_for_entries_impl(&entries, &token).unwrap();
                assert!(
                    stats
                        .unstaged
                        .values()
                        .all(|stats| *stats == LineStats::UNKNOWN)
                );
            });
        }
    }

    /// Refresh cost of 200 untracked `working-tree-encoding` files in a
    /// repository of 20,000 tracked ones.
    #[test]
    #[ignore = "timing probe"]
    fn perf_untracked_encoded_line_stats_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(
            dir,
            ".gitattributes",
            "*.u16 working-tree-encoding=UTF-16LE-BOM\n",
        );
        for ix in 0..20_000u32 {
            write_file(
                dir,
                &format!("tracked/d{:03}/f{ix:05}.txt", ix % 200),
                "x\n",
            );
        }
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-q", "-m", "seed"]);
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("a\nb\n".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        for ix in 0..200u32 {
            std::fs::write(dir.join(format!("new{ix:03}.u16")), &utf16).unwrap();
        }
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
        assert_eq!(entries.len(), 200);
        print_timing("untracked encoded", || {
            let stats = repo.line_stats_for_entries_impl(&entries, &token).unwrap();
            assert!(
                stats
                    .unstaged
                    .values()
                    .all(|stats| stats.additions == Some(2))
            );
        });
    }

    /// Refresh cost with real `git-lfs`: 20 modified 1 MiB payloads.
    #[test]
    #[ignore = "timing probe; needs git-lfs"]
    fn perf_lfs_unstaged_line_stats_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        git_success(dir, &["lfs", "install", "--local"]);
        git_success(dir, &["lfs", "track", "*.bin"]);
        let payload = |seed: u8| -> Vec<u8> {
            (0..1024 * 1024u32)
                .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8 ^ seed)
                .collect()
        };
        for ix in 0..20u8 {
            std::fs::write(dir.join(format!("asset{ix:02}.bin")), payload(ix)).unwrap();
        }
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-q", "-m", "payloads"]);
        for ix in 0..20u8 {
            std::fs::write(dir.join(format!("asset{ix:02}.bin")), payload(ix ^ 0x5a)).unwrap();
        }
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        print_timing("lfs unstaged line stats refresh", || {
            let stats = repo.uncommitted_line_stats_impl(&token).unwrap();
            assert_eq!(stats.unstaged.len(), 20);
        });
    }

    #[test]
    fn review_line_stats_memo_separates_identical_pairs_in_each_lane() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        for path in ["staged.txt", "unstaged.txt"] {
            write_file(dir, path, "before\n");
        }
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        for path in ["staged.txt", "unstaged.txt"] {
            write_file(dir, path, "after\n");
        }
        git_success(dir, &["add", "staged.txt"]);
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        take_line_stats_diffs_for_tests();
        let first = repo.uncommitted_line_stats_impl(&token).unwrap();
        assert_eq!(
            take_line_stats_diffs_for_tests(),
            2,
            "each lane must compute its own entry before reusing it"
        );
        assert_eq!(repo.uncommitted_line_stats_impl(&token).unwrap(), first);
        assert_eq!(take_line_stats_diffs_for_tests(), 0);
    }

    #[test]
    fn line_stats_do_not_run_annex_filters_on_ordinary_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "notes.txt", "before\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        // Annex installs a catch-all filter, including files stored in Git.
        write_file(dir, ".gitattributes", "* filter=annex\n");
        git_success(
            dir,
            &["config", "filter.annex.clean", "printf 'filtered\\n'; cat"],
        );
        git_success(dir, &["config", "filter.annex.required", "true"]);
        let repo = open_repo(dir);
        write_file(dir, "notes.txt", "after\nextra\n");
        let entries = [FileStatus {
            path: "notes.txt".into(),
            kind: FileStatusKind::Modified,
            conflict: None,
        }];
        let stats = repo
            .line_stats_for_entries_impl(&entries, &CancellationToken::new())
            .unwrap();
        assert_eq!(
            counts(stats.unstaged.get(std::path::Path::new("notes.txt"))),
            Some((Some(2), Some(1)))
        );
    }

    #[test]
    fn annex_payload_counts_never_reuse_staged_pointer_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        git_success(dir, &["config", "annex.uuid", "test-annex"]);
        for path in ["staged.bin", "unstaged.bin"] {
            write_file(dir, path, "/annex/objects/WORM-s8-m1--payload\n");
        }
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "pointers"]);
        for path in ["staged.bin", "unstaged.bin"] {
            write_file(dir, path, "payload\n");
        }
        git_success(dir, &["add", "staged.bin"]);
        let stats = open_repo(dir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap();
        assert_eq!(
            stats.staged[std::path::Path::new("staged.bin")].additions,
            Some(1)
        );
        assert_eq!(
            stats.unstaged[std::path::Path::new("unstaged.bin")],
            LineStats::UNKNOWN
        );
    }

    #[test]
    fn supplied_status_counts_match_standalone_across_file_kinds() {
        use gitcomet_core::services::GitRepository;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        let token = CancellationToken::new();
        let check = || {
            let repo = open_repo(dir);
            let status = repo.status(&token).unwrap();
            assert_eq!(
                repo.uncommitted_line_stats_for_status(&status, &token)
                    .unwrap(),
                repo.uncommitted_line_stats(&token).unwrap()
            );
        };
        check(); // Empty, unborn repository.
        write_file(dir, ".gitattributes", "*.txt text eol=crlf\n");
        for name in ["edit.txt", "delete.txt", "rename.txt"] {
            write_file(dir, name, &lines("base", 30));
        }
        std::fs::write(dir.join("binary.bin"), b"\0binary").unwrap();
        git_success(dir, &["add", "."]);
        check(); // Staged additions on an unborn HEAD.
        git_success(dir, &["commit", "-m", "seed"]);
        git_success(dir, &["mv", "rename.txt", "renamed.txt"]);
        write_file(dir, "renamed.txt", &lines("base", 31));
        write_file(dir, "edit.txt", "base\r\nstaged\r\n");
        git_success(dir, &["add", "."]);
        write_file(dir, "edit.txt", "base\r\nstaged\r\nunstaged\r\n");
        std::fs::remove_file(dir.join("delete.txt")).unwrap();
        std::fs::write(dir.join("binary.bin"), b"\0changed binary").unwrap();
        write_file(dir, "untracked.txt", "new\n");
        check(); // Mixed staged/unstaged, rename, deletion, binary, CRLF, untracked.
    }

    #[test]
    fn empty_supplied_status_does_not_launch_clean_filter_or_rescan_worktree() {
        use gitcomet_core::services::GitRepository;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "asset.bin", "before\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        write_file(dir, ".gitattributes", "*.bin filter=broken\n");
        git_success(
            dir,
            &[
                "config",
                "filter.broken.clean",
                "gitcomet-intentionally-missing-filter",
            ],
        );
        git_success(dir, &["config", "filter.broken.required", "true"]);
        write_file(dir, "asset.bin", "after!\n");
        std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join("asset.bin"))
            .unwrap()
            .set_times(
                std::fs::FileTimes::new().set_modified(
                    std::time::SystemTime::now() - std::time::Duration::from_secs(120),
                ),
            )
            .unwrap();
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        let empty = gitcomet_core::domain::RepoStatus::default();
        let counts = repo
            .uncommitted_line_stats_for_status(&empty, &token)
            .unwrap();
        assert!(counts.staged.is_empty() && counts.unstaged.is_empty());
        // Control: a traversal actually needs the broken clean filter.
        assert!(repo.worktree_status(&token).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn unstaged_symlinks_have_counts() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        symlink("missing-before", dir.join("link")).unwrap();
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        std::fs::remove_file(dir.join("link")).unwrap();
        symlink("missing-after", dir.join("link")).unwrap();
        let stats = open_repo(dir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap();
        assert_eq!(
            counts(stats.unstaged.get(std::path::Path::new("link"))),
            Some((Some(1), Some(1)))
        );
    }

    /// APFS and HFS+ refuse non-UTF-8 file names with `EILSEQ`, so this
    /// fixture cannot be created on macOS.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn unstaged_non_utf8_paths_have_counts() {
        use std::os::unix::ffi::OsStringExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"file-\xff.txt".to_vec()));
        std::fs::write(dir.join(&path), "before\n").unwrap();
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        std::fs::write(dir.join(&path), "after\n").unwrap();
        let stats = open_repo(dir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap();
        assert_eq!(counts(stats.unstaged.get(&path)), Some((Some(1), Some(1))));
    }

    #[test]
    fn shared_pipeline_applies_each_files_attributes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, ".gitattributes", "*.txt text eol=crlf\nraw/* -text\n");
        write_file(dir, "a.txt", "base\n");
        write_file(dir, "nested/b.txt", "base\n");
        write_file(dir, "raw/c.txt", "base\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        for path in ["a.txt", "nested/b.txt", "raw/c.txt"] {
            write_file(dir, path, "base\r\nadded\r\n");
        }
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
        let stats = repo.line_stats_for_entries_impl(&entries, &token).unwrap();
        for path in ["a.txt", "nested/b.txt"] {
            assert_eq!(
                counts(stats.unstaged.get(std::path::Path::new(path))),
                Some((Some(1), Some(0)))
            );
        }
        assert_eq!(
            counts(stats.unstaged.get(std::path::Path::new("raw/c.txt"))),
            Some((Some(2), Some(1)))
        );
        // The supplied snapshot bounds the scan; it must not rediscover other changes.
        let status = gitcomet_core::domain::RepoStatus {
            unstaged: std::sync::Arc::new(entries[..1].to_vec()),
            staged: Default::default(),
        };
        let subset = gitcomet_core::services::GitRepository::uncommitted_line_stats_for_status(
            &repo, &status, &token,
        )
        .unwrap();
        assert_eq!(subset.unstaged.len(), 1);
    }

    fn lines(prefix: &str, count: usize) -> String {
        (0..count)
            .map(|index| format!("{prefix} line {index}\n"))
            .collect()
    }

    #[test]
    fn counts_both_lanes_independently() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "staged.txt", &lines("base", 5));
        write_file(workdir, "unstaged.txt", &lines("base", 5));
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);

        write_file(workdir, "staged.txt", &lines("base", 7));
        git_success(workdir, &["add", "staged.txt"]);
        write_file(workdir, "unstaged.txt", &lines("base", 4));

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");

        assert_eq!(
            counts(stats.staged.get(std::path::Path::new("staged.txt"))),
            Some((Some(2), Some(0)))
        );
        assert_eq!(
            counts(stats.unstaged.get(std::path::Path::new("unstaged.txt"))),
            Some((Some(0), Some(1)))
        );
        assert!(
            !stats
                .staged
                .contains_key(std::path::Path::new("unstaged.txt")),
            "an unstaged edit must not leak into the staged lane"
        );
    }

    /// Joining `git diff --numstat` by path would report the whole file as
    /// added; gix hands us both ids, so the counts are the real edit.
    #[test]
    fn a_staged_rename_reports_the_edit_not_the_whole_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "before.txt", &lines("base", 200));
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);

        git_success(workdir, &["mv", "before.txt", "after.txt"]);
        write_file(workdir, "after.txt", &lines("base", 201));
        git_success(workdir, &["add", "after.txt"]);

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");
        let renamed = stats
            .staged
            .get(std::path::Path::new("after.txt"))
            .copied()
            .expect("renamed file has counts");

        assert_eq!(renamed.additions, Some(1), "only the appended line is new");
        assert_eq!(renamed.deletions, Some(0));
    }

    /// The index holds nothing for an untracked file, so all of it is added,
    /// as the diff panel shows it.
    #[test]
    fn untracked_files_count_every_line_as_added() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "seed.txt", "seed\n");
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);

        write_file(workdir, "brand-new.txt", &lines("new", 3));
        write_file(workdir, "nested/unterminated.txt", "one\ntwo");
        write_file(workdir, "empty.txt", "");

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");
        let unstaged = |path: &str| counts(stats.unstaged.get(std::path::Path::new(path)));
        assert_eq!(unstaged("brand-new.txt"), Some((Some(3), Some(0))));
        assert_eq!(
            unstaged("nested/unterminated.txt"),
            Some((Some(2), Some(0)))
        );
        assert_eq!(unstaged("empty.txt"), Some((Some(0), Some(0))));
        assert!(stats.staged.is_empty(), "nothing was staged");
    }

    /// `git add -N` moves a new file from untracked to added; what it adds
    /// stays the same.
    #[test]
    fn an_untracked_file_counts_like_the_same_file_added_with_intent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "seed.txt", "seed\n");
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);
        write_file(workdir, "new.rs", "use a;\nfn a() {}\n");
        let new_file = |workdir: &std::path::Path| {
            open_repo(workdir)
                .uncommitted_line_stats_impl(&CancellationToken::new())
                .expect("line stats")
                .unstaged
                .get(std::path::Path::new("new.rs"))
                .copied()
        };

        let untracked = new_file(workdir);
        git_success(workdir, &["add", "-N", "new.rs"]);
        let intent_to_add = new_file(workdir);

        assert_eq!(counts(untracked.as_ref()), Some((Some(2), Some(0))));
        assert!(untracked.is_some_and(|stats| stats.edit.is_some()));
        assert_eq!(untracked, intent_to_add);
    }

    /// An untracked file is read once; later refreshes reuse its counts
    /// while it keeps its stamp.
    #[test]
    fn unchanged_untracked_files_are_not_read_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "seed.txt", "seed\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        write_file(dir, "a.txt", &lines("a", 3));
        write_file(dir, "nested/b.txt", &lines("b", 2));
        let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        take_worktree_reads_for_tests();

        let first = repo.uncommitted_line_stats_impl(&token).unwrap();
        assert_eq!(take_worktree_reads_for_tests(), 2);
        let second = repo.uncommitted_line_stats_impl(&token).unwrap();

        assert_eq!(second, first);
        assert_eq!(
            counts(second.unstaged.get(std::path::Path::new("a.txt"))),
            Some((Some(3), Some(0)))
        );
        // Off Unix a stamp has no inode or ctime, so it is never trusted.
        assert_eq!(
            take_worktree_reads_for_tests(),
            if cfg!(unix) { 0 } else { 2 }
        );
    }

    /// A binary untracked file is sniffed once; later refreshes reuse its
    /// unknown while it keeps its stamp.
    #[test]
    fn unchanged_untracked_binaries_are_not_read_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "seed.txt", "seed\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        std::fs::write(dir.join("a.bin"), b"\0binary").unwrap();
        std::fs::write(dir.join("b.bin"), b"also\0binary").unwrap();
        let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        take_worktree_reads_for_tests();

        repo.uncommitted_line_stats_impl(&token).unwrap();
        assert_eq!(take_worktree_reads_for_tests(), 2);
        let second = repo.uncommitted_line_stats_impl(&token).unwrap();

        for path in ["a.bin", "b.bin"] {
            assert_eq!(
                second.unstaged[std::path::Path::new(path)],
                LineStats::UNKNOWN,
                "{path}"
            );
        }
        // Off Unix a stamp has no inode or ctime, so it is never trusted.
        assert_eq!(
            take_worktree_reads_for_tests(),
            if cfg!(unix) { 0 } else { 2 }
        );
    }

    /// An edit that keeps the size and modification time still changes the
    /// ctime, so the file is counted again.
    #[test]
    fn an_edited_untracked_file_is_counted_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "seed.txt", "seed\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        let path = dir.join("notes.txt");
        std::fs::write(&path, "ab\ncd\n").unwrap();
        let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
        let repo = open_repo(dir);
        let notes = || {
            let stats = repo
                .uncommitted_line_stats_impl(&CancellationToken::new())
                .unwrap();
            counts(stats.unstaged.get(std::path::Path::new("notes.txt")))
        };
        assert_eq!(notes(), Some((Some(2), Some(0))));

        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, "abcde\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();

        assert_eq!(notes(), Some((Some(1), Some(0))));
    }

    /// A file inside the racy window can change again without its stamp
    /// moving, so its counts are not kept.
    #[test]
    fn a_racy_untracked_file_is_read_on_every_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "seed.txt", "seed\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        write_file(dir, "new.txt", &lines("new", 2));
        // Frozen right after the write, however slowly the scans run.
        #[cfg(unix)]
        let _clock = crate::repo::RacyClockSkew::set_for_path(
            dir.join("new.txt"),
            std::time::Duration::ZERO,
        );
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        take_worktree_reads_for_tests();

        let first = repo.uncommitted_line_stats_impl(&token).unwrap();
        let second = repo.uncommitted_line_stats_impl(&token).unwrap();

        assert_eq!(second, first);
        assert_eq!(take_worktree_reads_for_tests(), 2);
    }

    /// The memo keys on how a file converts: without its
    /// `working-tree-encoding`, the same UTF-16 bytes are binary.
    #[test]
    fn an_attribute_change_counts_untracked_files_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(
            dir,
            ".gitattributes",
            "*.txt working-tree-encoding=UTF-16LE-BOM\n",
        );
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("a\nb\nc\n".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        std::fs::write(dir.join("notes.txt"), utf16).unwrap();
        let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
        let repo = open_repo(dir);
        let notes = || {
            repo.uncommitted_line_stats_impl(&CancellationToken::new())
                .unwrap()
                .unstaged[std::path::Path::new("notes.txt")]
        };
        assert_eq!(counts(Some(&notes())), Some((Some(3), Some(0))));

        write_file(dir, ".gitattributes", "*.md text\n");

        assert_eq!(notes(), LineStats::UNKNOWN);
    }

    /// The memo keys on the conversion config: with `core.safecrlf`, a CRLF
    /// file that `core.autocrlf=input` would rewrite cannot be stored.
    #[test]
    fn a_conversion_config_change_counts_untracked_files_again() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "seed.txt", "seed\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        git_success(dir, &["config", "core.autocrlf", "input"]);
        std::fs::write(dir.join("crlf.txt"), "a\r\nb\r\n").unwrap();
        let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
        let repo = open_repo(dir);
        let crlf = || {
            repo.uncommitted_line_stats_impl(&CancellationToken::new())
                .unwrap()
                .unstaged[std::path::Path::new("crlf.txt")]
        };
        assert_eq!(counts(Some(&crlf())), Some((Some(2), Some(0))));

        git_success(dir, &["config", "core.safecrlf", "true"]);

        assert_eq!(crlf(), LineStats::UNKNOWN);
    }

    /// A clean program's output can change while the file does not, so the
    /// files it converts are counted on every refresh.
    #[cfg(unix)]
    #[test]
    fn files_a_clean_program_converts_are_counted_on_every_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, ".gitattributes", "*.dat filter=pick\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        git_success(dir, &["config", "filter.pick.clean", "cat"]);
        write_file(dir, "data.dat", "one\ntwo\n");
        let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
        let repo = open_repo(dir);
        let data = || {
            let stats = repo
                .uncommitted_line_stats_impl(&CancellationToken::new())
                .unwrap();
            counts(stats.unstaged.get(std::path::Path::new("data.dat")))
        };
        assert_eq!(data(), Some((Some(2), Some(0))));

        git_success(dir, &["config", "filter.pick.clean", "sed -n 1p"]);

        assert_eq!(data(), Some((Some(1), Some(0))));
    }

    /// A count kept before git-lfs was configured is not served after it:
    /// the LFS gate decides before the memo is consulted.
    #[test]
    fn an_lfs_driver_configured_after_a_count_leaves_the_file_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, ".gitattributes", "*.bin filter=lfs\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        write_file(dir, "new.bin", "payload\n");
        let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
        let repo = open_repo(dir);
        let new_bin = || {
            repo.uncommitted_line_stats_impl(&CancellationToken::new())
                .unwrap()
                .unstaged[std::path::Path::new("new.bin")]
        };
        // No driver is configured yet, so the file is read as it is.
        assert_eq!(counts(Some(&new_bin())), Some((Some(1), Some(0))));

        let ran = dir.join("clean-filter-ran");
        git_success(
            dir,
            &[
                "config",
                "filter.lfs.clean",
                &format!("touch '{}'; cat", ran.display()),
            ],
        );

        assert_eq!(new_bin(), LineStats::UNKNOWN);
        assert!(!ran.exists(), "the LFS clean filter must not run");
    }

    /// The memo holds only files the last scan listed, so paths that stop
    /// being untracked do not pile up.
    #[cfg(unix)]
    #[test]
    fn untracked_counts_are_kept_only_for_listed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "seed.txt", "seed\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        write_file(dir, "kept.txt", "kept\n");
        write_file(dir, "removed.txt", "removed\n");
        let _clock = crate::repo::RacyClockSkew::set(std::time::Duration::from_secs(30));
        let repo = open_repo(dir);
        let token = CancellationToken::new();
        let memo_paths = || {
            let mut paths: Vec<PathBuf> =
                repo.line_stats_memo().untracked.keys().cloned().collect();
            paths.sort();
            paths
        };
        repo.uncommitted_line_stats_impl(&token).unwrap();
        assert_eq!(
            memo_paths(),
            [PathBuf::from("kept.txt"), PathBuf::from("removed.txt")]
        );

        std::fs::remove_file(dir.join("removed.txt")).unwrap();
        repo.uncommitted_line_stats_impl(&token).unwrap();

        assert_eq!(memo_paths(), [PathBuf::from("kept.txt")]);
    }

    #[test]
    fn binary_and_oversized_files_report_unknown_counts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        std::fs::write(workdir.join("blob.bin"), b"\0\0\0binary\0\0").expect("write binary");
        git_success(workdir, &["add", "blob.bin"]);
        git_success(workdir, &["commit", "-m", "binary"]);
        std::fs::write(workdir.join("blob.bin"), b"\0\0\0changed\0\0").expect("edit binary");
        std::fs::write(workdir.join("new.bin"), b"\0\0\0new\0\0").expect("write binary");
        let oversized = "line\n".repeat(COMMIT_STATS_MAX_BLOB_BYTES / 5 + 1);
        write_file(workdir, "oversized.txt", &oversized);

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");

        for path in ["blob.bin", "new.bin", "oversized.txt"] {
            assert_eq!(
                stats.unstaged.get(std::path::Path::new(path)),
                Some(&LineStats::UNKNOWN),
                "{path} is reported as unknown, not zero"
            );
        }
    }

    /// A repo that closed mid-scan must not keep this running.
    #[test]
    fn a_cancelled_token_stops_the_scan() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "a.txt", &lines("base", 3));
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);
        write_file(workdir, "a.txt", &lines("edit", 3));

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let err = open_repo(workdir)
            .uncommitted_line_stats_impl(&cancelled)
            .expect_err("a cancelled scan must not return counts");
        assert!(
            matches!(err.kind(), gitcomet_core::error::ErrorKind::Cancelled),
            "expected Cancelled, got {err:?}"
        );

        // Control: the guard is not just refusing everything.
        assert!(
            open_repo(workdir)
                .uncommitted_line_stats_impl(&CancellationToken::new())
                .is_ok()
        );
    }

    #[test]
    fn rescans_diff_only_pairs_whose_content_moved() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "big.txt", &lines("base", 200));
        write_file(dir, "staged.txt", &lines("base", 20));
        std::fs::write(dir.join("blob.bin"), b"\0before").unwrap();
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        let edited = |from: usize| lines("base", from) + &lines("edited", 200 - from);
        write_file(dir, "big.txt", &edited(150));
        write_file(dir, "staged.txt", &lines("base", 25));
        git_success(dir, &["add", "staged.txt"]);
        std::fs::write(dir.join("blob.bin"), b"\0after").unwrap();

        let token = CancellationToken::new();
        let scan = |repo: &super::super::GixRepo| {
            let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
            repo.line_stats_for_entries_impl(&entries, &token).unwrap()
        };
        let repo = open_repo(dir);
        take_line_stats_diffs_for_tests();
        let first = scan(&repo);
        // big + staged; a binary worktree file is never diffed.
        assert_eq!(take_line_stats_diffs_for_tests(), 2);

        // An editor save of unchanged content.
        write_file(dir, "big.txt", &edited(150));
        assert_eq!(scan(&repo), first);
        assert_eq!(take_line_stats_diffs_for_tests(), 0);

        write_file(dir, "big.txt", &edited(100));
        let moved = scan(&repo);
        assert_eq!(take_line_stats_diffs_for_tests(), 1);
        assert_eq!(moved, scan(&open_repo(dir)));
        assert_eq!(
            counts(moved.unstaged.get(std::path::Path::new("big.txt"))),
            Some((Some(100), Some(100)))
        );

        // Moving a pair to the staged lane computes that lane's own entry.
        git_success(dir, &["add", "big.txt"]);
        take_line_stats_diffs_for_tests();
        let staged = scan(&repo);
        assert_eq!(take_line_stats_diffs_for_tests(), 1);
        assert_eq!(staged, scan(&open_repo(dir)));
        assert!(
            !staged
                .unstaged
                .contains_key(std::path::Path::new("big.txt"))
        );
    }

    /// A save between two staged walks reuses the first (a cache hit) and
    /// re-seeds the memo as that walk left it: counted pairs stay, unknown
    /// ones (a binary) are never kept, so the next walk sniffs the binary
    /// again and diffs only the newly staged file, save or no save.
    #[test]
    fn a_staged_walk_reuse_keeps_the_memo_as_the_walk_left_it() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        for name in ["a.txt", "b.txt", "c.txt"] {
            write_file(dir, name, &lines("base", 10));
        }
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        write_file(dir, "a.txt", &lines("base", 12));
        std::fs::write(dir.join("blob.bin"), b"\0staged").unwrap();
        git_success(dir, &["add", "a.txt", "blob.bin"]);

        let token = CancellationToken::new();
        let scan = |repo: &super::super::GixRepo| {
            let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
            repo.line_stats_for_entries_impl(&entries, &token).unwrap()
        };
        let repo = open_repo(dir);
        take_line_stats_diffs_for_tests();
        scan(&repo);
        assert_eq!(
            take_line_stats_diffs_for_tests(),
            2,
            "a.txt and the binary sniff"
        );

        write_file(dir, "b.txt", &lines("edited", 10));
        scan(&repo);
        assert_eq!(take_line_stats_diffs_for_tests(), 1, "the save: only b.txt");

        // Distinct content: the memo is keyed by blob ids, not paths, and
        // would serve c.txt from b.txt's pair otherwise.
        write_file(dir, "c.txt", &lines("changed", 10));
        git_success(dir, &["add", "c.txt"]);
        scan(&repo);
        assert_eq!(
            take_line_stats_diffs_for_tests(),
            2,
            "c.txt and the binary sniffed again; a.txt and b.txt from the memo"
        );
    }

    /// A worktree save changes neither HEAD nor the index, so the staged lane
    /// reuses its last walk; staging and committing each walk again.
    #[test]
    fn staged_counts_walk_again_only_when_head_or_index_moves() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "a.txt", &lines("base", 10));
        write_file(dir, "b.txt", &lines("base", 10));
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        write_file(dir, "a.txt", &lines("base", 12));
        git_success(dir, &["add", "a.txt"]);

        let token = CancellationToken::new();
        let scan = |repo: &super::super::GixRepo| {
            let entries = repo.worktree_status_cancellable_impl(&token).unwrap();
            repo.line_stats_for_entries_impl(&entries, &token).unwrap()
        };
        let repo = open_repo(dir);
        take_staged_walks_for_tests();
        let first = scan(&repo);
        assert_eq!(take_staged_walks_for_tests(), 1);

        write_file(dir, "b.txt", &lines("edited", 10));
        let saved = scan(&repo);
        assert_eq!(take_staged_walks_for_tests(), 0, "a save reuses the walk");
        assert_eq!(saved.staged, first.staged);
        assert_eq!(saved, scan(&open_repo(dir)));

        git_success(dir, &["add", "b.txt"]);
        take_staged_walks_for_tests();
        let staged = scan(&repo);
        assert_eq!(take_staged_walks_for_tests(), 1, "staging moves the index");
        assert_eq!(staged, scan(&open_repo(dir)));
        assert!(staged.staged.contains_key(std::path::Path::new("b.txt")));

        git_success(dir, &["commit", "-m", "both"]);
        take_staged_walks_for_tests();
        let committed = scan(&repo);
        assert_eq!(take_staged_walks_for_tests(), 1, "a commit moves HEAD");
        assert!(committed.staged.is_empty());

        // HEAD alone: a soft reset leaves the index file untouched.
        git_success(dir, &["reset", "--soft", "HEAD~1"]);
        let reset = scan(&repo);
        assert_eq!(take_staged_walks_for_tests(), 1, "a reset moves HEAD");
        assert_eq!(reset, scan(&open_repo(dir)));
        assert_eq!(reset.staged.len(), 2);
    }

    #[test]
    fn staged_walk_preserves_cancellation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        init_test_repo(tmp.path());
        write_file(tmp.path(), "a.txt", &lines("staged", 3));
        git_success(tmp.path(), &["add", "."]);

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let repo = open_repo(tmp.path()).repo();
        let err = staged_line_stats(&repo, &mut MemoScan::default(), &cancellation)
            .expect_err("cancelled staged walk");
        assert!(matches!(err.kind(), ErrorKind::Cancelled), "{err:?}");
        assert_eq!(
            staged_line_stats(&repo, &mut MemoScan::default(), &CancellationToken::new())
                .expect("uncancelled staged walk")
                .len(),
            1
        );
    }

    #[test]
    fn staged_counts_work_on_an_unborn_head() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "first.txt", &lines("new", 4));
        git_success(workdir, &["add", "first.txt"]);

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");
        assert_eq!(
            counts(stats.staged.get(std::path::Path::new("first.txt"))),
            Some((Some(4), Some(0)))
        );
    }

    /// A file the status walk listed but that is gone when read has no counts
    /// until the next walk: a gone untracked file is not an empty one, and the
    /// next walk lists a gone tracked file as deleted.
    #[test]
    fn a_file_gone_before_its_read_has_unknown_counts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "tracked.txt", &lines("base", 3));
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);
        std::fs::remove_file(workdir.join("tracked.txt")).expect("remove");
        let entries = [
            FileStatus {
                path: "tracked.txt".into(),
                kind: FileStatusKind::Modified,
                conflict: None,
            },
            FileStatus {
                path: "untracked.txt".into(),
                kind: FileStatusKind::Untracked,
                conflict: None,
            },
        ];

        let stats = open_repo(workdir)
            .line_stats_for_entries_impl(&entries, &CancellationToken::new())
            .expect("line stats");

        for path in ["tracked.txt", "untracked.txt"] {
            assert_eq!(
                stats.unstaged[std::path::Path::new(path)],
                LineStats::UNKNOWN,
                "{path}"
            );
        }
    }

    #[test]
    fn a_deleted_worktree_file_counts_every_line_as_removed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "gone.txt", &lines("base", 3));
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);
        std::fs::remove_file(workdir.join("gone.txt")).expect("remove");

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");
        assert_eq!(
            counts(stats.unstaged.get(std::path::Path::new("gone.txt"))),
            Some((Some(0), Some(3)))
        );
    }
    /// `a.rs` and `nested/b.rs` take the same edit (at different indents),
    /// `c.rs` a different one.
    fn seed_same_edit(workdir: &std::path::Path, from: &str, to: &str) {
        write_file(workdir, "a.rs", &format!("use {from};\nfn a() {{}}\n"));
        write_file(
            workdir,
            "nested/b.rs",
            &format!("    use {from};\nfn b() {{}}\n"),
        );
        write_file(workdir, "c.rs", &format!("use {from};\nfn c() {{}}\n"));
        if !to.is_empty() {
            write_file(workdir, "a.rs", &format!("use {to};\nfn a() {{}}\n"));
            write_file(
                workdir,
                "nested/b.rs",
                &format!("    use {to};\nfn b() {{}}\n"),
            );
            write_file(workdir, "c.rs", &format!("use {to}_other;\nfn c() {{}}\n"));
        }
    }

    fn assert_same_edit(
        edit: impl Fn(&str) -> Option<gitcomet_core::edit_signature::EditSignature>,
    ) {
        let a = edit("a.rs");
        assert!(a.is_some(), "a text edit has a signature");
        assert_eq!(a, edit("nested/b.rs"), "the same edit shares it");
        assert_ne!(a, edit("c.rs"), "a different edit does not");
    }

    fn head_id(workdir: &std::path::Path) -> gitcomet_core::domain::CommitId {
        let repo = gix::open(workdir).expect("open repo");
        gitcomet_core::domain::CommitId(repo.head_id().expect("head").to_string().into())
    }

    #[test]
    fn the_same_edit_shares_a_signature_in_both_uncommitted_lanes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        seed_same_edit(workdir, "old", "");
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);
        seed_same_edit(workdir, "old", "staged");
        git_success(workdir, &["add", "."]);
        seed_same_edit(workdir, "staged", "unstaged");

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");
        for lane in [&stats.staged, &stats.unstaged] {
            assert_same_edit(|path| lane[std::path::Path::new(path)].edit);
        }
        assert_ne!(
            stats.staged[std::path::Path::new("a.rs")].edit,
            stats.unstaged[std::path::Path::new("a.rs")].edit,
            "each lane signs its own edit"
        );
    }

    #[test]
    fn the_same_edit_shares_a_signature_in_a_commit_and_against_the_worktree() {
        use gitcomet_core::services::GitRepository as _;
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        seed_same_edit(workdir, "old", "");
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);
        let seed = head_id(workdir);
        seed_same_edit(workdir, "old", "new");
        git_success(workdir, &["commit", "-am", "edit"]);
        let repo = open_repo(workdir);

        let details = repo.commit_details(&head_id(workdir)).expect("details");
        let committed = |path: &str| {
            details
                .files
                .iter()
                .find(|f| f.path == std::path::Path::new(path))?
                .line_stats
                .edit
        };
        assert_same_edit(committed);

        // The worktree side has no blob; its counts come from numstat, so the
        // edit is read separately.
        seed_same_edit(workdir, "new", "newer");
        let against_worktree = repo
            .compare_files(
                &seed,
                None,
                &gitcomet_core::services::ComparisonOptions::direct(),
                &CancellationToken::new(),
            )
            .expect("comparison");
        let edit = |path: &str| {
            against_worktree
                .files
                .iter()
                .find(|f| f.path == std::path::Path::new(path))?
                .line_stats
                .edit
        };
        assert_same_edit(edit);
        assert_ne!(
            edit("a.rs"),
            committed("a.rs"),
            "measured from the seed, the edit is old to newer"
        );
    }
}
