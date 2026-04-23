use super::util::{
    SelectedConflictTarget, SelectedDiffLoadPlan, apply_selected_diff_load_plan_state,
    diff_target_preview_flags, selected_conflict_target, selected_diff_load_plan,
    start_conflict_target_reload, start_conflict_target_reload_with_mode,
    start_current_conflict_target_reload,
};
use crate::model::{
    AppState, ConflictFileLoadMode, DiagnosticKind, InlineSubmoduleDiffEntry,
    InlineSubmoduleDiffSection, InlineSubmoduleDiffState, Loadable, RepoId,
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
    repo_state: &mut crate::model::RepoState,
) -> Option<InlineSubmoduleDiffState> {
    repo_state.diff_state.inline_submodule_diff.take()
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
                to_commit_id: (*to_commit_id).clone(),
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

pub(super) fn select_diff(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
) -> Vec<Effect> {
    let mut effects = SelectDiffEffects::new();
    fill_select_diff_inline(state, repo_id, target, &mut effects);
    effects.into_vec()
}

pub(super) fn fill_select_diff_inline(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    effects: &mut SelectDiffEffects,
) {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return;
    };

    clear_inline_submodule_diff_state(repo_state);

    if let Some(conflict_target) = selected_conflict_target(repo_state, &target) {
        repo_state.diff_state.diff_target = Some(target.clone());
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

    repo_state.diff_state.diff_target = Some(target);
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

    let target = DiffTarget::WorkingTree {
        path: path.clone(),
        area: DiffArea::Unstaged,
    };
    repo_state.diff_state.diff_target = Some(target);
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

    repo_state.diff_state.diff_target = None;
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
    let rev = repo_state
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .map_or(1, |inline| inline.rev.wrapping_add(1));
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
    let (inline_rev, load_plan) = {
        let Some(inline) = repo_state.diff_state.inline_submodule_diff.as_mut() else {
            return Vec::new();
        };
        if selected_ix >= inline.entries.len() {
            return Vec::new();
        }

        let next_target = inline.entries[selected_ix].target.clone();
        if inline.selected_ix == selected_ix && inline.target == next_target {
            return Vec::new();
        }

        inline.selected_ix = selected_ix;
        inline.target = next_target;
        inline.rev = inline.rev.wrapping_add(1);
        let next_load_plan = inline_submodule_selected_diff_load_plan(&inline.target);
        apply_inline_submodule_diff_load_plan_state(inline, next_load_plan);
        (inline.rev, next_load_plan)
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
        if selected_conflict_target(repo_state, &target).is_some() {
            return Vec::new();
        }
        let current_plan = selected_diff_load_plan(repo_state, &target);
        if !current_plan.load_patch_diff {
            return Vec::new();
        }
        repo_state.diff_state.diff_rev = repo_state.diff_state.diff_rev.wrapping_add(1);
        repo_state.diff_state.diff = match result {
            Ok(v) => Loadable::Ready(Arc::new(v)),
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.bump_diff_state_rev();
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
        repo_state.diff_state.diff_file_rev = repo_state.diff_state.diff_file_rev.wrapping_add(1);
        repo_state.diff_state.diff_file = match result {
            Ok(v) => Loadable::Ready(v.map(Arc::new)),
            Err(e) => {
                super::util::push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.bump_diff_state_rev();
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
        repo_state.diff_state.diff_file_rev = repo_state.diff_state.diff_file_rev.wrapping_add(1);
        repo_state.diff_state.diff_file_image = match result {
            Ok(v) => Loadable::Ready(v.map(Arc::new)),
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
                if let Some(inline) = repo_state.diff_state.inline_submodule_diff.as_mut() {
                    if let Some(selected_ix) =
                        inline_submodule_entry_index(next_entries.as_slice(), &inline.target)
                    {
                        inline.entries = next_entries;
                        inline.selected_ix = selected_ix;
                    } else {
                        repo_state.diff_state.inline_submodule_diff = None;
                    }
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
    Vec::new()
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
