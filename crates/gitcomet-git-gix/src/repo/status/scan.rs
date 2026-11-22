//! The index-to-worktree half of a status: gix's check of the tracked files
//! beside its scan for untracked ones, the scan shared out over threads.
//!
//! gix scans on one thread, and asks before it descends into a directory. A
//! large tracked directory is declined there and scanned from a root of its
//! own by another thread. Each untracked file is reported by itself, never
//! folded into its directory, so two directories scanned apart report what
//! one scan of both does.

use gix::bstr::{BStr, BString, ByteSlice as _, ByteVec as _};
use gix::dir::{EntryRef, entry, walk};
use gix::status::plumbing::{index_as_worktree, index_as_worktree_with_renames as combined};
use gix::worktree::stack::State;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

/// Tracked files below a directory for it to be scanned from a root of its
/// own, which costs a classification of every directory on the way to it.
const SHARED_DIRECTORY_FILES: usize = 1024;
const SCANNERS_MAX: usize = 8;

type Found = Vec<(gix::dir::Entry, Option<entry::Status>)>;

/// What gix's `index_as_worktree_with_renames` reports without rename
/// tracking or sorting, in no particular order.
pub(super) fn index_as_worktree_with_untracked<'index, U, Find>(
    index: &'index gix::index::State,
    worktree: &Path,
    collector: &mut impl combined::VisitEntry<'index, ContentChange = (), SubmoduleStatus = U>,
    submodule: impl index_as_worktree::traits::SubmoduleStatus<Output = U> + Send + Clone,
    objects: Find,
    progress: &mut dyn gix::Progress,
    mut ctx: combined::Context<'_>,
    tracked: index_as_worktree::Options,
    untracked: walk::Options<'_>,
) -> gix::Result<combined::Outcome>
where
    U: Send + Clone,
    Find: gix::objs::Find + gix::objs::FindHeader + Send + Clone,
{
    // Only the scan needs ignores; content filters and pathspecs need
    // attributes. This is how gix splits the one stack it is given.
    let has_ignores = matches!(
        ctx.resource_cache.attr_stack.state_mut(),
        State::AttributesAndIgnoreStack { .. } | State::IgnoreStack(_)
    );
    let mut excludes = has_ignores.then(|| ctx.resource_cache.attr_stack.clone());
    if let Some(stack) = excludes.as_mut() {
        let state = stack.state_mut();
        if let State::AttributesAndIgnoreStack { ignore, .. } = state {
            *state = State::IgnoreStack(std::mem::take(ignore));
        }
    }
    let state = ctx.resource_cache.attr_stack.state_mut();
    if let State::AttributesAndIgnoreStack { attributes, .. } = state {
        *state = State::AttributesStack(std::mem::take(attributes));
    }
    let attributes = ctx.resource_cache.attr_stack;
    let pathspec_attributes = ctx
        .pathspec
        .patterns()
        .any(|pattern| !pattern.attributes.is_empty())
        .then(|| attributes.clone());
    let entries = &index.entries()[index
        .prefixed_entries_range(ctx.pathspec.common_prefix())
        .unwrap_or(0..index.entries().len())];

    let (scanners, shared_directory_files) = sharing(index, untracked);
    let roots = Roots::new();
    let scanner = Scanner {
        roots: &roots,
        shared_directory_files,
        index,
        worktree,
        dirwalk: ctx.dirwalk,
        should_interrupt: ctx.should_interrupt,
        options: untracked,
    };
    std::thread::scope(|scope| {
        let scans: Vec<_> = (0..scanners)
            .map(|_| {
                let (mut excludes, mut pathspec, objects) =
                    (excludes.clone(), ctx.pathspec.clone(), objects.clone());
                let mut pathspec_attributes = pathspec_attributes.clone();
                let scanner = &scanner;
                scope.spawn(move || {
                    scanner.scan(
                        excludes.as_mut(),
                        &mut pathspec,
                        pathspec_attributes.as_mut(),
                        &objects,
                    )
                })
            })
            .collect();

        let tracked = index_as_worktree(
            index,
            worktree,
            &mut Tracked { collector, entries },
            index_as_worktree::traits::FastEq,
            submodule,
            objects.clone(),
            progress,
            index_as_worktree::Context {
                pathspec: ctx.pathspec,
                stack: attributes,
                filter: ctx.resource_cache.filter.worktree_filter,
                should_interrupt: ctx.should_interrupt,
            },
            tracked,
        );
        if tracked.is_err() {
            roots.stop();
        }

        let mut walked = walk::Outcome::default();
        let mut failure = None;
        for scan in scans {
            match scan
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            {
                Ok((outcome, found)) => {
                    walked.read_dir_calls += outcome.read_dir_calls;
                    walked.returned_entries += outcome.returned_entries;
                    walked.seen_entries += outcome.seen_entries;
                    for (entry, collapsed_directory_status) in found {
                        collector.visit_entry(combined::Entry::DirectoryContents {
                            entry,
                            collapsed_directory_status,
                        });
                    }
                }
                Err(error) => failure = failure.or(Some(error)),
            }
        }
        let tracked_file_modification = tracked?;
        match failure {
            Some(error) => Err(error),
            None => Ok(combined::Outcome {
                tracked_file_modification,
                dirwalk: Some(walked),
                rewrites: None,
            }),
        }
    })
}

/// The threads to scan with, and the tracked files below a directory for it
/// to be shared. One thread scans as gix does. More share the work only
/// where nothing is folded into a directory, which is what makes directories
/// independent, and where there is enough below the root to share.
fn sharing(index: &gix::index::State, options: walk::Options<'_>) -> (usize, usize) {
    const ALONE: (usize, usize) = (1, usize::MAX);
    let folds = options.emit_untracked == walk::EmissionMode::CollapseDirectory
        || options.emit_ignored == Some(walk::EmissionMode::CollapseDirectory);
    if folds || options.emit_tracked {
        return ALONE;
    }
    #[cfg(test)]
    if let Some(sharing) = SHARING.get() {
        return sharing;
    }
    if index.entries().len() < 2 * SHARED_DIRECTORY_FILES {
        return ALONE;
    }
    let scanners = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(SCANNERS_MAX);
    (scanners, SHARED_DIRECTORY_FILES)
}

#[cfg(test)]
thread_local! {
    /// Scanning threads and the files a shared directory needs, for a
    /// fixture too small to share anything by itself.
    pub(super) static SHARING: std::cell::Cell<Option<(usize, usize)>> =
        const { std::cell::Cell::new(None) };
}

/// Passes gix's tracked-file changes on as it would itself.
struct Tracked<'a, 'index, C> {
    collector: &'a mut C,
    entries: &'index [gix::index::Entry],
}

impl<'index, U, C> index_as_worktree::VisitEntry<'index> for Tracked<'_, 'index, C>
where
    C: combined::VisitEntry<'index, ContentChange = (), SubmoduleStatus = U>,
{
    type ContentChange = ();
    type SubmoduleStatus = U;

    fn visit_entry(
        &mut self,
        _entries: &'index [gix::index::Entry],
        entry: &'index gix::index::Entry,
        entry_index: usize,
        rela_path: &'index BStr,
        status: index_as_worktree::EntryStatus<(), U>,
    ) {
        self.collector.visit_entry(combined::Entry::Modification {
            entries: self.entries,
            entry,
            entry_index,
            rela_path,
            status,
        });
    }
}

/// The directories waiting to be scanned, as paths in the worktree.
struct Roots {
    state: Mutex<RootsState>,
    changed: Condvar,
}

struct RootsState {
    waiting: Vec<BString>,
    scanning: usize,
    stopped: bool,
}

impl Roots {
    /// Starts with the worktree itself.
    fn new() -> Self {
        Self {
            state: Mutex::new(RootsState {
                waiting: vec![BString::default()],
                scanning: 0,
                stopped: false,
            }),
            changed: Condvar::new(),
        }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, RootsState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The next root, waiting while a scan may still add one. `None` once
    /// every root is scanned or a scan failed.
    fn next(&self) -> Option<Scanning<'_>> {
        let mut state = self.locked();
        loop {
            if state.stopped {
                return None;
            }
            if let Some(root) = state.waiting.pop() {
                state.scanning += 1;
                return Some(Scanning {
                    roots: self,
                    root,
                    finished: false,
                });
            }
            if state.scanning == 0 {
                return None;
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn add(&self, root: BString) {
        self.locked().waiting.push(root);
        self.changed.notify_one();
    }

    fn stop(&self) {
        self.locked().stopped = true;
        self.changed.notify_all();
    }
}

/// A root being scanned. Dropped unfinished, by an error or a panic, it
/// stops the other scans instead of leaving them waiting for more roots.
struct Scanning<'a> {
    roots: &'a Roots,
    root: BString,
    finished: bool,
}

impl Drop for Scanning<'_> {
    fn drop(&mut self) {
        let mut state = self.roots.locked();
        state.scanning -= 1;
        state.stopped |= !self.finished;
        if state.stopped || state.scanning == 0 {
            self.roots.changed.notify_all();
        }
    }
}

/// What every scanning thread shares.
struct Scanner<'a> {
    roots: &'a Roots,
    /// The tracked files below a directory for another thread to scan it.
    shared_directory_files: usize,
    index: &'a gix::index::State,
    worktree: &'a Path,
    dirwalk: combined::DirwalkContext<'a>,
    should_interrupt: &'a AtomicBool,
    options: walk::Options<'a>,
}

impl Scanner<'_> {
    /// Scans roots until none is left, with this thread's own matchers.
    fn scan(
        &self,
        mut excludes: Option<&mut gix::worktree::Stack>,
        pathspec: &mut gix::pathspec::Search,
        mut pathspec_attributes: Option<&mut gix::worktree::Stack>,
        objects: &(impl gix::objs::Find + gix::objs::FindHeader),
    ) -> gix::Result<(walk::Outcome, Found)> {
        let mut walked = walk::Outcome::default();
        let mut delegate = Delegate {
            scanner: self,
            root: BString::default(),
            below: BString::default(),
            found: Vec::new(),
        };
        while let Some(mut scanning) = self.roots.next() {
            delegate.root.clone_from(&scanning.root);
            let from = self.directory(delegate.root.as_bstr());
            let (outcome, _) = gix::dir::walk(
                self.worktree,
                walk::Context {
                    should_interrupt: Some(self.should_interrupt),
                    git_dir_realpath: self.dirwalk.git_dir_realpath,
                    current_dir: self.dirwalk.current_dir,
                    index: self.index,
                    ignore_case_index_lookup: self.dirwalk.ignore_case_index_lookup,
                    pathspec: &mut *pathspec,
                    pathspec_attributes: &mut |relative_path, case, is_dir, out| {
                        let stack = pathspec_attributes
                            .as_deref_mut()
                            .expect("can only be called if attributes are used in patterns");
                        let mode = if is_dir {
                            gix::index::entry::Mode::DIR
                        } else {
                            gix::index::entry::Mode::FILE
                        };
                        stack
                            .set_case(case)
                            .at_entry(relative_path, Some(mode), objects)
                            .is_ok_and(|platform| platform.matching_attributes(out))
                    },
                    excludes: excludes.as_deref_mut(),
                    objects,
                    explicit_traversal_root: Some(from.as_deref().unwrap_or(self.worktree)),
                },
                self.options,
                &mut delegate,
            )?;
            walked.read_dir_calls += outcome.read_dir_calls;
            walked.returned_entries += outcome.returned_entries;
            walked.seen_entries += outcome.seen_entries;
            scanning.finished = true;
        }
        Ok((walked, delegate.found))
    }

    /// Where `root` is on disk; `None` for the worktree itself and for a
    /// path this platform cannot spell.
    fn directory(&self, root: &BStr) -> Option<PathBuf> {
        if root.is_empty() {
            return None;
        }
        let relative = gix::path::from_bstr(root).ok()?;
        let mut directory = self.worktree.to_owned();
        directory.extend(relative.components());
        Some(directory)
    }
}

struct Delegate<'a> {
    scanner: &'a Scanner<'a>,
    /// The root this scan started from.
    root: BString,
    /// A buffer for the index prefix of a directory.
    below: BString,
    found: Found,
}

impl Delegate<'_> {
    /// Hands a large tracked directory to whichever thread is free. An
    /// untracked or ignored one is gix's to report, so it stays in this scan.
    fn shares(&mut self, directory: &EntryRef<'_>) -> bool {
        let scanner = self.scanner;
        let path: &BStr = directory.rela_path.as_ref();
        if directory.status != entry::Status::Tracked || path == self.root.as_bstr() {
            return false;
        }
        self.below.clear();
        self.below.push_str(path);
        self.below.push(b'/');
        let files = scanner
            .index
            .prefixed_entries_range(self.below.as_bstr())
            .map_or(0, |range| range.len());
        if files < scanner.shared_directory_files || scanner.directory(path).is_none() {
            return false;
        }
        scanner.roots.add(path.to_owned());
        true
    }
}

impl walk::Delegate for Delegate<'_> {
    fn emit(
        &mut self,
        entry: EntryRef<'_>,
        collapsed_directory_status: Option<entry::Status>,
    ) -> walk::Action {
        // Status never shows untracked entries of a kind Git cannot track.
        if entry.disk_kind != Some(entry::Kind::Untrackable) {
            self.found
                .push((entry.to_owned(), collapsed_directory_status));
        }
        if self.scanner.should_interrupt.load(Ordering::Relaxed) {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    }

    fn can_recurse(
        &mut self,
        entry: EntryRef<'_>,
        for_deletion: Option<walk::ForDeletionMode>,
        worktree_root_is_repository: bool,
    ) -> bool {
        entry.status.can_recurse(
            entry.disk_kind,
            entry.pathspec_match,
            for_deletion,
            worktree_root_is_repository,
        ) && !self.shares(&entry)
    }
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
