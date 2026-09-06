use super::history::gix_head_id_or_none;
use super::{
    GixRepo, LOG_FILE_FOLLOW_CACHE_LIMIT, LOG_PAGE_CACHE_LIMIT, LOG_PAGED_TOPO_WALK_CACHE_LIMIT,
    LOG_PAGED_WALK_CACHE_LIMIT, LogFileFollowCacheEntry, LogFileFollowCacheKey, LogPageCacheEntry,
    LogPageCacheKey, LogPageSeed, LogPagedWalk, LogPagedWalkCacheEntry, LogPagedWalkFilter,
    LogPagedWalkState, ShallowSnapshot, bstr_to_arc_str, oid_to_arc_str, submodules,
};
use crate::util::{
    bytes_to_text_preserving_utf8, parse_git_log_pretty_records_from_reader,
    path_buf_from_git_bytes, run_git_capture, run_git_parsed_stdout, unix_seconds_to_system_time,
    unix_seconds_to_system_time_or_epoch,
};
use gitcomet_core::domain::{
    Commit, CommitDetails, CommitFileChange, CommitId, CommitParentIds, EMPTY_TREE_ID, HistoryMode,
    LogCursor, LogPage, RecentCommitMessage, ReflogEntry, StashEntry,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::{CancellationToken, LogChunk, Result};
use gix::bstr::ByteSlice as _;
use gix::objs::FindExt as _;
use gix::traverse::commit::simple::CommitTimeOrder;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod commit_stats;
mod decode;
mod reflog;
mod repo_impl;
mod snapshot;
mod walk;

pub(super) use commit_stats::*;
pub(super) use decode::*;
pub(super) use reflog::*;
pub(super) use walk::*;

#[cfg(test)]
mod tests;
