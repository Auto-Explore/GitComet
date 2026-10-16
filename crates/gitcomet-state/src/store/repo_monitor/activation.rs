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
                repo.log_rev,
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
                && matches!(repo.log, Loadable::Ready(_))
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

impl CheckedActivation {
    /// Loads can finish while the monitor is inspecting the filesystem. Only
    /// the store can decide whether its current view still matches the input.
    pub(in crate::store) fn into_change(
        self,
        repo: &crate::model::RepoState,
    ) -> Option<RepoExternalChange> {
        if repo.lifetime() != self.context.lifetime {
            return None;
        }
        Some(if self.context == ActivationContext::for_repo(repo) {
            self.change
        } else {
            self.change.union(RepoExternalChange::all())
        })
    }
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
            for name in [
                "ORIG_HEAD",
                "FETCH_HEAD",
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
                        Ok(metadata) if metadata.is_dir() => break,
                        Ok(_) => {
                            return Err(std::io::Error::other(
                                "activation input has a non-directory ancestor",
                            ));
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
        // Fall back to the full refresh for unusual layouts instead of
        // mistaking an unchanged link for an unchanged target.
        if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(std::io::Error::other(
                "unsupported activation metadata input",
            ));
        }
        let directory = metadata.is_dir();
        let contents = if previous.is_some_and(|coverage| coverage.contents) {
            self.entries
                .get(path)
                .and_then(|stamp| stamp.as_ref())
                .and_then(|stamp| stamp.contents.clone())
        } else if read_contents && !directory {
            if metadata.len().saturating_add(self.bytes) > MAX_CONTROL_FILE_BYTES {
                return Err(std::io::Error::other(
                    "activation control file limit exceeded",
                ));
            }
            // HEAD, refs and configuration can be rewritten in place while
            // preserving timestamps. Their bytes are the actual input.
            use std::io::Read;
            let mut contents = Vec::new();
            fs::File::open(path)?
                .take(MAX_CONTROL_FILE_BYTES - self.bytes + 1)
                .read_to_end(&mut contents)?;
            self.bytes += contents.len() as u64;
            if self.bytes > MAX_CONTROL_FILE_BYTES {
                return Err(std::io::Error::other(
                    "activation control file limit exceeded",
                ));
            }
            Some(contents.into())
        } else {
            None
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::services::{GitRepository, RepositoryWatchInfo};

    struct Backend(Option<RepositoryWatchInfo>);
    impl GitBackend for Backend {
        fn open(&self, _: &Path) -> gitcomet_core::services::Result<Arc<dyn GitRepository>> {
            unreachable!("metadata comparison must not open a repository")
        }
        fn repository_watch_info(
            &self,
            _: &Path,
        ) -> gitcomet_core::services::Result<Option<RepositoryWatchInfo>> {
            Ok(self.0.clone())
        }
    }

    fn fixture() -> (tempfile::TempDir, Backend) {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git_dir.join("refs/heads/main"), "1111\n").unwrap();
        let backend = Backend(Some(RepositoryWatchInfo {
            git_dirs: vec![git_dir],
            ..Default::default()
        }));
        (dir, backend)
    }

    fn assert_working_changes_only(change: RepoExternalChange) {
        assert!(!change.git_state && !change.tags);
        assert!(change.worktree && change.index);
        assert!(change.large_file_support && change.text_attributes);
    }

    #[test]
    fn unchanged_focus_and_worktree_edits_do_not_reload_history() {
        let (dir, backend) = fixture();
        let mut refresh = ActivationRefresh::default();
        assert!(refresh.change(dir.path(), &backend).git_state);
        assert_working_changes_only(refresh.change(dir.path(), &backend));
        fs::write(
            dir.path().join("changed.txt"),
            "external edit without a watcher event",
        )
        .unwrap();
        assert_working_changes_only(refresh.change(dir.path(), &backend));
    }

    #[test]
    fn missed_git_metadata_changes_restore_full_refresh() {
        let (dir, backend) = fixture();
        let mut refresh = ActivationRefresh::default();
        refresh.change(dir.path(), &backend);
        for name in [
            "HEAD",
            "refs/heads/main",
            "refs/heads/nested/topic",
            "refs/tags/v1",
            "packed-refs",
            "reftable/tables.list",
            "shallow",
            "config",
            "MERGE_HEAD",
            "rebase-merge/head-name",
            "sequencer/todo",
            "logs/HEAD",
        ] {
            let path = dir.path().join(".git").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("new value for {name}\n")).unwrap();
            let change = refresh.change(dir.path(), &backend);
            assert!(change.git_state && change.tags, "missed change: {name}");
            assert_working_changes_only(refresh.change(dir.path(), &backend));
            fs::remove_file(&path).unwrap();
            assert!(
                refresh.change(dir.path(), &backend).git_state,
                "missed deletion: {name}"
            );
        }
    }

    #[test]
    fn common_directory_and_external_config_inputs_are_checked() {
        let (dir, mut backend) = fixture();
        let external = tempfile::tempdir().unwrap();
        let common = external.path().join("common");
        fs::create_dir_all(common.join("refs/heads")).unwrap();
        let config = external.path().join("included-config");
        let info = backend.0.as_mut().unwrap();
        info.git_dirs.push(common.clone());
        info.ignore_inputs.push(config.clone());
        let mut refresh = ActivationRefresh::default();
        refresh.change(dir.path(), &backend);
        assert_working_changes_only(refresh.change(dir.path(), &backend));
        fs::write(common.join("refs/heads/main"), "2222\n").unwrap();
        assert!(refresh.change(dir.path(), &backend).git_state);
        fs::write(&config, "[branch.main]\n remote = origin\n").unwrap();
        assert!(refresh.change(dir.path(), &backend).git_state);
        assert_working_changes_only(refresh.change(dir.path(), &backend));
    }

    #[test]
    fn index_only_changes_keep_history_and_tags_but_still_refresh_status() {
        let (dir, backend) = fixture();
        let mut refresh = ActivationRefresh::default();
        refresh.change(dir.path(), &backend);
        fs::write(dir.path().join(".git/index"), "new staged contents").unwrap();
        assert_working_changes_only(refresh.change(dir.path(), &backend));
        fs::remove_file(dir.path().join(".git/index")).unwrap();
        assert_working_changes_only(refresh.change(dir.path(), &backend));
    }

    #[test]
    fn duplicate_control_inputs_are_read_and_budgeted_once() {
        let (dir, mut backend) = fixture();
        let config = dir.path().join(".git/config");
        fs::write(&config, vec![b'#'; 3 * 1024 * 1024]).unwrap();
        backend
            .0
            .as_mut()
            .unwrap()
            .ignore_inputs
            .push(config.clone());
        let inputs = WatchInputs::load(dir.path(), &backend).unwrap();
        let snapshot = MetadataSnapshot::capture(dir.path(), &inputs).unwrap();
        // The same config is an explicit discovered input and a Git control.
        // Counting both would exceed the 4 MiB budget and disable narrowing.
        assert!(snapshot.bytes < 3 * 1024 * 1024 + 1024);
        assert_eq!(
            snapshot.entries[&config]
                .as_ref()
                .unwrap()
                .contents
                .as_ref()
                .unwrap()
                .len(),
            3 * 1024 * 1024
        );
    }

    #[test]
    fn unavailable_metadata_snapshot_does_not_request_watcher_rebuild() {
        let (dir, backend) = fixture();
        let inputs = WatchInputs::load(dir.path(), &backend).unwrap();
        let mut refresh = ActivationRefresh::default();
        refresh.change(dir.path(), &backend);
        fs::write(
            dir.path().join(".git/config"),
            vec![b'#'; MAX_CONTROL_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let probe = ActivationRefresh::prepare(dir.path(), &inputs);
        assert!(matches!(probe, Err(SnapshotError::Unavailable)));
        assert!(!refresh.needs_discovery(probe.as_ref()));
        assert!(
            refresh
                .complete(probe.ok(), ActivationContext::ready_for_test())
                .git_state
        );
        fs::remove_file(dir.path().join(".git/config")).unwrap();
        let recovered = ActivationRefresh::prepare(dir.path(), &inputs);
        assert!(recovered.is_ok());
        assert!(
            refresh.needs_discovery(recovered.as_ref()),
            "recovering a readable snapshot must rediscover exact control inputs"
        );
    }

    #[test]
    fn duplicate_visits_upgrade_recursive_and_content_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("child");
        fs::write(&file, "bytes").unwrap();
        let mut snapshot = MetadataSnapshot::default();
        snapshot.add(dir.path(), true, false).unwrap();
        snapshot.add(dir.path(), false, true).unwrap();
        snapshot.add(dir.path(), true, true).unwrap();
        assert_eq!(
            snapshot.entries[&file]
                .as_ref()
                .unwrap()
                .contents
                .as_deref(),
            Some(b"bytes".as_slice())
        );
        assert_eq!(snapshot.bytes, 5);
        snapshot.add(dir.path(), true, true).unwrap();
        snapshot.add(&file, false, true).unwrap();
        assert_eq!(snapshot.bytes, 5);
    }

    #[test]
    fn linked_admin_roots_are_scanned_once_and_new_roots_require_discovery() {
        let (dir, mut backend) = fixture();
        let linked = dir.path().join(".git/worktrees/linked");
        fs::create_dir_all(linked.join("logs/refs/heads")).unwrap();
        fs::write(linked.join("HEAD"), "ref: refs/heads/linked\n").unwrap();
        fs::write(linked.join("index"), "staged files").unwrap();
        fs::write(
            linked.join("logs/refs/heads/linked"),
            "unused branch reflog",
        )
        .unwrap();
        backend.0.as_mut().unwrap().git_dirs.push(linked.clone());
        let inputs = WatchInputs::load(dir.path(), &backend).unwrap();
        let snapshot = MetadataSnapshot::capture(dir.path(), &inputs).unwrap();
        assert!(!snapshot.entries.contains_key(&linked.join("index")));
        assert!(
            !snapshot
                .entries
                .contains_key(&linked.join("logs/refs/heads/linked"))
        );
        assert_eq!(snapshot.bytes, 49); // root HEAD/ref plus linked HEAD, once each
        let mut refresh = ActivationRefresh::default();
        refresh.complete(Some(snapshot), ActivationContext::ready_for_test());
        assert_working_changes_only(refresh.change(dir.path(), &backend));
        fs::write(linked.join("HEAD"), "ref: refs/heads/other\n").unwrap();
        assert!(refresh.change(dir.path(), &backend).git_state);
        fs::write(linked.join("gitdir"), "moved-checkout/.git\n").unwrap();
        let moved = ActivationRefresh::prepare(dir.path(), &inputs);
        assert!(refresh.needs_discovery(moved.as_ref()));
        assert!(refresh.change(dir.path(), &backend).git_state);
        fs::create_dir_all(dir.path().join(".git/worktrees/new")).unwrap();
        let current = ActivationRefresh::prepare(dir.path(), &inputs);
        assert!(refresh.needs_discovery(current.as_ref()));
    }

    #[test]
    fn only_the_displayed_head_reflog_invalidates_history() {
        let (dir, backend) = fixture();
        let mut refresh = ActivationRefresh::default();
        refresh.change(dir.path(), &backend);
        let logs = dir.path().join(".git/logs");
        fs::create_dir_all(logs.join("refs/heads")).unwrap();
        fs::write(logs.join("refs/heads/main"), "unused reflog").unwrap();
        assert_working_changes_only(refresh.change(dir.path(), &backend));
        fs::write(logs.join("HEAD"), "displayed reflog").unwrap();
        assert!(refresh.change(dir.path(), &backend).git_state);
    }

    #[test]
    fn incomplete_or_unsupported_discovery_never_skips_full_refresh() {
        let (dir, mut backend) = fixture();
        backend.0.as_mut().unwrap().discovery_incomplete = true;
        for backend in [backend, Backend(None)] {
            let mut refresh = ActivationRefresh::default();
            for _ in 0..2 {
                assert!(refresh.change(dir.path(), &backend).git_state);
            }
        }
    }

    #[test]
    fn metadata_errors_discard_the_baseline_and_recover_conservatively() {
        let (dir, backend) = fixture();
        let mut refresh = ActivationRefresh::default();
        refresh.change(dir.path(), &backend);
        // A regular file where info is expected makes deeper inputs
        // unreadable and must not be cached as an unchanged error.
        fs::write(dir.path().join(".git/info"), "invalid info directory").unwrap();
        assert!(refresh.change(dir.path(), &backend).git_state);
        assert!(refresh.change(dir.path(), &backend).git_state);
        fs::remove_file(dir.path().join(".git/info")).unwrap();
        fs::create_dir(dir.path().join(".git/info")).unwrap();
        assert!(refresh.change(dir.path(), &backend).git_state);
        assert_working_changes_only(refresh.change(dir.path(), &backend));
    }

    #[test]
    fn failed_or_cancelled_refresh_is_retried_with_identical_metadata() {
        let (dir, backend) = fixture();
        let mut refresh = ActivationRefresh::default();
        let mut context = ActivationContext::ready_for_test();
        assert!(
            refresh
                .check(dir.path(), &backend, context.clone())
                .git_state
        );
        assert_working_changes_only(refresh.check(dir.path(), &backend, context.clone()));
        // Even a failed load that retains Ready data increments diagnostics.
        context.diagnostics_seq += 1;
        assert!(
            refresh
                .check(dir.path(), &backend, context.clone())
                .git_state
        );
        assert_working_changes_only(refresh.check(dir.path(), &backend, context.clone()));
        context.epoch = context.epoch.next();
        assert!(
            refresh
                .check(dir.path(), &backend, context.clone())
                .git_state
        );
        context.ready = false;
        for _ in 0..2 {
            assert!(
                refresh
                    .check(dir.path(), &backend, context.clone())
                    .git_state
            );
        }
    }

    #[test]
    fn displayed_state_changes_invalidate_an_old_matching_disk_snapshot() {
        let (dir, backend) = fixture();
        let mut refresh = ActivationRefresh::default();
        let mut context = ActivationContext::ready_for_test();
        refresh.check(dir.path(), &backend, context.clone());
        assert_working_changes_only(refresh.check(dir.path(), &backend, context.clone()));
        // The watcher loaded B after activation at A; an unseen reset put disk
        // back at A. An unchanged disk snapshot must not keep displaying B.
        context.revisions[1] += 1;
        assert!(
            refresh
                .check(dir.path(), &backend, context.clone())
                .git_state
        );
        assert_working_changes_only(refresh.check(dir.path(), &backend, context));
    }

    #[test]
    fn ref_content_changes_are_detected_with_preserved_size_and_mtime() {
        let (dir, backend) = fixture();
        let mut refresh = ActivationRefresh::default();
        refresh.change(dir.path(), &backend);
        let path = dir.path().join(".git/refs/heads/main");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, "2222\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(refresh.change(dir.path(), &backend).git_state);
    }

    #[test]
    fn delayed_activation_reply_revalidates_current_state_and_repository_lifetime() {
        let mut repo = crate::model::RepoState::new_opening(
            RepoId(1),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("repo"),
            },
        );
        let context = ActivationContext::for_repo(&repo);
        let checked = || CheckedActivation {
            repo_id: repo.id,
            context: context.clone(),
            change: RepoExternalChange {
                git_state: false,
                tags: false,
                ..RepoExternalChange::all()
            },
        };
        assert!(!checked().into_change(&repo).unwrap().git_state);
        // The UI accepted a different history result while metadata was read.
        let pending = checked();
        repo.log_rev += 1;
        assert!(pending.into_change(&repo).unwrap().git_state);
        let reopened = crate::model::RepoState::new_opening(repo.id, repo.spec.clone());
        let pending = CheckedActivation {
            repo_id: repo.id,
            context,
            change: RepoExternalChange::all(),
        };
        assert!(pending.into_change(&reopened).is_none());
    }

    #[test]
    fn equal_successful_refreshes_preserve_the_production_activation_context() {
        use crate::model::Loadable;
        let mut repo = crate::model::RepoState::new_opening(
            RepoId(1),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("repo"),
            },
        );
        let page = Arc::new(gitcomet_core::domain::LogPage {
            commits: Vec::new(),
            next_cursor: None,
        });
        let apply = |repo: &mut crate::model::RepoState| {
            repo.set_head_branch(Loadable::Ready("main".into()));
            repo.set_log(Loadable::Ready(page.clone()));
            repo.set_branches(Loadable::Ready(Vec::new()));
            repo.set_remote_branches(Loadable::Ready(Vec::new()));
            repo.set_tags(Loadable::Ready(Vec::new()));
            repo.set_remotes(Loadable::Ready(Vec::new()));
            repo.set_stashes(Loadable::Ready(Vec::new()));
            repo.set_reflog(Loadable::Ready(Vec::new()));
            repo.set_worktrees(Loadable::Ready(Vec::new()));
            repo.set_upstream_divergence(Loadable::Ready(None));
            repo.set_rebase_in_progress(Loadable::Ready(false));
            repo.set_sequencer_state(Loadable::Ready(Default::default()));
            repo.set_merge_commit_message(Loadable::Ready(None));
        };
        apply(&mut repo);
        let before = ActivationContext::for_repo(&repo);
        assert!(before.ready);
        apply(&mut repo);
        assert_eq!(before, ActivationContext::for_repo(&repo));
    }
}
