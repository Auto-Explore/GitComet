use super::native;
use gitcomet_core::environment::EnvironmentSnapshot;
use gitcomet_core::fs_utils::{lock_private_file, write_private_file};
use gitcomet_core::startup_exit::{
    PanicCaptureId, SessionToken, current_session_token, lock_snapshot,
};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MARKER_PREFIX: &str = "session-in-progress-";
const MARKER_SUFFIX: &str = ".v1.json";
const MAX_RECORD_BYTES: u64 = 256 * 1024;
const MAX_LOCAL_CONTEXT_BYTES: u64 = 16 * 1024;
const LATEST_DIAGNOSTIC: &str = "latest-session.log";
const MAX_NATIVE_LOOKUPS_PER_LAUNCH: usize = 4;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionRecord {
    pub pid: u32,
    pub token: SessionToken,
    pub started_unix_ms: u64,
    pub version: String,
    identity: Option<native::ProcessIdentity>,
    state: SessionState,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum SessionState {
    Unfinished,
    Confirmed(Box<ConfirmedCrash>),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfirmedCrash {
    pub timestamp_unix_ms: u64,
    pub cause: CrashCause,
    pub environment: Option<EnvironmentSnapshot>,
    pub operation: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CrashCause {
    RustPanic {
        capture_id: Option<PanicCaptureId>,
        context: String,
        message: Option<String>,
        thread: Option<String>,
        last_observed_panic: Option<String>,
    },
    NativeFault {
        description: String,
        time_is_event: bool,
    },
}

static MARKER: Mutex<Option<File>> = Mutex::new(None);

impl SessionRecord {
    fn current() -> Self {
        Self {
            pid: std::process::id(),
            token: current_session_token(),
            started_unix_ms: super::unix_time_ms(),
            version: gitcomet_core::identity::current().version().into(),
            identity: None,
            state: SessionState::Unfinished,
        }
    }

    fn encode(&self) -> io::Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(io::Error::other("session record exceeds its size limit"));
        }
        Ok(bytes)
    }
}

fn marker_path(dir: &Path, pid: u32, token: SessionToken) -> PathBuf {
    dir.join(format!("{MARKER_PREFIX}{pid}-{token}{MARKER_SUFFIX}"))
}

pub(super) fn current_path(dir: &Path) -> PathBuf {
    marker_path(dir, std::process::id(), current_session_token())
}

fn marker_owner(path: &Path) -> Option<(u32, SessionToken)> {
    let name = path.file_name()?.to_str()?;
    let (pid, token) = name
        .strip_prefix(MARKER_PREFIX)?
        .strip_suffix(MARKER_SUFFIX)?
        .split_once('-')?;
    Some((pid.parse().ok()?, token.parse().ok()?))
}

pub(super) fn begin(dir: &Path) -> io::Result<()> {
    let mut lease = MARKER.lock().unwrap_or_else(|error| error.into_inner());
    if lease.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the UI session already started",
        ));
    }
    let mut record = SessionRecord::current();
    record.identity = native::current();
    let path = current_path(dir);
    let lock_path = path.with_extension("lease");
    let lock = lock_private_file(&lock_path)?;
    if let Err(error) = record
        .encode()
        .and_then(|bytes| write_private_file(&path, &bytes))
    {
        drop(lock);
        let _ = super::remove_file_if_exists(&lock_path);
        return Err(error);
    }
    *lease = Some(lock);
    Ok(())
}

pub(super) fn finish(dir: &Path) -> io::Result<()> {
    let mut lease = MARKER.lock().unwrap_or_else(|error| error.into_inner());
    let path = current_path(dir);
    if let Some((record, _)) = read(&path) {
        if matches!(record.state, SessionState::Confirmed(_)) {
            return Ok(());
        }
        if super::panic_capture::observation_path(dir, record.token).is_file() {
            let _lock = lock_snapshot(&path)?;
            save_manual_diagnostic(
                dir,
                &record,
                "The session completed shutdown after a panic was observed.",
            )?;
        }
    }
    super::remove_file_if_exists(&path)?;
    *lease = None;
    clear_retired_context(dir, &path, current_session_token())
}

pub(super) fn confirm_panic(
    dir: &Path,
    cause: CrashCause,
    timestamp_unix_ms: u64,
) -> io::Result<()> {
    let mut lease = MARKER.lock().unwrap_or_else(|error| error.into_inner());
    let path = current_path(dir);
    let mut record = read(&path).map_or_else(SessionRecord::current, |(record, _)| record);
    if matches!(record.state, SessionState::Confirmed(_)) {
        return Ok(());
    }
    record.state = SessionState::Confirmed(Box::new(ConfirmedCrash {
        timestamp_unix_ms,
        cause,
        environment: gitcomet_core::environment::try_cached(),
        operation: read_operation(dir, record.token),
    }));
    write_private_file(&path, &record.encode()?)?;
    *lease = None;
    log_cleanup_error(super::remove_file_if_exists(&path.with_extension("lease")));
    Ok(())
}

fn read(path: &Path) -> Option<(SessionRecord, Vec<u8>)> {
    let bytes = read_limited(path, MAX_RECORD_BYTES)?;
    let record: SessionRecord = serde_json::from_slice(&bytes).ok()?;
    if marker_owner(path) != Some((record.pid, record.token))
        || record
            .identity
            .as_ref()
            .is_some_and(|identity| identity.pid != record.pid)
    {
        return None;
    }
    Some((record, bytes))
}

pub(super) fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    if !path.symlink_metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

pub(super) fn recover(
    dir: &Path,
    find_fault: impl Fn(&native::ProcessIdentity, u64) -> Option<native::NativeFault>,
) -> io::Result<Option<super::StartupCrashReport>> {
    let _lock = lock_snapshot(&dir.join("session"))?;
    let mut pending = None;
    let mut latest = None;
    let mut remaining_lookups = MAX_NATIVE_LOOKUPS_PER_LAUNCH;
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if marker_owner(&path).is_none() {
            continue;
        }
        let Some((mut record, mut bytes)) = read_completed(&path) else {
            continue;
        };
        let (pid, token) = (record.pid, record.token);
        if matches!(record.state, SessionState::Unfinished) {
            let fault = match &record.identity {
                Some(identity) if remaining_lookups > 0 => {
                    remaining_lookups -= 1;
                    find_fault(identity, record.started_unix_ms)
                }
                Some(_) => continue,
                None => None,
            };
            let Some(fault) = fault else {
                if latest.as_ref().is_none_or(|previous: &SessionRecord| {
                    (record.started_unix_ms, record.token)
                        > (previous.started_unix_ms, previous.token)
                }) {
                    if let Some(previous) = latest.replace(record) {
                        log_cleanup_error(retire_finished(dir, previous.pid, previous.token));
                    }
                } else {
                    log_cleanup_error(retire_finished(dir, pid, token));
                }
                continue;
            };
            record.state = SessionState::Confirmed(Box::new(ConfirmedCrash {
                timestamp_unix_ms: fault.timestamp_unix_ms,
                cause: CrashCause::NativeFault {
                    description: fault.description,
                    time_is_event: fault.time_is_event,
                },
                environment: read_limited(
                    &super::environment_path(dir, token),
                    MAX_LOCAL_CONTEXT_BYTES,
                )
                .and_then(|bytes| serde_json::from_slice(&bytes).ok()),
                operation: read_operation(dir, token),
            }));
            let persisted = record.encode().and_then(|bytes| {
                write_private_file(&path, &bytes)?;
                Ok(bytes)
            });
            match persisted {
                Ok(encoded) => bytes = encoded,
                Err(error) => {
                    log_cleanup_error(Err(error));
                    continue;
                }
            }
        }
        let SessionState::Confirmed(ref crash) = record.state else {
            continue;
        };
        log_cleanup_error(clear_retired_context(dir, &path, token));
        let order = (crash.timestamp_unix_ms, token);
        if pending
            .as_ref()
            .is_none_or(|(previous, _)| order < *previous)
        {
            pending = Some((
                order,
                super::report::build_startup_report(path, &bytes, &record, crash),
            ));
        }
    }
    if let Some(record) = latest {
        log_cleanup_error(
            save_manual_diagnostic(
                dir,
                &record,
                "The session ended without completing shutdown.",
            )
            .and_then(|()| retire_finished(dir, record.pid, record.token)),
        );
    }
    Ok(pending.map(|(_, report)| report))
}

fn retire_finished(dir: &Path, pid: u32, token: SessionToken) -> io::Result<()> {
    let path = marker_path(dir, pid, token);
    super::remove_file_if_exists(&path)?;
    clear_retired_context(dir, &path, token)
}

fn clear_retired_context(dir: &Path, path: &Path, token: SessionToken) -> io::Result<()> {
    super::remove_file_if_exists(&path.with_extension("lease"))?;
    super::clear_context(dir, token)
}

fn read_completed(path: &Path) -> Option<(SessionRecord, Vec<u8>)> {
    let (record, bytes) = read(path)?;
    if matches!(record.state, SessionState::Confirmed(_)) {
        return Some((record, bytes));
    }
    let lease = match OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_extension("lease"))
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return read(path),
        Err(error) => {
            log_cleanup_error(Err(error));
            return None;
        }
    };
    if !lease.metadata().ok()?.is_file() {
        return None;
    }
    match lease.try_lock() {
        Ok(()) => read(path),
        Err(TryLockError::WouldBlock) => None,
        Err(TryLockError::Error(error)) => {
            log_cleanup_error(Err(error));
            None
        }
    }
}

fn log_cleanup_error(result: io::Result<()>) {
    if let Err(error) = result {
        eprintln!("Could not finish recovering previous session diagnostics: {error}");
    }
}

fn read_operation(dir: &Path, token: SessionToken) -> Option<String> {
    let bytes = read_limited(&super::last_operation_path_for_session(dir, token), 1024)?;
    let line = std::str::from_utf8(&bytes)
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("copy_source="))?;
    Some(line.to_owned())
}

fn save_manual_diagnostic(dir: &Path, record: &SessionRecord, outcome: &str) -> io::Result<()> {
    let path = dir.join(LATEST_DIAGNOSTIC);
    let previous_start = read_limited(&path, MAX_LOCAL_CONTEXT_BYTES)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| {
            text.lines()
                .next()?
                .strip_prefix("Session started (Unix ms): ")?
                .parse::<u64>()
                .ok()
        });
    if previous_start.is_some_and(|time| time >= record.started_unix_ms) {
        return Ok(());
    }
    let mut bytes = format!(
        "Session started (Unix ms): {}\nVersion: {}\n{outcome}\n",
        record.started_unix_ms, record.version,
    )
    .into_bytes();
    if let Some(operation) = read_operation(dir, record.token) {
        writeln!(bytes, "Last UI operation: {operation}")?;
    }
    for context in [
        super::runtime_error_path_for_session(dir, record.token),
        super::panic_capture::observation_path(dir, record.token),
    ] {
        let limit = MAX_LOCAL_CONTEXT_BYTES.saturating_sub(bytes.len() as u64);
        if let Some(context) = read_limited(&context, limit) {
            bytes.extend_from_slice(&context);
        }
    }
    write_private_file(&path, &bytes)
}

#[cfg(test)]
mod tests;
