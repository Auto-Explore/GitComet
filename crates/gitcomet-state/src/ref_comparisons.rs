use crate::model::{Loadable, RepoId, RepoLifetime};
use gitcomet_core::domain::CommitId;
use gitcomet_core::ref_comparison::{RefComparison, RefComparisonBatch, RefComparisonTarget};
use gitcomet_core::services::Result;
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

const DECORATION_PAGES: usize = 256;
pub const MAX_COMPARISON_PROJECTIONS: usize = 65_536;

#[derive(Clone, Debug)]
pub struct ComparisonDecorations {
    pages: Arc<Vec<Arc<FxHashMap<CommitId, RefComparison>>>>,
    len: usize,
}

impl Default for ComparisonDecorations {
    fn default() -> Self {
        Self {
            pages: Arc::new((0..DECORATION_PAGES).map(|_| Arc::default()).collect()),
            len: 0,
        }
    }
}

impl ComparisonDecorations {
    fn page(oid: &CommitId) -> usize {
        let mut hasher = FxHasher::default();
        oid.hash(&mut hasher);
        hasher.finish() as usize % DECORATION_PAGES
    }

    pub fn get(&self, oid: &CommitId) -> Option<&RefComparison> {
        self.pages[Self::page(oid)].get(oid)
    }

    pub(crate) fn publish(&mut self, results: Vec<RefComparison>) {
        let pages = Arc::make_mut(&mut self.pages);
        for result in results {
            let page = Arc::make_mut(&mut pages[Self::page(&result.subject)]);
            if self.len >= MAX_COMPARISON_PROJECTIONS && !page.contains_key(&result.subject) {
                continue;
            }
            if page.insert(result.subject.clone(), result).is_none() {
                self.len += 1;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct RefComparisonsState {
    pub target: Loadable<RefComparisonTarget>,
    pub decorations: ComparisonDecorations,
    pub branches: Arc<FxHashMap<String, CommitId>>,
    pub worktrees: Arc<FxHashMap<PathBuf, CommitId>>,
    pub rev: u64,
    pub(crate) generation: u64,
    pub(crate) publication_generation: u64,
    pub(crate) source: Option<(u64, u64, u64, u64, Option<String>)>,
    pub(crate) visible: Arc<Vec<CommitId>>,
    pub(crate) captured_target: Option<RefComparisonTarget>,
    pub(crate) resolved_source: Option<(u64, u64, u64, u64, Option<String>)>,
}

impl Default for RefComparisonsState {
    fn default() -> Self {
        Self {
            target: Loadable::NotLoaded,
            decorations: Default::default(),
            branches: Arc::default(),
            worktrees: Arc::default(),
            rev: 0,
            generation: 0,
            publication_generation: 0,
            source: None,
            visible: Arc::default(),
            captured_target: None,
            resolved_source: None,
        }
    }
}

impl RefComparisonsState {
    pub(crate) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.source = None;
        if matches!(self.target, Loadable::Ready(_)) {
            self.target = Loadable::Loading;
            self.rev = self.rev.wrapping_add(1);
        }
    }
}

#[derive(Debug)]
pub enum RefComparisonsMsg {
    Ensure {
        repo_id: RepoId,
    },
    Prioritize {
        repo_id: RepoId,
        subjects: Vec<CommitId>,
    },
    Deeper {
        repo_id: RepoId,
    },
    Target {
        repo_id: RepoId,
        lifetime: RepoLifetime,
        generation: u64,
        result: Result<RefComparisonTarget>,
    },
    Publish {
        repo_id: RepoId,
        lifetime: RepoLifetime,
        generation: u64,
        batch: RefComparisonBatch,
    },
}

#[derive(Clone, Debug)]
pub struct RefComparisonsEffect {
    pub priority_only: bool,
    pub repo_id: RepoId,
    pub lifetime: RepoLifetime,
    pub generation: u64,
    pub reference: Option<String>,
    pub subjects: Arc<Vec<CommitId>>,
    pub deep: bool,
}
