use super::*;
use crate::view::test_support::{self, TestBackend};
use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet, Tag};

fn tag_fixture() -> Arc<AppState> {
    tag_fixture_with_focus(HistoryFocusSet::default())
}

fn tag_fixture_with_focus(focus: HistoryFocusSet) -> Arc<AppState> {
    let mut state = long_list_tests::branch_fixture(1);
    let repo = &mut Arc::make_mut(&mut state).repos[0];
    repo.tags = Loadable::Ready(Arc::new(vec![Tag {
        name: "v1.0".into(),
        target: CommitId("abc123".into()),
    }]));
    repo.history_state.history_focus = focus;
    state
}

/// Draws the sidebar with one tag and returns the center of its row.
fn tag_row_center(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::Entity<GitCometView>,
    AppStore,
    &mut gpui::VisualTestContext,
    gpui::Point<Pixels>,
) {
    let (view, store, cx, _, center) = render_tag_row(cx, tag_fixture());
    (view, store, cx, center)
}

/// Draws the sidebar for `state` and returns its first tag row's index and
/// center.
fn render_tag_row(
    cx: &mut gpui::TestAppContext,
    state: Arc<AppState>,
) -> (
    gpui::Entity<GitCometView>,
    AppStore,
    &mut gpui::VisualTestContext,
    usize,
    gpui::Point<Pixels>,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    (view, store, cx, ix, bounds.center())
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

#[gpui::test]
fn sidebar_tag_row_has_no_focus_eye_while_unfocused(cx: &mut gpui::TestAppContext) {
    let (_view, _store, cx, ix, _) = render_tag_row(cx, tag_fixture());
    assert!(
        cx.debug_bounds(format!("sidebar_focus_eye_{ix}").leak())
            .is_none()
    );
}

/// While the history is focused, tag rows carry the eye in the same column as
/// the branch rows' eyes.
#[gpui::test]
fn sidebar_tag_row_eye_lines_up_with_the_branch_eyes(cx: &mut gpui::TestAppContext) {
    let focus = HistoryFocusSet::from_iter([HistoryFocus::local_branch("main")]);
    let (_view, _store, cx, ix, _) = render_tag_row(cx, tag_fixture_with_focus(focus));
    let tag_eye = cx
        .debug_bounds(format!("sidebar_focus_eye_{ix}").leak())
        .expect("the tag row carries an eye while focused");
    let branch_eye = (0..ix)
        .find_map(|other| cx.debug_bounds(format!("sidebar_focus_eye_{other}").leak()))
        .expect("a branch row's eye above the tags");
    assert_eq!(tag_eye.left(), branch_eye.left());
}

/// Clicking a tag's eye toggles that tag in the focus.
#[gpui::test]
fn sidebar_tag_eye_focuses_the_tag(cx: &mut gpui::TestAppContext) {
    let focus = HistoryFocusSet::from_iter([HistoryFocus::local_branch("main")]);
    let (_view, store, cx, ix, _) = render_tag_row(cx, tag_fixture_with_focus(focus));
    // The startup toast covers the bottom-left, where the tag row lands at the
    // default size; a taller window keeps the eye clear of it.
    cx.simulate_resize(gpui::size(px(1100.0), px(1800.0)));
    test_support::redraw(cx);
    let eye = cx
        .debug_bounds(format!("sidebar_focus_eye_{ix}").leak())
        .expect("the tag row carries an eye while focused");
    cx.simulate_click(eye.center(), gpui::Modifiers::default());

    let expected = HistoryFocusSet::from_iter([
        HistoryFocus::local_branch("main"),
        HistoryFocus::tag("v1.0"),
    ]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while store.snapshot().repos[0].history_state.history_focus != expected {
        assert!(
            std::time::Instant::now() < deadline,
            "the tag never joined the focus: {:?}",
            store.snapshot().repos[0].history_state.history_focus
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
