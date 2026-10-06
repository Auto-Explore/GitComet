use super::{
    GitlinkStatusCapabilityCacheEntry, GixRepo, RepoFileStamp, TreeIndexCacheEntry,
    conflict_stages::conflict_kind_from_stage_mask, git_ops::head_upstream_divergence,
    repo_file_stamp,
};
use crate::util::{git_workdir_cmd_for, path_buf_from_git_bytes, run_git_raw_output};
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
        let repo = self.status_repo();
        // Bare repositories can display history, but have no checkout whose
        // changes we can report. Do not walk the Git directory or compare HEAD
        // against a missing index (which would report every file as deleted).
        if repo.workdir().is_none() {
            return Ok(RepoStatus::default());
        }
        let index_stamp = repo_index_stamp(&repo);
        let may_have_gitlinks = self.may_have_gitlink_status_supplement(&repo, &index_stamp);
        cancellation.check_cancelled()?;

        // Check whether HEAD and the index file are unchanged since the last
        // status call.  When both match, the staged (Tree→Index) result is
        // identical and we can skip the tree comparison entirely, using the
        // cheaper index-worktree-only iterator.
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
            may_have_gitlinks,
            cancellation,
        )?;
        cancellation.check_cancelled()?;

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
        )
    }

    pub(super) fn worktree_status_impl(&self) -> Result<Vec<FileStatus>> {
        self.worktree_status_cancellable_impl(&CancellationToken::new())
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
            may_have_gitlinks,
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
            )?;
            cancellation.check_cancelled()?;
        }

        sort_and_dedup_status_entries(&mut unstaged);
        Ok(unstaged)
    }

    pub(super) fn staged_status_impl(&self) -> Result<Vec<FileStatus>> {
        self.staged_status_cancellable_impl(&CancellationToken::new())
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

    pub(super) fn upstream_divergence_impl(&self) -> Result<Option<UpstreamDivergence>> {
        self.upstream_divergence_cancellable_impl(&CancellationToken::new())
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
        supplement_gitlink_status_from_porcelain(workdir, repo, &mut staged, &mut unstaged)?;
    }

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
    for (path, conflict_kind) in gix_unmerged_conflicts(repo)? {
        if let Some(entry) = unstaged.iter_mut().find(|entry| entry.path == path) {
            entry.kind = FileStatusKind::Conflicted;
            entry.conflict = Some(conflict_kind);
        } else {
            unstaged.push(FileStatus {
                path,
                kind: FileStatusKind::Conflicted,
                conflict: Some(conflict_kind),
            });
        }
    }
    Ok(())
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
            collect_tree_index_change(change, &mut staged).or_erased()?;
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
            collect_tree_index_change_paths(change, &mut paths).or_erased()?;
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
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
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

/// Re-hashing less than this per walk is cheaper than holding a second copy of
/// the index.
const STAT_REFRESH_MIN_BYTES: u64 = 256 * 1024;
const STAT_REFRESH_MIN_ENTRIES: usize = 64;

fn collect_index_worktree_status_direct(
    repo: &gix::Repository,
    stat_refresh: &std::sync::Mutex<Option<StatRefreshedIndex>>,
    unstaged: &mut Vec<FileStatus>,
    may_have_gitlinks: bool,
    cancellation: &CancellationToken,
) -> Result<DirectIndexWorktreeStatus> {
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
    let stamp = filetime::FileTime::from_unix_time(filetime::FileTime::now().unix_seconds() - 1, 0);
    let result = collect_index_worktree_status_direct_from_index(
        repo,
        refreshed
            .as_ref()
            .map_or(&index, |refreshed| &refreshed.index),
        unstaged,
        may_have_gitlinks,
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
            if rehashed_bytes < STAT_REFRESH_MIN_BYTES && changes.len() < STAT_REFRESH_MIN_ENTRIES {
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
    may_have_gitlinks: bool,
    cancellation: &CancellationToken,
) -> Result<(DirectIndexWorktreeStatus, Vec<IndexWorktreeApplyChange>)> {
    let dirwalk_options = repo
        .dirwalk_options()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix status dirwalk options: {e}"
            )))
        })?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching);
    let collection = if may_have_gitlinks && crate::refs::has_modules(repo)? {
        let submodule = gix::status::index_worktree::BuiltinSubmoduleStatus::new(
            repo.clone().into_sync(),
            gix::status::Submodule::Given {
                ignore: gix::submodule::config::Ignore::All,
                check_dirty: false,
            },
        )
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix status submodules: {e}"))))?;
        collect_index_worktree_status_direct_with_submodule(
            repo,
            index,
            dirwalk_options,
            unstaged,
            submodule,
            cancellation,
        )?
    } else {
        collect_index_worktree_status_direct_with_submodule(
            repo,
            index,
            dirwalk_options,
            unstaged,
            NoopSubmoduleStatus,
            cancellation,
        )?
    };
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
    ) -> gix::ExnResult<Option<Self::Output>> {
        Ok(None)
    }
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
    let (pathspec, _pathspec_attr_stack) = gix::Pathspec::new(
        repo,
        false,
        std::iter::empty::<gix::bstr::BString>(),
        true,
        || -> gix::ExnResult<gix::worktree::Stack> {
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
    let accelerate_lookup = fs_caps.ignore_case.then(|| index.prepare_icase_backing());
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
    let mut collector = StatusEntryCollector::new(unstaged);
    let interrupt = StatusCancellation::new(cancellation)?;
    let mut progress = cancellation::progress(&interrupt.interrupt);
    gix::status::plumbing::index_as_worktree_with_renames(
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
            object_hash: repo.object_hash(),
            fscache: false,
            tracked_file_modifications: gix::status::plumbing::index_as_worktree::Options {
                fs: fs_caps,
                thread_limit: worker_limit::for_index(repo, index, cancellation)?,
                fscache: false,
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
            if is_service_owned_rela_path(entry.rela_path.as_ref()) {
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
            if is_service_owned_rela_path(dirwalk_entry.rela_path.as_ref()) {
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

/// Staging areas the filesystem service may leave in a worktree are never
/// user changes.
fn is_service_owned_rela_path(path: &gix::bstr::BStr) -> bool {
    path.split(|byte| *byte == b'/')
        .any(gitcomet_core::path_utils::is_service_owned_name)
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
) -> Result<()> {
    let mut command = git_workdir_cmd_for(workdir);
    command
        .arg("--literal-pathspecs")
        .arg("--no-optional-locks")
        .arg("status")
        .arg("--porcelain=v2")
        .arg("-z")
        .arg("--ignore-submodules=none");
    if let Some(paths) = gitlink_status_paths(repo, staged) {
        if paths.is_empty() {
            return Ok(());
        }
        command.arg("--").args(paths);
    }
    let output = match run_git_raw_output(command, "git status --porcelain=v2") {
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
            paths.push(path_buf_from_git_bytes(entry.path(&index), "gitlink status path").ok()?);
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

#[cfg(test)]
pub(crate) mod tests {
    use rustc_hash::FxHashMap;

    use super::{
        STAT_REFRESH_MIN_ENTRIES, apply_porcelain_v2_gitlink_status_record,
        collect_unmerged_conflicts, conflict_kind_from_stage_mask, map_directory_entry_status,
        map_entry_status, map_porcelain_v2_status_char, remove_conflicted_paths_from_staged,
        should_supplement_unmerged_conflicts, sort_and_dedup_status_entries, tree_id_for_commit,
    };
    use gitcomet_core::domain::{FileConflictKind, FileStatus, FileStatusKind};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::sync::OnceLock;

    #[cfg(unix)]
    use std::{fs::Permissions, os::unix::fs::PermissionsExt as _};

    struct TestGitEnv {
        _root: tempfile::TempDir,
        global_config: PathBuf,
        home_dir: PathBuf,
        xdg_config_home: PathBuf,
        gnupg_home: PathBuf,
    }

    fn ensure_isolated_git_test_env() -> &'static TestGitEnv {
        static ENV: OnceLock<TestGitEnv> = OnceLock::new();
        ENV.get_or_init(|| {
            let root = tempfile::tempdir().expect("test git env tempdir");
            let home_dir = root.path().join("home");
            let xdg_config_home = root.path().join("xdg");
            let gnupg_home = root.path().join("gnupg");
            let global_config = root.path().join("gitconfig");

            fs::create_dir_all(&home_dir).expect("test git home");
            fs::create_dir_all(&xdg_config_home).expect("test git xdg config home");
            fs::create_dir_all(&gnupg_home).expect("test gnupg home");
            fs::write(&global_config, "").expect("test global git config");

            #[cfg(unix)]
            fs::set_permissions(&gnupg_home, Permissions::from_mode(0o700))
                .expect("test gnupg home permissions");

            crate::install_test_git_command_environment(
                global_config.clone(),
                home_dir.clone(),
                xdg_config_home.clone(),
                gnupg_home.clone(),
            );

            TestGitEnv {
                _root: root,
                global_config,
                home_dir,
                xdg_config_home,
                gnupg_home,
            }
        })
    }

    fn git_command() -> Command {
        let env = ensure_isolated_git_test_env();
        let mut cmd = Command::new("git");
        cmd.env("GIT_CONFIG_NOSYSTEM", "1");
        cmd.env("GIT_CONFIG_GLOBAL", &env.global_config);
        cmd.env("HOME", &env.home_dir);
        cmd.env("XDG_CONFIG_HOME", &env.xdg_config_home);
        cmd.env("GNUPGHOME", &env.gnupg_home);
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("GCM_INTERACTIVE", "Never");
        cmd.env("GIT_ALLOW_PROTOCOL", "file");
        cmd
    }

    fn git_output(workdir: &Path, args: &[&str]) -> Output {
        git_command()
            .arg("-C")
            .arg(workdir)
            .args(args)
            .output()
            .expect("spawn git")
    }

    pub(crate) fn git_success(workdir: &Path, args: &[&str]) {
        let output = git_output(workdir, args);
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_expect_failure(workdir: &Path, args: &[&str]) -> Output {
        let output = git_output(workdir, args);
        assert!(
            !output.status.success(),
            "expected git {:?} to fail\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    pub(crate) fn init_test_repo(workdir: &Path) {
        let _ = ensure_isolated_git_test_env();
        git_success(workdir, &["init"]);
        for args in [
            ["config", "core.autocrlf", "false"].as_slice(),
            ["config", "core.eol", "lf"].as_slice(),
            ["config", "credential.helper", ""].as_slice(),
            ["config", "credential.interactive", "never"].as_slice(),
            ["config", "protocol.file.allow", "always"].as_slice(),
            ["config", "commit.gpgsign", "false"].as_slice(),
            ["config", "user.name", "Test User"].as_slice(),
            ["config", "user.email", "test@example.com"].as_slice(),
        ] {
            git_success(workdir, args);
        }
    }

    pub(crate) fn write_file(workdir: &Path, relative: &str, contents: &str) {
        let path = workdir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(path, contents).expect("write file");
    }

    pub(crate) fn open_repo(workdir: &Path) -> super::super::GixRepo {
        let thread_safe_repo = gix::open(workdir).expect("open repo").into_sync();
        super::super::GixRepo::new(workdir.to_path_buf(), thread_safe_repo)
    }

    fn file_status(path: &str, kind: FileStatusKind) -> FileStatus {
        FileStatus {
            path: PathBuf::from(path),
            kind,
            conflict: None,
        }
    }

    #[test]
    fn gitlink_query_excludes_ordinary_staged_paths_but_keeps_removed_and_replaced_links() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        init_test_repo(root);
        write_file(root, "ordinary.txt", "base\n");
        git_success(root, &["add", "ordinary.txt"]);
        git_success(root, &["commit", "-m", "base"]);
        let oid = crate::refs::head_oid(&gix::open(root).unwrap())
            .unwrap()
            .unwrap()
            .to_string();
        for path in ["removed", "replaced", "retained"] {
            git_success(
                root,
                &["update-index", "--add", "--cacheinfo", "160000", &oid, path],
            );
        }
        git_success(root, &["commit", "-m", "gitlinks"]);
        git_success(root, &["update-index", "--force-remove", "removed"]);
        git_success(root, &["update-index", "--force-remove", "replaced"]);
        write_file(root, "replaced", "regular file\n");
        write_file(root, "ordinary.txt", "changed\n");
        git_success(root, &["add", "replaced", "ordinary.txt"]);
        let staged = [
            file_status("ordinary.txt", FileStatusKind::Modified),
            file_status("removed", FileStatusKind::Deleted),
            file_status("replaced", FileStatusKind::Modified),
        ];
        assert_eq!(
            super::gitlink_status_paths(&gix::open(root).unwrap(), &staged),
            Some(
                ["removed", "replaced", "retained"]
                    .map(PathBuf::from)
                    .to_vec()
            )
        );
        git_success(root, &["update-index", "--force-remove", "retained"]);
        git_success(root, &["commit", "-m", "remove gitlinks"]);
        write_file(root, "ordinary.txt", "changed again\n");
        git_success(root, &["add", "ordinary.txt"]);
        assert_eq!(
            super::gitlink_status_paths(&gix::open(root).unwrap(), &staged[..1]),
            Some(Vec::new())
        );
    }

    fn conflicted_file_status(path: &str, conflict: FileConflictKind) -> FileStatus {
        FileStatus {
            path: PathBuf::from(path),
            kind: FileStatusKind::Conflicted,
            conflict: Some(conflict),
        }
    }

    fn setup_both_modified_text_conflict(workdir: &Path, path: &str) {
        init_test_repo(workdir);
        write_file(workdir, path, "base\n");
        git_success(workdir, &["add", path]);
        git_success(workdir, &["commit", "-m", "base"]);

        git_success(workdir, &["checkout", "-b", "feature"]);
        write_file(workdir, path, "theirs\n");
        git_success(workdir, &["commit", "-am", "theirs"]);

        git_success(workdir, &["checkout", "-"]);
        write_file(workdir, path, "ours\n");
        git_success(workdir, &["commit", "-am", "ours"]);

        let _ = git_expect_failure(workdir, &["merge", "feature"]);
    }

    #[test]
    fn conflict_kind_from_stage_mask_covers_all_shapes() {
        assert_eq!(
            conflict_kind_from_stage_mask(0b001),
            Some(FileConflictKind::BothDeleted)
        );
        assert_eq!(
            conflict_kind_from_stage_mask(0b010),
            Some(FileConflictKind::AddedByUs)
        );
        assert_eq!(
            conflict_kind_from_stage_mask(0b011),
            Some(FileConflictKind::DeletedByThem)
        );
        assert_eq!(
            conflict_kind_from_stage_mask(0b100),
            Some(FileConflictKind::AddedByThem)
        );
        assert_eq!(
            conflict_kind_from_stage_mask(0b101),
            Some(FileConflictKind::DeletedByUs)
        );
        assert_eq!(
            conflict_kind_from_stage_mask(0b110),
            Some(FileConflictKind::BothAdded)
        );
        assert_eq!(
            conflict_kind_from_stage_mask(0b111),
            Some(FileConflictKind::BothModified)
        );
        assert_eq!(conflict_kind_from_stage_mask(0), None);
    }

    #[test]
    fn collect_unmerged_conflicts_groups_stage_entries_by_path() {
        let stages = vec![
            (PathBuf::from("dd.txt"), 1),
            (PathBuf::from("au.txt"), 2),
            (PathBuf::from("ud.txt"), 1),
            (PathBuf::from("ud.txt"), 2),
            (PathBuf::from("ua.txt"), 3),
            (PathBuf::from("du.txt"), 1),
            (PathBuf::from("du.txt"), 3),
            (PathBuf::from("aa.txt"), 2),
            (PathBuf::from("aa.txt"), 3),
            (PathBuf::from("uu.txt"), 1),
            (PathBuf::from("uu.txt"), 2),
            (PathBuf::from("uu.txt"), 3),
        ];

        let parsed = collect_unmerged_conflicts(stages);
        let by_path = parsed
            .into_iter()
            .collect::<FxHashMap<PathBuf, FileConflictKind>>();

        assert_eq!(
            by_path.get(&PathBuf::from("dd.txt")),
            Some(&FileConflictKind::BothDeleted)
        );
        assert_eq!(
            by_path.get(&PathBuf::from("au.txt")),
            Some(&FileConflictKind::AddedByUs)
        );
        assert_eq!(
            by_path.get(&PathBuf::from("ud.txt")),
            Some(&FileConflictKind::DeletedByThem)
        );
        assert_eq!(
            by_path.get(&PathBuf::from("ua.txt")),
            Some(&FileConflictKind::AddedByThem)
        );
        assert_eq!(
            by_path.get(&PathBuf::from("du.txt")),
            Some(&FileConflictKind::DeletedByUs)
        );
        assert_eq!(
            by_path.get(&PathBuf::from("aa.txt")),
            Some(&FileConflictKind::BothAdded)
        );
        assert_eq!(
            by_path.get(&PathBuf::from("uu.txt")),
            Some(&FileConflictKind::BothModified)
        );
    }

    #[test]
    fn collect_unmerged_conflicts_accepts_a_lazily_filtered_iterator() {
        // The natural way to drop the intermediate `Vec` in `gix_unmerged_conflicts`
        // is to hand the index scan straight in. A `Filter` reports `(0, Some(len))`
        // for its size hint, so the capacity hint must come from the lower bound --
        // taking the upper one would silently reserve the whole index here.
        let stages = [
            (PathBuf::from("src/lib.rs"), 0u8),
            (PathBuf::from("src/main.rs"), 1u8),
            (PathBuf::from("src/main.rs"), 2u8),
            (PathBuf::from("src/main.rs"), 3u8),
        ];

        let parsed = collect_unmerged_conflicts(
            stages
                .into_iter()
                .filter(|(_, stage)| (1..=3).contains(stage)),
        );

        assert_eq!(
            parsed,
            vec![(PathBuf::from("src/main.rs"), FileConflictKind::BothModified)]
        );
    }

    #[test]
    fn collect_unmerged_conflicts_ignores_unconflicted_and_unknown_stages() {
        let stages = vec![
            (PathBuf::from("clean.txt"), 0),
            (PathBuf::from("ignored.txt"), 4),
            (PathBuf::from("conflicted.txt"), 2),
            (PathBuf::from("conflicted.txt"), 3),
        ];

        let parsed = collect_unmerged_conflicts(stages);
        assert_eq!(
            parsed,
            vec![(PathBuf::from("conflicted.txt"), FileConflictKind::BothAdded)]
        );
    }

    #[test]
    fn sort_and_dedup_status_entries_prefers_highest_priority_kind_per_path() {
        let mut entries = vec![
            file_status("b.txt", FileStatusKind::Modified),
            file_status("a.txt", FileStatusKind::Untracked),
            file_status("a.txt", FileStatusKind::Deleted),
            file_status("c.txt", FileStatusKind::Modified),
            file_status("c.txt", FileStatusKind::Added),
            file_status("d.txt", FileStatusKind::Modified),
            file_status("d.txt", FileStatusKind::Renamed),
        ];

        sort_and_dedup_status_entries(&mut entries);

        assert_eq!(
            entries,
            vec![
                file_status("a.txt", FileStatusKind::Deleted),
                file_status("b.txt", FileStatusKind::Modified),
                file_status("c.txt", FileStatusKind::Added),
                file_status("d.txt", FileStatusKind::Renamed),
            ]
        );
    }

    #[test]
    fn remove_conflicted_paths_from_staged_ignores_empty_input() {
        let expected = vec![file_status("a.txt", FileStatusKind::Modified)];
        let mut staged = expected.clone();

        remove_conflicted_paths_from_staged(&mut staged, std::iter::empty::<PathBuf>());

        assert_eq!(staged, expected);
    }

    #[test]
    fn remove_conflicted_paths_from_staged_removes_only_matching_paths() {
        let mut staged = vec![
            file_status("a.txt", FileStatusKind::Modified),
            file_status("b.txt", FileStatusKind::Added),
            file_status("c.txt", FileStatusKind::Deleted),
        ];

        remove_conflicted_paths_from_staged(
            &mut staged,
            [PathBuf::from("b.txt"), PathBuf::from("missing.txt")],
        );

        assert_eq!(
            staged,
            vec![
                file_status("a.txt", FileStatusKind::Modified),
                file_status("c.txt", FileStatusKind::Deleted),
            ]
        );
    }

    #[test]
    fn map_directory_entry_status_only_reports_untracked_entries() {
        use gix::dir::entry::Status;

        assert_eq!(
            map_directory_entry_status(Status::Untracked),
            Some(FileStatusKind::Untracked)
        );
        assert_eq!(map_directory_entry_status(Status::Tracked), None);
        assert_eq!(
            map_directory_entry_status(Status::Ignored(gix::ignore::Kind::Expendable)),
            None
        );
        assert_eq!(
            map_directory_entry_status(Status::Ignored(gix::ignore::Kind::Precious)),
            None
        );
        assert_eq!(map_directory_entry_status(Status::Pruned), None);
    }

    #[test]
    fn map_entry_status_maps_all_conflict_summaries() {
        use gix::status::plumbing::index_as_worktree::{Conflict, EntryStatus};

        for (summary, expected) in [
            (Conflict::BothDeleted, FileConflictKind::BothDeleted),
            (Conflict::AddedByUs, FileConflictKind::AddedByUs),
            (Conflict::DeletedByThem, FileConflictKind::DeletedByThem),
            (Conflict::AddedByThem, FileConflictKind::AddedByThem),
            (Conflict::DeletedByUs, FileConflictKind::DeletedByUs),
            (Conflict::BothAdded, FileConflictKind::BothAdded),
            (Conflict::BothModified, FileConflictKind::BothModified),
        ] {
            assert_eq!(
                map_entry_status::<(), ()>(EntryStatus::Conflict {
                    summary,
                    entries: Box::new([None, None, None]),
                }),
                (FileStatusKind::Conflicted, Some(expected))
            );
        }
    }

    #[test]
    fn map_entry_status_maps_non_conflict_variants() {
        use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};

        assert_eq!(
            map_entry_status::<(), ()>(EntryStatus::IntentToAdd),
            (FileStatusKind::Added, None)
        );
        assert_eq!(
            map_entry_status::<(), ()>(
                EntryStatus::NeedsUpdate(gix::index::entry::Stat::default())
            ),
            (FileStatusKind::Modified, None)
        );
        assert_eq!(
            map_entry_status::<(), ()>(EntryStatus::Change(Change::Removed)),
            (FileStatusKind::Deleted, None)
        );
        assert_eq!(
            map_entry_status::<(), ()>(EntryStatus::Change(Change::Type {
                worktree_mode: gix::index::entry::Mode::FILE,
            })),
            (FileStatusKind::Modified, None)
        );
        assert_eq!(
            map_entry_status::<(), ()>(EntryStatus::Change(Change::Modification {
                executable_bit_changed: false,
                content_change: None,
                set_entry_stat_size_zero: false,
            })),
            (FileStatusKind::Modified, None)
        );
        assert_eq!(
            map_entry_status::<(), ()>(EntryStatus::Change(Change::SubmoduleModification(()))),
            (FileStatusKind::Modified, None)
        );
    }

    #[test]
    fn map_porcelain_v2_status_char_maps_supported_values() {
        for (ch, expected) in [
            ('M', Some(FileStatusKind::Modified)),
            ('T', Some(FileStatusKind::Modified)),
            ('A', Some(FileStatusKind::Added)),
            ('D', Some(FileStatusKind::Deleted)),
            ('R', Some(FileStatusKind::Renamed)),
            ('U', Some(FileStatusKind::Conflicted)),
            ('.', None),
            ('?', None),
        ] {
            assert_eq!(map_porcelain_v2_status_char(ch), expected);
        }
    }

    #[test]
    fn supplement_unmerged_conflicts_runs_for_in_progress_repo() {
        assert!(should_supplement_unmerged_conflicts(true, false));
    }

    #[test]
    fn supplement_unmerged_conflicts_runs_for_reported_conflicts() {
        assert!(should_supplement_unmerged_conflicts(false, true));
    }

    #[test]
    fn supplement_unmerged_conflicts_skips_clean_repo() {
        assert!(!should_supplement_unmerged_conflicts(false, false));
    }

    #[test]
    fn porcelain_gitlink_record_maps_committed_unstaged_modification() {
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        apply_porcelain_v2_gitlink_status_record(
            b"1 .M SC.. 160000 160000 160000 1111111111111111111111111111111111111111 1111111111111111111111111111111111111111 chess3",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

        assert!(staged.is_empty());
        assert_eq!(unstaged.len(), 1);
        assert_eq!(unstaged[0].path, PathBuf::from("chess3"));
        assert_eq!(unstaged[0].kind, FileStatusKind::Modified);
    }

    #[test]
    fn porcelain_gitlink_record_maps_conflicted_status_chars_to_both_lanes() {
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        apply_porcelain_v2_gitlink_status_record(
            b"1 UU SC.. 160000 160000 160000 1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 chess3",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

        assert_eq!(
            staged,
            vec![file_status("chess3", FileStatusKind::Conflicted)]
        );
        assert_eq!(
            unstaged,
            vec![file_status("chess3", FileStatusKind::Conflicted)]
        );
    }

    #[test]
    fn porcelain_gitlink_record_maps_added_and_unstaged_modified() {
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        apply_porcelain_v2_gitlink_status_record(
            b"1 AM SC.. 000000 160000 160000 0000000000000000000000000000000000000000 2222222222222222222222222222222222222222 chess3",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].path, PathBuf::from("chess3"));
        assert_eq!(staged[0].kind, FileStatusKind::Added);

        assert_eq!(unstaged.len(), 1);
        assert_eq!(unstaged[0].path, PathBuf::from("chess3"));
        assert_eq!(unstaged[0].kind, FileStatusKind::Modified);
    }

    #[test]
    fn porcelain_gitlink_record_ignores_non_type_one_records() {
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        apply_porcelain_v2_gitlink_status_record(
            b"2 R. N... 160000 160000 160000 111 222 chess3",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

        assert!(staged.is_empty());
        assert!(unstaged.is_empty());
    }

    #[test]
    fn porcelain_gitlink_record_ignores_non_gitlink_modes() {
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        apply_porcelain_v2_gitlink_status_record(
            b"1 M. N... 100644 100644 100644 111 222 chess3",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

        assert!(staged.is_empty());
        assert!(unstaged.is_empty());
    }

    #[test]
    fn porcelain_gitlink_record_ignores_missing_path() {
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        apply_porcelain_v2_gitlink_status_record(
            b"1 M. SC.. 160000 160000 160000 111 222 ",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

        assert!(staged.is_empty());
        assert!(unstaged.is_empty());
    }

    #[test]
    fn porcelain_gitlink_record_preserves_spaces_in_path() {
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        apply_porcelain_v2_gitlink_status_record(
            b"1 .M SC.. 160000 160000 160000 1111111111111111111111111111111111111111 1111111111111111111111111111111111111111 submodule with spaces",
            &mut staged,
            &mut unstaged,
        )
        .unwrap();

        assert!(staged.is_empty());
        assert_eq!(unstaged.len(), 1);
        assert_eq!(unstaged[0].path, PathBuf::from("submodule with spaces"));
        assert_eq!(unstaged[0].kind, FileStatusKind::Modified);
    }

    #[cfg(unix)]
    #[test]
    fn porcelain_gitlink_record_preserves_non_utf8_path_bytes() {
        use std::os::unix::ffi::OsStrExt as _;

        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut record = b"1 .M SC.. 160000 160000 160000 1111111111111111111111111111111111111111 1111111111111111111111111111111111111111 submodule-".to_vec();
        record.push(0xff);
        apply_porcelain_v2_gitlink_status_record(&record, &mut staged, &mut unstaged).unwrap();

        assert!(staged.is_empty());
        assert_eq!(unstaged.len(), 1);
        assert_eq!(unstaged[0].path.as_os_str().as_bytes(), b"submodule-\xff");
        assert_eq!(unstaged[0].kind, FileStatusKind::Modified);
    }

    #[test]
    fn status_impl_matches_lane_specific_statuses() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "staged.txt", "base\n");
        write_file(workdir, "unstaged.txt", "base\n");
        git_success(workdir, &["add", "staged.txt", "unstaged.txt"]);
        git_success(workdir, &["commit", "-m", "initial"]);

        write_file(workdir, "staged.txt", "staged change\n");
        git_success(workdir, &["add", "staged.txt"]);
        write_file(workdir, "unstaged.txt", "unstaged change\n");
        write_file(workdir, "untracked.txt", "untracked\n");

        let gix_repo = open_repo(workdir);
        let combined = gix_repo.status_impl().expect("combined status");
        let staged = gix_repo.staged_status_impl().expect("staged status");
        let unstaged = gix_repo.worktree_status_impl().expect("worktree status");

        assert_eq!(*combined.staged, staged);
        assert_eq!(*combined.unstaged, unstaged);
        assert_eq!(
            staged,
            vec![file_status("staged.txt", FileStatusKind::Modified)]
        );
        assert_eq!(
            unstaged,
            vec![
                file_status("unstaged.txt", FileStatusKind::Modified),
                file_status("untracked.txt", FileStatusKind::Untracked),
            ]
        );
    }

    #[test]
    fn status_hides_filesystem_service_staging_areas() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "tracked.txt", "base\n");
        git_success(workdir, &["add", "tracked.txt"]);
        git_success(workdir, &["commit", "-m", "initial"]);
        write_file(workdir, ".gitcomet-operation-abc/item", "parked\n");
        write_file(
            workdir,
            "dir/.gitcomet-operation-def/recovery.log",
            "move\n",
        );
        write_file(workdir, "dir/.gitcomet-save-xyz", "staged\n");
        write_file(workdir, "dir/new.txt", "untracked\n");

        let gix_repo = open_repo(workdir);
        // The second call takes the cached-staged fast path.
        for _ in 0..2 {
            let status = gix_repo.status_impl().expect("status");
            assert_eq!(
                *status.unstaged,
                vec![file_status("dir/new.txt", FileStatusKind::Untracked)]
            );
        }
        assert_eq!(
            gix_repo.worktree_status_impl().expect("worktree status"),
            vec![file_status("dir/new.txt", FileStatusKind::Untracked)]
        );
    }

    #[test]
    fn file_edited_after_staging_appears_in_both_staged_and_unstaged_lanes() {
        // Reproduces the reported state: a file that is staged AND then edited again in the
        // worktree (`MM` in `git status`) must appear in BOTH the staged and the unstaged lanes —
        // not only staged. This is the backend truth the UI's Unstaged section must reflect once
        // the file-watcher delivers the edit event.
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "foo.txt", "v0\n");
        git_success(workdir, &["add", "foo.txt"]);
        git_success(workdir, &["commit", "-m", "initial"]);

        // Stage a modification...
        write_file(workdir, "foo.txt", "staged change\n");
        git_success(workdir, &["add", "foo.txt"]);
        // ...then edit the worktree again WITHOUT staging -> `MM`.
        write_file(workdir, "foo.txt", "further worktree edit\n");

        let gix_repo = open_repo(workdir);
        let status = gix_repo.status_impl().expect("combined status");
        assert_eq!(
            *status.staged,
            vec![file_status("foo.txt", FileStatusKind::Modified)],
            "a staged-and-re-edited file must appear in the staged lane"
        );
        assert_eq!(
            *status.unstaged,
            vec![file_status("foo.txt", FileStatusKind::Modified)],
            "the further worktree edit must also appear in the unstaged lane"
        );
        // The per-lane entry points must agree with the combined status.
        assert_eq!(
            gix_repo.staged_status_impl().expect("staged lane"),
            *status.staged
        );
        assert_eq!(
            gix_repo.worktree_status_impl().expect("worktree lane"),
            *status.unstaged
        );
    }

    #[test]
    fn staged_status_impl_on_unborn_head_uses_combined_status() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        write_file(workdir, "new.txt", "new\n");
        git_success(workdir, &["add", "new.txt"]);

        let gix_repo = open_repo(workdir);
        let combined = gix_repo.status_impl().expect("combined status");
        let staged = gix_repo.staged_status_impl().expect("staged status");

        assert_eq!(*combined.staged, staged);
        assert_eq!(staged, vec![file_status("new.txt", FileStatusKind::Added)]);
        assert!(combined.unstaged.is_empty());
    }

    #[test]
    fn status_impl_removes_conflicted_paths_from_staged_lane() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        setup_both_modified_text_conflict(workdir, "tracked.txt");

        let gix_repo = open_repo(workdir);
        let combined = gix_repo.status_impl().expect("combined status");
        let staged = gix_repo.staged_status_impl().expect("staged status");
        let worktree = gix_repo.worktree_status_impl().expect("worktree status");

        assert!(combined.staged.is_empty());
        assert!(staged.is_empty());
        assert_eq!(*combined.unstaged, worktree);
        assert_eq!(
            worktree,
            vec![conflicted_file_status(
                "tracked.txt",
                FileConflictKind::BothModified,
            )]
        );
    }

    #[test]
    fn staged_status_impl_resolves_head_to_tree_before_tree_index_diff() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);

        fs::write(workdir.join("tracked.txt"), "base\n").expect("write tracked file");
        git_success(workdir, &["add", "tracked.txt"]);
        git_success(workdir, &["commit", "-m", "initial"]);

        fs::write(workdir.join("tracked.txt"), "base\nchanged\n").expect("rewrite tracked file");
        git_success(workdir, &["add", "tracked.txt"]);

        let thread_safe_repo = gix::open(workdir).expect("open repo").into_sync();
        let gix_repo = super::super::GixRepo::new(workdir.to_path_buf(), thread_safe_repo);

        let head_commit_id = super::super::history::gix_head_id_or_none(
            &gix_repo.reopen_repo().expect("reopen repo"),
        )
        .expect("head lookup")
        .expect("head commit");
        let head_tree_id = tree_id_for_commit(
            &gix_repo.reopen_repo().expect("reopen repo"),
            &head_commit_id,
        )
        .expect("head tree");
        assert_ne!(head_commit_id, head_tree_id);

        let staged = gix_repo.staged_status_impl().expect("staged status");
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].path, PathBuf::from("tracked.txt"));
        assert_eq!(staged[0].kind, FileStatusKind::Modified);
    }

    #[test]
    fn read_index_trailing_hash_distinguishes_content_and_rejects_unusable_trailers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("index");
        let hash_len = gix::hash::Kind::Sha1.len_in_bytes();

        let trailing_hash = |bytes: &[u8]| {
            fs::write(&path, bytes).expect("write index");
            let mut file = std::fs::File::open(&path).expect("open index");
            super::read_index_trailing_hash(&mut file, bytes.len() as u64, gix::hash::Kind::Sha1)
        };

        // A plausible index layout: some body bytes followed by the trailing hash.
        let mut bytes = vec![7u8; 16 + hash_len];
        let trailer_start = bytes.len() - hash_len;
        bytes[trailer_start..].copy_from_slice(&[1u8; 20]);
        let hash_a = trailing_hash(&bytes).expect("trailing hash a");

        // Same length and (re)written file, but different trailing content.
        bytes[trailer_start..].copy_from_slice(&[2u8; 20]);
        let hash_b = trailing_hash(&bytes).expect("trailing hash b");
        assert_ne!(
            hash_a, hash_b,
            "different index content must yield different fingerprints"
        );

        // A null trailer (index.skipHash) is not a usable fingerprint and must be rejected so it
        // cannot make distinct index states compare equal.
        bytes[trailer_start..].copy_from_slice(&[0u8; 20]);
        assert!(
            trailing_hash(&bytes).is_none(),
            "a null trailing hash must be treated as absent"
        );

        // Files shorter than the hash length cannot carry a trailer.
        assert!(trailing_hash(&[0u8; 4]).is_none());

        // Missing files have no fingerprint.
        assert!(std::fs::File::open(tmp.path().join("missing")).is_err());
    }

    #[test]
    fn gitlink_capability_cache_invalidates_on_index_content_change_with_matching_len_and_mtime() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "tracked.txt", "base\n");
        git_success(workdir, &["add", "tracked.txt"]);
        git_success(workdir, &["commit", "-m", "initial"]);

        let gix_repo = open_repo(workdir);
        let repo = gix_repo.reopen_repo().expect("reopen repo");

        // Real state: no .gitmodules and no gitlink entries → no submodule supplement needed.
        assert!(
            !gix_repo.may_have_gitlink_status_supplement(&repo, &super::repo_index_stamp(&repo)),
            "a plain repo should not need the gitlink/submodule supplement"
        );

        // Poison the capability cache to simulate a stale `may_have_gitlinks = true` that was
        // computed from a *different* index content sharing the current length + mtime (an atomic
        // rewrite that collided on a coarse/cached-timestamp filesystem such as f2fs). A
        // len+mtime-only stamp cannot tell the two index states apart.
        let stale_index_stamp = super::repo_file_stamp(repo.index_path().as_path());
        let gitmodules_stamp = super::repo_file_stamp(workdir.join(".gitmodules").as_path());
        *gix_repo
            .gitlink_status_capability
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(super::super::GitlinkStatusCapabilityCacheEntry {
                gitmodules: gitmodules_stamp,
                index: stale_index_stamp,
                may_have_gitlinks: true,
            });

        // The index fingerprint must reject the stale entry and recompute the real (false) answer
        // rather than serving the poisoned value.
        assert!(
            !gix_repo.may_have_gitlink_status_supplement(&repo, &super::repo_index_stamp(&repo)),
            "gitlink capability must not be served stale on an index len+mtime collision"
        );
    }

    #[cfg(unix)]
    #[test]
    fn index_stamp_is_uncacheable_when_index_is_present_but_unreadable() {
        // A unix socket is a deterministic stand-in for "the index exists and is stat-able, but
        // File::open fails" (the real-world cases being a momentary permission flip or a Windows
        // sharing/AV lock). open() on a socket fails with ENXIO even for root, while stat()
        // succeeds. A stat-only (len+mtime) fallback could collide with an atomic index rewrite of
        // the same length and an unchanged/coarse mtime and false-hit the cache — exactly the bug
        // the content discriminators exist to prevent. So an unreadable-but-present index must
        // instead yield an *uncacheable* stamp: two such stamps must never compare equal, forcing a
        // fresh read until the index is readable again.
        use std::os::unix::net::UnixListener;

        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("index");
        let _listener = UnixListener::bind(&path).expect("bind unix socket as fake index");
        assert!(
            std::fs::File::open(&path).is_err(),
            "precondition: opening the socket file must fail"
        );
        assert!(
            std::fs::metadata(&path).is_ok(),
            "precondition: stat-ing the socket file must succeed"
        );

        let first = super::index_stamp_for(&path, gix::hash::Kind::Sha1);
        let second = super::index_stamp_for(&path, gix::hash::Kind::Sha1);
        assert_ne!(
            first, second,
            "two stamps taken while the index is unreadable must never compare equal, so the cache \
             is forced to miss rather than risk a stale hit"
        );
        assert_ne!(
            first,
            super::super::RepoFileStamp::default(),
            "an unreadable-index stamp must also differ from the absent-index default"
        );
    }

    #[cfg(unix)]
    #[test]
    fn index_stamp_detects_rewrites_without_content_hash_via_stat() {
        // Stand in for an `index.skipHash` repository (null trailer → no content fingerprint):
        // force `content_id` to `None` and confirm the inode/ctime discriminators still tell two
        // index revisions apart, so the staged cache cannot serve stale results there.
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "tracked.txt", "base\n");
        git_success(workdir, &["add", "tracked.txt"]);
        git_success(workdir, &["commit", "-m", "initial"]);

        let gix_repo = open_repo(workdir);
        let stamp_before = super::repo_index_stamp(&gix_repo.reopen_repo().expect("reopen repo"));
        assert!(
            stamp_before.inode.is_some() && stamp_before.ctime_nanos.is_some(),
            "the index stamp should carry inode + ctime discriminators on Unix"
        );

        // Rewrite the index by staging a same-length change to the tracked file.
        write_file(workdir, "tracked.txt", "next\n");
        git_success(workdir, &["add", "tracked.txt"]);
        let stamp_after = super::repo_index_stamp(&gix_repo.reopen_repo().expect("reopen repo"));

        let without_hash_before = super::super::RepoFileStamp {
            content_id: None,
            ..stamp_before
        };
        let without_hash_after = super::super::RepoFileStamp {
            content_id: None,
            ..stamp_after
        };
        assert_ne!(
            without_hash_before, without_hash_after,
            "an index rewrite must change the stamp even without the content hash (skipHash case)"
        );
    }

    #[test]
    fn staged_cache_invalidates_on_index_content_change_with_matching_len_and_mtime() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "tracked.txt", "base\n");
        git_success(workdir, &["add", "tracked.txt"]);
        git_success(workdir, &["commit", "-m", "initial"]);

        let gix_repo = open_repo(workdir);
        let repo = gix_repo.reopen_repo().expect("reopen repo");
        let head_oid = super::super::history::gix_head_id_or_none(&repo).expect("head lookup");
        let clean_stamp = super::repo_index_stamp(&repo);
        assert!(
            clean_stamp.content_id.is_some(),
            "the index fingerprint should be captured for a real repository"
        );

        // With a clean index nothing is staged; this also primes the cache.
        assert!(
            gix_repo
                .staged_status_impl()
                .expect("staged status")
                .is_empty()
        );

        // Reproduce the bug's precondition: a cached staged result whose stamp matches the
        // current index in length + mtime but reflects *different* index content (an atomic
        // rewrite that collided on a coarse/cached-timestamp filesystem such as f2fs). Only the
        // content fingerprint distinguishes the two.
        let stale_stamp = super::super::RepoFileStamp {
            content_id: Some(
                gix::ObjectId::from_hex(b"1111111111111111111111111111111111111111")
                    .expect("placeholder oid"),
            ),
            ..clean_stamp.clone()
        };
        *gix_repo
            .tree_index_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(super::super::TreeIndexCacheEntry {
                head_oid,
                index_stamp: stale_stamp,
                staged: vec![file_status("tracked.txt", FileStatusKind::Modified)],
            });

        // The content fingerprint differs, so the stale entry is rejected and the staged lane is
        // recomputed as empty instead of being served from the cache.
        assert!(
            gix_repo
                .staged_status_impl()
                .expect("recomputed staged status")
                .is_empty(),
            "a content-exact stamp must invalidate a stale staged cache on len+mtime collisions"
        );

        // The cache still works for an exact (content-matching) entry: a poisoned entry whose
        // stamp fully matches the current index is served verbatim, confirming we did not simply
        // disable caching.
        *gix_repo
            .tree_index_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(super::super::TreeIndexCacheEntry {
                head_oid,
                index_stamp: clean_stamp,
                staged: vec![file_status("sentinel.txt", FileStatusKind::Added)],
            });
        assert_eq!(
            gix_repo.staged_status_impl().expect("cached staged status"),
            vec![file_status("sentinel.txt", FileStatusKind::Added)],
            "an exact stamp match should still hit the staged cache"
        );
    }

    #[test]
    fn status_lanes_track_external_stage_unstage_transitions_across_the_cache() {
        // End-to-end check on a real repo: as external `git add` / `git reset` rewrite `.git/index`,
        // a file must move between the staged and unstaged lanes, and the staged cache (populated by
        // each `status_impl` call) must invalidate on every index rewrite so it never serves a
        // stale lane. This reproduces the original "file stuck in the wrong section" symptom.
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        write_file(workdir, "foo.txt", "v0\n");
        git_success(workdir, &["add", "foo.txt"]);
        git_success(workdir, &["commit", "-m", "initial"]);

        let gix_repo = open_repo(workdir);
        let modified = |kind| vec![file_status("foo.txt", kind)];

        // Stage a modification: foo is staged, nothing unstaged. Primes the staged cache.
        write_file(workdir, "foo.txt", "v1\n");
        git_success(workdir, &["add", "foo.txt"]);
        let status = gix_repo.status_impl().expect("status after staging");
        assert_eq!(*status.staged, modified(FileStatusKind::Modified));
        assert!(status.unstaged.is_empty());

        // Edit the worktree again WITHOUT staging: foo is now in BOTH lanes. The index is unchanged,
        // so the staged lane is served from the cache while the unstaged lane is recomputed.
        write_file(workdir, "foo.txt", "v2\n");
        let status = gix_repo.status_impl().expect("status with worktree edit");
        assert_eq!(
            *status.staged,
            modified(FileStatusKind::Modified),
            "an unstaged worktree edit must not disturb the staged lane (served from cache)"
        );
        assert_eq!(
            *status.unstaged,
            modified(FileStatusKind::Modified),
            "the new worktree edit must appear in the unstaged lane"
        );

        // Externally stage the new content (`git add`): foo leaves the unstaged lane but stays
        // staged. The index changed, so the cached staged result must be invalidated and recomputed.
        git_success(workdir, &["add", "foo.txt"]);
        let status = gix_repo.status_impl().expect("status after restaging");
        assert_eq!(*status.staged, modified(FileStatusKind::Modified));
        assert!(
            status.unstaged.is_empty(),
            "staging the worktree edit must clear the unstaged lane (cache must invalidate)"
        );

        // Externally unstage (`git reset`): foo leaves the staged lane and (re)enters the unstaged
        // lane — the exact transition the freshness bug got wrong. Again the cache must invalidate.
        git_success(workdir, &["reset", "HEAD", "--", "foo.txt"]);
        let status = gix_repo.status_impl().expect("status after unstaging");
        assert!(
            status.staged.is_empty(),
            "unstaging must remove foo from the staged lane (stale cache would keep it)"
        );
        assert_eq!(
            *status.unstaged,
            modified(FileStatusKind::Modified),
            "the unstaged file must appear in the unstaged lane"
        );

        // The per-lane entry points must agree with the combined status after all the churn.
        assert_eq!(
            gix_repo.staged_status_impl().expect("staged lane"),
            *status.staged
        );
        assert_eq!(
            gix_repo.worktree_status_impl().expect("worktree lane"),
            *status.unstaged
        );
    }

    /// The in-process pass has to beat two `git diff --numstat` spawns without
    /// moving the status walk. Reports timings rather than asserting; run with
    /// `-- --ignored --nocapture`.
    #[test]
    #[ignore = "timing probe"]
    fn perf_uncommitted_line_stats_baseline() {
        // ~10 KB per file. The `status_dirty_500_files` fixture writes ~30
        // bytes, so it measures lstat throughput, not content I/O.
        fn body(seed: usize, marker: &str) -> String {
            let mut out = String::with_capacity(10_240);
            for line in 0..200 {
                out.push_str(&format!(
                    "{marker} {seed:05} line {line:03} some representative source text here\n"
                ));
            }
            out
        }

        for (tracked, dirty, staged) in [(500usize, 25usize, 25usize), (500, 250, 0)] {
            let tmp = tempfile::tempdir().expect("tempdir");
            let workdir = tmp.path();
            init_test_repo(workdir);

            for index in 0..tracked {
                write_file(
                    workdir,
                    &format!("src/mod{:02}/file{index:04}.rs", index % 32),
                    &body(index, "base"),
                );
            }
            git_success(workdir, &["add", "."]);
            git_success(workdir, &["commit", "-q", "-m", "seed"]);

            for index in 0..dirty {
                write_file(
                    workdir,
                    &format!("src/mod{:02}/file{index:04}.rs", index % 32),
                    &body(index, "edit"),
                );
            }
            for index in dirty..dirty + staged {
                write_file(
                    workdir,
                    &format!("src/mod{:02}/file{index:04}.rs", index % 32),
                    &body(index, "edit"),
                );
            }
            if staged > 0 {
                git_success(workdir, &["add", "src"]);
                for index in 0..dirty {
                    write_file(
                        workdir,
                        &format!("src/mod{:02}/file{index:04}.rs", index % 32),
                        &body(index, "again"),
                    );
                }
            }

            let gix_repo = open_repo(workdir);
            // Warm the caches so the numbers compare work, not first touch.
            let _ = gix_repo.status_impl().expect("warmup status");
            let _ = git_output(workdir, &["diff", "--numstat", "-z"]);

            let _ = gix_repo
                .uncommitted_line_stats_impl(&gitcomet_core::services::CancellationToken::new())
                .expect("warmup line stats");

            let mut status_ms = u128::MAX;
            let mut spawn_ms = u128::MAX;
            let mut in_process_ms = u128::MAX;
            for _ in 0..5 {
                let start = std::time::Instant::now();
                let status = gix_repo.status_impl().expect("status");
                status_ms = status_ms.min(start.elapsed().as_millis());
                std::hint::black_box(status);

                let start = std::time::Instant::now();
                let a = git_output(workdir, &["diff", "--numstat", "-z", "--no-renames"]);
                let b = git_output(
                    workdir,
                    &["diff", "--cached", "--numstat", "-z", "--no-renames"],
                );
                spawn_ms = spawn_ms.min(start.elapsed().as_millis());
                std::hint::black_box((a, b));

                let start = std::time::Instant::now();
                let stats = gix_repo
                    .uncommitted_line_stats_impl(&gitcomet_core::services::CancellationToken::new())
                    .expect("line stats");
                in_process_ms = in_process_ms.min(start.elapsed().as_millis());
                std::hint::black_box(stats);
            }

            println!(
                "tracked={tracked} dirty={dirty} staged={staged}: gix status {status_ms} ms | \
                 two numstat spawns {spawn_ms} ms | in-process pass {in_process_ms} ms (best of 5)"
            );
        }
    }

    /// The first full scan must warm the same cache that lane-only scans use.
    #[test]
    fn cold_status_retains_clean_file_stats_and_invalidates_them_after_staging() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        init_test_repo(workdir);
        let timestamp = filetime::FileTime::now().unix_seconds() - 30;
        let touched = STAT_REFRESH_MIN_ENTRIES + 16;
        for ix in 0..touched {
            write_file(workdir, &format!("{ix:03}.txt"), "unchanged\n");
            filetime::set_file_mtime(
                workdir.join(format!("{ix:03}.txt")),
                filetime::FileTime::from_unix_time(timestamp, 0),
            )
            .unwrap();
        }
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-q", "-m", "seed"]);
        let index_before = std::fs::read(workdir.join(".git/index")).unwrap();
        for ix in 0..touched {
            filetime::set_file_mtime(
                workdir.join(format!("{ix:03}.txt")),
                filetime::FileTime::from_unix_time(timestamp + 1, 0),
            )
            .unwrap();
        }
        let repo = open_repo(workdir);
        let status = repo.status_impl().unwrap();
        assert!(status.staged.is_empty() && status.unstaged.is_empty());
        {
            let cache = repo.stat_refreshed_index.lock().unwrap();
            let cache = cache
                .as_ref()
                .expect("the very first status retains clean-file metadata");
            assert!(
                cache
                    .index
                    .entries()
                    .iter()
                    .all(|entry| entry.stat.mtime.secs == (timestamp + 1) as u32)
            );
        }
        assert!(repo.status_impl().unwrap().unstaged.is_empty());
        assert_eq!(
            std::fs::read(workdir.join(".git/index")).unwrap(),
            index_before,
            "status must not rewrite the disk index"
        );
        write_file(workdir, "000.txt", "different\n");
        assert_eq!(
            repo.status_impl().unwrap().unstaged.as_ref(),
            &[file_status("000.txt", FileStatusKind::Modified)]
        );
        git_success(workdir, &["add", "000.txt"]);
        let status = repo.status_impl().unwrap();
        assert!(status.unstaged.is_empty());
        assert_eq!(
            status.staged.as_ref(),
            &[file_status("000.txt", FileStatusKind::Modified)]
        );
    }

    #[cfg(unix)]
    #[test]
    fn repeated_status_does_not_run_clean_filters_again_after_the_first_scan() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        init_test_repo(workdir);
        write_file(workdir, ".gitattributes", "large.txt filter=count\n");
        write_file(workdir, "large.txt", &"unchanged text\n".repeat(40_000));
        let counter = workdir.join(".git/hash-count");
        let clean = format!(
            "printf x >> '{}'; cat",
            counter.to_str().unwrap().replace('\'', "'\\''")
        );
        git_success(workdir, &["config", "filter.count.clean", &clean]);
        let timestamp = filetime::FileTime::now().unix_seconds() - 30;
        filetime::set_file_mtime(
            workdir.join("large.txt"),
            filetime::FileTime::from_unix_time(timestamp, 0),
        )
        .unwrap();
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-q", "-m", "seed"]);
        std::fs::remove_file(workdir.join(".git/hash-count")).unwrap();
        filetime::set_file_mtime(
            workdir.join("large.txt"),
            filetime::FileTime::from_unix_time(timestamp + 1, 0),
        )
        .unwrap();
        let repo = open_repo(workdir);
        assert!(repo.status_impl().unwrap().unstaged.is_empty());
        let hashes = std::fs::read(workdir.join(".git/hash-count")).unwrap();
        assert!(
            !hashes.is_empty(),
            "the initial scan verified the stale file metadata"
        );
        assert!(repo.status_impl().unwrap().unstaged.is_empty());
        assert_eq!(
            std::fs::read(workdir.join(".git/hash-count")).unwrap(),
            hashes,
            "warm status must skip expensive filtering and hashing of unchanged contents"
        );
        write_file(workdir, "large.txt", &"different text\n".repeat(40_000));
        assert_eq!(
            repo.status_impl().unwrap().unstaged[0].path,
            PathBuf::from("large.txt")
        );
        assert!(
            std::fs::read(workdir.join(".git/hash-count"))
                .unwrap()
                .len()
                > hashes.len(),
            "a subsequent edit is still verified"
        );
    }

    /// The in-memory stat refresh moves the index timestamp past every entry it
    /// verified. A racily clean entry whose content did change must then keep
    /// comparing as changed, so the refresh zeroes its size as git's index
    /// write does; with only the new timestamp it would read as clean.

    #[test]
    fn stat_refresh_keeps_a_racily_modified_file_modified() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        // Rewriting the file below changes its ctime; the race is about mtime.
        git_success(workdir, &["config", "core.trustctime", "false"]);
        // Enough touched-but-unchanged files for the refresh to be kept.
        let touched = STAT_REFRESH_MIN_ENTRIES + 16;
        for index in 0..touched {
            write_file(workdir, &format!("touched/{index:03}.txt"), "same\n");
        }
        write_file(workdir, "racy.txt", "foo\n");
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-q", "-m", "seed"]);

        let now = filetime::FileTime::now().unix_seconds();
        let entry_mtime = filetime::FileTime::from_unix_time(now - 50, 0);
        filetime::set_file_mtime(workdir.join("racy.txt"), entry_mtime).expect("mtime");
        git_success(workdir, &["update-index", "-q", "--refresh"]);
        // Same size and mtime as the index entry, different content.
        write_file(workdir, "racy.txt", "bar\n");
        filetime::set_file_mtime(workdir.join("racy.txt"), entry_mtime).expect("mtime");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        for index in 0..touched {
            write_file(workdir, &format!("touched/{index:03}.txt"), "same\n");
        }
        // An index older than the entry makes it racy.
        filetime::set_file_mtime(
            workdir.join(".git/index"),
            filetime::FileTime::from_unix_time(now - 100, 0),
        )
        .expect("index mtime");

        let repo = open_repo(workdir);
        let expected = vec![file_status("racy.txt", FileStatusKind::Modified)];
        assert_eq!(repo.worktree_status_impl().expect("status"), expected);
        assert!(
            repo.stat_refreshed_index.lock().expect("lock").is_some(),
            "the touched files should have produced an in-memory refresh"
        );
        assert_eq!(repo.worktree_status_impl().expect("status"), expected);
    }

    /// A write landing right after a walk starts can carry a coarser mtime
    /// from the second before, below the walk's nanosecond clock. Git's
    /// index stamp is such an mtime itself; the in-memory stamp must stay
    /// behind the walk's second so that write still compares racy.
    #[test]
    fn stat_refresh_catches_a_write_stamped_just_before_the_walk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        init_test_repo(workdir);
        git_success(workdir, &["config", "core.trustctime", "false"]);
        let touched = STAT_REFRESH_MIN_ENTRIES + 16;
        for index in 0..touched {
            write_file(workdir, &format!("touched/{index:03}.txt"), "same\n");
        }
        write_file(workdir, "racy.txt", "foo\n");
        git_success(workdir, &["add", "."]);
        git_success(workdir, &["commit", "-q", "-m", "seed"]);
        // Touched, so the walk produces the in-memory refresh.
        for index in 0..touched {
            write_file(workdir, &format!("touched/{index:03}.txt"), "same\n");
        }
        let repo = open_repo(workdir);

        // Early in a second, so the walk below stays inside it.
        let now = loop {
            let now = filetime::FileTime::now();
            if now.nanoseconds() < 300_000_000 {
                break now;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        // The stamp a write a few milliseconds after the walk starts can get
        // from the file clock: the last instant of the previous second.
        let just_before = filetime::FileTime::from_unix_time(now.unix_seconds() - 1, 999_000_000);
        filetime::set_file_mtime(workdir.join("racy.txt"), just_before).expect("mtime");
        assert_eq!(repo.worktree_status_impl().expect("status"), vec![]);
        assert_eq!(
            filetime::FileTime::now().unix_seconds(),
            now.unix_seconds(),
            "the walk left the second it started in; rerun"
        );
        assert!(repo.stat_refreshed_index.lock().expect("lock").is_some());

        // Same size and stat as the refreshed entry, different content.
        write_file(workdir, "racy.txt", "bar\n");
        filetime::set_file_mtime(workdir.join("racy.txt"), just_before).expect("mtime");
        assert_eq!(
            repo.worktree_status_impl().expect("status"),
            vec![file_status("racy.txt", FileStatusKind::Modified)]
        );
    }

    /// Worktree status after files were touched without changing content, as
    /// build tools, formatters and editors saving unchanged buffers do. Status
    /// never writes the index back, so each refresh re-hashes every such file
    /// unless the refreshed stats are remembered.
    #[test]
    #[ignore = "timing probe"]
    fn timing_worktree_status_after_touch() {
        // Process CPU time: the hashing runs on every core, so wall time on a
        // wide machine hides most of it.
        fn cpu_ms() -> f64 {
            let stat = fs::read_to_string("/proc/self/stat").unwrap_or_default();
            let fields: Vec<&str> = stat
                .rsplit_once(')')
                .map(|(_, rest)| rest.split_whitespace().collect())
                .unwrap_or_default();
            let ticks: f64 = fields
                .get(11..13)
                .map(|t| t.iter().filter_map(|v| v.parse::<f64>().ok()).sum())
                .unwrap_or(0.0);
            ticks * 10.0
        }
        let body = "some representative source text here\n".repeat(256);
        for (files, attributes) in [(2_000usize, false), (10_000, false), (10_000, true)] {
            let tmp = tempfile::tempdir().expect("tempdir");
            let workdir = tmp.path();
            init_test_repo(workdir);
            if attributes {
                write_file(workdir, ".gitattributes", "* text=auto\n");
            }
            for index in 0..files {
                write_file(
                    workdir,
                    &format!("src/mod{:02}/file{index:05}.rs", index % 64),
                    &body,
                );
            }
            git_success(workdir, &["add", "."]);
            git_success(workdir, &["commit", "-q", "-m", "seed"]);
            // Rewrite identical bytes: new mtimes (past git's one-second
            // granularity), same content.
            std::thread::sleep(std::time::Duration::from_millis(1100));
            for index in 0..files {
                write_file(
                    workdir,
                    &format!("src/mod{:02}/file{index:05}.rs", index % 64),
                    &body,
                );
            }
            let gix_repo = open_repo(workdir);
            let start = std::time::Instant::now();
            let first = gix_repo.worktree_status_impl().expect("status");
            let first_ms = start.elapsed().as_secs_f64() * 1e3;
            assert!(first.is_empty(), "touched files are unchanged");
            // Refreshes are debounced (250 ms to 2 s): the next one starts in a
            // later second, when the touched files are no longer racy.
            std::thread::sleep(std::time::Duration::from_millis(1100));
            let mut best = f64::MAX;
            let cpu_start = cpu_ms();
            for _ in 0..20 {
                let start = std::time::Instant::now();
                let again = gix_repo.worktree_status_impl().expect("status");
                best = best.min(start.elapsed().as_secs_f64() * 1e3);
                assert!(again.is_empty());
            }
            let repeat_cpu = (cpu_ms() - cpu_start) / 20.0;
            // Control: the same tree once git has written the new stats back.
            git_success(workdir, &["update-index", "-q", "--refresh"]);
            let fresh = open_repo(workdir);
            let _ = fresh.worktree_status_impl().expect("status");
            let mut clean = f64::MAX;
            let cpu_start = cpu_ms();
            for _ in 0..20 {
                let start = std::time::Instant::now();
                let _ = fresh.worktree_status_impl().expect("status");
                clean = clean.min(start.elapsed().as_secs_f64() * 1e3);
            }
            let clean_cpu = (cpu_ms() - cpu_start) / 20.0;
            println!(
                "timing worktree_status_after_touch files={files} text_auto={attributes} first={first_ms:.2}ms repeat_best={best:.2}ms repeat_cpu={repeat_cpu:.1}ms refreshed_index={clean:.2}ms refreshed_cpu={clean_cpu:.1}ms"
            );
        }
    }
}
