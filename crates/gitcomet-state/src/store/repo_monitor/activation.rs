//! Focus refreshes must tolerate missed watcher events without reloading all
//! history and references when only working files could have changed.
use super::*;
use std::collections::BTreeMap;

const MAX_METADATA_ENTRIES: usize = 16_384;
const MAX_CONTROL_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Default)]
pub(super) struct ActivationRefresh {
    previous: Option<MetadataSnapshot>,
    context: Option<ActivationContext>,
}

/// A failed refresh can retain its old Ready values. Diagnostics and the load
/// epoch distinguish that state from a successful refresh of identical files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::store) struct ActivationContext {
    lifetime: u64,
    epoch: crate::model::RepoLoadEpoch,
    diagnostics_seq: u64,
    revisions: [u64; 10],
    upstream: crate::model::Loadable<Option<gitcomet_core::domain::UpstreamDivergence>>,
    sequencer: crate::model::Loadable<gitcomet_core::services::SequencerState>,
    merge_message: crate::model::Loadable<Option<String>>,
    rebase: crate::model::Loadable<bool>,
    ready: bool,
}

impl ActivationContext {
    #[cfg(test)]
    pub(super) fn ready_for_test() -> Self {
        Self {
            lifetime: 0,
            epoch: Default::default(),
            diagnostics_seq: 0,
            revisions: [0; 10],
            upstream: crate::model::Loadable::Ready(None),
            sequencer: crate::model::Loadable::Ready(Default::default()),
            merge_message: crate::model::Loadable::Ready(None),
            rebase: crate::model::Loadable::Ready(false),
            ready: true,
        }
    }
    pub(in crate::store) fn for_repo(repo: &crate::model::RepoState) -> Self {
        use crate::model::Loadable;
        Self {
            lifetime: repo.lifetime(),
            epoch: repo.load_epoch,
            diagnostics_seq: repo.feedback.diagnostics_seq,
            // A watched change can load B, then an unseen reset can restore
            // disk A. Disk A matching the last activation is insufficient:
            // the currently displayed state must still match that baseline.
            revisions: [
                repo.head_branch_rev,
                repo.history_state.log_rev,
                repo.branches_rev,
                repo.remote_branches_rev,
                repo.tags_rev,
                repo.remotes_rev,
                repo.stashes_rev,
                repo.reflog_rev,
                repo.worktrees_rev,
                repo.ops_rev,
            ],
            upstream: repo.upstream_divergence.clone(),
            sequencer: repo.sequencer_state.clone(),
            merge_message: repo.merge_commit_message.clone(),
            rebase: repo.rebase_in_progress.clone(),
            ready: matches!(repo.head_branch, Loadable::Ready(_))
                && matches!(repo.history_state.log, Loadable::Ready(_))
                && matches!(repo.branches, Loadable::Ready(_))
                && matches!(repo.remote_branches, Loadable::Ready(_))
                && matches!(repo.tags, Loadable::Ready(_))
                && matches!(repo.upstream_divergence, Loadable::Ready(_))
                && matches!(repo.sequencer_state, Loadable::Ready(_))
                && matches!(repo.merge_commit_message, Loadable::Ready(_)),
        }
    }
}

pub(in crate::store) struct CheckedActivation {
    pub repo_id: RepoId,
    pub context: ActivationContext,
    pub change: RepoExternalChange,
}

pub(in crate::store) struct ActivationDecision {
    pub change: RepoExternalChange,
    pub refresh_worktree_dirty: bool,
}

impl CheckedActivation {
    /// Loads can finish while the monitor is inspecting the filesystem. Only
    /// the store can decide whether its current view still matches the input.
    pub(in crate::store) fn into_refresh(
        self,
        repo: &crate::model::RepoState,
    ) -> Option<ActivationDecision> {
        if repo.lifetime() != self.context.lifetime {
            return None;
        }
        let change = if self.context == ActivationContext::for_repo(repo) {
            self.change
        } else {
            self.change.union(RepoExternalChange::all())
        };
        Some(ActivationDecision {
            // Full refreshes already request this scan. For a checked,
            // unchanged single checkout, its last successful empty result is
            // still complete: there is no other worktree to inspect. Linked
            // checkouts must still scan working files that native events miss.
            refresh_worktree_dirty: !change.git_state && !settled_single_checkout(repo),
            change,
        })
    }
}

fn settled_single_checkout(repo: &crate::model::RepoState) -> bool {
    use crate::model::Loadable;
    matches!(repo.open, Loadable::Ready(()))
        && matches!(&repo.worktrees, Loadable::Ready(worktrees)
            if worktrees.len() == 1 && worktrees[0].path == repo.spec.workdir)
        && matches!(&repo.worktree_dirty, Loadable::Ready(dirty) if dirty.is_empty())
        && !repo.loads_in_flight.any_in_flight()
        && repo.local_actions_in_flight == 0
}

impl ActivationRefresh {
    pub(super) fn prepare(
        workdir: &Path,
        inputs: &WatchInputs,
    ) -> Result<MetadataSnapshot, SnapshotError> {
        if !inputs.backend_discovered
            || inputs.info.discovery_incomplete
            || inputs.info.git_dirs.is_empty()
        {
            return Err(SnapshotError::Unavailable);
        }
        MetadataSnapshot::capture(workdir, inputs)
    }

    /// Exact control bytes also cover edits that preserve their timestamps.
    /// Let the monitor rediscover includes, submodules and linked checkouts in
    /// its existing revalidation pass, before accepting a new baseline.
    pub(super) fn needs_discovery(
        &self,
        current: Result<&MetadataSnapshot, &SnapshotError>,
    ) -> bool {
        match (self.previous.as_ref(), current) {
            (_, Err(SnapshotError::DiscoveryRequired)) => true,
            (_, Err(SnapshotError::Unavailable)) => false,
            (Some(previous), Ok(current)) => previous.discovery != current.discovery,
            // The monitor's metadata stamps alone cannot establish an exact
            // baseline after startup or an unreadable/oversized probe. A
            // timestamp-preserving config edit may have changed its includes.
            (None, Ok(_)) => true,
        }
    }

    pub(super) fn complete(
        &mut self,
        current: Option<MetadataSnapshot>,
        context: ActivationContext,
    ) -> RepoExternalChange {
        let unchanged = context.ready
            && self.context.as_ref() == Some(&context)
            && current.as_ref().is_some_and(|current| {
                self.previous
                    .as_ref()
                    .is_some_and(|previous| current.entries == previous.entries)
            });
        self.context = Some(context);
        self.previous = current;
        if unchanged {
            RepoExternalChange {
                git_state: false,
                tags: false,
                // Index validation belongs to the monitor: an index rewrite
                // can change submodule/ignore coverage without changing refs.
                // Working files still refresh when native events are missed.
                ..RepoExternalChange::all()
            }
        } else {
            RepoExternalChange::all()
        }
    }

    #[cfg(test)]
    pub(super) fn check(
        &mut self,
        workdir: &Path,
        backend: &dyn GitBackend,
        context: ActivationContext,
    ) -> RepoExternalChange {
        let current = WatchInputs::load(workdir, backend)
            .ok()
            .and_then(|inputs| Self::prepare(workdir, &inputs).ok());
        self.complete(current, context)
    }

    #[cfg(test)]
    pub(super) fn change(
        &mut self,
        workdir: &Path,
        backend: &dyn GitBackend,
    ) -> RepoExternalChange {
        self.check(workdir, backend, ActivationContext::ready_for_test())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileStamp {
    len: u64,
    modified: std::time::SystemTime,
    created: Option<std::time::SystemTime>,
    directory: bool,
    contents: Option<Arc<[u8]>>,
    link: Option<PathBuf>,
    resolved: Option<PathBuf>,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    ctime: (i64, i64),
}

#[derive(Clone, Copy, Debug, Default)]
struct Coverage {
    recursive: bool,
    recursive_contents: bool,
    contents: bool,
}

#[derive(Debug, Default)]
pub(super) struct MetadataSnapshot {
    entries: BTreeMap<PathBuf, Option<FileStamp>>,
    discovery: BTreeMap<PathBuf, Option<FileStamp>>,
    coverage: BTreeMap<PathBuf, Coverage>,
    file_contents: BTreeMap<PathBuf, Arc<[u8]>>,
    git_dirs: Vec<PathBuf>,
    inputs: Vec<PathBuf>,
    bytes: u64,
}

#[derive(Debug)]
pub(super) enum SnapshotError {
    DiscoveryRequired,
    Unavailable,
}

impl From<std::io::Error> for SnapshotError {
    fn from(_: std::io::Error) -> Self {
        Self::Unavailable
    }
}

impl MetadataSnapshot {
    fn capture(workdir: &Path, inputs: &WatchInputs) -> Result<Self, SnapshotError> {
        let mut snapshot = Self {
            git_dirs: inputs.info.git_dirs.clone(),
            inputs: inputs.inputs.clone(),
            ..Self::default()
        };
        for path in &inputs.inputs {
            snapshot.add_discovery(path)?;
        }
        snapshot.add_discovery(&workdir.join(".gitmodules"))?;
        for root in &inputs.info.git_dirs {
            snapshot.add_discovery(root)?;
            for name in ["HEAD", "config", "config.worktree", "commondir", "gitdir"] {
                snapshot.add_discovery(&root.join(name))?;
            }
            // Deliberately omit objects and private caches: reading their
            // contents is not necessary to detect a changed Git view. Ref
            // targets, replacements and shallow boundaries are included.
            // FETCH_HEAD is omitted too: every fetch rewrites it, and the refs
            // a fetch moves are read anyway.
            for name in [
                "ORIG_HEAD",
                "MERGE_HEAD",
                "MERGE_MSG",
                "MERGE_MODE",
                "AUTO_MERGE",
                "CHERRY_PICK_HEAD",
                "REVERT_HEAD",
                "REBASE_HEAD",
                "BISECT_LOG",
                "BISECT_START",
                "BISECT_NAMES",
                "BISECT_EXPECTED_REV",
                "packed-refs",
                "shallow",
                "info/grafts",
                "info/attributes",
            ] {
                snapshot.add(&root.join(name), false, true)?;
            }
            for name in [
                "refs",
                "reftable",
                "rebase-apply",
                "rebase-merge",
                "sequencer",
            ] {
                snapshot.add(&root.join(name), true, true)?;
            }
            // The application reads HEAD's reflog. Branch/tag reflogs are not
            // inputs to the displayed history or references.
            snapshot.add(&root.join("logs/HEAD"), false, false)?;
            // Discovery already returns each linked administrative root.
            // Inspect the topology here, then scan each root exactly once.
            let worktrees = root.join("worktrees");
            snapshot.add_discovery(&worktrees)?;
            if snapshot.entries[&worktrees]
                .as_ref()
                .is_some_and(|stamp| stamp.directory)
            {
                for entry in fs::read_dir(&worktrees)? {
                    let entry = entry?;
                    if entry.file_name().as_encoded_bytes().ends_with(b".lock") {
                        continue;
                    }
                    let path = entry.path();
                    snapshot.add_discovery(&path)?;
                    if !inputs.info.git_dirs.contains(&path) {
                        return Err(SnapshotError::DiscoveryRequired);
                    }
                }
            }
        }
        Ok(snapshot)
    }

    fn add_discovery(&mut self, path: &Path) -> std::io::Result<()> {
        self.add(path, false, true)?;
        if let Some(resolved) = self.entries[path]
            .as_ref()
            .and_then(|stamp| stamp.resolved.clone())
        {
            self.add_discovery(&resolved)?;
        }
        self.discovery
            .insert(path.to_path_buf(), self.entries[path].clone());
        Ok(())
    }

    pub(super) fn covers(&self, inputs: &WatchInputs) -> bool {
        inputs.backend_discovered
            && !inputs.info.discovery_incomplete
            && self.git_dirs == inputs.info.git_dirs
            && self.inputs == inputs.inputs
    }

    pub(super) fn discovery_still_matches(&self, current: &Self) -> bool {
        self.discovery
            .iter()
            .all(|(path, stamp)| current.discovery.get(path) == Some(stamp))
    }

    fn add(&mut self, path: &Path, recursive: bool, read_contents: bool) -> std::io::Result<()> {
        let previous = self.coverage.get(path).copied();
        if let Some(previous) = previous
            && (!recursive || previous.recursive)
            && (!recursive || !read_contents || previous.recursive_contents)
            && (!read_contents || previous.contents)
        {
            return Ok(());
        }
        if previous.is_none() && self.entries.len() >= MAX_METADATA_ENTRIES {
            return Err(std::io::Error::other("activation metadata limit exceeded"));
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Windows can report PATH_NOT_FOUND for a child of a regular
                // file, too. That is an unreadable layout, not a stable absent
                // optional input. Validate the nearest existing ancestor.
                for ancestor in path.ancestors().skip(1) {
                    match fs::symlink_metadata(ancestor) {
                        Ok(metadata) => {
                            let directory = if metadata_link(ancestor, &metadata)?.is_some() {
                                fs::metadata(ancestor)?.is_dir()
                            } else {
                                metadata.is_dir()
                            };
                            if !directory {
                                return Err(std::io::Error::other(
                                    "activation input has a non-directory ancestor",
                                ));
                            }
                            break;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
                self.entries.insert(path.to_path_buf(), None);
                self.coverage.insert(
                    path.to_path_buf(),
                    Coverage {
                        recursive,
                        recursive_contents: recursive && read_contents,
                        contents: read_contents,
                    },
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let link = metadata_link(path, &metadata)?;
        // The discovered ancestor/control links need only nonrecursive stamps.
        // A link inside refs or sequencer data could lead into objects/private
        // caches or an unrelated tree; retain the full-refresh fallback there.
        if recursive && link.is_some() {
            return Err(std::io::Error::other(
                "unsupported recursive activation metadata link",
            ));
        }
        if link.is_none() && !metadata.is_file() && !metadata.is_dir() {
            return Err(std::io::Error::other(
                "unsupported activation metadata input",
            ));
        }
        let resolved = link
            .as_ref()
            .map(|_| fs::canonicalize(path).map(|target| normalized(&target)))
            .transpose()?;
        let target = if let Some(resolved) = &resolved {
            // Ancestor links such as /var are explicit discovery inputs. Their
            // targets need identity stamps, not an unbounded directory walk.
            // Canonicalization rejects dangling links and link-resolution loops.
            if resolved == path {
                return Err(std::io::Error::other("unresolved activation metadata link"));
            }
            self.add(resolved, false, read_contents)?;
            Some(self.entries[resolved].as_ref().ok_or_else(|| {
                std::io::Error::other("activation metadata link target disappeared")
            })?)
        } else {
            None
        };
        let directory = target.map_or_else(|| metadata.is_dir(), |stamp| stamp.directory);
        let contents = if let Some(target) = target {
            target.contents.clone()
        } else if previous.is_some_and(|coverage| coverage.contents) {
            self.entries
                .get(path)
                .and_then(|stamp| stamp.as_ref())
                .and_then(|stamp| stamp.contents.clone())
        } else if read_contents && !directory {
            // Watch inputs intentionally retain lexical and resolved aliases.
            // Keep their discovery entries, but read and budget the existing
            // file only once within this snapshot.
            let identity = normalized(&fs::canonicalize(path)?);
            if let Some(contents) = self.file_contents.get(&identity) {
                Some(contents.clone())
            } else {
                if metadata.len().saturating_add(self.bytes) > MAX_CONTROL_FILE_BYTES {
                    return Err(std::io::Error::other(
                        "activation control file limit exceeded",
                    ));
                }
                // HEAD, refs and configuration can be rewritten in place while
                // preserving timestamps. Their bytes are the actual input.
                use std::io::Read;
                let mut contents = Vec::new();
                let file = fs::File::open(&identity)?;
                // Take does not inherit File's read_to_end capacity hint. Keep
                // this hint optional: the actual bytes still enforce the limit.
                let _ = contents.try_reserve_exact(metadata.len() as usize);
                file.take(MAX_CONTROL_FILE_BYTES - self.bytes + 1)
                    .read_to_end(&mut contents)?;
                self.bytes += contents.len() as u64;
                if self.bytes > MAX_CONTROL_FILE_BYTES {
                    return Err(std::io::Error::other(
                        "activation control file limit exceeded",
                    ));
                }
                let contents: Arc<[u8]> = contents.into();
                self.file_contents.insert(identity, contents.clone());
                Some(contents)
            }
        } else {
            None
        };
        // Adding a link's target may have used the final available entry.
        if previous.is_none() && self.entries.len() >= MAX_METADATA_ENTRIES {
            return Err(std::io::Error::other("activation metadata limit exceeded"));
        }
        self.entries.insert(
            path.to_path_buf(),
            Some(FileStamp {
                len: if directory { 0 } else { metadata.len() },
                // Directory identity is enough: recursive inputs enumerate every
                // child; a Git root's mtime changes with unrelated lock files.
                modified: if directory {
                    std::time::SystemTime::UNIX_EPOCH
                } else {
                    metadata.modified()?
                },
                created: metadata.created().ok(),
                directory,
                contents,
                link,
                resolved,
                #[cfg(unix)]
                inode: std::os::unix::fs::MetadataExt::ino(&metadata),
                #[cfg(unix)]
                ctime: if directory {
                    (0, 0)
                } else {
                    (
                        std::os::unix::fs::MetadataExt::ctime(&metadata),
                        std::os::unix::fs::MetadataExt::ctime_nsec(&metadata),
                    )
                },
            }),
        );
        self.coverage.insert(
            path.to_path_buf(),
            Coverage {
                recursive: recursive || previous.is_some_and(|coverage| coverage.recursive),
                recursive_contents: (recursive && read_contents)
                    || previous.is_some_and(|coverage| coverage.recursive_contents),
                contents: read_contents || previous.is_some_and(|coverage| coverage.contents),
            },
        );
        if recursive && directory {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                if !entry.file_name().as_encoded_bytes().ends_with(b".lock") {
                    self.add(&entry.path(), true, read_contents)?;
                }
            }
        }
        Ok(())
    }
}

/// `read_link` accepts ordinary symlinks and Windows junctions. Other reparse
/// layouts must fail closed rather than silently treating their target as a
/// plain, unchanged input.
fn metadata_link(path: &Path, metadata: &fs::Metadata) -> std::io::Result<Option<PathBuf>> {
    let link = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let link = link || {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    };
    if link {
        fs::read_link(path).map(Some)
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
