use crate::protocol::{DifftasticError, decode};
use gitcomet_core::diff_engine::{DiffEngineFallback, StructuralDiffOutcome};
use gitcomet_core::services::CancellationToken;
use process_wrap::std::*;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const VERSION: &str = env!("DIFFTASTIC_VERSION");
pub const BYTE_LIMIT: usize = 1_000_000;
const GRAPH_LIMIT: usize = 3_000_000;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(5);
const STDOUT_LIMIT: usize = 32 * 1024 * 1024;
const STDERR_LIMIT: usize = 64 * 1024;
const CONCURRENT_LIMIT: usize = 2;
static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);
static WORKERS: LazyLock<(Mutex<usize>, Condvar)> =
    LazyLock::new(|| (Mutex::new(0), Condvar::new()));

pub struct DiffInput<'a> {
    pub path: &'a Path,
    pub old: &'a str,
    pub new: &'a str,
}

pub fn compare(
    input: DiffInput<'_>,
    cancellation: &CancellationToken,
) -> Result<StructuralDiffOutcome, DifftasticError> {
    let operation = NEXT_OPERATION.fetch_add(1, Ordering::Relaxed);
    let started = Instant::now();
    log::debug!(
        "difftastic operation={operation} start old_bytes={} new_bytes={}",
        input.old.len(),
        input.new.len()
    );
    let result = compare_inner(input, cancellation);
    let outcome = match &result {
        Ok(StructuralDiffOutcome::Compared(_)) => "compared",
        Ok(StructuralDiffOutcome::Unchanged) => "unchanged",
        Ok(StructuralDiffOutcome::Fallback(_)) => "fallback",
        Err(DifftasticError::Cancelled) => "cancelled",
        Err(error) => {
            log::warn!("difftastic operation={operation} error={error}");
            "failed"
        }
    };
    log::debug!(
        "difftastic operation={operation} end outcome={outcome} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    result
}

fn compare_inner(
    input: DiffInput<'_>,
    cancellation: &CancellationToken,
) -> Result<StructuralDiffOutcome, DifftasticError> {
    if input.old.len().max(input.new.len()) > BYTE_LIMIT {
        return Ok(StructuralDiffOutcome::Fallback(
            DiffEngineFallback::ByteLimit,
        ));
    }
    if cancellation.is_cancelled() {
        return Err(DifftasticError::Cancelled);
    }
    if input.old == input.new {
        return Ok(StructuralDiffOutcome::Unchanged);
    }
    let executable = bundled_executable()?;
    let _worker = Worker::acquire(cancellation)?;
    compare_with_executable(&executable, input, cancellation)
}

pub fn bundled_executable() -> Result<PathBuf, DifftasticError> {
    bundled_executable_for(&std::env::current_exe()?)
}

fn bundled_executable_for(executable: &Path) -> Result<PathBuf, DifftasticError> {
    let executable = executable.canonicalize()?;
    let directory = executable.parent().ok_or(DifftasticError::Unavailable)?;
    let name = if cfg!(windows) { "difft.exe" } else { "difft" };
    let mut candidates = vec![directory.join("helpers").join(name)];
    if cfg!(target_os = "macos") {
        candidates.push(directory.join("../Helpers").join(name));
    }
    if cfg!(target_os = "linux") {
        candidates.push(
            directory
                .join("../libexec")
                .join(gitcomet_core::identity::current().executable_name())
                .join(name),
        );
    }
    if cfg!(debug_assertions) && directory.file_name().is_some_and(|name| name == "deps") {
        candidates.push(directory.join("../helpers").join(name));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or(DifftasticError::Unavailable)
}

fn compare_with_executable(
    executable: &Path,
    input: DiffInput<'_>,
    cancellation: &CancellationToken,
) -> Result<StructuralDiffOutcome, DifftasticError> {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    let mut version_command = command(executable);
    version_command.arg("--version");
    let version = run(version_command, cancellation, deadline, STDERR_LIMIT)?;
    if !String::from_utf8_lossy(&version.stdout)
        .split_whitespace()
        .any(|part| part == VERSION)
    {
        return Err(DifftasticError::Version);
    }
    let directory = tempfile::tempdir()?;
    let old_directory = directory.path().join("old");
    let new_directory = directory.path().join("new");
    std::fs::create_dir(&old_directory)?;
    std::fs::create_dir(&new_directory)?;
    let name = input
        .path
        .file_name()
        .ok_or_else(|| DifftasticError::InvalidOutput("comparison path has no filename".into()))?;
    let old_path = old_directory.join(name);
    let new_path = new_directory.join(name);
    std::fs::File::create(&old_path)?.write_all(input.old.as_bytes())?;
    std::fs::File::create(&new_path)?.write_all(input.new.as_bytes())?;
    let mut command = command(executable);
    command
        .current_dir(directory.path())
        .env("DFT_UNSTABLE", "yes")
        .args([
            "--display=json",
            "--color=never",
            "--strip-cr=off",
            "--parse-error-limit=0",
        ])
        .arg(format!("--byte-limit={BYTE_LIMIT}"))
        .arg(format!("--graph-limit={GRAPH_LIMIT}"))
        .arg("--")
        .arg(&old_path)
        .arg(&new_path);
    let output = run(command, cancellation, deadline, STDOUT_LIMIT)?;
    decode(&output.stdout, input.old, input.new)
}

fn command(executable: &Path) -> Command {
    let mut command = gitcomet_core::process::background_command(executable);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("DFT_")
            || key.to_string_lossy().starts_with("GIT_")
            || key == "RUST_LOG"
        {
            command.env_remove(key);
        }
    }
    command
}

struct Worker;

impl Worker {
    fn acquire(cancellation: &CancellationToken) -> Result<Self, DifftasticError> {
        let (count, available) = &*WORKERS;
        let mut count = count.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if cancellation.is_cancelled() {
                return Err(DifftasticError::Cancelled);
            }
            if *count < CONCURRENT_LIMIT {
                *count += 1;
                return Ok(Self);
            }
            count = available
                .wait_timeout(count, POLL_INTERVAL)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let (count, available) = &*WORKERS;
        *count.lock().unwrap_or_else(|error| error.into_inner()) -= 1;
        available.notify_one();
    }
}

fn run(
    mut command: Command,
    cancellation: &CancellationToken,
    deadline: Instant,
    stdout_limit: usize,
) -> Result<Output, DifftasticError> {
    if cancellation.is_cancelled() {
        return Err(DifftasticError::Cancelled);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut command = CommandWrap::from(command);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut flags = CreationFlags(Default::default());
        flags.0.0 = CREATE_NO_WINDOW;
        command.wrap(flags).wrap(JobObject);
    }
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    let mut child = command.spawn()?;
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = reader(child.stdout().take(), stdout_limit, Arc::clone(&overflow));
    let stderr = reader(child.stderr().take(), STDERR_LIMIT, Arc::clone(&overflow));
    let mut status = None;
    let failure = loop {
        if cancellation.is_cancelled() {
            break Some(DifftasticError::Cancelled);
        }
        if overflow.load(Ordering::Acquire) {
            break Some(DifftasticError::OutputLimit);
        }
        if Instant::now() >= deadline {
            break Some(DifftasticError::Timeout);
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(error) => break Some(error.into()),
            }
        }
        if status.is_some() && stdout.is_finished() && stderr.is_finished() {
            break None;
        }
        thread::sleep(POLL_INTERVAL);
    };
    if let Some(error) = failure {
        let _ = child.kill();
        let _ = child.wait();
        let _ = stdout.join();
        if let Ok(Ok(stderr)) = stderr.join() {
            log::debug!(
                "difftastic terminated error={error} stderr={}",
                String::from_utf8_lossy(&stderr)
            );
        }
        return Err(error);
    }
    let stdout = join_reader(stdout)?;
    let stderr = join_reader(stderr)?;
    if overflow.load(Ordering::Acquire) {
        return Err(DifftasticError::OutputLimit);
    }
    let status = status.ok_or_else(|| DifftasticError::Exit {
        status: None,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })?;
    if !status.success() {
        return Err(DifftasticError::Exit {
            status: status.code(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        });
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    limit: usize,
    overflow: Arc<AtomicBool>,
) -> JoinHandle<std::io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut output = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut buffer = [0; 8192];
            loop {
                let count = pipe.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                let keep = count.min(limit.saturating_sub(output.len()));
                output.extend_from_slice(&buffer[..keep]);
                if keep != count {
                    overflow.store(true, Ordering::Release);
                }
            }
        }
        Ok(output)
    })
}

fn join_reader(reader: JoinHandle<std::io::Result<Vec<u8>>>) -> Result<Vec<u8>, DifftasticError> {
    reader
        .join()
        .map_err(|_| DifftasticError::Io(std::io::Error::other("Difftastic output reader failed")))?
        .map_err(Into::into)
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
