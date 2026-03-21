use crate::msg::Msg;
use gitcomet_core::auth::{
    GITCOMET_AUTH_KIND_ENV, GITCOMET_AUTH_KIND_HOST_VERIFICATION, GITCOMET_AUTH_KIND_PASSPHRASE,
    GITCOMET_AUTH_KIND_USERNAME_PASSWORD, GITCOMET_AUTH_SECRET_ENV, GITCOMET_AUTH_USERNAME_ENV,
    GitAuthKind, StagedGitAuth, take_staged_git_auth,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::process::configure_background_command;
use gitcomet_core::services::CommandOutput;
use std::fs;
use std::io::{BufRead as _, BufReader, Read as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::super::executor::TaskExecutor;
use super::util::send_or_log;

const GIT_COMMAND_TIMEOUT_ENV: &str = "GITCOMET_GIT_COMMAND_TIMEOUT_SECS";
const GIT_COMMAND_TIMEOUT_DEFAULT_SECS: u64 = 300;
const GIT_COMMAND_WAIT_POLL: Duration = Duration::from_millis(100);
const GITCOMET_ASKPASS_PROMPT_LOG_ENV: &str = "GITCOMET_ASKPASS_PROMPT_LOG";
const ALLOWED_CLONE_URL_SCHEMES: [&str; 4] = ["https", "ssh", "git", "file"];

struct AskPassScript {
    _dir: tempfile::TempDir,
    path: PathBuf,
    prompt_log_path: PathBuf,
}

fn bytes_to_text_preserving_utf8(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(bytes.len());
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match std::str::from_utf8(&bytes[cursor..]) {
            Ok(valid) => {
                out.push_str(valid);
                break;
            }
            Err(err) => {
                let valid_len = err.valid_up_to();
                if valid_len > 0 {
                    let valid = &bytes[cursor..cursor + valid_len];
                    out.push_str(
                        std::str::from_utf8(valid)
                            .expect("slice identified by valid_up_to must be valid UTF-8"),
                    );
                    cursor += valid_len;
                }

                let invalid_len = err.error_len().unwrap_or(1);
                let invalid_end = cursor.saturating_add(invalid_len).min(bytes.len());
                for byte in &bytes[cursor..invalid_end] {
                    let _ = write!(out, "\\x{byte:02x}");
                }
                cursor = invalid_end;
            }
        }
    }

    out
}

fn git_command_timeout() -> Duration {
    std::env::var(GIT_COMMAND_TIMEOUT_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(GIT_COMMAND_TIMEOUT_DEFAULT_SECS))
}

fn is_windows_drive_path(url: &str) -> bool {
    let bytes = url.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn explicit_url_scheme_end(url: &str) -> Option<usize> {
    if is_windows_drive_path(url) {
        return None;
    }

    let mut chars = url.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }

    for (idx, ch) in chars {
        if ch == ':' {
            return Some(idx);
        }
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.')) {
            return None;
        }
    }

    None
}

fn validate_clone_url(url: &str) -> Result<(), Error> {
    let url = url.trim();
    if url.is_empty() {
        return Err(Error::new(ErrorKind::Backend(
            "clone URL cannot be empty".to_string(),
        )));
    }

    let Some(scheme_end) = explicit_url_scheme_end(url) else {
        return Ok(());
    };

    let scheme = url[..scheme_end].to_ascii_lowercase();
    if !ALLOWED_CLONE_URL_SCHEMES.contains(&scheme.as_str()) {
        return Err(Error::new(ErrorKind::Backend(format!(
            "unsupported clone URL scheme `{scheme}` (allowed: https, ssh, git, file)"
        ))));
    }

    if !url[scheme_end..].starts_with("://") {
        return Err(Error::new(ErrorKind::Backend(format!(
            "invalid clone URL format for `{scheme}`; expected `{scheme}://...`"
        ))));
    }

    Ok(())
}

fn take_pending_git_auth() -> Option<StagedGitAuth> {
    let auth = take_staged_git_auth()?;
    if auth.secret.is_empty() {
        return None;
    }
    Some(auth)
}

#[cfg(unix)]
fn askpass_script_contents() -> &'static [u8] {
    br#"#!/bin/sh
prompt="$1"
lower_prompt=$(printf '%s' "$prompt" | tr '[:upper:]' '[:lower:]')
if [ -n "${GITCOMET_ASKPASS_PROMPT_LOG:-}" ]; then
  case "$lower_prompt" in
    *authenticity\ of\ host*|*continue\ connecting*|*yes/no*|*fingerprint*)
      printf '%s\n' "$prompt" >> "${GITCOMET_ASKPASS_PROMPT_LOG}" ;;
  esac
fi
kind="${GITCOMET_AUTH_KIND:-}"
if [ "$kind" = "username_password" ]; then
  case "$lower_prompt" in
    *username*) printf '%s\n' "${GITCOMET_AUTH_USERNAME:-}" ;;
    *) printf '%s\n' "${GITCOMET_AUTH_SECRET:-}" ;;
  esac
elif [ "$kind" = "host_verification" ]; then
  case "$lower_prompt" in
    *continue\ connecting*|*yes/no*|*fingerprint*) printf '%s\n' "${GITCOMET_AUTH_SECRET:-}" ;;
    *) printf '\n' ;;
  esac
else
  printf '%s\n' "${GITCOMET_AUTH_SECRET:-}"
fi
"#
}

#[cfg(windows)]
fn askpass_script_contents() -> &'static [u8] {
    br#"@echo off
setlocal EnableDelayedExpansion
set "prompt=%~1"
if not "%GITCOMET_ASKPASS_PROMPT_LOG%"=="" (
  echo %prompt% | findstr /I /C:"authenticity of host" /C:"continue connecting" /C:"yes/no" /C:"fingerprint" >nul
  if not errorlevel 1 (
    >>"%GITCOMET_ASKPASS_PROMPT_LOG%" echo %prompt%
  )
)
if /I "%GITCOMET_AUTH_KIND%"=="username_password" (
  echo %prompt% | findstr /I "username" >nul
  if not errorlevel 1 (
    echo %GITCOMET_AUTH_USERNAME%
    exit /b 0
  )
  echo %GITCOMET_AUTH_SECRET%
  exit /b 0
)
if /I "%GITCOMET_AUTH_KIND%"=="host_verification" (
  echo %prompt% | findstr /I /C:"continue connecting" /C:"yes/no" /C:"fingerprint" >nul
  if not errorlevel 1 (
    echo %GITCOMET_AUTH_SECRET%
  )
  exit /b 0
)
echo %GITCOMET_AUTH_SECRET%
"#
}

fn create_askpass_script() -> Result<AskPassScript, Error> {
    let dir = tempfile::tempdir().map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    #[cfg(windows)]
    let script_name = "gitcomet-askpass.cmd";
    #[cfg(not(windows))]
    let script_name = "gitcomet-askpass.sh";
    let path = dir.path().join(script_name);
    let prompt_log_path = dir.path().join("gitcomet-askpass-prompt.log");

    fs::write(&path, askpass_script_contents()).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    fs::write(&prompt_log_path, b"").map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let mut permissions = fs::metadata(&path)
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    }
    Ok(AskPassScript {
        _dir: dir,
        path,
        prompt_log_path,
    })
}

fn configure_clone_auth_prompt(
    cmd: &mut Command,
    auth: Option<&StagedGitAuth>,
    askpass: &AskPassScript,
) {
    cmd.env("GIT_ASKPASS", &askpass.path);
    cmd.env("SSH_ASKPASS", &askpass.path);
    cmd.env("SSH_ASKPASS_REQUIRE", "force");
    cmd.env(GITCOMET_ASKPASS_PROMPT_LOG_ENV, &askpass.prompt_log_path);
    if cfg!(all(unix, not(target_os = "macos"))) && std::env::var_os("DISPLAY").is_none() {
        cmd.env("DISPLAY", "gitcomet:0");
    }

    if let Some(auth) = auth {
        let kind = match auth.kind {
            GitAuthKind::UsernamePassword => GITCOMET_AUTH_KIND_USERNAME_PASSWORD,
            GitAuthKind::Passphrase => GITCOMET_AUTH_KIND_PASSPHRASE,
            GitAuthKind::HostVerification => GITCOMET_AUTH_KIND_HOST_VERIFICATION,
        };
        cmd.env(GITCOMET_AUTH_KIND_ENV, kind);
        if let Some(username) = &auth.username {
            cmd.env(GITCOMET_AUTH_USERNAME_ENV, username);
        } else {
            cmd.env_remove(GITCOMET_AUTH_USERNAME_ENV);
        }
        cmd.env(GITCOMET_AUTH_SECRET_ENV, &auth.secret);
    } else {
        cmd.env_remove(GITCOMET_AUTH_KIND_ENV);
        cmd.env_remove(GITCOMET_AUTH_USERNAME_ENV);
        cmd.env_remove(GITCOMET_AUTH_SECRET_ENV);
    }
}

fn append_host_prompt_to_stderr(stderr: &mut String, askpass: &AskPassScript) {
    let Ok(raw_prompt_log) = fs::read_to_string(&askpass.prompt_log_path) else {
        return;
    };
    let prompt_log = raw_prompt_log.trim();
    if prompt_log.is_empty() {
        return;
    }
    if stderr.contains(prompt_log) {
        return;
    }

    if !stderr.is_empty() && !stderr.ends_with('\n') {
        stderr.push('\n');
    }
    stderr.push_str("SSH host verification prompt:\n");
    stderr.push_str(prompt_log);
    stderr.push('\n');
}

pub(super) fn schedule_clone_repo(
    executor: &TaskExecutor,
    msg_tx: mpsc::Sender<Msg>,
    url: String,
    dest: PathBuf,
    auth: Option<StagedGitAuth>,
) {
    executor.spawn(move || {
        if let Err(err) = validate_clone_url(&url) {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                    url,
                    dest,
                    result: Err(err),
                }),
            );
            return;
        }

        let mut cmd = Command::new("git");
        configure_background_command(&mut cmd);
        cmd.arg("-c")
            .arg("color.ui=false")
            .arg("clone")
            .arg("--progress")
            .arg(&url)
            .arg(&dest)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0");

        let askpass_script = match (|| {
            let auth = auth.or_else(take_pending_git_auth);
            let script = create_askpass_script()?;
            configure_clone_auth_prompt(&mut cmd, auth.as_ref(), &script);
            Ok::<AskPassScript, Error>(script)
        })() {
            Ok(script) => script,
            Err(err) => {
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                        url: url.clone(),
                        dest: dest.clone(),
                        result: Err(err),
                    }),
                );
                return;
            }
        };

        let command_str = format!("git clone --progress {} {}", url, dest.display());

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                let err = Error::new(ErrorKind::Io(e.kind()));
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                        url,
                        dest,
                        result: Err(err),
                    }),
                );
                return;
            }
        };
        let askpass_script = askpass_script;

        let stdout = child.stdout.take();
        let stdout_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stdout) = stdout {
                let _ = stdout.read_to_end(&mut buf);
            }
            bytes_to_text_preserving_utf8(&buf)
        });

        let stderr = child.stderr.take();
        let progress_dest = dest.clone();
        let progress_tx = msg_tx.clone();
        let stderr_handle = std::thread::spawn(move || {
            let mut stderr_acc = String::new();
            if let Some(stderr) = stderr {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    stderr_acc.push_str(&line);
                    stderr_acc.push('\n');
                    send_or_log(
                        &progress_tx,
                        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
                            dest: progress_dest.clone(),
                            line,
                        }),
                    );
                }
            }
            stderr_acc
        });

        let timeout = git_command_timeout();
        let start = Instant::now();
        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        timed_out = true;
                        let _ = child.kill();
                        break child.wait();
                    }
                    std::thread::sleep(GIT_COMMAND_WAIT_POLL);
                }
                Err(e) => break Err(e),
            }
        };
        let stdout_str = stdout_handle.join().unwrap_or_default();
        let mut stderr_acc = stderr_handle.join().unwrap_or_default();
        append_host_prompt_to_stderr(&mut stderr_acc, &askpass_script);

        let result = match status {
            Ok(status) => {
                if timed_out {
                    Err(Error::new(ErrorKind::Backend(format!(
                        "{command_str} timed out after {} seconds (set {GIT_COMMAND_TIMEOUT_ENV} to override)",
                        timeout.as_secs()
                    ))))
                } else {
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
            }
            Err(e) => Err(Error::new(ErrorKind::Io(e.kind()))),
        };

        let ok = result.is_ok();
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                url: url.clone(),
                dest: dest.clone(),
                result,
            }),
        );

        if ok {
            send_or_log(&msg_tx, Msg::OpenRepo(dest));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_clone_url_accepts_allowlisted_schemes() {
        assert!(validate_clone_url("https://example.com/org/repo.git").is_ok());
        assert!(validate_clone_url("ssh://git@example.com/org/repo.git").is_ok());
        assert!(validate_clone_url("git://example.com/org/repo.git").is_ok());
        assert!(validate_clone_url("file:///tmp/repo.git").is_ok());
    }

    #[test]
    fn validate_clone_url_rejects_unallowlisted_schemes() {
        assert!(validate_clone_url("ext::sh -c touch /tmp/pwned").is_err());
        assert!(validate_clone_url("http://example.com/org/repo.git").is_err());
    }

    #[test]
    fn validate_clone_url_keeps_schemeless_inputs_working() {
        assert!(validate_clone_url("/tmp/repo.git").is_ok());
        assert!(validate_clone_url("git@github.com:org/repo.git").is_ok());
        assert!(validate_clone_url("C:\\repos\\repo.git").is_ok());
    }

    #[test]
    fn validate_clone_url_rejects_malformed_allowlisted_schemes() {
        assert!(validate_clone_url("ssh:git@example.com/org/repo.git").is_err());
    }

    #[test]
    fn append_host_prompt_to_stderr_includes_logged_prompt_with_fingerprint() {
        let askpass = create_askpass_script().expect("askpass script creation");
        std::fs::write(
            &askpass.prompt_log_path,
            "The authenticity of host 'github.com (140.82.121.3)' can't be established.\nED25519 key fingerprint is: SHA256:+DiY...\nAre you sure you want to continue connecting (yes/no/[fingerprint])?",
        )
        .expect("write prompt log");

        let mut stderr = "Host key verification failed.\n".to_string();
        append_host_prompt_to_stderr(&mut stderr, &askpass);

        assert!(stderr.contains("SSH host verification prompt:"));
        assert!(stderr.contains("ED25519 key fingerprint is: SHA256:+DiY..."));
        assert!(stderr.contains("yes/no/[fingerprint]"));
    }

    #[test]
    fn append_host_prompt_to_stderr_skips_when_prompt_already_present() {
        let askpass = create_askpass_script().expect("askpass script creation");
        let prompt = "Are you sure you want to continue connecting (yes/no/[fingerprint])?";
        std::fs::write(&askpass.prompt_log_path, prompt).expect("write prompt log");

        let mut stderr = format!("Host key verification failed.\n{prompt}\n");
        append_host_prompt_to_stderr(&mut stderr, &askpass);

        assert_eq!(stderr.matches("SSH host verification prompt:").count(), 0);
        assert_eq!(stderr.matches(prompt).count(), 1);
    }
}
