use crate::domain::SharedLineText;
use rustc_hash::{FxHashMap, FxHasher};
use std::borrow::Cow;
use std::cell::{OnceCell, RefCell};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::marker::PhantomData;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDiffRowKind {
    Context,
    Add,
    Remove,
    Modify,
}

pub(super) const REPLACEMENT_ALIGN_CELL_BUDGET: usize = 50_000;
pub(super) const REPLACEMENT_GAP_COST: u32 = 100;
pub(super) const REPLACEMENT_PAIR_BASE_COST: u32 = 80;
pub(super) const REPLACEMENT_PAIR_SCALE_COST: u32 = 120;
pub(super) const REPLACEMENT_DISSIMILAR_PENALTY_COST: u32 = 40;
pub(super) const REPLACEMENT_DISSIMILAR_PENALTY_MIN_LEN: usize = 4;
pub(super) const ASCII_BITPARALLEL_MAX_PATTERN_LEN: usize = 128;
pub(super) const SIDE_BY_SIDE_HISTOGRAM_LINE_THRESHOLD: usize = 1_024;
pub(super) const SIDE_BY_SIDE_LINEAR_FALLBACK_LINE_THRESHOLD: usize = 100_000;
pub(super) const PATIENCE_POSITIONAL_FALLBACK_LINE_THRESHOLD: usize = 2_048;
pub(super) const SIDE_BY_SIDE_SPARSE_POSITIONAL_MAX_CHANGED_RATIO_DENOMINATOR: usize = 4;
pub(super) const SIDE_BY_SIDE_SPARSE_POSITIONAL_MAX_BLOCK_LEN: usize = 1;
// UTF-8 code points are at most 4 bytes wide, so 3 bytes of lookaround is
// enough to recover the nearest character boundary around any requested slice.
pub(super) const UTF8_SUBSLICE_BOUNDARY_LOOKAROUND_BYTES: usize = 3;

mod edits;
mod plan;
mod rows;
mod text;

pub(crate) use edits::*;
pub use plan::*;
pub use rows::*;
pub use text::*;

#[cfg(test)]
mod perf_probe_tests;
#[cfg(test)]
mod tests;
