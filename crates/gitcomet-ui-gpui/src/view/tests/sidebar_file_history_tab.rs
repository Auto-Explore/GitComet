//! The sidebar's file history tab: shown on demand, per repository, and
//! sharing the tab strip with Branches, Files and contributed tabs.

use super::extension_view_panels::{click, id, open_view, shows};
use super::*;
use crate::view::panels::ContextMenuAction;
use gitcomet_extension_api::PanelArea;

const PATH: &str = "src/main.rs";

fn open_history(view: &gpui::Entity<GitCometView>, cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.open_file_history(RepoId(1), PATH.into(), window, cx)
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
}

fn history_selected(view: &gpui::Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> bool {
    cx.update(|_window, app| {
        let pane = view.read(app).sidebar_pane.clone();
        pane.read(app).file_history_selected
    })
}

fn push_state(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    f: impl FnOnce(&mut AppState),
) {
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            let mut state = (*view.ui_model.read(cx).state).clone();
            f(&mut state);
            test_support::push_test_state(view, Arc::new(state), cx);
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
}

#[gpui::test]
fn opening_the_tab_selects_it_and_shows_its_content(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    assert!(!shows(cx, "sidebar_tab_file_history"));
    let mode = cx.update(|_window, app| view.read(app).ui_model.read(app).state.sidebar_mode);
    open_history(&view, cx);
    assert!(shows(cx, "sidebar_tab_file_history"));
    assert!(shows(cx, "sidebar_file_history_content"));
    assert!(!shows(cx, "sidebar_search_toggle"), "no tools on History");
    assert_eq!(
        cx.update(|_window, app| view.read(app).ui_model.read(app).state.sidebar_mode),
        mode,
        "the stored sidebar mode stays"
    );
}

#[gpui::test]
fn files_shows_the_browser_and_the_tab_stays_until_closed(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    open_history(&view, cx);
    click(cx, "sidebar_tab_files");
    assert!(!shows(cx, "sidebar_file_history_content"));
    assert!(shows(cx, "sidebar_search_toggle"));
    assert!(shows(cx, "sidebar_tab_file_history"));

    click(cx, "sidebar_tab_file_history");
    assert!(shows(cx, "sidebar_file_history_content"));

    let pane = cx.update(|_window, app| view.read(app).file_history_pane.clone());
    cx.update(|_window, app| pane.update(app, |pane, cx| pane.close(cx)));
    test_support::redraw(cx);
    assert!(!shows(cx, "sidebar_tab_file_history"));
    assert!(!shows(cx, "sidebar_file_history_content"));
    assert!(shows(cx, "sidebar_search_toggle"), "back on sidebar_mode");
}

#[gpui::test]
fn a_collapsed_sidebar_opens(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    cx.update(|_window, app| view.update(app, |view, cx| view.set_sidebar_collapsed(true, cx)));
    open_history(&view, cx);
    assert!(cx.update(|_window, app| !view.read(app).sidebar_collapsed));
    assert!(shows(cx, "sidebar_file_history_content"));
}

#[gpui::test]
fn the_tab_hides_while_another_repository_is_active(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    open_history(&view, cx);
    push_state(&view, cx, |state| {
        let mut other = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: PathBuf::from("/tmp/other-repo"),
            },
        );
        other.open = Loadable::Ready(());
        state.repos.push(other);
        state.active_repo = Some(RepoId(2));
    });
    assert!(!shows(cx, "sidebar_tab_file_history"));
    assert!(!shows(cx, "sidebar_file_history_content"));
    push_state(&view, cx, |state| state.active_repo = Some(RepoId(1)));
    assert!(shows(cx, "sidebar_tab_file_history"));
    assert!(shows(cx, "sidebar_file_history_content"));
}

#[gpui::test]
fn contributed_tabs_and_the_history_tab_exclude_each_other(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    click(cx, "repository_view_0");
    assert!(shows(cx, "panels_files"));

    open_history(&view, cx);
    assert!(history_selected(&view, cx));
    assert!(shows(cx, "sidebar_file_history_content"));
    assert!(!shows(cx, "panels_files"));

    click(cx, "sidebar_tab_extension_0");
    assert!(!history_selected(&view, cx));
    assert!(shows(cx, "panels_files"));
    assert!(!shows(cx, "sidebar_file_history_content"));

    click(cx, "sidebar_tab_file_history");
    assert!(shows(cx, "sidebar_file_history_content"));
    let host = cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let repository = cx.update(|_window, app| host.active_repository(app).unwrap().unwrap());
    cx.update(|_window, app| {
        host.show_panel_tab(&repository, PanelArea::Sidebar, &id("files"), app)
            .unwrap()
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(!history_selected(&view, cx));
    assert!(shows(cx, "panels_files"));
    assert!(!shows(cx, "sidebar_file_history_content"));
}

#[gpui::test]
fn locating_the_open_file_leaves_the_history_tab(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    push_state(&view, cx, |state| {
        state.repos[0].diff_state.diff_target = Some(DiffTarget::working_tree(
            PATH.into(),
            gitcomet_core::domain::DiffArea::Unstaged,
        ));
    });
    open_history(&view, cx);
    assert!(shows(cx, "sidebar_file_history_content"));
    cx.update(|_window, app| view.update(app, |view, cx| view.locate_open_file_in_explorer(cx)));
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(!shows(cx, "sidebar_file_history_content"));
}

/// Escape in the history filter clears the filter. It must not reach the
/// sidebar's own handler, which would close the open file search.
#[gpui::test]
fn escape_in_the_filter_keeps_the_sidebar_search_open(cx: &mut gpui::TestAppContext) {
    cx.update(crate::app::bind_text_input_keys_for_test);
    let (view, cx) = open_view(cx);
    let sidebar = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    cx.update(|_window, app| sidebar.update(app, |pane, _| pane.file_search_open = true));
    open_history(&view, cx);
    let pane = cx.update(|_window, app| view.read(app).file_history_pane.clone());

    let input = cx.update(|_w, app| pane.read(app).filter_input());
    cx.update(|_w, app| input.update(app, |input, cx| input.set_text("abc", cx)));
    cx.run_until_parked();

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(cx.update(|_w, app| input.read(app).text().to_string()), "");
    let focus = cx.update(|_w, app| input.read(app).focus_handle());
    cx.update(|window, _| assert!(focus.is_focused(window)));
    assert!(cx.update(|_w, app| sidebar.read(app).file_search_open));
    assert!(shows(cx, "sidebar_file_history_content"));
}

fn menu_commit(ix: usize) -> gitcomet_core::domain::Commit {
    gitcomet_core::domain::Commit {
        id: CommitId(format!("{ix:0>40}").into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: format!("Commit {ix:03}").into(),
        author: "Alice".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

/// Opens the tab over a loaded three-commit history and right-clicks row 1.
fn right_click_row_1(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    submodule: bool,
) {
    push_state(view, cx, |state| {
        let repo = &mut state.repos[0];
        repo.history_state.file_history_path = Some(PATH.into());
        repo.history_state.file_history = Loadable::Ready(
            gitcomet_core::domain::LogPage {
                commits: (0..3).map(menu_commit).collect(),
                next_cursor: None,
            }
            .into(),
        );
        if submodule {
            repo.head_gitlink_paths.insert(PATH.into());
        }
    });
    open_history(view, cx);
    let center = cx.debug_bounds("picker_prompt_item_1").unwrap().center();
    cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.run_until_parked();
    test_support::redraw(cx);
}

fn menu_kind(view: &gpui::Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> PopoverKind {
    cx.update(|_w, app| test_support::popover_kind(view.read(app), app))
        .expect("a popover is open")
}

fn menu_actions(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Vec<ContextMenuAction> {
    let kind = menu_kind(view, cx);
    let host = cx.update(|_w, app| view.read(app).popover_host.clone());
    cx.update(|_w, app| {
        host.update(app, |host, cx| {
            host.context_menu_actions_for_test(&kind, cx)
        })
    })
}

#[gpui::test]
fn right_click_on_a_row_opens_the_menu_for_that_commit(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    right_click_row_1(&view, cx, false);
    assert_eq!(
        menu_kind(&view, cx),
        PopoverKind::FileHistoryCommitMenu {
            repo_id: RepoId(1),
            commit_id: menu_commit(1).id,
            path: PATH.into(),
        }
    );
    let actions = menu_actions(&view, cx);
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, ContextMenuAction::ApplyFileChange { .. }))
    );
}

#[gpui::test]
fn the_menu_hides_apply_change_for_a_submodule(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    right_click_row_1(&view, cx, true);
    assert!(
        !menu_actions(&view, cx)
            .iter()
            .any(|a| matches!(a, ContextMenuAction::ApplyFileChange { .. }))
    );
}

fn activate(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    pick: impl Fn(&ContextMenuAction) -> bool,
) {
    let action = menu_actions(view, cx).into_iter().find(pick).unwrap();
    let host = cx.update(|_w, app| view.read(app).popover_host.clone());
    cx.update(|window, app| {
        host.update(app, |host, cx| {
            host.context_menu_activate_action(action, window, cx)
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
}

#[gpui::test]
fn revert_opens_the_confirmation(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    right_click_row_1(&view, cx, false);
    activate(&view, cx, |a| {
        matches!(a, ContextMenuAction::RevertCommit { .. })
    });
    assert_eq!(
        menu_kind(&view, cx),
        PopoverKind::RevertCommitConfirm {
            repo_id: RepoId(1),
            commit_id: menu_commit(1).id,
        }
    );
}

#[gpui::test]
fn copy_sha_keeps_the_tab_open(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    right_click_row_1(&view, cx, false);
    let sha = menu_commit(1).id;
    activate(
        &view,
        cx,
        |a| matches!(a, ContextMenuAction::CopyText { text } if text == sha.as_ref()),
    );
    assert!(shows(cx, "sidebar_file_history_content"));
    let host = cx.update(|_w, app| view.read(app).popover_host.clone());
    let copied = cx.update(|_w, app| host.update(app, |_, cx| crate::clipboard::read_text(cx)));
    assert_eq!(copied.as_deref(), Some(sha.as_ref()));
}
