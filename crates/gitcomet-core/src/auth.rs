use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread::ThreadId;

pub const GITCOMET_AUTH_KIND_ENV: &str = "GITCOMET_AUTH_KIND";
pub const GITCOMET_AUTH_USERNAME_ENV: &str = "GITCOMET_AUTH_USERNAME";
pub const GITCOMET_AUTH_SECRET_ENV: &str = "GITCOMET_AUTH_SECRET";
pub const GITCOMET_AUTH_CACHE_SIZE_ENV: &str = "GITCOMET_AUTH_CACHE_SIZE";
pub const GITCOMET_AUTH_CACHE_PROMPT_ENV_PREFIX: &str = "GITCOMET_AUTH_CACHE_PROMPT_";
pub const GITCOMET_AUTH_CACHE_SECRET_ENV_PREFIX: &str = "GITCOMET_AUTH_CACHE_SECRET_";

/// Marker the git layer prepends when it replays a logged SSH passphrase prompt
/// into a failed command's stderr, so the reducer can classify the failure as a
/// passphrase prompt without pattern-matching OpenSSH's error wording.
pub const SSH_PASSPHRASE_PROMPT_MARKER: &str = "SSH key passphrase prompt:";

/// POSIX askpass helper installed for git commands that may prompt.
///
/// Answers on stdout and logs host-verification and passphrase prompts to the
/// side files named by [`GITCOMET_ASKPASS_PROMPT_LOG_ENV`] and
/// [`GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG_ENV`], so a failed command can be
/// classified by what was actually asked for.
pub const ASKPASS_SCRIPT_UNIX: &[u8] = br#"#!/bin/sh
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

/// Windows askpass helper.
///
/// Every value is emitted with `echo(!VAR!` rather than `echo %VAR%`: in a batch
/// file an undefined `%VAR%` expands to nothing, so `echo %VAR%` degenerates to a
/// bare `echo` and prints `ECHO is on.` — which OpenSSH would then take as the
/// passphrase. `echo(` prints an empty line instead, and delayed expansion keeps
/// `&`, `|`, `<` and `>` inside a secret from being parsed as shell operators.
pub const ASKPASS_SCRIPT_WINDOWS: &[u8] = br#"@echo off
setlocal EnableDelayedExpansion
set "prompt=%~1"
if not "%GITCOMET_ASKPASS_PROMPT_LOG%"=="" (
  echo(!prompt!| findstr /I /C:"authenticity of host" /C:"continue connecting" /C:"yes/no" /C:"fingerprint" >nul
  if not errorlevel 1 (
    >>"%GITCOMET_ASKPASS_PROMPT_LOG%" echo(!prompt!
  )
)
if not "%GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG%"=="" (
  echo(!prompt!| findstr /I "passphrase" >nul
  if not errorlevel 1 (
    >>"%GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG%" echo(!prompt!
  )
)
if /I "%GITCOMET_AUTH_KIND%"=="username_password" (
  echo(!prompt!| findstr /I "username" >nul
  if not errorlevel 1 (
    echo(!GITCOMET_AUTH_USERNAME!
    exit /b 0
  )
  echo(!GITCOMET_AUTH_SECRET!
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
        echo(!cached_secret!
        exit /b 0
      )
    )
  )
  exit /b 0
)
if /I "%GITCOMET_AUTH_KIND%"=="host_verification" (
  echo(!prompt!| findstr /I /C:"continue connecting" /C:"yes/no" /C:"fingerprint" >nul
  if not errorlevel 1 (
    echo(!GITCOMET_AUTH_SECRET!
  )
  exit /b 0
)
echo(!GITCOMET_AUTH_SECRET!
"#;

/// The askpass helper body for the host platform.
pub fn askpass_script_contents() -> &'static [u8] {
    if cfg!(windows) {
        ASKPASS_SCRIPT_WINDOWS
    } else {
        ASKPASS_SCRIPT_UNIX
    }
}

/// Filename for the generated askpass helper on the host platform.
pub fn askpass_script_name() -> &'static str {
    if cfg!(windows) {
        "gitcomet-askpass.cmd"
    } else {
        "gitcomet-askpass.sh"
    }
}

pub const GITCOMET_AUTH_KIND_USERNAME_PASSWORD: &str = "username_password";
pub const GITCOMET_AUTH_KIND_PASSPHRASE: &str = "passphrase";
pub const GITCOMET_AUTH_KIND_PASSPHRASE_CACHED: &str = "passphrase_cached";
pub const GITCOMET_AUTH_KIND_HOST_VERIFICATION: &str = "host_verification";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitAuthKind {
    UsernamePassword,
    Passphrase,
    HostVerification,
}

#[derive(Clone, Eq, PartialEq)]
pub struct StagedGitAuth {
    pub kind: GitAuthKind,
    pub username: Option<String>,
    pub secret: String,
}

#[derive(Clone, Eq, PartialEq)]
struct PendingStagedGitAuth {
    auth: StagedGitAuth,
    owner_thread: Option<ThreadId>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct CachedPassphraseEntry {
    pub prompt: String,
    pub secret: String,
}

impl std::fmt::Debug for StagedGitAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const REDACTED: &str = "<redacted>";

        f.debug_struct("StagedGitAuth")
            .field("kind", &self.kind)
            .field("username", &self.username.as_ref().map(|_| REDACTED))
            .field("secret", &REDACTED)
            .finish()
    }
}

impl std::fmt::Debug for CachedPassphraseEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const REDACTED: &str = "<redacted>";

        f.debug_struct("CachedPassphraseEntry")
            .field("prompt", &self.prompt)
            .field("secret", &REDACTED)
            .finish()
    }
}

fn staged_git_auth_slot() -> &'static Mutex<Option<PendingStagedGitAuth>> {
    static SLOT: OnceLock<Mutex<Option<PendingStagedGitAuth>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn session_passphrase_slot() -> &'static Mutex<BTreeMap<String, String>> {
    static SLOT: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Lock a mutex, recovering from poison if a prior holder panicked.
fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn clear_staged_git_auth() {
    let mut guard = lock_or_recover(staged_git_auth_slot());
    *guard = None;
}

pub fn clear_session_passphrase() {
    let mut guard = lock_or_recover(session_passphrase_slot());
    guard.clear();
}

pub fn stage_git_auth(auth: StagedGitAuth) {
    let mut guard = lock_or_recover(staged_git_auth_slot());
    *guard = Some(PendingStagedGitAuth {
        auth,
        owner_thread: None,
    });
}

pub fn stage_git_auth_for_current_thread(auth: StagedGitAuth) {
    let mut guard = lock_or_recover(staged_git_auth_slot());
    *guard = Some(PendingStagedGitAuth {
        auth,
        owner_thread: Some(std::thread::current().id()),
    });
}

pub fn take_staged_git_auth() -> Option<StagedGitAuth> {
    let mut guard = lock_or_recover(staged_git_auth_slot());
    let current = std::thread::current().id();
    match guard.as_ref() {
        Some(pending)
            if pending.owner_thread.is_none() || pending.owner_thread == Some(current) =>
        {
            guard.take().map(|pending| pending.auth)
        }
        _ => None,
    }
}

pub fn remember_session_passphrase(prompt: &str, passphrase: &str) {
    if prompt.is_empty() || passphrase.is_empty() {
        return;
    }

    let mut guard = lock_or_recover(session_passphrase_slot());
    guard.insert(prompt.to_string(), passphrase.to_string());
}

pub fn load_session_passphrases() -> Vec<CachedPassphraseEntry> {
    let guard = lock_or_recover(session_passphrase_slot());
    guard
        .iter()
        .map(|(prompt, secret)| CachedPassphraseEntry {
            prompt: prompt.clone(),
            secret: secret.clone(),
        })
        .collect()
}

pub fn remember_passphrase_prompt_from_staged_git_auth(auth: &StagedGitAuth, prompt: Option<&str>) {
    if auth.kind == GitAuthKind::Passphrase
        && let Some(prompt) = prompt
    {
        remember_session_passphrase(prompt, &auth.secret);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ASKPASS_SCRIPT_WINDOWS, GitAuthKind, StagedGitAuth, clear_session_passphrase,
        load_session_passphrases, remember_passphrase_prompt_from_staged_git_auth,
        remember_session_passphrase,
    };
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn auth_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn staged_git_auth_debug_redacts_sensitive_fields() {
        let auth = StagedGitAuth {
            kind: GitAuthKind::UsernamePassword,
            username: Some("alice".to_string()),
            secret: "token-123".to_string(),
        };

        let rendered = format!("{auth:?}");
        assert!(rendered.contains("StagedGitAuth"));
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("alice"));
        assert!(!rendered.contains("token-123"));
    }

    #[test]
    fn session_passphrase_round_trips_and_clears() {
        let _lock = auth_test_lock();
        clear_session_passphrase();

        assert!(load_session_passphrases().is_empty());

        remember_session_passphrase("Enter passphrase for key '/tmp/key-a':", "ssh-passphrase-a");
        remember_session_passphrase("Enter passphrase for key '/tmp/key-b':", "ssh-passphrase-b");

        assert_eq!(load_session_passphrases().len(), 2);
        assert_eq!(
            load_session_passphrases()
                .iter()
                .find(|entry| entry.prompt == "Enter passphrase for key '/tmp/key-a':")
                .map(|entry| entry.secret.as_str()),
            Some("ssh-passphrase-a")
        );
        assert_eq!(
            load_session_passphrases()
                .iter()
                .find(|entry| entry.prompt == "Enter passphrase for key '/tmp/key-b':")
                .map(|entry| entry.secret.as_str()),
            Some("ssh-passphrase-b")
        );

        clear_session_passphrase();
        assert!(load_session_passphrases().is_empty());
    }

    #[test]
    fn remember_passphrase_prompt_from_staged_git_auth_caches_only_passphrases() {
        let _lock = auth_test_lock();
        clear_session_passphrase();

        remember_passphrase_prompt_from_staged_git_auth(
            &StagedGitAuth {
                kind: GitAuthKind::UsernamePassword,
                username: Some("alice".to_string()),
                secret: "token-123".to_string(),
            },
            Some("Enter passphrase for key '/tmp/key-a':"),
        );
        assert!(load_session_passphrases().is_empty());

        remember_passphrase_prompt_from_staged_git_auth(
            &StagedGitAuth {
                kind: GitAuthKind::HostVerification,
                username: None,
                secret: "yes".to_string(),
            },
            Some("Enter passphrase for key '/tmp/key-a':"),
        );
        assert!(load_session_passphrases().is_empty());

        remember_passphrase_prompt_from_staged_git_auth(
            &StagedGitAuth {
                kind: GitAuthKind::Passphrase,
                username: None,
                secret: "ssh-passphrase".to_string(),
            },
            None,
        );
        assert!(load_session_passphrases().is_empty());

        remember_passphrase_prompt_from_staged_git_auth(
            &StagedGitAuth {
                kind: GitAuthKind::Passphrase,
                username: None,
                secret: "ssh-passphrase".to_string(),
            },
            Some("Enter passphrase for key '/tmp/key-a':"),
        );
        assert_eq!(load_session_passphrases().len(), 1);
        assert_eq!(
            load_session_passphrases()[0].prompt,
            "Enter passphrase for key '/tmp/key-a':"
        );
        assert_eq!(load_session_passphrases()[0].secret, "ssh-passphrase");

        clear_session_passphrase();
    }

    #[test]
    fn remember_session_passphrase_overwrites_by_prompt() {
        let _lock = auth_test_lock();
        clear_session_passphrase();

        remember_session_passphrase("Enter passphrase for key '/tmp/key-a':", "first");
        remember_session_passphrase("Enter passphrase for key '/tmp/key-a':", "second");

        assert_eq!(load_session_passphrases().len(), 1);
        assert_eq!(load_session_passphrases()[0].secret, "second");

        clear_session_passphrase();
    }

    /// `echo %VAR%` with `VAR` unset collapses to a bare `echo`, which prints
    /// `ECHO is on.` — OpenSSH would take that as the passphrase. Every value
    /// must go out through `echo(` with delayed expansion instead.
    #[test]
    fn windows_askpass_script_never_echoes_percent_expansions() {
        let script = std::str::from_utf8(ASKPASS_SCRIPT_WINDOWS).expect("script is utf-8");

        for (idx, line) in script.lines().enumerate() {
            let trimmed = line.trim();
            // The interpreter directive, not a value-emitting `echo`.
            if trimmed == "@echo off" {
                continue;
            }
            let Some(pos) = trimmed.find("echo") else {
                continue;
            };
            let echo = &trimmed[pos..];
            assert!(
                echo.starts_with("echo("),
                "line {} uses a bare `echo`, which prints `ECHO is on.` when the \
                 value is empty: {trimmed}",
                idx + 1
            );
            // Everything up to the next pipe/redirect is the echoed value.
            let value = echo["echo(".len()..]
                .split(['|', '>'])
                .next()
                .unwrap_or_default();
            assert!(
                !value.contains('%'),
                "line {} percent-expands into `echo`; use delayed expansion so \
                 `&`, `|`, `<` and `>` in a secret are not parsed: {trimmed}",
                idx + 1
            );
        }

        // The fallthrough that answers a staged passphrase.
        assert!(script.contains("echo(!GITCOMET_AUTH_SECRET!"));
        assert!(script.contains("echo(!GITCOMET_AUTH_USERNAME!"));
        assert!(script.contains("echo(!cached_secret!"));
        assert!(script.contains("setlocal EnableDelayedExpansion"));
    }
}
