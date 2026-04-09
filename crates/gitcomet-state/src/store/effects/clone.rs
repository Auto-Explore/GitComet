use crate::msg::Msg;
use gitcomet_core::auth::{
    CachedPassphraseEntry, GITCOMET_AUTH_CACHE_PROMPT_ENV_PREFIX,
    GITCOMET_AUTH_CACHE_SECRET_ENV_PREFIX, GITCOMET_AUTH_CACHE_SIZE_ENV, GITCOMET_AUTH_KIND_ENV,
    GITCOMET_AUTH_KIND_HOST_VERIFICATION, GITCOMET_AUTH_KIND_PASSPHRASE,
    GITCOMET_AUTH_KIND_PASSPHRASE_CACHED, GITCOMET_AUTH_KIND_USERNAME_PASSWORD,
    GITCOMET_AUTH_SECRET_ENV, GITCOMET_AUTH_USERNAME_ENV, GitAuthKind, StagedGitAuth,
    load_session_passphrases, remember_passphrase_prompt_from_staged_git_auth,
    take_staged_git_auth,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::process::configure_background_command;
use gitcomet_core::services::CommandOutput;
use std::fs;
use std::io::{BufRead as _, BufReader, Read as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::super::executor::TaskExecutor;
use super::util::send_or_log;

const GIT_COMMAND_TIMEOUT_ENV: &str = "GITCOMET_GIT_COMMAND_TIMEOUT_SECS";
const GIT_COMMAND_TIMEOUT_DEFAULT_SECS: u64 = 300;
const GIT_COMMAND_WAIT_POLL: Duration = Duration::from_millis(100);
const GITCOMET_ASKPASS_PROMPT_LOG_ENV: &str = "GITCOMET_ASKPASS_PROMPT_LOG";
const GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG_ENV: &str = "GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG";
const ALLOWED_CLONE_URL_SCHEMES: [&str; 4] = ["https", "ssh", "git", "file"];

struct AskPassScript {
    _dir: tempfile::TempDir,
    path: PathBuf,
    host_prompt_log_path: PathBuf,
    passphrase_prompt_log_path: PathBuf,
}

#[derive(Clone, Eq, PartialEq)]
enum PromptAuth {
    Explicit(StagedGitAuth),
    CachedPassphrases(Vec<CachedPassphraseEntry>),
}

impl PromptAuth {
    fn from_explicit(auth: StagedGitAuth) -> Option<Self> {
        if auth.secret.is_empty() {
            return None;
        }
        Some(Self::Explicit(auth))
    }

    fn from_cached_passphrases(passphrases: Vec<CachedPassphraseEntry>) -> Option<Self> {
        if passphrases.is_empty() {
            return None;
        }
        Some(Self::CachedPassphrases(passphrases))
    }

    fn kind_env(&self) -> &'static str {
        match self {
            Self::Explicit(auth) => match auth.kind {
                GitAuthKind::UsernamePassword => GITCOMET_AUTH_KIND_USERNAME_PASSWORD,
                GitAuthKind::Passphrase => GITCOMET_AUTH_KIND_PASSPHRASE,
                GitAuthKind::HostVerification => GITCOMET_AUTH_KIND_HOST_VERIFICATION,
            },
            Self::CachedPassphrases(_) => GITCOMET_AUTH_KIND_PASSPHRASE_CACHED,
        }
    }

    fn username(&self) -> Option<&str> {
        match self {
            Self::Explicit(auth) => auth.username.as_deref(),
            Self::CachedPassphrases(_) => None,
        }
    }

    fn secret(&self) -> &str {
        match self {
            Self::Explicit(auth) => &auth.secret,
            Self::CachedPassphrases(_) => "",
        }
    }

    fn remember_on_success(&self, prompt: Option<&str>) {
        if let Self::Explicit(auth) = self {
            remember_passphrase_prompt_from_staged_git_auth(auth, prompt);
        }
    }
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

fn take_pending_git_auth() -> Option<PromptAuth> {
    take_staged_git_auth()
        .and_then(PromptAuth::from_explicit)
        .or_else(|| {
            let passphrases = load_session_passphrases();
            PromptAuth::from_cached_passphrases(passphrases)
        })
}

fn resolve_git_auth(auth: Option<StagedGitAuth>) -> Option<PromptAuth> {
    auth.and_then(PromptAuth::from_explicit)
        .or_else(take_pending_git_auth)
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
if [ -n "${GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG:-}" ]; then
  case "$lower_prompt" in
    *passphrase*)
      printf '%s\n' "$prompt" >> "${GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG}" ;;
  esac
fi
kind="${GITCOMET_AUTH_KIND:-}"
if [ "$kind" = "username_password" ]; then
  case "$lower_prompt" in
    *username*) printf '%s\n' "${GITCOMET_AUTH_USERNAME:-}" ;;
    *) printf '%s\n' "${GITCOMET_AUTH_SECRET:-}" ;;
  esac
elif [ "$kind" = "passphrase_cached" ]; then
  cache_size="${GITCOMET_AUTH_CACHE_SIZE:-0}"
  i=0
  while [ "$i" -lt "$cache_size" ]; do
    cached_prompt=$(printenv "GITCOMET_AUTH_CACHE_PROMPT_$i")
    if [ "$prompt" = "$cached_prompt" ]; then
      printenv "GITCOMET_AUTH_CACHE_SECRET_$i"
      exit 0
    fi
    i=$((i + 1))
  done
  printf '\n'
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
if not "%GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG%"=="" (
  echo %prompt% | findstr /I "passphrase" >nul
  if not errorlevel 1 (
    >>"%GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG%" echo %prompt%
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
if /I "%GITCOMET_AUTH_KIND%"=="passphrase_cached" (
  set "cache_size=%GITCOMET_AUTH_CACHE_SIZE%"
  if "!cache_size!"=="" set "cache_size=0"
  set /a cache_last=!cache_size!-1
  if !cache_last! GEQ 0 (
    for /L %%i in (0,1,!cache_last!) do (
      call set "cached_prompt=%%GITCOMET_AUTH_CACHE_PROMPT_%%i%%"
      if "!prompt!"=="!cached_prompt!" (
        call set "cached_secret=%%GITCOMET_AUTH_CACHE_SECRET_%%i%%"
        echo !cached_secret!
        exit /b 0
      )
    )
  )
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
    let host_prompt_log_path = dir.path().join("gitcomet-askpass-host-prompt.log");
    let passphrase_prompt_log_path = dir.path().join("gitcomet-askpass-passphrase-prompt.log");

    fs::write(&path, askpass_script_contents()).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    fs::write(&host_prompt_log_path, b"").map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
    fs::write(&passphrase_prompt_log_path, b"").map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
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
        host_prompt_log_path,
        passphrase_prompt_log_path,
    })
}

fn configure_clone_auth_prompt(
    cmd: &mut Command,
    auth: Option<&PromptAuth>,
    askpass: &AskPassScript,
) {
    cmd.env("GIT_ASKPASS", &askpass.path);
    cmd.env("SSH_ASKPASS", &askpass.path);
    cmd.env("SSH_ASKPASS_REQUIRE", "force");
    cmd.env(
        GITCOMET_ASKPASS_PROMPT_LOG_ENV,
        &askpass.host_prompt_log_path,
    );
    cmd.env(
        GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG_ENV,
        &askpass.passphrase_prompt_log_path,
    );
    if cfg!(all(unix, not(target_os = "macos"))) && std::env::var_os("DISPLAY").is_none() {
        cmd.env("DISPLAY", "gitcomet:0");
    }

    cmd.env(GITCOMET_AUTH_CACHE_SIZE_ENV, "0");
    if let Some(auth) = auth {
        match auth {
            PromptAuth::Explicit(_) => {
                cmd.env(GITCOMET_AUTH_KIND_ENV, auth.kind_env());
                if let Some(username) = auth.username() {
                    cmd.env(GITCOMET_AUTH_USERNAME_ENV, username);
                } else {
                    cmd.env_remove(GITCOMET_AUTH_USERNAME_ENV);
                }
                cmd.env(GITCOMET_AUTH_SECRET_ENV, auth.secret());
            }
            PromptAuth::CachedPassphrases(entries) => {
                cmd.env(GITCOMET_AUTH_KIND_ENV, auth.kind_env());
                cmd.env_remove(GITCOMET_AUTH_USERNAME_ENV);
                cmd.env_remove(GITCOMET_AUTH_SECRET_ENV);
                cmd.env(GITCOMET_AUTH_CACHE_SIZE_ENV, entries.len().to_string());
                for (idx, entry) in entries.iter().enumerate() {
                    cmd.env(
                        format!("{GITCOMET_AUTH_CACHE_PROMPT_ENV_PREFIX}{idx}"),
                        &entry.prompt,
                    );
                    cmd.env(
                        format!("{GITCOMET_AUTH_CACHE_SECRET_ENV_PREFIX}{idx}"),
                        &entry.secret,
                    );
                }
            }
        }
    } else {
        cmd.env_remove(GITCOMET_AUTH_KIND_ENV);
        cmd.env_remove(GITCOMET_AUTH_USERNAME_ENV);
        cmd.env_remove(GITCOMET_AUTH_SECRET_ENV);
    }
}

fn last_logged_passphrase_prompt(askpass: &AskPassScript) -> Option<String> {
    let raw = fs::read_to_string(&askpass.passphrase_prompt_log_path).ok()?;
    raw.lines()
        .rev()
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn remember_successful_prompt_auth(auth: Option<&PromptAuth>, askpass: &AskPassScript) {
    if let Some(auth) = auth {
        auth.remember_on_success(last_logged_passphrase_prompt(askpass).as_deref());
    }
}

fn append_host_prompt_to_stderr(stderr: &mut String, askpass: &AskPassScript) {
    let Ok(raw_prompt_log) = fs::read_to_string(&askpass.host_prompt_log_path) else {
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
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_LFS_SKIP_SMUDGE", "1");

        let (askpass_script, prompt_auth) = match (|| {
            let auth = resolve_git_auth(auth);
            let script = create_askpass_script()?;
            configure_clone_auth_prompt(&mut cmd, auth.as_ref(), &script);
            Ok::<(AskPassScript, Option<PromptAuth>), Error>((script, auth))
        })() {
            Ok(context) => context,
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
        let stdout = child.stdout.take();
        let stdout_handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stdout) = stdout {
                let _ = stdout.read_to_end(&mut buf);
            }
            bytes_to_text_preserving_utf8(&buf)
        });

        let stderr = child.stderr.take();
        let progress_dest = Arc::new(dest.clone());
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
                            dest: Arc::clone(&progress_dest),
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

        if result.is_ok() {
            remember_successful_prompt_auth(prompt_auth.as_ref(), &askpass_script);
        }

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
    use std::process::Command;

    fn command_env_value(cmd: &Command, key: &str) -> Option<String> {
        use std::ffi::OsStr;

        cmd.get_envs().find_map(|(k, v)| {
            if k == OsStr::new(key) {
                v.and_then(|value| value.to_str().map(ToOwned::to_owned))
            } else {
                None
            }
        })
    }

    fn command_env_removed(cmd: &Command, key: &str) -> bool {
        use std::ffi::OsStr;

        cmd.get_envs()
            .any(|(k, v)| k == OsStr::new(key) && v.is_none())
    }

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
            &askpass.host_prompt_log_path,
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
        std::fs::write(&askpass.host_prompt_log_path, prompt).expect("write prompt log");

        let mut stderr = format!("Host key verification failed.\n{prompt}\n");
        append_host_prompt_to_stderr(&mut stderr, &askpass);

        assert_eq!(stderr.matches("SSH host verification prompt:").count(), 0);
        assert_eq!(stderr.matches(prompt).count(), 1);
    }

    #[test]
    fn configure_clone_auth_prompt_sets_cached_passphrase_env_and_removes_username() {
        let askpass = create_askpass_script().expect("askpass script creation");
        let mut cmd = Command::new("git");
        cmd.env(GITCOMET_AUTH_USERNAME_ENV, "legacy-user");
        let auth = PromptAuth::CachedPassphrases(vec![
            CachedPassphraseEntry {
                prompt: "Enter passphrase for key '/tmp/key-a':".to_string(),
                secret: "ssh-passphrase-a".to_string(),
            },
            CachedPassphraseEntry {
                prompt: "Enter passphrase for key '/tmp/key-b':".to_string(),
                secret: "ssh-passphrase-b".to_string(),
            },
        ]);

        configure_clone_auth_prompt(&mut cmd, Some(&auth), &askpass);

        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_KIND_ENV).as_deref(),
            Some(GITCOMET_AUTH_KIND_PASSPHRASE_CACHED)
        );
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_USERNAME_ENV));
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_SECRET_ENV));
        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_CACHE_SIZE_ENV).as_deref(),
            Some("2")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_PROMPT_0").as_deref(),
            Some("Enter passphrase for key '/tmp/key-a':")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_SECRET_0").as_deref(),
            Some("ssh-passphrase-a")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_PROMPT_1").as_deref(),
            Some("Enter passphrase for key '/tmp/key-b':")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_SECRET_1").as_deref(),
            Some("ssh-passphrase-b")
        );
    }
}
