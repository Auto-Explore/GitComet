//! The conflict resolver's model: conflict marker parsing, two- and three-way
//! row projections, navigation, provenance, resolution and autosolve summaries,
//! and word highlights. Rendering and styled-text caches stay in the UI host.

mod split_row_index;
mod word_highlight;

use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use split_row_index::SparseLineIndex;
#[cfg(test)]
use split_row_index::{CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES, CONFLICT_SPLIT_PAGE_SIZE};
pub use split_row_index::{ConflictSplitRowIndex, TwoWaySplitProjection, TwoWaySplitVisibleRow};
#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
pub use word_highlight::compute_three_way_word_highlights;
#[cfg(feature = "benchmarks")]
pub use word_highlight::{TwoWayWordHighlights, compute_two_way_word_highlights};
pub use word_highlight::{compute_word_highlights_for_row, compute_word_highlights_for_texts};

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;

pub use gitcomet_core::conflict_output::ConflictOutputChoice as ConflictChoice;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConflictResolverViewMode {
    ThreeWay,
    TwoWayDiff,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConflictRenderingMode {
    EagerSmallFile,
    StreamedLargeFile,
}

impl ConflictRenderingMode {
    pub fn is_streamed_large_file(self) -> bool {
        matches!(self, Self::StreamedLargeFile)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub enum ConflictPickSide {
    Ours,
    Theirs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutosolveTraceMode {
    /// The safe rules and the subchunk split, applied automatically when the
    /// file opened. Whitespace-only, regex and history merges are never
    /// automatic — kdiff3 parity, see the section 30 auto-solve policy.
    OnOpen,
    #[cfg(any(test, feature = "test-support"))]
    History,
}

mod autosolve;
mod block_diff;
mod generated_text;
mod markers;
mod nav;
mod projection;
mod provenance;
mod resolution;
mod three_way;

pub use autosolve::*;
pub use block_diff::*;
pub use generated_text::*;
pub use markers::*;
pub use nav::*;
pub use projection::*;
pub use provenance::*;
pub use resolution::*;
pub use three_way::*;

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests;
