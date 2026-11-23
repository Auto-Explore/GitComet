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

use gitcomet_core::conflict_session::worktree_file_has_conflict_markers;
use gitcomet_state::msg::{StageGuard, conflicted_stage_paths};

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(in crate::view) enum StageIntent {
    Command {
        offer_confirmation: bool,
    },
    Status {
        clear_selection: bool,
    },
    Diff {
        target_revision: u64,
        multi: bool,
        consumes_selection: bool,
    },
}

pub(in crate::view) enum MarkerCheck {
    Clear(Option<StageGuard>),
    Marked(Vec<PathBuf>),
}

enum ScanPaths {
    Shown(Vec<PathBuf>),
    Discover(Arc<AppStore>),
}

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
    owner: gpui::EntityId,
    intent: StageIntent,
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
    fn stage_status_store(&self) -> Option<Arc<AppStore>>;
    fn live_conflict_paths(&self, shown: &RepoState, paths: &[PathBuf]) -> Option<ConflictPaths>;
}

fn conflict_paths(repo: &RepoState, paths: &[PathBuf]) -> ConflictPaths {
    let Some(entries) = repo.worktree_status_entries() else {
        return ConflictPaths::Unknown;
    };
    ConflictPaths::Known(conflicted_stage_paths(entries, paths))
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
    fn stage_status_store(&self) -> Option<Arc<AppStore>> {
        Some(self.store.clone())
    }
    fn stage_repository(&self) -> Option<&RepoState> {
        self.active_repo()
    }
    fn live_conflict_paths(&self, shown: &RepoState, paths: &[PathBuf]) -> Option<ConflictPaths> {
        selected_conflicts(&self.store, shown, paths)
    }
}

impl StageOwner for DetailsPaneView {
    fn stage_status_store(&self) -> Option<Arc<AppStore>> {
        Some(self.store.clone())
    }
    fn stage_repository(&self) -> Option<&RepoState> {
        self.active_repo()
    }
    fn live_conflict_paths(&self, shown: &RepoState, paths: &[PathBuf]) -> Option<ConflictPaths> {
        selected_conflicts(&self.store, shown, paths)
    }
}

impl StageOwner for DiffRenderer {
    fn stage_status_store(&self) -> Option<Arc<AppStore>> {
        self.store.stage_status_store()
    }
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
            || diff.diff_target_rev != self.bound_diff_state(shown).diff_target_rev
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
    intent: StageIntent,
    window: &mut Window,
    cx: &mut gpui::Context<T>,
    finish: impl FnOnce(&mut T, Vec<PathBuf>, MarkerCheck, &mut Window, &mut gpui::Context<T>) + 'static,
) {
    let Some(repo) = view.stage_repository().filter(|repo| repo.id == repo_id) else {
        return;
    };
    let key = ScanKey {
        window: window.window_handle().window_id(),
        owner: cx.entity_id(),
        intent,
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
    let mut scan = match status {
        ConflictPaths::Known(paths) if paths.is_empty() => {
            finish(
                view,
                Arc::unwrap_or_clone(key.paths),
                MarkerCheck::Clear(None),
                window,
                cx,
            );
            return;
        }
        ConflictPaths::Known(paths) => ScanPaths::Shown(paths),
        ConflictPaths::Unknown => match conflict_paths(repo, &key.paths).known() {
            Some(paths) if !paths.is_empty() => ScanPaths::Shown(paths),
            _ => match view.stage_status_store() {
                Some(store) => ScanPaths::Discover(store),
                None => return,
            },
        },
    };
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
            let requested = key.paths.clone();
            let lifetime = key.lifetime;
            let checked = executor.spawn(async move {
                let (paths, discovered) = match scan {
                    ScanPaths::Shown(paths) => (paths, false),
                    ScanPaths::Discover(store) => {
                        let repo = store.repository(repo_id, lifetime.get()).ok_or_else(|| "Repository status is unavailable. Retry staging when the repository is ready.".to_owned())?;
                        let status = repo.worktree_status(&gitcomet_core::services::CancellationToken::new()).map_err(|error| error.to_string())?;
                        (conflicted_stage_paths(&status, &requested), true)
                    }
                };
                let mut unresolved = Vec::new();
                for path in &paths {
                    if worktree_file_has_conflict_markers(&directory.join(path)).map_err(|error| format!("Could not check {} for conflict markers: {error}", path.display()))? {
                        unresolved.push(path.clone());
                    }
                }
                Ok::<_,String>((paths, unresolved, discovered))
            }).await;
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
                            let status = view.live_conflict_paths(shown, &key.paths)?;
                            let (checked, unresolved, discovered) = match checked {
                                Ok(check) => check,
                                Err(message) => {
                                    view.stage_status_store()?.dispatch(Msg::ReportError {repo_id: Some(repo_id), message});
                                    return None;
                                }
                            };
                            let result = match status {
                                ConflictPaths::Known(current) if current != checked && !current.is_empty() => return Some(ScanPaths::Shown(current)),
                                ConflictPaths::Known(current) if current.is_empty() => MarkerCheck::Clear(Some(StageGuard::CurrentStatus)),
                                ConflictPaths::Unknown if unresolved.is_empty() && !discovered => return view.stage_status_store().map(ScanPaths::Discover),
                                _ if !unresolved.is_empty() => MarkerCheck::Marked(unresolved),
                                _ if discovered => MarkerCheck::Clear(Some(StageGuard::CurrentStatus)),
                                _ => MarkerCheck::Clear(Some(StageGuard::Scanned(checked.into()))),
                            };
                            finish.take()?(view, key.paths.as_ref().clone(), result, window, cx);
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
            scan = next;
        }
        pending.remove(&key, &token);
    })
    .detach();
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
