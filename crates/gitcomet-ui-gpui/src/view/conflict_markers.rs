//! Guard against staging a merge that was never finished.
//!
//! Staging a conflicted file is how git is told the conflict is resolved, so a
//! file staged with its markers still in it silently commits `<<<<<<<` into the
//! branch. The paths this reports are offered to the user for confirmation
//! before the stage goes ahead.

use super::*;

/// How far into a worktree file the marker scan reads before giving up. The scan
/// runs on the UI thread while a click waits on it, and it reads to the end of a
/// file whose markers are already resolved — the common case — so this is what
/// bounds that. Big enough for a conflict spanning a whole ordinary source file,
/// small enough that a handful of them cost milliseconds; past it an unclosed
/// opener still warns rather than passing as resolved.
const MAX_SCANNED_BYTES: u64 = 16 * 1024 * 1024;

/// Read buffer for the scan. Larger than the 8KB default because the scan is a
/// straight sequential walk, and the file it walks furthest is the resolved one
/// it has to read to the end of.
const SCAN_BUFFER_BYTES: usize = 64 * 1024;

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

    // Set rather than a scan of `paths` per entry: "stage all" during a large
    // merge puts every changed path in both lists.
    let requested: FxHashSet<&std::path::Path> =
        paths.iter().map(std::path::PathBuf::as_path).collect();
    let workdir = &repo.spec.workdir;
    status
        .unstaged
        .iter()
        .filter(|entry| entry.conflict.is_some())
        .filter(|entry| requested.is_empty() || requested.contains(entry.path.as_path()))
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
        std::io::BufReader::with_capacity(SCAN_BUFFER_BYTES, file),
        MAX_SCANNED_BYTES,
    )
    .unwrap_or(false)
}

/// The confirmation to show before staging `paths`, or `None` when nothing about
/// the stage is unresolved and it can go ahead as issued.
///
/// `clear_selection` is passed through to the dialog: callers must resolve
/// `paths` out of the status row selection *without* consuming it, then hand
/// that answer over here, so a cancelled dialog leaves the selection standing.
pub(in crate::view) fn stage_confirm_popover(
    state: &AppState,
    repo_id: RepoId,
    paths: Vec<std::path::PathBuf>,
    clear_selection: bool,
) -> Option<PopoverKind> {
    let unresolved = unresolved_conflict_marker_paths(state, repo_id, &paths);
    (!unresolved.is_empty()).then_some(PopoverKind::StageConflictMarkersConfirm {
        repo_id,
        paths,
        unresolved,
        clear_selection,
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
