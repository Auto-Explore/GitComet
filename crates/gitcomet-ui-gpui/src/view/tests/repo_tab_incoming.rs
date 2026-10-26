use super::*;
use gitcomet_core::domain::{HeadTracking, UpstreamDivergence};
use gitcomet_state::model::{LoadKind, RepoIncomingSnapshot, RepoIncomingState};
use std::cell::Cell;
use std::rc::Rc;
use std::time::UNIX_EPOCH;

fn incoming_state(behind: usize) -> RepoIncomingState {
    RepoIncomingState::Current(RepoIncomingSnapshot {
        tracking: HeadTracking::Tracking {
            branch: "main".into(),
            upstream: Upstream {
                remote: "origin".into(),
                branch: "main".into(),
            },
            divergence: Some(UpstreamDivergence { ahead: 0, behind }),
        },
        checked_at: UNIX_EPOCH,
    })
}

fn tab_state() -> AppState {
    let mut active = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: "/tmp/active".into(),
        },
    );
    active.open = Loadable::Ready(());
    let mut inactive = RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: "/tmp/incoming-repository-with-a-long-name".into(),
        },
    );
    inactive.open = Loadable::NotLoaded;
    AppState {
        active_repo: Some(RepoId(1)),
        repos: vec![active, inactive],
        ..AppState::test_default()
    }
}

fn publish(view: &Entity<GitCometView>, state: AppState, cx: &mut gpui::VisualTestContext) {
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(view, Arc::new(state), cx)
        });
    });
    cx.run_until_parked();
    test_support::redraw(cx);
}

#[gpui::test]
fn repo_tab_incoming_updates_inactive_tabs_and_survives_selection_until_caught_up(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = tab_state();
    publish(&view, state.clone(), cx);
    assert!(cx.debug_bounds("repo_tab_incoming_2").is_none());
    let notifications = Rc::new(Cell::new(0));
    let _subscription = cx.update(|_window, app| {
        let bar = view.read(app).repo_tabs_bar.clone();
        let count = notifications.clone();
        app.observe(&bar, move |_, _| count.set(count.get() + 1))
    });
    let mut known = state;
    known.repos[1].incoming = incoming_state(3);
    publish(&view, known.clone(), cx);
    assert!(
        notifications.get() > 0,
        "inactive incoming data must notify the tab strip"
    );
    assert!(cx.debug_bounds("repo_tab_incoming_2").is_some());
    assert_eq!(known.active_repo, Some(RepoId(1)));
    assert!(matches!(known.repos[1].open, Loadable::NotLoaded));

    known.active_repo = Some(RepoId(2));
    known.repos[1].open = Loadable::Loading;
    known.repos[1]
        .loads_in_flight
        .request(LoadKind::UpstreamDivergence);
    publish(&view, known.clone(), cx);
    assert!(
        cx.debug_bounds("repo_tab_incoming_2").is_some(),
        "selecting the repository retains its last-known count while loading"
    );
    known.repos[1].incoming = incoming_state(0);
    publish(&view, known, cx);
    assert!(cx.debug_bounds("repo_tab_incoming_2").is_none());
}

#[gpui::test]
fn repo_tab_incoming_keeps_large_counts_visible_beside_hover_close_in_a_narrow_strip(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = tab_state();
    state.repos[1].incoming = incoming_state(12345);
    store_for_assert.replace_snapshot_for_test(Arc::new(state.clone()));
    publish(&view, state, cx);
    cx.simulate_resize(gpui::size(px(620.0), px(460.0)));
    test_support::redraw(cx);
    let tab_before = cx.debug_bounds("repo_tab_2").unwrap();
    let badge_before = cx.debug_bounds("repo_tab_incoming_2").unwrap();
    let label_before = cx.debug_bounds("repo_tab_label_2").unwrap();
    assert!(badge_before.left() >= tab_before.left());
    assert!(badge_before.right() <= tab_before.right());
    assert!(label_before.right() <= badge_before.left());
    cx.simulate_mouse_move(tab_before.center(), None, gpui::Modifiers::default());
    test_support::redraw(cx);
    let badge = cx.debug_bounds("repo_tab_incoming_2").unwrap();
    let close = cx.debug_bounds("repo_tab_close_2").unwrap();
    let fade = cx.debug_bounds("repo_tab_close_fade_2").unwrap();
    assert_eq!(
        badge, badge_before,
        "hover must not move or cover the incoming count"
    );
    assert!(
        badge.right() <= fade.left(),
        "the close fade must begin after the count"
    );
    assert!(badge.right() <= close.left());
    assert_eq!(label_before, cx.debug_bounds("repo_tab_label_2").unwrap());
    cx.simulate_mouse_down(
        close.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        close.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    store_for_assert
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    let closed = store_for_assert.snapshot();
    assert!(
        closed.repos.iter().all(|repo| repo.id != RepoId(2)),
        "close remains an independent action"
    );
    assert_eq!(
        closed.active_repo,
        Some(RepoId(1)),
        "closing an inactive tab must not activate it"
    );
}
