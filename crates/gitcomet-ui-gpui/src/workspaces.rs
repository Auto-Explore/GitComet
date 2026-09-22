use gitcomet_state::session::{
    self, PortableWindowPlacement, SavedWindowFrame, Workspace, WorkspaceId, WorkspaceLayout,
};
use gpui::{App, BorrowAppContext, WindowId};
use rustc_hash::FxHashMap;
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct WorkspaceManager {
    enabled: bool,
    persist_to_disk: bool,
    workspaces: Vec<Workspace>,
    window_workspaces: FxHashMap<WindowId, WorkspaceId>,
    focused_window: Option<WindowId>,
    active_workspace: Option<WorkspaceId>,
    next_activation_order: u64,
}

impl gpui::Global for WorkspaceManager {}

impl WorkspaceManager {
    fn enabled(workspaces: Vec<Workspace>, persist_to_disk: bool) -> Self {
        let next_activation_order = workspaces
            .iter()
            .map(|workspace| workspace.last_activation_order)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Self {
            enabled: true,
            persist_to_disk,
            workspaces,
            window_workspaces: FxHashMap::default(),
            focused_window: None,
            active_workspace: None,
            next_activation_order,
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
}

pub(crate) fn initialize(cx: &mut App, workspaces: Vec<Workspace>) {
    cx.set_global(WorkspaceManager::enabled(workspaces, true));
}

#[cfg(test)]
pub(crate) fn initialize_for_test(cx: &mut App, workspaces: Vec<Workspace>) {
    cx.set_global(WorkspaceManager::enabled(workspaces, false));
}

fn persist_if_changed(workspaces: Option<Vec<Workspace>>) {
    let Some(workspaces) = workspaces else {
        return;
    };
    if let Err(error) = session::persist_workspaces(&workspaces) {
        eprintln!("Failed to persist workspaces: {error}");
    }
}

pub(crate) fn persist_current<C>(cx: &mut C)
where
    C: BorrowAppContext,
{
    let workspaces = cx.update_default_global::<WorkspaceManager, _>(|manager, _cx| {
        (manager.enabled && manager.persist_to_disk).then(|| manager.workspaces.clone())
    });
    persist_if_changed(workspaces);
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
    C: BorrowAppContext,
{
    let (workspace_id, changed_workspaces) =
        cx.update_default_global::<WorkspaceManager, _>(|manager, _cx| {
            if !manager.enabled {
                return (requested_workspace_id, None);
            }

            if repositories.is_empty() {
                let workspace_id = requested_workspace_id
                    .or_else(|| manager.window_workspaces.get(&window_id).copied());
                if let Some(workspace_id) = workspace_id
                    && let Some(index) = manager.workspace_index(workspace_id)
                    && manager.workspaces[index].is_customized()
                {
                    manager.window_workspaces.insert(window_id, workspace_id);
                    let workspace = &mut manager.workspaces[index];
                    let mut changed = !workspace.repositories.is_empty()
                        || workspace.active_repository.is_some()
                        || !workspace.restore_on_launch;
                    workspace.repositories.clear();
                    workspace.active_repository = None;
                    workspace.restore_on_launch = true;
                    changed |= manager.replay_focus(window_id, workspace_id);
                    return (
                        Some(workspace_id),
                        (changed && manager.persist_to_disk).then(|| manager.workspaces.clone()),
                    );
                }
                manager.window_workspaces.remove(&window_id);
                if manager.focused_window == Some(window_id) {
                    manager.active_workspace = None;
                }
                let Some(workspace_id) = workspace_id else {
                    return (None, None);
                };
                let before = manager.workspaces.len();
                manager
                    .workspaces
                    .retain(|workspace| workspace.id != workspace_id);
                if manager.active_workspace == Some(workspace_id) {
                    manager.active_workspace = None;
                }
                let changed = (manager.workspaces.len() != before && manager.persist_to_disk)
                    .then(|| manager.workspaces.clone());
                return (None, changed);
            }

            let workspace_id = requested_workspace_id
                .or_else(|| manager.window_workspaces.get(&window_id).copied())
                .unwrap_or_default();
            manager.window_workspaces.insert(window_id, workspace_id);
            let active_repository = active_repository
                .filter(|active| repositories.contains(active))
                .or_else(|| repositories.first().cloned());

            let mut changed = false;
            if let Some(index) = manager.workspace_index(workspace_id) {
                let workspace = &mut manager.workspaces[index];
                if workspace.repositories != repositories {
                    workspace.repositories.clone_from(&repositories);
                    changed = true;
                }
                if workspace.active_repository != active_repository {
                    workspace.active_repository.clone_from(&active_repository);
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

            (
                Some(workspace_id),
                (changed && manager.persist_to_disk).then(|| manager.workspaces.clone()),
            )
        });
    persist_if_changed(changed_workspaces);
    workspace_id
}

pub(crate) fn mark_window_active<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowAppContext,
{
    let changed_workspaces = cx.update_default_global::<WorkspaceManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
        }
        manager.focused_window = Some(window_id);
        let Some(workspace_id) = manager.window_workspaces.get(&window_id).copied() else {
            manager.active_workspace = None;
            return None;
        };
        if manager.active_workspace == Some(workspace_id) {
            return None;
        }
        manager.activate(workspace_id).then_some(())?;
        manager.persist_to_disk.then(|| manager.workspaces.clone())
    });
    persist_if_changed(changed_workspaces);
}

pub(crate) fn mark_window_closed<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowAppContext,
{
    let changed_workspaces = cx.update_default_global::<WorkspaceManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
        }
        if manager.focused_window == Some(window_id) {
            manager.focused_window = None;
        }
        let workspace_id = manager.window_workspaces.remove(&window_id)?;
        if manager.active_workspace == Some(workspace_id) {
            manager.active_workspace = None;
        }
        let index = manager.workspace_index(workspace_id)?;
        if !manager.workspaces[index].restore_on_launch {
            return None;
        }
        manager.workspaces[index].restore_on_launch = false;
        manager.persist_to_disk.then(|| manager.workspaces.clone())
    });
    persist_if_changed(changed_workspaces);
}

/// Unmap a window from its workspace and mark that workspace closed, keeping
/// focus bookkeeping. Used when a Home window adopts a different workspace.
pub(crate) fn release_window_workspace<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowAppContext,
{
    let changed_workspaces = cx.update_default_global::<WorkspaceManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
        }
        let workspace_id = manager.window_workspaces.remove(&window_id)?;
        if manager.active_workspace == Some(workspace_id) {
            manager.active_workspace = None;
        }
        let index = manager.workspace_index(workspace_id)?;
        let workspace = &mut manager.workspaces[index];
        if !workspace.restore_on_launch {
            return None;
        }
        workspace.restore_on_launch = false;
        manager.persist_to_disk.then(|| manager.workspaces.clone())
    });
    persist_if_changed(changed_workspaces);
}

/// Remove a live window and its durable workspace entirely. This is used when a
/// repository move empties the source window of an anonymous workspace: unlike
/// an explicit user close, there is nothing left to recover. Callers keep a
/// customized workspace (and its window) instead.
pub(crate) fn discard_workspace_for_window<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowAppContext,
{
    let changed_workspaces = cx.update_default_global::<WorkspaceManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
        }
        if manager.focused_window == Some(window_id) {
            manager.focused_window = None;
        }
        let workspace_id = manager.window_workspaces.remove(&window_id)?;
        if manager.active_workspace == Some(workspace_id) {
            manager.active_workspace = None;
        }
        let before = manager.workspaces.len();
        manager
            .workspaces
            .retain(|workspace| workspace.id != workspace_id);
        (manager.persist_to_disk && manager.workspaces.len() != before)
            .then(|| manager.workspaces.clone())
    });
    persist_if_changed(changed_workspaces);
}

/// Remove a closed/stale durable workspace by identity. Recovery uses this when
/// every repository saved in the workspace already belongs to a live window.
pub(crate) fn discard_workspace<C>(cx: &mut C, workspace_id: WorkspaceId)
where
    C: BorrowAppContext,
{
    let changed_workspaces = cx.update_default_global::<WorkspaceManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
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
        (manager.persist_to_disk && manager.workspaces.len() != before)
            .then(|| manager.workspaces.clone())
    });
    persist_if_changed(changed_workspaces);
}

/// Apply `edit` to one workspace, live or recoverable, and persist on change.
/// A workspace left empty and uncustomized is removed, matching what
/// `sync_window` would do. Callers repaint any live window that owns it.
fn update_workspace<C>(
    cx: &mut C,
    workspace_id: WorkspaceId,
    edit: impl FnOnce(&mut Workspace) -> bool,
) -> bool
where
    C: BorrowAppContext,
{
    let (changed, changed_workspaces) =
        cx.update_default_global::<WorkspaceManager, _>(|manager, _cx| {
            if !manager.enabled {
                return (false, None);
            }
            let Some(index) = manager.workspace_index(workspace_id) else {
                return (false, None);
            };
            if !edit(&mut manager.workspaces[index]) {
                return (false, None);
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
            (
                true,
                manager.persist_to_disk.then(|| manager.workspaces.clone()),
            )
        });
    persist_if_changed(changed_workspaces);
    changed
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
    C: BorrowAppContext,
{
    update_workspace(cx, workspace_id, |workspace| {
        replace_if_changed(&mut workspace.color, color)
    })
}

/// A blank name clears it, falling back to the automatic name.
pub(crate) fn set_workspace_name<C>(cx: &mut C, workspace_id: WorkspaceId, name: &str) -> bool
where
    C: BorrowAppContext,
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
    C: BorrowAppContext,
{
    update_workspace(cx, workspace_id, |workspace| {
        replace_if_changed(&mut workspace.theme_mode, theme_mode)
    })
}

pub(crate) fn update_window_environment<C>(
    cx: &mut C,
    window_id: WindowId,
    layout: WorkspaceLayout,
    placement: Option<PortableWindowPlacement>,
) where
    C: BorrowAppContext,
{
    let changed_workspaces = cx.update_default_global::<WorkspaceManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
        }
        let workspace_id = manager.window_workspaces.get(&window_id).copied()?;
        let index = manager.workspace_index(workspace_id)?;
        let workspace = &mut manager.workspaces[index];
        let mut changed = false;
        if workspace.layout != layout {
            workspace.layout = layout;
            changed = true;
        }
        if let Some(placement) = placement {
            if workspace.placement != placement {
                workspace.placement = placement;
            }
            // Bounds are recorded in memory immediately so close/quit cannot
            // lose them. This debounced call is the point at which they are
            // also flushed during an otherwise idle session.
            changed = true;
        }
        (changed && manager.persist_to_disk).then(|| manager.workspaces.clone())
    });
    persist_if_changed(changed_workspaces);
}

pub(crate) fn record_window_placement<C>(
    cx: &mut C,
    window_id: WindowId,
    placement: PortableWindowPlacement,
) where
    C: BorrowAppContext,
{
    cx.update_default_global::<WorkspaceManager, _>(|manager, _cx| {
        if !manager.enabled {
            return;
        }
        let Some(workspace_id) = manager.window_workspaces.get(&window_id).copied() else {
            return;
        };
        let Some(index) = manager.workspace_index(workspace_id) else {
            return;
        };
        manager.workspaces[index].placement = placement;
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
pub(crate) const WORKSPACE_COLORS: [(Option<session::WorkspaceColor>, &str); 9] = [
    (None, "Default"),
    (Some(session::WorkspaceColor::Gray), "Gray"),
    (Some(session::WorkspaceColor::Red), "Red"),
    (Some(session::WorkspaceColor::Orange), "Orange"),
    (Some(session::WorkspaceColor::Yellow), "Yellow"),
    (Some(session::WorkspaceColor::Green), "Green"),
    (Some(session::WorkspaceColor::Blue), "Blue"),
    (Some(session::WorkspaceColor::Purple), "Purple"),
    (Some(session::WorkspaceColor::Pink), "Pink"),
];

pub(crate) fn repository_count_label(count: usize) -> String {
    match count {
        0 => "No repositories".to_string(),
        1 => "1 repository".to_string(),
        n => format!("{n} repositories"),
    }
}

/// Read-only view of the manager. Reads must not go through
/// `update_default_global`: gpui notifies every global observer on each lease,
/// and the title bar reads the manager per frame, so a leasing read plus an
/// observer is a repaint loop.
fn manager(cx: &App) -> Option<&WorkspaceManager> {
    cx.try_global::<WorkspaceManager>()
}

pub(crate) fn workspace_for_window(cx: &App, window_id: WindowId) -> Option<Workspace> {
    let manager = manager(cx)?;
    let id = manager.window_workspaces.get(&window_id)?;
    manager
        .workspaces
        .iter()
        .find(|workspace| workspace.id == *id)
        .cloned()
}

pub(crate) fn workspaces(cx: &App) -> Vec<Workspace> {
    manager(cx)
        .map(|manager| manager.workspaces.clone())
        .unwrap_or_default()
}

/// The workspace of the most recently focused window, if any.
pub(crate) fn active_workspace_id(cx: &App) -> Option<WorkspaceId> {
    manager(cx)?.active_workspace
}

pub(crate) fn workspace(cx: &App, id: WorkspaceId) -> Option<Workspace> {
    manager(cx)?
        .workspaces
        .iter()
        .find(|workspace| workspace.id == id)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
