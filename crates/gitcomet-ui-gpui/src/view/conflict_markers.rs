//! Guard against staging a merge that was never finished.
//!
//! Staging a conflicted file is how git is told the conflict is resolved, so a
//! file staged with its markers still in it silently commits `<<<<<<<` into the
//! branch. The paths this reports are offered to the user for confirmation
//! before the stage goes ahead.

use super::*;

/// How far into a worktree file the marker scan reads before giving up. The scan
/// streams and stops at the first closing marker, so this only bites on a file
/// whose markers never close — generous enough that a real conflict, which can
/// span a whole large file, is always resolved one way or the other.
const MAX_SCANNED_BYTES: u64 = 128 * 1024 * 1024;

/// Of `paths`, the ones git still reports as conflicted **and** whose worktree
/// file still contains conflict markers. An empty `paths` means "everything",
/// matching `Msg::StagePaths`.
///
/// Only conflicted paths are read, and an ordinary stage has none, so this costs
/// no file IO in the common case.
pub(in crate::view) fn unresolved_conflict_marker_paths(
    state: &AppState,
    repo_id: RepoId,
    paths: &[std::path::PathBuf],
) -> Vec<std::path::PathBuf> {
    let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    let Loadable::Ready(status) = &repo.status else {
        return Vec::new();
    };

    let workdir = &repo.spec.workdir;
    status
        .unstaged
        .iter()
        .filter(|entry| entry.conflict.is_some())
        .filter(|entry| paths.is_empty() || paths.contains(&entry.path))
        .filter(|entry| worktree_file_has_conflict_markers(&workdir.join(&entry.path)))
        .map(|entry| entry.path.clone())
        .collect()
}

fn worktree_file_has_conflict_markers(path: &std::path::Path) -> bool {
    // Streamed rather than read whole: a conflict can span a multi-megabyte
    // file, and sizing the file out of the scan is what silently skipped the
    // warning. The scan stops at the first closing marker.
    let Ok(file) = std::fs::File::open(path) else {
        // A path that cannot be read cannot be shown to have markers; staging
        // will surface whatever the real problem is.
        return false;
    };
    gitcomet_core::conflict_session::reader_has_conflict_markers(
        std::io::BufReader::new(file),
        MAX_SCANNED_BYTES,
    )
    .unwrap_or(false)
}

/// The confirmation to show before staging `paths`, or `None` when nothing about
/// the stage is unresolved and it can go ahead as issued.
pub(in crate::view) fn stage_confirm_popover(
    state: &AppState,
    repo_id: RepoId,
    paths: Vec<std::path::PathBuf>,
) -> Option<PopoverKind> {
    let unresolved = unresolved_conflict_marker_paths(state, repo_id, &paths);
    (!unresolved.is_empty()).then_some(PopoverKind::StageConflictMarkersConfirm {
        repo_id,
        paths,
        unresolved,
    })
}

/// Anchor for a stage confirmation raised by a keyboard shortcut, which has no
/// pointer position of its own.
pub(in crate::view) fn centered_dialog_anchor(window: &Window) -> gpui::Point<Pixels> {
    let bounds = window.window_bounds().get_bounds();
    gpui::point(
        (bounds.size.width * 0.5).max(px(64.0)),
        (bounds.size.height * 0.25).max(px(24.0)),
    )
}
