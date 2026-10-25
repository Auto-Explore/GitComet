use crate::model::{Loadable, RepoId, RepoLifetime};
use crate::ref_comparison_decorations::ComparisonDecorations;
use crate::ref_comparison_projections::ComparisonProjections;
use gitcomet_core::domain::CommitId;
use gitcomet_core::ref_comparison::{RefComparisonBatch, RefComparisonTarget};
use gitcomet_core::ref_comparison_memory::{ComparisonMemory, ComparisonReservation};
use gitcomet_core::services::Result;
use std::sync::Arc;

pub const MAX_COMPARISON_PROJECTIONS: usize = 65_536;
#[cfg(test)]
pub(crate) static MEMORY_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Default)]
pub struct ComparisonPriorities {
    pub subjects: Vec<CommitId>,
    _reservation: Option<Arc<ComparisonReservation>>,
}

impl std::ops::Deref for ComparisonPriorities {
    type Target = Vec<CommitId>;
    fn deref(&self) -> &Self::Target {
        &self.subjects
    }
}

impl ComparisonPriorities {
    pub(crate) fn capture(
        memory: &Arc<ComparisonMemory>,
        subjects: Vec<CommitId>,
    ) -> Option<Arc<Self>> {
        let reservation = memory.reserve(
            gitcomet_core::ref_comparison_memory::vector_bytes::<CommitId>(128)
                + 128 * gitcomet_core::ref_comparison_memory::COMPARISON_OID_BYTES,
        )?;
        Some(Arc::new(Self {
            subjects: subjects.into_iter().take(128).collect(),
            _reservation: Some(reservation),
        }))
    }
}

#[derive(Clone, Debug)]
pub struct RefComparisonsState {
    pub target: Loadable<RefComparisonTarget>,
    pub decorations: ComparisonDecorations,
    pub projections: Arc<ComparisonProjections>,
    pub(crate) memory: Option<Arc<ComparisonMemory>>,
    pub deferred: Option<gitcomet_core::ref_comparison::RefComparisonReason>,
    pub rev: u64,
    pub(crate) generation: u64,
    pub(crate) publication_generation: Option<u64>,
    pub(crate) context_epoch: u64,
    pub(crate) source: Option<(u64, u64, u64, u64, Option<u64>)>,
    pub(crate) visible: Arc<ComparisonPriorities>,
    pub(crate) captured_target: Option<RefComparisonTarget>,
    pub(crate) resolved_source: Option<(u64, u64, u64, u64, Option<u64>)>,
    pub(crate) target_reservation: Option<Arc<ComparisonReservation>>,
}

impl Default for RefComparisonsState {
    fn default() -> Self {
        Self {
            target: Loadable::NotLoaded,
            decorations: Default::default(),
            projections: Arc::default(),
            memory: None,
            deferred: None,
            rev: 0,
            generation: 0,
            publication_generation: None,
            context_epoch: 0,
            source: None,
            visible: Arc::default(),
            captured_target: None,
            resolved_source: None,
            target_reservation: None,
        }
    }
}

impl RefComparisonsState {
    pub(crate) fn forget_subjects(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.publication_generation = None;
        self.resolved_source = None;
        self.captured_target = None;
        self.target_reservation = None;
        self.projections = Arc::default();
        self.decorations = ComparisonDecorations::default();
        self.deferred = None;
    }

    pub(crate) fn invalidate_context(&mut self) {
        self.invalidate();
        self.context_epoch = self.context_epoch.wrapping_add(1);
        self.publication_generation = None;
        self.decorations = ComparisonDecorations::default();
        self.deferred = None;
    }

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
    Deferred {
        repo_id: RepoId,
        lifetime: RepoLifetime,
        generation: u64,
        reason: gitcomet_core::ref_comparison::RefComparisonReason,
        reservation: Arc<ComparisonReservation>,
    },
    Target {
        repo_id: RepoId,
        lifetime: RepoLifetime,
        generation: u64,
        result: Result<RefComparisonTarget>,
        reservation: Option<Arc<ComparisonReservation>>,
    },
    Publish {
        repo_id: RepoId,
        lifetime: RepoLifetime,
        generation: u64,
        batch: RefComparisonBatch,
        reservation: Arc<ComparisonReservation>,
    },
}

#[derive(Clone, Debug)]
pub enum RefComparisonsEffect {
    Release {
        lifetime: RepoLifetime,
    },
    Prioritize {
        lifetime: RepoLifetime,
        subjects: Arc<ComparisonPriorities>,
    },
    Compare(RefComparisonSubscription),
}

#[derive(Clone, Debug)]
pub struct RefComparisonSubscription {
    pub repo_id: RepoId,
    pub lifetime: RepoLifetime,
    pub generation: u64,
    pub reference: Option<String>,
    pub projections: Arc<ComparisonProjections>,
    pub memory: Arc<ComparisonMemory>,
    pub visible: Arc<ComparisonPriorities>,
    pub deep: bool,
}
