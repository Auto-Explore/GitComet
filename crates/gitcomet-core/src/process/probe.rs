//! Bounded, cancellable output collection for executable diagnostics.
use crate::services::CancellationToken;
use process_wrap::std::*;
use std::io::{self, Read};
use std::process::{Command, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(crate) fn probe_output(
    mut command: Command,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> io::Result<Output> {
    if cancellation.is_cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut command = CommandWrap::from(command);
    #[cfg(windows)]
    {
        let mut flags = CreationFlags(Default::default());
        flags.0.0 = 0x0800_0000; // CREATE_NO_WINDOW
        command.wrap(flags).wrap(JobObject);
    }
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    let deadline = Instant::now() + timeout;
    let mut child = command.spawn()?;
    let stdout = reader(child.stdout().take());
    let stderr = reader(child.stderr().take());
    drop(command);
    let mut status = None;
    let failure = loop {
        if cancellation.is_cancelled() {
            break Some(io::Error::from(io::ErrorKind::Interrupted));
        }
        if Instant::now() >= deadline {
            break Some(io::Error::from(io::ErrorKind::TimedOut));
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(error) => break Some(error),
            }
        }
        if status.is_some() && stdout.is_finished() && stderr.is_finished() {
            break None;
        }
        thread::sleep(Duration::from_millis(5));
    };
    if let Some(error) = failure {
        // Own the group/job until inherited pipes close, even if its leader exited.
        let _ = child.kill();
        let _ = child.wait();
        let _ = stdout.join();
        let _ = stderr.join();
        return Err(error);
    }
    Ok(Output {
        status: status.expect("completed child"),
        stdout: stdout.join().unwrap_or_else(|_| Ok(Vec::new()))?,
        stderr: stderr.join().unwrap_or_else(|_| Ok(Vec::new()))?,
    })
}

fn reader<R: Read + Send + 'static>(pipe: Option<R>) -> JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut output = Vec::new();
        if let Some(mut pipe) = pipe {
            // Drain all output, retaining at most 64 KiB of diagnostic text.
            let mut buffer = [0u8; 8192];
            loop {
                let count = pipe.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                let keep = count.min((64 * 1024usize).saturating_sub(output.len()));
                output.extend_from_slice(&buffer[..keep]);
            }
        }
        Ok(output)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_command(mode: &str, marker: &std::path::Path) -> Command {
        let mut command = crate::process::background_command(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "process::probe::tests::probe_fixture_child",
                "--nocapture",
            ])
            .env("GITCOMET_PROBE_FIXTURE", mode)
            .env("GITCOMET_PROBE_MARKER", marker);
        command
    }

    #[test]
    #[ignore = "subprocess fixture"]
    #[allow(clippy::zombie_processes)] // This fixture deliberately exits before its descendant.
    fn probe_fixture_child() {
        let mode = std::env::var("GITCOMET_PROBE_FIXTURE").unwrap();
        let marker = std::path::PathBuf::from(std::env::var_os("GITCOMET_PROBE_MARKER").unwrap());
        if mode == "parent" {
            let _child = fixture_command("grandchild", &marker).spawn().unwrap();
            return; // The grandchild still owns the inherited output pipes.
        }
        std::fs::write(marker.with_extension("started"), b"started").unwrap();
        thread::sleep(Duration::from_secs(2));
        std::fs::write(marker, b"escaped cancellation").unwrap();
    }

    #[test]
    fn deadline_includes_inherited_pipes_and_terminates_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("escaped");
        let started = Instant::now();
        let result = probe_output(
            fixture_command("parent", &marker),
            Duration::from_millis(500),
            &CancellationToken::new(),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(marker.with_extension("started").exists());
        thread::sleep(Duration::from_secs(2));
        assert!(!marker.exists(), "descendant escaped the owned group/job");
    }

    #[test]
    fn cancellation_stops_a_running_probe_and_prevents_future_spawns() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("cancelled");
        let cancellation = CancellationToken::new();
        let token = cancellation.clone();
        let command = fixture_command("grandchild", &marker);
        let work = thread::spawn(move || probe_output(command, Duration::from_secs(5), &token));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !marker.with_extension("started").exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        cancellation.cancel();
        assert_eq!(
            work.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert_eq!(
            probe_output(
                fixture_command("grandchild", &marker),
                Duration::from_secs(5),
                &cancellation
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Interrupted
        );
        assert!(!marker.exists());
    }
}
