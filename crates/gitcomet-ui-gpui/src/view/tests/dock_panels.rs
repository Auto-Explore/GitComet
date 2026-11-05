//! The window's docks: panes that survive a frame they are not drawn in,
//! popped-out panels coming back, the branch list's data requests across
//! panes, and the layout menu behind a section header's own menu.

use super::*;
use crate::view::dock::{
    DockCommand, DockHalf, DockMenuEntry, DockMenuTarget, DockSide, PanelId, panel_ids, section_ids,
};
use crate::view::panels::ContextMenuAction;

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

/// Local, Remote, Worktrees, Submodules and git-annex headers open menus of
/// their own on right-click; each of those leads on to the layout menu.
#[gpui::test]
fn a_section_headers_own_menu_leads_to_its_layout_menu(cx: &mut gpui::TestAppContext) {
    let (_store, view, cx) = open_repo_window(cx);
    let repo_id = RepoId(1);
    let host = cx.update(|_window, app| view.read(app).popover_host.clone());
    let menus = [
        (
            PopoverKind::BranchSectionMenu {
                repo_id,
                section: BranchSection::Local,
            },
            section_ids::LOCAL_BRANCHES,
        ),
        (
            PopoverKind::BranchSectionMenu {
                repo_id,
                section: BranchSection::Remote,
            },
            section_ids::REMOTE_BRANCHES,
        ),
        (
            PopoverKind::worktree(repo_id, WorktreePopoverKind::SectionMenu),
            section_ids::WORKTREES,
        ),
        (
            PopoverKind::submodule(repo_id, SubmodulePopoverKind::SectionMenu),
            section_ids::SUBMODULES,
        ),
        (
            PopoverKind::annex(repo_id, AnnexPopoverKind::SectionMenu),
            section_ids::ANNEX,
        ),
    ];
    for (kind, section) in menus {
        let actions = cx.update(|_window, app| {
            host.update(app, |host, cx| {
                host.context_menu_actions_for_test(&kind, cx)
            })
        });
        assert!(
            actions.iter().any(|action| matches!(
                action,
                ContextMenuAction::OpenDockMenu(DockMenuTarget::Section(id))
                    if id.as_str() == section
            )),
            "{section}'s header menu offers its layout menu"
        );
    }

    // Choosing it swaps the header's menu for the layout menu.
    let kind = PopoverKind::worktree(repo_id, WorktreePopoverKind::SectionMenu);
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.open_popover_at(kind.clone(), gpui::point(px(120.0), px(120.0)), window, cx)
        })
    });
    test_support::redraw(cx);
    let layout = cx
        .update(|_window, app| {
            host.update(app, |host, cx| {
                host.context_menu_actions_for_test(&kind, cx)
            })
        })
        .into_iter()
        .find(|action| matches!(action, ContextMenuAction::OpenDockMenu(_)))
        .unwrap();
    cx.update(|window, app| {
        host.update(app, |host, cx| {
            host.context_menu_activate_action(layout, window, cx)
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    match cx.update(|_window, app| test_support::popover_kind(view.read(app), app)) {
        Some(PopoverKind::DockMenu(menu)) => assert!(
            menu.entries
                .contains(&DockMenuEntry::Header("Worktrees".into())),
            "the worktrees section's layout menu: {menu:?}"
        ),
        other => panic!("expected the layout menu, got {other:?}"),
    }
}
