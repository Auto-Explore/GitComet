use super::*;

#[derive(Clone, Debug)]
pub(super) struct SettingsRuntimeInfo {
    pub(super) git: GitRuntimeInfo,
    pub(super) app_version_display: SharedString,
    pub(super) operating_system: SharedString,
}

#[derive(Clone, Debug)]
pub(super) struct GitRuntimeInfo {
    pub(super) runtime: GitRuntimeState,
    pub(super) version_display: SharedString,
    pub(super) compatibility: GitCompatibility,
    pub(super) detail: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GitCompatibility {
    Supported,
    TooOld,
    Unknown,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct GitVersion {
    pub(super) major: u32,
    pub(super) minor: u32,
}

#[derive(Clone, Debug)]
pub(super) struct TerminalSettingsStatus {
    pub(super) is_error: bool,
    pub(super) text: SharedString,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TerminalProgramInputTarget {
    ExternalTerminal,
}

impl SettingsWindowView {
    pub(super) fn selected_git_executable_path(&self) -> Option<std::path::PathBuf> {
        match self.git_executable_mode {
            GitExecutableMode::SystemPath => None,
            GitExecutableMode::Custom => {
                let trimmed = self.git_custom_path_draft.trim();
                Some(if trimmed.is_empty() {
                    std::path::PathBuf::new()
                } else {
                    std::path::PathBuf::from(trimmed)
                })
            }
        }
    }

    pub(super) fn sync_git_runtime_state(
        &mut self,
        runtime: GitRuntimeState,
        cx: &mut gpui::Context<Self>,
    ) {
        self.git_executable_mode = GitExecutableMode::from_preference(&runtime.preference);
        if let GitExecutablePreference::Custom(path) = &runtime.preference {
            let next_draft = if path.as_os_str().is_empty() {
                String::new()
            } else {
                path.display().to_string()
            };
            if self.git_custom_path_draft != next_draft {
                self.git_custom_path_draft = next_draft.clone();
                self.git_executable_input
                    .update(cx, |input, cx| input.set_text(next_draft, cx));
            }
        }

        self.runtime_info = SettingsRuntimeInfo::from_runtime(runtime.clone());
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, _cx| {
            view.store
                .dispatch(Msg::SetGitRuntimeState(runtime.clone()));
        });
        cx.notify();
    }

    pub(super) fn apply_git_executable_settings(&mut self, cx: &mut gpui::Context<Self>) {
        let runtime = install_git_executable_path(self.selected_git_executable_path());
        self.sync_git_runtime_state(runtime, cx);
    }

    pub(super) fn set_git_executable_mode(
        &mut self,
        mode: GitExecutableMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.git_executable_mode == mode {
            return;
        }

        self.git_executable_mode = mode;
        self.apply_git_executable_settings(cx);
    }
}

impl SettingsRuntimeInfo {
    pub(super) fn detect() -> Self {
        Self::from_runtime(refresh_git_runtime())
    }

    pub(super) fn from_runtime(runtime: GitRuntimeState) -> Self {
        Self {
            git: git_runtime_info_from_state(runtime),
            app_version_display: format!("GitComet v{}", env!("CARGO_PKG_VERSION")).into(),
            operating_system: format!(
                "{} ({})",
                os_display_name(std::env::consts::OS),
                std::env::consts::ARCH
            )
            .into(),
        }
    }
}

/// Human-readable OS name for the Environment card ("windows" reads like a
/// debug dump; "Windows" reads like a product).
pub(super) fn os_display_name(os: &str) -> &str {
    match os {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        "freebsd" => "FreeBSD",
        other => other,
    }
}

pub(super) fn git_runtime_info_from_state(runtime: GitRuntimeState) -> GitRuntimeInfo {
    let compatibility_message =
        format!("GitComet has been tested only with Git {MIN_GIT_MAJOR}.{MIN_GIT_MINOR} or newer.");
    let compatibility = if !runtime.is_available() {
        GitCompatibility::Unavailable
    } else {
        match runtime.version_output().and_then(parse_git_version) {
            Some(version) if is_supported_git_version(version) => GitCompatibility::Supported,
            Some(_) => GitCompatibility::TooOld,
            None => GitCompatibility::Unknown,
        }
    };

    let version_display = runtime
        .version_output()
        .unwrap_or("Unavailable")
        .to_string()
        .into();

    let detail = match compatibility {
        GitCompatibility::Supported => None,
        GitCompatibility::TooOld | GitCompatibility::Unknown => Some(compatibility_message.into()),
        GitCompatibility::Unavailable => runtime
            .unavailable_detail()
            .map(|detail| SharedString::from(detail.to_string())),
    };

    GitRuntimeInfo {
        runtime,
        version_display,
        compatibility,
        detail,
    }
}

pub(super) fn parse_git_version(raw: &str) -> Option<GitVersion> {
    raw.split_whitespace().find_map(parse_git_version_token)
}

pub(super) fn parse_git_version_token(token: &str) -> Option<GitVersion> {
    let mut parts = token.split('.');
    let major = parse_u32_prefix(parts.next()?)?;
    let minor = parse_u32_prefix(parts.next()?)?;
    Some(GitVersion { major, minor })
}

pub(super) fn parse_u32_prefix(part: &str) -> Option<u32> {
    let end = part
        .char_indices()
        .find_map(|(ix, ch)| (!ch.is_ascii_digit()).then_some(ix))
        .unwrap_or(part.len());
    if end == 0 {
        return None;
    }
    part[..end].parse::<u32>().ok()
}

pub(super) fn is_supported_git_version(version: GitVersion) -> bool {
    version.major > MIN_GIT_MAJOR
        || (version.major == MIN_GIT_MAJOR && version.minor >= MIN_GIT_MINOR)
}
