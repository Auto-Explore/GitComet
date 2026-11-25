use super::*;

const CRASH_ISSUE_TEMPLATE: &str = "crash_report.md";
const MAX_TITLE_CHARS: usize = 96;

pub(super) fn build_startup_report(crash_log_path: PathBuf, crash_log: &str) -> StartupCrashReport {
    let parsed = parse_crash_log(crash_log);
    let issue_title = build_issue_title(&parsed);
    let issue_body = build_issue_body(&parsed, &crash_log_path);
    let summary_message = parsed
        .message
        .as_deref()
        .map(single_line_text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown failure".to_string());
    let summary_location = parsed
        .location
        .as_deref()
        .map(single_line_text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown location".to_string());

    let reason = parsed.exit_reason.unwrap_or(StartupExitReason::UnknownExit);
    let legacy_marker = parsed.failure_kind.is_none()
        && parsed.backtrace.is_empty()
        && parsed.location.is_none()
        && summary_message.ends_with("did not exit cleanly");
    let summary = if legacy_marker || parsed.exit_reason.is_some() {
        reason.description().to_owned()
    } else {
        format!(
            "{} at {}",
            truncate_chars(&summary_message, 160),
            truncate_chars(&summary_location, 160)
        )
    };

    StartupCrashReport {
        snapshot: StartupExitReport::new_with_session(
            crash_log_path.clone(),
            crash_log.as_bytes(),
            reason,
            parsed.session_token,
        ),
        issue_url: build_issue_url(&issue_title, &issue_body),
        summary,
        crash_log_path,
    }
}

#[derive(Default)]
pub(super) struct ParsedCrashLog {
    pub(super) exit_reason: Option<StartupExitReason>,
    pub(super) session_token: Option<gitcomet_core::startup_exit::SessionToken>,
    pub(super) environment: Option<EnvironmentSnapshot>,
    pub(super) os: Option<String>,
    pub(super) arch: Option<String>,
    pub(super) failure_kind: Option<String>,
    pub(super) timestamp_unix_ms: Option<String>,
    pub(super) crate_name: Option<String>,
    pub(super) crate_version: Option<String>,
    pub(super) thread: Option<String>,
    pub(super) location: Option<String>,
    pub(super) message: Option<String>,
    pub(super) info: Option<String>,
    pub(super) failure_context: Option<String>,
    pub(super) copy_source: Option<String>,
    pub(super) clipboard_backend: Option<String>,
    pub(super) backtrace: String,
}

pub(super) fn parse_crash_log(crash_log: &str) -> ParsedCrashLog {
    let mut parsed = ParsedCrashLog::default();
    let mut in_backtrace = false;
    let mut expect_session_token = false;

    for raw_line in crash_log.lines() {
        let line = raw_line.trim_end_matches('\r');

        if std::mem::take(&mut expect_session_token)
            && let Some(token) = line.strip_prefix("session_token=")
        {
            parsed.session_token = token.parse().ok();
            continue;
        }

        if line == "=== GitComet operation context ===" || line == "=== GitComet environment ===" {
            in_backtrace = false;
            continue;
        }

        if line.starts_with("=== GitComet ") && line.ends_with(" ===") {
            reset_parsed_failure(&mut parsed);
            parsed.failure_kind = if line.contains("panic") {
                Some("panic".to_string())
            } else if line.contains("runtime error") {
                Some("runtime-error".to_string())
            } else if line.contains("abnormal exit") {
                Some("abnormal-exit".to_string())
            } else {
                None
            };
            expect_session_token = matches!(
                parsed.failure_kind.as_deref(),
                Some("abnormal-exit" | "panic")
            );
            if expect_session_token {
                parsed.session_token = None;
            }
            match parsed.failure_kind.as_deref() {
                Some("abnormal-exit") => parsed.exit_reason = Some(StartupExitReason::UnknownExit),
                Some("panic") => parsed.exit_reason = None,
                _ => {}
            }
            in_backtrace = false;
            continue;
        }

        if in_backtrace {
            parsed.backtrace.push_str(line);
            parsed.backtrace.push('\n');
            continue;
        }

        if let Some(json) = line.strip_prefix("environment_json=") {
            parsed.environment = serde_json::from_str(json).ok();
            continue;
        }
        if let Some(os) = line.strip_prefix("os=") {
            parsed.os = Some(os.into());
            continue;
        }
        if let Some(arch) = line.strip_prefix("arch=") {
            parsed.arch = Some(arch.into());
            continue;
        }

        if line == "backtrace:" {
            in_backtrace = true;
            continue;
        }

        if let Some(rest) = line.strip_prefix("backtrace:") {
            in_backtrace = true;
            let rest = rest.trim_start();
            if !rest.is_empty() {
                parsed.backtrace.push_str(rest);
                parsed.backtrace.push('\n');
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("timestamp_unix_ms=") {
            parsed.timestamp_unix_ms = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("failure_kind=") {
            let kind = rest.trim();
            match kind {
                "abnormal-exit" => parsed.exit_reason = Some(StartupExitReason::UnknownExit),
                "x11-event-loop-error" => {
                    parsed.exit_reason = Some(StartupExitReason::X11EventLoopError)
                }
                "wayland-event-loop-error" => {
                    parsed.exit_reason = Some(StartupExitReason::WaylandEventLoopError)
                }
                "panic" | "returned-error" => parsed.exit_reason = None,
                _ => {}
            }
            parsed.failure_kind = Some(kind.to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("crate=") {
            if let Some((name, version)) = rest.split_once(" version=") {
                parsed.crate_name = Some(name.trim().to_string());
                parsed.crate_version = Some(version.trim().to_string());
            } else {
                parsed.crate_name = Some(rest.trim().to_string());
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("thread=") {
            parsed.thread = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("location=") {
            parsed.location = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("message=") {
            parsed.message = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("info=") {
            parsed.info = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("failure_context=") {
            parsed.failure_context = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("copy_source=") {
            parsed.copy_source = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("clipboard_backend=") {
            parsed.clipboard_backend = Some(rest.trim().to_string());
        }
    }

    parsed
}

fn reset_parsed_failure(parsed: &mut ParsedCrashLog) {
    parsed.environment = None;
    parsed.os = None;
    parsed.arch = None;
    parsed.failure_kind = None;
    parsed.timestamp_unix_ms = None;
    parsed.crate_name = None;
    parsed.crate_version = None;
    parsed.thread = None;
    parsed.location = None;
    parsed.message = None;
    parsed.info = None;
    parsed.backtrace.clear();
}

/// The prefilled new-issue page, or empty when the product has no tracker.
pub(super) fn build_issue_url(title: &str, body: &str) -> String {
    let Some(new_issue) = identity::current().links().new_issue.as_deref() else {
        return String::new();
    };
    format!(
        "{new_issue}?template={}&title={}&body={}",
        percent_encode(CRASH_ISSUE_TEMPLATE),
        percent_encode(title),
        percent_encode(body)
    )
}

fn build_issue_title(parsed: &ParsedCrashLog) -> String {
    let failure_message = parsed
        .message
        .as_deref()
        .map(single_line_text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown failure".to_string());
    format!(
        "Crash: {}",
        truncate_chars(&failure_message, MAX_TITLE_CHARS)
    )
}

pub(super) fn build_issue_body(parsed: &ParsedCrashLog, crash_log_path: &Path) -> String {
    let product = identity::current().display_name();
    let crate_name = parsed
        .crate_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(identity::current().executable_name());
    let crate_version = parsed
        .crate_version
        .as_deref()
        .or_else(|| {
            parsed
                .environment
                .as_ref()
                .map(|snapshot| snapshot.app_version.as_str())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or("Unavailable");
    let timestamp = parsed
        .timestamp_unix_ms
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");
    let thread = parsed
        .thread
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");
    let failure_kind = parsed
        .failure_kind
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");
    let location = parsed
        .location
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");
    let message = parsed
        .message
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown failure message>");
    let info = parsed
        .info
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown failure info>");
    let copy_source = parsed
        .copy_source
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<no UI operation recorded>");
    let failure_context = parsed
        .failure_context
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");

    let backtrace = {
        let trimmed = parsed.backtrace.trim();
        if trimmed.is_empty() {
            "<no backtrace captured>".to_string()
        } else {
            truncate_chars(trimmed, MAX_BACKTRACE_CHARS)
        }
    };

    let mut body = String::new();
    let _ = writeln!(body, "## Crash Summary");
    let _ = writeln!(body);
    let _ = writeln!(
        body,
        "<!-- Please describe what you were doing right before the crash. -->"
    );
    let _ = writeln!(body, "{product} ended unexpectedly.");
    let _ = writeln!(body);

    let _ = writeln!(body, "## Environment");
    let _ = writeln!(body);
    let _ = writeln!(body, "- {product} crate: `{crate_name}`");
    let _ = writeln!(body, "- {product} version: `{crate_version}`");
    if let Some(environment) = &parsed.environment {
        let _ = writeln!(body, "\n```text\n{}```\n", environment.summary());
    } else {
        let _ = writeln!(
            body,
            "- OS: `{}`",
            parsed.os.as_deref().unwrap_or("Unavailable")
        );
        let _ = writeln!(
            body,
            "- Arch: `{}`",
            parsed.arch.as_deref().unwrap_or("Unavailable")
        );
    }
    let _ = writeln!(body, "- Crash timestamp (unix ms): `{timestamp}`");
    let _ = writeln!(body, "- Thread: `{thread}`");
    let _ = writeln!(body, "- Failure kind: `{failure_kind}`");
    let _ = writeln!(body, "- Failure location: `{location}`");
    let _ = writeln!(body, "- Failure context: `{failure_context}`");
    let _ = writeln!(body, "- Last UI operation: `{copy_source}`");
    if let Some(clipboard_backend) = parsed
        .clipboard_backend
        .as_deref()
        .filter(|backend| !backend.is_empty())
    {
        let _ = writeln!(body, "- Clipboard backend: `{clipboard_backend}`");
    }
    let _ = writeln!(body, "- Crash log path: `{}`", crash_log_path.display());
    let _ = writeln!(body);

    let _ = writeln!(body, "## Failure Message");
    let _ = writeln!(body);
    let _ = writeln!(body, "```text");
    let _ = writeln!(body, "{message}");
    let _ = writeln!(body, "```");
    let _ = writeln!(body);

    let _ = writeln!(body, "## Failure Info");
    let _ = writeln!(body);
    let _ = writeln!(body, "```text");
    let _ = writeln!(body, "{info}");
    let _ = writeln!(body, "```");
    let _ = writeln!(body);

    let _ = writeln!(body, "## Backtrace (trimmed)");
    let _ = writeln!(body);
    let _ = writeln!(body, "```text");
    let _ = writeln!(body, "{backtrace}");
    let _ = writeln!(body, "```");
    body
}

pub(super) fn percent_encode(input: &str) -> String {
    gitcomet_core::url_encoding::encode_component(input).collect()
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let mut out = String::with_capacity(max_chars);
    for (idx, ch) in input.chars().enumerate() {
        if idx + 3 >= max_chars {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}
