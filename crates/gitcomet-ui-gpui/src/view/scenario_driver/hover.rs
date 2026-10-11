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
        if let Some(pending) = &mut self.pending
            && pending.row_id == row_id
        {
            pending.same_target.push(sequence);
            records.push((
                "hover_same_target_pending",
                json!({"sequence": sequence,
                "op": sequence, "row_id": row_id, "causal_sequence": pending.sequence}),
            ));
            return records;
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
        if event == "hover_target"
            && let Some(native) = window.native_input_diagnostic()
        {
            detail["arrival_qpc"] = json!(native.arrival_qpc);
            detail["qpc_frequency"] = json!(native.qpc_frequency);
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
mod tests;
