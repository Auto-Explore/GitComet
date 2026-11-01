use gitcomet_core::conflict_session::{
    ConflictRegionSourceRanges, ConflictRegionText, ConflictResolverStrategy, ConflictSession,
};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictRegionOrigin {
    pub(crate) regions: Arc<[ConflictRegionIdentity]>,
    pub(crate) source_ranges: Arc<[ConflictRegionSourceRanges]>,
    pub(crate) basis: ConflictRegionBasis,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConflictRegionIdentity {
    pub(crate) base: Option<ConflictRegionText>,
    pub(crate) ours: ConflictRegionText,
    pub(crate) theirs: ConflictRegionText,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ConflictRegionBasis {
    Provisional { marker_projection: Option<Arc<str>> },
    StageBacked,
}

impl ConflictRegionOrigin {
    pub fn from_session(session: &ConflictSession) -> Self {
        let basis = if session.strategy == ConflictResolverStrategy::FullTextResolver
            && session.base.is_absent()
            && session.ours.is_absent()
            && session.theirs.is_absent()
        {
            ConflictRegionBasis::Provisional {
                marker_projection: session.marker_projection.clone(),
            }
        } else {
            ConflictRegionBasis::StageBacked
        };
        Self {
            regions: session
                .regions
                .iter()
                .map(|region| ConflictRegionIdentity {
                    base: region.base.clone(),
                    ours: region.ours.clone(),
                    theirs: region.theirs.clone(),
                })
                .collect(),
            source_ranges: session.region_source_ranges.clone().into(),
            basis,
        }
    }

    pub(crate) fn is_provisional(&self) -> bool {
        matches!(self.basis, ConflictRegionBasis::Provisional { .. })
    }

    pub fn matches_session(&self, session: &ConflictSession) -> bool {
        let is_provisional = session.strategy == ConflictResolverStrategy::FullTextResolver
            && session.base.is_absent()
            && session.ours.is_absent()
            && session.theirs.is_absent();
        self.is_provisional() == is_provisional
            && (!is_provisional || self.marker_projection() == session.marker_projection.as_deref())
            && self.source_ranges.as_ref() == session.region_source_ranges
            && self.regions.len() == session.regions.len()
            && self
                .regions
                .iter()
                .zip(&session.regions)
                .all(|(origin, current)| {
                    origin.base == current.base
                        && origin.ours == current.ours
                        && origin.theirs == current.theirs
                })
    }

    pub(crate) fn marker_projection(&self) -> Option<&str> {
        match &self.basis {
            ConflictRegionBasis::Provisional { marker_projection } => marker_projection.as_deref(),
            ConflictRegionBasis::StageBacked => None,
        }
    }
}
