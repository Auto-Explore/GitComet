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
                visual_studio_roots.push(program_files);
            }
            if let Some(program_files_x86) =
                env::var_os("ProgramFiles(x86)").filter(|v| !v.is_empty())
            {
                visual_studio_roots.push(PathBuf::from(program_files_x86));
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

pub(crate) fn external_editor_options(
    saved: Option<&ExternalCodeEditorSetting>,
) -> Vec<ExternalEditorOption> {
    let detected = detect_external_editors();
    external_editor_options_from_detected(saved, detected)
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
        std::cell::RefCell::new(None);
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

pub(crate) fn launch_configured_editor(target: &Path) -> Result<(), ExternalEditorError> {
    let setting = configured_setting().ok_or(ExternalEditorError::NotConfigured)?;
    launch_editor(&setting, target)
}

#[cfg(test)]
pub(crate) fn launch_command_for_configured_editor(
    target: &Path,
) -> Result<ExternalEditorLaunchCommand, ExternalEditorError> {
    let setting = configured_setting().ok_or(ExternalEditorError::NotConfigured)?;
    launch_command_for_setting(&setting, target)
}

pub(crate) fn launch_editor(
    setting: &ExternalCodeEditorSetting,
    target: &Path,
) -> Result<(), ExternalEditorError> {
    let command = launch_command_for_setting(setting, target)?;
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
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn find_first_program(path_dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    find_programs(path_dirs, names).into_iter().next()
}

fn program_candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut candidates = vec![dir.join(name)];
    if Path::new(name).extension().is_none() {
        candidates.push(dir.join(format!("{name}.exe")));
        candidates.push(dir.join(format!("{name}.cmd")));
        candidates.push(dir.join(format!("{name}.bat")));
    }
    candidates
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
    for root in &env.jetbrains_toolbox_dirs {
        if !root.is_dir() {
            continue;
        }
        for spec in PATH_EDITOR_SPECS
            .iter()
            .filter(|spec| spec.id.starts_with("jetbrains-") || spec.id == "android-studio")
        {
            for path in find_named_files_limited(root, spec.names, 8) {
                push_detected(editors, seen, spec, path);
            }
        }
    }
}

fn find_named_files_limited(root: &Path, names: &[&str], max_depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    find_named_files_limited_inner(root, names, max_depth, &mut found);
    found
}

fn find_named_files_limited_inner(
    dir: &Path,
    names: &[&str],
    remaining_depth: usize,
    found: &mut Vec<PathBuf>,
) {
    if remaining_depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_named_files_limited_inner(&path, names, remaining_depth - 1, found);
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if names.contains(&file_name) && !found.iter().any(|p| p == &path) {
            found.push(path);
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
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "gitcomet-external-editor-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, b"").expect("create file");
    }

    fn touch_executable(path: &Path) {
        touch(path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(path)
                .expect("read file metadata")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).expect("mark file executable");
        }
    }

    fn ids(editors: &[DetectedExternalEditor]) -> Vec<String> {
        editors.iter().map(|editor| editor.id.clone()).collect()
    }

    #[test]
    fn detects_editors_from_path_and_dedupes_by_id_and_path() {
        let dir = temp_dir("path");
        let code = dir.join("code");
        touch_executable(&code);
        let env = ExternalEditorDetectionEnv {
            path_dirs: vec![dir.clone(), dir],
            ..ExternalEditorDetectionEnv::default()
        };

        let editors = detect_external_editors_with_env(&env);

        assert_eq!(ids(&editors), vec!["vscode"]);
        assert_eq!(editors[0].path, code);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn does_not_detect_xed_as_xcode_outside_macos() {
        let dir = temp_dir("xed-non-macos");
        touch_executable(&dir.join("xed"));
        let env = ExternalEditorDetectionEnv {
            path_dirs: vec![dir],
            ..ExternalEditorDetectionEnv::default()
        };

        let editors = detect_external_editors_with_env(&env);

        assert!(
            editors.iter().all(|editor| editor.id != "xcode"),
            "expected Linux xed to avoid Xcode detection"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ignores_non_executable_path_program_candidates_on_unix() {
        let dir = temp_dir("path-non-executable");
        touch(&dir.join("code"));
        touch_executable(&dir.join("vim"));
        touch(&dir.join("xterm"));
        let env = ExternalEditorDetectionEnv {
            path_dirs: vec![dir],
            ..ExternalEditorDetectionEnv::default()
        };

        let editors = detect_external_editors_with_env(&env);

        assert!(
            editors.iter().all(|editor| editor.id != "vscode"),
            "expected non-executable `code` PATH file to be ignored"
        );
        assert!(
            editors.iter().all(|editor| editor.id != "vim-terminal"),
            "expected non-executable terminal launcher to be ignored"
        );
    }

    #[test]
    fn detects_jetbrains_toolbox_paths() {
        let root = temp_dir("toolbox");
        let idea = root.join("IDEA-U/ch-0/251.1/bin/idea.sh");
        touch(&idea);
        let env = ExternalEditorDetectionEnv {
            jetbrains_toolbox_dirs: vec![root],
            ..ExternalEditorDetectionEnv::default()
        };

        let editors = detect_external_editors_with_env(&env);

        assert!(editors.iter().any(|editor| {
            editor.id == "jetbrains-idea" && editor.label == "IntelliJ IDEA" && editor.path == idea
        }));
    }

    #[test]
    fn detects_macos_app_bundle_fallbacks_from_configured_roots() {
        let apps = temp_dir("apps");
        let xcode = apps.join("Xcode.app");
        fs::create_dir_all(&xcode).expect("create xcode bundle");
        let env = ExternalEditorDetectionEnv {
            application_dirs: vec![apps],
            ..ExternalEditorDetectionEnv::default()
        };

        let editors = detect_external_editors_with_env(&env);

        assert!(editors.iter().any(|editor| {
            editor.id == "xcode" && editor.label == "Xcode" && editor.path == xcode
        }));
    }

    #[test]
    fn detects_visual_studio_editions() {
        let root = temp_dir("visual-studio");
        let devenv = root.join("Microsoft Visual Studio/2022/Professional/Common7/IDE/devenv.exe");
        touch(&devenv);
        let env = ExternalEditorDetectionEnv {
            visual_studio_roots: vec![root],
            ..ExternalEditorDetectionEnv::default()
        };

        let editors = detect_external_editors_with_env(&env);

        assert!(editors.iter().any(|editor| {
            editor.id == "visual-studio-professional"
                && editor.label == "Visual Studio Professional"
                && editor.path == devenv
        }));
    }

    #[test]
    fn terminal_vim_is_gated_by_terminal_launcher_and_shadowed_by_gui() {
        let dir = temp_dir("terminal");
        touch_executable(&dir.join("vim"));
        let no_terminal = ExternalEditorDetectionEnv {
            path_dirs: vec![dir.clone()],
            ..ExternalEditorDetectionEnv::default()
        };
        assert!(
            detect_external_editors_with_env(&no_terminal)
                .iter()
                .all(|editor| editor.id != "vim-terminal")
        );

        touch_executable(&dir.join("xterm"));
        let with_terminal = ExternalEditorDetectionEnv {
            path_dirs: vec![dir.clone()],
            ..ExternalEditorDetectionEnv::default()
        };
        assert!(
            detect_external_editors_with_env(&with_terminal)
                .iter()
                .any(|editor| editor.id == "vim-terminal")
        );

        touch_executable(&dir.join("gvim"));
        let with_gui = ExternalEditorDetectionEnv {
            path_dirs: vec![dir],
            ..ExternalEditorDetectionEnv::default()
        };
        let editors = detect_external_editors_with_env(&with_gui);
        assert!(editors.iter().any(|editor| editor.id == "vim-gui"));
        assert!(editors.iter().all(|editor| editor.id != "vim-terminal"));
    }

    #[test]
    fn terminal_vim_launch_command_uses_terminal_specific_argv() {
        let target = Path::new("/tmp/repo");
        let editor = PathBuf::from("/usr/bin/vim");
        let setting = ExternalCodeEditorSetting::Detected {
            id: "vim-terminal".to_string(),
            path: editor.clone(),
        };

        for (terminal_name, expected_prefix) in [
            ("gnome-terminal", vec![OsString::from("--")]),
            ("xfce4-terminal", vec![OsString::from("-x")]),
            ("xterm", vec![OsString::from("-e")]),
        ] {
            let dir = temp_dir(terminal_name);
            let terminal = dir.join(terminal_name);
            touch_executable(&terminal);
            let env = ExternalEditorDetectionEnv {
                path_dirs: vec![dir],
                ..ExternalEditorDetectionEnv::default()
            };

            let command = launch_command_for_setting_with_env(&setting, target, &env)
                .expect("build terminal Vim command");

            let mut expected_args = expected_prefix;
            expected_args.push(editor.as_os_str().to_os_string());
            expected_args.push(target.as_os_str().to_os_string());
            assert_eq!(command.program, terminal);
            assert_eq!(
                command.args, expected_args,
                "expected {terminal_name} argv to pass the editor target correctly"
            );
        }
    }

    #[test]
    fn saved_missing_selection_is_included_in_options() {
        let setting = ExternalCodeEditorSetting::Detected {
            id: "vscode".to_string(),
            path: PathBuf::from("/definitely/missing/code"),
        };

        let options = external_editor_options_from_detected(Some(&setting), Vec::new());

        assert!(options.iter().any(|option| {
            option.missing
                && option.label == "Visual Studio Code (missing)"
                && matches!(&option.kind, ExternalEditorOptionKind::Detected(saved) if saved == &setting)
        }));
    }

    #[test]
    fn launch_command_for_detected_editor_appends_target() {
        let setting = ExternalCodeEditorSetting::Detected {
            id: "vscode".to_string(),
            path: PathBuf::from("/usr/bin/code"),
        };
        let target = Path::new("/tmp/repo");

        let command = launch_command_for_setting_with_env(
            &setting,
            target,
            &ExternalEditorDetectionEnv::default(),
        )
        .expect("build launch command");

        assert_eq!(command.program, PathBuf::from("/usr/bin/code"));
        assert_eq!(command.args, vec![OsString::from("/tmp/repo")]);
    }

    #[test]
    fn configured_launch_command_uses_runtime_override_before_session_persist_finishes() {
        let _guard = configured_setting_override_test_guard();
        set_configured_setting_override(Some(ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("/usr/bin/editor"),
            arguments: Some("--reuse-window".to_string()),
        }));

        let command = launch_command_for_configured_editor(Path::new("/tmp/repo"))
            .expect("runtime override should be launchable without reading session state");

        assert_eq!(command.program, PathBuf::from("/usr/bin/editor"));
        assert_eq!(
            command.args,
            vec![
                OsString::from("--reuse-window"),
                OsString::from("/tmp/repo")
            ]
        );
    }

    #[test]
    fn configured_launch_command_respects_runtime_clear_before_session_persist_finishes() {
        let _guard = configured_setting_override_test_guard();
        set_configured_setting_override(None);

        let err = launch_command_for_configured_editor(Path::new("/tmp/repo")).unwrap_err();

        assert_eq!(err, ExternalEditorError::NotConfigured);
    }

    #[test]
    fn configured_setting_override_is_thread_local_in_tests() {
        let _guard = configured_setting_override_test_guard();
        let setting = ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("/usr/bin/editor"),
            arguments: None,
        };
        set_configured_setting_override(Some(setting.clone()));

        assert_eq!(
            configured_setting_preference_override(),
            Some(Some(setting))
        );

        let other_thread_override = std::thread::spawn(configured_setting_preference_override)
            .join()
            .expect("read override from another test thread");

        assert_eq!(other_thread_override, None);
    }

    #[test]
    fn app_bundle_launch_command_routes_through_macos_open() {
        let command = macos_app_bundle_launch_command(
            Path::new("/Applications/Example.app"),
            Path::new("/tmp/repo"),
            None,
        )
        .expect("build app bundle command");

        assert_eq!(command.program, PathBuf::from("open"));
        assert_eq!(
            command.args,
            vec![
                OsString::from("-a"),
                OsString::from("/Applications/Example.app"),
                OsString::from("/tmp/repo"),
            ]
        );
    }

    #[test]
    fn app_bundle_launch_command_routes_custom_args_through_macos_open_args() {
        let command = macos_app_bundle_launch_command(
            Path::new("/Applications/Example.app"),
            Path::new("/tmp/repo"),
            Some("--reuse-window {path}"),
        )
        .expect("build app bundle command with args");

        assert_eq!(command.program, PathBuf::from("open"));
        assert_eq!(
            command.args,
            vec![
                OsString::from("-a"),
                OsString::from("/Applications/Example.app"),
                OsString::from("--args"),
                OsString::from("--reuse-window"),
                OsString::from("/tmp/repo"),
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn custom_app_bundle_setting_routes_through_macos_open() {
        let setting = ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("/Applications/Example.app"),
            arguments: None,
        };

        let command = launch_command_for_setting_with_env(
            &setting,
            Path::new("/tmp/repo"),
            &ExternalEditorDetectionEnv::default(),
        )
        .expect("build custom app bundle command");

        assert_eq!(command.program, PathBuf::from("open"));
        assert_eq!(
            command.args,
            vec![
                OsString::from("-a"),
                OsString::from("/Applications/Example.app"),
                OsString::from("/tmp/repo"),
            ]
        );
    }

    #[test]
    fn custom_launch_args_substitute_path_placeholder() {
        let setting = ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("/usr/bin/editor"),
            arguments: Some("--reuse-window \"{path}\" --line 5".to_string()),
        };

        let command = launch_command_for_setting_with_env(
            &setting,
            Path::new("/tmp/project/src main.rs"),
            &ExternalEditorDetectionEnv::default(),
        )
        .expect("build custom command");

        assert_eq!(
            command.args,
            vec![
                OsString::from("--reuse-window"),
                OsString::from("/tmp/project/src main.rs"),
                OsString::from("--line"),
                OsString::from("5"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn custom_launch_args_preserve_non_utf8_path_bytes_in_placeholder() {
        use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

        let target = PathBuf::from(OsString::from_vec(b"/tmp/gitcomet-\xff/repo".to_vec()));
        let setting = ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("/usr/bin/editor"),
            arguments: Some("--open {path} --marker=before-{path}-after".to_string()),
        };

        let command = launch_command_for_setting_with_env(
            &setting,
            &target,
            &ExternalEditorDetectionEnv::default(),
        )
        .expect("build custom command");

        assert_eq!(command.args[0], OsString::from("--open"));
        assert_eq!(
            command.args[1].as_os_str().as_bytes(),
            target.as_os_str().as_bytes()
        );
        assert_eq!(
            command.args[2].as_os_str().as_bytes(),
            b"--marker=before-/tmp/gitcomet-\xff/repo-after"
        );
    }

    #[test]
    fn custom_launch_args_append_target_when_placeholder_is_omitted() {
        let setting = ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("/usr/bin/editor"),
            arguments: Some("--reuse-window".to_string()),
        };

        let command = launch_command_for_setting_with_env(
            &setting,
            Path::new("/tmp/repo"),
            &ExternalEditorDetectionEnv::default(),
        )
        .expect("build custom command");

        assert_eq!(
            command.args,
            vec![
                OsString::from("--reuse-window"),
                OsString::from("/tmp/repo")
            ]
        );
    }

    #[test]
    fn custom_launch_args_preserve_windows_style_backslashes() {
        let setting = ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("editor.exe"),
            arguments: Some("--profile \"C:\\Users\\Sampo Main\\Editor\"".to_string()),
        };

        let command = launch_command_for_setting_with_env(
            &setting,
            Path::new("C:\\repo"),
            &ExternalEditorDetectionEnv::default(),
        )
        .expect("build custom command");

        assert_eq!(
            command.args,
            vec![
                OsString::from("--profile"),
                OsString::from("C:\\Users\\Sampo Main\\Editor"),
                OsString::from("C:\\repo"),
            ]
        );
    }

    #[test]
    fn custom_launch_args_preserve_trailing_windows_backslash_before_closing_quote() {
        let setting = ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("editor.exe"),
            arguments: Some("--profile \"C:\\Tools\\Editor\\\"".to_string()),
        };

        let command = launch_command_for_setting_with_env(
            &setting,
            Path::new("C:\\repo"),
            &ExternalEditorDetectionEnv::default(),
        )
        .expect("build custom command");

        assert_eq!(
            command.args,
            vec![
                OsString::from("--profile"),
                OsString::from("C:\\Tools\\Editor\\"),
                OsString::from("C:\\repo"),
            ]
        );
    }

    #[test]
    fn custom_launch_args_reject_unclosed_quotes() {
        let setting = ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("/usr/bin/editor"),
            arguments: Some("\"--bad".to_string()),
        };

        let err = launch_command_for_setting_with_env(
            &setting,
            Path::new("/tmp/repo"),
            &ExternalEditorDetectionEnv::default(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ExternalEditorError::InvalidCustomArguments(_)
        ));
    }

    #[test]
    fn configured_setting_caches_session_value_on_repeated_calls() {
        let _guard = configured_setting_override_test_guard();

        assert!(session_setting_cache_value().is_none());

        let first = configured_setting();
        let cached = session_setting_cache_value();

        assert_eq!(
            first,
            cached.and_then(|c| c),
            "configured_setting() should populate the cache with the session-derived value"
        );

        let second = configured_setting();
        assert_eq!(
            first, second,
            "repeated calls should return the same cached value"
        );
    }

    #[test]
    fn configured_setting_override_invalidates_session_cache() {
        let _guard = configured_setting_override_test_guard();

        configured_setting();
        assert!(
            session_setting_cache_value().is_some(),
            "cache should be populated after first configured_setting() call"
        );

        set_configured_setting_override(Some(ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("/usr/bin/editor"),
            arguments: None,
        }));

        assert!(
            session_setting_cache_value().is_none(),
            "cache should be invalidated after set_configured_setting_override is called"
        );

        let result = configured_setting();
        assert!(
            result.is_some(),
            "should return override value without touching cache"
        );
    }

    #[test]
    fn configured_setting_override_clear_invalidates_session_cache() {
        let _guard = configured_setting_override_test_guard();

        set_configured_setting_override(Some(ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("/usr/bin/editor"),
            arguments: None,
        }));
        assert!(configured_setting().is_some(), "override should be active");

        clear_configured_setting_override();

        assert!(
            session_setting_cache_value().is_none(),
            "cache should be invalidated when override is cleared"
        );

        assert_eq!(
            configured_setting_preference_override(),
            None,
            "override should be cleared"
        );
    }
}
