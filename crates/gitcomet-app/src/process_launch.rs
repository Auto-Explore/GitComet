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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub(crate) const STARTUP_SOCKET_ARG: &str = "--__gitcomet-startup-socket";
const READY: &[u8] = b"READY 1\n";
const ACK: &[u8] = b"ACK 1\n";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const ACK_TIMEOUT: Duration = Duration::from_secs(2);
const SUPERVISOR_POLL: Duration = Duration::from_millis(10);
const CANCEL_GRACE: Duration = Duration::from_secs(1);

pub(crate) fn relaunch_command(executable: &OsStr, args: &[OsString]) -> Command {
    let mut command = Command::new(executable);
    command.args(args.iter().skip(1));
    command.env_remove("GITCOMET_SKIP_TERMINAL_DETACH");
    command
}

pub(crate) fn notify_ready(endpoint: &Path) -> Result<(), String> {
    notify_ready_io(endpoint)
        .map_err(|error| format!("Could not acknowledge browser startup: {error}"))
}

fn notify_ready_io(endpoint: &Path) -> io::Result<()> {
    let mut stream = UnixStream::connect(endpoint)?;
    stream.set_read_timeout(Some(ACK_TIMEOUT))?;
    stream.set_write_timeout(Some(ACK_TIMEOUT))?;
    let null = std::fs::OpenOptions::new().write(true).open("/dev/null")?;
    {
        let mut stderr = io::stderr().lock();
        stderr.flush()?;
        rustix::stdio::dup2_stderr(&null)?;
    }
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

pub(crate) fn launch(args: &[OsString]) -> io::Result<i32> {
    let startup = StartupListener::new()?;
    let executable = std::env::current_exe()?;
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

impl StartupListener {
    fn new() -> io::Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("gc-start-")
            .tempdir_in("/tmp")?;
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
        stream.set_nonblocking(false)?;
        stream.set_write_timeout(Some(ACK_TIMEOUT))?;
        stream.write_all(ACK)?;
        Ok(true)
    }
}

struct StartupSignals {
    received: Arc<AtomicUsize>,
    registrations: Vec<signal_hook::SigId>,
}

impl StartupSignals {
    fn new() -> io::Result<Self> {
        let mut signals = Self {
            received: Arc::new(AtomicUsize::new(0)),
            registrations: Vec::new(),
        };
        for signal in [
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
        ] {
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
    mut startup: StartupListener,
    timeout: Duration,
) -> io::Result<i32> {
    let signals = StartupSignals::new()?;
    command.stdin(Stdio::null()).stdout(Stdio::null());
    let child = CommandWrap::from(command).wrap(ProcessSession).spawn()?;
    let group = Pid::from_raw(i32::try_from(child.id()).expect("Unix PID fits i32"))
        .expect("spawned child has a PID");
    let mut pending = PendingChild {
        child: child.into_inner(),
        group,
        handed_off: false,
    };
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
