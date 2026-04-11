use crate::path_utils::canonicalize_or_original;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{OnceLock, RwLock};

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
            GitExecutableAvailability::Unavailable { .. } => None,
        }
    }

    pub fn unavailable_detail(&self) -> Option<&str> {
        match &self.availability {
            GitExecutableAvailability::Available { .. } => None,
            GitExecutableAvailability::Unavailable { detail } => Some(detail.as_str()),
        }
    }
}

fn git_runtime_slot() -> &'static RwLock<GitRuntimeState> {
    static SLOT: OnceLock<RwLock<GitRuntimeState>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(probe_git_runtime(GitExecutablePreference::SystemPath)))
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
    git_command_for_preference(&current_git_runtime().preference)
}

pub fn current_git_runtime() -> GitRuntimeState {
    git_runtime_slot()
        .read()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

pub fn current_git_executable_preference() -> GitExecutablePreference {
    current_git_runtime().preference
}

pub fn install_git_executable_preference(preference: GitExecutablePreference) -> GitRuntimeState {
    let next = probe_git_runtime(preference);
    *git_runtime_slot()
        .write()
        .unwrap_or_else(|err| err.into_inner()) = next.clone();
    next
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
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    canonicalize_or_original(path)
}

fn git_command_for_preference(preference: &GitExecutablePreference) -> Command {
    background_command(preference.command_program())
}

fn probe_git_runtime(preference: GitExecutablePreference) -> GitRuntimeState {
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

    let availability = match command.output() {
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

    #[test]
    fn normalize_git_executable_path_makes_relative_paths_absolute() {
        let path = normalize_git_executable_path(PathBuf::from("test-git"));
        assert!(
            path.is_absolute(),
            "expected absolute path, got {}",
            path.display()
        );
    }

    #[test]
    fn install_git_executable_preference_reports_missing_custom_path() {
        let _lock = lock_git_runtime_test();
        let original = current_git_executable_preference();

        let missing = std::env::temp_dir().join("gitcomet-missing-git-executable");
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

        let _ = install_git_executable_preference(original);
    }

    #[test]
    fn install_git_executable_preference_uses_custom_executable() {
        let _lock = lock_git_runtime_test();
        let original = current_git_executable_preference();

        let dir = tempfile::tempdir().expect("create temp dir");
        #[cfg(unix)]
        let script = dir.path().join("git");
        #[cfg(windows)]
        let script = dir.path().join("git.cmd");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            fs::write(&script, "#!/bin/sh\necho 'git version 9.9.9-test'\n").expect("write script");
            let mut permissions = fs::metadata(&script).expect("metadata").permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&script, permissions).expect("set permissions");
        }

        #[cfg(windows)]
        {
            fs::write(&script, "@echo off\r\necho git version 9.9.9-test\r\n")
                .expect("write script");
        }

        let state =
            install_git_executable_preference(GitExecutablePreference::Custom(script.clone()));
        assert!(state.is_available());
        assert_eq!(state.version_output(), Some("git version 9.9.9-test"));

        let _ = install_git_executable_preference(original);
    }

    #[test]
    fn install_git_executable_preference_reports_missing_custom_selection() {
        let _lock = lock_git_runtime_test();
        let original = current_git_executable_preference();

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

        let _ = install_git_executable_preference(original);
    }
}
