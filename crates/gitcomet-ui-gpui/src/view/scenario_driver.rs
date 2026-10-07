//! Opt-in scripted scenarios for measuring the live application.
//!
//! `GITCOMET_UI_SCENARIO=<file.json>` runs the steps in that file against the
//! first main window once it opens, then quits. It needs the UI probe with
//! JSONL output (`GITCOMET_UI_PROBE=1`, `GITCOMET_UI_PROBE_JSONL=<path>`),
//! which receives every record. Without the variable nothing here runs.
//!
//! Input goes through the production paths: keys via
//! `Window::dispatch_keystroke` (keymap, interceptors, text input), mouse and
//! wheel events via `Window::dispatch_event` at positions taken from the
//! rendered layout. Each input is its own traced operation, so its store,
//! worker and publication stages share its id. Inputs follow a fixed schedule;
//! a late input is dispatched late rather than skipped, and the delay is
//! recorded. A completion witness per input proves the result reached the UI;
//! when a later input's witness holds first, the earlier input is recorded as
//! superseded (coalesced) instead of complete.
//!
//! See scripts/profiling/live-ui.py for the harness that writes scenarios and
//! analyses the records.

use super::*;
use gitcomet_core::history_perf::{self, Work};
use gitcomet_core::op_trace::{self, Stage};
use gpui::{
    AnyWindowHandle, AsyncApp, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

mod operations;
mod windows;
use operations::Operation;

const SCENARIO_ENV: &str = "GITCOMET_UI_SCENARIO";
const DEFAULT_WITNESS_TIMEOUT: Duration = Duration::from_secs(15);
/// Re-check spacing while witnesses are pending. Entity notifications catch
/// most changes at once, but some results land in several steps (a search
/// worker restarting, say); this bounds the witness-time error.
const WITNESS_POLL: Duration = Duration::from_millis(4);

#[derive(Debug, Deserialize)]
struct Scenario {
    steps: Vec<Step>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Step {
    /// Waits until the active repository is open with status, history and a
    /// laid-out history list. Setup; not measured.
    WaitReady {
        #[serde(default = "default_ready_timeout_ms")]
        timeout_ms: u64,
        #[serde(default)]
        indexed: bool,
    },
    /// Sleeps; used to let startup work finish before measuring.
    Settle {
        ms: u64,
    },
    /// Ends the previous phase and starts a named one.
    Phase {
        name: String,
    },
    /// Moves keyboard focus inside the window.
    Focus {
        target: FocusTarget,
    },
    /// Presses a key (`Keystroke::parse` syntax) on a fixed schedule.
    Keys {
        key: String,
        repeat: usize,
        interval_ms: u64,
        #[serde(default)]
        witness: Option<WitnessKind>,
    },
    /// Types each character on a fixed schedule.
    Type {
        text: String,
        interval_ms: u64,
        #[serde(default)]
        witness: Option<WitnessKind>,
    },
    /// Paste via the production clipboard and text input shortcut.
    Paste {
        text: String,
        #[serde(default)]
        witness: Option<WitnessKind>,
    },
    /// Assert the final filter result, including keyboard selection. This reads
    /// the existing cache; it must never complete filtering on the app's behalf.
    ExpectRepoPicker {
        query: String,
        matches: usize,
        selected: Option<usize>,
    },
    /// Wheel events over a target on a fixed schedule; `delta_px` is
    /// negative to scroll down. Direction flips every `flip_every` events.
    Scroll {
        target: ScrollTarget,
        delta_px: f32,
        repeat: usize,
        interval_ms: u64,
        #[serde(default)]
        flip_every: Option<usize>,
        #[serde(default)]
        witness: Option<WitnessKind>,
    },
    /// Clicks a row of a list.
    Click {
        target: ClickTarget,
        #[serde(default)]
        witness: Option<WitnessKind>,
    },
    /// Writes every path at once (a checkout or build touching many files),
    /// then restores them (or deletes the new ones) the next round. With
    /// `expect_status` each round's witness is the status list showing all
    /// of them changed, then none; paths in ignored directories take none,
    /// and their cost shows in the refresh work the stage records capture.
    WriteFiles {
        paths: Vec<PathBuf>,
        contents: String,
        rounds: usize,
        interval_ms: u64,
        #[serde(default = "default_true")]
        expect_status: bool,
    },
    /// Runs a command-palette command (setup, e.g. `toggle-terminal`). A
    /// witness makes the next step wait until the store has applied it.
    Command {
        id: String,
        #[serde(default)]
        witness: Option<WitnessKind>,
    },
    /// Opens a repository as the Open Repository flow does once a folder is
    /// picked, as a traced input whose witness is that repository loaded.
    OpenRepo {
        path: PathBuf,
        #[serde(default = "default_true")]
        wait: bool,
    },
    /// Open a real application window and make it the input target.
    NewWindow {
        name: String,
    },
    /// Activate a named window. The initial window is named `primary`.
    SwitchWindow {
        name: String,
    },
    /// Close another named window through the normal close guards.
    CloseWindow {
        name: String,
    },
    /// Assert and record the live main windows and their repository state.
    ExpectWindows {
        count: usize,
    },
    /// Minimizes the window (idle measurements).
    Minimize,
    /// Coordinates are fractions of the rendered history viewport. Sweeps
    /// exercise real hit testing; a dwell can require a visible message card.
    MovePointer {
        positions: Vec<[f32; 2]>,
        repeat: usize,
        interval_ms: u64,
        #[serde(default)]
        witness: Option<WitnessKind>,
    },
    DragHistory {
        to_fraction: f32,
        steps: usize,
        interval_ms: u64,
        #[serde(default)]
        wait_for_rows: bool,
    },
    /// Starts through the production store, then returns so subsequent input
    /// runs concurrently with the operation. WaitOperation proves completion.
    StartOperation {
        operation: String,
        #[serde(default)]
        path: PathBuf,
        #[serde(default)]
        remote: String,
        #[serde(default)]
        url: String,
        #[serde(default)]
        dest: PathBuf,
    },
    WaitOperation {
        timeout_ms: u64,
        #[serde(default)]
        cancelled: bool,
    },
    CancelOperation,
    /// Use the activity dialog's production minimize handler, then prove it
    /// no longer occludes the history targeted by subsequent input.
    MinimizeActivity,
}

fn default_ready_timeout_ms() -> u64 {
    120_000
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FocusTarget {
    History,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScrollTarget {
    History,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "list", rename_all = "snake_case")]
enum ClickTarget {
    /// A row of the unstaged/changed files list, by display index.
    UnstagedRow { index: usize },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WitnessKind {
    /// The selection changed and the selected commit's details are loaded.
    CommitDetails,
    /// A diff target is selected and none of its loads is in flight.
    DiffLoaded,
    /// The history list moved.
    HistoryScrolled,
    HistoryJumpLoaded,
    /// The diff search settled on the typed query.
    SearchSettled,
    RepoPickerFiltered,
    /// The repository at `path` is active with status and history loaded.
    RepoOpen {
        path: PathBuf,
    },
    /// No open repository has `path` as its work tree.
    RepoClosed {
        path: PathBuf,
    },
    MessageHover,
}

impl WitnessKind {
    fn name(&self) -> &'static str {
        match self {
            Self::CommitDetails => "commit_details",
            Self::DiffLoaded => "diff_loaded",
            Self::HistoryScrolled => "history_scrolled",
            Self::HistoryJumpLoaded => "history_jump_loaded",
            Self::SearchSettled => "search_settled",
            Self::RepoPickerFiltered => "repo_picker_filtered",
            Self::RepoOpen { .. } => "repo_open",
            Self::RepoClosed { .. } => "repo_closed",
            Self::MessageHover => "message_hover",
        }
    }
}

/// What an input's witness compares against, captured at dispatch.
#[derive(Clone, Debug)]
enum Baseline {
    Selection(Option<CommitId>),
    ScrollPosition(f64),
    Query(SharedString),
    None,
}

struct Pending {
    op: u64,
    kind: WitnessKind,
    baseline: Baseline,
    /// A later selection resolved while this one was pending.
    target: Option<CommitId>,
}

struct Driver {
    window: AnyWindowHandle,
    windows: FxHashMap<String, AnyWindowHandle>,
    view: Entity<GitCometView>,
    changed: smol::channel::Receiver<()>,
    _observers: Vec<gpui::Subscription>,
    errors: Vec<String>,
    phase: Option<String>,
    operation: Option<Operation>,
    pointer_expected: Option<Point<Pixels>>,
    work_start: [u64; Work::ALL.len()],
    #[cfg(feature = "perf-alloc")]
    allocation_start: crate::perf_alloc::PerfAllocMetrics,
}

pub(crate) fn start_if_requested(cx: &mut App) {
    let Some(path) = std::env::var_os(SCENARIO_ENV).filter(|path| !path.is_empty()) else {
        return;
    };
    let scenario = match std::fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| serde_json::from_slice::<Scenario>(&bytes).map_err(|e| e.to_string()))
    {
        Ok(scenario) => scenario,
        Err(error) => {
            // A scenario run must end: a harness waits for this process.
            eprintln!("ui scenario: cannot load {path:?}: {error}");
            std::process::exit(2);
        }
    };
    if !crate::ui_probe::jsonl_enabled() {
        eprintln!("ui scenario: needs GITCOMET_UI_PROBE=1 and GITCOMET_UI_PROBE_JSONL");
        std::process::exit(2);
    }
    cx.spawn(async move |cx| run(scenario, cx).await).detach();
}

async fn run(scenario: Scenario, cx: &mut AsyncApp) {
    record(
        "scenario_diagnostics",
        json!({"allocation_tracking": cfg!(feature = "perf-alloc")}),
    );
    let mut driver = match Driver::attach(cx).await {
        Ok(driver) => driver,
        Err(error) => {
            record(
                "scenario_end",
                json!({"outcome": "failed", "errors": [error]}),
            );
            finish(cx).await;
            return;
        }
    };
    for (index, step) in scenario.steps.iter().enumerate() {
        if let Err(error) = driver.step(step, cx).await {
            driver
                .errors
                .push(format!("step {index} ({step:?}): {error}"));
            break;
        }
    }
    if driver.operation.is_some() {
        let _ = driver.cancel_operation(cx);
        driver
            .errors
            .push("scenario ended with an unwitnessed operation".into());
    }
    driver.end_phase();
    let work = history_perf::snapshot();
    let counts: serde_json::Map<String, Value> = Work::ALL
        .iter()
        .enumerate()
        .map(|(index, kind)| (kind.name().into(), json!(work[index])))
        .collect();
    record("scenario_work_total", json!({"counts": counts}));
    let outcome = if driver.errors.is_empty() {
        "passed"
    } else {
        "failed"
    };
    record(
        "scenario_end",
        json!({"outcome": outcome, "errors": driver.errors}),
    );
    finish(cx).await;
}

async fn finish(cx: &mut AsyncApp) {
    // The probe drains frames and stage records once per interval; wait for
    // one more so the last inputs keep their records.
    cx.background_executor()
        .timer(crate::ui_probe::interval() + Duration::from_millis(300))
        .await;
    crate::ui_probe::flush(Duration::from_secs(5));
    cx.update(|cx| cx.quit());
}

/// The keystroke typing `ch` produces. Built directly because
/// `Keystroke::parse` treats `-` as a modifier separator.
fn text_keystroke(ch: char) -> Keystroke {
    let (key, key_char) = match ch {
        ' ' => ("space".to_owned(), " ".to_owned()),
        '\n' => ("enter".to_owned(), "\n".to_owned()),
        ch => (ch.to_lowercase().to_string(), ch.to_string()),
    };
    Keystroke {
        modifiers: Modifiers {
            shift: ch.is_uppercase(),
            ..Modifiers::default()
        },
        key,
        key_char: Some(key_char),
    }
}

fn record(event: &'static str, detail: Value) {
    crate::ui_probe::scenario_record(event, detail);
}

impl Driver {
    async fn attach(cx: &mut AsyncApp) -> Result<Self, String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let (window, view) = loop {
            let found = cx.update(|cx| {
                cx.windows().into_iter().find_map(|window| {
                    let view = window.downcast::<GitCometView>()?.entity(cx).ok()?;
                    Some((window, view))
                })
            });
            if let Some(found) = found {
                break found;
            }
            if Instant::now() > deadline {
                return Err("no main window within 60 s".into());
            }
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
        };
        let (changed, observers) = Self::observe_view(&view, cx);
        Ok(Self {
            window,
            windows: [("primary".into(), window)].into_iter().collect(),
            view,
            changed,
            _observers: observers,
            errors: Vec::new(),
            phase: None,
            operation: None,
            pointer_expected: None,
            work_start: history_perf::snapshot(),
            #[cfg(feature = "perf-alloc")]
            allocation_start: crate::perf_alloc::current_alloc_metrics(),
        })
    }

    fn observe_view(
        view: &Entity<GitCometView>,
        cx: &mut AsyncApp,
    ) -> (smol::channel::Receiver<()>, Vec<gpui::Subscription>) {
        let (tx, changed) = smol::channel::bounded(1);
        let observers = cx.update(|cx| {
            let (ui_model, main_pane, details_pane) = {
                let view = view.read(cx);
                (
                    view.ui_model.clone(),
                    view.main_pane.clone(),
                    view.details_pane.clone(),
                )
            };
            let history = main_pane.read(cx).history_view.clone();
            let notify = move |tx: &smol::channel::Sender<()>| {
                let _ = tx.try_send(());
            };
            vec![
                // Every state publication, so a witness never waits on a pane
                // happening to notify (the fallback is WITNESS_POLL).
                cx.observe(&ui_model, {
                    let tx = tx.clone();
                    move |_, _| notify(&tx)
                }),
                cx.observe(view, {
                    let tx = tx.clone();
                    move |_, _| notify(&tx)
                }),
                cx.observe(&main_pane, {
                    let tx = tx.clone();
                    move |_, _| notify(&tx)
                }),
                cx.observe(&details_pane, {
                    let tx = tx.clone();
                    move |_, _| notify(&tx)
                }),
                cx.observe(&history, move |_, _| notify(&tx)),
            ]
        });
        (changed, observers)
    }

    fn end_phase(&mut self) {
        if let Some(name) = self.phase.take() {
            let end = history_perf::snapshot();
            let counts: serde_json::Map<String, Value> = Work::ALL
                .iter()
                .enumerate()
                .map(|(index, work)| {
                    (
                        work.name().into(),
                        json!(end[index].saturating_sub(self.work_start[index])),
                    )
                })
                .collect();
            record("scenario_work", json!({"phase": name, "counts": counts}));
            if let Some((retained_bytes, pinned_bytes, retained_rows)) =
                history_perf::shared_memory()
            {
                record(
                    "scenario_history_memory",
                    json!({"phase": name,
                    "retained_bytes": retained_bytes, "caller_pinned_bytes": pinned_bytes,
                    "retained_rows": retained_rows}),
                );
            }
            #[cfg(feature = "perf-alloc")]
            {
                let metrics =
                    crate::perf_alloc::current_alloc_metrics().delta_since(self.allocation_start);
                let mut payload = serde_json::Map::new();
                payload.insert("phase".into(), json!(name));
                metrics.append_to_payload(&mut payload);
                record("scenario_allocations", Value::Object(payload));
            }
            record("scenario_phase", json!({"name": name, "state": "end"}));
        }
    }

    async fn sleep(&self, duration: Duration, cx: &mut AsyncApp) {
        if !duration.is_zero() {
            cx.background_executor().timer(duration).await;
        }
    }

    async fn step(&mut self, step: &Step, cx: &mut AsyncApp) -> Result<(), String> {
        match step {
            Step::WaitReady {
                timeout_ms,
                indexed,
            } => self.wait_ready(*timeout_ms, *indexed, cx).await,
            Step::Settle { ms } => {
                self.sleep(Duration::from_millis(*ms), cx).await;
                Ok(())
            }
            Step::Phase { name } => {
                self.end_phase();
                self.record_windows(cx)?;
                record("scenario_phase", json!({"name": name, "state": "begin"}));
                self.phase = Some(name.clone());
                self.work_start = history_perf::snapshot();
                #[cfg(feature = "perf-alloc")]
                {
                    self.allocation_start = crate::perf_alloc::current_alloc_metrics();
                }
                Ok(())
            }
            Step::Focus { target } => self.focus(*target, cx),
            Step::Keys {
                key,
                repeat,
                interval_ms,
                witness,
            } => {
                let keystroke = Keystroke::parse(key).map_err(|e| e.to_string())?;
                self.scheduled(
                    *repeat,
                    *interval_ms,
                    witness.clone(),
                    cx,
                    |_, window, cx| {
                        window.dispatch_keystroke(keystroke.clone(), cx);
                    },
                )
                .await
            }
            Step::Type {
                text,
                interval_ms,
                witness,
            } => {
                let chars: Vec<char> = text.chars().collect();
                let keystrokes: Vec<Keystroke> =
                    chars.iter().map(|&ch| text_keystroke(ch)).collect();
                self.scheduled(
                    chars.len(),
                    *interval_ms,
                    witness.clone(),
                    cx,
                    |ix, window, cx| {
                        window.dispatch_keystroke(keystrokes[ix].clone(), cx);
                    },
                )
                .await
            }
            Step::Paste { text, witness } => {
                cx.update(|cx| {
                    self.view.update(cx, |_, cx| {
                        crate::clipboard::write_text(
                            cx,
                            text.clone(),
                            crate::clipboard::CopySource::TextInputShortcut,
                        )
                    })
                });
                // Native clipboard ownership is asynchronous (Wayland needs
                // a compositor offer). Finish setup before timing the shortcut.
                let deadline = Instant::now() + Duration::from_secs(5);
                while !cx.update(|cx| {
                    self.view.update(cx, |_, cx| {
                        crate::clipboard::read_text(cx).as_ref() == Some(text)
                    })
                }) {
                    if Instant::now() >= deadline {
                        return Err("clipboard did not accept scenario text; check native input focus/ownership".into());
                    }
                    self.sleep(Duration::from_millis(10), cx).await;
                }
                let paste = Keystroke::parse("secondary-v").map_err(|e| e.to_string())?;
                self.scheduled(1, 0, witness.clone(), cx, |_, window, cx| {
                    window.dispatch_keystroke(paste.clone(), cx);
                })
                .await
            }
            Step::ExpectRepoPicker {
                query,
                matches,
                selected,
            } => {
                let actual = cx.update(|cx| {
                    self.view
                        .read(cx)
                        .popover_host
                        .read(cx)
                        .scenario_repo_picker_state(cx)
                });
                let expected = Some((query.clone(), Some(*matches), *selected));
                if actual != expected {
                    return Err(format!(
                        "repository picker: expected {expected:?}, got {actual:?}"
                    ));
                }
                record(
                    "scenario_assertion",
                    json!({"kind": "repo_picker", "query": query, "matches": matches, "selected": selected}),
                );
                Ok(())
            }
            Step::Scroll {
                target,
                delta_px,
                repeat,
                interval_ms,
                flip_every,
                witness,
            } => {
                let position = self
                    .scroll_position_for(*target, cx)
                    .ok_or("no scroll target bounds")?;
                self.scheduled(
                    *repeat,
                    *interval_ms,
                    witness.clone(),
                    cx,
                    |ix, window, cx| {
                        let flipped =
                            flip_every.is_some_and(|every| every > 0 && (ix / every) % 2 == 1);
                        let delta = if flipped { -delta_px } else { *delta_px };
                        window.dispatch_event(
                            PlatformInput::ScrollWheel(ScrollWheelEvent {
                                position,
                                delta: ScrollDelta::Pixels(point(px(0.0), px(delta))),
                                modifiers: Modifiers::default(),
                                touch_phase: TouchPhase::Moved,
                            }),
                            cx,
                        );
                    },
                )
                .await
            }
            Step::Click { target, witness } => {
                let position = self
                    .click_position_for(*target, cx)
                    .ok_or("no click target bounds")?;
                self.scheduled(1, 0, witness.clone(), cx, |_, window, cx| {
                    let down = MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    };
                    window.dispatch_event(PlatformInput::MouseDown(down), cx);
                    window.dispatch_event(
                        PlatformInput::MouseUp(MouseUpEvent {
                            button: MouseButton::Left,
                            position,
                            modifiers: Modifiers::default(),
                            click_count: 1,
                        }),
                        cx,
                    );
                })
                .await
            }
            Step::WriteFiles {
                paths,
                contents,
                rounds,
                interval_ms,
                expect_status,
            } => {
                self.write_files(paths, contents, *rounds, *interval_ms, *expect_status, cx)
                    .await
            }
            Step::Command { id, witness } => {
                let view = self.view.clone();
                let id = id.clone();
                let Some(witness) = witness else {
                    return self
                        .window
                        .update(cx, move |_, window, cx| {
                            view.update(cx, |view, cx| view.execute_command(&id, Some(window), cx));
                        })
                        .map_err(|e| e.to_string());
                };
                let witness = match witness {
                    WitnessKind::RepoClosed { path } => WitnessKind::RepoClosed {
                        path: std::fs::canonicalize(path)
                            .map_err(|e| format!("{}: {e}", path.display()))?,
                    },
                    other => other.clone(),
                };
                self.scheduled(1, 0, Some(witness), cx, move |_, window, cx| {
                    view.update(cx, |view, cx| view.execute_command(&id, Some(window), cx));
                })
                .await
            }
            Step::NewWindow { name } => self.new_window(name, cx).await,
            Step::SwitchWindow { name } => self.switch_window(name, cx).await,
            Step::CloseWindow { name } => self.close_window(name, cx).await,
            Step::ExpectWindows { count } => {
                let actual = self.record_windows(cx)?;
                if actual != *count {
                    return Err(format!("expected {count} main windows, found {actual}"));
                }
                Ok(())
            }
            Step::OpenRepo { path, wait } => {
                let path =
                    std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
                let store = cx.update(|cx| Arc::clone(&self.view.read(cx).store));
                let witness = WitnessKind::RepoOpen { path: path.clone() };
                self.scheduled(1, 0, wait.then_some(witness), cx, move |_, _, _| {
                    store.dispatch(Msg::OpenRepo(path.clone()));
                })
                .await
            }
            Step::Minimize => self
                .window
                .update(cx, |_, window, _| window.minimize_window())
                .map_err(|e| e.to_string()),
            Step::MovePointer {
                positions,
                repeat,
                interval_ms,
                witness,
            } => {
                if positions.is_empty()
                    || *repeat == 0
                    || positions
                        .iter()
                        .flatten()
                        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                {
                    return Err(
                        "pointer positions must be viewport fractions; repeat must be positive"
                            .into(),
                    );
                }
                let bounds = cx
                    .update(|cx| {
                        self.view
                            .read(cx)
                            .main_pane
                            .read(cx)
                            .history_view
                            .read(cx)
                            .history_viewport_bounds()
                    })
                    .ok_or("history has no viewport")?;
                let points: Vec<_> = positions
                    .iter()
                    .map(|p| {
                        point(
                            bounds.left() + bounds.size.width * p[0],
                            bounds.top() + bounds.size.height * p[1],
                        )
                    })
                    .collect();
                self.pointer_expected = Some(points[(*repeat - 1) % points.len()]);
                self.scheduled(
                    *repeat,
                    *interval_ms,
                    witness.clone(),
                    cx,
                    |ix, window, cx| {
                        window.dispatch_event(
                            PlatformInput::MouseMove(MouseMoveEvent {
                                position: points[ix % points.len()],
                                pressed_button: None,
                                modifiers: Modifiers::default(),
                            }),
                            cx,
                        );
                    },
                )
                .await
            }
            Step::DragHistory {
                to_fraction,
                steps,
                interval_ms,
                wait_for_rows,
            } => {
                if !(0.01..=0.99).contains(to_fraction) || *steps == 0 {
                    return Err("drag needs positive steps and destination in 0.01..=0.99".into());
                }
                let (start, end) = cx
                    .update(|cx| {
                        self.view
                            .read(cx)
                            .main_pane
                            .read(cx)
                            .history_view
                            .read(cx)
                            .scenario_drag_points(*to_fraction)
                    })
                    .ok_or("history has no viewport")?;
                self.window
                    .update(cx, |_, window, cx| {
                        window.dispatch_event(
                            PlatformInput::MouseDown(MouseDownEvent {
                                button: MouseButton::Left,
                                position: start,
                                modifiers: Modifiers::default(),
                                click_count: 1,
                                first_mouse: false,
                            }),
                            cx,
                        )
                    })
                    .map_err(|e| e.to_string())?;
                let result = self
                    .scheduled(
                        *steps,
                        *interval_ms,
                        Some(if *wait_for_rows {
                            WitnessKind::HistoryJumpLoaded
                        } else {
                            WitnessKind::HistoryScrolled
                        }),
                        cx,
                        |ix, window, cx| {
                            let progress = (ix + 1) as f32 / *steps as f32;
                            window.dispatch_event(
                                PlatformInput::MouseMove(MouseMoveEvent {
                                    position: point(
                                        start.x,
                                        start.y + (end.y - start.y) * progress,
                                    ),
                                    pressed_button: Some(MouseButton::Left),
                                    modifiers: Modifiers::default(),
                                }),
                                cx,
                            );
                        },
                    )
                    .await;
                self.window
                    .update(cx, |_, window, cx| {
                        window.dispatch_event(
                            PlatformInput::MouseUp(MouseUpEvent {
                                button: MouseButton::Left,
                                position: end,
                                modifiers: Modifiers::default(),
                                click_count: 1,
                            }),
                            cx,
                        )
                    })
                    .map_err(|e| e.to_string())?;
                result
            }
            Step::StartOperation {
                operation,
                path,
                remote,
                url,
                dest,
            } => {
                self.start_operation(operation, path, remote, url, dest, cx)
                    .await
            }
            Step::WaitOperation {
                timeout_ms,
                cancelled,
            } => self.wait_operation(*timeout_ms, *cancelled, cx).await,
            Step::CancelOperation => self.cancel_operation(cx),
            Step::MinimizeActivity => self.minimize_activity(cx).await,
        }
    }

    async fn wait_ready(
        &mut self,
        timeout_ms: u64,
        indexed: bool,
        cx: &mut AsyncApp,
    ) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let ready = cx.update(|cx| {
                let view = self.view.read(cx);
                let Some(repo) = view.active_repo() else {
                    return false;
                };
                let history = view.main_pane.read(cx).history_view.read(cx);
                matches!(repo.open, Loadable::Ready(()))
                    && matches!(repo.status, Loadable::Ready(_))
                    && matches!(repo.history_state.log, Loadable::Ready(_))
                    && history.history_viewport_bounds().is_some()
                    && (!indexed
                        || history
                            .scenario_indexed_window()
                            .is_some_and(|(_, _, _, loaded)| loaded))
            });
            if ready {
                let geometry = cx.update(|cx| {
                    let view = self.view.read(cx);
                    let history = view.main_pane.read(cx).history_view.read(cx);
                    let bounds = history.history_viewport_bounds().expect("ready viewport");
                    let commits =
                        view.active_repo()
                            .and_then(|repo| match &repo.history_state.log {
                                Loadable::Ready(page) => Some(page.commits.len()),
                                _ => None,
                            });
                    json!({"loaded_commits": commits, "indexed_window": history.scenario_indexed_window(), "viewport": [
                        f32::from(bounds.left()), f32::from(bounds.top()),
                        f32::from(bounds.size.width), f32::from(bounds.size.height)]})
                });
                record("scenario_ready", geometry);
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "repository not ready within {timeout_ms} ms (require indexed rows: {indexed})"
                ));
            }
            self.sleep(Duration::from_millis(50), cx).await;
        }
    }

    fn focus(&self, target: FocusTarget, cx: &mut AsyncApp) -> Result<(), String> {
        let view = self.view.clone();
        self.window
            .update(cx, move |_, window, cx| match target {
                FocusTarget::History => {
                    let history = view.read(cx).main_pane.read(cx).history_view.clone();
                    let handle = history.read(cx).history_panel_focus_handle.clone();
                    window.focus(&handle, cx);
                }
            })
            .map_err(|e| e.to_string())
    }

    fn scroll_position_for(
        &self,
        target: ScrollTarget,
        cx: &mut AsyncApp,
    ) -> Option<Point<Pixels>> {
        cx.update(|cx| match target {
            ScrollTarget::History => {
                let history = self.view.read(cx).main_pane.read(cx).history_view.read(cx);
                history
                    .history_viewport_bounds()
                    .map(|bounds| bounds.center())
            }
        })
    }

    fn click_position_for(&self, target: ClickTarget, cx: &mut AsyncApp) -> Option<Point<Pixels>> {
        cx.update(|cx| {
            let view = self.view.read(cx);
            match target {
                ClickTarget::UnstagedRow { index } => {
                    let details = view.details_pane.read(cx);
                    let bounds = details.unstaged_scroll.0.borrow().base_handle.bounds();
                    let offset = details.unstaged_scroll.0.borrow().base_handle.offset();
                    // Outside a window update `UiScale::current` is the default,
                    // not this window's zoom.
                    let row = details.ui_scale().row_height(24.0, 32.0);
                    (bounds.size.height > px(0.0)).then(|| {
                        point(
                            bounds.left() + px(60.0),
                            bounds.top() + offset.y + row * (index as f32 + 0.5),
                        )
                    })
                }
            }
        })
    }

    fn baseline_for(&self, kind: &WitnessKind, cx: &mut AsyncApp) -> Baseline {
        cx.update(|cx| {
            let view = self.view.read(cx);
            match kind {
                WitnessKind::CommitDetails => Baseline::Selection(
                    view.active_repo()
                        .and_then(|repo| repo.history_state.selected_commit.clone()),
                ),
                WitnessKind::HistoryScrolled | WitnessKind::HistoryJumpLoaded => {
                    Baseline::ScrollPosition(
                        view.main_pane
                            .read(cx)
                            .history_view
                            .read(cx)
                            .history_scroll_position(),
                    )
                }
                WitnessKind::SearchSettled => {
                    Baseline::Query(view.main_pane.read(cx).diff_search_query.clone())
                }
                WitnessKind::RepoPickerFiltered => view
                    .popover_host
                    .read(cx)
                    .scenario_repo_picker_state(cx)
                    .map(|(query, _, _)| Baseline::Query(query.into()))
                    .unwrap_or(Baseline::None),
                WitnessKind::DiffLoaded
                | WitnessKind::RepoOpen { .. }
                | WitnessKind::RepoClosed { .. }
                | WitnessKind::MessageHover => Baseline::None,
            }
        })
    }

    /// Whether `kind` holds for an input dispatched against `baseline`.
    /// `target` pins the commit a selection resolved to once it is seen.
    fn witness_holds(
        &self,
        kind: &WitnessKind,
        baseline: &Baseline,
        target: &mut Option<CommitId>,
        cx: &mut AsyncApp,
    ) -> bool {
        cx.update(|cx| {
            let view = self.view.read(cx);
            if let WitnessKind::RepoClosed { path } = kind {
                return !view
                    .state
                    .repos
                    .iter()
                    .any(|repo| repo.spec.workdir == *path);
            }
            let Some(repo) = view.active_repo() else {
                return false;
            };
            match kind {
                WitnessKind::RepoPickerFiltered => view
                    .popover_host
                    .read(cx)
                    .scenario_repo_picker_state(cx)
                    .is_some_and(|(query, count, _)| {
                        count.is_some()
                            && match baseline {
                                Baseline::Query(before) => query.as_str() != before.as_ref(),
                                _ => true,
                            }
                    }),
                WitnessKind::MessageHover => self.pointer_expected.is_some_and(|position| {
                    view.commit_message_hover_host
                        .read(cx)
                        .scenario_contains_source(position)
                }),
                WitnessKind::CommitDetails => {
                    let selected = repo.history_state.selected_commit.clone();
                    if target.is_none() {
                        let moved = match baseline {
                            Baseline::Selection(before) => selected != *before,
                            _ => selected.is_some(),
                        };
                        if !moved {
                            return false;
                        }
                        *target = selected.clone();
                    }
                    selected == *target
                        && matches!(&repo.history_state.commit_details,
                            Loadable::Ready(details) if Some(&details.id) == target.as_ref())
                }
                WitnessKind::RepoClosed { .. } => unreachable!("answered above"),
                WitnessKind::RepoOpen { path } => {
                    repo.spec.workdir == *path
                        && matches!(repo.open, Loadable::Ready(()))
                        && matches!(repo.status, Loadable::Ready(_))
                        && matches!(repo.history_state.log, Loadable::Ready(_))
                }
                WitnessKind::DiffLoaded => {
                    let diff = &repo.diff_state;
                    diff.diff_target.is_some()
                        && !matches!(diff.diff, Loadable::Loading)
                        && !matches!(diff.diff_file, Loadable::Loading)
                        && !diff.diff_reload_in_flight
                }
                WitnessKind::HistoryScrolled | WitnessKind::HistoryJumpLoaded => {
                    if matches!(kind, WitnessKind::HistoryJumpLoaded)
                        && !view
                            .main_pane
                            .read(cx)
                            .history_view
                            .read(cx)
                            .scenario_indexed_window()
                            .is_some_and(|(_, _, _, loaded)| loaded)
                    {
                        return false;
                    }
                    let position = view
                        .main_pane
                        .read(cx)
                        .history_view
                        .read(cx)
                        .history_scroll_position();
                    match baseline {
                        Baseline::ScrollPosition(before) => position != *before,
                        _ => true,
                    }
                }
                WitnessKind::SearchSettled => {
                    let main = view.main_pane.read(cx);
                    let changed = match baseline {
                        Baseline::Query(before) => main.diff_search_query != *before,
                        _ => true,
                    };
                    changed && main.diff_search_active && !main.diff_search_result_pending()
                }
            }
        })
    }

    /// Dispatches `repeat` inputs at fixed times `interval_ms` apart, each as
    /// its own traced operation, and waits for their witnesses.
    async fn scheduled(
        &mut self,
        repeat: usize,
        interval_ms: u64,
        witness: Option<WitnessKind>,
        cx: &mut AsyncApp,
        mut input: impl FnMut(usize, &mut Window, &mut App),
    ) -> Result<(), String> {
        let started = Instant::now();
        let interval = Duration::from_millis(interval_ms);
        let mut pending: Vec<Pending> = Vec::new();
        for ix in 0..repeat {
            let scheduled = started + interval * u32::try_from(ix).unwrap_or(u32::MAX);
            // Resolve witnesses until the next input is due.
            loop {
                let _ = self.operation_status(cx);
                self.resolve(&mut pending, cx);
                let now = Instant::now();
                if now >= scheduled {
                    break;
                }
                let wait = scheduled - now;
                let wait = if pending.is_empty() {
                    wait
                } else {
                    wait.min(WITNESS_POLL)
                };
                self.wait_for_change(wait, cx).await;
            }
            let op = op_trace::next_op();
            let baseline = witness
                .as_ref()
                .map_or(Baseline::None, |kind| self.baseline_for(kind, cx));
            let expects = u64::from(witness.is_some());
            let dispatched = self.window.update(cx, |_, window, cx| {
                record(
                    "scenario_input",
                    json!({"op": op, "window": format!("{:?}", window.window_handle().window_id()),
                    "intentional_dwell": matches!(witness, Some(WitnessKind::MessageHover))}),
                );
                // `b` says whether a witness will follow.
                op_trace::record(
                    Stage::Input,
                    op,
                    "scenario",
                    op_trace::instant_ns(scheduled),
                    expects,
                );
                let handling = Instant::now();
                {
                    let _scope = op_trace::scope(op);
                    input(ix, window, cx);
                }
                op_trace::record(
                    Stage::InputHandled,
                    op,
                    "scenario",
                    op_trace::duration_ns(handling.elapsed()),
                    0,
                );
            });
            dispatched.map_err(|e| format!("window closed: {e}"))?;
            if let Some(kind) = witness.clone() {
                pending.push(Pending {
                    op,
                    kind,
                    baseline,
                    target: None,
                });
            }
        }
        let deadline = Instant::now() + DEFAULT_WITNESS_TIMEOUT;
        while !pending.is_empty() {
            let _ = self.operation_status(cx);
            self.resolve(&mut pending, cx);
            if pending.is_empty() {
                break;
            }
            let now = Instant::now();
            if now > deadline {
                return Err(format!(
                    "{} input(s) never reached their witness ({}); state: {}",
                    pending.len(),
                    pending[0].kind.name(),
                    self.describe(&pending[0].kind, cx)
                ));
            }
            self.wait_for_change((deadline - now).min(WITNESS_POLL), cx)
                .await;
        }
        Ok(())
    }

    /// Marks the newest satisfied witness complete and every older pending
    /// input superseded: later input replaced its result before it showed.
    fn resolve(&self, pending: &mut Vec<Pending>, cx: &mut AsyncApp) {
        let mut satisfied = None;
        for ix in (0..pending.len()).rev() {
            let mut target = pending[ix].target.clone();
            let kind = pending[ix].kind.clone();
            let holds = self.witness_holds(&kind, &pending[ix].baseline, &mut target, cx);
            pending[ix].target = target;
            if holds {
                satisfied = Some(ix);
                break;
            }
        }
        let Some(satisfied) = satisfied else {
            return;
        };
        for (ix, item) in pending.drain(..=satisfied).enumerate() {
            let complete = ix == satisfied;
            let label = if complete {
                item.kind.name()
            } else {
                "superseded"
            };
            op_trace::record(Stage::Witness, item.op, label, u64::from(complete), 0);
        }
    }

    /// The state a witness reads, for failure reports.
    fn describe(&self, kind: &WitnessKind, cx: &mut AsyncApp) -> String {
        cx.update(|cx| {
            let view = self.view.read(cx);
            if let WitnessKind::RepoClosed { path } = kind {
                return format!(
                    "wanted closed={} open={}",
                    path.display(),
                    view.state.repos.len()
                );
            }
            let Some(repo) = view.active_repo() else {
                return "no active repository".to_owned();
            };
            match kind {
                WitnessKind::RepoPickerFiltered => format!(
                    "picker={:?}",
                    view.popover_host.read(cx).scenario_repo_picker_state(cx)
                ),
                WitnessKind::MessageHover => format!("pointer={:?}", self.pointer_expected),
                WitnessKind::CommitDetails => format!(
                    "selected={:?} details_ready={}",
                    repo.history_state.selected_commit,
                    matches!(repo.history_state.commit_details, Loadable::Ready(_))
                ),
                WitnessKind::RepoClosed { .. } => unreachable!("answered above"),
                WitnessKind::RepoOpen { path } => format!(
                    "wanted={} active={} open={:?}",
                    path.display(),
                    repo.spec.workdir.display(),
                    matches!(repo.open, Loadable::Ready(()))
                ),
                WitnessKind::DiffLoaded => format!(
                    "target={:?} reload_in_flight={}",
                    repo.diff_state.diff_target, repo.diff_state.diff_reload_in_flight
                ),
                WitnessKind::HistoryScrolled | WitnessKind::HistoryJumpLoaded => format!(
                    "position={}",
                    view.main_pane
                        .read(cx)
                        .history_view
                        .read(cx)
                        .history_scroll_position()
                ),
                WitnessKind::SearchSettled => {
                    let main = view.main_pane.read(cx);
                    format!(
                        "active={} query={:?} running={} pending={:?} matches={}",
                        main.diff_search_active,
                        main.diff_search_query,
                        main.diff_search_worker_running,
                        main.diff_search_pending_previous_query,
                        main.diff_search_matches.len()
                    )
                }
            }
        })
    }

    async fn wait_for_change(&self, limit: Duration, cx: &mut AsyncApp) {
        let changed = self.changed.clone();
        let timer = cx.background_executor().timer(limit);
        smol::future::or(
            async move {
                let _ = changed.recv().await;
            },
            timer,
        )
        .await;
    }

    async fn write_files(
        &mut self,
        paths: &[PathBuf],
        contents: &str,
        rounds: usize,
        interval_ms: u64,
        expect_status: bool,
        cx: &mut AsyncApp,
    ) -> Result<(), String> {
        let workdir = cx
            .update(|cx| self.view.read(cx).active_repo_workdir())
            .ok_or("no active repository")?;
        // `None` marks a path that did not exist: restoring deletes it.
        let files: Vec<_> = paths.iter().map(|path| workdir.join(path)).collect();
        let originals = Arc::new(
            cx.background_executor()
                .spawn(async move {
                    files
                        .into_iter()
                        .map(|full| {
                            let original = match std::fs::read(&full) {
                                Ok(bytes) => Some(bytes),
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                                Err(error) => return Err(format!("{}: {error}", full.display())),
                            };
                            Ok((full, original))
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .await?,
        );
        let contents: Arc<str> = contents.into();
        let started = Instant::now();
        let interval = Duration::from_millis(interval_ms);
        let mut result = Ok(());
        for ix in 0..rounds {
            let scheduled = started + interval * u32::try_from(ix).unwrap_or(u32::MAX);
            let now = Instant::now();
            if scheduled > now {
                self.sleep(scheduled - now, cx).await;
            }
            let dirty = ix % 2 == 0;
            let op = op_trace::next_op();
            op_trace::record(
                Stage::Input,
                op,
                "file_write",
                op_trace::instant_ns(scheduled),
                u64::from(expect_status),
            );
            let handling = Instant::now();
            // These represent an external editor/build. Harness filesystem
            // work must not manufacture a stall on the application's UI thread.
            let files = Arc::clone(&originals);
            let contents = Arc::clone(&contents);
            result = cx
                .background_executor()
                .spawn(
                    async move { write_fixture_files(&files, dirty.then_some(contents.as_ref())) },
                )
                .await;
            op_trace::record(
                Stage::InputHandled,
                op,
                "file_write",
                op_trace::duration_ns(handling.elapsed()),
                1,
            );
            if result.is_err() {
                break;
            }
            if !expect_status {
                continue;
            }
            // The watcher, not this op, drives the refresh: wait for the
            // status list to reflect the round before the next one.
            let deadline = Instant::now() + DEFAULT_WITNESS_TIMEOUT;
            loop {
                let reflected = cx.update(|cx| {
                    // The lane the status list renders, not the combined status.
                    self.view.read(cx).active_repo().is_some_and(|repo| {
                        let Some(entries) = repo.worktree_status_entries() else {
                            return false;
                        };
                        paths.iter().all(|path| {
                            let listed = entries
                                .iter()
                                .any(|entry| entry.path.as_path() == path.as_path());
                            listed == dirty
                        })
                    })
                });
                if reflected {
                    op_trace::record(Stage::Witness, op, "status_reflects_save", 1, 0);
                    break;
                }
                let now = Instant::now();
                if now > deadline {
                    result = Err(format!("round {ix} never reached the status list"));
                    break;
                }
                self.wait_for_change((deadline - now).min(WITNESS_POLL), cx)
                    .await;
            }
            if result.is_err() {
                break;
            }
        }
        // Leave the tree as it was, whatever happened.
        let restored = cx
            .background_executor()
            .spawn(async move { write_fixture_files(&originals, None) })
            .await;
        result.and(restored)
    }
}

fn write_fixture_files(
    originals: &[(PathBuf, Option<Vec<u8>>)],
    contents: Option<&str>,
) -> Result<(), String> {
    let mut result = Ok(());
    for (full, original) in originals {
        let written = match (contents, original) {
            (Some(contents), _) => full
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(full, contents)),
            (None, Some(original)) => std::fs::write(full, original),
            (None, None) => match std::fs::remove_file(full) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                result => result,
            },
        };
        if let Err(error) = written {
            result = Err(format!("{}: {error}", full.display()));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every step shape scripts/profiling/live-ui.py writes must parse; a
    /// rename on either side would otherwise fail only in a live run.
    #[test]
    fn harness_step_shapes_parse() {
        let scenario: Scenario = serde_json::from_value(json!({
            "steps": [
                {"do": "wait_ready", "timeout_ms": 180000, "indexed": true},
                {"do": "settle", "ms": 3000},
                {"do": "phase", "name": "select"},
                {"do": "focus", "target": "history"},
                {"do": "keys", "key": "down", "repeat": 240, "interval_ms": 120,
                 "witness": {"kind": "commit_details"}},
                {"do": "keys", "key": "secondary-f", "repeat": 1, "interval_ms": 0},
                {"do": "type", "text": "needle", "interval_ms": 150,
                 "witness": {"kind": "search_settled"}},
                {"do": "scroll", "target": "history", "delta_px": -96, "repeat": 1200,
                 "interval_ms": 16, "flip_every": 150, "witness": {"kind": "history_scrolled"}},
                {"do": "click", "target": {"list": "unstaged_row", "index": 0},
                 "witness": {"kind": "diff_loaded"}},
                {"do": "write_files", "paths": ["src/a.txt", "src/b.txt"], "contents": "x\n",
                 "rounds": 10, "interval_ms": 2000},
                {"do": "write_files", "paths": ["target/churn-1.txt"], "contents": "x\n",
                 "rounds": 60, "interval_ms": 500, "expect_status": false},
                {"do": "command", "id": "toggle-terminal"},
                {"do": "command", "id": "close-repo-tab",
                 "witness": {"kind": "repo_closed", "path": "/tmp"}},
                {"do": "open_repo", "path": "/tmp"},
                {"do": "minimize"},
                {"do": "paste", "text": "component-017", "witness": {"kind": "repo_picker_filtered"}},
                {"do": "expect_repo_picker", "query": "component-017", "matches": 1, "selected": 0}
            ]
        }))
        .expect("parse scenario");
        assert_eq!(scenario.steps.len(), 17);
        assert!(matches!(
            scenario.steps[4],
            Step::Keys {
                repeat: 240,
                witness: Some(WitnessKind::CommitDetails),
                ..
            }
        ));
    }

    #[test]
    fn typed_text_keeps_dashes_and_case() {
        let dash = text_keystroke('-');
        assert_eq!(
            (dash.key.as_str(), dash.key_char.as_deref()),
            ("-", Some("-"))
        );
        let upper = text_keystroke('G');
        assert!(upper.modifiers.shift);
        assert_eq!(upper.key_char.as_deref(), Some("G"));
        assert_eq!(text_keystroke('\n').key, "enter");
    }

    #[test]
    fn unknown_steps_are_rejected_rather_than_skipped() {
        let parsed = serde_json::from_value::<Scenario>(json!({
            "steps": [{"do": "wait_redy"}]
        }));
        assert!(parsed.is_err());
    }

    #[test]
    fn performance_operation_and_pointer_steps_parse() {
        let scenario: Scenario = serde_json::from_value(json!({"steps": [
            {"do": "move_pointer", "positions": [[0.55, 0.08]], "repeat": 1,
             "interval_ms": 0, "witness": {"kind": "message_hover"}},
            {"do": "drag_history", "to_fraction": 0.8, "steps": 90, "interval_ms": 16},
            {"do": "start_operation", "operation": "clone", "url": "http://127.0.0.1/remote.git",
             "dest": "/tmp/disposable-clone"},
            {"do": "cancel_operation"},
            {"do": "minimize_activity"},
            {"do": "wait_operation", "timeout_ms": 15000, "cancelled": true}
        ]}))
        .expect("parse performance steps");
        assert_eq!(scenario.steps.len(), 6);
    }

    #[test]
    fn multi_window_steps_keep_repository_waits_explicit() {
        let scenario: Scenario = serde_json::from_value(json!({"steps": [
            {"do": "new_window", "name": "other"},
            {"do": "open_repo", "path": "/tmp/repo", "wait": false},
            {"do": "switch_window", "name": "primary"},
            {"do": "expect_windows", "count": 2},
            {"do": "close_window", "name": "other"},
            {"do": "open_repo", "path": "/tmp/repo"}
        ]}))
        .unwrap();
        assert!(matches!(
            scenario.steps[1],
            Step::OpenRepo { wait: false, .. }
        ));
        assert!(matches!(
            scenario.steps[5],
            Step::OpenRepo { wait: true, .. }
        ));
    }
}
