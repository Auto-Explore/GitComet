use super::{
    GitlinkStatusCapabilityCacheEntry, GixRepo, RepoFileStamp, TreeIndexCacheEntry,
    conflict_stages::conflict_kind_from_stage_mask, git_ops::head_upstream_divergence,
    repo_file_stamp,
};
use crate::util::{
    git_workdir_cmd_for, path_buf_from_git_bytes, push_literal_paths, run_git_background_output,
};
use gitcomet_core::domain::{
    FileConflictKind, FileStatus, FileStatusKind, RepoStatus, UpstreamDivergence,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use gix::error::ResultExt as _;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

mod cancellation;
mod worker_limit;

#[cfg(test)]
#[path = "status/cancellation_tests.rs"]
mod cancellation_tests;

#[cfg(test)]
#[path = "status/refresh_tests.rs"]
mod refresh_tests;

use cancellation::StatusCancellation;

impl GixRepo {
    fn may_have_gitlink_status_supplement(
        &self,
        repo: &gix::Repository,
        index_stamp: &RepoFileStamp,
    ) -> bool {
        let gitmodules = repo_file_stamp(self.spec.workdir.join(".gitmodules").as_path());
        // Key on the content-exact index stamp (not the collision-prone length/mtime one) so the
        // gitlink-capability cache cannot be served stale on an index rewrite whose length and
        // mtime collide — the same hardening the staged-status cache relies on. The caller already
        // computed this stamp (also used for the staged cache), so reuse it rather than re-reading
        // `.git/index`.
        let index = index_stamp.clone();

        if let Some(cached) = self
            .gitlink_status_capability
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .filter(|cached| cached.gitmodules == gitmodules && cached.index == index)
            .map(|cached| cached.may_have_gitlinks)
        {
            return cached;
        }

        let may_have_gitlinks = if gitmodules.exists {
            true
        } else {
            let Ok(index_state) = repo.index_or_empty() else {
                return false;
            };
            index_state
                .entries()
                .iter()
                .any(|entry| entry.mode == gix::index::entry::Mode::COMMIT)
        };

        *self
            .gitlink_status_capability
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(GitlinkStatusCapabilityCacheEntry {
                gitmodules,
                index,
                may_have_gitlinks,
            });
        may_have_gitlinks
    }

    pub(super) fn status_impl(&self) -> Result<RepoStatus> {
        self.status_cancellable_impl(&CancellationToken::new())
    }

    pub(super) fn status_cancellable_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<RepoStatus> {
        cancellation.check_cancelled()?;
        #[cfg(feature = "benchmarks")]
        let timing = crate::status_trace::StageTimer::new("repository-config");
        let repo = self.status_repo();
        #[cfg(feature = "benchmarks")]
        drop(timing);
        // Bare repositories can display history, but have no checkout whose
        // changes we can report. Do not walk the Git directory or compare HEAD
        // against a missing index (which would report every file as deleted).
        if repo.workdir().is_none() {
            return Ok(RepoStatus::default());
        }
        #[cfg(feature = "benchmarks")]
        let timing = crate::status_trace::StageTimer::new("index-fingerprint-gitlinks");
        let index_stamp = repo_index_stamp(&repo);
        let may_have_gitlinks = self.may_have_gitlink_status_supplement(&repo, &index_stamp);
        cancellation.check_cancelled()?;
        #[cfg(feature = "benchmarks")]
        drop(timing);

        // Check whether HEAD and the index file are unchanged since the last
        // status call.  When both match, the staged (Tree→Index) result is
        // identical and we can skip the tree comparison entirely, using the
        // cheaper index-worktree-only iterator.
        #[cfg(feature = "benchmarks")]
        let timing = crate::status_trace::StageTimer::new("staged-status");
        let head_oid = super::history::gix_head_id_or_none(&repo)?;
        cancellation.check_cancelled()?;

        let cached_staged = {
            let guard = self
                .tree_index_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard
                .as_ref()
                .filter(|c| c.head_oid == head_oid && c.index_stamp == index_stamp)
                .map(|c| c.staged.clone())
        };
        let used_cached_staged = cached_staged.is_some();

        let staged = if let Some(cached_staged) = cached_staged {
            cached_staged
        } else {
            let tree = crate::refs::head_tree_id_or_empty(&repo)?;
            collect_staged_status_from_tree_index(&repo, &tree)?
        };
        cancellation.check_cancelled()?;
        #[cfg(feature = "benchmarks")]
        drop(timing);
        // Use the same direct worktree walk on the first scan as on cached
        // scans. The generic status iterator keeps its stat updates private
        // and can only persist them to disk; status must remain read-only.
        // This walk retains clean-file stats in memory immediately instead
        // of hashing touched-but-unchanged files again on the second scan.
        let mut unstaged = Vec::new();
        let direct = collect_index_worktree_status_direct(
            &repo,
            &self.stat_refreshed_index,
            &mut unstaged,
            cancellation,
        )?;
        cancellation.check_cancelled()?;

        #[cfg(feature = "benchmarks")]
        let _timing = crate::status_trace::StageTimer::new("finalize-and-gitlink-supplement");
        if !used_cached_staged {
            *self
                .tree_index_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(TreeIndexCacheEntry {
                head_oid,
                index_stamp,
                staged: staged.clone(),
            });
        }

        cancellation.check_cancelled()?;
        finalize_status(
            &self.spec.workdir,
            &repo,
            may_have_gitlinks,
            staged,
            unstaged,
            direct.has_conflicted_unstaged,
            cancellation,
        )
    }

    pub(super) fn worktree_status_cancellable_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<FileStatus>> {
        cancellation.check_cancelled()?;
        let repo = self.status_repo();
        if repo.workdir().is_none() {
            return Ok(Vec::new());
        }
        let index_stamp = repo_index_stamp(&repo);
        let may_have_gitlinks = self.may_have_gitlink_status_supplement(&repo, &index_stamp);
        let mut unstaged = Vec::new();
        let direct = collect_index_worktree_status_direct(
            &repo,
            &self.stat_refreshed_index,
            &mut unstaged,
            cancellation,
        )?;
        cancellation.check_cancelled()?;

        if should_supplement_unmerged_conflicts(
            crate::refs::operation_state(&repo)?.is_some(),
            direct.has_conflicted_unstaged,
        ) {
            apply_unmerged_conflicts(&repo, &mut unstaged)?;
            cancellation.check_cancelled()?;
        }

        if may_have_gitlinks {
            supplement_gitlink_status_from_porcelain(
                &self.spec.workdir,
                &repo,
                &mut Vec::new(),
                &mut unstaged,
                cancellation,
            )?;
            cancellation.check_cancelled()?;
        }

        sort_and_dedup_status_entries(&mut unstaged);
        Ok(unstaged)
    }

    pub(super) fn staged_status_cancellable_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<FileStatus>> {
        cancellation.check_cancelled()?;
        let repo = self.repo();
        if repo.workdir().is_none() {
            return Ok(Vec::new());
        }
        let head_oid = super::history::gix_head_id_or_none(&repo)?;
        let index_stamp = repo_index_stamp(&repo);
        cancellation.check_cancelled()?;

        if let Some(cached) = self.cached_staged_status(head_oid, &index_stamp) {
            return Ok(cached);
        }

        let Some(head_oid) = head_oid else {
            return self
                .status_cancellable_impl(cancellation)
                .map(|status| std::sync::Arc::unwrap_or_clone(status.staged));
        };

        // `tree_index_status()` diffs a tree against the index, so resolve HEAD to HEAD^{tree}
        // while continuing to cache by commit id.
        let head_tree_id = tree_id_for_commit(&repo, &head_oid)?;
        let mut staged = collect_staged_status_from_tree_index(&repo, &head_tree_id)?;
        cancellation.check_cancelled()?;
        if self.may_have_gitlink_status_supplement(&repo, &index_stamp) {
            supplement_gitlink_status_from_porcelain(
                &self.spec.workdir,
                &repo,
                &mut staged,
                &mut Vec::new(),
                cancellation,
            )?;
            cancellation.check_cancelled()?;
        }
        sort_and_dedup_status_entries(&mut staged);
        remove_conflicted_paths_from_staged(
            &mut staged,
            gix_unmerged_conflicts(&repo)?
                .into_iter()
                .map(|(path, _)| path),
        );
        self.store_staged_status_cache(Some(head_oid), index_stamp, &staged);
        Ok(staged)
    }

    /// Every index path a staged change occupies, both sides of a staged rename
    /// included. `staged_status_impl` reports a rename as its destination alone,
    /// which is all a status list needs but not enough to reset one: the source
    /// path still carries the staged deletion, and resetting only the
    /// destination leaves half a rename in the index.
    ///
    /// Conflicted paths are excluded, as they are from `staged_status_impl`.
    pub(super) fn staged_index_paths_impl(&self) -> Result<Vec<PathBuf>> {
        let repo = self.repo();
        let Some(head_oid) = super::history::gix_head_id_or_none(&repo)? else {
            return Ok(self
                .status_impl()?
                .staged
                .iter()
                .map(|entry| entry.path.clone())
                .collect());
        };

        let head_tree_id = tree_id_for_commit(&repo, &head_oid)?;
        let mut paths = collect_staged_index_paths_from_tree_index(&repo, &head_tree_id)?;
        let conflicted: FxHashSet<PathBuf> = gix_unmerged_conflicts(&repo)?
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        paths.retain(|path| !conflicted.contains(path));
        paths.sort_unstable();
        paths.dedup();
        Ok(paths)
    }

    pub(super) fn upstream_divergence_cancellable_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Option<UpstreamDivergence>> {
        cancellation.check_cancelled()?;
        // A fresh open so upstream config written since this repo was opened
        // (`branch.<name>.remote`/`merge`) is honoured; the walk itself is
        // memoized by tip pair, which is where the time went.
        let repo = self.reopen_repo()?;
        head_upstream_divergence(&repo, &self.divergence_cache, Some(cancellation))
    }

    fn cached_staged_status(
        &self,
        head_oid: Option<gix::ObjectId>,
        index_stamp: &RepoFileStamp,
    ) -> Option<Vec<FileStatus>> {
        let guard = self
            .tree_index_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .as_ref()
            .filter(|cached| cached.head_oid == head_oid && &cached.index_stamp == index_stamp)
            .map(|cached| cached.staged.clone())
    }

    fn store_staged_status_cache(
        &self,
        head_oid: Option<gix::ObjectId>,
        index_stamp: RepoFileStamp,
        staged: &[FileStatus],
    ) {
        *self
            .tree_index_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(TreeIndexCacheEntry {
            head_oid,
            index_stamp,
            staged: staged.to_vec(),
        });
    }
}

fn should_supplement_unmerged_conflicts(
    repo_has_in_progress_state: bool,
    has_conflicted_unstaged: bool,
) -> bool {
    repo_has_in_progress_state || has_conflicted_unstaged
}

fn finalize_status(
    workdir: &Path,
    repo: &gix::Repository,
    may_have_gitlinks: bool,
    mut staged: Vec<FileStatus>,
    mut unstaged: Vec<FileStatus>,
    has_conflicted_unstaged: bool,
    cancellation: &CancellationToken,
) -> Result<RepoStatus> {
    // Some platforms may omit certain unmerged shapes (notably stage-1-only both-deleted
    // conflicts) from gix status output. Supplement conflict entries from the index's unmerged
    // stages only when the repository is in an in-progress operation or gix already surfaced
    // conflicts.
    if should_supplement_unmerged_conflicts(
        crate::refs::operation_state(repo)?.is_some(),
        has_conflicted_unstaged,
    ) {
        apply_unmerged_conflicts(repo, &mut unstaged)?;
    }

    // Only shell out for gitlink/submodule status when the repo is likely to contain submodules
    // or gitlinks. This avoids a full `git status` subprocess on every refresh for the common
    // case.
    if may_have_gitlinks {
        supplement_gitlink_status_from_porcelain(
            workdir,
            repo,
            &mut staged,
            &mut unstaged,
            cancellation,
        )?;
    }
    cancellation.check_cancelled()?;

    sort_and_dedup_status_entries(&mut staged);
    sort_and_dedup_status_entries(&mut unstaged);
    remove_conflicted_paths_from_staged(
        &mut staged,
        unstaged
            .iter()
            .filter(|entry| entry.kind == FileStatusKind::Conflicted)
            .map(|entry| entry.path.clone()),
    );

    Ok(RepoStatus {
        staged: std::sync::Arc::new(staged),
        unstaged: std::sync::Arc::new(unstaged),
    })
}

fn apply_unmerged_conflicts(repo: &gix::Repository, unstaged: &mut Vec<FileStatus>) -> Result<()> {
    merge_unmerged_conflicts(unstaged, gix_unmerged_conflicts(repo)?);
    Ok(())
}

/// Marks each conflicted path's first unstaged entry, or adds one.
fn merge_unmerged_conflicts(
    unstaged: &mut Vec<FileStatus>,
    conflicts: Vec<(PathBuf, FileConflictKind)>,
) {
    if conflicts.is_empty() {
        return;
    }
    // Conflict paths are distinct, so indexing the entries up front finds the
    // same entry a scan per conflict did.
    let targets: Vec<Option<usize>> = {
        let mut first = FxHashMap::with_capacity_and_hasher(unstaged.len(), Default::default());
        for (ix, entry) in unstaged.iter().enumerate() {
            first.entry(entry.path.as_path()).or_insert(ix);
        }
        conflicts
            .iter()
            .map(|(path, _)| first.get(path.as_path()).copied())
            .collect()
    };
    for ((path, conflict_kind), target) in conflicts.into_iter().zip(targets) {
        match target {
            Some(ix) => {
                unstaged[ix].kind = FileStatusKind::Conflicted;
                unstaged[ix].conflict = Some(conflict_kind);
            }
            None => unstaged.push(FileStatus {
                path,
                kind: FileStatusKind::Conflicted,
                conflict: Some(conflict_kind),
            }),
        }
    }
}

pub(super) fn tree_id_for_commit(
    repo: &gix::Repository,
    commit_id: &gix::ObjectId,
) -> Result<gix::ObjectId> {
    repo.find_commit(*commit_id)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit lookup: {e}"))))?
        .tree_id()
        .map(|id| id.detach())
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit tree id: {e}"))))
}

fn collect_staged_status_from_tree_index(
    repo: &gix::Repository,
    head_oid: &gix::ObjectId,
) -> Result<Vec<FileStatus>> {
    let index = repo
        .index_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    let mut staged = Vec::new();
    repo.tree_index_status(
        head_oid,
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
            collect_tree_index_change(change, &mut staged).or_error()?;
            Ok(std::ops::ControlFlow::Continue(()))
        },
    )
    .map_err(|e| Error::new(ErrorKind::Backend(format!("gix tree/index status: {e}"))))?;
    Ok(staged)
}

fn collect_staged_index_paths_from_tree_index(
    repo: &gix::Repository,
    head_tree_id: &gix::ObjectId,
) -> Result<Vec<PathBuf>> {
    let index = repo
        .index_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    let mut paths = Vec::new();
    repo.tree_index_status(
        head_tree_id,
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
            collect_tree_index_change_paths(change, &mut paths).or_error()?;
            Ok(std::ops::ControlFlow::Continue(()))
        },
    )
    .map_err(|e| Error::new(ErrorKind::Backend(format!("gix tree/index status: {e}"))))?;
    Ok(paths)
}

/// Collect the index paths a single TreeIndex change occupies.
fn collect_tree_index_change_paths(
    change: gix::diff::index::ChangeRef<'_, '_>,
    paths: &mut Vec<PathBuf>,
) -> Result<()> {
    use gix::diff::index::ChangeRef;

    match change {
        ChangeRef::Addition { location, .. } => paths.push(path_buf_from_git_bytes(
            location.as_ref(),
            "gix status staged addition path",
        )?),
        ChangeRef::Deletion { location, .. } => paths.push(path_buf_from_git_bytes(
            location.as_ref(),
            "gix status staged deletion path",
        )?),
        ChangeRef::Modification { location, .. } => paths.push(path_buf_from_git_bytes(
            location.as_ref(),
            "gix status staged modification path",
        )?),
        ChangeRef::Rewrite {
            location,
            source_location,
            copy,
            ..
        } => {
            paths.push(path_buf_from_git_bytes(
                location.as_ref(),
                "gix status staged rewrite path",
            )?);
            // A rename stages a deletion of the source; a copy leaves the source
            // exactly as HEAD has it, so it is not staged and must not be reset.
            if !copy {
                paths.push(path_buf_from_git_bytes(
                    source_location.as_ref(),
                    "gix status staged rewrite source path",
                )?);
            }
        }
    }
    Ok(())
}

fn kind_priority(kind: FileStatusKind) -> u8 {
    match kind {
        FileStatusKind::Conflicted => 5,
        FileStatusKind::Renamed => 4,
        FileStatusKind::Deleted => 3,
        FileStatusKind::Added => 2,
        FileStatusKind::Modified => 1,
        FileStatusKind::Untracked => 0,
    }
}

fn sort_and_dedup_status_entries(entries: &mut Vec<FileStatus>) {
    entries.sort_unstable_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| kind_priority(b.kind).cmp(&kind_priority(a.kind)))
    });
    entries.dedup_by(|a, b| a.path == b.path);
}

fn remove_conflicted_paths_from_staged(
    staged: &mut Vec<FileStatus>,
    conflicted: impl IntoIterator<Item = PathBuf>,
) {
    let conflicted: FxHashSet<PathBuf> = conflicted.into_iter().collect();
    if !conflicted.is_empty() {
        staged.retain(|entry| !conflicted.contains(&entry.path));
    }
}

/// File stamp for `.git/index`, hardened so the staged-status cache is invalidated
/// whenever the index changes even when length and mtime collide (atomic rewrite of
/// the same tracked entries on a filesystem with coarse or cached timestamps, e.g.
/// f2fs). Combines the trailing content hash (content-exact when present) with the
/// inode + ctime, which change on every lock-file + rename index rewrite and so also
/// cover `index.skipHash` repositories whose trailer is a useless null hash.
///
/// Opens `.git/index` a single time and derives every field from that one handle.
pub(super) fn repo_index_stamp(repo: &gix::Repository) -> RepoFileStamp {
    index_stamp_for(repo.index_path().as_path(), repo.object_hash())
}

fn index_stamp_for(path: &Path, hash_kind: gix::hash::Kind) -> RepoFileStamp {
    // A genuinely-absent index is a stable state, so the all-empty default (exists=false) is a safe,
    // cacheable stamp. But if the index *exists yet is momentarily unreadable* (a permission flip, or
    // a Windows sharing / AV lock) we must NOT fall back to the length+mtime stat stamp: during that
    // window an atomic rewrite with an identical length and unchanged/coarse mtime would make two
    // such stamps compare equal and serve a stale cache hit — the exact collision this stamp exists
    // to prevent. Return an uncacheable stamp so the read is forced fresh until the index is
    // readable again.
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return RepoFileStamp::default();
        }
        Err(_) => return RepoFileStamp::uncacheable(),
    };
    let Ok(metadata) = file.metadata() else {
        return RepoFileStamp::uncacheable();
    };

    let mut stamp = RepoFileStamp {
        exists: true,
        len: metadata.len(),
        modified: metadata.modified().ok(),
        content_id: read_index_trailing_hash(&mut file, metadata.len(), hash_kind),
        ..RepoFileStamp::default()
    };
    set_index_stat_discriminators(&mut stamp, &metadata);
    stamp
}

/// Reads the trailing checksum that git/gix append to `.git/index` (a hash over the
/// entire index content), from an already-open handle. Returns `None` if the file is
/// truncated, the trailer cannot be read, or it is the null hash (written by
/// `index.skipHash`, where it cannot distinguish index states). Callers then rely on
/// the stat discriminators / length / mtime, which only risk a redundant recompute,
/// never a stale cache hit.
fn read_index_trailing_hash(
    file: &mut std::fs::File,
    len: u64,
    hash_kind: gix::hash::Kind,
) -> Option<gix::ObjectId> {
    use std::io::{Read, Seek, SeekFrom};

    let hash_len = hash_kind.len_in_bytes();
    if len < hash_len as u64 {
        return None;
    }
    file.seek(SeekFrom::End(-(hash_len as i64))).ok()?;
    // Stack buffer sized for the longest supported hash (SHA-256 = 32 bytes); avoids a heap
    // allocation on this per-status-refresh path. `get_mut` also guards `hash_len <= 32`.
    let mut buf = [0u8; 32];
    let buf = buf.get_mut(..hash_len)?;
    file.read_exact(buf).ok()?;
    let oid = gix::ObjectId::try_from(&*buf).ok()?;
    (!oid.is_null()).then_some(oid)
}

#[cfg(unix)]
fn set_index_stat_discriminators(stamp: &mut RepoFileStamp, metadata: &std::fs::Metadata) {
    use std::os::unix::fs::MetadataExt;
    stamp.inode = Some(metadata.ino());
    stamp.ctime_nanos =
        Some((metadata.ctime() as i128) * 1_000_000_000 + metadata.ctime_nsec() as i128);
}

#[cfg(not(unix))]
fn set_index_stat_discriminators(_stamp: &mut RepoFileStamp, _metadata: &std::fs::Metadata) {}

pub(super) fn gix_unmerged_conflicts(
    repo: &gix::Repository,
) -> Result<Vec<(PathBuf, FileConflictKind)>> {
    let index = crate::refs::index_or_load_from_head_or_empty(repo)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))).with_source(e))?;
    let path_backing = index.path_backing();
    let mut stage_entries = Vec::new();

    for entry in index.entries() {
        let stage = entry.stage_raw() as u8;
        if !(1..=3).contains(&stage) {
            continue;
        }

        let path = path_buf_from_git_bytes(
            entry.path_in(path_backing).as_ref(),
            "gix index unmerged conflict path",
        )?;
        stage_entries.push((path, stage));
    }

    Ok(collect_unmerged_conflicts(stage_entries))
}

fn collect_unmerged_conflicts(
    stage_entries: impl IntoIterator<Item = (PathBuf, u8)>,
) -> Vec<(PathBuf, FileConflictKind)> {
    let stage_entries = stage_entries.into_iter();
    let mut stage_masks =
        FxHashMap::with_capacity_and_hasher(stage_entries.size_hint().0, Default::default());

    for (path, stage) in stage_entries {
        let Some(shift) = stage.checked_sub(1) else {
            continue;
        };
        if shift > 2 {
            continue;
        }

        let bit = 1u8 << shift;
        stage_masks
            .entry(path)
            .and_modify(|mask| *mask |= bit)
            .or_insert(bit);
    }

    let mut conflicts = stage_masks
        .into_iter()
        .filter_map(|(path, mask)| conflict_kind_from_stage_mask(mask).map(|kind| (path, kind)))
        .collect::<Vec<_>>();
    conflicts.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    conflicts
}

/// The on-disk index with the stats a worktree walk found stale but
/// content-clean, as `git status` would write them back. Status never writes
/// `.git/index`, so without this
/// every refresh re-hashes each file that was touched without changing, which
/// is ~20x the walk with `text=auto` filtering.
pub(super) struct StatRefreshedIndex {
    /// gix's snapshot of `.git/index` this copy was derived from; any rewrite
    /// of the file replaces the snapshot and drops the copy.
    base: gix::worktree::Index,
    index: gix::index::File,
}

/// Avoid a second large index for a few inexpensive content checks.
const STAT_REFRESH_MIN_BYTES: u64 = 256 * 1024;
const STAT_REFRESH_MIN_ENTRIES: usize = 64;
/// Small indexes are cheap to retain, even when a few small files took costly
/// external clean filters to verify. Byte count alone does not capture that cost.
const STAT_REFRESH_SMALL_INDEX_ENTRIES: usize = 128;

fn collect_index_worktree_status_direct(
    repo: &gix::Repository,
    stat_refresh: &std::sync::Mutex<Option<StatRefreshedIndex>>,
    unstaged: &mut Vec<FileStatus>,
    cancellation: &CancellationToken,
) -> Result<DirectIndexWorktreeStatus> {
    #[cfg(feature = "benchmarks")]
    let timing = crate::status_trace::StageTimer::new("index-snapshot");
    let index = repo
        .index_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    let refreshed = stat_refresh
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .filter(|refreshed| std::sync::Arc::ptr_eq(&refreshed.base, &index));
    // Every stat the walk records is read after this instant. gix compares
    // the stamp with mtimes in whole seconds, and a write landing just after
    // it can carry a coarser mtime from the second before (the file clock
    // lags the wall clock), so the stamp stays one second back: such a write
    // still compares racy, as it would against git's own index mtime.
    let walk_started = filetime::FileTime::now();
    #[cfg(test)]
    let walk_started = tests::STAT_REFRESH_CLOCK.with(|clock| clock.get().unwrap_or(walk_started));
    let stamp = filetime::FileTime::from_unix_time(walk_started.unix_seconds() - 1, 0);
    #[cfg(feature = "benchmarks")]
    drop(timing);
    let result = collect_index_worktree_status_direct_from_index(
        repo,
        refreshed
            .as_ref()
            .map_or(&index, |refreshed| &refreshed.index),
        unstaged,
        cancellation,
    )
    .and_then(|result| {
        cancellation.check_cancelled()?;
        Ok(result)
    });
    let (status, changes) = match result {
        Ok(result) => result,
        Err(error) => {
            // An interrupted walk cannot commit partial stat updates. Keep the
            // previous complete cache unless another walk has already replaced it.
            let mut cache = stat_refresh.lock().unwrap_or_else(|e| e.into_inner());
            if cache.is_none() {
                *cache = refreshed;
            }
            return Err(error);
        }
    };
    #[cfg(feature = "benchmarks")]
    let _timing = crate::status_trace::StageTimer::new("retain-index-stats");
    let refreshed = apply_stat_refresh(refreshed, &index, changes, stamp);
    *stat_refresh
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = refreshed;
    Ok(status)
}

/// Applies a walk's stat updates to the in-memory copy, as git's index write
/// would: new stats for content-clean entries, size 0 for racily modified ones
/// (so they keep comparing as changed once the timestamp moves past them).
fn apply_stat_refresh(
    refreshed: Option<StatRefreshedIndex>,
    base: &gix::worktree::Index,
    changes: Vec<IndexWorktreeApplyChange>,
    stamp: filetime::FileTime,
) -> Option<StatRefreshedIndex> {
    if changes.is_empty() {
        return refreshed;
    }
    let mut refreshed = match refreshed {
        Some(refreshed) => refreshed,
        None => {
            let rehashed_bytes: u64 = changes
                .iter()
                .map(|change| match change {
                    IndexWorktreeApplyChange::NewStat { stat, .. } => u64::from(stat.size),
                    IndexWorktreeApplyChange::SetSizeToZero { .. } => 0,
                })
                .sum();
            if base.entries().len() > STAT_REFRESH_SMALL_INDEX_ENTRIES
                && rehashed_bytes < STAT_REFRESH_MIN_BYTES
                && changes.len() < STAT_REFRESH_MIN_ENTRIES
            {
                return None;
            }
            StatRefreshedIndex {
                base: base.clone(),
                index: (***base).clone(),
            }
        }
    };
    let entries = refreshed.index.entries_mut();
    for change in changes {
        match change {
            IndexWorktreeApplyChange::NewStat { entry_index, stat } => {
                if let Some(entry) = entries.get_mut(entry_index) {
                    entry.stat = stat;
                }
            }
            IndexWorktreeApplyChange::SetSizeToZero { entry_index } => {
                if let Some(entry) = entries.get_mut(entry_index) {
                    entry.stat.size = 0;
                }
            }
        }
    }
    refreshed.index.set_timestamp(stamp);
    Some(refreshed)
}

fn collect_index_worktree_status_direct_from_index(
    repo: &gix::Repository,
    index: &gix::index::File,
    unstaged: &mut Vec<FileStatus>,
    cancellation: &CancellationToken,
) -> Result<(DirectIndexWorktreeStatus, Vec<IndexWorktreeApplyChange>)> {
    #[cfg(feature = "benchmarks")]
    let timing = crate::status_trace::StageTimer::new("walk-options");
    let dirwalk_options = repo
        .dirwalk_options()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix status dirwalk options: {e}"
            )))
        })?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching);
    #[cfg(feature = "benchmarks")]
    drop(timing);
    // Git supplies child status below. The previous built-in provider used
    // Ignore::All, which also always returns None but still loads .gitmodules
    // and inspects each child's repository layout before ignoring the result.
    let collection = collect_index_worktree_status_direct_with_submodule(
        repo,
        index,
        dirwalk_options,
        unstaged,
        NoopSubmoduleStatus,
        cancellation,
    )?;
    Ok((
        DirectIndexWorktreeStatus {
            has_conflicted_unstaged: collection.has_conflicted_unstaged,
            // Status stays read-only: the stat refresh lives in memory only.
        },
        collection.index_changes,
    ))
}

#[derive(Clone, Copy)]
struct NoopSubmoduleStatus;

impl gix::status::plumbing::index_as_worktree::traits::SubmoduleStatus for NoopSubmoduleStatus {
    type Output = gix::submodule::Status;

    fn status(
        &mut self,
        _entry: &gix::index::Entry,
        _rela_path: &gix::bstr::BStr,
    ) -> gix::Result<Option<Self::Output>> {
        Ok(None)
    }
}

fn filesystem_cache_enabled(repo: &gix::Repository) -> bool {
    // Match gix's normal status path. Repository opens use its lenient config
    // policy, so an absent or invalid value gets the platform default.
    repo.config_snapshot()
        .boolean(gix::config::tree::Core::FS_CACHE)
        .unwrap_or(cfg!(windows))
}

fn collect_index_worktree_status_direct_with_submodule<S>(
    repo: &gix::Repository,
    index: &gix::index::File,
    dirwalk_options: gix::dirwalk::Options,
    unstaged: &mut Vec<FileStatus>,
    submodule: S,
    cancellation: &CancellationToken,
) -> Result<StatusEntryCollection>
where
    S: gix::status::plumbing::index_as_worktree::traits::SubmoduleStatus<
            Output = gix::submodule::Status,
        > + Send
        + Clone,
{
    #[cfg(feature = "benchmarks")]
    let timing = crate::status_trace::StageTimer::new("attributes-and-excludes");
    let workdir = repo
        .workdir()
        .ok_or_else(|| Error::new(ErrorKind::Backend("gix status missing workdir".into())))?;
    let attrs_and_excludes = repo
        .attributes(
            index,
            gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
            gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            None,
        )
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix status attributes: {e}"))))?;
    #[cfg(feature = "benchmarks")]
    drop(timing);
    #[cfg(feature = "benchmarks")]
    let timing = crate::status_trace::StageTimer::new("pathspec-realpath-fs-options");
    let (pathspec, _pathspec_attr_stack) = gix::Pathspec::new(
        repo,
        false,
        std::iter::empty::<gix::bstr::BString>(),
        true,
        || -> gix::Result<gix::worktree::Stack> {
            unreachable!("empty direct-status patterns never require pathspec attributes")
        },
    )
    .map_err(|e| Error::new(ErrorKind::Backend(format!("gix status pathspec: {e}"))))?
    .into_parts();
    let git_dir_realpath = gix::path::realpath_opts(
        repo.git_dir(),
        repo.current_dir(),
        gix::path::realpath::MAX_SYMLINKS,
    )
    .map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix status git dir realpath: {e}"
        )))
    })?;
    let fs_caps = repo
        .filesystem_options()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix status fs options: {e}"))))?;
    let fscache = filesystem_cache_enabled(repo);
    #[cfg(feature = "benchmarks")]
    drop(timing);
    #[cfg(feature = "benchmarks")]
    let timing = crate::status_trace::StageTimer::new("case-insensitive-index");
    let accelerate_lookup = fs_caps.ignore_case.then(|| index.prepare_icase_backing());
    #[cfg(feature = "benchmarks")]
    drop(timing);
    #[cfg(feature = "benchmarks")]
    let timing = crate::status_trace::StageTimer::new("resource-cache");
    let resource_cache = gix::diff::resource_cache(
        repo,
        gix::diff::blob::pipeline::Mode::ToGit,
        attrs_and_excludes.detach(),
        gix::diff::blob::pipeline::WorktreeRoots {
            old_root: None,
            new_root: Some(workdir.to_owned()),
        },
    )
    .map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix status resource cache: {e}"
        )))
    })?;
    #[cfg(feature = "benchmarks")]
    drop(timing);
    #[cfg(feature = "benchmarks")]
    let timing = crate::status_trace::StageTimer::new("cancellation-watcher");
    let mut collector = StatusEntryCollector::new(unstaged);
    let interrupt = StatusCancellation::new(cancellation)?;
    let mut progress = cancellation::progress(&interrupt.interrupt);
    #[cfg(feature = "benchmarks")]
    drop(timing);
    #[cfg(feature = "benchmarks")]
    let timing = crate::status_trace::StageTimer::new("worker-policy");
    let thread_limit = worker_limit::for_index(repo, index, cancellation)?;
    #[cfg(feature = "benchmarks")]
    drop(timing);
    #[cfg(feature = "benchmarks")]
    let timing = crate::status_trace::StageTimer::new("native-walk");
    let outcome = gix::status::plumbing::index_as_worktree_with_renames(
        index,
        workdir,
        &mut collector,
        gix::status::plumbing::index_as_worktree::traits::FastEq,
        submodule,
        repo.objects
            .clone()
            .into_arc()
            .expect("arc conversion always works"),
        &mut progress,
        gix::status::plumbing::index_as_worktree_with_renames::Context {
            pathspec,
            resource_cache,
            should_interrupt: &interrupt.interrupt,
            dirwalk: gix::status::plumbing::index_as_worktree_with_renames::DirwalkContext {
                git_dir_realpath: git_dir_realpath.as_path(),
                current_dir: repo.current_dir(),
                ignore_case_index_lookup: accelerate_lookup.as_ref(),
            },
        },
        gix::status::plumbing::index_as_worktree_with_renames::Options {
            sorting: None,
            fscache,
            tracked_file_modifications: gix::status::plumbing::index_as_worktree::Options {
                fs: fs_caps,
                thread_limit,
                fscache,
                stat: repo.stat_options().map_err(|e| {
                    Error::new(ErrorKind::Backend(format!("gix status stat options: {e}")))
                })?,
            },
            dirwalk: Some(dirwalk_options.into()),
            rewrites: None,
        },
    )
    .map_err(|e| {
        if cancellation.is_cancelled() {
            Error::new(ErrorKind::Cancelled)
        } else {
            Error::new(ErrorKind::Backend(format!(
                "gix status index/worktree: {e}"
            )))
        }
    })?;
    #[cfg(feature = "benchmarks")]
    {
        drop(timing);
        crate::status_trace::record_walk(&outcome, fscache, thread_limit);
    }
    #[cfg(not(feature = "benchmarks"))]
    let _ = outcome;
    cancellation.check_cancelled()?;

    collector.finish()
}

struct StatusEntryCollector<'a> {
    unstaged: &'a mut Vec<FileStatus>,
    has_conflicted_unstaged: bool,
    index_changes: Vec<IndexWorktreeApplyChange>,
    error: Option<Error>,
}

impl<'a> StatusEntryCollector<'a> {
    fn new(unstaged: &'a mut Vec<FileStatus>) -> Self {
        Self {
            unstaged,
            has_conflicted_unstaged: false,
            index_changes: Vec::new(),
            error: None,
        }
    }

    fn finish(self) -> Result<StatusEntryCollection> {
        if let Some(err) = self.error {
            Err(err)
        } else {
            Ok(StatusEntryCollection {
                has_conflicted_unstaged: self.has_conflicted_unstaged,
                index_changes: self.index_changes,
            })
        }
    }
}

impl<'a, 'index> gix::status::plumbing::index_as_worktree_with_renames::VisitEntry<'index>
    for StatusEntryCollector<'a>
{
    type ContentChange = ();
    type SubmoduleStatus = gix::submodule::Status;

    fn visit_entry(
        &mut self,
        entry: gix::status::plumbing::index_as_worktree_with_renames::Entry<
            'index,
            Self::ContentChange,
            Self::SubmoduleStatus,
        >,
    ) {
        if self.error.is_some() {
            return;
        }
        if let Err(err) = collect_index_worktree_status_entry(
            entry,
            self.unstaged,
            &mut self.has_conflicted_unstaged,
            &mut self.index_changes,
        ) {
            self.error = Some(err);
        }
    }
}

fn collect_index_worktree_status_entry<U>(
    entry: gix::status::plumbing::index_as_worktree_with_renames::Entry<'_, (), U>,
    unstaged: &mut Vec<FileStatus>,
    has_conflicted_unstaged: &mut bool,
    index_changes: &mut Vec<IndexWorktreeApplyChange>,
) -> Result<()> {
    match entry {
        gix::status::plumbing::index_as_worktree_with_renames::Entry::Modification {
            entry_index,
            rela_path,
            status,
            ..
        } => {
            if let gix::status::plumbing::index_as_worktree::EntryStatus::NeedsUpdate(stat) =
                &status
            {
                index_changes.push(IndexWorktreeApplyChange::NewStat {
                    entry_index,
                    stat: *stat,
                });
                return Ok(());
            }
            if matches!(
                &status,
                gix::status::plumbing::index_as_worktree::EntryStatus::Change(
                    gix::status::plumbing::index_as_worktree::Change::Modification {
                        set_entry_stat_size_zero: true,
                        ..
                    },
                )
            ) {
                index_changes.push(IndexWorktreeApplyChange::SetSizeToZero { entry_index });
            }
            let path = path_buf_from_git_bytes(
                rela_path.as_ref(),
                "gix status index/worktree modification path",
            )?;
            let (kind, conflict) = map_entry_status(status);
            push_unstaged_status(
                unstaged,
                has_conflicted_unstaged,
                FileStatus {
                    path,
                    kind,
                    conflict,
                },
            );
        }
        gix::status::plumbing::index_as_worktree_with_renames::Entry::DirectoryContents {
            entry,
            ..
        } => {
            // Staging areas the filesystem service leaves are never user changes.
            if gitcomet_core::path_utils::has_service_owned_git_component(entry.rela_path.as_ref())
            {
                return Ok(());
            }
            let Some(kind) = map_directory_entry_status(entry.status) else {
                return Ok(());
            };
            let path = path_buf_from_git_bytes(
                entry.rela_path.as_ref(),
                "gix status directory entry path",
            )?;
            push_unstaged_status(
                unstaged,
                has_conflicted_unstaged,
                FileStatus {
                    path,
                    kind,
                    conflict: None,
                },
            );
        }
        gix::status::plumbing::index_as_worktree_with_renames::Entry::Rewrite {
            source,
            dirwalk_entry,
            copy,
            ..
        } => {
            if gitcomet_core::path_utils::has_service_owned_git_component(
                dirwalk_entry.rela_path.as_ref(),
            ) {
                return push_rewrite_into_service_area(
                    source.rela_path(),
                    copy,
                    unstaged,
                    has_conflicted_unstaged,
                );
            }
            let kind = if copy {
                FileStatusKind::Added
            } else {
                FileStatusKind::Renamed
            };
            let path = path_buf_from_git_bytes(
                dirwalk_entry.rela_path.as_ref(),
                "gix status rewrite path",
            )?;
            push_unstaged_status(
                unstaged,
                has_conflicted_unstaged,
                FileStatus {
                    path,
                    kind,
                    conflict: None,
                },
            );
        }
    }
    Ok(())
}

/// A file parked in a staging area was removed from where it was tracked.
fn push_rewrite_into_service_area(
    source: &gix::bstr::BStr,
    copy: bool,
    unstaged: &mut Vec<FileStatus>,
    has_conflicted_unstaged: &mut bool,
) -> Result<()> {
    if copy {
        return Ok(());
    }
    let path = path_buf_from_git_bytes(source, "gix status rewrite source path")?;
    push_unstaged_status(
        unstaged,
        has_conflicted_unstaged,
        FileStatus {
            path,
            kind: FileStatusKind::Deleted,
            conflict: None,
        },
    );
    Ok(())
}

fn push_unstaged_status(
    unstaged: &mut Vec<FileStatus>,
    has_conflicted_unstaged: &mut bool,
    entry: FileStatus,
) {
    *has_conflicted_unstaged |= entry.kind == FileStatusKind::Conflicted;
    unstaged.push(entry);
}

struct DirectIndexWorktreeStatus {
    has_conflicted_unstaged: bool,
}

struct StatusEntryCollection {
    has_conflicted_unstaged: bool,
    index_changes: Vec<IndexWorktreeApplyChange>,
}

enum IndexWorktreeApplyChange {
    NewStat {
        entry_index: usize,
        stat: gix::index::entry::Stat,
    },
    SetSizeToZero {
        entry_index: usize,
    },
}

/// Collect a single TreeIndex change into the `staged` list.
fn collect_tree_index_change(
    change: gix::diff::index::ChangeRef<'_, '_>,
    staged: &mut Vec<FileStatus>,
) -> Result<()> {
    use gix::diff::index::ChangeRef;

    let (path, kind) = match change {
        ChangeRef::Addition { location, .. } => (
            path_buf_from_git_bytes(location.as_ref(), "gix status staged addition path")?,
            FileStatusKind::Added,
        ),
        ChangeRef::Deletion { location, .. } => (
            path_buf_from_git_bytes(location.as_ref(), "gix status staged deletion path")?,
            FileStatusKind::Deleted,
        ),
        ChangeRef::Modification { location, .. } => (
            path_buf_from_git_bytes(location.as_ref(), "gix status staged modification path")?,
            FileStatusKind::Modified,
        ),
        ChangeRef::Rewrite { location, copy, .. } => (
            path_buf_from_git_bytes(location.as_ref(), "gix status staged rewrite path")?,
            if copy {
                FileStatusKind::Added
            } else {
                FileStatusKind::Renamed
            },
        ),
    };

    staged.push(FileStatus {
        path,
        kind,
        conflict: None,
    });
    Ok(())
}

fn map_entry_status<T, U>(
    status: gix::status::plumbing::index_as_worktree::EntryStatus<T, U>,
) -> (FileStatusKind, Option<FileConflictKind>) {
    use gix::status::plumbing::index_as_worktree::{Change, Conflict, EntryStatus};

    match status {
        EntryStatus::Conflict { summary, .. } => (
            FileStatusKind::Conflicted,
            Some(match summary {
                Conflict::BothDeleted => FileConflictKind::BothDeleted,
                Conflict::AddedByUs => FileConflictKind::AddedByUs,
                Conflict::DeletedByThem => FileConflictKind::DeletedByThem,
                Conflict::AddedByThem => FileConflictKind::AddedByThem,
                Conflict::DeletedByUs => FileConflictKind::DeletedByUs,
                Conflict::BothAdded => FileConflictKind::BothAdded,
                Conflict::BothModified => FileConflictKind::BothModified,
            }),
        ),
        EntryStatus::IntentToAdd => (FileStatusKind::Added, None),
        EntryStatus::NeedsUpdate(_) => (FileStatusKind::Modified, None),
        EntryStatus::Change(change) => (
            match change {
                Change::Removed => FileStatusKind::Deleted,
                Change::Type { .. } => FileStatusKind::Modified,
                Change::Modification { .. } => FileStatusKind::Modified,
                Change::SubmoduleModification(_) => FileStatusKind::Modified,
            },
            None,
        ),
    }
}

fn map_directory_entry_status(status: gix::dir::entry::Status) -> Option<FileStatusKind> {
    match status {
        // Directory-walk entries represent an unstaged change only when they are
        // genuinely untracked. `Tracked` entries are traversal metadata and must
        // not become synthetic "modified" files.
        gix::dir::entry::Status::Untracked => Some(FileStatusKind::Untracked),
        gix::dir::entry::Status::Ignored(_)
        | gix::dir::entry::Status::Tracked
        | gix::dir::entry::Status::Pruned => None,
    }
}

fn map_porcelain_v2_status_char(ch: char) -> Option<FileStatusKind> {
    match ch {
        'M' | 'T' => Some(FileStatusKind::Modified),
        'A' => Some(FileStatusKind::Added),
        'D' => Some(FileStatusKind::Deleted),
        'R' => Some(FileStatusKind::Renamed),
        'U' => Some(FileStatusKind::Conflicted),
        _ => None,
    }
}

fn push_status_entry(entries: &mut Vec<FileStatus>, path: PathBuf, kind: FileStatusKind) {
    // Deduplication is handled by sort_and_dedup() after all entries are collected.
    entries.push(FileStatus {
        path,
        kind,
        conflict: None,
    });
}

fn apply_porcelain_v2_gitlink_status_record(
    record: &[u8],
    staged: &mut Vec<FileStatus>,
    unstaged: &mut Vec<FileStatus>,
) -> Result<()> {
    let mut parts = record.splitn(9, |byte| *byte == b' ');
    let Some(kind) = parts.next() else {
        return Ok(());
    };
    if kind != b"1" {
        return Ok(());
    }

    let xy = parts.next().unwrap_or_default();
    let _sub = parts.next();
    let m_head = parts.next().unwrap_or_default();
    let m_index = parts.next().unwrap_or_default();
    let m_worktree = parts.next().unwrap_or_default();
    let _h_head = parts.next();
    let _h_index = parts.next();
    let path = parts.next().unwrap_or_default();

    if path.is_empty() {
        return Ok(());
    }

    let is_gitlink = m_head == b"160000" || m_index == b"160000" || m_worktree == b"160000";
    if !is_gitlink {
        return Ok(());
    }

    let x = xy.first().copied().map(char::from).unwrap_or('.');
    let y = xy.get(1).copied().map(char::from).unwrap_or('.');
    let path = path_buf_from_git_bytes(path, "git status porcelain v2 gitlink path")?;

    if let Some(kind) = map_porcelain_v2_status_char(x) {
        push_status_entry(staged, path.clone(), kind);
    }
    if let Some(kind) = map_porcelain_v2_status_char(y) {
        push_status_entry(unstaged, path, kind);
    }

    Ok(())
}

fn supplement_gitlink_status_from_porcelain(
    workdir: &Path,
    repo: &gix::Repository,
    staged: &mut Vec<FileStatus>,
    unstaged: &mut Vec<FileStatus>,
    cancellation: &CancellationToken,
) -> Result<()> {
    cancellation.check_cancelled()?;
    let mut command = git_workdir_cmd_for(workdir);
    command
        .arg("--no-optional-locks")
        .arg("status")
        .arg("--porcelain=v2")
        .arg("-z")
        .arg("--ignore-submodules=none");
    if let Some(paths) = gitlink_status_paths(repo, staged) {
        if paths.is_empty() {
            return Ok(());
        }
        push_literal_paths(&mut command, paths);
    }
    let output = match run_git_background_output(command, "git status --porcelain=v2", cancellation)
    {
        Ok(output) => output,
        // Gitlink supplementation is best-effort parity glue on top of the primary
        // gix status result. If the subprocess itself times out, keep the base status.
        Err(err) if matches!(err.kind(), ErrorKind::Git(_)) => return Ok(()),
        Err(err) => return Err(err),
    };

    if !output.status.success() {
        return Ok(());
    }

    let mut records = output.stdout.split(|b| *b == 0).peekable();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        match record[0] {
            b'1' => {
                let _ = apply_porcelain_v2_gitlink_status_record(record, staged, unstaged);
            }
            b'2' => {
                // Rename/copy records carry an additional NUL-separated path.
                let _ = records.next();
            }
            _ => {}
        }
    }

    Ok(())
}

// Restrict only the supplemental Git query; gix remains responsible for ordinary
// paths. None means the selection is uncertain and requires a full Git query,
// while Some(empty) proves there is nothing left to supplement.
fn gitlink_status_paths(repo: &gix::Repository, staged: &[FileStatus]) -> Option<Vec<PathBuf>> {
    let index = repo.index_or_empty().ok()?;
    // Collapsed sparse directories can hide gitlinks. Let Git expand them for
    // the existing complete query instead of treating this as an empty list.
    if index.is_sparse() {
        return None;
    }
    let mut paths = Vec::new();
    for entry in index.entries() {
        if entry.mode == gix::index::entry::Mode::COMMIT {
            let path = path_buf_from_git_bytes(entry.path(&index), "gitlink status path").ok()?;
            // An unchanged gitlink with an empty checkout has no child status
            // to inspect. Recheck the filesystem each time: initializing the
            // child must restore the Git query without an index rewrite.
            if entry.stage_raw() == 0
                && !entry.flags.intersects(
                    gix::index::entry::Flags::SKIP_WORKTREE
                        | gix::index::entry::Flags::INTENT_TO_ADD,
                )
                && !staged
                    .iter()
                    .any(|change| change.path.starts_with(&path) || path.starts_with(&change.path))
                && repo
                    .workdir()
                    .is_some_and(|workdir| is_empty_gitlink_checkout(workdir, &path))
            {
                continue;
            }
            paths.push(path);
        }
    }
    // Deleted/replaced gitlinks are absent from the index. Check only the
    // staged paths in HEAD so ordinary staged files do not broaden the query
    // or prevent the empty-selection fast path.
    if !staged.is_empty()
        && let Some(head) = super::history::gix_head_id_or_none(repo).ok()?
    {
        let tree = repo.find_object(head).ok()?.peel_to_tree().ok()?;
        for entry in staged {
            if tree
                .lookup_entry_by_path(&entry.path)
                .ok()?
                .is_some_and(|entry| entry.mode().is_commit())
            {
                paths.push(entry.path.clone());
            }
        }
    }
    paths.sort_unstable();
    paths.dedup();
    // Leave room for the executable, workdir, options and Windows quoting in
    // CreateProcess's 32K command line. Large selections keep the full query.
    let bytes: usize = paths
        .iter()
        .map(|path| path.as_os_str().len() * 2 + 3)
        .sum();
    (bytes < 16_000).then_some(paths)
}

fn is_empty_gitlink_checkout(workdir: &Path, path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }
    let mut absolute = workdir.to_path_buf();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        absolute.push(name);
        // Do not follow symlinks/junctions in the checkout or its ancestors.
        // Missing paths, replacements and unreadable metadata stay with Git.
        let Ok(metadata) = std::fs::symlink_metadata(&absolute) else {
            return false;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return false;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt as _;
            // FILE_ATTRIBUTE_REPARSE_POINT also covers junctions and other
            // redirects which need not be exposed as Rust symlink types.
            if metadata.file_attributes() & 0x400 != 0 {
                return false;
            }
        }
        if components.peek().is_some() {
            // A nested repository boundary makes this layout uncertain.
            match std::fs::symlink_metadata(absolute.join(".git")) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return false,
            }
        }
    }
    // Even a broken .git file, another hidden file, or an enumeration error
    // retains the existing query. Only a positively empty directory is safe.
    std::fs::read_dir(absolute).is_ok_and(|mut entries| entries.next().is_none())
}

#[cfg(test)]
mod tests;
