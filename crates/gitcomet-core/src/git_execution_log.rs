pub mod context;
pub mod output;

use crate::error::{Error, ErrorKind, GitFailureId};
use crate::git_operation::GitOutputStream;
use crate::services::Result;
use context::{ExecutionContext, Origin};
use output::{METADATA_LIMIT, OutputBuffer, OutputPreview, bounded_text, sanitize};
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

const ENTRY_LIMIT: usize = 100;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RepoLogKey(PathBuf);

impl RepoLogKey {
    pub fn new(path: &Path) -> Self {
        let mut missing = Vec::new();
        let mut ancestor = path;
        while !ancestor.exists() {
            let Some(name) = ancestor.file_name() else {
                break;
            };
            missing.push(name.to_os_string());
            let Some(parent) = ancestor.parent() else {
                break;
            };
            ancestor = parent;
        }
        let mut root = crate::path_utils::canonicalize_or_original(ancestor.to_path_buf());
        for name in missing.into_iter().rev() {
            root.push(name);
        }
        Self(root)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionKind {
    Git,
    Gix,
}

impl std::fmt::Display for ExecutionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Git => "Git",
            Self::Gix => "gix",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionOutcome {
    Success,
    NonzeroExit,
    Failed(String),
    Cancelled,
    TimedOut,
    Interrupted,
}

impl ExecutionOutcome {
    pub fn is_failure(&self) -> bool {
        !matches!(self, Self::Success)
    }
}

impl std::fmt::Display for ExecutionOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Success => "Completed",
            Self::NonzeroExit => "Nonzero exit",
            Self::Failed(_) => "Failed",
            Self::Cancelled => "Cancelled",
            Self::TimedOut => "Timed out",
            Self::Interrupted => "Interrupted",
        })
    }
}

#[derive(Clone, Debug)]
pub enum ExecutionState {
    Running,
    Finished {
        outcome: ExecutionOutcome,
        finished_at: SystemTime,
    },
}

#[derive(Clone, Debug)]
pub struct ExecutionEntry {
    pub id: u64,
    pub repository: RepoLogKey,
    pub context: ExecutionContext,
    pub kind: ExecutionKind,
    pub command: String,
    pub issuer: String,
    pub working_directory: Option<PathBuf>,
    pub thread: String,
    pub started_at: SystemTime,
    pub elapsed: Duration,
    pub state: ExecutionState,
    pub exit_code: Option<i32>,
    pub stdout: OutputPreview,
    pub stderr: OutputPreview,
}

impl ExecutionEntry {
    pub fn is_failure(&self) -> bool {
        matches!(&self.state, ExecutionState::Finished { outcome, .. } if outcome.is_failure())
    }

    pub fn status(&self) -> String {
        match &self.state {
            ExecutionState::Running => "Running".into(),
            ExecutionState::Finished { outcome, .. } => outcome.to_string(),
        }
    }

    pub fn details(&self) -> String {
        let error = match &self.state {
            ExecutionState::Finished {
                outcome: ExecutionOutcome::Failed(detail),
                ..
            } => format!("\nError: {detail}"),
            _ => String::new(),
        };
        format!(
            "{}\nBackend: {}\nSource: {}\nIssuer: {}\nThread: {}\nRepository: {}\nWorking directory: {}\nStarted: {:?}\nDuration: {:.3} ms\nStatus: {}\nExit code: {:?}\nExecution: {} · Action: {} · Parent: {:?}{error}\n\nstdout ({} bytes, {} omitted):\n{}\n\nstderr ({} bytes, {} omitted):\n{}",
            self.command,
            self.kind,
            self.context.origin,
            self.issuer,
            self.thread,
            self.repository.path().display(),
            self.working_directory
                .as_deref()
                .map_or_else(|| "Unavailable".into(), |p| p.display().to_string()),
            self.started_at,
            self.elapsed.as_secs_f64() * 1000.0,
            self.status(),
            self.exit_code,
            self.id,
            self.context.action_id,
            self.context.parent_execution,
            self.stdout.total_bytes,
            self.stdout.omitted_bytes,
            self.stdout.text,
            self.stderr.total_bytes,
            self.stderr.omitted_bytes,
            self.stderr.text
        )
    }
}

struct LiveEntry {
    entry: ExecutionEntry,
    started: Instant,
    stdout: OutputBuffer,
    stderr: OutputBuffer,
    sensitive: bool,
    successful_exit: bool,
}

impl LiveEntry {
    fn snapshot(&self) -> ExecutionEntry {
        let mut entry = self.entry.clone();
        if matches!(entry.state, ExecutionState::Running) {
            entry.elapsed = self.started.elapsed();
        }
        entry.stdout = self.stdout.snapshot(self.sensitive);
        entry.stderr = self.stderr.snapshot(self.sensitive);
        entry
    }
}

type Changed = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct RepositoryLog {
    entries: VecDeque<Arc<Mutex<LiveEntry>>>,
    subscribers: HashMap<u64, Changed>,
}

#[derive(Default)]
struct LogInner {
    repositories: Mutex<HashMap<RepoLogKey, RepositoryLog>>,
    sequence: AtomicU64,
}

#[derive(Clone, Default)]
pub struct GitExecutionLog {
    inner: Arc<LogInner>,
}

impl GitExecutionLog {
    pub fn global() -> &'static Self {
        static LOG: LazyLock<GitExecutionLog> = LazyLock::new(GitExecutionLog::default);
        &LOG
    }

    pub fn begin(
        &self,
        repository: &RepoLogKey,
        kind: ExecutionKind,
        command: &str,
        issuer: &str,
    ) -> ExecutionGuard {
        let mut context =
            context::current().unwrap_or_else(|| ExecutionContext::new(Origin::Unknown));
        context.origin = context.origin.bounded();
        let command = bounded_text(&sanitize(command), METADATA_LIMIT / 2);
        let issuer = bounded_text(&sanitize(issuer), METADATA_LIMIT / 4);
        let mut repositories = self
            .inner
            .repositories
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let id = self.inner.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let live = Arc::new(Mutex::new(LiveEntry {
            entry: ExecutionEntry {
                id,
                repository: repository.clone(),
                context: context.clone(),
                kind,
                command,
                issuer,
                working_directory: None,
                thread: bounded_text(
                    &format!(
                        "{} ({:?})",
                        std::thread::current().name().unwrap_or("unnamed"),
                        std::thread::current().id()
                    ),
                    512,
                ),
                started_at: SystemTime::now(),
                elapsed: Duration::ZERO,
                state: ExecutionState::Running,
                exit_code: None,
                stdout: OutputPreview::default(),
                stderr: OutputPreview::default(),
            },
            started: Instant::now(),
            stdout: OutputBuffer::default(),
            stderr: OutputBuffer::default(),
            sensitive: false,
            successful_exit: true,
        }));
        let log = repositories.entry(repository.clone()).or_default();
        log.entries.push_back(Arc::clone(&live));
        if log.entries.len() > ENTRY_LIMIT {
            log.entries.pop_front();
        }
        drop(repositories);
        let handle = OutputCapture {
            entry: Arc::downgrade(&live),
            log: self.clone(),
            repository: repository.clone(),
        };
        let mut child_context = context;
        child_context.repository = Some(repository.clone());
        child_context.parent_execution = Some(id);
        self.notify(repository);
        ExecutionGuard {
            capture: Some(handle),
            context: child_context,
            finished: false,
        }
    }

    pub fn snapshot(&self, key: &RepoLogKey) -> Vec<ExecutionEntry> {
        let entries: Vec<_> = self
            .inner
            .repositories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .map(|log| log.entries.iter().rev().cloned().collect())
            .unwrap_or_default();
        entries
            .into_iter()
            .map(|entry| entry.lock().unwrap_or_else(|e| e.into_inner()).snapshot())
            .collect()
    }

    pub fn clear(&self, key: &RepoLogKey) {
        if let Some(log) = self
            .inner
            .repositories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(key)
        {
            log.entries.clear();
        }
        self.notify(key);
    }

    pub fn subscribe(
        &self,
        key: &RepoLogKey,
        callback: impl Fn() + Send + Sync + 'static,
    ) -> LogSubscription {
        let id = self.inner.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        self.inner
            .repositories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key.clone())
            .or_default()
            .subscribers
            .insert(id, Arc::new(callback));
        LogSubscription {
            log: Arc::downgrade(&self.inner),
            key: key.clone(),
            id,
        }
    }

    fn notify(&self, key: &RepoLogKey) {
        let callbacks: Vec<_> = self
            .inner
            .repositories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .map(|log| log.subscribers.values().cloned().collect())
            .unwrap_or_default();
        for callback in callbacks {
            callback();
        }
    }
}

pub struct LogSubscription {
    log: Weak<LogInner>,
    key: RepoLogKey,
    id: u64,
}

impl Drop for LogSubscription {
    fn drop(&mut self) {
        if let Some(log) = self.log.upgrade()
            && let Some(repository) = log
                .repositories
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&self.key)
        {
            repository.subscribers.remove(&self.id);
        }
    }
}

#[derive(Clone)]
pub struct OutputCapture {
    entry: Weak<Mutex<LiveEntry>>,
    log: GitExecutionLog,
    repository: RepoLogKey,
}

impl OutputCapture {
    fn update(&self, update: impl FnOnce(&mut LiveEntry)) {
        let Some(entry) = self.entry.upgrade() else {
            return;
        };
        update(&mut entry.lock().unwrap_or_else(|e| e.into_inner()));
        self.log.notify(&self.repository);
    }

    pub fn append(&self, stream: GitOutputStream, bytes: &[u8]) {
        self.update(|entry| match stream {
            GitOutputStream::Stdout => entry.stdout.append(bytes),
            GitOutputStream::Stderr => entry.stderr.append(bytes),
        });
    }
}

pub struct ExecutionGuard {
    capture: Option<OutputCapture>,
    context: ExecutionContext,
    finished: bool,
}

impl ExecutionGuard {
    pub fn command(command: &Command, issuer: &str) -> Self {
        let mut directory = command.get_current_dir().map(Path::to_path_buf);
        let mut arguments = command.get_args();
        while let Some(argument) = arguments.next() {
            if argument == "-C"
                && let Some(path) = arguments.next()
            {
                directory = Some(
                    directory
                        .as_ref()
                        .map_or_else(|| PathBuf::from(path), |cwd| cwd.join(path)),
                );
            } else if matches!(
                argument.to_str(),
                Some("-c" | "--config-env" | "--git-dir" | "--work-tree")
            ) {
                arguments.next();
            } else if argument == "--" || !argument.as_encoded_bytes().starts_with(b"-") {
                break;
            }
        }
        let key = directory
            .as_deref()
            .map(RepoLogKey::new)
            .or_else(|| context::current().and_then(|ctx| ctx.repository));
        match key {
            Some(key) => Self::command_in(&key, command, issuer),
            None => Self {
                capture: None,
                context: context::current()
                    .unwrap_or_else(|| ExecutionContext::new(Origin::Unknown)),
                finished: false,
            },
        }
    }

    pub fn command_in(repository: &RepoLogKey, command: &Command, issuer: &str) -> Self {
        let mut invocation =
            bounded_text(&format!("{:?}", command.get_program()), METADATA_LIMIT / 2);
        let mut sensitive = false;
        let mut redact_next = false;
        for argument in command.get_args() {
            let text = argument.to_string_lossy();
            let (key, value) = text
                .split_once('=')
                .map_or((text.as_ref(), None), |(key, value)| (key, Some(value)));
            let secret = output::sensitive_key(key);
            sensitive |= secret;
            let argument = if redact_next {
                "<redacted>".to_owned()
            } else if secret && value.is_some() {
                format!("{key}=<redacted>")
            } else {
                sanitize(&text)
            };
            redact_next = secret && value.is_none() && text.starts_with('-');
            let remaining = (METADATA_LIMIT / 2).saturating_sub(invocation.len());
            if remaining < 8 {
                invocation.push('…');
                break;
            }
            invocation.push(' ');
            let _ = write!(
                invocation,
                "{}",
                bounded_text(&format!("{argument:?}"), remaining - 1)
            );
        }
        let guard =
            GitExecutionLog::global().begin(repository, ExecutionKind::Git, &invocation, issuer);
        if let Some(capture) = &guard.capture {
            capture.update(|entry| {
                entry.sensitive = sensitive;
                entry.entry.working_directory = command
                    .get_current_dir()
                    .map(Path::to_path_buf)
                    .or_else(|| std::env::current_dir().ok());
            });
        }
        guard
    }

    pub fn output(&self) -> Option<OutputCapture> {
        self.capture.clone()
    }

    pub fn exited(&self, status: ExitStatus, accepted: bool) {
        if let Some(capture) = &self.capture {
            capture.update(|entry| {
                entry.entry.exit_code = status.code();
                entry.successful_exit = status.success() || accepted;
            });
        }
    }

    pub fn finish(&mut self, outcome: ExecutionOutcome) {
        self.finished = true;
        let outcome = match outcome {
            ExecutionOutcome::Failed(detail) => {
                ExecutionOutcome::Failed(bounded_text(&sanitize(&detail), METADATA_LIMIT / 4))
            }
            other => other,
        };
        if let Some(capture) = &self.capture {
            capture.update(|entry| {
                entry.stdout.flush_pending();
                entry.stderr.flush_pending();
                entry.entry.elapsed = entry.started.elapsed();
                entry.entry.state = ExecutionState::Finished {
                    outcome,
                    finished_at: SystemTime::now(),
                };
            });
        }
    }

    pub fn finish_result<T>(&mut self, result: &Result<T>) {
        let outcome = match result {
            Ok(_) => {
                let successful = self
                    .capture
                    .as_ref()
                    .and_then(|capture| capture.entry.upgrade())
                    .is_none_or(|entry| {
                        entry
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .successful_exit
                    });
                if successful {
                    ExecutionOutcome::Success
                } else {
                    ExecutionOutcome::NonzeroExit
                }
            }
            Err(error) => outcome_from_error(error),
        };
        self.finish(outcome);
    }

    pub fn run<T>(mut self, run: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let context = self.context.clone();
        let _scope = context.attach();
        let result = run(&self);
        self.finish_result(&result);
        result
    }
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(ExecutionOutcome::Interrupted);
        }
    }
}

fn outcome_from_error(error: &Error) -> ExecutionOutcome {
    match error.kind() {
        ErrorKind::Cancelled => ExecutionOutcome::Cancelled,
        ErrorKind::Git(failure) if failure.id() == GitFailureId::Timeout => {
            ExecutionOutcome::TimedOut
        }
        _ => ExecutionOutcome::Failed(bounded_text(
            &sanitize(&error.to_string()),
            METADATA_LIMIT / 4,
        )),
    }
}

pub fn run_native<T>(
    repository: &RepoLogKey,
    operation: &str,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    GitExecutionLog::global()
        .begin(repository, ExecutionKind::Gix, operation, operation)
        .run(|_| run())
}

#[cfg(test)]
mod tests;
