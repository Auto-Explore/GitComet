use super::*;
use crate::view::test_support::{self, TestBackend};
use gitcomet_core::domain::Tag;

fn tag_fixture() -> Arc<AppState> {
    let mut state = long_list_tests::branch_fixture(1);
    Arc::make_mut(&mut state).repos[0].tags = Loadable::Ready(Arc::new(vec![Tag {
        name: "v1.0".into(),
        target: CommitId("abc123".into()),
    }]));
    state
}

/// Draws the sidebar with one tag and returns the center of its row.
fn tag_row_center<'a>(
    cx: &'a mut gpui::TestAppContext,
) -> (
    gpui::Entity<GitCometView>,
    AppStore,
    &'a mut gpui::VisualTestContext,
    gpui::Point<Pixels>,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let state = tag_fixture();
    store.replace_snapshot_for_test(state.clone());
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    cx.simulate_resize(gpui::size(px(1100.0), px(1100.0)));
    test_support::redraw(cx);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    let ix = cx.update(|_, app| {
        pane.update(app, |pane, _| {
            pane.branch_sidebar_presentation_cached()
                .unwrap()
                .rows
                .iter()
                .position(|row| matches!(row, BranchSidebarRow::TagItem { .. }))
                .expect("a tag row")
        })
    });
    let bounds = cx
        .debug_bounds(format!("tag_sidebar_row_{ix}").leak())
        .expect("the tag row is painted");
    (view, store, cx, bounds.center())
}

#[gpui::test]
fn sidebar_tag_row_click_reveals_the_tag_commit(cx: &mut gpui::TestAppContext) {
    let (_view, store, cx, center) = tag_row_center(cx);
    cx.simulate_click(center, gpui::Modifiers::default());
    cx.run_until_parked();
    // The reveal is reduced on the store's worker thread.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while store.snapshot().repos[0]
        .history_state
        .reveal_target
        .is_none()
    {
        assert!(std::time::Instant::now() < deadline, "no RevealCommit");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        store.snapshot().repos[0].history_state.reveal_target,
        Some(CommitId("abc123".into()))
    );
}

#[gpui::test]
fn sidebar_tag_row_right_click_opens_the_tag_menu(cx: &mut gpui::TestAppContext) {
    let (view, _store, cx, center) = tag_row_center(cx);
    cx.simulate_mouse_down(center, MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Right, gpui::Modifiers::default());
    test_support::redraw(cx);
    assert!(cx.update(|_, app| matches!(
        test_support::popover_kind(view.read(app), app),
        Some(PopoverKind::TagMenu { commit_id, name: Some(name), .. })
            if commit_id == CommitId("abc123".into()) && name == "v1.0"
    )));
}
