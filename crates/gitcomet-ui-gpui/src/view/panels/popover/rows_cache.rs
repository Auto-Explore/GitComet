//! Memoises the row model every picker builds.
//!
//! `PopoverHost` is an uncached overlay view, so anything that notifies it —
//! most often a hover moving from one row to the next — re-renders the whole
//! popover. Rebuilding every branch label, remote label and metadata line on
//! each of those frames is what made the workspace and branch badge pickers feel
//! sluggish. The rows only change when the data behind them changes, so they are
//! built once per change and reused until then.
//!
//! Going through here also gives a picker one list rather than two: the rows the
//! panel renders and the list the arrow keys walk come out of the same build, so
//! Enter cannot land on a different row than the highlighted one.
//!
//! The cache lives in a `RefCell` because the row builders read the repository
//! out of the host while the cache slot is written, and the pickers reach here
//! from both `&PopoverHost` and `&mut PopoverHost` call sites.
//!
//! **The signature must name every input the rows read.** A missing one shows
//! stale rows with no visible error — the same trap [`super::fingerprint`]
//! documents. Each picker computes its own signature next to the code that reads
//! those inputs, so the two stay in each other's sight.

use super::*;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::SystemTime;

/// Relative dates ("3 mins ago") on the branch rows' detail line would make
/// every clock reading a distinct key, so the reading is bucketed: rows rebuild
/// at most once a minute for the sake of their timestamps.
const DATE_BUCKET_SECS: u64 = 60;

/// Which picker a cache slot belongs to. Slots are per-picker already; this only
/// guards against a key being compared across pickers if they ever share one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RowsCacheOwner {
    BranchCheckout,
    BranchRefs,
    Workspace,
    RepoPicker,
    Stash,
    FileHistory,
    Submodule,
    Worktree,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RowsCacheKey {
    owner: RowsCacheOwner,
    /// Digest of every input the picker's rows are built from, computed by the
    /// picker itself with [`signature`]. A `u64` rather than the values because
    /// the inputs are lists — holding them by value would mean cloning every
    /// path, ref name or commit id on every render, which costs more than the
    /// rebuild it is here to save.
    signature: u64,
    query: String,
    /// Sections folded away while these rows were laid out. Part of the key
    /// because [`get_or_build`] resolves the layout, and a folded section drops
    /// its rows from it.
    collapsed: BTreeSet<gpui::SharedString>,
}

impl RowsCacheKey {
    pub(super) fn new(owner: RowsCacheOwner, signature: u64, query: &str) -> Self {
        Self {
            owner,
            signature,
            query: query.to_string(),
            collapsed: BTreeSet::new(),
        }
    }

    /// For the one picker that folds sections away. Everything else lays its rows
    /// out with nothing folded.
    pub(super) fn with_collapsed(mut self, collapsed: &BTreeSet<gpui::SharedString>) -> Self {
        self.collapsed = collapsed.clone();
        self
    }

    fn query(&self) -> &str {
        &self.query
    }

    fn collapsed(&self) -> &BTreeSet<gpui::SharedString> {
        &self.collapsed
    }
}

/// Digests the inputs a picker's rows are built from into one `u64`.
///
/// Every input the build closure reads has to be hashed in here — a missing one
/// leaves the picker showing rows built from data that has since changed, with
/// nothing on screen to say so.
pub(super) fn signature(
    inputs: impl FnOnce(&mut std::collections::hash_map::DefaultHasher),
) -> u64 {
    use std::hash::Hasher;

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    inputs(&mut hasher);
    hasher.finish()
}

/// Which arm of a `Loadable` is in hand, for signatures whose rows change shape
/// between "loading" and "ready" (most builders return early on the others).
pub(super) fn loadable_kind<T>(loadable: &Loadable<T>) -> u8 {
    match loadable {
        Loadable::NotLoaded => 0,
        Loadable::Loading => 1,
        Loadable::Ready(_) => 2,
        Loadable::Error(_) => 3,
    }
}

/// Coarsens a clock reading for signatures whose rows show relative dates, so
/// they rebuild at most once a bucket rather than on every frame.
pub(super) fn date_bucket(now: SystemTime) -> u64 {
    now.duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_secs() / DATE_BUCKET_SECS)
        .unwrap_or(0)
}

/// A built row model plus the layout that filtering produced for it.
pub(super) struct CachedRows<T> {
    /// Rows in pre-filter order, which is the index space `PickerPrompt` reports
    /// back to `on_select` and the one `marked_index` lives in.
    pub(super) items: Rc<[components::PickerPromptItem]>,
    pub(super) payloads: Rc<[T]>,
    pub(super) layout: Rc<components::PickerPromptLayout>,
    pub(super) marked_index: Option<usize>,
}

impl<T> CachedRows<T> {
    /// Used when there is no repository to build rows from.
    pub(super) fn empty() -> Rc<Self> {
        Rc::new(Self {
            items: Rc::from(Vec::new()),
            payloads: Rc::from(Vec::new()),
            layout: Rc::new(components::PickerPromptLayout::default()),
            marked_index: None,
        })
    }

    /// Payloads of the rows that survived the filter, in the order the picker
    /// renders them — the list keyboard navigation walks, so Enter can never
    /// land on a different row than the highlighted one.
    pub(super) fn filtered_payloads(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.layout
            .item_indices
            .iter()
            .filter_map(|ix| self.payloads.get(*ix).cloned())
            .collect()
    }
}

pub(super) struct RowsCache<T> {
    slot: RefCell<Option<(RowsCacheKey, Rc<CachedRows<T>>)>>,
}

impl<T> Default for RowsCache<T> {
    fn default() -> Self {
        Self {
            slot: RefCell::new(None),
        }
    }
}

impl<T> RowsCache<T> {
    /// Drops the cached rows. Called when a picker opens so a stale list cannot
    /// flash before the first rebuild.
    pub(super) fn clear(&self) {
        self.slot.borrow_mut().take();
    }
}

/// Returns the cached rows for `key`, building them with `build` on a miss.
///
/// `build` receives the clock reading `key` was bucketed from, so the rows and
/// the key agree on what "now" was.
pub(super) fn get_or_build<T, F>(
    cache: &RowsCache<T>,
    key: RowsCacheKey,
    build: F,
) -> Rc<CachedRows<T>>
where
    F: FnOnce(SystemTime) -> (Vec<components::PickerPromptItem>, Vec<T>, Option<usize>),
{
    if let Some((cached_key, cached)) = cache.slot.borrow().as_ref()
        && *cached_key == key
    {
        return Rc::clone(cached);
    }

    let (items, payloads, marked_index) = build(SystemTime::now());
    // The collapsed set is empty for every picker but the repository one, where
    // this is the same call `picker_prompt_layout` makes.
    let layout =
        components::picker_prompt_layout_with_collapsed(&items, key.query(), key.collapsed());
    let built = Rc::new(CachedRows {
        items: Rc::from(items),
        payloads: Rc::from(payloads),
        layout: Rc::new(layout),
        marked_index,
    });
    *cache.slot.borrow_mut() = Some((key, Rc::clone(&built)));
    built
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A named revision bump, and the label the assertion reports it under.
    type RevisionBump = (&'static str, fn(&mut RepoState));

    fn repo() -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            gitcomet_core::domain::RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/rows_cache"),
            },
        );
        repo.open = Loadable::Ready(());
        repo.head_branch = Loadable::Ready("main".to_string());
        repo
    }

    /// Counts builds so the tests can tell a reuse from a rebuild.
    fn build_once(cache: &RowsCache<u8>, key: RowsCacheKey, builds: &std::cell::Cell<usize>) {
        get_or_build(cache, key, |_now| {
            builds.set(builds.get() + 1);
            (
                vec![components::PickerPromptItem::plain("main")],
                vec![0u8],
                None,
            )
        });
    }

    fn workspace_key(repo: &RepoState, query: &str) -> RowsCacheKey {
        RowsCacheKey::new(
            RowsCacheOwner::Workspace,
            workspace_picker::rows_signature(repo),
            query,
        )
    }

    #[test]
    fn an_unchanged_repo_reuses_the_built_rows() {
        let repo = repo();
        let cache = RowsCache::default();
        let builds = std::cell::Cell::new(0);

        build_once(&cache, workspace_key(&repo, ""), &builds);
        build_once(&cache, workspace_key(&repo, ""), &builds);

        assert_eq!(builds.get(), 1, "the second frame must reuse the rows");
    }

    /// An input missing from a signature shows stale rows with no visible error,
    /// so each picker's is checked against the fields its builder reads rather
    /// than trusted. The signatures live beside those builders; these assert they
    /// stayed in step.
    #[test]
    fn every_input_the_branch_rows_read_invalidates_them() {
        let bumps: Vec<RevisionBump> = vec![
            ("head_branch_rev", |repo| {
                repo.head_branch_rev = repo.head_branch_rev.wrapping_add(1)
            }),
            ("branches_rev", |repo| {
                repo.branches_rev = repo.branches_rev.wrapping_add(1)
            }),
            ("remote_branches_rev", |repo| {
                repo.remote_branches_rev = repo.remote_branches_rev.wrapping_add(1)
            }),
            ("ref_metadata_rev", |repo| {
                repo.ref_metadata_rev = repo.ref_metadata_rev.wrapping_add(1)
            }),
        ];

        for (label, bump) in bumps {
            let mut repo = repo();
            let before = branch_picker::rows_signature(&repo);
            bump(&mut repo);
            assert_ne!(
                before,
                branch_picker::rows_signature(&repo),
                "{label} must invalidate the branch rows"
            );
        }
    }

    #[test]
    fn the_workspace_signature_tracks_worktrees_head_and_the_active_workdir() {
        let checks: Vec<RevisionBump> = vec![
            ("worktrees_rev", |repo| {
                repo.worktrees_rev = repo.worktrees_rev.wrapping_add(1)
            }),
            ("head_branch_rev", |repo| {
                repo.head_branch_rev = repo.head_branch_rev.wrapping_add(1)
            }),
            ("workdir", |repo| {
                repo.spec.workdir = std::path::PathBuf::from("/tmp/rows_cache_other")
            }),
            ("open", |repo| repo.open = Loadable::Loading),
        ];

        for (label, bump) in checks {
            let mut repo = repo();
            let before = workspace_picker::rows_signature(&repo);
            bump(&mut repo);
            assert_ne!(
                before,
                workspace_picker::rows_signature(&repo),
                "{label} must invalidate the workspace rows"
            );
        }
    }

    #[test]
    fn typing_rebuilds_the_rows() {
        let repo = repo();
        let cache = RowsCache::default();
        let builds = std::cell::Cell::new(0);

        build_once(&cache, workspace_key(&repo, ""), &builds);
        build_once(&cache, workspace_key(&repo, "fea"), &builds);

        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn the_branch_signature_buckets_the_clock_so_relative_dates_refresh() {
        // Aligned to a bucket boundary: the split points are fixed, so an
        // arbitrary reading can sit anywhere inside its bucket.
        let now =
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 * DATE_BUCKET_SECS);

        assert_eq!(
            date_bucket(now),
            date_bucket(now + std::time::Duration::from_secs(DATE_BUCKET_SECS - 1)),
            "readings inside one bucket must share a key"
        );
        assert_ne!(
            date_bucket(now),
            date_bucket(now + std::time::Duration::from_secs(DATE_BUCKET_SECS)),
            "a new bucket must rebuild the rows so the dates advance"
        );
    }

    #[test]
    fn filtered_payloads_follow_the_layout_order() {
        let items = vec![
            components::PickerPromptItem::plain("zulu-a"),
            components::PickerPromptItem::plain("alpha"),
        ];
        let layout = components::picker_prompt_layout(&items, "a");
        let cached = CachedRows {
            items: Rc::from(items),
            payloads: Rc::from(vec![10u8, 20u8]),
            layout: Rc::new(layout),
            marked_index: None,
        };

        // "alpha" matches at index 0 and sorts ahead of "zulu-a"'s later hit, so
        // the payloads come back in render order, not declaration order.
        assert_eq!(cached.filtered_payloads(), vec![20u8, 10u8]);
    }
}
