//! State and pure helpers shared by the diff renderer's implementations.

use super::*;

pub(in crate::view) mod conflict_projection;
mod conflict_provenance;
mod conflict_segments;
pub(in crate::view) mod line_index;
mod mergetool;
pub(in crate::view) mod presentation;
pub(in crate::view) mod state;

pub(super) use conflict_projection::*;
pub(super) use conflict_provenance::*;
pub(super) use conflict_segments::*;
pub(super) use line_index::*;
pub(super) use mergetool::*;
pub(super) use presentation::*;
pub(super) use state::*;
