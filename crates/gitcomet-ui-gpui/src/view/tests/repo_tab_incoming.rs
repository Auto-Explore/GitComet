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
    let mut loading = known.clone();
    loading.repos[1].open = Loadable::Loading;
    loading.repos[1]
        .loads_in_flight
        .request(LoadKind::UpstreamDivergence);
    publish(&view, loading, cx);
    assert!(
        cx.debug_bounds("repo_tab_busy_spinner_2").is_some()
            && cx.debug_bounds("repo_tab_incoming_2").is_none(),
        "the loading spinner outranks the count in the status slot"
    );
    known.repos[1].open = Loadable::Ready(());
    publish(&view, known.clone(), cx);
    assert!(
        cx.debug_bounds("repo_tab_incoming_2").is_some(),
        "selecting the repository retains its last-known count once loaded"
    );
    known.repos[1].incoming = incoming_state(0);
    publish(&view, known, cx);
    assert!(cx.debug_bounds("repo_tab_incoming_2").is_none());
}

#[gpui::test]
fn repo_tab_incoming_takes_the_status_slot_without_narrowing_the_name(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = tab_state();
    state.repos[1].incoming = incoming_state(12345);
    // Same-length name without a count: the reference for the name's room.
    let mut twin = RepoState::new_opening(
        RepoId(3),
        RepoSpec {
            workdir: "/tmp/incoming-repository-with-a-long-twin".into(),
        },
    );
    twin.open = Loadable::NotLoaded;
    state.repos.push(twin);
    store_for_assert.replace_snapshot_for_test(Arc::new(state.clone()));
    publish(&view, state.clone(), cx);
    cx.simulate_resize(gpui::size(px(620.0), px(460.0)));
    test_support::redraw(cx);
    // Resize starts the tab's 100ms slide on the deterministic executor clock.
    // Drawing more frames alone does not advance that animation.
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.update(|window, app| {
        window.simulate_next_frame(app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    let tab_before = cx.debug_bounds("repo_tab_2").unwrap();
    let badge_before = cx.debug_bounds("repo_tab_incoming_2").unwrap();
    let label_before = cx.debug_bounds("repo_tab_label_2").unwrap();
    assert!(cx.debug_bounds("repo_tab_initials_2").is_none());

    // Measure the reference with its lazy status/name inside the viewport,
    // then restore the incoming tab before checking hover and close ownership.
    let scroll_tabs = |cx: &mut gpui::VisualTestContext, delta_x: f32| {
        let (before, max_scroll, position) = cx.update(|_, app| {
            let root = view.read(app);
            let (scrolled, max_scroll) = test_support::repo_tab_scroll(root, app);
            (
                scrolled,
                max_scroll,
                test_support::repo_tab_strip_viewport(root, app).center(),
            )
        });
        for (x, touch_phase) in [
            (0.0, gpui::TouchPhase::Started),
            (delta_x, gpui::TouchPhase::Moved),
            (0.0, gpui::TouchPhase::Ended),
        ] {
            cx.simulate_event(gpui::ScrollWheelEvent {
                position,
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(x), px(0.0))),
                touch_phase,
                modifiers: gpui::Modifiers::default(),
            });
            cx.update(|window, app| {
                window.simulate_next_frame(app);
                let _ = window.draw(app);
            });
            cx.run_until_parked();
        }
        let (actual, actual_max) =
            cx.update(|_, app| test_support::repo_tab_scroll(view.read(app), app));
        let expected = (before - px(delta_x)).clamp(px(0.0), max_scroll);
        assert_eq!(actual_max, max_scroll);
        assert!(
            (actual - expected).abs() <= px(0.5),
            "the wheel must reveal the measured tab range: before={before:?}, delta_x={delta_x}, expected={expected:?}, actual={actual:?}"
        );
    };
    let remaining_scroll = cx.update(|_, app| {
        let (scrolled, max_scroll) = test_support::repo_tab_scroll(view.read(app), app);
        max_scroll - scrolled
    });
    assert!(
        remaining_scroll > px(0.0),
        "the 620px fixture must overflow"
    );
    scroll_tabs(cx, -f32::from(remaining_scroll));
    let twin_tab = cx.debug_bounds("repo_tab_3").unwrap();
    let viewport = cx.update(|_, app| test_support::repo_tab_strip_viewport(view.read(app), app));
    // GPUI's existing extent excludes the leading margin, as in the tab-strip
    // presentation regression; the status/name must still fit strictly inside.
    assert!(twin_tab.left() >= viewport.left() - px(0.5));
    assert!(
        twin_tab.right() <= viewport.right() + px(components::Tab::HORIZONTAL_MARGIN_PX) + px(0.5)
    );
    let twin_mark = cx.debug_bounds("repo_tab_initials_3").unwrap();
    let twin_label = cx.debug_bounds("repo_tab_label_3").unwrap();
    assert!(twin_mark.left() >= viewport.left() && twin_label.right() <= viewport.right());
    scroll_tabs(cx, f32::from(remaining_scroll));
    assert_eq!(cx.debug_bounds("repo_tab_2").unwrap(), tab_before);
    assert_eq!(
        badge_before.left() - tab_before.left(),
        twin_mark.left() - twin_tab.left(),
        "the count sits where the repository mark sits"
    );
    assert!(badge_before.right() <= label_before.left());
    assert_eq!(tab_before.size.width, twin_tab.size.width);
    assert_eq!(
        label_before.right() - tab_before.left(),
        twin_label.right() - twin_tab.left(),
        "no trailing room is reserved after the name"
    );
    assert_eq!(
        label_before.size.width + badge_before.size.width,
        twin_label.size.width + twin_mark.size.width,
        "the count costs the name only its width beyond the mark's square"
    );
    cx.simulate_mouse_move(tab_before.center(), None, gpui::Modifiers::default());
    test_support::redraw(cx);
    let badge = cx.debug_bounds("repo_tab_incoming_2").unwrap();
    let close = cx.debug_bounds("repo_tab_close_2").unwrap();
    let fade = cx.debug_bounds("repo_tab_close_fade_2").unwrap();
    assert_eq!(badge, badge_before, "hover must not move the count");
    assert!(badge.right() <= fade.left());
    assert!(close.right() <= tab_before.right());
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

#[gpui::test]
fn repo_tab_missing_warning_outranks_the_incoming_count(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = tab_state();
    state.repos[1].incoming = incoming_state(3);
    publish(&view, state.clone(), cx);
    assert!(cx.debug_bounds("repo_tab_incoming_2").is_some());
    state.repos[1].feedback_mut().missing_on_disk = true;
    publish(&view, state, cx);
    assert!(cx.debug_bounds("repo_tab_incoming_2").is_none());
    assert!(cx.debug_bounds("repo_tab_initials_2").is_none());
}
