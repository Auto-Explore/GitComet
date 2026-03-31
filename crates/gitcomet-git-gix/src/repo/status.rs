use super::{
    GitlinkStatusCapabilityCacheEntry, GixRepo, RepoFileStamp, TreeIndexCacheEntry,
    conflict_stages::conflict_kind_from_stage_mask, git_ops::head_upstream_divergence,
};
use crate::util::{git_workdir_cmd_for, path_buf_from_git_bytes, run_git_raw_output};
use gitcomet_core::domain::{
    FileConflictKind, FileStatus, FileStatusKind, RepoStatus, UpstreamDivergence,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

impl GixRepo {
    fn may_have_gitlink_status_supplement(&self, repo: &gix::Repository) -> bool {
        let gitmodules = repo_file_stamp(self.spec.workdir.join(".gitmodules").as_path());
        let index = repo_file_stamp(repo.index_path().as_path());

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
        let repo = self._repo.to_thread_local();
        let may_have_gitlinks = self.may_have_gitlink_status_supplement(&repo);

        // Check whether HEAD and the index file are unchanged since the last
        // status call.  When both match, the staged (Tree→Index) result is
        // identical and we can skip the tree comparison entirely, using the
        // cheaper index-worktree-only iterator.
        let head_oid = super::history::gix_head_id_or_none(&repo)?;
        let index_stamp = repo_file_stamp(repo.index_path().as_path());

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
            has_conflicted_unstaged = direct.has_conflicted_unstaged;
            (cached_staged, direct.index_stamp_after_write)
        } else {
            // Full path: run both Tree→Index and Index→Worktree comparisons.
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

        let mut staged = staged;

        // Some platforms may omit certain unmerged shapes (notably stage-1-only
        // both-deleted conflicts) from gix status output. Supplement conflict
        // entries from the index's unmerged stages only when the repository is
        // in an in-progress operation or gix already surfaced conflicts.
        if should_supplement_unmerged_conflicts(repo.state().is_some(), has_conflicted_unstaged) {
            for (path, conflict_kind) in gix_unmerged_conflicts(&repo)? {
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
        }

        // Only shell out for gitlink/submodule status when the repo is likely
        // to contain submodules or gitlinks.  This avoids a full `git status`
        // subprocess on every refresh for the common case.
        if may_have_gitlinks {
            supplement_gitlink_status_from_porcelain(
                &self.spec.workdir,
                &mut staged,
                &mut unstaged,
            )?;
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

        fn sort_and_dedup(entries: &mut Vec<FileStatus>) {
            entries.sort_unstable_by(|a, b| {
                a.path
                    .cmp(&b.path)
                    .then_with(|| kind_priority(b.kind).cmp(&kind_priority(a.kind)))
            });
            entries.dedup_by(|a, b| a.path == b.path);
        }

        sort_and_dedup(&mut staged);
        sort_and_dedup(&mut unstaged);

        // gix may report unmerged entries (conflicts) as both Index/Worktree and Tree/Index
        // changes, which causes the same path to show up in both sections in the UI. Mirror
        // `git status` behavior by showing conflicted paths only once.
        let conflicted: HashSet<std::path::PathBuf> = unstaged
            .iter()
            .filter(|e| e.kind == FileStatusKind::Conflicted)
            .map(|e| e.path.clone())
            .collect();
        if !conflicted.is_empty() {
            staged.retain(|e| !conflicted.contains(&e.path));
        }

        Ok(RepoStatus { staged, unstaged })
    }

    pub(super) fn upstream_divergence_impl(&self) -> Result<Option<UpstreamDivergence>> {
        let repo = self.reopen_repo()?;
        head_upstream_divergence(&repo)
    }
}

fn should_supplement_unmerged_conflicts(
    repo_has_in_progress_state: bool,
    has_conflicted_unstaged: bool,
) -> bool {
    repo_has_in_progress_state || has_conflicted_unstaged
}

fn repo_file_stamp(path: &Path) -> RepoFileStamp {
    match std::fs::metadata(path) {
        Ok(metadata) => RepoFileStamp {
            exists: true,
            len: metadata.len(),
            modified: metadata.modified().ok(),
        },
        Err(_) => RepoFileStamp::default(),
    }
}

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
            tracked_file_modifications: gix::status::plumbing::index_as_worktree::Options {
                fs: fs_caps,
                thread_limit: None,
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
            entry_index,
            ..
        } => {
            if let gix::status::plumbing::index_as_worktree::EntryStatus::NeedsUpdate(stat) =
                &status
            {
                let _ = entry_index;
                let _ = stat;
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
                let _ = entry_index;
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
    // Avoid rewriting `.git/index` during status reads. The repo monitor maps index updates to
    // worktree refreshes, so gix's stat write-back can self-trigger a refresh loop even when the
    // status payload itself is unchanged.
    None
}

fn maybe_persist_direct_index_changes(
    _repo: &gix::Repository,
    _index: &gix::worktree::Index,
    _index_changes: Vec<IndexWorktreeApplyChange>,
) -> Option<RepoFileStamp> {
    // Same rationale as `maybe_persist_status_outcome_changes`: keep status collection read-only
    // so monitor-driven refreshes do not recursively manufacture new worktree events.
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
        conflict_kind_from_stage_mask, map_directory_entry_status,
        should_supplement_unmerged_conflicts,
    };
    use gitcomet_core::domain::{FileConflictKind, FileStatusKind};
    use rustc_hash::FxHashMap as HashMap;
    use std::path::PathBuf;

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
}
