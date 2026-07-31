use crate::model::{AppState, RepoId};
use crate::msg::{
    ConflictAutosolveMode, ConflictAutosolveStats, ConflictBulkChoice, ConflictRegionChoice,
    ConflictRegionResolutionUpdate, Effect, RepoPath,
};
use gitcomet_core::conflict_session::{
    AutosolveConfidence, AutosolveRule, ConflictRegion, ConflictRegionEditOutcome,
    ConflictRegionResolution, ConflictRegionSplitBoundaries, ConflictResolverStrategy,
    HistoryAutosolveOptions, RegexAutosolveOptions, join_conflict_regions_text,
    split_conflict_region_text,
};
use gitcomet_core::merge::{MergeSource, OrderedSelection};
use std::collections::BTreeMap;
use std::path::Path;

pub(super) fn set_hide_resolved(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    hide_resolved: bool,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    repo_state.set_conflict_hide_resolved(hide_resolved);
    Vec::new()
}

pub(super) fn apply_bulk_choice(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    choice: ConflictBulkChoice,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path.as_path() {
        return Vec::new();
    }

    let applied = apply_bulk_choice_to_session(session, choice);
    if applied > 0 {
        session.sync_merge_plan_from_regions();
        repo_state.bump_conflict_rev();
    }
    Vec::new()
}

pub(super) fn set_region_choice(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    choice: ConflictRegionChoice,
) -> Vec<Effect> {
    set_region_choice_inline(state, repo_id, path, region_index, choice);
    Vec::new()
}

pub(super) fn toggle_region_source(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    source: MergeSource,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path.as_path() {
        return Vec::new();
    }
    if session.toggle_region_source(region_index, source) {
        repo_state.bump_conflict_rev();
    }
    Vec::new()
}

pub(super) fn replace_region_selection(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    selection: OrderedSelection,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path.as_path() {
        return Vec::new();
    }
    if session.replace_region_selection(region_index, selection) {
        repo_state.bump_conflict_rev();
    }
    Vec::new()
}

#[inline]
pub(super) fn set_region_choice_inline(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    choice: ConflictRegionChoice,
) {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return;
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return;
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return;
    };
    if session.path != path.as_path() {
        return;
    }

    let Some(region) = session.regions.get_mut(region_index) else {
        return;
    };
    let Some(next_resolution) = (match choice {
        ConflictRegionChoice::Base => region
            .base
            .as_ref()
            .map(|_| ConflictRegionResolution::PickBase),
        ConflictRegionChoice::Ours => Some(ConflictRegionResolution::PickOurs),
        ConflictRegionChoice::Theirs => Some(ConflictRegionResolution::PickTheirs),
        ConflictRegionChoice::Both => Some(ConflictRegionResolution::PickBoth),
    }) else {
        return;
    };

    if region.resolution != next_resolution {
        region.resolution = next_resolution;
        session.sync_merge_plan_from_regions();
        repo_state.bump_conflict_rev();
    }
}

#[derive(Clone, Copy)]
enum ConflictRegionEdit {
    Split(ConflictRegionSplitBoundaries),
    Join,
}

#[derive(Clone)]
enum SplitResolutionCarry {
    Same(ConflictRegionResolution),
    Sources(OrderedSelection),
    Auto {
        rule: AutosolveRule,
        confidence: AutosolveConfidence,
        sources: OrderedSelection,
    },
}

fn region_text_for_selection(
    region: &ConflictRegion,
    selection: &OrderedSelection,
) -> Option<String> {
    let mut text = String::new();
    for source in selection.iter() {
        match (region.base.is_some(), source) {
            (true, MergeSource::A) => text.push_str(region.base.as_deref()?),
            (true, MergeSource::B) => text.push_str(region.ours.as_str()),
            (true, MergeSource::C) => text.push_str(region.theirs.as_str()),
            (false, MergeSource::A) => text.push_str(region.ours.as_str()),
            (false, MergeSource::B) => text.push_str(region.theirs.as_str()),
            (false, MergeSource::C) => return None,
        }
    }
    Some(text)
}

fn unique_selection_for_region_content(
    region: &ConflictRegion,
    content: &str,
) -> Option<OrderedSelection> {
    let sources: &[MergeSource] = if region.base.is_some() {
        &[MergeSource::A, MergeSource::B, MergeSource::C]
    } else {
        &[MergeSource::A, MergeSource::B]
    };
    let mut matches = Vec::new();
    for &first in sources {
        let single = OrderedSelection::from(first);
        if region_text_for_selection(region, &single).as_deref() == Some(content) {
            matches.push(single);
        }
        for &second in sources {
            if first == second {
                continue;
            }
            let pair = OrderedSelection::from_sources([first, second]);
            if region_text_for_selection(region, &pair).as_deref() == Some(content) {
                matches.push(pair);
            }
            for &third in sources {
                if third == first || third == second {
                    continue;
                }
                let triple = OrderedSelection::from_sources([first, second, third]);
                if region_text_for_selection(region, &triple).as_deref() == Some(content) {
                    matches.push(triple);
                }
            }
        }
    }
    (matches.len() == 1).then(|| matches.remove(0))
}

fn split_resolution_carry(region: &ConflictRegion) -> Option<SplitResolutionCarry> {
    match &region.resolution {
        ConflictRegionResolution::Unresolved
        | ConflictRegionResolution::PickBase
        | ConflictRegionResolution::PickOurs
        | ConflictRegionResolution::PickTheirs
        | ConflictRegionResolution::PickBoth
        | ConflictRegionResolution::Sources(_) => {
            Some(SplitResolutionCarry::Same(region.resolution.clone()))
        }
        ConflictRegionResolution::ManualEdit(content) => {
            unique_selection_for_region_content(region, content).map(SplitResolutionCarry::Sources)
        }
        ConflictRegionResolution::AutoResolved {
            rule,
            confidence,
            content,
        } => unique_selection_for_region_content(region, content).map(|sources| {
            SplitResolutionCarry::Auto {
                rule: *rule,
                confidence: *confidence,
                sources,
            }
        }),
    }
}

fn split_child_resolution(
    carry: &SplitResolutionCarry,
    child: &ConflictRegion,
) -> ConflictRegionResolution {
    match carry {
        SplitResolutionCarry::Same(resolution) => resolution.clone(),
        SplitResolutionCarry::Sources(selection) => {
            ConflictRegionResolution::Sources(selection.clone())
        }
        SplitResolutionCarry::Auto {
            rule,
            confidence,
            sources,
        } => ConflictRegionResolution::AutoResolved {
            rule: *rule,
            confidence: *confidence,
            content: region_text_for_selection(child, sources).unwrap_or_default(),
        },
    }
}

/// Rewrite conflict block `region_index` into 2–3 in-memory blocks at the
/// given block-local boundaries. Split parts inherit the source selection and
/// every other region keeps its resolution (indices shift past the split).
pub(super) fn split_region(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    boundaries: ConflictRegionSplitBoundaries,
    expected_conflict_rev: u64,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    if repo_state.conflict_state.conflict_rev != expected_conflict_rev {
        return Vec::new();
    }
    edit_regions(
        state,
        repo_id,
        path,
        region_index,
        ConflictRegionEdit::Split(boundaries),
    )
}

/// Merge conflict blocks `region_index` and `region_index + 1` in memory
/// (context between them is absorbed into every side). The joined region opens
/// unresolved and later regions shift down by one.
pub(super) fn join_regions(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    expected_conflict_rev: u64,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    if repo_state.conflict_state.conflict_rev != expected_conflict_rev {
        return Vec::new();
    }
    edit_regions(state, repo_id, path, region_index, ConflictRegionEdit::Join)
}

fn edit_regions(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    edit: ConflictRegionEdit,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path.as_path() {
        return Vec::new();
    }
    if session.strategy != ConflictResolverStrategy::FullTextResolver {
        return Vec::new();
    }
    let Some(current) = session.marker_projection.as_ref() else {
        return Vec::new();
    };
    let split_carry = match edit {
        ConflictRegionEdit::Split(_) => {
            let Some(region) = session.regions.get(region_index) else {
                return Vec::new();
            };
            let Some(carry) = split_resolution_carry(region) else {
                // Free-form output has no provable child ownership. Preserve
                // it intact instead of cloning or discarding it.
                return Vec::new();
            };
            Some(carry)
        }
        ConflictRegionEdit::Join => None,
    };

    let Some(ConflictRegionEditOutcome { new_text, parts }) = (match edit {
        ConflictRegionEdit::Split(boundaries) => {
            split_conflict_region_text(current, region_index, boundaries)
        }
        ConflictRegionEdit::Join => join_conflict_regions_text(current, region_index),
    }) else {
        return Vec::new();
    };

    // Explicit resolution carry-over: split parts preserve the original
    // selection; a joined block opens unresolved. Everything else keeps its
    // resolution at its shifted index.
    let old: Vec<ConflictRegionResolution> = session
        .regions
        .iter()
        .map(|region| region.resolution.clone())
        .collect();
    let old_plan_blocks = session.region_plan_blocks.clone();
    let shared: std::sync::Arc<str> = std::sync::Arc::from(new_text.as_str());
    session.marker_projection = Some(std::sync::Arc::clone(&shared));
    session.parse_regions_from_shared_text(std::sync::Arc::clone(&shared));
    if !old_plan_blocks.is_empty() {
        let reconciled = match edit {
            ConflictRegionEdit::Split(_) => {
                session.reconcile_merge_plan_after_split(&old_plan_blocks, region_index, parts)
            }
            ConflictRegionEdit::Join => {
                session.reconcile_merge_plan_after_join(&old_plan_blocks, region_index)
            }
        };
        if !reconciled {
            // An unusual independently-cut row cannot be represented as
            // contiguous aligned blocks. Keep the marker session correct and
            // avoid exposing a stale plan until the next stage refresh.
            session.merge_plan = None;
            session.region_plan_blocks.clear();
        }
    }
    for (ix, region) in session.regions.iter_mut().enumerate() {
        region.resolution = if ix < region_index {
            old.get(ix)
                .cloned()
                .unwrap_or(ConflictRegionResolution::Unresolved)
        } else if ix < region_index + parts {
            match edit {
                ConflictRegionEdit::Split(_) => split_carry
                    .as_ref()
                    .map(|carry| split_child_resolution(carry, region))
                    .unwrap_or(ConflictRegionResolution::Unresolved),
                ConflictRegionEdit::Join => ConflictRegionResolution::Unresolved,
            }
        } else {
            // Split consumed 1 old region for `parts` new ones; join consumed
            // 2 old regions for 1 new one.
            let consumed = match edit {
                ConflictRegionEdit::Split(_) => 1,
                ConflictRegionEdit::Join => 2,
            };
            old.get(ix - parts + consumed)
                .cloned()
                .unwrap_or(ConflictRegionResolution::Unresolved)
        };
    }
    session.sync_merge_plan_from_regions();
    session.has_pending_structural_edits = true;

    // The worktree is intentionally untouched until Save. The revision is
    // sufficient for embedded and focused views to rebuild from session text;
    // the pending-structure bit keeps that projection across same-path reloads.
    repo_state.bump_conflict_rev();
    Vec::new()
}

pub(super) fn sync_region_resolutions(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    updates: Vec<ConflictRegionResolutionUpdate>,
) -> Vec<Effect> {
    if updates.is_empty() {
        return Vec::new();
    }
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path.as_path() {
        return Vec::new();
    }

    let mut latest_by_region: BTreeMap<usize, ConflictRegionResolution> = BTreeMap::new();
    for update in updates {
        latest_by_region.insert(update.region_index, update.resolution);
    }

    let mut changed = 0usize;
    for (region_index, resolution) in latest_by_region {
        let Some(region) = session.regions.get_mut(region_index) else {
            continue;
        };
        if matches!(resolution, ConflictRegionResolution::PickBase) && region.base.is_none() {
            continue;
        }
        if region.resolution != resolution {
            region.resolution = resolution;
            changed += 1;
        }
    }

    if changed > 0 {
        session.sync_merge_plan_from_regions();
        repo_state.bump_conflict_rev();
    }
    Vec::new()
}

pub(super) fn apply_autosolve(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    mode: ConflictAutosolveMode,
    whitespace_normalize: bool,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return Vec::new();
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return Vec::new();
    };
    if session.path != path.as_path() {
        return Vec::new();
    }

    let stats = apply_autosolve_to_session(session, mode, whitespace_normalize);
    if stats.total_resolved() > 0 {
        session.sync_merge_plan_from_regions();
        repo_state.bump_conflict_rev();
    }
    Vec::new()
}

pub(super) fn reset_resolutions(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
) -> Vec<Effect> {
    reset_resolutions_inline(state, repo_id, path);
    Vec::new()
}

#[inline]
pub(super) fn reset_resolutions_inline(state: &mut AppState, repo_id: RepoId, path: RepoPath) {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return;
    };
    if !matches_current_conflict_path(repo_state, path.as_path()) {
        return;
    }
    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return;
    };
    if session.path != path.as_path() {
        return;
    }

    let reset_count = reset_session_resolutions(session);
    if reset_count > 0 {
        session.sync_merge_plan_from_regions();
        repo_state.bump_conflict_rev();
    }
}

fn matches_current_conflict_path(repo_state: &crate::model::RepoState, path: &Path) -> bool {
    repo_state.conflict_state.conflict_file_path.as_deref() == Some(path)
        || repo_state
            .conflict_state
            .conflict_session
            .as_ref()
            .is_some_and(|session| session.path.as_path() == path)
}

fn apply_bulk_choice_to_session(
    session: &mut gitcomet_core::conflict_session::ConflictSession,
    choice: ConflictBulkChoice,
) -> usize {
    let mut applied = 0usize;

    for region in &mut session.regions {
        if region.resolution.is_resolved() {
            continue;
        }
        let Some(next) = (match choice {
            ConflictBulkChoice::Base => region
                .base
                .as_ref()
                .map(|_| ConflictRegionResolution::PickBase),
            ConflictBulkChoice::Ours => Some(ConflictRegionResolution::PickOurs),
            ConflictBulkChoice::Theirs => Some(ConflictRegionResolution::PickTheirs),
            ConflictBulkChoice::Both => Some(ConflictRegionResolution::PickBoth),
        }) else {
            continue;
        };
        region.resolution = next;
        applied += 1;
    }

    applied
}

pub(super) fn apply_autosolve_to_session(
    session: &mut gitcomet_core::conflict_session::ConflictSession,
    mode: ConflictAutosolveMode,
    whitespace_normalize: bool,
) -> ConflictAutosolveStats {
    let mut stats = ConflictAutosolveStats::default();
    match mode {
        ConflictAutosolveMode::Safe | ConflictAutosolveMode::Regex => {
            stats.pass1 = session.auto_resolve_safe_with_options(whitespace_normalize);
            stats.pass2_split = session.auto_resolve_pass2();
            if stats.pass2_split > 0 {
                stats.pass1_after_split =
                    session.auto_resolve_safe_with_options(whitespace_normalize);
            }
            if mode == ConflictAutosolveMode::Regex {
                stats.regex =
                    session.auto_resolve_regex(&RegexAutosolveOptions::whitespace_insensitive());
            }
        }
        ConflictAutosolveMode::History => {
            stats.history = session.auto_resolve_history(&HistoryAutosolveOptions::bullet_list());
        }
    }
    stats
}

fn reset_session_resolutions(
    session: &mut gitcomet_core::conflict_session::ConflictSession,
) -> usize {
    let mut reset = 0usize;
    for region in &mut session.regions {
        if matches!(region.resolution, ConflictRegionResolution::Unresolved) {
            continue;
        }
        region.resolution = ConflictRegionResolution::Unresolved;
        reset += 1;
    }
    reset
}
