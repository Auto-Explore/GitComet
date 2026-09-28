use gitcomet_state::session::{
    self, PortableWindowPlacement, SavedWindowFrame, Workspace, WorkspaceId, WorkspaceLayout,
};
use gpui::{App, BorrowAppContext, WindowId};
use rustc_hash::FxHashMap;
use std::borrow::BorrowMut;
use std::cell::{Ref, RefCell};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// Durable workspaces and which live window holds each. State sits behind a
/// `RefCell` so an edit leases the global (notifying its observers, such as an
/// open Settings window) only when it actually changed something.
#[derive(Default)]
pub(crate) struct WorkspaceManager {
    state: RefCell<ManagerState>,
}

impl gpui::Global for WorkspaceManager {}

#[derive(Default)]
struct ManagerState {
    enabled: bool,
    writer: Option<Arc<WorkspaceWriter>>,
    workspaces: Vec<Workspace>,
    window_workspaces: FxHashMap<WindowId, WorkspaceId>,
    focused_window: Option<WindowId>,
    active_workspace: Option<WorkspaceId>,
    next_activation_order: u64,
    revision: u64,
    /// Bounds recorded in memory since the last write.
    placement_unsaved: bool,
}

/// What an edit did: nothing, something observers show, or something that
/// also belongs on disk.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    None,
    Notify,
    Persist,
}

impl Change {
    /// `Persist` when durable data changed, else `Notify` when only the live
    /// window mapping did.
    fn of(changed: bool, remapped: bool) -> Self {
        match (changed, remapped) {
            (true, _) => Self::Persist,
            (false, true) => Self::Notify,
            (false, false) => Self::None,
        }
    }
}

impl ManagerState {
    fn enabled(workspaces: Vec<Workspace>, writer: Option<Arc<WorkspaceWriter>>) -> Self {
        let next_activation_order = workspaces
            .iter()
            .map(|workspace| workspace.last_activation_order)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Self {
            enabled: true,
            writer,
            workspaces,
            next_activation_order,
            ..Self::default()
        }
    }

    fn workspace_index(&self, id: WorkspaceId) -> Option<usize> {
        self.workspaces
            .iter()
            .position(|workspace| workspace.id == id)
    }

    fn allocate_activation_order(&mut self) -> u64 {
        let order = self.next_activation_order;
        self.next_activation_order = self.next_activation_order.saturating_add(1);
        order
    }

    /// Make `workspace_id` the frontmost workspace and stamp its open time.
    fn activate(&mut self, workspace_id: WorkspaceId) -> bool {
        self.active_workspace = Some(workspace_id);
        let order = self.allocate_activation_order();
        let Some(index) = self.workspace_index(workspace_id) else {
            return false;
        };
        let workspace = &mut self.workspaces[index];
        workspace.last_activation_order = order;
        workspace.last_opened_at = Some(session::unix_time_now());
        true
    }

    /// Focus can arrive while a window is still empty and has no workspace
    /// mapping. Replay it once the window gains a workspace so the previously
    /// focused one is not mistaken for the frontmost.
    fn replay_focus(&mut self, window_id: WindowId, workspace_id: WorkspaceId) -> bool {
        self.focused_window == Some(window_id)
            && self.active_workspace != Some(workspace_id)
            && self.activate(workspace_id)
    }

    fn pending_write(&mut self) -> Option<(Arc<WorkspaceWriter>, Vec<Workspace>)> {
        self.placement_unsaved = false;
        Some((Arc::clone(self.writer.as_ref()?), self.workspaces.clone()))
    }
}

/// Run `edit` on the manager, then notify observers and queue a disk write as
/// it reports. Without an initialized manager it sees a disabled state.
fn edit<C, R>(cx: &mut C, edit: impl FnOnce(&mut ManagerState) -> (R, Change)) -> R
where
    C: BorrowMut<App>,
{
    let app = cx.borrow_mut();
    let Some(manager) = app.try_global::<WorkspaceManager>() else {
        return edit(&mut ManagerState::default()).0;
    };
    let (result, change, write) = {
        let mut state = manager.state.borrow_mut();
        let (result, change) = edit(&mut state);
        if change != Change::None {
            state.revision = state.revision.wrapping_add(1);
        }
        let write = (change == Change::Persist)
            .then(|| state.pending_write())
            .flatten();
        (result, change, write)
    };
    if change != Change::None {
        app.update_global::<WorkspaceManager, _>(|_, _| {});
    }
    if let Some((writer, workspaces)) = write {
        writer.enqueue(workspaces);
    }
    result
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

type WorkspaceSink = dyn Fn(&[Workspace]) + Send + Sync;

/// Writes workspace snapshots off the UI thread (each is an fsync'd session
/// rewrite). Only the newest queued snapshot is written.
struct WorkspaceWriter {
    pending: Mutex<Option<Vec<Workspace>>>,
    wake: Condvar,
    /// Held across a write, so a queued snapshot cannot land after `write_now`.
    writing: Mutex<()>,
    worker_running: AtomicBool,
    sink: Box<WorkspaceSink>,
}

impl WorkspaceWriter {
    fn spawn(sink: Box<WorkspaceSink>) -> Arc<Self> {
        let writer = Arc::new(Self {
            pending: Mutex::new(None),
            wake: Condvar::new(),
            writing: Mutex::new(()),
            worker_running: AtomicBool::new(true),
            sink,
        });
        let worker = Arc::clone(&writer);
        let spawned = std::thread::Builder::new()
            .name("gitcomet-workspace-writer".to_string())
            .spawn(move || worker.run());
        if spawned.is_err() {
            writer.worker_running.store(false, Ordering::SeqCst);
        }
        writer
    }

    fn enqueue(&self, workspaces: Vec<Workspace>) {
        if !self.worker_running.load(Ordering::SeqCst) {
            self.write_now(&workspaces);
            return;
        }
        *lock(&self.pending) = Some(workspaces);
        self.wake.notify_one();
    }

    /// Write synchronously, superseding anything still queued. Used at quit.
    fn write_now(&self, workspaces: &[Workspace]) {
        let _writing = lock(&self.writing);
        lock(&self.pending).take();
        (self.sink)(workspaces);
    }

    fn run(&self) {
        loop {
            {
                let mut pending = lock(&self.pending);
                while pending.is_none() {
                    pending = self
                        .wake
                        .wait(pending)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            }
            let _writing = lock(&self.writing);
            let Some(workspaces) = lock(&self.pending).take() else {
                continue;
            };
            (self.sink)(&workspaces);
        }
    }
}

fn persist_workspaces_to_session(workspaces: &[Workspace]) {
    if let Err(error) = session::persist_workspaces(workspaces) {
        eprintln!("Failed to persist workspaces: {error}");
    }
}

pub(crate) fn initialize(cx: &mut App, workspaces: Vec<Workspace>) {
    let writer = WorkspaceWriter::spawn(Box::new(persist_workspaces_to_session));
    cx.set_global(WorkspaceManager {
        state: RefCell::new(ManagerState::enabled(workspaces, Some(writer))),
    });
}

#[cfg(test)]
pub(crate) fn initialize_for_test(cx: &mut App, workspaces: Vec<Workspace>) {
    cx.set_global(WorkspaceManager {
        state: RefCell::new(ManagerState::enabled(workspaces, None)),
    });
}

/// Queue a write of bounds recorded since the last one.
pub(crate) fn persist_unsaved_placement(cx: &mut App) {
    edit(cx, |manager| {
        let unsaved = manager.enabled && manager.placement_unsaved;
        ((), Change::of(unsaved, false))
    });
}

/// Write the current workspaces synchronously; the app is about to exit.
pub(crate) fn flush_to_disk(cx: &mut App) {
    let write = cx.try_global::<WorkspaceManager>().and_then(|manager| {
        let mut state = manager.state.borrow_mut();
        state.enabled.then(|| state.pending_write()).flatten()
    });
    if let Some((writer, workspaces)) = write {
        writer.write_now(&workspaces);
    }
}

/// Synchronize durable workspace membership with one normal window and return the
/// workspace identity the view should retain. Empty ephemeral windows remain
/// identity-less; an empty window deletes an anonymous workspace but keeps a
/// customized one (the window then shows Home inside it).
pub(crate) fn sync_window<C>(
    cx: &mut C,
    window_id: WindowId,
    requested_workspace_id: Option<WorkspaceId>,
    repositories: Vec<PathBuf>,
    active_repository: Option<PathBuf>,
) -> Option<WorkspaceId>
where
    C: BorrowMut<App>,
{
    edit(cx, |manager| {
        if !manager.enabled {
            return (requested_workspace_id, Change::None);
        }

        if repositories.is_empty() {
            let workspace_id = requested_workspace_id
                .or_else(|| manager.window_workspaces.get(&window_id).copied());
            if let Some(workspace_id) = workspace_id
                && let Some(index) = manager.workspace_index(workspace_id)
                && manager.workspaces[index].is_customized()
            {
                let remapped =
                    manager.window_workspaces.insert(window_id, workspace_id) != Some(workspace_id);
                let workspace = &mut manager.workspaces[index];
                let mut changed = !workspace.repositories.is_empty()
                    || workspace.active_repository.is_some()
                    || !workspace.restore_on_launch;
                workspace.repositories.clear();
                workspace.active_repository = None;
                workspace.restore_on_launch = true;
                changed |= manager.replay_focus(window_id, workspace_id);
                return (Some(workspace_id), Change::of(changed, remapped));
            }
            let mut unmapped = manager.window_workspaces.remove(&window_id).is_some();
            if manager.focused_window == Some(window_id) && manager.active_workspace.is_some() {
                manager.active_workspace = None;
                unmapped = true;
            }
            let Some(workspace_id) = workspace_id else {
                return (None, Change::of(false, unmapped));
            };
            let before = manager.workspaces.len();
            manager
                .workspaces
                .retain(|workspace| workspace.id != workspace_id);
            if manager.active_workspace == Some(workspace_id) {
                manager.active_workspace = None;
            }
            return (
                None,
                Change::of(manager.workspaces.len() != before, unmapped),
            );
        }

        let workspace_id = requested_workspace_id
            .or_else(|| manager.window_workspaces.get(&window_id).copied())
            .unwrap_or_default();
        let remapped =
            manager.window_workspaces.insert(window_id, workspace_id) != Some(workspace_id);
        let active_repository = active_repository
            .filter(|active| repositories.contains(active))
            .or_else(|| repositories.first().cloned());

        let mut changed = false;
        if let Some(index) = manager.workspace_index(workspace_id) {
            let workspace = &mut manager.workspaces[index];
            if workspace.repositories != repositories {
                workspace.repositories = repositories;
                changed = true;
            }
            if workspace.active_repository != active_repository {
                workspace.active_repository = active_repository;
                changed = true;
            }
            if !workspace.restore_on_launch {
                workspace.restore_on_launch = true;
                changed = true;
            }
        } else {
            let mut workspace = Workspace::new(repositories);
            workspace.id = workspace_id;
            workspace.active_repository = active_repository;
            workspace.last_activation_order = manager.allocate_activation_order();
            manager.workspaces.push(workspace);
            changed = true;
        }

        changed |= manager.replay_focus(window_id, workspace_id);

        (Some(workspace_id), Change::of(changed, remapped))
    })
}

pub(crate) fn mark_window_active<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowMut<App>,
{
    edit(cx, |manager| {
        if !manager.enabled {
            return ((), Change::None);
        }
        manager.focused_window = Some(window_id);
        let Some(workspace_id) = manager.window_workspaces.get(&window_id).copied() else {
            let had_active = manager.active_workspace.take().is_some();
            return ((), Change::of(false, had_active));
        };
        if manager.active_workspace == Some(workspace_id) {
            return ((), Change::None);
        }
        manager.activate(workspace_id);
        // Focus order is immediately observable, but joins the next meaningful
        // workspace write (or shutdown flush) instead of fsyncing on each focus.
        ((), Change::Notify)
    });
}

pub(crate) fn mark_window_closed<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowMut<App>,
{
    edit(cx, |manager| {
        if !manager.enabled {
            return ((), Change::None);
        }
        if manager.focused_window == Some(window_id) {
            manager.focused_window = None;
        }
        let Some(workspace_id) = manager.window_workspaces.remove(&window_id) else {
            return ((), Change::None);
        };
        if manager.active_workspace == Some(workspace_id) {
            manager.active_workspace = None;
        }
        let Some(index) = manager.workspace_index(workspace_id) else {
            return ((), Change::Notify);
        };
        let workspace = &mut manager.workspaces[index];
        if !workspace.restore_on_launch {
            return ((), Change::Notify);
        }
        workspace.restore_on_launch = false;
        ((), Change::Persist)
    });
}

/// Unmap a window from its workspace and mark that workspace closed, keeping
/// focus bookkeeping. Used when a Home window adopts a different workspace.
pub(crate) fn release_window_workspace<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowMut<App>,
{
    edit(cx, |manager| {
        if !manager.enabled {
            return ((), Change::None);
        }
        let Some(workspace_id) = manager.window_workspaces.remove(&window_id) else {
            return ((), Change::None);
        };
        if manager.active_workspace == Some(workspace_id) {
            manager.active_workspace = None;
        }
        let Some(index) = manager.workspace_index(workspace_id) else {
            return ((), Change::Notify);
        };
        let workspace = &mut manager.workspaces[index];
        if !workspace.restore_on_launch {
            return ((), Change::Notify);
        }
        workspace.restore_on_launch = false;
        ((), Change::Persist)
    });
}

/// Remove a live window and its durable workspace entirely. This is used when a
/// repository move empties the source window of an anonymous workspace: unlike
/// an explicit user close, there is nothing left to recover. Callers keep a
/// customized workspace (and its window) instead.
pub(crate) fn discard_workspace_for_window<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowMut<App>,
{
    edit(cx, |manager| {
        if !manager.enabled {
            return ((), Change::None);
        }
        if manager.focused_window == Some(window_id) {
            manager.focused_window = None;
        }
        let Some(workspace_id) = manager.window_workspaces.remove(&window_id) else {
            return ((), Change::None);
        };
        if manager.active_workspace == Some(workspace_id) {
            manager.active_workspace = None;
        }
        let before = manager.workspaces.len();
        manager
            .workspaces
            .retain(|workspace| workspace.id != workspace_id);
        ((), Change::of(manager.workspaces.len() != before, true))
    });
}

/// Remove a closed/stale durable workspace by identity. Recovery uses this when
/// every repository saved in the workspace already belongs to a live window.
pub(crate) fn discard_workspace<C>(cx: &mut C, workspace_id: WorkspaceId)
where
    C: BorrowMut<App>,
{
    edit(cx, |manager| {
        if !manager.enabled {
            return ((), Change::None);
        }
        manager
            .window_workspaces
            .retain(|_, mapped_workspace_id| *mapped_workspace_id != workspace_id);
        if manager.active_workspace == Some(workspace_id) {
            manager.active_workspace = None;
        }
        let before = manager.workspaces.len();
        manager
            .workspaces
            .retain(|workspace| workspace.id != workspace_id);
        ((), Change::of(manager.workspaces.len() != before, false))
    });
}

/// Apply `edit` to one workspace, live or recoverable, and persist on change.
/// A workspace left empty and uncustomized is removed, matching what
/// `sync_window` would do. Callers repaint any live window that owns it.
fn update_workspace<C>(
    cx: &mut C,
    workspace_id: WorkspaceId,
    edit_workspace: impl FnOnce(&mut Workspace) -> bool,
) -> bool
where
    C: BorrowMut<App>,
{
    edit(cx, |manager| {
        if !manager.enabled {
            return (false, Change::None);
        }
        let Some(index) = manager.workspace_index(workspace_id) else {
            return (false, Change::None);
        };
        if !edit_workspace(&mut manager.workspaces[index]) {
            return (false, Change::None);
        }
        let workspace = &manager.workspaces[index];
        if workspace.repositories.is_empty() && !workspace.is_customized() {
            manager.workspaces.remove(index);
            manager
                .window_workspaces
                .retain(|_, mapped| *mapped != workspace_id);
            if manager.active_workspace == Some(workspace_id) {
                manager.active_workspace = None;
            }
        }
        (true, Change::Persist)
    })
}

fn replace_if_changed<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

pub(crate) fn set_workspace_color<C>(
    cx: &mut C,
    workspace_id: WorkspaceId,
    color: Option<session::WorkspaceColor>,
) -> bool
where
    C: BorrowMut<App>,
{
    update_workspace(cx, workspace_id, |workspace| {
        replace_if_changed(&mut workspace.color, color)
    })
}

/// A blank name clears it, falling back to the automatic name.
pub(crate) fn set_workspace_name<C>(cx: &mut C, workspace_id: WorkspaceId, name: &str) -> bool
where
    C: BorrowMut<App>,
{
    let name = Some(name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    update_workspace(cx, workspace_id, |workspace| {
        replace_if_changed(&mut workspace.custom_name, name)
    })
}

/// `None` follows the app theme; otherwise a `theme_mode` key.
pub(crate) fn set_workspace_theme_mode<C>(
    cx: &mut C,
    workspace_id: WorkspaceId,
    theme_mode: Option<String>,
) -> bool
where
    C: BorrowMut<App>,
{
    update_workspace(cx, workspace_id, |workspace| {
        replace_if_changed(&mut workspace.theme_mode, theme_mode)
    })
}

/// Save a window's layout, plus any bounds recorded since the last write.
pub(crate) fn update_window_environment<C>(
    cx: &mut C,
    window_id: WindowId,
    layout: WorkspaceLayout,
    placement: Option<PortableWindowPlacement>,
) where
    C: BorrowMut<App>,
{
    edit(cx, |manager| {
        if !manager.enabled {
            return ((), Change::None);
        }
        let Some(index) = manager
            .window_workspaces
            .get(&window_id)
            .and_then(|workspace_id| manager.workspace_index(*workspace_id))
        else {
            return ((), Change::None);
        };
        let workspace = &mut manager.workspaces[index];
        let mut changed = replace_if_changed(&mut workspace.layout, layout);
        if let Some(placement) = placement {
            changed |= replace_if_changed(&mut workspace.placement, placement);
        }
        ((), Change::of(changed || manager.placement_unsaved, false))
    });
}

/// Record bounds in memory only; `update_window_environment` or quit writes
/// them. No observer shows placement, so this never notifies.
pub(crate) fn record_window_placement<C>(
    cx: &mut C,
    window_id: WindowId,
    placement: PortableWindowPlacement,
) where
    C: BorrowMut<App>,
{
    edit(cx, |manager| {
        if !manager.enabled {
            return ((), Change::None);
        }
        let Some(index) = manager
            .window_workspaces
            .get(&window_id)
            .and_then(|workspace_id| manager.workspace_index(*workspace_id))
        else {
            return ((), Change::None);
        };
        if replace_if_changed(&mut manager.workspaces[index].placement, placement) {
            manager.placement_unsaved = true;
        }
        ((), Change::None)
    });
}

/// Rebase a saved frame into the usable area of the display available now.
/// The relative position is retained when the old usable display frame is
/// known, then both dimensions and origin are clamped so no window can restore
/// entirely off-screen after a monitor or DPI change.
pub(crate) fn rebase_window_frame(
    saved: SavedWindowFrame,
    captured_visible: Option<SavedWindowFrame>,
    current_visible: SavedWindowFrame,
    minimum_width: u32,
    minimum_height: u32,
) -> SavedWindowFrame {
    let available_width = current_visible.width.max(1);
    let available_height = current_visible.height.max(1);
    let width = saved
        .width
        .max(minimum_width.min(available_width))
        .min(available_width);
    let height = saved
        .height
        .max(minimum_height.min(available_height))
        .min(available_height);

    fn rebase_axis(
        saved_origin: i32,
        saved_size: u32,
        captured_origin: Option<i32>,
        captured_size: Option<u32>,
        current_origin: i32,
        current_size: u32,
        restored_size: u32,
    ) -> i32 {
        let current_travel = current_size.saturating_sub(restored_size) as f64;
        let proposed = match (captured_origin, captured_size) {
            (Some(old_origin), Some(old_size)) => {
                let old_travel = old_size.saturating_sub(saved_size) as f64;
                let relative = if old_travel > 0.0 {
                    ((saved_origin as f64 - old_origin as f64) / old_travel).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                current_origin as f64 + relative * current_travel
            }
            _ => saved_origin as f64,
        };
        let low = current_origin as f64;
        let high = low + current_travel;
        proposed.clamp(low, high).round() as i32
    }

    SavedWindowFrame {
        x: rebase_axis(
            saved.x,
            saved.width,
            captured_visible.map(|frame| frame.x),
            captured_visible.map(|frame| frame.width),
            current_visible.x,
            current_visible.width,
            width,
        ),
        y: rebase_axis(
            saved.y,
            saved.height,
            captured_visible.map(|frame| frame.y),
            captured_visible.map(|frame| frame.height),
            current_visible.y,
            current_visible.height,
            height,
        ),
        width,
        height,
    }
}

/// Title-bar colours offered for a workspace, `None` being the theme default.
pub(crate) const WORKSPACE_COLORS: [(Option<session::WorkspaceColor>, &str); 15] = [
    (None, "Default"),
    (Some(session::WorkspaceColor::Gray), "Gray"),
    (Some(session::WorkspaceColor::Brown), "Brown"),
    (Some(session::WorkspaceColor::Red), "Red"),
    (Some(session::WorkspaceColor::Orange), "Orange"),
    (Some(session::WorkspaceColor::Yellow), "Yellow"),
    (Some(session::WorkspaceColor::Lime), "Lime"),
    (Some(session::WorkspaceColor::Green), "Green"),
    (Some(session::WorkspaceColor::Teal), "Teal"),
    (Some(session::WorkspaceColor::Cyan), "Cyan"),
    (Some(session::WorkspaceColor::Blue), "Blue"),
    (Some(session::WorkspaceColor::Indigo), "Indigo"),
    (Some(session::WorkspaceColor::Purple), "Purple"),
    (Some(session::WorkspaceColor::Magenta), "Magenta"),
    (Some(session::WorkspaceColor::Pink), "Pink"),
];

pub(crate) fn repository_count_label(count: usize) -> String {
    match count {
        0 => "No repositories".to_string(),
        1 => "1 repository".to_string(),
        n => format!("{n} repositories"),
    }
}

/// Read-only view of the manager. Reads must not lease the global: gpui
/// notifies every global observer on each lease, and the title bar reads the
/// manager per frame, so a leasing read plus an observer is a repaint loop.
fn manager(cx: &App) -> Option<Ref<'_, ManagerState>> {
    cx.try_global::<WorkspaceManager>()
        .map(|manager| manager.state.borrow())
}

pub(crate) fn workspace_for_window(cx: &App, window_id: WindowId) -> Option<Workspace> {
    with_workspace_for_window(cx, window_id, Clone::clone)
}

pub(crate) fn with_workspace_for_window<R>(
    cx: &App,
    window_id: WindowId,
    read: impl FnOnce(&Workspace) -> R,
) -> Option<R> {
    let manager = manager(cx)?;
    let id = manager.window_workspaces.get(&window_id)?;
    manager
        .workspaces
        .iter()
        .find(|workspace| workspace.id == *id)
        .map(read)
}

pub(crate) fn revision(cx: &App) -> Option<u64> {
    // Initialization must invalidate rows cached before a manager existed.
    manager(cx).map(|manager| manager.revision)
}

/// Shared ordering for Home, Settings, and the workspace picker.
pub(crate) fn sort_workspaces(workspaces: &mut [Workspace]) {
    workspaces.sort_by_key(|workspace| {
        (
            !workspace.restore_on_launch,
            std::cmp::Reverse(workspace.last_activation_order),
        )
    });
}

pub(crate) fn workspaces(cx: &App) -> Vec<Workspace> {
    manager(cx)
        .map(|manager| manager.workspaces.clone())
        .unwrap_or_default()
}

/// Whether a live window currently holds this workspace.
pub(crate) fn is_open_in_a_window(cx: &App, id: WorkspaceId) -> bool {
    manager(cx).is_some_and(|manager| {
        manager
            .window_workspaces
            .values()
            .any(|mapped| *mapped == id)
    })
}

/// The workspace of the most recently focused window, if any.
pub(crate) fn active_workspace_id(cx: &App) -> Option<WorkspaceId> {
    manager(cx)?.active_workspace
}

pub(crate) fn workspace(cx: &App, id: WorkspaceId) -> Option<Workspace> {
    with_workspace(cx, id, Clone::clone)
}

pub(crate) fn with_workspace<R>(
    cx: &App,
    id: WorkspaceId,
    read: impl FnOnce(&Workspace) -> R,
) -> Option<R> {
    manager(cx)?
        .workspaces
        .iter()
        .find(|workspace| workspace.id == id)
        .map(read)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn pr530_focus_changes_persist_only_when_flushed(cx: &mut gpui::TestAppContext) {
        let first = cx.add_window(|_, _| gpui::Empty);
        let second = cx.add_window(|_, _| gpui::Empty);
        cx.update(|cx| initialize_for_test(cx, Vec::new()));
        for window in [first, second] {
            cx.update(|cx| {
                sync_window(
                    cx,
                    window.window_id(),
                    None,
                    vec![PathBuf::from(format!("/repo/{:?}", window.window_id()))],
                    None,
                )
            });
        }
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&writes);
        cx.update(|cx| {
            // Synchronous test sink makes every queued write observable.
            cx.global::<WorkspaceManager>().state.borrow_mut().writer =
                Some(Arc::new(WorkspaceWriter {
                    pending: Mutex::new(None),
                    wake: Condvar::new(),
                    writing: Mutex::new(()),
                    worker_running: AtomicBool::new(false),
                    sink: Box::new(move |workspaces| lock(&recorded).push(workspaces.to_vec())),
                }));
            for window in [first, second, first, second] {
                mark_window_active(cx, window.window_id());
            }
        });
        assert!(
            lock(&writes).is_empty(),
            "focus alone must not rewrite session.json"
        );
        cx.update(flush_to_disk);
        let writes = lock(&writes);
        assert_eq!(writes.len(), 1);
        let latest = writes[0]
            .iter()
            .max_by_key(|w| w.last_activation_order)
            .unwrap();
        assert_eq!(Some(latest.id), cx.update(|cx| active_workspace_id(cx)));
    }

    fn path(value: &str) -> PathBuf {
        PathBuf::from(value)
    }

    #[gpui::test]
    fn customized_workspace_survives_losing_its_last_repository(cx: &mut gpui::TestAppContext) {
        let window = cx.add_window(|_, _| gpui::Empty);
        cx.update(|cx| initialize_for_test(cx, Vec::new()));
        let id = cx.update(|cx| {
            sync_window(cx, window.window_id(), None, vec![path("/repos/a")], None)
                .expect("durable workspace")
        });
        cx.update(|cx| assert!(set_workspace_name(cx, id, "  Client work ")));

        let kept = cx.update(|cx| sync_window(cx, window.window_id(), Some(id), Vec::new(), None));

        assert_eq!(kept, Some(id), "the window keeps its customized workspace");
        let workspace = cx
            .update(|cx| workspace_for_window(cx, window.window_id()))
            .expect("still mapped to the window");
        assert_eq!(workspace.custom_name.as_deref(), Some("Client work"));
        assert!(workspace.repositories.is_empty());
        assert!(workspace.restore_on_launch);
    }

    #[gpui::test]
    fn clearing_the_last_customization_of_an_empty_workspace_removes_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = cx.add_window(|_, _| gpui::Empty);
        cx.update(|cx| initialize_for_test(cx, Vec::new()));
        let id = cx.update(|cx| {
            sync_window(cx, window.window_id(), None, vec![path("/repos/a")], None)
                .expect("durable workspace")
        });
        cx.update(|cx| {
            assert!(set_workspace_theme_mode(cx, id, Some("tokyo_night".into())));
            assert!(
                !set_workspace_theme_mode(cx, id, Some("tokyo_night".into())),
                "an unchanged value reports no change"
            );
        });
        cx.update(|cx| sync_window(cx, window.window_id(), Some(id), Vec::new(), None));
        assert!(cx.update(|cx| workspace(cx, id)).is_some());

        cx.update(|cx| assert!(set_workspace_theme_mode(cx, id, None)));

        assert!(cx.update(|cx| workspace(cx, id)).is_none());
        assert!(
            cx.update(|cx| workspace_for_window(cx, window.window_id()))
                .is_none()
        );
    }

    #[gpui::test]
    fn focusing_a_window_stamps_its_workspace_open_time(cx: &mut gpui::TestAppContext) {
        let window = cx.add_window(|_, _| gpui::Empty);
        cx.update(|cx| initialize_for_test(cx, Vec::new()));
        let id = cx.update(|cx| {
            sync_window(cx, window.window_id(), None, vec![path("/repos/a")], None)
                .expect("durable workspace")
        });
        assert_eq!(
            cx.update(|cx| workspace(cx, id)).unwrap().last_opened_at,
            None
        );

        cx.update(|cx| mark_window_active(cx, window.window_id()));

        assert!(
            cx.update(|cx| workspace(cx, id))
                .unwrap()
                .last_opened_at
                .is_some()
        );
    }

    #[gpui::test]
    fn reads_do_not_notify_global_observers(cx: &mut gpui::TestAppContext) {
        use std::cell::Cell;
        use std::rc::Rc;

        let window = cx.add_window(|_, _| gpui::Empty);
        cx.update(|cx| initialize_for_test(cx, Vec::new()));
        let workspace_id = cx.update(|cx| {
            sync_window(
                cx,
                window.window_id(),
                None,
                vec![path("/repos/a")],
                Some(path("/repos/a")),
            )
            .expect("durable group")
        });
        cx.run_until_parked();

        let notifications = Rc::new(Cell::new(0usize));
        let counter = Rc::clone(&notifications);
        let _subscription = cx.update(|cx| {
            cx.observe_global::<WorkspaceManager>(move |_cx| {
                counter.set(counter.get() + 1);
            })
        });

        cx.update(|cx| {
            for _ in 0..8 {
                let _ = workspaces(cx);
                let _ = workspace(cx, workspace_id);
                let _ = workspace_for_window(cx, window.window_id());
            }
        });
        cx.run_until_parked();
        assert_eq!(notifications.get(), 0, "reads must not lease the global");

        cx.update(|cx| {
            assert!(set_workspace_color(
                cx,
                workspace_id,
                Some(session::WorkspaceColor::Blue)
            ));
        });
        cx.run_until_parked();
        assert_eq!(
            notifications.get(),
            1,
            "a mutation still notifies observers"
        );
    }

    #[gpui::test]
    fn live_windows_keep_independent_repository_workspaces(cx: &mut gpui::TestAppContext) {
        let first = cx.add_window(|_, _| gpui::Empty);
        let second = cx.add_window(|_, _| gpui::Empty);
        cx.update(|cx| initialize_for_test(cx, Vec::new()));

        let first_id = cx.update(|cx| {
            sync_window(
                cx,
                first.window_id(),
                None,
                vec![path("/repos/a"), path("/repos/b")],
                Some(path("/repos/b")),
            )
            .expect("first durable group")
        });
        let second_id = cx.update(|cx| {
            sync_window(
                cx,
                second.window_id(),
                None,
                vec![path("/repos/f")],
                Some(path("/repos/f")),
            )
            .expect("second durable group")
        });

        assert_ne!(first_id, second_id);
        let workspaces = cx.update(|cx| workspaces(cx));
        assert_eq!(workspaces.len(), 2);
        assert_eq!(
            workspaces
                .iter()
                .find(|workspace| workspace.id == first_id)
                .expect("first group")
                .repositories,
            vec![path("/repos/a"), path("/repos/b")]
        );
        assert_eq!(
            workspaces
                .iter()
                .find(|workspace| workspace.id == second_id)
                .expect("second group")
                .repositories,
            vec![path("/repos/f")]
        );
    }

    #[gpui::test]
    fn close_hides_a_workspace_from_launch_but_keeps_it_recoverable(cx: &mut gpui::TestAppContext) {
        let window = cx.add_window(|_, _| gpui::Empty);
        let mut saved = Workspace::new(vec![path("/repos/a")]);
        saved.id = WorkspaceId::from_u128(1);
        let saved_id = saved.id;
        cx.update(|cx| initialize_for_test(cx, vec![saved]));
        let placement = PortableWindowPlacement {
            normal_frame: Some(SavedWindowFrame {
                x: 40,
                y: 60,
                width: 1200,
                height: 800,
            }),
            ..Default::default()
        };

        cx.update(|cx| {
            let id = sync_window(
                cx,
                window.window_id(),
                Some(saved_id),
                vec![path("/repos/a")],
                Some(path("/repos/a")),
            );
            assert_eq!(id, Some(saved_id));
            record_window_placement(cx, window.window_id(), placement.clone());
            mark_window_closed(cx, window.window_id());
        });

        let saved = cx
            .update(|cx| workspace(cx, saved_id))
            .expect("recoverable group");
        assert!(!saved.restore_on_launch);
        assert_eq!(saved.repositories, vec![path("/repos/a")]);
        assert_eq!(saved.placement, placement);
    }

    #[gpui::test]
    fn workspace_color_can_be_set_and_restored_to_the_theme_default(cx: &mut gpui::TestAppContext) {
        let mut saved = Workspace::new(vec![path("/repos/a")]);
        saved.id = WorkspaceId::from_u128(1);
        let saved_id = saved.id;
        cx.update(|cx| initialize_for_test(cx, vec![saved]));

        assert!(cx.update(|cx| {
            set_workspace_color(cx, saved_id, Some(session::WorkspaceColor::Blue))
        }));
        assert_eq!(
            cx.update(|cx| workspace(cx, saved_id).and_then(|workspace| workspace.color)),
            Some(session::WorkspaceColor::Blue)
        );
        assert!(cx.update(|cx| set_workspace_color(cx, saved_id, None)));
        assert_eq!(
            cx.update(|cx| workspace(cx, saved_id).and_then(|workspace| workspace.color)),
            None
        );
        assert!(
            !cx.update(|cx| set_workspace_color(cx, saved_id, None)),
            "selecting the current color should be a no-op"
        );
    }

    #[gpui::test]
    fn empty_ephemeral_window_is_not_saved_and_empty_durable_window_is_deleted(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = cx.add_window(|_, _| gpui::Empty);
        cx.update(|cx| initialize_for_test(cx, Vec::new()));

        assert_eq!(
            cx.update(|cx| sync_window(cx, window.window_id(), None, Vec::new(), None)),
            None
        );
        assert!(cx.update(|cx| workspaces(cx)).is_empty());

        let workspace_id = cx
            .update(|cx| {
                sync_window(
                    cx,
                    window.window_id(),
                    None,
                    vec![path("/repos/a")],
                    Some(path("/repos/a")),
                )
            })
            .expect("durable group after first repository");
        assert!(cx.update(|cx| workspace(cx, workspace_id)).is_some());

        assert_eq!(
            cx.update(|cx| {
                sync_window(cx, window.window_id(), Some(workspace_id), Vec::new(), None)
            }),
            None
        );
        assert!(cx.update(|cx| workspaces(cx)).is_empty());
    }

    #[gpui::test]
    fn review_regression_lifecycle_first_workspace_replays_the_empty_windows_focus(
        cx: &mut gpui::TestAppContext,
    ) {
        let first = cx.add_window(|_, _| gpui::Empty);
        let second = cx.add_window(|_, _| gpui::Empty);
        cx.update(|cx| initialize_for_test(cx, Vec::new()));

        let first_workspace = cx
            .update(|cx| {
                sync_window(
                    cx,
                    first.window_id(),
                    None,
                    vec![path("/repos/first")],
                    Some(path("/repos/first")),
                )
            })
            .expect("first group");
        cx.update(|cx| mark_window_active(cx, first.window_id()));

        // The second window receives focus while it is still ephemeral, then
        // becomes durable only when its first repository is added.
        cx.update(|cx| mark_window_active(cx, second.window_id()));
        let second_workspace = cx
            .update(|cx| {
                sync_window(
                    cx,
                    second.window_id(),
                    None,
                    vec![path("/repos/second")],
                    Some(path("/repos/second")),
                )
            })
            .expect("second group");

        // Clicking back must make the first window newest. If the provisional
        // focus was forgotten, active_workspace still says "first" and suppresses
        // this activation update.
        cx.update(|cx| mark_window_active(cx, first.window_id()));
        let workspaces = cx.update(|cx| workspaces(cx));
        let activation = |id| {
            workspaces
                .iter()
                .find(|workspace| workspace.id == id)
                .expect("saved group")
                .last_activation_order
        };
        assert!(
            activation(first_workspace) > activation(second_workspace),
            "the last clicked window must be restored frontmost"
        );
    }

    #[test]
    fn placement_rebases_relative_position_and_clamps_to_a_smaller_display() {
        let restored = rebase_window_frame(
            SavedWindowFrame {
                x: 960,
                y: 540,
                width: 1600,
                height: 1000,
            },
            Some(SavedWindowFrame {
                x: 0,
                y: 0,
                width: 3840,
                height: 2160,
            }),
            SavedWindowFrame {
                x: 0,
                y: 24,
                width: 1366,
                height: 744,
            },
            800,
            600,
        );

        assert_eq!(restored.width, 1366);
        assert_eq!(restored.height, 744);
        assert_eq!(restored.x, 0);
        assert_eq!(restored.y, 24);
    }

    #[test]
    fn placement_without_old_display_metadata_is_clamped_on_screen() {
        let restored = rebase_window_frame(
            SavedWindowFrame {
                x: -9000,
                y: 9000,
                width: 900,
                height: 700,
            },
            None,
            SavedWindowFrame {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1040,
            },
            800,
            600,
        );

        assert_eq!(restored.x, -1920);
        assert_eq!(restored.y, 340);
        assert_eq!(restored.width, 900);
        assert_eq!(restored.height, 700);
    }

    /// A writer whose sink records each write and holds the first one until
    /// `release` is sent, so tests can queue behind an in-progress write.
    fn gated_writer() -> (
        Arc<WorkspaceWriter>,
        std::sync::mpsc::Sender<()>,
        std::sync::mpsc::Receiver<(std::thread::ThreadId, Vec<PathBuf>)>,
    ) {
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = Mutex::new(Some(release_rx));
        let (written_tx, written_rx) = std::sync::mpsc::channel();
        let written_tx = Mutex::new(written_tx);
        let writer = WorkspaceWriter::spawn(Box::new(move |workspaces: &[Workspace]| {
            if let Some(release) = lock(&release_rx).take() {
                let _ = release.recv();
            }
            let repositories = workspaces
                .iter()
                .flat_map(|workspace| workspace.repositories.clone())
                .collect();
            let _ = lock(&written_tx).send((std::thread::current().id(), repositories));
        }));
        (writer, release_tx, written_rx)
    }

    fn snapshot(repo: &str) -> Vec<Workspace> {
        vec![Workspace::new(vec![path(repo)])]
    }

    /// Session writes fsync, so they run off the UI thread; while one is in
    /// progress, later snapshots collapse into the newest.
    #[test]
    fn workspace_writes_leave_the_calling_thread_and_keep_the_newest() {
        let (writer, release, written) = gated_writer();
        let timeout = std::time::Duration::from_secs(5);

        writer.enqueue(snapshot("/repos/a"));
        // Let the worker take `a` and block in the sink before queueing more.
        std::thread::sleep(std::time::Duration::from_millis(50));
        writer.enqueue(snapshot("/repos/b"));
        writer.enqueue(snapshot("/repos/c"));
        release.send(()).unwrap();

        let first = written.recv_timeout(timeout).expect("first write");
        let second = written.recv_timeout(timeout).expect("second write");
        assert_eq!(first.1, vec![path("/repos/a")]);
        assert_eq!(second.1, vec![path("/repos/c")], "`b` was superseded");
        assert_ne!(first.0, std::thread::current().id());
        assert!(
            written
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err()
        );
    }

    /// The quit flush writes synchronously and is the last write: a snapshot
    /// queued before it may land first, never after.
    #[test]
    fn quit_flush_is_the_last_workspace_write() {
        let (writer, release, written) = gated_writer();

        writer.enqueue(snapshot("/repos/a"));
        std::thread::sleep(std::time::Duration::from_millis(50));
        writer.enqueue(snapshot("/repos/stale"));
        let flusher = {
            let writer = Arc::clone(&writer);
            std::thread::spawn(move || writer.write_now(&snapshot("/repos/final")))
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        release.send(()).unwrap();
        flusher.join().unwrap();

        let writes: Vec<_> = std::iter::from_fn(|| {
            written
                .recv_timeout(std::time::Duration::from_millis(200))
                .ok()
        })
        .map(|(_, repositories)| repositories)
        .collect();
        assert_eq!(writes.first(), Some(&vec![path("/repos/a")]));
        assert_eq!(
            writes.last(),
            Some(&vec![path("/repos/final")]),
            "nothing may land after the quit flush: {writes:?}"
        );
    }

    /// Main-thread cost of one focus change: the old synchronous session
    /// write versus queueing it. Run with `--ignored --nocapture`.
    #[test]
    #[ignore = "timing probe"]
    fn timing_workspace_persist_on_the_calling_thread() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("tempdir on disk");
        let session_path = dir.path().join("session.json");
        let workspaces: Vec<Workspace> = (0..6)
            .map(|index| {
                Workspace::new(
                    (0..4)
                        .map(|repo| path(&format!("/home/user/src/project-{index}/repo-{repo}")))
                        .collect(),
                )
            })
            .collect();
        let median = |mut samples: Vec<std::time::Duration>| {
            samples.sort();
            samples[samples.len() / 2]
        };

        let sync: Vec<_> = (0..40)
            .map(|_| {
                let started = std::time::Instant::now();
                session::persist_workspaces_to_path(&workspaces, &session_path).unwrap();
                started.elapsed()
            })
            .collect();
        let target = session_path.clone();
        let writer = WorkspaceWriter::spawn(Box::new(move |workspaces: &[Workspace]| {
            let _ = session::persist_workspaces_to_path(workspaces, &target);
        }));
        let queued: Vec<_> = (0..40)
            .map(|_| {
                let started = std::time::Instant::now();
                writer.enqueue(workspaces.clone());
                let elapsed = started.elapsed();
                std::thread::sleep(std::time::Duration::from_millis(5));
                elapsed
            })
            .collect();
        println!(
            "timing workspace_persist sync_median={:?} queued_median={:?}",
            median(sync),
            median(queued)
        );
    }
}
