use super::log::{
    CommitStatsScratch, commit_file_line_stats, commit_stats_looks_binary, line_stats_from_bytes,
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
use rustc_hash::FxHashMap;
use std::path::PathBuf;

/// Mirrors the blob-side cap in `commit_stats`.
pub(super) const WORKTREE_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Whether a worktree entry is a regular file the lanes read whole.
fn within_worktree_cap(metadata: &std::fs::Metadata) -> bool {
    metadata.is_file() && metadata.len() <= WORKTREE_MAX_BYTES
}

/// A worktree file's raw bytes; `None` past the cap or unreadable.
pub(super) fn read_worktree_file_capped(full: &std::path::Path) -> Option<Vec<u8>> {
    std::fs::metadata(full)
        .ok()
        .filter(within_worktree_cap)
        .and_then(|_| std::fs::read(full).ok())
}

type LineStatsKey = (DiffArea, Option<gix::ObjectId>, Option<gix::ObjectId>);

/// The two lanes have different rules for pointers. Keep counts separate,
/// and classify index blobs by id before touching any worktree payload.
#[derive(Default)]
pub(super) struct LineStatsMemo {
    counts: FxHashMap<LineStatsKey, LineStats>,
    pointer_index_blobs: FxHashMap<gix::ObjectId, IndexPointer>,
}

/// What an index blob stands in for, decided from its first bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IndexPointer {
    None,
    Annex,
    Lfs,
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
        // Unknown is cheap to rediscover (size cap, binary sniff) or may be
        // transient (an unreadable object), so it is never kept.
        if entry.additions.is_some() {
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
            previous.counts.extend(next.counts);
            previous
                .pointer_index_blobs
                .extend(next.pointer_index_blobs);
            previous
        };
        result
    }

    /// Edits for a commit-to-worktree file list, whose counts came from
    /// `git diff --numstat` without the contents. Reads both sides under the
    /// caps the counts use, and only for files those could count.
    pub(super) fn add_worktree_edits(
        &self,
        files: &mut [gitcomet_core::domain::CommitFileChange],
        cancellation: &CancellationToken,
    ) -> Result<()> {
        if files.len() > super::log::COMMIT_STATS_MAX_FILES {
            return Ok(());
        }
        let repo = self.status_repo();
        let mut pipeline = repo.filter_pipeline(None).ok();
        let (mut old, mut new) = (Vec::new(), Vec::new());
        for file in files
            .iter_mut()
            .filter(|file| file.edit.is_none() && file.additions.is_some() && !file.is_submodule)
        {
            cancellation.check_cancelled()?;
            let old_id = file
                .old_id
                .as_ref()
                .and_then(|id| gix::ObjectId::from_hex(id.0.as_bytes()).ok());
            if !read_commit_stats_blob(&repo, old_id, &mut old) {
                continue;
            }
            new.clear();
            if file.kind != FileStatusKind::Deleted
                && !read_worktree_git_bytes(self, pipeline.as_mut(), &file.path, &mut new)
            {
                continue;
            }
            file.edit = line_stats_from_bytes(&old, &new).edit;
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
                if stats.additions.is_some() {
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
            cancellation.check_cancelled().or_erased()?;
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
                .or_erased()?;
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
/// filter pipeline so CRLF and clean filters apply before counting.
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
    // Retain attribute caches and reusable filter processes across all files.
    let mut pipeline = repo.filter_pipeline(None).ok();
    let mut lfs_clean = LfsCleanFilter::Unprobed;

    for entry in entries.iter() {
        // Untracked is in neither index lane; conflicted has no single before.
        if matches!(
            entry.kind,
            FileStatusKind::Untracked | FileStatusKind::Conflicted
        ) {
            continue;
        }
        // Per file: this loop reads both sides of each one.
        cancellation.check_cancelled()?;
        let stats = unstaged_entry_line_stats(
            gix_repo,
            repo,
            &index,
            entry,
            &mut index_blob,
            &mut worktree,
            pipeline.as_mut(),
            memo,
            &mut lfs_clean,
        );
        out.insert(entry.path.clone(), stats);
    }

    Ok(out)
}

fn unstaged_entry_line_stats<'repo>(
    gix_repo: &super::GixRepo,
    repo: &'repo gix::Repository,
    index: &gix::index::File,
    entry: &FileStatus,
    index_blob: &mut Vec<u8>,
    worktree: &mut Vec<u8>,
    pipeline: Option<&mut (
        gix::filter::Pipeline<'_>,
        gix::worktree::IndexPersistedOrInMemory,
    )>,
    memo: &mut MemoScan,
    lfs_clean: &mut LfsCleanFilter<'repo>,
) -> LineStats {
    let rel = gix::path::into_bstr(entry.path.as_path());
    let index_id = index.entry_by_path(rel.as_ref()).map(|found| found.id);

    let mut index_blob_loaded = false;
    if let Some(id) = index_id {
        let pointer = memo.index_pointer(id, || {
            let header = repo.find_header(id).ok()?;
            if header.kind() != gix::object::Kind::Blob {
                return None;
            }
            if header.size() > gitcomet_core::annex::POINTER_MAX_BYTES as u64 {
                return Some(IndexPointer::None);
            }
            // A larger blob cannot be a pointer; leave its decompression to
            // a diff miss, after the worktree's size and binary checks.
            index_blob.clear();
            index_blob_loaded = repo.objects.find_blob(&id, index_blob).is_ok();
            index_blob_loaded.then(|| {
                if gitcomet_core::annex::key_from_pointer(index_blob).is_some() {
                    IndexPointer::Annex
                } else if gitcomet_core::lfs::parse_pointer(index_blob).is_some() {
                    IndexPointer::Lfs
                } else {
                    IndexPointer::None
                }
            })
        });
        match pointer {
            // Unlocked content cannot be compared without annex's clean
            // filter. Its size and hash cannot make these counts known.
            Some(IndexPointer::Annex) | None => return LineStats::UNKNOWN,
            // Reading a modified payload would start `git lfs filter-process`
            // and hash (and store) it on every refresh, only to count the two
            // pointer lines that changed. A deletion, or a pointer the filter
            // does not clean, reads no filter and keeps its counts.
            Some(IndexPointer::Lfs)
                if entry.kind != FileStatusKind::Deleted
                    && lfs_clean.runs_for(repo, index, rel.as_ref()) =>
            {
                return LineStats::UNKNOWN;
            }
            Some(_) => {}
        }
    }

    worktree.clear();
    if entry.kind != FileStatusKind::Deleted
        && !read_worktree_git_bytes(gix_repo, pipeline, &entry.path, worktree)
    {
        return LineStats::UNKNOWN;
    }
    // Binary on this side means no counts, whatever the index holds: skip
    // the hash, the index read and the memo.
    if commit_stats_looks_binary(worktree) {
        return LineStats::UNKNOWN;
    }

    let mut diff = || {
        if !index_blob_loaded && !read_commit_stats_blob(repo, index_id, index_blob) {
            return LineStats::UNKNOWN;
        }
        line_stats_from_bytes(index_blob.as_slice(), worktree.as_slice())
    };
    // Hashing costs a fraction of the diff an unchanged file then skips.
    match gix::objs::compute_hash(repo.object_hash(), gix::objs::Kind::Blob, worktree) {
        Ok(worktree_id) => memo.counts((DiffArea::Unstaged, index_id, Some(worktree_id)), diff),
        Err(_) => diff(),
    }
}

/// Reads a worktree file as git would store it. `false` means over the size
/// cap or unreadable; binary is left to `line_stats_from_bytes`.
/// Whether reading a path runs the LFS clean filter: `filter=lfs` selects it
/// and a driver is configured. Probed at the first LFS pointer of a scan.
enum LfsCleanFilter<'repo> {
    Unprobed,
    Off,
    On(Box<LfsAttributes<'repo>>),
}

struct LfsAttributes<'repo> {
    stack: gix::AttributeStack<'repo>,
    outcome: gix::attrs::search::Outcome,
}

impl<'repo> LfsCleanFilter<'repo> {
    fn runs_for(
        &mut self,
        repo: &'repo gix::Repository,
        index: &gix::index::File,
        path: &gix::bstr::BStr,
    ) -> bool {
        if let Self::Unprobed = self {
            let config = repo.config_snapshot();
            let configured = ["filter.lfs.process", "filter.lfs.clean"]
                .into_iter()
                .any(|key| config.string(key).is_some());
            *self = configured
                .then(|| {
                    repo.attributes_only(
                        index,
                        gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
                    )
                    .ok()
                })
                .flatten()
                .map_or(Self::Off, |stack| {
                    Self::On(Box::new(LfsAttributes {
                        outcome: stack.selected_attribute_matches(["filter"]),
                        stack,
                    }))
                });
        }
        let Self::On(attributes) = self else {
            return false;
        };
        let LfsAttributes { stack, outcome } = attributes.as_mut();
        let Ok(platform) = stack.at_entry(path, None) else {
            // Unknown attributes: assume the filter runs, as before.
            return true;
        };
        platform.matching_attributes(outcome);
        outcome.iter_selected().any(|matched| {
            matches!(matched.assignment.state, gix::attrs::StateRef::Value(value) if value.as_bstr() == "lfs")
        })
    }
}

fn read_worktree_git_bytes(
    gix_repo: &super::GixRepo,
    pipeline: Option<&mut (
        gix::filter::Pipeline<'_>,
        gix::worktree::IndexPersistedOrInMemory,
    )>,
    relative: &std::path::Path,
    out: &mut Vec<u8>,
) -> bool {
    use std::io::Read as _;

    #[cfg(test)]
    LINE_STATS_WORKTREE_READS.with(|reads| reads.set(reads.get() + 1));

    let full = gix_repo.spec.workdir.join(relative);
    let Ok(metadata) = std::fs::symlink_metadata(&full) else {
        // Vanished between the status walk and here; empty diffs as a deletion.
        return true;
    };
    if metadata.file_type().is_symlink() {
        let Ok(target) = std::fs::read_link(&full) else {
            return false;
        };
        out.extend_from_slice(gix::path::into_bstr(target).as_ref());
        return true;
    }
    if !within_worktree_cap(&metadata) {
        return false;
    }
    let Some((pipeline, index)) = pipeline else {
        return false;
    };
    let Ok(file) = std::fs::File::open(&full) else {
        return false;
    };
    let Ok(converted) = pipeline.convert_to_git(file, relative, index) else {
        return false;
    };

    use gix::filter::plumbing::pipeline::convert::ToGitOutcome;
    match converted {
        ToGitOutcome::Unchanged(mut file) => file.read_to_end(out).is_ok(),
        ToGitOutcome::Process(mut stream) => stream.read_to_end(out).is_ok(),
        ToGitOutcome::Buffer(bytes) => {
            out.extend_from_slice(bytes);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::status::tests::{git_success, init_test_repo, open_repo, write_file};

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
        LINE_STATS_WORKTREE_READS.with(|reads| reads.set(0));
        for _ in 0..2 {
            assert_eq!(
                repo.line_stats_for_entries_impl(&entries, &CancellationToken::new())
                    .unwrap()
                    .unstaged[std::path::Path::new("asset.bin")],
                LineStats::UNKNOWN
            );
        }
        assert_eq!(
            LINE_STATS_WORKTREE_READS.with(|reads| reads.get()),
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
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "pointers"]);
        // A staged change is pointer against pointer, both already stored.
        write_file(dir, "staged.bin", &pointer('b', 7));
        git_success(dir, &["add", "staged.bin"]);
        let ran = dir.join("clean-filter-ran");
        write_file(dir, ".gitattributes", "*.bin filter=lfs\n");
        git_success(
            dir,
            &[
                "config",
                "filter.lfs.clean",
                &format!("touch '{}'; cat", ran.display()),
            ],
        );
        std::fs::write(dir.join("unstaged.bin"), b"smudged payload\n").unwrap();
        let repo = open_repo(dir);
        LINE_STATS_WORKTREE_READS.with(|reads| reads.set(0));
        for _ in 0..2 {
            let stats = repo
                .uncommitted_line_stats_impl(&CancellationToken::new())
                .unwrap();
            assert_eq!(
                stats.unstaged[std::path::Path::new("unstaged.bin")],
                LineStats::UNKNOWN
            );
            assert_eq!(
                counts(stats.staged.get(std::path::Path::new("staged.bin"))),
                Some((Some(2), Some(2)))
            );
        }
        assert_eq!(LINE_STATS_WORKTREE_READS.with(|reads| reads.get()), 0);
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
        assert_eq!(unstaged("tracked.dat"), LineStats::UNKNOWN);
        assert!(!ran.exists(), "the LFS clean filter must not run");
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
        let mut samples = Vec::new();
        for _ in 0..6 {
            let started = std::time::Instant::now();
            let stats = repo.uncommitted_line_stats_impl(&token).unwrap();
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(stats.unstaged.len(), 20);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "lfs unstaged line stats refresh: min {:.1} ms, median {:.1} ms, max {:.1} ms",
            samples[0], samples[3], samples[5]
        );
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
            let status = repo.status_cancellable(&token).unwrap();
            assert_eq!(
                repo.uncommitted_line_stats_for_status_cancellable(&status, &token)
                    .unwrap(),
                repo.uncommitted_line_stats_cancellable(&token).unwrap()
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
            .uncommitted_line_stats_for_status_cancellable(&empty, &token)
            .unwrap();
        assert!(counts.staged.is_empty() && counts.unstaged.is_empty());
        // Control: a traversal actually needs the broken clean filter.
        assert!(repo.worktree_status_cancellable(&token).is_err());
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
        let subset =
            gitcomet_core::services::GitRepository::uncommitted_line_stats_for_status_cancellable(
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

    #[test]
    fn untracked_and_binary_report_no_counts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "seed.txt", "seed\n");
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-m", "seed"]);

        std::fs::write(workdir.join("blob.bin"), b"\0\0\0binary\0\0").expect("write binary");
        git_success(workdir, &["add", "blob.bin"]);
        git_success(workdir, &["commit", "-m", "binary"]);
        std::fs::write(workdir.join("blob.bin"), b"\0\0\0changed\0\0").expect("edit binary");
        write_file(workdir, "brand-new.txt", "hello\n");

        let stats = open_repo(workdir)
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .expect("line stats");

        assert_eq!(
            stats.unstaged.get(std::path::Path::new("blob.bin")),
            Some(&LineStats::UNKNOWN),
            "binary files are reported as unknown, not zero"
        );
        assert!(
            !stats
                .unstaged
                .contains_key(std::path::Path::new("brand-new.txt")),
            "untracked files are in neither index lane"
        );
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
                .edit
        };
        assert_same_edit(committed);

        // The worktree side has no blob; its counts come from numstat, so the
        // edit is read separately.
        seed_same_edit(workdir, "new", "newer");
        let against_worktree = repo.diff_range_files(&seed, None).expect("range");
        let edit = |path: &str| {
            against_worktree
                .iter()
                .find(|f| f.path == std::path::Path::new(path))?
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
