//! Detection of the programs Git runs to verify commit signatures.
//!
//! `git log --format=%G?` silently reports "no signature" when the verifier
//! cannot be started, so GitComet probes for them up front: gpg checks OpenPGP
//! signatures (and X.509 through gpgsm, which ships with GnuPG) and ssh-keygen
//! checks SSH signatures.
//!
//! Git resolves those programs with its own PATH, which is not GitComet's: Git
//! for Windows adds its bundled `usr\bin`, and a custom Git adds its directory.
//! A bare program name is therefore probed *through* Git as a `!program` alias.
//! Git executes such an alias directly, without a shell, when the name contains
//! no shell metacharacters, which [`classify_program`] guarantees.
//!
//! Verification waits for discovery. After discovery, an inconclusive probe
//! lets Git try; a definite "not found" excludes that verifier's formats.

use crate::domain::{SignatureFormat, SignatureFormats};
use crate::process::{
    background_command, bytes_to_text_preserving_utf8, current_git_runtime,
    git_command_for_preference, probe_output,
};
use crate::services::CancellationToken;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

pub const DEFAULT_GPG_PROGRAM: &str = "gpg";
pub const DEFAULT_SSH_KEYGEN_PROGRAM: &str = "ssh-keygen";

const PROBE_ALIAS: &str = "gitcomet-signing-probe";
/// A wrapper script that never exits must not pin the probe forever.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SigningToolAvailability {
    /// No probe has completed. Do not start speculative verification.
    NotChecked,
    /// The completed probe was inconclusive; let Git try.
    Unknown,
    Available {
        version: Option<String>,
    },
    NotFound {
        detail: String,
    },
}

impl SigningToolAvailability {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SigningTool {
    /// The program Git is configured to run (`gpg.program`, `gpg.ssh.program`),
    /// or the default name.
    pub program: String,
    pub availability: SigningToolAvailability,
}

impl SigningTool {
    fn unknown(program: &str) -> Self {
        Self {
            program: program.to_string(),
            availability: SigningToolAvailability::NotChecked,
        }
    }

    pub fn is_default_program(&self, default: &str) -> bool {
        self.program == default
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SigningToolsState {
    pub gpg: SigningTool,
    pub ssh_keygen: SigningTool,
}

/// Unprobed, so constructing app state never spawns processes.
impl Default for SigningToolsState {
    fn default() -> Self {
        Self {
            gpg: SigningTool::unknown(DEFAULT_GPG_PROGRAM),
            ssh_keygen: SigningTool::unknown(DEFAULT_SSH_KEYGEN_PROGRAM),
        }
    }
}

impl SigningToolsState {
    /// The signature formats whose verifier was not positively ruled out.
    pub fn usable_formats(&self) -> SignatureFormats {
        let mut formats = SignatureFormats::NONE;
        if matches!(
            self.gpg.availability,
            SigningToolAvailability::Available { .. } | SigningToolAvailability::Unknown
        ) {
            formats = formats
                .with(SignatureFormat::OpenPgp)
                .with(SignatureFormat::X509);
        }
        if matches!(
            self.ssh_keygen.availability,
            SigningToolAvailability::Available { .. } | SigningToolAvailability::Unknown
        ) {
            formats = formats.with(SignatureFormat::Ssh);
        }
        formats
    }
}

/// Probes the signing programs of the currently installed Git runtime. Blocks
/// for a few process spawns, so call it off the UI thread.
pub fn detect_signing_tools() -> SigningToolsState {
    detect_signing_tools_cancellable(&CancellationToken::new())
}

pub fn detect_signing_tools_cancellable(cancellation: &CancellationToken) -> SigningToolsState {
    let runtime = current_git_runtime();
    if !runtime.is_available() || cancellation.is_cancelled() {
        return SigningToolsState::default();
    }
    detect_signing_tools_with_cancellation(
        &|| git_command_for_preference(&runtime.preference),
        cancellation,
    )
}

/// Injectable factory for deterministic tests and callers with a frozen runtime.
pub fn detect_signing_tools_with(git: &(dyn Fn() -> Command + Sync)) -> SigningToolsState {
    detect_signing_tools_with_cancellation(git, &CancellationToken::new())
}

pub fn detect_signing_tools_with_cancellation(
    git: &(dyn Fn() -> Command + Sync),
    cancellation: &CancellationToken,
) -> SigningToolsState {
    if cancellation.is_cancelled() {
        return SigningToolsState::default();
    }
    let programs = read_signing_programs(git, cancellation);
    let gpg_program = programs
        .gpg
        .unwrap_or_else(|| DEFAULT_GPG_PROGRAM.to_string());
    let ssh_keygen_program = programs
        .ssh_keygen
        .unwrap_or_else(|| DEFAULT_SSH_KEYGEN_PROGRAM.to_string());

    let (gpg, ssh_keygen) = std::thread::scope(|scope| {
        let gpg = scope.spawn(|| detect_gpg(git, &gpg_program, cancellation));
        let ssh_keygen = detect_ssh_keygen(git, &ssh_keygen_program, cancellation);
        (
            gpg.join().unwrap_or(SigningToolAvailability::Unknown),
            ssh_keygen,
        )
    });

    SigningToolsState {
        gpg: SigningTool {
            program: gpg_program,
            availability: gpg,
        },
        ssh_keygen: SigningTool {
            program: ssh_keygen_program,
            availability: ssh_keygen,
        },
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct SigningPrograms {
    gpg: Option<String>,
    ssh_keygen: Option<String>,
}

fn read_signing_programs(
    git: &(dyn Fn() -> Command + Sync),
    cancellation: &CancellationToken,
) -> SigningPrograms {
    let mut command = git();
    command
        .args([
            "config",
            "--show-scope",
            "--get-regexp",
            r"^gpg\.(program|openpgp\.program|ssh\.program)$",
        ])
        .current_dir(std::env::temp_dir());
    match output_with_timeout_cancellable(command, PROBE_TIMEOUT, cancellation) {
        Ok(Some(output)) if output.status.success() => {
            parse_signing_programs(&bytes_to_text_preserving_utf8(&output.stdout))
        }
        // Exit 1 means none of the keys is set.
        _ => SigningPrograms::default(),
    }
}

/// Parses `git config --show-scope --get-regexp` lines (`<scope>\t<key> <value>`).
/// Repository scopes are ignored: detection is app-wide, not per repository.
fn parse_signing_programs(output: &str) -> SigningPrograms {
    let mut programs = SigningPrograms::default();
    for line in output.lines() {
        let Some((scope, entry)) = line.split_once('\t') else {
            continue;
        };
        if matches!(scope, "local" | "worktree") {
            continue;
        }
        let Some((key, value)) = entry.split_once(' ') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        // Later entries override earlier ones, as they do for Git.
        match key.to_ascii_lowercase().as_str() {
            "gpg.program" | "gpg.openpgp.program" => programs.gpg = Some(value.to_string()),
            "gpg.ssh.program" => programs.ssh_keygen = Some(value.to_string()),
            _ => {}
        }
    }
    programs
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProgramKind {
    /// Looked up on Git's PATH.
    Bare,
    Absolute,
    /// Relative to a repository, or otherwise not resolvable app-wide.
    Unresolvable,
}

fn classify_program(program: &str) -> ProgramKind {
    let is_bare = !program.is_empty()
        && !program.starts_with('-')
        && program
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'));
    if is_bare {
        ProgramKind::Bare
    } else if Path::new(program).is_absolute() {
        ProgramKind::Absolute
    } else {
        ProgramKind::Unresolvable
    }
}

enum ProbeOutcome {
    Ran(Output),
    NotFound(String),
    Inconclusive,
}

fn probe_program(
    git: &(dyn Fn() -> Command + Sync),
    program: &str,
    args: &[&str],
    cancellation: &CancellationToken,
) -> ProbeOutcome {
    let kind = classify_program(program);
    let mut command = match kind {
        ProgramKind::Bare => {
            let mut command = git();
            command
                .arg("-c")
                .arg(format!("alias.{PROBE_ALIAS}=!{program}"))
                .arg(PROBE_ALIAS);
            command
        }
        ProgramKind::Absolute => background_command(program),
        ProgramKind::Unresolvable => return ProbeOutcome::Inconclusive,
    };
    command
        .args(args)
        .current_dir(std::env::temp_dir())
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C");

    match output_with_timeout_cancellable(command, PROBE_TIMEOUT, cancellation) {
        Ok(Some(output)) if kind == ProgramKind::Bare && alias_program_missing(&output) => {
            ProbeOutcome::NotFound(format!("`{program}` was not found on Git's PATH."))
        }
        Ok(Some(output)) => ProbeOutcome::Ran(output),
        Ok(None) => ProbeOutcome::Inconclusive,
        Err(err)
            if kind == ProgramKind::Absolute
                && matches!(
                    err.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
        {
            ProbeOutcome::NotFound(format!("{program} could not be run: {err}"))
        }
        Err(_) => ProbeOutcome::Inconclusive,
    }
}

/// Git reports an alias program it cannot start with
/// `fatal: while expanding alias '<alias>': '<program>': <error>` and exit 128.
fn alias_program_missing(output: &Output) -> bool {
    output.status.code() == Some(128)
        && bytes_to_text_preserving_utf8(&output.stderr)
            .contains(&format!("while expanding alias '{PROBE_ALIAS}'"))
}

fn detect_gpg(
    git: &(dyn Fn() -> Command + Sync),
    program: &str,
    cancellation: &CancellationToken,
) -> SigningToolAvailability {
    match probe_program(git, program, &["--version"], cancellation) {
        ProbeOutcome::Ran(output) => SigningToolAvailability::Available {
            version: first_line(&output.stdout).or_else(|| first_line(&output.stderr)),
        },
        ProbeOutcome::NotFound(detail) => SigningToolAvailability::NotFound { detail },
        ProbeOutcome::Inconclusive => SigningToolAvailability::Unknown,
    }
}

fn detect_ssh_keygen(
    git: &(dyn Fn() -> Command + Sync),
    program: &str,
    cancellation: &CancellationToken,
) -> SigningToolAvailability {
    // ssh-keygen has no version flag; an unknown option prints usage and exits 1.
    match probe_program(git, program, &["-?"], cancellation) {
        ProbeOutcome::Ran(_) => SigningToolAvailability::Available {
            version: openssh_version(git, program, cancellation),
        },
        ProbeOutcome::NotFound(detail) => SigningToolAvailability::NotFound { detail },
        ProbeOutcome::Inconclusive => SigningToolAvailability::Unknown,
    }
}

/// ssh-keygen's version, read from the `ssh` client installed beside it.
fn openssh_version(
    git: &(dyn Fn() -> Command + Sync),
    ssh_keygen: &str,
    cancellation: &CancellationToken,
) -> Option<String> {
    let ssh = match classify_program(ssh_keygen) {
        ProgramKind::Bare if ssh_keygen == DEFAULT_SSH_KEYGEN_PROGRAM => "ssh".to_string(),
        ProgramKind::Absolute => {
            let ssh = Path::new(ssh_keygen)
                .with_file_name(format!("ssh{}", std::env::consts::EXE_SUFFIX));
            if !ssh.is_file() {
                return None;
            }
            ssh.to_str()?.to_string()
        }
        _ => return None,
    };
    match probe_program(git, &ssh, &["-V"], cancellation) {
        ProbeOutcome::Ran(output) if output.status.success() => {
            first_line(&output.stderr).or_else(|| first_line(&output.stdout))
        }
        _ => None,
    }
}

fn first_line(bytes: &[u8]) -> Option<String> {
    bytes_to_text_preserving_utf8(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn output_with_timeout_cancellable(
    command: Command,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> std::io::Result<Option<Output>> {
    match probe_output(command, timeout, cancellation) {
        Ok(output) => Ok(Some(output)),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(availability: SigningToolAvailability) -> SigningTool {
        SigningTool {
            program: "tool".to_string(),
            availability,
        }
    }

    fn not_found() -> SigningToolAvailability {
        SigningToolAvailability::NotFound {
            detail: "missing".to_string(),
        }
    }

    #[test]
    fn unprobed_tools_do_not_start_verification() {
        assert_eq!(
            SigningToolsState::default().usable_formats(),
            SignatureFormats::NONE
        );
    }

    #[test]
    fn a_missing_gpg_rules_out_openpgp_and_x509_only() {
        let state = SigningToolsState {
            gpg: tool(not_found()),
            ssh_keygen: tool(SigningToolAvailability::Available { version: None }),
        };
        let formats = state.usable_formats();
        assert!(!formats.contains(SignatureFormat::OpenPgp));
        assert!(!formats.contains(SignatureFormat::X509));
        assert!(formats.contains(SignatureFormat::Ssh));
    }

    #[test]
    fn missing_tools_leave_no_usable_format() {
        let state = SigningToolsState {
            gpg: tool(not_found()),
            ssh_keygen: tool(not_found()),
        };
        assert!(state.usable_formats().is_empty());
    }

    #[test]
    fn config_parsing_keeps_the_last_app_wide_value() {
        let programs = parse_signing_programs(
            "system\tgpg.program /usr/bin/gpg\n\
             global\tgpg.openpgp.program C:/Program Files/GnuPG/bin/gpg.exe\n\
             local\tgpg.program repo-gpg\n\
             global\tgpg.ssh.program ssh-keygen-wrapper\n",
        );
        assert_eq!(
            programs,
            SigningPrograms {
                gpg: Some("C:/Program Files/GnuPG/bin/gpg.exe".to_string()),
                ssh_keygen: Some("ssh-keygen-wrapper".to_string()),
            }
        );
        assert_eq!(parse_signing_programs(""), SigningPrograms::default());
    }

    #[test]
    fn only_plain_names_are_probed_through_git() {
        for bare in ["gpg", "gpg2", "ssh-keygen", "gpg.exe", "smime_sign+1"] {
            assert_eq!(classify_program(bare), ProgramKind::Bare, "{bare}");
        }
        for unresolvable in ["", "-c", "gpg;rm", "$(gpg)", "gpg --x", "bin/gpg", "~/gpg"] {
            assert_eq!(
                classify_program(unresolvable),
                ProgramKind::Unresolvable,
                "{unresolvable}"
            );
        }
        #[cfg(unix)]
        assert_eq!(classify_program("/usr/bin/gpg"), ProgramKind::Absolute);
        #[cfg(windows)]
        assert_eq!(
            classify_program(r"C:\Program Files\GnuPG\bin\gpg.exe"),
            ProgramKind::Absolute
        );
    }

    #[cfg(unix)]
    mod with_git {
        use super::*;
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;
        use std::path::PathBuf;

        struct Fixture {
            _dir: tempfile::TempDir,
            bin: PathBuf,
            global_config: PathBuf,
        }

        impl Fixture {
            fn new() -> Option<Self> {
                let git_runs = background_command("git")
                    .arg("--version")
                    .output()
                    .is_ok_and(|output| output.status.success());
                if !git_runs {
                    eprintln!("skipping: git is unavailable");
                    return None;
                }
                let dir = tempfile::tempdir().expect("create temp dir");
                let bin = dir.path().join("bin");
                fs::create_dir(&bin).expect("create bin dir");
                let global_config = dir.path().join("gitconfig");
                fs::write(&global_config, "").expect("write gitconfig");
                Some(Self {
                    _dir: dir,
                    bin,
                    global_config,
                })
            }

            fn script(&self, name: &str, body: &str) -> PathBuf {
                let path = self.bin.join(name);
                fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
                let mut permissions = fs::metadata(&path).expect("metadata").permissions();
                permissions.set_mode(0o700);
                fs::set_permissions(&path, permissions).expect("chmod script");
                path
            }

            fn config(&self, contents: &str) {
                fs::write(&self.global_config, contents).expect("write gitconfig");
            }

            fn detect(&self) -> SigningToolsState {
                let mut path = vec![self.bin.clone()];
                path.extend(std::env::split_paths(
                    &std::env::var_os("PATH").unwrap_or_default(),
                ));
                let path = std::env::join_paths(path).expect("join PATH");
                let global_config = self.global_config.clone();
                detect_signing_tools_with(&move || {
                    let mut command = background_command("git");
                    command
                        .env("PATH", &path)
                        .env("GIT_CONFIG_NOSYSTEM", "1")
                        .env("GIT_CONFIG_GLOBAL", &global_config);
                    command
                })
            }
        }

        #[test]
        fn a_configured_program_on_gits_path_reports_its_version() {
            let Some(fixture) = Fixture::new() else {
                return;
            };
            fixture.script(
                "gitcomet-test-gpg",
                "printf 'gpg (GnuPG) 9.9.9-test\\nlibgcrypt 1.0\\n'",
            );
            fixture.config("[gpg]\n\tprogram = gitcomet-test-gpg\n");

            let state = fixture.detect();

            assert_eq!(state.gpg.program, "gitcomet-test-gpg");
            assert_eq!(
                state.gpg.availability,
                SigningToolAvailability::Available {
                    version: Some("gpg (GnuPG) 9.9.9-test".to_string())
                }
            );
        }

        #[test]
        fn a_program_missing_from_gits_path_is_not_found() {
            let Some(fixture) = Fixture::new() else {
                return;
            };
            fixture.config("[gpg]\n\tprogram = gitcomet-test-missing-gpg\n");

            let state = fixture.detect();

            assert!(
                state.gpg.availability.is_not_found(),
                "{:?}",
                state.gpg.availability
            );
            assert!(!state.usable_formats().contains(SignatureFormat::OpenPgp));
        }

        #[test]
        fn an_absolute_ssh_keygen_reports_the_sibling_ssh_version() {
            let Some(fixture) = Fixture::new() else {
                return;
            };
            let ssh_keygen = fixture.script(
                "ssh-keygen",
                "printf 'usage: ssh-keygen [-Y verify]\\n' >&2; exit 1",
            );
            fixture.script("ssh", "printf 'OpenSSH_9.9p9-test\\n' >&2");
            fixture.config(&format!(
                "[gpg \"ssh\"]\n\tprogram = {}\n",
                ssh_keygen.display()
            ));

            let state = fixture.detect();

            assert_eq!(
                state.ssh_keygen.availability,
                SigningToolAvailability::Available {
                    version: Some("OpenSSH_9.9p9-test".to_string())
                }
            );
        }

        #[test]
        fn a_missing_absolute_ssh_keygen_is_not_found() {
            let Some(fixture) = Fixture::new() else {
                return;
            };
            let missing = fixture.bin.join("no-such-ssh-keygen");
            fixture.config(&format!(
                "[gpg \"ssh\"]\n\tprogram = {}\n",
                missing.display()
            ));

            let state = fixture.detect();

            assert!(state.ssh_keygen.availability.is_not_found());
            assert!(!state.usable_formats().contains(SignatureFormat::Ssh));
        }
    }
}
