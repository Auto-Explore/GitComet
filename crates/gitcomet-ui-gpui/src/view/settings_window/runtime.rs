use super::*;

impl Drop for SettingsWindowView {
    fn drop(&mut self) {
        self.signing_tools_cancellation.cancel();
        self.large_file_tools_cancellation.cancel();
    }
}

#[derive(Clone, Debug)]
pub(super) struct SettingsRuntimeInfo {
    pub(super) git: GitRuntimeInfo,
    /// `None` until the background probe finishes.
    pub(super) signing_tools: Option<SigningToolsState>,
    /// `None` while the `git lfs` / `git annex` probe runs.
    pub(super) large_file_tools: Option<gitcomet_core::large_file_tools::LargeFileToolsState>,
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
    Checking,
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

        let signing_tools = self.runtime_info.signing_tools.take();
        self.runtime_info = SettingsRuntimeInfo::from_runtime(runtime.clone());
        self.runtime_info.signing_tools = signing_tools;
        // A different Git resolves gpg and ssh-keygen with a different PATH.
        self.cancel_signing_tools_probe();
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, _cx| {
            view.cancel_signing_tools_probe();
            view.store
                .dispatch(Msg::SetGitRuntimeState(runtime.clone()));
        });
        cx.notify();
    }

    pub(super) fn apply_git_executable_settings(&mut self, cx: &mut gpui::Context<Self>) {
        let runtime = select_git_executable_path(self.selected_git_executable_path());
        self.sync_git_runtime_state(runtime, cx);
        super::super::runtime_probe::request(cx, true);
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

impl SettingsWindowView {
    pub(in crate::view) fn apply_probed_runtime(
        &mut self,
        runtime: GitRuntimeState,
        cx: &mut gpui::Context<Self>,
    ) {
        self.runtime_info = SettingsRuntimeInfo::from_runtime(runtime);
        self.refresh_signing_tools(cx);
        self.refresh_large_file_tools(cx);
        cx.notify();
    }

    pub(super) fn refresh_large_file_tools(&mut self, cx: &mut gpui::Context<Self>) {
        self.large_file_tools_cancellation.cancel();
        self.large_file_tools_probe = None;
        if cfg!(test) || !current_git_runtime().is_available() {
            return;
        }
        self.large_file_tools_cancellation = Default::default();
        let cancellation = self.large_file_tools_cancellation.clone();
        let runtime = current_git_runtime();
        self.runtime_info.large_file_tools = None;
        let detection = cx.background_spawn(async move {
            gitcomet_core::large_file_tools::detect_large_file_tools_cancellable(&cancellation)
        });
        self.large_file_tools_probe = Some(cx.spawn(async move |view, cx| {
            let tools = detection.await;
            let _ = view.update(cx, |this, cx| {
                if current_git_runtime() != runtime {
                    return;
                }
                this.runtime_info.large_file_tools = Some(tools);
                cx.notify();
            });
        }));
    }

    pub(super) fn cancel_signing_tools_probe(&mut self) {
        self.signing_tools_cancellation.cancel();
        self.signing_tools_probe = None;
        self.runtime_info.signing_tools = Some(SigningToolsState::default());
    }

    pub(super) fn refresh_signing_tools(&mut self, cx: &mut gpui::Context<Self>) {
        self.cancel_signing_tools_probe();
        if cfg!(test)
            || !self.history_verify_commit_signatures
            || !current_git_runtime().is_available()
        {
            return;
        }
        self.signing_tools_cancellation = Default::default();
        let cancellation = self.signing_tools_cancellation.clone();
        let runtime = current_git_runtime();
        self.runtime_info.signing_tools = None;
        let detection = cx.background_spawn(async move {
            gitcomet_core::signing_tools::detect_signing_tools_cancellable(&cancellation)
        });
        self.signing_tools_probe = Some(cx.spawn(async move |view, cx| {
            let tools = detection.await;
            let _ = view.update(cx, |this, cx| {
                if !this.history_verify_commit_signatures || current_git_runtime() != runtime {
                    return;
                }
                this.runtime_info.signing_tools = Some(tools);
                cx.notify();
            });
        }));
    }
}

impl SettingsRuntimeInfo {
    pub(super) fn detect() -> Self {
        Self::from_runtime(current_git_runtime())
    }

    pub(super) fn from_runtime(runtime: GitRuntimeState) -> Self {
        Self {
            git: git_runtime_info_from_state(runtime),
            signing_tools: Some(SigningToolsState::default()),
            large_file_tools: Some(Default::default()),
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
    let compatibility = if matches!(
        runtime.availability,
        gitcomet_core::process::GitExecutableAvailability::Checking
    ) {
        GitCompatibility::Checking
    } else if !runtime.is_available() {
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
        .unwrap_or(if compatibility == GitCompatibility::Checking {
            "Checking..."
        } else {
            "Unavailable"
        })
        .to_string()
        .into();

    let detail = match compatibility {
        GitCompatibility::Supported | GitCompatibility::Checking => None,
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

pub(super) const GPG_DESCRIPTION: &str =
    "Verifies GPG and X.509 commit signatures, such as commits made on GitHub.";
pub(super) const SSH_KEYGEN_DESCRIPTION: &str = "Verifies SSH commit signatures.";
pub(super) const GIT_LFS_DESCRIPTION: &str =
    "Stores large files outside Git history; needed to check out, fetch and push them.";
pub(super) const GIT_ANNEX_DESCRIPTION: &str =
    "Manages annexed file content across repositories and special remotes.";

pub(super) fn git_lfs_info(
    tools: Option<&gitcomet_core::large_file_tools::LargeFileToolsState>,
) -> SigningToolInfo {
    large_file_tool_info(
        tools.map(|tools| &tools.git_lfs),
        "git-lfs",
        "Git LFS files",
    )
}

pub(super) fn git_annex_info(
    tools: Option<&gitcomet_core::large_file_tools::LargeFileToolsState>,
) -> SigningToolInfo {
    large_file_tool_info(
        tools.map(|tools| &tools.git_annex),
        "git-annex",
        "Annexed files",
    )
}

fn large_file_tool_info(
    availability: Option<&SigningToolAvailability>,
    program: &str,
    files: &str,
) -> SigningToolInfo {
    let Some(availability) = availability else {
        return SigningToolInfo {
            status: SigningToolStatus::Detecting,
            version_display: SharedString::default(),
            detail: None,
        };
    };
    match availability {
        SigningToolAvailability::NotChecked => SigningToolInfo {
            status: SigningToolStatus::NotChecked,
            version_display: "Not checked".into(),
            detail: None,
        },
        SigningToolAvailability::Available { version } => SigningToolInfo {
            status: SigningToolStatus::Found,
            version_display: version.as_deref().unwrap_or(program).to_string().into(),
            detail: None,
        },
        SigningToolAvailability::NotFound { detail } => SigningToolInfo {
            status: SigningToolStatus::NotFound,
            version_display: program.to_string().into(),
            detail: Some(
                format!(
                    "{detail} {files} can be browsed but not fetched or pushed. Install {program} where Git can find it."
                )
                .into(),
            ),
        },
        SigningToolAvailability::Unknown => SigningToolInfo {
            status: SigningToolStatus::Unknown,
            version_display: program.to_string().into(),
            detail: Some(format!("GitComet could not tell whether Git can run {program}.").into()),
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SigningToolStatus {
    Detecting,
    NotChecked,
    Found,
    NotFound,
    Unknown,
}

#[derive(Clone, Debug)]
pub(super) struct SigningToolInfo {
    pub(super) status: SigningToolStatus,
    pub(super) version_display: SharedString,
    pub(super) detail: Option<SharedString>,
}

pub(super) fn gpg_info(tools: Option<&SigningToolsState>) -> SigningToolInfo {
    signing_tool_info(
        tools.map(|tools| &tools.gpg),
        DEFAULT_GPG_PROGRAM,
        "gpg.program",
        "GPG and X.509 commit signatures",
    )
}

pub(super) fn ssh_keygen_info(tools: Option<&SigningToolsState>) -> SigningToolInfo {
    signing_tool_info(
        tools.map(|tools| &tools.ssh_keygen),
        DEFAULT_SSH_KEYGEN_PROGRAM,
        "gpg.ssh.program",
        "SSH commit signatures",
    )
}

fn signing_tool_info(
    tool: Option<&SigningTool>,
    default_program: &str,
    config_key: &str,
    signatures: &str,
) -> SigningToolInfo {
    let Some(tool) = tool else {
        return SigningToolInfo {
            status: SigningToolStatus::Detecting,
            version_display: SharedString::default(),
            detail: None,
        };
    };
    let program = tool.program.as_str();
    match &tool.availability {
        SigningToolAvailability::NotChecked => SigningToolInfo {
            status: SigningToolStatus::NotChecked, version_display: "Not checked".into(),
            detail: Some("Enable commit signature verification in History settings to check signing tools.".into()),
        },
        SigningToolAvailability::Available { version } => SigningToolInfo {
            status: SigningToolStatus::Found,
            version_display: version.as_deref().unwrap_or(program).to_string().into(),
            detail: (!tool.is_default_program(default_program))
                .then(|| format!("Configured with `{config_key}`: {program}").into()),
        },
        SigningToolAvailability::NotFound { detail } => SigningToolInfo {
            status: SigningToolStatus::NotFound,
            version_display: program.to_string().into(),
            detail: Some(
                format!(
                    "{detail} {signatures} are not verified. Install it, or set `{config_key}` to its full path."
                )
                .into(),
            ),
        },
        SigningToolAvailability::Unknown => SigningToolInfo {
            status: SigningToolStatus::Unknown,
            version_display: program.to_string().into(),
            detail: Some(
                format!("Could not check `{program}`. Git still tries to verify {signatures}.")
                    .into(),
            ),
        },
    }
}
