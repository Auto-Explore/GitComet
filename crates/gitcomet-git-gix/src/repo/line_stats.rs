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
    let mut scratch = CommitStatsScratch::default();

    let result = super::status::tree_index_changes(
        repo,
        &head_tree_id,
        "gix tree/index line stats",
        |change| {
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
    result?;

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
mod tests;
