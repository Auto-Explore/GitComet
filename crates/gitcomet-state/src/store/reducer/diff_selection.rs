use super::actions_emit_effects::invalidate_loaded_blame;
use super::util::{
    SelectedConflictTarget, SelectedDiffLoadPlan, apply_selected_diff_load_plan_state,
    diff_target_preview_flags, selected_conflict_target, selected_diff_load_plan,
    start_conflict_target_reload, start_conflict_target_reload_with_mode,
    start_current_conflict_target_reload,
};
use crate::model::{
    AppState, ConflictFileLoadMode, DiagnosticKind, FileEditReturnView, InlineSubmoduleDiffEntry,
    InlineSubmoduleDiffSection, InlineSubmoduleDiffState, Loadable, RepoId, RepoState,
    ViewHistoryEntry,
};
use crate::msg::Effect;
use gitcomet_core::domain::{
    Diff, DiffArea, DiffPreviewTextFile, DiffPreviewTextSide, DiffTarget, FileDiffImage,
    FileDiffText, SubmoduleDiffRange, SubmoduleDiffSummary,
};
use gitcomet_core::error::Error;
use smallvec::SmallVec;
use std::sync::Arc;

pub(crate) const SELECT_DIFF_INLINE_EFFECT_CAPACITY: usize = 3;
pub(crate) type SelectDiffEffects = SmallVec<[Effect; SELECT_DIFF_INLINE_EFFECT_CAPACITY]>;

fn clear_inline_submodule_diff_state(
    repo_state: &mut RepoState,
) -> Option<InlineSubmoduleDiffState> {
    repo_state.diff_state.inline_submodule_diff.take()
}

fn next_inline_submodule_diff_rev(repo_state: &mut RepoState) -> u64 {
    let rev = repo_state
        .diff_state
        .inline_submodule_diff_rev
        .wrapping_add(1);
    repo_state.diff_state.inline_submodule_diff_rev = rev;
    rev
}

fn inline_submodule_entries_from_range(
    range: &SubmoduleDiffRange,
) -> impl Iterator<Item = InlineSubmoduleDiffEntry> + '_ {
    let range_commits = match (range.from.as_ref(), range.to.as_ref()) {
        (Some(from_commit_id), Some(to_commit_id)) => Some((from_commit_id, to_commit_id)),
        _ => None,
    };

    range.changes.iter().filter_map(move |change| {
        let (from_commit_id, to_commit_id) = range_commits.as_ref()?;
        Some(InlineSubmoduleDiffEntry {
            path: change.path.clone(),
            kind: change.kind,
            target: DiffTarget::CommitRange {
                from_commit_id: (*from_commit_id).clone(),
                to_commit_id: Some((*to_commit_id).clone()),
                path: Some(change.path.clone()),
            },
            section: InlineSubmoduleDiffSection::Range(range.kind),
        })
    })
}

fn inline_submodule_entries_from_summary(
    summary: &SubmoduleDiffSummary,
) -> Vec<InlineSubmoduleDiffEntry> {
    let mut entries = Vec::new();
    for range in &summary.ranges {
        entries.extend(inline_submodule_entries_from_range(range));
    }
    entries.extend(
        summary
            .live_staged
            .iter()
            .map(|change| InlineSubmoduleDiffEntry {
                path: change.path.clone(),
                kind: change.kind,
                target: DiffTarget::WorkingTree {
                    path: change.path.clone(),
                    area: DiffArea::Staged,
                },
                section: InlineSubmoduleDiffSection::LiveStaged,
            }),
    );
    entries.extend(
        summary
            .live_unstaged
            .iter()
            .map(|change| InlineSubmoduleDiffEntry {
                path: change.path.clone(),
                kind: change.kind,
                target: DiffTarget::WorkingTree {
                    path: change.path.clone(),
                    area: DiffArea::Unstaged,
                },
                section: InlineSubmoduleDiffSection::LiveUnstaged,
            }),
    );
    entries
}

fn inline_submodule_entry_index(
    entries: &[InlineSubmoduleDiffEntry],
    target: &DiffTarget,
) -> Option<usize> {
    entries.iter().position(|entry| &entry.target == target)
}

fn inline_submodule_selected_diff_load_plan(target: &DiffTarget) -> SelectedDiffLoadPlan {
    let supports_file = matches!(
        target,
        DiffTarget::WorkingTree { .. }
            | DiffTarget::Commit { path: Some(_), .. }
            | DiffTarget::CommitRange { path: Some(_), .. }
    );
    let preview = diff_target_preview_flags(target);

    SelectedDiffLoadPlan {
        load_patch_diff: true,
        load_file_text: supports_file && (!preview.wants_image || preview.is_svg),
        preview_text_side: None,
        load_submodule_summary: false,
        load_file_image: supports_file && preview.wants_image,
    }
}

fn apply_inline_submodule_diff_load_plan_state(
    inline: &mut InlineSubmoduleDiffState,
    load_plan: SelectedDiffLoadPlan,
) {
    inline.diff_rev = 0;
    inline.diff = if load_plan.load_patch_diff {
        Loadable::Loading
    } else {
        Loadable::NotLoaded
    };
    inline.diff_file_rev = 0;
    inline.diff_file = if load_plan.load_file_text {
        Loadable::Loading
    } else {
        Loadable::NotLoaded
    };
    inline.diff_file_image = if load_plan.load_file_image {
        Loadable::Loading
    } else {
        Loadable::NotLoaded
    };
}

fn push_inline_submodule_diff_load_effects(
    repo_id: RepoId,
    inline_rev: u64,
    load_plan: SelectedDiffLoadPlan,
    effects: &mut SelectDiffEffects,
) {
    if load_plan.load_patch_diff {
        effects.push(Effect::LoadInlineSubmoduleSelectedDiff {
            repo_id,
            inline_rev,
        });
    }
    if load_plan.load_file_text {
        effects.push(Effect::LoadInlineSubmoduleSelectedDiffFile {
            repo_id,
            inline_rev,
        });
    }
    if load_plan.load_file_image {
        effects.push(Effect::LoadInlineSubmoduleSelectedDiffFileImage {
            repo_id,
            inline_rev,
        });
    }
}

/// What the main content pane shows for a freshly selected target. Passed
/// through every selection path so no route can leave the two view flags on
/// `DiffState` disagreeing — in particular so a plain diff selection always
/// leaves edit mode behind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ContentViewMode {
    /// A diff of the target (or the conflict resolver, when it is conflicted).
    Diff,
    /// The whole file, syntax highlighted, read-only.
    Preview,
    /// The whole file, editable. Only reachable for `WorkingTree` targets.
    Edit,
}

impl ContentViewMode {
    /// Both file-content modes render the file rather than a patch.
    fn is_content_view(self) -> bool {
        matches!(self, Self::Preview | Self::Edit)
    }
}

pub(super) fn select_diff(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
) -> Vec<Effect> {
    let mut effects = SelectDiffEffects::new();
    fill_select_diff_inline(state, repo_id, target, ContentViewMode::Diff, &mut effects);
    effects.into_vec()
}

/// Open `source`/`path` as a full-content file preview in the main pane,
/// reusing the added/removed-file preview renderer (no diff, no green/red).
pub(super) fn open_file_content(
    state: &mut AppState,
    repo_id: RepoId,
    source: gitcomet_core::domain::FileSource,
    path: std::path::PathBuf,
) -> Vec<Effect> {
    let Some(target) = content_view_target(source.clone(), path.clone()) else {
        return Vec::new();
    };
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state
            .view_history
            .record(ViewHistoryEntry { source, path });
    }
    let mut effects = SelectDiffEffects::new();
    fill_select_diff_inline(
        state,
        repo_id,
        target,
        ContentViewMode::Preview,
        &mut effects,
    );
    effects.into_vec()
}

/// Open `path` as an editable buffer over the file on disk.
///
/// `source` only decides which file-version history entry is recorded — the
/// target is always the working tree, because the editor edits the workspace
/// copy even when the action was invoked from a commit's file list. Returns no
/// effects for sources that have no working-tree file.
pub(super) fn open_file_editor(
    state: &mut AppState,
    repo_id: RepoId,
    path: std::path::PathBuf,
) -> Vec<Effect> {
    let return_view = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| {
            if repo.diff_state.edit_mode {
                return repo.diff_state.edit_return_view.clone();
            }
            repo.diff_state
                .diff_target
                .clone()
                .map(|target| FileEditReturnView {
                    target,
                    content_preview: repo.diff_state.content_preview,
                })
        });
    let target = DiffTarget::WorkingTree {
        path: path.clone(),
        area: DiffArea::Unstaged,
    };
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.view_history.record(ViewHistoryEntry {
            source: gitcomet_core::domain::FileSource::WorkingDirectory,
            path,
        });
    }
    let mut effects = SelectDiffEffects::new();
    fill_select_diff_inline(state, repo_id, target, ContentViewMode::Edit, &mut effects);
    if let Some(repo_state) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
        repo_state.diff_state.edit_return_view = return_view;
    }
    effects.into_vec()
}

/// Leave the editor and reload the view that was behind it.
///
/// Restores the diff or read-only content preview that opened the editor. When
/// there is no recorded origin (for example a restored legacy session), it
/// falls back to the read-only working-tree preview of the edited file.
pub(super) fn exit_diff_edit_mode(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !repo_state.diff_state.edit_mode {
        return Vec::new();
    }
    let return_view = repo_state.diff_state.edit_return_view.take();
    let fallback_target = repo_state.diff_state.diff_target.clone();

    let (target, mode) = match return_view {
        Some(return_view) => (
            Some(return_view.target),
            if return_view.content_preview {
                ContentViewMode::Preview
            } else {
                ContentViewMode::Diff
            },
        ),
        None => (fallback_target, ContentViewMode::Preview),
    };
    let Some(target) = target else {
        let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        repo_state.diff_state.edit_mode = false;
        repo_state.diff_state.content_preview = true;
        repo_state.bump_diff_state_rev();
        return Vec::new();
    };

    if mode == ContentViewMode::Preview
        && let Some(entry) = view_history_entry_for_target(&target)
        && let Some(repo_state) = state.repos.iter_mut().find(|repo| repo.id == repo_id)
    {
        repo_state.view_history.seek_or_record(entry);
    }

    let mut effects = SelectDiffEffects::new();
    fill_select_diff_inline(state, repo_id, target, mode, &mut effects);
    effects.into_vec()
}

/// Map a `(source, path)` content view to its `DiffTarget`. Returns `None` for
/// the unwired `Branch` source.
fn content_view_target(
    source: gitcomet_core::domain::FileSource,
    path: std::path::PathBuf,
) -> Option<DiffTarget> {
    match source {
        gitcomet_core::domain::FileSource::WorkingDirectory => Some(DiffTarget::WorkingTree {
            path,
            area: DiffArea::Unstaged,
        }),
        gitcomet_core::domain::FileSource::Commit(commit_id) => Some(DiffTarget::Commit {
            commit_id,
            path: Some(path),
        }),
        // Branch file listing is not wired, so this is unreachable from the UI.
        gitcomet_core::domain::FileSource::Branch(_) => None,
    }
}

/// Reverse of [`content_view_target`]: the [`ViewHistoryEntry`] a file-content
/// `target` corresponds to, or `None` when `target` is not a file-content view
/// (range/full-tree diffs). Used to realign the viewer's file-version history
/// when a global navigation restores a file-content view.
fn view_history_entry_for_target(target: &DiffTarget) -> Option<ViewHistoryEntry> {
    match target {
        DiffTarget::Commit {
            commit_id,
            path: Some(path),
        } => Some(ViewHistoryEntry {
            source: gitcomet_core::domain::FileSource::Commit(commit_id.clone()),
            path: path.clone(),
        }),
        DiffTarget::WorkingTree {
            path,
            area: DiffArea::Unstaged,
        } => Some(ViewHistoryEntry {
            source: gitcomet_core::domain::FileSource::WorkingDirectory,
            path: path.clone(),
        }),
        _ => None,
    }
}

/// Step the viewer's back/forward history and replay the resulting view without
/// recording it (so navigation doesn't mutate the stack).
pub(super) fn viewer_nav(
    state: &mut AppState,
    repo_id: RepoId,
    dir: crate::model::ViewNavDir,
) -> Vec<Effect> {
    let target = {
        let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        let Some(entry) = repo_state.view_history.step(dir) else {
            return Vec::new();
        };
        content_view_target(entry.source, entry.path)
    };
    let Some(target) = target else {
        return Vec::new();
    };
    let mut effects = SelectDiffEffects::new();
    fill_select_diff_inline(
        state,
        repo_id,
        target,
        ContentViewMode::Preview,
        &mut effects,
    );
    effects.into_vec()
}

/// Step the broad global navigation history and restore the resulting main-view
/// snapshot (diff/file content, history log, and/or commit selection) without
/// recording it as a new destination.
pub(super) fn global_nav(
    state: &mut AppState,
    repo_id: RepoId,
    dir: crate::model::ViewNavDir,
) -> Vec<Effect> {
    let snapshot = {
        let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        repo_state.nav_history.step(dir)
    };
    let Some(snapshot) = snapshot else {
        return Vec::new();
    };

    let mut effects: Vec<Effect> = Vec::new();

    // Restore the selected commit (history-log selection / commit details).
    match snapshot.selected_commit {
        Some(commit_id) => {
            let sel_effects = super::effects::select_commit(state, repo_id, commit_id.clone());
            // `select_commit` no-ops when this commit is already selected, but a
            // prior nav step or a cancelled load may have left its details
            // unloaded. Reload unless the details already shown are for this
            // exact commit. We deliberately do NOT skip merely because a load is
            // in flight: that load may be for a *different* commit (a stale or
            // cancelled select) whose result the id-guard will drop, which would
            // otherwise leave the details pane stuck Loading forever. A redundant
            // load for the same commit is cheap — this runs once per nav step —
            // and idempotent.
            let select_was_noop = sel_effects.is_empty();
            effects.extend(sel_effects);
            if select_was_noop
                && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
            {
                let needs_load = !matches!(
                    &repo_state.history_state.commit_details,
                    Loadable::Ready(details) if details.id == commit_id
                );
                if needs_load {
                    repo_state.set_commit_details(Loadable::NotLoaded);
                    effects.push(Effect::LoadCommitDetails { repo_id, commit_id });
                }
            }
        }
        None => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.set_selected_commit(None);
                repo_state.set_commit_details(Loadable::NotLoaded);
            }
        }
    }

    // Restore the two-point comparison. This has to run before the diff-target
    // restore below: entering a comparison clears the diff pane, so doing it
    // afterwards would wipe the very target this step is meant to show.
    //
    // The multi-selection that may have started the comparison is not part of
    // the snapshot, so `compare_range` drops it and the restored view names
    // itself after the endpoints instead of "N commits selected". The files it
    // lists are the same either way — the endpoints are what the comparison is.
    let restore_range = {
        let Some(repo_state) = state.repos.iter().find(|r| r.id == repo_id) else {
            return effects;
        };
        repo_state.history_state.range_selection != snapshot.range_selection
    };
    if restore_range {
        match snapshot.range_selection {
            Some(range) => effects.extend(super::effects::compare_range(
                state,
                repo_id,
                range.from,
                range.to,
                range.from_label,
                range.to_label,
                super::effects::ComparisonSource::Explicit,
            )),
            None => {
                if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                    repo_state.clear_range_comparison();
                }
            }
        }
    }

    // Restore the main content target: a diff/file view, or the history log.
    match snapshot.diff_target {
        Some(target) => {
            // When this global step lands on a file-content view, realign the
            // viewer's file-version history (a separate stack) onto the file now
            // shown, so its prev/next-version buttons step relative to it instead
            // of a stale cursor left from earlier file opens.
            if snapshot.content_preview
                && let Some(entry) = view_history_entry_for_target(&target)
                && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
            {
                repo_state.view_history.seek_or_record(entry);
            }
            let mut inline = SelectDiffEffects::new();
            // Edit mode is part of the destination, so a step can land in the
            // editor and a step away can leave it. `edit_mode` is only ever set
            // together with `content_preview`, so it is checked first.
            let mode = if snapshot.edit_mode {
                ContentViewMode::Edit
            } else if snapshot.content_preview {
                ContentViewMode::Preview
            } else {
                ContentViewMode::Diff
            };
            fill_select_diff_inline(state, repo_id, target, mode, &mut inline);
            effects.extend(inline.into_vec());
        }
        None => {
            effects.extend(clear_diff_selection(state, repo_id));
        }
    }

    effects
}

pub(super) fn fill_select_diff_inline(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    mode: ContentViewMode,
    effects: &mut SelectDiffEffects,
) {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return;
    };

    let content_preview = mode.is_content_view();
    clear_inline_submodule_diff_state(repo_state);
    repo_state.diff_state.content_preview = content_preview;
    repo_state.diff_state.edit_mode = mode == ContentViewMode::Edit;
    if mode != ContentViewMode::Edit {
        repo_state.diff_state.edit_return_view = None;
    }

    if !content_preview && let Some(conflict_target) = selected_conflict_target(repo_state, &target)
    {
        repo_state.set_diff_target(Some(target.clone()));
        repo_state.diff_state.diff = Loadable::NotLoaded;
        repo_state.diff_state.diff_file = Loadable::NotLoaded;
        repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
        repo_state.diff_state.submodule_summary = Loadable::NotLoaded;
        repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
        repo_state.bump_diff_state_rev();
        let conflict_effects = match conflict_target {
            SelectedConflictTarget::Current => start_current_conflict_target_reload(repo_state),
            SelectedConflictTarget::Path(path) => start_conflict_target_reload(repo_state, path),
        };
        debug_assert!(conflict_effects.len() <= SELECT_DIFF_INLINE_EFFECT_CAPACITY);
        effects.extend(conflict_effects);
        return;
    }

    repo_state.set_diff_target(Some(target));
    let load_plan = {
        let target = repo_state
            .diff_state
            .diff_target
            .as_ref()
            .expect("diff target set before load planning");
        selected_diff_load_plan(repo_state, target)
    };
    apply_selected_diff_load_plan_state(repo_state, load_plan);
    repo_state.bump_diff_state_rev();

    effects.push(Effect::LoadSelectedDiff {
        repo_id,
        load_patch_diff: load_plan.load_patch_diff,
        load_file_text: load_plan.load_file_text,
        preview_text_side: load_plan.preview_text_side,
        load_submodule_summary: load_plan.load_submodule_summary,
        load_file_image: load_plan.load_file_image,
    });
}

pub(super) fn select_conflict_diff(
    state: &mut AppState,
    repo_id: RepoId,
    path: std::path::PathBuf,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    clear_inline_submodule_diff_state(repo_state);
    repo_state.diff_state.content_preview = false;
    repo_state.diff_state.edit_mode = false;
    repo_state.diff_state.edit_return_view = None;

    let target = DiffTarget::WorkingTree {
        path: path.clone(),
        area: DiffArea::Unstaged,
    };
    repo_state.set_diff_target(Some(target));
    repo_state.diff_state.diff = Loadable::NotLoaded;
    repo_state.diff_state.diff_file = Loadable::NotLoaded;
    repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
    repo_state.diff_state.submodule_summary = Loadable::NotLoaded;
    repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
    repo_state.bump_diff_state_rev();

    start_conflict_target_reload_with_mode(repo_state, &path, ConflictFileLoadMode::CurrentOnly)
}

pub(super) fn clear_diff_selection(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    clear_inline_submodule_diff_state(repo_state);
    repo_state.diff_state.content_preview = false;
    repo_state.diff_state.edit_mode = false;
    repo_state.diff_state.edit_return_view = None;

    repo_state.set_diff_target(None);
    repo_state.diff_state.diff = Loadable::NotLoaded;
    repo_state.diff_state.diff_file = Loadable::NotLoaded;
    repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
    repo_state.diff_state.submodule_summary = Loadable::NotLoaded;
    repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
    repo_state.bump_diff_state_rev();
    Vec::new()
}

pub(super) fn open_inline_submodule_diff(
    state: &mut AppState,
    repo_id: RepoId,
    submodule_repo_path: std::path::PathBuf,
    parent_submodule_path: std::path::PathBuf,
    entries: Vec<InlineSubmoduleDiffEntry>,
    selected_ix: usize,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if selected_ix >= entries.len() {
        return Vec::new();
    }

    let target = entries[selected_ix].target.clone();
    let load_plan = inline_submodule_selected_diff_load_plan(&target);
    let rev = next_inline_submodule_diff_rev(repo_state);
    repo_state.diff_state.inline_submodule_diff = Some(InlineSubmoduleDiffState {
        submodule_repo_path,
        parent_submodule_path,
        entries,
        selected_ix,
        target,
        rev,
        diff_rev: 0,
        diff: if load_plan.load_patch_diff {
            Loadable::Loading
        } else {
            Loadable::NotLoaded
        },
        diff_file_rev: 0,
        diff_file: if load_plan.load_file_text {
            Loadable::Loading
        } else {
            Loadable::NotLoaded
        },
        diff_file_image: if load_plan.load_file_image {
            Loadable::Loading
        } else {
            Loadable::NotLoaded
        },
    });
    repo_state.bump_diff_state_rev();

    let mut effects = SelectDiffEffects::new();
    push_inline_submodule_diff_load_effects(repo_id, rev, load_plan, &mut effects);
    effects.into_vec()
}

pub(super) fn select_inline_submodule_diff(
    state: &mut AppState,
    repo_id: RepoId,
    selected_ix: usize,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let next_target = {
        let Some(inline) = repo_state.diff_state.inline_submodule_diff.as_ref() else {
            return Vec::new();
        };
        if selected_ix >= inline.entries.len() {
            return Vec::new();
        }

        let next_target = inline.entries[selected_ix].target.clone();
        if inline.selected_ix == selected_ix && inline.target == next_target {
            return Vec::new();
        }
        next_target
    };

    let inline_rev = next_inline_submodule_diff_rev(repo_state);
    let load_plan = {
        let Some(inline) = repo_state.diff_state.inline_submodule_diff.as_mut() else {
            return Vec::new();
        };
        inline.selected_ix = selected_ix;
        inline.target = next_target;
        inline.rev = inline_rev;
        let next_load_plan = inline_submodule_selected_diff_load_plan(&inline.target);
        apply_inline_submodule_diff_load_plan_state(inline, next_load_plan);
        next_load_plan
    };
    repo_state.bump_diff_state_rev();

    let mut effects = SelectDiffEffects::new();
    push_inline_submodule_diff_load_effects(repo_id, inline_rev, load_plan, &mut effects);
    effects.into_vec()
}

pub(super) fn close_inline_submodule_diff(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if clear_inline_submodule_diff_state(repo_state).is_some() {
        repo_state.bump_diff_state_rev();
    }
    Vec::new()
}

pub(super) fn stage_hunk(repo_id: RepoId, patch: String) -> Vec<Effect> {
    vec![Effect::StageHunk { repo_id, patch }]
}

pub(super) fn unstage_hunk(repo_id: RepoId, patch: String) -> Vec<Effect> {
    vec![Effect::UnstageHunk { repo_id, patch }]
}

pub(super) fn apply_worktree_patch(repo_id: RepoId, patch: String, reverse: bool) -> Vec<Effect> {
    vec![Effect::ApplyWorktreePatch {
        repo_id,
        patch,
        reverse,
    }]
}

pub(super) fn diff_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    result: std::result::Result<Diff, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.diff_state.diff_target.as_ref() == Some(&target)
    {
        // The reload this was waiting for has arrived, whatever it carries and
        // whichever branch below claims it: the rows on screen are about to stop
        // being a generation behind the index.
        repo_state.diff_state.diff_reload_in_flight = false;

        if selected_conflict_target(repo_state, &target).is_some() {
            return Vec::new();
        }
        let current_plan = selected_diff_load_plan(repo_state, &target);
        if !current_plan.load_patch_diff {
            return Vec::new();
        }
        match result {
            // A refresh that found no change must not churn the UI: keep the
            // existing `Arc` so pointer-identity fingerprints stay put, leave
            // the revs alone, and leave blame painted. `Loading`/`Error`/
            // `NotLoaded` → `Ready` always falls through to the bump below, so
            // a freshly selected diff is unaffected.
            Ok(v) if matches!(&repo_state.diff_state.diff, Loadable::Ready(cur) if **cur == v) => {}
            Ok(v) => {
                repo_state.diff_state.diff_rev = repo_state.diff_state.diff_rev.wrapping_add(1);
                repo_state.diff_state.diff = Loadable::Ready(Arc::new(v));
                repo_state.bump_diff_state_rev();
                // The annotation column is derived from the content the diff
                // shows, so blame is stale exactly when that content changed.
                invalidate_loaded_blame(repo_state);
            }
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.diff_state.diff_rev = repo_state.diff_state.diff_rev.wrapping_add(1);
                repo_state.diff_state.diff = Loadable::Error(e.to_string());
                repo_state.bump_diff_state_rev();
            }
        }
    }
    Vec::new()
}

pub(super) fn diff_file_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    result: std::result::Result<Option<FileDiffText>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.diff_state.diff_target.as_ref() == Some(&target)
    {
        if selected_conflict_target(repo_state, &target).is_some() {
            return Vec::new();
        }
        let current_plan = selected_diff_load_plan(repo_state, &target);
        if !current_plan.load_file_text {
            return Vec::new();
        }
        // Same content-driven guard as `diff_loaded`. Required, not merely
        // symmetric: in content-preview mode `load_patch_diff` is false and
        // `diff_loaded` returns early above, so `diff_file` is the only signal
        // that the shown content changed.
        match result {
            Ok(v)
                if matches!(&repo_state.diff_state.diff_file, Loadable::Ready(cur)
                    if cur.as_deref() == v.as_ref()) => {}
            Ok(v) => {
                repo_state.diff_state.diff_file_rev =
                    repo_state.diff_state.diff_file_rev.wrapping_add(1);
                repo_state.diff_state.diff_file = Loadable::Ready(v.map(Arc::new));
                repo_state.bump_diff_state_rev();
                invalidate_loaded_blame(repo_state);
            }
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.diff_state.diff_file_rev =
                    repo_state.diff_state.diff_file_rev.wrapping_add(1);
                repo_state.diff_state.diff_file = Loadable::Error(e.to_string());
                repo_state.bump_diff_state_rev();
            }
        }
    }
    Vec::new()
}

pub(super) fn diff_file_image_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    result: std::result::Result<Option<FileDiffImage>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.diff_state.diff_target.as_ref() == Some(&target)
    {
        if selected_conflict_target(repo_state, &target).is_some() {
            return Vec::new();
        }
        let current_plan = selected_diff_load_plan(repo_state, &target);
        if !current_plan.load_file_image {
            return Vec::new();
        }
        let content_preview = repo_state.diff_state.content_preview;
        repo_state.diff_state.diff_file_rev = repo_state.diff_state.diff_file_rev.wrapping_add(1);
        repo_state.diff_state.diff_file_image = match result {
            Ok(v) => {
                let v = v.map(|mut image| {
                    // The content view shows the file itself, not a before/after
                    // diff — drop the old side so it renders as a single image.
                    if content_preview {
                        image.old = None;
                    }
                    image
                });
                Loadable::Ready(v.map(Arc::new))
            }
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.bump_diff_state_rev();
    }
    Vec::new()
}

pub(super) fn diff_preview_text_file_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    side: DiffPreviewTextSide,
    result: std::result::Result<Option<std::path::PathBuf>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.diff_state.diff_target.as_ref() == Some(&target)
    {
        let current_plan = selected_diff_load_plan(repo_state, &target);
        if current_plan.preview_text_side != Some(side) {
            return Vec::new();
        }

        repo_state.diff_state.diff_preview_text_file_rev = repo_state
            .diff_state
            .diff_preview_text_file_rev
            .wrapping_add(1);
        repo_state.diff_state.diff_preview_text_file = match result {
            Ok(path) => {
                Loadable::Ready(path.map(|path| Arc::new(DiffPreviewTextFile { path, side })))
            }
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.bump_diff_state_rev();
    }
    Vec::new()
}

pub(super) fn submodule_summary_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    result: std::result::Result<SubmoduleDiffSummary, Error>,
) -> Vec<Effect> {
    let mut effects = SelectDiffEffects::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        if repo_state.diff_state.diff_target.as_ref() != Some(&target) {
            return Vec::new();
        }

        let current_plan = repo_state
            .diff_state
            .diff_target
            .as_ref()
            .map(|target| selected_diff_load_plan(repo_state, target))
            .unwrap_or_default();
        if !current_plan.load_submodule_summary {
            return Vec::new();
        }

        repo_state.diff_state.submodule_summary_rev =
            repo_state.diff_state.submodule_summary_rev.wrapping_add(1);
        repo_state.diff_state.submodule_summary = match result {
            Ok(summary) => {
                let next_entries = inline_submodule_entries_from_summary(&summary);
                let had_inline = repo_state.diff_state.inline_submodule_diff.is_some();
                let selected_inline = repo_state
                    .diff_state
                    .inline_submodule_diff
                    .as_ref()
                    .and_then(|inline| {
                        inline_submodule_entry_index(next_entries.as_slice(), &inline.target)
                            .map(|selected_ix| (selected_ix, inline.target.clone()))
                    });

                if let Some((selected_ix, inline_target)) = selected_inline {
                    let load_plan = inline_submodule_selected_diff_load_plan(&inline_target);
                    let inline_rev = next_inline_submodule_diff_rev(repo_state);
                    if let Some(inline) = repo_state.diff_state.inline_submodule_diff.as_mut() {
                        inline.entries = next_entries;
                        inline.selected_ix = selected_ix;
                        inline.target = inline_target;
                        inline.rev = inline_rev;
                        apply_inline_submodule_diff_load_plan_state(inline, load_plan);
                    }
                    push_inline_submodule_diff_load_effects(
                        repo_id,
                        inline_rev,
                        load_plan,
                        &mut effects,
                    );
                } else if had_inline {
                    repo_state.diff_state.inline_submodule_diff = None;
                }
                Loadable::Ready(Arc::new(summary))
            }
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.bump_diff_state_rev();
    }
    effects.into_vec()
}

pub(super) fn inline_submodule_diff_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    inline_rev: u64,
    target: DiffTarget,
    result: std::result::Result<Diff, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let (next_diff, diagnostic) = match result {
            Ok(diff) => (Loadable::Ready(Arc::new(diff)), None),
            Err(e) => {
                let message = e.to_string();
                (Loadable::Error(message.clone()), Some(message))
            }
        };
        {
            let Some(inline) = repo_state.diff_state.inline_submodule_diff.as_mut() else {
                return Vec::new();
            };
            if inline.rev != inline_rev || inline.target != target {
                return Vec::new();
            }
            inline.diff_rev = inline.diff_rev.wrapping_add(1);
            inline.diff = next_diff;
        }
        if let Some(message) = diagnostic {
            super::util::push_diagnostic(repo_state, DiagnosticKind::Error, message);
        }
        repo_state.bump_diff_state_rev();
    }
    Vec::new()
}

pub(super) fn inline_submodule_diff_file_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    inline_rev: u64,
    target: DiffTarget,
    result: std::result::Result<Option<FileDiffText>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let (next_diff_file, diagnostic) = match result {
            Ok(file) => (Loadable::Ready(file.map(Arc::new)), None),
            Err(e) => {
                let message = e.to_string();
                (Loadable::Error(message.clone()), Some(message))
            }
        };
        {
            let Some(inline) = repo_state.diff_state.inline_submodule_diff.as_mut() else {
                return Vec::new();
            };
            if inline.rev != inline_rev || inline.target != target {
                return Vec::new();
            }
            inline.diff_file_rev = inline.diff_file_rev.wrapping_add(1);
            inline.diff_file = next_diff_file;
        }
        if let Some(message) = diagnostic {
            super::util::push_diagnostic(repo_state, DiagnosticKind::Error, message);
        }
        repo_state.bump_diff_state_rev();
    }
    Vec::new()
}

pub(super) fn inline_submodule_diff_file_image_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    inline_rev: u64,
    target: DiffTarget,
    result: std::result::Result<Option<FileDiffImage>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let (next_diff_image, diagnostic) = match result {
            Ok(file) => (Loadable::Ready(file.map(Arc::new)), None),
            Err(e) => {
                let message = e.to_string();
                (Loadable::Error(message.clone()), Some(message))
            }
        };
        {
            let Some(inline) = repo_state.diff_state.inline_submodule_diff.as_mut() else {
                return Vec::new();
            };
            if inline.rev != inline_rev || inline.target != target {
                return Vec::new();
            }
            inline.diff_file_rev = inline.diff_file_rev.wrapping_add(1);
            inline.diff_file_image = next_diff_image;
        }
        if let Some(message) = diagnostic {
            super::util::push_diagnostic(repo_state, DiagnosticKind::Error, message);
        }
        repo_state.bump_diff_state_rev();
    }
    Vec::new()
}
