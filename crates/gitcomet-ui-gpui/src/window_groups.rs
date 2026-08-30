use gitcomet_state::session::{
    self, PortableWindowPlacement, SavedWindowFrame, SavedWindowGroup, WindowGroupId,
    WindowGroupLayout,
};
use gpui::{App, BorrowAppContext, WindowId};
use rustc_hash::FxHashMap;
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct WindowGroupManager {
    enabled: bool,
    persist_to_disk: bool,
    groups: Vec<SavedWindowGroup>,
    window_groups: FxHashMap<WindowId, WindowGroupId>,
    focused_window: Option<WindowId>,
    active_group: Option<WindowGroupId>,
    next_activation_order: u64,
}

impl gpui::Global for WindowGroupManager {}

impl WindowGroupManager {
    fn enabled(groups: Vec<SavedWindowGroup>, persist_to_disk: bool) -> Self {
        let next_activation_order = groups
            .iter()
            .map(|group| group.last_activation_order)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Self {
            enabled: true,
            persist_to_disk,
            groups,
            window_groups: FxHashMap::default(),
            focused_window: None,
            active_group: None,
            next_activation_order,
        }
    }

    fn group_index(&self, id: WindowGroupId) -> Option<usize> {
        self.groups.iter().position(|group| group.id == id)
    }

    fn allocate_activation_order(&mut self) -> u64 {
        let order = self.next_activation_order;
        self.next_activation_order = self.next_activation_order.saturating_add(1);
        order
    }
}

pub(crate) fn initialize(cx: &mut App, groups: Vec<SavedWindowGroup>) {
    cx.set_global(WindowGroupManager::enabled(groups, true));
}

#[cfg(test)]
pub(crate) fn initialize_for_test(cx: &mut App, groups: Vec<SavedWindowGroup>) {
    cx.set_global(WindowGroupManager::enabled(groups, false));
}

fn persist_if_changed(groups: Option<Vec<SavedWindowGroup>>) {
    let Some(groups) = groups else {
        return;
    };
    if let Err(error) = session::persist_window_groups(&groups) {
        eprintln!("Failed to persist window groups: {error}");
    }
}

pub(crate) fn persist_current<C>(cx: &mut C)
where
    C: BorrowAppContext,
{
    let groups = cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
        (manager.enabled && manager.persist_to_disk).then(|| manager.groups.clone())
    });
    persist_if_changed(groups);
}

/// Synchronize durable group membership with one normal window and return the
/// group identity the view should retain. Empty ephemeral windows remain
/// identity-less; an empty durable window deletes its group.
pub(crate) fn sync_window<C>(
    cx: &mut C,
    window_id: WindowId,
    requested_group_id: Option<WindowGroupId>,
    repositories: Vec<PathBuf>,
    active_repository: Option<PathBuf>,
) -> Option<WindowGroupId>
where
    C: BorrowAppContext,
{
    let (group_id, changed_groups) =
        cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
            if !manager.enabled {
                return (requested_group_id, None);
            }

            if repositories.is_empty() {
                let group_id =
                    requested_group_id.or_else(|| manager.window_groups.get(&window_id).copied());
                manager.window_groups.remove(&window_id);
                if manager.focused_window == Some(window_id) {
                    manager.active_group = None;
                }
                let Some(group_id) = group_id else {
                    return (None, None);
                };
                let before = manager.groups.len();
                manager.groups.retain(|group| group.id != group_id);
                if manager.active_group == Some(group_id) {
                    manager.active_group = None;
                }
                let changed = (manager.groups.len() != before && manager.persist_to_disk)
                    .then(|| manager.groups.clone());
                return (None, changed);
            }

            let group_id = requested_group_id
                .or_else(|| manager.window_groups.get(&window_id).copied())
                .unwrap_or_default();
            manager.window_groups.insert(window_id, group_id);
            let active_repository = active_repository
                .filter(|active| repositories.contains(active))
                .or_else(|| repositories.first().cloned());

            let mut changed = false;
            if let Some(index) = manager.group_index(group_id) {
                let group = &mut manager.groups[index];
                if group.repositories != repositories {
                    group.repositories.clone_from(&repositories);
                    changed = true;
                }
                if group.active_repository != active_repository {
                    group.active_repository.clone_from(&active_repository);
                    changed = true;
                }
                if !group.restore_on_launch {
                    group.restore_on_launch = true;
                    changed = true;
                }
            } else {
                let mut group = SavedWindowGroup::new(repositories);
                group.id = group_id;
                group.active_repository = active_repository;
                group.last_activation_order = manager.allocate_activation_order();
                manager.groups.push(group);
                changed = true;
            }

            // Focus can arrive while a new window is still empty and therefore
            // has no group mapping. Replay it when the first repository makes
            // the window durable so the previously focused group is no longer
            // mistaken for the frontmost one.
            if manager.focused_window == Some(window_id) && manager.active_group != Some(group_id) {
                manager.active_group = Some(group_id);
                let order = manager.allocate_activation_order();
                if let Some(index) = manager.group_index(group_id) {
                    manager.groups[index].last_activation_order = order;
                    changed = true;
                }
            }

            (
                Some(group_id),
                (changed && manager.persist_to_disk).then(|| manager.groups.clone()),
            )
        });
    persist_if_changed(changed_groups);
    group_id
}

pub(crate) fn mark_window_active<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowAppContext,
{
    let changed_groups = cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
        }
        manager.focused_window = Some(window_id);
        let Some(group_id) = manager.window_groups.get(&window_id).copied() else {
            manager.active_group = None;
            return None;
        };
        if manager.active_group == Some(group_id) {
            return None;
        }
        manager.active_group = Some(group_id);
        let order = manager.allocate_activation_order();
        let index = manager.group_index(group_id)?;
        manager.groups[index].last_activation_order = order;
        manager.persist_to_disk.then(|| manager.groups.clone())
    });
    persist_if_changed(changed_groups);
}

pub(crate) fn mark_window_closed<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowAppContext,
{
    let changed_groups = cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
        }
        if manager.focused_window == Some(window_id) {
            manager.focused_window = None;
        }
        let group_id = manager.window_groups.remove(&window_id)?;
        if manager.active_group == Some(group_id) {
            manager.active_group = None;
        }
        let index = manager.group_index(group_id)?;
        if !manager.groups[index].restore_on_launch {
            return None;
        }
        manager.groups[index].restore_on_launch = false;
        manager.persist_to_disk.then(|| manager.groups.clone())
    });
    persist_if_changed(changed_groups);
}

/// Remove a live window and its durable group entirely. This is used when a
/// repository move empties the source window: unlike an explicit user close,
/// there is no group left to recover from the picker.
pub(crate) fn discard_window_group<C>(cx: &mut C, window_id: WindowId)
where
    C: BorrowAppContext,
{
    let changed_groups = cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
        }
        if manager.focused_window == Some(window_id) {
            manager.focused_window = None;
        }
        let group_id = manager.window_groups.remove(&window_id)?;
        if manager.active_group == Some(group_id) {
            manager.active_group = None;
        }
        let before = manager.groups.len();
        manager.groups.retain(|group| group.id != group_id);
        (manager.persist_to_disk && manager.groups.len() != before).then(|| manager.groups.clone())
    });
    persist_if_changed(changed_groups);
}

/// Remove a closed/stale durable group by identity. Recovery uses this when
/// every repository saved in the group already belongs to a live window.
pub(crate) fn discard_group<C>(cx: &mut C, group_id: WindowGroupId)
where
    C: BorrowAppContext,
{
    let changed_groups = cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
        }
        manager
            .window_groups
            .retain(|_, mapped_group_id| *mapped_group_id != group_id);
        if manager.active_group == Some(group_id) {
            manager.active_group = None;
        }
        let before = manager.groups.len();
        manager.groups.retain(|group| group.id != group_id);
        (manager.persist_to_disk && manager.groups.len() != before).then(|| manager.groups.clone())
    });
    persist_if_changed(changed_groups);
}

/// Change the visual identity of one durable window group. This applies to
/// both live and recoverable groups; the caller is responsible for repainting
/// any live window that currently owns the group.
pub(crate) fn set_group_color<C>(
    cx: &mut C,
    group_id: WindowGroupId,
    color: Option<session::WindowGroupColor>,
) -> bool
where
    C: BorrowAppContext,
{
    let (changed, changed_groups) =
        cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
            if !manager.enabled {
                return (false, None);
            }
            let Some(index) = manager.group_index(group_id) else {
                return (false, None);
            };
            if manager.groups[index].color == color {
                return (false, None);
            }
            manager.groups[index].color = color;
            (
                true,
                manager.persist_to_disk.then(|| manager.groups.clone()),
            )
        });
    persist_if_changed(changed_groups);
    changed
}

pub(crate) fn update_window_environment<C>(
    cx: &mut C,
    window_id: WindowId,
    layout: WindowGroupLayout,
    placement: Option<PortableWindowPlacement>,
) where
    C: BorrowAppContext,
{
    let changed_groups = cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
        if !manager.enabled {
            return None;
        }
        let group_id = manager.window_groups.get(&window_id).copied()?;
        let index = manager.group_index(group_id)?;
        let group = &mut manager.groups[index];
        let mut changed = false;
        if group.layout != layout {
            group.layout = layout;
            changed = true;
        }
        if let Some(placement) = placement {
            if group.placement != placement {
                group.placement = placement;
            }
            // Bounds are recorded in memory immediately so close/quit cannot
            // lose them. This debounced call is the point at which they are
            // also flushed during an otherwise idle session.
            changed = true;
        }
        (changed && manager.persist_to_disk).then(|| manager.groups.clone())
    });
    persist_if_changed(changed_groups);
}

pub(crate) fn record_window_placement<C>(
    cx: &mut C,
    window_id: WindowId,
    placement: PortableWindowPlacement,
) where
    C: BorrowAppContext,
{
    cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
        if !manager.enabled {
            return;
        }
        let Some(group_id) = manager.window_groups.get(&window_id).copied() else {
            return;
        };
        let Some(index) = manager.group_index(group_id) else {
            return;
        };
        manager.groups[index].placement = placement;
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

pub(crate) fn group_for_window<C>(cx: &mut C, window_id: WindowId) -> Option<SavedWindowGroup>
where
    C: BorrowAppContext,
{
    cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
        let id = manager.window_groups.get(&window_id)?;
        manager.groups.iter().find(|group| group.id == *id).cloned()
    })
}

pub(crate) fn groups<C>(cx: &mut C) -> Vec<SavedWindowGroup>
where
    C: BorrowAppContext,
{
    cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| manager.groups.clone())
}

pub(crate) fn group<C>(cx: &mut C, id: WindowGroupId) -> Option<SavedWindowGroup>
where
    C: BorrowAppContext,
{
    cx.update_default_global::<WindowGroupManager, _>(|manager, _cx| {
        manager.groups.iter().find(|group| group.id == id).cloned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(value: &str) -> PathBuf {
        PathBuf::from(value)
    }

    #[gpui::test]
    fn live_windows_keep_independent_repository_groups(cx: &mut gpui::TestAppContext) {
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
        let groups = cx.update(groups);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups
                .iter()
                .find(|group| group.id == first_id)
                .expect("first group")
                .repositories,
            vec![path("/repos/a"), path("/repos/b")]
        );
        assert_eq!(
            groups
                .iter()
                .find(|group| group.id == second_id)
                .expect("second group")
                .repositories,
            vec![path("/repos/f")]
        );
    }

    #[gpui::test]
    fn close_hides_a_group_from_launch_but_keeps_it_recoverable(cx: &mut gpui::TestAppContext) {
        let window = cx.add_window(|_, _| gpui::Empty);
        let mut saved = SavedWindowGroup::new(vec![path("/repos/a")]);
        saved.id = WindowGroupId::from_u128(1);
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
            .update(|cx| group(cx, saved_id))
            .expect("recoverable group");
        assert!(!saved.restore_on_launch);
        assert_eq!(saved.repositories, vec![path("/repos/a")]);
        assert_eq!(saved.placement, placement);
    }

    #[gpui::test]
    fn window_group_color_can_be_set_and_restored_to_the_theme_default(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut saved = SavedWindowGroup::new(vec![path("/repos/a")]);
        saved.id = WindowGroupId::from_u128(1);
        let saved_id = saved.id;
        cx.update(|cx| initialize_for_test(cx, vec![saved]));

        assert!(
            cx.update(|cx| {
                set_group_color(cx, saved_id, Some(session::WindowGroupColor::Blue))
            })
        );
        assert_eq!(
            cx.update(|cx| group(cx, saved_id).and_then(|group| group.color)),
            Some(session::WindowGroupColor::Blue)
        );
        assert!(cx.update(|cx| set_group_color(cx, saved_id, None)));
        assert_eq!(
            cx.update(|cx| group(cx, saved_id).and_then(|group| group.color)),
            None
        );
        assert!(
            !cx.update(|cx| set_group_color(cx, saved_id, None)),
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
        assert!(cx.update(groups).is_empty());

        let group_id = cx
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
        assert!(cx.update(|cx| group(cx, group_id)).is_some());

        assert_eq!(
            cx.update(|cx| {
                sync_window(cx, window.window_id(), Some(group_id), Vec::new(), None)
            }),
            None
        );
        assert!(cx.update(groups).is_empty());
    }

    #[gpui::test]
    fn review_regression_lifecycle_first_group_replays_the_empty_windows_focus(
        cx: &mut gpui::TestAppContext,
    ) {
        let first = cx.add_window(|_, _| gpui::Empty);
        let second = cx.add_window(|_, _| gpui::Empty);
        cx.update(|cx| initialize_for_test(cx, Vec::new()));

        let first_group = cx
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
        let second_group = cx
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
        // focus was forgotten, active_group still says "first" and suppresses
        // this activation update.
        cx.update(|cx| mark_window_active(cx, first.window_id()));
        let groups = cx.update(groups);
        let activation = |id| {
            groups
                .iter()
                .find(|group| group.id == id)
                .expect("saved group")
                .last_activation_order
        };
        assert!(
            activation(first_group) > activation(second_group),
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
