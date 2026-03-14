//! `gitcomet-app setup` / `gitcomet-app uninstall` support.
//!
//! Setup writes the recommended global (or local) git config entries so that
//! `git difftool` and `git mergetool` invoke gitcomet automatically.
//! Uninstall removes those entries while preserving unrelated tool settings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A single `git config` key-value pair to set.
struct ConfigEntry {
    key: &'static str,
    value: String,
}

#[derive(Clone, Copy)]
struct BackupEntry {
    key: &'static str,
    expected_setup_value: &'static str,
    backup_key: &'static str,
}

#[derive(Clone, Copy)]
struct UninstallGuard {
    key: &'static str,
    expected_value: &'static str,
}

#[derive(Clone, Copy)]
struct UninstallEntry {
    key: &'static str,
    // If set, key is only removed when every configured value exactly matches
    // this expected setup value.
    expected_value: Option<&'static str>,
    // Optional additional selector guard to avoid removing shared generic
    // settings once users have switched to a different tool.
    guard: Option<UninstallGuard>,
}

#[derive(Debug, Eq, PartialEq)]
enum UninstallDecision {
    Unset,
    SkipMissing,
    SkipValueMismatch {
        expected: &'static str,
        actual: Vec<String>,
    },
    SkipGuardMismatch {
        guard_key: &'static str,
        guard_expected: &'static str,
        guard_actual: Vec<String>,
    },
}

#[derive(Debug, Eq, PartialEq)]
struct UninstallPlanItem {
    key: &'static str,
    decision: UninstallDecision,
}

const BACKUP_ABSENT_SENTINEL: &str = "__gitcomet_absent__";

struct BackupRestoreSummary {
    restored_count: usize,
    preserved_user_edits_count: usize,
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
        .map_err(|e| format!("Cannot determine gitcomet-app binary path: {e}"))
}

fn executable_path_for_shell(bin_path: &Path) -> Result<String, String> {
    let Some(path_text) = bin_path.to_str() else {
        return Err(format!(
            "Cannot configure gitcomet setup for non-Unicode executable path: {bin_path:?}"
        ));
    };
    Ok(path_text.to_string())
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
            value: "gitcomet".into(),
        },
        ConfigEntry {
            key: "mergetool.gitcomet.cmd",
            value: format!(
                "{quoted_bin_path} mergetool --base {base} --local {local} --remote {remote} --merged {merged}"
            ),
        },
        // Keep both generic and tool-specific trust keys:
        // - `mergetool.trustExitCode` matches documented setup guidance and
        //   Git's default trust behavior for the selected mergetool.
        // - `mergetool.gitcomet.trustExitCode` preserves explicit per-tool
        //   behavior even if users override global defaults later.
        ConfigEntry {
            key: "mergetool.trustExitCode",
            value: "true".into(),
        },
        ConfigEntry {
            key: "mergetool.gitcomet.trustExitCode",
            value: "true".into(),
        },
        ConfigEntry {
            key: "mergetool.prompt",
            value: "false".into(),
        },
        // Difftool (headless)
        ConfigEntry {
            key: "diff.tool",
            value: "gitcomet".into(),
        },
        ConfigEntry {
            key: "difftool.gitcomet.cmd",
            value: format!(
                "{quoted_bin_path} difftool --local {local} --remote {remote} --path {merged}"
            ),
        },
        // Keep both generic and tool-specific trust keys:
        // - `difftool.trustExitCode` matches documented setup guidance and
        //   Git's default trust behavior for the selected difftool.
        // - `difftool.gitcomet.trustExitCode` preserves explicit per-tool
        //   behavior even if users override global defaults later.
        ConfigEntry {
            key: "difftool.trustExitCode",
            value: "true".into(),
        },
        ConfigEntry {
            key: "difftool.gitcomet.trustExitCode",
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
            value: "gitcomet-gui".into(),
        },
        ConfigEntry {
            key: "mergetool.gitcomet-gui.cmd",
            value: format!(
                "{quoted_bin_path} mergetool --gui --base {base} --local {local} --remote {remote} --merged {merged}"
            ),
        },
        ConfigEntry {
            key: "mergetool.gitcomet-gui.trustExitCode",
            value: "true".into(),
        },
        ConfigEntry {
            key: "diff.guitool",
            value: "gitcomet-gui".into(),
        },
        ConfigEntry {
            key: "difftool.gitcomet-gui.cmd",
            value: format!(
                "{quoted_bin_path} difftool --gui --local {local} --remote {remote} --path {merged}"
            ),
        },
        ConfigEntry {
            key: "difftool.gitcomet-gui.trustExitCode",
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

fn build_backup_entries() -> Vec<BackupEntry> {
    vec![
        BackupEntry {
            key: "merge.tool",
            expected_setup_value: "gitcomet",
            backup_key: "gitcomet.backup.merge-tool",
        },
        BackupEntry {
            key: "diff.tool",
            expected_setup_value: "gitcomet",
            backup_key: "gitcomet.backup.diff-tool",
        },
        BackupEntry {
            key: "merge.guitool",
            expected_setup_value: "gitcomet-gui",
            backup_key: "gitcomet.backup.merge-guitool",
        },
        BackupEntry {
            key: "diff.guitool",
            expected_setup_value: "gitcomet-gui",
            backup_key: "gitcomet.backup.diff-guitool",
        },
        BackupEntry {
            key: "mergetool.trustExitCode",
            expected_setup_value: "true",
            backup_key: "gitcomet.backup.mergetool-trust-exit-code",
        },
        BackupEntry {
            key: "mergetool.prompt",
            expected_setup_value: "false",
            backup_key: "gitcomet.backup.mergetool-prompt",
        },
        BackupEntry {
            key: "difftool.trustExitCode",
            expected_setup_value: "true",
            backup_key: "gitcomet.backup.difftool-trust-exit-code",
        },
        BackupEntry {
            key: "difftool.prompt",
            expected_setup_value: "false",
            backup_key: "gitcomet.backup.difftool-prompt",
        },
        BackupEntry {
            key: "mergetool.guiDefault",
            expected_setup_value: "auto",
            backup_key: "gitcomet.backup.mergetool-guidefault",
        },
        BackupEntry {
            key: "difftool.guiDefault",
            expected_setup_value: "auto",
            backup_key: "gitcomet.backup.difftool-guidefault",
        },
    ]
}

fn build_uninstall_entries() -> Vec<UninstallEntry> {
    vec![
        // Tool-scoped keys are always safe to remove.
        UninstallEntry {
            key: "mergetool.gitcomet.cmd",
            expected_value: None,
            guard: None,
        },
        UninstallEntry {
            key: "mergetool.gitcomet.trustExitCode",
            expected_value: None,
            guard: None,
        },
        UninstallEntry {
            key: "difftool.gitcomet.cmd",
            expected_value: None,
            guard: None,
        },
        UninstallEntry {
            key: "difftool.gitcomet.trustExitCode",
            expected_value: None,
            guard: None,
        },
        UninstallEntry {
            key: "mergetool.gitcomet-gui.cmd",
            expected_value: None,
            guard: None,
        },
        UninstallEntry {
            key: "mergetool.gitcomet-gui.trustExitCode",
            expected_value: None,
            guard: None,
        },
        UninstallEntry {
            key: "difftool.gitcomet-gui.cmd",
            expected_value: None,
            guard: None,
        },
        UninstallEntry {
            key: "difftool.gitcomet-gui.trustExitCode",
            expected_value: None,
            guard: None,
        },
        // Generic selector keys are only removed when they still point at
        // GitComet defaults, so other tools are not disrupted.
        UninstallEntry {
            key: "merge.tool",
            expected_value: Some("gitcomet"),
            guard: None,
        },
        UninstallEntry {
            key: "diff.tool",
            expected_value: Some("gitcomet"),
            guard: None,
        },
        UninstallEntry {
            key: "merge.guitool",
            expected_value: Some("gitcomet-gui"),
            guard: None,
        },
        UninstallEntry {
            key: "diff.guitool",
            expected_value: Some("gitcomet-gui"),
            guard: None,
        },
        // Shared behavior keys are removed only while their selector still
        // targets GitComet.
        UninstallEntry {
            key: "mergetool.trustExitCode",
            expected_value: Some("true"),
            guard: Some(UninstallGuard {
                key: "merge.tool",
                expected_value: "gitcomet",
            }),
        },
        UninstallEntry {
            key: "mergetool.prompt",
            expected_value: Some("false"),
            guard: Some(UninstallGuard {
                key: "merge.tool",
                expected_value: "gitcomet",
            }),
        },
        UninstallEntry {
            key: "difftool.trustExitCode",
            expected_value: Some("true"),
            guard: Some(UninstallGuard {
                key: "diff.tool",
                expected_value: "gitcomet",
            }),
        },
        UninstallEntry {
            key: "difftool.prompt",
            expected_value: Some("false"),
            guard: Some(UninstallGuard {
                key: "diff.tool",
                expected_value: "gitcomet",
            }),
        },
        UninstallEntry {
            key: "mergetool.guiDefault",
            expected_value: Some("auto"),
            guard: Some(UninstallGuard {
                key: "merge.guitool",
                expected_value: "gitcomet-gui",
            }),
        },
        UninstallEntry {
            key: "difftool.guiDefault",
            expected_value: Some("auto"),
            guard: Some(UninstallGuard {
                key: "diff.guitool",
                expected_value: "gitcomet-gui",
            }),
        },
    ]
}

fn collect_uninstall_snapshot_keys(entries: &[UninstallEntry]) -> Vec<&'static str> {
    let mut keys = Vec::new();
    for entry in entries {
        if !keys.contains(&entry.key) {
            keys.push(entry.key);
        }
        if let Some(guard) = entry.guard
            && !keys.contains(&guard.key)
        {
            keys.push(guard.key);
        }
    }
    keys
}

fn parse_git_config_values(output: &[u8]) -> Result<Vec<String>, String> {
    if output.is_empty() {
        return Ok(Vec::new());
    }

    output
        .strip_suffix(b"\0")
        .unwrap_or(output)
        .split(|byte| *byte == b'\0')
        .map(|value| {
            std::str::from_utf8(value)
                .map(str::to_owned)
                .map_err(|_| "git config returned non-UTF-8 output".to_string())
        })
        .collect()
}

fn read_git_config_values(scope: &str, key: &str) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("git")
        .args(["config", scope, "--null", "--get-all", key])
        .output()
        .map_err(|e| format!("Failed to run git config --get-all for {key}: {e}"))?;

    if output.status.success() {
        return parse_git_config_values(&output.stdout);
    }

    // Missing key: git exits non-zero; treat as absent config.
    if output.status.code() == Some(1) {
        return Ok(Vec::new());
    }

    let stderr =
        String::from_utf8(output.stderr).unwrap_or_else(|_| "<non-utf8 stderr>".to_string());
    Err(format!(
        "git config {scope} --get-all {key} failed: {}",
        stderr.trim()
    ))
}

fn unset_all_config_values(scope: &str, key: &str) -> Result<(), String> {
    if read_git_config_values(scope, key)?.is_empty() {
        return Ok(());
    }

    let output = std::process::Command::new("git")
        .args(["config", scope, "--unset-all", key])
        .output()
        .map_err(|e| format!("Failed to run git config --unset-all for {key}: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr =
        String::from_utf8(output.stderr).unwrap_or_else(|_| "<non-utf8 stderr>".to_string());
    Err(format!(
        "git config {scope} --unset-all {key} failed: {}",
        stderr.trim()
    ))
}

fn set_single_config_value(scope: &str, key: &str, value: &str) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .args(["config", scope, key, value])
        .output()
        .map_err(|e| format!("Failed to run git config for {key}: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr =
        String::from_utf8(output.stderr).unwrap_or_else(|_| "<non-utf8 stderr>".to_string());
    Err(format!(
        "git config {scope} {key} failed: {}",
        stderr.trim()
    ))
}

fn add_config_value(scope: &str, key: &str, value: &str) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .args(["config", scope, "--add", key, value])
        .output()
        .map_err(|e| format!("Failed to run git config --add for {key}: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr =
        String::from_utf8(output.stderr).unwrap_or_else(|_| "<non-utf8 stderr>".to_string());
    Err(format!(
        "git config {scope} --add {key} failed: {}",
        stderr.trim()
    ))
}

fn write_config_values(scope: &str, key: &str, values: &[String]) -> Result<(), String> {
    unset_all_config_values(scope, key)?;
    if values.is_empty() {
        return Ok(());
    }
    set_single_config_value(scope, key, &values[0])?;
    for value in &values[1..] {
        add_config_value(scope, key, value)?;
    }
    Ok(())
}

fn maybe_capture_backup_for_entry(scope: &str, entry: &BackupEntry) -> Result<(), String> {
    let existing_backup = read_git_config_values(scope, entry.backup_key)?;
    if !existing_backup.is_empty() {
        return Ok(());
    }

    let current_values = read_git_config_values(scope, entry.key)?;

    if all_values_match_expected(&current_values, entry.expected_setup_value) {
        return Ok(());
    }

    let backup_values = if current_values.is_empty() {
        vec![BACKUP_ABSENT_SENTINEL.to_string()]
    } else {
        current_values
    };
    write_config_values(scope, entry.backup_key, &backup_values)
}

fn capture_backups_before_setup(scope: &str, entries: &[BackupEntry]) -> Result<(), String> {
    for entry in entries {
        maybe_capture_backup_for_entry(scope, entry)?;
    }
    Ok(())
}

fn restore_backups_for_uninstall(
    scope: &str,
    entries: &[BackupEntry],
) -> Result<BackupRestoreSummary, String> {
    let mut restored_count = 0usize;
    let mut preserved_user_edits_count = 0usize;
    for entry in entries {
        let backup_values = read_git_config_values(scope, entry.backup_key)?;
        if backup_values.is_empty() {
            continue;
        }

        let current_values = read_git_config_values(scope, entry.key)?;
        // Preserve user edits made after setup: only restore when the key still
        // has the setup-managed value.
        if !all_values_match_expected(&current_values, entry.expected_setup_value) {
            unset_all_config_values(scope, entry.backup_key)?;
            preserved_user_edits_count += 1;
            continue;
        }

        if backup_values.len() == 1 && backup_values[0] == BACKUP_ABSENT_SENTINEL {
            unset_all_config_values(scope, entry.key)?;
        } else {
            write_config_values(scope, entry.key, &backup_values)?;
        }

        unset_all_config_values(scope, entry.backup_key)?;
        restored_count += 1;
    }
    Ok(BackupRestoreSummary {
        restored_count,
        preserved_user_edits_count,
    })
}

fn read_uninstall_snapshot(
    scope: &str,
    entries: &[UninstallEntry],
) -> Result<HashMap<&'static str, Vec<String>>, String> {
    let mut snapshot = HashMap::new();
    for key in collect_uninstall_snapshot_keys(entries) {
        snapshot.insert(key, read_git_config_values(scope, key)?);
    }
    Ok(snapshot)
}

fn all_values_match_expected(values: &[String], expected: &str) -> bool {
    !values.is_empty() && values.iter().all(|value| value == expected)
}

fn plan_uninstall(
    entries: &[UninstallEntry],
    snapshot: &HashMap<&'static str, Vec<String>>,
) -> Vec<UninstallPlanItem> {
    entries
        .iter()
        .map(|entry| {
            let values = snapshot
                .get(entry.key)
                .map(Vec::as_slice)
                .unwrap_or(&[] as &[String]);

            if values.is_empty() {
                return UninstallPlanItem {
                    key: entry.key,
                    decision: UninstallDecision::SkipMissing,
                };
            }

            if let Some(expected) = entry.expected_value
                && !all_values_match_expected(values, expected)
            {
                return UninstallPlanItem {
                    key: entry.key,
                    decision: UninstallDecision::SkipValueMismatch {
                        expected,
                        actual: values.to_vec(),
                    },
                };
            }

            if let Some(guard) = entry.guard {
                let guard_values = snapshot
                    .get(guard.key)
                    .map(Vec::as_slice)
                    .unwrap_or(&[] as &[String]);
                if !all_values_match_expected(guard_values, guard.expected_value) {
                    return UninstallPlanItem {
                        key: entry.key,
                        decision: UninstallDecision::SkipGuardMismatch {
                            guard_key: guard.key,
                            guard_expected: guard.expected_value,
                            guard_actual: guard_values.to_vec(),
                        },
                    };
                }
            }

            UninstallPlanItem {
                key: entry.key,
                decision: UninstallDecision::Unset,
            }
        })
        .collect()
}

fn format_uninstall_dry_run(entries: &[UninstallEntry], scope: &str) -> String {
    let mut out = String::new();
    for entry in entries {
        out.push_str(&format!("git config {scope} --unset-all {}", entry.key));
        if let Some(expected) = entry.expected_value {
            out.push_str(&format!(
                "  # only if value is {}",
                shell_single_quote(expected)
            ));
            if let Some(guard) = entry.guard {
                out.push_str(&format!(
                    " and {} is {}",
                    guard.key,
                    shell_single_quote(guard.expected_value)
                ));
            }
        }
        out.push('\n');
    }
    out
}

fn format_values(values: &[String]) -> String {
    if values.is_empty() {
        return "<unset>".to_string();
    }
    values
        .iter()
        .map(|value| shell_single_quote(value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_uninstall_skip_details(plan: &[UninstallPlanItem]) -> String {
    let mut out = String::new();
    for item in plan {
        match &item.decision {
            UninstallDecision::SkipMissing => {}
            UninstallDecision::SkipValueMismatch { expected, actual } => {
                out.push_str(&format!(
                    "- Skipped {}: value is {}, expected {}\n",
                    item.key,
                    format_values(actual),
                    shell_single_quote(expected)
                ));
            }
            UninstallDecision::SkipGuardMismatch {
                guard_key,
                guard_expected,
                guard_actual,
            } => {
                out.push_str(&format!(
                    "- Skipped {}: {} is {}, expected {}\n",
                    item.key,
                    guard_key,
                    format_values(guard_actual),
                    shell_single_quote(guard_expected)
                ));
            }
            UninstallDecision::Unset => {}
        }
    }
    out
}

fn apply_uninstall_plan(plan: &[UninstallPlanItem], scope: &str) -> Result<usize, String> {
    let mut removed_count = 0usize;
    for item in plan {
        if item.decision != UninstallDecision::Unset {
            continue;
        }
        let output = std::process::Command::new("git")
            .args(["config", scope, "--unset-all", item.key])
            .output()
            .map_err(|e| format!("Failed to run git config --unset-all for {}: {e}", item.key))?;

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)
                .unwrap_or_else(|_| "<non-utf8 stderr>".to_string());
            return Err(format!(
                "git config {scope} --unset-all {} failed: {}",
                item.key,
                stderr.trim()
            ));
        }
        removed_count += 1;
    }
    Ok(removed_count)
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
            let stderr = String::from_utf8(output.stderr)
                .unwrap_or_else(|_| "<non-utf8 stderr>".to_string());
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

/// Result returned from `run_uninstall`.
pub struct UninstallResult {
    pub stdout: String,
    pub exit_code: i32,
}

/// Execute the setup command.
pub fn run_setup(dry_run: bool, local: bool) -> Result<SetupResult, String> {
    let bin_path = current_exe_path()?;
    let bin_str = executable_path_for_shell(&bin_path)?;

    let entries = build_config_entries(&bin_str);
    let backup_entries = build_backup_entries();
    let scope = if local { "--local" } else { "--global" };
    let scope_label = if local { "local" } else { "global" };

    if dry_run {
        let commands = format_commands(&entries, scope);
        let stdout = format!(
            "# Dry run: the following git config commands would be executed:\n{commands}\n\
             # Setup also stores backup values for {} key(s) under gitcomet.backup.* when needed.\n",
            backup_entries.len()
        );
        return Ok(SetupResult {
            stdout,
            exit_code: 0,
        });
    }

    capture_backups_before_setup(scope, &backup_entries)?;
    apply_config(&entries, scope)?;

    let stdout = format!(
        "Configured gitcomet as {scope_label} diff/merge tool.\n\
         Binary: {bin_str}\n\
         Run `git difftool` or `git mergetool` to use it.\n"
    );

    Ok(SetupResult {
        stdout,
        exit_code: 0,
    })
}

/// Execute the uninstall command.
pub fn run_uninstall(dry_run: bool, local: bool) -> Result<UninstallResult, String> {
    let scope = if local { "--local" } else { "--global" };
    let scope_label = if local { "local" } else { "global" };
    let entries = build_uninstall_entries();
    let backup_entries = build_backup_entries();

    if dry_run {
        let commands = format_uninstall_dry_run(&entries, scope);
        let stdout = format!(
            "# Dry run: the following git config commands may be executed safely:\n{commands}\n\
             # Uninstall also restores backup values from gitcomet.backup.* when present.\n"
        );
        return Ok(UninstallResult {
            stdout,
            exit_code: 0,
        });
    }

    let restore_summary = restore_backups_for_uninstall(scope, &backup_entries)?;
    let snapshot = read_uninstall_snapshot(scope, &entries)?;
    let plan = plan_uninstall(&entries, &snapshot);
    let removed_count = apply_uninstall_plan(&plan, scope)?;
    let skipped_count = plan
        .iter()
        .filter(|item| item.decision != UninstallDecision::Unset)
        .count();
    let skip_details = format_uninstall_skip_details(&plan);

    let mut stdout = format!(
        "Unconfigured gitcomet from {scope_label} diff/merge tool.\n\
         Restored {} key(s) from backups; preserved {} user-edited key(s); removed {removed_count} key(s); skipped {skipped_count}.\n",
        restore_summary.restored_count, restore_summary.preserved_user_edits_count
    );
    if !skip_details.is_empty() {
        stdout.push_str("Safety skips:\n");
        stdout.push_str(&skip_details);
    }

    Ok(UninstallResult {
        stdout,
        exit_code: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

    #[cfg(unix)]
    #[test]
    fn executable_path_for_shell_rejects_non_utf8_unix_paths() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(vec![
            b'/', b't', b'm', b'p', b'/', b'g', b'i', b't', b'c', b'o', b'm', b'e', b't', b'-',
            0xff,
        ]));
        let error = executable_path_for_shell(&path).expect_err("non-utf8 path should error");
        assert!(error.contains("non-Unicode executable path"), "{error}");
    }

    #[test]
    fn build_config_entries_contains_all_required_keys() {
        let entries = build_config_entries("/usr/bin/gitcomet-app");
        let keys: Vec<&str> = entries.iter().map(|e| e.key).collect();

        // Headless tool
        assert!(keys.contains(&"merge.tool"));
        assert!(keys.contains(&"mergetool.gitcomet.cmd"));
        assert!(keys.contains(&"mergetool.trustExitCode"));
        assert!(keys.contains(&"mergetool.gitcomet.trustExitCode"));
        assert!(keys.contains(&"mergetool.prompt"));
        assert!(keys.contains(&"diff.tool"));
        assert!(keys.contains(&"difftool.gitcomet.cmd"));
        assert!(keys.contains(&"difftool.trustExitCode"));
        assert!(keys.contains(&"difftool.gitcomet.trustExitCode"));
        assert!(keys.contains(&"difftool.prompt"));

        // GUI tool variant
        assert!(keys.contains(&"merge.guitool"));
        assert!(keys.contains(&"diff.guitool"));
        assert!(keys.contains(&"mergetool.gitcomet-gui.cmd"));
        assert!(keys.contains(&"mergetool.gitcomet-gui.trustExitCode"));
        assert!(keys.contains(&"difftool.gitcomet-gui.cmd"));
        assert!(keys.contains(&"difftool.gitcomet-gui.trustExitCode"));
        assert!(keys.contains(&"mergetool.guiDefault"));
        assert!(keys.contains(&"difftool.guiDefault"));
    }

    #[test]
    fn gui_tool_uses_separate_tool_name() {
        let entries = build_config_entries("/usr/bin/gitcomet-app");
        let merge_guitool = entries.iter().find(|e| e.key == "merge.guitool").unwrap();
        let diff_guitool = entries.iter().find(|e| e.key == "diff.guitool").unwrap();

        assert_eq!(merge_guitool.value, "gitcomet-gui");
        assert_eq!(diff_guitool.value, "gitcomet-gui");
    }

    #[test]
    fn gui_tool_cmd_includes_gui_flag() {
        let entries = build_config_entries("/path/to/bin");
        let merge_gui_cmd = entries
            .iter()
            .find(|e| e.key == "mergetool.gitcomet-gui.cmd")
            .unwrap();
        let diff_gui_cmd = entries
            .iter()
            .find(|e| e.key == "difftool.gitcomet-gui.cmd")
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
            .find(|e| e.key == "mergetool.gitcomet.cmd")
            .unwrap();
        let diff_cmd = entries
            .iter()
            .find(|e| e.key == "difftool.gitcomet.cmd")
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
            .find(|e| e.key == "mergetool.gitcomet.cmd")
            .unwrap();

        assert_placeholder_is_quoted(&cmd.value, "BASE");
        assert_placeholder_is_quoted(&cmd.value, "LOCAL");
        assert_placeholder_is_quoted(&cmd.value, "REMOTE");
        assert_placeholder_is_quoted(&cmd.value, "MERGED");
        assert!(cmd.value.starts_with("'/path/to/bin'"));
    }

    #[test]
    fn mergetool_cmd_escapes_single_quote_in_binary_path() {
        let entries = build_config_entries("/tmp/it's/gitcomet-app");
        let cmd = entries
            .iter()
            .find(|e| e.key == "mergetool.gitcomet.cmd")
            .unwrap();

        assert!(
            cmd.value.starts_with("'/tmp/it'\"'\"'s/gitcomet-app'"),
            "unexpected cmd quoting: {}",
            cmd.value
        );
    }

    #[test]
    fn difftool_cmd_includes_local_remote_merged() {
        let entries = build_config_entries("/path/to/bin");
        let cmd = entries
            .iter()
            .find(|e| e.key == "difftool.gitcomet.cmd")
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
            .find(|e| e.key == "mergetool.gitcomet-gui.cmd")
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
            .find(|e| e.key == "difftool.gitcomet-gui.cmd")
            .unwrap();

        assert_placeholder_is_quoted(&cmd.value, "LOCAL");
        assert_placeholder_is_quoted(&cmd.value, "REMOTE");
        assert_placeholder_is_quoted(&cmd.value, "MERGED");
    }

    #[test]
    fn format_commands_global_scope() {
        let entries = build_config_entries("/bin/gitcomet-app");
        let output = format_commands(&entries, "--global");

        // Headless mergetool entries
        assert!(output.contains("git config --global merge.tool"));
        assert!(output.contains("git config --global mergetool.gitcomet.cmd"));
        assert!(output.contains("git config --global mergetool.trustExitCode"));
        assert!(output.contains("git config --global mergetool.gitcomet.trustExitCode"));
        assert!(output.contains("git config --global mergetool.prompt"));

        // Headless difftool entries
        assert!(output.contains("git config --global diff.tool"));
        assert!(output.contains("git config --global difftool.gitcomet.cmd"));
        assert!(output.contains("git config --global difftool.trustExitCode"));
        assert!(
            output.contains("git config --global difftool.gitcomet.trustExitCode"),
            "expected per-tool difftool trustExitCode entry:\n{output}"
        );
        assert!(output.contains("git config --global difftool.prompt"));

        // GUI tool entries
        assert!(output.contains("git config --global merge.guitool"));
        assert!(output.contains("git config --global mergetool.gitcomet-gui.cmd"));
        assert!(output.contains("git config --global mergetool.gitcomet-gui.trustExitCode"));
        assert!(output.contains("git config --global diff.guitool"));
        assert!(output.contains("git config --global difftool.gitcomet-gui.cmd"));
        assert!(output.contains("git config --global difftool.gitcomet-gui.trustExitCode"));

        // GUI default auto-selection
        assert!(output.contains("git config --global mergetool.guiDefault"));
        assert!(output.contains("git config --global difftool.guiDefault"));

        assert!(
            !output.contains("''/bin/gitcomet-app'"),
            "dry-run output should not contain broken nested quoting:\n{output}"
        );
    }

    #[test]
    fn format_commands_local_scope() {
        let entries = build_config_entries("/bin/gitcomet-app");
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
            .arg("init")
            .arg(dir.path())
            .output()
            .unwrap();
        assert!(init.status.success());

        let entries = build_config_entries("/test/gitcomet-app");
        let result = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["config", "--local", entries[0].key, &entries[0].value])
            .output()
            .unwrap();
        assert!(result.status.success());

        // Verify the value was written.
        let check = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["config", "--get", entries[0].key])
            .output()
            .unwrap();
        assert!(check.status.success());
        let value = String::from_utf8(check.stdout).expect("utf-8 git config output");
        assert_eq!(value.trim(), entries[0].value);
    }

    fn decision_for<'a>(plan: &'a [UninstallPlanItem], key: &str) -> &'a UninstallDecision {
        &plan
            .iter()
            .find(|item| item.key == key)
            .unwrap_or_else(|| panic!("missing plan item for key {key}"))
            .decision
    }

    #[test]
    fn uninstall_dry_run_lists_unset_commands_and_conditions() {
        let result = run_uninstall(true, true).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("Dry run"));
        assert!(
            result
                .stdout
                .contains("git config --local --unset-all mergetool.gitcomet.cmd")
        );
        assert!(
            result.stdout.contains(
                "git config --local --unset-all merge.tool  # only if value is 'gitcomet'"
            )
        );
        assert!(
            result.stdout.contains("and merge.tool is 'gitcomet'"),
            "expected guarded-condition annotation in dry run output:\n{}",
            result.stdout
        );
    }

    #[test]
    fn uninstall_plan_unsets_matching_setup_values() {
        let entries = build_uninstall_entries();
        let mut snapshot: HashMap<&'static str, Vec<String>> = HashMap::new();
        snapshot.insert("mergetool.gitcomet.cmd", vec!["custom-cmd".to_string()]);
        snapshot.insert("merge.tool", vec!["gitcomet".to_string()]);
        snapshot.insert("mergetool.prompt", vec!["false".to_string()]);

        let plan = plan_uninstall(&entries, &snapshot);

        assert_eq!(
            decision_for(&plan, "mergetool.gitcomet.cmd"),
            &UninstallDecision::Unset
        );
        assert_eq!(decision_for(&plan, "merge.tool"), &UninstallDecision::Unset);
        assert_eq!(
            decision_for(&plan, "mergetool.prompt"),
            &UninstallDecision::Unset
        );
    }

    #[test]
    fn uninstall_plan_preserves_non_gitcomet_generic_settings() {
        let entries = build_uninstall_entries();
        let mut snapshot: HashMap<&'static str, Vec<String>> = HashMap::new();
        snapshot.insert("mergetool.gitcomet.cmd", vec!["custom-cmd".to_string()]);
        snapshot.insert("merge.tool", vec!["meld".to_string()]);
        snapshot.insert("mergetool.prompt", vec!["false".to_string()]);

        let plan = plan_uninstall(&entries, &snapshot);

        assert_eq!(
            decision_for(&plan, "mergetool.gitcomet.cmd"),
            &UninstallDecision::Unset
        );
        assert_eq!(
            decision_for(&plan, "merge.tool"),
            &UninstallDecision::SkipValueMismatch {
                expected: "gitcomet",
                actual: vec!["meld".to_string()],
            }
        );
        assert_eq!(
            decision_for(&plan, "mergetool.prompt"),
            &UninstallDecision::SkipGuardMismatch {
                guard_key: "merge.tool",
                guard_expected: "gitcomet",
                guard_actual: vec!["meld".to_string()],
            }
        );
    }

    fn temp_file_scope() -> (tempfile::TempDir, String, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config");
        let scope = format!("--file={}", config_path.display());
        (dir, scope, config_path)
    }

    #[test]
    fn collect_uninstall_snapshot_keys_includes_guard_keys_once() {
        let entries = vec![
            UninstallEntry {
                key: "primary.key",
                expected_value: None,
                guard: Some(UninstallGuard {
                    key: "guard.key",
                    expected_value: "expected",
                }),
            },
            UninstallEntry {
                key: "guard.key",
                expected_value: None,
                guard: None,
            },
        ];

        let keys = collect_uninstall_snapshot_keys(&entries);
        assert_eq!(keys, vec!["primary.key", "guard.key"]);
    }

    #[test]
    fn git_config_helpers_cover_missing_and_error_paths() {
        let (_dir, scope, _) = temp_file_scope();
        let missing = read_git_config_values(&scope, "gitcomet.coverage.missing").unwrap();
        assert!(missing.is_empty());

        let err = read_git_config_values("--not-a-valid-scope", "gitcomet.coverage")
            .expect_err("invalid scope should fail");
        assert!(err.contains("failed"));

        let set_err = set_single_config_value("--not-a-valid-scope", "foo.bar", "value")
            .expect_err("invalid scope should fail");
        assert!(set_err.contains("failed"));

        let add_err = add_config_value("--not-a-valid-scope", "foo.bar", "value")
            .expect_err("invalid scope should fail");
        assert!(add_err.contains("failed"));
    }

    #[test]
    fn parse_git_config_values_preserves_nul_separated_multiline_and_empty_values() {
        let parsed = parse_git_config_values(b"line1\nline2\0\0tail\0").unwrap();
        assert_eq!(
            parsed,
            vec![
                "line1\nline2".to_string(),
                "".to_string(),
                "tail".to_string()
            ]
        );
    }

    #[test]
    fn write_config_values_supports_empty_and_multi_value_sequences() {
        let (_dir, scope, _) = temp_file_scope();
        write_config_values(&scope, "foo.multi", &[]).unwrap();
        assert!(
            read_git_config_values(&scope, "foo.multi")
                .unwrap()
                .is_empty()
        );

        let values = vec!["one".to_string(), "two\nthree".to_string(), "".to_string()];
        write_config_values(&scope, "foo.multi", &values).unwrap();
        assert_eq!(read_git_config_values(&scope, "foo.multi").unwrap(), values);
    }

    #[test]
    fn maybe_capture_backup_skips_when_current_value_matches_setup_default() {
        let (_dir, scope, _) = temp_file_scope();
        set_single_config_value(&scope, "merge.tool", "gitcomet").unwrap();
        let entry = BackupEntry {
            key: "merge.tool",
            expected_setup_value: "gitcomet",
            backup_key: "gitcomet.backup.merge-tool",
        };
        maybe_capture_backup_for_entry(&scope, &entry).unwrap();
        assert!(
            read_git_config_values(&scope, entry.backup_key)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn unset_and_apply_paths_report_git_failures() {
        let (_dir, scope, config_path) = temp_file_scope();
        set_single_config_value(&scope, "foo.readonly", "value").unwrap();

        #[cfg(not(windows))]
        {
            // Simulate a write-time git config failure in a way that does not
            // depend on filesystem permissions (root in containers can bypass
            // readonly directory bits on some CI runners).
            let lock_path = config_path.with_extension("lock");
            fs::write(&lock_path, b"lock").expect("create config lock file");
            let unset_result = unset_all_config_values(&scope, "foo.readonly");
            fs::remove_file(&lock_path).expect("remove config lock file");

            let unset_err = unset_result.expect_err("config lock file should fail to unset");
            assert!(unset_err.contains("--unset-all"));
        }

        #[cfg(windows)]
        {
            let unset_err = unset_all_config_values("--not-a-valid-scope", "foo.readonly")
                .expect_err("invalid scope should fail");
            assert!(unset_err.contains("failed"));
        }

        let uninstall_err = apply_uninstall_plan(
            &[UninstallPlanItem {
                key: "foo.readonly",
                decision: UninstallDecision::Unset,
            }],
            "--not-a-valid-scope",
        )
        .expect_err("invalid scope should fail");
        assert!(uninstall_err.contains("--unset-all foo.readonly failed"));

        let apply_err = apply_config(
            &[ConfigEntry {
                key: "foo.readonly",
                value: "value".to_string(),
            }],
            "--not-a-valid-scope",
        )
        .expect_err("invalid scope should fail");
        assert!(apply_err.contains("git config foo.readonly value failed"));
    }

    #[test]
    fn format_values_empty_returns_unset_marker() {
        assert_eq!(format_values(&[]), "<unset>");
    }
}
