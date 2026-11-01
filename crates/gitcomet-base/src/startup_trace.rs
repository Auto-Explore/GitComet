//! Opt-in startup stage timings, available before the native UI exists.
//!
//! Set `GITCOMET_STARTUP_TRACE` to a local file path. Each line is JSON; begin
//! records survive a stalled stage and end records include its duration. The
//! file is appended, with a process id on every record. No repository paths,
//! environment values or command arguments are recorded.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static TRACE: OnceLock<Option<Trace>> = OnceLock::new();

struct Trace {
    origin: Instant,
    file: Mutex<File>,
}

fn trace() -> Option<&'static Trace> {
    TRACE
        .get_or_init(|| {
            let path = std::env::var_os("GITCOMET_STARTUP_TRACE").filter(|p| !p.is_empty())?;
            let origin = Instant::now();
            match OpenOptions::new().create(true).append(true).open(path) {
                Ok(file) => Some(Trace {
                    origin,
                    file: Mutex::new(file),
                }),
                Err(error) => {
                    crate::write_stderr_line(format_args!("Cannot open startup trace: {error}"));
                    None
                }
            }
        })
        .as_ref()
}

pub fn is_enabled() -> bool {
    trace().is_some()
}

/// A timestamp relative to the earliest startup trace call in this process.
pub fn mark(name: &'static str) {
    if let Some(trace) = trace() {
        trace.write("mark", name, None);
    }
}

/// Times a synchronous stage. Keep the guard alive only for that stage, not
/// for the whole event loop. Disabled tracing does not read the clock.
#[must_use = "the startup stage ends when its guard drops"]
pub fn span(name: &'static str) -> Span {
    Span(trace().map(|trace| {
        trace.write("begin", name, None);
        (name, Instant::now())
    }))
}

pub struct Span(Option<(&'static str, Instant)>);

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((name, started)) = self.0
            && let Some(trace) = trace()
        {
            trace.write("end", name, Some(started.elapsed().as_secs_f64() * 1_000.0));
        }
    }
}

impl Trace {
    fn write(&self, kind: &str, name: &str, duration_ms: Option<f64>) {
        let thread = std::thread::current();
        let record = record(
            kind,
            name,
            self.origin.elapsed().as_secs_f64() * 1_000.0,
            duration_ms,
            thread.name(),
        );
        if let Ok(mut file) = self.file.lock() {
            let _ = write_record(&mut *file, &record);
        }
    }
}

fn write_record(writer: &mut impl Write, record: &serde_json::Value) -> std::io::Result<()> {
    // Value's Display implementation streams many tiny writes. Assemble one
    // complete record so tracing does not add a filesystem call per JSON token.
    let line = format!("{record}\n");
    writer.write_all(line.as_bytes())
}

fn record(
    kind: &str,
    name: &str,
    elapsed_ms: f64,
    duration_ms: Option<f64>,
    thread: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "kind": kind,
        "name": name,
        "elapsed_ms": elapsed_ms,
        "duration_ms": duration_ms,
        "pid": std::process::id(),
        "thread": thread,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_records_are_json_with_distinct_elapsed_and_stage_duration() {
        let value = record("end", "native_application", 125.0, Some(95.0), Some("main"));
        let parsed: serde_json::Value = serde_json::from_str(&value.to_string()).unwrap();
        assert_eq!(parsed["elapsed_ms"], 125.0);
        assert_eq!(parsed["duration_ms"], 95.0);
        assert_eq!(parsed["pid"], std::process::id());
        assert_eq!(parsed["name"], "native_application");
    }

    #[test]
    fn trace_writes_each_record_as_a_complete_line_without_waiting_for_shutdown() {
        let temporary = tempfile::NamedTempFile::new().unwrap();
        let trace = Trace {
            origin: Instant::now(),
            file: Mutex::new(temporary.reopen().unwrap()),
        };
        trace.write("begin", "font\"catalog\n日本語", None);
        let first = std::fs::read_to_string(temporary.path()).unwrap();
        assert_eq!(first.lines().count(), 1);
        let first: serde_json::Value = serde_json::from_str(first.trim_end()).unwrap();
        assert_eq!(first["name"], "font\"catalog\n日本語");
        trace.write("end", "font\"catalog\n日本語", Some(2.0));
        let contents = std::fs::read_to_string(temporary.path()).unwrap();
        let rows: Vec<serde_json::Value> = contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "begin");
        assert_eq!(rows[0]["name"], "font\"catalog\n日本語");
        assert!(rows[0]["duration_ms"].is_null());
        assert_eq!(rows[1]["duration_ms"], 2.0);
    }

    #[test]
    fn trace_serializes_one_complete_record_per_write_without_changing_timestamps() {
        #[derive(Default)]
        struct Writes(Vec<Vec<u8>>);
        impl std::io::Write for Writes {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.push(bytes.to_vec());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let value = record(
            "end",
            "quote\" slash\\ newline\n雪",
            125.125,
            Some(95.25),
            Some("worker\tthread"),
        );
        let mut writer = Writes::default();
        write_record(&mut writer, &value).unwrap();
        assert_eq!(
            writer.0.len(),
            1,
            "JSON tokens must not cause separate writes"
        );
        let bytes = &writer.0[0];
        assert_eq!(bytes.last(), Some(&b'\n'));
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
        let parsed: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(parsed, value);
        assert_eq!(parsed["elapsed_ms"], 125.125);
        assert_eq!(parsed["duration_ms"], 95.25);
    }
}
