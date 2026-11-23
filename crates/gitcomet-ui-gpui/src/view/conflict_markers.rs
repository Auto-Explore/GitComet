//! Guard against staging a merge that was never finished.
//!
//! Staging a conflicted file is how git is told the conflict is resolved, so a
//! file staged with its markers still in it silently commits `<<<<<<<` into the
//! branch. The paths this reports are offered to the user for confirmation
//! before the stage goes ahead.

#![allow(
    clippy::disallowed_methods,
    reason = "bounded marker scan before a stage"
)]

use super::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const MAX_SCANNED_BYTES: u64 = 16 * 1024 * 1024;
const SCAN_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Default)]
struct PendingScans(Rc<RefCell<FxHashMap<ScanKey, Rc<()>>>>);

impl gpui::Global for PendingScans {}

impl PendingScans {
    fn current(&self, key: &ScanKey, token: &Rc<()>) -> bool {
        self.0
            .borrow()
            .get(key)
            .is_some_and(|current| Rc::ptr_eq(current, token))
    }

    fn remove(&self, key: &ScanKey, token: &Rc<()>) {
        if self.current(key, token) {
            self.0.borrow_mut().remove(key);
        }
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct ScanKey {
    window: gpui::WindowId,
    lifetime: gitcomet_state::model::RepoLifetime,
    paths: Arc<Vec<PathBuf>>,
}

pub(in crate::view) enum ConflictPaths {
    Unknown,
    Known(Vec<PathBuf>),
}

impl ConflictPaths {
    fn known(self) -> Option<Vec<PathBuf>> {
        match self {
            Self::Known(paths) => Some(paths),
            Self::Unknown => None,
        }
    }
}

pub(in crate::view) trait StageOwner: Sized + 'static {
    fn stage_repository(&self) -> Option<&RepoState>;
    fn live_conflict_paths(&self, shown: &RepoState, paths: &[PathBuf]) -> Option<ConflictPaths>;
}

fn conflict_paths(repo: &RepoState, paths: &[PathBuf]) -> ConflictPaths {
    let Some(entries) = repo.worktree_status_entries() else {
        return ConflictPaths::Unknown;
    };
    if !entries.iter().any(|entry| entry.conflict.is_some()) {
        return ConflictPaths::Known(Vec::new());
    }
    let requested: FxHashSet<_> = paths.iter().collect();
    let conflicts = entries
        .iter()
        .filter(|entry| {
            entry.conflict.is_some() && (requested.is_empty() || requested.contains(&entry.path))
        })
        .map(|entry| entry.path.clone())
        .collect();
    ConflictPaths::Known(conflicts)
}

fn current_repo<'a>(live: &'a AppState, shown: &RepoState) -> Option<&'a RepoState> {
    live.repos
        .iter()
        .find(|repo| repo.id == shown.id && repo.lifetime_id() == shown.lifetime_id())
}

fn selected_conflicts(
    store: &AppStore,
    shown: &RepoState,
    paths: &[PathBuf],
) -> Option<ConflictPaths> {
    let live = store.snapshot();
    (live.active_repo == Some(shown.id)).then_some(())?;
    Some(conflict_paths(current_repo(&live, shown)?, paths))
}

impl StageOwner for GitCometView {
    fn stage_repository(&self) -> Option<&RepoState> {
        self.active_repo()
    }
    fn live_conflict_paths(&self, shown: &RepoState, paths: &[PathBuf]) -> Option<ConflictPaths> {
        selected_conflicts(&self.store, shown, paths)
    }
}

impl StageOwner for DetailsPaneView {
    fn stage_repository(&self) -> Option<&RepoState> {
        self.active_repo()
    }
    fn live_conflict_paths(&self, shown: &RepoState, paths: &[PathBuf]) -> Option<ConflictPaths> {
        selected_conflicts(&self.store, shown, paths)
    }
}

impl StageOwner for DiffRenderer {
    fn stage_repository(&self) -> Option<&RepoState> {
        self.active_repo()
    }
    fn live_conflict_paths(&self, shown: &RepoState, paths: &[PathBuf]) -> Option<ConflictPaths> {
        let live = self.store.snapshot()?;
        let repo = current_repo(&live, shown)?;
        let diff = match self.store.binding {
            Some(binding) => binding.diff(repo)?,
            None if live.active_repo == Some(repo.id) => &repo.diff_state,
            None => return None,
        };
        if !self.store.policy.allow_stage
            || diff.content_preview
            || diff.diff_target != self.bound_diff_state(shown).diff_target
        {
            return None;
        }
        Some(conflict_paths(repo, paths))
    }
}

pub(in crate::view) fn check<T: StageOwner>(
    view: &mut T,
    repo_id: RepoId,
    paths: Vec<PathBuf>,
    window: &mut Window,
    cx: &mut gpui::Context<T>,
    finish: impl FnOnce(&mut T, Vec<PathBuf>, Vec<PathBuf>, &mut Window, &mut gpui::Context<T>)
    + 'static,
) {
    let Some(repo) = view.stage_repository().filter(|repo| repo.id == repo_id) else {
        return;
    };
    let key = ScanKey {
        window: window.window_handle().window_id(),
        lifetime: repo.lifetime_id(),
        paths: Arc::new(paths),
    };
    let pending = cx.default_global::<PendingScans>().clone();
    if pending.0.borrow().contains_key(&key) {
        return;
    }
    let Some(status) = view.live_conflict_paths(repo, &key.paths) else {
        return;
    };
    let mut conflicts = status
        .known()
        .or_else(|| conflict_paths(repo, &key.paths).known())
        .unwrap_or_default();
    if conflicts.is_empty() {
        finish(
            view,
            Arc::unwrap_or_clone(key.paths),
            Vec::new(),
            window,
            cx,
        );
        return;
    }
    let workdir = repo.spec.workdir.clone();
    let token = Rc::new(());
    pending.0.borrow_mut().insert(key.clone(), token.clone());
    let cancel = cx.observe_keystrokes({
        let pending = pending.clone();
        let key = key.clone();
        let token = token.clone();
        move |_, event, window, _| {
            if window.window_handle().window_id() == key.window
                && event.keystroke.key == "escape"
                && !event.keystroke.modifiers.modified()
            {
                pending.remove(&key, &token);
            }
        }
    });
    let executor = cx.background_executor().clone();
    cx.spawn_in(window, async move |owner, cx| {
        let _cancel = cancel;
        let mut finish = Some(finish);
        loop {
            let directory = workdir.clone();
            let (checked, unresolved) = executor
                .spawn(async move {
                    let unresolved = conflicts
                        .iter()
                        .filter(|path| worktree_file_has_conflict_markers(&directory.join(path)))
                        .cloned()
                        .collect::<Vec<_>>();
                    (conflicts, unresolved)
                })
                .await;
            if !pending.current(&key, &token) {
                return;
            }
            let next = cx
                .update(|window, app| {
                    owner
                        .update(app, |view, cx| {
                            let shown = view.stage_repository()?;
                            if shown.id != repo_id || shown.lifetime_id() != key.lifetime {
                                return None;
                            }
                            let unresolved = match view.live_conflict_paths(shown, &key.paths)? {
                                ConflictPaths::Known(current) if current == checked => unresolved,
                                ConflictPaths::Known(current) if current.is_empty() => Vec::new(),
                                ConflictPaths::Known(current) => return Some(current),
                                ConflictPaths::Unknown if unresolved.is_empty() => return None,
                                ConflictPaths::Unknown => unresolved,
                            };
                            finish.take()?(
                                view,
                                key.paths.as_ref().clone(),
                                unresolved,
                                window,
                                cx,
                            );
                            None
                        })
                        .ok()
                        .flatten()
                })
                .ok()
                .flatten();
            let Some(next) = next else {
                break;
            };
            conflicts = next;
        }
        pending.remove(&key, &token);
    })
    .detach();
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

/// Anchor for a stage confirmation raised by a keyboard shortcut, which has no
/// pointer position of its own.
pub(in crate::view) fn centered_dialog_anchor(window: &Window) -> gpui::Point<Pixels> {
    let surface = crate::kit::window_chrome::window_surface_bounds(window);
    gpui::point(
        surface.left() + (surface.size.width * 0.5).max(px(64.0)),
        surface.top() + (surface.size.height * 0.25).max(px(24.0)),
    )
}
