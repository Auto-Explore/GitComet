//! Whether Git can run `git lfs` and `git annex`. Both are git subcommands, so
//! they are probed through the configured Git: that resolves them on Git's
//! PATH, which is what every filter, hook and command will use.

use crate::process::{
    bytes_to_text_preserving_utf8, current_git_runtime, git_command_for_preference, probe_output,
};
use crate::services::CancellationToken;
pub use crate::signing_tools::SigningToolAvailability as ToolAvailability;
use std::process::Command;
use std::time::Duration;

/// A wrapper that never exits must not pin the probe forever.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LargeFileToolsState {
    pub git_lfs: ToolAvailability,
    pub git_annex: ToolAvailability,
}

/// Unprobed, so constructing app state never spawns processes.
impl Default for LargeFileToolsState {
    fn default() -> Self {
        Self {
            git_lfs: ToolAvailability::NotChecked,
            git_annex: ToolAvailability::NotChecked,
        }
    }
}

/// Blocks for two process spawns; call it off the UI thread.
pub fn detect_large_file_tools_cancellable(
    cancellation: &CancellationToken,
) -> LargeFileToolsState {
    let runtime = current_git_runtime();
    if !runtime.is_available() || cancellation.is_cancelled() {
        return LargeFileToolsState::default();
    }
    detect_large_file_tools_with(
        &|| git_command_for_preference(&runtime.preference),
        cancellation,
    )
}

/// Injectable factory for deterministic tests.
pub fn detect_large_file_tools_with(
    git: &(dyn Fn() -> Command + Sync),
    cancellation: &CancellationToken,
) -> LargeFileToolsState {
    std::thread::scope(|scope| {
        let lfs = scope.spawn(|| probe_subcommand(git, &["lfs", "version"], cancellation));
        let annex = probe_subcommand(git, &["annex", "version", "--raw"], cancellation);
        LargeFileToolsState {
            git_lfs: lfs.join().unwrap_or(ToolAvailability::Unknown),
            git_annex: annex,
        }
    })
}

fn probe_subcommand(
    git: &(dyn Fn() -> Command + Sync),
    args: &[&str],
    cancellation: &CancellationToken,
) -> ToolAvailability {
    if cancellation.is_cancelled() {
        return ToolAvailability::NotChecked;
    }
    let mut command = git();
    // "is not a git command" is translated; match it in C.
    command.args(args).env("LC_ALL", "C").env("LANGUAGE", "C");
    let output = match probe_output(command, PROBE_TIMEOUT, cancellation) {
        Ok(output) => output,
        Err(_) => return ToolAvailability::Unknown,
    };
    let stdout = bytes_to_text_preserving_utf8(&output.stdout);
    let stderr = bytes_to_text_preserving_utf8(&output.stderr);
    let first = |text: &str| {
        text.lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string)
    };
    if output.status.success() {
        return ToolAvailability::Available {
            version: first(&stdout).or_else(|| first(&stderr)),
        };
    }
    // `git: 'lfs' is not a git command. See 'git --help'.`
    if stderr.contains("is not a git command") {
        return ToolAvailability::NotFound {
            detail: format!("Git cannot run `git {}`.", args[0]),
        };
    }
    ToolAvailability::Unknown
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn fake_git(script: &'static str) -> impl Fn() -> Command + Sync {
        move || {
            let mut command = Command::new("sh");
            command.args(["-c", script, "git"]);
            command
        }
    }

    #[test]
    fn reports_versions_when_subcommands_run() {
        let git = fake_git(
            r#"case "$1" in lfs) echo "git-lfs/3.8.0 (GitHub; linux amd64; go 1.27.0)";; annex) echo "10.20260901";; esac"#,
        );
        let state = detect_large_file_tools_with(&git, &CancellationToken::new());
        assert_eq!(
            state.git_lfs,
            ToolAvailability::Available {
                version: Some("git-lfs/3.8.0 (GitHub; linux amd64; go 1.27.0)".into())
            }
        );
        assert_eq!(
            state.git_annex,
            ToolAvailability::Available {
                version: Some("10.20260901".into())
            }
        );
    }

    #[test]
    fn missing_subcommand_is_not_found_and_other_failures_are_unknown() {
        let git = fake_git(
            r#"case "$1" in lfs) echo "git: 'lfs' is not a git command. See 'git --help'." >&2; exit 1;; *) echo boom >&2; exit 2;; esac"#,
        );
        let state = detect_large_file_tools_with(&git, &CancellationToken::new());
        assert!(state.git_lfs.is_not_found(), "{:?}", state.git_lfs);
        assert_eq!(state.git_annex, ToolAvailability::Unknown);
    }

    /// Git translates "is not a git command"; the probe must read it in C.
    #[test]
    fn missing_subcommand_is_recognised_under_a_translated_git() {
        let git = fake_git(
            r#"if [ "${LC_ALL:-}" = C ]; then msg="is not a git command"; else msg="ist kein Git-Befehl"; fi
               echo "git: '$1' $msg. See 'git --help'." >&2; exit 1"#,
        );
        let state = detect_large_file_tools_with(&git, &CancellationToken::new());
        assert!(state.git_lfs.is_not_found(), "{:?}", state.git_lfs);
        assert!(state.git_annex.is_not_found(), "{:?}", state.git_annex);
    }

    #[test]
    fn cancelled_probe_reports_not_checked() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let state = detect_large_file_tools_with(&fake_git("echo x"), &cancellation);
        assert_eq!(state, LargeFileToolsState::default());
    }
}
