use super::diff_text::{
    DiffSyntaxBudget, DiffSyntaxLanguage, DiffSyntaxMode, PrepareDiffSyntaxDocumentResult,
    diff_syntax_language_for_path, inject_background_prepared_diff_syntax_document,
    prepare_diff_syntax_document_with_budget_reuse_text,
};
use super::*;
use crate::kit::text_model::TextModel;
use crate::kit::{
    benchmark_text_input_runs_legacy_visible_window,
    benchmark_text_input_runs_streamed_visible_window,
    benchmark_text_input_shaping_slice as hash_text_input_shaping_slice,
    benchmark_text_input_wrap_rows_for_line as estimate_text_input_wrap_rows_for_line,
};
use crate::theme::AppTheme;
use crate::view::history_graph;
use crate::view::mod_helpers::{
    PaneResizeHandle, PaneResizeState, StatusMultiSelection, StatusSection,
};
use crate::view::next_pane_resize_drag_width;
use crate::view::panes::main::{
    AsciiCaseInsensitiveNeedle, DiffSearchQueryReuse,
    diff_cache::{
        PagedFileDiffRows, PagedPatchDiffRows, PagedPatchSplitRows, PatchInlineVisibleMap,
    },
    diff_search_query_reuse,
};
use crate::view::path_display;
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::domain::{
    Branch, Commit, CommitDetails, CommitFileChange, CommitId, Diff, DiffArea, DiffLine,
    DiffRowProvider, DiffTarget, FileDiffText, FileStatus, FileStatusKind, LogCursor, LogPage,
    LogScope, Remote, RemoteBranch, RepoSpec, RepoStatus, StashEntry, Submodule, SubmoduleStatus,
    Tag, Upstream, UpstreamDivergence, Worktree,
};
use gitcomet_core::git_ops_trace::{self, GitOpTraceSnapshot};
use gitcomet_core::services::{GitBackend, GitRepository};
use gitcomet_git_gix::GixBackend;
use gitcomet_state::benchmarks::{
    dispatch_sync, reset_conflict_resolutions_sync, set_conflict_region_choice_sync,
    with_select_diff_sync, with_set_active_repo_sync, with_stage_path_sync, with_stage_paths_sync,
    with_unstage_path_sync, with_unstage_paths_sync,
};
use gitcomet_state::model::{AppState, ConflictFile, Loadable, RepoId, RepoState};
use gitcomet_state::msg::{Effect, InternalMsg, Msg, RepoPath, RepoPathList};
use rustc_hash::FxHasher;
use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write as _;
use std::ops::Range;
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tempfile::TempDir;

mod conflict;
mod diff_fixtures;
mod git_ops;
mod real_repo;
mod repo_history;
mod runtime_fixtures;
mod scroll_fixtures;
mod search_fixtures;
mod status_fixtures;
mod support;
mod syntax;
mod text_fixtures;

pub use conflict::*;
pub(crate) use diff_fixtures::should_hide_unified_diff_header_for_bench;
pub use diff_fixtures::*;
pub use git_ops::*;
pub(crate) use git_ops::{
    build_git_ops_status_repo, git_command, git_ops_status_relative_path, git_stdout,
    hash_parsed_diff, hash_repo_status, run_git,
};
pub use real_repo::*;
pub use repo_history::*;
pub(crate) use repo_history::{
    CommitDetailsMessageRenderConfig, CommitDetailsMessageRenderState,
    CommitDetailsVisibleMessageLine, hash_branch_sidebar_rows, reset_repo_switch_bench_state,
};
pub use runtime_fixtures::*;
pub use scroll_fixtures::*;
pub use search_fixtures::*;
pub use status_fixtures::*;
pub(crate) use support::*;
pub use syntax::*;
pub use text_fixtures::*;
pub(crate) use text_fixtures::{estimate_tabbed_wrap_rows, wrap_columns_for_benchmark_width};

// Re-export frame timing capture from view::perf for use in benchmark harnesses.
#[cfg(feature = "benchmarks")]
pub use crate::view::perf::{FrameTimingCapture, FrameTimingStats};

#[cfg(test)]
mod tests;
