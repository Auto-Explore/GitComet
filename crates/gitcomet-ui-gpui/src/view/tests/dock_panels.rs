//! The window's docks: panes that survive a frame they are not drawn in,
//! popped-out panels coming back, and the branch list's data requests
//! across panes.

use super::*;
use crate::view::dock::{DockCommand, DockHalf, DockSide, PanelId, panel_ids};

fn open_repo_window(
    cx: &mut gpui::TestAppContext,
) -> (
    AppStore,
    gpui::Entity<GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.git_runtime = available_git_runtime_state();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    cx.simulate_resize(gpui::size(px(1600.0), px(900.0)));
    test_support::redraw(cx);
    (store, view, cx)
}

fn run(view: &gpui::Entity<GitCometView>, cx: &mut gpui::VisualTestContext, command: DockCommand) {
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.run_dock_command(command, Some(window), cx)
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
}

fn panel(id: &str) -> PanelId {
    PanelId::from(id)
}

fn dock_pane_ids(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Vec<gpui::EntityId> {
    cx.update(|_window, app| {
        view.read(app)
            .dock_hosts
            .sidebar_panes()
            .map(|pane| pane.entity_id())
            .collect()
    })
}

/// Ctrl+W removes a window without asking whether it may close, so the
/// panel has to come back when the window's view goes, not on the OS close.
#[gpui::test]
fn a_popped_out_panel_comes_back_when_its_window_is_removed(cx: &mut gpui::TestAppContext) {
    let (_store, view, cx) = open_repo_window(cx);
    let branches = panel(panel_ids::BRANCHES);
    let windows = |cx: &mut gpui::VisualTestContext| cx.cx.update(|app| app.windows());
    let popped_out = |view: &gpui::Entity<GitCometView>, cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.read(app)
                .is_dock_panel_popped_out(&PanelId::from(panel_ids::BRANCHES))
        })
    };
    assert!(cx.debug_bounds("sidebar_tab_branches").is_some());
    let before = windows(cx);

    run(&view, cx, DockCommand::PopOut(branches.clone()));
    let after = windows(cx);
    assert_eq!(after.len(), before.len() + 1);
    let popped = *after
        .iter()
        .find(|window| !before.contains(window))
        .unwrap();
    assert!(popped_out(&view, cx));
    assert!(
        cx.debug_bounds("sidebar_tab_branches").is_none(),
        "Branches left the main window"
    );

    cx.cx.update(|app| {
        popped
            .update(app, |_, window, _| window.remove_window())
            .unwrap()
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert_eq!(windows(cx).len(), before.len());
    assert!(!popped_out(&view, cx), "the panel is docked again");
    assert!(
        cx.debug_bounds("sidebar_tab_branches").is_some(),
        "Branches draws in the main window again"
    );

    // And it can go out again.
    run(&view, cx, DockCommand::PopOut(branches));
    assert_eq!(windows(cx).len(), before.len() + 1);
    assert!(popped_out(&view, cx));
}

/// A tab strip draws only its shown panel. The pane of a hidden tab keeps
/// its scroll, filter and selection, so it must not be made anew.
#[gpui::test]
fn a_moved_panels_pane_survives_switching_tabs(cx: &mut gpui::TestAppContext) {
    let (_store, view, cx) = open_repo_window(cx);
    assert!(dock_pane_ids(&view, cx).is_empty(), "the factory layout");
    run(
        &view,
        cx,
        DockCommand::MovePanel {
            panel: panel(panel_ids::FILES),
            side: DockSide::Right,
            half: DockHalf::Primary,
        },
    );
    let panes = dock_pane_ids(&view, cx);
    assert_eq!(panes.len(), 1, "Files got a pane of its own");

    run(&view, cx, DockCommand::ShowPanel(panel(panel_ids::CHANGES)));
    assert_eq!(dock_pane_ids(&view, cx), panes);
    run(&view, cx, DockCommand::ShowPanel(panel(panel_ids::FILES)));
    assert_eq!(dock_pane_ids(&view, cx), panes, "the same pane shows again");

    // Putting Files back where it was drops the pane.
    run(
        &view,
        cx,
        DockCommand::RestorePanel(panel(panel_ids::FILES)),
    );
    assert!(dock_pane_ids(&view, cx).is_empty());
}

/// The store keeps one data request per repository and reloads only what it
/// asks for. A pane showing only Files must not replace the branch list's.
#[gpui::test]
fn moving_files_keeps_the_branch_lists_data_requested(cx: &mut gpui::TestAppContext) {
    let (store, view, cx) = open_repo_window(cx);
    test_support::drain_store_worker(&view, cx);
    let request = store.snapshot().repos[0].sidebar_data_request;
    assert!(
        request.stashes && request.submodules,
        "the open sidebar asks for every section: {request:?}"
    );

    run(
        &view,
        cx,
        DockCommand::MovePanel {
            panel: panel(panel_ids::FILES),
            side: DockSide::Right,
            half: DockHalf::Primary,
        },
    );
    assert_eq!(dock_pane_ids(&view, cx).len(), 1);
    test_support::drain_store_worker(&view, cx);
    let request = store.snapshot().repos[0].sidebar_data_request;
    assert!(
        request.stashes && request.submodules,
        "Branches still needs its sections: {request:?}"
    );
}
