use super::*;
use crate::view::test_support::{self, TestBackend};
use gitcomet_core::domain::StashEntry;

fn stash_fixture() -> Arc<AppState> {
    let mut state = long_list_tests::branch_fixture(1);
    let repo = &mut Arc::make_mut(&mut state).repos[0];
    repo.stashes = Loadable::Ready(Arc::new(vec![StashEntry {
        index: 0,
        id: CommitId("c0ffee".into()),
        message: "stash message".into(),
        created_at: None,
    }]));
    state
}

#[gpui::test]
fn sidebar_stash_row_click_reveals_the_stash_commit(cx: &mut gpui::TestAppContext) {
    let state = stash_fixture();
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
    let center = cx
        .debug_bounds("stash_sidebar_row_0")
        .expect("the stash row is painted")
        .center();

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
        Some(CommitId("c0ffee".into()))
    );
}
