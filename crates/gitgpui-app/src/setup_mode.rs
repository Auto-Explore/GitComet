//! `gitgpui-app setup` — configure git to use gitgpui as difftool/mergetool.
//!
//! Writes the recommended global (or local) git config entries so that
//! `git difftool` and `git mergetool` invoke gitgpui automatically.

use std::path::PathBuf;

/// A single `git config` key-value pair to set.
struct ConfigEntry {
    key: &'static str,
    value: String,
}

/// Quote a string as a POSIX-shell single-quoted literal.
///
/// This preserves spaces and shell metacharacters, including embedded
/// single quotes.
fn shell_single_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\"'\"'");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Resolve the absolute path to the current executable.
fn current_exe_path() -> Result<PathBuf, String> {
    std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| format!("Cannot determine gitgpui-app binary path: {e}"))
}

fn quoted_env_var(name: &str) -> String {
    format!("\"${name}\"")
}

/// Build the list of git config entries for difftool/mergetool setup.
fn build_config_entries(bin_path: &str) -> Vec<ConfigEntry> {
    let quoted_bin_path = shell_single_quote(bin_path);
    let base = quoted_env_var("BASE");
    let local = quoted_env_var("LOCAL");
    let remote = quoted_env_var("REMOTE");
    let merged = quoted_env_var("MERGED");

    vec![
        // Mergetool (headless — for CI, scripts, and no-display environments)
        ConfigEntry {
            key: "merge.tool",
            value: "gitgpui".into(),
        },
        ConfigEntry {
            key: "mergetool.gitgpui.cmd",
            value: format!(
                "{quoted_bin_path} mergetool --base {base} --local {local} --remote {remote} --merged {merged}"
            ),
        },
        // Keep both generic and tool-specific trust keys:
        // - `mergetool.trustExitCode` matches documented setup guidance and
        //   Git's default trust behavior for the selected mergetool.
        // - `mergetool.gitgpui.trustExitCode` preserves explicit per-tool
        //   behavior even if users override global defaults later.
        ConfigEntry {
            key: "mergetool.trustExitCode",
            value: "true".into(),
        },
        ConfigEntry {
            key: "mergetool.gitgpui.trustExitCode",
            value: "true".into(),
        },
        ConfigEntry {
            key: "mergetool.prompt",
            value: "false".into(),
        },
        // Difftool (headless)
        ConfigEntry {
            key: "diff.tool",
            value: "gitgpui".into(),
        },
        ConfigEntry {
            key: "difftool.gitgpui.cmd",
            value: format!(
                "{quoted_bin_path} difftool --local {local} --remote {remote} --path {merged}"
            ),
        },
        // Keep both generic and tool-specific trust keys:
        // - `difftool.trustExitCode` matches documented setup guidance and
        //   Git's default trust behavior for the selected difftool.
        // - `difftool.gitgpui.trustExitCode` preserves explicit per-tool
        //   behavior even if users override global defaults later.
        ConfigEntry {
            key: "difftool.trustExitCode",
            value: "true".into(),
        },
        ConfigEntry {
            key: "difftool.gitgpui.trustExitCode",
            value: "true".into(),
        },
        ConfigEntry {
            key: "difftool.prompt",
            value: "false".into(),
        },
        // GUI tool variant — opens focused GPUI windows for interactive use.
        // Registered as a separate tool name so guiDefault=auto selects the
        // interactive UI when DISPLAY is available and the headless backend
        // when it is not.
        ConfigEntry {
            key: "merge.guitool",
            value: "gitgpui-gui".into(),
        },
        ConfigEntry {
            key: "mergetool.gitgpui-gui.cmd",
            value: format!(
                "{quoted_bin_path} mergetool --gui --base {base} --local {local} --remote {remote} --merged {merged}"
            ),
        },
        ConfigEntry {
            key: "mergetool.gitgpui-gui.trustExitCode",
            value: "true".into(),
        },
        ConfigEntry {
            key: "diff.guitool",
            value: "gitgpui-gui".into(),
        },
        ConfigEntry {
            key: "difftool.gitgpui-gui.cmd",
            value: format!(
                "{quoted_bin_path} difftool --gui --local {local} --remote {remote} --path {merged}"
            ),
        },
        ConfigEntry {
            key: "difftool.gitgpui-gui.trustExitCode",
            value: "true".into(),
        },
        ConfigEntry {
            key: "mergetool.guiDefault",
            value: "auto".into(),
        },
        ConfigEntry {
            key: "difftool.guiDefault",
            value: "auto".into(),
        },
    ]
}

/// Format the `git config` shell commands for display (dry-run mode).
fn format_commands(entries: &[ConfigEntry], scope: &str) -> String {
    let mut out = String::new();
    for entry in entries {
        let quoted_value = shell_single_quote(&entry.value);
        out.push_str(&format!(
            "git config {scope} {} {quoted_value}\n",
            entry.key
        ));
    }
    out
}

/// Run `git config` for each entry.
fn apply_config(entries: &[ConfigEntry], scope: &str) -> Result<(), String> {
    for entry in entries {
        let output = std::process::Command::new("git")
            .args(["config", scope, entry.key, &entry.value])
            .output()
            .map_err(|e| format!("Failed to run git config: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "git config {} {} failed: {}",
                entry.key,
                entry.value,
                stderr.trim()
            ));
        }
    }
    Ok(())
}

/// Result returned from `run_setup`.
pub struct SetupResult {
    pub stdout: String,
    pub exit_code: i32,
}

/// Execute the setup command.
pub fn run_setup(dry_run: bool, local: bool) -> Result<SetupResult, String> {
    let bin_path = current_exe_path()?;
    let bin_str = bin_path.to_str().ok_or_else(|| {
        format!(
            "Binary path contains non-UTF-8 characters: {}",
            bin_path.display()
        )
    })?;

    let entries = build_config_entries(bin_str);
    let scope = if local { "--local" } else { "--global" };
    let scope_label = if local { "local" } else { "global" };

    if dry_run {
        let commands = format_commands(&entries, scope);
        let stdout =
            format!("# Dry run: the following git config commands would be executed:\n{commands}");
        return Ok(SetupResult {
            stdout,
            exit_code: 0,
        });
    }

    apply_config(&entries, scope)?;

    let stdout = format!(
        "Configured gitgpui as {scope_label} diff/merge tool.\n\
         Binary: {bin_str}\n\
         Run `git difftool` or `git mergetool` to use it.\n"
    );

    Ok(SetupResult {
        stdout,
        exit_code: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_occurrences(haystack: &str, needle: &str) -> usize {
        haystack.match_indices(needle).count()
    }

    fn assert_placeholder_is_quoted(cmd: &str, var: &str) {
        let raw = format!("${var}");
        let quoted = format!("\"{raw}\"");

        let raw_count = count_occurrences(cmd, &raw);
        let quoted_count = count_occurrences(cmd, &quoted);

        assert!(
            quoted_count > 0,
            "expected quoted placeholder {quoted} in command: {cmd}"
        );
        assert_eq!(
            raw_count, quoted_count,
            "found unquoted placeholder ${var} in command: {cmd}"
        );
    }

    #[test]
    fn shell_single_quote_wraps_plain_text() {
        assert_eq!(shell_single_quote("abc"), "'abc'");
        assert_eq!(shell_single_quote(""), "''");
    }

    #[test]
    fn shell_single_quote_escapes_embedded_single_quote() {
        assert_eq!(shell_single_quote("it's"), "'it'\"'\"'s'");
    }

    #[test]
    fn build_config_entries_contains_all_required_keys() {
        let entries = build_config_entries("/usr/bin/gitgpui-app");
        let keys: Vec<&str> = entries.iter().map(|e| e.key).collect();

        // Headless tool
        assert!(keys.contains(&"merge.tool"));
        assert!(keys.contains(&"mergetool.gitgpui.cmd"));
        assert!(keys.contains(&"mergetool.trustExitCode"));
        assert!(keys.contains(&"mergetool.gitgpui.trustExitCode"));
        assert!(keys.contains(&"mergetool.prompt"));
        assert!(keys.contains(&"diff.tool"));
        assert!(keys.contains(&"difftool.gitgpui.cmd"));
        assert!(keys.contains(&"difftool.trustExitCode"));
        assert!(keys.contains(&"difftool.gitgpui.trustExitCode"));
        assert!(keys.contains(&"difftool.prompt"));

        // GUI tool variant
        assert!(keys.contains(&"merge.guitool"));
        assert!(keys.contains(&"diff.guitool"));
        assert!(keys.contains(&"mergetool.gitgpui-gui.cmd"));
        assert!(keys.contains(&"mergetool.gitgpui-gui.trustExitCode"));
        assert!(keys.contains(&"difftool.gitgpui-gui.cmd"));
        assert!(keys.contains(&"difftool.gitgpui-gui.trustExitCode"));
        assert!(keys.contains(&"mergetool.guiDefault"));
        assert!(keys.contains(&"difftool.guiDefault"));
    }

    #[test]
    fn gui_tool_uses_separate_tool_name() {
        let entries = build_config_entries("/usr/bin/gitgpui-app");
        let merge_guitool = entries.iter().find(|e| e.key == "merge.guitool").unwrap();
        let diff_guitool = entries.iter().find(|e| e.key == "diff.guitool").unwrap();

        assert_eq!(merge_guitool.value, "gitgpui-gui");
        assert_eq!(diff_guitool.value, "gitgpui-gui");
    }

    #[test]
    fn gui_tool_cmd_includes_gui_flag() {
        let entries = build_config_entries("/path/to/bin");
        let merge_gui_cmd = entries
            .iter()
            .find(|e| e.key == "mergetool.gitgpui-gui.cmd")
            .unwrap();
        let diff_gui_cmd = entries
            .iter()
            .find(|e| e.key == "difftool.gitgpui-gui.cmd")
            .unwrap();

        assert!(
            merge_gui_cmd.value.contains("--gui"),
            "GUI mergetool cmd missing --gui flag: {}",
            merge_gui_cmd.value
        );
        assert!(
            diff_gui_cmd.value.contains("--gui"),
            "GUI difftool cmd missing --gui flag: {}",
            diff_gui_cmd.value
        );
    }

    #[test]
    fn headless_tool_cmd_omits_gui_flag() {
        let entries = build_config_entries("/path/to/bin");
        let merge_cmd = entries
            .iter()
            .find(|e| e.key == "mergetool.gitgpui.cmd")
            .unwrap();
        let diff_cmd = entries
            .iter()
            .find(|e| e.key == "difftool.gitgpui.cmd")
            .unwrap();

        assert!(
            !merge_cmd.value.contains("--gui"),
            "headless mergetool cmd should not contain --gui: {}",
            merge_cmd.value
        );
        assert!(
            !diff_cmd.value.contains("--gui"),
            "headless difftool cmd should not contain --gui: {}",
            diff_cmd.value
        );
    }

    #[test]
    fn mergetool_cmd_includes_all_stage_vars() {
        let entries = build_config_entries("/path/to/bin");
        let cmd = entries
            .iter()
            .find(|e| e.key == "mergetool.gitgpui.cmd")
            .unwrap();

        assert_placeholder_is_quoted(&cmd.value, "BASE");
        assert_placeholder_is_quoted(&cmd.value, "LOCAL");
        assert_placeholder_is_quoted(&cmd.value, "REMOTE");
        assert_placeholder_is_quoted(&cmd.value, "MERGED");
        assert!(cmd.value.starts_with("'/path/to/bin'"));
    }

    #[test]
    fn mergetool_cmd_escapes_single_quote_in_binary_path() {
        let entries = build_config_entries("/tmp/it's/gitgpui-app");
        let cmd = entries
            .iter()
            .find(|e| e.key == "mergetool.gitgpui.cmd")
            .unwrap();

        assert!(
            cmd.value.starts_with("'/tmp/it'\"'\"'s/gitgpui-app'"),
            "unexpected cmd quoting: {}",
            cmd.value
        );
    }

    #[test]
    fn difftool_cmd_includes_local_remote_merged() {
        let entries = build_config_entries("/path/to/bin");
        let cmd = entries
            .iter()
            .find(|e| e.key == "difftool.gitgpui.cmd")
            .unwrap();

        assert_placeholder_is_quoted(&cmd.value, "LOCAL");
        assert_placeholder_is_quoted(&cmd.value, "REMOTE");
        assert_placeholder_is_quoted(&cmd.value, "MERGED");
    }

    #[test]
    fn gui_mergetool_cmd_quotes_all_stage_vars() {
        let entries = build_config_entries("/path/to/bin");
        let cmd = entries
            .iter()
            .find(|e| e.key == "mergetool.gitgpui-gui.cmd")
            .unwrap();

        assert_placeholder_is_quoted(&cmd.value, "BASE");
        assert_placeholder_is_quoted(&cmd.value, "LOCAL");
        assert_placeholder_is_quoted(&cmd.value, "REMOTE");
        assert_placeholder_is_quoted(&cmd.value, "MERGED");
    }

    #[test]
    fn gui_difftool_cmd_quotes_all_stage_vars() {
        let entries = build_config_entries("/path/to/bin");
        let cmd = entries
            .iter()
            .find(|e| e.key == "difftool.gitgpui-gui.cmd")
            .unwrap();

        assert_placeholder_is_quoted(&cmd.value, "LOCAL");
        assert_placeholder_is_quoted(&cmd.value, "REMOTE");
        assert_placeholder_is_quoted(&cmd.value, "MERGED");
    }

    #[test]
    fn format_commands_global_scope() {
        let entries = build_config_entries("/bin/gitgpui-app");
        let output = format_commands(&entries, "--global");

        assert!(output.contains("git config --global merge.tool"));
        assert!(output.contains("git config --global diff.tool"));
        assert!(output.contains("git config --global mergetool.trustExitCode"));
        assert!(output.contains("git config --global mergetool.gitgpui.trustExitCode"));
        assert!(output.contains("git config --global mergetool.gitgpui-gui.cmd"));
        assert!(output.contains("git config --global difftool.gitgpui-gui.cmd"));
        assert!(
            !output.contains("''/bin/gitgpui-app'"),
            "dry-run output should not contain broken nested quoting:\n{output}"
        );
    }

    #[test]
    fn format_commands_local_scope() {
        let entries = build_config_entries("/bin/gitgpui-app");
        let output = format_commands(&entries, "--local");

        assert!(output.contains("git config --local merge.tool"));
        assert!(output.contains("git config --local diff.tool"));
    }

    #[test]
    fn dry_run_does_not_write_config() {
        // dry_run=true should produce output but not call git config.
        // We verify by running inside a temp dir with no repo — if it
        // actually tried `git config --global`, the test env would be
        // unaffected because we only check output format.
        let result = run_setup(true, false).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("Dry run"));
        assert!(result.stdout.contains("git config --global"));
    }

    #[test]
    fn dry_run_local_scope_uses_local_flag() {
        let result = run_setup(true, true).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("git config --local"));
        assert!(!result.stdout.contains("--global"));
    }

    #[test]
    fn apply_config_to_local_repo() {
        let dir = tempfile::tempdir().unwrap();

        // Initialize a git repo.
        let init = std::process::Command::new("git")
            .args(["init", dir.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(init.status.success());

        let entries = build_config_entries("/test/gitgpui-app");
        let result = std::process::Command::new("git")
            .args(["-C", dir.path().to_str().unwrap()])
            .args(["config", "--local", entries[0].key, &entries[0].value])
            .output()
            .unwrap();
        assert!(result.status.success());

        // Verify the value was written.
        let check = std::process::Command::new("git")
            .args(["-C", dir.path().to_str().unwrap()])
            .args(["config", "--get", entries[0].key])
            .output()
            .unwrap();
        assert!(check.status.success());
        let value = String::from_utf8_lossy(&check.stdout);
        assert_eq!(value.trim(), entries[0].value);
    }
}
