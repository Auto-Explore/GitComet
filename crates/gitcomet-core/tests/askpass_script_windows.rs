#![cfg(windows)]

//! Executes the generated Windows askpass helper through `cmd.exe`.
//!
//! The helper is a batch script, so its behaviour cannot be verified by reading
//! the Rust that emits it — batch has its own expansion rules, and one of them
//! caused a real bug: `echo %VAR%` with `VAR` unset collapses to a bare `echo`,
//! which prints `ECHO is on.`. OpenSSH took that string as the passphrase, so a
//! signed commit failed with `incorrect passphrase supplied to decrypt private
//! key` even though the user had never been asked for one.
//!
//! These tests only run on Windows. On any other host the file compiles to
//! nothing, so the coverage exists exactly where the risk does.

use gitcomet_core::auth::{ASKPASS_SCRIPT_WINDOWS, askpass_script_name};
use std::path::PathBuf;
use std::process::Command;

/// Every variable the helper reads. Cleared before each run so an "unset"
/// variable is genuinely unset rather than inherited from the test runner.
const AUTH_ENV: &[&str] = &[
    "GITCOMET_AUTH_KIND",
    "GITCOMET_AUTH_USERNAME",
    "GITCOMET_AUTH_SECRET",
    "GITCOMET_AUTH_CACHE_SIZE",
    "GITCOMET_AUTH_CACHE_PROMPT_0",
    "GITCOMET_AUTH_CACHE_SECRET_0",
];

struct Askpass {
    _dir: tempfile::TempDir,
    path: PathBuf,
    host_log: PathBuf,
    passphrase_log: PathBuf,
}

impl Askpass {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create tempdir");
        let path = dir.path().join(askpass_script_name());
        let host_log = dir.path().join("gitcomet-askpass-host-prompt.log");
        let passphrase_log = dir.path().join("gitcomet-askpass-passphrase-prompt.log");

        std::fs::write(&path, ASKPASS_SCRIPT_WINDOWS).expect("write askpass script");
        std::fs::write(&host_log, b"").expect("create host prompt log");
        std::fs::write(&passphrase_log, b"").expect("create passphrase prompt log");

        Self {
            _dir: dir,
            path,
            host_log,
            passphrase_log,
        }
    }

    /// Run the helper the way OpenSSH does: one argument, the prompt text, with
    /// the answer taken from stdout.
    fn answer(&self, prompt: &str, env: &[(&str, &str)]) -> String {
        let mut cmd = Command::new(&self.path);
        cmd.arg(prompt);
        cmd.env("GITCOMET_ASKPASS_PROMPT_LOG", &self.host_log);
        cmd.env(
            "GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG",
            &self.passphrase_log,
        );
        for key in AUTH_ENV {
            cmd.env_remove(key);
        }
        for (key, value) in env {
            cmd.env(key, value);
        }

        let out = cmd.output().expect("run askpass script");
        // OpenSSH truncates the answer at the first CR or LF.
        String::from_utf8_lossy(&out.stdout)
            .split(['\r', '\n'])
            .next()
            .unwrap_or_default()
            .to_string()
    }

    fn host_log(&self) -> String {
        std::fs::read_to_string(&self.host_log).expect("read host prompt log")
    }

    fn passphrase_log(&self) -> String {
        std::fs::read_to_string(&self.passphrase_log).expect("read passphrase prompt log")
    }
}

/// The regression test for the reported bug. With nothing staged the helper must
/// answer with an empty line, never with batch's `ECHO is on.` banner.
#[test]
fn no_staged_secret_answers_with_an_empty_line() {
    let askpass = Askpass::new();

    let answer = askpass.answer("Enter passphrase for key: ", &[]);

    assert!(
        !answer.to_ascii_uppercase().contains("ECHO"),
        "helper leaked batch output as the passphrase: {answer:?}"
    );
    assert!(
        answer.is_empty(),
        "expected an empty answer when nothing is staged, got {answer:?}"
    );
}

#[test]
fn staged_passphrase_is_answered_verbatim() {
    let askpass = Askpass::new();

    let answer = askpass.answer(
        "Enter passphrase for key: ",
        &[
            ("GITCOMET_AUTH_KIND", "passphrase"),
            ("GITCOMET_AUTH_SECRET", "hunter2"),
        ],
    );

    assert_eq!(answer, "hunter2");
}

/// Delayed expansion (`!VAR!`) is what keeps these from being parsed as batch
/// operators. Under the old `echo %VAR%` form this answer came back truncated.
#[test]
fn secret_with_shell_metacharacters_is_answered_verbatim() {
    let askpass = Askpass::new();
    let secret = "p@ss&word|with<meta>chars^and(parens)";

    let answer = askpass.answer(
        "Enter passphrase for key: ",
        &[
            ("GITCOMET_AUTH_KIND", "passphrase"),
            ("GITCOMET_AUTH_SECRET", secret),
        ],
    );

    assert_eq!(answer, secret);
}

/// `!` is the one character delayed expansion itself treats specially. If this
/// is the only failing test, see `docs/windows-ssh-signing-testing.md` — the
/// remedy is to toggle delayed expansion off around the final `echo`.
#[test]
fn secret_with_exclamation_marks_is_answered_verbatim() {
    let askpass = Askpass::new();
    let secret = "let-me-in!please!";

    let answer = askpass.answer(
        "Enter passphrase for key: ",
        &[
            ("GITCOMET_AUTH_KIND", "passphrase"),
            ("GITCOMET_AUTH_SECRET", secret),
        ],
    );

    assert_eq!(answer, secret);
}

/// The logged prompt is what lets a failed command be classified as "a
/// passphrase was asked for", so it must survive the round trip through cmd.
#[test]
fn ssh_keygen_passphrase_prompt_is_logged() {
    let askpass = Askpass::new();

    askpass.answer(
        "Enter passphrase for \"C:\\Users\\dev\\.ssh\\id_ed25519_signing\": ",
        &[],
    );

    let logged = askpass.passphrase_log();
    assert!(
        logged.to_ascii_lowercase().contains("passphrase"),
        "expected the passphrase prompt to be logged, got {logged:?}"
    );
    assert!(
        askpass.host_log().trim().is_empty(),
        "a passphrase prompt must not be logged as a host prompt"
    );
}

#[test]
fn host_verification_prompt_is_logged_and_answered() {
    let askpass = Askpass::new();
    let prompt = "Are you sure you want to continue connecting (yes/no/[fingerprint])? ";

    let answer = askpass.answer(
        prompt,
        &[
            ("GITCOMET_AUTH_KIND", "host_verification"),
            ("GITCOMET_AUTH_SECRET", "yes"),
        ],
    );

    assert_eq!(answer, "yes");
    assert!(
        askpass
            .host_log()
            .to_ascii_lowercase()
            .contains("connecting"),
        "expected the host prompt to be logged, got {:?}",
        askpass.host_log()
    );
}

#[test]
fn username_password_answers_each_prompt_with_its_own_value() {
    let askpass = Askpass::new();
    let env = [
        ("GITCOMET_AUTH_KIND", "username_password"),
        ("GITCOMET_AUTH_USERNAME", "octocat"),
        ("GITCOMET_AUTH_SECRET", "ghp_token"),
    ];

    assert_eq!(
        askpass.answer("Username for 'https://example.com': ", &env),
        "octocat"
    );
    assert_eq!(
        askpass.answer("Password for 'https://octocat@example.com': ", &env),
        "ghp_token"
    );
}

/// The session cache keys on the exact prompt text, so a second signed commit
/// reuses the passphrase instead of prompting again.
#[test]
fn cached_passphrase_answers_only_its_own_prompt() {
    let askpass = Askpass::new();
    let env = [
        ("GITCOMET_AUTH_KIND", "passphrase_cached"),
        ("GITCOMET_AUTH_CACHE_SIZE", "1"),
        (
            "GITCOMET_AUTH_CACHE_PROMPT_0",
            "Enter passphrase for key-a:",
        ),
        ("GITCOMET_AUTH_CACHE_SECRET_0", "cached-secret"),
    ];

    assert_eq!(
        askpass.answer("Enter passphrase for key-a:", &env),
        "cached-secret"
    );
    assert_eq!(
        askpass.answer("Enter passphrase for key-b:", &env),
        "",
        "a different key must not be answered with the cached passphrase"
    );
}
