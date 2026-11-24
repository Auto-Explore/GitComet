mod probe;
use crate::services::CancellationToken;
pub(crate) use probe::probe_output;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Condvar, Mutex, OnceLock, RwLock};
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

/// A `major.minor` git version, as reported by `git --version`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
}

impl GitVersion {
    /// Oldest git GitComet supports; older installs get an update notice.
    pub const MINIMUM: Self = Self {
        major: 2,
        minor: 53,
    };
    /// First git with `git maintenance is-needed`.
    pub const MAINTENANCE_IS_NEEDED: Self = Self {
        major: 2,
        minor: 53,
    };

    /// The first version token of `git --version` output, e.g.
    /// `git version 2.45.1.windows.1`.
    pub fn parse(raw: &str) -> Option<Self> {
        raw.split_whitespace().find_map(Self::parse_token)
    }

    fn parse_token(token: &str) -> Option<Self> {
        let mut parts = token.split('.');
        let major = parse_u32_prefix(parts.next()?)?;
        let minor = parse_u32_prefix(parts.next()?)?;
        Some(Self { major, minor })
    }

    pub fn is_supported(self) -> bool {
        self >= Self::MINIMUM
    }
}

impl std::fmt::Display for GitVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

fn parse_u32_prefix(part: &str) -> Option<u32> {
    let end = part
        .char_indices()
        .find_map(|(ix, ch)| (!ch.is_ascii_digit()).then_some(ix))
        .unwrap_or(part.len());
    if end == 0 {
        return None;
    }
    part[..end].parse::<u32>().ok()
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

    /// The detected git version, when git is available and reports one.
    pub fn version(&self) -> Option<GitVersion> {
        self.version_output().and_then(GitVersion::parse)
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
        let abandoned_probe = std::mem::replace(&mut slot.probing, false);
        slot.last_probe = None;
        slot.state = GitRuntimeState {
            preference,
            availability: GitExecutableAvailability::Checking,
        };
        let state = slot.state.clone();
        drop(slot);
        if abandoned_probe {
            notify_probe_settled();
        }
        return state;
    }
    slot.state.clone()
}

/// Owned request with a frozen executable and generation. `run` belongs on a
/// worker thread; dropping a request before running it releases the claim.
pub struct GitRuntimeProbe {
    preference: GitExecutablePreference,
    generation: u64,
    cancellation: CancellationToken,
    /// An unrun claim learned nothing, so dropping it must not rate-limit the
    /// next request.
    ran: bool,
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
        ran: false,
    })
}

impl GitRuntimeProbe {
    pub fn run(mut self) -> Option<GitRuntimeState> {
        self.ran = true;
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
        let settled = slot.generation == self.generation;
        if settled {
            slot.probing = false;
            if !self.ran {
                slot.last_probe = None;
            }
        }
        // Never wake waiters while holding the slot: they read it under the
        // signal's lock.
        drop(slot);
        if settled {
            notify_probe_settled();
        }
    }
}

/// Wakes `wait_for_git_runtime_probe` callers. Notifiers take the lock after
/// clearing `probing`, so a waiter cannot miss the wake between its check of
/// the slot and its wait.
fn probe_settled_signal() -> &'static (Mutex<()>, Condvar) {
    static SIGNAL: OnceLock<(Mutex<()>, Condvar)> = OnceLock::new();
    SIGNAL.get_or_init(|| (Mutex::new(()), Condvar::new()))
}

fn notify_probe_settled() {
    let (lock, settled) = probe_settled_signal();
    let _guard = lock.lock().unwrap_or_else(|err| err.into_inner());
    settled.notify_all();
}

/// The settled runtime, waiting for a probe someone else started if one is
/// running. The browser starts one before GPUI and its first window exist, so
/// that a window can open its repositories without first waiting for
/// `git --version`; that probe reports to no window, so a window adopts its
/// result here instead of probing again.
///
/// `None` while nothing is known yet: no probe ran, the running one was
/// abandoned (the executable preference changed), or `timeout` passed first.
pub fn wait_for_git_runtime_probe(timeout: Duration) -> Option<GitRuntimeState> {
    let deadline = Instant::now() + timeout;
    let (lock, settled) = probe_settled_signal();
    let mut guard = lock.lock().unwrap_or_else(|err| err.into_inner());
    loop {
        {
            let slot = git_runtime_slot()
                .read()
                .unwrap_or_else(|err| err.into_inner());
            if !slot.probing {
                return (!matches!(slot.state.availability, GitExecutableAvailability::Checking))
                    .then(|| slot.state.clone());
            }
        }
        let remaining = deadline.checked_duration_since(Instant::now())?;
        guard = settled
            .wait_timeout(guard, remaining)
            .unwrap_or_else(|err| err.into_inner())
            .0;
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
    // No gc/repack behind a command: GitComet recommends maintenance and runs
    // it only when asked. An explicit `git maintenance run` ignores this.
    command.arg("-c").arg("maintenance.auto=false");
    // GitComet reads Git's messages, so they must be English; a UTF-8
    // charset keeps hooks and helpers from failing on non-ASCII text.
    let inherited: Vec<_> = std::env::vars_os().collect();
    for (name, value) in git_locale_env(&inherited, utf8_locale) {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    if let GitExecutablePreference::Custom(path) = preference
        && let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        prepend_command_path(&mut command, parent);
    }
    command
}

/// Environment changes that make Git print English in a UTF-8 charset, given
/// the inherited environment. `LANGUAGE` picks the messages even over the
/// user's `LC_ALL`; numbers, dates and sorting keep the user's locale.
fn git_locale_env(
    inherited: &[(OsString, OsString)],
    utf8_locale: impl FnOnce() -> Option<&'static str>,
) -> Vec<(OsString, Option<OsString>)> {
    let var = |name: &str| {
        inherited
            .iter()
            .find(|(key, value)| key == name && !value.is_empty())
            .map(|(_, value)| value.clone())
    };
    let mut changes = vec![("LANGUAGE".into(), Some("en".into()))];
    let all = var("LC_ALL");
    let charset = all
        .clone()
        .or_else(|| var("LC_CTYPE"))
        .or_else(|| var("LANG"));
    let names_utf8 = |locale: &OsStr| {
        let locale = locale.to_string_lossy().to_ascii_lowercase();
        locale.contains("utf-8") || locale.contains("utf8")
    };
    if charset.as_deref().is_some_and(names_utf8) {
        return changes;
    }
    let Some(utf8) = utf8_locale() else {
        return changes;
    };
    if let Some(all) = all {
        // LC_ALL would override LC_CTYPE. LANG gives the user's locale to
        // every other category, which the LC_* variables LC_ALL hid must not
        // take over.
        changes.extend(
            inherited
                .iter()
                .filter(|(name, _)| name.to_str().is_some_and(|name| name.starts_with("LC_")))
                .map(|(name, _)| (name.clone(), None)),
        );
        changes.push(("LANG".into(), Some(all)));
    }
    changes.push(("LC_CTYPE".into(), Some(utf8.into())));
    changes
}

/// A UTF-8 locale this machine has, for when the user's charset is not UTF-8.
/// Linux installs C.UTF-8 on most systems but not all, and a missing locale
/// makes Perl and bash hooks print warnings.
fn utf8_locale() -> Option<&'static str> {
    if cfg!(windows) {
        // Git for Windows runs in UTF-8 already.
        None
    } else if cfg!(target_os = "macos") {
        Some("UTF-8")
    } else {
        static INSTALLED: OnceLock<bool> = OnceLock::new();
        INSTALLED
            .get_or_init(|| {
                background_command("locale")
                    .arg("-a")
                    .output()
                    .is_ok_and(|output| {
                        String::from_utf8_lossy(&output.stdout).lines().any(|name| {
                            name.eq_ignore_ascii_case("C.UTF-8")
                                || name.eq_ignore_ascii_case("C.utf8")
                        })
                    })
            })
            .then_some("C.UTF-8")
    }
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
pub use gitcomet_base::write_stderr_line;

#[cfg(test)]
pub fn lock_git_runtime_test() -> std::sync::MutexGuard<'static, ()> {
    git_runtime_test_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests;
