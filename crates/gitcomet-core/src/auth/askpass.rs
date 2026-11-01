//! Askpass helper script support shared by the gix backend and the state
//! clone effect.
//!
//! Git and SSH run an askpass helper to obtain credentials without a TTY.
//! GitComet stages the user's credentials in [`super::stage_git_auth`] and
//! hands them to the helper through environment variables, so secrets never
//! appear on a command line. Prompt text the helper echoes back to its log
//! files records whether each request received a response. Independently
//! verified key passphrases can be reused even if Git later fails locally.

use super::{
    AuthChallenge, AuthOutcome, CachedPassphraseEntry, GITCOMET_AUTH_CACHE_PROMPT_ENV_PREFIX,
    GITCOMET_AUTH_CACHE_SECRET_ENV_PREFIX, GITCOMET_AUTH_CACHE_SIZE_ENV, GITCOMET_AUTH_KIND_ENV,
    GITCOMET_AUTH_KIND_HOST_VERIFICATION, GITCOMET_AUTH_KIND_PASSPHRASE,
    GITCOMET_AUTH_KIND_PASSPHRASE_CACHED, GITCOMET_AUTH_KIND_USERNAME_PASSWORD,
    GITCOMET_AUTH_SECRET_ENV, GITCOMET_AUTH_USERNAME_ENV, GitAuthKind, PassphraseValidation,
    SSH_PASSPHRASE_PROMPT_MARKER, StagedGitAuth, load_session_passphrases, take_staged_git_auth,
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
const RESPONSE_LOG_ENV: &str = "GITCOMET_ASKPASS_RESPONSE_LOG";
const TARGET_PROMPT_ENV: &str = "GITCOMET_AUTH_TARGET_PROMPT";
const USERNAME_PROMPT_ENV: &str = "GITCOMET_AUTH_USERNAME_PROMPT";
const PASSWORD_PROMPT_ENV: &str = "GITCOMET_AUTH_PASSWORD_PROMPT";
const HOST_KEY_PROMPT_ENV: &str = "GITCOMET_AUTH_HOST_KEY_PROMPT";
#[cfg(test)]
#[path = "askpass_tests.rs"]
mod outcome_tests;
const GITCOMET_ASKPASS_PROMPT_INPUT_ENV: &str = "GITCOMET_ASKPASS_PROMPT_INPUT";

/// A written askpass script plus its prompt log paths, cleaned up on drop.
///
/// The temp directory holds the script and both log files; dropping the
/// struct removes all of them.
pub struct AskPassScript {
    _dir: tempfile::TempDir,
    path: PathBuf,
    prompt_input_path: PathBuf,
    host_prompt_log_path: PathBuf,
    passphrase_prompt_log_path: PathBuf,
    response_log_path: PathBuf,
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
answer=""
if [ "$kind" = "username_password" ]; then
  if [ -n "${GITCOMET_AUTH_USERNAME_PROMPT:-}" ] \
    && [ "$prompt" = "$GITCOMET_AUTH_USERNAME_PROMPT" ]; then
    answer="${GITCOMET_AUTH_USERNAME:-}"
  elif [ -n "${GITCOMET_AUTH_PASSWORD_PROMPT:-}" ] \
    && [ "$prompt" = "$GITCOMET_AUTH_PASSWORD_PROMPT" ]; then
    answer="${GITCOMET_AUTH_SECRET:-}"
  elif [ -z "${GITCOMET_AUTH_TARGET_PROMPT:-}" ] \
    || [ "$prompt" = "$GITCOMET_AUTH_TARGET_PROMPT" ]; then
    case "$lower_prompt" in
      *username*) answer="${GITCOMET_AUTH_USERNAME:-}" ;;
      *password*) answer="${GITCOMET_AUTH_SECRET:-}" ;;
    esac
  fi
elif [ "$kind" = "passphrase" ]; then
  case "$lower_prompt" in
    *passphrase*|*enter\ pin*)
      if [ -z "${GITCOMET_AUTH_TARGET_PROMPT:-}" ] || [ "$prompt" = "$GITCOMET_AUTH_TARGET_PROMPT" ]; then
        answer="${GITCOMET_AUTH_SECRET:-}"
      fi ;;
  esac
elif [ "$kind" = "host_verification" ]; then
  case "$lower_prompt" in
    *continue\ connecting*|*yes/no*|*fingerprint*)
      unaddressed=$(printf '%s' "$prompt" | sed "1s/ ([^()]*)'/'/")
      if [ -n "${GITCOMET_AUTH_HOST_KEY_PROMPT:-}" ] && [ "$unaddressed" = "$GITCOMET_AUTH_HOST_KEY_PROMPT" ]; then
        answer="${GITCOMET_AUTH_SECRET:-}"
      fi ;;
  esac
fi
if [ -z "$answer" ]; then
  cache_size="${GITCOMET_AUTH_CACHE_SIZE:-0}"
  i=0
  while [ "$i" -lt "$cache_size" ]; do
    cached_prompt=$(printenv "GITCOMET_AUTH_CACHE_PROMPT_$i")
    if [ "$prompt" = "$cached_prompt" ]; then
      answer=$(printenv "GITCOMET_AUTH_CACHE_SECRET_$i")
      break
    fi
    i=$((i + 1))
  done
fi
if [ -n "${GITCOMET_ASKPASS_RESPONSE_LOG:-}" ]; then
  if [ -n "$answer" ]; then status=S; else status=M; fi
  printf '%s:%s\n' "$status" "$prompt" >> "$GITCOMET_ASKPASS_RESPONSE_LOG"
fi
printf '%s\n' "$answer"
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
rem A .cmd launched by Command is invoked as: cmd.exe /c ""script" "prompt""
rem Reading the first argument directly would substitute the untrusted prompt
rem back into batch syntax.
rem Instead, take it from cmd's original command line via delayed expansion.
rem Git for Windows may invoke a no-space helper path without quotes, while
rem Command quotes it when a parent directory contains spaces. Strip either
rem form using only constant delimiters, then canonicalize quotes so the same
rem key prompt has a stable session-cache key on every invocation.
set "cmdcmdline="
set "prompt=!cmdcmdline:*gitcomet-askpass.cmd =!"
set "prompt=!prompt:*" "=!"
set "prompt=!prompt:"=!"
:trim_prompt
if "!prompt:~-1!"==" " (
  set "prompt=!prompt:~0,-1!"
  goto trim_prompt
)
set "findstr=!SystemRoot!\System32\findstr.exe"
> "!GITCOMET_ASKPASS_PROMPT_INPUT!" echo(!prompt!
if not "!GITCOMET_ASKPASS_PROMPT_LOG!"=="" (
  "!findstr!" /I /C:"authenticity of host" /C:"continue connecting" /C:"yes/no" /C:"fingerprint" "!GITCOMET_ASKPASS_PROMPT_INPUT!" >nul
  if not errorlevel 1 (
    >>"!GITCOMET_ASKPASS_PROMPT_LOG!" echo(!prompt!
  )
)
if not "!GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG!"=="" (
  "!findstr!" /I "passphrase" "!GITCOMET_ASKPASS_PROMPT_INPUT!" >nul
  if not errorlevel 1 (
    >>"!GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG!" echo(!prompt!
  )
)
set "answer="
if /I "!GITCOMET_AUTH_KIND!"=="username_password" (
  if not "!GITCOMET_AUTH_USERNAME_PROMPT!"=="" if "!prompt!"=="!GITCOMET_AUTH_USERNAME_PROMPT!" (
    set "answer=!GITCOMET_AUTH_USERNAME!"
    goto respond
  )
  if not "!GITCOMET_AUTH_PASSWORD_PROMPT!"=="" if "!prompt!"=="!GITCOMET_AUTH_PASSWORD_PROMPT!" (
    set "answer=!GITCOMET_AUTH_SECRET!"
    goto cached
  )
  set "bound="
  if "!GITCOMET_AUTH_TARGET_PROMPT!"=="" set "bound=1"
  if "!prompt!"=="!GITCOMET_AUTH_TARGET_PROMPT!" set "bound=1"
  if not defined bound goto cached
  "!findstr!" /I "username" "!GITCOMET_ASKPASS_PROMPT_INPUT!" >nul
  if not errorlevel 1 (
    set "answer=!GITCOMET_AUTH_USERNAME!"
    goto respond
  )
  "!findstr!" /I "password" "!GITCOMET_ASKPASS_PROMPT_INPUT!" >nul
  if not errorlevel 1 set "answer=!GITCOMET_AUTH_SECRET!"
  goto cached
)
if /I "!GITCOMET_AUTH_KIND!"=="passphrase" (
  "!findstr!" /I /C:"passphrase" /C:"enter pin" "!GITCOMET_ASKPASS_PROMPT_INPUT!" >nul
  if not errorlevel 1 (
    if "!GITCOMET_AUTH_TARGET_PROMPT!"=="" set "answer=!GITCOMET_AUTH_SECRET!"
    if "!prompt!"=="!GITCOMET_AUTH_TARGET_PROMPT!" set "answer=!GITCOMET_AUTH_SECRET!"
  )
)
if /I "!GITCOMET_AUTH_KIND!"=="host_verification" (
  "!findstr!" /I /C:"continue connecting" /C:"yes/no" /C:"fingerprint" "!GITCOMET_ASKPASS_PROMPT_INPUT!" >nul
  if not errorlevel 1 (
    if not "!GITCOMET_AUTH_TARGET_PROMPT!"=="" if "!prompt!"=="!GITCOMET_AUTH_TARGET_PROMPT!" set "answer=!GITCOMET_AUTH_SECRET!"
  )
)
:cached
if "!answer!"=="" (
  set "cache_size=!GITCOMET_AUTH_CACHE_SIZE!"
  if "!cache_size!"=="" set "cache_size=0"
  set /a cache_last=!cache_size!-1
  if !cache_last! GEQ 0 (
    for /L %%i in (0,1,!cache_last!) do (
      set "cached_prompt=!GITCOMET_AUTH_CACHE_PROMPT_%%i!"
      if "!prompt!"=="!cached_prompt!" (
        set "answer=!GITCOMET_AUTH_CACHE_SECRET_%%i!"
        goto respond
      )
    )
  )
)
:respond
if "!GITCOMET_ASKPASS_RESPONSE_LOG!"=="" goto answer
set "GITCOMET_ASKPASS_RESPONSE=S:!prompt!"
if "!answer!"=="" set "GITCOMET_ASKPASS_RESPONSE=M:!prompt!"
rem Git runs without a console window, so this one has the system OEM code
rem page and echo would mangle a non-ASCII key path. cmd /u writes the record
rem as UTF-16 instead. With delayed expansion off here, the child receives
rem the variable names and expands them itself, never re-parsing the prompt.
setlocal DisableDelayedExpansion
>nul "%SystemRoot%\System32\cmd.exe" /d /u /v:on /c ">>"!GITCOMET_ASKPASS_RESPONSE_LOG!" echo(!GITCOMET_ASKPASS_RESPONSE!"
endlocal
:answer
echo(!answer!
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
    let prompt_input_path = dir.path().join("gitcomet-askpass-prompt-input.txt");
    let host_prompt_log_path = dir.path().join("gitcomet-askpass-host-prompt.log");
    let passphrase_prompt_log_path = dir.path().join("gitcomet-askpass-passphrase-prompt.log");
    let response_log_path = dir.path().join(format!(
        "{}-askpass-responses.log",
        crate::identity::current().executable_name(),
    ));

    fs::write(&path, askpass_script_contents())?;
    fs::write(&prompt_input_path, b"")?;
    fs::write(&host_prompt_log_path, b"")?;
    fs::write(&passphrase_prompt_log_path, b"")?;
    fs::write(&response_log_path, b"")?;

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
        prompt_input_path,
        host_prompt_log_path,
        passphrase_prompt_log_path,
        response_log_path,
    })
}

pub fn configure_git_auth_prompt(
    cmd: &mut Command,
    auth: Option<&PromptAuth>,
    askpass: &AskPassScript,
) {
    configure_prompt_env(cmd, auth, askpass, true);
}

/// `session_cache` also hands the helper every verified session passphrase,
/// so explicit credentials for one key still unlock the others.
fn configure_prompt_env(
    cmd: &mut Command,
    auth: Option<&PromptAuth>,
    askpass: &AskPassScript,
    session_cache: bool,
) {
    cmd.env("GIT_ASKPASS", &askpass.path);
    cmd.env("SSH_ASKPASS", &askpass.path);
    cmd.env("SSH_ASKPASS_REQUIRE", "force");
    cmd.env(RESPONSE_LOG_ENV, &askpass.response_log_path);
    cmd.env_remove(TARGET_PROMPT_ENV);
    cmd.env_remove(USERNAME_PROMPT_ENV);
    cmd.env_remove(PASSWORD_PROMPT_ENV);
    cmd.env_remove(HOST_KEY_PROMPT_ENV);
    cmd.env(
        GITCOMET_ASKPASS_PROMPT_LOG_ENV,
        &askpass.host_prompt_log_path,
    );
    cmd.env(
        GITCOMET_ASKPASS_PROMPT_INPUT_ENV,
        &askpass.prompt_input_path,
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
            PromptAuth::Explicit(explicit) => {
                cmd.env(GITCOMET_AUTH_KIND_ENV, auth.kind_env());
                if let Some(username) = auth.username() {
                    cmd.env(GITCOMET_AUTH_USERNAME_ENV, username);
                } else {
                    cmd.env_remove(GITCOMET_AUTH_USERNAME_ENV);
                }
                cmd.env(GITCOMET_AUTH_SECRET_ENV, auth.secret());
                if let Some(challenge) = explicit.challenge.as_ref() {
                    cmd.env(TARGET_PROMPT_ENV, normalized_prompt(&challenge.prompt));
                    if explicit.kind == GitAuthKind::UsernamePassword
                        && let Some((username_prompt, password_prompt)) =
                            credential_prompt_pair(&challenge.prompt, auth.username())
                    {
                        cmd.env(USERNAME_PROMPT_ENV, normalized_prompt(&username_prompt));
                        cmd.env(PASSWORD_PROMPT_ENV, normalized_prompt(&password_prompt));
                    }
                    if explicit.kind == GitAuthKind::HostVerification {
                        cmd.env(
                            HOST_KEY_PROMPT_ENV,
                            host_key_prompt_without_address(&challenge.prompt),
                        );
                    }
                }
                if session_cache {
                    configure_cached_passphrases(cmd, &load_session_passphrases());
                }
            }
            PromptAuth::CachedPassphrases(entries) => {
                cmd.env(GITCOMET_AUTH_KIND_ENV, auth.kind_env());
                cmd.env_remove(GITCOMET_AUTH_USERNAME_ENV);
                cmd.env_remove(GITCOMET_AUTH_SECRET_ENV);
                configure_cached_passphrases(cmd, entries);
            }
        }
    } else {
        cmd.env_remove(GITCOMET_AUTH_KIND_ENV);
        cmd.env_remove(GITCOMET_AUTH_USERNAME_ENV);
        cmd.env_remove(GITCOMET_AUTH_SECRET_ENV);
    }
}

/// Git asks `Username for '<url>': ` and then `Password for '<user>@<url>': `
/// with `<user>` percent-encoded (`credential.c`). Typed credentials answer
/// that pair for the challenge's URL only: the same command can reach other
/// hosts (`fetch --all`, a submodule), and those must not receive them.
fn credential_prompt_pair(
    challenge_prompt: &str,
    username: Option<&str>,
) -> Option<(String, String)> {
    let prompt = challenge_prompt.trim_end();
    let url = prompt
        .strip_prefix("Username for '")
        .or_else(|| prompt.strip_prefix("Password for '"))?
        .strip_suffix("':")?;
    let (scheme, rest) = url.split_once("://")?;
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let host_and_path = match rest[..authority_end].rfind('@') {
        Some(at) => &rest[at + 1..],
        None => rest,
    };
    let username_prompt = format!("Username for '{scheme}://{host_and_path}': ");
    let password_prompt = match username.filter(|username| !username.is_empty()) {
        Some(username) => format!(
            "Password for '{scheme}://{}@{host_and_path}': ",
            crate::url_encoding::encode_component(username)
        ),
        None => format!("Password for '{scheme}://{host_and_path}': "),
    };
    Some((username_prompt, password_prompt))
}

/// OpenSSH names the host and the address it reached, `'github.com
/// (140.82.121.3)'`, and a round-robin host answers the replay from another
/// address. A confirmation binds the host name and key, so the POSIX helper
/// drops the address the same way (`sed "1s/ ([^()]*)'/'/"`) before comparing.
fn host_key_prompt_without_address(prompt: &str) -> String {
    let prompt = prompt.trim_end_matches('\n');
    let (first, rest) = prompt.split_once('\n').unwrap_or((prompt, ""));
    let mut first = first.to_string();
    if let Some(close) = first.find(")'")
        && let Some(open) = first[..close].rfind(" (")
        && !first[open + 2..close].contains(['(', ')'])
    {
        first.replace_range(open..close + 1, "");
    }
    if prompt.contains('\n') {
        format!("{first}\n{rest}")
    } else {
        first
    }
}

fn normalized_prompt(prompt: &str) -> String {
    if cfg!(windows) {
        prompt.trim_end().replace('"', "")
    } else {
        prompt.to_string()
    }
}

fn configure_cached_passphrases(cmd: &mut Command, entries: &[CachedPassphraseEntry]) {
    cmd.env(GITCOMET_AUTH_CACHE_SIZE_ENV, entries.len().to_string());
    for (idx, entry) in entries.iter().enumerate() {
        cmd.env(
            format!("{GITCOMET_AUTH_CACHE_PROMPT_ENV_PREFIX}{idx}"),
            normalized_prompt(&entry.prompt),
        );
        cmd.env(
            format!("{GITCOMET_AUTH_CACHE_SECRET_ENV_PREFIX}{idx}"),
            &entry.secret,
        );
    }
}

fn prompt_kind(prompt: &str) -> Option<GitAuthKind> {
    let lower = prompt.to_ascii_lowercase();
    // A security key's PIN ("Enter PIN for ED25519-SK key ...") is entered and
    // answered exactly like a key passphrase.
    if lower.contains("passphrase") || lower.contains("enter pin") {
        Some(GitAuthKind::Passphrase)
    } else if lower.contains("continue connecting")
        || lower.contains("yes/no")
        || lower.contains("fingerprint")
        || lower.contains("authenticity of host")
    {
        Some(GitAuthKind::HostVerification)
    } else if lower.contains("username") || lower.contains("password") {
        Some(GitAuthKind::UsernamePassword)
    } else {
        None
    }
}

/// The Windows helper writes UTF-16 (`cmd /u`); the POSIX one writes the
/// prompt's bytes. One undecodable byte must not hide every other record.
fn read_response_log(askpass: &AskPassScript) -> String {
    let bytes = fs::read(&askpass.response_log_path).unwrap_or_default();
    if cfg!(windows) {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Each helper call appends `S:` (answered) or `M:` (missing) and its prompt.
/// OpenSSH's host-key prompt spans several lines (the fingerprint is on the
/// second), so a line without a status continues the previous prompt.
fn observed_responses(askpass: &AskPassScript) -> Vec<(bool, AuthChallenge)> {
    let log = read_response_log(askpass);
    let mut records: Vec<(bool, String)> = Vec::new();
    for line in log.lines() {
        if let Some(prompt) = line.strip_prefix("S:") {
            records.push((true, prompt.to_string()));
        } else if let Some(prompt) = line.strip_prefix("M:") {
            records.push((false, prompt.to_string()));
        } else if let Some((_, prompt)) = records.last_mut() {
            prompt.push('\n');
            prompt.push_str(line);
        }
    }
    let mut responses: Vec<(bool, AuthChallenge)> = Vec::with_capacity(records.len());
    for (supplied, prompt) in records {
        // ssh repeats one key's prompt after each wrong answer. Resolving a
        // challenge reads and hashes the key, so do that once per prompt.
        let seen = responses
            .iter()
            .find(|(_, challenge)| challenge.prompt == prompt)
            .map(|(_, challenge)| challenge.clone());
        let challenge = match seen {
            Some(challenge) => challenge,
            None => match prompt_kind(&prompt) {
                Some(kind) => AuthChallenge::from_prompt(kind, prompt),
                None => continue,
            },
        };
        responses.push((supplied, challenge));
    }
    responses
}

fn response_is_verified(auth: Option<&PromptAuth>, challenge: &AuthChallenge) -> bool {
    let explicit_secret = match auth {
        Some(PromptAuth::Explicit(auth))
            if auth.kind == GitAuthKind::Passphrase
                && auth.challenge.as_ref().is_none_or(|target| {
                    normalized_prompt(&target.prompt) == normalized_prompt(&challenge.prompt)
                }) =>
        {
            Some(auth.secret.as_str())
        }
        _ => None,
    };
    super::challenge::challenge_is_verified(challenge, explicit_secret)
}

/// A historical askpass call is never evidence that the final Git failure
/// needs another password. Only an unanswered request or explicit rejection
/// can reopen authentication.
pub fn resolve_auth_outcome(
    auth: Option<&PromptAuth>,
    askpass: &AskPassScript,
    stderr: &[u8],
    success: bool,
) -> AuthOutcome {
    let responses = observed_responses(askpass);
    if success {
        return if responses
            .iter()
            .any(|(supplied, challenge)| *supplied && response_is_verified(auth, challenge))
        {
            AuthOutcome::Verified
        } else if responses.iter().any(|(supplied, _)| *supplied) {
            AuthOutcome::Supplied
        } else {
            AuthOutcome::NotRequested
        };
    }
    let lower = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    // A completed fetch may have tried an unused identity before Git reached
    // a local refusal. That earlier missing/rejected answer is not the cause
    // of this operation's final failure.
    let local_failure = crate::error::git_output_reports_local_refusal(&lower);
    if !local_failure && let Some((false, challenge)) = responses.last() {
        return AuthOutcome::NeedsInput(challenge.clone());
    }
    let rejected_passphrase = lower.contains("incorrect passphrase")
        || lower.contains("bad passphrase")
        || lower.contains("bad decrypt");
    let rejected_password = lower.contains("authentication failed")
        || lower.contains("invalid username or password")
        || lower.contains("http basic: access denied");
    if !local_failure
        && let Some((true, challenge)) = responses.last()
        && match challenge.kind {
            GitAuthKind::Passphrase => {
                rejected_passphrase && !response_is_verified(auth, challenge)
            }
            GitAuthKind::UsernamePassword => rejected_password,
            GitAuthKind::HostVerification => false,
        }
    {
        super::challenge::forget_challenge(challenge);
        return AuthOutcome::Rejected(challenge.clone());
    }
    // An unanswered host-key or credential prompt aborts its connection (an
    // unused key passphrase does not), so a replay that answered a later
    // host still has to ask about this one.
    if !local_failure
        && let Some((_, challenge)) = responses.iter().find(|(supplied, challenge)| {
            !supplied
                && match challenge.kind {
                    GitAuthKind::HostVerification | GitAuthKind::UsernamePassword => true,
                    GitAuthKind::Passphrase => false,
                }
        })
    {
        return AuthOutcome::NeedsInput(challenge.clone());
    }
    if responses
        .iter()
        .any(|(supplied, challenge)| *supplied && response_is_verified(auth, challenge))
    {
        AuthOutcome::Verified
    } else if responses.iter().any(|(supplied, _)| *supplied) {
        AuthOutcome::Supplied
    } else {
        AuthOutcome::NotRequested
    }
}

/// Independently unlock a known local key. The public-key output and an
/// answered askpass invocation are both required; arbitrary tool errors do
/// not establish that a passphrase was rejected.
pub fn validate_passphrase(
    challenge: &AuthChallenge,
    auth: &StagedGitAuth,
    cancellation: &crate::services::CancellationToken,
) -> PassphraseValidation {
    let Some(path) = challenge.key_path.as_ref() else {
        return PassphraseValidation::Unknown;
    };
    if challenge.kind != GitAuthKind::Passphrase || auth.kind != GitAuthKind::Passphrase {
        return PassphraseValidation::Unknown;
    }
    let Some(before) = super::challenge::key_fingerprint(path) else {
        return PassphraseValidation::Unknown;
    };
    if challenge.key_fingerprint != Some(before) {
        return PassphraseValidation::Unknown;
    }
    let Ok(askpass) = create_askpass_script() else {
        return PassphraseValidation::Unknown;
    };
    // This verifier reads exactly the key supplied with -f. Windows OpenSSH
    // can ask only "Enter passphrase:" here, without including its path, so
    // answer any passphrase prompt with the submitted secret (no target), and
    // give the helper no other cached passphrase at all.
    let unbound = PromptAuth::Explicit(StagedGitAuth {
        challenge: None,
        ..auth.clone()
    });
    let mut cmd = crate::process::background_command("ssh-keygen");
    cmd.arg("-y").arg("-f").arg(path).env("LC_ALL", "C");
    configure_prompt_env(&mut cmd, Some(&unbound), &askpass, false);
    let Ok(output) = crate::process::probe_output(
        cmd,
        git_command_timeout().min(Duration::from_secs(30)),
        cancellation,
    ) else {
        return PassphraseValidation::Unknown;
    };
    if cancellation.is_cancelled()
        || super::challenge::key_fingerprint(path) != Some(before)
        || !observed_responses(&askpass)
            .iter()
            .any(|(supplied, _)| *supplied)
    {
        return PassphraseValidation::Unknown;
    }
    let public_key = String::from_utf8_lossy(&output.stdout);
    let mut fields = public_key.split_whitespace();
    let valid_public_key = fields.next().is_some_and(|kind| {
        kind.starts_with("ssh-") || kind.starts_with("ecdsa-") || kind.starts_with("sk-")
    }) && fields.next().is_some_and(|data| {
        !data.is_empty()
            && data
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"+/=".contains(&byte))
    });
    if output.status.success() && valid_public_key {
        PassphraseValidation::Verified
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        if stderr.contains("incorrect passphrase") || stderr.contains("bad passphrase") {
            super::challenge::forget_challenge(challenge);
            PassphraseValidation::Rejected
        } else {
            PassphraseValidation::Unknown
        }
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

/// Appends a passphrase prompt observed by askpass to a failed command's
/// stderr for diagnostics. This history is not an authentication decision;
/// callers must attach the separately resolved [`AuthOutcome`].
pub fn append_passphrase_prompt_to_stderr(stderr: &mut Vec<u8>, askpass: &AskPassScript) {
    let Some(prompt) = last_logged_passphrase_prompt(askpass) else {
        return;
    };
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return;
    }

    if String::from_utf8_lossy(stderr).contains(SSH_PASSPHRASE_PROMPT_MARKER) {
        return;
    }

    if !stderr.is_empty() && !stderr.ends_with(b"\n") {
        stderr.push(b'\n');
    }
    stderr.extend_from_slice(SSH_PASSPHRASE_PROMPT_MARKER.as_bytes());
    stderr.push(b'\n');
    stderr.extend_from_slice(prompt.as_bytes());
    stderr.push(b'\n');
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
            script.contains("echo(!prompt!"),
            "the prompt is still inspected"
        );
        assert!(
            !script.contains("%~1"),
            "expanding the prompt argument directly re-parses its metacharacters"
        );
        assert!(
            script.contains("!cmdcmdline:"),
            "the prompt must be read from cmd's original command line"
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

    #[test]
    fn append_passphrase_prompt_replays_logged_prompt_once() {
        let askpass = create_askpass_script().expect("askpass script");
        std::fs::write(
            askpass.passphrase_prompt_log_path(),
            "Enter passphrase for \"C:\\Users\\dev\\.ssh\\id_ed25519\": \n",
        )
        .expect("write prompt log");
        let mut stderr =
            b"Load key: incorrect passphrase supplied to decrypt private key\n".to_vec();

        append_passphrase_prompt_to_stderr(&mut stderr, &askpass);
        append_passphrase_prompt_to_stderr(&mut stderr, &askpass);

        let rendered = String::from_utf8(stderr).expect("utf-8 stderr");
        assert_eq!(rendered.matches(SSH_PASSPHRASE_PROMPT_MARKER).count(), 1);
        assert!(rendered.contains("Enter passphrase for"));
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
        let auth = PromptAuth::Explicit(StagedGitAuth {
            kind: GitAuthKind::UsernamePassword,
            username: Some("user".to_string()),
            secret: "s3cret".to_string(),
            challenge: None,
        });
        let mut command = Command::new(askpass.path());
        command.arg(&prompt).env("PATH", "");
        configure_git_auth_prompt(&mut command, Some(&auth), &askpass);
        let output = command.output().expect("run askpass helper");

        assert!(
            !marker.exists(),
            "metacharacters in the prompt were executed by cmd; status: {:?}, stdout: {:?}, stderr: {:?}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "askpass helper wrote an error: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "s3cret");
    }
}
