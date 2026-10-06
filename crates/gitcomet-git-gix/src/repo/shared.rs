//! Process-local ownership. Registries never keep a repository or index alive.
use super::{GixRepo, log};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::history_index::{HistoryIndex, HistoryIndexHandle, HistoryIndexProgress};
use gitcomet_core::services::{CancellationToken, HistorySnapshot, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak, mpsc};
use std::time::Duration;

pub(super) fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The open handle prevents filesystem identity reuse while an owner is alive.
/// Failed identity reads deliberately disable sharing, rather than falling back
/// to a path that may now name a different repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Identity {
    path: PathBuf,
    file: Option<Arc<same_file::Handle>>,
    nonce: u64,
}

impl Identity {
    pub(crate) fn read(path: &Path) -> Self {
        Self::read_with_fallback(path, next_id())
    }

    /// Failed reads are private to an owner, but stable for that owner's
    /// lifetime. A later successful read still changes the identity.
    fn read_with_fallback(path: &Path, private_scope: u64) -> Self {
        let file = same_file::Handle::from_path(path).ok().map(Arc::new);
        Self {
            path: path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
            nonce: if file.is_some() { 0 } else { private_scope },
            file,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeIdentity {
    root: Identity,
    git: Identity,
    common: Identity,
}

impl WorktreeIdentity {
    pub(crate) fn new(workdir: &Path, repo: &gix::Repository) -> Self {
        Self {
            root: Identity::read(workdir),
            git: Identity::read(repo.git_dir()),
            common: Identity::read(repo.common_dir()),
        }
    }
}

/// Unreadable inputs compare equal only within the same repository owner.
type StoreInput<T> = std::result::Result<T, (u64, String)>;

#[derive(Debug, Eq, PartialEq)]
struct StoreKey {
    objects: Identity,
    alternates: StoreInput<Vec<Identity>>,
    hash: gix::hash::Kind,
    replacements: Vec<(gix::ObjectId, gix::ObjectId)>,
    replacement_refs: StoreInput<Vec<(Vec<u8>, String)>>,
    multi_pack: bool,
    // Capture effective values from the freshly resolved, worktree-local config.
    // Compression/caches are handle options, but requiring equality here also
    // avoids changing the defaults of newly opened loose stores.
    options: Vec<Option<Vec<u8>>>,
    trust: String,
}

impl StoreKey {
    fn read(repo: &gix::Repository, private_scope: u64) -> Self {
        let store = repo.objects.store_ref();
        let alternates = gix::odb::alternate::resolve(
            store.path().to_path_buf(),
            &std::env::current_dir().unwrap_or_default(),
        );
        let alternates = alternates
            .map(|paths| {
                paths
                    .iter()
                    .map(|path| Identity::read_with_fallback(path, private_scope))
                    .collect()
            })
            .map_err(|error| (private_scope, format!("{error:?}")));
        let config = repo.config_snapshot();
        // Include the namespace even when this gix configuration/ref backend
        // suppresses installed mappings. An external replacement edit must
        // invalidate old history registrations in either case.
        let prefix = config
            .string("gitoxide.objects.replaceRefBase")
            .map(|value| value.to_vec())
            .unwrap_or_else(|| b"refs/replace/".to_vec());
        let replacement_refs = (|| -> Result<Vec<_>> {
            let refs = crate::refs::view(repo)?;
            refs.prefixed(&prefix)?
                .map(|reference| {
                    let reference = reference?;
                    Ok((
                        reference.name().as_bstr().to_vec(),
                        format!("{:?}", reference.target()),
                    ))
                })
                .collect()
        })()
        .map_err(|error| (private_scope, error.to_string()));
        Self {
            objects: Identity::read_with_fallback(store.path(), private_scope),
            alternates,
            hash: repo.object_hash(),
            replacements: store.replacements().collect(),
            replacement_refs,
            multi_pack: store.use_multi_pack_index(),
            options: [
                "gitoxide.objects.allocLimit",
                "gitoxide.objects.allocLimitIfReducedTrust",
                "core.looseCompression",
                "core.compression",
                "gitoxide.objects.cacheLimit",
                "core.deltaBaseCacheLimit",
                "gitoxide.core.deltaBaseCacheLimit",
                "core.useReplaceRefs",
                "gitoxide.objects.replaceRefBase",
            ]
            .map(|key| config.string(key).map(|value| value.to_vec()))
            .into(),
            trust: format!("{:?}", repo.git_dir_trust()),
        }
    }
}

pub(super) struct CommonRepository {
    pub id: u64,
    identity: Identity,
    stores: Mutex<Vec<Weak<SharedStore>>>,
    pub maintenance: Mutex<()>,
}

static COMMON: Mutex<Vec<Weak<CommonRepository>>> = Mutex::new(Vec::new());

impl CommonRepository {
    pub fn get(repo: &gix::Repository, private_scope: u64) -> Arc<Self> {
        let identity = Identity::read_with_fallback(repo.common_dir(), private_scope);
        let mut entries = COMMON.lock().expect("common repositories");
        entries.retain(|entry| entry.strong_count() != 0);
        if let Some(common) = entries
            .iter()
            .filter_map(Weak::upgrade)
            .find(|common| common.identity == identity)
        {
            return common;
        }
        let common = Arc::new(Self {
            id: next_id(),
            identity,
            stores: Mutex::new(Vec::new()),
            maintenance: Mutex::new(()),
        });
        entries.push(Arc::downgrade(&common));
        common
    }

    pub fn attach(
        self: &Arc<Self>,
        repo: &mut gix::ThreadSafeRepository,
        private_scope: u64,
    ) -> Arc<SharedStore> {
        let key = StoreKey::read(&repo.to_thread_local(), private_scope);
        let mut stores = self.stores.lock().expect("shared object stores");
        stores.retain(|store| store.strong_count() != 0);
        let previous = stores
            .iter()
            .rev()
            .filter_map(Weak::upgrade)
            .find(|store| store.key == key);
        if let Some(shared) = &previous
            && shared.reusable.load(Ordering::Acquire)
        {
            repo.objects = shared.objects.clone();
            return shared.clone();
        }
        let shared = Arc::new(SharedStore {
            // Replacing mappings after an I/O failure preserves interpretation
            // and completed indexes only when the freshly read key still agrees.
            id: previous.as_ref().map_or_else(next_id, |store| store.id),
            mapping_id: next_id(),
            common: self.clone(),
            key,
            objects: repo.objects.clone(),
            reusable: AtomicBool::new(true),
            topology: previous
                .as_ref()
                .map(|store| store.topology.clone())
                .unwrap_or_default(),
            indexes: previous
                .as_ref()
                .map(|store| store.indexes.clone())
                .unwrap_or_default(),
            decode: previous
                .as_ref()
                .map(|store| store.decode.clone())
                .unwrap_or_default(),
        });
        stores.push(Arc::downgrade(&shared));
        shared
    }

    /// Drop persistent mappings on the next attachment without invalidating
    /// immutable history. Active readers may finish on their old mappings.
    pub fn rotate_mappings(&self) {
        let stores = self.stores.lock().expect("shared object stores");
        for store in stores.iter().filter_map(Weak::upgrade) {
            store.reusable.store(false, Ordering::Release);
        }
    }

    pub fn invalidate(&self) {
        let mut stores = self.stores.lock().expect("shared object stores");
        for store in stores.iter().filter_map(Weak::upgrade) {
            let mut indexes = store.indexes.lock().expect("shared indexes");
            for entry in indexes.iter() {
                if let Some(build) = entry.build.upgrade() {
                    build.cancellation.cancel();
                }
            }
            indexes.clear();
            drop(indexes);
            // Cancel builders before waiting for their topology lock.
            store.topology.clear();
        }
        stores.clear();
        super::shared_ranges::invalidate(self.id);
    }
}

pub(super) struct SharedStore {
    /// Object interpretation, stable across I/O recovery.
    pub id: u64,
    /// Physical mappings, changed on every rotation.
    pub mapping_id: u64,
    pub common: Arc<CommonRepository>,
    key: StoreKey,
    objects: Arc<gix::odb::Store>,
    reusable: AtomicBool,
    pub topology: Arc<log::TopologyCache>,
    indexes: Arc<Mutex<Vec<IndexEntry>>>,
    pub decode: Arc<Mutex<()>>,
}

struct IndexEntry {
    key: log::HistoryQuery,
    snapshot: Option<Weak<str>>,
    index: Weak<HistoryIndex>,
    build: Weak<Build>,
}

impl IndexEntry {
    fn get<'a>(entries: &'a mut Vec<Self>, query: &log::HistoryQuery) -> &'a mut Self {
        entries.retain(|entry| {
            entry.index.strong_count() != 0
                || entry.build.strong_count() != 0
                || entry
                    .snapshot
                    .as_ref()
                    .is_some_and(|value| value.strong_count() != 0)
        });
        let at = entries
            .iter()
            .position(|entry| &entry.key == query)
            .unwrap_or_else(|| {
                entries.push(Self {
                    key: query.clone(),
                    snapshot: None,
                    index: Weak::new(),
                    build: Weak::new(),
                });
                entries.len() - 1
            });
        &mut entries[at]
    }

    fn snapshot(&mut self) -> HistorySnapshot {
        if let Some(snapshot) = self.snapshot.as_ref().and_then(Weak::upgrade) {
            return HistorySnapshot(snapshot);
        }
        let snapshot = self.key.snapshot();
        self.snapshot = Some(Arc::downgrade(&snapshot.0));
        snapshot
    }
}

#[derive(Default)]
struct BuildState {
    progress: Option<HistoryIndexProgress>,
    result: Option<Result<HistoryIndexHandle>>,
    requesters: usize,
}

struct Build {
    state: Mutex<BuildState>,
    changed: Condvar,
    cancellation: CancellationToken,
}

/// One lease per requester; closing a window never cancels another's work.
struct Request(Arc<Build>);

impl Drop for Request {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("history build");
        state.requesters -= 1;
        if state.requesters == 0 {
            self.0.cancellation.cancel();
        }
    }
}

type Job = Box<dyn FnOnce() + Send>;
fn queue(job: Job) -> Result<()> {
    static QUEUE: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let job = Box::new(gitcomet_core::op_trace::wrap_task("shared-history", job));
    QUEUE
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("shared-history".into())
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        job();
                    }
                })
                .expect("history worker");
            tx
        })
        .send(job)
        .map_err(|_| Error::new(ErrorKind::Backend("history worker stopped".into())))
}

impl SharedStore {
    pub fn snapshot(&self, query: &log::HistoryQuery) -> HistorySnapshot {
        let mut entries = self.indexes.lock().expect("shared indexes");
        IndexEntry::get(&mut entries, query).snapshot()
    }

    pub fn owns_index(&self, index: &HistoryIndexHandle) -> bool {
        self.indexes
            .lock()
            .expect("shared indexes")
            .iter()
            .any(|entry| entry.index.ptr_eq(&Arc::downgrade(index)))
    }

    pub fn index(
        self: &Arc<Self>,
        repo: gix::ThreadSafeRepository,
        query: log::HistoryQuery,
        cancellation: &CancellationToken,
        on_progress: &mut dyn FnMut(HistoryIndexProgress),
    ) -> Result<HistoryIndexHandle> {
        cancellation.check_cancelled()?;
        // Registration precedes queueing. Weak completed entries never pin a
        // history after its last window has gone away.
        let mut entries = self.indexes.lock().expect("shared indexes");
        let entry = IndexEntry::get(&mut entries, &query);
        if let Some(index) = entry.index.upgrade() {
            return Ok(index);
        }
        let snapshot = entry.snapshot();
        let existing = entry
            .build
            .upgrade()
            .filter(|build| !build.cancellation.is_cancelled());
        let (build, start) = match existing {
            Some(build) => (build, false),
            None => (
                Arc::new(Build {
                    state: Mutex::new(BuildState::default()),
                    changed: Condvar::new(),
                    cancellation: CancellationToken::new(),
                }),
                true,
            ),
        };
        // Also synchronize with the last lease's drop. A cancelled flight must
        // never acquire a new requester between the check and this increment.
        let mut state = build.state.lock().expect("history build");
        if build.cancellation.is_cancelled() {
            drop(state);
            drop(entries);
            return self.index(repo, query, cancellation, on_progress);
        }
        state.requesters += 1;
        drop(state);
        let request = Request(build.clone());
        entry.build = Arc::downgrade(&build);
        drop(entries);
        if start {
            let shared = self.clone();
            let worker = build.clone();
            queue(Box::new(gitcomet_core::history_perf::with_capture_context(
                move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        GixRepo::build_resolved_history_index(
                            &repo,
                            &query,
                            snapshot,
                            &shared.topology,
                            &worker.cancellation,
                            &mut |progress| {
                                worker.state.lock().expect("history build").progress =
                                    Some(progress);
                                worker.changed.notify_all();
                            },
                        )
                    }))
                    .unwrap_or_else(|_| {
                        Err(Error::new(ErrorKind::Backend(
                            "history build panicked".into(),
                        )))
                    });
                    if let Ok(index) = &result {
                        let mut entries = shared.indexes.lock().expect("shared indexes");
                        if let Some(entry) = entries.iter_mut().find(|entry| {
                            entry.key == query && entry.build.ptr_eq(&Arc::downgrade(&worker))
                        }) {
                            entry.index = Arc::downgrade(index);
                        }
                    }
                    worker.state.lock().expect("history build").result = Some(result);
                    worker.changed.notify_all();
                },
            )))?;
        }
        let mut seen = None;
        loop {
            cancellation.check_cancelled()?;
            let state = request.0.state.lock().expect("history build");
            let progress = state.progress.clone();
            let result = state.result.clone();
            if progress != seen {
                drop(state);
                if let Some(progress) = &progress {
                    on_progress(progress.clone());
                }
                seen = progress;
            } else if let Some(result) = result {
                drop(state);
                cancellation.check_cancelled()?;
                return result;
            } else {
                drop(
                    request
                        .0
                        .changed
                        .wait_timeout(state, Duration::from_millis(20))
                        .expect("history build"),
                );
            }
        }
    }
}

impl Drop for CommonRepository {
    fn drop(&mut self) {
        super::shared_ranges::invalidate(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::HistoryMode;

    #[test]
    fn alternate_resolution_errors_are_stable_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init(dir.path()).unwrap();
        let objects = repo.objects.store_ref().path();
        std::fs::create_dir_all(objects.join("info")).unwrap();
        std::fs::write(
            objects.join("info/alternates"),
            format!("{}\n", objects.display()),
        )
        .unwrap();
        let scope = next_id();
        let first = StoreKey::read(&repo, scope);
        assert!(
            first.alternates.is_err(),
            "the alternate cycle must fail resolution"
        );
        assert_eq!(first, StoreKey::read(&repo, scope));
        assert_ne!(first, StoreKey::read(&repo, next_id()));
    }

    fn wait_until(mut ready: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !ready() {
            assert!(
                std::time::Instant::now() < deadline,
                "request registration timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    struct Gate(Option<mpsc::Sender<()>>);
    impl Drop for Gate {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    #[test]
    fn requesters_cancel_independently_before_the_shared_build_queue() {
        for cancel_all in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let repo = Arc::new(GixRepo::new(
                dir.path().into(),
                gix::init(dir.path()).unwrap().into_sync(),
            ));
            let shared = repo.shared_store();
            let tokens: Vec<_> = (0..4).map(|_| CancellationToken::new()).collect();
            std::thread::scope(|scope| {
                let (tx, rx) = mpsc::channel();
                let gate = Gate(Some(tx));
                queue(Box::new(move || {
                    let _ = rx.recv();
                }))
                .unwrap();
                let threads: Vec<_> = tokens
                    .iter()
                    .map(|token| {
                        let repo = repo.clone();
                        scope.spawn(move || {
                            let mut progress = Vec::new();
                            let result = repo.build_history_index_impl(
                                HistoryMode::AllBranches,
                                None,
                                token,
                                &mut |value| progress.push(value),
                            );
                            (result, progress)
                        })
                    })
                    .collect();
                wait_until(|| {
                    shared
                        .indexes
                        .lock()
                        .unwrap()
                        .first()
                        .and_then(|entry| entry.build.upgrade())
                        .is_some_and(|build| build.state.lock().unwrap().requesters == 4)
                });
                let build = shared.indexes.lock().unwrap()[0].build.upgrade().unwrap();
                tokens[0].cancel();
                if cancel_all {
                    for token in &tokens[1..] {
                        token.cancel();
                    }
                }
                wait_until(|| {
                    build.state.lock().unwrap().requesters == if cancel_all { 0 } else { 3 }
                });
                assert_eq!(build.cancellation.is_cancelled(), cancel_all);
                drop(gate);
                let mut indexes = Vec::new();
                for (ix, thread) in threads.into_iter().enumerate() {
                    let (result, progress) = thread.join().unwrap();
                    if ix == 0 || cancel_all {
                        assert!(matches!(result.unwrap_err().kind(), ErrorKind::Cancelled));
                    } else {
                        assert!(!progress.is_empty());
                        indexes.push(result.unwrap().unwrap());
                    }
                }
                assert!(indexes.iter().all(|index| Arc::ptr_eq(index, &indexes[0])));
            });
            // A cancelled registration cannot poison a later request.
            assert!(
                repo.build_history_index_impl(
                    HistoryMode::AllBranches,
                    None,
                    &CancellationToken::new(),
                    &mut |_| {}
                )
                .unwrap()
                .is_some()
            );
        }
    }
}
