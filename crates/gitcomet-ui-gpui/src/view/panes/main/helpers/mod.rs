//! The main pane's state and the free functions its impls share.
//!
//! `state` declares `MainPaneView` and the types its fields hold; the other
//! modules are pure helpers, re-exported here so `helpers::*` paths still work.

use super::*;

mod conflict_projection;
mod conflict_provenance;
mod conflict_segments;
mod line_index;
mod mergetool;
mod presentation;
mod state;

pub(in crate::view) use conflict_projection::*;
pub(super) use conflict_provenance::*;
pub(super) use conflict_segments::*;
pub(in crate::view) use line_index::*;
pub(super) use mergetool::*;
pub(in crate::view) use presentation::*;
pub(crate) use state::*;
