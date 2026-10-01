//! Conflict resolver actions and state sync for [`MainPaneView`].
//!
//! Extracted from `actions_impl.rs`: mergetool bootstrap tracing, conflict
//! navigation, pick/choice application, output editing ops, session
//! resolution sync, and autosolve dispatch. Children split it by concern:
//! `sync` (with its pure `bootstrap` phases), `navigation`, `resolution`,
//! `region_edits`, `output`, and `view_mode`.

use super::core_impl::uniform_list_base_handle;
use super::helpers::*;
use super::*;
use crate::kit::text_model::TextModelSnapshot;
use gitcomet_core::mergetool_trace::{
    self, MergetoolTraceEvent, MergetoolTraceRenderingMode, MergetoolTraceSideStats,
    MergetoolTraceStage,
};
use rustc_hash::{FxHashMap, FxHasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

mod bootstrap;
mod navigation;
mod output;
mod region_edits;
mod resolution;
mod sync;
mod view_mode;
