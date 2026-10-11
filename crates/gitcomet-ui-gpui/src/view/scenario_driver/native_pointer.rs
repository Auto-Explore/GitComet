//! File rendezvous with the Windows SendInput replay worker; synthetic dispatch remains explicit.
use super::*;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::rc::Rc;

const REQUEST_ENV: &str = "GITCOMET_UI_NATIVE_INPUT_JSONL";
const ACK_ENV: &str = "GITCOMET_UI_NATIVE_INPUT_ACK_JSONL";
// WM_MOUSEMOVE's extra information preserves only the low 32 bits on Windows.
// The upper 16 of those bits carry the tag; a run must not reuse a sequence.
const MAX_NATIVE_SEQUENCE: u64 = u16::MAX as u64;

#[derive(Debug, Deserialize)]
struct NativeAck {
    sequence: u64,
    success: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    qpc_before: Option<u64>,
    #[serde(default)]
    qpc_after: Option<u64>,
    #[serde(default)]
    qpc_frequency: Option<u64>,
    #[serde(default)]
    dispatch_unix_ms: Option<f64>,
}

struct NativeReplay {
    requests: File,
    ack_path: PathBuf,
    acknowledgements: Option<File>,
    partial: String,
    expected: BTreeSet<u64>,
    received: BTreeSet<u64>,
}

impl NativeReplay {
    fn open() -> Result<Self, String> {
        if !cfg!(target_os = "windows") {
            return Err("native pointer replay requires Windows".into());
        }
        let path = |name| {
            std::env::var_os(name)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| format!("native pointer replay requires {name}"))
        };
        let requests = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path(REQUEST_ENV)?)
            .map_err(|error| format!("open native pointer request file: {error}"))?;
        Ok(Self {
            requests,
            ack_path: path(ACK_ENV)?,
            acknowledgements: None,
            partial: String::new(),
            expected: BTreeSet::new(),
            received: BTreeSet::new(),
        })
    }

    fn request(
        &mut self,
        sequence: u64,
        position: Point<Pixels>,
        scheduled: Instant,
        window: &Window,
    ) -> Result<(), String> {
        if sequence == 0 || sequence > MAX_NATIVE_SEQUENCE {
            return Err("native pointer sequence must be 1..=65535 without reuse per run".into());
        }
        if self.expected.contains(&sequence) {
            return Err(format!("native pointer sequence {sequence} was reused"));
        }
        let (hwnd, x_px, y_px) = window
            .native_pointer_target(position)
            .ok_or("native pointer target is unavailable for this window")?;
        let scheduled_at_ms = op_trace::instant_ns(scheduled) as f64 / 1e6;
        let delay_ms = scheduled.elapsed().as_secs_f64() * 1000.0;
        let scheduled_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs_f64()
            * 1000.0
            - delay_ms;
        let request = json!({"sequence": sequence,
            "window": format!("{:?}", window.window_handle().window_id()),
            "hwnd": hwnd, "x_px": x_px, "y_px": y_px,
            "scheduled_at_ms": scheduled_at_ms, "scheduled_unix_ms": scheduled_unix_ms});
        serde_json::to_writer(&mut self.requests, &request).map_err(|error| error.to_string())?;
        self.requests
            .write_all(b"\n")
            .and_then(|_| self.requests.flush())
            .map_err(|error| error.to_string())?;
        self.expected.insert(sequence);
        Ok(())
    }

    fn poll(&mut self) -> Result<bool, String> {
        if self.acknowledgements.is_none() {
            match File::open(&self.ack_path) {
                Ok(file) => self.acknowledgements = Some(file),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => {
                    return Err(format!("open native pointer acknowledgement file: {error}"));
                }
            }
        }
        let mut bytes = String::new();
        self.acknowledgements
            .as_mut()
            .expect("opened acknowledgement file")
            .take(64 * 1024)
            .read_to_string(&mut bytes)
            .map_err(|error| error.to_string())?;
        self.partial.push_str(&bytes);
        while let Some(newline) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=newline).collect();
            let ack: NativeAck = serde_json::from_str(line.trim())
                .map_err(|error| format!("invalid native pointer acknowledgement: {error}"))?;
            if !self.expected.contains(&ack.sequence) || !self.received.insert(ack.sequence) {
                continue;
            }
            record(
                "native_input_ack",
                json!({"sequence": ack.sequence, "success": ack.success,
                "qpc_before": ack.qpc_before, "qpc_after": ack.qpc_after,
                "qpc_frequency": ack.qpc_frequency, "dispatch_unix_ms": ack.dispatch_unix_ms,
                "error": ack.error}),
            );
            if !ack.success {
                return Err(format!(
                    "native pointer input {} failed: {}",
                    ack.sequence,
                    ack.error
                        .unwrap_or_else(|| "SendInput worker rejected the input".into())
                ));
            }
        }
        if self.partial.len() > 8192 {
            return Err("native pointer acknowledgement line exceeded its bound".into());
        }
        Ok(self.expected == self.received)
    }
}

impl Driver {
    pub(super) async fn move_pointer(
        &mut self,
        points: Vec<Point<Pixels>>,
        repeat: usize,
        interval_ms: u64,
        hover_rows: bool,
        witness: Option<WitnessKind>,
        cx: &mut AsyncApp,
    ) -> Result<(), String> {
        let route = std::env::var("GITCOMET_UI_INPUT_ROUTE").unwrap_or_else(|_| "synthetic".into());
        let native = match route.as_str() {
            "synthetic" => None,
            "native" => Some(Rc::new(RefCell::new(NativeReplay::open()?))),
            _ => return Err(format!("unknown GITCOMET_UI_INPUT_ROUTE={route:?}")),
        };
        record(
            "scenario_input_route",
            json!({"route": route, "kind": "mouse_move", "hover_rows": hover_rows}),
        );
        let native_for_input = native.clone();
        let error = Rc::new(RefCell::new(None));
        let input_error = error.clone();
        let started = Instant::now();
        let interval = Duration::from_millis(interval_ms);
        self.scheduled(repeat, interval_ms, witness, cx, move |ix, window, cx| {
            if input_error.borrow().is_some() {
                return;
            }
            let sequence = op_trace::Stamp::capture().map_or(0, |stamp| stamp.op);
            let position = points[ix % points.len()];
            let source = if native_for_input.is_some() {
                "native"
            } else {
                "synthetic"
            };
            let result = (if hover_rows {
                hover::register(window.window_handle().window_id(), sequence, source)
            } else {
                Ok(())
            })
            .and_then(|_| {
                if let Some(native) = &native_for_input {
                    native.borrow_mut().request(
                        sequence,
                        position,
                        started + interval * u32::try_from(ix).unwrap_or(u32::MAX),
                        window,
                    )
                } else {
                    window.dispatch_event(
                        PlatformInput::MouseMove(MouseMoveEvent {
                            position,
                            pressed_button: None,
                            modifiers: Modifiers::default(),
                        }),
                        cx,
                    );
                    Ok(())
                }
            });
            if let Err(message) = result {
                *input_error.borrow_mut() = Some(message);
            }
        })
        .await?;
        if let Some(error) = error.borrow_mut().take() {
            return Err(error);
        }
        let deadline = Instant::now() + DEFAULT_WITNESS_TIMEOUT;
        loop {
            let acknowledged = match &native {
                Some(native) => native.borrow_mut().poll()?,
                None => true,
            };
            if acknowledged && (!hover_rows || hover::settled(self.window.window_id())) {
                break;
            }
            if Instant::now() >= deadline {
                return Err(
                    "final pointer input did not reach its actual hover paint/unchanged witness"
                        .into(),
                );
            }
            self.sleep(Duration::from_millis(2), cx).await;
        }
        if hover_rows {
            record(
                "hover_final_witness",
                json!({"window": format!("{:?}", self.window.window_id()),
                "outcome": "painted_or_unchanged", "route": route}),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_native_ack_does_not_require_or_invent_clock_fields() {
        let ack: NativeAck =
            serde_json::from_str(r#"{"sequence":42,"success":false,"error":"not foreground"}"#)
                .unwrap();
        assert!(!ack.success);
        assert_eq!(ack.qpc_after, None);
        assert_eq!(ack.error.as_deref(), Some("not foreground"));
    }

    #[gpui::test]
    fn native_sequence_bound_rejects_before_target_resolution_or_file_writes(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let request_path = directory.path().join("requests.jsonl");
        let mut replay = NativeReplay {
            requests: File::create(&request_path).unwrap(),
            ack_path: directory.path().join("acks.jsonl"),
            acknowledgements: None,
            partial: String::new(),
            expected: BTreeSet::new(),
            received: BTreeSet::new(),
        };
        let (_, visual) = cx.add_window_view(|_, _| gpui::Empty);
        visual.update(|window, _| {
            let position = point(px(1.0), px(1.0));
            for sequence in [0, 65536, u64::MAX] {
                let error = replay
                    .request(sequence, position, Instant::now(), window)
                    .unwrap_err();
                assert!(error.contains("1..=65535"));
            }
            // The maximum payload is accepted by the guard, then rejected by
            // TestWindow because it cannot supply a real native HWND target.
            assert!(
                replay
                    .request(65535, position, Instant::now(), window)
                    .unwrap_err()
                    .contains("target is unavailable")
            );
            assert!(replay.expected.is_empty());
            replay.expected.insert(11);
            assert!(
                replay
                    .request(11, position, Instant::now(), window)
                    .unwrap_err()
                    .contains("was reused")
            );
        });
        assert_eq!(replay.expected.len(), 1);
        assert!(std::fs::read(request_path).unwrap().is_empty());
    }
}
