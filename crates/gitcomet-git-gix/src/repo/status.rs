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
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

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
        let repo = self._repo.to_thread_local();
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

        let mut unstaged = Vec::new();
        let mut has_conflicted_unstaged = false;

        let (staged, index_stamp_after_write) = if let Some(cached_staged) = cached_staged {
            // Fast path: HEAD and index unchanged — skip Tree→Index comparison and
            // collect Index→Worktree changes directly without the generic iterator's
            // extra thread/channel hop.
            let direct =
                collect_index_worktree_status_direct(&repo, &mut unstaged, may_have_gitlinks)?;
            cancellation.check_cancelled()?;
            has_conflicted_unstaged = direct.has_conflicted_unstaged;
            (cached_staged, direct.index_stamp_after_write)
        } else {
            // Full path: run both Tree→Index and Index→Worktree comparisons.
            cancellation.check_cancelled()?;
            let platform = repo
                .status(gix::progress::Discard)
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix status platform: {e}"))))?
                // GitComet supplements gitlink/submodule status separately to match
                // `git status` parity, so skip gix's default submodule probing on the
                // common no-submodule path.
                .index_worktree_submodules(None)
                .untracked_files(gix::status::UntrackedFiles::Files);
            let mut staged = Vec::new();
            let mut iter = platform
                .into_iter(std::iter::empty::<gix::bstr::BString>())
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix status iter: {e}"))))?;

            for item in iter.by_ref() {
                cancellation.check_cancelled()?;
                let item = item
                    .map_err(|e| Error::new(ErrorKind::Backend(format!("gix status item: {e}"))))?;

                match item {
                    gix::status::Item::IndexWorktree(item) => {
                        collect_index_worktree_item(
                            item,
                            &mut unstaged,
                            &mut has_conflicted_unstaged,
                        )?;
                    }

                    gix::status::Item::TreeIndex(change) => {
                        collect_tree_index_change(change, &mut staged)?;
                    }
                }
            }
            let index_stamp_after_write =
                maybe_persist_status_outcome_changes(iter.into_outcome(), &repo.index_path());

            (staged, index_stamp_after_write)
        };
        let final_index_stamp = index_stamp_after_write
            .clone()
            .unwrap_or_else(|| index_stamp.clone());

        if !used_cached_staged || index_stamp_after_write.is_some() {
            // Status write-back updates only index stat metadata, not staged content. Refresh the
            // cache stamp so repeated clean refreshes can keep skipping Tree→Index work.
            *self
                .tree_index_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(TreeIndexCacheEntry {
                head_oid,
                index_stamp: final_index_stamp.clone(),
                staged: staged.clone(),
            });
        }

        if used_cached_staged && let Some(updated_index_stamp) = index_stamp_after_write {
            let mut cache = self
                .gitlink_status_capability
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(cached) = cache.as_mut() {
                cached.index = updated_index_stamp;
            }
        }

        cancellation.check_cancelled()?;
        finalize_status(
            &self.spec.workdir,
            &repo,
            may_have_gitlinks,
            staged,
            unstaged,
            has_conflicted_unstaged,
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
        let repo = self._repo.to_thread_local();
        let index_stamp = repo_index_stamp(&repo);
        let may_have_gitlinks = self.may_have_gitlink_status_supplement(&repo, &index_stamp);
        let mut unstaged = Vec::new();
        let direct = collect_index_worktree_status_direct(&repo, &mut unstaged, may_have_gitlinks)?;
        cancellation.check_cancelled()?;

        if should_supplement_unmerged_conflicts(
            repo.state().is_some(),
            direct.has_conflicted_unstaged,
        ) {
            apply_unmerged_conflicts(&repo, &mut unstaged)?;
            cancellation.check_cancelled()?;
        }

        if may_have_gitlinks {
            supplement_gitlink_status_from_porcelain(
                &self.spec.workdir,
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
        let repo = self._repo.to_thread_local();
        let head_oid = super::history::gix_head_id_or_none(&repo)?;
        let index_stamp = repo_index_stamp(&repo);
        cancellation.check_cancelled()?;

        if let Some(cached) = self.cached_staged_status(head_oid, &index_stamp) {
            return Ok(cached);
        }

        let Some(head_oid) = head_oid else {
            return self
                .status_cancellable_impl(cancellation)
                .map(|status| status.staged);
        };

        // `tree_index_status()` diffs a tree against the index, so resolve HEAD to HEAD^{tree}
        // while continuing to cache by commit id.
        let head_tree_id = tree_id_for_commit(&repo, &head_oid)?;
        let mut staged = collect_staged_status_from_tree_index(&repo, &head_tree_id)?;
        cancellation.check_cancelled()?;
        if self.may_have_gitlink_status_supplement(&repo, &index_stamp) {
            supplement_gitlink_status_from_porcelain(
                &self.spec.workdir,
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

    pub(super) fn upstream_divergence_impl(&self) -> Result<Option<UpstreamDivergence>> {
        self.upstream_divergence_cancellable_impl(&CancellationToken::new())
    }

    pub(super) fn upstream_divergence_cancellable_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Option<UpstreamDivergence>> {
        cancellation.check_cancelled()?;
        let repo = self.reopen_repo()?;
        let divergence = head_upstream_divergence(&repo)?;
        cancellation.check_cancelled()?;
        Ok(divergence)
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
    if should_supplement_unmerged_conflicts(repo.state().is_some(), has_conflicted_unstaged) {
        apply_unmerged_conflicts(repo, &mut unstaged)?;
    }

    // Only shell out for gitlink/submodule status when the repo is likely to contain submodules
    // or gitlinks. This avoids a full `git status` subprocess on every refresh for the common
    // case.
    if may_have_gitlinks {
        supplement_gitlink_status_from_porcelain(workdir, &mut staged, &mut unstaged)?;
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

    Ok(RepoStatus { staged, unstaged })
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

fn tree_id_for_commit(repo: &gix::Repository, commit_id: &gix::ObjectId) -> Result<gix::ObjectId> {
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
        gix::status::tree_index::TrackRenames::AsConfigured,
        |change, _, _| {
            collect_tree_index_change(change, &mut staged)?;
            Ok::<_, Error>(std::ops::ControlFlow::Continue(()))
        },
    )
    .map_err(|e| Error::new(ErrorKind::Backend(format!("gix tree/index status: {e}"))))?;
    Ok(staged)
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
    let conflicted: HashSet<PathBuf> = conflicted.into_iter().collect();
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
fn repo_index_stamp(repo: &gix::Repository) -> RepoFileStamp {
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

fn gix_unmerged_conflicts(repo: &gix::Repository) -> Result<Vec<(PathBuf, FileConflictKind)>> {
    let index = repo
        .index_or_load_from_head_or_empty()
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
    let mut stage_masks: HashMap<PathBuf, u8> = HashMap::default();

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

/// Collect a single IndexWorktree item into the `unstaged` list.  Shared by
/// both the full status iterator and the index-worktree-only fast path.
fn collect_index_worktree_item(
    item: gix::status::index_worktree::Item,
    unstaged: &mut Vec<FileStatus>,
    has_conflicted_unstaged: &mut bool,
) -> Result<()> {
    match item {
        gix::status::index_worktree::Item::Modification {
            rela_path, status, ..
        } => {
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
        gix::status::index_worktree::Item::DirectoryContents { entry, .. } => {
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
        gix::status::index_worktree::Item::Rewrite {
            dirwalk_entry,
            copy,
            ..
        } => {
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

fn collect_index_worktree_status_direct(
    repo: &gix::Repository,
    unstaged: &mut Vec<FileStatus>,
    may_have_gitlinks: bool,
) -> Result<DirectIndexWorktreeStatus> {
    let index = repo
        .index_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    collect_index_worktree_status_direct_from_index(repo, &index, unstaged, may_have_gitlinks)
}

fn collect_index_worktree_status_direct_from_index(
    repo: &gix::Repository,
    index: &gix::worktree::Index,
    unstaged: &mut Vec<FileStatus>,
    may_have_gitlinks: bool,
) -> Result<DirectIndexWorktreeStatus> {
    let dirwalk_options = repo
        .dirwalk_options()
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix status dirwalk options: {e}"
            )))
        })?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching);
    let collection = if may_have_gitlinks {
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
        )?
    } else {
        collect_index_worktree_status_direct_with_submodule(
            repo,
            index,
            dirwalk_options,
            unstaged,
            NoopSubmoduleStatus,
        )?
    };
    let index_stamp_after_write =
        maybe_persist_direct_index_changes(repo, index, collection.index_changes);
    Ok(DirectIndexWorktreeStatus {
        has_conflicted_unstaged: collection.has_conflicted_unstaged,
        index_stamp_after_write,
    })
}

#[derive(Clone, Copy)]
struct NoopSubmoduleStatus;

impl gix::status::plumbing::index_as_worktree::traits::SubmoduleStatus for NoopSubmoduleStatus {
    type Output = gix::submodule::Status;
    type Error = Infallible;

    fn status(
        &mut self,
        _entry: &gix::index::Entry,
        _rela_path: &gix::bstr::BStr,
    ) -> std::result::Result<Option<Self::Output>, Self::Error> {
        Ok(None)
    }
}

fn collect_index_worktree_status_direct_with_submodule<S, E>(
    repo: &gix::Repository,
    index: &gix::worktree::Index,
    dirwalk_options: gix::dirwalk::Options,
    unstaged: &mut Vec<FileStatus>,
    submodule: S,
) -> Result<StatusEntryCollection>
where
    S: gix::status::plumbing::index_as_worktree::traits::SubmoduleStatus<
            Output = gix::submodule::Status,
            Error = E,
        > + Send
        + Clone,
    E: std::error::Error + Send + Sync + 'static,
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
        || -> std::result::Result<
            gix::worktree::Stack,
            Box<dyn std::error::Error + Send + Sync + 'static>,
        > {
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
    let mut progress = gix::progress::Discard;
    let should_interrupt = AtomicBool::new(false);
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
            should_interrupt: &should_interrupt,
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
                thread_limit: None,
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
        Error::new(ErrorKind::Backend(format!(
            "gix status index/worktree: {e}"
        )))
    })?;

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
            rela_path,
            status,
            ..
        } => {
            if let gix::status::plumbing::index_as_worktree::EntryStatus::NeedsUpdate(_stat) =
                &status
            {
                index_changes.push(IndexWorktreeApplyChange::NewStat);
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
                index_changes.push(IndexWorktreeApplyChange::SetSizeToZero);
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
            dirwalk_entry,
            copy,
            ..
        } => {
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
    index_stamp_after_write: Option<RepoFileStamp>,
}

struct StatusEntryCollection {
    has_conflicted_unstaged: bool,
    index_changes: Vec<IndexWorktreeApplyChange>,
}

enum IndexWorktreeApplyChange {
    NewStat,
    SetSizeToZero,
}

fn maybe_persist_status_outcome_changes(
    _outcome: Option<gix::status::Outcome>,
    _index_path: &Path,
) -> Option<RepoFileStamp> {
    // INVARIANT: status reads must not rewrite `.git/index`. The repo monitor maps index updates
    // to refreshes, so gix's stat write-back would self-trigger a refresh loop even when the
    // status payload is unchanged.
    //
    // The reducer relies on this: `finish_status_lane_replay` (gitcomet-state) now *always*
    // replays a coalesced refresh, on the assumption that a completed status read produces no
    // filesystem events. If this ever returns `Some(..)` (persisting a write-back), revisit that
    // reducer logic first or the loop returns.
    None
}

fn maybe_persist_direct_index_changes(
    _repo: &gix::Repository,
    _index: &gix::worktree::Index,
    _index_changes: Vec<IndexWorktreeApplyChange>,
) -> Option<RepoFileStamp> {
    // Same invariant as `maybe_persist_status_outcome_changes`: keep status collection read-only
    // so monitor-driven refreshes do not recursively manufacture new worktree events, which the
    // reducer's unconditional replay depends on.
    None
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
    staged: &mut Vec<FileStatus>,
    unstaged: &mut Vec<FileStatus>,
) -> Result<()> {
    let mut command = git_workdir_cmd_for(workdir);
    command
        .arg("--no-optional-locks")
        .arg("status")
        .arg("--porcelain=v2")
        .arg("-z")
        .arg("--ignore-submodules=none");
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

#[cfg(test)]
mod tests {
    use super::{
        apply_porcelain_v2_gitlink_status_record, collect_unmerged_conflicts,
        conflict_kind_from_stage_mask, map_directory_entry_status, map_entry_status,
        map_porcelain_v2_status_char, remove_conflicted_paths_from_staged,
        should_supplement_unmerged_conflicts, sort_and_dedup_status_entries, tree_id_for_commit,
    };
    use gitcomet_core::domain::{FileConflictKind, FileStatus, FileStatusKind};
    use rustc_hash::FxHashMap as HashMap;
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

    fn git_success(workdir: &Path, args: &[&str]) {
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

    fn init_test_repo(workdir: &Path) {
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

    fn write_file(workdir: &Path, relative: &str, contents: &str) {
        let path = workdir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(path, contents).expect("write file");
    }

    fn open_repo(workdir: &Path) -> super::super::GixRepo {
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
            .collect::<HashMap<PathBuf, FileConflictKind>>();

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

        assert_eq!(combined.staged, staged);
        assert_eq!(combined.unstaged, unstaged);
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
            status.staged,
            vec![file_status("foo.txt", FileStatusKind::Modified)],
            "a staged-and-re-edited file must appear in the staged lane"
        );
        assert_eq!(
            status.unstaged,
            vec![file_status("foo.txt", FileStatusKind::Modified)],
            "the further worktree edit must also appear in the unstaged lane"
        );
        // The per-lane entry points must agree with the combined status.
        assert_eq!(
            gix_repo.staged_status_impl().expect("staged lane"),
            status.staged
        );
        assert_eq!(
            gix_repo.worktree_status_impl().expect("worktree lane"),
            status.unstaged
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

        assert_eq!(combined.staged, staged);
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
        assert_eq!(combined.unstaged, worktree);
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
        assert_eq!(status.staged, modified(FileStatusKind::Modified));
        assert!(status.unstaged.is_empty());

        // Edit the worktree again WITHOUT staging: foo is now in BOTH lanes. The index is unchanged,
        // so the staged lane is served from the cache while the unstaged lane is recomputed.
        write_file(workdir, "foo.txt", "v2\n");
        let status = gix_repo.status_impl().expect("status with worktree edit");
        assert_eq!(
            status.staged,
            modified(FileStatusKind::Modified),
            "an unstaged worktree edit must not disturb the staged lane (served from cache)"
        );
        assert_eq!(
            status.unstaged,
            modified(FileStatusKind::Modified),
            "the new worktree edit must appear in the unstaged lane"
        );

        // Externally stage the new content (`git add`): foo leaves the unstaged lane but stays
        // staged. The index changed, so the cached staged result must be invalidated and recomputed.
        git_success(workdir, &["add", "foo.txt"]);
        let status = gix_repo.status_impl().expect("status after restaging");
        assert_eq!(status.staged, modified(FileStatusKind::Modified));
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
            status.unstaged,
            modified(FileStatusKind::Modified),
            "the unstaged file must appear in the unstaged lane"
        );

        // The per-lane entry points must agree with the combined status after all the churn.
        assert_eq!(
            gix_repo.staged_status_impl().expect("staged lane"),
            status.staged
        );
        assert_eq!(
            gix_repo.worktree_status_impl().expect("worktree lane"),
            status.unstaged
        );
    }
}
