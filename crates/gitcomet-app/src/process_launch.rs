use crate::cli::exit_code;
use process_wrap::std::{ChildWrapper, CommandWrap, ProcessSession};
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub(crate) const STARTUP_SOCKET_ARG: &str = "--__gitcomet-startup-socket";
const READY: &[u8] = b"READY 1\n";
const ACK: &[u8] = b"ACK 1\n";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const ACK_TIMEOUT: Duration = Duration::from_secs(2);
const SUPERVISOR_POLL: Duration = Duration::from_millis(10);
const CANCEL_GRACE: Duration = Duration::from_secs(1);
// Below sun_path (108 on Linux, 104 on macOS) with room for the NUL.
const MAX_SOCKET_PATH: usize = 100;
const SOCKET_SUFFIX: &str = "/gc-start-XXXXXX/ready";

#[derive(Debug)]
pub(crate) enum LaunchError {
    /// Nothing was started; the caller may run the browser in-process.
    Setup(io::Error),
    /// The browser process ran and was stopped before confirming startup.
    Startup(io::Error),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Setup(error) | Self::Startup(error) => error.fmt(f),
        }
    }
}

pub(crate) fn relaunch_command(executable: &OsStr, args: &[OsString]) -> Command {
    let mut command = Command::new(executable);
    command.args(args.iter().skip(1));
    command
}

// Stderr goes quiet before READY so nothing reaches the terminal after the
// launcher returns. A failed exchange is reported once on the original stderr.
pub(crate) fn notify_ready(endpoint: &Path) -> Result<(), String> {
    let terminal = detach_stderr();
    notify_ready_io(endpoint).map_err(|error| {
        let message = format!("Could not acknowledge browser startup: {error}");
        // Not eprintln!: it panics with EIO on a hung-up terminal.
        let _ = match terminal {
            Ok(terminal) => writeln!(std::fs::File::from(terminal), "{message}"),
            Err(_) => writeln!(io::stderr(), "{message}"),
        };
        message
    })
}

fn detach_stderr() -> io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::AsFd as _;
    let null = std::fs::OpenOptions::new().write(true).open("/dev/null")?;
    let mut stderr = io::stderr().lock();
    stderr.flush()?;
    let terminal = rustix::io::dup(stderr.as_fd())?;
    rustix::stdio::dup2_stderr(&null)?;
    Ok(terminal)
}

fn notify_ready_io(endpoint: &Path) -> io::Result<()> {
    let mut stream = UnixStream::connect(endpoint)?;
    stream.set_read_timeout(Some(ACK_TIMEOUT))?;
    stream.set_write_timeout(Some(ACK_TIMEOUT))?;
    stream.write_all(READY)?;
    let mut acknowledgement = [0; ACK.len()];
    stream.read_exact(&mut acknowledgement)?;
    if acknowledgement != ACK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid startup acknowledgement",
        ));
    }
    Ok(())
}

pub(crate) fn launch(args: &[OsString]) -> Result<i32, LaunchError> {
    let startup = StartupListener::new().map_err(LaunchError::Setup)?;
    let executable = std::env::current_exe().map_err(LaunchError::Setup)?;
    let mut child_args = Vec::with_capacity(args.len() + 2);
    child_args.push(executable.as_os_str().to_owned());
    child_args.push(STARTUP_SOCKET_ARG.into());
    child_args.push(startup.endpoint().into_os_string());
    child_args.extend(args.iter().skip(1).cloned());
    supervise(
        relaunch_command(executable.as_os_str(), &child_args),
        startup,
        STARTUP_TIMEOUT,
    )
}

struct StartupListener {
    directory: tempfile::TempDir,
    listener: UnixListener,
    connection: Option<UnixStream>,
    received: usize,
    message: [u8; READY.len()],
}

// Per-user runtime dir first: a shared or full /tmp must not block launch.
fn startup_directories() -> Vec<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let mut directories = Vec::new();
    for directory in runtime
        .into_iter()
        .chain([std::env::temp_dir(), PathBuf::from("/tmp")])
    {
        if !directories.contains(&directory) {
            directories.push(directory);
        }
    }
    directories
}

impl StartupListener {
    fn new() -> io::Result<Self> {
        Self::new_in(&startup_directories())
    }

    fn new_in(directories: &[PathBuf]) -> io::Result<Self> {
        let mut last_error = None;
        for parent in directories {
            if parent.as_os_str().len() + SOCKET_SUFFIX.len() > MAX_SOCKET_PATH {
                last_error = Some(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is too long for a socket path", parent.display()),
                ));
                continue;
            }
            match Self::bind_in(parent) {
                Ok(listener) => return Ok(listener),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| io::Error::other("no startup socket directory")))
    }

    fn bind_in(parent: &Path) -> io::Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("gc-start-")
            .tempdir_in(parent)?;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        let listener = UnixListener::bind(directory.path().join("ready"))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            directory,
            listener,
            connection: None,
            received: 0,
            message: [0; READY.len()],
        })
    }

    fn endpoint(&self) -> PathBuf {
        self.directory.path().join("ready")
    }

    fn poll(&mut self) -> io::Result<bool> {
        if self.connection.is_none() {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true)?;
                    self.connection = Some(stream);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        let stream = self
            .connection
            .as_mut()
            .expect("accepted startup connection");
        match stream.read(&mut self.message[self.received..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "startup connection closed before readiness",
                ));
            }
            Ok(count) => self.received += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Ok(false),
            Err(error) => return Err(error),
        }
        if self.message[..self.received] != READY[..self.received] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid browser startup message",
            ));
        }
        if self.received < READY.len() {
            return Ok(false);
        }
        // READY means the window is up: the browser owns the session even if
        // the ACK is lost, so a failed write must not get it killed.
        let _ = stream
            .set_nonblocking(false)
            .and_then(|()| stream.set_write_timeout(Some(ACK_TIMEOUT)))
            .and_then(|()| stream.write_all(ACK));
        Ok(true)
    }
}

struct StartupSignals {
    received: Arc<AtomicUsize>,
    registrations: Vec<signal_hook::SigId>,
    restore_defaults: Arc<AtomicBool>,
}

impl StartupSignals {
    fn new() -> io::Result<Self> {
        let mut signals = Self {
            received: Arc::new(AtomicUsize::new(0)),
            registrations: Vec::new(),
            restore_defaults: Arc::new(AtomicBool::new(false)),
        };
        for signal in [
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
        ] {
            // Kept for the process lifetime: unregistering alone would leave
            // the signal ignored if the browser then runs in the foreground.
            signal_hook::flag::register_conditional_default(
                signal,
                signals.restore_defaults.clone(),
            )?;
            signals
                .registrations
                .push(signal_hook::flag::register_usize(
                    signal,
                    signals.received.clone(),
                    signal as usize,
                )?);
        }
        Ok(signals)
    }
}

impl Drop for StartupSignals {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            signal_hook::low_level::unregister(registration);
        }
        self.restore_defaults.store(true, Ordering::Relaxed);
    }
}

struct PendingChild {
    child: Box<dyn ChildWrapper>,
    group: Pid,
    handed_off: bool,
}

impl PendingChild {
    fn forward_cancellation(&self, signal: Signal) -> io::Result<()> {
        kill_process_group(self.group, signal)?;
        let deadline = Instant::now() + CANCEL_GRACE;
        while Instant::now() < deadline {
            if waitid(
                WaitId::Pid(self.group),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )?
            .is_some()
            {
                break;
            }
            std::thread::sleep(SUPERVISOR_POLL);
        }
        Ok(())
    }
}

impl Drop for PendingChild {
    fn drop(&mut self) {
        if !self.handed_off {
            let _ = kill_process_group(self.group, Signal::KILL);
            let _ = self.child.wait();
        }
    }
}

fn supervise(
    mut command: Command,
    startup: StartupListener,
    timeout: Duration,
) -> Result<i32, LaunchError> {
    let signals = StartupSignals::new().map_err(LaunchError::Setup)?;
    command.stdin(Stdio::null()).stdout(Stdio::null());
    let child = CommandWrap::from(command)
        .wrap(ProcessSession)
        .spawn()
        .map_err(LaunchError::Setup)?;
    let group = Pid::from_raw(i32::try_from(child.id()).expect("Unix PID fits i32"))
        .expect("spawned child has a PID");
    let pending = PendingChild {
        child: child.into_inner(),
        group,
        handed_off: false,
    };
    await_startup(pending, startup, &signals, timeout).map_err(LaunchError::Startup)
}

fn await_startup(
    mut pending: PendingChild,
    mut startup: StartupListener,
    signals: &StartupSignals,
    timeout: Duration,
) -> io::Result<i32> {
    let group = pending.group;
    let deadline = Instant::now() + timeout;
    loop {
        let signal = signals.received.load(Ordering::Relaxed);
        if signal != 0 {
            let _ = pending.forward_cancellation(
                Signal::from_named_raw(signal as i32).expect("registered termination signal"),
            );
            return Ok(128 + signal as i32);
        }
        if startup.poll()? {
            pending.handed_off = true;
            return Ok(exit_code::SUCCESS);
        }
        if let Some(status) = waitid(
            WaitId::Pid(group),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )? {
            if startup.poll()? {
                pending.handed_off = true;
                return Ok(exit_code::SUCCESS);
            }
            return Err(io::Error::other(format!(
                "browser exited before confirming startup ({status:?})"
            )));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "browser startup was not confirmed within 60 seconds; the launched process was stopped",
            ));
        }
        std::thread::sleep(SUPERVISOR_POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

#[cfg(test)]
mod tests;
