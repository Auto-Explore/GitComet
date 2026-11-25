use super::StartupCrashReport;
use super::session::{ConfirmedCrash, CrashCause, SessionRecord};
use gitcomet_core::identity;
use gitcomet_core::startup_exit::{StartupExitReason, StartupExitReport};
use std::fmt::Write;
use std::path::PathBuf;

const MAX_TITLE_CHARS: usize = 96;

pub(super) fn build_startup_report(
    path: PathBuf,
    contents: &[u8],
    record: &SessionRecord,
    crash: &ConfirmedCrash,
) -> StartupCrashReport {
    let (reason, capture_id, description) = match &crash.cause {
        CrashCause::RustPanic {
            capture_id,
            message,
            ..
        } => (
            StartupExitReason::RustPanic,
            *capture_id,
            message.as_deref().map(super::single_line_text),
        ),
        CrashCause::NativeFault { description, .. } => (
            StartupExitReason::NativeFault,
            None,
            Some(description.clone()),
        ),
    };
    let description = description
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| reason.description().into());
    let mut snapshot =
        StartupExitReport::new_with_session(path.clone(), contents, reason, Some(record.token));
    if let Some(id) = capture_id {
        snapshot = snapshot.with_panic_capture_id(id);
    }
    let time_is_event = matches!(
        crash.cause,
        CrashCause::NativeFault {
            time_is_event: true,
            ..
        }
    );
    let time_label = if time_is_event {
        "OS event recorded"
    } else {
        "Crash time"
    };
    let summary = match display_time(crash.timestamp_unix_ms) {
        Some(time) => format!("{} · {time_label} {time}", truncate(&description, 160)),
        None => truncate(&description, 160),
    };
    let issue_url = identity::current()
        .links()
        .new_issue
        .as_deref()
        .map_or_else(String::new, |url| {
            let title = format!("Crash: {}", truncate(&description, MAX_TITLE_CHARS));
            let body = issue_body(record, crash, time_label, &description);
            build_issue_url(url, &title, &body)
        });
    StartupCrashReport {
        issue_url,
        summary,
        crash_log_path: path,
        snapshot,
    }
}

fn display_time(ms: u64) -> Option<String> {
    let timestamp = jiff::Timestamp::from_millisecond(i64::try_from(ms).ok()?).ok()?;
    Some(
        timestamp
            .to_zoned_datetime(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d %H:%M:%S %Z")
            .to_string(),
    )
}

fn issue_body(
    record: &SessionRecord,
    crash: &ConfirmedCrash,
    time_label: &str,
    description: &str,
) -> String {
    let product = identity::current().display_name();
    let mut body = format!(
        "## Crash Summary\n\n<!-- Please describe what you were doing right before the crash. -->\n{description}\n\n## Environment\n\n- {product} version: `{}`\n- {time_label} (Unix ms): `{}`\n",
        record.version, crash.timestamp_unix_ms,
    );
    if let Some(environment) = &crash.environment {
        let _ = writeln!(body, "\n```text\n{}```\n", environment.summary());
    }
    if let Some(operation) = &crash.operation {
        let _ = writeln!(body, "- Last UI operation: `{operation}`");
    }
    match &crash.cause {
        CrashCause::RustPanic {
            thread, context, ..
        } => {
            let _ = writeln!(
                body,
                "- Failure kind: Rust panic\n- Failure context: `{context}`"
            );
            if let Some(thread) = thread {
                let _ = writeln!(body, "- Thread: `{thread}`");
            }
        }
        CrashCause::NativeFault { .. } => {
            let _ = writeln!(body, "- Failure kind: Native fault");
        }
    }
    body
}

fn build_issue_url(url: &str, title: &str, body: &str) -> String {
    format!(
        "{url}?template=crash_report.md&title={}&body={}",
        percent_encode(title),
        percent_encode(body)
    )
}

fn percent_encode(input: &str) -> String {
    gitcomet_core::url_encoding::encode_component(input).collect()
}

fn truncate(input: &str, max_chars: usize) -> String {
    match input.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &input[..index]),
        None => input.to_owned(),
    }
}
