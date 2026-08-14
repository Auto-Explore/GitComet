use super::util::{
    EffectAccumulator, apply_selected_diff_load_plan_state, diff_reload_effects, push_diagnostic,
    push_notification, selected_diff_load_plan,
};
use crate::model::{
    AppNotificationKind, AppState, CommitMultiSelection, ConflictFileLoadMode, DiagnosticKind,
    ForeignDiffOrigin, Loadable, RangeSelection, RepoId, RepoLoadsInFlight, RepoState,
    SidebarDataRequest, SidebarMode,
};
use crate::msg::{CommitSelectMode, ConflictAutosolveMode, Effect};
use gitcomet_core::conflict_session::{
    ConflictPayload, ConflictRegionResolution, ConflictRegionSourceRanges,
    ConflictResolverStrategy, ConflictSession, reconstruct_conflict_marker_sides,
};
use gitcomet_core::domain::{
    Branch, CommitDetails, CommitFileChange, CommitId, EMPTY_TREE_ID, FileEntry, FileSource,
    FileStatusKind, LogPage, RecentCommitMessage, RefMetadata, ReflogEntry, Remote, RemoteBranch,
    RemoteTag, RepoStatus, StashEntry, Submodule, Tag, UpstreamDivergence, Worktree,
    WorktreeDirtySummary,
};
use gitcomet_core::error::Error;
use gitcomet_core::merge::{MergeSource, OrderedSelection};
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) fn file_history_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    result: std::result::Result<LogPage, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.history_state.file_history_path.as_ref() == Some(&path)
    {
        repo_state.history_state.file_history = match result {
            Ok(v) => Loadable::Ready(Arc::new(v)),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
    }
    Vec::new()
}

pub(super) fn blame_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    source: gitcomet_core::domain::BlameSource,
    result: std::result::Result<Vec<gitcomet_core::services::BlameLine>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.history_state.blame_path.as_ref() == Some(&path)
        && repo_state.history_state.blame_source.as_ref() == Some(&source)
    {
        let retained = repo_state.history_state.retained_blame_while_loading.take();
        repo_state.history_state.blame = match result {
            // Reuse the retained allocation when the reload produced identical
            // annotations, so the view's `Arc`-identity fingerprints and the
            // memoized blame time range stay valid and nothing repaints.
            Ok(v) => Loadable::Ready(match retained {
                Some(prev) if *prev == v => prev,
                _ => Arc::new(v),
            }),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
    }
    Vec::new()
}

pub(super) fn conflict_file_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    result: std::result::Result<Option<crate::model::ConflictFile>, Error>,
    conflict_session: Option<ConflictSession>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.conflict_state.conflict_file_path.as_ref() == Some(&path)
    {
        // A same-path reload stashes the previous session (see
        // `reset_conflict_target_reload_state`); it counts as the existing
        // session for resolution restore. Ordinary reloads suppress on-open
        // autosolve, while the provisional CurrentOnly -> Full upgrade does not.
        let stashed_session = repo_state.conflict_state.session_pending_restore.take();
        let existing_session = repo_state
            .conflict_state
            .conflict_session
            .as_ref()
            .or(stashed_session.as_ref());
        // CurrentOnly sessions are provisional: they preserve marker-backed
        // picks during the fast first paint, but the subsequent Full load is
        // still the first stage-backed open and must run on-open autosolve.
        let fresh_open =
            existing_session.is_none_or(conflict_session_uses_provisional_stage_inputs);
        let session = conflict_session.or_else(|| match &result {
            Ok(Some(file)) => build_conflict_session(repo_state, file),
            _ => None,
        });
        let session = session.map(|mut session| {
            if let Some(existing_session) = existing_session {
                restore_conflict_session_resolutions(existing_session, &mut session);
            }
            session
        });
        let session_is_provisional = session
            .as_ref()
            .is_some_and(conflict_session_uses_provisional_stage_inputs);
        let value = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        let keep_stashed_session = session.is_none() && stashed_session.is_some();
        repo_state.set_conflict_file(value);
        repo_state.set_conflict_session(session);
        if keep_stashed_session {
            repo_state.conflict_state.session_pending_restore = stashed_session;
        }
        if fresh_open
            && !session_is_provisional
            && repo_state.conflict_state.conflict_session.is_some()
        {
            auto_resolve_session_on_open(repo_state, &path);
        }
    }
    Vec::new()
}

/// UI_DESIGN.md section 30 auto-solve policy: only the always-safe rules
/// (identical sides, one-side-changed) and the subchunk split apply
/// automatically when a conflicted file first opens in the resolver.
///
/// Whitespace-only conflicts and regex normalization are deliberately left
/// alone, matching KDiff3: its `WhiteSpace2FileMergeDefault` /
/// `WhiteSpace3FileMergeDefault` both default to "Manual Choice"
/// (`e_SrcSelector::None`), so `MergeResultWindow::merge` skips
/// `updateDefaults` for whitespace blocks, and `RunRegExpAutoMergeOnMergeStart`
/// defaults to false. Both still run behind the explicit Auto-solve action, as
/// does the Low tier (history merge).
///
/// Reloads of an already stage-backed file keep user resolutions via
/// [`restore_conflict_session_resolutions`] and are never re-autosolved, so a
/// region the user deliberately un-resolved stays unresolved. A provisional
/// CurrentOnly session waits to run this policy until its Full upgrade.
fn auto_resolve_session_on_open(repo_state: &mut RepoState, path: &Path) {
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return;
    };
    if session.strategy != ConflictResolverStrategy::FullTextResolver {
        return;
    }
    let total_before = session.total_regions();
    let unresolved_before = session.unsolved_count();
    if unresolved_before == 0 {
        return;
    }

    let stats = super::conflict_interactions::apply_autosolve_to_session(
        session,
        ConflictAutosolveMode::Safe,
        false,
    );
    if stats.total_resolved() == 0 {
        return;
    }
    session.sync_merge_plan_from_regions();
    let unresolved_after = session.unsolved_count();
    let total_after = session.total_regions();

    super::util::push_action_log(
        repo_state,
        true,
        format!("telemetry.conflict_autosolve.on_open {}", path.display()),
        super::util::conflict_autosolve_telemetry_summary(
            ConflictAutosolveMode::Safe,
            Some(path),
            total_before,
            total_after,
            unresolved_before,
            unresolved_after,
            stats,
        ),
        None,
    );
}

fn restore_conflict_session_resolutions(existing: &ConflictSession, next: &mut ConflictSession) {
    if existing.path != next.path {
        return;
    }

    // Split/join rewrites the in-memory marker projection without touching the
    // worktree until Save. If Git stages are unchanged, keep that complete
    // structural projection across same-path watcher or explicit reloads.
    if existing.has_pending_structural_edits
        && existing.conflict_kind == next.conflict_kind
        && existing.strategy == next.strategy
        && existing.base == next.base
        && existing.ours == next.ours
        && existing.theirs == next.theirs
    {
        next.marker_projection = existing.marker_projection.clone();
        next.regions = existing.regions.clone();
        next.region_source_ranges = existing.region_source_ranges.clone();
        next.merge_plan = existing.merge_plan.clone();
        next.merge_plan_fallback = existing.merge_plan_fallback;
        next.region_plan_blocks = existing.region_plan_blocks.clone();
        next.has_pending_structural_edits = true;
        return;
    }

    let same_region =
        |left: &gitcomet_core::conflict_session::ConflictRegion,
         right: &gitcomet_core::conflict_session::ConflictRegion| {
            left.base == right.base && left.ours == right.ours && left.theirs == right.theirs
        };
    let existing_is_provisional = conflict_session_uses_provisional_stage_inputs(existing);
    let next_has_base_source = !next.base.is_absent();
    let matches_existing =
        |previous: &gitcomet_core::conflict_session::ConflictRegion,
         current: &gitcomet_core::conflict_session::ConflictRegion| {
            (previous.base == current.base || (existing_is_provisional && previous.base.is_none()))
                && previous.ours == current.ours
                && previous.theirs == current.theirs
        };

    // The common reload case is positionally identical. Preserve every
    // resolution, including duplicate-content regions, without ambiguity.
    if existing.regions.len() == next.regions.len()
        && existing
            .regions
            .iter()
            .zip(next.regions.iter())
            .all(|(previous, current)| matches_existing(previous, current))
    {
        for (previous, current) in existing.regions.iter().zip(next.regions.iter_mut()) {
            current.resolution =
                restored_region_resolution(previous, existing_is_provisional, next_has_base_source);
        }
        next.sync_merge_plan_from_regions();
        return;
    }

    next.restore_plan_decisions_from(existing);

    // When the region sequence changed, only restore identities that are
    // unique on both sides. This aligns insertions/deletions while avoiding
    // silently assigning the wrong resolution among indistinguishable
    // duplicate blocks.
    let next_unique: Vec<bool> = next
        .regions
        .iter()
        .map(|region| {
            next.regions
                .iter()
                .filter(|candidate| same_region(region, candidate))
                .take(2)
                .count()
                == 1
        })
        .collect();
    let mut cursor = 0usize;
    for (current_ix, current) in next.regions.iter_mut().enumerate() {
        if !next_unique[current_ix]
            || existing
                .regions
                .iter()
                .filter(|candidate| matches_existing(candidate, current))
                .take(2)
                .count()
                != 1
        {
            continue;
        }
        let Some(found) = existing.regions.get(cursor..).and_then(|remaining| {
            remaining
                .iter()
                .position(|previous| matches_existing(previous, current))
        }) else {
            continue;
        };
        current.resolution = restored_region_resolution(
            &existing.regions[cursor + found],
            existing_is_provisional,
            next_has_base_source,
        );
        cursor += found + 1;
    }
    restore_provisional_resolutions_by_source_overlap(existing, next);
    next.sync_merge_plan_from_regions();
}

fn source_ranges_overlap(left: &std::ops::Range<usize>, right: &std::ops::Range<usize>) -> bool {
    if left.is_empty() || right.is_empty() {
        return left.is_empty() && right.is_empty() && left.start == right.start;
    }
    left.start < right.end && right.start < left.end
}

fn conflict_ranges_overlap(
    previous: &ConflictRegionSourceRanges,
    current: &ConflictRegionSourceRanges,
) -> bool {
    source_ranges_overlap(&previous.ours, &current.ours)
        || source_ranges_overlap(&previous.theirs, &current.theirs)
}

fn source_backed_resolution(resolution: &ConflictRegionResolution) -> bool {
    matches!(
        resolution,
        ConflictRegionResolution::PickBase
            | ConflictRegionResolution::PickOurs
            | ConflictRegionResolution::PickTheirs
            | ConflictRegionResolution::PickBoth
            | ConflictRegionResolution::Sources(_)
    )
}

fn restore_provisional_resolutions_by_source_overlap(
    existing: &ConflictSession,
    next: &mut ConflictSession,
) {
    if !conflict_session_uses_provisional_stage_inputs(existing)
        || existing.region_source_ranges.len() != existing.regions.len()
        || next.region_source_ranges.len() != next.regions.len()
    {
        return;
    }
    let Some(marker_projection) = existing.marker_projection.as_deref() else {
        return;
    };
    let (projected_ours, projected_theirs) = reconstruct_conflict_marker_sides(marker_projection);
    let (Some(next_ours), Some(next_theirs)) = (next.ours.as_text(), next.theirs.as_text()) else {
        return;
    };
    if projected_ours != next_ours || projected_theirs != next_theirs {
        return;
    }

    let next_has_base_source = !next.base.is_absent();
    let restored: Vec<Option<ConflictRegionResolution>> = next
        .region_source_ranges
        .iter()
        .map(|current_ranges| {
            let mut candidates = existing
                .region_source_ranges
                .iter()
                .enumerate()
                .filter(|(_, previous_ranges)| {
                    conflict_ranges_overlap(previous_ranges, current_ranges)
                })
                .map(|(index, _)| &existing.regions[index]);
            let first = candidates.next()?;
            if !source_backed_resolution(&first.resolution) {
                return None;
            }
            let decision = restored_region_resolution(first, true, next_has_base_source);
            candidates
                .all(|region| {
                    source_backed_resolution(&region.resolution)
                        && restored_region_resolution(region, true, next_has_base_source)
                            == decision
                })
                .then_some(decision)
        })
        .collect();

    for (region, restored) in next.regions.iter_mut().zip(restored) {
        if matches!(region.resolution, ConflictRegionResolution::Unresolved)
            && let Some(restored) = restored
        {
            region.resolution = restored;
        }
    }
}

fn restored_region_resolution(
    previous: &gitcomet_core::conflict_session::ConflictRegion,
    existing_is_provisional: bool,
    next_has_base_source: bool,
) -> ConflictRegionResolution {
    let resolution = previous.resolution.clone();
    if !existing_is_provisional || previous.base.is_some() || !next_has_base_source {
        return resolution;
    }

    // A CurrentOnly two-way marker block numbers ours/theirs as A/B. A
    // full three-source session numbers base/ours/theirs as A/B/C, so carry
    // early ordered picks into the loaded session's source space.
    match resolution {
        ConflictRegionResolution::Sources(selection) => ConflictRegionResolution::Sources(
            OrderedSelection::from_sources(selection.iter().map(|source| match source {
                MergeSource::A => MergeSource::B,
                MergeSource::B | MergeSource::C => MergeSource::C,
            })),
        ),
        other => other,
    }
}

fn conflict_session_uses_provisional_stage_inputs(session: &ConflictSession) -> bool {
    session.strategy == gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver
        && session.base.is_absent()
        && session.ours.is_absent()
        && session.theirs.is_absent()
}

/// Build a `ConflictSession` from a loaded `ConflictFile` and the current repo status.
///
/// Looks up the `FileConflictKind` from the status entries. Full loads derive
/// text boundaries from immutable Git stages; CurrentOnly loads use a
/// provisional marker-backed session until those stages arrive.
fn build_conflict_session(
    repo_state: &crate::model::RepoState,
    file: &crate::model::ConflictFile,
) -> Option<ConflictSession> {
    // Look up the conflict kind from the repo's status entries.
    let conflict_kind = repo_state
        .worktree_status_entries()?
        .iter()
        .find(|e| e.path == file.path && e.kind == FileStatusKind::Conflicted)
        .and_then(|e| e.conflict)?;

    let base = ConflictPayload::from_stage_parts(file.base_bytes.clone(), file.base.clone());
    let ours = ConflictPayload::from_stage_parts(file.ours_bytes.clone(), file.ours.clone());
    let theirs = ConflictPayload::from_stage_parts(file.theirs_bytes.clone(), file.theirs.clone());

    let is_binary = base.is_binary() || ours.is_binary() || theirs.is_binary();
    let strategy = gitcomet_core::conflict_session::ConflictResolverStrategy::for_conflict(
        conflict_kind,
        is_binary,
    );

    if strategy == gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver
        && base.is_absent()
        && ours.is_absent()
        && theirs.is_absent()
    {
        // CurrentOnly intentionally omits the immutable stages. Build a
        // provisional session from the worktree markers so first-paint picks
        // have real regions; the Full upgrade replaces its inputs and retains
        // matching choices.
        file.current.as_ref().map(|current| {
            ConflictSession::from_merged_shared_text(
                file.path.to_path_buf(),
                conflict_kind,
                base,
                ours,
                theirs,
                current.clone(),
            )
        })
    } else if strategy
        == gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver
    {
        let current = file
            .current
            .as_ref()
            .map(|text| ConflictPayload::Text(text.clone()))
            .or_else(|| {
                file.current_bytes
                    .as_ref()
                    .map(|bytes| ConflictPayload::Binary(bytes.clone()))
            });
        Some(ConflictSession::from_stage_inputs_with_current(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
            current,
        ))
    } else if let Some(current) = file.current.as_ref() {
        Some(ConflictSession::from_merged_shared_text(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
            current.clone(),
        ))
    } else if let Some(current) = file.current_bytes.as_ref() {
        Some(ConflictSession::new_with_current(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
            ConflictPayload::Binary(current.clone()),
        ))
    } else {
        Some(ConflictSession::new(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
        ))
    }
}

pub(super) fn worktrees_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Worktree>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let worktrees = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_worktrees(worktrees);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::WORKTREES)
        {
            effects.push(Effect::LoadWorktrees { repo_id });
        }
    }
    effects
}

pub(super) fn worktree_dirty_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<WorktreeDirtySummary>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let worktree_dirty = match result {
            Ok(v) => Loadable::Ready(v),
            // A worktree that cannot be opened (removed, on an unmounted
            // volume) is a routine condition, not something worth a diagnostic
            // banner -- the scan simply reports nothing for it.
            Err(e) => Loadable::Error(e.to_string()),
        };
        repo_state.set_worktree_dirty(worktree_dirty);
        // A selected worktree row only exists while that worktree has changes.
        // Once it goes clean -- committed, stashed, reverted -- or drops out of a
        // failed scan, its row is gone, and a selection pointing at a row nothing
        // renders leaves the details pane with nothing to show and no way back.
        let selected_worktree_is_gone = repo_state
            .history_state
            .worktree_selection
            .as_ref()
            .is_some_and(|selected| match &repo_state.worktree_dirty {
                Loadable::Ready(dirty) => !dirty.iter().any(|summary| &summary.path == selected),
                _ => true,
            });
        if selected_worktree_is_gone {
            repo_state.set_worktree_selection(None);
        }
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::WORKTREE_DIRTY)
        {
            // Rebuilt rather than repeated: the selection may have moved while
            // the finished scan was running, and the repeat should carry the
            // file lists of whatever is selected now.
            effects.push(worktree_dirty_effect(repo_state));
        }
    }
    effects
}

pub(super) fn ref_metadata_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<(String, RefMetadata)>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let ref_metadata = match result {
            Ok(entries) => Loadable::Ready(entries.into_iter().collect()),
            // A backend that does not implement this will never implement it,
            // so latch an empty map rather than `Error` — callers retry on
            // `Error`, which would re-schedule a doomed load on every open.
            Err(e) if matches!(e.kind(), gitcomet_core::error::ErrorKind::Unsupported(_)) => {
                Loadable::Ready(std::collections::HashMap::new())
            }
            // Deliberately no diagnostic: this data only decorates picker rows,
            // which fall back to name-only. A transient failure must not raise
            // an error banner on every picker open.
            Err(e) => Loadable::Error(e.to_string()),
        };
        repo_state.set_ref_metadata(ref_metadata);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::REF_METADATA)
        {
            effects.push(Effect::LoadRefMetadata { repo_id });
        }
    }
    effects
}

pub(super) fn submodules_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Submodule>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let submodules = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                if matches!(e.kind(), gitcomet_core::error::ErrorKind::Cancelled) {
                    Loadable::NotLoaded
                } else {
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    Loadable::Error(e.to_string())
                }
            }
        };
        repo_state.set_submodules(submodules);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::SUBMODULES)
        {
            effects.push(Effect::LoadSubmodules { repo_id });
        }
    }
    effects
}

pub(super) fn select_commit(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
) -> Vec<Effect> {
    select_commit_multi(
        state,
        repo_id,
        commit_id,
        CommitSelectMode::Single,
        None,
        None,
    )
}

pub(super) fn select_commit_multi(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
    mode: CommitSelectMode,
    clicked_index: Option<usize>,
    visible_order: Option<Vec<CommitId>>,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    let log_rev = repo_state.history_state.log_rev;
    let mut sel = repo_state.history_state.multi_selection.clone();

    let focus = match mode {
        CommitSelectMode::Single => {
            collapse_multi_selection_to(&mut sel, commit_id.clone(), clicked_index, log_rev);
            commit_id
        }
        CommitSelectMode::Toggle => {
            if let Some(ix) = sel.commits.iter().position(|c| *c == commit_id) {
                sel.commits.remove(ix);
                let Some(focus) = sel.commits.last().cloned() else {
                    // Toggled the last commit away: clear the selection
                    // entirely (also dissolves the multi-selection).
                    repo_state.set_selected_commit(None);
                    repo_state.set_commit_details(Loadable::NotLoaded);
                    return Vec::new();
                };
                focus
            } else {
                sel.commits.push(commit_id.clone());
                sel.anchor = Some(commit_id.clone());
                sel.anchor_index = clicked_index;
                sel.anchor_log_rev = Some(log_rev);
                commit_id
            }
        }
        CommitSelectMode::Range => {
            let entries = visible_order.as_deref().unwrap_or(&[]);
            let clicked_ix = commit_selection_entry_index(entries, &commit_id, clicked_index);
            match clicked_ix {
                None => {
                    collapse_multi_selection_to(
                        &mut sel,
                        commit_id.clone(),
                        clicked_index,
                        log_rev,
                    );
                }
                Some(clicked_ix) => {
                    let anchor_ix = sel
                        .anchor
                        .as_ref()
                        .and_then(|anchor| {
                            let trusted_hint = sel
                                .anchor_index
                                .filter(|_| sel.anchor_log_rev == Some(log_rev));
                            commit_selection_entry_index(entries, anchor, trusted_hint)
                        })
                        .unwrap_or(clicked_ix);
                    let (a, b) = if anchor_ix <= clicked_ix {
                        (anchor_ix, clicked_ix)
                    } else {
                        (clicked_ix, anchor_ix)
                    };
                    sel.commits = entries[a..=b].to_vec();
                    if sel.anchor.is_none() {
                        sel.anchor = Some(commit_id.clone());
                    }
                    sel.anchor_index = Some(anchor_ix);
                    sel.anchor_log_rev = Some(log_rev);
                }
            }
            commit_id
        }
        CommitSelectMode::PreserveIfSelected => {
            // Keep an existing multi-selection intact when the clicked commit
            // is already part of it — only the focus moves. Otherwise collapse
            // to the clicked commit like a plain click.
            if !sel.commits.contains(&commit_id) {
                collapse_multi_selection_to(&mut sel, commit_id.clone(), clicked_index, log_rev);
            }
            commit_id
        }
    };

    repo_state.set_commit_multi_selection(sel);

    // Two or more selected commits enter "compare" mode: the details pane shows
    // the merged diff of the whole selection — every selected commit's own
    // changes, combined — instead of a plain list. A single commit (or a
    // selection that can't be resolved in the loaded log) falls back to the
    // single/multi-list behavior below.
    let range_pair = {
        let selected = &repo_state.history_state.multi_selection.commits;
        (selected.len() >= 2)
            .then(|| merged_selection_range(repo_state, selected))
            .flatten()
    };

    match range_pair {
        Some((from, to)) => {
            // Keep the focused commit selected (selection-derived UI stays
            // coherent) but don't load its details — the comparison view takes
            // over the details pane, so a single-commit detail load is wasted.
            // Leaving comparison mode is what reconciles the details pane again.
            repo_state.set_selected_commit(Some(focus));
            let from_label = range_endpoint_label(&from);
            let to_label = range_endpoint_label(&to);
            compare_range(
                state,
                repo_id,
                from,
                Some(to),
                from_label,
                to_label,
                ComparisonSource::MultiSelection,
            )
        }
        None => {
            let left_comparison = repo_state.clear_range_comparison();
            let mut effects = select_commit_and_load_details(repo_state, repo_id, focus);
            if left_comparison && effects.is_empty() {
                // `select_commit_and_load_details` no-ops when the focus is
                // already selected — exactly the case when collapsing a
                // comparison back to its focused commit, whose selection was
                // made without a details load. Only comparisons can leave that
                // gap, so re-selecting a commit otherwise stays a no-op.
                effects = reconcile_selected_commit_details(repo_state, repo_id);
            }
            effects
        }
    }
}

/// Emit a details load when the loaded commit details don't describe
/// `selected_commit`. Entering comparison mode deliberately moves the selection
/// without loading details, so every path that leaves comparison mode has to
/// reconcile — otherwise the pane keeps rendering the previously loaded commit's
/// message and file list under a different commit's selection.
fn reconcile_selected_commit_details(repo_state: &mut RepoState, repo_id: RepoId) -> Vec<Effect> {
    let Some(commit_id) = repo_state.history_state.selected_commit.clone() else {
        return Vec::new();
    };
    if matches!(
        &repo_state.history_state.commit_details,
        Loadable::Ready(details) if details.id == commit_id
    ) {
        return Vec::new();
    }
    repo_state.set_commit_details(Loadable::NotLoaded);
    vec![Effect::LoadCommitDetails { repo_id, commit_id }]
}

/// Endpoints for the merged diff of a multi-commit selection. `to` is the newest
/// selected commit; `from` is the *parent* of the oldest selected commit, so the
/// combined patch includes every selected commit's own changes — matching the
/// "merged diff of N commits" the comparison view presents. The history log is
/// newest-first, so the smallest index is the newest commit and the largest is
/// the oldest.
///
/// Falls back to the empty tree as `from` when the oldest selected commit is a
/// root commit (no parent), so the changes it introduces are part of the merged
/// diff like every other selected commit's — using the root itself as the base
/// would silently drop them. Returns `None` unless the log is loaded and every
/// selected commit resolves within it, so the caller leaves comparison mode
/// rather than guess.
fn merged_selection_range(
    repo_state: &RepoState,
    selected: &[CommitId],
) -> Option<(CommitId, CommitId)> {
    let Loadable::Ready(page) = &repo_state.history_state.log else {
        return None;
    };
    let mut newest_ix: Option<usize> = None;
    let mut oldest_ix: Option<usize> = None;
    for id in selected {
        let ix = page.commits.iter().position(|c| &c.id == id)?;
        newest_ix = Some(newest_ix.map_or(ix, |n: usize| n.min(ix)));
        oldest_ix = Some(oldest_ix.map_or(ix, |o: usize| o.max(ix)));
    }
    let newest = &page.commits[newest_ix?];
    let oldest = &page.commits[oldest_ix?];
    let from = oldest
        .parent_ids
        .first()
        .cloned()
        .unwrap_or_else(|| CommitId(EMPTY_TREE_ID.into()));
    Some((from, newest.id.clone()))
}

/// Label for a comparison endpoint in the UI/menus: an abbreviated commit id,
/// or a name for the empty-tree base, whose sha would be meaningless on screen.
fn range_endpoint_label(id: &CommitId) -> String {
    let full = id.as_ref();
    if full == EMPTY_TREE_ID {
        return "start of history".to_string();
    }
    full.get(..8).unwrap_or(full).to_string()
}

/// Where a comparison came from, which decides whether the multi-selection
/// describes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComparisonSource {
    /// The multi-selection *is* the comparison — its merged diff. The selection
    /// stays, and the UI names the comparison after it.
    MultiSelection,
    /// An explicit two-point compare: mark/compare, a branch, a tag, or the
    /// working tree. Any multi-selection left over from earlier clicks describes
    /// something else entirely, so it is dropped rather than left to mislabel
    /// the comparison and supply the wrong preview cards.
    Explicit,
}

/// Enter "compare two points" mode: record the ordered `from`/`to` pair and load
/// the changed-file list. A `to` of `None` compares `from` against the live
/// working tree. The diff pane is left empty (any prior selection is cleared) so
/// the comparison presents the file side-selection first — the user opens an
/// individual file's range diff by clicking it, rather than the whole range
/// patch opening automatically. Reused by multi-commit selection, the
/// mark/compare context-menu flow, and the compare-with-working-tree action.
pub(super) fn compare_range(
    state: &mut AppState,
    repo_id: RepoId,
    from: CommitId,
    to: Option<CommitId>,
    from_label: String,
    to_label: String,
    source: ComparisonSource,
) -> Vec<Effect> {
    let request = {
        let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        if source == ComparisonSource::Explicit {
            repo_state.set_commit_multi_selection(CommitMultiSelection::default());
        }
        repo_state.set_range_selection(Some(RangeSelection {
            from: from.clone(),
            to: to.clone(),
            from_label,
            to_label,
        }));
        repo_state.set_range_files(Loadable::Loading);
        repo_state.begin_range_files_load()
    };

    let mut effects = super::diff_selection::clear_diff_selection(state, repo_id);
    effects.push(Effect::LoadRangeFiles {
        repo_id,
        from,
        to,
        request,
    });
    effects
}

/// Dismiss an active range comparison: clear the selection, the file list, and
/// the range diff from the diff pane, then put the details pane back on the
/// commit that stays selected.
pub(super) fn clear_comparison(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let mut effects = {
        let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        repo_state.set_commit_multi_selection(CommitMultiSelection::default());
        repo_state.clear_range_comparison();
        // Entering the comparison moved `selected_commit` without loading its
        // details, so the pane would otherwise fall back to whichever commit's
        // details happened to be loaded last.
        reconcile_selected_commit_details(repo_state, repo_id)
    };
    effects.extend(super::diff_selection::clear_diff_selection(state, repo_id));
    effects
}

pub(super) fn mark_for_comparison(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
    label: String,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.comparison_mark = Some(crate::model::ComparisonMark { commit_id, label });
    }
    Vec::new()
}

pub(super) fn clear_comparison_mark(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.comparison_mark = None;
    }
    Vec::new()
}

/// Compare the marked point (base) against `commit_id` (tip). No-op when nothing
/// is marked or the mark equals the target.
pub(super) fn compare_with_marked(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
    label: String,
) -> Vec<Effect> {
    let mark = {
        let Some(repo_state) = state.repos.iter().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        match &repo_state.comparison_mark {
            Some(mark) if mark.commit_id != commit_id => mark.clone(),
            _ => return Vec::new(),
        }
    };
    compare_range(
        state,
        repo_id,
        mark.commit_id,
        Some(commit_id),
        mark.label,
        label,
        ComparisonSource::Explicit,
    )
}

pub(super) fn range_files_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    from: CommitId,
    to: Option<CommitId>,
    request: u64,
    result: std::result::Result<Vec<CommitFileChange>, Error>,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    // Only the newest issued load may land. A commit↔working-tree comparison
    // keeps the same `(from, to)` across every refresh, so that pair cannot tell
    // an overtaken reply from a current one — the request id can.
    if request != repo_state.history_state.range_files_request {
        return Vec::new();
    }
    repo_state.history_state.range_files_in_flight = false;

    // Two different guards, both needed. The id above rejects an *overtaken*
    // reply — one this repo did ask for, just not most recently. This one
    // rejects a reply that does not describe the comparison on screen at all,
    // whatever its id, so the list can never be filled from endpoints the user
    // is not looking at. See `range_files_loaded_populates_only_the_current_comparison`.
    let still_current = repo_state
        .history_state
        .range_selection
        .as_ref()
        .is_some_and(|range| range.from == from && range.to == to);
    if !still_current {
        repo_state.history_state.range_files_refresh_queued = false;
        return Vec::new();
    }

    let next = match result {
        Ok(files) => Loadable::Ready(Arc::new(files)),
        Err(e) => {
            push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
            Loadable::Error(e.to_string())
        }
    };
    repo_state.set_range_files(next);

    // The worktree moved again while this load was running; run one more so the
    // list ends up describing the final state rather than the state mid-flight.
    if !std::mem::take(&mut repo_state.history_state.range_files_refresh_queued) {
        return Vec::new();
    }
    vec![Effect::LoadRangeFiles {
        repo_id,
        from,
        to,
        request: repo_state.begin_range_files_load(),
    }]
}

fn collapse_multi_selection_to(
    sel: &mut crate::model::CommitMultiSelection,
    commit_id: CommitId,
    clicked_index: Option<usize>,
    log_rev: u64,
) {
    sel.commits.clear();
    sel.commits.push(commit_id.clone());
    sel.anchor = Some(commit_id);
    sel.anchor_index = clicked_index;
    sel.anchor_log_rev = Some(log_rev);
}

/// Resolves `target`'s index in `entries`, preferring the index hint when it
/// still points at the target.
fn commit_selection_entry_index(
    entries: &[CommitId],
    target: &CommitId,
    index_hint: Option<usize>,
) -> Option<usize> {
    index_hint
        .filter(|&ix| entries.get(ix) == Some(target))
        .or_else(|| entries.iter().position(|id| id == target))
}

pub(super) fn select_commit_and_load_details(
    repo_state: &mut RepoState,
    repo_id: RepoId,
    commit_id: CommitId,
) -> Vec<Effect> {
    if repo_state.history_state.selected_commit.as_ref() == Some(&commit_id) {
        return Vec::new();
    }

    repo_state.set_selected_commit(Some(commit_id.clone()));
    let already_loaded = matches!(
        &repo_state.history_state.commit_details,
        Loadable::Ready(details) if details.id == commit_id
    );
    if already_loaded {
        return Vec::new();
    }

    if matches!(
        repo_state.history_state.commit_details,
        Loadable::Error(_) | Loadable::NotLoaded
    ) {
        repo_state.set_commit_details(Loadable::NotLoaded);
    }
    vec![Effect::LoadCommitDetails { repo_id, commit_id }]
}

pub(super) fn select_worktree_uncommitted(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    // Whatever this displaces -- another worktree's open diff, say -- is retired
    // by `retire_orphaned_worktree_diffs` once the reducer settles.
    repo_state.set_worktree_selection(Some(path));
    repo_state.set_commit_details(Loadable::NotLoaded);

    // Only the selected worktree's changed files are carried in state, so the row
    // that was just selected needs a scan to fetch its own. The counts are already
    // on screen and stay there while it runs.
    request_worktree_dirty_effect(repo_state)
        .into_iter()
        .collect()
}

/// Retires an inline diff belonging to a linked worktree that is no longer the
/// selected one.
///
/// The diff pane renders an inline foreign diff in preference to the diff target,
/// so one whose worktree row is gone keeps another checkout's file -- and its
/// origin chip -- on screen with no row left to deselect it. A worktree selection
/// ends in more ways than it begins: switching worktrees, selecting any commit
/// (`set_selected_commit` clears it as a side effect), clearing the selection, and
/// a scan that no longer lists the worktree. Rather than remember all four, this
/// runs once after every message and states the invariant directly.
///
/// Submodule-origin inline diffs are untouched: they never had a worktree row.
pub(super) fn retire_orphaned_worktree_diffs(state: &mut AppState) {
    for repo_state in &mut state.repos {
        let selected = repo_state.history_state.worktree_selection.as_deref();
        let orphaned = repo_state
            .diff_state
            .inline_submodule_diff
            .as_ref()
            .is_some_and(|inline| {
                matches!(inline.origin, ForeignDiffOrigin::Worktree { .. })
                    && Some(inline.submodule_repo_path.as_path()) != selected
            });
        if !orphaned {
            continue;
        }

        // Exactly what `CloseInlineSubmoduleDiff` clears, and no more. The inline
        // diff carries its own `diff`/`diff_file`/`diff_file_image` inside
        // `InlineSubmoduleDiffState`, so dropping it drops every loadable it ever
        // owned. `diff_target` and the diff-state loadables beside it belong to
        // the commit or working-tree file selected *behind* the inline diff --
        // opening one never touched them -- and the pane falls back to that file
        // once the inline diff is gone. Clearing them here blanked the pane
        // instead.
        repo_state.diff_state.inline_submodule_diff = None;
        repo_state.bump_diff_state_rev();
    }
}

pub(super) fn clear_commit_selection(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    repo_state.set_selected_commit(None);
    repo_state.set_commit_details(Loadable::NotLoaded);
    Vec::new()
}

pub(super) fn append_ensure_sidebar_data_effects(
    repo_state: &mut RepoState,
    effects: &mut impl EffectAccumulator,
) {
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return;
    }

    let repo_id = repo_state.id;
    let request = repo_state.sidebar_data_request;

    if request.worktrees && matches!(repo_state.worktrees, Loadable::NotLoaded) {
        repo_state.set_worktrees(Loadable::Loading);
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::WORKTREES)
        {
            effects.push_effect(Effect::LoadWorktrees { repo_id });
        }
    }

    if request.submodules && matches!(repo_state.submodules, Loadable::NotLoaded) {
        repo_state.set_submodules(Loadable::Loading);
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::SUBMODULES)
        {
            effects.push_effect(Effect::LoadSubmodules { repo_id });
        }
    }

    if request.stashes && matches!(repo_state.stashes, Loadable::NotLoaded) {
        repo_state.set_stashes(Loadable::Loading);
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::STASHES)
        {
            effects.push_effect(Effect::LoadStashes { repo_id, limit: 50 });
        }
    }
}

pub(super) fn ensure_sidebar_data(
    state: &mut AppState,
    repo_id: RepoId,
    request: SidebarDataRequest,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    repo_state.set_sidebar_data_request(request);
    let mut effects = Vec::new();
    append_ensure_sidebar_data_effects(repo_state, &mut effects);
    effects
}

pub(super) fn load_stashes(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_stashes(Loadable::Loading);
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::STASHES)
    {
        vec![Effect::LoadStashes { repo_id, limit: 50 }]
    } else {
        Vec::new()
    }
}

pub(super) fn refresh_branches(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::BRANCHES)
    {
        vec![Effect::LoadBranches { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_tags(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_tags(Loadable::Loading);
    if repo_state.loads_in_flight.request(RepoLoadsInFlight::TAGS) {
        vec![Effect::LoadTags { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_remote_tags(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_remote_tags(Loadable::Loading);
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::REMOTE_TAGS)
    {
        vec![Effect::LoadRemoteTags { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_conflict_file(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    mode: ConflictFileLoadMode,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let same_path = repo_state.conflict_state.conflict_file_path.as_ref() == Some(&path);
    repo_state.set_conflict_file_path(Some(path.clone()));
    super::util::reset_conflict_target_reload_state(repo_state, mode, same_path);
    vec![Effect::LoadConflictFile {
        repo_id,
        path,
        mode,
    }]
}

pub(super) fn load_reflog(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.reflog = Loadable::Loading;
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::REFLOG)
    {
        vec![Effect::LoadReflog {
            repo_id,
            limit: 200,
        }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_hover_commit_message(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    // Already showing or fetching this commit: hovering the same row again must
    // not re-issue the read.
    if repo_state
        .hover_commit_message
        .as_ref()
        .is_some_and(|(id, state)| *id == commit_id && !matches!(state, Loadable::Error(_)))
    {
        return Vec::new();
    }
    repo_state.set_hover_commit_message(commit_id.clone(), Loadable::Loading);
    vec![Effect::LoadHoverCommitMessage { repo_id, commit_id }]
}

pub(super) fn hover_commit_message_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
    result: std::result::Result<String, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        // A result for a commit the pointer has already left is stale.
        && repo_state
            .hover_commit_message
            .as_ref()
            .is_some_and(|(id, _)| *id == commit_id)
    {
        let value = match result {
            Ok(message) => Loadable::Ready(Arc::from(message.as_str())),
            // Deliberately not a diagnostic: a hover that loses its race with a
            // background fetch is not something to tell the user about.
            Err(e) => Loadable::Error(e.to_string()),
        };
        repo_state.set_hover_commit_message(commit_id, value);
    }
    Vec::new()
}

pub(super) fn load_recent_commit_messages(
    state: &mut AppState,
    repo_id: RepoId,
    limit: usize,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(()))
        || matches!(repo_state.recent_commit_messages, Loadable::Loading)
    {
        return Vec::new();
    }
    repo_state.set_recent_commit_messages(Loadable::Loading);
    let request_rev = repo_state.recent_commit_messages_rev;
    vec![Effect::LoadRecentCommitMessages {
        repo_id,
        limit,
        request_rev,
    }]
}

pub(super) fn recent_commit_messages_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    request_rev: u64,
    result: std::result::Result<Vec<RecentCommitMessage>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.recent_commit_messages_rev == request_rev
    {
        let value = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_recent_commit_messages(value);
    }
    Vec::new()
}

pub(super) fn load_file_history(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    limit: usize,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.history_state.file_history_path = Some(path.clone());
    repo_state.history_state.file_history = Loadable::Loading;
    vec![Effect::LoadFileHistory {
        repo_id,
        path,
        limit,
    }]
}

pub(super) fn load_blame(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    source: gitcomet_core::domain::BlameSource,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    // The view dispatches this from `MainPaneView::render` against an
    // asynchronously pushed `AppState` snapshot, and `AppStore::dispatch` is a
    // channel send, so several frames can ask for the same blame before the
    // `Loading` snapshot reaches the view. Without this guard each of those
    // frames forks another `git blame --line-porcelain` for the same file.
    // `blame_path` + `blame_source` identify the request exactly, which a
    // repo-wide `RepoLoadsInFlight` bit could not.
    let same_target = repo_state.history_state.blame_path.as_ref() == Some(&path)
        && repo_state.history_state.blame_source.as_ref() == Some(&source);
    if same_target && repo_state.history_state.blame.is_loading() {
        return Vec::new();
    }
    if same_target {
        // Reloading the same file: keep the current annotations painted until
        // the new ones land.
        repo_state.retain_blame_while_loading();
    } else {
        // Re-targeting: anything held over describes a different file.
        repo_state.clear_retained_blame();
    }
    repo_state.history_state.blame_path = Some(path.clone());
    repo_state.history_state.blame_source = Some(source.clone());
    repo_state.history_state.blame = Loadable::Loading;
    vec![Effect::LoadBlame {
        repo_id,
        path,
        source,
    }]
}

pub(super) fn load_worktrees(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_worktrees(Loadable::Loading);
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREES)
    {
        vec![Effect::LoadWorktrees { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_worktree_dirty(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    // Unlike the other loaders this one does not flip to `Loading`: the counts
    // stay on screen while a rescan runs, so a window-focus refresh does not
    // blank the rows it is about to redraw identically.
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREE_DIRTY)
    {
        vec![worktree_dirty_effect(repo_state)]
    } else {
        Vec::new()
    }
}

/// Queues a rescan of the other worktrees' uncommitted changes, if one is not
/// already running. Returns `None` when a scan is in flight, so callers can
/// fire this from several triggers without stacking up repeated full scans.
///
/// The watcher-driven trigger fires on every git-state flush, and a full scan
/// runs `status` on every other worktree, so what bounds the cost is worth
/// spelling out. First, what does *not* reach here: `.git/index` is classified
/// as `RepoExternalChange::Index`, not `git_state` (`repo_monitor.rs`,
/// `is_git_index_path`), so the common edit-stage-unstage loop -- which writes
/// nothing else -- costs no scan at all. A linked worktree's own index sits at
/// `.git/worktrees/<name>/index` and is deliberately outside that test, so
/// changes there do still arrive as git-state and do still earn a scan.
/// Then, for what does reach here: the monitor debounces raw events at 250ms
/// with a 2s ceiling
/// (`repo_monitor.rs`), and `request` admits at most one scan in flight plus one
/// queued. A storm therefore costs one scan at a time, never a growing queue,
/// and always ends with one trailing scan — dropping the queued repeat instead
/// would be cheaper but could leave the counts stale after the last event.
/// There is deliberately no time-based throttle here: this reducer has no clock,
/// and the ones that do (window focus, `view/mod.rs`) ride their own.
pub(super) fn request_worktree_dirty_effect(repo_state: &mut RepoState) -> Option<Effect> {
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return None;
    }
    repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREE_DIRTY)
        .then(|| worktree_dirty_effect(repo_state))
}

/// The scan effect, aimed at whichever worktree row is selected.
///
/// Built in one place so every trigger -- watcher flush, window focus, selecting
/// a row -- asks for the file lists of the worktree that is actually on screen,
/// and for counts alone everywhere else.
pub(super) fn worktree_dirty_effect(repo_state: &RepoState) -> Effect {
    Effect::LoadWorktreeDirty {
        repo_id: repo_state.id,
        workdir: repo_state.spec.workdir.clone(),
        files_for: repo_state.history_state.worktree_selection.clone(),
    }
}

pub(super) fn load_ref_metadata(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_ref_metadata(Loadable::Loading);
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::REF_METADATA)
    {
        vec![Effect::LoadRefMetadata { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_submodules(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_submodules(Loadable::Loading);
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::SUBMODULES)
    {
        vec![Effect::LoadSubmodules { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_file_browser(
    state: &mut AppState,
    repo_id: RepoId,
    source: FileSource,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.file_browser.source = source.clone();
    repo_state.file_browser.entries = Loadable::Loading;
    repo_state.file_browser.bump_rev();
    vec![Effect::LoadFileBrowser { repo_id, source }]
}

/// Expand every directory on the way to `path` so the file explorer can show it.
///
/// Also clears the search query: the filtered view builds its rows from matches
/// and force-expands their ancestors, ignoring `expanded_dirs` entirely, so a
/// reveal into a filtered tree would scroll to a row index that does not mean
/// what the caller computed.
pub(super) fn reveal_file_browser_path(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    // `ancestors()` yields the path itself first — skip it, a file is not a
    // directory to expand — and stops before the empty root component.
    for ancestor in path.ancestors().skip(1) {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        repo_state
            .file_browser
            .expanded_dirs
            .insert(Arc::new(ancestor.to_path_buf()));
    }
    if !repo_state.file_browser.search_query.is_empty() {
        repo_state.file_browser.search_query.clear();
    }
    repo_state.file_browser.bump_rev();
    Vec::new()
}

pub(super) fn toggle_file_browser_dir(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let path = Arc::new(path);
        if repo_state.file_browser.expanded_dirs.contains(&path) {
            repo_state.file_browser.expanded_dirs.remove(&path);
        } else {
            repo_state.file_browser.expanded_dirs.insert(path);
        }
        repo_state.file_browser.bump_rev();
    }
    Vec::new()
}

pub(super) fn set_file_browser_search(
    state: &mut AppState,
    repo_id: RepoId,
    query: String,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.file_browser.search_query != query
    {
        repo_state.file_browser.search_query = query;
        repo_state.file_browser.bump_rev();
    }
    Vec::new()
}

pub(super) fn set_file_browser_source(
    state: &mut AppState,
    repo_id: RepoId,
    source: FileSource,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.file_browser.source != source
    {
        repo_state.file_browser.source = source.clone();
        repo_state.file_browser.entries = Loadable::NotLoaded;
        repo_state.file_browser.expanded_dirs.clear();
        repo_state.file_browser.search_query.clear();
        repo_state.file_browser.bump_rev();
        return vec![Effect::LoadFileBrowser { repo_id, source }];
    }
    Vec::new()
}

pub(super) fn set_sidebar_mode(state: &mut AppState, mode: SidebarMode) -> Vec<Effect> {
    if state.sidebar_mode != mode {
        state.sidebar_mode = mode;

        if mode == SidebarMode::Files
            && let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id)
            && matches!(
                repo.file_browser.entries,
                Loadable::NotLoaded | Loadable::Error(_)
            )
        {
            let source = repo.file_browser.source.clone();
            return vec![Effect::LoadFileBrowser { repo_id, source }];
        }
    }
    Vec::new()
}

pub(super) fn browse_repository_at_commit(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
) -> Vec<Effect> {
    const BROWSE_HISTORY_CAP: usize = 32;
    // Capture the open file (if any) before re-targeting it to the new point.
    let reopen_path = browse_open_content_path(state, repo_id);
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && !repo_state.browse_history.contains(&commit_id)
    {
        repo_state.browse_history.push(commit_id.clone());
        if repo_state.browse_history.len() > BROWSE_HISTORY_CAP {
            repo_state.browse_history.remove(0);
        }
    }
    state.sidebar_mode = SidebarMode::Files;
    let mut effects =
        set_file_browser_source(state, repo_id, FileSource::Commit(commit_id.clone()));
    if let Some(path) = reopen_path
        && effects
            .iter()
            .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
    {
        effects.extend(super::diff_selection::open_file_content(
            state,
            repo_id,
            FileSource::Commit(commit_id),
            path,
        ));
    }
    effects
}

pub(super) fn reset_browse_to_live(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let reopen_path = browse_open_content_path(state, repo_id);
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.browse_history.clear();
    }
    let mut effects = set_file_browser_source(state, repo_id, FileSource::WorkingDirectory);
    if let Some(path) = reopen_path
        && effects
            .iter()
            .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
    {
        effects.extend(super::diff_selection::open_file_content(
            state,
            repo_id,
            FileSource::WorkingDirectory,
            path,
        ));
    }
    effects
}

/// Path of the file currently shown as full content (if any), so a browse-point
/// change can re-open the same file at the new point.
fn browse_open_content_path(state: &AppState, repo_id: RepoId) -> Option<std::path::PathBuf> {
    let repo = state.repos.iter().find(|r| r.id == repo_id)?;
    if !repo.diff_state.content_preview {
        return None;
    }
    match &repo.diff_state.diff_target {
        Some(gitcomet_core::domain::DiffTarget::Commit { path: Some(p), .. }) => Some(p.clone()),
        Some(gitcomet_core::domain::DiffTarget::WorkingTree { path, .. }) => Some(path.clone()),
        _ => None,
    }
}

pub(super) fn file_browser_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    source: FileSource,
    result: std::result::Result<Vec<FileEntry>, gitcomet_core::error::Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        if repo_state.file_browser.source != source {
            return Vec::new();
        }
        repo_state.file_browser.entries = match result {
            Ok(v) => Loadable::Ready(Arc::new(v)),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.file_browser.bump_rev();
    }
    Vec::new()
}

pub(super) fn branches_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Branch>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let branches = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_branches(branches);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::BRANCHES)
        {
            effects.push(Effect::LoadBranches { repo_id });
        }
    }
    effects
}

pub(super) fn remotes_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Remote>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let remotes = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_remotes(remotes);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::REMOTES)
        {
            effects.push(Effect::LoadRemotes { repo_id });
        }
    }
    effects
}

pub(super) fn remote_branches_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<RemoteBranch>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let branches = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_remote_branches(branches);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::REMOTE_BRANCHES)
        {
            effects.push(Effect::LoadRemoteBranches { repo_id });
        }
    }
    effects
}

pub(super) fn status_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<RepoStatus, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        match result {
            Ok(next) => {
                let status_unchanged = matches!(
                    &repo_state.status,
                    Loadable::Ready(prev) if prev.as_ref() == &next
                );
                if !status_unchanged {
                    repo_state.set_status(Loadable::Ready(Arc::new(next)));
                }
                clear_resolved_conflict_context(repo_state);
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.set_status(Loadable::Error(e.to_string()));
            }
        }
        finish_status_lane_replay(
            repo_state,
            RepoLoadsInFlight::WORKTREE_STATUS,
            Effect::LoadWorktreeStatus { repo_id },
            &mut effects,
        );
        finish_status_lane_replay(
            repo_state,
            RepoLoadsInFlight::STAGED_STATUS,
            Effect::LoadStagedStatus { repo_id },
            &mut effects,
        );
    }
    effects
}

pub(super) fn worktree_status_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<gitcomet_core::domain::FileStatus>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        match result {
            Ok(next) => {
                let status_unchanged = matches!(&repo_state.worktree_status, Loadable::Ready(prev) if prev.as_slice() == next.as_slice());
                if !status_unchanged {
                    repo_state.set_worktree_status(Loadable::Ready(next));
                }
                clear_resolved_conflict_context(repo_state);
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.set_worktree_status(Loadable::Error(e.to_string()));
            }
        }
        finish_status_lane_replay(
            repo_state,
            RepoLoadsInFlight::WORKTREE_STATUS,
            Effect::LoadWorktreeStatus { repo_id },
            &mut effects,
        );
    }
    effects
}

pub(super) fn staged_status_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<gitcomet_core::domain::FileStatus>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        match result {
            Ok(next) => {
                let status_unchanged = matches!(&repo_state.staged_status, Loadable::Ready(prev) if prev.as_slice() == next.as_slice());
                if !status_unchanged {
                    repo_state.set_staged_status(Loadable::Ready(next));
                }
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.set_staged_status(Loadable::Error(e.to_string()));
            }
        }
        finish_status_lane_replay(
            repo_state,
            RepoLoadsInFlight::STAGED_STATUS,
            Effect::LoadStagedStatus { repo_id },
            &mut effects,
        );
    }
    effects
}

fn finish_status_lane_replay(
    repo_state: &mut crate::model::RepoState,
    flag: u32,
    replay_effect: Effect,
    effects: &mut Vec<Effect>,
) {
    // A pending request means a refresh was coalesced while this load was in flight — a genuine
    // external change or a just-completed action. Always replay it, even when the loaded payload
    // matches what is currently displayed: the in-flight load may have read the working tree or
    // index just *before* the change landed, so the coalesced refresh is the only chance to
    // observe it. Suppressing it on an unchanged payload (as a previous revision did) drops real
    // external changes and leaves stale entries in the uncommitted view.
    //
    // This cannot self-sustain a refresh loop: status reads are read-only (the gix backend's
    // `maybe_persist_*` helpers never rewrite `.git/index`, and worktree reads emit only ignored
    // `Access` events), so a completed status load never manufactures the filesystem event that
    // would set `pending` again.
    if repo_state.loads_in_flight.finish(flag) {
        effects.push(replay_effect);
    }
}

/// Clear conflict-file/session state when the tracked conflict path is no longer
/// present as an unresolved conflict in status.
fn clear_resolved_conflict_context(repo_state: &mut crate::model::RepoState) {
    let Some(conflict_path) = repo_state.conflict_state.conflict_file_path.as_ref() else {
        return;
    };
    let still_conflicted = repo_state.worktree_status_entries().is_none_or(|status| {
        status
            .iter()
            .any(|entry| entry.path == *conflict_path && entry.kind == FileStatusKind::Conflicted)
    });
    if still_conflicted {
        return;
    }

    repo_state.set_conflict_file_path(None);
    repo_state.set_conflict_file_load_mode(ConflictFileLoadMode::CurrentOnly);
    repo_state.set_conflict_file(Loadable::NotLoaded);
    repo_state.conflict_state.session_pending_restore = None;
    repo_state.set_conflict_session(None);
    repo_state.set_conflict_hide_resolved(false);
}

pub(super) fn head_branch_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<String, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let head_branch = match result {
            Ok(v) => {
                if v == "HEAD" {
                    if repo_state.detached_head_commit.is_none()
                        && repo_state
                            .history_state
                            .history_scope
                            .guarantees_head_visibility()
                        && let Loadable::Ready(page) = &repo_state.log
                    {
                        repo_state
                            .set_detached_head_commit(page.commits.first().map(|c| c.id.clone()));
                    }
                } else {
                    repo_state.set_detached_head_commit(None);
                }
                Loadable::Ready(v)
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_head_branch(head_branch);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::HEAD_BRANCH)
        {
            effects.push(Effect::LoadHeadBranch { repo_id });
        }
    }
    effects
}

pub(super) fn upstream_divergence_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Option<UpstreamDivergence>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let value = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_upstream_divergence(value);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::UPSTREAM_DIVERGENCE)
        {
            effects.push(Effect::LoadUpstreamDivergence { repo_id });
        }
    }
    effects
}

pub(super) fn tags_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Tag>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let tags = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                if matches!(e.kind(), gitcomet_core::error::ErrorKind::Unsupported(_)) {
                    Loadable::Ready(Vec::new())
                } else if matches!(e.kind(), gitcomet_core::error::ErrorKind::Cancelled) {
                    Loadable::NotLoaded
                } else {
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    Loadable::Error(e.to_string())
                }
            }
        };
        repo_state.set_tags(tags);
        if repo_state.loads_in_flight.finish(RepoLoadsInFlight::TAGS) {
            effects.push(Effect::LoadTags { repo_id });
        }
    }
    effects
}

pub(super) fn remote_tags_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<RemoteTag>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let remote_tags = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                if matches!(e.kind(), gitcomet_core::error::ErrorKind::Unsupported(_)) {
                    Loadable::Ready(Vec::new())
                } else if matches!(e.kind(), gitcomet_core::error::ErrorKind::Cancelled) {
                    Loadable::NotLoaded
                } else {
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    Loadable::Error(e.to_string())
                }
            }
        };
        repo_state.set_remote_tags(remote_tags);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::REMOTE_TAGS)
        {
            effects.push(Effect::LoadRemoteTags { repo_id });
        }
    }
    effects
}

pub(super) fn stashes_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<StashEntry>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let stashes = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_stashes(stashes);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::STASHES)
        {
            effects.push(Effect::LoadStashes { repo_id, limit: 50 });
        }
    }
    effects
}

pub(super) fn reflog_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<ReflogEntry>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.reflog = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        if repo_state.loads_in_flight.finish(RepoLoadsInFlight::REFLOG) {
            effects.push(Effect::LoadReflog {
                repo_id,
                limit: 200,
            });
        }
    }
    effects
}

/// Validates the current multi-selection against the loaded log and HEAD.
/// This is the single reducer-side gate for every squash entry point.
pub(super) fn squash_plan_for_repo(
    repo_state: &RepoState,
) -> Option<gitcomet_core::squash::SquashPlan> {
    let Loadable::Ready(page) = &repo_state.log else {
        return None;
    };
    let head = repo_state.head_commit_id()?;
    gitcomet_core::squash::squash_eligibility(
        &page.commits,
        &repo_state.history_state.multi_selection.commits,
        &head,
    )
}

pub(super) fn prepare_squash(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let Some(plan) = squash_plan_for_repo(repo_state) else {
        repo_state.history_state.squash_preview_pending = None;
        repo_state.set_squash_preview(Loadable::NotLoaded);
        return Vec::new();
    };

    repo_state.history_state.squash_preview_pending =
        Some((plan.oldest.clone(), plan.head.clone()));
    repo_state.set_squash_preview(Loadable::Loading);
    vec![Effect::LoadSquashMessagePreview {
        repo_id,
        oldest: plan.oldest,
        head: plan.head,
    }]
}

pub(super) fn squash_message_preview_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    oldest: CommitId,
    head: CommitId,
    result: std::result::Result<String, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        // Accept the result only if it still matches the range we last asked
        // for. Keying off the recorded request (not the live plan) means a
        // transiently-invalid plan — e.g. HEAD momentarily unresolved during a
        // concurrent reload — does not drop the result and strand the preview
        // on Loading forever.
        let matches_request = repo_state.history_state.squash_preview_pending.as_ref()
            == Some(&(oldest.clone(), head.clone()));
        if matches_request {
            repo_state.history_state.squash_preview_pending = None;
            let value = match result {
                Ok(message) => {
                    let (subject, body) = gitcomet_core::squash::split_subject_body(&message);
                    Loadable::Ready(crate::model::SquashPreview {
                        oldest,
                        head,
                        subject,
                        body,
                    })
                }
                Err(e) => {
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    Loadable::Error(e.to_string())
                }
            };
            repo_state.set_squash_preview(value);
        }
    }
    Vec::new()
}

pub(super) fn squash_rebase_setup_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    base: String,
    actual_head: CommitId,
    selected_ids: Vec<CommitId>,
    reword_id: CommitId,
    message: String,
    count: usize,
    result: std::result::Result<Vec<InteractiveRebaseEntry>, Error>,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    let entries = match result {
        Ok(entries) => entries,
        Err(e) => {
            push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
            push_notification(
                state,
                AppNotificationKind::Error,
                format!("Failed to load commits for squash rebase: {e}"),
            );
            return Vec::new();
        }
    };

    let selected_strs: HashSet<&str> = selected_ids.iter().map(|id| id.as_ref()).collect();

    // The list loaded asynchronously, so re-validate it against the plan the
    // user confirmed before rewriting history. `git log --reverse base..HEAD`
    // yields commits oldest-first, so the last entry is the live HEAD.
    let head_unchanged = entries
        .last()
        .is_some_and(|e| e.commit_id == actual_head.as_ref());

    let mut matched = 0usize;
    let mut reword_found = false;
    let todo: Vec<InteractiveRebaseEntry> = entries
        .into_iter()
        .map(|mut entry| {
            if entry.commit_id == reword_id.as_ref() {
                entry.action = InteractiveRebaseAction::Reword;
                entry.new_message = Some(message.clone());
                reword_found = true;
                matched += 1;
            } else if selected_strs.contains(entry.commit_id.as_str()) {
                entry.action = InteractiveRebaseAction::Fixup;
                matched += 1;
            }
            entry
        })
        .collect();

    // Every selected commit must appear exactly once in the live range and the
    // oldest must have become the reword anchor; otherwise HEAD moved or the
    // range drifted between confirmation and now, and rewriting would either be
    // a silent no-op or touch the wrong commits. `matched == count` also
    // catches a selection count that disagrees with what was actually planned.
    if !head_unchanged || !reword_found || matched != count {
        push_notification(
            state,
            AppNotificationKind::Warning,
            "Squash cancelled: the selected commits are no longer squashable.".to_string(),
        );
        return Vec::new();
    }

    super::begin_local_action(state, repo_id);
    vec![Effect::InteractiveRebase {
        repo_id,
        base,
        entries: todo,
        // Automated squash rebase — no editor window; reports as "Rebase".
        interactive: false,
    }]
}

/// Start revealing a commit referenced from elsewhere.
///
/// The reference is remembered and resolved off-thread. Selecting only happens
/// once it resolves, so a reference that turns out to be a build id or a Gerrit
/// change id never sends the log walking.
pub(super) fn reveal_commit(
    state: &mut AppState,
    repo_id: RepoId,
    reference: CommitId,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.set_reveal_target(Some(reference.clone()));
    vec![Effect::ResolveCommitForReveal { repo_id, reference }]
}

pub(super) fn finish_commit_reveal(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.set_reveal_target(None);
    }
    Vec::new()
}

pub(super) fn commit_reveal_resolved(
    state: &mut AppState,
    repo_id: RepoId,
    reference: CommitId,
    result: std::result::Result<CommitDetails, Error>,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    // A reply for a reveal the user has already left behind.
    if repo_state.history_state.reveal_target.as_ref() != Some(&reference) {
        return Vec::new();
    }

    let details = match result {
        Ok(details) => details,
        Err(e) => {
            repo_state.set_reveal_target(None);
            push_notification(
                state,
                crate::model::AppNotificationKind::Warning,
                format!("Could not find commit {reference}: {e}"),
            );
            return Vec::new();
        }
    };

    // Publish the details before selecting: the selection path then sees them
    // already loaded and does not ask git for the same commit twice.
    let commit_id = details.id.clone();
    repo_state.set_reveal_target(Some(commit_id.clone()));
    repo_state.set_commit_details(Loadable::Ready(Arc::new(details)));
    select_commit(state, repo_id, commit_id)
}

pub(super) fn commit_details_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
    result: std::result::Result<CommitDetails, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.history_state.selected_commit.as_ref() == Some(&commit_id)
    {
        let selected_target = repo_state.diff_state.diff_target.clone();
        let previous_plan = selected_target
            .as_ref()
            .map(|target| selected_diff_load_plan(repo_state, target));
        let value = match result {
            Ok(v) => Loadable::Ready(Arc::new(v)),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_commit_details(value);

        if let Some(target @ gitcomet_core::domain::DiffTarget::Commit { .. }) = selected_target {
            let next_plan = selected_diff_load_plan(repo_state, &target);
            if previous_plan != Some(next_plan) {
                apply_selected_diff_load_plan_state(repo_state, next_plan);
                repo_state.bump_diff_state_rev();
                return diff_reload_effects(repo_state, repo_id, target);
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConflictFile, RepoState, SidebarDataRequest, SidebarMode};
    use gitcomet_core::domain::{
        DiffArea, DiffTarget, FileConflictKind, FileEntry, FileEntryKind, FileSource, FileStatus,
        LogScope, RepoSpec,
    };
    use gitcomet_core::error::{Error, ErrorKind};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    fn backend_error(message: &str) -> Error {
        Error::new(ErrorKind::Backend(message.to_string()))
    }

    fn unsupported_error() -> Error {
        Error::new(ErrorKind::Unsupported("unsupported"))
    }

    fn empty_log_page() -> LogPage {
        LogPage {
            commits: Vec::new(),
            next_cursor: None,
        }
    }

    fn commit_details_for(id: CommitId) -> CommitDetails {
        CommitDetails {
            id,
            message: "message".to_string(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at_unix: 0,
            committed_at: "now".to_string(),
            committed_at_unix: 0,
            parent_ids: Vec::new(),
            files: Vec::new(),
        }
    }

    #[test]
    fn browse_history_pushes_dedups_and_go_live_clears() {
        let mut state = AppState::default();
        state.repos.push(RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state.active_repo = Some(RepoId(1));

        let a = CommitId("aaaaaaaa".into());
        let b = CommitId("bbbbbbbb".into());

        browse_repository_at_commit(&mut state, RepoId(1), a.clone());
        browse_repository_at_commit(&mut state, RepoId(1), b.clone());
        // Re-browsing an existing point does not duplicate it, just makes it current.
        browse_repository_at_commit(&mut state, RepoId(1), a.clone());

        let repo = &state.repos[0];
        assert_eq!(repo.browse_history, vec![a.clone(), b.clone()]);
        assert_eq!(repo.browsing_commit(), Some(&a));
        assert_eq!(state.sidebar_mode, SidebarMode::Files);

        reset_browse_to_live(&mut state, RepoId(1));
        let repo = &state.repos[0];
        assert!(repo.browse_history.is_empty());
        assert_eq!(repo.browsing_commit(), None);
        assert!(matches!(
            repo.file_browser.source,
            gitcomet_core::domain::FileSource::WorkingDirectory
        ));
    }

    fn conflicted_status(path: &Path, conflict: FileConflictKind) -> RepoStatus {
        RepoStatus {
            staged: Vec::new(),
            unstaged: vec![FileStatus {
                path: path.to_path_buf(),
                kind: FileStatusKind::Conflicted,
                conflict: Some(conflict),
            }],
        }
    }

    fn empty_conflict_file(path: &Path) -> ConflictFile {
        ConflictFile {
            path: path.to_path_buf().into(),
            base_bytes: None,
            ours_bytes: None,
            theirs_bytes: None,
            current_bytes: None,
            base: None,
            ours: None,
            theirs: None,
            current: None,
        }
    }

    fn new_state_with_repo(repo_id: RepoId) -> AppState {
        let mut state = AppState::default();
        state.repos.push(RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state
    }

    fn repo_mut(state: &mut AppState, repo_id: RepoId) -> &mut RepoState {
        state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo_id)
            .expect("repo not found")
    }

    fn mark_repo_open_ready(state: &mut AppState, repo_id: RepoId) {
        repo_mut(state, repo_id).set_open(Loadable::Ready(()));
    }

    fn mark_pending(state: &mut AppState, repo_id: RepoId, flag: u32) {
        let repo = repo_mut(state, repo_id);
        assert!(repo.loads_in_flight.request(flag));
        assert!(!repo.loads_in_flight.request(flag));
    }

    #[test]
    fn unknown_repo_handlers_are_noops() {
        let mut state = AppState::default();
        let repo_id = RepoId(42);
        let path = PathBuf::from("tracked.txt");
        let commit_id = CommitId("abc".into());

        assert!(
            file_history_loaded(&mut state, repo_id, path.clone(), Ok(empty_log_page())).is_empty()
        );
        assert!(
            blame_loaded(
                &mut state,
                repo_id,
                path.clone(),
                gitcomet_core::domain::BlameSource::Revision(None),
                Ok(Vec::new())
            )
            .is_empty()
        );
        assert!(conflict_file_loaded(&mut state, repo_id, path.clone(), Ok(None), None).is_empty());
        assert!(worktrees_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(submodules_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(select_commit(&mut state, repo_id, commit_id.clone()).is_empty());
        assert!(clear_commit_selection(&mut state, repo_id).is_empty());
        assert!(load_stashes(&mut state, repo_id).is_empty());
        assert!(refresh_branches(&mut state, repo_id).is_empty());
        assert!(
            load_conflict_file(
                &mut state,
                repo_id,
                path.clone(),
                ConflictFileLoadMode::CurrentOnly,
            )
            .is_empty()
        );
        assert!(load_reflog(&mut state, repo_id).is_empty());
        assert!(load_file_history(&mut state, repo_id, path.clone(), 25).is_empty());
        assert!(
            load_blame(
                &mut state,
                repo_id,
                path.clone(),
                gitcomet_core::domain::BlameSource::Revision(Some("HEAD".to_string()))
            )
            .is_empty()
        );
        assert!(load_worktrees(&mut state, repo_id).is_empty());
        assert!(load_submodules(&mut state, repo_id).is_empty());
        assert!(branches_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(remotes_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(remote_branches_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(status_loaded(&mut state, repo_id, Ok(RepoStatus::default())).is_empty());
        assert!(head_branch_loaded(&mut state, repo_id, Ok("main".to_string())).is_empty());
        assert!(upstream_divergence_loaded(&mut state, repo_id, Ok(None)).is_empty());
        assert!(tags_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(remote_tags_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(stashes_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(reflog_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(
            commit_details_loaded(
                &mut state,
                repo_id,
                commit_id.clone(),
                Ok(commit_details_for(commit_id))
            )
            .is_empty()
        );
        assert!(load_file_browser(&mut state, repo_id, FileSource::WorkingDirectory).is_empty());
        assert!(toggle_file_browser_dir(&mut state, repo_id, PathBuf::from("src")).is_empty());
        assert!(set_file_browser_search(&mut state, repo_id, "query".to_string()).is_empty());
        assert!(
            set_file_browser_source(&mut state, repo_id, FileSource::WorkingDirectory).is_empty()
        );
        assert!(set_sidebar_mode(&mut state, SidebarMode::Files).is_empty());
        assert!(
            file_browser_loaded(
                &mut state,
                repo_id,
                FileSource::WorkingDirectory,
                Ok(Vec::new())
            )
            .is_empty()
        );
    }

    #[test]
    fn file_history_loaded_updates_only_matching_path_and_reports_errors() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let tracked = PathBuf::from("tracked.txt");

        repo_mut(&mut state, repo_id)
            .history_state
            .file_history_path = Some(tracked.clone());
        file_history_loaded(
            &mut state,
            repo_id,
            PathBuf::from("other.txt"),
            Ok(empty_log_page()),
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.file_history,
            Loadable::NotLoaded
        ));

        file_history_loaded(&mut state, repo_id, tracked.clone(), Ok(empty_log_page()));
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.file_history,
            Loadable::Ready(_)
        ));

        file_history_loaded(
            &mut state,
            repo_id,
            tracked,
            Err(backend_error("file history failed")),
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(
            repo.history_state.file_history,
            Loadable::Error(_)
        ));
        assert_eq!(repo.diagnostics.len(), 1);
    }

    #[test]
    fn blame_loaded_requires_matching_path_and_source() {
        use gitcomet_core::domain::BlameSource;

        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("src/lib.rs");
        let source = BlameSource::Revision(Some("HEAD~1".to_string()));

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.blame_path = Some(path.clone());
            repo.history_state.blame_source = Some(source.clone());
        }

        blame_loaded(
            &mut state,
            repo_id,
            path.clone(),
            BlameSource::Revision(Some("different".to_string())),
            Ok(Vec::new()),
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.blame,
            Loadable::NotLoaded
        ));

        blame_loaded(
            &mut state,
            repo_id,
            path.clone(),
            source.clone(),
            Ok(Vec::new()),
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.blame,
            Loadable::Ready(_)
        ));

        blame_loaded(
            &mut state,
            repo_id,
            path,
            source,
            Err(backend_error("blame failed")),
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.history_state.blame, Loadable::Error(_)));
        assert_eq!(repo.diagnostics.len(), 1);
    }

    #[test]
    fn conflict_file_loaded_builds_session_from_merged_markers() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("conflict.txt");

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_conflict_file_path(Some(path.clone()));
            repo.set_status(Loadable::Ready(Arc::new(conflicted_status(
                &path,
                FileConflictKind::BothModified,
            ))));
        }

        let file = ConflictFile {
            path: path.clone().into(),
            base_bytes: None,
            ours_bytes: None,
            theirs_bytes: None,
            current_bytes: None,
            base: Some("base\n".to_string().into()),
            ours: Some("ours\n".to_string().into()),
            theirs: Some("theirs\n".to_string().into()),
            current: Some(
                "pre\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\npost\n"
                    .to_string()
                    .into(),
            ),
        };

        conflict_file_loaded(&mut state, repo_id, path.clone(), Ok(Some(file)), None);
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(
            repo.conflict_state.conflict_file,
            Loadable::Ready(Some(_))
        ));
        let session = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("session");
        assert_eq!(session.path, path);
        assert_eq!(session.conflict_kind, FileConflictKind::BothModified);
        assert!(!session.regions.is_empty());
    }

    #[test]
    fn conflict_file_loaded_uses_synthetic_session_for_non_marker_payloads() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("binary-conflict.bin");

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_conflict_file_path(Some(path.clone()));
            repo.set_status(Loadable::Ready(Arc::new(conflicted_status(
                &path,
                FileConflictKind::BothModified,
            ))));
        }

        let file = ConflictFile {
            path: path.clone().into(),
            base_bytes: Some(vec![0xff, 0x00].into()),
            ours_bytes: Some(b"ours\n".to_vec().into()),
            theirs_bytes: Some(b"theirs\n".to_vec().into()),
            current_bytes: None,
            base: None,
            ours: None,
            theirs: None,
            current: None,
        };

        conflict_file_loaded(&mut state, repo_id, path, Ok(Some(file)), None);
        let repo = repo_mut(&mut state, repo_id);
        let session = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("session");
        assert!(session.base.is_binary());
    }

    #[test]
    fn conflict_file_loaded_prefers_provided_session_and_records_errors() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let tracked_path = PathBuf::from("tracked.txt");
        let other_path = PathBuf::from("other.txt");

        repo_mut(&mut state, repo_id).set_conflict_file_path(Some(tracked_path.clone()));
        let provided = ConflictSession::new(
            tracked_path.clone(),
            FileConflictKind::BothAdded,
            ConflictPayload::Absent,
            ConflictPayload::Text("ours\n".to_string().into()),
            ConflictPayload::Text("theirs\n".to_string().into()),
        );

        conflict_file_loaded(
            &mut state,
            repo_id,
            tracked_path.clone(),
            Err(backend_error("conflict failed")),
            Some(provided.clone()),
        );
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(
                repo.conflict_state.conflict_file,
                Loadable::Error(_)
            ));
            let session = repo
                .conflict_state
                .conflict_session
                .as_ref()
                .expect("session");
            assert_eq!(session.path, provided.path);
            assert_eq!(session.conflict_kind, provided.conflict_kind);
            assert_eq!(session.strategy, provided.strategy);
            assert_eq!(session.ours.as_text(), provided.ours.as_text());
            assert_eq!(session.theirs.as_text(), provided.theirs.as_text());
            assert_eq!(repo.diagnostics.len(), 1);
        }

        conflict_file_loaded(
            &mut state,
            repo_id,
            other_path,
            Ok(Some(empty_conflict_file(&tracked_path))),
            None,
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(
            repo.conflict_state.conflict_file,
            Loadable::Error(_)
        ));
        let session = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("session");
        assert_eq!(session.path, provided.path);
        assert_eq!(session.conflict_kind, provided.conflict_kind);
        assert_eq!(session.strategy, provided.strategy);
    }

    #[test]
    fn load_requests_set_loading_and_emit_effects() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let conflict_path = PathBuf::from("conflict.txt");
        let history_path = PathBuf::from("src/lib.rs");
        let blame_path = PathBuf::from("src/main.rs");
        mark_repo_open_ready(&mut state, repo_id);

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_conflict_file(Loadable::Ready(Some(empty_conflict_file(&conflict_path))));
            repo.set_conflict_session(Some(ConflictSession::new(
                conflict_path.clone(),
                FileConflictKind::BothAdded,
                ConflictPayload::Absent,
                ConflictPayload::Text("ours".to_string().into()),
                ConflictPayload::Text("theirs".to_string().into()),
            )));
            repo.set_conflict_hide_resolved(true);
        }

        let effects = load_conflict_file(
            &mut state,
            repo_id,
            conflict_path.clone(),
            ConflictFileLoadMode::CurrentOnly,
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadConflictFile {
                repo_id: rid,
                ref path,
                mode: ConflictFileLoadMode::CurrentOnly
            } if rid == repo_id && path == &conflict_path
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(
                repo.conflict_state.conflict_file_path.as_ref(),
                Some(&conflict_path)
            );
            assert!(repo.conflict_state.conflict_file.is_loading());
            assert!(repo.conflict_state.conflict_session.is_none());
            assert!(!repo.conflict_state.conflict_hide_resolved);
        }

        let effects = load_file_history(&mut state, repo_id, history_path.clone(), 25);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadFileHistory {
                repo_id: rid,
                ref path,
                limit
            } if rid == repo_id && path == &history_path && limit == 25
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(
                repo.history_state.file_history_path.as_ref(),
                Some(&history_path)
            );
            assert!(repo.history_state.file_history.is_loading());
        }

        let effects = load_blame(
            &mut state,
            repo_id,
            blame_path.clone(),
            gitcomet_core::domain::BlameSource::Revision(Some("HEAD".to_string())),
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadBlame {
                repo_id: rid,
                ref path,
                source: gitcomet_core::domain::BlameSource::Revision(Some(ref rev))
            } if rid == repo_id && path == &blame_path && rev == "HEAD"
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.history_state.blame_path.as_ref(), Some(&blame_path));
            assert_eq!(
                repo.history_state.blame_source,
                Some(gitcomet_core::domain::BlameSource::Revision(Some(
                    "HEAD".to_string()
                )))
            );
            assert!(repo.history_state.blame.is_loading());
        }

        let effects = load_worktrees(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktrees { repo_id: rid } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).worktrees.is_loading());
        assert!(load_worktrees(&mut state, repo_id).is_empty());

        let effects = load_submodules(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadSubmodules { repo_id: rid } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).submodules.is_loading());

        let effects = load_tags(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadTags { repo_id: rid } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).tags.is_loading());
        assert!(load_tags(&mut state, repo_id).is_empty());

        let effects = load_stashes(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadStashes {
                repo_id: rid,
                limit: 50
            } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).stashes.is_loading());

        assert!(load_stashes(&mut state, repo_id).is_empty());

        let effects = refresh_branches(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadBranches { repo_id: rid } if rid == repo_id
        ));
        assert!(refresh_branches(&mut state, repo_id).is_empty());

        let effects = load_reflog(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadReflog {
                repo_id: rid,
                limit: 200
            } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).reflog.is_loading());
        assert!(load_reflog(&mut state, repo_id).is_empty());

        let effects = load_file_browser(&mut state, repo_id, FileSource::WorkingDirectory);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadFileBrowser {
                repo_id: rid,
                ref source
            } if rid == repo_id && matches!(source, FileSource::WorkingDirectory)
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(repo.file_browser.entries, Loadable::Loading));
            assert_eq!(repo.file_browser.source, FileSource::WorkingDirectory);
        }
    }

    #[test]
    fn pre_open_worktree_and_submodule_loads_are_noops() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        assert!(load_worktrees(&mut state, repo_id).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).worktrees,
            Loadable::NotLoaded
        ));
        assert!(
            !repo_mut(&mut state, repo_id)
                .loads_in_flight
                .is_in_flight(RepoLoadsInFlight::WORKTREES)
        );

        assert!(load_submodules(&mut state, repo_id).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).submodules,
            Loadable::NotLoaded
        ));
    }

    #[test]
    fn ensure_sidebar_data_stores_request_before_repo_is_open() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let request = SidebarDataRequest {
            worktrees: true,
            submodules: true,
            stashes: true,
        };

        assert!(ensure_sidebar_data(&mut state, repo_id, request).is_empty());

        let repo = repo_mut(&mut state, repo_id);
        assert_eq!(repo.sidebar_data_request, request);
        assert!(matches!(repo.worktrees, Loadable::NotLoaded));
        assert!(matches!(repo.submodules, Loadable::NotLoaded));
        assert!(matches!(repo.stashes, Loadable::NotLoaded));
    }

    #[test]
    fn ensure_sidebar_data_loads_only_missing_requested_sections() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        mark_repo_open_ready(&mut state, repo_id);
        repo_mut(&mut state, repo_id).set_submodules(Loadable::Ready(Vec::new()));

        let request = SidebarDataRequest {
            worktrees: true,
            submodules: false,
            stashes: true,
        };
        let effects = ensure_sidebar_data(&mut state, repo_id, request);

        assert!(effects.iter().any(
            |effect| matches!(effect, Effect::LoadWorktrees { repo_id: rid } if *rid == repo_id)
        ));
        assert!(!effects.iter().any(
            |effect| matches!(effect, Effect::LoadSubmodules { repo_id: rid } if *rid == repo_id)
        ));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadStashes {
                repo_id: rid,
                limit: 50
            } if *rid == repo_id
        )));

        let repo = repo_mut(&mut state, repo_id);
        assert!(repo.worktrees.is_loading());
        assert!(matches!(repo.submodules, Loadable::Ready(_)));
        assert!(repo.stashes.is_loading());

        assert!(ensure_sidebar_data(&mut state, repo_id, request).is_empty());
    }

    #[test]
    fn select_and_clear_commit_selection_cover_all_branches() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let commit_a = CommitId("a".into());
        let commit_b = CommitId("b".into());

        repo_mut(&mut state, repo_id).set_commit_details(Loadable::Error("old".to_string()));
        let effects = select_commit(&mut state, repo_id, commit_a.clone());
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadCommitDetails {
                repo_id: rid,
                ref commit_id
            } if rid == repo_id && commit_id == &commit_a
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.history_state.selected_commit.as_ref(), Some(&commit_a));
            assert!(matches!(
                repo.history_state.commit_details,
                Loadable::NotLoaded
            ));
        }

        assert!(select_commit(&mut state, repo_id, commit_a.clone()).is_empty());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_selected_commit(Some(commit_b.clone()));
            repo.set_commit_details(Loadable::Ready(Arc::new(commit_details_for(
                commit_a.clone(),
            ))));
        }
        assert!(select_commit(&mut state, repo_id, commit_a.clone()).is_empty());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_selected_commit(Some(commit_a.clone()));
            repo.set_commit_details(Loadable::Loading);
        }
        let effects = select_commit(&mut state, repo_id, commit_b.clone());
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadCommitDetails {
                repo_id: rid,
                ref commit_id
            } if rid == repo_id && commit_id == &commit_b
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.commit_details,
            Loadable::Loading
        ));

        assert!(clear_commit_selection(&mut state, repo_id).is_empty());
        let repo = repo_mut(&mut state, repo_id);
        assert!(repo.history_state.selected_commit.is_none());
        assert!(matches!(
            repo.history_state.commit_details,
            Loadable::NotLoaded
        ));
    }

    fn multi_selection(
        state: &mut AppState,
        repo_id: RepoId,
    ) -> crate::model::CommitMultiSelection {
        repo_mut(state, repo_id)
            .history_state
            .multi_selection
            .clone()
    }

    #[test]
    fn toggle_click_adds_and_removes_commits() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let a = CommitId("a".into());
        let b = CommitId("b".into());

        select_commit(&mut state, repo_id, a.clone());
        select_commit_multi(
            &mut state,
            repo_id,
            b.clone(),
            CommitSelectMode::Toggle,
            Some(1),
            None,
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![a.clone(), b.clone()]);
        assert_eq!(sel.anchor.as_ref(), Some(&b));
        assert_eq!(
            repo_mut(&mut state, repo_id).history_state.selected_commit,
            Some(b.clone())
        );

        // Toggling a selected commit removes it; focus falls back to the last
        // remaining commit.
        select_commit_multi(
            &mut state,
            repo_id,
            b.clone(),
            CommitSelectMode::Toggle,
            Some(1),
            None,
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![a.clone()]);
        assert_eq!(
            repo_mut(&mut state, repo_id).history_state.selected_commit,
            Some(a.clone())
        );

        // Toggling the last commit away clears the whole selection.
        select_commit_multi(
            &mut state,
            repo_id,
            a,
            CommitSelectMode::Toggle,
            Some(0),
            None,
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(repo.history_state.selected_commit.is_none());
        assert!(repo.history_state.multi_selection.commits.is_empty());
    }

    #[test]
    fn preserve_if_selected_moves_focus_without_collapsing() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let a = CommitId("a".into());
        let b = CommitId("b".into());
        let c = CommitId("c".into());

        select_commit(&mut state, repo_id, a.clone());
        select_commit_multi(
            &mut state,
            repo_id,
            b.clone(),
            CommitSelectMode::Toggle,
            Some(1),
            None,
        );
        assert_eq!(
            repo_mut(&mut state, repo_id).history_state.selected_commit,
            Some(b.clone())
        );

        // Right-click a commit already in the selection: the set is preserved,
        // only the focus moves.
        select_commit_multi(
            &mut state,
            repo_id,
            a.clone(),
            CommitSelectMode::PreserveIfSelected,
            None,
            None,
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![a.clone(), b.clone()]);
        assert_eq!(
            repo_mut(&mut state, repo_id).history_state.selected_commit,
            Some(a.clone())
        );

        // Right-click a commit outside the selection: collapse to it.
        select_commit_multi(
            &mut state,
            repo_id,
            c.clone(),
            CommitSelectMode::PreserveIfSelected,
            None,
            None,
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![c.clone()]);
        assert_eq!(
            repo_mut(&mut state, repo_id).history_state.selected_commit,
            Some(c)
        );
    }

    #[test]
    fn squash_preview_accepted_by_pending_request_even_when_plan_invalid() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let oldest = CommitId("old".into());
        let head = CommitId("head".into());
        // A request is in flight but the plan is transiently invalid (no Ready
        // log here). The returning result must still be accepted rather than
        // stranding the preview on Loading forever.
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.squash_preview_pending = Some((oldest.clone(), head.clone()));
            repo.set_squash_preview(Loadable::Loading);
        }
        let effects = squash_message_preview_loaded(
            &mut state,
            repo_id,
            oldest.clone(),
            head.clone(),
            Ok("Subject line\n\nBody text".to_string()),
        );
        assert!(effects.is_empty());
        let repo = repo_mut(&mut state, repo_id);
        match &repo.history_state.squash_preview {
            Loadable::Ready(preview) => {
                assert_eq!(preview.subject, "Subject line");
                assert_eq!(preview.body, "Body text");
                assert_eq!(preview.oldest, oldest);
                assert_eq!(preview.head, head);
            }
            other => panic!("expected Ready preview, got {other:?}"),
        }
        assert!(repo.history_state.squash_preview_pending.is_none());
    }

    #[test]
    fn squash_preview_dropped_when_request_range_differs() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.squash_preview_pending =
                Some((CommitId("new_old".into()), CommitId("new_head".into())));
            repo.set_squash_preview(Loadable::Loading);
        }
        // A stale result for a range we are no longer waiting on is ignored.
        squash_message_preview_loaded(
            &mut state,
            repo_id,
            CommitId("old".into()),
            CommitId("head".into()),
            Ok("stale".to_string()),
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(
            repo.history_state.squash_preview,
            Loadable::Loading
        ));
        assert!(repo.history_state.squash_preview_pending.is_some());
    }

    #[test]
    fn shift_click_selects_range_from_anchor_in_both_directions() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let ids: Vec<CommitId> = ["a", "b", "c", "d"]
            .iter()
            .map(|s| CommitId((*s).into()))
            .collect();

        select_commit(&mut state, repo_id, ids[1].clone());
        select_commit_multi(
            &mut state,
            repo_id,
            ids[3].clone(),
            CommitSelectMode::Range,
            Some(3),
            Some(ids.clone()),
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, ids[1..=3].to_vec());
        assert_eq!(sel.anchor.as_ref(), Some(&ids[1]));

        // Extending upward from the same anchor replaces the range.
        select_commit_multi(
            &mut state,
            repo_id,
            ids[0].clone(),
            CommitSelectMode::Range,
            Some(0),
            Some(ids.clone()),
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, ids[0..=1].to_vec());
    }

    #[test]
    fn shift_click_ignores_stale_anchor_index_hint() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let ids: Vec<CommitId> = ["a", "b", "c", "d"]
            .iter()
            .map(|s| CommitId((*s).into()))
            .collect();

        select_commit(&mut state, repo_id, ids[0].clone());
        {
            // Simulate a log reload shifting rows: the anchor hint index now
            // points elsewhere and the stored log rev no longer matches.
            let repo = repo_mut(&mut state, repo_id);
            let mut sel = repo.history_state.multi_selection.clone();
            sel.anchor_index = Some(3);
            sel.anchor_log_rev = Some(repo.history_state.log_rev.wrapping_add(1));
            repo.set_commit_multi_selection(sel);
        }
        select_commit_multi(
            &mut state,
            repo_id,
            ids[2].clone(),
            CommitSelectMode::Range,
            Some(2),
            Some(ids.clone()),
        );
        // The anchor is re-resolved by id, so the range is a..=c, not c..=d.
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, ids[0..=2].to_vec());
    }

    #[test]
    fn plain_click_collapses_multi_selection() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let a = CommitId("a".into());
        let b = CommitId("b".into());

        select_commit(&mut state, repo_id, a.clone());
        select_commit_multi(
            &mut state,
            repo_id,
            b.clone(),
            CommitSelectMode::Toggle,
            None,
            None,
        );
        assert_eq!(multi_selection(&mut state, repo_id).commits.len(), 2);

        select_commit(&mut state, repo_id, a.clone());
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![a.clone()]);
        assert_eq!(sel.anchor.as_ref(), Some(&a));
    }

    #[test]
    fn range_click_without_entries_falls_back_to_single() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let a = CommitId("a".into());
        let b = CommitId("b".into());

        select_commit(&mut state, repo_id, a);
        select_commit_multi(
            &mut state,
            repo_id,
            b.clone(),
            CommitSelectMode::Range,
            None,
            None,
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![b]);
    }

    fn test_commit(id: &str, parent: Option<&str>) -> gitcomet_core::domain::Commit {
        gitcomet_core::domain::Commit {
            id: CommitId(id.into()),
            parent_ids: parent
                .map(|p| smallvec::smallvec![CommitId(p.into())])
                .unwrap_or_default(),
            summary: "s".into(),
            author: "a".into(),
            time: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn multi_selection_compares_merged_diff_from_oldest_parent_to_newest() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        // Log is newest-first; each of c2..c4 has a parent, c1 is the root.
        repo_mut(&mut state, repo_id).set_log(Loadable::Ready(Arc::new(LogPage {
            commits: vec![
                test_commit("c4", Some("c3")),
                test_commit("c3", Some("c2")),
                test_commit("c2", Some("c1")),
                test_commit("c1", None),
            ],
            next_cursor: None,
        })));

        // Select c4 (newest) and c2 (oldest of the pair). The merged diff spans
        // c2's parent (c1) → c4, so every selected commit's own changes show.
        select_commit(&mut state, repo_id, CommitId("c4".into()));
        let effects = select_commit_multi(
            &mut state,
            repo_id,
            CommitId("c2".into()),
            CommitSelectMode::Toggle,
            Some(2),
            None,
        );

        let range = repo_mut(&mut state, repo_id)
            .history_state
            .range_selection
            .clone()
            .expect("range comparison active for a multi-selection");
        assert_eq!(range.from, CommitId("c1".into()));
        assert_eq!(range.to, Some(CommitId("c4".into())));
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::LoadRangeFiles { from, to, .. }
                if *from == CommitId("c1".into()) && *to == Some(CommitId("c4".into()))
        )));
    }

    #[test]
    fn multi_selection_reaching_root_uses_the_empty_tree_as_base() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_log(Loadable::Ready(Arc::new(LogPage {
            commits: vec![test_commit("c2", Some("c1")), test_commit("c1", None)],
            next_cursor: None,
        })));

        select_commit(&mut state, repo_id, CommitId("c2".into()));
        select_commit_multi(
            &mut state,
            repo_id,
            CommitId("c1".into()),
            CommitSelectMode::Toggle,
            Some(1),
            None,
        );

        let range = repo_mut(&mut state, repo_id)
            .history_state
            .range_selection
            .clone()
            .expect("range comparison active");
        // The oldest selected commit is the root and has no parent to diff from.
        // Basing on the root itself would drop everything it introduces from the
        // merged diff, so the empty tree is the base instead.
        assert_eq!(range.from, CommitId(EMPTY_TREE_ID.into()));
        assert_eq!(range.from_label, "start of history");
        assert_eq!(range.to, Some(CommitId("c2".into())));
    }

    #[test]
    fn clearing_selection_dissolves_multi_selection() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let a = CommitId("a".into());
        let b = CommitId("b".into());

        select_commit(&mut state, repo_id, a);
        select_commit_multi(&mut state, repo_id, b, CommitSelectMode::Toggle, None, None);
        assert_eq!(multi_selection(&mut state, repo_id).commits.len(), 2);

        clear_commit_selection(&mut state, repo_id);
        let repo = repo_mut(&mut state, repo_id);
        assert!(repo.history_state.multi_selection.commits.is_empty());
        assert!(repo.history_state.multi_selection.anchor.is_none());
    }

    #[test]
    fn loaded_handlers_reschedule_when_pending() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::BRANCHES);
        let effects = branches_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadBranches { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).branches,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::REMOTES);
        let effects = remotes_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadRemotes { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).remotes,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::REMOTE_BRANCHES);
        let effects = remote_branches_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadRemoteBranches { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_branches,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::WORKTREES);
        let effects = worktrees_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktrees { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).worktrees,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::HEAD_BRANCH);
        let effects = head_branch_loaded(&mut state, repo_id, Ok("main".to_string()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadHeadBranch { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).head_branch,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::UPSTREAM_DIVERGENCE);
        let effects = upstream_divergence_loaded(
            &mut state,
            repo_id,
            Ok(Some(UpstreamDivergence {
                ahead: 1,
                behind: 2,
            })),
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadUpstreamDivergence { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).upstream_divergence,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::STASHES);
        let effects = stashes_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadStashes {
                repo_id: rid,
                limit: 50
            } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).stashes,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::REFLOG);
        let effects = reflog_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadReflog {
                repo_id: rid,
                limit: 200
            } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).reflog,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::TAGS);
        let effects = tags_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadTags { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).tags,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::REMOTE_TAGS);
        let effects = remote_tags_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadRemoteTags { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_tags,
            Loadable::Ready(_)
        ));
    }

    #[test]
    fn status_lanes_replay_pending_refresh_even_when_payload_unchanged() {
        // A refresh coalesced while a status load was in flight must still be replayed when the
        // load completes with an unchanged payload: the in-flight read may have observed the
        // working tree/index just before an external change landed, so the coalesced refresh is
        // the only chance to pick it up. Dropping it (as a previous revision did) left stale
        // entries in the uncommitted view.
        let repo_id = RepoId(1);

        // Combined status load: an unchanged payload still replays the coalesced refresh and
        // re-arms the lane.
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_status(Loadable::Ready(Arc::new(RepoStatus::default())));
        mark_pending(&mut state, repo_id, RepoLoadsInFlight::WORKTREE_STATUS);
        let effects = status_loaded(&mut state, repo_id, Ok(RepoStatus::default()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktreeStatus { repo_id: rid } if rid == repo_id
        ));
        assert!(
            repo_mut(&mut state, repo_id)
                .loads_in_flight
                .is_in_flight(RepoLoadsInFlight::WORKTREE_STATUS),
            "the replayed load should re-arm the lane"
        );

        // Worktree-only lane.
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_worktree_status(Loadable::Ready(Vec::new()));
        mark_pending(&mut state, repo_id, RepoLoadsInFlight::WORKTREE_STATUS);
        let effects = worktree_status_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktreeStatus { repo_id: rid } if rid == repo_id
        ));

        // Staged-only lane.
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_staged_status(Loadable::Ready(Vec::new()));
        mark_pending(&mut state, repo_id, RepoLoadsInFlight::STAGED_STATUS);
        let effects = staged_status_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadStagedStatus { repo_id: rid } if rid == repo_id
        ));
    }

    #[test]
    fn head_branch_loaded_clears_detached_head_commit_when_attached() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_detached_head_commit(Some(CommitId("c1".into())));

        let _ = head_branch_loaded(&mut state, repo_id, Ok("main".to_string()));

        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.head_branch, Loadable::Ready(ref v) if v == "main"));
        assert!(repo.detached_head_commit.is_none());
    }

    #[test]
    fn head_branch_loaded_backfills_detached_head_commit_from_log() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_log(Loadable::Ready(Arc::new(LogPage {
            commits: vec![gitcomet_core::domain::Commit {
                id: CommitId("c1".into()),
                parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                summary: "s".into(),
                author: "a".into(),
                time: std::time::SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        })));

        let _ = head_branch_loaded(&mut state, repo_id, Ok("HEAD".to_string()));

        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.head_branch, Loadable::Ready(ref v) if v == "HEAD"));
        assert_eq!(repo.detached_head_commit, Some(CommitId("c1".into())));
    }

    #[test]
    fn head_branch_loaded_does_not_backfill_detached_head_commit_from_filtered_logs() {
        for (scope, page) in [
            (
                LogScope::NoMerges,
                LogPage {
                    commits: vec![gitcomet_core::domain::Commit {
                        id: CommitId("visible-non-merge".into()),
                        parent_ids: smallvec::smallvec![CommitId("hidden-head".into())],
                        summary: "visible".into(),
                        author: "a".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    }],
                    next_cursor: None,
                },
            ),
            (
                LogScope::MergesOnly,
                LogPage {
                    commits: vec![gitcomet_core::domain::Commit {
                        id: CommitId("visible-merge".into()),
                        parent_ids: smallvec::smallvec![
                            CommitId("p0".into()),
                            CommitId("p1".into())
                        ],
                        summary: "merge".into(),
                        author: "a".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    }],
                    next_cursor: None,
                },
            ),
        ] {
            let repo_id = RepoId(1);
            let mut state = new_state_with_repo(repo_id);
            repo_mut(&mut state, repo_id).history_state.history_scope = scope;
            repo_mut(&mut state, repo_id).set_log(Loadable::Ready(Arc::new(page)));

            let _ = head_branch_loaded(&mut state, repo_id, Ok("HEAD".to_string()));

            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(repo.head_branch, Loadable::Ready(ref v) if v == "HEAD"));
            assert!(
                repo.detached_head_commit.is_none(),
                "{scope:?} should not infer detached HEAD from filtered log contents"
            );
        }
    }

    #[test]
    fn loaded_handler_error_paths_record_diagnostics() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        assert!(branches_loaded(&mut state, repo_id, Err(backend_error("branches"))).is_empty());
        assert!(remotes_loaded(&mut state, repo_id, Err(backend_error("remotes"))).is_empty());
        assert!(
            remote_branches_loaded(&mut state, repo_id, Err(backend_error("remote branches")))
                .is_empty()
        );
        assert!(head_branch_loaded(&mut state, repo_id, Err(backend_error("head"))).is_empty());
        assert!(
            upstream_divergence_loaded(&mut state, repo_id, Err(backend_error("upstream")))
                .is_empty()
        );
        assert!(stashes_loaded(&mut state, repo_id, Err(backend_error("stashes"))).is_empty());
        assert!(reflog_loaded(&mut state, repo_id, Err(backend_error("reflog"))).is_empty());
        assert!(worktrees_loaded(&mut state, repo_id, Err(backend_error("worktrees"))).is_empty());
        assert!(
            submodules_loaded(&mut state, repo_id, Err(backend_error("submodules"))).is_empty()
        );
        assert!(
            file_browser_loaded(
                &mut state,
                repo_id,
                FileSource::WorkingDirectory,
                Err(backend_error("file_browser")),
            )
            .is_empty()
        );

        assert!(matches!(
            repo_mut(&mut state, repo_id).branches,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).remotes,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_branches,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).head_branch,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).upstream_divergence,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).stashes,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).reflog,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).worktrees,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).submodules,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).file_browser.entries,
            Loadable::Error(_)
        ));

        let repo = repo_mut(&mut state, repo_id);
        assert_eq!(repo.diagnostics.len(), 10);
    }

    #[test]
    fn status_loaded_clears_resolved_conflicts_and_preserves_unresolved_ones() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("conflict.txt");

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_status(Loadable::Ready(Arc::new(conflicted_status(
                &path,
                FileConflictKind::BothModified,
            ))));
            repo.set_conflict_file_path(Some(path.clone()));
            repo.set_conflict_file(Loadable::Ready(Some(empty_conflict_file(&path))));
            repo.set_conflict_session(Some(ConflictSession::new(
                path.clone(),
                FileConflictKind::BothModified,
                ConflictPayload::Text("base\n".to_string().into()),
                ConflictPayload::Text("ours\n".to_string().into()),
                ConflictPayload::Text("theirs\n".to_string().into()),
            )));
            repo.set_conflict_hide_resolved(true);
        }
        mark_pending(&mut state, repo_id, RepoLoadsInFlight::WORKTREE_STATUS);
        let effects = status_loaded(&mut state, repo_id, Ok(RepoStatus::default()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktreeStatus { repo_id: rid } if rid == repo_id
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(repo.status, Loadable::Ready(_)));
            assert!(repo.conflict_state.conflict_file_path.is_none());
            assert!(matches!(
                repo.conflict_state.conflict_file,
                Loadable::NotLoaded
            ));
            assert!(repo.conflict_state.conflict_session.is_none());
            assert!(!repo.conflict_state.conflict_hide_resolved);
        }

        {
            let repo = repo_mut(&mut state, repo_id);
            let unresolved = conflicted_status(&path, FileConflictKind::BothModified);
            repo.set_status(Loadable::Ready(Arc::new(unresolved.clone())));
            repo.set_conflict_file_path(Some(path.clone()));
            repo.set_conflict_file(Loadable::Ready(Some(empty_conflict_file(&path))));
            repo.set_conflict_session(Some(ConflictSession::new(
                path.clone(),
                FileConflictKind::BothModified,
                ConflictPayload::Text("base\n".to_string().into()),
                ConflictPayload::Text("ours\n".to_string().into()),
                ConflictPayload::Text("theirs\n".to_string().into()),
            )));
            repo.set_conflict_hide_resolved(true);
        }
        let unresolved = conflicted_status(&path, FileConflictKind::BothModified);
        assert!(status_loaded(&mut state, repo_id, Ok(unresolved)).is_empty());
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.conflict_state.conflict_file_path.as_ref(), Some(&path));
            assert!(repo.conflict_state.conflict_session.is_some());
            assert!(repo.conflict_state.conflict_hide_resolved);
        }

        assert!(status_loaded(&mut state, repo_id, Err(backend_error("status"))).is_empty());
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.status, Loadable::Error(_)));
        assert!(!repo.diagnostics.is_empty());
    }

    #[test]
    fn tags_and_remote_tags_handle_unsupported_as_empty_ready() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        assert!(tags_loaded(&mut state, repo_id, Err(unsupported_error())).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).tags,
            Loadable::Ready(_)
        ));
        assert_eq!(repo_mut(&mut state, repo_id).diagnostics.len(), 0);

        assert!(remote_tags_loaded(&mut state, repo_id, Err(unsupported_error())).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_tags,
            Loadable::Ready(_)
        ));
        assert_eq!(repo_mut(&mut state, repo_id).diagnostics.len(), 0);

        assert!(tags_loaded(&mut state, repo_id, Err(backend_error("tags"))).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).tags,
            Loadable::Error(_)
        ));

        assert!(
            remote_tags_loaded(&mut state, repo_id, Err(backend_error("remote tags"))).is_empty()
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_tags,
            Loadable::Error(_)
        ));
        assert_eq!(repo_mut(&mut state, repo_id).diagnostics.len(), 2);
    }

    #[test]
    fn cancelled_metadata_results_reset_to_not_loaded_without_diagnostics() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let cancelled = || Error::new(ErrorKind::Cancelled);

        assert!(tags_loaded(&mut state, repo_id, Err(cancelled())).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).tags,
            Loadable::NotLoaded
        ));

        assert!(remote_tags_loaded(&mut state, repo_id, Err(cancelled())).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_tags,
            Loadable::NotLoaded
        ));

        assert!(submodules_loaded(&mut state, repo_id, Err(cancelled())).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).submodules,
            Loadable::NotLoaded
        ));
        assert_eq!(repo_mut(&mut state, repo_id).diagnostics.len(), 0);
    }

    #[test]
    fn commit_details_loaded_requires_selected_commit_match() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let selected = CommitId("selected".into());
        let other = CommitId("other".into());

        repo_mut(&mut state, repo_id).set_selected_commit(Some(selected.clone()));
        commit_details_loaded(
            &mut state,
            repo_id,
            other.clone(),
            Ok(commit_details_for(other.clone())),
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.commit_details,
            Loadable::NotLoaded
        ));

        commit_details_loaded(
            &mut state,
            repo_id,
            selected.clone(),
            Ok(commit_details_for(selected.clone())),
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.commit_details,
            Loadable::Ready(_)
        ));

        commit_details_loaded(&mut state, repo_id, selected, Err(backend_error("details")));
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(
            repo.history_state.commit_details,
            Loadable::Error(_)
        ));
        assert_eq!(repo.diagnostics.len(), 1);
    }

    #[test]
    fn file_browser_loaded_updates_state_and_records_errors() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).file_browser.source = FileSource::WorkingDirectory;

        let entries = vec![FileEntry {
            name: "src".to_string(),
            path: Arc::new(PathBuf::from("src")),
            kind: FileEntryKind::Directory,
            depth: 0,
        }];
        let source = FileSource::WorkingDirectory;

        let effects = file_browser_loaded(&mut state, repo_id, source.clone(), Ok(entries));
        assert!(effects.is_empty());
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(repo.file_browser.entries, Loadable::Ready(_)));
            if let Loadable::Ready(arc) = &repo.file_browser.entries {
                assert_eq!(arc.len(), 1);
                assert_eq!(arc[0].name, "src");
            }
        }

        file_browser_loaded(
            &mut state,
            repo_id,
            source,
            Err(backend_error("tree failed")),
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.file_browser.entries, Loadable::Error(_)));
        assert_eq!(repo.diagnostics.len(), 1);
    }

    #[test]
    fn file_browser_loaded_discards_stale_results() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).file_browser.source = FileSource::Branch("main".to_string());

        let entries = vec![FileEntry {
            name: "stale.txt".to_string(),
            path: Arc::new(PathBuf::from("stale.txt")),
            kind: FileEntryKind::File,
            depth: 0,
        }];
        let wrong_source = FileSource::WorkingDirectory;

        let effects = file_browser_loaded(&mut state, repo_id, wrong_source, Ok(entries));
        assert!(effects.is_empty());
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.file_browser.entries, Loadable::NotLoaded));
        assert_eq!(
            repo.file_browser.source,
            FileSource::Branch("main".to_string())
        );
    }

    #[test]
    fn reveal_file_browser_path_expands_every_ancestor_and_clears_the_search() {
        let mut state = AppState::default();
        let repo_id = RepoId(1);
        state.repos.push(RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state.repos[0].file_browser.search_query = "main".to_string();
        let rev_before = state.repos[0].file_browser.file_browser_rev;

        reveal_file_browser_path(
            &mut state,
            repo_id,
            PathBuf::from("crates/gitcomet-ui-gpui/src/main.rs"),
        );

        let expanded = &state.repos[0].file_browser.expanded_dirs;
        for dir in [
            "crates",
            "crates/gitcomet-ui-gpui",
            "crates/gitcomet-ui-gpui/src",
        ] {
            assert!(
                expanded.contains(&Arc::new(PathBuf::from(dir))),
                "{dir} must be expanded so the file's row is visible"
            );
        }
        assert!(
            !expanded.contains(&Arc::new(PathBuf::from(
                "crates/gitcomet-ui-gpui/src/main.rs"
            ))),
            "the file itself is not a directory to expand"
        );
        assert!(
            state.repos[0].file_browser.search_query.is_empty(),
            "a filtered tree builds its rows from matches, so the search has to go"
        );
        assert_ne!(state.repos[0].file_browser.file_browser_rev, rev_before);
    }

    #[test]
    fn toggle_file_browser_dir_expands_and_collapses() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let dir = PathBuf::from("src/sub");

        let initial_rev = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;

        let effects = toggle_file_browser_dir(&mut state, repo_id, dir.clone());
        assert!(effects.is_empty());
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(
                repo.file_browser
                    .expanded_dirs
                    .contains(&Arc::new(dir.clone()))
            );
            assert!(repo.file_browser.file_browser_rev > initial_rev);
        }

        let rev_after_expand = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;
        let effects = toggle_file_browser_dir(&mut state, repo_id, dir.clone());
        assert!(effects.is_empty());
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(!repo.file_browser.expanded_dirs.contains(&Arc::new(dir)));
            assert!(repo.file_browser.file_browser_rev > rev_after_expand);
        }
    }

    #[test]
    fn set_file_browser_search_updates_query_and_rev() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        let initial_rev = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;

        let effects = set_file_browser_search(&mut state, repo_id, "test".to_string());
        assert!(effects.is_empty());
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.file_browser.search_query, "test");
            assert!(repo.file_browser.file_browser_rev > initial_rev);
        }

        let rev_after_first = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;
        let effects = set_file_browser_search(&mut state, repo_id, "test".to_string());
        assert!(effects.is_empty());
        assert_eq!(
            repo_mut(&mut state, repo_id).file_browser.file_browser_rev,
            rev_after_first
        );

        let effects = set_file_browser_search(&mut state, repo_id, "".to_string());
        assert!(effects.is_empty());
        assert_eq!(repo_mut(&mut state, repo_id).file_browser.search_query, "");
    }

    #[test]
    fn set_file_browser_source_resets_and_emits_load() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let commit_id = CommitId("abcdefgh".into());
        let source = FileSource::Commit(commit_id);

        let effects = set_file_browser_source(&mut state, repo_id, source.clone());
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::LoadFileBrowser { .. }));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.file_browser.source, source);
            assert!(matches!(repo.file_browser.entries, Loadable::NotLoaded));
            assert!(repo.file_browser.expanded_dirs.is_empty());
            assert!(repo.file_browser.search_query.is_empty());
        }

        let effects = set_file_browser_source(&mut state, repo_id, source);
        assert!(effects.is_empty());
    }

    #[test]
    fn set_sidebar_mode_triggers_file_browser_load_and_retries_on_error() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
        assert_eq!(state.sidebar_mode, SidebarMode::Files);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );

        repo_mut(&mut state, repo_id).file_browser.entries = Loadable::Ready(Arc::new(Vec::new()));
        set_sidebar_mode(&mut state, SidebarMode::Branches);
        let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );

        repo_mut(&mut state, repo_id).file_browser.entries = Loadable::Error("fail".to_string());
        set_sidebar_mode(&mut state, SidebarMode::Branches);
        let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );
    }

    #[test]
    fn load_file_browser_sets_loading_and_emits_effect() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let initial_rev = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;

        let effects = load_file_browser(&mut state, repo_id, FileSource::WorkingDirectory);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadFileBrowser {
                repo_id: rid,
                ..
            } if rid == repo_id
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(repo.file_browser.entries, Loadable::Loading));
            assert_eq!(repo.file_browser.source, FileSource::WorkingDirectory);
            assert!(repo.file_browser.file_browser_rev > initial_rev);
        }
    }

    #[test]
    fn load_file_browser_noop_when_repo_not_open() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        // open is Loading (set by new_opening), not Ready

        let effects = load_file_browser(&mut state, repo_id, FileSource::WorkingDirectory);
        assert!(effects.is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).file_browser.entries,
            Loadable::NotLoaded
        ));
    }

    #[test]
    fn browse_open_content_path_returns_correct_paths() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        // content_preview is false → None
        assert!(browse_open_content_path(&state, repo_id).is_none());

        // Set content_preview = true with Commit target
        let commit_id = CommitId("abc123".into());
        let path = PathBuf::from("src/main.rs");
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.content_preview = true;
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit_id.clone(),
                path: Some(path.clone()),
            });
        }
        assert_eq!(
            browse_open_content_path(&state, repo_id),
            Some(path.clone())
        );

        // WorkingTree target
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
                path: path.clone(),
                area: DiffArea::Unstaged,
            });
        }
        assert_eq!(
            browse_open_content_path(&state, repo_id),
            Some(path.clone())
        );

        // Commit with path: None → None
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id,
                path: None,
            });
        }
        assert!(browse_open_content_path(&state, repo_id).is_none());

        // diff_target is None → None
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.diff_target = None;
        }
        assert!(browse_open_content_path(&state, repo_id).is_none());

        // Unknown repo → None
        assert!(browse_open_content_path(&state, RepoId(999)).is_none());
    }

    #[test]
    fn browse_repository_at_commit_reopens_active_file() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let file_path = PathBuf::from("src/lib.rs");
        let commit_a = CommitId("aaaaaaaa".into());
        let commit_b = CommitId("bbbbbbbb".into());

        // Set up a content-preview file open at commit_a
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.content_preview = true;
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit_a.clone(),
                path: Some(file_path.clone()),
            });
        }

        // Browse commit_b — should reopen file at commit_b
        let effects = browse_repository_at_commit(&mut state, repo_id, commit_b.clone());
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id: rid,
                ..
            } if *rid == repo_id
        )));
    }

    #[test]
    fn reset_browse_to_live_reopens_active_file() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let file_path = PathBuf::from("README.md");
        let commit_id = CommitId("abcd1234".into());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.content_preview = true;
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit_id.clone(),
                path: Some(file_path.clone()),
            });
            repo.file_browser.source = FileSource::Commit(commit_id);
        }

        let effects = reset_browse_to_live(&mut state, repo_id);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id: rid,
                ..
            } if *rid == repo_id
        )));
    }

    #[test]
    fn browse_repository_at_commit_no_reopen_when_content_preview_is_false() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let commit_a = CommitId("aaaaaaaa".into());
        let commit_b = CommitId("bbbbbbbb".into());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.content_preview = false;
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit_a,
                path: Some(PathBuf::from("src/lib.rs")),
            });
        }

        let effects = browse_repository_at_commit(&mut state, repo_id, commit_b);
        // Should not contain LoadSelectedDiff (no file reopen)
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::LoadSelectedDiff { .. }))
        );
    }

    #[test]
    fn browse_history_evicts_oldest_when_exceeding_cap() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        const CAP: usize = 32;
        for i in 0..CAP + 3 {
            browse_repository_at_commit(
                &mut state,
                repo_id,
                CommitId(format!("commit{i:08}").into()),
            );
        }

        let repo = repo_mut(&mut state, repo_id);
        assert_eq!(repo.browse_history.len(), CAP);
        assert_eq!(
            repo.browse_history[0].0.as_ref(),
            "commit00000003".to_string()
        );
        assert_eq!(
            repo.browse_history[CAP - 1].0.as_ref(),
            format!("commit{:08}", CAP + 2)
        );
    }

    #[test]
    fn browse_history_rebrowse_does_not_move_to_mru() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let a = CommitId("aaaaaaaa".into());
        let b = CommitId("bbbbbbbb".into());
        let c = CommitId("cccccccc".into());

        browse_repository_at_commit(&mut state, repo_id, a.clone());
        browse_repository_at_commit(&mut state, repo_id, b.clone());
        browse_repository_at_commit(&mut state, repo_id, c.clone());
        // Re-browse a — should NOT move to end
        browse_repository_at_commit(&mut state, repo_id, a.clone());

        let repo = repo_mut(&mut state, repo_id);
        assert_eq!(repo.browse_history.len(), 3);
        // a stays at position 0, not moved to end
        assert_eq!(repo.browse_history[0], a);
        assert_eq!(repo.browse_history[1], b);
        assert_eq!(repo.browse_history[2], c);
    }

    #[test]
    fn set_sidebar_mode_noop_without_active_repo() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        mark_repo_open_ready(&mut state, repo_id);
        state.active_repo = None;

        let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
        assert!(effects.is_empty());
        assert_eq!(state.sidebar_mode, SidebarMode::Files);
    }

    #[test]
    fn set_sidebar_mode_emits_load_even_when_repo_not_ready() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        // repo.open is Loading (set by new_opening), not Ready

        let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
        // set_sidebar_mode does NOT check repo.open — it emits LoadFileBrowser,
        // but load_file_browser will be a no-op when open isn't Ready.
        // The effect IS emitted (the no-op is downstream in the effect handler).
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );
    }

    #[test]
    fn browse_repository_at_commit_same_commit_with_file_open_does_not_reopen() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let file_path = PathBuf::from("src/main.rs");
        let commit_id = CommitId("deadbeef".into());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.file_browser.source = FileSource::Commit(commit_id.clone());
            repo.diff_state.content_preview = true;
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit_id.clone(),
                path: Some(file_path),
            });
        }

        // Browse the SAME commit — source unchanged, no LoadFileBrowser emitted
        let effects = browse_repository_at_commit(&mut state, repo_id, commit_id);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::LoadSelectedDiff { .. }))
        );
    }

    fn blame_line(line: &str) -> gitcomet_core::services::BlameLine {
        gitcomet_core::services::BlameLine {
            commit_id: Arc::from("1111111111111111111111111111111111111111"),
            author: Arc::from("Ada"),
            author_time_unix: Some(1_700_000_000),
            summary: Arc::from("initial"),
            body: None,
            line: line.to_string(),
            prior_exists: true,
            source_path: None,
            prior_commit: None,
        }
    }

    #[test]
    fn load_blame_dedupes_same_target_while_loading() {
        // `MainPaneView::render` dispatches from an asynchronously pushed state
        // snapshot, so a render burst (e.g. during a window resize) can ask for
        // the same blame many times before the `Loading` snapshot arrives. Each
        // duplicate would fork another `git blame` subprocess.
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("src/lib.rs");
        let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);

        let effects = load_blame(&mut state, repo_id, path.clone(), source.clone());
        assert_eq!(effects.len(), 1);
        assert!(load_blame(&mut state, repo_id, path.clone(), source.clone()).is_empty());
        assert!(load_blame(&mut state, repo_id, path, source).is_empty());
    }

    #[test]
    fn load_blame_reloads_when_target_changes_while_loading() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);

        load_blame(
            &mut state,
            repo_id,
            PathBuf::from("src/lib.rs"),
            source.clone(),
        );
        let other = PathBuf::from("src/main.rs");
        let effects = load_blame(&mut state, repo_id, other.clone(), source);
        assert_eq!(effects.len(), 1);
        assert_eq!(
            repo_mut(&mut state, repo_id)
                .history_state
                .blame_path
                .as_ref(),
            Some(&other)
        );
    }

    #[test]
    fn load_blame_retains_ready_annotations_for_the_same_target() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("src/lib.rs");
        let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);
        let lines = Arc::new(vec![blame_line("let x = 1;")]);
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.blame_path = Some(path.clone());
            repo.history_state.blame_source = Some(source.clone());
            repo.history_state.blame = Loadable::Ready(Arc::clone(&lines));
        }

        load_blame(&mut state, repo_id, path, source);

        let repo = repo_mut(&mut state, repo_id);
        assert!(repo.history_state.blame.is_loading());
        assert!(
            repo.history_state
                .retained_blame_while_loading
                .as_ref()
                .is_some_and(|held| Arc::ptr_eq(held, &lines)),
            "the annotation column must keep painting while the same target reloads"
        );
    }

    #[test]
    fn load_blame_drops_retained_annotations_when_retargeting() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.blame_path = Some(PathBuf::from("src/lib.rs"));
            repo.history_state.blame_source = Some(source.clone());
            repo.history_state.blame = Loadable::Ready(Arc::new(vec![blame_line("let x = 1;")]));
        }

        load_blame(&mut state, repo_id, PathBuf::from("src/main.rs"), source);

        assert!(
            repo_mut(&mut state, repo_id)
                .history_state
                .retained_blame_while_loading
                .is_none(),
            "annotations for a different file must never be painted"
        );
    }

    #[test]
    fn blame_loaded_reuses_the_retained_allocation_when_unchanged() {
        // An identical reload must not produce a new `Arc`: the view keys its
        // notify fingerprint and its memoized blame time range on Arc identity.
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("src/lib.rs");
        let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);
        let lines = Arc::new(vec![blame_line("let x = 1;")]);
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.blame_path = Some(path.clone());
            repo.history_state.blame_source = Some(source.clone());
            repo.history_state.blame = Loadable::Ready(Arc::clone(&lines));
        }
        load_blame(&mut state, repo_id, path.clone(), source.clone());

        blame_loaded(
            &mut state,
            repo_id,
            path,
            source,
            Ok(vec![blame_line("let x = 1;")]),
        );

        let repo = repo_mut(&mut state, repo_id);
        assert!(
            matches!(&repo.history_state.blame, Loadable::Ready(got) if Arc::ptr_eq(got, &lines))
        );
        assert!(repo.history_state.retained_blame_while_loading.is_none());
    }

    #[test]
    fn blame_loaded_replaces_the_retained_allocation_when_changed() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("src/lib.rs");
        let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);
        let lines = Arc::new(vec![blame_line("let x = 1;")]);
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.blame_path = Some(path.clone());
            repo.history_state.blame_source = Some(source.clone());
            repo.history_state.blame = Loadable::Ready(Arc::clone(&lines));
        }
        load_blame(&mut state, repo_id, path.clone(), source.clone());

        blame_loaded(
            &mut state,
            repo_id,
            path,
            source,
            Ok(vec![blame_line("let x = 2;")]),
        );

        let repo = repo_mut(&mut state, repo_id);
        assert!(
            matches!(&repo.history_state.blame, Loadable::Ready(got) if !Arc::ptr_eq(got, &lines))
        );
        assert!(repo.history_state.retained_blame_while_loading.is_none());
    }

    #[test]
    fn file_browser_loaded_cancelled_error_records_diagnostic() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).file_browser.source = FileSource::WorkingDirectory;

        let cancelled = Error::new(ErrorKind::Cancelled);
        let effects = file_browser_loaded(
            &mut state,
            repo_id,
            FileSource::WorkingDirectory,
            Err(cancelled),
        );
        assert!(effects.is_empty());
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.file_browser.entries, Loadable::Error(_)));
        assert_eq!(repo.diagnostics.len(), 1);
    }
}
