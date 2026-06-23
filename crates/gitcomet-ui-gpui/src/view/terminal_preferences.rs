use gitcomet_state::session;
use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

const LINUX_AUTOMATIC_TERMINALS: &[LinuxAutomaticTerminal] = &[
    LinuxAutomaticTerminal::new("kgx", &["--working-directory"]),
    LinuxAutomaticTerminal::new("ptyxis", &["--working-directory"]),
    LinuxAutomaticTerminal::new("gnome-terminal", &["--working-directory"]),
    LinuxAutomaticTerminal::new("konsole", &["--workdir"]),
    LinuxAutomaticTerminal::new("xfce4-terminal", &["--working-directory"]),
    LinuxAutomaticTerminal::new("mate-terminal", &["--working-directory"]),
    LinuxAutomaticTerminal::new("tilix", &["--working-directory"]),
    LinuxAutomaticTerminal::new("kitty", &["--directory"]),
    LinuxAutomaticTerminal::new("wezterm", &["start", "--cwd"]),
    LinuxAutomaticTerminal::new("alacritty", &["--working-directory"]),
    LinuxAutomaticTerminal::new("footclient", &["--working-directory"]),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub(in crate::view) enum ActionBarTerminalTarget {
    #[default]
    Embedded,
    External,
}

impl ActionBarTerminalTarget {
    pub(in crate::view) fn from_key(raw: &str) -> Option<Self> {
        match raw.trim() {
            "embedded" => Some(Self::Embedded),
            "external" => Some(Self::External),
            _ => None,
        }
    }

    pub(in crate::view) fn key(self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::External => "external",
        }
    }

    pub(in crate::view) fn label(self) -> &'static str {
        match self {
            Self::Embedded => "Embedded terminal",
            Self::External => "External terminal",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub(in crate::view) enum ExternalTerminalMode {
    #[default]
    SystemDefault,
    CustomProgram,
}

impl ExternalTerminalMode {
    pub(in crate::view) fn from_key(raw: &str) -> Option<Self> {
        match raw.trim() {
            "system_default" => Some(Self::SystemDefault),
            "custom_program" => Some(Self::CustomProgram),
            _ => None,
        }
    }

    pub(in crate::view) fn key(self) -> &'static str {
        match self {
            Self::SystemDefault => "system_default",
            Self::CustomProgram => "custom_program",
        }
    }

    pub(in crate::view) fn label(self) -> &'static str {
        match self {
            Self::SystemDefault => "System default",
            Self::CustomProgram => "Custom launcher",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct TerminalPreferences {
    pub(in crate::view) external_terminal_mode: ExternalTerminalMode,
    pub(in crate::view) external_terminal_program: String,
    pub(in crate::view) external_terminal_args: Vec<String>,
    pub(in crate::view) action_bar_terminal_target: ActionBarTerminalTarget,
}

impl Default for TerminalPreferences {
    fn default() -> Self {
        Self {
            external_terminal_mode: ExternalTerminalMode::SystemDefault,
            external_terminal_program: String::new(),
            external_terminal_args: Vec::new(),
            action_bar_terminal_target: ActionBarTerminalTarget::Embedded,
        }
    }
}

impl TerminalPreferences {
    pub(in crate::view) fn from_ui_session(ui_session: &session::UiSession) -> Self {
        let mut preferences = Self::default();
        if let Some(mode) = ui_session
            .terminal_external_mode
            .as_deref()
            .and_then(ExternalTerminalMode::from_key)
        {
            preferences.external_terminal_mode = mode;
        }
        if let Some(program) = ui_session.terminal_external_program.as_ref() {
            preferences.external_terminal_program = program.clone();
        }
        if let Some(args) = ui_session.terminal_external_args.as_ref() {
            preferences.external_terminal_args = args
                .iter()
                .map(|arg| arg.trim().to_string())
                .filter(|arg| !arg.is_empty())
                .collect();
        }
        if let Some(target) = ui_session
            .terminal_action_bar_target
            .as_deref()
            .and_then(ActionBarTerminalTarget::from_key)
        {
            preferences.action_bar_terminal_target = target;
        }
        preferences
    }

    pub(in crate::view) fn apply_to_ui_settings(&self, settings: &mut session::UiSettings) {
        settings.terminal_external_mode = Some(self.external_terminal_mode.key().to_string());
        settings.terminal_external_program = Some(self.external_terminal_program.clone());
        settings.terminal_external_args = Some(self.external_terminal_args.clone());
        settings.terminal_action_bar_target =
            Some(self.action_bar_terminal_target.key().to_string());
    }

    pub(in crate::view) fn external_summary(&self) -> String {
        match self.external_terminal_mode {
            ExternalTerminalMode::SystemDefault => "System default (best effort)".to_string(),
            ExternalTerminalMode::CustomProgram => {
                let program = self.external_terminal_program.trim();
                if program.is_empty() {
                    "Custom launcher (not set)".to_string()
                } else {
                    format!("Custom: {program}")
                }
            }
        }
    }

    pub(in crate::view) fn external_args_multiline(&self) -> String {
        format_terminal_args_multiline(&self.external_terminal_args)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct ExternalTerminalLaunchContext {
    pub(in crate::view) cwd: PathBuf,
    pub(in crate::view) repo_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct ExternalTerminalLaunchSpec {
    pub(in crate::view) program: OsString,
    pub(in crate::view) args: Vec<OsString>,
    pub(in crate::view) current_dir: Option<PathBuf>,
}

impl ExternalTerminalLaunchSpec {
    pub(in crate::view) fn launch(&self) -> io::Result<()> {
        let mut command = std::process::Command::new(&self.program);
        command.args(&self.args);
        if let Some(current_dir) = self.current_dir.as_ref() {
            command.current_dir(current_dir);
        }
        let _ = command.spawn()?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LinuxAutomaticTerminal {
    program: &'static str,
    args_prefix: &'static [&'static str],
}

impl LinuxAutomaticTerminal {
    const fn new(program: &'static str, args_prefix: &'static [&'static str]) -> Self {
        Self {
            program,
            args_prefix,
        }
    }
}

pub(in crate::view) fn format_terminal_args_multiline(args: &[String]) -> String {
    args.join("\n")
}

pub(in crate::view) fn parse_terminal_args_multiline(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(in crate::view) fn resolve_embedded_shell_program() -> Result<PathBuf, String> {
    resolve_automatic_embedded_shell_program()
        .ok_or_else(|| "No shell program was found for the embedded terminal.".to_string())
}

pub(in crate::view) fn launch_external_terminal_from_preferences(
    preferences: &TerminalPreferences,
    context: &ExternalTerminalLaunchContext,
) -> Result<(), String> {
    let spec = resolve_external_terminal_launch_spec(preferences, context)?;
    spec.launch().map_err(|err| err.to_string())
}

pub(in crate::view) fn resolve_external_terminal_launch_spec(
    preferences: &TerminalPreferences,
    context: &ExternalTerminalLaunchContext,
) -> Result<ExternalTerminalLaunchSpec, String> {
    match preferences.external_terminal_mode {
        ExternalTerminalMode::SystemDefault => {
            resolve_system_default_external_terminal_launch_spec(context)
        }
        ExternalTerminalMode::CustomProgram => {
            resolve_custom_external_terminal_launch_spec(preferences, context)
        }
    }
}

fn resolve_system_default_external_terminal_launch_spec(
    context: &ExternalTerminalLaunchContext,
) -> Result<ExternalTerminalLaunchSpec, String> {
    resolve_automatic_external_terminal_launch_spec(context)
}

fn resolve_automatic_external_terminal_launch_spec(
    context: &ExternalTerminalLaunchContext,
) -> Result<ExternalTerminalLaunchSpec, String> {
    resolve_automatic_external_terminal_launch_spec_with_lookup(context, find_executable_in_path)
}

fn resolve_automatic_external_terminal_launch_spec_with_lookup<F>(
    context: &ExternalTerminalLaunchContext,
    mut find_executable: F,
) -> Result<ExternalTerminalLaunchSpec, String>
where
    F: FnMut(&str) -> Option<PathBuf>,
{
    #[cfg(target_os = "macos")]
    {
        let _ = &mut find_executable;
        return Ok(ExternalTerminalLaunchSpec {
            program: OsString::from("open"),
            args: vec![
                OsString::from("-a"),
                OsString::from("Terminal"),
                context.cwd.as_os_str().to_os_string(),
            ],
            current_dir: None,
        });
    }

    #[cfg(target_os = "windows")]
    {
        if find_executable("wt.exe").is_some() {
            return Ok(ExternalTerminalLaunchSpec {
                program: OsString::from("wt.exe"),
                args: vec![
                    OsString::from("new-tab"),
                    OsString::from("--startingDirectory"),
                    context.cwd.as_os_str().to_os_string(),
                ],
                current_dir: Some(context.cwd.clone()),
            });
        }

        return Ok(ExternalTerminalLaunchSpec {
            program: OsString::from("cmd.exe"),
            args: vec![
                OsString::from("/K"),
                OsString::from(format!("cd /d {}", windows_cmd_quote_path(&context.cwd))),
            ],
            current_dir: Some(context.cwd.clone()),
        });
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        for candidate in LINUX_AUTOMATIC_TERMINALS {
            if find_executable(candidate.program).is_some() {
                let mut args = candidate
                    .args_prefix
                    .iter()
                    .copied()
                    .map(OsString::from)
                    .collect::<Vec<_>>();
                args.push(context.cwd.as_os_str().to_os_string());
                return Ok(ExternalTerminalLaunchSpec {
                    program: OsString::from(candidate.program),
                    args,
                    current_dir: Some(context.cwd.clone()),
                });
            }
        }

        if find_executable("xterm").is_some() {
            return Ok(ExternalTerminalLaunchSpec {
                program: OsString::from("xterm"),
                args: vec![
                    OsString::from("-e"),
                    OsString::from("sh"),
                    OsString::from("-lc"),
                    OsString::from(format!(
                        "cd {} && exec \"${{SHELL:-/bin/sh}}\"",
                        shell_single_quote(&context.cwd.to_string_lossy())
                    )),
                ],
                current_dir: Some(context.cwd.clone()),
            });
        }

        Err(
            "No supported terminal launcher was found on PATH. Configure a custom launcher in Settings > Terminal."
                .to_string(),
        )
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd"
    )))]
    {
        let _ = &mut find_executable;
        let _ = context;
        Err("Terminal launching is not supported on this platform.".to_string())
    }
}

fn resolve_custom_external_terminal_launch_spec(
    preferences: &TerminalPreferences,
    context: &ExternalTerminalLaunchContext,
) -> Result<ExternalTerminalLaunchSpec, String> {
    let program = preferences.external_terminal_program.trim();
    if program.is_empty() {
        return Err("Set a custom terminal launcher program in Settings > Terminal.".to_string());
    }

    let substituted_args = preferences
        .external_terminal_args
        .iter()
        .map(|arg| OsString::from(substitute_launch_placeholders(arg, context)))
        .collect::<Vec<_>>();

    #[cfg(target_os = "macos")]
    {
        if looks_like_macos_app_bundle(program) {
            let mut args = vec![OsString::from("-a"), OsString::from(program)];
            if substituted_args.is_empty() {
                args.push(context.cwd.as_os_str().to_os_string());
            } else {
                args.push(OsString::from("--args"));
                args.extend(substituted_args);
            }
            return Ok(ExternalTerminalLaunchSpec {
                program: OsString::from("open"),
                args,
                current_dir: None,
            });
        }
    }

    Ok(ExternalTerminalLaunchSpec {
        program: OsString::from(program),
        args: substituted_args,
        current_dir: Some(context.cwd.clone()),
    })
}

fn resolve_automatic_embedded_shell_program() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        resolve_automatic_embedded_shell_program_with(
            env::var_os("COMSPEC"),
            find_executable_in_path,
        )
    }

    #[cfg(not(target_os = "windows"))]
    {
        resolve_automatic_embedded_shell_program_from_shell(env::var_os("SHELL"))
    }
}

#[cfg(target_os = "windows")]
fn resolve_automatic_embedded_shell_program_with<F>(
    comspec: Option<OsString>,
    mut find_executable: F,
) -> Option<PathBuf>
where
    F: FnMut(&str) -> Option<PathBuf>,
{
    comspec
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| find_executable("pwsh.exe"))
        .or_else(|| find_executable("powershell.exe"))
        .or_else(|| find_executable("cmd.exe"))
        .or_else(|| Some(PathBuf::from("cmd.exe")))
}

#[cfg(not(target_os = "windows"))]
fn resolve_automatic_embedded_shell_program_from_shell(shell: Option<OsString>) -> Option<PathBuf> {
    shell
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from("/bin/sh")))
}

fn substitute_launch_placeholders(raw: &str, context: &ExternalTerminalLaunchContext) -> String {
    raw.replace("{cwd}", &context.cwd.to_string_lossy())
        .replace("{repo_name}", context.repo_name.as_deref().unwrap_or(""))
}

fn find_executable_in_path(name: &str) -> Option<PathBuf> {
    find_executable_in_path_with_env(name, env::var_os("PATH"), env::var_os("PATHEXT"))
}

fn find_executable_in_path_with_env(
    name: &str,
    path: Option<OsString>,
    pathext: Option<OsString>,
) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.exists().then(|| candidate.to_path_buf());
    }

    let path = path?;
    #[cfg(target_os = "windows")]
    let pathext = pathext
        .and_then(|value| value.into_string().ok())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_ascii_lowercase())
        .collect::<Vec<_>>();
    #[cfg(not(target_os = "windows"))]
    let _ = pathext;

    for directory in env::split_paths(&path) {
        let path = directory.join(name);
        if path.is_file() {
            return Some(path);
        }

        #[cfg(target_os = "windows")]
        if candidate.extension().is_none() {
            for extension in &pathext {
                let candidate = directory.join(format!("{name}{extension}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

fn shell_single_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "windows")]
fn windows_cmd_quote_path(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

#[cfg(target_os = "macos")]
fn looks_like_macos_app_bundle(program: &str) -> bool {
    program.ends_with(".app")
        || Path::new(program)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gitcomet_terminal_preferences_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&dir).expect("test directory should be created");
        dir
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn write_executable(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join(name);
        fs::write(&path, b"#!/bin/sh\n").expect("launcher stub should be written");
        let mut permissions = fs::metadata(&path)
            .expect("launcher stub metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("launcher stub should be executable");
        path
    }

    #[test]
    fn parse_terminal_args_multiline_ignores_blank_lines() {
        assert_eq!(
            parse_terminal_args_multiline(" --foo \n\n{cwd}\n  "),
            vec!["--foo".to_string(), "{cwd}".to_string()]
        );
        assert_eq!(
            format_terminal_args_multiline(&["--foo".to_string(), "{cwd}".to_string()]),
            "--foo\n{cwd}"
        );
    }

    #[test]
    fn terminal_preferences_from_ui_session_ignores_invalid_modes_and_trims_args() {
        let ui_session = session::UiSession {
            terminal_external_mode: Some(" nope ".to_string()),
            terminal_external_program: Some(" wezterm ".to_string()),
            terminal_external_args: Some(vec![
                " start ".to_string(),
                "".to_string(),
                " {cwd} ".to_string(),
            ]),
            terminal_action_bar_target: Some(" external ".to_string()),
            ..session::UiSession::default()
        };

        let preferences = TerminalPreferences::from_ui_session(&ui_session);
        assert_eq!(
            preferences.external_terminal_mode,
            ExternalTerminalMode::SystemDefault
        );
        assert_eq!(preferences.external_terminal_program, " wezterm ");
        assert_eq!(
            preferences.external_terminal_args,
            vec!["start".to_string(), "{cwd}".to_string()]
        );
        assert_eq!(
            preferences.action_bar_terminal_target,
            ActionBarTerminalTarget::External
        );
    }

    #[test]
    fn terminal_preference_summaries_report_custom_missing_values() {
        let preferences = TerminalPreferences {
            external_terminal_mode: ExternalTerminalMode::CustomProgram,
            ..TerminalPreferences::default()
        };

        assert_eq!(preferences.external_summary(), "Custom launcher (not set)");
    }

    #[test]
    fn terminal_preferences_round_trip_via_ui_settings() {
        let preferences = TerminalPreferences {
            external_terminal_mode: ExternalTerminalMode::CustomProgram,
            external_terminal_program: "wezterm".to_string(),
            external_terminal_args: vec![
                "start".to_string(),
                "--cwd".to_string(),
                "{cwd}".to_string(),
            ],
            action_bar_terminal_target: ActionBarTerminalTarget::External,
        };

        let mut settings = session::UiSettings::default();
        preferences.apply_to_ui_settings(&mut settings);
        assert_eq!(
            settings.terminal_external_mode.as_deref(),
            Some("custom_program")
        );
        assert_eq!(
            settings.terminal_external_args,
            Some(vec![
                "start".to_string(),
                "--cwd".to_string(),
                "{cwd}".to_string()
            ])
        );
        assert_eq!(
            settings.terminal_action_bar_target.as_deref(),
            Some("external")
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn resolve_embedded_shell_program_automatic_prefers_shell_env_and_falls_back_to_bin_sh() {
        let automatic = resolve_automatic_embedded_shell_program_from_shell(Some(OsString::from(
            "/tmp/gitcomet-shell",
        )))
        .expect("automatic shell should resolve");
        assert_eq!(automatic, PathBuf::from("/tmp/gitcomet-shell"));

        let fallback = resolve_automatic_embedded_shell_program_from_shell(None)
            .expect("automatic shell should fall back");
        assert_eq!(fallback, PathBuf::from("/bin/sh"));
    }

    #[test]
    fn custom_launch_spec_substitutes_placeholders() {
        let preferences = TerminalPreferences {
            external_terminal_mode: ExternalTerminalMode::CustomProgram,
            external_terminal_program: "wezterm".to_string(),
            external_terminal_args: vec![
                "start".to_string(),
                "--cwd".to_string(),
                "{cwd}".to_string(),
                "--class".to_string(),
                "{repo_name}".to_string(),
            ],
            ..TerminalPreferences::default()
        };
        let context = ExternalTerminalLaunchContext {
            cwd: PathBuf::from("/tmp/gitcomet"),
            repo_name: Some("gitcomet".to_string()),
        };

        let spec = resolve_custom_external_terminal_launch_spec(&preferences, &context)
            .expect("custom terminal spec");
        assert_eq!(spec.program, OsString::from("wezterm"));
        assert_eq!(
            spec.args,
            vec![
                OsString::from("start"),
                OsString::from("--cwd"),
                OsString::from("/tmp/gitcomet"),
                OsString::from("--class"),
                OsString::from("gitcomet"),
            ]
        );
    }

    #[test]
    fn custom_launch_spec_substitutes_missing_repo_name_with_empty_string() {
        let preferences = TerminalPreferences {
            external_terminal_mode: ExternalTerminalMode::CustomProgram,
            external_terminal_program: "launcher".to_string(),
            external_terminal_args: vec!["--title".to_string(), "{repo_name}".to_string()],
            ..TerminalPreferences::default()
        };
        let context = ExternalTerminalLaunchContext {
            cwd: PathBuf::from("/tmp/gitcomet"),
            repo_name: None,
        };

        let spec = resolve_custom_external_terminal_launch_spec(&preferences, &context)
            .expect("custom terminal spec");
        assert_eq!(
            spec.args,
            vec![OsString::from("--title"), OsString::from("")]
        );
    }

    #[test]
    fn resolve_custom_external_terminal_launch_spec_requires_program() {
        let preferences = TerminalPreferences {
            external_terminal_mode: ExternalTerminalMode::CustomProgram,
            external_terminal_program: "   ".to_string(),
            ..TerminalPreferences::default()
        };
        let context = ExternalTerminalLaunchContext {
            cwd: PathBuf::from("/tmp/gitcomet"),
            repo_name: Some("gitcomet".to_string()),
        };

        let err = resolve_custom_external_terminal_launch_spec(&preferences, &context)
            .expect_err("blank external launcher program");
        assert_eq!(
            err,
            "Set a custom terminal launcher program in Settings > Terminal."
        );
    }

    #[test]
    fn shell_single_quote_escapes_single_quotes() {
        assert_eq!(shell_single_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn find_executable_in_path_accepts_direct_paths_with_components() {
        let dir = temp_test_dir("direct-path");
        let direct = dir.join("launcher");
        fs::write(&direct, b"stub").expect("stub file should be written");
        assert_eq!(
            find_executable_in_path(&direct.to_string_lossy()),
            Some(direct)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn automatic_external_terminal_prefers_first_supported_candidate_on_path() {
        let dir = temp_test_dir("linux-auto");
        write_executable(&dir, "kitty");
        write_executable(&dir, "wezterm");
        let context = ExternalTerminalLaunchContext {
            cwd: PathBuf::from("/tmp/gitcomet"),
            repo_name: Some("gitcomet".to_string()),
        };

        let spec = resolve_automatic_external_terminal_launch_spec_with_lookup(&context, |name| {
            let candidate = dir.join(name);
            candidate.exists().then_some(candidate)
        })
        .expect("automatic terminal launcher");
        assert_eq!(spec.program, OsString::from("kitty"));
        assert_eq!(
            spec.args,
            vec![
                OsString::from("--directory"),
                OsString::from("/tmp/gitcomet"),
            ]
        );
        assert_eq!(spec.current_dir, Some(PathBuf::from("/tmp/gitcomet")));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn automatic_external_terminal_uses_xterm_shell_wrapper_when_needed() {
        let dir = temp_test_dir("linux-xterm");
        write_executable(&dir, "xterm");
        let context = ExternalTerminalLaunchContext {
            cwd: PathBuf::from("/tmp/gitcomet path/it's here"),
            repo_name: Some("gitcomet".to_string()),
        };

        let spec = resolve_automatic_external_terminal_launch_spec_with_lookup(&context, |name| {
            let candidate = dir.join(name);
            candidate.exists().then_some(candidate)
        })
        .expect("xterm fallback");
        assert_eq!(spec.program, OsString::from("xterm"));
        assert_eq!(
            spec.args,
            vec![
                OsString::from("-e"),
                OsString::from("sh"),
                OsString::from("-lc"),
                OsString::from(
                    "cd '/tmp/gitcomet path/it'\"'\"'s here' && exec \"${SHELL:-/bin/sh}\""
                ),
            ]
        );
        assert_eq!(
            spec.current_dir,
            Some(PathBuf::from("/tmp/gitcomet path/it's here"))
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn automatic_external_terminal_errors_when_no_supported_launcher_exists() {
        let dir = temp_test_dir("linux-none");
        let context = ExternalTerminalLaunchContext {
            cwd: PathBuf::from("/tmp/gitcomet"),
            repo_name: Some("gitcomet".to_string()),
        };

        let err = resolve_automatic_external_terminal_launch_spec_with_lookup(&context, |name| {
            let candidate = dir.join(name);
            candidate.exists().then_some(candidate)
        })
        .expect_err("missing launcher should error");
        assert_eq!(
            err,
            "No supported terminal launcher was found on PATH. Configure a custom launcher in Settings > Terminal."
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
