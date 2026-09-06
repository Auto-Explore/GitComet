use super::util::{
    EffectAccumulator, apply_selected_diff_load_plan_state, diff_reload_effects, push_diagnostic,
    push_notification, selected_diff_load_plan,
};
use crate::model::{
    AppNotificationKind, AppState, CommitMultiSelection, ConflictFileLoadMode, DiagnosticKind,
    FileBrowserSettings, ForeignDiffOrigin, Loadable, PendingFileBrowserReopen, RangeSelection,
    RepoId, RepoLoadsInFlight, RepoState, SidebarDataRequest, SidebarMode,
};
use crate::msg::{CommitSelectMode, ConflictAutosolveMode, Effect};
use gitcomet_core::conflict_session::{
    ConflictPayload, ConflictRegionResolution, ConflictRegionSourceRanges,
    ConflictResolverStrategy, ConflictSession, reconstruct_conflict_marker_sides,
};
use gitcomet_core::domain::{
    Branch, CommitDetails, CommitFileChange, CommitId, EMPTY_TREE_ID, FileEntry, FileSource,
    FileStatusKind, LogCursor, LogPage, RecentCommitMessage, RefMetadata, ReflogEntry, Remote,
    RemoteBranch, RemoteTag, RepoStatus, StashEntry, Submodule, Tag, UpstreamDivergence, Worktree,
    WorktreeDirtySummary,
};
use gitcomet_core::error::Error;
use gitcomet_core::merge::{MergeSource, OrderedSelection};
use gitcomet_core::services::{GitRepository, InteractiveRebaseAction, InteractiveRebaseEntry};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// "Everything after the cursor" for the rest of a file's history: cursor
/// pages come from one cached follow walk, so one request costs the same as
/// many and the picker gets a complete, searchable list.
const FILE_HISTORY_REMAINDER_LIMIT: usize = usize::MAX;

pub(super) fn file_history_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    cursor: Option<LogCursor>,
    result: std::result::Result<Arc<LogPage>, Error>,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if repo_state.history_state.file_history_path.as_ref() != Some(&path) {
        return Vec::new();
    }

    let page = match cursor {
        None => match result {
            Ok(page) => page,
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.history_state.file_history = Loadable::Error(e.to_string());
                return Vec::new();
            }
        },
        Some(cursor) => {
            // A continuation only extends the page that asked for it. The
            // popover reloads the first page on every open, so a late answer
            // to an earlier open must not be appended to a fresh page.
            let Loadable::Ready(current) = &repo_state.history_state.file_history else {
                return Vec::new();
            };
            if current.next_cursor.as_ref() != Some(&cursor) {
                return Vec::new();
            }
            let mut commits = current.commits.clone();
            let next_cursor = match result {
                Ok(rest) => {
                    let rest = Arc::unwrap_or_clone(rest);
                    commits.extend(rest.commits);
                    rest.next_cursor
                }
                Err(e) => {
                    // Keep what is loaded; clearing the cursor stops the
                    // picker from reporting that older commits are coming.
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    None
                }
            };
            Arc::new(LogPage {
                commits,
                next_cursor,
            })
        }
    };

    let effects = page
        .next_cursor
        .clone()
        .map(|cursor| Effect::LoadFileHistory {
            repo_id,
            path,
            limit: FILE_HISTORY_REMAINDER_LIMIT,
            cursor: Some(cursor),
        })
        .into_iter()
        .collect();
    repo_state.history_state.file_history = Loadable::Ready(page);
    effects
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

/// Auto-solve policy: only the always-safe rules
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
    let mut inline_refresh = None;
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        match result {
            Ok(v) => repo_state.set_worktree_dirty(Loadable::Ready(v)),
            // A worktree that cannot be opened (removed, on an unmounted
            // volume) is a routine condition, not something worth a diagnostic
            // banner -- the scan simply reports nothing for it, per worktree,
            // inside the scan.
            //
            // A failure of the whole reply is a different thing: it means the
            // scan never ran (cancelled load, repo handle gone, git runtime
            // unavailable), not that the worktrees are clean. Overwriting a good
            // list with it would blank every row and -- through
            // `selected_worktree_is_gone` below -- drop the selection and close
            // the inline diff the user is reading. Keep the last known counts on
            // screen, and record the error only when there is nothing to keep.
            Err(e) => {
                if !matches!(repo_state.worktree_dirty, Loadable::Ready(_)) {
                    // A cancelled scan is not a failure worth showing: the load
                    // it belonged to was abandoned deliberately, and the trigger
                    // that abandoned it queues another. Anything else is a real
                    // failure and the pane should say so rather than sit on
                    // `Loading` forever.
                    let next = if matches!(e.kind(), gitcomet_core::error::ErrorKind::Cancelled) {
                        Loadable::NotLoaded
                    } else {
                        Loadable::Error(e.to_string())
                    };
                    repo_state.set_worktree_dirty(next);
                }
            }
        }
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
                // Anything else is the absence of an answer, not the answer that
                // the row is gone. Dropping the selection on it would close the
                // user's open diff every time a scan is cancelled.
                _ => false,
            });
        if selected_worktree_is_gone {
            repo_state.set_worktree_selection(None);
        }
        inline_refresh = refresh_worktree_inline_diff_entries(repo_state);
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
    // Outside the borrow above.
    match inline_refresh {
        // The file changed sides (staged <-> unstaged): a different target, so
        // the pane must drop what it is showing and load the new one.
        Some(WorktreeInlineRefresh::Reselect(ix)) => {
            effects.extend(super::diff_selection::select_inline_submodule_diff(
                state, repo_id, ix,
            ));
        }
        // The target did not move, but this scan is the only notice we get that
        // the file behind it may have been edited -- nothing else invalidates a
        // linked worktree's patch.
        Some(WorktreeInlineRefresh::Reload) => {
            effects.extend(
                super::diff_selection::refresh_inline_submodule_selected_diff(state, repo_id),
            );
        }
        None => {}
    }
    effects
}

/// What a landed scan asks of the linked-worktree diff that is open over it.
enum WorktreeInlineRefresh {
    /// The selected file now sits at another index, under another target.
    Reselect(usize),
    /// The selected row still points at the same target; only its contents can
    /// have moved.
    Reload,
}

/// Re-resolves an open linked-worktree inline diff against a scan that has just
/// landed.
///
/// The entry list is a snapshot of the worktree's changed files taken when a row
/// was clicked, while the rows themselves are rebuilt from every scan. Left
/// alone, a rescan that adds or removes a file shifts the row indices out from
/// under `selected_ix`: the pane highlights whichever file now sits at that
/// index, and steps to neighbours that may no longer be changed at all. Submodule
/// inline diffs need none of this -- their entries come from a fixed commit.
///
/// Returns what the caller should do with the diff once the borrow ends. `None`
/// when there is nothing open to refresh -- and when the file the diff shows is
/// no longer changed, in which case the diff is closed outright, the same way a
/// vanished row retires one.
fn refresh_worktree_inline_diff_entries(
    repo_state: &mut RepoState,
) -> Option<WorktreeInlineRefresh> {
    let (entries, selected, origin) = {
        let inline = repo_state.diff_state.inline_submodule_diff.as_ref()?;
        if !matches!(inline.origin, ForeignDiffOrigin::Worktree { .. }) {
            return None;
        }
        let Loadable::Ready(dirty) = &repo_state.worktree_dirty else {
            return None;
        };
        let summary = dirty
            .iter()
            .find(|summary| summary.path == inline.submodule_repo_path)?;
        let entries = crate::model::worktree_inline_diff_entries(summary);
        let selected = inline.entries.get(inline.selected_ix).and_then(|shown| {
            // Matched on the whole target, not the path: a file that is staged
            // *and* modified again appears twice, once per half, and a path-only
            // match always resolves to the staged copy -- so the pane silently
            // swapped sides under anyone reading the unstaged one.
            entries
                .iter()
                .position(|entry| entry.target == shown.target)
                // Only once the exact target is gone does the same path in the
                // other half become the best answer: staging what is on screen
                // retires its unstaged entry, and following the file there beats
                // closing the diff.
                .or_else(|| entries.iter().position(|entry| entry.path == shown.path))
        });
        // The chip labelling the diff reads `origin`, which was captured when the
        // row was clicked. A checkout in that worktree moves the branch under it.
        let origin = ForeignDiffOrigin::Worktree {
            branch: summary.branch.clone(),
            detached: summary.detached,
        };
        (entries, selected, origin)
    };

    let Some(selected) = selected else {
        repo_state.diff_state.inline_submodule_diff = None;
        repo_state.bump_diff_state_rev();
        return None;
    };

    let inline = repo_state.diff_state.inline_submodule_diff.as_mut()?;
    let target_moved = entries[selected].target != inline.target;
    let changed =
        entries != inline.entries || selected != inline.selected_ix || origin != inline.origin;
    inline.entries = entries;
    inline.selected_ix = selected;
    inline.origin = origin;
    if changed {
        repo_state.bump_diff_state_rev();
    }
    Some(if target_moved {
        WorktreeInlineRefresh::Reselect(selected)
    } else {
        WorktreeInlineRefresh::Reload
    })
}

pub(super) fn ref_metadata_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Arc<FxHashMap<String, RefMetadata>>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let ref_metadata = match result {
            Ok(metadata) => Loadable::Ready(metadata),
            // A backend that does not implement this will never implement it,
            // so latch an empty map rather than `Error` — callers retry on
            // `Error`, which would re-schedule a doomed load on every open.
            Err(e) if matches!(e.kind(), gitcomet_core::error::ErrorKind::Unsupported(_)) => {
                Loadable::Ready(Arc::new(FxHashMap::default()))
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
    // One pass over the page with the selection hashed, instead of a page
    // scan per selected id: a shift-click over a large selection was
    // quadratic.
    let wanted: FxHashSet<&str> = selected.iter().map(|id| id.as_ref()).collect();
    let mut newest_ix: Option<usize> = None;
    let mut oldest_ix: Option<usize> = None;
    let mut found = 0usize;
    for (ix, commit) in page.commits.iter().enumerate() {
        if !wanted.contains(commit.id.as_ref()) {
            continue;
        }
        newest_ix.get_or_insert(ix);
        oldest_ix = Some(ix);
        found += 1;
        if found == wanted.len() {
            break;
        }
    }
    if found != wanted.len() {
        return None;
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
        repo_state.navigation.comparison_mark =
            Some(crate::model::ComparisonMark { commit_id, label });
    }
    Vec::new()
}

pub(super) fn clear_comparison_mark(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.navigation.comparison_mark = None;
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
        match &repo_state.navigation.comparison_mark {
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
    // Idempotent on purpose. A pending history reveal re-drives on every render
    // of the history panel, so this message arrives once per frame for as long
    // as pagination takes to reach the target. Re-running the body each time
    // would bump `commit_details_rev` -- which the details pane hashes, so the
    // repaint drives the next render -- and re-arm a full `git status` walk
    // across every linked worktree.
    if repo_state.history_state.worktree_selection.as_deref() == Some(path.as_path()) {
        return Vec::new();
    }
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
    repo_state.set_reflog(Loadable::Loading);
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
        cursor: None,
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
    if !matches!(repo_state.open, Loadable::Ready(())) || repo_state.file_browser.source != source {
        return Vec::new();
    }
    // A refresh can come from an older sidebar snapshot after browsing has
    // moved or exited. Only explicit browsing actions may change the source.
    if !matches!(repo_state.file_browser.entries, Loadable::Ready(_)) {
        repo_state.file_browser.entries = Loadable::Loading;
    }
    repo_state.file_browser.bump_rev();
    request_file_browser_load(repo_state).into_iter().collect()
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

/// Whether a query actually filters the file tree, and so force-expands every
/// directory and ignores `expanded_dirs`.
///
/// The search input is multiline and stores what was typed verbatim, so a lone
/// space is a non-empty query that filters nothing. Mirrors the view's
/// `file_browser_search_is_active`.
fn file_browser_query_filters(query: &str) -> bool {
    query.lines().any(|line| !line.trim().is_empty())
}

fn file_browser_is_filtered(repo_state: &RepoState) -> bool {
    file_browser_query_filters(&repo_state.file_browser.search_query)
}

#[cfg(test)]
mod file_browser_filter_tests {
    use super::file_browser_query_filters;

    /// The same table the view asserts in
    /// `file_browser_search_predicate_agrees_with_the_renderers_matchers`.
    /// The predicate lives in both crates and cannot be shared, so the two
    /// tables are what keep them from drifting: change one, change both.
    ///
    /// Calls the real predicate rather than restating it: a copy here would
    /// stay green through exactly the drift it exists to catch.
    #[test]
    fn filtered_predicate_matches_the_views_table() {
        for (query, expected) in [
            ("", false),
            (" ", false),
            ("\n", false),
            ("  \n \t ", false),
            ("a", true),
            (" a ", true),
            ("a\nb", true),
            ("\na", true),
            ("#comment", true),
        ] {
            assert_eq!(
                file_browser_query_filters(query),
                expected,
                "disagreement for {query:?}"
            );
        }
    }
}

pub(super) fn toggle_file_browser_dir(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        // A filtered tree renders every directory expanded and never reads
        // `expanded_dirs`, so a toggle here would move nothing on screen and
        // then silently reshape the tree the moment the search was cleared.
        if file_browser_is_filtered(repo_state) {
            return Vec::new();
        }
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

/// Expand or collapse `path` and every directory under it.
///
/// The backend enumerates the whole tree in one pass, so every descendant is
/// already in `entries` and this needs no loading. `starts_with` on the flat
/// list also covers `path` itself, which is what makes "Expand all under here"
/// open the folder it was invoked on.
pub(super) fn set_file_browser_dir_expanded_recursive(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    expanded: bool,
) -> Vec<Effect> {
    // `Path::starts_with("")` is true of every path, so an empty path would
    // reach the whole tree and a collapse would wipe `expanded_dirs` outright.
    // The branch-group sibling guards this the same way.
    if path.as_os_str().is_empty() {
        return Vec::new();
    }
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    // Frozen while a search filters the tree, for the same reason a single
    // toggle is.
    if file_browser_is_filtered(repo_state) {
        return Vec::new();
    }
    let Loadable::Ready(entries) = &repo_state.file_browser.entries else {
        return Vec::new();
    };

    // Cloning the Arc releases the borrow on `file_browser` so `expanded_dirs`
    // can be written while the entry list is walked.
    let entries = Arc::clone(entries);
    let mut changed = false;
    for entry in entries.iter() {
        if entry.kind != gitcomet_core::domain::FileEntryKind::Directory
            || !entry.path.starts_with(&path)
        {
            continue;
        }
        // Each entry already owns its path as an `Arc`, so expanding reuses it
        // rather than allocating a second copy per directory.
        changed |= if expanded {
            repo_state
                .file_browser
                .expanded_dirs
                .insert(Arc::clone(&entry.path))
        } else {
            repo_state.file_browser.expanded_dirs.remove(&entry.path)
        };
    }

    if changed {
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

pub(super) fn request_file_browser_load(repo_state: &mut RepoState) -> Option<Effect> {
    // Opening repos have no backend handle yet. Claiming the lane here would
    // block the first real load when RepoOpenedOk installs the handle.
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return None;
    }
    repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::FILE_BROWSER)
        .then(|| Effect::LoadFileBrowser {
            repo_id: repo_state.id,
            source: repo_state.file_browser.source.clone(),
        })
}

/// Point the file browser at `source` without touching the tree's shape.
///
/// Rows already on screen stay up (marked `stale`, so `needs_load()` still asks
/// for the walk) and `expanded_dirs`/`search_query` survive: a browse point that
/// moves with the history selection must not collapse the tree on every step.
/// The open content preview is remembered for `file_browser_loaded` to
/// re-target, or close, once the listing says whether the file exists there.
/// Returns whether the source actually changed.
pub(super) fn retarget_file_browser(repo_state: &mut RepoState, source: FileSource) -> bool {
    if repo_state.file_browser.source == source {
        return false;
    }
    repo_state.file_browser.pending_reopen = browse_open_content_path(repo_state);
    repo_state.file_browser.source = source;
    if matches!(repo_state.file_browser.entries, Loadable::Ready(_)) {
        repo_state.file_browser.stale = true;
    } else {
        repo_state.file_browser.entries = Loadable::NotLoaded;
        repo_state.file_browser.stale = false;
    }
    repo_state.file_browser.bump_rev();
    true
}

pub(super) fn set_file_browser_source(
    state: &mut AppState,
    repo_id: RepoId,
    source: FileSource,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    // An explicit choice of browse point acknowledges the current selection:
    // following must not undo it until the selection next moves.
    repo_state
        .file_browser
        .set_active(matches!(source, FileSource::Commit(_)));
    repo_state.file_browser.followed_selection_rev =
        Some(repo_state.history_state.selected_commit_rev);
    if !retarget_file_browser(repo_state, source) {
        return Vec::new();
    }
    request_file_browser_load(repo_state).into_iter().collect()
}

/// Move the browse point to the history selection while the Files tab shows.
///
/// Keyed on `selected_commit_rev`, so starting at another commit sticks until
/// the selection next moves. Exited browsing stays live. A hidden tab records
/// nothing and catches up when it is shown. Returns whether the source changed.
pub(super) fn sync_file_browser_to_selection(
    repo_state: &mut RepoState,
    follow: bool,
    sidebar_mode: SidebarMode,
) -> bool {
    if !repo_state.file_browser.active || !follow || sidebar_mode != SidebarMode::Files {
        return false;
    }
    let rev = repo_state.history_state.selected_commit_rev;
    if repo_state.file_browser.followed_selection_rev == Some(rev) {
        return false;
    }
    repo_state.file_browser.followed_selection_rev = Some(rev);
    let want = repo_state
        .history_state
        .selected_commit
        .clone()
        .map(FileSource::Commit)
        .unwrap_or(FileSource::WorkingDirectory);
    retarget_file_browser(repo_state, want)
}

/// Runs after every reduced message: keeps the active repo's Files tab on the
/// selected history row. The lane coalescing in `request_file_browser_load`
/// turns an arrow-key burst into one walk plus one queued re-walk.
pub(super) fn follow_history_selection(state: &mut AppState, effects: &mut impl EffectAccumulator) {
    let follow = state.file_browser_settings.follow_selected_commit;
    let sidebar_mode = state.sidebar_mode;
    if !follow || sidebar_mode != SidebarMode::Files {
        return;
    }
    let Some(repo_id) = state.active_repo else {
        return;
    };
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return;
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return;
    }
    if sync_file_browser_to_selection(repo_state, follow, sidebar_mode)
        && let Some(effect) = request_file_browser_load(repo_state)
    {
        effects.push_effect(effect);
    }
}

pub(super) fn set_file_browser_settings(
    state: &mut AppState,
    settings: FileBrowserSettings,
) -> Vec<Effect> {
    let turned_on =
        settings.follow_selected_commit && !state.file_browser_settings.follow_selected_commit;
    state.file_browser_settings = settings;
    if turned_on {
        // Forget what was followed so the post-reduce hook syncs right away.
        for repo in &mut state.repos {
            repo.file_browser.followed_selection_rev = None;
        }
    }
    Vec::new()
}

pub(super) fn set_sidebar_mode(state: &mut AppState, mode: SidebarMode) -> Vec<Effect> {
    if state.sidebar_mode != mode {
        state.sidebar_mode = mode;
        let follow = state.file_browser_settings.follow_selected_commit;

        if mode == SidebarMode::Files
            && let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id)
        {
            // Retarget first, so the one load below carries the selection's
            // source instead of walking the old one and then walking again.
            sync_file_browser_to_selection(repo, follow, mode);
            if repo.file_browser.needs_load() {
                return request_file_browser_load(repo).into_iter().collect();
            }
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
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && !repo_state.navigation.browse_history.contains(&commit_id)
    {
        repo_state.navigation.browse_history.push(commit_id.clone());
        if repo_state.navigation.browse_history.len() > BROWSE_HISTORY_CAP {
            repo_state.navigation.browse_history.remove(0);
        }
    }
    state.sidebar_mode = SidebarMode::Files;
    set_file_browser_source(state, repo_id, FileSource::Commit(commit_id))
}

pub(super) fn reset_browse_to_live(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.navigation.browse_history.clear();
    }
    set_file_browser_source(state, repo_id, FileSource::WorkingDirectory)
}

/// The file shown as full content (if any), so a browse-point change can show
/// the same file at the new point. Never the editor: yanking an edit buffer
/// out from under the user is worse than a preview that lags the tree.
fn browse_open_content_path(repo: &RepoState) -> Option<PendingFileBrowserReopen> {
    if !repo.diff_state.content_preview || repo.diff_state.edit_mode {
        return None;
    }
    let path = match &repo.diff_state.diff_target {
        Some(gitcomet_core::domain::DiffTarget::Commit { path: Some(p), .. }) => p.clone(),
        Some(gitcomet_core::domain::DiffTarget::WorkingTree { path, .. }) => path.clone(),
        _ => return None,
    };
    Some(PendingFileBrowserReopen {
        path,
        diff_target_rev: repo.diff_state.diff_target_rev,
    })
}

enum ReopenDecision {
    Skip,
    Open,
    Close,
}

/// Settle the preview captured at retarget time now that the listing for
/// `source` is in: show the same file there, or close the view when the file
/// does not exist at that point.
fn reopen_after_retarget(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    source: FileSource,
    entries: &[FileEntry],
    reopen: PendingFileBrowserReopen,
) -> Vec<Effect> {
    let decision = {
        let Some(repo) = state.repos.iter().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        // The user moved on meanwhile: opened something else, closed the
        // view, or entered the editor.
        if repo.diff_state.diff_target_rev != reopen.diff_target_rev
            || repo.diff_state.edit_mode
            || !repo.diff_state.content_preview
        {
            ReopenDecision::Skip
        } else if !entries.iter().any(|entry| {
            entry.kind == gitcomet_core::domain::FileEntryKind::File
                && entry.path.as_path() == reopen.path.as_path()
        }) {
            ReopenDecision::Close
        } else if super::diff_selection::content_view_target(source.clone(), reopen.path.clone())
            == repo.diff_state.diff_target
        {
            // Already showing this file at this point (retarget bounced back).
            ReopenDecision::Skip
        } else {
            ReopenDecision::Open
        }
    };
    match decision {
        ReopenDecision::Skip => Vec::new(),
        ReopenDecision::Close => super::diff_selection::clear_diff_selection(state, repo_id),
        ReopenDecision::Open => {
            super::diff_selection::open_file_content(repos, state, repo_id, source, reopen.path)
        }
    }
}

pub(super) fn file_browser_loaded(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    source: FileSource,
    result: std::result::Result<Vec<FileEntry>, gitcomet_core::error::Error>,
) -> Vec<Effect> {
    let (has_pending, reopen) = {
        let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
            return Vec::new();
        };

        // Release the lane before the stale-source guard: a reply for a source the
        // user has already navigated away from still ends the walk that was running,
        // and the request queued behind it is the one that matters now.
        let has_pending = repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::FILE_BROWSER);

        let mut reopen = None;
        if repo_state.file_browser.source == source {
            let pending = repo_state.file_browser.pending_reopen.take();
            repo_state.file_browser.entries = match result {
                Ok(v) => {
                    let entries = Arc::new(v);
                    reopen = pending.map(|pending| (Arc::clone(&entries), pending));
                    Loadable::Ready(entries)
                }
                Err(e) => {
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    Loadable::Error(e.to_string())
                }
            };
            repo_state.file_browser.stale = false;
            repo_state.file_browser.bump_rev();
        }
        (has_pending, reopen)
    };

    let mut effects = Vec::new();
    if let Some((entries, pending)) = reopen {
        effects.extend(reopen_after_retarget(
            repos, state, repo_id, source, &entries, pending,
        ));
    }

    if has_pending && let Some(repo_state) = state.repos.iter().find(|r| r.id == repo_id) {
        effects.push(Effect::LoadFileBrowser {
            repo_id,
            source: repo_state.file_browser.source.clone(),
        });
    }
    effects
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
        let refresh_pending = repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::REMOTE_BRANCHES);
        let remote_mutation_in_flight =
            repo_state.pull_in_flight > 0 || repo_state.push_in_flight > 0;

        // Fetch/pull/prune watcher events can complete against a transient ref
        // namespace, and a coalesced request means this reply was superseded.
        // Keep the last complete snapshot until a post-command/latest load can
        // replace it atomically instead of flashing an intermediate list.
        if remote_mutation_in_flight || refresh_pending {
            if refresh_pending {
                effects.push(Effect::LoadRemoteBranches { repo_id });
            }
            return effects;
        }

        let branches = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_remote_branches(branches);
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
        let next = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_reflog(next);
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

    let selected_strs: FxHashSet<&str> = selected_ids.iter().map(|id| id.as_ref()).collect();

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
#[cfg(test)]
mod tests;
