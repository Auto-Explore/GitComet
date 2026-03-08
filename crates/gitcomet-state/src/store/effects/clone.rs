use crate::msg::Msg;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::CommandOutput;
use std::io::{BufRead as _, BufReader, Read as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;

use super::super::executor::TaskExecutor;
use super::util::send_or_log;

pub(super) fn schedule_clone_repo(
    executor: &TaskExecutor,
    msg_tx: mpsc::Sender<Msg>,
    url: String,
    dest: PathBuf,
) {
    executor.spawn(move || {
        let mut cmd = Command::new("git");
        cmd.arg("-c")
            .arg("color.ui=false")
            .arg("clone")
            .arg("--progress")
            .arg(&url)
            .arg(&dest)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let command_str = format!("git clone --progress {} {}", url, dest.display());

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                let err = Error::new(ErrorKind::Io(e.kind()));
                send_or_log(
                    &msg_tx,
                    Msg::CloneRepoFinished {
                        url,
                        dest,
                        result: Err(err),
                    },
                );
                return;
            }
        };

        let stdout = child.stdout.take();
        let stdout_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stdout) = stdout {
                let _ = stdout.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        });

        let stderr = child.stderr.take();
        let mut stderr_acc = String::new();
        if let Some(stderr) = stderr {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                stderr_acc.push_str(&line);
                stderr_acc.push('\n');
                send_or_log(
                    &msg_tx,
                    Msg::CloneRepoProgress {
                        dest: dest.clone(),
                        line,
                    },
                );
            }
        }

        let status = child.wait();
        let stdout_str = stdout_handle.join().unwrap_or_default();

        let result = match status {
            Ok(status) => {
                let out = CommandOutput {
                    command: command_str,
                    stdout: stdout_str,
                    stderr: stderr_acc,
                    exit_code: status.code(),
                };
                if status.success() {
                    Ok(out)
                } else {
                    let combined = out.combined();
                    let message = if combined.is_empty() {
                        format!("{} failed", out.command)
                    } else {
                        format!("{} failed: {combined}", out.command)
                    };
                    Err(Error::new(ErrorKind::Backend(message)))
                }
            }
            Err(e) => Err(Error::new(ErrorKind::Io(e.kind()))),
        };

        let ok = result.is_ok();
        send_or_log(
            &msg_tx,
            Msg::CloneRepoFinished {
                url: url.clone(),
                dest: dest.clone(),
                result,
            },
        );

        if ok {
            send_or_log(&msg_tx, Msg::OpenRepo(dest));
        }
    });
}
