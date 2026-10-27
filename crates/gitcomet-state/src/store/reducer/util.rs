use super::conflict_load::{start_conflict_target_reload, start_current_conflict_target_reload};
use crate::model::LoadKind;
use crate::model::{
    AppNotification, AppNotificationKind, AppState, CommandLogEntry, DiagnosticEntry,
    DiagnosticKind, GitLogSettings, Loadable, RepoId, RepoState,
};
use crate::msg::{ConflictAutosolveMode, ConflictAutosolveStats, Effect, RepoCommandKind};
use crate::msg::{ReferenceEffect, StatusEffect};
use crate::store::reducer::prepared::ReducerInputs;
#[cfg(test)]
use gitcomet_core::domain::Upstream;
use gitcomet_core::domain::{
    CommitId, DiffArea, DiffTarget, FileStatus, FileStatusKind, SignatureFormats,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::CommandOutput;
use rustc_hash::FxHashSet;
use smallvec::{Array, SmallVec};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::time::SystemTime;

/// Default page size for log fetches.
pub(super) const DEFAULT_LOG_PAGE_SIZE: usize = 200;

/// Replace pending demand with the selection and current viewport. Attempts are
/// memoized only when started, so scrolling never marks skipped commits done.
pub(super) fn verify_commit_signatures_effect(
    formats: SignatureFormats,
    repo_state: &mut RepoState,
    repo_id: RepoId,
    ids: impl IntoIterator<Item = CommitId>,
) -> Option<Effect> {
    if formats.is_empty() {
        return None;
    }
    let history = &mut repo_state.history_state;
    let mut unique = FxHashSet::default();
    let pending: Vec<_> = ids
        .into_iter()
        .chain(history.selected_commit.iter().cloned())
        .chain(history.commit_signatures_visible.iter().cloned())
        .filter(|id| {
            !history.commit_signatures_requested.contains(id)
                && !history.commit_signatures.contains_key(id)
                && unique.insert(id.clone())
        })
        .take(257)
        .collect();
    history.commit_signatures_queue.clear();
    history
        .commit_signatures_queue
        .extend(pending.chunks(16).map(Arc::from));
    if history.commit_signatures_in_flight {
        return None;
    }
    let commit_ids = history.commit_signatures_queue.pop_front()?;
    let requested = Arc::make_mut(&mut history.commit_signatures_requested);
    let order = Arc::make_mut(&mut history.commit_signatures_attempt_order);
    for id in commit_ids.iter() {
        if requested.insert(id.clone()) {
            order.push_back(id.clone());
        }
    }
    while order.len() > 4096 {
        let id = order.pop_front().unwrap();
        requested.remove(&id);
        if history.commit_signatures.contains_key(&id) {
            Arc::make_mut(&mut history.commit_signatures).remove(&id);
            history.commit_signatures_rev = history.commit_signatures_rev.wrapping_add(1);
        }
    }
    history.commit_signatures_in_flight = true;
    history.commit_signatures_batch = history.commit_signatures_batch.wrapping_add(1);
    Some(Effect::VerifyCommitSignatures {
        repo_id,
        epoch: history.commit_signatures_epoch,
        batch: history.commit_signatures_batch,
        cancellation: history.commit_signatures_cancellation.clone(),
        commit_ids,
        formats,
    })
}

pub(super) fn reverify_loaded_commit_signatures_effect(
    formats: SignatureFormats,
    repo_state: &mut RepoState,
) -> Option<Effect> {
    repo_state.clear_commit_signatures();
    // The UI republishes its viewport for the new epoch; never scan loaded pages.
    verify_commit_signatures_effect(formats, repo_state, repo_state.id, [])
}

pub(super) fn reverify_all_commit_signatures_effects(state: &mut AppState) -> Vec<Effect> {
    let formats = state.signature_verification_formats();
    let active = state.active_repo;
    state
        .repos
        .iter_mut()
        .filter_map(|repo| {
            reverify_loaded_commit_signatures_effect(
                if active == Some(repo.id) {
                    formats
                } else {
                    SignatureFormats::NONE
                },
                repo,
            )
        })
        .collect()
}

pub(super) fn set_commit_signature_targets(
    state: &mut AppState,
    repo_id: RepoId,
    epoch: u64,
    commit_ids: Arc<[CommitId]>,
) -> Vec<Effect> {
    if !state.git_log_settings.verify_commit_signatures || state.active_repo != Some(repo_id) {
        return Vec::new();
    }
    let formats = state.signature_verification_formats();
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    if epoch != repo.history_state.commit_signatures_epoch {
        return Vec::new();
    }
    repo.history_state.commit_signatures_visible = if commit_ids.len() > 256 {
        Arc::from(&commit_ids[..256])
    } else {
        commit_ids
    };
    verify_commit_signatures_effect(formats, repo, repo_id, [])
        .into_iter()
        .collect()
}
const DIFF_RELOAD_MAX_EFFECTS: usize = 3;
const PRIMARY_REFRESH_MAX_EFFECTS: usize = 5;
const FULL_REFRESH_MAX_EFFECTS: usize = 8;
const BACKGROUND_METADATA_MAX_EFFECTS: usize = 3;

pub(super) trait EffectAccumulator {
    fn push_effect(&mut self, effect: Effect);
}

impl EffectAccumulator for Vec<Effect> {
    fn push_effect(&mut self, effect: Effect) {
        self.push(effect);
    }
}

impl<A> EffectAccumulator for SmallVec<A>
where
    A: Array<Item = Effect>,
{
    fn push_effect(&mut self, effect: Effect) {
        self.push(effect);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct DiffTargetPreviewFlags {
    pub wants_image: bool,
    pub is_svg: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SelectedDiffLoadPlan {
    pub load_patch_diff: bool,
    pub load_file_text: bool,
    pub preview_text_side: Option<gitcomet_core::domain::DiffPreviewTextSide>,
    pub load_submodule_summary: bool,
    pub load_file_image: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SelectedConflictTarget<'a> {
    Current,
    Path(&'a Path),
}

fn path_preview_flags(path: &Path) -> DiffTargetPreviewFlags {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return DiffTargetPreviewFlags::default();
    };

    match ext.as_bytes() {
        [a, b, c] => match (
            a.to_ascii_lowercase(),
            b.to_ascii_lowercase(),
            c.to_ascii_lowercase(),
        ) {
            (b's', b'v', b'g') => DiffTargetPreviewFlags {
                wants_image: true,
                is_svg: true,
            },
            (b'p', b'n', b'g')
            | (b'j', b'p', b'g')
            | (b'g', b'i', b'f')
            | (b'b', b'm', b'p')
            | (b'i', b'c', b'o')
            | (b't', b'i', b'f') => DiffTargetPreviewFlags {
                wants_image: true,
                is_svg: false,
            },
            _ => DiffTargetPreviewFlags::default(),
        },
        [a, b, c, d] => match (
            a.to_ascii_lowercase(),
            b.to_ascii_lowercase(),
            c.to_ascii_lowercase(),
            d.to_ascii_lowercase(),
        ) {
            (b'j', b'p', b'e', b'g') | (b'w', b'e', b'b', b'p') => DiffTargetPreviewFlags {
                wants_image: true,
                is_svg: false,
            },
            (b't', b'i', b'f', b'f') => DiffTargetPreviewFlags {
                wants_image: true,
                is_svg: false,
            },
            _ => DiffTargetPreviewFlags::default(),
        },
        _ => DiffTargetPreviewFlags::default(),
    }
}

/// Whether `target` is a file whose name does not say if it is an image: one
/// without an extension. Its bytes are loaded to look.
pub(super) fn probes_image_content(target: &DiffTarget) -> bool {
    target
        .file_path()
        .is_some_and(|path| path.extension().is_none())
}

/// Whether `bytes` start with the signature of an image the image view
/// decodes. Only signatures no text file starts with count, which leaves out
/// BMP's and ICO's two bytes.
pub(super) fn has_image_signature(bytes: &[u8]) -> bool {
    const SIGNATURES: [&[u8]; 6] = [
        b"\x89PNG\r\n\x1a\n",
        b"\xff\xd8\xff",
        b"GIF87a",
        b"GIF89a",
        b"II*\0",
        b"MM\0*",
    ];
    SIGNATURES
        .iter()
        .any(|signature| bytes.starts_with(signature))
        || (bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
}

pub(super) fn diff_target_preview_flags(target: &DiffTarget) -> DiffTargetPreviewFlags {
    match target {
        DiffTarget::WorkingTree { path, .. } => path_preview_flags(path),
        DiffTarget::Commit { path, .. }
        | DiffTarget::CommitRange {
            path: Some(path), ..
        } => path_preview_flags(path),
        _ => DiffTargetPreviewFlags::default(),
    }
}

#[cfg(test)]
pub(super) fn diff_target_wants_image_preview(target: &DiffTarget) -> bool {
    diff_target_preview_flags(target).wants_image
}

#[cfg(test)]
pub(super) fn diff_target_is_svg(target: &DiffTarget) -> bool {
    diff_target_preview_flags(target).is_svg
}

enum DiffTargetClassification {
    Submodule,
    Ordinary(Option<gitcomet_core::domain::DiffPreviewTextSide>),
}

fn classify_diff_target(
    inputs: &ReducerInputs,
    repo_state: &RepoState,
    target: &DiffTarget,
) -> DiffTargetClassification {
    use gitcomet_core::domain::DiffPreviewTextSide;

    let preview_text_side = match target {
        DiffTarget::WorkingTree { path, area, .. } => {
            let mut entries = repo_state
                .status_entries_for_area(*area)
                .into_iter()
                .flatten()
                .filter(|entry| entry.path == *path);
            let first = entries.next();
            let is_submodule = match worktree_submodule_source_for_entry(repo_state, path, first) {
                WorktreeSubmoduleSource::Known(value) => value,
                WorktreeSubmoduleSource::Marker => inputs.worktree_marker(repo_state, path),
            };
            if is_submodule {
                return DiffTargetClassification::Submodule;
            }
            if repo_state.diff_state.content_preview {
                return DiffTargetClassification::Ordinary(None);
            }
            first
                .into_iter()
                .chain(entries)
                .find_map(|entry| match entry.kind {
                    FileStatusKind::Untracked | FileStatusKind::Added => {
                        Some(DiffPreviewTextSide::New)
                    }
                    FileStatusKind::Deleted => Some(DiffPreviewTextSide::Old),
                    FileStatusKind::Modified
                    | FileStatusKind::Renamed
                    | FileStatusKind::Conflicted => None,
                })
        }
        DiffTarget::Commit {
            commit_id, path, ..
        } => {
            let Loadable::Ready(details) = &repo_state.history_state.commit_details else {
                return DiffTargetClassification::Ordinary(None);
            };
            if &details.id != commit_id {
                return DiffTargetClassification::Ordinary(None);
            }
            if details
                .files
                .iter()
                .any(|file| file.path == *path && file.is_submodule)
            {
                return DiffTargetClassification::Submodule;
            }
            if repo_state.diff_state.content_preview {
                return DiffTargetClassification::Ordinary(None);
            }
            details.files.iter().find_map(|file| {
                (file.path == *path && !file.is_submodule).then_some(match file.kind {
                    FileStatusKind::Added => Some(DiffPreviewTextSide::New),
                    FileStatusKind::Deleted => Some(DiffPreviewTextSide::Old),
                    FileStatusKind::Modified
                    | FileStatusKind::Renamed
                    | FileStatusKind::Conflicted
                    | FileStatusKind::Untracked => None,
                })?
            })
        }
        DiffTarget::CommitRange { .. } => None,
    };
    DiffTargetClassification::Ordinary(preview_text_side)
}

pub(super) fn selected_diff_load_plan(
    inputs: &ReducerInputs,
    repo_state: &RepoState,
    target: &DiffTarget,
) -> SelectedDiffLoadPlan {
    let target_preview_text_side = match classify_diff_target(inputs, repo_state, target) {
        DiffTargetClassification::Submodule => {
            return SelectedDiffLoadPlan {
                load_patch_diff: false,
                load_file_text: false,
                preview_text_side: None,
                load_submodule_summary: true,
                load_file_image: false,
            };
        }
        DiffTargetClassification::Ordinary(side) => side,
    };

    let supports_file = matches!(
        target,
        DiffTarget::WorkingTree { .. }
            | DiffTarget::Commit { .. }
            | DiffTarget::CommitRange { path: Some(_), .. }
    );
    let preview = if repo_state.diff_state.content_image_target.as_ref() == Some(target) {
        DiffTargetPreviewFlags {
            wants_image: true,
            is_svg: false,
        }
    } else {
        diff_target_preview_flags(target)
    };
    let content_preview = repo_state.diff_state.content_preview;
    let preview_only = content_preview || target_preview_text_side.is_some();
    let preview_text_side = if supports_file && (!preview.wants_image || preview.is_svg) {
        if content_preview {
            // Commit content is read from a blob temp file (New side); working-tree
            // content is read straight from disk by the worktree preview and needs
            // no preview-text-file load.
            matches!(target, DiffTarget::Commit { .. })
                .then_some(gitcomet_core::domain::DiffPreviewTextSide::New)
        } else {
            target_preview_text_side
        }
    } else {
        None
    };

    SelectedDiffLoadPlan {
        load_patch_diff: supports_file && !preview_only,
        // An SVG counts as an image, so it never reaches the text-file preview
        // path and the diff pane's Code view is the only place its source is
        // ever shown. That view reads the loaded file text, so it has to load
        // even for the preview-only targets — added, deleted, untracked — that
        // a plain text file would render straight from the worktree instead.
        load_file_text: supports_file
            && (preview.is_svg || (!preview.wants_image && !preview_only)),
        preview_text_side,
        load_submodule_summary: false,
        load_file_image: supports_file && preview.wants_image,
    }
}

/// A `LoadSelectedDiff` also reads the selected file's attributes; loaded
/// ones stay until they are replaced.
pub(super) fn mark_text_attributes_loading(repo_state: &mut RepoState) {
    let has_file = repo_state
        .diff_state
        .diff_target
        .as_ref()
        .and_then(DiffTarget::file_path)
        .is_some();
    if has_file && !matches!(repo_state.diff_state.text_attributes, Loadable::Ready(_)) {
        repo_state.diff_state.text_attributes = Loadable::Loading;
        repo_state.diff_state.text_attributes_rev =
            repo_state.diff_state.text_attributes_rev.wrapping_add(1);
    }
}

/// Refresh attributes independently of content: even an immutable commit
/// view follows the working tree's current attributes and repository config.
pub(super) fn reload_selected_text_attributes(repo_state: &mut RepoState) -> Option<Effect> {
    repo_state.diff_state.diff_target.as_ref()?.file_path()?;
    mark_text_attributes_loading(repo_state);
    Some(Effect::LoadSelectedDiff {
        repo_id: repo_state.id,
        load_patch_diff: false,
        load_file_text: false,
        preview_text_side: None,
        load_submodule_summary: false,
        load_file_image: false,
    })
}

/// Read the open file again because what decides its decoding changed: as a
/// conflict when it is one, otherwise as a new generation that keeps the
/// content on screen and drops loads still reading it the old way.
pub(super) fn reload_selected_file_text(
    inputs: &ReducerInputs,
    repo_state: &mut RepoState,
    target: &DiffTarget,
) -> Vec<Effect> {
    if let Some(conflict_target) = selected_conflict_target(repo_state, target) {
        return match conflict_target {
            SelectedConflictTarget::Current => start_current_conflict_target_reload(repo_state),
            SelectedConflictTarget::Path(path) => start_conflict_target_reload(repo_state, path),
        };
    }
    repo_state.diff_state.diff_target_rev = repo_state.diff_state.diff_target_rev.wrapping_add(1);
    let load_plan = selected_diff_load_plan(inputs, repo_state, target);
    apply_selected_diff_load_plan_state_with_reload_mode(
        repo_state,
        load_plan,
        DiffReloadMode::KeepLoaded,
    );
    repo_state.bump_diff_state_rev();
    vec![Effect::LoadSelectedDiff {
        repo_id: repo_state.id,
        load_patch_diff: load_plan.load_patch_diff,
        load_file_text: load_plan.load_file_text,
        preview_text_side: load_plan.preview_text_side,
        load_submodule_summary: load_plan.load_submodule_summary,
        load_file_image: load_plan.load_file_image,
    }]
}

pub(super) fn apply_selected_diff_load_plan_state(
    repo_state: &mut RepoState,
    load_plan: SelectedDiffLoadPlan,
) {
    apply_selected_diff_load_plan_state_with_reload_mode(
        repo_state,
        load_plan,
        DiffReloadMode::Blank,
    );
}

/// What a reload does with content that is already on screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffReloadMode {
    /// Drop it: the diff is switching to something else, so the old content is
    /// not what the user asked for and must not linger.
    Blank,
    /// Keep it until the new content arrives. For a reload of the *same* target
    /// — after staging a hunk or line, say — blanking makes the pane flash a
    /// "Loading" placeholder for a frame or two even when almost nothing about
    /// the file changed.
    ///
    /// What stays on screen is then a generation behind the index, so this also
    /// raises `diff_reload_in_flight` for as long as that is true.
    KeepLoaded,
}

pub(super) fn apply_selected_diff_load_plan_state_with_reload_mode(
    repo_state: &mut RepoState,
    load_plan: SelectedDiffLoadPlan,
    mode: DiffReloadMode,
) {
    fn reloading<T>(current: &Loadable<T>, mode: DiffReloadMode) -> Loadable<T>
    where
        T: Clone,
    {
        match (mode, current) {
            (DiffReloadMode::KeepLoaded, Loadable::Ready(value)) => Loadable::Ready(value.clone()),
            _ => Loadable::Loading,
        }
    }

    // Blanking leaves nothing stale to build a patch out of, so only the keeping
    // mode raises the flag — and it lowers it again, which is what stops a
    // target change from stranding it set.
    repo_state.diff_state.diff_reload_in_flight = matches!(mode, DiffReloadMode::KeepLoaded);

    repo_state.diff_state.diff = if load_plan.load_patch_diff {
        reloading(&repo_state.diff_state.diff, mode)
    } else {
        Loadable::NotLoaded
    };
    repo_state.diff_state.diff_file = if load_plan.load_file_text {
        reloading(&repo_state.diff_state.diff_file, mode)
    } else {
        Loadable::NotLoaded
    };
    repo_state.diff_state.diff_preview_text_file = if load_plan.preview_text_side.is_some() {
        reloading(&repo_state.diff_state.diff_preview_text_file, mode)
    } else {
        Loadable::NotLoaded
    };
    repo_state.diff_state.submodule_summary = if load_plan.load_submodule_summary {
        reloading(&repo_state.diff_state.submodule_summary, mode)
    } else {
        Loadable::NotLoaded
    };
    repo_state.diff_state.diff_file_image = if load_plan.load_file_image {
        reloading(&repo_state.diff_state.diff_file_image, mode)
    } else {
        Loadable::NotLoaded
    };
}

pub(super) enum WorktreeSubmoduleSource {
    Known(bool),
    Marker,
}

pub(super) fn worktree_submodule_source(
    repo_state: &RepoState,
    area: DiffArea,
    path: &Path,
) -> WorktreeSubmoduleSource {
    worktree_submodule_source_for_entry(
        repo_state,
        path,
        repo_state.status_entry_for_path(area, path),
    )
}

pub(super) fn worktree_submodule_source_for_entry(
    repo_state: &RepoState,
    path: &Path,
    entry: Option<&FileStatus>,
) -> WorktreeSubmoduleSource {
    let Some(entry) = entry else {
        return WorktreeSubmoduleSource::Known(false);
    };
    if entry.kind == FileStatusKind::Untracked {
        return WorktreeSubmoduleSource::Known(false);
    }
    if let Loadable::Ready(submodules) = &repo_state.submodules
        && submodules.iter().any(|submodule| submodule.path == path)
    {
        return WorktreeSubmoduleSource::Known(true);
    }
    if entry.kind == FileStatusKind::Deleted {
        return WorktreeSubmoduleSource::Known(repo_state.head_gitlink_paths.contains(path));
    }
    WorktreeSubmoduleSource::Marker
}

pub(super) fn selected_conflict_target<'a>(
    repo_state: &RepoState,
    target: &'a DiffTarget,
) -> Option<SelectedConflictTarget<'a>> {
    let DiffTarget::WorkingTree { path, area, .. } = target else {
        return None;
    };
    if *area != DiffArea::Unstaged {
        return None;
    }

    if repo_state.conflict_state.conflict_file_path.as_deref() == Some(path.as_path()) {
        return Some(SelectedConflictTarget::Current);
    }

    // Fast path: skip the full scan when status has no unstaged conflicts.
    if !repo_state.has_unstaged_conflicts {
        return None;
    }

    repo_state
        .worktree_status_entries()?
        .iter()
        .find(|entry| entry.path == *path && entry.kind == FileStatusKind::Conflicted)
        .map(|_| SelectedConflictTarget::Path(path.as_path()))
}

pub(super) fn diff_reload_effect_count(
    inputs: &ReducerInputs,
    repo_state: &RepoState,
    target: &DiffTarget,
) -> usize {
    let plan = selected_diff_load_plan(inputs, repo_state, target);

    let mut count = usize::from(plan.load_patch_diff);
    if plan.load_submodule_summary {
        count += 1;
    }
    if plan.load_file_image {
        count += 1;
    }
    if plan.load_file_text {
        count += 1;
    }
    if plan.preview_text_side.is_some() {
        count += 1;
    }

    debug_assert!(count <= DIFF_RELOAD_MAX_EFFECTS);
    count
}

pub(super) fn diff_reload_effects(
    inputs: &ReducerInputs,
    repo_state: &RepoState,
    repo_id: RepoId,
    target: DiffTarget,
) -> Vec<Effect> {
    let mut effects = Vec::with_capacity(diff_reload_effect_count(inputs, repo_state, &target));
    append_diff_reload_effects(inputs, &mut effects, repo_state, repo_id, target);
    effects
}

pub(super) fn append_diff_reload_effects(
    inputs: &ReducerInputs,
    effects: &mut impl EffectAccumulator,
    repo_state: &RepoState,
    repo_id: RepoId,
    target: DiffTarget,
) {
    let plan = selected_diff_load_plan(inputs, repo_state, &target);

    if plan.load_submodule_summary {
        effects.push_effect(Effect::LoadSubmoduleSummary {
            repo_id,
            target: target.clone(),
        });
    }
    if plan.load_patch_diff {
        effects.push_effect(Effect::LoadDiff {
            repo_id,
            target: target.clone(),
        });
    }
    if plan.load_file_image {
        effects.push_effect(Effect::LoadDiffFileImage {
            repo_id,
            target: target.clone(),
        });
    }
    if let Some(side) = plan.preview_text_side {
        effects.push_effect(Effect::LoadDiffPreviewTextFile {
            repo_id,
            target: target.clone(),
            side,
        });
    }
    if plan.load_file_text {
        effects.push_effect(Effect::LoadDiffFile { repo_id, target });
    }
}

pub(super) fn refresh_primary_effect_capacity() -> usize {
    PRIMARY_REFRESH_MAX_EFFECTS
}

fn should_auto_fetch_history_tags(git_log_settings: GitLogSettings) -> bool {
    git_log_settings.show_history_tags && git_log_settings.auto_fetch_tags_on_repo_activation()
}

pub(super) fn append_requested_status_refresh_effects(
    repo_state: &mut RepoState,
    effects: &mut impl EffectAccumulator,
) {
    if repo_state.is_bare() {
        repo_state.set_status(Loadable::Ready(Arc::new(
            gitcomet_core::domain::RepoStatus {
                staged: Arc::new(Vec::new()),
                unstaged: Arc::new(Vec::new()),
            },
        )));
        repo_state.set_uncommitted_line_stats(Loadable::Ready(Arc::default()));
        repo_state.set_uncommitted_large_files(Arc::default());
        return;
    }
    let repo_id = repo_state.id;
    let load_worktree = repo_state.loads_in_flight.request(LoadKind::WorktreeStatus);
    let load_staged = repo_state.loads_in_flight.request(LoadKind::StagedStatus);

    match (load_worktree, load_staged) {
        (true, true) => effects.push_effect(Effect::Status(StatusEffect::LoadStatus { repo_id })),
        (true, false) => {
            effects.push_effect(Effect::Status(StatusEffect::LoadWorktreeStatus { repo_id }))
        }
        (false, true) => {
            effects.push_effect(Effect::Status(StatusEffect::LoadStagedStatus { repo_id }))
        }
        (false, false) => {}
    }
    repo_state.loads_in_flight.invalidate_line_stats();
}

/// Reuse the settled status lanes; never fall back to an older combined status.
pub(super) fn append_ready_line_stats_effect(
    repo_state: &mut RepoState,
    effects: &mut impl EffectAccumulator,
) {
    let repo_id = repo_state.id;
    let ready = matches!(
        (&repo_state.staged_status, &repo_state.worktree_status),
        (Loadable::Ready(_), Loadable::Ready(_))
    );
    if let Some(generation) = repo_state.loads_in_flight.start_line_stats(ready) {
        let (Loadable::Ready(staged), Loadable::Ready(unstaged)) =
            (&repo_state.staged_status, &repo_state.worktree_status)
        else {
            unreachable!("start_line_stats requires ready status lanes");
        };
        if staged.is_empty() && unstaged.is_empty() {
            // Both fresh lanes and their replays have settled. There are no
            // rows to count or classify, so finish through the same generation
            // protocol without another backend task and completion message.
            let current = repo_state.loads_in_flight.finish_line_stats(generation);
            debug_assert!(current);
            repo_state.set_uncommitted_line_stats(Loadable::Ready(Arc::default()));
            repo_state.set_uncommitted_large_files(Arc::default());
            return;
        }
        crate::store::repo_load_trace::trace!(
            "line_stats_start repo_id={:?} generation={} snapshot=reused staged={} unstaged={}",
            repo_id,
            generation,
            staged.len(),
            unstaged.len()
        );
        let large_files = repo_state.large_file_support_active();
        effects.push_effect(Effect::Status(StatusEffect::LoadUncommittedLineStats {
            repo_id,
            generation,
            status: std::sync::Arc::new(gitcomet_core::domain::RepoStatus {
                staged: std::sync::Arc::clone(staged),
                unstaged: std::sync::Arc::clone(unstaged),
            }),
            large_files,
        }));
    }
}

fn push_rebase_and_merge_refresh_effect(effects: &mut impl EffectAccumulator, repo_id: RepoId) {
    effects.push_effect(Effect::LoadRebaseAndMergeState { repo_id });
}

fn append_requested_rebase_and_merge_refresh_effects(
    repo_state: &mut RepoState,
    effects: &mut impl EffectAccumulator,
) {
    if repo_state.is_bare() {
        repo_state.set_rebase_in_progress(Loadable::Ready(false));
        repo_state.set_sequencer_state(Loadable::Ready(Default::default()));
        repo_state.set_merge_commit_message(Loadable::Ready(None));
        return;
    }
    let repo_id = repo_state.id;
    let load_rebase = repo_state.loads_in_flight.request(LoadKind::RebaseState);
    let load_merge_commit_message = repo_state
        .loads_in_flight
        .request(LoadKind::MergeCommitMessage);

    match (load_rebase, load_merge_commit_message) {
        (true, true) => push_rebase_and_merge_refresh_effect(effects, repo_id),
        (true, false) => effects.push_effect(Effect::LoadRebaseState { repo_id }),
        (false, true) => effects.push_effect(Effect::LoadMergeCommitMessage { repo_id }),
        (false, false) => {}
    }
}

pub(super) fn refresh_primary_effects(repo_state: &mut RepoState) -> Vec<Effect> {
    let mut effects = Vec::with_capacity(refresh_primary_effect_capacity());
    append_refresh_primary_effects(repo_state, &mut effects);
    effects
}

/// The request for a fresh first page of `repo_state`'s history, under whatever
/// walk and author filter it currently has.
pub(super) fn first_page_log_request(repo_state: &RepoState) -> crate::model::PendingLogLoad {
    crate::model::PendingLogLoad {
        walk: repo_state.history_walk(),
        author: repo_state.history_state.history_author_filter.clone(),
        focus: repo_state.history_state.history_focus.clone(),
        limit: DEFAULT_LOG_PAGE_SIZE,
        cursor: None,
    }
}

/// Preserve the loaded extent. The effects layer captures the Ready page and
/// asks the backend for a snapshot refresh; this limit also describes the
/// initial extent when a queued request is promoted.
pub(super) fn refresh_log_request(repo_state: &RepoState) -> crate::model::PendingLogLoad {
    crate::model::PendingLogLoad {
        limit: refresh_log_limit(repo_state),
        ..first_page_log_request(repo_state)
    }
}

pub(super) fn refresh_log_limit(repo_state: &RepoState) -> usize {
    match &repo_state.log {
        Loadable::Ready(page) => DEFAULT_LOG_PAGE_SIZE.max(page.commits.len()),
        _ => DEFAULT_LOG_PAGE_SIZE,
    }
}

/// Requests `load` and returns the effect that starts it, or `None` when it was
/// coalesced into a walk already in flight. The effect carries the sequence
/// number the request was given, which is how its replies are recognised.
pub(super) fn request_log_effect(
    repo_state: &mut RepoState,
    load: crate::model::PendingLogLoad,
) -> Option<Effect> {
    let repo_id = repo_state.id;
    let seq = repo_state.loads_in_flight.request_log(load.clone())?;
    let crate::model::PendingLogLoad {
        walk,
        author,
        focus,
        limit,
        cursor,
    } = load;
    Some(Effect::LoadLog {
        repo_id,
        seq,
        walk,
        author,
        focus,
        limit,
        cursor,
    })
}

pub(super) fn append_refresh_primary_effects(
    repo_state: &mut RepoState,
    effects: &mut impl EffectAccumulator,
) {
    let repo_id = repo_state.id;
    let log_request = refresh_log_request(repo_state);

    if !repo_state.is_bare()
        && let Some(seq) = repo_state
            .loads_in_flight
            .request_primary_refresh_batch(log_request.clone())
    {
        repo_state.set_log_loading_more(false);
        effects.push_effect(Effect::References(ReferenceEffect::LoadHeadBranch {
            repo_id,
        }));
        effects.push_effect(Effect::References(
            ReferenceEffect::LoadUpstreamDivergence { repo_id },
        ));
        push_rebase_and_merge_refresh_effect(effects, repo_id);
        effects.push_effect(Effect::Status(StatusEffect::LoadStatus { repo_id }));
        // This batch short-circuits the status-refresh funnel.
        repo_state.loads_in_flight.invalidate_line_stats();
        effects.push_effect(Effect::LoadLog {
            repo_id,
            seq,
            walk: log_request.walk,
            author: log_request.author,
            focus: log_request.focus,
            limit: log_request.limit,
            cursor: log_request.cursor,
        });
        return;
    }

    if repo_state.loads_in_flight.request(LoadKind::HeadBranch) {
        effects.push_effect(Effect::References(ReferenceEffect::LoadHeadBranch {
            repo_id,
        }));
    }
    if repo_state
        .loads_in_flight
        .request(LoadKind::UpstreamDivergence)
    {
        effects.push_effect(Effect::References(
            ReferenceEffect::LoadUpstreamDivergence { repo_id },
        ));
    }
    append_requested_rebase_and_merge_refresh_effects(repo_state, effects);
    append_requested_status_refresh_effects(repo_state, effects);
    if let Some(effect) = request_log_effect(repo_state, log_request) {
        // Block pagination while a refresh log load is in flight, to avoid concurrent LogLoaded
        // merges with different cursors.
        repo_state.set_log_loading_more(false);
        effects.push_effect(effect);
    }
}

pub(super) fn refresh_full_effect_capacity() -> usize {
    FULL_REFRESH_MAX_EFFECTS
}

pub(super) fn background_metadata_effect_capacity() -> usize {
    BACKGROUND_METADATA_MAX_EFFECTS
}

pub(super) fn refresh_full_effects(
    repo_state: &mut RepoState,
    git_log_settings: GitLogSettings,
) -> Vec<Effect> {
    let mut effects = Vec::with_capacity(refresh_full_effect_capacity());
    append_refresh_full_effects(repo_state, git_log_settings, &mut effects);
    effects
}

pub(super) fn append_refresh_full_effects(
    repo_state: &mut RepoState,
    _git_log_settings: GitLogSettings,
    effects: &mut impl EffectAccumulator,
) {
    let repo_id = repo_state.id;

    // Prioritize UI-critical loads (status + log) early. The executor is a FIFO queue, so this
    // ordering can materially impact perceived responsiveness when switching repositories.
    if repo_state.loads_in_flight.request(LoadKind::HeadBranch) {
        effects.push_effect(Effect::References(ReferenceEffect::LoadHeadBranch {
            repo_id,
        }));
    }
    if repo_state
        .loads_in_flight
        .request(LoadKind::UpstreamDivergence)
    {
        effects.push_effect(Effect::References(
            ReferenceEffect::LoadUpstreamDivergence { repo_id },
        ));
    }
    append_requested_status_refresh_effects(repo_state, effects);
    let log_request = refresh_log_request(repo_state);
    if let Some(effect) = request_log_effect(repo_state, log_request) {
        repo_state.set_log_loading_more(false);
        effects.push_effect(effect);
    }
    if repo_state.loads_in_flight.request(LoadKind::Branches) {
        effects.push_effect(Effect::References(ReferenceEffect::LoadBranches {
            repo_id,
        }));
    }
    if repo_state.loads_in_flight.request(LoadKind::Remotes) {
        effects.push_effect(Effect::References(ReferenceEffect::LoadRemotes { repo_id }));
    }
    if repo_state.loads_in_flight.request(LoadKind::RemoteBranches) {
        effects.push_effect(Effect::References(ReferenceEffect::LoadRemoteBranches {
            repo_id,
        }));
    }
    append_requested_rebase_and_merge_refresh_effects(repo_state, effects);
}

pub(super) fn append_auto_background_metadata_effects(
    repo_state: &mut RepoState,
    git_log_settings: GitLogSettings,
    effects: &mut impl EffectAccumulator,
) {
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return;
    }

    let repo_id = repo_state.id;
    if should_auto_fetch_history_tags(git_log_settings) {
        if matches!(repo_state.tags, Loadable::NotLoaded | Loadable::Error(_)) {
            repo_state.set_tags(Loadable::Loading);
            if repo_state.loads_in_flight.request(LoadKind::Tags) {
                effects.push_effect(Effect::References(ReferenceEffect::LoadTags { repo_id }));
            }
        }

        if matches!(
            repo_state.remote_tags,
            Loadable::NotLoaded | Loadable::Error(_)
        ) {
            repo_state.set_remote_tags(Loadable::Loading);
            if repo_state.loads_in_flight.request(LoadKind::RemoteTags) {
                effects.push_effect(Effect::References(ReferenceEffect::LoadRemoteTags {
                    repo_id,
                }));
            }
        }
    }

    if matches!(
        repo_state.submodules,
        Loadable::NotLoaded | Loadable::Error(_)
    ) {
        repo_state.set_submodules(Loadable::Loading);
        if repo_state.loads_in_flight.request(LoadKind::Submodules) {
            effects.push_effect(Effect::LoadSubmodules { repo_id });
        }
    }
}

pub(super) fn dedup_paths_in_order(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(paths.len());
    let mut seen: FxHashSet<PathBuf> = FxHashSet::default();
    for p in paths {
        if !seen.insert(p.clone()) {
            continue;
        }
        out.push(p);
    }
    out
}

pub(super) fn normalize_repo_path(path: PathBuf) -> PathBuf {
    let path = if path.is_relative() {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    } else {
        path
    };

    canonicalize_path(path)
}

pub(super) fn canonicalize_path(path: PathBuf) -> PathBuf {
    super::super::canonicalize_path(path)
}

pub(super) fn push_notification(
    inputs: &ReducerInputs,
    state: &mut AppState,
    kind: AppNotificationKind,
    message: String,
) {
    state.push_notification(AppNotification {
        time: inputs.now(),
        kind,
        message,
    });
}

pub(super) fn push_diagnostic(
    inputs: &ReducerInputs,
    repo_state: &mut RepoState,
    kind: DiagnosticKind,
    message: String,
) {
    repo_state.feedback_mut().push_diagnostic(DiagnosticEntry {
        time: inputs.now(),
        kind,
        message,
    });
}

pub(super) fn handle_session_persist_result(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: Option<RepoId>,
    action: &'static str,
    result: io::Result<()>,
) {
    let Err(error) = result else {
        return;
    };
    let message = format!("Failed to persist session state while {action}: {error}");
    push_notification(inputs, state, AppNotificationKind::Error, message.clone());
    if let Some(repo_id) = repo_id
        && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
    {
        push_diagnostic(inputs, repo_state, DiagnosticKind::Error, message);
    }
}

/// Staging and unstaging a hunk or line is a direct, visible edit: the diff
/// redraws without the change, which is the whole feedback the user needs. A
/// toast for each one just stacks up while working through a file.
fn command_success_is_worth_announcing(command: &RepoCommandKind) -> bool {
    !matches!(
        command,
        RepoCommandKind::StageHunk | RepoCommandKind::UnstageHunk
    )
}

pub(super) fn push_command_log(
    inputs: &ReducerInputs,
    repo_state: &mut RepoState,
    ok: bool,
    command: &RepoCommandKind,
    output: &CommandOutput,
    error: Option<&Error>,
    handled_by_auth_prompt: bool,
) {
    let (command_text, summary) = summarize_command(command, output, ok, error);
    mark_auth_prompt_hook_operation(repo_state, handled_by_auth_prompt);

    let entry = CommandLogEntry {
        time: inputs.now(),
        ok,
        command: command_text,
        summary,
        stdout: command_log_text(&output.stdout),
        stderr: if output.stderr.is_empty() {
            error
                .map(format_error_for_user)
                .map_or_else(|| Arc::from(""), |text| command_log_text(&text))
        } else {
            command_log_text(&output.stderr)
        },
        announce_success: command_success_is_worth_announcing(command),
        handled_by_auth_prompt,
        failure_id: error.and_then(|error| match error.kind() {
            ErrorKind::Git(failure) => Some(failure.id()),
            _ => None,
        }),
        hook_operation_id: repo_state.feedback.command_log_operation_id,
    };
    repo_state.feedback_mut().push_command_result(entry);
}

pub(super) fn push_action_log(
    inputs: &ReducerInputs,
    repo_state: &mut RepoState,
    ok: bool,
    command: String,
    summary: String,
    error: Option<&Error>,
    handled_by_auth_prompt: bool,
) {
    mark_auth_prompt_hook_operation(repo_state, handled_by_auth_prompt);
    let entry = CommandLogEntry {
        time: inputs.now(),
        ok,
        command,
        summary,
        stdout: Arc::from(""),
        stderr: error
            .map(format_error_for_user)
            .map_or_else(|| Arc::from(""), |text| command_log_text(&text)),
        announce_success: true,
        handled_by_auth_prompt,
        failure_id: error.and_then(|error| match error.kind() {
            ErrorKind::Git(failure) => Some(failure.id()),
            _ => None,
        }),
        hook_operation_id: repo_state.feedback.command_log_operation_id,
    };
    repo_state.feedback_mut().push_command_result(entry);
}

/// Nested command completion already carries its exact reporting owner. Mark
/// that attempt before the outer operation finishes, without consulting the
/// currently visible prompt or affecting another operation's feedback.
fn mark_auth_prompt_hook_operation(repo_state: &mut RepoState, handled_by_auth_prompt: bool) {
    if handled_by_auth_prompt
        && let Some(operation_id) = repo_state.feedback.command_log_operation_id
        && let Some(operation) = repo_state
            .feedback_mut()
            .hook_activity
            .iter_mut()
            .find(|operation| operation.id == operation_id)
    {
        operation.handled_by_auth_prompt = true;
    }
}

/// Per-stream cap for command output kept in the log; the tail is what a
/// user reads when a command fails, and the hook-activity log uses the same
/// bound.
const MAX_COMMAND_LOG_OUTPUT_BYTES: usize = 256 * 1024;

fn command_log_text(text: &str) -> Arc<str> {
    if text.len() <= MAX_COMMAND_LOG_OUTPUT_BYTES {
        Arc::from(text)
    } else {
        Arc::from(super::git_hook_activity::utf8_tail(
            text,
            MAX_COMMAND_LOG_OUTPUT_BYTES,
        ))
    }
}

pub(super) fn conflict_autosolve_telemetry_command(
    mode: ConflictAutosolveMode,
    path: Option<&Path>,
) -> String {
    let mut command = format!("telemetry.conflict_autosolve.{}", mode.as_str());
    if let Some(path) = path {
        command.push(' ');
        command.push_str(
            &path
                .to_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("{path:?}")),
        );
    }
    command
}

pub(super) fn conflict_autosolve_telemetry_summary(
    mode: ConflictAutosolveMode,
    path: Option<&Path>,
    total_conflicts_before: usize,
    total_conflicts_after: usize,
    unresolved_before: usize,
    unresolved_after: usize,
    stats: ConflictAutosolveStats,
) -> String {
    let resolved = stats.total_resolved();
    let mode_label = match mode {
        ConflictAutosolveMode::Safe => "safe",
        ConflictAutosolveMode::Regex => "regex",
        ConflictAutosolveMode::History => "history",
    };

    let path_label = path
        .map(|p| format!(" in {}", p.display()))
        .unwrap_or_default();

    let mut details = Vec::new();
    if stats.pass1 > 0 {
        details.push(format!("pass1={}", stats.pass1));
    }
    if stats.pass2_split > 0 {
        details.push(format!("pass2_split={}", stats.pass2_split));
    }
    if stats.pass1_after_split > 0 {
        details.push(format!("pass1_after_split={}", stats.pass1_after_split));
    }
    if stats.regex > 0 {
        details.push(format!("regex={}", stats.regex));
    }
    if stats.history > 0 {
        details.push(format!("history={}", stats.history));
    }
    let details = if details.is_empty() {
        "details=none".to_string()
    } else {
        details.join(", ")
    };

    format!(
        "Conflict autosolve ({mode_label}): resolved {resolved}; unresolved {unresolved_before} -> {unresolved_after}; conflicts {total_conflicts_before} -> {total_conflicts_after}{path_label} ({details})"
    )
}

/// A sequencer command (rebase / cherry-pick / continue) that reported Ok
/// with a non-zero exit paused at a conflict — the backend maps only
/// genuine pauses to Ok — so its summary must not read as completed.
fn sequencer_paused(output: &CommandOutput) -> bool {
    output.exit_code.is_some_and(|code| code != 0)
}

/// Continue/abort share one UI action and backend entry point for rebases,
/// `git am`, cherry-picks, and reverts. Use the command that actually ran so
/// they are not all recorded as rebases in action history.
fn sequencer_operation_label(output: &CommandOutput, error: Option<&Error>) -> &'static str {
    let label_for = |command: &str| {
        let command = command.trim_start();
        if command.starts_with("git cherry-pick") {
            Some("Cherry-pick")
        } else if command.starts_with("git revert") {
            Some("Revert")
        } else {
            None
        }
    };
    label_for(&output.command)
        .or_else(|| {
            error
                .and_then(try_format_git_backend_error)
                .and_then(|(command, _)| label_for(&command))
        })
        .unwrap_or("Rebase")
}

fn summarize_command(
    command: &RepoCommandKind,
    output: &CommandOutput,
    ok: bool,
    error: Option<&Error>,
) -> (String, String) {
    use gitcomet_core::services::ConflictSide;

    if !ok {
        let label = match command {
            RepoCommandKind::FetchAll | RepoCommandKind::FetchRefspecs { .. } => "Fetch",
            RepoCommandKind::PruneMergedBranches => "Prune merged branches",
            RepoCommandKind::PruneLocalTags => "Prune local tags",
            RepoCommandKind::RunMaintenance => "Maintenance",
            RepoCommandKind::Pull { .. } => "Pull",
            RepoCommandKind::PullBranch { .. } => "Pull",
            RepoCommandKind::MergeRef { .. } => "Merge",
            RepoCommandKind::SquashRef { .. } => "Squash",
            RepoCommandKind::PushWithTags { request } => request.mode.label(),
            RepoCommandKind::Push => "Push",
            RepoCommandKind::PushAfterCommit { .. } => "Push after commit",
            RepoCommandKind::ForcePush => "Force push",
            RepoCommandKind::ForcePushWithLease { .. } => "Force push with lease",
            RepoCommandKind::PushSetUpstream { .. } => "Push",
            RepoCommandKind::SetUpstreamBranch { .. } => "Set as tracking upstream",
            RepoCommandKind::UnsetUpstreamBranch { .. } => "Unlink upstream branch",
            RepoCommandKind::DeleteRemoteBranch { .. } => "Delete remote branch",
            RepoCommandKind::DeleteRemoteBranches { .. } => "Delete remote branches",
            RepoCommandKind::PushTag { .. } => "Push tag",
            RepoCommandKind::DeleteRemoteTag { .. } => "Delete remote tag",
            RepoCommandKind::Reset { .. } => "Reset",
            RepoCommandKind::SquashCommits { .. } => "Squash",
            RepoCommandKind::Rebase { .. } => "Rebase",
            RepoCommandKind::RebaseContinue | RepoCommandKind::RebaseAbort => {
                sequencer_operation_label(output, error)
            }
            RepoCommandKind::InteractiveRebase { interactive, .. } => {
                if *interactive {
                    "Interactive rebase"
                } else {
                    "Rebase"
                }
            }
            RepoCommandKind::InteractiveCherryPick { .. } => "Cherry-pick",
            RepoCommandKind::CherryPick { .. } => "Cherry-pick",
            RepoCommandKind::Revert { .. } => "Revert",
            RepoCommandKind::ApplyFileChange { .. } => "Apply change",
            RepoCommandKind::MergeAbort => "Merge",
            RepoCommandKind::CreateTag { .. } => "Tag",
            RepoCommandKind::DeleteTag { .. } => "Tag",
            RepoCommandKind::AddRemote { .. } => "Remote",
            RepoCommandKind::RemoveRemote { .. } => "Remote",
            RepoCommandKind::SetRemoteUrl { .. } => "Remote",
            RepoCommandKind::CheckoutConflict { side, .. } => match side {
                ConflictSide::Ours => "Checkout ours",
                ConflictSide::Theirs => "Checkout theirs",
            },
            RepoCommandKind::AcceptConflictDeletion { .. } => "Accept deletion",
            RepoCommandKind::CheckoutConflictBase { .. } => "Checkout base",
            RepoCommandKind::LaunchMergetool { .. } => "Mergetool",
            RepoCommandKind::SaveWorktreeFile { .. } => "Save file",
            RepoCommandKind::AppendGitignorePatterns { .. } => "Update .gitignore",
            RepoCommandKind::LargeFile { command } => command.label(),
            RepoCommandKind::AppendGitattributesRule { .. } => "Update .gitattributes",
            RepoCommandKind::ExportPatch { .. } | RepoCommandKind::ApplyPatch { .. } => "Patch",
            RepoCommandKind::AddWorktree { .. }
            | RepoCommandKind::RemoveWorktree { .. }
            | RepoCommandKind::ForceRemoveWorktree { .. } => "Worktree",
            RepoCommandKind::AddSubmodule { .. }
            | RepoCommandKind::UpdateSubmodules { .. }
            | RepoCommandKind::LoadSubmodule { .. }
            | RepoCommandKind::ChangeSubmodulePointer { .. }
            | RepoCommandKind::RemoveSubmodule { .. } => "Submodule",
            RepoCommandKind::StageHunk | RepoCommandKind::UnstageHunk => "Hunk",
            RepoCommandKind::ApplyWorktreePatch { reverse } => {
                if *reverse {
                    "Discard"
                } else {
                    "Patch"
                }
            }
        };
        if let Some(error) = error
            && let Some((git_command, details)) = try_format_git_backend_error(error)
        {
            // A pull or merge that stopped at conflicts did its job; say so.
            let outcome = match error.kind() {
                ErrorKind::Git(failure) if failure.id() == GitFailureId::StoppedAtConflicts => {
                    "stopped at conflicts"
                }
                ErrorKind::Git(failure)
                    if failure.id() == GitFailureId::WorktreeWouldBeOverwritten =>
                {
                    "blocked by local changes"
                }
                _ => "failed",
            };
            return (git_command, format!("{label} {outcome}:\n\n{details}"));
        }

        return (
            output.command.clone().if_empty_else(|| label.to_string()),
            error
                .map(|e| format!("{label} failed:\n\n{}", format_error_for_user(e)))
                .unwrap_or_else(|| format!("{label} failed")),
        );
    }

    let summary = match command {
        RepoCommandKind::FetchAll | RepoCommandKind::FetchRefspecs { .. } => {
            if output.stderr.trim().is_empty() && output.stdout.trim().is_empty() {
                "Fetch: Already up to date".to_string()
            } else {
                "Fetch: Synchronized".to_string()
            }
        }
        RepoCommandKind::PruneMergedBranches => "Prune merged branches: Completed".to_string(),
        RepoCommandKind::PruneLocalTags => "Prune local tags: Completed".to_string(),
        RepoCommandKind::RunMaintenance => {
            // The run prints nothing over a pipe; the backend returns its
            // check instead when git found nothing to do.
            if output.command == gitcomet_core::services::MAINTENANCE_CHECK_COMMAND {
                "Maintenance: Not needed, the repository is already optimized".to_string()
            } else {
                "Maintenance: Completed".to_string()
            }
        }
        RepoCommandKind::Pull { .. } => {
            if output.stdout.contains("Already up to date") {
                "Pull: Already up to date".to_string()
            } else if output.stdout.starts_with("Updating") {
                "Pull: Fast-forwarded".to_string()
            } else if output.stdout.starts_with("Merge") {
                "Pull: Merged".to_string()
            } else if output.stdout.contains("Successfully rebased") {
                "Pull: Rebasing complete".to_string()
            } else {
                "Pull: Completed".to_string()
            }
        }
        RepoCommandKind::PullBranch { remote, branch } => {
            let base = if output.stdout.contains("Already up to date") {
                "Already up to date"
            } else if output.stdout.starts_with("Updating") {
                "Fast-forwarded"
            } else if output.stdout.starts_with("Merge") {
                "Merged"
            } else {
                "Completed"
            };
            format!("Pull {remote}/{branch}: {base}")
        }
        RepoCommandKind::MergeRef { reference } => {
            let base = if output.stdout.contains("Already up to date") {
                "Already up to date"
            } else if output.stdout.contains("Fast-forward")
                || output.stdout.starts_with("Updating")
            {
                "Fast-forwarded"
            } else if output.stdout.contains("Merge made by") {
                "Merged"
            } else {
                "Completed"
            };
            format!("Merge {reference}: {base}")
        }
        RepoCommandKind::SquashRef { reference } => {
            let base = if output.stdout.contains("Already up to date") {
                "Already up to date"
            } else if output.stdout.contains("Squash commit -- not updating HEAD")
                || output
                    .stdout
                    .contains("Automatic merge went well; stopped before committing as requested")
            {
                "Staged"
            } else {
                "Completed"
            };
            format!("Squash {reference}: {base}")
        }
        RepoCommandKind::Push => {
            if output.stderr.contains("Everything up-to-date") {
                "Push: Everything up-to-date".to_string()
            } else {
                "Push: Completed".to_string()
            }
        }
        RepoCommandKind::PushAfterCommit { set_upstream, .. } => {
            let base = if output.stderr.contains("Everything up-to-date") {
                "Everything up-to-date"
            } else {
                "Completed"
            };
            if *set_upstream {
                format!("Push after commit -u: {base}")
            } else {
                format!("Push after commit: {base}")
            }
        }
        RepoCommandKind::ForcePush => {
            if output.stderr.contains("Everything up-to-date") {
                "Force push: Everything up-to-date".to_string()
            } else {
                "Force push: Completed".to_string()
            }
        }
        RepoCommandKind::ForcePushWithLease { .. } => {
            if output.stderr.contains("Everything up-to-date") {
                "Force push with lease: Everything up-to-date".to_string()
            } else {
                "Force push with lease: Completed".to_string()
            }
        }
        RepoCommandKind::PushWithTags { request } => format!(
            "{} to {}/{}: Completed",
            request.mode.label(),
            request.remote,
            request.branch
        ),
        RepoCommandKind::PushSetUpstream { remote, branch } => {
            let base = if output.stderr.contains("Everything up-to-date") {
                "Everything up-to-date"
            } else {
                "Completed"
            };
            format!("Push -u {remote}/{branch}: {base}")
        }
        RepoCommandKind::SetUpstreamBranch { branch, upstream } => {
            format!(
                "Branch {branch}: Upstream set to {}/{}",
                upstream.remote, upstream.branch
            )
        }
        RepoCommandKind::UnsetUpstreamBranch { branch } => {
            format!("Branch {branch}: Upstream unlinked")
        }
        RepoCommandKind::DeleteRemoteBranch { remote, branch } => {
            format!("Remote branch {remote}/{branch}: Deleted")
        }
        RepoCommandKind::DeleteRemoteBranches { remote, branches } => {
            let noun = crate::name_summary::branch_noun(branches.len());
            format!("{} remote {noun} on {remote}: Deleted", branches.len())
        }
        RepoCommandKind::PushTag { remote, name } => {
            if output.stderr.contains("Everything up-to-date") {
                format!("Tag {name} → {remote}: Already up-to-date")
            } else {
                format!("Tag {name} → {remote}: Pushed")
            }
        }
        RepoCommandKind::DeleteRemoteTag { remote, name } => {
            format!("Tag {name} on {remote}: Deleted")
        }
        RepoCommandKind::CheckoutConflict { side, .. } => match side {
            ConflictSide::Ours => "Resolved using ours".to_string(),
            ConflictSide::Theirs => "Resolved using theirs".to_string(),
        },
        RepoCommandKind::AcceptConflictDeletion { path } => {
            format!("Resolved by accepting deletion → {}", path.display())
        }
        RepoCommandKind::CheckoutConflictBase { path } => {
            format!("Resolved using base → {}", path.display())
        }
        RepoCommandKind::LaunchMergetool { path } => {
            format!("Mergetool: Resolved {}", path.display())
        }
        RepoCommandKind::SaveWorktreeFile { path, stage } => {
            if *stage {
                format!("Saved and staged → {}", path.display())
            } else {
                format!("Saved → {}", path.display())
            }
        }
        RepoCommandKind::LargeFile { command } => format!("{}: done", command.label()),
        // Deliberately "added to .gitignore" rather than "ignored": a later
        // negation, a nested .gitignore or .git/info/exclude can still win, and
        // promising an outcome we did not verify would be a lie the user only
        // catches when the file stays in the list.
        // The worker skips the write when every pattern is already there, and
        // announcing "Added …" for a run that changed nothing would send the
        // user looking for a file that has not moved.
        RepoCommandKind::AppendGitattributesRule { rule } => {
            if output.stdout.trim() == gitcomet_core::gitattributes::NOTHING_TO_ADD {
                "Already in .gitattributes; nothing added".to_string()
            } else {
                format!("Added {rule} to .gitattributes")
            }
        }
        RepoCommandKind::AppendGitignorePatterns { patterns } => {
            if output.stdout.trim() == gitcomet_core::gitignore::NOTHING_TO_ADD {
                "Already in .gitignore; nothing added".to_string()
            } else {
                match patterns.as_slice() {
                    [pattern] => format!("Added {pattern} to .gitignore"),
                    patterns => format!("Added {} patterns to .gitignore", patterns.len()),
                }
            }
        }
        RepoCommandKind::Reset { mode, target } => {
            let mode = match mode {
                gitcomet_core::services::ResetMode::Soft => "soft",
                gitcomet_core::services::ResetMode::Mixed => "mixed",
                gitcomet_core::services::ResetMode::Hard => "hard",
            };
            format!("Reset (--{mode}) {target}: Completed")
        }
        RepoCommandKind::SquashCommits { count, .. } => {
            format!("Squash {count} commits: Completed")
        }
        RepoCommandKind::Rebase { onto } => format!("Rebase onto {onto}: Completed"),
        RepoCommandKind::RebaseContinue => {
            let operation = sequencer_operation_label(output, None);
            if output.command == gitcomet_core::services::REVERT_SKIP_COMMAND {
                "Revert: Skipped the revert the resolution left empty".to_string()
            } else if sequencer_paused(output) {
                format!("{operation}: Paused at the next conflict")
            } else {
                format!("{operation}: Continued")
            }
        }
        RepoCommandKind::RebaseAbort => {
            if output
                .stdout
                .contains(gitcomet_core::services::REVERT_ABORT_KEPT_HEAD_SENTINEL)
            {
                "Revert: Sequence cleared; HEAD was left where it is".to_string()
            } else {
                format!("{}: Aborted", sequencer_operation_label(output, None))
            }
        }
        RepoCommandKind::InteractiveRebase { base, interactive } => {
            let state = if sequencer_paused(output) {
                "Paused at a conflict"
            } else {
                "Completed"
            };
            if *interactive {
                format!("Interactive rebase onto {base}: {state}")
            } else {
                format!("Rebase onto {base}: {state}")
            }
        }
        RepoCommandKind::InteractiveCherryPick {
            entries,
            commit: false,
        } => {
            if output
                .stdout
                .contains("GITCOMET_CHERRY_PICK_ALREADY_APPLIED")
            {
                "Current branch already has all the changes from the cherry-picked commits."
                    .to_string()
            } else {
                format!("Cherry-picked {} commits without committing", entries.len())
            }
        }
        RepoCommandKind::InteractiveCherryPick {
            entries,
            commit: true,
        } => {
            let state = if sequencer_paused(output) {
                "Paused at a conflict"
            } else {
                "Completed"
            };
            format!("Cherry-pick {} commits: {state}", entries.len())
        }
        RepoCommandKind::CherryPick {
            commit_id,
            commit,
            summary,
            ..
        } => {
            if output
                .stdout
                .contains("GITCOMET_CHERRY_PICK_ALREADY_APPLIED")
            {
                "Current branch already has all the changes from the cherry-picked commit."
                    .to_string()
            } else {
                let short = commit_id.short();
                let summary = summary.lines().next().unwrap_or("").trim();
                if *commit {
                    format!("Cherry-picked {short}: {summary}")
                } else {
                    format!("Cherry-picked {short} without committing: {summary}")
                }
            }
        }
        RepoCommandKind::ApplyFileChange { target, commit, .. } => {
            let revision = gitcomet_core::services::apply_change_revision(&target.source);
            let already_applied = output
                .stdout
                .contains(gitcomet_core::services::APPLY_FILE_CHANGE_ALREADY_APPLIED_SENTINEL);
            match (target.paths.as_slice(), already_applied, *commit) {
                ([path], true, _) => format!(
                    "Current branch already has the change to {} from {revision}.",
                    path.display()
                ),
                ([path], false, true) => format!(
                    "Applied and committed the change to {} from {revision}",
                    path.display()
                ),
                ([path], false, false) => format!(
                    "Applied the change to {} from {revision} without committing",
                    path.display()
                ),
                (paths, true, _) => format!(
                    "Current branch already has the changes to these {} files from {revision}.",
                    paths.len()
                ),
                (paths, false, true) => format!(
                    "Applied and committed the changes to {} files from {revision}",
                    paths.len()
                ),
                (paths, false, false) => format!(
                    "Applied the changes to {} files from {revision} without committing",
                    paths.len()
                ),
            }
        }
        RepoCommandKind::Revert {
            commit_id,
            commit,
            summary,
            ..
        } => {
            let short = commit_id.short();
            if output
                .stdout
                .contains(gitcomet_core::services::REVERT_NOTHING_TO_REVERT_SENTINEL)
            {
                format!(
                    "Nothing to revert: the current branch no longer has the changes from {short}."
                )
            } else {
                let summary = summary.lines().next().unwrap_or("").trim();
                let subject = if summary.is_empty() {
                    String::new()
                } else {
                    format!(": {summary}")
                };
                if *commit {
                    format!("Reverted {short}{subject}")
                } else {
                    format!("Reverted {short} without committing{subject}")
                }
            }
        }
        RepoCommandKind::MergeAbort => "Merge: Aborted".to_string(),
        RepoCommandKind::CreateTag { name, target, .. } => {
            format!("Tag {name} → {target}: Created")
        }
        RepoCommandKind::DeleteTag { name } => format!("Tag {name}: Deleted"),
        RepoCommandKind::AddRemote { name, .. } => format!("Remote {name}: Added"),
        RepoCommandKind::RemoveRemote { name } => format!("Remote {name}: Removed"),
        RepoCommandKind::SetRemoteUrl { name, kind, .. } => {
            let kind = match kind {
                gitcomet_core::services::RemoteUrlKind::Fetch => "fetch",
                gitcomet_core::services::RemoteUrlKind::Push => "push",
            };
            format!("Remote {name} ({kind}): URL updated")
        }
        RepoCommandKind::ExportPatch { dest, .. } => {
            format!("Patch exported → {}", dest.display())
        }
        RepoCommandKind::ApplyPatch { patch } => format!("Patch applied → {}", patch.display()),
        RepoCommandKind::AddWorktree { path, checkout, .. } => format!(
            "Worktree added → {} ({})",
            path.display(),
            crate::msg::worktree_checkout_label(checkout)
        ),
        RepoCommandKind::RemoveWorktree { path } => {
            format!("Worktree removed → {}", path.display())
        }
        RepoCommandKind::ForceRemoveWorktree { path } => {
            format!("Worktree force removed → {}", path.display())
        }
        RepoCommandKind::AddSubmodule { path, .. } => {
            format!("Submodule added → {}", path.display())
        }
        RepoCommandKind::UpdateSubmodules { .. } => "Submodules: Updated".to_string(),
        RepoCommandKind::LoadSubmodule { path, .. } => {
            format!("Submodule loaded → {}", path.display())
        }
        RepoCommandKind::ChangeSubmodulePointer { path, reference } => {
            format!(
                "Submodule pointer updated → {} ({reference})",
                path.display()
            )
        }
        RepoCommandKind::RemoveSubmodule { path } => {
            format!("Submodule removed → {}", path.display())
        }
        RepoCommandKind::StageHunk => "Hunk staged".to_string(),
        RepoCommandKind::UnstageHunk => "Hunk unstaged".to_string(),
        RepoCommandKind::ApplyWorktreePatch { reverse } => {
            if *reverse {
                "Changes discarded".to_string()
            } else {
                "Patch applied".to_string()
            }
        }
    };

    (output.command.clone(), summary)
}

pub(super) fn format_error_for_user(error: &Error) -> String {
    match error.kind() {
        ErrorKind::Git(failure) => failure.to_string(),
        _ => error.to_string(),
    }
}

pub(super) fn format_failure_summary(label: &str, error: &Error) -> String {
    if let Some((_git_command, details)) = try_format_git_backend_error(error) {
        return format!("{label} failed:\n\n{details}");
    }
    format!("{label} failed:\n\n{}", format_error_for_user(error))
}

fn try_format_git_backend_error(error: &Error) -> Option<(String, String)> {
    match error.kind() {
        ErrorKind::Git(failure) => try_format_structured_git_failure(failure),
        ErrorKind::Backend(message) => try_format_git_backend_error_message(message),
        _ => None,
    }
}

fn try_format_structured_git_failure(failure: &GitFailure) -> Option<(String, String)> {
    let command = failure.command().trim().to_string();
    if !command.starts_with("git ") {
        return None;
    }
    let rendered = render_command_and_output(&command, failure.detail());
    Some((command, rendered))
}

fn try_format_git_backend_error_message(message: &str) -> Option<(String, String)> {
    let (command, output) = parse_failed_command_message(message)?;
    if !command.trim_start().starts_with("git ") {
        return None;
    }

    let rendered = render_command_and_output(&command, output.as_deref());
    Some((command, rendered))
}

fn parse_failed_command_message(message: &str) -> Option<(String, Option<String>)> {
    if let Some(idx) = message.find(" failed:") {
        let command = message[..idx].trim_end().to_string();
        let mut output = &message[(idx + " failed:".len())..];
        if output.starts_with(' ') {
            output = &output[1..];
        }
        let output = output.trim_end_matches(['\r', '\n']).to_string();
        return Some((command, (!output.is_empty()).then_some(output)));
    }

    let trimmed = message.trim_end_matches(['\r', '\n']);
    if let Some(command) = trimmed.strip_suffix(" failed") {
        return Some((command.trim_end().to_string(), None));
    }

    None
}

fn render_command_and_output(command: &str, output: Option<&str>) -> String {
    let command = command.replace(['\n', '\r'], " ");
    let command = command.trim();

    let output_len = output.map_or(0, |s| s.len());
    let mut rendered = String::with_capacity(command.len() + output_len + 16);
    append_code_block(&mut rendered, command);

    if let Some(output) = output {
        let output = output.trim_end_matches(['\r', '\n']);
        if !output.is_empty() {
            rendered.push_str("\n\n");
            append_code_block(&mut rendered, output);
        }
    }

    rendered
}

fn append_code_block(out: &mut String, text: &str) {
    for (ix, line) in text.lines().enumerate() {
        if ix > 0 {
            out.push('\n');
        }
        out.push_str("    ");
        out.push_str(line);
    }
}

trait IfEmptyElse {
    fn if_empty_else(self, f: impl FnOnce() -> String) -> String;
}

impl IfEmptyElse for String {
    fn if_empty_else(self, f: impl FnOnce() -> String) -> String {
        if self.trim().is_empty() { f() } else { self }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod delete_remote_branches_summary_tests;
