//! Askpass helper script support shared by the gix backend and the state
//! clone effect.
//!
//! Git and SSH run an askpass helper to obtain credentials without a TTY.
//! GitComet stages the user's credentials in [`super::stage_git_auth`] and
//! hands them to the helper through environment variables, so secrets never
//! appear on a command line. Prompt text the helper echoes back to its log
//! files is used to cache passphrases by prompt and to surface SSH
//! host-verification prompts in the command's stderr.

use super::{
    CachedPassphraseEntry, GITCOMET_AUTH_CACHE_PROMPT_ENV_PREFIX,
    GITCOMET_AUTH_CACHE_SECRET_ENV_PREFIX, GITCOMET_AUTH_CACHE_SIZE_ENV, GITCOMET_AUTH_KIND_ENV,
    GITCOMET_AUTH_KIND_HOST_VERIFICATION, GITCOMET_AUTH_KIND_PASSPHRASE,
    GITCOMET_AUTH_KIND_PASSPHRASE_CACHED, GITCOMET_AUTH_KIND_USERNAME_PASSWORD,
    GITCOMET_AUTH_SECRET_ENV, GITCOMET_AUTH_USERNAME_ENV, GitAuthKind, StagedGitAuth,
    load_session_passphrases, remember_passphrase_prompt_from_staged_git_auth,
    take_staged_git_auth,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const GIT_COMMAND_TIMEOUT_ENV: &str = "GITCOMET_GIT_COMMAND_TIMEOUT_SECS";
pub const GIT_COMMAND_TIMEOUT_DEFAULT_SECS: u64 = 300;
pub const GITCOMET_ASKPASS_PROMPT_LOG_ENV: &str = "GITCOMET_ASKPASS_PROMPT_LOG";
pub const GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG_ENV: &str =
    "GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG";

/// A written askpass script plus its prompt log paths, cleaned up on drop.
///
/// The temp directory holds the script and both log files; dropping the
/// struct removes all of them.
pub struct AskPassScript {
    _dir: tempfile::TempDir,
    path: PathBuf,
    host_prompt_log_path: PathBuf,
    passphrase_prompt_log_path: PathBuf,
}

impl AskPassScript {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn host_prompt_log_path(&self) -> &Path {
        &self.host_prompt_log_path
    }

    pub fn passphrase_prompt_log_path(&self) -> &Path {
        &self.passphrase_prompt_log_path
    }
}

/// The credentials an askpass script answers prompts from.
#[derive(Clone, Eq, PartialEq)]
pub enum PromptAuth {
    Explicit(StagedGitAuth),
    CachedPassphrases(Vec<CachedPassphraseEntry>),
}

impl PromptAuth {
    pub fn from_explicit(auth: StagedGitAuth) -> Option<Self> {
        if auth.secret.is_empty() {
            return None;
        }
        Some(Self::Explicit(auth))
    }

    pub fn from_cached_passphrases(passphrases: Vec<CachedPassphraseEntry>) -> Option<Self> {
        if passphrases.is_empty() {
            return None;
        }
        Some(Self::CachedPassphrases(passphrases))
    }

    pub fn kind_env(&self) -> &'static str {
        match self {
            Self::Explicit(auth) => match auth.kind {
                GitAuthKind::UsernamePassword => GITCOMET_AUTH_KIND_USERNAME_PASSWORD,
                GitAuthKind::Passphrase => GITCOMET_AUTH_KIND_PASSPHRASE,
                GitAuthKind::HostVerification => GITCOMET_AUTH_KIND_HOST_VERIFICATION,
            },
            Self::CachedPassphrases(_) => GITCOMET_AUTH_KIND_PASSPHRASE_CACHED,
        }
    }

    pub fn username(&self) -> Option<&str> {
        match self {
            Self::Explicit(auth) => auth.username.as_deref(),
            Self::CachedPassphrases(_) => None,
        }
    }

    pub fn secret(&self) -> &str {
        match self {
            Self::Explicit(auth) => &auth.secret,
            Self::CachedPassphrases(_) => "",
        }
    }

    pub fn remember_on_success(&self, prompt: Option<&str>) {
        if let Self::Explicit(auth) = self {
            remember_passphrase_prompt_from_staged_git_auth(auth, prompt);
        }
    }
}

/// Resolves the auth an askpass-bearing command should run with: the explicit
/// staged auth when provided, otherwise anything staged, then the session
/// passphrase cache.
pub fn resolve_git_auth(auth: Option<StagedGitAuth>) -> Option<PromptAuth> {
    auth.and_then(PromptAuth::from_explicit)
        .or_else(take_pending_git_auth)
}

pub fn take_pending_git_auth() -> Option<PromptAuth> {
    take_staged_git_auth()
        .and_then(PromptAuth::from_explicit)
        .or_else(|| {
            let passphrases = load_session_passphrases();
            PromptAuth::from_cached_passphrases(passphrases)
        })
}

pub fn git_command_timeout() -> Duration {
    std::env::var(GIT_COMMAND_TIMEOUT_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(GIT_COMMAND_TIMEOUT_DEFAULT_SECS))
}

/// The POSIX helper. Every use of the prompt is double-quoted, so text git
/// or ssh hands us is never re-parsed by the shell.
pub const UNIX_ASKPASS_SCRIPT: &[u8] = br#"#!/bin/sh
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
"#;

/// The Windows helper.
///
/// The prompt is git's or ssh's text and embeds the remote URL, which the
/// repository being fetched controls, so it must only ever be read through
/// delayed expansion (`!prompt!`): `%prompt%` is substituted before `cmd`
/// parses the line, and a `&`, `|` or `>` in the value would then run as a
/// command. `EnableDelayedExpansion` is on from the first line for that reason.
pub const WINDOWS_ASKPASS_SCRIPT: &[u8] = br#"@echo off
setlocal EnableDelayedExpansion
set "prompt=%~1"
if not "%GITCOMET_ASKPASS_PROMPT_LOG%"=="" (
  echo !prompt! | findstr /I /C:"authenticity of host" /C:"continue connecting" /C:"yes/no" /C:"fingerprint" >nul
  if not errorlevel 1 (
    >>"%GITCOMET_ASKPASS_PROMPT_LOG%" echo !prompt!
  )
)
if not "%GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG%"=="" (
  echo !prompt! | findstr /I "passphrase" >nul
  if not errorlevel 1 (
    >>"%GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG%" echo !prompt!
  )
)
if /I "%GITCOMET_AUTH_KIND%"=="username_password" (
  echo !prompt! | findstr /I "username" >nul
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
  echo !prompt! | findstr /I /C:"continue connecting" /C:"yes/no" /C:"fingerprint" >nul
  if not errorlevel 1 (
    echo %GITCOMET_AUTH_SECRET%
  )
  exit /b 0
)
echo %GITCOMET_AUTH_SECRET%
"#;

pub fn askpass_script_contents() -> &'static [u8] {
    if cfg!(windows) {
        WINDOWS_ASKPASS_SCRIPT
    } else {
        UNIX_ASKPASS_SCRIPT
    }
}

pub fn create_askpass_script() -> std::io::Result<AskPassScript> {
    let dir = tempfile::tempdir()?;
    #[cfg(windows)]
    let script_name = "gitcomet-askpass.cmd";
    #[cfg(not(windows))]
    let script_name = "gitcomet-askpass.sh";
    let path = dir.path().join(script_name);
    let host_prompt_log_path = dir.path().join("gitcomet-askpass-host-prompt.log");
    let passphrase_prompt_log_path = dir.path().join("gitcomet-askpass-passphrase-prompt.log");

    fs::write(&path, askpass_script_contents())?;
    fs::write(&host_prompt_log_path, b"")?;
    fs::write(&passphrase_prompt_log_path, b"")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions)?;
    }

    Ok(AskPassScript {
        _dir: dir,
        path,
        host_prompt_log_path,
        passphrase_prompt_log_path,
    })
}

pub fn configure_git_auth_prompt(
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

/// The last non-empty prompt the askpass script logged for a passphrase
/// prompt, which is the prompt the child actually echoed its response to.
pub fn last_logged_passphrase_prompt(askpass: &AskPassScript) -> Option<String> {
    let raw = fs::read_to_string(&askpass.passphrase_prompt_log_path).ok()?;
    raw.lines()
        .rev()
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

pub fn remember_successful_prompt_auth(auth: Option<&PromptAuth>, askpass: &AskPassScript) {
    if let Some(auth) = auth {
        auth.remember_on_success(last_logged_passphrase_prompt(askpass).as_deref());
    }
}

/// Appends the logged SSH host-verification prompt to byte stderr when the
/// child did not already echo it.
///
/// Byte stderr is the canonical askpass representation; callers convert to
/// text only where they need it.
pub fn append_host_prompt_to_stderr(stderr: &mut Vec<u8>, askpass: &AskPassScript) {
    let Ok(raw_prompt_log) = fs::read_to_string(&askpass.host_prompt_log_path) else {
        return;
    };
    let prompt_log = raw_prompt_log.trim();
    if prompt_log.is_empty() {
        return;
    }

    let stderr_text = String::from_utf8_lossy(stderr);
    if stderr_text.contains(prompt_log) {
        return;
    }

    if !stderr.is_empty() && !stderr.ends_with(b"\n") {
        stderr.push(b'\n');
    }
    stderr.extend_from_slice(b"SSH host verification prompt:\n");
    stderr.extend_from_slice(prompt_log.as_bytes());
    stderr.push(b'\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Windows helper must only read the prompt through delayed expansion.
    /// `%prompt%` is substituted before `cmd` tokenises the line, so a `&`
    /// in a remote URL echoed back by git would run as a command.
    #[test]
    fn windows_askpass_script_never_expands_the_prompt_at_parse_time() {
        let script = std::str::from_utf8(WINDOWS_ASKPASS_SCRIPT).expect("ascii script");
        assert!(
            script.starts_with("@echo off\r\nsetlocal EnableDelayedExpansion")
                || script.starts_with("@echo off\nsetlocal EnableDelayedExpansion"),
            "delayed expansion must be enabled before the prompt is used"
        );
        for (number, line) in script.lines().enumerate() {
            assert!(
                !line.contains("%prompt%"),
                "line {}: `{line}` re-parses the prompt; use !prompt!",
                number + 1
            );
        }
        assert!(
            script.contains("echo !prompt!"),
            "the prompt is still inspected"
        );
    }

    #[test]
    fn unix_askpass_script_quotes_every_prompt_use() {
        let script = std::str::from_utf8(UNIX_ASKPASS_SCRIPT).expect("ascii script");
        for (number, line) in script.lines().enumerate() {
            let mut rest = line;
            while let Some(index) = rest.find("$prompt") {
                let quoted = rest[..index].ends_with('"') || rest[..index].ends_with("${");
                assert!(
                    quoted,
                    "line {}: `{line}` uses $prompt unquoted",
                    number + 1
                );
                rest = &rest[index + "$prompt".len()..];
            }
        }
    }

    /// Runs the real helper the way git does — the prompt as one argument —
    /// with a prompt shaped like a credential request for a hostile URL.
    #[cfg(windows)]
    #[test]
    fn windows_askpass_script_does_not_execute_metacharacters_in_the_prompt() {
        let askpass = create_askpass_script().expect("askpass script");
        let marker = askpass.path().with_file_name("injected.txt");
        let prompt = format!(
            "Password for 'https://user&echo pwned>\"{}\"&x@example.invalid': ",
            marker.display()
        );
        let output = Command::new(askpass.path())
            .arg(&prompt)
            .env(GITCOMET_AUTH_KIND_ENV, GITCOMET_AUTH_KIND_USERNAME_PASSWORD)
            .env(GITCOMET_AUTH_USERNAME_ENV, "user")
            .env(GITCOMET_AUTH_SECRET_ENV, "s3cret")
            .env(
                GITCOMET_ASKPASS_PROMPT_LOG_ENV,
                askpass.host_prompt_log_path(),
            )
            .env(
                GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG_ENV,
                askpass.passphrase_prompt_log_path(),
            )
            .output()
            .expect("run askpass helper");

        assert!(
            !marker.exists(),
            "metacharacters in the prompt were executed by cmd"
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "s3cret");
    }
}
