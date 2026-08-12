//! Memoises the row model the badge pickers build.
//!
//! `PopoverHost` is an uncached overlay view, so anything that notifies it —
//! most often a hover moving from one row to the next — re-renders the whole
//! popover. Rebuilding every branch label, remote label and metadata line on
//! each of those frames is what made the workspace and branch badge pickers feel
//! sluggish. The rows only change when the repository data behind them changes,
//! so they are built once per change and reused until then.
//!
//! The cache lives in a `RefCell` because the row builders read the repository
//! out of the host while the cache slot is written, and the pickers reach here
//! from both `&PopoverHost` and `&mut PopoverHost` call sites.
//!
//! **The key must name every revision the rows read.** A missing one shows stale
//! rows with no visible error — the same trap [`super::fingerprint`] documents,
//! whose match arms for these two popover kinds list the same revisions.

use super::*;
use std::cell::RefCell;
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
    Workspace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RowsCacheKey {
    owner: RowsCacheOwner,
    repo_id: RepoId,
    /// Revisions of every field the row builders read.
    revs: RowsCacheRevs,
    /// The workspace picker marks the row whose path is the active workdir.
    workdir: std::path::PathBuf,
    query: String,
    date_bucket: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RowsCacheRevs {
    head_branch: u64,
    branches: u64,
    remote_branches: u64,
    ref_metadata: u64,
    worktrees: u64,
    /// `Loadable` kind of the repo's open state — the builders return early
    /// while a repository is still opening.
    open: u8,
}

impl RowsCacheKey {
    /// Key for the branch badge's checkout picker. Mirrors the
    /// `PopoverKind::BranchPicker` arm of [`super::fingerprint`].
    pub(super) fn for_branch_checkout(repo: &RepoState, query: &str) -> Self {
        Self {
            owner: RowsCacheOwner::BranchCheckout,
            repo_id: repo.id,
            revs: RowsCacheRevs {
                head_branch: repo.head_branch_rev,
                branches: repo.branches_rev,
                remote_branches: repo.remote_branches_rev,
                ref_metadata: repo.ref_metadata_rev,
                ..RowsCacheRevs::default()
            },
            workdir: std::path::PathBuf::new(),
            query: query.to_string(),
            date_bucket: date_bucket(SystemTime::now()),
        }
    }

    /// Key for the workspace badge's picker. Mirrors the
    /// `RepoPopoverKind::Worktree` arm of [`super::fingerprint`], which tracks
    /// HEAD as well because the create row names the ref it would branch from.
    pub(super) fn for_workspace(repo: &RepoState, query: &str) -> Self {
        Self {
            owner: RowsCacheOwner::Workspace,
            repo_id: repo.id,
            revs: RowsCacheRevs {
                head_branch: repo.head_branch_rev,
                worktrees: repo.worktrees_rev,
                open: loadable_kind(&repo.open),
                ..RowsCacheRevs::default()
            },
            workdir: repo.spec.workdir.clone(),
            query: query.to_string(),
            // No relative dates on these rows, so the clock is not part of the
            // key: the worktree rows say what is checked out and where, not when.
            date_bucket: 0,
        }
    }

    fn query(&self) -> &str {
        &self.query
    }
}

fn loadable_kind<T>(loadable: &Loadable<T>) -> u8 {
    match loadable {
        Loadable::NotLoaded => 0,
        Loadable::Loading => 1,
        Loadable::Ready(_) => 2,
        Loadable::Error(_) => 3,
    }
}

fn date_bucket(now: SystemTime) -> u64 {
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
    let layout = components::picker_prompt_layout(&items, key.query());
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

    #[test]
    fn an_unchanged_repo_reuses_the_built_rows() {
        let repo = repo();
        let cache = RowsCache::default();
        let builds = std::cell::Cell::new(0);

        build_once(&cache, RowsCacheKey::for_workspace(&repo, ""), &builds);
        build_once(&cache, RowsCacheKey::for_workspace(&repo, ""), &builds);

        assert_eq!(builds.get(), 1, "the second frame must reuse the rows");
    }

    #[test]
    fn every_revision_the_rows_read_invalidates_them() {
        // A revision missing from the key shows stale rows with no visible
        // error, so each one is checked here rather than trusted.
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
            let cache = RowsCache::default();
            let builds = std::cell::Cell::new(0);
            build_once(
                &cache,
                RowsCacheKey::for_branch_checkout(&repo, ""),
                &builds,
            );

            bump(&mut repo);
            build_once(
                &cache,
                RowsCacheKey::for_branch_checkout(&repo, ""),
                &builds,
            );

            assert_eq!(builds.get(), 2, "{label} must invalidate the branch rows");
        }
    }

    #[test]
    fn the_workspace_key_tracks_worktrees_head_and_the_active_workdir() {
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
            let cache = RowsCache::default();
            let builds = std::cell::Cell::new(0);
            build_once(&cache, RowsCacheKey::for_workspace(&repo, ""), &builds);

            bump(&mut repo);
            build_once(&cache, RowsCacheKey::for_workspace(&repo, ""), &builds);

            assert_eq!(
                builds.get(),
                2,
                "{label} must invalidate the workspace rows"
            );
        }
    }

    #[test]
    fn typing_rebuilds_the_rows() {
        let repo = repo();
        let cache = RowsCache::default();
        let builds = std::cell::Cell::new(0);

        build_once(&cache, RowsCacheKey::for_workspace(&repo, ""), &builds);
        build_once(&cache, RowsCacheKey::for_workspace(&repo, "fea"), &builds);

        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn the_branch_key_buckets_the_clock_so_relative_dates_refresh() {
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
