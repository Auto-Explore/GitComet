use super::*;
use std::rc::Rc;

#[test]
fn hover_requires_new_highlight_and_old_row_clear_in_the_same_scene() {
    let mut tracker = HoverTracker::default();
    tracker.paint(1, true, 1);
    tracker.register(11, "synthetic").unwrap();
    tracker.observe(11, 2);
    assert!(tracker.paint(2, true, 2).is_empty());
    assert!(!tracker.settled());
    let records = tracker.paint(1, false, 2);
    assert_eq!(records[0].0, "hover_paint");
    assert_eq!(records[0].1["scene_revision"], 2);
    assert!(tracker.settled());
}

#[test]
fn rapid_targets_supersede_unpainted_inputs_without_claiming_latency() {
    let mut tracker = HoverTracker::default();
    tracker.register(1, "native").unwrap();
    tracker.register(2, "native").unwrap();
    let records = tracker.observe(2, 3);
    assert!(
        records
            .iter()
            .any(|(event, detail)| *event == "hover_superseded" && detail["sequence"] == 1)
    );
    tracker.register(3, "native").unwrap();
    let records = tracker.observe(3, 4);
    assert!(
        records
            .iter()
            .any(|(event, detail)| *event == "hover_superseded" && detail["sequence"] == 2)
    );
    assert!(tracker.paint(3, false, 1).is_empty());
    assert_eq!(tracker.paint(4, true, 1)[0].1["sequence"], 3);
    assert!(tracker.settled());
}

#[test]
fn same_row_inputs_do_not_invent_new_causal_paints() {
    let mut tracker = HoverTracker::default();
    tracker.register(1, "synthetic").unwrap();
    tracker.observe(1, 4);
    tracker.register(2, "synthetic").unwrap();
    assert_eq!(tracker.observe(2, 4)[1].0, "hover_same_target_pending");
    let records = tracker.paint(4, true, 8);
    assert_eq!(
        records
            .iter()
            .filter(|(event, _)| *event == "hover_paint")
            .count(),
        1
    );
    tracker.register(3, "synthetic").unwrap();
    assert_eq!(tracker.observe(3, 4)[1].0, "hover_unchanged");
    assert!(tracker.settled());
}

struct HoverRows {
    clicks: Rc<std::cell::Cell<usize>>,
}

impl Render for HoverRows {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let row = |id| {
            let clicks = self.clicks.clone();
            attach_diagnostic(
                div()
                    .id(id)
                    .debug_selector(move || format!("hover_probe_row_{id}"))
                    .w(px(200.0))
                    .h(px(36.0))
                    .hover(|style| style.bg(gpui::rgba(0x335577ff)))
                    .on_mouse_down(MouseButton::Left, move |_, _, _| {
                        clicks.set(clicks.get() + 1)
                    }),
                id,
            )
        };
        div().flex().flex_col().children([row(1), row(2)])
    }
}

#[gpui::test]
fn diagnostic_trailer_witnesses_hover_without_occluding_clicks(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let clicks = Rc::new(std::cell::Cell::new(0));
    let (_, cx) = cx.add_window_view(|_, _| HoverRows {
        clicks: clicks.clone(),
    });
    let id = cx.update(|window, _| window.window_handle().window_id());
    install(id);
    let first = cx.debug_bounds("hover_probe_row_1").unwrap().center();
    let second = cx.debug_bounds("hover_probe_row_2").unwrap().center();
    cx.simulate_mouse_move(first, None, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let old_revision = TRACKERS.with(|trackers| trackers.borrow()[&id].painted_hover.unwrap().1);
    register(id, 11, "synthetic").unwrap();
    TRACKERS.with(|trackers| {
        trackers.borrow_mut().get_mut(&id).unwrap().observe(11, 2);
    });
    cx.simulate_mouse_move(second, None, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(settled(id));
    TRACKERS.with(|trackers| {
        let trackers = trackers.borrow();
        let tracker = &trackers[&id];
        assert_eq!(tracker.painted_hover.unwrap().0, 2);
        assert!(tracker.painted_hover.unwrap().1 > old_revision);
        assert_eq!(tracker.painted_rows.get(&1), Some(&false));
    });
    cx.simulate_mouse_down(second, MouseButton::Left, Modifiers::default());
    assert_eq!(
        clicks.get(),
        1,
        "the normal diagnostic hitbox must preserve the row click"
    );
    uninstall(id);
}

#[gpui::test]
fn window_tracker_lifecycle_preserves_survivor_and_releases_closed_windows(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let baseline = TRACKERS.with(|trackers| trackers.borrow().len());
    let keeper = {
        let (_, visual) = cx.add_window_view(|_, _| HoverRows {
            clicks: Rc::new(std::cell::Cell::new(0)),
        });
        let id = visual.update(|window, _| window.window_handle().window_id());
        install(id);
        let first = visual.debug_bounds("hover_probe_row_1").unwrap().center();
        visual.simulate_mouse_move(first, None, Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, app| {
            let _ = window.draw(app);
        });
        register(id, 999, "synthetic").unwrap();
        id
    };

    for cycle in 0..32 {
        let (_, visual) = cx.add_window_view(|_, _| HoverRows {
            clicks: Rc::new(std::cell::Cell::new(0)),
        });
        let id = visual.update(|window, _| window.window_handle().window_id());
        install(id);
        let first = visual.debug_bounds("hover_probe_row_1").unwrap().center();
        visual.simulate_mouse_move(first, None, Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, app| {
            let _ = window.draw(app);
        });
        register(id, 11, "synthetic").unwrap();
        TRACKERS.with(|trackers| {
            let mut trackers = trackers.borrow_mut();
            let tracker = trackers.get_mut(&id).unwrap();
            tracker.observe(11, 2);
            assert_eq!(tracker.pending.as_ref().unwrap().previous_row_id, Some(1));
            // A first measured input must not claim the new highlight until
            // the initially painted row has cleared in that same scene.
            assert!(
                tracker
                    .paint(2, true, tracker.paint_revision + 1)
                    .is_empty()
            );
            tracker.register(12, "native").unwrap();
            tracker.observe(12, 2);
            tracker.register(13, "native").unwrap();
        });
        install(id);
        TRACKERS.with(|trackers| {
            let trackers = trackers.borrow();
            let tracker = &trackers[&id];
            assert_eq!(tracker.pending.as_ref().unwrap().same_target, vec![12]);
            assert!(tracker.unseen.contains_key(&13));
        });
        // Match Driver::close_window: release diagnostics only once GPUI
        // has actually removed the window, not when a close is requested.
        visual.update(|window, _| window.remove_window());
        visual.run_until_parked();
        assert!(
            !visual
                .windows()
                .iter()
                .any(|window| window.window_id() == id)
        );
        uninstall(id);
        TRACKERS.with(|trackers| {
            let trackers = trackers.borrow();
            assert_eq!(trackers.len(), baseline + 1, "cycle {cycle}");
            assert!(!trackers.contains_key(&id));
            assert!(trackers[&keeper].unseen.contains_key(&999));
            assert_eq!(trackers[&keeper].painted_hover.unwrap().0, 1);
        });
    }
    uninstall(keeper);
    assert_eq!(TRACKERS.with(|trackers| trackers.borrow().len()), baseline);
}
