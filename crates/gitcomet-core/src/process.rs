mod probe;
use crate::services::CancellationToken;
pub(crate) use probe::probe_output;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub enum GitExecutablePreference {
    #[default]
    SystemPath,
    Custom(PathBuf),
}

impl GitExecutablePreference {
    pub fn from_optional_path(path: Option<PathBuf>) -> Self {
        match path {
            Some(path) if path.as_os_str().is_empty() => Self::Custom(PathBuf::new()),
            Some(path) => Self::Custom(normalize_git_executable_path(path)),
            _ => Self::SystemPath,
        }
    }

    pub fn custom_path(&self) -> Option<&Path> {
        match self {
            Self::SystemPath => None,
            Self::Custom(path) => Some(path.as_path()),
        }
    }

    pub fn display_label(&self) -> String {
        match self {
            Self::SystemPath => "System PATH".to_string(),
            Self::Custom(path) if path.as_os_str().is_empty() => {
                "Custom executable (not selected)".to_string()
            }
            Self::Custom(path) => path.display().to_string(),
        }
    }

    fn command_program(&self) -> OsString {
        match self {
            Self::SystemPath => OsString::from("git"),
            Self::Custom(path) => path.as_os_str().to_os_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitExecutableAvailability {
    Checking,
    Available { version_output: String },
    Unavailable { detail: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRuntimeState {
    pub preference: GitExecutablePreference,
    pub availability: GitExecutableAvailability,
}

impl Default for GitRuntimeState {
    fn default() -> Self {
        current_git_runtime()
    }
}

impl GitRuntimeState {
    pub fn is_available(&self) -> bool {
        matches!(
            self.availability,
            GitExecutableAvailability::Available { .. }
        )
    }

    pub fn version_output(&self) -> Option<&str> {
        match &self.availability {
            GitExecutableAvailability::Available { version_output } => {
                Some(version_output.as_str())
            }
            GitExecutableAvailability::Checking | GitExecutableAvailability::Unavailable { .. } => {
                None
            }
        }
    }

    pub fn unavailable_detail(&self) -> Option<&str> {
        match &self.availability {
            GitExecutableAvailability::Checking | GitExecutableAvailability::Available { .. } => {
                None
            }
            GitExecutableAvailability::Unavailable { detail } => Some(detail.as_str()),
        }
    }
}

struct RuntimeSlot {
    state: GitRuntimeState,
    generation: u64,
    probing: bool,
    last_probe: Option<Instant>,
    cancellation: CancellationToken,
}

impl RuntimeSlot {
    fn new() -> Self {
        Self {
            state: GitRuntimeState {
                preference: GitExecutablePreference::SystemPath,
                availability: GitExecutableAvailability::Checking,
            },
            generation: 0,
            probing: false,
            last_probe: None,
            cancellation: CancellationToken::new(),
        }
    }
}

fn git_runtime_slot() -> &'static RwLock<RuntimeSlot> {
    static SLOT: OnceLock<RwLock<RuntimeSlot>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(RuntimeSlot::new()))
}

/// Select an executable without starting it. GUI callers probe asynchronously.
pub fn select_git_executable_path(path: Option<PathBuf>) -> GitRuntimeState {
    select_git_executable_preference(GitExecutablePreference::from_optional_path(path))
}

pub fn select_git_executable_preference(preference: GitExecutablePreference) -> GitRuntimeState {
    let mut slot = git_runtime_slot()
        .write()
        .unwrap_or_else(|err| err.into_inner());
    if slot.state.preference != preference {
        slot.cancellation.cancel();
        slot.cancellation = CancellationToken::new();
        slot.generation = slot.generation.wrapping_add(1);
        slot.probing = false;
        slot.last_probe = None;
        slot.state = GitRuntimeState {
            preference,
            availability: GitExecutableAvailability::Checking,
        };
    }
    slot.state.clone()
}

/// Owned request with a frozen executable and generation. `run` belongs on a
/// worker thread; dropping a request before running it releases the claim.
pub struct GitRuntimeProbe {
    preference: GitExecutablePreference,
    generation: u64,
    cancellation: CancellationToken,
}

/// Shared across windows: coalesce in-flight probes and rate-limit recovery.
/// Healthy focus events do not probe. Diagnostics may explicitly force a check.
pub fn begin_git_runtime_probe(force: bool) -> Option<GitRuntimeProbe> {
    let mut slot = git_runtime_slot()
        .write()
        .unwrap_or_else(|err| err.into_inner());
    if slot.probing
        || (!force
            && (slot.state.is_available()
                || slot
                    .last_probe
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(10))))
    {
        return None;
    }
    slot.probing = true;
    slot.generation = slot.generation.wrapping_add(1);
    slot.last_probe = Some(Instant::now());
    Some(GitRuntimeProbe {
        preference: slot.state.preference.clone(),
        generation: slot.generation,
        cancellation: slot.cancellation.clone(),
    })
}

impl GitRuntimeProbe {
    pub fn run(self) -> Option<GitRuntimeState> {
        let next = probe_git_runtime(self.preference.clone(), &self.cancellation);
        let mut slot = git_runtime_slot()
            .write()
            .unwrap_or_else(|err| err.into_inner());
        if self.cancellation.is_cancelled() || slot.generation != self.generation {
            return None;
        }
        slot.state = next.clone();
        slot.probing = false;
        Some(next)
    }
}

impl Drop for GitRuntimeProbe {
    fn drop(&mut self) {
        let mut slot = git_runtime_slot()
            .write()
            .unwrap_or_else(|err| err.into_inner());
        if slot.generation == self.generation {
            slot.probing = false;
        }
    }
}

#[cfg(test)]
fn git_runtime_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Create a background subprocess command preconfigured to avoid creating a
/// visible console window on Windows.
pub fn background_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    configure_background_command(&mut command);
    command
}

pub fn git_command() -> Command {
    git_command_for_preference(&current_git_executable_preference())
}

pub fn current_git_runtime() -> GitRuntimeState {
    git_runtime_slot()
        .read()
        .unwrap_or_else(|err| err.into_inner())
        .state
        .clone()
}

pub fn current_git_executable_preference() -> GitExecutablePreference {
    git_runtime_slot()
        .read()
        .unwrap_or_else(|err| err.into_inner())
        .state
        .preference
        .clone()
}

pub fn install_git_executable_preference(preference: GitExecutablePreference) -> GitRuntimeState {
    select_git_executable_preference(preference);
    begin_git_runtime_probe(true)
        .and_then(GitRuntimeProbe::run)
        .unwrap_or_else(current_git_runtime)
}

pub fn install_git_executable_path(path: Option<PathBuf>) -> GitRuntimeState {
    install_git_executable_preference(GitExecutablePreference::from_optional_path(path))
}

pub fn refresh_git_runtime() -> GitRuntimeState {
    let preference = current_git_executable_preference();
    install_git_executable_preference(preference)
}

/// Configure a background subprocess so it does not create a visible console
/// window on Windows when GitComet is running as a GUI-subsystem app.
pub fn configure_background_command(command: &mut std::process::Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

pub fn normalize_git_executable_path(path: PathBuf) -> PathBuf {
    if path.as_os_str().is_empty() {
        return path;
    }
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub(crate) fn git_command_for_preference(preference: &GitExecutablePreference) -> Command {
    let mut command = background_command(preference.command_program());
    // Repository config must not enable `ext::`, which runs an arbitrary
    // command. Set in the one constructor so no call site can forget it.
    command.arg("-c").arg("protocol.ext.allow=never");
    if let GitExecutablePreference::Custom(path) = preference
        && let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        prepend_command_path(&mut command, parent);
    }
    command
}

fn prepend_command_path(command: &mut Command, path: &Path) {
    let mut paths = Vec::new();
    paths.push(path.to_path_buf());
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        command.env("PATH", joined);
    }
}

fn probe_git_runtime(
    preference: GitExecutablePreference,
    cancellation: &CancellationToken,
) -> GitRuntimeState {
    if matches!(
        &preference,
        GitExecutablePreference::Custom(path) if path.as_os_str().is_empty()
    ) {
        return GitRuntimeState {
            preference,
            availability: GitExecutableAvailability::Unavailable {
                detail: "Custom Git executable is not configured. Choose an executable or switch back to System PATH.".to_string(),
            },
        };
    }

    let executable_label = preference.display_label();
    let mut command = git_command_for_preference(&preference);
    command.arg("--version");

    let availability = match probe_output(command, Duration::from_secs(5), cancellation) {
        Ok(output) if output.status.success() => {
            let version_output = if !output.stdout.is_empty() {
                bytes_to_text_preserving_utf8(&output.stdout)
                    .trim()
                    .to_string()
            } else {
                bytes_to_text_preserving_utf8(&output.stderr)
                    .trim()
                    .to_string()
            };
            if version_output.is_empty() {
                GitExecutableAvailability::Unavailable {
                    detail: format!(
                        "Git executable at {executable_label} returned no version text."
                    ),
                }
            } else {
                GitExecutableAvailability::Available { version_output }
            }
        }
        Ok(output) => {
            let detail = bytes_to_text_preserving_utf8(&output.stderr)
                .trim()
                .to_string();
            let detail = if detail.is_empty() {
                format!(
                    "Git executable at {executable_label} exited with {status}.",
                    status = output.status
                )
            } else {
                format!("Git executable at {executable_label} failed: {detail}")
            };
            GitExecutableAvailability::Unavailable { detail }
        }
        Err(err) => GitExecutableAvailability::Unavailable {
            detail: match preference {
                GitExecutablePreference::SystemPath => {
                    format!("Git executable was not found in System PATH: {err}")
                }
                GitExecutablePreference::Custom(_) => {
                    format!("Configured Git executable at {executable_label} is unavailable: {err}")
                }
            },
        },
    };

    GitRuntimeState {
        preference,
        availability,
    }
}

/// Renders byte output as UTF-8 text, escaping each invalid byte as `\xNN`.
pub fn bytes_to_text_preserving_utf8(bytes: &[u8]) -> String {
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

/// Writes one diagnostic line to stderr, ignoring any write failure.
///
/// Deliberately not `eprintln!`, which panics when stderr cannot be written.
/// Recovery and background diagnostics run on threads with no unwind guard;
/// a release build sets `windows_subsystem = "windows"`, so a GitComet launched
/// from Explorer has no stderr at all and a panic here would kill the very
/// worker the recovery exists to keep alive.
pub fn write_stderr_line(args: std::fmt::Arguments<'_>) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{args}");
}

#[cfg(test)]
pub fn lock_git_runtime_test() -> std::sync::MutexGuard<'static, ()> {
    git_runtime_test_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    struct GitRuntimePreferenceResetGuard {
        original: GitExecutablePreference,
    }

    impl GitRuntimePreferenceResetGuard {
        fn install(preference: GitExecutablePreference) -> Self {
            let original = current_git_executable_preference();
            let _ = install_git_executable_preference(preference);
            Self { original }
        }
    }

    impl Drop for GitRuntimePreferenceResetGuard {
        fn drop(&mut self) {
            let _ = install_git_executable_preference(self.original.clone());
        }
    }

    fn git_runtime_probe_count(probe_log: &Path) -> usize {
        fs::read_to_string(probe_log)
            .unwrap_or_else(|err| {
                panic!(
                    "read probe log {}: {err}; runtime: {:?}",
                    probe_log.display(),
                    current_git_runtime()
                )
            })
            .lines()
            .count()
    }

    #[test]
    fn selecting_and_reading_runtime_never_spawns_and_concurrent_requests_coalesce() {
        let _lock = lock_git_runtime_test();
        let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("probes");
        let (_script_dir, script) = create_git_probe_script("git version test", "", 0, Some(&log));
        select_git_executable_path(Some(script));
        for _ in 0..100 {
            assert_eq!(
                GitRuntimeState::default().availability,
                GitExecutableAvailability::Checking
            );
            let _ = git_command();
        }
        assert!(!log.exists());
        let request = begin_git_runtime_probe(false).unwrap();
        assert!(begin_git_runtime_probe(true).is_none());
        assert!(!log.exists());
        assert!(request.run().unwrap().is_available());
        for _ in 0..100 {
            assert!(begin_git_runtime_probe(false).is_none());
        }
        assert_eq!(git_runtime_probe_count(&log), 1);
    }

    #[test]
    fn superseded_probe_cannot_overwrite_new_preference_or_release_its_claim() {
        let _lock = lock_git_runtime_test();
        let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());
        let (_a, a) = create_git_probe_script("git version old", "", 0, None);
        let (_b, b) = create_git_probe_script("git version new", "", 0, None);
        select_git_executable_path(Some(a));
        let old = begin_git_runtime_probe(false).unwrap();
        select_git_executable_path(Some(b.clone()));
        let new = begin_git_runtime_probe(false).unwrap();
        assert!(old.run().is_none());
        assert!(begin_git_runtime_probe(true).is_none());
        assert_eq!(
            current_git_executable_preference().custom_path(),
            Some(b.as_path())
        );
        assert_eq!(new.run().unwrap().version_output(), Some("git version new"));
    }

    #[test]
    fn unavailable_focus_recovery_is_throttled_but_diagnostics_can_recheck() {
        let _lock = lock_git_runtime_test();
        let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());
        select_git_executable_path(Some(PathBuf::new()));
        assert!(
            !begin_git_runtime_probe(false)
                .unwrap()
                .run()
                .unwrap()
                .is_available()
        );
        assert!(begin_git_runtime_probe(false).is_none());
        assert!(begin_git_runtime_probe(true).is_some());
    }

    fn create_git_probe_script(
        stdout: &str,
        stderr: &str,
        exit_code: i32,
        probe_log: Option<&Path>,
    ) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create temp dir");
        #[cfg(unix)]
        let script = dir.path().join("git");
        #[cfg(windows)]
        let script = dir.path().join("git.cmd");

        write_git_probe_script(&script, stdout, stderr, exit_code, probe_log);
        (dir, script)
    }

    #[cfg(unix)]
    fn write_executable_script(script_path: &Path, script: &str) {
        // A concurrent test can fork while fs::write holds the script open,
        // inheriting the writable fd until exec even though it is CLOEXEC.
        // Probing the script in that window fails with ETXTBSY. Write in a
        // child instead so the test runner never owns that writable fd, and
        // wait for the writer to exit before making the script executable.
        // https://github.com/rust-lang/rust/issues/114554
        let output = Command::new("/bin/sh")
            .args([
                "-c",
                "umask 077; printf '%s' \"$2\" > \"$1\"",
                "write-git-fixture",
            ])
            .arg(script_path)
            .arg(script)
            .output()
            .expect("run git fixture writer");
        assert!(
            output.status.success(),
            "write git fixture {} failed ({}): {}",
            script_path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        fs::set_permissions(script_path, fs::Permissions::from_mode(0o700))
            .expect("set git fixture permissions");
    }

    #[cfg(unix)]
    fn write_git_probe_script(
        script_path: &Path,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
        probe_log: Option<&Path>,
    ) {
        let mut script = String::from("#!/bin/sh\n");
        if let Some(probe_log) = probe_log {
            script.push_str(&format!("printf 'probe\\n' >> '{}'\n", probe_log.display()));
        }
        if !stdout.is_empty() {
            script.push_str(&format!("printf '%s\\n' '{stdout}'\n"));
        }
        if !stderr.is_empty() {
            script.push_str(&format!("printf '%s\\n' '{stderr}' >&2\n"));
        }
        script.push_str(&format!("exit {exit_code}\n"));

        write_executable_script(script_path, &script);
    }

    #[cfg(windows)]
    fn write_git_probe_script(
        script_path: &Path,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
        probe_log: Option<&Path>,
    ) {
        let mut script = String::from("@echo off\r\n");
        if let Some(probe_log) = probe_log {
            script.push_str(&format!(">>\"{}\" echo probe\r\n", probe_log.display()));
        }
        if !stdout.is_empty() {
            script.push_str(&format!("echo {stdout}\r\n"));
        }
        if !stderr.is_empty() {
            script.push_str(&format!("1>&2 echo {stderr}\r\n"));
        }
        script.push_str(&format!("exit /b {exit_code}\r\n"));
        fs::write(script_path, script).expect("write git probe script");
    }

    #[test]
    fn normalize_git_executable_path_makes_relative_paths_absolute() {
        let path = normalize_git_executable_path(PathBuf::from("test-git"));
        assert!(
            path.is_absolute(),
            "expected absolute path, got {}",
            path.display()
        );
    }

    #[cfg(unix)]
    #[test]
    fn normalize_git_executable_path_preserves_absolute_symlink() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let target = dir.path().join("store-git");
        let link = dir.path().join("profile-git");
        fs::write(&target, b"git").expect("write target");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");

        let normalized = normalize_git_executable_path(link.clone());

        assert_eq!(normalized, link);
        assert_ne!(normalized, target);
    }

    #[test]
    fn git_executable_preference_from_optional_path_covers_all_variants() {
        let relative = PathBuf::from("test-git");

        assert_eq!(
            GitExecutablePreference::from_optional_path(None),
            GitExecutablePreference::SystemPath
        );
        assert_eq!(
            GitExecutablePreference::from_optional_path(Some(PathBuf::new())),
            GitExecutablePreference::Custom(PathBuf::new())
        );
        assert_eq!(
            GitExecutablePreference::from_optional_path(Some(relative.clone())),
            GitExecutablePreference::Custom(normalize_git_executable_path(relative))
        );
    }

    #[test]
    fn git_executable_preference_custom_path_and_display_label_cover_variants() {
        let custom = GitExecutablePreference::Custom(PathBuf::from("/opt/git/bin/git"));
        let empty_custom = GitExecutablePreference::Custom(PathBuf::new());

        assert_eq!(GitExecutablePreference::SystemPath.custom_path(), None);
        assert_eq!(custom.custom_path(), Some(Path::new("/opt/git/bin/git")));
        assert_eq!(empty_custom.custom_path(), Some(Path::new("")));

        assert_eq!(
            GitExecutablePreference::SystemPath.display_label(),
            "System PATH"
        );
        assert_eq!(
            empty_custom.display_label(),
            "Custom executable (not selected)"
        );
        assert_eq!(custom.display_label(), "/opt/git/bin/git");
    }

    #[test]
    fn install_git_executable_preference_reports_missing_custom_path() {
        let _lock = lock_git_runtime_test();
        let missing = std::env::temp_dir().join("gitcomet-missing-git-executable");
        let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

        let state =
            install_git_executable_preference(GitExecutablePreference::Custom(missing.clone()));

        assert!(!state.is_available());
        assert_eq!(
            state.preference,
            GitExecutablePreference::Custom(missing.clone())
        );
        assert!(
            state
                .unavailable_detail()
                .expect("expected unavailable detail")
                .contains(&missing.display().to_string())
        );
    }

    #[test]
    fn install_git_executable_preference_uses_custom_executable_stdout() {
        let _lock = lock_git_runtime_test();
        let (_dir, script) = create_git_probe_script("git version 9.9.9-test", "", 0, None);
        let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

        let state =
            install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));
        assert!(state.is_available());
        assert_eq!(state.version_output(), Some("git version 9.9.9-test"));
    }

    #[test]
    fn install_git_executable_preference_uses_stderr_when_stdout_is_empty() {
        let _lock = lock_git_runtime_test();
        let (_dir, script) = create_git_probe_script("", "git version 8.8.8-test", 0, None);
        let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

        let state =
            install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));

        assert!(state.is_available());
        assert_eq!(state.version_output(), Some("git version 8.8.8-test"));
    }

    #[test]
    fn install_git_executable_preference_reports_empty_version_output() {
        let _lock = lock_git_runtime_test();
        let (_dir, script) = create_git_probe_script("", "", 0, None);
        let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

        let state =
            install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));

        assert!(!state.is_available());
        assert_eq!(
            state.unavailable_detail(),
            Some(
                &format!(
                    "Git executable at {} returned no version text.",
                    script.display()
                )[..]
            )
        );
    }

    #[test]
    fn install_git_executable_preference_reports_process_failure_with_stderr() {
        let _lock = lock_git_runtime_test();
        let (_dir, script) = create_git_probe_script("", "fatal: not a git executable", 7, None);
        let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

        let state =
            install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));

        assert!(!state.is_available());
        assert_eq!(
            state.unavailable_detail(),
            Some(
                &format!(
                    "Git executable at {} failed: fatal: not a git executable",
                    script.display()
                )[..]
            )
        );
    }

    #[test]
    fn install_git_executable_preference_reports_process_failure_without_stderr() {
        let _lock = lock_git_runtime_test();
        let (_dir, script) = create_git_probe_script("", "", 7, None);
        let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

        let state =
            install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));

        assert!(!state.is_available());
        let detail = state
            .unavailable_detail()
            .expect("expected unavailable detail");
        assert!(detail.contains(&script.display().to_string()));
        assert!(detail.contains("exited with"));
    }

    #[test]
    fn install_git_executable_preference_reports_missing_custom_selection() {
        let _lock = lock_git_runtime_test();
        let _restore = GitRuntimePreferenceResetGuard::install(current_git_executable_preference());

        let state =
            install_git_executable_preference(GitExecutablePreference::Custom(PathBuf::new()));

        assert!(!state.is_available());
        assert_eq!(
            state.preference,
            GitExecutablePreference::Custom(PathBuf::new())
        );
        assert_eq!(
            state.unavailable_detail(),
            Some(
                "Custom Git executable is not configured. Choose an executable or switch back to System PATH."
            )
        );
    }

    #[test]
    fn git_command_uses_installed_preference() {
        let _lock = lock_git_runtime_test();
        let temp = tempfile::tempdir().expect("create temp dir");
        let probe_log = temp.path().join("git-probes.log");
        let (_dir, script) =
            create_git_probe_script("git version 7.7.7-test", "", 0, Some(&probe_log));
        let _restore =
            GitRuntimePreferenceResetGuard::install(GitExecutablePreference::Custom(script));

        assert_eq!(
            git_runtime_probe_count(&probe_log),
            1,
            "installing a custom executable should probe exactly once"
        );

        let output = git_command()
            .arg("--version")
            .output()
            .expect("run configured git command");
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "git version 7.7.7-test"
        );
        assert_eq!(
            git_runtime_probe_count(&probe_log),
            2,
            "running the returned command should invoke the configured executable"
        );
    }

    #[test]
    fn git_command_prepends_custom_git_parent_to_path() {
        let _lock = lock_git_runtime_test();
        let (_dir, script) = create_git_probe_script("git version 5.5.5-test", "", 0, None);
        let _restore = GitRuntimePreferenceResetGuard::install(GitExecutablePreference::Custom(
            script.clone(),
        ));

        let command = git_command();
        let path_env = command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("PATH"))
            .and_then(|(_, value)| value.map(ToOwned::to_owned))
            .expect("custom git command should set PATH");
        let paths: Vec<_> = std::env::split_paths(&path_env).collect();

        assert_eq!(
            paths.first().map(PathBuf::as_path),
            script.parent(),
            "custom git parent should be first in PATH"
        );
    }

    #[cfg(unix)]
    #[test]
    fn custom_git_can_resolve_sibling_helper_from_path() {
        let _lock = lock_git_runtime_test();
        let dir = tempfile::tempdir().expect("create temp dir");
        let git = dir.path().join("git");
        let gpg = dir.path().join("gpg");

        write_executable_script(&git, "#!/bin/sh\ngpg --version\n");
        write_executable_script(&gpg, "#!/bin/sh\nprintf 'sibling gpg 1.0\\n'\n");

        let _restore =
            GitRuntimePreferenceResetGuard::install(GitExecutablePreference::Custom(git));

        assert_eq!(
            current_git_runtime().version_output(),
            Some("sibling gpg 1.0")
        );
    }

    #[test]
    fn refresh_git_runtime_reprobes_current_preference() {
        let _lock = lock_git_runtime_test();
        let temp = tempfile::tempdir().expect("create temp dir");
        let probe_log = temp.path().join("git-probes.log");
        let (_dir, script) =
            create_git_probe_script("git version 6.6.6-test", "", 0, Some(&probe_log));
        let _restore =
            GitRuntimePreferenceResetGuard::install(GitExecutablePreference::Custom(script));

        assert_eq!(git_runtime_probe_count(&probe_log), 1);
        let refreshed = refresh_git_runtime();

        assert!(refreshed.is_available());
        assert_eq!(refreshed.version_output(), Some("git version 6.6.6-test"));
        assert_eq!(
            git_runtime_probe_count(&probe_log),
            2,
            "refresh should re-run the current runtime probe"
        );
    }

    #[test]
    fn bytes_to_text_preserving_utf8_preserves_valid_utf8() {
        assert_eq!(
            bytes_to_text_preserving_utf8("cafe 日本語".as_bytes()),
            "cafe 日本語"
        );
    }

    #[test]
    fn bytes_to_text_preserving_utf8_escapes_invalid_bytes_without_losing_valid_segments() {
        let input = b"ok\xffstill-valid\xc3\xa9\x80done";
        assert_eq!(
            bytes_to_text_preserving_utf8(input),
            "ok\\xffstill-validé\\x80done"
        );
    }

    #[test]
    fn bytes_to_text_preserving_utf8_escapes_truncated_multibyte_tail() {
        assert_eq!(
            bytes_to_text_preserving_utf8(b"snowman \xe2\x98"),
            "snowman \\xe2\\x98"
        );
    }

    #[test]
    fn bytes_to_text_preserving_utf8_handles_empty_input() {
        assert_eq!(bytes_to_text_preserving_utf8(b""), "");
    }
}
