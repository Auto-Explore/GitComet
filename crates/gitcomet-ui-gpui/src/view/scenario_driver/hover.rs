//! Causal hover diagnostics for scripted runs. No state or elements are installed in normal runs.
use super::*;
use gpui::{HitboxBehavior, WindowId};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

const MAX_UNSEEN_INPUTS: usize = 4096;

thread_local! {
    static TRACKERS: RefCell<HashMap<WindowId, HoverTracker>> = RefCell::new(HashMap::new());
}

#[derive(Default)]
struct HoverTracker {
    unseen: BTreeMap<u64, &'static str>,
    pending: Option<PendingPaint>,
    painted_hover: Option<(usize, u64)>,
    painted_rows: HashMap<usize, bool>,
    paint_revision: u64,
    last_requested: u64,
    last_terminal: u64,
}

struct PendingPaint {
    sequence: u64,
    row_id: usize,
    previous_row_id: Option<usize>,
    same_target: Vec<u64>,
}

impl HoverTracker {
    fn register(&mut self, sequence: u64, source: &'static str) -> Result<(), String> {
        if self.unseen.len() >= MAX_UNSEEN_INPUTS {
            return Err("hover input queue exceeded its diagnostic bound".into());
        }
        self.last_requested = sequence;
        self.unseen.insert(sequence, source);
        Ok(())
    }

    fn observe(&mut self, sequence: u64, row_id: usize) -> Vec<(&'static str, Value)> {
        let Some(source) = self.unseen.remove(&sequence) else {
            return Vec::new();
        };
        let mut records = vec![(
            "hover_target",
            json!({"sequence": sequence, "op": sequence,
            "row_id": row_id, "source": source}),
        )];
        // SendInput/WM_MOUSEMOVE may coalesce requests before application dispatch.
        let older: Vec<_> = self.unseen.range(..sequence).map(|(&id, _)| id).collect();
        for older in older {
            self.unseen.remove(&older);
            self.last_terminal = self.last_terminal.max(older);
            records.push((
                "hover_superseded",
                json!({"sequence": older, "op": older,
                "newer_sequence": sequence, "reason": "native_event_coalesced"}),
            ));
        }
        if let Some(pending) = &mut self.pending {
            if pending.row_id == row_id {
                pending.same_target.push(sequence);
                records.push((
                    "hover_same_target_pending",
                    json!({"sequence": sequence,
                    "op": sequence, "row_id": row_id, "causal_sequence": pending.sequence}),
                ));
                return records;
            }
        }
        if let Some(pending) = self.pending.take() {
            for older in std::iter::once(pending.sequence).chain(pending.same_target) {
                self.last_terminal = self.last_terminal.max(older);
                records.push((
                    "hover_superseded",
                    json!({"sequence": older, "op": older,
                    "newer_sequence": sequence, "reason": "replaced_before_paint"}),
                ));
            }
        }
        if let Some((painted_row, revision)) = self.painted_hover
            && painted_row == row_id
        {
            self.last_terminal = self.last_terminal.max(sequence);
            records.push((
                "hover_unchanged",
                json!({"sequence": sequence, "op": sequence,
                "row_id": row_id, "scene_revision": revision}),
            ));
            return records;
        }
        self.pending = Some(PendingPaint {
            sequence,
            row_id,
            previous_row_id: self.painted_hover.map(|(row, _)| row),
            same_target: Vec::new(),
        });
        records
    }

    fn paint(&mut self, row_id: usize, hovered: bool, revision: u64) -> Vec<(&'static str, Value)> {
        if self.paint_revision != revision {
            self.paint_revision = revision;
            self.painted_rows.clear();
        }
        self.painted_rows.insert(row_id, hovered);
        let Some(pending) = &self.pending else {
            if hovered {
                self.painted_hover = Some((row_id, revision));
            } else if self.painted_hover.is_some_and(|(row, _)| row == row_id) {
                self.painted_hover = None;
            }
            return Vec::new();
        };
        let previous_cleared = pending
            .previous_row_id
            .is_none_or(|previous| self.painted_rows.get(&previous) == Some(&false));
        if self.painted_rows.get(&pending.row_id) != Some(&true) || !previous_cleared {
            return Vec::new();
        }
        let pending = self.pending.take().expect("checked pending hover paint");
        self.painted_hover = Some((pending.row_id, revision));
        self.last_terminal = self.last_terminal.max(pending.sequence);
        let mut records = vec![(
            "hover_paint",
            json!({"sequence": pending.sequence,
            "op": pending.sequence, "row_id": pending.row_id, "hovered": true,
            "previous_row_id": pending.previous_row_id, "previous_cleared": previous_cleared,
            "scene_revision": revision}),
        )];
        for sequence in pending.same_target {
            self.last_terminal = self.last_terminal.max(sequence);
            records.push((
                "hover_unchanged",
                json!({"sequence": sequence, "op": sequence,
                "row_id": pending.row_id, "scene_revision": revision,
                "causal_sequence": pending.sequence}),
            ));
        }
        records
    }

    fn settled(&self) -> bool {
        self.last_requested > 0
            && self.unseen.is_empty()
            && self.pending.is_none()
            && self.last_terminal >= self.last_requested
    }
}

pub(in crate::view) fn enabled() -> bool {
    static SCENARIO: OnceLock<bool> = OnceLock::new();
    crate::ui_probe::jsonl_enabled()
        && *SCENARIO
            .get_or_init(|| std::env::var_os(SCENARIO_ENV).is_some_and(|path| !path.is_empty()))
}

pub(super) fn install(window: WindowId) {
    TRACKERS.with(|trackers| {
        trackers.borrow_mut().entry(window).or_default();
    });
}

pub(super) fn uninstall(window: WindowId) {
    TRACKERS.with(|trackers| {
        trackers.borrow_mut().remove(&window);
    });
}

pub(super) fn register(
    window: WindowId,
    sequence: u64,
    source: &'static str,
) -> Result<(), String> {
    TRACKERS.with(|trackers| {
        trackers
            .borrow_mut()
            .entry(window)
            .or_default()
            .register(sequence, source)
    })
}

pub(super) fn settled(window: WindowId) -> bool {
    TRACKERS.with(|trackers| {
        trackers
            .borrow()
            .get(&window)
            .is_some_and(HoverTracker::settled)
    })
}

fn emit(records: Vec<(&'static str, Value)>, window: &Window) {
    for (event, mut detail) in records {
        detail["window"] = json!(format!("{:?}", window.window_handle().window_id()));
        if event == "hover_target" {
            if let Some(native) = window.native_input_diagnostic() {
                detail["arrival_qpc"] = json!(native.arrival_qpc);
                detail["qpc_frequency"] = json!(native.qpc_frequency);
            }
        }
        record(event, detail);
    }
}

/// An empty paint trailer observes the exact row hitbox after its visible children paint.
/// The extra layout/hitbox is diagnostic only and identical in both sides of a paired run.
pub(in crate::view) fn attach(
    row: gpui::Stateful<gpui::Div>,
    row_id: usize,
) -> gpui::Stateful<gpui::Div> {
    if !enabled() {
        return row;
    }
    attach_diagnostic(row, row_id)
}

fn attach_diagnostic(row: gpui::Stateful<gpui::Div>, row_id: usize) -> gpui::Stateful<gpui::Div> {
    row.relative().child(
        gpui::canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                let id = window.window_handle().window_id();
                let records = TRACKERS.with(|trackers| {
                    trackers
                        .borrow_mut()
                        .get_mut(&id)
                        .map_or_else(Vec::new, |tracker| {
                            tracker.paint(
                                row_id,
                                hitbox.is_hovered(window),
                                window.scene_revision(),
                            )
                        })
                });
                emit(records, window);
                window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, _| {
                    if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                        return;
                    }
                    let sequence = window
                        .native_input_diagnostic()
                        .map(|native| native.sequence)
                        .or_else(|| op_trace::Stamp::capture().map(|stamp| stamp.op))
                        .unwrap_or(0);
                    if sequence == 0 {
                        return;
                    }
                    let records = TRACKERS.with(|trackers| {
                        trackers
                            .borrow_mut()
                            .get_mut(&id)
                            .map_or_else(Vec::new, |tracker| tracker.observe(sequence, row_id))
                    });
                    emit(records, window);
                });
            },
        )
        .absolute()
        .inset_0()
        .size_full(),
    )
}

#[cfg(test)]
mod tests {
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
        let old_revision =
            TRACKERS.with(|trackers| trackers.borrow()[&id].painted_hover.unwrap().1);
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
}
