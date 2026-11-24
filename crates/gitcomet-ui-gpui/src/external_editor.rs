#![allow(
    clippy::disallowed_methods,
    reason = "scans desktop entries and PATH for installed editors"
)]

use gitcomet_core::process::background_command;
use gitcomet_state::session::{self, ExternalCodeEditorSetting};
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const CUSTOM_PATH_PLACEHOLDER: &str = "{path}";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DetectedExternalEditor {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) path: PathBuf,
    terminal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExternalEditorOptionKind {
    None,
    Detected(ExternalCodeEditorSetting),
    Custom,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExternalEditorOption {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) detail: Option<String>,
    pub(crate) missing: bool,
    pub(crate) kind: ExternalEditorOptionKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExternalEditorLaunchCommand {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<OsString>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExternalEditorError {
    NotConfigured,
    EmptyCustomExecutable,
    InvalidCustomArguments(String),
    NoTerminalLauncher,
    Spawn(String),
}

impl fmt::Display for ExternalEditorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "External code editor is not configured"),
            Self::EmptyCustomExecutable => write!(f, "Custom editor executable is empty"),
            Self::InvalidCustomArguments(err) => {
                write!(f, "Invalid custom editor arguments: {err}")
            }
            Self::NoTerminalLauncher => write!(f, "No supported terminal launcher was found"),
            Self::Spawn(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ExternalEditorError {}

impl From<io::Error> for ExternalEditorError {
    fn from(value: io::Error) -> Self {
        Self::Spawn(value.to_string())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExternalEditorDetectionEnv {
    pub(crate) path_dirs: Vec<PathBuf>,
    pub(crate) application_dirs: Vec<PathBuf>,
    pub(crate) jetbrains_toolbox_dirs: Vec<PathBuf>,
    pub(crate) jetbrains_install_dirs: Vec<PathBuf>,
    pub(crate) visual_studio_roots: Vec<PathBuf>,
}

impl ExternalEditorDetectionEnv {
    pub(crate) fn from_current_process() -> Self {
        let path_dirs = env::var_os("PATH")
            .map(|path| env::split_paths(&path).collect())
            .unwrap_or_default();

        let mut application_dirs = Vec::new();
        let mut jetbrains_toolbox_dirs = Vec::new();
        #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
        let mut jetbrains_install_dirs = Vec::new();
        #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
        let mut visual_studio_roots = Vec::new();

        #[cfg(target_os = "macos")]
        {
            application_dirs.push(PathBuf::from("/Applications"));
            if let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty()) {
                let home = PathBuf::from(home);
                application_dirs.push(home.join("Applications"));
                jetbrains_toolbox_dirs
                    .push(home.join("Library/Application Support/JetBrains/Toolbox/apps"));
            }
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            application_dirs.push(PathBuf::from("/usr/share/applications"));
            application_dirs.push(PathBuf::from("/opt"));
            if let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty()) {
                let home = PathBuf::from(home);
                application_dirs.push(home.join(".local/share/applications"));
                jetbrains_toolbox_dirs.push(home.join(".local/share/JetBrains/Toolbox/apps"));
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(program_files) = env::var_os("ProgramFiles").filter(|v| !v.is_empty()) {
                let program_files = PathBuf::from(program_files);
                application_dirs.push(program_files.clone());
                jetbrains_install_dirs.push(program_files.join("JetBrains"));
                visual_studio_roots.push(program_files);
            }
            if let Some(program_files_x86) =
                env::var_os("ProgramFiles(x86)").filter(|v| !v.is_empty())
            {
                let program_files_x86 = PathBuf::from(program_files_x86);
                jetbrains_install_dirs.push(program_files_x86.join("JetBrains"));
                visual_studio_roots.push(program_files_x86);
            }
            if let Some(local_app_data) = env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
                jetbrains_toolbox_dirs
                    .push(PathBuf::from(local_app_data).join("JetBrains/Toolbox/apps"));
            }
        }

        Self {
            path_dirs,
            application_dirs,
            jetbrains_toolbox_dirs,
            jetbrains_install_dirs,
            visual_studio_roots,
        }
    }
}

#[derive(Clone, Copy)]
struct PathEditorSpec {
    id: &'static str,
    label: &'static str,
    names: &'static [&'static str],
    terminal: bool,
}

const PATH_EDITOR_SPECS: &[PathEditorSpec] = &[
    PathEditorSpec {
        id: "vscode",
        label: "Visual Studio Code",
        names: &["code"],
        terminal: false,
    },
    PathEditorSpec {
        id: "vscode-insiders",
        label: "Visual Studio Code Insiders",
        names: &["code-insiders"],
        terminal: false,
    },
    PathEditorSpec {
        id: "vscodium",
        label: "VSCodium",
        names: &["codium", "vscodium"],
        terminal: false,
    },
    PathEditorSpec {
        id: "code-oss",
        label: "Code - OSS",
        names: &["code-oss"],
        terminal: false,
    },
    PathEditorSpec {
        id: "zed",
        label: "Zed",
        names: &["zed", "zeditor"],
        terminal: false,
    },
    PathEditorSpec {
        id: "sublime-text",
        label: "Sublime Text",
        names: &["subl", "sublime_text", "sublime_text.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "xcode",
        label: "Xcode",
        names: &["xed"],
        terminal: false,
    },
    PathEditorSpec {
        id: "android-studio",
        label: "Android Studio",
        names: &["studio", "studio.sh", "studio64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "jetbrains-idea",
        label: "IntelliJ IDEA",
        names: &["idea", "idea.sh", "idea64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "jetbrains-pycharm",
        label: "PyCharm",
        names: &["pycharm", "pycharm.sh", "pycharm64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "jetbrains-webstorm",
        label: "WebStorm",
        names: &["webstorm", "webstorm.sh", "webstorm64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "jetbrains-goland",
        label: "GoLand",
        names: &["goland", "goland.sh", "goland64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "jetbrains-clion",
        label: "CLion",
        names: &["clion", "clion.sh", "clion64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "jetbrains-rider",
        label: "Rider",
        names: &["rider", "rider.sh", "rider64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "jetbrains-rustrover",
        label: "RustRover",
        names: &["rustrover", "rustrover.sh", "rustrover64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "jetbrains-rubymine",
        label: "RubyMine",
        names: &["rubymine", "rubymine.sh", "rubymine64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "jetbrains-phpstorm",
        label: "PhpStorm",
        names: &["phpstorm", "phpstorm.sh", "phpstorm64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "jetbrains-datagrip",
        label: "DataGrip",
        names: &["datagrip", "datagrip.sh", "datagrip64.exe"],
        terminal: false,
    },
    PathEditorSpec {
        id: "neovim-gui",
        label: "Neovim",
        names: &["nvim-qt", "neovide"],
        terminal: false,
    },
    PathEditorSpec {
        id: "vim-gui",
        label: "Vim",
        names: &["gvim", "mvim"],
        terminal: false,
    },
];

const TERMINAL_EDITOR_SPECS: &[PathEditorSpec] = &[
    PathEditorSpec {
        id: "neovim-terminal",
        label: "Neovim (terminal)",
        names: &["nvim"],
        terminal: true,
    },
    PathEditorSpec {
        id: "vim-terminal",
        label: "Vim (terminal)",
        names: &["vim"],
        terminal: true,
    },
];

const TERMINAL_LAUNCHERS: &[&str] = &[
    "wt.exe",
    "wt",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "x-terminal-emulator",
    "alacritty",
    "kitty",
    "wezterm",
    "xterm",
];

#[derive(Clone, Copy)]
struct MacAppSpec {
    id: &'static str,
    label: &'static str,
    bundle_name: &'static str,
    executable_relatives: &'static [&'static str],
}

const MAC_APP_SPECS: &[MacAppSpec] = &[
    MacAppSpec {
        id: "vscode",
        label: "Visual Studio Code",
        bundle_name: "Visual Studio Code.app",
        executable_relatives: &["Contents/Resources/app/bin/code"],
    },
    MacAppSpec {
        id: "zed",
        label: "Zed",
        bundle_name: "Zed.app",
        executable_relatives: &["Contents/MacOS/cli", "Contents/MacOS/Zed"],
    },
    MacAppSpec {
        id: "sublime-text",
        label: "Sublime Text",
        bundle_name: "Sublime Text.app",
        executable_relatives: &["Contents/SharedSupport/bin/subl"],
    },
    MacAppSpec {
        id: "xcode",
        label: "Xcode",
        bundle_name: "Xcode.app",
        executable_relatives: &[],
    },
    MacAppSpec {
        id: "android-studio",
        label: "Android Studio",
        bundle_name: "Android Studio.app",
        executable_relatives: &["Contents/MacOS/studio"],
    },
];

pub(crate) fn detect_external_editors() -> Vec<DetectedExternalEditor> {
    detect_external_editors_with_env(&ExternalEditorDetectionEnv::from_current_process())
}

pub(crate) fn detect_external_editors_with_env(
    env: &ExternalEditorDetectionEnv,
) -> Vec<DetectedExternalEditor> {
    let mut editors = Vec::new();
    let mut seen = BTreeSet::new();

    for spec in PATH_EDITOR_SPECS
        .iter()
        .filter(|spec| path_editor_spec_supported(spec))
    {
        for path in find_programs(&env.path_dirs, spec.names) {
            push_detected(&mut editors, &mut seen, spec, path);
        }
    }

    detect_macos_app_bundles(env, &mut editors, &mut seen);
    detect_linux_desktop_editors(env, &mut editors, &mut seen);
    detect_jetbrains_toolbox(env, &mut editors, &mut seen);
    detect_visual_studio(env, &mut editors, &mut seen);

    let has_terminal = detect_terminal_launcher(env).is_some();
    if has_terminal {
        for spec in TERMINAL_EDITOR_SPECS {
            if terminal_editor_shadowed_by_gui(spec.id, &editors) {
                continue;
            }
            for path in find_programs(&env.path_dirs, spec.names) {
                push_detected(&mut editors, &mut seen, spec, path);
            }
        }
    }

    editors
}

pub(crate) fn external_editor_options_from_detected(
    saved: Option<&ExternalCodeEditorSetting>,
    detected: Vec<DetectedExternalEditor>,
) -> Vec<ExternalEditorOption> {
    let mut options = vec![ExternalEditorOption {
        id: "external_editor_none".to_string(),
        label: "None".to_string(),
        detail: Some("Do not show external editor actions".to_string()),
        missing: false,
        kind: ExternalEditorOptionKind::None,
    }];

    if let Some(ExternalCodeEditorSetting::Detected { id, path }) = saved {
        let already_detected = detected
            .iter()
            .any(|editor| editor.id == *id && editor.path == *path);
        if !already_detected {
            let missing = !path.exists();
            options.push(ExternalEditorOption {
                id: format!("external_editor_saved_{id}"),
                label: if missing {
                    format!("{} (missing)", editor_label_for_id(id))
                } else {
                    editor_label_for_id(id).to_string()
                },
                detail: Some(if missing {
                    format!("Missing: {}", path.display())
                } else {
                    format!("Saved: {}", path.display())
                }),
                missing,
                kind: ExternalEditorOptionKind::Detected(ExternalCodeEditorSetting::Detected {
                    id: id.clone(),
                    path: path.clone(),
                }),
            });
        }
    }

    for editor in detected {
        let setting = ExternalCodeEditorSetting::Detected {
            id: editor.id.clone(),
            path: editor.path.clone(),
        };
        options.push(ExternalEditorOption {
            id: format!(
                "external_editor_{}_{}",
                sanitize_debug_id(&editor.id),
                options.len()
            ),
            label: editor.label,
            detail: Some(editor.path.display().to_string()),
            missing: false,
            kind: ExternalEditorOptionKind::Detected(setting),
        });
    }

    options.push(ExternalEditorOption {
        id: "external_editor_custom".to_string(),
        label: "Custom...".to_string(),
        detail: Some("Use a custom command or executable path".to_string()),
        missing: false,
        kind: ExternalEditorOptionKind::Custom,
    });

    options
}

pub(crate) fn configured_setting_from_session() -> Option<ExternalCodeEditorSetting> {
    session::load()
        .external_code_editor
        .filter(setting_is_configured)
}

static CONFIGURED_SETTING_OVERRIDE: OnceLock<Mutex<Option<Option<ExternalCodeEditorSetting>>>> =
    OnceLock::new();

fn configured_setting_override() -> &'static Mutex<Option<Option<ExternalCodeEditorSetting>>> {
    CONFIGURED_SETTING_OVERRIDE.get_or_init(|| Mutex::new(None))
}

static SESSION_SETTING_CACHE: OnceLock<Mutex<Option<Option<ExternalCodeEditorSetting>>>> =
    OnceLock::new();

fn session_setting_cache() -> &'static Mutex<Option<Option<ExternalCodeEditorSetting>>> {
    SESSION_SETTING_CACHE.get_or_init(|| Mutex::new(None))
}

fn invalidate_session_setting_cache() {
    *session_setting_cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = None;
}

#[cfg(test)]
thread_local! {
    static TEST_CONFIGURED_SETTING_OVERRIDE: std::cell::RefCell<Option<Option<ExternalCodeEditorSetting>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(not(test))]
pub(crate) fn set_configured_setting_override(setting: Option<ExternalCodeEditorSetting>) {
    *configured_setting_override()
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = Some(setting);
    invalidate_session_setting_cache();
}

#[cfg(test)]
pub(crate) fn set_configured_setting_override(setting: Option<ExternalCodeEditorSetting>) {
    TEST_CONFIGURED_SETTING_OVERRIDE.with(|override_setting| {
        *override_setting.borrow_mut() = Some(setting);
    });
    invalidate_session_setting_cache();
}

#[cfg(test)]
fn clear_configured_setting_override() {
    TEST_CONFIGURED_SETTING_OVERRIDE.with(|override_setting| {
        *override_setting.borrow_mut() = None;
    });
    *configured_setting_override()
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = None;
    invalidate_session_setting_cache();
}

#[cfg(test)]
static CONFIGURED_SETTING_OVERRIDE_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[cfg(test)]
pub(crate) struct ConfiguredSettingOverrideTestGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
pub(crate) fn configured_setting_override_test_guard() -> ConfiguredSettingOverrideTestGuard {
    let lock = CONFIGURED_SETTING_OVERRIDE_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    clear_configured_setting_override();
    ConfiguredSettingOverrideTestGuard { _lock: lock }
}

#[cfg(test)]
impl Drop for ConfiguredSettingOverrideTestGuard {
    fn drop(&mut self) {
        clear_configured_setting_override();
    }
}

#[cfg(test)]
fn session_setting_cache_value() -> Option<Option<ExternalCodeEditorSetting>> {
    session_setting_cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

#[cfg(not(test))]
fn configured_setting_from_override() -> Option<Option<ExternalCodeEditorSetting>> {
    configured_setting_override()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

#[cfg(test)]
fn configured_setting_from_override() -> Option<Option<ExternalCodeEditorSetting>> {
    TEST_CONFIGURED_SETTING_OVERRIDE
        .with(|override_setting| override_setting.borrow().clone())
        .or_else(|| {
            configured_setting_override()
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        })
}

pub(crate) fn configured_setting_preference_override() -> Option<Option<ExternalCodeEditorSetting>>
{
    configured_setting_from_override()
}

pub(crate) fn configured_setting() -> Option<ExternalCodeEditorSetting> {
    match configured_setting_from_override() {
        Some(setting) => setting.filter(setting_is_configured),
        None => {
            let mut cache = session_setting_cache()
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if let Some(cached) = cache.as_ref() {
                return cached.clone();
            }
            let setting = configured_setting_from_session();
            *cache = Some(setting.clone());
            setting
        }
    }
}

pub(crate) fn setting_is_configured(setting: &ExternalCodeEditorSetting) -> bool {
    match setting {
        ExternalCodeEditorSetting::Detected { id, path } => {
            !id.trim().is_empty() && !path.as_os_str().is_empty()
        }
        ExternalCodeEditorSetting::Custom { executable, .. } => !executable.as_os_str().is_empty(),
    }
}

pub(crate) fn label_for_setting(setting: Option<&ExternalCodeEditorSetting>) -> String {
    match setting {
        None => "None".to_string(),
        Some(ExternalCodeEditorSetting::Detected { id, path }) => {
            if !path.exists() {
                format!("{} (missing)", editor_label_for_id(id))
            } else {
                editor_label_for_id(id).to_string()
            }
        }
        Some(ExternalCodeEditorSetting::Custom { executable, .. }) => {
            if executable.as_os_str().is_empty() {
                "Custom".to_string()
            } else {
                format!("Custom: {}", executable.display())
            }
        }
    }
}

/// Resolve the command that opens `target` in the configured editor.
///
/// The main-thread half of launching an editor: every failure worth reporting
/// straight away — no editor configured, an empty or unparsable custom command,
/// no terminal launcher — surfaces here. Hand the result to
/// [`spawn_launch_command`] off the main thread; see
/// `crate::view::platform_open::spawn_launch` for why.
///
/// Note this half is not free: the `vim-terminal`/`neovim-terminal` settings
/// probe every PATH directory for a terminal launcher on each call. That was
/// always true, and it does not pump the Windows message queue, but a PATH
/// entry on an unreachable share can still stall the click. Caching it (as
/// [`configured_setting`] already caches the setting itself) is a follow-up.
#[cfg(test)]
pub(crate) fn launch_command_for_configured_editor(
    target: &Path,
) -> Result<ExternalEditorLaunchCommand, ExternalEditorError> {
    let setting = configured_setting().ok_or(ExternalEditorError::NotConfigured)?;
    launch_command_for_setting(&setting, target)
}

/// Spawn a command resolved by [`launch_command_for_configured_editor`].
///
/// Blocking, and on Windows it can pump the message queue — run it on a
/// background thread, never inside a GPUI event handler.
pub(crate) fn spawn_launch_command(
    command: ExternalEditorLaunchCommand,
) -> Result<(), ExternalEditorError> {
    let mut process = background_command(command.program.as_os_str());
    process.args(command.args);
    process.spawn()?;
    Ok(())
}

pub(crate) fn launch_command_for_setting(
    setting: &ExternalCodeEditorSetting,
    target: &Path,
) -> Result<ExternalEditorLaunchCommand, ExternalEditorError> {
    launch_command_for_setting_with_env(
        setting,
        target,
        &ExternalEditorDetectionEnv::from_current_process(),
    )
}

pub(crate) fn launch_command_for_setting_with_env(
    setting: &ExternalCodeEditorSetting,
    target: &Path,
    env: &ExternalEditorDetectionEnv,
) -> Result<ExternalEditorLaunchCommand, ExternalEditorError> {
    match setting {
        ExternalCodeEditorSetting::Detected { id, path } => {
            if id == "vim-terminal" || id == "neovim-terminal" {
                return terminal_editor_launch_command(path, target, env);
            }
            Ok(default_editor_launch_command(path, target))
        }
        ExternalCodeEditorSetting::Custom {
            executable,
            arguments,
        } => {
            if executable.as_os_str().is_empty() {
                return Err(ExternalEditorError::EmptyCustomExecutable);
            }
            #[cfg(target_os = "macos")]
            if is_app_bundle_path(executable) {
                return macos_app_bundle_launch_command(executable, target, arguments.as_deref());
            }

            let args = custom_launch_args(arguments.as_deref().unwrap_or(""), target)?;
            Ok(ExternalEditorLaunchCommand {
                program: executable.clone(),
                args,
            })
        }
    }
}

fn default_editor_launch_command(path: &Path, target: &Path) -> ExternalEditorLaunchCommand {
    #[cfg(target_os = "macos")]
    {
        if is_app_bundle_path(path) {
            return macos_app_bundle_launch_command(path, target, None)
                .expect("default app bundle command has no custom arguments");
        }
    }

    ExternalEditorLaunchCommand {
        program: path.to_path_buf(),
        args: vec![target.as_os_str().to_os_string()],
    }
}

fn terminal_editor_launch_command(
    editor: &Path,
    target: &Path,
    env: &ExternalEditorDetectionEnv,
) -> Result<ExternalEditorLaunchCommand, ExternalEditorError> {
    let terminal = detect_terminal_launcher(env).ok_or(ExternalEditorError::NoTerminalLauncher)?;
    let name = terminal
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut args = Vec::new();
    if name == "wt" || name == "wt.exe" {
        args.push(OsString::from("new-tab"));
    } else if name == "wezterm" {
        args.push(OsString::from("start"));
        args.push(OsString::from("--"));
    } else if name == "gnome-terminal" {
        args.push(OsString::from("--"));
    } else if name == "xfce4-terminal" {
        args.push(OsString::from("-x"));
    } else {
        args.push(OsString::from("-e"));
    }
    args.push(editor.as_os_str().to_os_string());
    args.push(target.as_os_str().to_os_string());

    Ok(ExternalEditorLaunchCommand {
        program: terminal,
        args,
    })
}

fn custom_launch_args(raw: &str, target: &Path) -> Result<Vec<OsString>, ExternalEditorError> {
    let (tokens, saw_placeholder) = parse_custom_argument_tokens(raw)?;
    let mut args = tokens
        .into_iter()
        .map(|token| custom_argument_token_to_os_string(&token, target))
        .collect::<Vec<_>>();
    if !saw_placeholder {
        args.push(target.as_os_str().to_os_string());
    }
    Ok(args)
}

fn parse_custom_argument_tokens(raw: &str) -> Result<(Vec<String>, bool), ExternalEditorError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut saw_placeholder = false;

    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && quote != Some('\'') {
            let next = chars.peek().copied();
            let after_next = chars.clone().nth(1);
            if let Some(next) = next {
                let quote_would_end_arg = quote == Some(next)
                    && after_next.is_none_or(|after_quote| after_quote.is_whitespace());
                if !quote_would_end_arg
                    && (next == '\\' || next == '\'' || next == '"' || next.is_whitespace())
                {
                    current.push(chars.next().expect("peeked argument escape"));
                    continue;
                }
            }
        }

        match quote {
            Some(q) if ch == q => {
                quote = None;
            }
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
            }
            None if ch.is_whitespace() => {
                push_custom_argument_token(&mut tokens, &mut current, &mut saw_placeholder);
            }
            None => current.push(ch),
        }
    }

    if let Some(q) = quote {
        return Err(ExternalEditorError::InvalidCustomArguments(format!(
            "unclosed {q} quote"
        )));
    }

    push_custom_argument_token(&mut tokens, &mut current, &mut saw_placeholder);
    Ok((tokens, saw_placeholder))
}

fn push_custom_argument_token(
    tokens: &mut Vec<String>,
    current: &mut String,
    saw_placeholder: &mut bool,
) {
    if current.is_empty() {
        return;
    }
    if current.contains(CUSTOM_PATH_PLACEHOLDER) {
        *saw_placeholder = true;
    }
    tokens.push(std::mem::take(current));
}

fn custom_argument_token_to_os_string(token: &str, target: &Path) -> OsString {
    if !token.contains(CUSTOM_PATH_PLACEHOLDER) {
        return OsString::from(token);
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

        let needle = CUSTOM_PATH_PLACEHOLDER.as_bytes();
        let mut remaining = token.as_bytes();
        let target_bytes = target.as_os_str().as_bytes();
        let mut output = Vec::with_capacity(token.len() + target_bytes.len());

        while let Some(index) = remaining
            .windows(needle.len())
            .position(|window| window == needle)
        {
            output.extend_from_slice(&remaining[..index]);
            output.extend_from_slice(target_bytes);
            remaining = &remaining[index + needle.len()..];
        }
        output.extend_from_slice(remaining);

        OsString::from_vec(output)
    }

    #[cfg(not(unix))]
    {
        OsString::from(token.replace(CUSTOM_PATH_PLACEHOLDER, &target.to_string_lossy()))
    }
}

fn push_detected(
    editors: &mut Vec<DetectedExternalEditor>,
    seen: &mut BTreeSet<(String, PathBuf)>,
    spec: &PathEditorSpec,
    path: PathBuf,
) {
    let key = (spec.id.to_string(), path.clone());
    if !seen.insert(key) {
        return;
    }
    editors.push(DetectedExternalEditor {
        id: spec.id.to_string(),
        label: spec.label.to_string(),
        path,
        terminal: spec.terminal,
    });
}

fn terminal_editor_shadowed_by_gui(id: &str, editors: &[DetectedExternalEditor]) -> bool {
    match id {
        "vim-terminal" => editors
            .iter()
            .any(|editor| editor.id == "vim-gui" && !editor.terminal),
        "neovim-terminal" => editors
            .iter()
            .any(|editor| editor.id == "neovim-gui" && !editor.terminal),
        _ => false,
    }
}

fn path_editor_spec_supported(spec: &PathEditorSpec) -> bool {
    spec.id != "xcode" || cfg!(target_os = "macos")
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn is_app_bundle_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext == std::ffi::OsStr::new("app"))
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_app_bundle_launch_command(
    app: &Path,
    target: &Path,
    arguments: Option<&str>,
) -> Result<ExternalEditorLaunchCommand, ExternalEditorError> {
    let mut args = vec![OsString::from("-a"), app.as_os_str().to_os_string()];
    match arguments {
        Some(arguments) if !arguments.trim().is_empty() => {
            args.push(OsString::from("--args"));
            args.extend(custom_launch_args(arguments, target)?);
        }
        _ => {
            args.push(target.as_os_str().to_os_string());
        }
    }
    Ok(ExternalEditorLaunchCommand {
        program: PathBuf::from("open"),
        args,
    })
}

fn find_programs(path_dirs: &[PathBuf], names: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in path_dirs {
        for name in names {
            for candidate in program_candidates(dir, name) {
                if is_executable_program(&candidate) && !found.iter().any(|p| p == &candidate) {
                    found.push(candidate);
                }
            }
        }
    }
    found
}

fn is_executable_program(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    }

    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn find_first_program(path_dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    find_programs(path_dirs, names).into_iter().next()
}

fn program_candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    if Path::new(name).extension().is_some() {
        return vec![dir.join(name)];
    }

    // Windows cannot execute extensionless files, so the bare launcher (e.g. a
    // unix shell script shipped alongside the real `.exe`) must be ignored,
    // otherwise the same editor is detected twice.
    #[cfg(windows)]
    {
        vec![
            dir.join(format!("{name}.exe")),
            dir.join(format!("{name}.cmd")),
            dir.join(format!("{name}.bat")),
        ]
    }

    #[cfg(not(windows))]
    {
        vec![
            dir.join(name),
            dir.join(format!("{name}.exe")),
            dir.join(format!("{name}.cmd")),
            dir.join(format!("{name}.bat")),
        ]
    }
}

fn detect_terminal_launcher(env: &ExternalEditorDetectionEnv) -> Option<PathBuf> {
    find_first_program(&env.path_dirs, TERMINAL_LAUNCHERS)
}

fn detect_macos_app_bundles(
    env: &ExternalEditorDetectionEnv,
    editors: &mut Vec<DetectedExternalEditor>,
    seen: &mut BTreeSet<(String, PathBuf)>,
) {
    for dir in &env.application_dirs {
        for spec in MAC_APP_SPECS {
            let bundle = dir.join(spec.bundle_name);
            if !bundle.exists() {
                continue;
            }
            let path = spec
                .executable_relatives
                .iter()
                .map(|relative| bundle.join(relative))
                .find(|path| path.is_file())
                .unwrap_or(bundle);
            let key = (spec.id.to_string(), path.clone());
            if !seen.insert(key) {
                continue;
            }
            editors.push(DetectedExternalEditor {
                id: spec.id.to_string(),
                label: spec.label.to_string(),
                path,
                terminal: false,
            });
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn detect_linux_desktop_editors(
    env: &ExternalEditorDetectionEnv,
    editors: &mut Vec<DetectedExternalEditor>,
    seen: &mut BTreeSet<(String, PathBuf)>,
) {
    for dir in &env.application_dirs {
        if !dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "desktop") {
                continue;
            }
            let Some(exec) = parse_desktop_file_exec(&path) else {
                continue;
            };
            let Some(exec_name) = std::path::Path::new(&exec)
                .file_name()
                .and_then(|name| name.to_str())
            else {
                continue;
            };
            let Some(spec) = PATH_EDITOR_SPECS
                .iter()
                .chain(TERMINAL_EDITOR_SPECS.iter())
                .find(|spec| spec.names.contains(&exec_name))
            else {
                continue;
            };
            let resolved = if std::path::Path::new(&exec).is_absolute() {
                let candidate = PathBuf::from(&exec);
                if is_executable_program(&candidate) {
                    candidate
                } else {
                    match find_first_program(&env.path_dirs, spec.names) {
                        Some(path) => path,
                        None => continue,
                    }
                }
            } else {
                match find_first_program(&env.path_dirs, spec.names) {
                    Some(path) => path,
                    None => continue,
                }
            };
            push_detected(editors, seen, spec, resolved);
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn detect_linux_desktop_editors(
    _env: &ExternalEditorDetectionEnv,
    _editors: &mut Vec<DetectedExternalEditor>,
    _seen: &mut BTreeSet<(String, PathBuf)>,
) {
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn parse_desktop_file_exec(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut in_desktop_entry = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_desktop_entry = trimmed == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Exec=") {
            let exec = desktop_exec_first_token(rest);
            if !exec.is_empty() {
                return Some(exec);
            }
        }
    }
    None
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn desktop_exec_first_token(raw: &str) -> String {
    let mut result = String::new();
    let mut chars = raw.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some(q) if ch == q => {
                quote = None;
            }
            Some(_) => {
                result.push(ch);
            }
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
            }
            None if ch.is_whitespace() => {
                break;
            }
            None if ch == '%' => {
                let next = chars.peek().copied();
                if next.is_some_and(|c| c.is_ascii_alphabetic()) {
                    chars.next();
                    break;
                }
                result.push(ch);
            }
            None => {
                result.push(ch);
            }
        }
    }

    result
}

fn detect_jetbrains_toolbox(
    env: &ExternalEditorDetectionEnv,
    editors: &mut Vec<DetectedExternalEditor>,
    seen: &mut BTreeSet<(String, PathBuf)>,
) {
    let specs: Vec<&PathEditorSpec> = PATH_EDITOR_SPECS
        .iter()
        .filter(|spec| spec.id.starts_with("jetbrains-") || spec.id == "android-studio")
        .collect();

    // Standalone installs use `<root>/<product>/bin/<launcher>`. Probe those
    // fixed paths instead of walking the product's bundled runtime, plugins and
    // libraries.
    for root in &env.jetbrains_install_dirs {
        for product_dir in child_directories(root) {
            probe_jetbrains_launchers(&product_dir, &specs, editors, seen);
        }
    }

    // Toolbox installs use either `<root>/<product>/<version>` or
    // `<root>/<product>/<channel>/<version>`, with the launcher in `bin`
    // (Windows/Linux) or an app bundle (macOS). Stop as soon as an install
    // directory is recognized; never descend into its runtime or plugins.
    for root in &env.jetbrains_toolbox_dirs {
        for product_dir in child_directories(root) {
            for channel_or_version_dir in child_directories(&product_dir) {
                if probe_jetbrains_toolbox_install(&channel_or_version_dir, &specs, editors, seen) {
                    continue;
                }
                for version_dir in child_directories(&channel_or_version_dir) {
                    probe_jetbrains_toolbox_install(&version_dir, &specs, editors, seen);
                }
            }
        }
    }
}

fn child_directories(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            file_type.is_dir().then(|| entry.path())
        })
        .collect()
}

fn probe_jetbrains_launchers(
    product_dir: &Path,
    specs: &[&PathEditorSpec],
    editors: &mut Vec<DetectedExternalEditor>,
    seen: &mut BTreeSet<(String, PathBuf)>,
) {
    for spec in specs {
        for name in spec.names {
            let path = product_dir.join("bin").join(name);
            if path.is_file() {
                push_detected(editors, seen, spec, path);
            }
        }
    }
}

/// Probe one Toolbox version directory. Returns whether its shape identifies it
/// as an install, allowing the caller to stop before entering large subtrees.
fn probe_jetbrains_toolbox_install(
    install_dir: &Path,
    specs: &[&PathEditorSpec],
    editors: &mut Vec<DetectedExternalEditor>,
    seen: &mut BTreeSet<(String, PathBuf)>,
) -> bool {
    let has_bin = install_dir.join("bin").is_dir();
    let app_dirs: Vec<PathBuf> = child_directories(install_dir)
        .into_iter()
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        })
        .collect();
    if !has_bin && app_dirs.is_empty() {
        return false;
    }

    probe_jetbrains_launchers(install_dir, specs, editors, seen);
    for app_dir in app_dirs {
        probe_jetbrains_macos_bundle(&app_dir, specs, editors, seen);
    }
    true
}

fn probe_jetbrains_macos_bundle(
    app_dir: &Path,
    specs: &[&PathEditorSpec],
    editors: &mut Vec<DetectedExternalEditor>,
    seen: &mut BTreeSet<(String, PathBuf)>,
) {
    for spec in specs {
        for name in spec.names {
            let path = app_dir.join("Contents/MacOS").join(name);
            if path.is_file() {
                push_detected(editors, seen, spec, path);
            }
        }
    }
}

fn detect_visual_studio(
    env: &ExternalEditorDetectionEnv,
    editors: &mut Vec<DetectedExternalEditor>,
    seen: &mut BTreeSet<(String, PathBuf)>,
) {
    const YEARS: &[&str] = &["2022", "2019", "2017"];
    const EDITIONS: &[&str] = &["Professional", "Community", "Enterprise", "BuildTools"];

    for root in &env.visual_studio_roots {
        let base_candidates = [root.join("Microsoft Visual Studio"), root.clone()];
        for base in base_candidates {
            for year in YEARS {
                for edition in EDITIONS {
                    let path = base.join(year).join(edition).join("Common7/IDE/devenv.exe");
                    if !path.is_file() {
                        continue;
                    }
                    let id = format!("visual-studio-{}", edition.to_ascii_lowercase());
                    let key = (id.clone(), path.clone());
                    if !seen.insert(key) {
                        continue;
                    }
                    editors.push(DetectedExternalEditor {
                        id,
                        label: format!("Visual Studio {edition}"),
                        path,
                        terminal: false,
                    });
                }
            }
        }
    }
}

fn editor_label_for_id(id: &str) -> &'static str {
    PATH_EDITOR_SPECS
        .iter()
        .chain(TERMINAL_EDITOR_SPECS.iter())
        .find(|spec| spec.id == id)
        .map(|spec| spec.label)
        .or_else(|| {
            MAC_APP_SPECS
                .iter()
                .find(|spec| spec.id == id)
                .map(|spec| spec.label)
        })
        .or(match id {
            "visual-studio-professional" => Some("Visual Studio Professional"),
            "visual-studio-community" => Some("Visual Studio Community"),
            "visual-studio-enterprise" => Some("Visual Studio Enterprise"),
            "visual-studio-buildtools" => Some("Visual Studio BuildTools"),
            _ => None,
        })
        .unwrap_or("External editor")
}

fn sanitize_debug_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
